//! Helpers for fetching the details for a given supergraph from the platform API.
use crate::graphos::{
    self,
    platform_query::{self, PlatformQuery},
};

use apollo_compiler::{
    Name, Node, Schema, ast,
    ast::{Directive, Value},
    collections::IndexMap,
    schema::{Component, ExtendedType},
};
use graphql_client::GraphQLQuery;
use std::{collections::HashMap, fs, io, path::Path};
use tracing::{debug, error, info, warn};

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
    pub fn rewrite_connector_urls(&mut self, base_url: &str) -> Result<(), &'static str> {
        let mut schema = Schema::parse(&self.supergraph_sdl, "supergraph.graphql")
            .map_err(|_| "Unable to parse supergraph schema")?;

        replace_sourceless_connector_urls(&mut schema, base_url);
        replace_sourced_connector_urls(&mut schema, base_url);

        self.supergraph_sdl = schema.to_string();
        Ok(())
    }

    /// Rewrite this schema so a Router 3 / federation 3 build accepts it.
    ///
    /// Composition published graphs before the September 2025 GraphQL spec tightened two rules
    /// around `@deprecated`, and such a graph is rejected at startup. This applies the same two
    /// fixes as composition's own fed3 compat shim (apollographql/router#10029):
    ///
    /// 1. `@deprecated(reason: null)` loses the `reason` argument, since it is no longer
    ///    nullable. The directive keeps its default reason.
    /// 2. `@deprecated` is removed from a field whose interface declares that field without
    ///    deprecating it, since an implementing field may now only be deprecated alongside it.
    ///
    /// Unlike the shim, (2) is applied only to fields the interface actually declares. The shim
    /// also strips fields the implementing type adds of its own, which the spec rule does not
    /// cover; leaving those alone keeps the schema under test closer to the published one.
    pub fn apply_fed3_compat(&mut self) -> Result<(), &'static str> {
        let mut schema = Schema::parse(&self.supergraph_sdl, "supergraph.graphql")
            .map_err(|_| "Unable to parse supergraph schema")?;

        let interface_fields = collect_interface_fields(&schema);
        strip_null_deprecation_reasons(&mut schema);
        strip_deprecated_implementing_fields(&mut schema, &interface_fields);

        self.supergraph_sdl = schema.to_string();
        Ok(())
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
                info!("variant is a contract; fetching subgraph schemas from source variant");
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

/// Rewrite the given supergraph SDL to set the provided subgraph URLs in place of what is
/// currently there.
fn rewrite_subgraph_urls(sdl: &str, subgraph_urls: &HashMap<String, String>) -> Option<String> {
    let mut schema = Schema::parse(sdl, "supergraph.graphql").ok()?;

    let join_graph_enum = match schema.types.get_mut("join__Graph") {
        Some(ExtendedType::Enum(e)) => e,
        Some(_) => {
            warn!("join__Graph exists but is not an enum type");
            return None;
        }
        None => {
            warn!("supergraph SDL missing join__Graph enum");
            return None;
        }
    };

    let mut rewritten_count = 0;

    for value_def in join_graph_enum.get_mut()?.values.values_mut() {
        for directive in value_def.get_mut()?.directives.0.iter_mut() {
            if directive.name.as_str() != "join__graph" {
                continue;
            }

            let sg_name = directive.specified_argument_by_name("name")?;
            let sg_name_str = sg_name.as_str()?;

            let url = match subgraph_urls.get(sg_name_str) {
                Some(u) => u,
                None => {
                    warn!("URL map missing entry for a subgraph in the schema");
                    return None;
                }
            };

            *directive.get_mut()?.specified_argument_by_name_mut("url")? =
                Node::new(Value::String(url.clone()));

            rewritten_count += 1;
        }
    }

    debug!(
        rewritten = rewritten_count,
        provided = subgraph_urls.len(),
        "rewrote subgraph URLs in supergraph SDL"
    );

    Some(schema.to_string())
}

/// Map each interface in the schema to its field names and whether each is deprecated.
fn collect_interface_fields(schema: &Schema) -> HashMap<Name, HashMap<Name, bool>> {
    schema
        .types
        .values()
        .filter_map(|ty| match ty {
            ExtendedType::Interface(interface) => Some(interface),
            _ => None,
        })
        .map(|interface| {
            let fields = interface
                .fields
                .iter()
                .map(|(name, field)| (name.clone(), has_deprecated(&field.directives)))
                .collect();

            (interface.name.clone(), fields)
        })
        .collect()
}

fn has_deprecated(directives: &ast::DirectiveList) -> bool {
    directives.iter().any(|d| d.name == "deprecated")
}

/// Drop a `reason: null` argument from every `@deprecated` in the list.
fn strip_null_reason(directives: &mut ast::DirectiveList) {
    for directive in directives.iter_mut() {
        if directive.name != "deprecated" {
            continue;
        }

        let Some(directive) = directive.get_mut() else {
            continue;
        };

        directive
            .arguments
            .retain(|arg| !(arg.name == "reason" && matches!(*arg.value, Value::Null)));
    }
}

fn strip_null_deprecation_reasons(schema: &mut Schema) {
    for ty in schema.types.values_mut() {
        match ty {
            ExtendedType::Object(object) => {
                strip_null_reason_from_fields(&mut object.make_mut().fields)
            }
            ExtendedType::Interface(interface) => {
                strip_null_reason_from_fields(&mut interface.make_mut().fields)
            }
            ExtendedType::Enum(enum_type) => {
                for value in enum_type.make_mut().values.values_mut() {
                    strip_null_reason(&mut value.make_mut().directives);
                }
            }
            ExtendedType::InputObject(input) => {
                for field in input.make_mut().fields.values_mut() {
                    strip_null_reason(&mut field.make_mut().directives);
                }
            }
            _ => {}
        }
    }
}

/// `@deprecated` is valid on a field's arguments as well as the field itself.
fn strip_null_reason_from_fields(fields: &mut IndexMap<Name, Component<ast::FieldDefinition>>) {
    for field in fields.values_mut() {
        let field = field.make_mut();
        strip_null_reason(&mut field.directives);

        for arg in field.arguments.iter_mut() {
            strip_null_reason(&mut arg.make_mut().directives);
        }
    }
}

fn strip_deprecated_implementing_fields(
    schema: &mut Schema,
    interface_fields: &HashMap<Name, HashMap<Name, bool>>,
) {
    for ty in schema.types.values_mut() {
        let (implements, fields) = match ty {
            ExtendedType::Object(object) => {
                let object = object.make_mut();
                (&object.implements_interfaces, &mut object.fields)
            }
            ExtendedType::Interface(interface) => {
                let interface = interface.make_mut();
                (&interface.implements_interfaces, &mut interface.fields)
            }
            _ => continue,
        };

        for interface_name in implements {
            let Some(declared) = interface_fields.get(&interface_name.name) else {
                continue;
            };

            for (field_name, field) in fields.iter_mut() {
                // Only a field the interface itself declares without deprecating.
                if declared.get(field_name) == Some(&false) {
                    field
                        .make_mut()
                        .directives
                        .retain(|d| d.name != "deprecated");
                }
            }
        }
    }
}

fn is_join_directive_named(directive: &Node<Directive>, name: &str) -> bool {
    directive.name == "join__directive"
        && directive
            .specified_argument_by_name("name")
            .and_then(|v| v.as_str())
            .is_some_and(|n| n == name)
}

fn get_directive_args_map(directive: &mut Node<Directive>) -> Option<&mut [(Name, Node<Value>)]> {
    match directive
        .get_mut()
        .and_then(|d| d.specified_argument_by_name_mut("args"))
        .and_then(|a| a.get_mut())
    {
        Some(Value::Object(args_map)) => Some(args_map),
        _ => None,
    }
}

fn rewrite_connector_url(args_map: &mut [(Name, Node<Value>)], url_keys: &[&str], url: &str) {
    let http_entry = match args_map.iter_mut().find(|(key, _)| key.as_str() == "http") {
        Some(entry) => entry,
        None => return,
    };

    let http_map = match http_entry.1.get_mut() {
        Some(Value::Object(http_map)) => http_map,
        _ => return,
    };

    let url_node = match http_map
        .iter_mut()
        .find(|(key, _)| url_keys.contains(&key.as_str()))
    {
        Some((_, url_node)) => url_node,
        None => return,
    };

    debug!("rewriting {url_node} connector URL in supergraph SDL to {url}");
    *url_node = Node::new(Value::String(url.to_string()));
}

fn replace_sourced_connector_urls(schema: &mut Schema, base_url: &str) {
    let schema_def = match schema.schema_definition.get_mut() {
        Some(def) => def,
        None => return,
    };

    for directive in schema_def.directives.iter_mut() {
        if !is_join_directive_named(directive, "source") {
            continue;
        }

        let args_map = match get_directive_args_map(directive) {
            Some(args_map) => args_map,
            None => continue,
        };

        rewrite_connector_url(args_map, &["baseURL"], base_url);
    }
}

fn replace_sourceless_connector_urls(schema: &mut Schema, base_url: &str) {
    let http_verbs = ["GET", "POST", "PUT", "PATCH", "DELETE"];

    for schema_type in ["Query", "Mutation"] {
        let extended_type = match schema.types.get_mut(schema_type) {
            Some(ExtendedType::Object(t)) => t,
            _ => continue,
        };

        debug!(schema_type, "replacing connector URLs in type");

        let object_type = match extended_type.get_mut() {
            Some(object_type) => object_type,
            None => continue,
        };

        for (field_name, field_definition) in &mut object_type.fields {
            let field_definition = match field_definition.get_mut() {
                Some(field_definition) => field_definition,
                None => continue,
            };

            for directive in field_definition.directives.iter_mut() {
                if !is_join_directive_named(directive, "connect") {
                    continue;
                }

                let args_map = match get_directive_args_map(directive) {
                    Some(args_map) => args_map,
                    None => continue,
                };

                // Skip connect directives that contain a source, as they are not "sourceless connectors"
                if args_map.iter().any(|(key, _)| key.as_str() == "source") {
                    continue;
                }

                rewrite_connector_url(args_map, &http_verbs, &format!("{base_url}/{field_name}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use serde::Deserialize;
    use simple_test_case::dir_cases;

    #[dir_cases("crates/rtf-integrations/resources/test_data/supergraph_details/valid")]
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

    #[dir_cases("crates/rtf-integrations/resources/test_data/supergraph_details/invalid")]
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
    fn rewrite_subgraph_urls_all_urls_updated() {
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
    fn rewrite_subgraph_urls_invalid_sdl_returns_none() {
        let result = rewrite_subgraph_urls("not valid graphql {{{", &HashMap::new());
        assert!(result.is_none());
    }

    #[test]
    fn rewrite_subgraph_urls_missing_join_graph_returns_none() {
        let sdl = "type Query { hello: String }";
        let result = rewrite_subgraph_urls(sdl, &HashMap::new());
        assert!(result.is_none());
    }

    #[test]
    fn rewrite_subgraph_urls_join_graph_wrong_type_returns_none() {
        // join__Graph exists but as a scalar, not an enum
        let sdl = "scalar join__Graph\ntype Query { hello: String }";
        let result = rewrite_subgraph_urls(sdl, &HashMap::new());
        assert!(result.is_none());
    }

    #[test]
    fn rewrite_subgraph_urls_partial_url_map_returns_none() {
        let sdl = include_str!("../../../resources/test_data/simple-supergraph.graphql");
        let partial_urls: HashMap<String, String> = [
            ("accounts".into(), "http://accounts".into()),
            // missing inventory, products, reviews
        ]
        .into_iter()
        .collect();

        let result = rewrite_subgraph_urls(sdl, &partial_urls);
        assert!(result.is_none());
    }

    #[test]
    fn rewrite_subgraph_urls_extra_urls_ignored() {
        let sdl = include_str!("../../../resources/test_data/simple-supergraph.graphql");
        let subgraph_urls: HashMap<String, String> = [
            ("accounts".into(), "accounts_url".into()),
            ("inventory".into(), "inventory_url".into()),
            ("products".into(), "products_url".into()),
            ("reviews".into(), "reviews_url".into()),
            ("nonexistent".into(), "should_be_ignored".into()),
        ]
        .into_iter()
        .collect();

        let result = rewrite_subgraph_urls(sdl, &subgraph_urls);
        assert!(result.is_some());
    }

    fn fed3_compat_fixture() -> SupergraphDetails {
        SupergraphDetails {
            graph_id: "".to_string(),
            variant: "".to_string(),
            supergraph_sdl: include_str!("../../../resources/test_data/fed3-compat.graphql")
                .to_string(),
            subgraphs: vec![],
        }
    }

    /// The name a `@deprecated` sits on, for every remaining occurrence. Enum values carry no
    /// type, so this takes the leading token rather than splitting on `:`.
    fn fields_still_deprecated(sdl: &str) -> Vec<&str> {
        sdl.lines()
            .filter(|line| line.contains("@deprecated"))
            .filter_map(|line| line.split_whitespace().next())
            .map(|token| token.trim_end_matches(':'))
            .collect()
    }

    #[test]
    fn fed3_compat_strips_deprecated_implementing_a_non_deprecated_interface_field() {
        let mut sd = fed3_compat_fixture();

        sd.apply_fed3_compat().unwrap();

        // Node.id and Timestamped.createdAt are not deprecated, so Product's must not be.
        assert!(!sd.supergraph_sdl.contains("Node.id is not deprecated"));
        assert!(!fields_still_deprecated(&sd.supergraph_sdl).contains(&"createdAt"));
    }

    #[test]
    fn fed3_compat_keeps_deprecated_matching_the_interface() {
        let mut sd = fed3_compat_fixture();

        sd.apply_fed3_compat().unwrap();

        // Node.legacyId is itself deprecated, so the implementing field may stay deprecated.
        assert_eq!(
            sd.supergraph_sdl
                .matches(r#"@deprecated(reason: "use id")"#)
                .count(),
            2,
            "both Node.legacyId and Product.legacyId should keep their deprecation"
        );
    }

    #[test]
    fn fed3_compat_leaves_fields_the_interface_does_not_declare() {
        let mut sd = fed3_compat_fixture();

        sd.apply_fed3_compat().unwrap();

        assert!(sd.supergraph_sdl.contains("not an interface field"));
    }

    #[test]
    fn fed3_compat_applies_to_an_interface_implementing_an_interface() {
        let mut sd = fed3_compat_fixture();

        sd.apply_fed3_compat().unwrap();

        assert!(
            !sd.supergraph_sdl
                .contains("interface implementing an interface")
        );
    }

    #[test]
    fn fed3_compat_strips_null_reasons_but_keeps_the_directive() {
        let mut sd = fed3_compat_fixture();

        sd.apply_fed3_compat().unwrap();

        assert!(
            !sd.supergraph_sdl.contains("reason: null"),
            "reason is non-nullable under the 2025 spec"
        );
        // The enum value, input field and field argument all keep a bare @deprecated.
        for field in ["RETIRED", "legacyTerm", "currency"] {
            assert!(
                fields_still_deprecated(&sd.supergraph_sdl).contains(&field),
                "{field} should keep a bare @deprecated"
            );
        }
    }

    #[test]
    fn fed3_compat_invalid_sdl_is_an_error() {
        let mut sd = fed3_compat_fixture();
        sd.supergraph_sdl = "not valid graphql {{{".to_string();

        assert!(sd.apply_fed3_compat().is_err());
    }

    #[test]
    fn rewriting_connector_urls_works() {
        let sdl = include_str!("../../../resources/test_data/connectors/connectors.graphql");
        let mut sd = SupergraphDetails {
            graph_id: "".to_string(),
            variant: "".to_string(),
            supergraph_sdl: sdl.to_string(),
            subgraphs: vec![],
        };

        sd.rewrite_connector_urls("http://host.docker.internal:3000")
            .unwrap();

        assert!(sd.supergraph_sdl.contains(
            r#"{name: "ecomm", http: {baseURL: "http://host.docker.internal:3000", headers: []}})"#
        ));
    }

    #[test]
    fn rewriting_sourceless_connector_urls_works() {
        let sdl =
            include_str!("../../../resources/test_data/connectors/sourceless-connectors.graphql");
        let mut sd = SupergraphDetails {
            graph_id: "".to_string(),
            variant: "".to_string(),
            supergraph_sdl: sdl.to_string(),
            subgraphs: vec![],
        };

        let _ = sd.rewrite_connector_urls("http://host.docker.internal:3000");

        assert!(sd.supergraph_sdl.contains(r#"[Product] @join__directive(graphs: [PRODUCTS], name: "connect", args: {http: {GET: "http://host.docker.internal:3000/products"}, selection: "$.products {\nid\nname\ndescription\n}"})"#));
    }

    #[test]
    fn rewrite_connector_urls_invalid_sdl_returns_failure() {
        let mut sd = SupergraphDetails {
            graph_id: "".to_string(),
            variant: "".to_string(),
            supergraph_sdl: "not valid graphql {{{".to_string(),
            subgraphs: vec![],
        };
        let result = sd.rewrite_connector_urls("http://localhost:3000");
        assert!(result.is_err());
    }

    #[test]
    fn rewrite_connector_urls_no_connectors_succeeds() {
        let sdl = "type Query { hello: String }";
        let mut sd = SupergraphDetails {
            graph_id: "".to_string(),
            variant: "".to_string(),
            supergraph_sdl: sdl.to_string(),
            subgraphs: vec![],
        };

        let result = sd.rewrite_connector_urls("http://localhost:3000");
        // Should succeed even without connectors - just a no-op
        assert!(result.is_ok());

        // No semantic changes should have been made. Only formatting changes from parsing
        let expected = Schema::parse(sdl, "").unwrap().to_string();
        assert_eq!(sd.supergraph_sdl, expected);
    }
}
