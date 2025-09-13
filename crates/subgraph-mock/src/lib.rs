use crate::{config::Config, latency::LatencyGenerator};
use apollo_compiler::{Schema, validation::Valid};
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
    pub config: PathBuf,

    /// Path to the supergraph SDL that the server should mock
    #[arg(short, long)]
    pub schema: PathBuf,
}

impl Args {
    /// Load and initialise the configuration based on command line args
    pub fn init(self) -> anyhow::Result<u16> {
        info!("loading and parsing config file");
        let cfg: Config = serde_yaml::from_slice(&fs::read(self.config)?)?;
        let (port, latency_generator, headers) = cfg.into_parts();

        info!("loading and parsing supergraph schema");
        match Schema::parse_and_validate(fs::read_to_string(&self.schema)?, self.schema) {
            Ok(schema) => SUPERGRAPH_SCHEMA.set(schema).unwrap(),
            Err(e) => panic!("ERROR: invalid supergraph schema\n{}", e.errors),
        };

        ADDITIONAL_HEADERS.set(headers).unwrap();
        LATENCY_GENERATOR.set(latency_generator).unwrap();

        Ok(port)
    }
}
