use rtf_config::{
    Execution, SourceDir, StableSource, context::ResolutionContext, formats::TestPlan,
    templating::Scalar,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, path::PathBuf};

/// An error that can be encountered when parsing runtime overrides to RTF templating variables.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read variables file")]
    ReadFile(#[from] io::Error),

    #[error("invalid variables file")]
    InvalidVariablesFile(#[from] serde_json::Error),

    #[error("expected \"key=value\", got {0:?}")]
    InvalidVariableFormat(String),

    #[error("no key provided for variable {0:?}")]
    MissingKey(String),

    #[error("invalid value for {key:?}")]
    InvalidScalar {
        key: String,
        #[source]
        error: serde_yaml::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ScalarOrArray {
    Scalar(Scalar),
    Array(Vec<Scalar>),
}

#[derive(Debug, Default)]
pub struct Variables {
    /// A single additional templating variable in the form "key=value"
    pub var: Vec<String>,
    /// Path to a JSON file containing additional template variables
    pub vars: Option<PathBuf>,
}

impl Variables {
    /// Parse variables coming from CLI flags into a form that we can merge with the test plan.
    ///
    /// Returns the parsed variables alongside the [SourceDir] of the `--vars` file, if one was
    /// provided. Callers are responsible for incorporating this into `Sources` via
    /// `Sources::with_variables_file` before calling [ResolutionContext::set_sources].
    pub fn parse(
        self,
        ctx: &impl ResolutionContext,
    ) -> Result<(ParsedVariables, Option<SourceDir>)> {
        let variable_json_data = match self.vars.as_ref() {
            Some(path) => {
                let s = ctx.read_path_to_string(path)?;
                let variables_json: HashMap<String, ScalarOrArray> = serde_json::from_str(&s)?;
                let source_dir = ctx
                    .canonicalize_path(path)?
                    .parent()
                    .expect("we just read the file so we know it has a parent")
                    .to_owned();

                Some((StableSource::VariablesFile, variables_json, source_dir))
            }

            None => None,
        };

        let vars_file_src = variable_json_data
            .as_ref()
            .map(|(_, _, src)| SourceDir::local(src));
        let variable_json_data = variable_json_data.map(|(stable_src, json, _)| (stable_src, json));

        Ok((self.parse_inner(variable_json_data)?, vars_file_src))
    }

    fn parse_inner(
        self,
        variable_json_data: Option<(StableSource, HashMap<String, ScalarOrArray>)>,
    ) -> Result<ParsedVariables> {
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
                .ok_or_else(|| Error::InvalidVariableFormat(kv.clone()))?;

            if k.is_empty() {
                return Err(Error::MissingKey(v.to_string()));
            }

            let v: Scalar = serde_yaml::from_str(v).map_err(|error| Error::InvalidScalar {
                key: k.to_string(),
                error,
            })?;
            variable_sources.insert(k.to_string(), StableSource::Cli);
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
    ///
    /// Returns the variable sources map alongside the [SourceDir] of the `--vars` file, if one
    /// was provided. Callers are responsible for incorporating this into [rtf_config::formats::Sources] via
    /// [ResolutionContext::set_sources].
    pub fn merge<E: Execution>(
        self,
        test_plan: &mut TestPlan<E>,
        ctx: &impl ResolutionContext,
    ) -> Result<(HashMap<String, StableSource>, Option<SourceDir>)> {
        let (parsed, vars_file_src) = self.parse(ctx)?;
        let variable_sources = parsed.merge_into(test_plan)?;

        Ok((variable_sources, vars_file_src))
    }
}

/// Result of parsing CLI variables, containing scalar variables, array variables, and their sources.
#[derive(Debug)]
pub struct ParsedVariables {
    pub variables: HashMap<String, Scalar>,
    pub matrix_dimensions: HashMap<String, Vec<Scalar>>,
    pub variable_sources: HashMap<String, StableSource>,
}

impl ParsedVariables {
    /// Reconstruct the flat map of runtime overrides — scalars and matrix-dimension arrays in a
    /// single object, mirroring the user's `--vars` / `-v` input. This is the form captured for the
    /// REP payload.
    ///
    /// A key can appear in both maps when the same name is supplied as a `-v` scalar and a `--vars`
    /// array (`parse` keeps both; `merge` resolves it). Matrix dimensions are emitted first so a
    /// scalar wins such a collision, matching the CLI-over-file merge precedence.
    pub fn as_flat(&self) -> HashMap<String, ScalarOrArray> {
        self.matrix_dimensions
            .clone()
            .into_iter()
            .map(|(k, v)| (k, ScalarOrArray::Array(v)))
            .chain(
                self.variables
                    .clone()
                    .into_iter()
                    .map(|(k, v)| (k, ScalarOrArray::Scalar(v))),
            )
            .collect()
    }

    /// Merge these parsed variables into a test plan, returning the per-variable source map.
    /// See [`Variables::merge`] for the precedence rules applied.
    pub fn merge_into<E: Execution>(
        self,
        test_plan: &mut TestPlan<E>,
    ) -> Result<HashMap<String, StableSource>> {
        self.merge_inner(&mut test_plan.variables, &mut test_plan.matrix.dimensions)
    }

    #[inline]
    fn merge_inner(
        self,
        variables_from_test_plan: &mut HashMap<String, Scalar>,
        matrix_from_test_plan: &mut HashMap<String, Vec<Scalar>>,
    ) -> Result<HashMap<String, StableSource>> {
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
    use rtf_config::{StableSource, templating::Scalar};
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
        let from_cli = Variables {
            var: vec![
                "bar=love".to_string(),
                "qux=123".to_string(),
                "qux=456".to_string(),
            ],
            vars: Some(PathBuf::from("my-variables.json")),
        };

        let parsed = from_cli
            .parse_inner(Some((
                StableSource::VariablesFile,
                variables_json!({
                    "bar": "laugh", "baz": false
                }),
            )))
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
        let from_cli = Variables {
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
            .parse_inner(Some((
                StableSource::VariablesFile,
                variables_json!({
                    "foo": [2], "baz": 42
                }),
            )))
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
        let from_cli = Variables {
            var: vec![val.to_string()],
            vars: None,
        };

        let res = from_cli.parse_inner(None);

        assert!(res.is_err(), "expected error, ended up with {res:?}");
    }

    #[test]
    fn as_flat_preserves_scalars_and_arrays() {
        let parsed = ParsedVariables {
            variables: variables_map!("foo" => 42, "name" => "live"),
            matrix_dimensions: HashMap::from([(
                "tier".to_string(),
                vec![1.into(), 2.into(), 3.into()],
            )]),
            variable_sources: HashMap::new(),
        };

        let flat = parsed.as_flat();

        assert_eq!(flat.len(), 3);
        assert_eq!(
            flat.get("foo"),
            Some(&ScalarOrArray::Scalar(Scalar::from(42)))
        );
        assert_eq!(
            flat.get("name"),
            Some(&ScalarOrArray::Scalar(Scalar::from("live")))
        );
        assert_eq!(
            flat.get("tier"),
            Some(&ScalarOrArray::Array(vec![1.into(), 2.into(), 3.into()]))
        );
    }

    #[test]
    fn as_flat_scalar_wins_when_key_is_both_scalar_and_dimension() {
        let parsed = ParsedVariables {
            variables: variables_map!("dupe" => 99),
            matrix_dimensions: HashMap::from([("dupe".to_string(), vec![1.into(), 2.into()])]),
            variable_sources: HashMap::new(),
        };

        let flat = parsed.as_flat();

        assert_eq!(flat.len(), 1);
        assert_eq!(
            flat.get("dupe"),
            Some(&ScalarOrArray::Scalar(Scalar::from(99)))
        );
    }
}
