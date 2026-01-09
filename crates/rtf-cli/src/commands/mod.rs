//! The various user facing commands exposed through the CLI.
//!
//! We follow the [git CLI design][0] of splitting user facing commands into low level "plumbing"
//! and high level "porcelain" categories.
//!
//! [0]: https://git-scm.com/docs
use anyhow::{anyhow, bail};
use rtf_config::{
    context::{Context, PathKind, ResolutionContext},
    formats::{self, TestPlanConfig},
};
use std::{
    collections::HashMap,
    env::{self, current_dir},
    path::PathBuf,
};

pub mod plumbing;
pub mod porcelain;

pub(crate) fn get_context() -> Context {
    let env_vars: HashMap<String, String> = env::vars_os()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_string(),
                v.to_string_lossy().to_string(),
            )
        })
        .collect();

    Context::new_from_env_vars(env_vars)
}

pub(crate) fn get_context_and_check_outdir(out_dir: &str) -> anyhow::Result<(Context, PathBuf)> {
    let ctx = get_context();

    let out_dir = current_dir()?.join(out_dir);
    match ctx.path_kind(&out_dir) {
        PathKind::File => bail!("{} is not a directory", out_dir.display()),
        PathKind::OccupiedDir => {
            bail!("{} already exists and is non-empty", out_dir.display())
        }
        _ => Ok((ctx, out_dir)),
    }
}

/// Handles loading a local test plan and displaying user facing errors
async fn load_and_resolve_test_plan_from_local(
    path: &str,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<TestPlanConfig> {
    match TestPlanConfig::try_load_and_resolve_from_path(path, ctx).await {
        Ok(test_plan) => Ok(test_plan),
        Err(e) => match e {
            formats::Error::Io(e) => bail!("Unable to load test plan from {path}: {e}"),
            formats::Error::Yaml(e) => bail!("Unable to parse test plan yaml: {e}"),
            _ => bail!("Unable to load and resolve test plan: {e}"),
        },
    }
}

/// Handles loading a test plan from github and displaying user facing errors
async fn load_and_resolve_test_plan_from_github(
    test_plan_path: &str,
    git_ref: Option<String>,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<TestPlanConfig> {
    let (org, repo_and_path) = test_plan_path.split_once('/').ok_or(anyhow!(
        "invalid GitHub uri: \"{test_plan_path}\" - GitHub uri must be in format ORG/REPO/PATH"
    ))?;
    let (repo, path) = repo_and_path.split_once('/').ok_or(anyhow!(
        "invalid GitHub uri: \"{test_plan_path}\" - GitHub uri must be in format ORG/REPO/PATH"
    ))?;

    match TestPlanConfig::try_load_and_resolve_from_github(org, repo, path, git_ref, ctx).await {
        Ok(test_plan) => Ok(test_plan),
        Err(e) => bail!("Unable to load and resolve test plan from GitHub: {e}"),
    }
}
