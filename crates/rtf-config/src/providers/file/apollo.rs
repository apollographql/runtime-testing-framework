// Unit tests for the parsing and validation of the providers in this file
// are part of the suite of tests in the mod.rs file
use crate::{
    context::ResolutionContext,
    providers::{
        self,
        file::{AsUtf8FileContent, Source},
    },
    templating::{self, Field, Scalar, Template},
    validation::{self, Validate},
};
use rtf_core::supergraph::details::SupergraphDetails;
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
        let client = match ctx.platform_client() {
            Some(client) => client,
            None => panic!(
                "Should not be trying to fetch the supergraph file from GraphOS without a defined platform client"
            ),
        };
        let supergraph = SupergraphDetails::fetch(
            self.graph_id.as_resolved().clone(),
            self.variant.as_resolved().clone(),
            client,
        )
        .await?;

        Ok(supergraph.supergraph_sdl)
    }
}

impl Template for GraphosSupergraph {
    fn has_pending_fields(&self) -> bool {
        self.graph_id.has_pending_fields() || self.variant.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        let mut vals = self.graph_id.required_values();
        vals.extend(self.variant.required_values());

        vals
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(
            self.graph_id.try_resolve_nested(path, "graph_id", values),
        );
        errs.append(self.variant.try_resolve_nested(path, "variant", values));

        errs.into_result(())
    }
}

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
