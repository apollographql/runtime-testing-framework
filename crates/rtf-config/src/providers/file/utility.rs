//! File providers that act as combinators or otherwise modify the output of other providers
use crate::{
    SourceDir,
    checks::{self, Check},
    context::ResolutionContext,
    enum_impl_as_utf8_file_content, enum_impl_check, merge_yaml,
    providers::{
        self, Result,
        command::CommandSection,
        file::{
            AsUtf8FileContent, FileProvider, InlineFile, RelativeFile, RequiredFile,
            ResolveAndWrite, apollo::GraphosSubgraphRouterUrlOverrides, github::GithubFile,
        },
    },
    templating::{self, Scalar, Template, TemplateContext},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use tracing::error;

/// # Text file provider
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
    pub(crate) base: TextFileProvider,
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(untagged)]
pub enum Overrides {
    One(TextFileProvider),
    Array(Vec<TextFileProvider>),
}

impl Overrides {
    fn len(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Array(ts) => ts.len(),
        }
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

/// Conditionally run one of several providers.
///
/// Conditionally run a file provider from an ordered list based on simple "where" clauses that
/// make use of the provided templating variables. The first case with a "where" clause that holds
/// will be run as the output of this provider.
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

    #[serde(skip, default)]
    variables: HashMap<String, Scalar>,
}

impl Template for Conditional {
    fn has_pending_fields(&self) -> bool {
        self.cases.has_pending_fields()
    }

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
        path: &mut Vec<String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        self.variables = ctx.variables().clone();
        self.cases.try_template(path, file_source, ctx)
    }
}

impl ResolveAndWrite for Conditional {
    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        let target = target.as_ref();

        for case in self.cases.iter() {
            if case.where_clause.holds_for(&self.variables) {
                // we need to pin this future on the heap to be able to poll it in order to avoid a
                // recursively defined future (which is infinitely sized)
                return Box::pin(case.inner.resolve_and_write(target, ctx)).await;
            }
        }

        panic!("should have errored in try_check due to having no matching cases");
    }
}

impl Check for Conditional {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        if self.cases.is_empty() {
            return Err(checks::Errors::new(
                checks::ErrorKind::EmptyArray,
                "conditional file providers require at least one case",
                path,
            ));
        }

        let mut errs = checks::ErrorBuilder::new();

        for case in self.cases.iter() {
            errs.append(case.inner.try_check_nested(path, "inner", ctx));
        }

        if self
            .cases
            .iter()
            .all(|case| !case.where_clause.holds_for(&self.variables))
        {
            errs.push(
                checks::ErrorKind::NoMatchingCases,
                "at least one case must hold for the given templating variables",
                path,
            );
        }

        errs.into_result(())
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
        providers::file::{
            FileProvider, NamedFileProvider,
            tests::{
                assert_check_errors, assert_resolve_and_write_error,
                assert_resolve_and_write_success,
            },
        },
        templating::Field,
    };
    use assert_fs::{TempDir, fixture::PathChild};
    use indoc::indoc;
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

    fn one(content: &str) -> Overrides {
        Overrides::One(TextFileProvider::Inline(InlineFile {
            content: content.to_string(),
        }))
    }

    fn arr(files: &[&str]) -> Overrides {
        Overrides::Array(
            files
                .iter()
                .map(|content| {
                    TextFileProvider::Inline(InlineFile {
                        content: content.to_string(),
                    })
                })
                .collect(),
        )
    }

    fn merge_yaml(base: &str, overrides: Overrides) -> FileProvider {
        FileProvider::MergeYaml(MergeYaml {
            base: TextFileProvider::Inline(InlineFile {
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
            base: TextFileProvider::Inline(InlineFile {
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
            base: TextFileProvider::Inline(InlineFile {
                content: "some content".to_string(),
            }),
            overrides: Overrides::One(TextFileProvider::Inline(InlineFile {
                content: "override content".to_string(),
            })),
        };

        let ctx = Context::new();
        let res = merge_yaml.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test_case(
        TextFileProvider::Required(RequiredFile {message: "will fail check".to_string(),}),
        Overrides::One(TextFileProvider::Inline(InlineFile {content: "override content".to_string(),})),
        &[ErrorKind::RequiredFileMissing];
        "base only"
    )]
    #[test_case(
        TextFileProvider::Inline(InlineFile {content: "some content".to_string(),}),
        Overrides::One(TextFileProvider::Required(RequiredFile {message: "will fail check".to_string(),})),
        &[ErrorKind::RequiredFileMissing];
        "single override only"
    )]
    #[test_case(
        TextFileProvider::Inline(InlineFile {content: "some content".to_string(),}),
        Overrides::Array(vec![TextFileProvider::Required(RequiredFile {message: "will fail check".to_string(),}),TextFileProvider::Required(RequiredFile {message: "will fail check".to_string(),})]),
        &[ErrorKind::RequiredFileMissing, ErrorKind::RequiredFileMissing];
        "multiple overrides only"
    )]
    #[test_case(
        TextFileProvider::Required(RequiredFile {message: "will fail check".to_string(),}),
        Overrides::One(TextFileProvider::Required(RequiredFile {message: "will fail check".to_string(),})),
        &[ErrorKind::RequiredFileMissing, ErrorKind::RequiredFileMissing];
        "base and single override"
    )]
    #[test]
    fn merge_yaml_check_errors(
        base: TextFileProvider,
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

    #[test_case(Field::Pending("foo".to_string()), true; "field is pending")]
    #[test_case(Field::Resolved("foo".to_string()), false; "field is resolved")]
    #[test]
    fn conditional_has_pending_fields(f: Field<String>, expected: bool) {
        let fp = Conditional {
            cases: vec![ConditionalCase {
                where_clause: WhereClause {
                    var: "bar".to_string(),
                    comp: VarComp::Eq(42.into()),
                },
                inner: FileProvider::RelativePath(RelativeFile { path: f, src: None }),
            }],
            variables: HashMap::default(),
        };

        let res = fp.has_pending_fields();
        assert_eq!(res, expected)
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
            variables: HashMap::default(),
        };

        let res = fp.required_variables();
        assert_eq!(res, expected)
    }

    #[test]
    fn conditional_try_template_succeeds() {
        let mut fp = Conditional {
            cases: vec![ConditionalCase {
                where_clause: WhereClause {
                    var: "bar".to_string(),
                    comp: VarComp::Eq(42.into()),
                },
                inner: FileProvider::RelativePath(RelativeFile {
                    path: Field::Pending("path".to_string()),
                    src: None,
                }),
            }],
            variables: HashMap::default(),
        };

        let ctx = template_context!(&["path"]);

        let res = fp.try_template(&mut Vec::new(), &SourceDir::local("/"), &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test]
    fn conditional_try_template_unknown_variable_error() {
        let mut fp = Conditional {
            cases: vec![ConditionalCase {
                where_clause: WhereClause {
                    var: "bar".to_string(),
                    comp: VarComp::Eq(42.into()),
                },
                inner: FileProvider::RelativePath(RelativeFile {
                    path: Field::Pending("path".to_string()),
                    src: None,
                }),
            }],
            variables: HashMap::default(),
        };
        let ctx = template_context!(&["unused"]);

        let res = fp.try_template(&mut vec!["test".to_string()], &SourceDir::local("/"), &ctx);
        assert!(res.is_err(), "expected templating to error, got {res:?}");

        let errors = res.unwrap_err();
        let error = errors.unwrap_single();
        let error_kind = error.kind;
        let error_path = error.path;
        assert_eq!(
            error_kind,
            templating::ErrorKind::UnknownVariable,
            "expected ErrorKind to match"
        );
        assert_eq!(error_path, "test.inner.path", "expected path to match")
    }

    #[test]
    fn conditional_check_error_path_correct() {
        let fp = Conditional {
            cases: vec![ConditionalCase {
                where_clause: WhereClause {
                    var: "bar".to_string(),
                    comp: VarComp::Eq(42.into()),
                },
                inner: FileProvider::RelativePath(RelativeFile {
                    path: Field::Resolved("does/not/exist/relative.txt".to_string()),
                    src: Some(SourceDir::local("/foo")),
                }),
            }],
            variables: HashMap::from([("bar".to_string(), 42.into())]),
        };

        let res = fp.try_check(&mut vec!["path".to_string()], &Context::new());
        assert!(res.is_err(), "expected to check to error, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.path, "path.inner", "{err:?}")
    }

    #[test]
    fn conditional_check_error_no_matching_cases() {
        let fp = Conditional {
            cases: vec![ConditionalCase {
                where_clause: WhereClause {
                    var: "bar".to_string(),
                    comp: VarComp::Eq(42.into()),
                },
                inner: FileProvider::Inline(InlineFile {
                    content: String::new(),
                }),
            }],
            variables: HashMap::from([("bar".to_string(), 7.into())]),
        };

        let res = fp.try_check(&mut vec!["path".to_string()], &Context::new());
        assert!(res.is_err(), "expected to check to error, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.kind, checks::ErrorKind::NoMatchingCases)
    }

    #[test]
    fn conditional_check_error_empty_cases() {
        let fp = Conditional {
            cases: Vec::default(),
            variables: HashMap::default(),
        };

        let res = fp.try_check(&mut vec!["path".to_string()], &Context::new());
        assert!(res.is_err(), "expected to check to error, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.kind, checks::ErrorKind::EmptyArray)
    }

    #[test_case(42, "case 1"; "first case")]
    #[test_case(7, "case 2"; "second case")]
    #[tokio::test]
    async fn conditional_resolve_and_write_success(bar_val: usize, expected_content: &str) {
        let temp = TempDir::new().unwrap();
        let target = temp.child("conditional.txt");

        let fp = FileProvider::Conditional(Conditional {
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
            variables: HashMap::from([("bar".to_string(), bar_val.into())]),
        });

        assert_resolve_and_write_success(fp, &target, &mut Context::new(), expected_content).await;
    }
}
