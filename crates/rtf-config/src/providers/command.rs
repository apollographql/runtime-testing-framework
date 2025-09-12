use crate::{
    checks::{self, Check, duplicate_keys},
    context::ResolutionContext,
    enum_impl_as_utf8_file_content, enum_impl_check, enum_impl_template,
    providers::{
        self, Provider,
        file::{
            AsUtf8FileContent, InlineFile, NamedFileProvider, RelativeFile, RequiredFile,
            ResolveAndWrite, Source,
        },
    },
    templating::{self, Field, Scalar, Template},
};
use schemars::JsonSchema;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor, value::MapAccessDeserializer},
};
use std::{collections::HashMap, fmt, io, path::Path};
use tracing::{error, trace};

/// The environment variable used to provide the location of the output directory to user specified
/// commands
const OUTDIR: &str = "OUTDIR";
const OUTFILE: &str = "RTF_OUTPUT";
const PROVIDER_DIR: &str = "providers";

/// # Command Section
///
/// Defines an executable command along with environment variables that should be set prior to
/// execution and file providers that should be made available.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct CommandSection {
    /// The command to be run
    pub command: RawCommand,
    /// Environment variables to set
    #[serde(default)]
    pub env_vars: HashMap<String, Field<Scalar>>,
    /// File providers to run and make available prior to execution
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
        ctx: &mut impl ResolutionContext,
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
        let env_vars = self.all_env_vars(out_dir, &out_file, ctx)?;

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
                let file_path = ctx
                    .known_provider_output_path(Provider::Command {
                        name: &spec.name,
                        cmd: &spec.command_provider,
                    })
                    .ok_or(providers::Error::MissingProviderOutput {
                        name: spec.name.clone(),
                    })?;

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

        // It is possible that no output is set in the environment setup script. If the output is
        // blank then default to an empty json object. If there is an error reading the user defined
        // output to the file then we still pass that error to the user. This is a quality of life
        // improvement so if the user does define any output the rtf execution will continue.
        let output = match ctx.read_path_to_string(&out_file) {
            Ok(s) => {
                ctx.remove_file(out_file)?;
                s
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => "{}".to_string(),
            Err(e) => return Err(e.into()),
        };

        Ok(output)
    }

    /// Combine the base environment variables we have with the ones coming from the file providers
    /// we need to run. The `out_dir` argument here needs to match the one used when running
    /// and outputting the content of the file providers.
    pub fn all_env_vars(
        &self,
        out_dir: &Path,
        out_file: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<HashMap<String, String>> {
        let mut vars: HashMap<String, String> = self
            .env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_resolved().to_string()))
            .collect();

        for nfp in self.file_providers.iter() {
            let path = ctx
                .known_provider_output_path(Provider::File { fp: &nfp.provider })
                .ok_or(providers::Error::MissingProviderOutput {
                    name: nfp.name.clone(),
                })?;
            vars.insert(nfp.env_var.clone(), path.to_string_lossy().to_string());
        }

        vars.insert(OUTDIR.to_string(), out_dir.display().to_string());
        vars.insert(OUTFILE.to_string(), out_file.display().to_string());

        Ok(vars)
    }

    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory.
    ///
    /// [0]: crate::providers::file::FileProvider
    pub async fn run_providers(
        &self,
        out_dir: &Path,
        src: &Source,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        let provider_dir = out_dir.join(PROVIDER_DIR);

        if let RawCommand::Spec(spec) = &self.command
            && ctx
                .known_provider_output_path(Provider::Command {
                    name: &spec.name,
                    cmd: &spec.command_provider,
                })
                .is_none()
        {
            trace!(name=%spec.name, "running command provider");
            let file_path = provider_dir.join(&spec.name);
            spec.command_provider
                .resolve_and_write(&file_path, src, ctx)
                .await?;
            ctx.make_executable(&file_path)?;
            ctx.store_provider_output_path(
                Provider::Command {
                    name: &spec.name,
                    cmd: &spec.command_provider,
                },
                file_path,
            );
        }

        for nfp in self.file_providers.iter() {
            if ctx
                .known_provider_output_path(Provider::File { fp: &nfp.provider })
                .is_some()
            {
                continue;
            }

            trace!(name=%nfp.name, "running command provider");
            let file_path = provider_dir.join(&nfp.name);
            nfp.resolve_and_write(&file_path, src, ctx).await?;
            ctx.store_provider_output_path(Provider::File { fp: &nfp.provider }, file_path);
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

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();

        errs.append(self.command.try_template_nested(path, "command", values));

        path.push("env_vars".to_string());

        for (name, f) in self.env_vars.iter_mut() {
            let tail = name.clone();
            errs.append(f.try_template_nested(path, tail, values));
        }

        path.pop();
        path.push("file_providers".to_string());

        for nfp in self.file_providers.iter_mut() {
            let tail = nfp.env_var.clone();
            errs.append(nfp.try_template_nested(path, tail, values));
        }

        errs.into_result(())
    }
}

impl Check for CommandSection {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        errs.append(self.command.try_check_nested(path, "command", src, ctx));

        for nfp in self.file_providers.iter() {
            let tail = nfp.name.clone();
            errs.append(nfp.provider.try_check_nested(path, tail, src, ctx));
        }

        let env_var_names = self
            .env_vars
            .keys()
            .map(|k| k.as_str())
            .chain(self.file_providers.iter().map(|f| f.env_var.as_str()));

        let duplicates = duplicate_keys(env_var_names, |name| name);

        if !duplicates.is_empty() {
            errs.push(
                checks::ErrorKind::DuplicateEnvironmentVariables,
                duplicates.join("\n"),
                path,
            );
        }

        errs.into_result(())
    }
}

/// # Raw Command
#[derive(Debug, Clone, PartialEq, JsonSchema)]
#[serde(untagged)]
pub enum RawCommand {
    /// A named executable and arguments to run
    String(String),
    /// An explicit specification for a command to be run
    Spec(CommandSpec),
}

impl Serialize for RawCommand {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::String(s) => serializer.serialize_str(s),
            Self::Spec(spec) => spec.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for RawCommand {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<RawCommand, D::Error> {
        deserializer.deserialize_any(CommandVisitor)
    }
}

struct CommandVisitor;

impl<'de> Visitor<'de> for CommandVisitor {
    type Value = RawCommand;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string or a valid command spec")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(RawCommand::String(value.to_owned()))
    }

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        // MapAccessDeserializer is a wrapper that turns a MapAccess into a Deserializer, allowing
        // it to be used as the input to CommandSpec's Deserialize implementation.
        // CommandSpec then deserializes itself using the entries from the map visitor.
        let res = CommandSpec::deserialize(MapAccessDeserializer::new(map));
        let spec = match res {
            Ok(data) => data,
            Err(err) => {
                error!("malformed command spec");
                return Err(err);
            }
        };

        Ok(RawCommand::Spec(spec))
    }
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

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        match self {
            RawCommand::String(_s) => Ok(()),
            RawCommand::Spec(spec) => spec.try_template(path, values),
        }
    }
}

impl Check for RawCommand {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        match self {
            RawCommand::String(_s) => Ok(()),
            RawCommand::Spec(spec) => spec.try_check(path, src, ctx),
        }
    }
}

/// # Command Spec
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct CommandSpec {
    /// The name of the command to run
    pub name: String,
    /// A provider to produce the command that should be run
    #[serde(flatten)]
    pub command_provider: CommandProvider,
    /// Arguments to the command
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

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.command_provider.try_template_nested(
            path,
            "command_provider",
            values,
        ));

        for arg in self.args.iter_mut() {
            errs.append(arg.try_template_nested(path, stringify!(arg), values));
        }

        errs.into_result(())
    }
}

impl Check for CommandSpec {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        errs.append(
            self.command_provider
                .try_check_nested(path, "command_provider", src, ctx),
        );

        errs.into_result(())
    }
}

/// # Command Provider
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CommandProvider {
    Inline(InlineFile),
    RelativePath(RelativeFile),
    Required(RequiredFile),
}

// Each time we add a new variant to the CommandProvider enum above we need to remember to add it
// to the macro invocation below in order to update the trait implementations for the enum. (You
// can't really forget to do this as the compiler will complain about missing match arms if you
// do!)
macro_rules! enum_impl_command_provider {
    ($($variant:ident),+) => {
        enum_impl_check!(CommandProvider => $($variant),+);
        enum_impl_template!(CommandProvider => $($variant),+);
        enum_impl_as_utf8_file_content!(CommandProvider => $($variant),+);
    };
}

enum_impl_command_provider!(Inline, RelativePath, Required);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::{Context, PathKind},
        providers::{
            Provider,
            file::{FileProvider, InlineFile},
        },
        txtar_context::NullClient,
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

        let res = section.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected successful check but got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/command/parse-failures")]
    #[test]
    fn parse_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let res: serde_yaml::Result<CommandSection> = serde_yaml::from_str(config);

        assert!(res.is_err(), "expected invalid YAML, got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/command/check-failures")]
    #[test]
    fn check_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "check-errors");

        let section: CommandSection = match serde_yaml::from_str(config) {
            Ok(section) => section,
            Err(e) => panic!("expected a valid CommandSection, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/command/check-failures")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));
        let res = section.try_check(&mut Vec::new(), &src, &ctx);

        assert!(res.is_err(), "expected check failures");
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

        let res = command.try_template(&mut Vec::new(), &values);

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

        let res = command.try_template(&mut Vec::new(), &values);

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
        writes: Mutex<HashMap<String, usize>>,
        fp_output_paths: HashMap<String, PathBuf>,
    }

    impl ResolutionContext for MockCommandContext {
        type PlatformClient = NullClient;
        type GithubClient = NullClient;
        type HttpClient = NullClient;

        fn store_provider_output_path(&mut self, provider: Provider<'_>, path: PathBuf) {
            let key = serde_yaml::to_string(&provider).unwrap();

            self.fp_output_paths.insert(key, path);
        }

        fn known_provider_output_path(&self, provider: Provider<'_>) -> Option<PathBuf> {
            let key = serde_yaml::to_string(&provider).ok()?;

            self.fp_output_paths.get(&key).cloned()
        }

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

            // Record each time we write to support checking that we correctly use the provider
            // cache
            *self
                .writes
                .lock()
                .unwrap()
                .entry(path.as_ref().display().to_string())
                .or_default() += 1;

            Ok(())
        }

        fn path_kind(&self, _path: impl AsRef<Path>) -> crate::context::PathKind {
            PathKind::File
        }

        fn canonicalize_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
            relative_path.as_ref().canonicalize()
        }

        fn make_executable(&self, _path: impl AsRef<Path>) -> io::Result<()> {
            Ok(())
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
            command: RawCommand::Spec(CommandSpec {
                name: "example.sh".to_string(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: "command".to_string(),
                }),
                args: Vec::new(),
            }),
            env_vars: [
                ("FOO", Scalar::from("hello")),
                ("BAR", Scalar::from("world")),
                ("BAZ", Scalar::from(true)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), Field::Resolved(v)))
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

    #[tokio::test]
    async fn all_env_vars_includes_file_providers() {
        let c = test_cmd_section();
        let mut ctx = MockCommandContext::default();
        let dir = PathBuf::from("/example-dir");
        let src = Source::Local {
            abs_path: dir.join("example.yaml"),
        };

        c.run_providers(&dir, &src, &mut ctx)
            .await
            .expect("providers failed to run");

        let env_vars = c
            .all_env_vars(
                &PathBuf::from("/example-dir"),
                &PathBuf::from("/example-dir/RTF_OUTPUT"),
                &ctx,
            )
            .unwrap();

        // Should NOT include the command script, only file providers
        let expected: HashMap<String, String> = [
            ("FOO", "hello"),
            ("BAR", "world"),
            ("BAZ", "true"),
            ("OUTDIR", "/example-dir"),
            ("FP1", "/example-dir/providers/fp1.txt"),
            ("FP2", "/example-dir/providers/fp2.txt"),
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
        let mut ctx = MockCommandContext::default();
        let dir = PathBuf::from("/example-dir");
        let src = Source::Local {
            abs_path: dir.join("example.yaml"),
        };

        let res = c.run_providers(&dir, &src, &mut ctx).await;
        assert!(res.is_ok(), "unexpected error: {res:?}");

        let written_files = ctx.written_files.into_inner().unwrap();
        let expected: HashMap<String, String> = [
            ("/example-dir/providers/example.sh", "command"),
            ("/example-dir/providers/fp1.txt", "foo"),
            ("/example-dir/providers/fp2.txt", "bar"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        assert_eq!(written_files, expected);
    }

    #[tokio::test]
    async fn the_provider_cache_is_used_to_avoid_rerunning_providers() {
        let c = test_cmd_section();
        let mut ctx = MockCommandContext::default();
        let dir = PathBuf::from("/example-dir");
        let src = Source::Local {
            abs_path: dir.join("example.yaml"),
        };

        let writes = ctx.writes.lock().unwrap().clone();
        assert!(
            writes.is_empty(),
            "should start without any recorded writes"
        );

        // Running the providers should result in each provider writing output once
        let expected: HashMap<String, usize> = [
            ("/example-dir/providers/example.sh".to_string(), 1),
            ("/example-dir/providers/fp1.txt".to_string(), 1),
            ("/example-dir/providers/fp2.txt".to_string(), 1),
        ]
        .into_iter()
        .collect();

        let res = c.run_providers(&dir, &src, &mut ctx).await;
        assert!(res.is_ok(), "unexpected error: {res:?}");

        let writes = ctx.writes.lock().unwrap().clone();
        assert_eq!(writes, expected);

        // Running the providers a second time should still succeed and should not result in any
        // further calls to ctx.write
        let res = c.run_providers(&dir, &src, &mut ctx).await;
        assert!(res.is_ok(), "unexpected error: {res:?}");

        let writes = ctx.writes.into_inner().unwrap();
        assert_eq!(writes, expected);
    }

    #[test_case("WRITE_OUTPUT foo", "foo"; "with output")]
    #[test_case("command-with-no-output", "{}"; "without output")]
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
