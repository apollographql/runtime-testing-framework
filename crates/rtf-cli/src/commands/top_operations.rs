use anyhow::{Context, bail};
use rtf_core::supergraph::{
    details::SupergraphDetails, operations::top_studio_operations::generate_canned_ops,
};
use std::{env, fs, path::PathBuf};
use tracing::{debug, info};

/// Generate canned operation data using the top n operations from studio for a given graph
pub async fn top_operations(
    graph_id: String,
    variant: String,
    n: usize,
    skip_mutations: bool,
    out_dir: String,
    staging: bool,
) -> anyhow::Result<()> {
    let key = env::var("APOLLO_KEY").context("APOLLO_KEY environment variable is required")?;
    debug!("checking output directory");
    let base = PathBuf::from(out_dir);
    if base.exists() {
        bail!("output dir ({}) already exists: exiting", base.display());
    }

    let details = SupergraphDetails::fetch(graph_id, variant, &key, staging).await?;
    let ops = generate_canned_ops(&details, n, skip_mutations, &key, staging).await?;

    info!("writing out canned queries");
    fs::create_dir(&base)?;

    for op in ops.into_iter() {
        op.write(&base)?;
    }

    Ok(())
}
