//! The various user facing commands exposed through the CLI.
//!
//! We follow the [git CLI design][0] of splitting user facing commands into low level "plumbing"
//! and high level "porcelain" categories.
//!
//! [0]: https://git-scm.com/docs
use anyhow::bail;
use rtf_config::context::{Context, PathKind, ResolutionContext};
use std::{
    collections::HashMap,
    env::{self, current_dir},
    path::PathBuf,
};
use tracing::info;

pub mod plumbing;
pub mod porcelain;

fn get_context_and_outdir(out_dir: &str) -> anyhow::Result<(Context, PathBuf)> {
    let env_vars: HashMap<String, String> = env::vars_os()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_string(),
                v.to_string_lossy().to_string(),
            )
        })
        .collect();

    info!("setting up context and output directory");
    let ctx = Context::new_from_env_vars(env_vars);

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
