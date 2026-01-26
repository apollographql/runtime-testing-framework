//! Traits for running RTF test plan sections.
//!
//! Running a given test plan section (scenario, environment) is split into two stages: resolution
//! of file providers and execution of the command itself.
use crate::{context::ResolutionContext, providers};
use std::{
    io,
    path::{Path, PathBuf},
};

/// The environment variable used to provide the location of the output directory to user specified
/// commands
pub const OUTDIR: &str = "OUTDIR";
pub const OUTPUT_PATH: &str = "RTF_OUTPUT";
pub const PROVIDER_DIR: &str = "providers";

#[allow(async_fn_in_trait)]
pub trait Resolve {
    /// Run all of the [FileProviders][0] associated with this command and write out their file
    /// contents to the specified directory.
    ///
    /// [0]: crate::providers::file::FileProvider
    async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()>;
}

pub trait Execute {
    fn command_name(&self) -> &str;

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
    ) -> providers::Result<()>;
}

#[allow(async_fn_in_trait)]
pub trait ResolveAndExecute: Resolve + Execute {
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
    /// [1]: CommandSection::run_providers_and_execute
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

impl<T> ResolveAndExecute for T where T: Resolve + Execute {}

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
