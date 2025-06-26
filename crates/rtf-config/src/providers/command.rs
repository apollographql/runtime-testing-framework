use crate::{
    context::ResolutionContext,
    providers::{
        self,
        file::{
            InlineFile, NamedFileProvider, RelativeFile, RequiredFile, ResolveAndWrite, Source,
        },
    },
    templating::{self, Field, Scalar, Template},
    validation::{self, Validate, duplicate_keys},
};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, path::Path};

/// The environment variable used to provide the location of the output directory to user specified
/// commands
const OUTDIR: &str = "OUTDIR";
const OUTFILE: &str = "RTF_OUTPUT";

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandSection {
    pub command: RawCommand,
    #[serde(default)]
    pub env_vars: HashMap<String, Field<String>>,
    #[serde(default)]
    pub file_providers: Vec<NamedFileProvider>,
}

impl CommandSection {
    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory before executing the command with the specified
    /// environment, returning the standard output.
    ///
    /// [0]: crate::providers::file::FileProvider
    pub async fn run_providers_and_execute(
        &self,
        out_dir: &Path,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        self.run_providers(out_dir, src, ctx).await?;
        self.execute(out_dir, ctx)
    }

    /// Execute this command with the specified environment, returning the standard output.
    ///
    /// [CommandSection::run_providers] must have been run successfully before calling this method
    /// in order to ensure that all file providers have written out their file content to the
    /// expected location.
    pub fn execute(
        &self,
        out_dir: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let out_file = out_dir.join(OUTFILE);
        let env_vars = self.all_env_vars(out_dir, &out_file);

        match &self.command {
            RawCommand::String(s) => {
                let command = s.clone();
                let mut it = command.split_whitespace();
                let prog = match it.next() {
                    Some(prog) => prog,
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "no command provided",
                        )
                        .into());
                    }
                };

                ctx.run_command_blocking(prog, it, &env_vars)?;
            }

            RawCommand::Spec(spec) => {
                let file_path = out_dir.join(spec.name.clone());
                let prog = match file_path.to_str() {
                    Some(v) => v,
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unable to convert command path to valid string",
                        )
                        .into());
                    }
                };
                let it = spec.args.iter().map(|arg| arg.as_resolved().as_str());
                ctx.run_command_blocking(prog, it, &env_vars)?;
            }
        }

        let output = match ctx.read_path_to_string(&out_file) {
            Ok(s) => {
                ctx.remove_file(out_file)?;
                s
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };

        Ok(output)
    }

    /// Combine the base environment variables we have with the ones coming from the file providers
    /// we need to run. The `out_dir` argument here needs to match the one used when running
    /// and outputting the content of the file providers.
    pub fn all_env_vars(&self, out_dir: &Path, out_file: &Path) -> HashMap<String, String> {
        let mut vars: HashMap<String, String> = self
            .env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_resolved().clone()))
            .collect();

        for nfp in self.file_providers.iter() {
            let path = out_dir.join(&nfp.name).display().to_string();
            vars.insert(nfp.env_var.clone(), path);
        }

        vars.insert(OUTDIR.to_string(), out_dir.display().to_string());
        vars.insert(OUTFILE.to_string(), out_file.display().to_string());

        vars
    }

    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory.
    ///
    /// [0]: crate::providers::file::FileProvider
    pub async fn run_providers(
        &self,
        provider_dir: &Path,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        if let RawCommand::Spec(spec) = &self.command {
            let file_path = provider_dir.join(&spec.name);
            spec.command_provider
                .resolve_and_write(&file_path, src, ctx)
                .await?;
            ctx.make_executable(&file_path)?;
        }

        for nfp in self.file_providers.iter() {
            nfp.resolve_and_write(&provider_dir.join(&nfp.name), src, ctx)
                .await?;
        }

        Ok(())
    }
}

impl Template for CommandSection {
    fn has_pending_fields(&self) -> bool {
        self.command.has_pending_fields()
            | self.env_vars.values().any(|f| f.has_pending_fields())
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

        vals.extend(self.command.required_values());

        vals
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();

        errs.append(self.command.try_resolve_nested(path, "command", values));

        path.push("env_vars".to_string());

        for (name, f) in self.env_vars.iter_mut() {
            let tail = name.clone();
            errs.append(f.try_resolve_nested(path, tail, values));
        }

        path.pop();
        path.push("file_providers".to_string());

        for nfp in self.file_providers.iter_mut() {
            let tail = nfp.env_var.clone();
            errs.append(nfp.try_resolve_nested(path, tail, values));
        }

        errs.into_result(())
    }
}

impl Validate for CommandSection {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        let mut errs = validation::ErrorBuilder::new();

        errs.append(self.command.try_validate_nested(path, "command", src, ctx));

        for nfp in self.file_providers.iter() {
            let tail = nfp.name.clone();
            errs.append(nfp.provider.try_validate_nested(path, tail, src, ctx));
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RawCommand {
    String(String),
    Spec(CommandSpec),
}

impl Default for RawCommand {
    fn default() -> Self {
        Self::String(String::default())
    }
}

impl Template for RawCommand {
    fn has_pending_fields(&self) -> bool {
        match self {
            RawCommand::String(_s) => false,
            RawCommand::Spec(spec) => spec.has_pending_fields(),
        }
    }

    fn required_values(&self) -> Vec<String> {
        match self {
            RawCommand::String(_s) => Vec::new(),
            RawCommand::Spec(spec) => spec.required_values(),
        }
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        match self {
            RawCommand::String(_s) => Ok(()),
            RawCommand::Spec(spec) => spec.try_resolve(path, values),
        }
    }
}

impl Validate for RawCommand {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        match self {
            RawCommand::String(_s) => Ok(()),
            RawCommand::Spec(spec) => spec.try_validate(path, src, ctx),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandSpec {
    pub name: String,
    #[serde(flatten)]
    pub command_provider: CommandProvider,
    #[serde(default)]
    pub args: Vec<Field<String>>,
}

impl Template for CommandSpec {
    fn has_pending_fields(&self) -> bool {
        self.command_provider.has_pending_fields()
            | self.args.iter().any(|f| f.has_pending_fields())
    }

    fn required_values(&self) -> Vec<String> {
        let mut vals = self.command_provider.required_values();

        for arg in self.args.iter() {
            vals.extend(arg.required_values());
        }

        vals
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.command_provider.try_resolve_nested(
            path,
            "command_provider",
            values,
        ));

        for arg in self.args.iter_mut() {
            errs.append(arg.try_resolve_nested(path, stringify!(arg), values));
        }

        errs.into_result(())
    }
}

impl Validate for CommandSpec {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        let mut errs = validation::ErrorBuilder::new();

        errs.append(
            self.command_provider
                .try_validate_nested(path, "command_provider", src, ctx),
        );

        errs.into_result(())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[enum_dispatch(Template, Validate, AsUtf8FileContent)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CommandProvider {
    Inline(InlineFile),
    RelativePath(RelativeFile),
    Required(RequiredFile),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::NullHttpClient;
    use crate::{
        context::{Context, NullPlatformClient, PathKind},
        providers::file::{FileProvider, InlineFile},
    };
    use simple_test_case::{dir_cases, test_case};
    use simple_txtar::Archive;
    use std::{path::PathBuf, sync::Mutex};

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

        let dir = PathBuf::from("resources/provider-tests/command/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));

        let res = section.try_validate(&mut Vec::new(), &src, &ctx);
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

        let dir = PathBuf::from("resources/provider-tests/command/validation-failures")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));
        let res = section.try_validate(&mut Vec::new(), &src, &ctx);

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

    #[dir_cases("crates/rtf-config/resources/provider-tests/command/valid-templates")]
    #[test]
    fn valid_templated_providers(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let raw_expected = get_file(&arr, "after-templating");

        let mut command: CommandSection = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = serde_yaml::from_str(raw_values).unwrap();
        let expected: CommandSection = serde_yaml::from_str(raw_expected).unwrap();

        assert!(command.has_pending_fields(), "fields should be pending");

        let res = command.try_resolve(&mut Vec::new(), &values);

        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(!command.has_pending_fields(), "fields should be resolved");
        assert_eq!(command, expected);
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/command/invalid-templates")]
    #[test]
    fn invalid_templated_providers(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let expected = get_file(&arr, "templating-errors");

        let mut command: CommandSection = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = serde_yaml::from_str(raw_values).unwrap();

        assert!(command.has_pending_fields(), "fields should be pending");

        let res = command.try_resolve(&mut Vec::new(), &values);

        assert!(
            command.has_pending_fields(),
            "fields should still be pending"
        );

        let errs = res.unwrap_err().into_vec();
        let str_errs: Vec<String> = errs.iter().map(|e| format!("{:?}", e.kind)).collect();

        assert_eq!(str_errs.join("\n"), expected.trim());
    }

    /// Stub implementation of ResolutionContext for testing command execution that tracks which
    /// files have been written to disk.
    #[derive(Debug, Default)]
    struct MockCommandContext {
        written_files: Mutex<HashMap<String, String>>,
    }

    impl ResolutionContext for MockCommandContext {
        type PlatformClient = NullPlatformClient;
        type HttpClient = NullHttpClient;

        fn run_command_blocking<'a>(
            &self,
            prog: &str,
            args: impl IntoIterator<Item = &'a str>,
            env_vars: &HashMap<String, String>,
        ) -> io::Result<()> {
            if prog == "WRITE_OUTPUT" {
                let path = env_vars.get(OUTFILE).expect("outfile env var not set");
                let content = args.into_iter().next().expect("no args").to_string();

                self.written_files
                    .lock()
                    .unwrap()
                    .insert(path.to_string(), content);
            }

            Ok(())
        }

        fn write(&self, path: impl AsRef<Path>, content: impl AsRef<[u8]>) -> io::Result<()> {
            let s = String::from_utf8(content.as_ref().to_vec()).expect("valid utf8");
            self.written_files
                .lock()
                .unwrap()
                .insert(path.as_ref().display().to_string(), s);

            Ok(())
        }

        fn path_kind(&self, _path: impl AsRef<Path>) -> crate::context::PathKind {
            PathKind::File
        }

        fn canonicalize_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
            relative_path.as_ref().canonicalize()
        }

        fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
            let k = path.as_ref().display().to_string();

            self.written_files
                .lock()
                .unwrap()
                .get(&k)
                .cloned()
                .ok_or(io::Error::new(io::ErrorKind::NotFound, ""))
        }

        fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
            let k = path.as_ref().display().to_string();
            self.written_files.lock().unwrap().remove(&k);

            Ok(())
        }

        fn set_current_dir(&mut self, _path: impl AsRef<Path>) -> io::Result<()> {
            Ok(())
        }

        fn create_dir_all(&self, _path: impl AsRef<Path>) -> io::Result<()> {
            Ok(())
        }
    }

    fn test_cmd_section() -> CommandSection {
        CommandSection {
            command: RawCommand::String(String::default()),
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
        let env_vars = c.all_env_vars(
            &PathBuf::from("/example-dir"),
            &PathBuf::from("/example-dir/RTF_OUTPUT"),
        );

        let expected: HashMap<String, String> = [
            ("FOO", "hello"),
            ("BAR", "world"),
            ("OUTDIR", "/example-dir"),
            ("FP1", "/example-dir/fp1.txt"),
            ("FP2", "/example-dir/fp2.txt"),
            ("RTF_OUTPUT", "/example-dir/RTF_OUTPUT"),
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
        let dir = PathBuf::from("/example-dir");
        let src = Source::Local {
            abs_path: dir.join("example.yaml"),
        };

        let res = c.run_providers(&dir, &src, &ctx).await;
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

    #[test_case("WRITE_OUTPUT foo", "foo"; "with output")]
    #[test_case("command-with-no-output", ""; "without output")]
    #[tokio::test]
    async fn execute_returns_the_contents_of_the_output_file_and_removes_it(
        command: &str,
        expected_output: &str,
    ) {
        let c = CommandSection {
            // See the implementation of MockCommandContext::run_command_blocking
            command: RawCommand::String(command.to_string()),
            env_vars: HashMap::new(),
            file_providers: Vec::new(),
        };

        let ctx = MockCommandContext::default();
        let dir = PathBuf::from("/example-dir");

        let output = c.execute(&dir, &ctx).expect("command to succeed");
        assert_eq!(output, expected_output, "unexpected output");

        let written_files = ctx.written_files.into_inner().unwrap();
        let k = dir.join(OUTFILE).display().to_string();

        assert!(
            !written_files.contains_key(&k),
            "should have removed the outfile"
        );
    }
}
