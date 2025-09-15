use crate::{config::Config, latency::LatencyGenerator};
use apollo_compiler::{
    Node, Schema,
    ast::{FieldDefinition, InputValueDefinition, Type},
    collections::IndexSet,
    name,
    schema::{Component, ComponentName, ComponentOrigin, ExtendedType, ScalarType, UnionType},
    validation::Valid,
};
use hyper::{HeaderMap, header::HeaderValue};
use std::{fs, path::PathBuf, sync::OnceLock};
use tracing::info;

pub mod config;
pub mod handle;
pub mod latency;

static ADDITIONAL_HEADERS: OnceLock<HeaderMap<HeaderValue>> = OnceLock::new();
static LATENCY_GENERATOR: OnceLock<LatencyGenerator> = OnceLock::new();
static SUPERGRAPH_SCHEMA: OnceLock<Valid<Schema>> = OnceLock::new();

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
                info!("loading and parsing config file");
                serde_yaml::from_slice(&fs::read(path)?)?
            }
            None => {
                info!("using default config");
                Config::default()
            }
        };

        let (port, latency_generator, headers) = cfg.into_parts();

        info!("loading and parsing supergraph schema");
        match Schema::parse(fs::read_to_string(&self.schema)?, self.schema) {
            Ok(mut schema) => {
                patch_supergraph_for_entities(&mut schema);
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

        ADDITIONAL_HEADERS.set(headers).unwrap();
        LATENCY_GENERATOR.set(latency_generator).unwrap();

        Ok(port)
    }
}

/// We need to be able to intercept and handle queries for entities.
/// { _entities(representations: [_Any!]!): [_Entity]!
fn patch_supergraph_for_entities(schema: &mut Schema) {
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
}
