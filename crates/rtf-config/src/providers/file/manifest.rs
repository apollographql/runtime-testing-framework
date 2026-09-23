use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    enum_impl_resolve_and_write,
    inlining::{self, Inline, InlineMode, InlinedProvider, provider_cache_key},
    providers::{
        self,
        file::{
            AsUtf8FileContent, InlineDir, InlineFile, RelativeDir, RelativeFile, RequiredFile,
            ResolveAndWrite, ResolveFileContent, StableSource, check_relative_path_specifiers,
            enum_impl_check, github::GithubFile, utility::TemplatedFile,
        },
    },
    run::{ExtractRelativeFiles, try_read_relative_dir, try_read_relative_file},
    templating::{self, Template, TemplateContext},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    pin::Pin,
    str::FromStr,
};

/// # Named Manifest File Provider
///
/// Shared metadata that wraps every manifest file provider. This is specifically for the
/// ManifestEnvironment and is implemented to restrict how users can specify manifest files in that
/// config.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct NamedManifestFileProvider {
    /// The name to use for the output produced by this provider
    ///
    /// For single-file providers, this is the output filename.
    /// For directory providers, this is the directory name.
    pub name: String,
    #[serde(flatten)]
    pub provider: ManifestFileProvider,
}

impl Deref for NamedManifestFileProvider {
    type Target = ManifestFileProvider;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

impl DerefMut for NamedManifestFileProvider {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.provider
    }
}

impl Template for NamedManifestFileProvider {
    fn required_variables(&self) -> Vec<String> {
        self.provider.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let tail = self.name.clone();
        self.provider
            .validate_context_nested(path, &tail, allowed_variables, file_source, ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        // Normalize the name: replace '.' and '/' with '_'
        let tail = self.name.replace(['.', '/'], "_");

        self.provider
            .try_template_nested(path, &tail, file_source, ctx)
    }
}

impl Check for NamedManifestFileProvider {
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
pub enum ManifestFileProvider {
    GithubFile(GithubFile),
    Inline(InlineFile),
    InlineDir(InlineDir),
    RelativeDir(RelativeDir),
    RelativePath(RelativeFile),
    Required(RequiredFile),
    Templated(TemplatedFile),
}

impl ManifestFileProvider {
    pub fn github_permalink(&self, ctx: &impl ResolutionContext) -> Option<String> {
        match self {
            Self::GithubFile(gh) => Some(gh.permalink()),

            Self::RelativePath(rf) => rf.src.as_ref().and_then(|src| {
                ctx.source_dir_for(src)
                    .github_permalink_for(rf.path.as_resolved(), true)
            }),

            Self::RelativeDir(rd) => rd.src.as_ref().and_then(|src| {
                ctx.source_dir_for(src)
                    .github_permalink_for(rd.path.as_resolved(), false)
            }),

            Self::Inline(_) | Self::InlineDir(_) | Self::Required(_) | Self::Templated(_) => None,
        }
    }
}

impl Inline for ManifestFileProvider {
    fn try_inline<'a>(
        &'a mut self,
        mode: InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if matches!(self, Self::Inline(_) | Self::InlineDir(_)) {
                return Ok(());
            }

            let key = provider_cache_key(&*self);
            if let Some(cached) = cache.get(&key) {
                match cached.clone() {
                    InlinedProvider::File(f) => *self = Self::Inline(f),
                    InlinedProvider::Dir(d) => *self = Self::InlineDir(d),
                }

                return Ok(());
            }

            match (&mut *self, mode) {
                (ManifestFileProvider::RelativeDir(inner), _) => {
                    *self =
                        ManifestFileProvider::InlineDir(inner.try_into_inline_files(ctx).await?);
                }
                (ManifestFileProvider::RelativePath(inner), _) => {
                    *self = ManifestFileProvider::Inline(inner.try_into_inline_file(ctx).await?);
                }
                (_, InlineMode::RelativeFiles) => return Ok(()),
                (ManifestFileProvider::GithubFile(inner), InlineMode::All) => {
                    *self = ManifestFileProvider::Inline(inner.try_into_inline_file(ctx).await?);
                }
                (
                    ManifestFileProvider::Inline(_)
                    | ManifestFileProvider::InlineDir(_)
                    | ManifestFileProvider::Templated(_),
                    InlineMode::All,
                ) => return Ok(()),
                (ManifestFileProvider::Required(inner), InlineMode::All) => {
                    *self = ManifestFileProvider::Inline(inner.try_into_inline_file(ctx).await?);
                }
            }

            match self {
                Self::Inline(f) => {
                    cache.insert(key, InlinedProvider::File(f.clone()));
                }
                Self::InlineDir(d) => {
                    cache.insert(key, InlinedProvider::Dir(d.clone()));
                }
                _ => {}
            }

            Ok(())
        })
    }
}

macro_rules! enum_impl_manifest_file_provider {
    ($($variant:ident,)+) => {
        enum_impl_check!(ManifestFileProvider => $($variant),+);
        enum_impl_resolve_and_write!(ManifestFileProvider => $($variant),+);
    };
}

enum_impl_manifest_file_provider!(
    GithubFile,
    Inline,
    InlineDir,
    RelativeDir,
    RelativePath,
    Required,
    Templated,
);

impl ExtractRelativeFiles for ManifestFileProvider {
    async fn try_extract_relative_files(
        &self,
        files: &mut HashMap<(StableSource, String), String>,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        match self {
            Self::RelativePath(rf) => try_read_relative_file(rf, files, ctx).await,
            Self::RelativeDir(rd) => try_read_relative_dir(rd, files, ctx).await,

            Self::GithubFile(_)
            | Self::Inline(_)
            | Self::InlineDir(_)
            | Self::Required(_)
            | Self::Templated(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        mock_context::MockContext,
        providers::file::{DirFile, SourceDir},
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

    #[test_case(Field::Pending("foo".to_string()), &["foo"]; "field is required")]
    #[test_case(Field::Resolved("foo".to_string()), &[]; "no fields required")]
    #[test]
    fn named_compose_file_provider_required_variables(f: Field<String>, expected: &[&str]) {
        let nfp = NamedManifestFileProvider {
            name: "inline.yaml".to_string(),
            provider: ManifestFileProvider::RelativePath(RelativeFile { path: f, src: None }),
        };

        let res = nfp.required_variables();
        assert_eq!(
            res, expected,
            "tests that required_variables has expected value"
        )
    }

    #[test]
    fn named_compose_file_provider_try_template_succeeds() {
        let mut nfp = NamedManifestFileProvider {
            name: "inline.yaml".to_string(),
            provider: ManifestFileProvider::RelativePath(RelativeFile {
                path: Field::Pending("path".to_string()),
                src: None,
            }),
        };
        let ctx = template_context!(&["path"]);

        let res = nfp.try_template(&mut Vec::new(), &StableSource::TestPlan, &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test]
    fn named_compose_file_provider_try_template_unknown_variable_error() {
        let mut nfp = NamedManifestFileProvider {
            name: "relative.yaml".to_string(),
            provider: ManifestFileProvider::RelativePath(RelativeFile {
                path: Field::Pending("path".to_string()),
                src: None,
            }),
        };
        let ctx = template_context!(&["unused"]);

        let res = nfp.try_template(&mut vec!["path".to_string()], &StableSource::TestPlan, &ctx);
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
        let nfp = NamedManifestFileProvider {
            name: "relative.yaml".to_string(),
            provider: ManifestFileProvider::RelativePath(RelativeFile {
                path: Field::Resolved("does/not/exist/relative.yaml".to_string()),
                src: Some(StableSource::TestPlan),
            }),
        };

        let ctx = MockContext::with_http_client(&[]).with_source(SourceDir::local("/foo"));
        let res = nfp.try_check(&mut vec!["path".to_string()], &ctx);
        assert!(res.is_err(), "expected to check to error, got {res:?}");
        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.path, "path.relative_yaml")
    }

    #[test]
    fn named_compose_file_provider_check_error_name_path_invalid() {
        let nfp = NamedManifestFileProvider {
            name: "../inline.yaml".to_string(),
            provider: ManifestFileProvider::Inline(InlineFile {
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
        let nfp = NamedManifestFileProvider {
            name: "compose-dir".to_string(),
            provider: ManifestFileProvider::InlineDir(InlineDir {
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

    #[tokio::test]
    async fn compose_file_provider_extract_relative_path() {
        use crate::providers::test_helpers::create_temp_dir_with_file;

        let content = "services: {}";
        let (temp, _) = create_temp_dir_with_file("compose.yaml", content);
        let ctx = MockContext::with_http_client(&[])
            .with_source(SourceDir::local(temp.path().canonicalize().unwrap()));

        let provider = ManifestFileProvider::RelativePath(RelativeFile {
            path: Field::Resolved("compose.yaml".to_string()),
            src: Some(StableSource::TestPlan),
        });
        let mut files = HashMap::new();

        let res = provider.try_extract_relative_files(&mut files, &ctx).await;

        assert!(res.is_ok(), "expected ok, got {res:?}");
        assert_eq!(
            files.get(&(StableSource::TestPlan, "compose.yaml".to_string())),
            Some(&content.to_string())
        );
    }

    #[tokio::test]
    async fn compose_file_provider_extract_relative_dir() {
        use crate::providers::test_helpers::create_temp_dir_with_file;

        let (temp, _) = create_temp_dir_with_file("dir/compose.yaml", "services: {}");
        let ctx = MockContext::with_http_client(&[])
            .with_source(SourceDir::local(temp.path().canonicalize().unwrap()));

        let provider = ManifestFileProvider::RelativeDir(RelativeDir {
            path: Field::Resolved("dir".to_string()),
            files: vec!["compose.yaml".to_string()],
            src: Some(StableSource::TestPlan),
        });
        let mut files = HashMap::new();

        let res = provider.try_extract_relative_files(&mut files, &ctx).await;

        assert!(res.is_ok(), "expected ok, got {res:?}");
        assert_eq!(
            files.get(&(StableSource::TestPlan, "dir/compose.yaml".to_string())),
            Some(&"services: {}".to_string())
        );
    }

    #[tokio::test]
    async fn compose_file_provider_extract_inline_unchanged() {
        let provider = ManifestFileProvider::Inline(InlineFile {
            content: "services: {}".to_string(),
        });
        let ctx = MockContext::with_http_client(&[]);
        let mut files = HashMap::new();

        let res = provider.try_extract_relative_files(&mut files, &ctx).await;

        assert!(res.is_ok(), "expected ok, got {res:?}");
        assert!(files.is_empty());
    }
}
