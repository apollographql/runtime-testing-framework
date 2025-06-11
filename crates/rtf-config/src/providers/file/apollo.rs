// Unit tests for the parsing and validation of the providers in this file
// are part of the suite of tests in the mod.rs file
use crate::{
    context::ResolutionContext,
    impl_template,
    providers::{
        self,
        file::{AsUtf8FileContent, Source},
    },
    templating::{self, Field, Scalar, Template},
    validation::{self, Validate},
};
use rtf_core::graphos::supergraph::{
    SupergraphDetails,
    operations::{fetch_offline_license, top_studio_operations::generate_canned_ops},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The user specifies the graph id and variant that should be used to fetch
/// a supergraph file from the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GraphosSupergraph {
    pub graph_id: Field<String>,
    pub variant: Field<String>,
}

impl AsUtf8FileContent for GraphosSupergraph {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let client = ctx.platform_client().expect("to have a platform client");
        let supergraph = SupergraphDetails::fetch(
            self.graph_id.as_resolved().clone(),
            self.variant.as_resolved().clone(),
            client,
        )
        .await?;

        Ok(supergraph.supergraph_sdl)
    }
}

impl_template!(GraphosSupergraph => [graph_id, variant]);

impl Validate for GraphosSupergraph {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        if ctx.platform_client().is_none() {
            return Err(validation::Errors::new(
                validation::ErrorKind::MissingGraphOsApiKey,
                "",
                path,
            ));
        }

        Ok(())
    }
}

/// The user specifies the graph id and variant that should be used to fetch
/// a supergraph file from the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GraphosCannedOps {
    pub graph_id: Field<String>,
    pub variant: Field<String>,
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
        let client = ctx.platform_client().expect("to have a platform client");
        let details = SupergraphDetails::fetch(
            self.graph_id.as_resolved().clone(),
            self.variant.as_resolved().clone(),
            client,
        )
        .await?;

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

impl_template!(GraphosCannedOps => [graph_id, variant, top_n, skip_mutations]);

impl Validate for GraphosCannedOps {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        if ctx.platform_client().is_none() {
            return Err(validation::Errors::new(
                validation::ErrorKind::MissingGraphOsApiKey,
                "",
                path,
            ));
        }

        Ok(())
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
        if ctx.platform_client().is_none() {
            return Err(validation::Errors::new(
                validation::ErrorKind::MissingGraphOsApiKey,
                "",
                path,
            ));
        }

        Ok(())
    }
}
