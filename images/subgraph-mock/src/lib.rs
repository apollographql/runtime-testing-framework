use crate::{config::Config, handle::graphql::ResponseGenerationConfig, latency::LatencyGenerator};
use anyhow::Error;
use apollo_compiler::{
    Node, Schema,
    ast::{FieldDefinition, InputValueDefinition, Type},
    collections::IndexSet,
    name,
    schema::{
        Component, ComponentName, ComponentOrigin, DirectiveDefinition, DirectiveLocation,
        ExtendedType, ScalarType, UnionType,
    },
    ty,
    validation::Valid,
};
use hyper::{HeaderMap, header::HeaderValue};
use serde_yaml::Value;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use tracing::{info, warn};

pub mod config;
pub mod handle;
pub mod latency;

static RESPONSE_GENERATION_CONFIG: OnceLock<ResponseGenerationConfig> = OnceLock::new();
static ADDITIONAL_HEADERS: OnceLock<HeaderMap<HeaderValue>> = OnceLock::new();
static LATENCY_GENERATOR: OnceLock<LatencyGenerator> = OnceLock::new();
static SUPERGRAPH_SCHEMA: OnceLock<Valid<Schema>> = OnceLock::new();
static CACHE_RESPONSES: AtomicBool = AtomicBool::new(true);

static SUBGRAPH_CACHE_RESPONSES: OnceLock<HashMap<String, bool>> = OnceLock::new();
static SUBGRAPH_HEADERS: OnceLock<HashMap<String, HeaderMap<HeaderValue>>> = OnceLock::new();
static SUBGRAPH_LATENCY_GENERATORS: OnceLock<HashMap<String, LatencyGenerator>> = OnceLock::new();
static SUBGRAPH_RESPONSE_GENERATION_CONFIGS: OnceLock<HashMap<String, ResponseGenerationConfig>> =
    OnceLock::new();

// Allowed in the YAML, but not represented in the actual Config struct as we
// neither want nor need that data structure to be recursive.
const SUBGRAPH_OVERRIDES_KEY: &str = "subgraph_overrides";

/// A general purpose subgraph mock.
#[derive(Debug, clap::Parser)]
#[clap(about, name = "subgraph-mock", long_about = None)]
pub struct Args {
    /// Path to the config file that should be used to configure the server
    #[arg(short, long)]
    pub config: Option<PathBuf>,

    /// Path to the supergraph SDL that the server should mock
    #[arg(short, long)]
    pub schema: PathBuf,
}

impl Args {
    /// Load and initialise the configuration based on command line args
    pub fn init(self) -> anyhow::Result<u16> {
        let cfg = match self.config {
            Some(path) => {
                info!(path=%path.display(), "loading and parsing config file");
                let mut base: Value = serde_yaml::from_slice(&fs::read(path)?)?;
                let mapping = base
                    .as_mapping_mut()
                    .ok_or_else(|| Error::msg("config file must be a mapping"))?;

                let mut subgraph_cache_responses = HashMap::new();
                let mut subgraph_headers = HashMap::new();
                let mut subgraph_latency_generators = HashMap::new();
                let mut subgraph_response_generation_configs = HashMap::new();

                if let Some(overrides) = mapping.remove(SUBGRAPH_OVERRIDES_KEY) {
                    match overrides {
                        Value::Mapping(mapping) => {
                            for (subgraph_name, subgraph_override) in mapping {
                                let mut subgraph_config = base.clone();

                                let override_mapping =
                                    subgraph_override.as_mapping().ok_or_else(|| {
                                        Error::msg("subgraph override must be a mapping")
                                    })?;

                                if override_mapping.contains_key("port") {
                                    warn!("port overrides for subgraphs will be ignored")
                                }

                                merge_yaml(subgraph_override, &mut subgraph_config);
                                let parsed_config: Config =
                                    serde_yaml::from_value(subgraph_config)?;
                                let subgraph_name: String = serde_yaml::from_value(subgraph_name)?;

                                info!("generating customized config for {}", subgraph_name);
                                let (
                                    _port,
                                    cache_responses,
                                    latency_generator,
                                    headers,
                                    response_generation,
                                ) = parsed_config.into_parts();

                                subgraph_cache_responses
                                    .insert(subgraph_name.clone(), cache_responses);
                                subgraph_latency_generators
                                    .insert(subgraph_name.clone(), latency_generator);
                                subgraph_headers.insert(subgraph_name.clone(), headers);
                                subgraph_response_generation_configs
                                    .insert(subgraph_name, response_generation);
                            }
                        }
                        _ => return Err(Error::msg("config file must be a mapping")),
                    }
                }

                SUBGRAPH_CACHE_RESPONSES
                    .set(subgraph_cache_responses)
                    .unwrap();
                SUBGRAPH_HEADERS.set(subgraph_headers).unwrap();
                SUBGRAPH_LATENCY_GENERATORS
                    .set(subgraph_latency_generators)
                    .unwrap();
                SUBGRAPH_RESPONSE_GENERATION_CONFIGS
                    .set(subgraph_response_generation_configs)
                    .unwrap();

                serde_yaml::from_value(base)?
            }
            None => {
                info!("using default config");
                Config::default()
            }
        };

        info!("parsing default subgraph config");
        let (port, cache_responses, latency_generator, headers, response_generation) =
            cfg.into_parts();

        info!(path=%self.schema.display(), "loading and parsing supergraph schema");
        match Schema::parse(fs::read_to_string(&self.schema)?, self.schema) {
            Ok(mut schema) => {
                patch_supergraph(&mut schema);
                match schema.validate() {
                    Ok(schema) => SUPERGRAPH_SCHEMA.set(schema).unwrap(),
                    Err(e) => panic!(
                        "ERROR: invalid supergraph schema following patching\n{}",
                        e.errors
                    ),
                }
            }

            Err(e) => panic!("ERROR: invalid supergraph schema\n{}", e.errors),
        };

        RESPONSE_GENERATION_CONFIG.set(response_generation).unwrap();
        ADDITIONAL_HEADERS.set(headers).unwrap();
        LATENCY_GENERATOR.set(latency_generator).unwrap();
        CACHE_RESPONSES.store(cache_responses, Ordering::Relaxed);

        Ok(port)
    }
}

/// A function for merging yaml overrides with the base config. This differs slightly from rtf-config in that
/// it does *not* combine arrays, since arrays are effectively scalar values that should be replaced, not merged,
/// in the context of the subgraph config. We may also want to revisit the mapping merge logic if it ends up being
/// unintuitive in the context of configuration such as the latency waveforms.
fn merge_yaml(overrides: serde_yaml::Value, base: &mut serde_yaml::Value) {
    use serde_yaml::Value;

    match (overrides, base) {
        // If both values are mappings we add all keys from src into dst.
        (Value::Mapping(override_map), Value::Mapping(base_map)) => {
            for (key, override_val) in override_map.into_iter() {
                // If a key is present in both maps then we recursively merge the values,
                // otherwise we just insert the src key into dst directly.
                match base_map.get_mut(&key) {
                    Some(base_val) => merge_yaml(override_val, base_val),
                    None => _ = base_map.insert(key, override_val),
                };
            }
        }

        // Otherwise we replace base with overrides
        (overrides, base) => *base = overrides,
    }
}

/// We need to be able to intercept and handle queries for entities:
/// { _entities(representations: [_Any!]!): [_Entity]!
///
/// The router also auto-supports the @defer and @stream directive so schemas may be using them without
/// importing / defining them directly. In that case we need to inject them into the schema in
/// order for the validation of our queries to succeed.
///
/// See https://www.apollographql.com/docs/graphos/routing/operations/defer
///
/// The directive definitions are copied from here:
///   https://github.com/apollographql/router/blob/23e580e22a4401cc2e7a952b241a1ec955b29c99/apollo-federation/src/api_schema.rs#L156https://github.com/apollographql/router/blob/23e580e22a4401cc2e7a952b241a1ec955b29c99/apollo-federation/src/api_schema.rs#L156
fn patch_supergraph(schema: &mut Schema) {
    // Grab _everything_ for our _Entity union. This is a lot more than the true _Entity union for
    // any of the actual subgraphs but it at least means that we can correctly parse the queries
    // coming from the client.
    let members: IndexSet<ComponentName> = schema
        .types
        .iter()
        .filter(|(_, ty)| ty.is_object())
        .map(|(name, _)| ComponentName {
            origin: ComponentOrigin::Definition,
            name: name.clone(),
        })
        .collect();

    // Inject our _Entity union
    schema.types.insert(
        name!("_Entity"),
        ExtendedType::Union(Node::new(UnionType {
            description: None,
            name: name!("_Entity"),
            directives: Default::default(),
            members,
        })),
    );

    // Inject our stub _Any scalar
    schema.types.insert(
        name!("_Any"),
        ExtendedType::Scalar(Node::new(ScalarType {
            description: None,
            name: name!("_Any"),
            directives: Default::default(),
        })),
    );

    // Inject the _entities query itself
    let query_type_name = &schema.schema_definition.query.as_ref().unwrap().name;
    let query_root = match schema.types.get_mut(query_type_name).unwrap() {
        ExtendedType::Object(obj) => obj,
        _ => panic!("query root is not an object"),
    };

    query_root.make_mut().fields.insert(
        name!("_entities"),
        Component::new(FieldDefinition {
            description: None,
            name: name!("_entities"),
            arguments: vec![Node::new(InputValueDefinition {
                description: None,
                name: name!("representations"),
                ty: Node::new(Type::NonNullList(Box::new(Type::NonNullNamed(name!(
                    "_Any"
                ))))),
                default_value: None,
                directives: Default::default(),
            })],
            ty: Type::NonNullList(Box::new(Type::Named(name!("_Entity")))),
            directives: Default::default(),
        }),
    );

    // Matching the behaviour in the Router:
    //   https://github.com/apollographql/router/blob/23e580e22a4401cc2e7a952b241a1ec955b29c99/apollo-federation/src/api_schema.rs#L139-L149
    if !schema.directive_definitions.contains_key(&name!("defer")) {
        schema
            .directive_definitions
            .insert(name!("defer"), defer_definition());
    }
    if !schema.directive_definitions.contains_key(&name!("stream")) {
        schema
            .directive_definitions
            .insert(name!("stream"), stream_definition());
    }
}

fn defer_definition() -> Node<DirectiveDefinition> {
    Node::new(DirectiveDefinition {
        description: None,
        name: name!("defer"),
        arguments: vec![
            Node::new(InputValueDefinition {
                description: None,
                name: name!("label"),
                ty: ty!(String).into(),
                default_value: None,
                directives: Default::default(),
            }),
            Node::new(InputValueDefinition {
                description: None,
                name: name!("if"),
                ty: ty!(Boolean!).into(),
                default_value: Some(true.into()),
                directives: Default::default(),
            }),
        ],
        repeatable: false,
        locations: vec![
            DirectiveLocation::FragmentSpread,
            DirectiveLocation::InlineFragment,
        ],
    })
}

fn stream_definition() -> Node<DirectiveDefinition> {
    Node::new(DirectiveDefinition {
        description: None,
        name: name!("stream"),
        arguments: vec![
            Node::new(InputValueDefinition {
                description: None,
                name: name!("label"),
                ty: ty!(String).into(),
                default_value: None,
                directives: Default::default(),
            }),
            Node::new(InputValueDefinition {
                description: None,
                name: name!("if"),
                ty: ty!(Boolean!).into(),
                default_value: Some(true.into()),
                directives: Default::default(),
            }),
            Node::new(InputValueDefinition {
                description: None,
                name: name!("initialCount"),
                ty: ty!(Int).into(),
                default_value: Some(0.into()),
                directives: Default::default(),
            }),
        ],
        repeatable: false,
        locations: vec![DirectiveLocation::Field],
    })
}
