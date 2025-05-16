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
pub(crate) fn replace_template_strings_with_values(
    config: &str,
    values: HashMap<&str, &str>,
) -> String {
    let mut config = config.to_string();

    for (key, value) in values {
        // Create the expected template string from the key
        let key_template_str = format!("{{{{ value.{} }}}}", key);

        config = config.replace(&key_template_str, &value);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_template_values_single_key_and_value() {
        let mut values = HashMap::new();
        values.insert("foo", "bar");

        let config = "parameters:\n  some_param: {{ value.foo }}".to_string();

        let config = replace_template_strings_with_values(&config, values);

        assert_eq!(config, "parameters:\n  some_param: bar")
    }

    #[test]
    fn replace_template_values_multiple_value_keys() {
        let mut values = HashMap::new();
        values.insert("foo", "bar");
        values.insert("hello", "goodbye");

        let config =
            "parameters:\n  some_param: {{ value.foo }}\n  another_param: {{ value.hello }}";

        let config = replace_template_strings_with_values(&config, values);

        assert_eq!(
            config,
            "parameters:\n  some_param: bar\n  another_param: goodbye"
        )
    }

    #[test]
    fn replace_template_values_multiple_template_strings_same_key() {
        let mut values = HashMap::new();
        values.insert("foo", "bar");

        let config = "parameters:\n  some_param: {{ value.foo }}\n  another_param: {{ value.foo }}";

        let config = replace_template_strings_with_values(&config, values);

        assert_eq!(
            config,
            "parameters:\n  some_param: bar\n  another_param: bar"
        )
    }
}
