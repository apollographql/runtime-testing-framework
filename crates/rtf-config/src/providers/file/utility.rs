//! File providers that act as combinators or otherwise modify the output of other providers
use crate::{
    SourceDir,
    checks::{self, Check},
    context::ResolutionContext,
    enum_impl_as_utf8_file_content, enum_impl_check,
    inlining::{self, InlineMode},
    merge_yaml,
    providers::{
        self, Result,
        command::CommandSection,
        file::{
            AsUtf8FileContent, FileProvider, InlineFile, RelativeFile, RequiredFile,
            ResolveAndWrite, apollo::GraphosSubgraphRouterUrlOverrides, github::GithubFile,
        },
    },
    run::{Execute, RunProviders},
    templating::{
        self, Scalar, Template, TemplateContext, extract_template_vars, interpolate_variables,
    },
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use tracing::error;

/// # Templated file
///
/// Write a file whose content is an inline string with `${variable}` patterns interpolated
/// from the RTF template variables defined for the current run.
///
/// ```yaml
/// - name: config.json
///   env_var: CONFIG_FILE
///   kind: templated
///   content: |
///     { "endpoint": "${router_url}" }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct TemplatedFile {
    /// The file content with optional `${variable}` interpolation patterns.
    pub(crate) content: String,
}

impl Template for TemplatedFile {
    fn required_variables(&self) -> Vec<String> {
        extract_template_vars(&self.content)
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        _file_source: &SourceDir,
        _ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();

        for var in self.required_variables() {
            if !allowed_variables.contains(&var) {
                errs.push(templating::ErrorKind::UnknownVariable, &var, path);
            }
        }

        errs.into_result(())
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        _file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        match interpolate_variables(&self.content, ctx.variables()) {
            Ok(interpolated) => {
                self.content = interpolated;
                Ok(())
            }
            Err(unresolved) => {
                let mut errs = templating::ErrorBuilder::new();
                for var in unresolved {
                    errs.push(templating::ErrorKind::UnknownVariable, var, path);
                }
                errs.into_result(())
            }
        }
    }
}

impl AsUtf8FileContent for TemplatedFile {
    async fn try_get_file_content(
        &self,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        Ok(self.content.clone())
    }
}

impl Check for TemplatedFile {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}

/// # Merge file provider
///
/// A subset of file providers that can be merged into a base YAML file.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MergeFileProvider {
    GithubFile(GithubFile),
    GraphosSubgraphRouterUrlOverrides(GraphosSubgraphRouterUrlOverrides),
    Inline(InlineFile),
    RelativePath(RelativeFile),
    Required(RequiredFile),
    Templated(TemplatedFile),
}

impl MergeFileProvider {
    pub(crate) async fn inline_all_relative_paths<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> inlining::Result<()> {
        if let Self::RelativePath(relative_path) = self {
            *self = Self::Inline(relative_path.try_into_inline_file(ctx).await?);
        }

        Ok(())
    }
}

// Each time we add a new variant to the MergeFileProvider enum above we need to remember to add it
// to the macro invocation below in order to update the trait implementations for the enum. (You
// can't really forget to do this as the compiler will complain about missing match arms if you
// do!)
macro_rules! enum_impl_merge_file_provider {
    ($($variant:ident),+) => {
        enum_impl_check!(MergeFileProvider => $($variant),+);
        enum_impl_as_utf8_file_content!(MergeFileProvider => $($variant),+);
    };
}

enum_impl_merge_file_provider!(
    GithubFile,
    GraphosSubgraphRouterUrlOverrides,
    Inline,
    RelativePath,
    Required,
    Templated
);

/// # Merge YAML
///
/// Merge the YAML output of text based file providers into a single YAML file.
///
/// Matching keys in the overrides file will replace scalar values, concatenate arrays
/// and merge keys for maps.
///
/// When merging a single overrides file the overrides provider can be specified directly
/// under the `overrides` key:
/// ```yaml
/// - name: router-config.yaml
///   env_var: ROUTER_CONFIG
///   kind: merge_yaml
///   base:
///     kind: relative_path
///     path: "data/base-router-config.yaml"
///   overrides:
///     kind: relative_path
///     path: "../my-overrides.yaml"
/// ```
///
/// When merging multiple overrides files, specify the providers in the order you want
/// to merge them as an array:
/// ```yaml
/// - name: router-config.yaml
///   env_var: ROUTER_CONFIG
///   kind: merge_yaml
///   base:
///     kind: relative_path
///     path: "data/base-router-config.yaml"
///   overrides:
///     - kind: relative_path
///       path: "../my-overrides.yaml"
///     - kind: relative_path
///       path: "../my-other-overrides.yaml"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct MergeYaml {
    /// A base YAML file to start with.
    pub(crate) base: MergeFileProvider,
    /// One or more YAML files to merge on top of the base file in sequence.
    pub(crate) overrides: Overrides,
}

impl AsUtf8FileContent for MergeYaml {
    async fn try_get_file_content(&self, ctx: &impl ResolutionContext) -> Result<String> {
        let base_str = self.base.try_get_file_content(ctx).await?;
        let mut overrides = Vec::with_capacity(self.overrides.len());

        match &self.overrides {
            Overrides::One(t) => overrides.push(t.try_get_file_content(ctx).await?),
            Overrides::Array(ts) => {
                for t in ts.iter() {
                    overrides.push(t.try_get_file_content(ctx).await?);
                }
            }
        }

        let mut base: serde_yaml::Value = match serde_yaml::from_str(&base_str) {
            Ok(v) => v,
            Err(e) => {
                error!("found invalid YAML in base file for merge_yaml file provider");
                return Err(e.into());
            }
        };

        for (i, s) in overrides.iter().enumerate() {
            let overrides: serde_yaml::Value = match serde_yaml::from_str(s) {
                Ok(v) => v,
                Err(e) => {
                    let i = i + 1;
                    let n = self.overrides.len();
                    error!(
                        "found invalid YAML in overrides file {i}/{n} for merge_yaml file provider"
                    );
                    return Err(e.into());
                }
            };
            merge_yaml(overrides, &mut base);
        }

        Ok(serde_yaml::to_string(&base)?.trim().to_string())
    }
}

impl Check for MergeYaml {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();
        errs.append(self.base.try_check(path, ctx));

        match &self.overrides {
            Overrides::One(t) => errs.append(t.try_check(path, ctx)),
            Overrides::Array(ts) => {
                for t in ts.iter() {
                    errs.append(t.try_check(path, ctx));
                }
            }
        }

        errs.into_result(())
    }
}

impl MergeYaml {
    pub(crate) async fn inline_all_relative_paths<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> inlining::Result<()> {
        let mut errs = inlining::ErrorBuilder::new();

        errs.append(self.base.inline_all_relative_paths(ctx).await);
        errs.append(self.overrides.inline_all_relative_paths(ctx).await);

        errs.into_result(())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(untagged)]
pub enum Overrides {
    One(MergeFileProvider),
    Array(Vec<MergeFileProvider>),
}

impl Overrides {
    fn len(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Array(ts) => ts.len(),
        }
    }

    async fn inline_all_relative_paths<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> inlining::Result<()> {
        let mut errs = inlining::ErrorBuilder::new();

        match self {
            Self::One(merge_file_provider) => {
                errs.append(merge_file_provider.inline_all_relative_paths(ctx).await);
            }
            Self::Array(merge_file_providers) => {
                for mfp in merge_file_providers.iter_mut() {
                    errs.append(mfp.inline_all_relative_paths(ctx).await);
                }
            }
        }

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

impl FromCommand {
    pub(crate) fn new(inner: CommandSection) -> Self {
        Self { inner }
    }

    pub(crate) async fn inline(
        &mut self,
        mode: &InlineMode,
        ctx: &impl ResolutionContext,
    ) -> inlining::Result<()> {
        self.inner.inline(mode, ctx).await
    }
}

impl ResolveAndWrite for FromCommand {
    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        let target = target.as_ref();

        // We construct the sub-directory for namespacing file provider output from the
        // CommandSection using the file stem (file name minus extension).
        let file_stem = target
            .file_stem()
            .expect("to have a file stem")
            .to_string_lossy();

        // out_dir is already the providers directory (e.g., /output/providers/), so we pass it
        // directly as providers_dir without appending PROVIDER_DIR again.
        let out_dir = ctx.dir_containing(target);

        self.inner
            .run_providers_and_execute(
                &file_stem,
                &out_dir,
                target.to_path_buf(),
                out_dir.to_path_buf(),
                ctx,
            )
            .await?;

        Ok(())
    }
}

impl Check for FromCommand {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        self.inner.try_check(path, ctx)
    }
}

/// # Conditional
///
/// Conditionally run a file provider from an ordered list based on simple "where" clauses that
/// make use of the provided templating variables. The first case with a "where" clause that holds
/// will be run as the output of this provider.
///
/// ### Writing where clauses
///
/// The "where" clause on each case is a simple comparison against a single templating variable.
/// You must include the `var` key which accepts a string variable name that is required to be
/// defined within the test plan containing this provider. You may then assert that the variable
/// is equal (`eq`) or not equal (`ne`) to a given scalar value.
///
/// If none of the provider where clauses match, this provider will error during static analysis
/// checks.
///
/// ```yaml
/// - name: conditional_config.json
///   env_var: CONDITIONAL_CONFIG
///   kind: conditional
///   cases:
///     - where: { var: test_type, eq: load }
///       kind: relative_path
///       path: data/config-load.json
///
///     - where: { var: test_type, eq: ramp }
///       kind: relative_path
///       path: data/config-ramp.json
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Conditional {
    /// The ordered list of cases to be checked against the variables used for templating the test
    /// plan.
    cases: Vec<ConditionalCase>,
}

impl Template for Conditional {
    fn required_variables(&self) -> Vec<String> {
        let mut vars = HashSet::new();

        // The variables used in where clauses count as required variables so we need to include
        // those as well here.
        for case in self.cases.iter() {
            vars.extend(case.inner.required_variables());
            vars.insert(case.where_clause.var.clone());
        }

        let mut vars: Vec<String> = vars.into_iter().collect();
        vars.sort_unstable();

        vars
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        self.cases
            .validate_context(path, allowed_variables, file_source, ctx)
    }

    fn try_template(
        &mut self,
        _path: &mut Vec<String>,
        _file_source: &SourceDir,
        _ctx: &TemplateContext,
    ) -> templating::Result<()> {
        panic!(
            "Should not be able to get here. Conditional providers should have been collapsed when templating the config."
        )
    }
}

// AsUtf8FileContent needs to be implemented for Conditional to be a valid FileProvider that can be added to the NamedFileProvider enum
// However, the AsUtf8FileContent methods should never actually be called
impl AsUtf8FileContent for Conditional {
    async fn try_get_file_content(
        &self,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        panic!(
            "Should not be able to get here. Conditional providers should have been collapsed when templating the config."
        )
    }
}

// Check needs to be implemented for Conditional to be a valid FileProvider that can be added to the NamedFileProvider enum
// However, the Check methods should never actually be called
impl Check for Conditional {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        panic!(
            "Should not be able to get here. Conditional providers should have been collapsed when templating the config."
        )
    }
}

impl Conditional {
    /// Try to collapse this conditional provider into its matching case.
    ///
    /// If successful, the return of this method is the first case with a "where" clause that
    /// holds for the given `TemplateContext` and all cases before it will have been dropped. In
    /// the case that this method returns an error, _all_ cases will have been dropped.
    pub(crate) fn try_collapse(
        &mut self,
        path: &[String],
        ctx: &TemplateContext,
    ) -> templating::Result<FileProvider> {
        for case in self.cases.drain(..) {
            if case.where_clause.holds_for(ctx.variables()) {
                return Ok(case.inner);
            }
        }

        Err(templating::Errors::new(
            templating::ErrorKind::NoMatchingCases,
            "at least one case must hold for the given templating variables",
            path,
        ))
    }
}

/// A conditional "where" clause guarding the execution of an associated file provider
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct ConditionalCase {
    /// A conditional clause that must hold for this case to be run
    #[serde(rename = "where")]
    #[template(skip)]
    where_clause: WhereClause,

    #[serde(flatten)]
    inner: FileProvider,
}

/// A conditional clause based on the value of a templating variable
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct WhereClause {
    /// The variable being checked
    var: String,
    #[serde(flatten)]
    comp: VarComp,
}

impl WhereClause {
    fn holds_for(&self, vars: &HashMap<String, Scalar>) -> bool {
        let val = match vars.get(&self.var) {
            Some(val) => val,
            None => return false,
        };

        match &self.comp {
            VarComp::Eq(s) => val == s,
            VarComp::Ne(s) => val != s,
        }
    }
}

/// The comparison being run against the value of the given templating variable
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum VarComp {
    /// Require the variable to be equal to the given scalar value
    Eq(Scalar),
    /// Require the variable to be not equal to the given scalar value
    Ne(Scalar),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        checks::ErrorKind,
        context::Context,
        providers::{
            file::{
                FileProvider, NamedFileProvider,
                tests::{
                    assert_check_errors, assert_resolve_and_write_error,
                    assert_resolve_and_write_success,
                },
            },
            test_helpers::create_temp_dir_with_file,
        },
        templating::{Field, TemplateContext},
    };
    use assert_fs::{TempDir, fixture::PathChild};
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::collections::{HashMap, HashSet};

    fn one(content: &str) -> Overrides {
        Overrides::One(MergeFileProvider::Inline(InlineFile {
            content: content.to_string(),
        }))
    }

    fn arr(files: &[&str]) -> Overrides {
        Overrides::Array(
            files
                .iter()
                .map(|content| {
                    MergeFileProvider::Inline(InlineFile {
                        content: content.to_string(),
                    })
                })
                .collect(),
        )
    }

    fn merge_yaml(base: &str, overrides: Overrides) -> FileProvider {
        FileProvider::MergeYaml(MergeYaml {
            base: MergeFileProvider::Inline(InlineFile {
                content: base.to_string(),
            }),
            overrides,
        })
    }

    #[test_case(one("key1: X"), "key1: X\nkey2: B"; "override one")]
    #[test_case(arr(&["key1: X", "key2: Y"]), "key1: X\nkey2: Y"; "override two")]
    #[test_case(arr(&["key1: X", "key1: Y"]), "key1: Y\nkey2: B"; "override two replacing same key")]
    #[test_case(arr(&["key1: X\nkey2: Y", "key1: Z"]), "key1: Z\nkey2: Y"; "override two layered")]
    #[test_case(arr(&["key3: X", "key1: Y", "key2: Z"]), "key1: Y\nkey2: Z\nkey3: X"; "override three")]
    #[test_case(arr(&["key1: X", "key1: Y", "key1: Z"]), "key1: Z\nkey2: B"; "override three replacing same key")]
    #[test_case(arr(&["key3: X\nkey2: Y\nkey4: Z", "key1: W", "key2: V"]), "key1: W\nkey2: V\nkey3: X\nkey4: Z"; "override three layered")]
    #[tokio::test]
    async fn merge_yaml_resolve_expected_output(overrides: Overrides, expected: &str) {
        let provider = MergeYaml {
            base: MergeFileProvider::Inline(InlineFile {
                content: "key1: A\nkey2: B".into(),
            }),
            overrides,
        };

        let ctx = Context::new();

        let s = provider
            .try_get_file_content(&ctx)
            .await
            .expect("provider to run successfully");

        assert_eq!(s, expected);
    }

    #[test]
    fn merge_yaml_check_success() {
        let merge_yaml = MergeYaml {
            base: MergeFileProvider::Inline(InlineFile {
                content: "some content".to_string(),
            }),
            overrides: Overrides::One(MergeFileProvider::Inline(InlineFile {
                content: "override content".to_string(),
            })),
        };

        let ctx = Context::new();
        let res = merge_yaml.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test_case(
        MergeFileProvider::Required(RequiredFile {message: "will fail check".to_string(),}),
        Overrides::One(MergeFileProvider::Inline(InlineFile {content: "override content".to_string(),})),
        &[ErrorKind::RequiredFileMissing];
        "base only"
    )]
    #[test_case(
        MergeFileProvider::Inline(InlineFile {content: "some content".to_string(),}),
        Overrides::One(MergeFileProvider::Required(RequiredFile {message: "will fail check".to_string(),})),
        &[ErrorKind::RequiredFileMissing];
        "single override only"
    )]
    #[test_case(
        MergeFileProvider::Inline(InlineFile {content: "some content".to_string(),}),
        Overrides::Array(vec![MergeFileProvider::Required(RequiredFile {message: "will fail check".to_string(),}),MergeFileProvider::Required(RequiredFile {message: "will fail check".to_string(),})]),
        &[ErrorKind::RequiredFileMissing, ErrorKind::RequiredFileMissing];
        "multiple overrides only"
    )]
    #[test_case(
        MergeFileProvider::Required(RequiredFile {message: "will fail check".to_string(),}),
        Overrides::One(MergeFileProvider::Required(RequiredFile {message: "will fail check".to_string(),})),
        &[ErrorKind::RequiredFileMissing, ErrorKind::RequiredFileMissing];
        "base and single override"
    )]
    #[test]
    fn merge_yaml_check_errors(
        base: MergeFileProvider,
        overrides: Overrides,
        expected_err_kinds: &[checks::ErrorKind],
    ) {
        let merge_yaml = MergeYaml { base, overrides };

        let ctx = Context::new();

        assert_check_errors(merge_yaml, &ctx, expected_err_kinds);
    }

    #[test]
    fn from_command_check_success() {
        let from_command = FromCommand {
            inner: CommandSection {
                ..CommandSection::empty()
            },
        };

        let ctx = Context::new();

        let res = from_command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn from_command_check_errors() {
        let from_command = FromCommand {
            inner: CommandSection {
                file_providers: vec![NamedFileProvider {
                    name: "required".to_string(),
                    env_var: "REQUIRED".to_string(),
                    provider: FileProvider::Required(RequiredFile {
                        message: "this will error".to_string(),
                    }),
                }],
                ..CommandSection::empty()
            },
        };

        let ctx = Context::new();

        assert_check_errors(from_command, &ctx, &[ErrorKind::RequiredFileMissing]);
    }

    #[tokio::test]
    async fn merge_yaml_resolve_and_write_single_override_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("merged.yaml");

        let base_yaml = indoc!(
            r#"
            key1: 
              nested_key: base1
            key2: base2
            key3:
            - value1
            - value2
        "#
        );
        let override_yaml = indoc!(
            r#"
            key1: 
              nested_key: override1
            key2: override2
            key3:
            - overridevalue1
        "#
        );

        let expected_content = indoc!(
            r#"
            key1:
              nested_key: override1
            key2: override2
            key3:
            - value1
            - value2
            - overridevalue1"#
        );

        let mut ctx = Context::new();
        let merge_yaml = merge_yaml(base_yaml, one(override_yaml));

        assert_resolve_and_write_success(merge_yaml, &target, &mut ctx, expected_content).await
    }

    #[tokio::test]
    async fn merge_yaml_resolve_and_write_multiple_overrides_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("merged.yaml");

        let base_yaml = indoc!(
            r#"
            key1: 
              nested_key: base1
            key2: base2
            key3:
            - value1
            - value2
        "#
        );
        let override_one_yaml = indoc!(
            r#"
            key1: 
              nested_key: override1
            key3:
            - overridevalue1
        "#
        );
        let override_two_yaml = indoc!(
            r#"
            key2: override2
        "#
        );

        let expected_content = indoc!(
            r#"
            key1:
              nested_key: override1
            key2: override2
            key3:
            - value1
            - value2
            - overridevalue1"#
        );

        let mut ctx = Context::new();
        let merge_yaml = merge_yaml(base_yaml, arr(&[override_one_yaml, override_two_yaml]));

        assert_resolve_and_write_success(merge_yaml, &target, &mut ctx, expected_content).await
    }

    #[tokio::test]
    async fn merge_yaml_resolve_and_write_base_not_yaml_errors() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("merged.yaml");

        // Note the opening " after r#" that is NOT closed
        let base_yaml = indoc!(r#""an unclosed string"#);
        let override_yaml = indoc!(
            r#"
            key: value
        "#
        );

        let expected_err =
            "found unexpected end of stream at line 1 column 20, while scanning a quoted scalar";

        let mut ctx = Context::new();
        let merge_yaml = merge_yaml(base_yaml, one(override_yaml));

        assert_resolve_and_write_error(merge_yaml, &target, &mut ctx, expected_err).await
    }

    #[tokio::test]
    async fn merge_yaml_resolve_and_write_single_override_not_yaml_errors() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("merged.yaml");

        let base_yaml = indoc!(
            r#"
            key: value
        "#
        );
        // Note the opening " after r#" that is NOT closed
        let override_yaml = indoc!(r#""an unclosed string"#);

        let expected_err =
            "found unexpected end of stream at line 1 column 20, while scanning a quoted scalar";

        let mut ctx = Context::new();
        let merge_yaml = merge_yaml(base_yaml, one(override_yaml));

        assert_resolve_and_write_error(merge_yaml, &target, &mut ctx, expected_err).await
    }

    #[tokio::test]
    async fn merge_yaml_resolve_and_write_one_of_multiple_overrides_not_yaml_errors() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("merged.yaml");

        let base_yaml = indoc!(
            r#"
            key: value
        "#
        );
        let override_one_yaml = indoc!(
            r#"
            key:value
        "#
        );
        // Note the opening " after r#" that is NOT closed
        let override_two_yaml = indoc!(r#""an unclosed string"#);

        let expected_err =
            "found unexpected end of stream at line 1 column 20, while scanning a quoted scalar";

        let mut ctx = Context::new();
        let merge_yaml = merge_yaml(base_yaml, arr(&[override_one_yaml, override_two_yaml]));

        assert_resolve_and_write_error(merge_yaml, &target, &mut ctx, expected_err).await
    }

    #[test_case(Field::Pending("foo".to_string()), &["bar", "foo"]; "required field and where clause")]
    #[test_case(Field::Resolved("foo".to_string()), &["bar"]; "where clause only")]
    #[test]
    fn conditional_required_variables(f: Field<String>, expected: &[&str]) {
        let fp = Conditional {
            cases: vec![ConditionalCase {
                where_clause: WhereClause {
                    var: "bar".to_string(),
                    comp: VarComp::Eq(42.into()),
                },
                inner: FileProvider::RelativePath(RelativeFile { path: f, src: None }),
            }],
        };

        let res = fp.required_variables();
        assert_eq!(res, expected)
    }

    #[test]
    #[should_panic(
        expected = "Should not be able to get here. Conditional providers should have been collapsed when templating the config."
    )]
    fn conditional_try_template_panics() {
        let mut fp = Conditional { cases: vec![] };
        let _ = fp.try_template(
            &mut Vec::new(),
            &SourceDir::Local {
                abs_path: Default::default(),
            },
            &TemplateContext::new_stubbed(HashMap::new()),
        );
    }

    #[test]
    #[should_panic(
        expected = "Should not be able to get here. Conditional providers should have been collapsed when templating the config."
    )]
    fn conditional_try_check_panics() {
        let fp = Conditional { cases: vec![] };
        let _ = fp.try_check(&mut Vec::new(), &Context::new());
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Conditional providers should have been collapsed when templating the config."
    )]
    async fn conditional_try_get_file_content_panics() {
        let fp = Conditional { cases: vec![] };
        let _ = fp.try_get_file_content(&Context::new()).await;
    }

    #[test_case(42, "case 1"; "first case")]
    #[test_case(7, "case 2"; "second case")]
    #[test]
    fn conditional_try_collapse_succeeds(bar_val: usize, expected_content: &str) {
        let mut fp = Conditional {
            cases: vec![
                ConditionalCase {
                    where_clause: WhereClause {
                        var: "bar".to_string(),
                        comp: VarComp::Eq(42.into()),
                    },
                    inner: FileProvider::Inline(InlineFile {
                        content: "case 1".to_string(),
                    }),
                },
                ConditionalCase {
                    where_clause: WhereClause {
                        var: "bar".to_string(),
                        comp: VarComp::Eq(7.into()),
                    },
                    inner: FileProvider::Inline(InlineFile {
                        content: "case 2".to_string(),
                    }),
                },
            ],
        };

        let res = fp.try_collapse(
            &[],
            &TemplateContext::new_stubbed(HashMap::from([("bar".to_string(), bar_val.into())])),
        );

        assert!(res.is_ok(), "expected Ok, got {res:?}");
        assert_eq!(
            res.unwrap(),
            FileProvider::Inline(InlineFile {
                content: expected_content.to_string()
            })
        );
    }

    #[test]
    fn conditional_try_collapse_errors_with_no_matching_cases() {
        let mut fp = Conditional { cases: vec![] };

        let res = fp.try_collapse(&[], &TemplateContext::new_stubbed(HashMap::new()));

        assert!(res.is_err(), "expected Err, got {res:?}");
        assert_eq!(
            res.unwrap_err().unwrap_single().kind,
            templating::ErrorKind::NoMatchingCases
        );
    }

    #[tokio::test]
    async fn text_file_provider_inline_all_relative_paths_succeeds() {
        let ctx = Context::new();
        let file_content = "example file content";
        let relative_file_path = "file.txt";
        let (temp, _file_to_read) = create_temp_dir_with_file(relative_file_path, file_content);
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());

        let mut text_file_provider = MergeFileProvider::RelativePath(RelativeFile {
            path: Field::Resolved(relative_file_path.to_string()),
            src: Some(src.clone()),
        });

        let result = text_file_provider.inline_all_relative_paths(&ctx).await;
        let expected_text_file_provider = MergeFileProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });
        assert!(
            result.is_ok(),
            "Expected inline_all_relative_paths to succeed, got {result:?}"
        );
        assert_eq!(text_file_provider, expected_text_file_provider);
    }

    #[tokio::test]
    async fn text_file_provider_inline_all_relative_paths_succeeds_and_leaves_inline_text_file_provider_unchanged()
     {
        let ctx = Context::new();
        let file_content = "example file content";

        let mut text_file_provider = MergeFileProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });
        let expected_text_file_provider = text_file_provider.clone();

        let result = text_file_provider.inline_all_relative_paths(&ctx).await;
        assert!(
            result.is_ok(),
            "Expected inline_all_relative_paths to succeed, got {result:?}"
        );
        assert_eq!(text_file_provider, expected_text_file_provider);
    }

    #[test_case(1; "one override")]
    #[test_case(2; "multiple overrides")]
    #[tokio::test]
    async fn merge_yaml_inline_all_relative_paths_succeeds(num_overrides: u8) {
        let ctx = Context::new();
        let file_content = "example file content";
        let relative_file_path = "file.txt";
        let (temp, _file_to_read) = create_temp_dir_with_file(relative_file_path, file_content);
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());

        let overrides = match num_overrides {
            1 => Overrides::One(MergeFileProvider::RelativePath(RelativeFile {
                path: Field::Resolved(relative_file_path.to_string()),
                src: Some(src.clone()),
            })),
            _ => Overrides::Array(vec![
                MergeFileProvider::RelativePath(RelativeFile {
                    path: Field::Resolved(relative_file_path.to_string()),
                    src: Some(src.clone()),
                }),
                MergeFileProvider::RelativePath(RelativeFile {
                    path: Field::Resolved(relative_file_path.to_string()),
                    src: Some(src.clone()),
                }),
            ]),
        };

        let mut merge_yaml = MergeYaml {
            base: MergeFileProvider::RelativePath(RelativeFile {
                path: Field::Resolved(relative_file_path.to_string()),
                src: Some(src),
            }),
            overrides,
        };

        let result = merge_yaml.inline_all_relative_paths(&ctx).await;

        let expected_overrides = match num_overrides {
            1 => Overrides::One(MergeFileProvider::Inline(InlineFile {
                content: file_content.to_string(),
            })),
            _ => Overrides::Array(vec![
                MergeFileProvider::Inline(InlineFile {
                    content: file_content.to_string(),
                }),
                MergeFileProvider::Inline(InlineFile {
                    content: file_content.to_string(),
                }),
            ]),
        };
        let expected_merge_yaml = MergeYaml {
            base: MergeFileProvider::Inline(InlineFile {
                content: file_content.to_string(),
            }),
            overrides: expected_overrides,
        };

        assert!(
            result.is_ok(),
            "Expected inline_all_relative_paths to succeed, got {result:?}"
        );
        assert_eq!(merge_yaml, expected_merge_yaml);
    }

    #[tokio::test]
    async fn merge_yaml_inline_succeeds() {
        let ctx = Context::new();
        let base_content = "key1: base_value";
        let override_content = "key2: override_value";

        let mut file_provider = FileProvider::MergeYaml(MergeYaml {
            base: MergeFileProvider::Inline(InlineFile {
                content: base_content.to_string(),
            }),
            overrides: Overrides::One(MergeFileProvider::Inline(InlineFile {
                content: override_content.to_string(),
            })),
        });

        let result = file_provider.inline(&InlineMode::All, &ctx).await;

        assert!(result.is_ok(), "Expected inline to succeed, got {result:?}");
        assert_eq!(
            file_provider,
            FileProvider::Inline(InlineFile {
                content: "key1: base_value\nkey2: override_value".to_string(),
            }),
            "Expected merge_yaml to be inlined with merged content"
        );
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Conditional providers should have been collapsed when templating the config."
    )]
    async fn conditional_inline_panics() {
        let ctx = Context::new();
        let mut file_provider = FileProvider::Conditional(Conditional {
            cases: vec![ConditionalCase {
                where_clause: WhereClause {
                    var: "test_var".to_string(),
                    comp: VarComp::Eq(42.into()),
                },
                inner: FileProvider::Inline(InlineFile {
                    content: "content".to_string(),
                }),
            }],
        });

        let _res = file_provider.inline(&InlineMode::All, &ctx).await;
    }

    fn templated(content: &str) -> TemplatedFile {
        TemplatedFile {
            content: content.to_string(),
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, Scalar> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect()
    }

    #[tokio::test]
    async fn templated_file_resolves_variables() {
        let mut tf = templated("hello ${name}!");
        let ctx = TemplateContext::new_stubbed(vars(&[("name", "world")]));

        tf.try_template(&mut Vec::new(), &SourceDir::local("/"), &ctx)
            .expect("try_template to succeed");

        let result = tf
            .try_get_file_content(&Context::new())
            .await
            .expect("try_get_file_content to succeed");

        assert_eq!(result, "hello world!");
    }

    #[test]
    fn templated_file_try_template_errors_on_unknown_variable() {
        let mut tf = templated("value: ${unknown}");
        let ctx = TemplateContext::new_stubbed(HashMap::new());

        let res = tf.try_template(&mut Vec::new(), &SourceDir::local("/"), &ctx);

        assert!(res.is_err(), "expected error, got {res:?}");
        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.kind, templating::ErrorKind::UnknownVariable);
        assert_eq!(err.message, "unknown");
    }

    #[test]
    fn templated_file_check_always_succeeds() {
        let tf = templated("${unresolved_is_fine_at_check_time}");
        let ctx = Context::new();
        let res = tf.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn templated_file_required_variables() {
        let tf = templated("${foo} and ${bar}");
        let mut vars = tf.required_variables();
        vars.sort_unstable();
        assert_eq!(vars, vec!["bar", "foo"]);
    }

    #[test]
    fn templated_file_validate_context_errors_on_unknown() {
        let tf = templated("${known} and ${unknown}");
        let known = "known".to_string();
        let allowed: HashSet<&String> = HashSet::from([&known]);
        let ctx = TemplateContext::new_stubbed(HashMap::new());

        let res = tf.validate_context(&mut Vec::new(), &allowed, &SourceDir::local("/"), &ctx);

        assert!(res.is_err(), "expected error, got {res:?}");
        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.kind, templating::ErrorKind::UnknownVariable);
        assert_eq!(err.message, "unknown");
    }
}
