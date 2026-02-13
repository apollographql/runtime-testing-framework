//! Traits for running RTF test plan sections.
//!
//! Running a given test plan section (scenario, environment) is split into two stages: resolution
//! of file providers and execution of the command itself.
use crate::{
    context::ResolutionContext,
    inlining::{self, InlineMode},
    providers::{
        self, Provider,
        file::{NamedFileProvider, ResolveAndWrite, compose::NamedComposeFileProvider},
    },
};
use std::{
    io,
    path::{Path, PathBuf},
    pin::Pin,
};
use tracing::trace;

/// The environment variable used to provide the location of the output directory to user specified
/// commands
pub const OUTDIR: &str = "OUTDIR";
pub const OUTPUT_PATH: &str = "RTF_OUTPUT";
pub const PROVIDER_DIR: &str = "providers";

/// Well-known run metadata key for the docker compose network name.
/// Stored during environment setup and read during scenario execution.
pub const DOCKER_COMPOSE_NETWORK: &str = "DOCKER_COMPOSE_NETWORK";

#[allow(async_fn_in_trait)]
pub trait RunProviders {
    /// Run all of the [FileProviders][0] contained within this type and write out their file
    /// contents to the specified directory.
    ///
    /// [0]: crate::providers::file::FileProvider
    async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()>;

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
    async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        for nfp in self.iter() {
            let cache_key = Provider::File { fp: &nfp.provider };
            if ctx.known_provider_output_path(cache_key).is_some() {
                continue;
            }

            trace!(name=%nfp.name, "running file provider");
            let file_path = providers_dir.join(&nfp.name);

            // We need to box the future here in order to prevent us ending up with a recursive
            // type definition for the Future we are building with this method. We end up being
            // recursively defined because of the FromCommand file provider which is just a wrapper
            // around CommandSection, meaning that the call to resolve_and_write below ends up calling
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
    async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        for nfp in self.iter() {
            let cache_key = Provider::ComposeFile { fp: &nfp.provider };
            if ctx.known_provider_output_path(cache_key).is_some() {
                continue;
            }

            trace!(name=%nfp.name, "running file provider");
            let file_path = providers_dir.join(&nfp.name);

            // We need to box the future here in order to prevent us ending up with a recursive
            // type definition for the Future we are building with this method. We end up being
            // recursively defined because of the FromCommand file provider which is just a wrapper
            // around CommandSection, meaning that the call to resolve_and_write below ends up calling
            // back into run_providers_and_execute which then calls this method (run_providers).
            match Box::pin(nfp.resolve_and_write(&file_path, ctx)).await {
                Ok(b) => b,
                Err(e) => {
                    return Err(providers::Error::ResolveAndWriteFailed {
                        name: nfp.name.to_string(),
                        err: e.to_string(),
                    });
                }
            };

            ctx.store_provider_output_path(Provider::ComposeFile { fp: &nfp.provider }, file_path);
        }

        Ok(())
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

#[allow(async_fn_in_trait)]
pub trait Execute: RunProviders {
    fn command_name(&self) -> &str;

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
    ) -> providers::Result<()>;

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
