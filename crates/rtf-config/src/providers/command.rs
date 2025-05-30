use crate::{
    context::ResolutionContext,
    providers::{
        self,
        file::{AsUtf8FileContent, NamedFileProvider},
    },
    templating::{self, Field, Scalar, Template},
    validation::{self, Validate, duplicate_keys},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, path::Path, process::ExitStatus};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandSection {
    pub command: String,
    #[serde(default)]
    pub env_vars: HashMap<String, Field<String>>,
    #[serde(default)]
    pub file_providers: Vec<NamedFileProvider>,
}

impl CommandSection {
    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory before executing the command with the specified
    /// environment.
    ///
    /// [0]: crate::providers::file::FileProvider
    pub async fn run_providers_and_execute(
        &self,
        provider_dir: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<ExitStatus> {
        self.run_providers(provider_dir, ctx).await?;
        self.execute(provider_dir, ctx)
    }

    /// Execute this command with the specified environment.
    ///
    /// [CommandSection::run_providers] must have been run successfully before calling this method
    /// in order to ensure that all file providers have written out their file content to the
    /// expected location.
    pub fn execute(
        &self,
        provider_dir: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<ExitStatus> {
        let mut args: Vec<&str> = self.command.split_whitespace().collect();
        if args.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "no command provided").into());
        }

        let prog = args.remove(0);
        let env_vars = self.all_env_vars(provider_dir);
        let status = ctx.run_command_blocking(prog, &args, &env_vars)?;

        Ok(status)
    }

    /// Combine the base environment variables we have with the ones coming from the file providers
    /// we need to run. The `provider_dir` argument here needs to match the one used when running
    /// and outputting the content of the file providers.
    pub fn all_env_vars(&self, provider_dir: &Path) -> HashMap<String, String> {
        let mut vars: HashMap<String, String> = self
            .env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_resolved().clone()))
            .collect();

        for nfp in self.file_providers.iter() {
            let path = provider_dir.join(&nfp.name).display().to_string();
            vars.insert(nfp.env_var.clone(), path);
        }

        vars
    }

    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory.
    ///
    /// [0]: crate::providers::file::FileProvider
    pub async fn run_providers(
        &self,
        provider_dir: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        for nfp in self.file_providers.iter() {
            let content = nfp.try_get_file_content(ctx).await?;
            let file_path = provider_dir.join(&nfp.name);
            ctx.write(file_path, content)?;
        }

        Ok(())
    }
}

impl Template for CommandSection {
    fn has_pending_fields(&self) -> bool {
        self.env_vars.values().any(|f| f.has_pending_fields())
            | self
                .file_providers
                .iter()
                .any(|nfp| nfp.provider.has_pending_fields())
    }

    fn required_values(&self) -> Vec<String> {
        let mut vals: Vec<String> = self
            .env_vars
            .values()
            .flat_map(|f| f.required_values())
            .collect();

        for nfp in self.file_providers.iter() {
            vals.extend(nfp.required_values());
        }

        vals
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();
        path.push("env_vars".to_string());

        for (name, f) in self.env_vars.iter_mut() {
            let tail = name.clone();
            if let Err(e) = f.try_resolve_nested(path, tail, values) {
                errs.extend(e);
            };
        }

        path.pop();
        path.push("file_providers".to_string());

        for nfp in self.file_providers.iter_mut() {
            let tail = nfp.env_var.clone();
            if let Err(e) = nfp.try_resolve_nested(path, tail, values) {
                errs.extend(e);
            };
        }

        errs.into_result(())
    }
}

impl Validate for CommandSection {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        let mut errs = validation::ErrorBuilder::new();

        for nfp in self.file_providers.iter() {
            let tail = nfp.name.clone();
            if let Err(e) = nfp.provider.try_validate_nested(path, tail, ctx) {
                errs.extend(e);
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
                path,
            );
        }

        errs.into_result(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        providers::file::{FileProvider, InlineFile},
    };
    use simple_test_case::dir_cases;
    use simple_txtar::Archive;
    use std::{os::unix::process::ExitStatusExt, path::PathBuf, sync::Mutex};

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

        let res = section.try_validate(&mut Vec::new(), &ctx);
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
        let res = section.try_validate(&mut Vec::new(), &ctx);

        assert!(res.is_err(), "expected validation failures");
        let errs = res.unwrap_err();

        // Validation Errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(
            &concatenated_errs, expected,
            "wrong validation errors: {errs:?}"
        );
    }

    /// Stub implementation of ResolutionContext for testing command execution that tracks which
    /// files have been written to disk.
    #[derive(Debug, Default)]
    struct MockCommandContext {
        written_files: Mutex<HashMap<String, String>>,
    }

    impl ResolutionContext for MockCommandContext {
        fn run_command_blocking(
            &self,
            _prog: &str,
            _args: &[&str],
            _env_vars: &HashMap<String, String>,
        ) -> io::Result<ExitStatus> {
            Ok(ExitStatus::from_raw(0))
        }

        fn write(&self, path: impl AsRef<Path>, content: impl AsRef<[u8]>) -> io::Result<()> {
            let s = String::from_utf8(content.as_ref().to_vec()).expect("valid utf8");
            self.written_files
                .lock()
                .unwrap()
                .insert(path.as_ref().display().to_string(), s);

            Ok(())
        }

        fn path_exists(&self, _path: impl AsRef<Path>) -> bool {
            true
        }

        fn resolve_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
            Ok(relative_path.as_ref().to_path_buf())
        }

        fn path_is_file(&self, _path: impl AsRef<Path>) -> bool {
            true
        }

        fn read_path_to_string(&self, _path: impl AsRef<Path>) -> io::Result<String> {
            Ok(String::new())
        }
    }

    fn test_cmd_section() -> CommandSection {
        CommandSection {
            command: String::default(),
            env_vars: [("FOO", "hello"), ("BAR", "world")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), Field::Resolved(v.to_string())))
                .collect(),
            file_providers: vec![
                NamedFileProvider {
                    name: "fp1.txt".to_string(),
                    env_var: "FP1".to_string(),
                    provider: FileProvider::Inline(InlineFile {
                        content: "foo".to_string(),
                    }),
                },
                NamedFileProvider {
                    name: "fp2.txt".to_string(),
                    env_var: "FP2".to_string(),
                    provider: FileProvider::Inline(InlineFile {
                        content: "bar".to_string(),
                    }),
                },
            ],
        }
    }

    #[test]
    fn all_env_vars_includes_file_providers() {
        let c = test_cmd_section();
        let env_vars = c.all_env_vars(&PathBuf::from("/example-dir"));

        let expected: HashMap<String, String> = [
            ("FOO", "hello"),
            ("BAR", "world"),
            ("FP1", "/example-dir/fp1.txt"),
            ("FP2", "/example-dir/fp2.txt"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        assert_eq!(env_vars, expected);
    }

    #[tokio::test]
    async fn run_providers_writes_the_expected_files() {
        let c = test_cmd_section();
        let ctx = MockCommandContext::default();

        let res = c.run_providers(&PathBuf::from("/example-dir"), &ctx).await;
        assert!(res.is_ok(), "unexpected error: {res:?}");

        let written_files = ctx.written_files.into_inner().unwrap();
        let expected: HashMap<String, String> = [
            ("/example-dir/fp1.txt", "foo"),
            ("/example-dir/fp2.txt", "bar"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        assert_eq!(written_files, expected);
    }
}
