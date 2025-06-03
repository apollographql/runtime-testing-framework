//! The various user facing commands exposed through the CLI.
//!
//! We follow the [git CLI design][0] of splitting user facing commands into low level "plumbing"
//! and high level "porcelain" categories.
//!
//! [0]: https://git-scm.com/docs
use anyhow::bail;
use rtf_config::context::{Context, PathKind, ResolutionContext};
use std::{env::current_dir, path::PathBuf};
use tracing::info;

pub mod plumbing;
pub mod porcelain;

fn get_context_and_outdir(
    config_file_path: &str,
    out_dir: &str,
) -> anyhow::Result<(Context, PathBuf)> {
    info!("setting up context and output directory");
    let full_path = PathBuf::from(config_file_path).canonicalize()?;
    let config_dir = full_path.parent().unwrap().to_path_buf();
    let ctx = Context::new(&config_dir);

    // output directories are created relative to the directory we were run from
    let out_dir = current_dir()?.join(out_dir);
    match ctx.path_kind(&out_dir) {
        PathKind::File => bail!("{} is not a directory", out_dir.display()),
        PathKind::OccupiedDir => {
            bail!("{} already exists and is non-empty", out_dir.display())
        }
        _ => (),
    }

    Ok((ctx, out_dir))
}
