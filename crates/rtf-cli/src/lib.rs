//! Runtime Testing Framework CLI - a swiss army knife for testing the Apollo Runtime
use anyhow::{Context, anyhow};
use rtf_config::{
    SourceDir, context::ResolutionContext, formats::TestPlanConfig, templating::Scalar,
};
use serde::Deserialize;
use std::collections::HashMap;

pub mod cli;
pub mod commands;

/// The environment variable to set to control logging within the rtf CLI
pub const LOG_LEVEL_ENV_VAR: &str = "APOLLO_RTF_LOG";

// The `cli.rs` file is pulled in as an inline module so we can generate markdown help for the CLI
// interface in `build.rs`. Any dependencies we make use of in that file need to be included in the
// build dependencies (rather than main crate dependencies), so we implement this method here
// instead.

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ScalarOrArray {
    Scalar(Scalar),
    Array(Vec<Scalar>),
}

impl cli::Variables {
    /// Parse variables coming from CLI flags into a form that we can merge with the test plan
    pub(crate) fn parse(
        self,
        cwd_source: &SourceDir,
        ctx: &impl ResolutionContext,
    ) -> anyhow::Result<ParsedVariables> {
        let variable_json_data = match self.vars.as_ref() {
            Some(path) => {
                let s = ctx.read_path_to_string(path)?;
                let variables_json: HashMap<String, ScalarOrArray> =
                    serde_json::from_str(&s).context("invalid variables file")?;

                let source_dir = ctx
                    .canonicalize_path(path)?
                    .parent()
                    .expect("we just read the file so we know it has a parent")
                    .to_owned();

                Some((SourceDir::local(source_dir), variables_json))
            }

            None => None,
        };

        self.parse_inner(variable_json_data, cwd_source)
    }

    fn parse_inner(
        self,
        variable_json_data: Option<(SourceDir, HashMap<String, ScalarOrArray>)>,
        cwd_source: &SourceDir,
    ) -> anyhow::Result<ParsedVariables> {
        let mut variables = HashMap::new();
        let mut matrix_dimensions = HashMap::new();
        let mut variable_sources = HashMap::new();

        // --vars variables read from a file can be individual variables or matrix dimensions
        if let Some((source, from_variables)) = variable_json_data {
            for (k, v) in from_variables.into_iter() {
                match v {
                    ScalarOrArray::Scalar(s) => {
                        variables.insert(k.clone(), s);
                    }
                    ScalarOrArray::Array(arr) => {
                        matrix_dimensions.insert(k.clone(), arr);
                    }
                }

                variable_sources.insert(k, source.clone());
            }
        }

        // -v variables are always scalar
        for kv in self.var.into_iter() {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| anyhow!("expected \"key=value\", got {kv:?}"))?;

            if k.is_empty() {
                return Err(anyhow!("no key provided for variable {v:?}"));
            }

            let v: Scalar = serde_yaml::from_str(v).context(format!("invalid value for {k:?}"))?;
            variable_sources.insert(k.to_string(), cwd_source.clone());
            variables.insert(k.to_string(), v);
        }

        Ok(ParsedVariables {
            variables,
            matrix_dimensions,
            variable_sources,
        })
    }

    /// Merge any variables obtained from the CLI with the ones found in a test plan, removing any
    /// existing variable or matrix definitions with the same key.
    pub fn merge(
        self,
        test_plan: &mut TestPlanConfig,
        cwd_source: &SourceDir,
        ctx: &mut impl ResolutionContext,
    ) -> anyhow::Result<HashMap<String, SourceDir>> {
        self.parse(cwd_source, ctx)?
            .merge_inner(&mut test_plan.variables, &mut test_plan.matrix.dimensions)
    }
}

/// Result of parsing CLI variables, containing scalar variables, array variables, and their sources.
#[derive(Debug)]
pub(crate) struct ParsedVariables {
    variables: HashMap<String, Scalar>,
    matrix_dimensions: HashMap<String, Vec<Scalar>>,
    variable_sources: HashMap<String, SourceDir>,
}

impl ParsedVariables {
    #[inline]
    fn merge_inner(
        self,
        variables_from_test_plan: &mut HashMap<String, Scalar>,
        matrix_from_test_plan: &mut HashMap<String, Vec<Scalar>>,
    ) -> anyhow::Result<HashMap<String, SourceDir>> {
        for (k, dim) in self.matrix_dimensions.into_iter() {
            variables_from_test_plan.remove(&k);
            matrix_from_test_plan.insert(k.clone(), dim);
        }

        for (k, v) in self.variables.into_iter() {
            matrix_from_test_plan.remove(&k);
            variables_from_test_plan.insert(k.clone(), v);
        }

        Ok(self.variable_sources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtf_config::templating::Scalar;
    use serde_json::json;
    use simple_test_case::test_case;
    use std::path::PathBuf;

    macro_rules! variables_map {
        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();
            $( m.insert($k.to_string(), Scalar::try_from($v).unwrap()); )+
            m
        }};
    }

    macro_rules! variables_json {
        ($($tok:tt)*) => {
            serde_json::from_value(json!($($tok)*)).unwrap()
        };
    }

    #[test]
    fn variables_are_merged_in_the_correct_order() {
        let mut variables = variables_map!("foo" => 42, "bar" => "live", "baz" => true);
        let mut matrix = HashMap::default();
        let from_cli = cli::Variables {
            var: vec![
                "bar=love".to_string(),
                "qux=123".to_string(),
                "qux=456".to_string(),
            ],
            vars: Some(PathBuf::from("my-variables.json")),
        };

        let parsed = from_cli
            .parse_inner(
                Some((
                    SourceDir::local("/json_variables"),
                    variables_json!({
                        "bar": "laugh", "baz": false
                    }),
                )),
                &SourceDir::local("/cli"),
            )
            .unwrap();
        parsed.merge_inner(&mut variables, &mut matrix).unwrap();

        assert!(matrix.is_empty());

        // foo is not overwritten and should remain what was in the test plan
        assert_eq!(variables.get("foo"), Some(&Scalar::from(42)));

        // bar is overwritten from both the variables json file and a cli arg: cli should be preferred
        assert_eq!(variables.get("bar"), Some(&Scalar::from("love")));

        // baz is overwritten in the variables json file
        assert_eq!(variables.get("baz"), Some(&Scalar::from(false)));

        // qux is an additional variable added as a cli arg twice: we should get the last variable set
        assert_eq!(variables.get("qux"), Some(&Scalar::from(456)));
    }

    #[test]
    fn overrides_remove_conflicting_existing_keys() {
        // Start with foo as a variable and bar and baz a matrix dimensions
        let mut variables = variables_map!("foo" => 42);
        let mut matrix: HashMap<String, Vec<Scalar>> = HashMap::default();
        matrix.insert("bar".into(), vec![1.into()]);
        matrix.insert("baz".into(), vec![2.into()]);

        // change bar to a variable from the cli
        // change baz to a variable from variables.json
        // change foo to a matrix dimension from variables.json
        let from_cli = cli::Variables {
            var: vec!["bar=3".to_string()],
            vars: Some(PathBuf::from("my-variables.json")),
        };

        // Keys should start mutually exclusive
        let mut initial_variables: Vec<&String> = variables.keys().collect();
        let mut initial_matrix_dimensions: Vec<&String> = matrix.keys().collect();
        initial_variables.sort_unstable();
        initial_matrix_dimensions.sort_unstable();

        assert_eq!(&initial_variables, &["foo"]);
        assert_eq!(&initial_matrix_dimensions, &["bar", "baz"]);

        let parsed = from_cli
            .parse_inner(
                Some((
                    SourceDir::local("/json_variables"),
                    variables_json!({
                        "foo": [2], "baz": 42
                    }),
                )),
                &SourceDir::local("/cli"),
            )
            .unwrap();
        parsed.merge_inner(&mut variables, &mut matrix).unwrap();

        // Keys should end mutually exclusive but flipped
        let mut final_variables: Vec<&String> = variables.keys().collect();
        let mut final_matrix_dimensions: Vec<&String> = matrix.keys().collect();
        final_variables.sort_unstable();
        final_matrix_dimensions.sort_unstable();

        assert_eq!(&final_variables, &["bar", "baz"]);
        assert_eq!(&final_matrix_dimensions, &["foo"]);
    }

    #[test_case("bar should have an equals before variable"; "no equals")]
    #[test_case("bar=[1, 2, 3]"; "invalid scalar")]
    #[test_case("bar="; "no variable")]
    #[test_case("=variable"; "no key")]
    #[test_case(""; "empty string")]
    #[test]
    fn an_invalid_variable_returns_an_error(val: &str) {
        let from_cli = cli::Variables {
            var: vec![val.to_string()],
            vars: None,
        };

        let res = from_cli.parse_inner(None, &SourceDir::local("/cli"));

        assert!(res.is_err(), "expected error, ended up with {res:?}");
    }
}
