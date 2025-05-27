use crate::{
    providers::{
        Context,
        file::{IntoUtf8FileContent, NamedFileProvider},
    },
    templating::{self, Field, Scalar, Templatable},
    validation::{self, duplicate_keys},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandSection {
    pub command: String,
    #[serde(default)]
    pub env_vars: HashMap<String, Field<String>>,
    #[serde(default)]
    pub file_providers: Vec<NamedFileProvider>,
}

impl Templatable for CommandSection {
    fn has_pending_fields(&self) -> bool {
        self.env_vars.values().any(|f| f.has_pending_fields())
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
        errs: &mut Vec<templating::Error>,
    ) {
        for f in self.env_vars.values_mut() {
            f.try_resolve_nested(path, "env_var", values, errs)
        }

        path.push("file_provider".to_string());

        for nfp in self.file_providers.iter_mut() {
            let tail = nfp.name.clone();
            nfp.try_resolve_nested(path, tail, values, errs);
        }
    }
}

impl CommandSection {
    pub fn validate(&self, ctx: &Context) -> validation::Result<()> {
        let mut errs = validation::ErrorBuilder::new();

        for nfp in self.file_providers.iter() {
            if let Err(e) = nfp.provider.validate(ctx) {
                errs.extend_with_prefix(e, format!("file provider {:?}:", nfp.name));
            }
        }

        let env_var_names = self
            .env_vars
            .keys()
            .map(|k| k.as_str())
            .chain(self.file_providers.iter().map(|f| f.env_var.as_str()));

        let duplicates = duplicate_keys(env_var_names, |name| name);

        if !duplicates.is_empty() {
            errs.push(
                validation::ErrorKind::DuplicateEnvironmentVariables,
                duplicates.join("\n"),
            );
        }

        errs.into_result(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::dir_cases;
    use simple_txtar::Archive;
    use std::path::PathBuf;

    /// Load a txtar [Archive] from the given file content and print the top level comment if there
    /// is one before returning it.
    fn load_archive(content: &str) -> Archive {
        let arr = Archive::from(content);
        let comment = arr.comment();
        if !comment.is_empty() {
            println!("{}", comment.trim());
        }

        arr
    }

    /// Read the requested file from the archive, panicking if it is missing
    fn get_file<'a>(arr: &'a Archive, fname: &str) -> &'a str {
        match arr.get(fname) {
            Some(f) => f.content.trim(),
            None => {
                panic!("required txtar file section {fname:?} was missing");
            }
        }
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/command/valid")]
    #[tokio::test]
    async fn valid_providers(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");

        let section: CommandSection = match serde_yaml::from_str(config) {
            Ok(section) => section,
            Err(e) => panic!("expected a valid CommandSection, got: {e}"),
        };

        let ctx = Context::new(
            PathBuf::from("resources/provider-tests/command/valid")
                .canonicalize()
                .unwrap(),
        );

        let res = section.validate(&ctx);
        assert!(res.is_ok(), "expected to validate but got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/command/parse-failures")]
    #[test]
    fn parse_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let res: serde_yaml::Result<CommandSection> = serde_yaml::from_str(config);

        assert!(res.is_err(), "expected invalid YAML, got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/command/validation-failures")]
    #[test]
    fn validation_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "validation-errors");

        let section: CommandSection = match serde_yaml::from_str(config) {
            Ok(section) => section,
            Err(e) => panic!("expected a valid CommandSection, got: {e}"),
        };

        let ctx = Context::new(
            PathBuf::from("resources/provider-tests/command/validation-failures")
                .canonicalize()
                .unwrap(),
        );
        let res = section.validate(&ctx);

        assert!(res.is_err(), "expected validation failures");
        let errs = res.unwrap_err();

        // Validation Errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind()));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(
            &concatenated_errs, expected,
            "wrong validation errors: {errs:?}"
        );
    }
}
