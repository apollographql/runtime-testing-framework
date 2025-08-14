// Unit tests for the parsing and validation of the providers in this file
// are part of the suite of tests in the mod.rs file
use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    impl_template,
    providers::{
        self,
        file::{AsUtf8FileContent, ResolveAndWrite, Source},
    },
    templating::{self, Field, Scalar, Template},
};
use rtf_core::graphos::supergraph::{
    SupergraphDetails,
    operations::{fetch_offline_license, top_studio_operations::generate_canned_ops},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// # GraphOS Supergraph SDL
///
/// The user specifies the ref that should be used to fetch a supergraph SDL
/// file from the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct GraphosSupergraph {
    /// The Apollo graph ref to pull supergraph SDL for.
    pub graph_ref: Field<String>,
}

impl AsUtf8FileContent for GraphosSupergraph {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let (graph_id, variant) = self
            .graph_ref
            .as_resolved()
            .split_once('@')
            .expect("validated graph_ref");

        ctx.with_supergraph_details(graph_id, variant, |details| {
            Ok(details.supergraph_sdl.clone())
        })
        .await
    }
}

impl_template!(GraphosSupergraph => [graph_ref]);

impl Check for GraphosSupergraph {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS subgraph SDL
///
/// The user specifies the graph ref that should be used to fetch a subgraph
/// SDL files from the GraphOS API.
///
/// Note that this file proivider will output a directory of SDL schema files, one for each
/// subgraph.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct GraphosSubgraphs {
    /// The Apollo graph ref to pull subgraph SDL files for.
    pub graph_ref: Field<String>,
}

impl ResolveAndWrite for GraphosSubgraphs {
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<(PathBuf, String)>> {
        let (graph_id, variant) = self
            .graph_ref
            .as_resolved()
            .split_once('@')
            .expect("validated graph_ref");

        let subgraphs = ctx
            .with_supergraph_details(graph_id, variant, |details| Ok(details.subgraphs.clone()))
            .await?;

        let dir = target.as_ref();
        let contents: Vec<_> = subgraphs
            .into_iter()
            .map(|sg| (dir.join(sg.name).with_extension("graphql"), sg.sdl))
            .collect();

        Ok(contents)
    }
}

impl_template!(GraphosSubgraphs => [graph_ref]);

impl Check for GraphosSubgraphs {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS Canned Operations
///
/// The user specifies the graph ref and parameters that should be used to
/// generate canned GraphQL requests based on operations data obtained from
/// the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct GraphosCannedOps {
    /// The Apollo graph ref to pull operations for.
    pub graph_ref: Field<String>,
    /// The number of operations to attempt to fetch.
    ///
    /// Defaults to 20 if unset.
    #[serde(default = "default_top_n")]
    pub top_n: Field<usize>,
    /// Whether or not to include mutations in the returned operations.
    ///
    /// Defaults to false if unset.
    #[serde(default)]
    pub skip_mutations: Field<bool>,
}

fn default_top_n() -> Field<usize> {
    Field::Resolved(20)
}

impl AsUtf8FileContent for GraphosCannedOps {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let (graph_id, variant) = self
            .graph_ref
            .as_resolved()
            .split_once('@')
            .expect("validated graph_ref");

        let details: Arc<SupergraphDetails> = ctx
            .with_supergraph_details(graph_id, variant, |details| Ok(details.clone()))
            .await?;

        let client = ctx.platform_client().expect("to have a platform client");
        let canned_ops = generate_canned_ops(
            &details,
            *self.top_n.as_resolved(),
            *self.skip_mutations.as_resolved(),
            client,
        )
        .await?;

        // Create a json line file for each of the canned operations
        let json_file = canned_ops
            .iter()
            .map(|v| v.to_json_string())
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");

        Ok(json_file)
    }
}

impl_template!(GraphosCannedOps => [graph_ref, top_n, skip_mutations]);

impl Check for GraphosCannedOps {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS Offline License
///
/// The user specifies the graph id that should be used to fetch an offline license from the
/// GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct OfflineGraphosLicense {
    /// The Apollo graph ref to pull an offline license for.
    pub graph_id: Field<String>,
}

impl AsUtf8FileContent for OfflineGraphosLicense {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let client = ctx.platform_client().expect("to have a platform client");
        let license = fetch_offline_license(self.graph_id.as_resolved(), client).await?;

        Ok(license)
    }
}

impl_template!(OfflineGraphosLicense => [graph_id]);

impl Check for OfflineGraphosLicense {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_client(path, ctx)
    }
}

fn validate_graph_ref_and_client(
    graph_ref: &str,
    path: &[String],
    ctx: &impl ResolutionContext,
) -> checks::Result<()> {
    let mut errs = checks::ErrorBuilder::new();
    if !graph_ref.contains('@') {
        errs.push(
            checks::ErrorKind::InvalidGraphRef,
            format!("expected a string of the form 'graph_id@variant', got {graph_ref}"),
            path,
        );
    }
    errs.append(validate_client(path, ctx));

    errs.into_result(())
}

fn validate_client(path: &[String], ctx: &impl ResolutionContext) -> checks::Result<()> {
    if ctx.platform_client().is_none() {
        return Err(checks::Errors::new(
            checks::ErrorKind::MissingGraphOsApiKey,
            "expected os env key APOLLO_KEY",
            path,
        ));
    }

    Ok(())
}
