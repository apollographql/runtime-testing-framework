use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    providers::{
        Result,
        file::{AsUtf8FileContent, Source},
    },
    templating::Field,
};
use rtf_core::github::Client;
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// # GitHub File
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
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        mock_context::MockContext,
        providers::file::{
            FileProvider, ResolveAndWrite,
            tests::{assert_check_errors, assert_resolve_and_write_success},
        },
    };
    use assert_fs::{TempDir, fixture::PathChild};

    fn github_file() -> GithubFile {
        GithubFile {
            org: Field::Resolved("org".to_string()),
            repo: Field::Resolved("repo".to_string()),
            path: Field::Resolved("path".to_string()),
            git_ref: None,
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
    async fn github_file_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("github.txt");

        let expected_content = "some content";

        let mut ctx = MockContext::with_github_client(expected_content);
        let src = Source::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        let github_file = FileProvider::GithubFile(github_file());

        assert_resolve_and_write_success(github_file, &target, &src, &mut ctx, expected_content)
            .await;
    }

    #[tokio::test]
    #[should_panic(expected = "to have a GitHub client")]
    async fn github_file_resolve_and_write_no_github_client_panics() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("github.txt");

        let mut ctx = Context::new();
        let src = Source::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        let github_file = FileProvider::GithubFile(github_file());

        let _res = github_file.resolve_and_write(&target, &src, &mut ctx).await;
    }
}
