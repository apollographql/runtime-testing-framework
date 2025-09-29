//! File providers that act as combinators or otherwise modify the output of other providers
use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    enum_impl_as_utf8_file_content, enum_impl_check, merge_yaml,
    providers::{
        self, Result,
        command::CommandSection,
        file::{
            AsUtf8FileContent, InlineFile, RelativeFile, RequiredFile, ResolveAndWrite, Source,
            apollo::GraphosSubgraphRouterUrlOverrides, github::GithubFile,
        },
    },
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::error;

/// # Text File Provider
///
/// A subset of file providers that can produce arbitrary utf-8 text as their output.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TextFileProvider {
    GithubFile(GithubFile),
    GraphosSubgraphRouterUrlOverrides(GraphosSubgraphRouterUrlOverrides),
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
        enum_impl_as_utf8_file_content!(TextFileProvider => $($variant),+);
    };
}

enum_impl_text_file_provider!(
    GithubFile,
    GraphosSubgraphRouterUrlOverrides,
    Inline,
    RelativePath,
    Required
);

/// # Merge YAML
///
/// Merge the YAML output of two text based file providers into a single YAML file.
///
/// Matching keys in the overrides file will replace scalar values, concatenate arrays
/// and merge keys for maps.
///
/// ```yaml
/// - name: router-config.yaml
///   env_var: ROUTER_CONFIG
///   kind: merge_yaml
///   base:
///     kind: relative_path
///     path: "data/base-router-config.yaml"
///   overrides:
///     kind: graphos_subgraph_router_url_overrides
///     graph_ref: "foo@bar"
///     url_format: "docker"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct MergeYaml {
    /// A base YAML file to start with.
    pub(crate) base: TextFileProvider,
    /// An second YAML file to merge on top of the base file.
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

impl Check for MergeYaml {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();
        errs.append(self.base.try_check(path, src, ctx));
        errs.append(self.overrides.try_check(path, src, ctx));

        errs.into_result(())
    }
}

/// # From command
///
/// Run a command provider and use its output as a file provider resource.
///
/// As with all other command providers, you can provide both environment variables
/// and other file providers as inputs to the command being executed. RTF will use
/// the contents of the `$RTF_OUTPUT` path as the output of this provider, supporting
/// both writing a single file to that path and creating a directory at that path
/// containing multiple files.
///
/// ```yaml
/// - name: vegeta-ops.json
///   env_var: VEGETA_OPS
///   kind: from_command
///   command:
///     name: format-for-vegeta.sh
///     kind: relative_path
///     path: scripts/format-for-vegeta.sh
///   env_vars:
///     ROUTER_URL: "http://127.0.0.1:4000/"
///   file_providers:
///     - name: canned_ops.json
///       env_var: CANNED_OPS_FILE
///       kind: graphos_canned_ops
///       graph_ref: "my@graph"
///       top_n: 20
///       skip_mutations: true
/// ```
///
/// ### format-for-vegeta.sh
/// ```bash
/// #!/usr/bin/env sh
/// while read -r req; do
///   if [[ "$OSTYPE" == "darwin"* ]]; then
///     encoded=$(echo "$req" | base64 -b 0)
///   else
///     encoded=$(echo "$req" | base64 -w 0)
///   fi
///   
///   jq -nc \
///     --arg body "$encoded" \
///     --arg url "$ROUTER_URL" \
///     '{
///       "body": $body,
///       "header": { "Content-type": ["application/json"] },
///       "method": "POST",
///       "url": $url
///     }' >> "$RTF_OUTPUT"
/// done <"$CANNED_OPS_FILE"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct FromCommand {
    #[serde(flatten)]
    inner: CommandSection,
}

impl ResolveAndWrite for FromCommand {
    async fn try_get_all_file_contents(
        &self,
        _target: impl AsRef<Path>,
        _src: &Source,
        _ctx: &mut impl ResolutionContext,
    ) -> providers::Result<Vec<(PathBuf, String)>> {
        todo!("we need to break this method out of ResolveAndWrite and rework the providers tests");
    }

    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        let target = target.as_ref();
        let out_dir = ctx.dir_containing(target);
        self.inner
            .run_providers_and_execute(&out_dir, Some(target.into()), src, ctx)
            .await?;

        Ok(())
    }
}

impl Check for FromCommand {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        self.inner.try_check(path, src, ctx)
    }
}
