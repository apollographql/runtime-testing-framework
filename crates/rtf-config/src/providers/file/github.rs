use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    providers::{
        Result,
        file::{AsUtf8FileContent, Source},
    },
    templating::{self, Field, Scalar, Template},
};
use rtf_core::github::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The user specifies a path to a file within a GitHub repository, optionally providing a specific
/// ref of the repository to pull the file from. If no ref is providing then the provider will pull
/// the version of the file found on the default branch.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GithubFile {
    pub(crate) org: Field<String>,
    pub(crate) repo: Field<String>,
    pub(crate) path: Field<String>,
    pub(crate) git_ref: Option<Field<String>>,
}

impl Template for GithubFile {
    fn has_pending_fields(&self) -> bool {
        self.org.has_pending_fields()
            || self.repo.has_pending_fields()
            || self.path.has_pending_fields()
            || self
                .git_ref
                .as_ref()
                .map(|field| field.has_pending_fields())
                .unwrap_or(false)
    }

    fn required_values(&self) -> Vec<String> {
        let mut vals: Vec<String> = [&self.org, &self.repo, &self.path]
            .iter()
            .flat_map(|field| field.required_values())
            .collect();

        if let Some(field) = self.git_ref.as_ref() {
            vals.extend(field.required_values());
        }

        vals
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();
        errs.append(self.org.try_template(path, values));
        errs.append(self.repo.try_template(path, values));
        errs.append(self.path.try_template(path, values));

        if let Some(field) = self.git_ref.as_mut() {
            errs.append(field.try_template(path, values));
        }

        errs.into_result(())
    }
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
