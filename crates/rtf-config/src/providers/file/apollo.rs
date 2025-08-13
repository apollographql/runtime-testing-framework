// Unit tests for the parsing and validation of the providers in this file
// are part of the suite of tests in the mod.rs file
use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    impl_template,
    providers::{
        self,
        file::{AsUtf8FileContent, ResolveAndWrite, Source},
    },
    templating::{self, Field, Scalar, Template},
};
use rtf_core::graphos::supergraph::{
    SupergraphDetails,
    operations::{fetch_offline_license, top_studio_operations::generate_canned_ops},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// # GraphOS Supergraph SDL
///
/// The user specifies the ref that should be used to fetch a supergraph SDL
/// file from the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct GraphosSupergraph {
    /// The Apollo graph ref to pull supergraph SDL for.
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

impl Check for GraphosSupergraph {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS subgraph SDL
///
/// The user specifies the graph ref that should be used to fetch a subgraph
/// SDL files from the GraphOS API.
///
/// Note that this file proivider will output a directory of SDL schema files, one for each
/// subgraph.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct GraphosSubgraphs {
    /// The Apollo graph ref to pull subgraph SDL files for.
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

impl Check for GraphosSubgraphs {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS Supergraph Docker Compose
///
/// The user specifies the graph ref that should be used to fetch subgraph
/// SDL files from the GraphOS API and generates a docker compose file that
/// runs all the subgraphs
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct GraphosSubgraphDockerCompose {
    pub graph_ref: Field<String>,
}

impl AsUtf8FileContent for GraphosSubgraphDockerCompose {
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

        let subgraphs = ctx
            .with_supergraph_details(graph_id, variant, |details| Ok(details.subgraphs.clone()))
            .await?;

        // The current subgraph mock service needs the full supergraph schema to run NOT the subgraph schema.
        // This is counter-intuitive and will be addressed when we create a new subgraph mocking service.
        let supergraph = ctx
            .with_supergraph_details(graph_id, variant, |details| {
                Ok(details.supergraph_sdl.clone())
            })
            .await?;

        let mut services: HashMap<String, SubgraphService> = HashMap::new();
        let mut port = 4001;
        for sg in subgraphs {
            let mut resources: HashMap<String, Resource> = HashMap::new();
            resources.insert(
                "limits".to_string(),
                Resource {
                    cpus: "0.1".to_string(),
                    memory: "1G".to_string(),
                },
            );
            resources.insert(
                "reservations".to_string(),
                Resource {
                    cpus: "0.1".to_string(),
                    memory: "512M".to_string(),
                },
            );

            let service = SubgraphService {
                image: "ghcr.io/apollographql/runtime-testing-framework/router-scale-subgraph:main"
                    .to_string(),
                container_name: sg.name.clone(),
                command: vec![
                    "-schema".to_string(),
                    "/app/supergraph.graphql".to_string(),
                    "-latency=5ms".to_string(),
                    "-sine-period=10s".to_string(),
                    "-sine-amplitude=2ms".to_string(),
                ],
                configs: vec![SubgraphConfig {
                    source: "supergraph.graphql".to_string(),
                    target: "/app/supergraph.graphql".to_string(),
                }],
                ports: vec![format!("{}:8080", port)],
                restart: "unless-stopped".to_string(),
                deploy: Deploy {
                    resources: Resources { resources },
                },
                mem_swappiness: 0,
            };

            services.insert(sg.name.clone(), service);
            port += 1;
        }

        let mut configs: HashMap<String, Config> = HashMap::new();
        configs.insert(
            "supergraph.graphql".to_string(),
            Config {
                content: supergraph,
            },
        );

        let compose = Compose {
            services: SubgraphServices { services },
            configs: Configs { configs },
        };
        let compose_yaml = serde_yaml::to_string(&compose)?;

        // Here follows the structs required to create a properly formatted docker compose file
        // The serde flatten from a HashMap allows us to create multiple similar objects with
        // uniquely named keys (so each service is named after the subgraph for example)
        #[derive(Serialize)]
        struct Compose {
            services: SubgraphServices,
            configs: Configs,
        }

        #[derive(Serialize)]
        struct SubgraphServices {
            #[serde(flatten)]
            services: HashMap<String, SubgraphService>,
        }

        #[derive(Serialize)]
        struct SubgraphService {
            image: String,
            container_name: String,
            command: Vec<String>,
            configs: Vec<SubgraphConfig>,
            ports: Vec<String>,
            restart: String,
            deploy: Deploy,
            mem_swappiness: i64,
        }

        #[derive(Serialize)]
        struct SubgraphConfig {
            source: String,
            target: String,
        }

        #[derive(Serialize)]
        struct Configs {
            #[serde(flatten)]
            configs: HashMap<String, Config>,
        }

        #[derive(Serialize)]
        struct Config {
            content: String,
        }

        #[derive(Serialize)]
        struct Deploy {
            resources: Resources,
        }

        #[derive(Serialize)]
        struct Resources {
            #[serde(flatten)]
            resources: HashMap<String, Resource>,
        }

        #[derive(Serialize)]
        struct Resource {
            cpus: String,
            memory: String,
        }

        Ok(compose_yaml)
    }
}

impl_template!(GraphosSubgraphDockerCompose => [graph_ref]);

impl Check for GraphosSubgraphDockerCompose {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS Canned Operations
///
/// The user specifies the graph ref and parameters that should be used to
/// generate canned GraphQL requests based on operations data obtained from
/// the GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct GraphosCannedOps {
    /// The Apollo graph ref to pull operations for.
    pub graph_ref: Field<String>,
    /// The number of operations to attempt to fetch.
    ///
    /// Defaults to 20 if unset.
    #[serde(default = "default_top_n")]
    pub top_n: Field<usize>,
    /// Whether or not to include mutations in the returned operations.
    ///
    /// Defaults to false if unset.
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

impl Check for GraphosCannedOps {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS Offline License
///
/// The user specifies the graph id that should be used to fetch an offline license from the
/// GraphOS API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct OfflineGraphosLicense {
    /// The Apollo graph ref to pull an offline license for.
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

impl Check for OfflineGraphosLicense {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_client(path, ctx)
    }
}

fn validate_graph_ref_and_client(
    graph_ref: &str,
    path: &[String],
    ctx: &impl ResolutionContext,
) -> checks::Result<()> {
    let mut errs = checks::ErrorBuilder::new();
    if !graph_ref.contains('@') {
        errs.push(
            checks::ErrorKind::InvalidGraphRef,
            format!("expected a string of the form 'graph_id@variant', got {graph_ref}"),
            path,
        );
    }
    errs.append(validate_client(path, ctx));

    errs.into_result(())
}

fn validate_client(path: &[String], ctx: &impl ResolutionContext) -> checks::Result<()> {
    if ctx.platform_client().is_none() {
        return Err(checks::Errors::new(
            checks::ErrorKind::MissingGraphOsApiKey,
            "expected os env key APOLLO_KEY",
            path,
        ));
    }

    Ok(())
}
