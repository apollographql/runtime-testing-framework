use rtf_config::{
    Prepare, SourceDir, StableSource, context::ResolutionContext, formats::TestPlan,
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
pub enum VariableOverride {
    Scalar(Scalar),
    Array(Vec<Scalar>),
    Compound(Vec<HashMap<String, Scalar>>),
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
                let variables_json: HashMap<String, VariableOverride> = serde_json::from_str(&s)?;
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
        variable_json_data: Option<(StableSource, HashMap<String, VariableOverride>)>,
    ) -> Result<ParsedVariables> {
        let mut variables = HashMap::new();
        let mut matrix_dimensions = HashMap::new();
        let mut compound_dimensions = HashMap::new();
        let mut variable_sources = HashMap::new();

        // --vars variables read from a file can be individual variables, matrix dimensions, or
        // compound matrix dimension groups
        if let Some((source, from_variables)) = variable_json_data {
            for (k, v) in from_variables.into_iter() {
                match v {
                    VariableOverride::Scalar(s) => {
                        variables.insert(k.clone(), s);
                    }
                    VariableOverride::Array(arr) => {
                        matrix_dimensions.insert(k.clone(), arr);
                    }
                    VariableOverride::Compound(entries) => {
                        compound_dimensions.insert(k.clone(), entries);
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
            compound_dimensions,
            variable_sources,
        })
    }

    /// Merge any variables obtained from the CLI with the ones found in a test plan, removing any
    /// existing variable or matrix definitions with the same key.
    ///
    /// Returns the variable sources map alongside the [SourceDir] of the `--vars` file, if one
    /// was provided. Callers are responsible for incorporating this into [rtf_config::formats::Sources] via
    /// [ResolutionContext::set_sources].
    pub fn merge<P: Prepare>(
        self,
        test_plan: &mut TestPlan<P>,
        ctx: &impl ResolutionContext,
    ) -> Result<(HashMap<String, StableSource>, Option<SourceDir>)> {
        let (parsed, vars_file_src) = self.parse(ctx)?;
        let variable_sources = parsed.merge_into(test_plan)?;

        Ok((variable_sources, vars_file_src))
    }
}

#[derive(Debug, Default)]
pub struct ParsedVariables {
    pub variables: HashMap<String, Scalar>,
    pub matrix_dimensions: HashMap<String, Vec<Scalar>>,
    pub compound_dimensions: HashMap<String, Vec<HashMap<String, Scalar>>>,
    pub variable_sources: HashMap<String, StableSource>,
}

impl ParsedVariables {
    pub fn from_flat(flat: HashMap<String, VariableOverride>) -> Self {
        let mut vars = Self::default();

        for (k, v) in flat.into_iter() {
            vars.variable_sources.insert(k.clone(), StableSource::Cli);

            match v {
                VariableOverride::Scalar(s) => {
                    vars.variables.insert(k, s);
                }

                VariableOverride::Array(arr) => {
                    vars.matrix_dimensions.insert(k, arr);
                }

                VariableOverride::Compound(entries) => {
                    vars.compound_dimensions.insert(k, entries);
                }
            }
        }

        vars
    }

    /// Reconstruct the flat map of runtime overrides — scalars, matrix-dimension arrays, and
    /// compound matrix dimension groups in a single object, mirroring the user's `--vars` / `-v`
    /// input. This is the form captured for the Orchestrator's trigger payload.
    ///
    /// A key can appear in more than one of these maps when the same name is supplied as a `-v`
    /// scalar and a `--vars` array or compound group (`parse` keeps both; `merge` resolves it).
    /// Compound groups are emitted first, then matrix dimensions, then scalars, so a scalar wins
    /// such a collision, matching the CLI-over-file merge precedence.
    pub fn as_flat(&self) -> HashMap<String, VariableOverride> {
        self.compound_dimensions
            .clone()
            .into_iter()
            .map(|(k, v)| (k, VariableOverride::Compound(v)))
            .chain(
                self.matrix_dimensions
                    .clone()
                    .into_iter()
                    .map(|(k, v)| (k, VariableOverride::Array(v))),
            )
            .chain(
                self.variables
                    .clone()
                    .into_iter()
                    .map(|(k, v)| (k, VariableOverride::Scalar(v))),
            )
            .collect()
    }

    /// Merge these parsed variables into a test plan, returning the per-variable source map.
    /// See [`Variables::merge`] for the precedence rules applied.
    pub fn merge_into<P: Prepare>(
        self,
        test_plan: &mut TestPlan<P>,
    ) -> Result<HashMap<String, StableSource>> {
        self.merge_inner(
            &mut test_plan.variables,
            &mut test_plan.matrix.dimensions,
            &mut test_plan.matrix.compound,
        )
    }

    #[inline]
    fn merge_inner(
        self,
        variables_from_test_plan: &mut HashMap<String, Scalar>,
        dimensions_from_test_plan: &mut HashMap<String, Vec<Scalar>>,
        compound_from_test_plan: &mut HashMap<String, Vec<HashMap<String, Scalar>>>,
    ) -> Result<HashMap<String, StableSource>> {
        for (k, dim) in self.matrix_dimensions.into_iter() {
            if dim.is_empty() && compound_from_test_plan.contains_key(&k) {
                // "k" here is the name of a compound dimension rather than a real variable name,
                // so there is nothing to insert. All we are doing is removing a named compound
                // dimension to free up those variables for use elsewhere.
                compound_from_test_plan.remove(&k);
                continue;
            }

            variables_from_test_plan.remove(&k);
            compound_from_test_plan.remove(&k);
            dimensions_from_test_plan.insert(k.clone(), dim);
        }

        for (k, entries) in self.compound_dimensions.into_iter() {
            variables_from_test_plan.remove(&k);
            dimensions_from_test_plan.remove(&k);
            compound_from_test_plan.insert(k.clone(), entries);
        }

        for (k, v) in self.variables.into_iter() {
            dimensions_from_test_plan.remove(&k);
            compound_from_test_plan.remove(&k);
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
        let mut dimensions = HashMap::default();
        let mut compound = HashMap::default();
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
        parsed
            .merge_inner(&mut variables, &mut dimensions, &mut compound)
            .unwrap();

        assert!(dimensions.is_empty());

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
        let mut dimensions: HashMap<String, Vec<Scalar>> = HashMap::default();
        dimensions.insert("bar".into(), vec![1.into()]);
        dimensions.insert("baz".into(), vec![2.into()]);
        let mut compound = HashMap::default();

        // change bar to a variable from the cli
        // change baz to a variable from variables.json
        // change foo to a matrix dimension from variables.json
        let from_cli = Variables {
            var: vec!["bar=3".to_string()],
            vars: Some(PathBuf::from("my-variables.json")),
        };

        // Keys should start mutually exclusive
        let mut initial_variables: Vec<&String> = variables.keys().collect();
        let mut initial_matrix_dimensions: Vec<&String> = dimensions.keys().collect();
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
        parsed
            .merge_inner(&mut variables, &mut dimensions, &mut compound)
            .unwrap();

        // Keys should end mutually exclusive but flipped
        let mut final_variables: Vec<&String> = variables.keys().collect();
        let mut final_matrix_dimensions: Vec<&String> = dimensions.keys().collect();
        final_variables.sort_unstable();
        final_matrix_dimensions.sort_unstable();

        assert_eq!(&final_variables, &["bar", "baz"]);
        assert_eq!(&final_matrix_dimensions, &["foo"]);
    }

    #[test]
    fn compound_override_adds_a_new_group() {
        let mut variables = HashMap::default();
        let mut dimensions = HashMap::default();
        let mut compound: HashMap<String, Vec<HashMap<String, Scalar>>> = HashMap::default();

        let from_cli = Variables {
            var: vec![],
            vars: Some(PathBuf::from("my-variables.json")),
        };
        let parsed = from_cli
            .parse_inner(Some((
                StableSource::VariablesFile,
                variables_json!({
                    "subjects": [
                        {"setup_subject": "fish", "scenario_subject": "chips"},
                        {"setup_subject": "bread", "scenario_subject": "butter"}
                    ]
                }),
            )))
            .unwrap();
        parsed
            .merge_inner(&mut variables, &mut dimensions, &mut compound)
            .unwrap();

        assert_eq!(compound.len(), 1);
        assert_eq!(
            compound.get("subjects"),
            Some(&vec![
                variables_map!("setup_subject" => "fish", "scenario_subject" => "chips"),
                variables_map!("setup_subject" => "bread", "scenario_subject" => "butter"),
            ])
        );
    }

    #[test]
    fn compound_override_replaces_an_existing_group() {
        let mut variables = HashMap::default();
        let mut dimensions = HashMap::default();
        let mut compound: HashMap<String, Vec<HashMap<String, Scalar>>> = HashMap::from([(
            "subjects".to_string(),
            vec![variables_map!("setup_subject" => "world", "scenario_subject" => "sailor")],
        )]);

        let from_cli = Variables {
            var: vec![],
            vars: Some(PathBuf::from("my-variables.json")),
        };
        let parsed = from_cli
            .parse_inner(Some((
                StableSource::VariablesFile,
                variables_json!({
                    "subjects": [
                        {"setup_subject": "fish", "scenario_subject": "chips"}
                    ]
                }),
            )))
            .unwrap();
        parsed
            .merge_inner(&mut variables, &mut dimensions, &mut compound)
            .unwrap();

        assert_eq!(
            compound.get("subjects"),
            Some(&vec![
                variables_map!("setup_subject" => "fish", "scenario_subject" => "chips")
            ])
        );
    }

    #[test]
    fn empty_array_override_removes_an_existing_compound_group() {
        let mut variables = HashMap::default();
        let mut dimensions = HashMap::default();
        let mut compound: HashMap<String, Vec<HashMap<String, Scalar>>> = HashMap::from([(
            "subjects".to_string(),
            vec![variables_map!("setup_subject" => "world", "scenario_subject" => "sailor")],
        )]);

        let from_cli = Variables {
            var: vec![],
            vars: Some(PathBuf::from("my-variables.json")),
        };
        let parsed = from_cli
            .parse_inner(Some((
                StableSource::VariablesFile,
                variables_json!({
                    "subjects": [],
                    "setup_subject": "fish",
                    "scenario_subject": "chips"
                }),
            )))
            .unwrap();
        parsed
            .merge_inner(&mut variables, &mut dimensions, &mut compound)
            .unwrap();

        assert!(compound.is_empty());
        assert!(dimensions.is_empty());
        assert_eq!(variables.get("setup_subject"), Some(&Scalar::from("fish")));
        assert_eq!(
            variables.get("scenario_subject"),
            Some(&Scalar::from("chips"))
        );
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
            compound_dimensions: HashMap::new(),
            variable_sources: HashMap::new(),
        };

        let flat = parsed.as_flat();

        assert_eq!(flat.len(), 3);
        assert_eq!(
            flat.get("foo"),
            Some(&VariableOverride::Scalar(Scalar::from(42)))
        );
        assert_eq!(
            flat.get("name"),
            Some(&VariableOverride::Scalar(Scalar::from("live")))
        );
        assert_eq!(
            flat.get("tier"),
            Some(&VariableOverride::Array(vec![1.into(), 2.into(), 3.into()]))
        );
    }

    #[test]
    fn as_flat_scalar_wins_when_key_is_both_scalar_and_dimension() {
        let parsed = ParsedVariables {
            variables: variables_map!("dupe" => 99),
            matrix_dimensions: HashMap::from([("dupe".to_string(), vec![1.into(), 2.into()])]),
            compound_dimensions: HashMap::new(),
            variable_sources: HashMap::new(),
        };

        let flat = parsed.as_flat();

        assert_eq!(flat.len(), 1);
        assert_eq!(
            flat.get("dupe"),
            Some(&VariableOverride::Scalar(Scalar::from(99)))
        );
    }

    #[test]
    fn as_flat_round_trips_a_compound_override() {
        let parsed = ParsedVariables {
            variables: HashMap::new(),
            matrix_dimensions: HashMap::new(),
            compound_dimensions: HashMap::from([(
                "subjects".to_string(),
                vec![variables_map!("setup_subject" => "fish", "scenario_subject" => "chips")],
            )]),
            variable_sources: HashMap::new(),
        };

        let flat = parsed.as_flat();

        assert_eq!(flat.len(), 1);
        assert_eq!(
            flat.get("subjects"),
            Some(&VariableOverride::Compound(vec![
                variables_map!("setup_subject" => "fish", "scenario_subject" => "chips")
            ]))
        );
    }
}
