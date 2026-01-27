use crate::{
    checks::{self, Check, duplicate_keys},
    context::ResolutionContext,
    enum_impl_check, inlining,
    providers::{
        self, Provider,
        file::{
            AsUtf8FileContent, InlineFile, NamedFileProvider, RelativeFile, RequiredFile,
            ResolveAndWrite,
        },
    },
    run::{Execute, OUTDIR, OUTPUT_PATH, RunProviders},
    templating::{Field, Scalar},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, future::Future, io, path::Path, pin::Pin};
use tracing::trace;

/// # Command Section
///
/// Defines an executable command along with environment variables that should be set prior to
/// execution and file providers that should be made available.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct CommandSection {
    /// The command to be run
    pub command: CommandSpec,
    /// Environment variables to set
    #[serde(default)]
    pub env_vars: HashMap<String, Field<Scalar>>,
    /// File providers to run and make available prior to execution
    #[serde(default)]
    pub file_providers: Vec<NamedFileProvider>,
}

impl CommandSection {
    fn execute_docker(
        &self,
        docker_command: &DockerCommand,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        let tp_args = self
            .command
            .args
            .iter()
            .map(|arg| arg.as_resolved().as_str());

        let env_vars = self.all_env_vars(out_dir, output_path, ctx)?;
        let (cmd, args) = docker_command.as_command_and_args(tp_args, &env_vars, ctx);

        ctx.run_command_blocking(cmd, args.iter().map(|s| s.as_str()), &HashMap::default())
            .map_err(Into::into)
    }

    fn execute_script(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        let env_vars = self.all_env_vars(out_dir, output_path, ctx)?;
        let args = self
            .command
            .args
            .iter()
            .map(|arg| arg.as_resolved().as_str());

        let file_path = ctx
            .known_provider_output_path(Provider::Command {
                name: &self.command.name,
                cmd: &self.command.command_provider,
            })
            .ok_or(providers::Error::MissingProviderOutput {
                name: self.command.name.clone(),
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

        ctx.run_command_blocking(prog, args, &env_vars)
            .map_err(Into::into)
    }

    fn explicit_env_vars(&self) -> HashMap<String, String> {
        self.env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_resolved().to_string()))
            .collect()
    }

    fn file_path_env_vars(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<HashMap<String, String>> {
        let mut vars = HashMap::with_capacity(self.file_providers.len() + 2);

        for nfp in self.file_providers.iter() {
            let path = ctx
                .known_provider_output_path(Provider::File { fp: &nfp.provider })
                .ok_or(providers::Error::MissingProviderOutput {
                    name: nfp.name.clone(),
                })?;
            vars.insert(nfp.env_var.clone(), path.to_string_lossy().to_string());
        }

        vars.insert(OUTDIR.to_string(), out_dir.display().to_string());
        vars.insert(OUTPUT_PATH.to_string(), output_path.display().to_string());

        Ok(vars)
    }

    /// Combine the base environment variables we have with the ones coming from the file providers
    /// we need to run. The `out_dir` argument here needs to match the one used when running
    /// and outputting the content of the file providers.
    pub fn all_env_vars(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<HashMap<String, String>> {
        let mut vars = self.explicit_env_vars();
        vars.extend(
            self.command
                .command_provider
                .map_file_path_env_vars(self.file_path_env_vars(out_dir, output_path, ctx)?, ctx),
        );

        Ok(vars)
    }

    /// Create an empty [CommandSection] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> CommandSection {
        CommandSection {
            command: CommandSpec {
                name: Default::default(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: "content".to_string(),
                }),
                args: Default::default(),
            },
            env_vars: Default::default(),
            file_providers: Default::default(),
        }
    }
}

impl RunProviders for CommandSection {
    async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        self.file_providers
            .run_providers(providers_dir, ctx)
            .await?;

        let cache_key = Provider::Command {
            name: &self.command.name,
            cmd: &self.command.command_provider,
        };

        if ctx.known_provider_output_path(cache_key).is_none() {
            self.command
                .try_resolve_as_script(providers_dir, ctx)
                .await?;
        }

        Ok(())
    }

    fn inline<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>> {
        Box::pin(async move {
            let mut errs = inlining::ErrorBuilder::new();

            errs.append(self.command.command_provider.inline(ctx).await);
            errs.append(self.file_providers.inline(ctx).await);
            errs.into_result(())
        })
    }

    fn inline_all_relative_paths<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>> {
        Box::pin(async move {
            let mut errs = inlining::ErrorBuilder::new();

            errs.append(
                self.command
                    .command_provider
                    .inline_all_relative_paths(ctx)
                    .await,
            );
            errs.append(self.file_providers.inline_all_relative_paths(ctx).await);
            errs.into_result(())
        })
    }
}

impl Execute for CommandSection {
    fn command_name(&self) -> &str {
        &self.command.name
    }

    fn execute(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        if let CommandProvider::Docker(inner) = &self.command.command_provider {
            self.execute_docker(inner, out_dir, output_path, ctx)
        } else {
            self.execute_script(out_dir, output_path, ctx)
        }
    }
}

impl Check for CommandSection {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        errs.append(self.command.try_check_nested(path, "command", ctx));

        for nfp in self.file_providers.iter() {
            errs.append(nfp.try_check(path, ctx));
        }

        // We are checking whether the env vars in the command are duplicates of any env vars
        // defined in the file providers. Each individual list has been checks for duplicates
        // by this point.
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

        let duplicates = duplicate_keys(
            self.file_providers.iter().map(|f| f.name.as_str()),
            |name| name,
        );
        if !duplicates.is_empty() {
            errs.push(
                checks::ErrorKind::DuplicateFileProviderNames,
                duplicates.join("\n"),
                path,
            );
        }

        errs.into_result(())
    }
}

/// # Command Spec
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct CommandSpec {
    /// The name of the command to run
    #[template(skip)]
    pub name: String,
    /// A provider to produce the command that should be run
    #[serde(flatten)]
    pub command_provider: CommandProvider,
    /// Arguments to the command
    #[serde(default)]
    pub args: Vec<Field<String>>,
}

impl CommandSpec {
    async fn try_resolve_as_script(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        trace!(name=%self.name, "running command provider");
        let file_path = providers_dir.join(&self.name);

        match &self.command_provider {
            CommandProvider::Inline(inner) => inner.resolve_and_write(&file_path, ctx).await?,
            CommandProvider::RelativePath(inner) => {
                inner.resolve_and_write(&file_path, ctx).await?
            }

            // This will panic but we want to ensure that we have a consistent error message
            // for the user so we call through to resolve_and_write to trigger it rather than
            // writing a custom panic message here.
            CommandProvider::Required(inner) => inner.resolve_and_write(&file_path, ctx).await?,

            // In the case of a Docker provider we don't rely on writing out the command itself as
            // a script at all. Instead we construct the required docker command directly as part
            // of execution.
            CommandProvider::Docker(_) => return Ok(()),
        }

        ctx.make_executable(&file_path)?;
        ctx.store_provider_output_path(
            Provider::Command {
                name: &self.name,
                cmd: &self.command_provider,
            },
            file_path,
        );

        Ok(())
    }
}

impl Check for CommandSpec {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        errs.append(
            self.command_provider
                .try_check_nested(path, "command_provider", ctx),
        );

        errs.into_result(())
    }
}

/// # Command Provider
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CommandProvider {
    Docker(DockerCommand),
    Inline(InlineFile),
    RelativePath(RelativeFile),
    Required(RequiredFile),
}

impl CommandProvider {
    pub(crate) async fn inline(&mut self, ctx: &impl ResolutionContext) -> inlining::Result<()> {
        match self {
            CommandProvider::Docker(_) | CommandProvider::Inline(_) => Ok(()),
            CommandProvider::RelativePath(inner) => {
                *self = CommandProvider::Inline(inner.try_into_inline_file(ctx).await?);

                Ok(())
            }
            CommandProvider::Required(inner) => {
                *self = CommandProvider::Inline(inner.try_into_inline_file(ctx).await?);

                Ok(())
            }
        }
    }

    pub(crate) async fn inline_all_relative_paths(
        &mut self,
        ctx: &impl ResolutionContext,
    ) -> inlining::Result<()> {
        if let Self::RelativePath(relative_path) = self {
            *self = Self::Inline(relative_path.try_into_inline_file(ctx).await?);
        }

        Ok(())
    }

    /// For variants other than Docker this is a no-op. Otherwise we update the prefix for all
    /// paths to match the standard mount point we use when running under docker.
    fn map_file_path_env_vars(
        &self,
        mut env_vars: HashMap<String, String>,
        ctx: &impl ResolutionContext,
    ) -> HashMap<String, String> {
        if !matches!(self, Self::Docker(_)) {
            return env_vars;
        }

        let out_dir = ctx.output_path().to_string_lossy();
        for path in env_vars.values_mut() {
            let in_container_path = match path.strip_prefix(out_dir.as_ref()) {
                Some(tail) => format!("/output{tail}"),
                None => {
                    panic!("provider path found that is outside of the output directory: {path}")
                }
            };

            *path = in_container_path;
        }

        env_vars
    }
}

// Each time we add a new variant to the CommandProvider enum above we need to remember to add it
// to the macro invocation below in order to update the trait implementations for the enum. (You
// can't really forget to do this as the compiler will complain about missing match arms if you
// do!)
enum_impl_check!(CommandProvider => Docker, Inline, RelativePath, Required);

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct DockerCommand {
    image: Field<String>,
    tag: Option<Field<String>>,
    command: Field<String>,
}

impl DockerCommand {
    fn as_command_and_args<'a>(
        &self,
        tp_args: impl IntoIterator<Item = &'a str>,
        env_vars: &HashMap<String, String>,
        ctx: &impl ResolutionContext,
    ) -> (&'static str, Vec<String>) {
        let image = match self.tag.as_ref() {
            Some(tag) => format!("{}:{}", self.image.as_resolved(), tag.as_resolved()),
            None => self.image.as_resolved().to_string(),
        };

        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-v".to_string(),
            format!("{}:/output", ctx.output_path().display()),
        ];

        for (k, v) in env_vars.iter() {
            args.extend(["-e".to_string(), format!("{k}={v}")]);
        }

        args.extend([
            image,
            "sh".to_string(),
            "-c".to_string(),
            self.command.as_resolved().to_string(),
        ]);
        args.extend(tp_args.into_iter().map(String::from));

        ("docker", args)
    }
}

impl Check for DockerCommand {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        // We deliberately _don't_ check for the presence of docker on the PATH as part of static
        // checks as we can't guarantee that checks are being run on the same system that will
        // ultimately execute the test plan.
        // This means that docker _not_ being on the PATH will result in a runtime error during
        // command execution, which matches the behaviour of missing dependencies in other
        // CommandProivider variants where we don't even know what dependencies the script has.

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;

    /// Create a Command Section with an inline file
    pub(crate) fn cmd_with_inline_file() -> CommandSection {
        CommandSection {
            command: CommandSpec {
                name: "name".to_string(),
                args: Vec::new(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: "some content".to_string(),
                }),
            },
            ..CommandSection::empty()
        }
    }

    /// Create a Command Section with a required file
    pub(crate) fn cmd_with_required_file() -> CommandSection {
        CommandSection {
            command: CommandSpec {
                name: "name".to_string(),
                args: Vec::new(),
                command_provider: CommandProvider::Required(RequiredFile {
                    message: "this will error".to_string(),
                }),
            },
            ..CommandSection::empty()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        SourceDir,
        context::{Context, PathKind},
        mock_context::NullClient,
        providers::{
            Provider,
            command::test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            file::{FileProvider, InlineFile},
            test_helpers::create_temp_dir_with_file,
        },
        run::{Execute, PROVIDER_DIR},
        templating::Template,
    };
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::{path::PathBuf, sync::Mutex};

    // Sample command yaml
    const FULL_INLINE: &str = indoc!(
        r#"
        command: 
          name: inline-command.sh
          kind: inline
          content: |
            #!/usr/bin/env sh
            echo "Hello, world!"
          args:
            - "{{ arg1 }}"
            - "{{ arg2 }}"
        env_vars:
          ENV_VAR_1: "{{ env_var_1 }}"
          ENV_VAR_2: "{{ env_var_2 }}"
        file_providers:
          - name: inline.txt
            env_var: INLINE
            kind: inline
            content: |
              some content
    "#
    );
    const PARTIAL_RELATIVE_PATH: &str = indoc!(
        r#"
        command: 
          name: relative-path.sh
          kind: relative_path
          path: "{{ path }}"
    "#
    );
    const REQUIRED: &str = indoc!(
        r#"
        command: 
          name: required.sh
          kind: required
          message: file is required
    "#
    );

    #[test_case(FULL_INLINE, &["arg1", "arg2", "env_var_1", "env_var_2"]; "full_inline")]
    #[test_case(PARTIAL_RELATIVE_PATH, &["path"]; "partial_relative_path")]
    #[test_case(REQUIRED, &[]; "required")]
    #[test]
    fn command_parse_and_template(content: &str, expected_variables: &[&str]) {
        let config: CommandSection = serde_yaml::from_str(content).unwrap();

        let mut res = config.required_variables();
        res.sort(); // Sorting so variables are in a determistic order for the assert_eq
        assert_eq!(res, expected_variables, "expected variables to match")
    }

    #[test]
    fn command_spec_check_success() {
        let command = CommandSpec {
            name: "name".to_string(),
            args: Vec::new(),
            command_provider: CommandProvider::Inline(InlineFile {
                content: "some content".to_string(),
            }),
        };

        let ctx = Context::new();
        let res = command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn command_spec_check_command_provider_errors() {
        let command = CommandSpec {
            name: "name".to_string(),
            args: Vec::new(),
            command_provider: CommandProvider::Required(RequiredFile {
                message: "this will error".to_string(),
            }),
        };

        let ctx = Context::new();
        let res = command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(
            err.kind,
            checks::ErrorKind::RequiredFileMissing,
            "check the error kind is correct"
        );
    }

    #[test]
    fn command_section_check_success() {
        let command = cmd_with_inline_file();

        let ctx = Context::new();
        let res = command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn command_section_check_command_errors() {
        let command = cmd_with_required_file();

        let ctx = Context::new();
        let res = command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(
            err.kind,
            checks::ErrorKind::RequiredFileMissing,
            "check the error kind is correct"
        );
    }

    #[test]
    fn command_section_check_file_provider_errors() {
        let command = CommandSection {
            file_providers: vec![NamedFileProvider {
                name: "required".to_string(),
                env_var: "REQUIRED".to_string(),
                provider: FileProvider::Required(RequiredFile {
                    message: "this will error".to_string(),
                }),
            }],
            ..CommandSection::empty()
        };

        let ctx = Context::new();

        let res = command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(
            err.kind,
            checks::ErrorKind::RequiredFileMissing,
            "check the error kind is correct"
        );
    }

    #[test]
    fn command_section_check_duplicate_file_provider_name_errors() {
        let command = CommandSection {
            file_providers: vec![
                NamedFileProvider {
                    name: "same-name.txt".to_string(),
                    env_var: "A".to_string(),
                    provider: FileProvider::Inline(InlineFile {
                        content: "first".to_string(),
                    }),
                },
                NamedFileProvider {
                    name: "same-name.txt".to_string(),
                    env_var: "B".to_string(),
                    provider: FileProvider::Inline(InlineFile {
                        content: "second".to_string(),
                    }),
                },
            ],
            ..CommandSection::empty()
        };

        let ctx = Context::new();
        let res = command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.kind, checks::ErrorKind::DuplicateFileProviderNames);
    }

    #[test]
    fn command_section_check_duplicate_env_var_errors() {
        let mut env_vars: HashMap<String, Field<Scalar>> = HashMap::new();
        env_vars.insert("A".to_string(), Field::Resolved("A".into()));
        env_vars.insert("B".to_string(), Field::Resolved("B".into()));

        let command = CommandSection {
            env_vars,
            file_providers: vec![NamedFileProvider {
                name: "inline".to_string(),
                env_var: "A".to_string(),
                provider: FileProvider::Inline(InlineFile {
                    content: "some content".to_string(),
                }),
            }],
            ..CommandSection::empty()
        };

        let ctx = Context::new();
        let res = command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(
            err.kind,
            checks::ErrorKind::DuplicateEnvironmentVariables,
            "check the error kind is correct"
        );
    }

    #[test]
    fn command_section_check_combined_errors() {
        let mut env_vars: HashMap<String, Field<Scalar>> = HashMap::new();
        env_vars.insert("A".to_string(), Field::Resolved("A".into()));
        env_vars.insert("B".to_string(), Field::Resolved("B".into()));

        let command = CommandSection {
            env_vars,
            file_providers: vec![
                NamedFileProvider {
                    name: "inline".to_string(),
                    env_var: "A".to_string(),
                    provider: FileProvider::Inline(InlineFile {
                        content: "some content".to_string(),
                    }),
                },
                NamedFileProvider {
                    name: "required".to_string(),
                    env_var: "REQUIRED".to_string(),
                    provider: FileProvider::Required(RequiredFile {
                        message: "this will error".to_string(),
                    }),
                },
            ],
            ..cmd_with_required_file()
        };

        let ctx = Context::new();
        let res = command.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err();
        let err_kinds: Vec<checks::ErrorKind> = err.iter().map(|e| e.kind).collect();
        let expected_kinds = vec![
            checks::ErrorKind::RequiredFileMissing,
            checks::ErrorKind::RequiredFileMissing,
            checks::ErrorKind::DuplicateEnvironmentVariables,
        ];
        assert_eq!(
            err_kinds, expected_kinds,
            "check the error kinds are correct"
        );
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

        fn http_client(&self) -> &Self::HttpClient {
            &NullClient
        }

        fn set_output_path(&mut self, _path: impl Into<PathBuf>) {}

        fn output_path(&self) -> &Path {
            Path::new("")
        }

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
            _prog: &str,
            args: impl IntoIterator<Item = &'a str>,
            env_vars: &HashMap<String, String>,
        ) -> io::Result<()> {
            let path = env_vars
                .get(OUTPUT_PATH)
                .expect("RTF_OUTPUT env var not set");
            let mut iter = args.into_iter();
            if let Some(content) = iter.next() {
                self.written_files
                    .lock()
                    .unwrap()
                    .insert(path.to_string(), content.to_string());
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

        fn create_dir_all(&self, _path: impl AsRef<Path>) -> io::Result<()> {
            Ok(())
        }
    }

    fn test_cmd_section() -> CommandSection {
        CommandSection {
            command: CommandSpec {
                name: "example.sh".to_string(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: "command".to_string(),
                }),
                args: Vec::new(),
            },
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
    async fn command_section_all_env_vars_includes_file_providers() {
        let c = test_cmd_section();
        let mut ctx = MockCommandContext::default();
        let dir = PathBuf::from("/example-dir");

        c.run_providers(&dir.join(PROVIDER_DIR), &mut ctx)
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
    async fn command_section_run_providers_writes_expected_files() {
        let c = test_cmd_section();
        let mut ctx = MockCommandContext::default();
        let dir = PathBuf::from("/example-dir");

        let res = c.run_providers(&dir.join(PROVIDER_DIR), &mut ctx).await;
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
    async fn command_section_provider_cache_used_to_avoid_rerunning() {
        let c = test_cmd_section();
        let mut ctx = MockCommandContext::default();
        let dir = PathBuf::from("/example-dir");

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

        let res = c.run_providers(&dir.join(PROVIDER_DIR), &mut ctx).await;
        assert!(res.is_ok(), "unexpected error: {res:?}");

        let writes = ctx.writes.lock().unwrap().clone();
        assert_eq!(writes, expected);

        // Running the providers a second time should still succeed and should not result in any
        // further calls to ctx.write
        let res = c.run_providers(&dir.join(PROVIDER_DIR), &mut ctx).await;
        assert!(res.is_ok(), "unexpected error: {res:?}");

        let writes = ctx.writes.into_inner().unwrap();
        assert_eq!(writes, expected);
    }

    #[test_case(vec![(Field::Resolved("foo".to_string()))], "foo"; "with output")]
    #[test_case(Vec::new(), "{}"; "without output")]
    #[tokio::test]
    async fn run_providers_and_execute_returns_expected_output(
        args: Vec<Field<String>>,
        expected_output: &str,
    ) {
        let c = CommandSection {
            // See the implementation of MockCommandContext::run_command_blocking
            command: CommandSpec {
                name: "example.sh".to_string(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: "command".to_string(),
                }),
                args,
            },
            env_vars: HashMap::new(),
            file_providers: Vec::new(),
        };

        let mut ctx = MockCommandContext::default();
        let dir = PathBuf::from("/example-dir");

        let output = c
            .run_providers_and_execute_for_output("test", &dir, &mut ctx)
            .await
            .expect("command to succeed");

        assert_eq!(output, expected_output, "unexpected output");

        let written_files = ctx.written_files.into_inner().unwrap();
        let k = dir.join(OUTPUT_PATH).display().to_string();

        assert!(
            !written_files.contains_key(&k),
            "should have removed the outfile"
        );
    }

    #[tokio::test]
    async fn command_provider_inline_all_relative_paths_succeeds() {
        let ctx = Context::new();
        let file_content = "example file content";
        let relative_file_path = "file.txt";
        let (temp, _file_to_read) = create_temp_dir_with_file(relative_file_path, file_content);
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());

        let mut command_provider = CommandProvider::RelativePath(RelativeFile {
            path: Field::Resolved(relative_file_path.to_string()),
            src: Some(src),
        });

        let result = command_provider.inline_all_relative_paths(&ctx).await;
        assert!(
            result.is_ok(),
            "Expected inline_all_relative_paths to succeed, got {result:?}"
        );
        assert_eq!(
            command_provider,
            CommandProvider::Inline(InlineFile {
                content: file_content.to_string(),
            })
        );
    }

    #[tokio::test]
    async fn command_provider_inline_all_relative_paths_succeeds_and_leaves_inline_command_provider_unchanged()
     {
        let ctx = Context::new();
        let file_content = "example file content";
        let mut command_provider = CommandProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });
        let expected_command_provider = command_provider.clone();

        let result = command_provider.inline_all_relative_paths(&ctx).await;
        assert!(
            result.is_ok(),
            "Expected inline_all_relative_paths to succeed, got {result:?}"
        );
        assert_eq!(command_provider, expected_command_provider);
    }

    #[tokio::test]
    async fn command_section_inline_succeeds() {
        let ctx = Context::new();
        let file_content = "example file content";
        let relative_file_path = "file.txt";
        let (temp, _file_to_read) = create_temp_dir_with_file(relative_file_path, file_content);
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());

        let inline_file_provider = FileProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });

        let mut command_section = CommandSection {
            command: CommandSpec {
                name: "example.sh".to_string(),
                command_provider: CommandProvider::RelativePath(RelativeFile {
                    path: Field::Resolved(relative_file_path.to_string()),
                    src: Some(src.clone()),
                }),
                args: Vec::new(),
            },
            env_vars: HashMap::new(),
            file_providers: vec![
                NamedFileProvider {
                    name: "fp1.txt".to_string(),
                    env_var: "FP1".to_string(),
                    provider: FileProvider::RelativePath(RelativeFile {
                        path: Field::Resolved(relative_file_path.to_string()),
                        src: Some(src.clone()),
                    }),
                },
                NamedFileProvider {
                    name: "fp2.txt".to_string(),
                    env_var: "FP2".to_string(),
                    provider: inline_file_provider.clone(),
                },
            ],
        };

        let result = command_section.inline(&ctx).await;
        assert!(
            result.is_ok(),
            "Expected inline_all_relative_paths to succeed, got {result:?}"
        );

        let expected = CommandSection {
            command: CommandSpec {
                name: "example.sh".to_string(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: file_content.to_string(),
                }),
                args: Vec::new(),
            },
            env_vars: HashMap::new(),
            file_providers: vec![
                NamedFileProvider {
                    name: "fp1.txt".to_string(),
                    env_var: "FP1".to_string(),
                    provider: inline_file_provider.clone(),
                },
                NamedFileProvider {
                    name: "fp2.txt".to_string(),
                    env_var: "FP2".to_string(),
                    provider: inline_file_provider,
                },
            ],
        };

        assert_eq!(command_section, expected);
    }

    #[tokio::test]
    async fn command_section_inline_all_relative_providers_succeeds() {
        let ctx = Context::new();
        let file_content = "example file content";
        let relative_file_path = "file.txt";
        let (temp, _file_to_read) = create_temp_dir_with_file(relative_file_path, file_content);
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());

        let inline_file_provider = FileProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });

        let mut command_section = CommandSection {
            command: CommandSpec {
                name: "example.sh".to_string(),
                command_provider: CommandProvider::RelativePath(RelativeFile {
                    path: Field::Resolved(relative_file_path.to_string()),
                    src: Some(src.clone()),
                }),
                args: Vec::new(),
            },
            env_vars: HashMap::new(),
            file_providers: vec![
                NamedFileProvider {
                    name: "fp1.txt".to_string(),
                    env_var: "FP1".to_string(),
                    provider: FileProvider::RelativePath(RelativeFile {
                        path: Field::Resolved(relative_file_path.to_string()),
                        src: Some(src.clone()),
                    }),
                },
                NamedFileProvider {
                    name: "fp2.txt".to_string(),
                    env_var: "FP2".to_string(),
                    provider: inline_file_provider.clone(),
                },
            ],
        };

        let result = command_section.inline_all_relative_paths(&ctx).await;
        assert!(
            result.is_ok(),
            "Expected inline_all_relative_paths to succeed, got {result:?}"
        );

        let expected = CommandSection {
            command: CommandSpec {
                name: "example.sh".to_string(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: file_content.to_string(),
                }),
                args: Vec::new(),
            },
            env_vars: HashMap::new(),
            file_providers: vec![
                NamedFileProvider {
                    name: "fp1.txt".to_string(),
                    env_var: "FP1".to_string(),
                    provider: inline_file_provider.clone(),
                },
                NamedFileProvider {
                    name: "fp2.txt".to_string(),
                    env_var: "FP2".to_string(),
                    provider: inline_file_provider,
                },
            ],
        };

        assert_eq!(command_section, expected);
    }

    #[test_case(CommandProvider::Inline(InlineFile { content: String::new() }); "inline")]
    #[test_case(CommandProvider::RelativePath(RelativeFile { path: Field::Resolved(String::new()), src: None }); "relative path")]
    #[test]
    fn map_file_path_env_vars_is_noop_for_non_docker(cp: CommandProvider) {
        let env_vars: HashMap<String, String> = [
            ("FOO", "/output/providers/foo.txt"),
            ("BAR", "/output/providers/a/bar.txt"),
            ("BAZ", "/output/providers/b/c/baz.txt"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        let mapped = cp.map_file_path_env_vars(env_vars.clone(), &Context::new());

        assert_eq!(mapped, env_vars);
    }

    #[test]
    fn map_file_path_env_vars_sets_correct_paths_for_docker() {
        let output_path = "/home/bob/x/y/z/output";

        let original_env_vars: HashMap<String, String> = [
            ("FOO", "providers/foo.txt"),
            ("BAR", "providers/a/bar.txt"),
            ("BAZ", "providers/b/c/baz.txt"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), format!("{output_path}/{v}")))
        .collect();

        let mut ctx = Context::new();
        ctx.set_output_path(output_path);

        let cp = CommandProvider::Docker(DockerCommand {
            image: Field::Resolved(String::new()),
            tag: None,
            command: Field::Resolved(String::new()),
        });

        let mapped = cp.map_file_path_env_vars(original_env_vars, &ctx);

        let expected: HashMap<String, String> = [
            ("FOO", "/output/providers/foo.txt"),
            ("BAR", "/output/providers/a/bar.txt"),
            ("BAZ", "/output/providers/b/c/baz.txt"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        assert_eq!(mapped, expected);
    }
}
