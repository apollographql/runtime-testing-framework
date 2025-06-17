// Unit tests for the parsing and validation of the providers in this file
// are part of the suite of tests in the mod.rs file
use crate::{
    context::ResolutionContext,
    impl_template,
    providers::{
        self,
        file::{AsUtf8FileContent, ResolveAndWrite, Source},
    },
    templating::{self, Field, Scalar, Template},
    validation::{self, Validate},
};
use rtf_core::graphos::supergraph::{
    SupergraphDetails,
    operations::{fetch_offline_license, top_studio_operations::generate_canned_ops},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The user specifies the ref that should be used to fetch a supergraph SDL
/// file from the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GraphosSupergraph {
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

impl Validate for GraphosSupergraph {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// The user specifies the graph ref that should be used to fetch a subgraph
/// SDL files from the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GraphosSubgraphs {
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

impl Validate for GraphosSubgraphs {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// The user specifies the graph ref and parameters that should be used to
/// generate canned GraphQL requests based on operations data obtained from
/// the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GraphosCannedOps {
    pub graph_ref: Field<String>,
    #[serde(default = "default_top_n")]
    pub top_n: Field<usize>,
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

impl Validate for GraphosCannedOps {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// The user specifies the graph id that should be used to fetch an offline license from the
/// GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct OfflineGraphosLicense {
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

impl Validate for OfflineGraphosLicense {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        validate_client(path, ctx)
    }
}

fn validate_graph_ref_and_client(
    graph_ref: &str,
    path: &[String],
    ctx: &impl ResolutionContext,
) -> validation::Result<()> {
    let mut errs = validation::ErrorBuilder::new();
    if !graph_ref.contains('@') {
        errs.push(
            validation::ErrorKind::InvalidGraphRef,
            "expected a string of the form 'graph_id@variant'",
            path,
        );
    }
    errs.append(validate_client(path, ctx));

    errs.into_result(())
}

fn validate_client(path: &[String], ctx: &impl ResolutionContext) -> validation::Result<()> {
    if ctx.platform_client().is_none() {
        return Err(validation::Errors::new(
            validation::ErrorKind::MissingGraphOsApiKey,
            "",
            path,
        ));
    }

    Ok(())
}
