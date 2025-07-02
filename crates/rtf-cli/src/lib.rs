//! Runtime Testing Framework CLI - a swiss army knife for testing the Apollo Runtime
use anyhow::{Context, anyhow};
use rtf_config::{context::ResolutionContext, templating::Scalar};
use std::{collections::HashMap, io, path::Path};

pub mod cli;
pub mod commands;

// The `cli.rs` file is pulled in as an inline module so we can generate markdown help for the CLI
// interface in `build.rs`. Any dependencies we make use of in that file need to be included in the
// build dependencies (rather than main crate dependencies), so we implement this method here
// instead.

impl cli::Values {
    /// Merge any values obtained from the CLI with the ones found in a test plan.
    pub fn merge(
        self,
        from_test_plan: &mut HashMap<String, Scalar>,
        ctx: &mut impl ResolutionContext,
    ) -> anyhow::Result<()> {
        self.merge_inner(from_test_plan, |path| ctx.read_path_to_string(path))?;
        ctx.set_values(from_test_plan);

        Ok(())
    }

    #[inline]
    fn merge_inner(
        self,
        from_test_plan: &mut HashMap<String, Scalar>,
        read_file: impl Fn(&Path) -> io::Result<String>,
    ) -> anyhow::Result<()> {
        if let Some(path) = self.values.as_ref() {
            let s = (read_file)(path)
                .context(format!("Unable to read values file {}", path.display()))?;
            let from_values: HashMap<String, Scalar> =
                serde_json::from_str(&s).context("invalid values file")?;

            from_test_plan.extend(from_values);
        }

        for kv in self.value.into_iter() {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| anyhow!("expected \"key=value\", got {kv:?}"))?;

            if k.is_empty() {
                return Err(anyhow!("no key provided for value {v:?}"));
            }

            let v: Scalar = serde_yaml::from_str(v).context(format!("invalid value for {k:?}"))?;
            from_test_plan.insert(k.to_string(), v);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;
    use std::path::PathBuf;

    macro_rules! values_map {
        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();
            $( m.insert($k.to_string(), rtf_config::templating::Scalar::try_from($v).unwrap()); )+
            m
        }};
    }

    #[test]
    fn values_are_merged_in_the_correct_order() {
        let mut from_test_plan = values_map!("foo" => 42, "bar" => "live", "baz" => true);
        let from_cli = cli::Values {
            value: vec![
                "bar=love".to_string(),
                "qux=123".to_string(),
                "qux=456".to_string(),
            ],
            values: Some(PathBuf::from("my-values.json")),
        };
        let read_file =
            |_: &Path| io::Result::Ok(r#"{ "bar": "laugh", "baz": false }"#.to_string());

        from_cli
            .merge_inner(&mut from_test_plan, read_file)
            .unwrap();

        // foo is not overwritten and should remain what was in the test plan
        assert_eq!(from_test_plan.get("foo"), Some(&Scalar::from(42)));

        // bar is overwritten from both the values json file and a cli arg: cli should be preferred
        assert_eq!(from_test_plan.get("bar"), Some(&Scalar::from("love")));

        // baz is overwritten in the values json file
        assert_eq!(from_test_plan.get("baz"), Some(&Scalar::from(false)));

        // qux is an additional value added as a cli arg twice: we should get the last value set
        assert_eq!(from_test_plan.get("qux"), Some(&Scalar::from(456)));
    }

    #[test_case("bar should have an equals before value"; "no equals")]
    #[test_case("bar=[1, 2, 3]"; "invalid scalar")]
    #[test_case("bar="; "no value")]
    #[test_case("=value"; "no key")]
    #[test_case(""; "empty string")]
    #[test]
    fn an_invalid_value_returns_an_error(val: &str) {
        let from_cli = cli::Values {
            value: vec![val.to_string()],
            values: None,
        };

        let mut vals = HashMap::default();
        let res = from_cli.merge_inner(&mut vals, |_: &Path| panic!("should not be called"));

        assert!(res.is_err(), "expected error, ended up with {vals:?}");
    }
}
