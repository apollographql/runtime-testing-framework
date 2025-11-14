//! Runtime Testing Framework CLI - a swiss army knife for testing the Apollo Runtime
use anyhow::{Context, anyhow};
use rtf_config::{Source, context::ResolutionContext, formats::TestPlanConfig, templating::Scalar};
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
    /// Merge any variables obtained from the CLI with the ones found in a test plan, removing any
    /// existing variable or matrix definitions with the same key.
    pub fn merge(
        self,
        test_plan: &mut TestPlanConfig,
        cwd_source: &Source,
        ctx: &mut impl ResolutionContext,
    ) -> anyhow::Result<HashMap<String, Source>> {
        let variable_json_data = match self.vars.as_ref() {
            Some(path) => {
                let s = ctx.read_path_to_string(path)?;
                let variables_json: HashMap<String, ScalarOrArray> =
                    serde_json::from_str(&s).context("invalid variables file")?;

                Some((Source::local(ctx.canonicalize_path(path)?), variables_json))
            }

            None => None,
        };

        let override_sources = self.merge_inner(
            &mut test_plan.variables,
            &mut test_plan.matrix.dimensions,
            variable_json_data,
            cwd_source,
        )?;
        ctx.set_variables(&test_plan.variables);

        Ok(override_sources)
    }

    #[inline]
    fn merge_inner(
        self,
        variables_from_test_plan: &mut HashMap<String, Scalar>,
        matrix_from_test_plan: &mut HashMap<String, Vec<Scalar>>,
        variables_json_data: Option<(Source, HashMap<String, ScalarOrArray>)>,
        cwd_source: &Source,
    ) -> anyhow::Result<HashMap<String, Source>> {
        let mut override_sources = HashMap::new();

        // Variables read from a file can be used to specify either individual variables or matrix arrays
        if let Some((source, from_variables)) = variables_json_data {
            for (k, v) in from_variables.into_iter() {
                match v {
                    ScalarOrArray::Scalar(s) => {
                        matrix_from_test_plan.remove(&k);
                        variables_from_test_plan.insert(k.clone(), s);
                        override_sources.insert(k, source.clone());
                    }
                    ScalarOrArray::Array(arr) => {
                        variables_from_test_plan.remove(&k);
                        matrix_from_test_plan.insert(k.clone(), arr);
                        override_sources.insert(k, source.clone());
                    }
                }
            }
        }

        // Variables provided on the command line always specify a single variable
        for kv in self.var.into_iter() {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| anyhow!("expected \"key=value\", got {kv:?}"))?;

            if k.is_empty() {
                return Err(anyhow!("no key provided for variable {v:?}"));
            }

            let v: Scalar = serde_yaml::from_str(v).context(format!("invalid value for {k:?}"))?;
            matrix_from_test_plan.remove(k);
            variables_from_test_plan.insert(k.to_string(), v);
            override_sources.insert(k.to_string(), cwd_source.clone());
        }

        Ok(override_sources)
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

        from_cli
            .merge_inner(
                &mut variables,
                &mut matrix,
                Some((
                    Source::local("/json_variables"),
                    variables_json!({
                        "bar": "laugh", "baz": false
                    }),
                )),
                &Source::local("/cli"),
            )
            .unwrap();

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

        from_cli
            .merge_inner(
                &mut variables,
                &mut matrix,
                Some((
                    Source::local("/json_variables"),
                    variables_json!({
                        "foo": [2], "baz": 42
                    }),
                )),
                &Source::local("/cli"),
            )
            .unwrap();

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

        let mut vals = HashMap::default();
        let mut mat = HashMap::default();
        let res = from_cli.merge_inner(&mut vals, &mut mat, None, &Source::local("/cli"));

        assert!(res.is_err(), "expected error, ended up with {vals:?}");
    }
}
