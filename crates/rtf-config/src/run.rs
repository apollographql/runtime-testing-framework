//! Traits for running RTF test plan sections.
//!
//! Running a given test plan section (scenario, environment) is split into two stages: resolution
//! of file providers and execution of the command itself.
use crate::{
    StableSource,
    context::ResolutionContext,
    inlining::{self, InlineMode},
    providers::{
        self,
        command::CommandProvider,
        file::{
            AsUtf8FileContent, FileProvider, NamedFileProvider, RelativeDir, RelativeFile,
            ResolveAndWrite,
            compose::{ComposeFileProvider, NamedComposeFileProvider},
        },
    },
};
use serde::Serialize;
use std::{
    collections::HashMap,
    fmt, io,
    path::{Path, PathBuf},
    pin::Pin,
};
use tracing::{debug, trace};

/// The environment variable used to provide the location of the output directory to user specified
/// commands
pub const OUTDIR: &str = "OUTDIR";
pub const OUTPUT_PATH: &str = "RTF_OUTPUT";
pub const PROVIDER_DIR: &str = "providers";

/// Well-known run metadata key for the docker compose network name.
/// Stored during environment setup and read during scenario execution.
pub const DOCKER_COMPOSE_NETWORK: &str = "DOCKER_COMPOSE_NETWORK";

/// Wrapper enum for supporting caching of provider output
#[derive(Debug, Copy, Clone, Serialize)]
pub enum Provider<'a> {
    File {
        fp: &'a FileProvider,
    },
    Command {
        name: &'a str,
        cmd: &'a CommandProvider,
    },
    ComposeFile {
        fp: &'a ComposeFileProvider,
    },
}

pub(crate) async fn try_read_relative_file(
    rf: &RelativeFile,
    files: &mut HashMap<(StableSource, String), String>,
    ctx: &impl ResolutionContext,
) -> providers::Result<()> {
    let key = (
        rf.src.clone().expect("no source"),
        rf.path.as_resolved().to_string(),
    );
    if files.contains_key(&key) {
        return Ok(());
    }

    let content = rf.try_get_file_content(ctx).await?;
    files.insert(key, content);

    Ok(())
}

pub(crate) async fn try_read_relative_dir(
    rd: &RelativeDir,
    files: &mut HashMap<(StableSource, String), String>,
    ctx: &impl ResolutionContext,
) -> providers::Result<()> {
    for rf in rd.as_relative_files() {
        try_read_relative_file(&rf, files, ctx).await?;
    }

    Ok(())
}

#[allow(async_fn_in_trait)]
pub(crate) trait ExtractRelativeFiles {
    async fn try_extract_relative_files(
        &self,
        files: &mut HashMap<(StableSource, String), String>,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()>;
}

#[allow(async_fn_in_trait)]
pub trait RunProviders {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)>;

    async fn try_extract_relative_files(
        &self,
        files: &mut HashMap<(StableSource, String), String>,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        for (_, provider) in self.named_providers() {
            match provider {
                Provider::File { fp } => fp.try_extract_relative_files(files, ctx).await?,
                Provider::Command { cmd, .. } => cmd.try_extract_relative_files(files, ctx).await?,
                Provider::ComposeFile { fp } => fp.try_extract_relative_files(files, ctx).await?,
            }
        }

        Ok(())
    }

    /// Run all of the [FileProviders][0] contained within this type and write out their file
    /// contents to the specified directory.
    ///
    /// [0]: crate::providers::file::FileProvider
    async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        for (name, provider) in self.named_providers() {
            if ctx.known_provider_output_path(provider).is_some() {
                continue;
            }

            trace!(%name, "running file provider");
            let file_path = providers_dir.join(name);

            // We need to box the futures here in order to prevent us ending up with a recursive
            // type definition for the Future we are building with this method. We end up being
            // recursively defined because of the FromCommand file provider which is just a wrapper
            // around CommandSection, meaning that the call to resolve_and_write below ends up calling
            // back into run_providers_and_execute which then calls this method (run_providers).
            let res = match provider {
                Provider::File { fp } => Box::pin(fp.resolve_and_write(&file_path, ctx)).await,

                Provider::Command { cmd, .. } => Box::pin(cmd.resolve_and_write(&file_path, ctx))
                    .await
                    .and_then(|_| ctx.make_executable(&file_path).map_err(Into::into)),

                Provider::ComposeFile { fp } => {
                    Box::pin(fp.resolve_and_write(&file_path, ctx)).await
                }
            };

            if let Err(e) = res {
                return Err(providers::Error::ResolveAndWriteFailed {
                    name: name.to_owned(),
                    err: e.to_string(),
                });
            };

            ctx.store_provider_output_path(provider, file_path);
        }

        Ok(())
    }

    // We need to pin these futures on the heap to be able to poll it in order to avoid a
    // recursively defined future (which is infinitely sized). We end up being recursively
    // defined because of the FromCommand file provider which is just a wrapper around the
    // CommandSection struct.

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>>;
}

impl RunProviders for Vec<NamedFileProvider> {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        self.iter()
            .map(|nfp| (nfp.name.as_str(), Provider::File { fp: &nfp.provider }))
            .collect()
    }

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>> {
        Box::pin(async move {
            let mut errs = inlining::ErrorBuilder::new();

            for nfp in self.iter_mut() {
                errs.append(nfp.provider.inline(mode, ctx).await);
            }

            errs.into_result(())
        })
    }
}

impl RunProviders for Vec<NamedComposeFileProvider> {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        self.iter()
            .map(|nfp| {
                (
                    nfp.name.as_str(),
                    Provider::ComposeFile { fp: &nfp.provider },
                )
            })
            .collect()
    }

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>> {
        Box::pin(async move {
            let mut errs = inlining::ErrorBuilder::new();

            for nfp in self.iter_mut() {
                errs.append(nfp.provider.inline(mode, ctx).await);
            }

            errs.into_result(())
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct ExecuteArgs {
    pub prog: String,
    pub args: Vec<String>,
    pub env_vars: HashMap<String, String>,
}

impl fmt::Display for ExecuteArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (k, v) in self.env_vars.iter() {
            // trailing space to pad for next pair / prog
            write!(f, "{k}={v:?} ")?;
        }

        f.write_str(&self.prog)?;

        for arg in self.args.iter() {
            // leading space to pad for prog / prev arg
            write!(f, " {arg:?}")?;
        }

        Ok(())
    }
}

#[allow(async_fn_in_trait)]
pub trait Execute: RunProviders {
    fn command_name(&self) -> &str;

    fn as_execute_args(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<ExecuteArgs>;

    /// Execute this command with the specified environment, returning the output path used.
    ///
    /// [RunProviders::run_providers] must have been run successfully before calling this method
    /// in order to ensure that all file providers have written out their file content to the
    /// expected location.
    fn execute(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        let e_args = self.as_execute_args(out_dir, output_path, ctx)?;

        debug!(command=%e_args, "Executing command");

        ctx.run_command_blocking(
            &e_args.prog,
            e_args.args.iter().map(|s| s.as_str()),
            &e_args.env_vars,
        )
        .map_err(Into::into)
    }

    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory before executing the command with the specified
    /// environment, returning the the output path passed to the command.
    ///
    /// [0]: crate::providers::file::FileProvider
    async fn run_providers_and_execute(
        &self,
        name: &str,
        out_dir: &Path,
        output_path: PathBuf,
        providers_dir: PathBuf,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<PathBuf> {
        self.run_providers(&providers_dir.join(format!("{name}_providers")), ctx)
            .await?;

        if let Err(e) = self.execute(out_dir, &output_path, ctx) {
            return Err(providers::Error::CommandFailed {
                name: self.command_name().to_string(),
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
    /// [1]: Execute::run_providers_and_execute
    async fn run_providers_and_execute_for_output(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        let output_path = self
            .run_providers_and_execute(
                name,
                out_dir,
                out_dir.join(OUTPUT_PATH),
                out_dir.join(PROVIDER_DIR),
                ctx,
            )
            .await?;

        try_read_output_and_remove(&output_path, ctx)
    }
}

/// It is possible that no output is set in the environment setup script. If the output is
/// blank then default to an empty json object. If there is an error reading the user defined
/// output to the file then we still pass that error to the user. This is a quality of life
/// improvement so if the user does not define any output the rtf execution will continue.
pub(crate) fn try_read_output_and_remove(
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
