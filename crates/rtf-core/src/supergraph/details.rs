//! Helpers for fetching the details for a given supergraph from the platform API.
use crate::platform_query::{self, PlatformQuery};
use anyhow::{Context, bail}; // TODO: replace with thiserror
use graphql_client::GraphQLQuery;
use std::{fs, path::Path};
use tracing::{debug, error, info};

/// An error encountered while attempting to fetch details for a supergraph from the platform API.
#[derive(Debug, thiserror::Error)]
#[error("unable to fetch details for {graph_id}@{variant}: {cause}")]
pub struct FetchError {
    /// The graph ID being fetched
    pub graph_id: String,
    /// The graph variant being fetched
    pub variant: String,
    /// The error encountered
    pub cause: FetchErrorCause,
}

/// Causes of errors when fetching supergraph details from studio.
/// Should always be wrapped in a [FetchError] providing the graph ID and variant that the error is
/// associated with.
#[derive(Debug, thiserror::Error)]
pub enum FetchErrorCause {
    /// An empty string was returned for the supergraph SDL
    #[error("empty supergraph schema returned")]
    EmptySchema,

    /// A graphQL error was encountered while attempting to pull supergraph details
    #[error(transparent)]
    Graphql(#[from] platform_query::Error),

    /// There was no build information for the latest launch
    #[error("no build found as part of the latest launch found")]
    NoBuild,

    /// There was no latest launch
    #[error("no latest launch found")]
    NoLaunch,

    /// No subgraphs were found
    #[error("no subgraphs found")]
    NoSubgraphs,

    /// The requested operation could not be found in studio
    #[error("not a known operation in studio")]
    UnknownOperation,

    /// The requested supergraph could not be found in Studio
    #[error("not a known supergraph in studio")]
    UnknownSupergraph,

    /// The requested supergraph variant could not be found in Studio
    #[error("not a known variant in studio")]
    UnknownVariant,
}

/// Subgraph schema details for a single named subgraph
#[derive(Debug, PartialEq, Eq)]
pub struct Subgraph {
    /// The name of this subgraph
    pub name: String,
    /// The full SDL format schema for this subgraph
    pub sdl: String,
}

/// Supergraph schema details
///
/// To pull supergraph details from studio see [SupergraphDetails::fetch].
#[derive(Debug, PartialEq, Eq)]
pub struct SupergraphDetails {
    /// The GraphOS graphID for this supergraph
    pub graph_id: String,
    /// The GraphOS variant for this supergraph
    pub variant: String,
    /// The full SDL format schema for this supergraph
    pub supergraph_sdl: String,
    /// The list of subgraphs associated with this supergraph
    pub subgraphs: Vec<Subgraph>,
}

impl SupergraphDetails {
    /// Attempt to fetch the data we need for writing out the supergraph and subgraph schemas
    /// associated with a particular graph ID and variant.
    ///
    /// # Errors
    /// This method will error if the underlying graphQL request fails or if the data returned is
    /// insufficient for us to construct [SupergraphDetails]. Failure modes are enumerated and
    /// documented as part of [FetchErrorCause].
    pub async fn fetch(
        graph_id: String,
        variant: String,
        api_key: &str,
        staging: bool,
    ) -> Result<Self, FetchError> {
        info!(%graph_id, %variant, "pulling supergraph details");
        let graph_ref = raw_supergraph_details::Variables {
            graph_id: graph_id.clone(),
            variant: variant.clone(),
        };

        RawSupergraphDetails::fetch(graph_ref, api_key, staging)
            .await
            .map_err(|cause| FetchError {
                graph_id,
                variant,
                cause,
            })
    }

    /// Write out only the schemas held in this [SupergraphDetails].
    ///
    /// See [Self::write_schemas_and_test_config] for also updating and writing out a [TestSpec].
    pub fn write_schemas(&self, out_dir: &Path) -> anyhow::Result<()> {
        debug!("writing supergraph SDL");
        fs::write(out_dir.join("supergraph.graphql"), &self.supergraph_sdl)?;

        debug!("writing supergraph SDLs");
        let sg_dir = out_dir.join("subgraphs");
        if sg_dir.exists() && !sg_dir.is_dir() {
            bail!("{} is not a directory", sg_dir.display());
        } else if !sg_dir.exists() {
            debug!("  creating subgraph directory: {}", sg_dir.display());
            fs::create_dir(&sg_dir).context("unable to create subgraph directory")?;
        }

        for sg in self.subgraphs.iter() {
            debug!("  writing subgraph: {}", sg.name);
            fs::write(sg_dir.join(format!("{}.graphql", sg.name)), &sg.sdl)
                .context("failed to write out subgraph schema")?;
        }

        Ok(())
    }
}

// This is just us defining a type alias for our derived code to use as a custom graphQL scalar
// See https://github.com/graphql-rust/graphql-client?tab=readme-ov-file#custom-scalars
type GraphQLDocument = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "resources/engine-prod-schema.graphql",
    query_path = "resources/queries/supergraph-details.graphql",
    response_derives = "Deserialize",
    variables_derives = "Clone"
)]
struct RawSupergraphDetails;

impl PlatformQuery for RawSupergraphDetails {
    type Output = SupergraphDetails;
    type Error = FetchErrorCause;

    fn try_parse(
        data: Self::ResponseData,
        raw_supergraph_details::Variables { graph_id, variant }: raw_supergraph_details::Variables,
    ) -> Result<SupergraphDetails, FetchErrorCause> {
        use raw_supergraph_details::RawSupergraphDetailsServiceVariantLatestApprovedLaunchBuildResult as BuildRes;

        let graph_variant = data
            .service
            .ok_or(FetchErrorCause::UnknownSupergraph)?
            .variant
            .ok_or(FetchErrorCause::UnknownVariant)?;

        let build_result = graph_variant
            .latest_approved_launch
            .ok_or(FetchErrorCause::NoLaunch)?
            .build
            .and_then(|build| build.result)
            .ok_or(FetchErrorCause::NoBuild)?;

        let supergraph_sdl = match build_result {
            BuildRes::BuildFailure => return Err(FetchErrorCause::NoBuild),
            BuildRes::BuildSuccess(res) => res.core_schema.core_document,
        };

        if supergraph_sdl.is_empty() {
            return Err(FetchErrorCause::EmptySchema);
        }

        let subgraphs: Vec<_> = match graph_variant.source_variant {
            Some(v) => {
                // TODO: have a better log message here (this is just lifted from fetchsup)
                info!("This variant is a contract, using fallback subgraph location");
                v.subgraphs
                    .ok_or(FetchErrorCause::NoSubgraphs)?
                    .into_iter()
                    .map(|raw| Subgraph {
                        name: raw.name,
                        sdl: raw.active_partial_schema.sdl,
                    })
                    .collect()
            }
            None => graph_variant
                .subgraphs
                .ok_or(FetchErrorCause::NoSubgraphs)?
                .into_iter()
                .map(|raw| Subgraph {
                    name: raw.name,
                    sdl: raw.active_partial_schema.sdl,
                })
                .collect(),
        };

        if subgraphs.is_empty() {
            return Err(FetchErrorCause::NoSubgraphs);
        }

        Ok(SupergraphDetails {
            graph_id: graph_id.to_string(),
            variant: variant.to_string(),
            supergraph_sdl,
            subgraphs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use serde::Deserialize;
    use simple_test_case::dir_cases;

    #[dir_cases("crates/rtf-core/resources/test_data/supergraph_details/valid")]
    #[test]
    fn parse_ok(path: &str, contents: &str) -> anyhow::Result<()> {
        let raw: <RawSupergraphDetails as GraphQLQuery>::ResponseData =
            serde_json::from_str(contents)
                .context(format!("{path} contains malformed test data"))?;

        let details = RawSupergraphDetails::try_parse(
            raw,
            raw_supergraph_details::Variables {
                graph_id: "foo".to_string(),
                variant: "bar".to_string(),
            },
        )?;

        let expected = SupergraphDetails {
            graph_id: "foo".to_string(),
            variant: "bar".to_string(),
            supergraph_sdl: "SDL".to_string(),
            subgraphs: vec![
                Subgraph {
                    name: "sg1".to_string(),
                    sdl: "sg1-SDL".to_string(),
                },
                Subgraph {
                    name: "sg2".to_string(),
                    sdl: "sg2-SDL".to_string(),
                },
            ],
        };

        assert_eq!(details, expected);

        Ok(())
    }

    #[derive(Deserialize)]
    struct ErrDetailsCase {
        expected_error: String,
        data: serde_json::Value,
    }

    #[dir_cases("crates/rtf-core/resources/test_data/supergraph_details/invalid")]
    #[test]
    fn parse_err(path: &str, contents: &str) -> anyhow::Result<()> {
        let ErrDetailsCase {
            expected_error,
            data,
        } = serde_json::from_str(contents)
            .context(format!("{path} contains malformed test data"))?;

        let raw: <RawSupergraphDetails as GraphQLQuery>::ResponseData =
            serde_json::from_value(data).context(format!("{path} contains malformed test data"))?;

        let res = RawSupergraphDetails::try_parse(
            raw,
            raw_supergraph_details::Variables {
                graph_id: "foo".to_string(),
                variant: "bar".to_string(),
            },
        );

        let cause = match res {
            Ok(_) => panic!("expected error but details were valid"),
            Err(cause) => cause,
        };

        match (expected_error.as_str(), cause) {
            ("EmptySchema", FetchErrorCause::EmptySchema) => (),
            ("NoBuild", FetchErrorCause::NoBuild) => (),
            ("NoLaunch", FetchErrorCause::NoLaunch) => (),
            ("NoSubgraphs", FetchErrorCause::NoSubgraphs) => (),
            ("UnknownSupergraph", FetchErrorCause::UnknownSupergraph) => (),
            ("UnknownVariant", FetchErrorCause::UnknownVariant) => (),
            (err, cause) => panic!("expected {err}, got {cause:?}"),
        }

        Ok(())
    }
}
