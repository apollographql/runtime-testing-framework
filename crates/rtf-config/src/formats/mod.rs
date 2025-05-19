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
/// This takes a map of value keys, constructs the expected template format of `"{{ key }}"`,
/// looks for this in the provided config string and replaces it with the actual value
#[allow(dead_code)]
pub(crate) fn apply_values(
    config: impl Into<String>,
    values: &HashMap<String, serde_json::Value>,
) -> String {
    let mut config = config.into();

    for (key, value) in values.iter() {
        // The string here looks crazy, we have to escape curly brackets with another curly bracket
        // So when we want { we have to specify {{
        // This will end up looking like: "{{ key }}"
        let key_template_str = format!("\"{{{{ {key} }}}}\"");

        config = config.replace(&key_template_str, &value.to_string());
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use simple_test_case::test_case;

    // Helper macro to create a HashMap<String, serde_json::Value> where the Value can be any valid json object.
    // Intended to be used for easily creating test values for testing templating
    macro_rules! values_map {
        () => {
            ::std::collections::HashMap::<String, ::serde_json::Value>::new()
        };

        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();

            $(
                m.insert($k.to_string(), ::serde_json::json!($v));
            )+

            m
        }};
    }

    #[test_case(
        values_map!(),
        "\"foo\"",
        "\"foo\"";
        "empty_value"
    )]
    #[test_case(
        values_map!("foo" => "bar"),
        "\"{{ foo }}\"",
        "\"bar\"";
        "single_string_value"
    )]
    #[test_case(
        values_map!("foo" => 42),
        "\"{{ foo }}\"",
        "42";
        "single_integer_value"
    )]
    #[test_case(
        values_map!("foo" => 42.42),
        "\"{{ foo }}\"",
        "42.42";
        "single_float_value"
    )]
    #[test_case(
        values_map!("foo" => true),
        "\"{{ foo }}\"",
        "true";
        "single_bool_value"
    )]
    #[test_case(
        values_map!("foo" => "bar", "baz" => "qux"),
        "\"{{ foo }}\"\n\"{{ baz }}\"",
        "\"bar\"\n\"qux\"";
        "multiple_string_values"
    )]
    #[test_case(
        values_map!("foo" => "bar"),
        "\"{{ foo }}\"\n\"{{ foo }}\"",
        "\"bar\"\n\"bar\"";
        "same_string_value_multiple_times"
    )]
    #[test]
    fn apply_template_values_works(values: HashMap<String, Value>, config: &str, expected: &str) {
        let config = apply_values(config, &values);
        assert_eq!(config, expected)
    }
}
