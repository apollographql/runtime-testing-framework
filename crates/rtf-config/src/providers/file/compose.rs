use crate::{
    SourceDir,
    checks::{self, Check},
    context::ResolutionContext,
    enum_impl_resolve_and_write,
    inlining::{self, InlineMode},
    providers::file::{
        AsUtf8FileContent, InlineDir, InlineFile, RelativeDir, RelativeFile, RequiredFile,
        ResolveAndWrite, ResolveFileContent, check_relative_path_specifiers, enum_impl_check,
        github::GithubFile,
    },
    templating::{self, Template, TemplateContext},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    str::FromStr,
};

/// # Named Compose File Provider
///
/// Shared metadata that wraps every docker compose file provider. This is specifically
/// for the DockerComposeEnvironment and is implemented to restrict how users can specify
/// docker compose files in that config.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct NamedComposeFileProvider {
    /// The name to use for the output produced by this provider
    ///
    /// For single-file providers, this is the output filename.
    /// For directory providers, this is the directory name.
    pub name: String,
    #[serde(flatten)]
    pub provider: ComposeFileProvider,
}

impl Deref for NamedComposeFileProvider {
    type Target = ComposeFileProvider;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

impl DerefMut for NamedComposeFileProvider {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.provider
    }
}

impl Template for NamedComposeFileProvider {
    fn has_pending_fields(&self) -> bool {
        self.provider.has_pending_fields()
    }

    fn required_variables(&self) -> Vec<String> {
        self.provider.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let tail = self.name.clone();
        self.provider
            .validate_context_nested(path, &tail, allowed_variables, file_source, ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        // Normalize the name: replace '.' and '/' with '_'
        let tail = self.name.replace(['.', '/'], "_");

        self.provider
            .try_template_nested(path, &tail, file_source, ctx)
    }
}

impl Check for NamedComposeFileProvider {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        // Normalize the name: replace '.' and '/' with '_'
        let tail = self.name.clone().replace(['.', '/'], "_");

        let mut errs = checks::ErrorBuilder::new();
        let mut err_path = path.clone();
        err_path.push(tail.clone());

        let name_relative_path = match PathBuf::from_str(&self.name) {
            Ok(p) => p,
            Err(_) => {
                errs.push(
                    checks::ErrorKind::InvalidRelativePath,
                    "file provider name is not a valid path",
                    &err_path,
                );
                PathBuf::new()
            }
        };
        errs.append(check_relative_path_specifiers(
            &name_relative_path,
            &mut err_path,
        ));

        errs.append(self.provider.try_check_nested(path, tail, ctx));

        errs.into_result(())
    }
}

/// Compose file provider
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComposeFileProvider {
    GithubFile(GithubFile),
    Inline(InlineFile),
    InlineDir(InlineDir),
    RelativeDir(RelativeDir),
    RelativePath(RelativeFile),
    Required(RequiredFile),
}

impl ComposeFileProvider {
    pub async fn inline(
        &mut self,
        mode: &InlineMode,
        ctx: &impl ResolutionContext,
    ) -> inlining::Result<()> {
        match (&mut *self, mode) {
            (ComposeFileProvider::RelativeDir(inner), _) => {
                *self = ComposeFileProvider::InlineDir(inner.try_into_inline_files(ctx).await?);

                Ok(())
            }
            (ComposeFileProvider::RelativePath(inner), _) => {
                *self = ComposeFileProvider::Inline(inner.try_into_inline_file(ctx).await?);

                Ok(())
            }
            (_, InlineMode::RelativeFiles) => Ok(()),
            (ComposeFileProvider::GithubFile(inner), InlineMode::All) => {
                *self = ComposeFileProvider::Inline(inner.try_into_inline_file(ctx).await?);

                Ok(())
            }
            (
                ComposeFileProvider::Inline(_) | ComposeFileProvider::InlineDir(_),
                InlineMode::All,
            ) => Ok(()),
            (ComposeFileProvider::Required(inner), InlineMode::All) => {
                *self = ComposeFileProvider::Inline(inner.try_into_inline_file(ctx).await?);

                Ok(())
            }
        }
    }
}

macro_rules! enum_impl_compose_file_provider {
    ($($variant:ident,)+) => {
        enum_impl_check!(ComposeFileProvider => $($variant),+);
        enum_impl_resolve_and_write!(ComposeFileProvider => $($variant),+);
    };
}

enum_impl_compose_file_provider!(
    GithubFile,
    Inline,
    InlineDir,
    RelativeDir,
    RelativePath,
    Required,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        providers::file::DirFile,
        templating::{ErrorKind, Field, Scalar},
    };
    use simple_test_case::test_case;

    macro_rules! template_context {
        ($slice:expr) => {{
            let mut m = ::std::collections::HashMap::new();
            for k in $slice {
                m.insert(k.to_string(), Scalar::from(k.to_string()));
            }

            TemplateContext::new_stubbed(m)
        }};
    }

    #[test_case(Field::Pending("foo".to_string()), true; "field is pending")]
    #[test_case(Field::Resolved("foo".to_string()), false; "field is resolved")]
    #[test]
    fn named_compose_file_provider_has_pending_fields(f: Field<String>, expected: bool) {
        let nfp = NamedComposeFileProvider {
            name: "inline.yaml".to_string(),
            provider: ComposeFileProvider::RelativePath(RelativeFile { path: f, src: None }),
        };

        let res = nfp.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

    #[test_case(Field::Pending("foo".to_string()), &["foo"]; "field is required")]
    #[test_case(Field::Resolved("foo".to_string()), &[]; "no fields required")]
    #[test]
    fn named_compose_file_provider_required_variables(f: Field<String>, expected: &[&str]) {
        let nfp = NamedComposeFileProvider {
            name: "inline.yaml".to_string(),
            provider: ComposeFileProvider::RelativePath(RelativeFile { path: f, src: None }),
        };

        let res = nfp.required_variables();
        assert_eq!(
            res, expected,
            "tests that required_variables has expected value"
        )
    }

    #[test]
    fn named_compose_file_provider_try_template_succeeds() {
        let mut nfp = NamedComposeFileProvider {
            name: "inline.yaml".to_string(),
            provider: ComposeFileProvider::RelativePath(RelativeFile {
                path: Field::Pending("path".to_string()),
                src: None,
            }),
        };
        let ctx = template_context!(&["path"]);

        let res = nfp.try_template(&mut Vec::new(), &SourceDir::local("/"), &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test]
    fn named_compose_file_provider_try_template_unknown_variable_error() {
        let mut nfp = NamedComposeFileProvider {
            name: "relative.yaml".to_string(),
            provider: ComposeFileProvider::RelativePath(RelativeFile {
                path: Field::Pending("path".to_string()),
                src: None,
            }),
        };
        let ctx = template_context!(&["unused"]);

        let res = nfp.try_template(&mut vec!["path".to_string()], &SourceDir::local("/"), &ctx);
        assert!(res.is_err(), "expected templating to error, got {res:?}");

        let errors = res.unwrap_err();
        let error = errors.unwrap_single();
        let error_kind = error.kind;
        let error_path = error.path;
        assert_eq!(
            error_kind,
            ErrorKind::UnknownVariable,
            "expected ErrorKind to match"
        );
        assert_eq!(
            error_path, "path.relative_yaml.path",
            "expected path to match"
        )
    }

    #[test]
    fn named_compose_file_provider_check_error_path_correct() {
        let nfp = NamedComposeFileProvider {
            name: "relative.yaml".to_string(),
            provider: ComposeFileProvider::RelativePath(RelativeFile {
                path: Field::Resolved("does/not/exist/relative.yaml".to_string()),
                src: Some(SourceDir::local("/foo")),
            }),
        };

        let res = nfp.try_check(&mut vec!["path".to_string()], &Context::new());
        assert!(res.is_err(), "expected to check to error, got {res:?}");
        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.path, "path.relative_yaml")
    }

    #[test]
    fn named_compose_file_provider_check_error_name_path_invalid() {
        let nfp = NamedComposeFileProvider {
            name: "../inline.yaml".to_string(),
            provider: ComposeFileProvider::Inline(InlineFile {
                content: "content".to_string(),
            }),
        };

        let res = nfp.try_check(&mut vec!["path".to_string()], &Context::new());
        assert!(res.is_err(), "expected to check to error, got {res:?}");
        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.path, "path.___inline_yaml");
        assert_eq!(err.kind, checks::ErrorKind::InvalidPathSpecifiers)
    }

    #[test]
    fn inline_dir_compose_file_provider_check_succeeds() {
        let nfp = NamedComposeFileProvider {
            name: "compose-dir".to_string(),
            provider: ComposeFileProvider::InlineDir(InlineDir {
                files: vec![
                    DirFile {
                        path: PathBuf::from("base.yaml"),
                        content: "content".to_string(),
                    },
                    DirFile {
                        path: PathBuf::from("overlay.yaml"),
                        content: "content".to_string(),
                    },
                ],
            }),
        };

        let res = nfp.try_check(&mut Vec::new(), &Context::new());
        assert!(res.is_ok(), "Expected check to succeed, got {res:?}");
    }
}
