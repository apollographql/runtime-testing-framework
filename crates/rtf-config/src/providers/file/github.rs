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
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        if ctx.github_client().is_none() {
            return Err(checks::Errors::new(
                checks::ErrorKind::MissingGithubApiKey,
                "",
                path,
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::Context, providers::file::tests::assert_check_errors};

    #[test]
    fn try_check_github_file_success() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let github_file = GithubFile {
            org: Field::Resolved("org".to_string()),
            repo: Field::Resolved("repo".to_string()),
            path: Field::Resolved("path".to_string()),
            git_ref: None,
        };

        let mut ctx = Context::new();
        ctx.with_github_config("dummy_token");
        let src = Source::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        let res = github_file.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn try_check_github_file_missing_github_api_key() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let github_file = GithubFile {
            org: Field::Resolved("org".to_string()),
            repo: Field::Resolved("repo".to_string()),
            path: Field::Resolved("path".to_string()),
            git_ref: None,
        };

        let ctx = Context::new();
        let src = Source::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        assert_check_errors(
            github_file,
            &src,
            &ctx,
            &[checks::ErrorKind::MissingGithubApiKey],
        );
    }
}
