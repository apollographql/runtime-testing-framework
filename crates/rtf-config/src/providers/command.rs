use crate::{
    checks::{self, Check, duplicate_keys},
    context::ResolutionContext,
    enum_impl_as_utf8_file_content, enum_impl_check,
    providers::{
        self, Provider,
        file::{
            AsUtf8FileContent, InlineFile, NamedFileProvider, RelativeFile, RequiredFile,
            ResolveAndWrite,
        },
    },
    templating::{Field, Scalar},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};
use tracing::trace;

/// The environment variable used to provide the location of the output directory to user specified
/// commands
pub const OUTDIR: &str = "OUTDIR";
pub const OUTPUT_PATH: &str = "RTF_OUTPUT";
pub const PROVIDER_DIR: &str = "providers";

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
    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory before executing the command with the specified
    /// environment, returning the the output path passed to the command.
    ///
    /// [0]: crate::providers::file::FileProvider
    pub async fn run_providers_and_execute(
        &self,
        out_dir: &Path,
        output_path: PathBuf,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<PathBuf> {
        self.run_providers(&out_dir.join(PROVIDER_DIR), ctx).await?;
        if let Err(e) = self.execute(out_dir, &output_path, ctx) {
            return Err(providers::Error::CommandFailed {
                name: self.command.name.to_string(),
                err: e.to_string(),
            });
        };

        Ok(output_path)
    }

    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory before executing the command with the specified
    /// environment, returning the the content written to [OUTPUT_PATH] and removing the output
    /// file if it was created.
    ///
    /// This method assumes that the command being executed is writing a single file out at the
    /// provided output path rather than a directory of files. To run a command and leave the
    /// resources available on disk after execution, use [run_providers_and_execute][1] instead.
    ///
    /// [0]: crate::providers::file::FileProvider
    /// [1]: CommandSection::run_providers_and_execute
    pub async fn run_providers_and_execute_for_output(
        &self,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        let output_path = self
            .run_providers_and_execute(out_dir, out_dir.join(OUTPUT_PATH), ctx)
            .await?;

        try_read_output_and_remove(&output_path, ctx)
    }

    /// Execute this command with the specified environment, returning the output path used.
    ///
    /// [CommandSection::run_providers] must have been run successfully before calling this method
    /// in order to ensure that all file providers have written out their file content to the
    /// expected location.
    fn execute(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        let env_vars = self.all_env_vars(out_dir, output_path, ctx)?;

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
        let it = self
            .command
            .args
            .iter()
            .map(|arg| arg.as_resolved().as_str());
        ctx.run_command_blocking(prog, it, &env_vars)?;

        Ok(())
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
        vars.insert(OUTPUT_PATH.to_string(), output_path.display().to_string());

        Ok(vars)
    }

    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory.
    ///
    /// [0]: crate::providers::file::FileProvider
    pub async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        if ctx
            .known_provider_output_path(Provider::Command {
                name: &self.command.name,
                cmd: &self.command.command_provider,
            })
            .is_none()
        {
            trace!(name=%self.command.name, "running command provider");
            let file_path = providers_dir.join(&self.command.name);
            self.command
                .command_provider
                .resolve_and_write(&file_path, ctx)
                .await?;
            ctx.make_executable(&file_path)?;
            ctx.store_provider_output_path(
                Provider::Command {
                    name: &self.command.name,
                    cmd: &self.command.command_provider,
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
            let file_path = providers_dir.join(&nfp.name);

            // We need to box the future here in order to prevent us ending up with a recursive
            // type definition for the Future we are building with this method. We end up being
            // recursively defined because of the FromCommand file provider which is just a wrapper
            // around this struct, meaning that the call to resolve_and_write below ends up calling
            // back into run_providers_and_execute which then calls this method (run_providers).
            match Box::pin(nfp.resolve_and_write(&file_path, ctx)).await {
                Ok(b) => b,
                Err(e) => {
                    return Err(providers::Error::ResolveAndWriteFailed {
                        name: nfp.env_var.to_string(),
                        err: e.to_string(),
                    });
                }
            };

            ctx.store_provider_output_path(Provider::File { fp: &nfp.provider }, file_path);
        }

        Ok(())
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

        errs.into_result(())
    }
}

/// It is possible that no output is set in the environment setup script. If the output is
/// blank then default to an empty json object. If there is an error reading the user defined
/// output to the file then we still pass that error to the user. This is a quality of life
/// improvement so if the user does not define any output the rtf execution will continue.
fn try_read_output_and_remove(
    output_path: &Path,
    ctx: &impl ResolutionContext,
) -> providers::Result<String> {
    let output = match ctx.read_path_to_string(output_path) {
        Ok(s) => {
            ctx.remove_file(output_path)?;
            s
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => "{}".to_string(),
        Err(e) => return Err(e.into()),
    };

    Ok(output)
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
        enum_impl_as_utf8_file_content!(CommandProvider => $($variant),+);
    };
}

enum_impl_command_provider!(Inline, RelativePath, Required);

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
        context::{Context, PathKind},
        mock_context::NullClient,
        providers::{
            Provider,
            command::test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            file::{FileProvider, InlineFile},
        },
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

        fn set_current_dir(&mut self, _path: impl AsRef<Path>) -> io::Result<()> {
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
            .run_providers_and_execute_for_output(&dir, &mut ctx)
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
}
