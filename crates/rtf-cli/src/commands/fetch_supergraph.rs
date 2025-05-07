use anyhow::{Context, bail};
use rtf_core::supergraph::details::SupergraphDetails;
use std::{env, fs, path::PathBuf};
use tracing::debug;

/// Attempt to fetch and parse the details we need for a given supergraph before writing them out
/// to disk in the given directory:
///   - a subdirectory containing all of the subgraph SDLs
///   - the supergraph SDL
pub async fn fetch_supergraph(
    graph_id: String,
    variant: String,
    out_dir: impl Into<PathBuf>,
    staging: bool,
) -> anyhow::Result<()> {
    let key = env::var("APOLLO_KEY").context("APOLLO_KEY environment variable is required")?;

    debug!("checking output directory");
    let base = out_dir.into();
    if base.exists() {
        bail!("output dir ({}) already exists: exiting", base.display());
    }

    let details = SupergraphDetails::fetch(graph_id, variant, &key, staging).await?;

    debug!("creating output directory");
    fs::create_dir(&base).context("unable to create output directory")?;

    details.write_schemas(&base)
}
