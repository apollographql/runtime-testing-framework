//! Helpers for fetching the details for a given supergraph from the platform API.
use crate::graphos::{
    self,
    platform_query::{self, PlatformQuery},
};
use apollo_compiler::schema::ObjectType;
use apollo_compiler::{Name, Node, Schema, ast::Value, schema::ExtendedType};
use graphql_client::GraphQLQuery;
use std::{collections::HashMap, fs, io, path::Path};
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

    /// Offline licenses are not enabled for this organisation
    #[error("offline licenses are not enabled for this organisation")]
    OfflineLicenseNotEnabled,

    /// The requested operation could not be found in studio
    #[error("not a known operation in studio")]
    UnknownOperation,

    /// The requested organisation could not be found in studio
    #[error("not a known organisation in studio")]
    UnknownOrganisation,

    /// The requested supergraph could not be found in Studio
    #[error("not a known supergraph in studio")]
    UnknownSupergraph,

    /// The requested supergraph variant could not be found in Studio
    #[error("not a known variant in studio")]
    UnknownVariant,
}

/// Subgraph schema details for a single named subgraph
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subgraph {
    /// The name of this subgraph
    pub name: String,
    /// The full SDL format schema for this subgraph
    pub sdl: String,
}

/// Supergraph schema details
///
/// To pull supergraph details from studio see [SupergraphDetails::fetch].
#[derive(Debug, Clone, PartialEq, Eq)]
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
        graph_id: impl Into<String>,
        variant: impl Into<String>,
        client: &impl platform_query::Client,
    ) -> Result<Self, FetchError> {
        let graph_id = graph_id.into();
        let variant = variant.into();

        info!(%graph_id, %variant, "pulling supergraph details");
        let graph_ref = raw_supergraph_details::Variables {
            graph_id: graph_id.clone(),
            variant: variant.clone(),
        };

        RawSupergraphDetails::fetch(graph_ref, client)
            .await
            .map_err(|cause| FetchError {
                graph_id,
                variant,
                cause,
            })
    }

    /// Attempt to rewrite the subgraph url directives in this schema to use the provided urls
    /// instead.
    pub fn rewrite_subgraph_urls(
        &mut self,
        subgraph_urls: &HashMap<String, String>,
    ) -> Result<(), &'static str> {
        match rewrite_subgraph_urls(&self.supergraph_sdl, subgraph_urls) {
            Some(new_sdl) => {
                self.supergraph_sdl = new_sdl;
                Ok(())
            }

            None => {
                error!("Unable to rewrite subgraph URLs");
                Err("Unable to rewrite subgraph URLs")
            }
        }
    }

    /// Attempt to rewrite the connector url directives in this schema to use the provided urls
    /// instead.
    pub fn rewrite_connector_urls(&mut self) -> Result<(), &'static str> {
        match rewrite_connector_urls(&self.supergraph_sdl) {
            Some(new_sdl) => {
                self.supergraph_sdl = new_sdl;
                Ok(())
            }

            None => {
                error!("Unable to rewrite connector URLs");
                Err("Unable to rewrite connector URLs")
            }
        }
    }

    /// Write out only the schemas held in this [SupergraphDetails].
    pub fn write_schemas(&self, out_dir: &Path) -> graphos::Result<()> {
        debug!("writing supergraph SDL");
        fs::write(out_dir.join("supergraph.graphql"), &self.supergraph_sdl)?;

        debug!("writing supergraph SDLs");
        let sg_dir = out_dir.join("subgraphs");
        if sg_dir.exists() && !sg_dir.is_dir() {
            return Err(graphos::Error::Io(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{} is not a directory", sg_dir.display()),
            )));
        } else if !sg_dir.exists() {
            debug!("  creating subgraph directory: {}", sg_dir.display());
            fs::create_dir(&sg_dir)?;
        }

        for sg in self.subgraphs.iter() {
            debug!("  writing subgraph: {}", sg.name);
            fs::write(sg_dir.join(format!("{}.graphql", sg.name)), &sg.sdl)?;
        }

        Ok(())
    }
}

// This is just us defining a type alias for our derived code to use as a custom graphQL scalar
// See <https://github.com/graphql-rust/graphql-client?tab=readme-ov-file#custom-scalars>
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
                info!("this variant is a contract; using fallback subgraph location");
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

// FIXME: this needs actual logging and testing!
/// Rewrite the given supergraph SDL to set the provided subgraph URLs in place of what is
/// currently there.
fn rewrite_subgraph_urls(sdl: &str, subgraph_urls: &HashMap<String, String>) -> Option<String> {
    let mut schema = Schema::parse(sdl, "supergraph.graphql").unwrap();
    let join_graph_enum = match schema.types.get_mut("join__Graph")? {
        ExtendedType::Enum(e) => e,
        _ => return None,
    };

    for value_def in join_graph_enum.get_mut()?.values.values_mut() {
        for directive in value_def.get_mut()?.directives.0.iter_mut() {
            if directive.name.as_str() != "join__graph" {
                continue;
            }

            let sg_name = directive.specified_argument_by_name("name")?;
            let url = subgraph_urls.get(sg_name.as_str()?)?;
            *directive.get_mut()?.specified_argument_by_name_mut("url")? =
                Node::new(Value::String(url.clone()));
        }
    }

    Some(schema.to_string())
}

// FIXME: this needs actual logging and testing!
/// Rewrite the given supergraph SDL to set the provided connector URLs in place of what is
/// currently there.
fn rewrite_connector_urls(sdl: &str) -> Option<String> {
    let mut schema = Schema::parse(sdl, "supergraph.graphql").unwrap();

    for schema_type in vec!["Query", "Mutation"] {
        if let Some(ExtendedType::Object(extended_type)) = schema.types.get_mut(schema_type) {
            println!(
                "Attempting to replace connectors urls in {} type",
                schema_type
            );
            replace_type_field_url(extended_type, vec!["GET", "POST"])?;
        };
    }

    for directive in schema.schema_definition.get_mut()?.directives.iter_mut() {
        if directive.name != "join__directive" {
            continue;
        }
        if directive.specified_argument_by_name("name")?.as_str()? != "source" {
            continue;
        }

        let Value::Object(args_map) = directive
            .get_mut()?
            .specified_argument_by_name_mut("args")?
            .get_mut()?
        else {
            continue;
        };

        rewrite_url(args_map, &vec!["baseURL"])?;
    }

    Some(schema.to_string())
}

fn rewrite_url(args_map: &mut Vec<(Name, Node<Value>)>, url_keys: &Vec<&str>) -> Option<()> {
    if let Some((_, http_node)) = args_map.iter_mut().find(|(key, _)| key.as_str() == "http") {
        if let Value::Object(http_map) = http_node.get_mut()? {
            if let Some((_, base_url_node)) = http_map
                .iter_mut()
                .find(|(key, _)| url_keys.contains(&key.as_str()))
            {
                *base_url_node = Node::new(Value::String("www.test.com".to_string()));
            }
        }
    }
    Some(())
}

fn replace_type_field_url(query: &mut Node<ObjectType>, url_keys: Vec<&str>) -> Option<()> {
    for (_, field_definition) in &mut query.get_mut()?.fields {
        for directive in field_definition.get_mut()?.directives.iter_mut() {
            if directive.name != "join__directive" {
                continue;
            }
            if directive.specified_argument_by_name("name")?.as_str()? != "connect" {
                continue;
            }
            let Value::Object(args_map) = directive
                .get_mut()?
                .specified_argument_by_name_mut("args")?
                .get_mut()?
            else {
                continue;
            };

            rewrite_url(args_map, &url_keys)?;
        }

        continue;
    }
    Some(())
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

    #[test]
    fn rewriting_subgraph_urls_works() {
        let sdl = include_str!("../../../resources/test_data/simple-supergraph.graphql");
        let subgraph_urls: HashMap<String, String> = [
            ("accounts".into(), "accounts_url".into()),
            ("inventory".into(), "inventory_url".into()),
            ("products".into(), "products_url".into()),
            ("reviews".into(), "reviews_url".into()),
        ]
        .into_iter()
        .collect();

        let s = rewrite_subgraph_urls(sdl, &subgraph_urls).unwrap();

        assert!(s.contains(r#"ACCOUNTS @join__graph(name: "accounts", url: "accounts_url")"#));
        assert!(s.contains(r#"INVENTORY @join__graph(name: "inventory", url: "inventory_url")"#));
        assert!(s.contains(r#"PRODUCTS @join__graph(name: "products", url: "products_url")"#));
        assert!(s.contains(r#"REVIEWS @join__graph(name: "reviews", url: "reviews_url")"#));
    }

    #[test]
    fn rewriting_connector_urls_works() {
        let sdl = include_str!("../../../resources/test_data/connectors/connectors.graphql");
        let s = rewrite_connector_urls(sdl).unwrap();

        assert!(s.contains(r#"{name: "ecomm", http: {baseURL: "www.test.com", headers: []}})"#));
    }

    #[test]
    fn rewriting_sourceless_connector_urls_works() {
        let sdl =
            include_str!("../../../resources/test_data/connectors/sourceless-connectors.graphql");
        let s = rewrite_connector_urls(sdl).unwrap();

        assert!(s.contains(r#"[Product] @join__directive(graphs: [PRODUCTS], name: "connect", args: {http: {GET: "www.test.com"}, selection: "$.products {\nid\nname\ndescription\n}"})"#));
    }
}
