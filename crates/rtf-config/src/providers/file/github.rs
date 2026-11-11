use crate::{
    DirFile, StableSource,
    checks::{self, Check},
    context::ResolutionContext,
    providers,
    providers::{
        Result,
        file::{AsUtf8FileContent, ResolveFileContent, check_dir_files, unique_sorted_files},
    },
    run::ExtractRelativeFiles,
    templating::Field,
};
use rtf_derive::Template;
use rtf_integrations::github::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt, path::Path};

/// # GitHub file
///
/// The user specifies a path to a file within a GitHub repository, optionally providing a specific
/// ref of the repository to pull the file from. If no ref is providing then the provider will pull
/// the version of the file found on the default branch.
///
/// ```yaml
/// - name: "my-file.txt"
///   env_var: MY_FILE
///   kind: github_file
///   org: "my-org"
///   repo: "my-repo"
///   path: "resources/test-data/my-file.txt"
///   git_ref: "some-ref"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct GithubFile {
    /// The GitHub org for the repository containing the target file
    pub(crate) org: Field<String>,
    /// The GitHub repository containing the target file
    pub(crate) repo: Field<String>,
    /// The absolute path from the root of the repository to the target file
    pub(crate) path: Field<String>,
    /// An optional git reference to pull the file from. This may be a full or partial commit hash,
    /// branch name, or tag.
    ///
    /// Defaults to the mainline branch as specified in GitHub if unset.
    pub(crate) git_ref: Option<Field<String>>,
}

impl AsUtf8FileContent for GithubFile {
    async fn try_get_file_content(&self, ctx: &impl ResolutionContext) -> Result<String> {
        let client = ctx.github_client().expect("to have a GitHub client");
        let content = client
            .string_file_content(
                self.org.as_resolved(),
                self.repo.as_resolved(),
                self.path.as_resolved(),
                self.git_ref.as_ref().map(|field| field.as_resolved()),
            )
            .await?;

        Ok(content)
    }
}

impl Check for GithubFile {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        if ctx.github_client().is_none() {
            return Err(checks::Errors::new(
                checks::ErrorKind::MissingGithubApiKey,
                "expected os env key GITHUB_TOKEN",
                path,
            ));
        }

        Ok(())
    }
}

impl ExtractRelativeFiles for GithubFile {
    async fn try_extract_relative_files(
        &self,
        _files: &mut HashMap<(StableSource, String), String>,
        _ctx: &impl ResolutionContext,
    ) -> Result<()> {
        Ok(())
    }
}

impl GithubFile {
    pub fn permalink(&self) -> String {
        github_permalink(
            self.org.as_resolved(),
            self.repo.as_resolved(),
            self.git_ref.as_ref().map(|f| f.as_resolved().as_str()),
            self.path.as_resolved(),
            true,
        )
    }
}

#[inline(always)]
pub(crate) fn github_permalink(
    org: &str,
    repo: &str,
    git_ref: Option<&str>,
    path: impl fmt::Display,
    is_file: bool,
) -> String {
    let kind = if is_file { "blob" } else { "tree" };

    format!(
        "https://github.com/{org}/{repo}/{kind}/{}/{path}",
        git_ref.unwrap_or("HEAD")
    )
}

/// # GitHub Directory
///
/// The user specifies a path to a directory within a GitHub repository
///
/// The absolute path from the root of the repository to a target directory within GitHub,
/// a list of files that should be made available as part of the test run, and, optionally,
/// a specific ref of the repository to pull the directory from. If no ref is provided,
/// then the provider will use the directory found on the default branch.
///
/// Only the specified files will be included and there is no way to wildcard multiple files.
///
/// ```yaml
/// - name: "my-data"
///   env_var: MY_DATA
///   kind: github_dir
///   org: "my-org"
///   repo: "my-repo"
///   git_ref: "some-ref"
///   path: "resources/test-data/my-file.txt"
///   files:
///     - "my-file.txt"
///     - "nested/my-nested-file.json"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct GithubDir {
    /// The GitHub org for the repository containing the target directory
    pub(crate) org: Field<String>,
    /// The GitHub repository containing the target directory
    pub(crate) repo: Field<String>,
    /// An optional git reference to pull the file from. This may be a full or partial commit hash,
    /// branch name, or tag.
    ///
    /// Defaults to the mainline branch as specified in GitHub if unset.
    pub(crate) git_ref: Option<Field<String>>,
    /// The absolute path from the root of the repository to the target directory.
    pub(crate) path: Field<String>,
    /// The file paths under this directory that should be included.
    #[template(skip)]
    pub(crate) files: Vec<String>,
}

impl ResolveFileContent for GithubDir {
    async fn try_get_all_file_contents<P: AsRef<Path> + Send>(
        &self,
        target: P,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<DirFile>> {
        let client = ctx.github_client().expect("to have a GitHub client");

        // Dedup to ensure that we don't pull in files multiple times
        let files = unique_sorted_files(&self.files);

        let org = self.org.as_resolved();
        let repo = self.repo.as_resolved();
        let dir_path = self.path.as_resolved();
        let git_ref = self.git_ref.as_ref().map(|field| field.as_resolved());

        let mut contents = Vec::with_capacity(self.files.len());

        for fname in files {
            let content = client
                .string_file_content(org, repo, &format!("{dir_path}/{fname}"), git_ref)
                .await?;
            contents.push(DirFile {
                path: target.as_ref().join(fname),
                content,
            });
        }

        Ok(contents)
    }
}

impl Check for GithubDir {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        if ctx.github_client().is_none() {
            return Err(checks::Errors::new(
                checks::ErrorKind::MissingGithubApiKey,
                "expected os env key GITHUB_TOKEN",
                path,
            ));
        }

        let mut errs = checks::ErrorBuilder::new();
        let _ = check_dir_files(&self.files, path, &mut errs);

        errs.into_result(())
    }
}

impl ExtractRelativeFiles for GithubDir {
    async fn try_extract_relative_files(
        &self,
        _files: &mut HashMap<(StableSource, String), String>,
        _ctx: &impl ResolutionContext,
    ) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        inlining::{Inline, InlineMode},
        mock_context::MockContext,
        providers::{
            file::{
                FileProvider, InlineDir, InlineFile, ResolveAndWrite,
                tests::{assert_check_errors, assert_resolve_and_write_success},
            },
            test_helpers::assert_file_content,
        },
    };
    use assert_fs::{TempDir, fixture::PathChild};
    use std::{collections::HashMap, path::PathBuf};

    fn github_file() -> GithubFile {
        GithubFile {
            org: Field::Resolved("org".to_string()),
            repo: Field::Resolved("repo".to_string()),
            path: Field::Resolved("path".to_string()),
            git_ref: None,
        }
    }

    fn github_dir() -> GithubDir {
        GithubDir {
            org: Field::Resolved("org".to_string()),
            repo: Field::Resolved("repo".to_string()),
            git_ref: Some(Field::Resolved("some-ref".to_string())),
            path: Field::Resolved("path".to_string()),
            files: vec!["one.txt".to_string(), "nested/two.txt".to_string()],
        }
    }

    #[test]
    fn github_file_check_success() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let github_file = github_file();

        let mut ctx = Context::new();
        ctx.with_github_config("dummy_token");

        let res = github_file.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn github_file_check_missing_github_api_key() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let github_file = github_file();

        let ctx = Context::new();

        assert_check_errors(github_file, &ctx, &[checks::ErrorKind::MissingGithubApiKey]);
    }

    #[tokio::test]
    async fn github_file_inline_all_files_success() {
        let expected_content = "some content";
        let expected_inline_provider = FileProvider::Inline(InlineFile {
            content: expected_content.to_string(),
        });

        let ctx = MockContext::with_github_client(&[("org/repo/path", expected_content)]);
        let mut github_file = FileProvider::GithubFile(github_file());

        let res = github_file
            .try_inline(InlineMode::All, &ctx, &mut HashMap::new())
            .await;
        assert!(res.is_ok(), "expected provider to inline, got {res:?}");
        assert_eq!(
            github_file, expected_inline_provider,
            "Expected inline_all_files to update the file provider to an inline file provider with the expected GitHub content"
        );
    }

    #[tokio::test]
    #[should_panic(expected = "to have a GitHub client")]
    async fn github_file_inline_all_files_no_github_client_panics() {
        let ctx = Context::new();
        let mut github_file = FileProvider::GithubFile(github_file());

        let _res = github_file
            .try_inline(InlineMode::All, &ctx, &mut HashMap::new())
            .await;
    }

    #[tokio::test]
    async fn github_file_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("github.txt");

        let expected_content = "some content";

        let mut ctx = MockContext::with_github_client(&[("org/repo/path", expected_content)]);
        let github_file = FileProvider::GithubFile(github_file());

        assert_resolve_and_write_success(github_file, &target, &mut ctx, expected_content).await;
    }

    #[tokio::test]
    #[should_panic(expected = "to have a GitHub client")]
    async fn github_file_resolve_and_write_no_github_client_panics() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("github.txt");

        let mut ctx = Context::new();
        let github_file = FileProvider::GithubFile(github_file());

        let _res = github_file.resolve_and_write(&target, &mut ctx).await;
    }

    #[test]
    fn github_dir_check_success() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let github_dir = github_dir();

        let mut ctx = Context::new();
        ctx.with_github_config("dummy_token");

        let res = github_dir.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn github_dir_check_missing_github_api_key() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let github_dir = github_dir();

        let ctx = Context::new();

        assert_check_errors(github_dir, &ctx, &[checks::ErrorKind::MissingGithubApiKey]);
    }

    #[tokio::test]
    async fn github_dir_inline_all_files_success() {
        let expected_inline_provider = FileProvider::InlineDir(InlineDir {
            files: vec![
                DirFile {
                    path: PathBuf::from("nested/two.txt"),
                    content: "world".to_string(),
                },
                DirFile {
                    path: PathBuf::from("one.txt"),
                    content: "hello".to_string(),
                },
            ],
        });

        let ctx = MockContext::with_github_client(&[
            ("org/repo/path/one.txt?ref=some-ref", "hello"),
            ("org/repo/path/nested/two.txt?ref=some-ref", "world"),
        ]);
        let mut github_dir = FileProvider::GithubDir(github_dir());

        let res = github_dir
            .try_inline(InlineMode::All, &ctx, &mut HashMap::new())
            .await;
        assert!(res.is_ok(), "expected provider to inline, got {res:?}");
        assert_eq!(
            github_dir, expected_inline_provider,
            "Expected inline_all_files to update the file provider to an inline dir provider with the expected GitHub content"
        );
    }

    #[tokio::test]
    #[should_panic(expected = "to have a GitHub client")]
    async fn github_dir_inline_all_files_no_github_client_panics() {
        let ctx = Context::new();
        let mut github_dir = FileProvider::GithubDir(github_dir());

        let _res = github_dir
            .try_inline(InlineMode::All, &ctx, &mut HashMap::new())
            .await;
    }

    #[tokio::test]
    async fn github_dir_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("output");

        let mut ctx = MockContext::with_github_client(&[
            ("org/repo/path/one.txt?ref=some-ref", "hello"),
            ("org/repo/path/nested/two.txt?ref=some-ref", "world"),
        ]);
        let github_dir = FileProvider::GithubDir(github_dir());

        let res = github_dir.resolve_and_write(&target, &mut ctx).await;
        assert!(res.is_ok(), "expected Ok, got {res:?}");

        assert_file_content(&target.child("one.txt"), "hello");
        assert_file_content(&target.child("nested/two.txt"), "world");
    }

    #[tokio::test]
    #[should_panic(expected = "to have a GitHub client")]
    async fn github_dir_resolve_and_write_no_github_client_panics() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("github.txt");

        let mut ctx = Context::new();
        let github_dir = FileProvider::GithubDir(github_dir());

        let _res = github_dir.resolve_and_write(&target, &mut ctx).await;
    }
}
