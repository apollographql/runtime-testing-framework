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

impl Template for GraphosCannedOps {
    fn has_pending_fields(&self) -> bool {
        self.graph_id.has_pending_fields()
            || self.variant.has_pending_fields()
            || self.top_n.has_pending_fields()
            || self.skip_mutations.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        let mut vals = self.graph_id.required_values();
        vals.extend(self.variant.required_values());
        vals.extend(self.top_n.required_values());
        vals.extend(self.skip_mutations.required_values());

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
        errs.append(self.top_n.try_resolve_nested(path, "top_n", values));
        errs.append(
            self.skip_mutations
                .try_resolve_nested(path, "skip_mutations", values),
        );

        errs.into_result(())
    }
}

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

impl Template for OfflineGraphosLicense {
    fn has_pending_fields(&self) -> bool {
        self.graph_id.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        self.graph_id.required_values()
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        self.graph_id.try_resolve_nested(path, "graph_id", values)
    }
}

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
