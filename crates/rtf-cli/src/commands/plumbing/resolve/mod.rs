//! Commands for resolving file providers independently of executing commands.
use std::collections::HashMap;

pub mod environment;
pub mod scenario;

pub use environment::resolve_environment;
pub use scenario::resolve_scenario;

/// Generate an env file from a map of environment variables.
fn generate_env_file(env_vars: HashMap<String, String>) -> String {
    let mut sorted_vars: Vec<_> = env_vars.into_iter().collect();
    sorted_vars.sort_by(|(a, _), (b, _)| a.cmp(b));

    sorted_vars
        .into_iter()
        .fold(String::new(), |mut acc, (key, value)| {
            acc.push_str(&format!("export {key}=\"{value}\"\n"));
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    #[test_case(&[], ""; "empty")]
    #[test_case(&[("FOO", "bar")], "export FOO=\"bar\"\n"; "single variable")]
    #[test_case(&[("FOO", "hello world")], "export FOO=\"hello world\"\n"; "value with space")]
    #[test_case(&[("ZZZ", "last"), ("AAA", "first")], "export AAA=\"first\"\nexport ZZZ=\"last\"\n"; "multiple variables")]
    #[test]
    fn generate_env_file_formats_correctly(vars: &[(&str, &str)], expected: &str) {
        let env_vars = HashMap::from_iter(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let result = generate_env_file(env_vars);

        assert_eq!(result, expected);
    }
}
