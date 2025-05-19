//! The various different config file formats that we support
use crate::{providers, validation};
use std::{collections::HashMap, io};

mod environment;
mod test_plan;

pub use environment::{EnvironmentConfig, RawEnvironmentConfig};
pub use test_plan::{BaseTestPlanConfig, RawBaseTestPlanConfig};

/// Errors that can be encountered resolving config files
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("one or more file providers failed to run:\n{}", .errs.join("\n"))]
    FailedFileProviders { errs: Vec<String> },

    #[error("the config file being parsed was invalid:\n{0}")]
    Validation(#[from] validation::Errors),

    // wrapped errors
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Provider(#[from] providers::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A helper function for replacing templated value strings in config with their actual values.
/// This takes a map of value keys, constructs the expected template format of {{ value.<key> }},
/// looks for this in the provided config string and replaces it with the actual value
pub(crate) fn apply_values(
    config: impl Into<String>,
    values: &HashMap<String, serde_json::Value>,
) -> String {
    let mut config = config.into();

    for (key, value) in values.iter() {
        // Create the expected template string from the key
        // Format is "{{ key }}"
        let key_template_str = format!("\"{{{{ {key} }}}}\"");
        println!("{key_template_str:?}");

        config = config.replace(&key_template_str, &value.to_string());
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    // TO DO: Make this into a macro so I can pass in values other than strings
    fn make_map(values: &[(&str, &str)]) -> HashMap<String, Value> {
        values
            .into_iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }

    #[test]
    fn apply_single_string_to_template() {
        let values = make_map(&[("foo", "bar")]);
        let config = "parameters:\n  some_param: \"{{ foo }}\"".to_string();

        let config = apply_values(&config, &values);
        assert_eq!(config, "parameters:\n  some_param: \"bar\"")
    }

    #[test]
    fn apply_single_integer_to_template() {
        let mut values = HashMap::new();
        values.insert("foo".to_string(), json!(true));
        let config = "parameters:\n  some_param: \"{{ foo }}\"".to_string();

        let config = apply_values(&config, &values);
        assert_eq!(config, "parameters:\n  some_param: true")
    }

    #[test]
    fn apply_single_float_to_template() {
        let mut values = HashMap::new();
        values.insert("foo".to_string(), json!(42.42));
        let config = "parameters:\n  some_param: \"{{ foo }}\"".to_string();

        let config = apply_values(&config, &values);
        assert_eq!(config, "parameters:\n  some_param: 42.42")
    }

    #[test]
    fn apply_single_bool_to_template() {
        let mut values = HashMap::new();
        values.insert("foo".to_string(), json!(42.42));
        let config = "parameters:\n  some_param: \"{{ foo }}\"".to_string();

        let config = apply_values(&config, &values);
        assert_eq!(config, "parameters:\n  some_param: 42.42")
    }

    #[test]
    fn apply_multiple_values_to_template() {
        let values = make_map(&[("foo", "bar"), ("hello", "goodbye")]);

        let config = "parameters:\n  some_param: \"{{ foo }}\"\n  another_param: \"{{ hello }}\""
            .to_string();

        let config = apply_values(&config, &values);
        assert_eq!(
            config,
            "parameters:\n  some_param: \"bar\"\n  another_param: \"goodbye\""
        )
    }

    #[test]
    fn apply_same_value_to_template_multiple_times() {
        let values = make_map(&[("foo", "bar")]);
        let config =
            "parameters:\n  some_param: \"{{ foo }}\"\n  another_param: \"{{ foo }}\"".to_string();

        let config = apply_values(&config, &values);

        assert_eq!(
            config,
            "parameters:\n  some_param: \"bar\"\n  another_param: \"bar\""
        )
    }
}
