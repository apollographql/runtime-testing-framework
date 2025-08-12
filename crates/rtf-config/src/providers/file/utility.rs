//! File providers that act as combinators or otherwise modify the output of other providers
use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    enum_impl_as_utf8_file_content, enum_impl_check, enum_impl_template, impl_template, merge_yaml,
    providers::{
        self, Result,
        file::{
            AsUtf8FileContent, InlineFile, RelativeFile, RequiredFile, Source, github::GithubFile,
        },
    },
    templating::{self, Scalar, Template},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::error;

/// A subset of file providers that can produce arbitrary utf-8 text
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TextFileProvider {
    GithubFile(GithubFile),
    Inline(InlineFile),
    RelativePath(RelativeFile),
    Required(RequiredFile),
}

// Each time we add a new variant to the TextFileProvider enum above we need to remember to add it
// to the macro invocation below in order to update the trait implementations for the enum. (You
// can't really forget to do this as the compiler will complain about missing match arms if you
// do!)
macro_rules! enum_impl_text_file_provider {
    ($($variant:ident),+) => {
        enum_impl_check!(TextFileProvider => $($variant),+);
        enum_impl_template!(TextFileProvider => $($variant),+);
        enum_impl_as_utf8_file_content!(TextFileProvider => $($variant),+);
    };
}

enum_impl_text_file_provider!(GithubFile, Inline, RelativePath, Required);

/// Merge the YAML output of two text based file providers into a single YAML file.
///
/// Matching keys in the overrides file will replace scalar values, concatenate arrays
/// and merge keys for maps.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MergeYaml {
    pub(crate) base: TextFileProvider,
    pub(crate) overrides: TextFileProvider,
}

impl AsUtf8FileContent for MergeYaml {
    async fn try_get_file_content(
        &self,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<String> {
        let base_str = self.base.try_get_file_content(src, ctx).await?;
        let overrides_str = self.overrides.try_get_file_content(src, ctx).await?;

        let mut base: serde_yaml::Value = match serde_yaml::from_str(&base_str) {
            Ok(v) => v,
            Err(e) => {
                error!("found invalid YAML in base file for merge_yaml file provider");
                return Err(e.into());
            }
        };
        let overrides: serde_yaml::Value = match serde_yaml::from_str(&overrides_str) {
            Ok(v) => v,
            Err(e) => {
                error!("found invalid YAML in overrides file for merge_yaml file provider");
                return Err(e.into());
            }
        };

        merge_yaml(overrides, &mut base);

        Ok(serde_yaml::to_string(&base)?.trim().to_string())
    }
}

impl_template!(MergeYaml => [base, overrides]);

impl Check for MergeYaml {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}
