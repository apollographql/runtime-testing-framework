// Unit tests for the parsing and validation of the providers in this file
// are part of the suite of tests in the mod.rs file
use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    providers::{
        self,
        file::{AsUtf8FileContent, ResolveFileContent, Source},
    },
    templating::Field,
};
use indoc::indoc;
use reqwest::StatusCode;
use rtf_core::{
    HttpClient,
    graphos::supergraph::{
        Subgraph, SupergraphDetails,
        operations::{
            canned_operations::{CannedOperation, canned_ops_for_ids, top_studio_canned_ops},
            fetch_offline_license,
        },
    },
};
use rtf_derive::Template;
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
///
/// ```yaml
/// - name: "supergraph.graphql"
///   env_var: SUPERGRAPH
///   kind: graphos_supergraph
///   graph_ref: graph@variant
///   with_subgraph_overrides: docker
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct GraphosSupergraph {
    /// The Apollo graph ref to pull supergraph SDL for.
    pub graph_ref: Field<String>,
    /// Replace the supergraph's subgraph urls with overridden values for testing.
    ///
    /// Defaults to null if unset.
    #[serde(default)]
    #[template(skip)]
    pub with_subgraph_overrides: Option<UrlFormat>,
}

impl GraphosSupergraph {
    fn content_from_details(&self, mut sg: Arc<SupergraphDetails>) -> String {
        match self.with_subgraph_overrides.as_ref() {
            Some(url_format) => {
                let sg = Arc::make_mut(&mut sg);
                let subgraph_urls = url_format.urls_for_subgraphs(&sg.subgraphs);
                sg.rewrite_subgraph_urls(&subgraph_urls)
                    .expect("unable to rewrite subgraph URLs");

                sg.supergraph_sdl.clone()
            }

            None => sg.supergraph_sdl.clone(),
        }
    }
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

        let sg = ctx
            .with_supergraph_details(graph_id, variant, |details| Ok(details.clone()))
            .await?;

        Ok(self.content_from_details(sg))
    }
}

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

/// # GraphOS Subgraph SDL
///
/// The user specifies the graph ref that should be used to fetch a subgraph
/// SDL files from the GraphOS API.
///
/// Note that this file proivider will output a directory of SDL schema files, one for each
/// subgraph.
///
/// ```yaml
/// - name: "subgraphs"
///   env_var: SUBGRAPHS
///   kind: graphos_subgraphs
///   graph_ref: graph@variant
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct GraphosSubgraphs {
    /// The Apollo graph ref to pull subgraph SDL files for.
    pub graph_ref: Field<String>,
}

impl GraphosSubgraphs {
    fn content_from_details(
        &self,
        sg: Arc<SupergraphDetails>,
        dir: &Path,
    ) -> Vec<(PathBuf, String)> {
        let contents: Vec<_> = sg
            .subgraphs
            .clone()
            .into_iter()
            .map(|sg| (dir.join(sg.name).with_extension("graphql"), sg.sdl))
            .collect();

        contents
    }
}

impl ResolveFileContent for GraphosSubgraphs {
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        _src: &Source,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<Vec<(PathBuf, String)>> {
        let (graph_id, variant) = self
            .graph_ref
            .as_resolved()
            .split_once('@')
            .expect("validated graph_ref");

        let sg = ctx
            .with_supergraph_details(graph_id, variant, |details| Ok(details.clone()))
            .await?;

        Ok(self.content_from_details(sg, target.as_ref()))
    }
}

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

/// # GraphOS Subgraph Docker Compose
///
/// The user specifies the graph ref that should be used to fetch the supergraph
/// SDL file from the GraphOS API and generates a docker compose file. It runs a
/// configurable number of subgraph services, mocking based on the supergraph
/// schema behind a loadbalancer.
///
/// ```yaml
/// - name: "subgraph-compose.yaml"
///   env_var: SUBGRAPH_COMPOSE
///   kind: graphos_subgraph_docker_compose
///   graph_ref: graph@variant
///   image: ghcr.io/apollographql/runtime-testing-framework/router-scale-subgraph:main
///   command:
///   - -schema
///   - /app/supergraph.graphql
///   replicas: 5
///   resource_limits:
///     cpus: '0.5'
///     memory: 1G
///   resource_reservations:
///    cpus: '0.1'
///     memory: 512M
///   mem_swappiness: 0
///   loadbalancer:
///     resource_limits:
///       cpus: '0.5'
///       memory: 1G
///     resource_reservations:
///       cpus: '0.1'
///       memory: 512M
///     mem_swappiness: 0
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct GraphosSubgraphDockerCompose {
    /// The Apollo graph ref to pull the supergraph for.
    pub graph_ref: Field<String>,
    /// The image the subgraph service runs.
    ///
    /// Defaults to ghcr.io/apollographql/runtime-testing-framework/router-scale-subgraph:main if unset.
    #[serde(default = "default_image")]
    pub image: Field<String>,
    /// The command that subgraph server image runs.
    ///
    /// Defaults to "-schema /app/supergraph.graphql" if unset.
    #[serde(default = "default_command")]
    #[template(skip)]
    pub command: Vec<String>,
    /// The number of subgraph services containers running.
    ///
    /// Defaults to 5 if unset.
    #[serde(default = "default_replicas")]
    pub replicas: Field<i32>,
    /// The resource limits for the subgraph containers.
    ///
    /// Defaults to cpus=0.5 and memory=1G if unset.
    #[serde(default = "default_limits")]
    pub resource_limits: Resources,
    /// The reserved resources for the subgraph containers.
    ///
    /// Defaults to cpus=0.1 and memory=512M if unset.
    #[serde(default = "default_reservations")]
    pub resource_reservations: Resources,
    /// Enable or disable memory swapping in the subgraph services.
    ///
    /// Defaults to 0 (disabled) if unset.
    #[serde(default = "default_mem_swappiness")]
    pub mem_swappiness: Field<i32>,
    /// The configuration for the subgraph's loadbalancer.
    #[serde(default = "default_loadbalancer")]
    pub loadbalancer: Loadbalancer,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct Loadbalancer {
    /// The resource limits for the loadbalancer.
    ///
    /// Defaults to cpus=0.5 and memory=1G if unset.
    pub resource_limits: Resources,
    /// The reserved resources for the loadbalancer.
    ///
    /// Defaults to cpus=0.1 and memory=512M if unset.
    pub resource_reservations: Resources,
    /// Enable or disable memory swapping in the loadbalancer.
    ///
    /// Defaults to 0 (disabled) if unset.
    pub mem_swappiness: Field<i32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct Resources {
    /// The cpus allocated for the resource.
    pub cpus: Field<String>,
    /// The memory allocated for the resource.
    pub memory: Field<String>,
}

fn default_image() -> Field<String> {
    Field::Resolved(
        "ghcr.io/apollographql/runtime-testing-framework/router-scale-subgraph:main".to_string(),
    )
}

fn default_command() -> Vec<String> {
    vec!["-schema".to_string(), "/app/supergraph.graphql".to_string()]
}

fn default_replicas() -> Field<i32> {
    Field::Resolved(5)
}

fn default_limits() -> Resources {
    Resources {
        cpus: Field::Resolved("0.5".to_string()),
        memory: Field::Resolved("1G".to_string()),
    }
}

fn default_reservations() -> Resources {
    Resources {
        cpus: Field::Resolved("0.1".to_string()),
        memory: Field::Resolved("512M".to_string()),
    }
}

fn default_mem_swappiness() -> Field<i32> {
    Field::Resolved(0)
}

fn default_loadbalancer() -> Loadbalancer {
    Loadbalancer {
        resource_limits: Resources {
            cpus: Field::Resolved("0.5".to_string()),
            memory: Field::Resolved("1G".to_string()),
        },
        resource_reservations: Resources {
            cpus: Field::Resolved("0.1".to_string()),
            memory: Field::Resolved("512M".to_string()),
        },
        mem_swappiness: Field::Resolved(0),
    }
}

impl GraphosSubgraphDockerCompose {
    fn content_from_details(&self, sg: Arc<SupergraphDetails>) -> providers::Result<String> {
        // We are inlining the supergraph file to a docker compose file. It will interpolate $ by default. To avoid
        // this we escape the $ symbols using $$
        // https://docs.docker.com/reference/compose-file/interpolation/
        let supergraph = sg.supergraph_sdl.replace("$", "$$");

        let mut subgraph_resources: HashMap<String, Resource> = HashMap::new();
        subgraph_resources.insert(
            "limits".to_string(),
            Resource {
                cpus: self.resource_limits.cpus.as_resolved().to_string(),
                memory: self.resource_limits.memory.as_resolved().to_string(),
            },
        );
        subgraph_resources.insert(
            "reservations".to_string(),
            Resource {
                cpus: self.resource_reservations.cpus.as_resolved().to_string(),
                memory: self.resource_reservations.memory.as_resolved().to_string(),
            },
        );

        let subgraph_service = SubgraphService {
            image: self.image.as_resolved().to_string(),
            command: self.command.clone(),
            configs: vec![ServiceConfig {
                source: "supergraph.graphql".to_string(),
                target: "/app/supergraph.graphql".to_string(),
            }],
            expose: vec!["8080".to_string()],
            restart: "unless-stopped".to_string(),
            deploy: DeployWithReplicas {
                replicas: *self.replicas.as_resolved(),
                resources: Resources {
                    resources: subgraph_resources,
                },
            },
            mem_swappiness: *self.mem_swappiness.as_resolved(),
        };

        let mut loadbalancer_resources: HashMap<String, Resource> = HashMap::new();
        loadbalancer_resources.insert(
            "limits".to_string(),
            Resource {
                cpus: self
                    .loadbalancer
                    .resource_limits
                    .cpus
                    .as_resolved()
                    .to_string(),
                memory: self
                    .loadbalancer
                    .resource_limits
                    .memory
                    .as_resolved()
                    .to_string(),
            },
        );
        loadbalancer_resources.insert(
            "reservations".to_string(),
            Resource {
                cpus: self
                    .loadbalancer
                    .resource_reservations
                    .cpus
                    .as_resolved()
                    .to_string(),
                memory: self
                    .loadbalancer
                    .resource_reservations
                    .memory
                    .as_resolved()
                    .to_string(),
            },
        );

        let loadbalancer_service = LoadbalancerService {
            image: "nginx:alpine".to_string(),
            container_name: "loadbalancer".to_string(),
            configs: vec![ServiceConfig {
                source: "nginx.conf".to_string(),
                target: "/etc/nginx/nginx.conf".to_string(),
            }],
            ports: vec!["8080:8080".to_string()],
            restart: "unless-stopped".to_string(),
            deploy: Deploy {
                resources: Resources {
                    resources: loadbalancer_resources,
                },
            },
            mem_swappiness: *self.loadbalancer.mem_swappiness.as_resolved(),
            depends_on: vec!["subgraph".to_string()],
        };

        let mut configs: HashMap<String, Config> = HashMap::new();
        configs.insert(
            "supergraph.graphql".to_string(),
            Config {
                content: supergraph,
            },
        );
        let nginx_config = indoc![
            r#"
            events {
                worker_connections 10000;
            }

            http {
                upstream backend {
                    server subgraph:8080;  # Docker DNS will resolve all replicas
                }

                server {
                    listen 8080;
                    location / {
                        proxy_pass http://backend;
                    }
                }
            }
        "#
        ];
        configs.insert(
            "nginx.conf".to_string(),
            Config {
                content: nginx_config.to_string(),
            },
        );

        let compose = Compose {
            services: Services {
                subgraph: subgraph_service,
                loadbalancer: loadbalancer_service,
            },
            configs: Configs { configs },
        };
        let compose_yaml = serde_yaml::to_string(&compose)?;

        return Ok(compose_yaml);

        #[derive(Serialize)]
        struct Compose {
            services: Services,
            configs: Configs,
        }

        #[derive(Serialize)]
        struct Services {
            subgraph: SubgraphService,
            loadbalancer: LoadbalancerService,
        }

        #[derive(Serialize)]
        struct SubgraphService {
            image: String,
            command: Vec<String>,
            configs: Vec<ServiceConfig>,
            expose: Vec<String>,
            restart: String,
            deploy: DeployWithReplicas,
            mem_swappiness: i32,
        }

        #[derive(Serialize)]
        struct LoadbalancerService {
            image: String,
            container_name: String,
            configs: Vec<ServiceConfig>,
            ports: Vec<String>,
            restart: String,
            deploy: Deploy,
            mem_swappiness: i32,
            depends_on: Vec<String>,
        }

        #[derive(Serialize)]
        struct ServiceConfig {
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
        struct DeployWithReplicas {
            replicas: i32,
            resources: Resources,
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
    }
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

        // The current subgraph mock service needs the full supergraph schema to run NOT the subgraph schema.
        // This is counter-intuitive and will be addressed when we create a new subgraph mocking service.
        let sg = ctx
            .with_supergraph_details(graph_id, variant, |details| Ok(details.clone()))
            .await?;

        self.content_from_details(sg)
    }
}

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

/// # GraphOS Supergraph Router URL Overrides
///
/// The user specifies the graph ref that should be used to fetch subgraph
/// SDL files from the GraphOS API and generates a the override_subgraph_urls
/// YAML snippet that can be merged into a router config file
///
/// This should be used when generating the subgraph docker compose using
/// [GraphosSubgraphDockerCompose]. This will ensure the router subgraph urls
/// map to the loadbalancer url in that compose file.
///
/// ```yaml
/// - name: subgraph-url-overrides.yaml
///   env_var: SUBGRAPH_URL_OVERRIDES
///   kind: graphos_subgraph_router_url_overrides
///   graph_ref: graph@variant
///   url_format: localhost
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct GraphosSubgraphRouterUrlOverrides {
    /// The Apollo graph ref to pull the subgraphs for.
    pub graph_ref: Field<String>,
    /// The format of the overrides url.
    #[serde(default = "default_url_format")]
    #[template(skip)]
    pub url_format: UrlFormat,
}

fn default_url_format() -> UrlFormat {
    UrlFormat::Localhost
}

/// The allowed url formats for the [GraphosSubgraphRouterUrlOverrides]
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UrlFormat {
    Localhost,
    Docker,
}

impl UrlFormat {
    fn urls_for_subgraphs(&self, subgraphs: &[Subgraph]) -> HashMap<String, String> {
        let mut subgraph_urls: HashMap<String, String> = HashMap::new();
        let base_port = 4001;

        for (offset, sg) in subgraphs.iter().enumerate() {
            let port = base_port + offset;

            let url = self.subgraph_url(&port);
            subgraph_urls.insert(sg.name.clone(), url);
        }

        subgraph_urls
    }

    fn subgraph_url(&self, port: &usize) -> String {
        match self {
            UrlFormat::Localhost => format!("http://localhost:{port}"),
            // The docker compose generated by rtf puts all subgraph containers behind a loadbalancer
            UrlFormat::Docker => "http://loadbalancer:8080".to_string(),
        }
    }
}

impl GraphosSubgraphRouterUrlOverrides {
    fn content_from_details(&self, sg: Arc<SupergraphDetails>) -> providers::Result<String> {
        let subgraph_urls = self.url_format.urls_for_subgraphs(&sg.subgraphs.clone());
        let overrides_yaml =
            serde_yaml::to_string(&serde_json::json!({"override_subgraph_url": subgraph_urls}))?;

        Ok(overrides_yaml)
    }
}

impl AsUtf8FileContent for GraphosSubgraphRouterUrlOverrides {
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

        let sg = ctx
            .with_supergraph_details(graph_id, variant, |details| Ok(details.clone()))
            .await?;

        self.content_from_details(sg)
    }
}

impl Check for GraphosSubgraphRouterUrlOverrides {
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
///
/// ```yaml
/// - name: canned_ops.json
///   env_var: CANNED_OPS_FILE
///   kind: graphos_canned_ops
///   graph_ref: graph@variant
///   top_n: 10
///   skip_mutations: true
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
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
        let canned_ops = top_studio_canned_ops(
            &details,
            *self.top_n.as_resolved(),
            *self.skip_mutations.as_resolved(),
            client,
        )
        .await?;

        canned_ops_json_lines(canned_ops)
    }
}

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

/// # GraphOS Canned Operations by ID
///
/// The user specifies the graph ref and parameters that should be used to
/// generate canned GraphQL requests based on operations data obtained from
/// the GraphOS API.
///
/// ```yaml
/// - name: canned_ops.json
///   env_var: CANNED_OPS_FILE
///   kind: graphos_canned_ops_by_id
///   graph_ref: graph@variant
///   operation_ids:
///     - 5b1f8a2a1bd4be697559013a23fcbcb9186afe77
///     - 3f56aa92aad650bbfc7ba481cbe029aba2f6c5f4
///     - 50b77d7351052abd84dcd2c2ccb63eff2fa2f94c
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct GraphosCannedOpsById {
    /// The Apollo graph ref to pull operations for.
    pub graph_ref: Field<String>,
    /// Operation IDs from the Apollo studio API for the operations you want to
    /// work with as queried from an `OperationInsightsListItem` in the Studio
    /// graphQL API.
    pub operation_ids: Vec<Field<String>>,
}

impl AsUtf8FileContent for GraphosCannedOpsById {
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
        let ids: Vec<String> = self
            .operation_ids
            .iter()
            .map(|id| id.as_resolved().clone())
            .collect();
        let canned_ops = canned_ops_for_ids(&details, ids, client).await?;

        canned_ops_json_lines(canned_ops)
    }
}

impl Check for GraphosCannedOpsById {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

fn canned_ops_json_lines(canned_ops: Vec<CannedOperation>) -> providers::Result<String> {
    // Create a json line file for each of the canned operations
    let mut json_file = canned_ops
        .iter()
        .map(|v| v.to_json_string())
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");

    // This appends a new line to the json file
    // Without this, when shell scripts iterate over the operations they count the lines to iterate over as N-1
    json_file.push('\n');

    Ok(json_file)
}

/// # GraphOS Offline License
///
/// The user specifies the graph id that should be used to fetch an offline license from the
/// GraphOS API.
///
/// ```yaml
/// - name: license.jwt
///   env_var: LICENSE
///   kind: graphos_offline_license
///   graph_id: graph
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct OfflineGraphosLicense {
    /// The Apollo graph id to pull an offline license for.
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

/// # Router Download Script
///
/// Produces a POSIX shell script that can be run in order to download a target version of the
/// Apollo Router.
///
/// ```yaml
/// - name: "router-download.sh"
///   env_var: ROUTER_DOWNLOAD
///   kind: router_download_script
///   version: "v2.6.0"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct RouterDownloadScript {
    /// The version of the Apollo Router to download.
    pub(crate) version: Field<String>,
}

impl AsUtf8FileContent for RouterDownloadScript {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let version = self.version.as_resolved();
        let url = format!("https://router.apollo.dev/download/nix/{version}");
        let response = ctx.http_client().get(&url).await?;
        if response.status == StatusCode::NOT_FOUND {
            return Err(providers::Error::UnknownRouterVersion(version.to_string()));
        }
        let script = std::str::from_utf8(&response.body)
            .map_err(|_| providers::Error::Utf8DecodingError)?
            .to_string();

        Ok(script)
    }
}

impl Check for RouterDownloadScript {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}

/// # Build Router From Source
///
/// A file provider used for building the Router from source at a specific git commit
/// or reference.
///
/// ```yaml
/// - name: "router-build.sh"
///   env_var: ROUTER_BUILD_SCRIPT
///   kind: build_router_from_source
///   git_ref: "some-ref"
///   rust_version: "1.89.0"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct BuildRouterFromSource {
    /// A git reference that can be passed to `git checkout`. This may be
    /// a full or partial commit hash, branch name, or tag.
    pub(crate) git_ref: Field<String>,

    /// A Rust version string that can be passed to `rustup run {rust_version}`,
    /// such as `"1.78.0"`, `"beta"`, or `"nightly"`.
    ///
    /// Defaults to `"stable"` if unset.
    #[serde(default = "default_rust_version")]
    pub(crate) rust_version: Field<String>,
}

fn default_rust_version() -> Field<String> {
    Field::Resolved("stable".to_string())
}

impl AsUtf8FileContent for BuildRouterFromSource {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let commit_ref = self.git_ref.as_resolved();
        let rust_version = self.rust_version.as_resolved();

        let install_script = format!(
            indoc!(
                r#"mkdir router-source && \
                cd router-source && \
                git clone https://github.com/apollographql/router.git && \
                cd router && \
                git checkout {} && \
                rustup toolchain install {} && \
                rustup run {} cargo build --release && \
                cp ${{CARGO_TARGET_DIR}}/release/router ~/.cargo/bin/"#
            ),
            commit_ref, rust_version, rust_version
        );

        Ok(install_script)
    }
}

impl Check for BuildRouterFromSource {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        checks::ErrorKind,
        context::Context,
        mock_context::MockContext,
        providers::file::{
            FileProvider,
            tests::{
                assert_check_errors, assert_resolve_and_write_error,
                assert_resolve_and_write_success,
            },
        },
    };
    use assert_fs::{TempDir, fixture::PathChild};
    use predicates::{Predicate, str::contains};
    use simple_test_case::test_case;

    /// Create a GraphOS Supergraph
    fn supergraph_fp(graph_ref: &str) -> FileProvider {
        FileProvider::GraphosSupergraph(supergraph(graph_ref))
    }
    fn supergraph(graph_ref: &str) -> GraphosSupergraph {
        GraphosSupergraph {
            graph_ref: Field::Resolved(graph_ref.to_string()),
            with_subgraph_overrides: None,
        }
    }

    /// Create a GraphOS Subgraphs
    fn subgraphs_fp(graph_ref: &str) -> FileProvider {
        FileProvider::GraphosSubgraphs(subgraphs(graph_ref))
    }
    fn subgraphs(graph_ref: &str) -> GraphosSubgraphs {
        GraphosSubgraphs {
            graph_ref: Field::Resolved(graph_ref.to_string()),
        }
    }

    /// Create a GraphOS Subgraphs Docker Compose
    fn subgraphs_compose_fp(graph_ref: &str) -> FileProvider {
        FileProvider::GraphosSubgraphDockerCompose(subgraphs_compose(graph_ref))
    }
    fn subgraphs_compose(graph_ref: &str) -> GraphosSubgraphDockerCompose {
        GraphosSubgraphDockerCompose {
            graph_ref: Field::Resolved(graph_ref.to_string()),
            image: Field::Resolved("image".to_string()),
            command: Vec::new(),
            replicas: Field::Resolved(1),
            resource_limits: Resources {
                cpus: Field::Resolved("1".to_string()),
                memory: Field::Resolved("1G".to_string()),
            },
            resource_reservations: Resources {
                cpus: Field::Resolved("1".to_string()),
                memory: Field::Resolved("1G".to_string()),
            },
            mem_swappiness: Field::Resolved(0),
            loadbalancer: Loadbalancer {
                resource_limits: Resources {
                    cpus: Field::Resolved("1".to_string()),
                    memory: Field::Resolved("1G".to_string()),
                },
                resource_reservations: Resources {
                    cpus: Field::Resolved("1".to_string()),
                    memory: Field::Resolved("1G".to_string()),
                },
                mem_swappiness: Field::Resolved(0),
            },
        }
    }

    /// Create a GraphOS Subgraphs URL Overrides
    fn subgraphs_overrides_fp(graph_ref: &str) -> FileProvider {
        FileProvider::GraphosSubgraphRouterUrlOverrides(subgraphs_overrides(graph_ref))
    }
    fn subgraphs_overrides(graph_ref: &str) -> GraphosSubgraphRouterUrlOverrides {
        GraphosSubgraphRouterUrlOverrides {
            graph_ref: Field::Resolved(graph_ref.to_string()),
            url_format: UrlFormat::Docker,
        }
    }

    /// Create a GraphOS Canned Ops
    fn canned_ops(graph_ref: &str) -> FileProvider {
        FileProvider::GraphosCannedOps(GraphosCannedOps {
            graph_ref: Field::Resolved(graph_ref.to_string()),
            top_n: Field::Resolved(20),
            skip_mutations: Field::Resolved(true),
        })
    }

    /// Create a GraphOS Canned Ops by ID
    fn canned_ops_by_id(graph_ref: &str) -> FileProvider {
        FileProvider::GraphosCannedOpsById(GraphosCannedOpsById {
            graph_ref: Field::Resolved(graph_ref.to_string()),
            operation_ids: Vec::new(),
        })
    }

    /// Helper function for supergraph sdl
    fn supergraph_sdl() -> &'static str {
        indoc!(
            r#"
            schema
                @link(url: "https://specs.apollo.dev/link/v1.0")
                @link(url: "https://specs.apollo.dev/join/v0.5", for: EXECUTION)
            {
                query: Query
            }

            directive @join__directive(graphs: [join__Graph!], name: String!, args: join__DirectiveArguments) repeatable on SCHEMA | OBJECT | INTERFACE | FIELD_DEFINITION

            directive @join__enumValue(graph: join__Graph!) repeatable on ENUM_VALUE

            directive @join__field(graph: join__Graph, requires: join__FieldSet, provides: join__FieldSet, type: String, external: Boolean, override: String, usedOverridden: Boolean, overrideLabel: String, contextArguments: [join__ContextArgument!]) repeatable on FIELD_DEFINITION | INPUT_FIELD_DEFINITION

            directive @join__graph(name: String!, url: String!) on ENUM_VALUE

            directive @join__implements(graph: join__Graph!, interface: String!) repeatable on OBJECT | INTERFACE

            directive @join__type(graph: join__Graph!, key: join__FieldSet, extension: Boolean! = false, resolvable: Boolean! = true, isInterfaceObject: Boolean! = false) repeatable on OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT | SCALAR

            directive @join__unionMember(graph: join__Graph!, member: String!) repeatable on UNION

            directive @link(url: String, as: String, for: link__Purpose, import: [link__Import]) repeatable on SCHEMA

            type Bar
            @join__type(graph: BAR)
            {
            id: ID!
            }

            type Foo
            @join__type(graph: FOO)
            {
                id: ID!
            }

            input join__ContextArgument {
                name: String!
                type: String!
                context: String!
                selection: join__FieldValue!
            }

            scalar join__DirectiveArguments

            scalar join__FieldSet

            scalar join__FieldValue

            enum join__Graph {
                BAR @join__graph(name: "bar", url: "https://bar.com")
                FOO @join__graph(name: "foo", url: "https://foo.com")
            }

            scalar link__Import

            enum link__Purpose {
            """
            `SECURITY` features provide metadata necessary to securely resolve fields.
            """
            SECURITY

            """
            `EXECUTION` features provide metadata necessary for operation execution.
            """
            EXECUTION
            }

            type Query
                @join__type(graph: BAR)
                @join__type(graph: FOO)
            {
                bar: Bar @join__field(graph: BAR)
                foo: Foo @join__field(graph: FOO)
            }
        "#
        )
    }

    /// Helper function for subgraph foo schema
    fn subgraph_foo() -> &'static str {
        indoc!(
            r#"
            extend type Query {
                foo: Foo
            }

            type Foo  {
                id: ID! 
            }

            extend schema
            @link(url: "https://specs.apollo.dev/federation/v2.10",
                    import: ["@key"])
        "#
        )
    }

    /// Helper function for subgraph bar schema
    fn subgraph_bar() -> &'static str {
        indoc!(
            r#"
            extend type Query {
                foo: Bar
            }

            type Bar  {
                id: ID! 
            }

            extend schema
            @link(url: "https://specs.apollo.dev/federation/v2.10",
                    import: ["@key"])
        "#
        )
    }

    /// Helper function for supergraph details
    fn supergraph_details() -> Arc<SupergraphDetails> {
        Arc::new(SupergraphDetails {
            graph_id: "graph".to_string(),
            variant: "variant".to_string(),
            supergraph_sdl: supergraph_sdl().to_string(),
            subgraphs: vec![
                Subgraph {
                    name: "foo".to_string(),
                    sdl: subgraph_foo().to_string(),
                },
                Subgraph {
                    name: "bar".to_string(),
                    sdl: subgraph_bar().to_string(),
                },
            ],
        })
    }

    #[test_case(supergraph_fp("graph@variant"), true, &[]; "supergraph success")]
    #[test_case(supergraph_fp("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "supergraph invalid ref")]
    #[test_case(supergraph_fp("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "supergraph missing key")]
    #[test_case(supergraph_fp("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "supergraph invalid ref and missing key")]
    #[test_case(subgraphs_fp("graph@variant"), true, &[]; "subgraphs success")]
    #[test_case(subgraphs_fp("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "subgraphs invalid ref")]
    #[test_case(subgraphs_fp("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "subgraphs missing key")]
    #[test_case(subgraphs_fp("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "subgraphs invalid ref and missing key")]
    #[test_case(subgraphs_compose_fp("graph@variant"), true, &[]; "subgraphs compose success")]
    #[test_case(subgraphs_compose_fp("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "subgraphs compose invalid ref")]
    #[test_case(subgraphs_compose_fp("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "subgraphs compose missing key")]
    #[test_case(subgraphs_compose_fp("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "subgraphs compose invalid ref and missing key")]
    #[test_case(subgraphs_overrides_fp("graph@variant"), true, &[]; "subgraphs overrides success")]
    #[test_case(subgraphs_overrides_fp("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "subgraphs overrides invalid ref")]
    #[test_case(subgraphs_overrides_fp("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "subgraphs overrides missing key")]
    #[test_case(subgraphs_overrides_fp("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "subgraphs overrides invalid ref and missing key")]
    #[test_case(canned_ops("graph@variant"), true, &[]; "canned ops success")]
    #[test_case(canned_ops("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "canned ops invalid ref")]
    #[test_case(canned_ops("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "canned ops missing key")]
    #[test_case(canned_ops("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "canned ops invalid ref and missing key")]
    #[test_case(canned_ops_by_id("graph@variant"), true, &[]; "canned ops by id success")]
    #[test_case(canned_ops_by_id("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "canned ops by id invalid ref")]
    #[test_case(canned_ops_by_id("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "canned ops by id missing key")]
    #[test_case(canned_ops_by_id("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "canned ops by id invalid ref and missing key")]
    #[test]
    fn try_check_graph_ref_providers(
        fp: FileProvider,
        with_platform_config: bool,
        expected_err_kinds: &[ErrorKind],
    ) {
        let src = Source::Local {
            abs_path: "/".into(),
        };
        let mut ctx = Context::new();
        if with_platform_config {
            ctx.with_platform_config("dummy_key", false, false);
        }

        if !expected_err_kinds.is_empty() {
            assert_check_errors(fp, &src, &ctx, expected_err_kinds);
        } else {
            let res = fp.try_check(&mut Vec::new(), &src, &ctx);
            assert!(res.is_ok(), "expected check to succeed, got {res:?}");
        }
    }

    #[test]
    fn try_check_offline_license_success() {
        let offline = OfflineGraphosLicense {
            graph_id: Field::Resolved("graph".to_string()),
        };

        let src = Source::Local {
            abs_path: "/".into(),
        };
        let mut ctx = Context::new();
        ctx.with_platform_config("dummy_key", false, false);

        let res = offline.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn try_check_offline_license_missing_api_key() {
        let offline = OfflineGraphosLicense {
            graph_id: Field::Resolved("graph".to_string()),
        };

        let src = Source::Local {
            abs_path: "/".into(),
        };
        let ctx = Context::new();

        assert_check_errors(offline, &src, &ctx, &[ErrorKind::MissingGraphOsApiKey]);
    }

    #[test]
    fn try_check_router_download_script_success() {
        let router_download = RouterDownloadScript {
            version: Field::Resolved("v2.0.0".to_string()),
        };

        let src = Source::Local {
            abs_path: "/".into(),
        };
        let ctx = Context::new();

        let res = router_download.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn try_check_build_router_from_source_success() {
        let build_from_source = BuildRouterFromSource {
            git_ref: Field::Resolved("ref".to_string()),
            rust_version: Field::Resolved("1.90.0".to_string()),
        };

        let src = Source::Local {
            abs_path: "/".into(),
        };
        let ctx = Context::new();

        let res = build_from_source.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[tokio::test]
    async fn content_from_details_supergraph_success() {
        let details = supergraph_details();
        let supergraph = supergraph("graph@variant");

        let expected_content = supergraph_sdl();
        let res = supergraph.content_from_details(details);
        assert_eq!(res, expected_content)
    }

    #[tokio::test]
    async fn content_from_details_supergraph_with_docker_overrides_success() {
        let details = supergraph_details();
        let supergraph = GraphosSupergraph {
            graph_ref: Field::Resolved("graph@variant".to_string()),
            with_subgraph_overrides: Some(UrlFormat::Docker),
        };

        // The supergraph file indentation is transformed so that assert_eq is not possible
        // Instead, we check that the foo and bar url references are as expected
        let expected_foo_url = r#"FOO @join__graph(name: "foo", url: "http://loadbalancer:8080")"#;
        let expected_bar_url = r#"BAR @join__graph(name: "bar", url: "http://loadbalancer:8080")"#;

        let res = supergraph.content_from_details(details);
        assert!(
            contains(expected_foo_url).eval(&res),
            "expected file to contain: {expected_foo_url:?}"
        );
        assert!(
            contains(expected_bar_url).eval(&res),
            "expected file to contain: {expected_bar_url:?}"
        );
    }

    #[tokio::test]
    async fn content_from_details_supergraph_with_lcoalhost_overrides_success() {
        let details = supergraph_details();
        let supergraph = GraphosSupergraph {
            graph_ref: Field::Resolved("graph@variant".to_string()),
            with_subgraph_overrides: Some(UrlFormat::Localhost),
        };

        // The supergraph file indentation is transformed so that assert_eq is not possible
        // Instead, we check that the foo and bar url references are as expected
        let expected_foo_url = r#"FOO @join__graph(name: "foo", url: "http://localhost:4001")"#;
        let expected_bar_url = r#"BAR @join__graph(name: "bar", url: "http://localhost:4002")"#;

        let res = supergraph.content_from_details(details);
        assert!(
            contains(expected_foo_url).eval(&res),
            "expected file to contain: {expected_foo_url:?}"
        );
        assert!(
            contains(expected_bar_url).eval(&res),
            "expected file to contain: {expected_bar_url:?}"
        );
    }

    #[tokio::test]
    async fn content_from_details_subgraphs_success() {
        let details = supergraph_details();
        let subgraphs = subgraphs("graph@variant");

        let base_path = Path::new("subgraphs");

        let expected_content: Vec<(PathBuf, String)> = vec![
            (base_path.join("foo.graphql"), subgraph_foo().to_string()),
            (base_path.join("bar.graphql"), subgraph_bar().to_string()),
        ];

        let res = subgraphs.content_from_details(details, base_path);
        assert_eq!(res, expected_content)
    }

    #[tokio::test]
    async fn content_from_details_subgraph_docker_compose_success() {
        let details = supergraph_details();
        let subgraphs_compose = subgraphs_compose("graph@variant");

        // We are testing the supergraph sdl gets added to the file by checking for a snippet of it
        let expected_contains = "Foo @join__field(graph: FOO)";

        // We are not doing a full content match as the maps in the yaml file do not get
        // written out in a deterministic order.
        let res = subgraphs_compose.content_from_details(details);
        assert!(res.is_ok(), "expected String, got {res:?}");
        assert!(
            contains(expected_contains).eval(&res.unwrap()),
            "expected file to contain: {expected_contains:?}"
        );
    }

    #[tokio::test]
    async fn content_from_details_subgraph_url_overrides_success() {
        let details = supergraph_details();
        let subgraphs_overrides = subgraphs_overrides("graph@variant");

        let expected_content = "override_subgraph_url:\n  bar: http://loadbalancer:8080\n  foo: http://loadbalancer:8080\n";

        let res = subgraphs_overrides.content_from_details(details);
        assert!(res.is_ok(), "expected String, got {res:?}");
        assert_eq!(res.unwrap(), expected_content);
    }

    #[test]
    fn canned_ops_json_formats_correctly() {
        let canned_ops: Vec<CannedOperation> = vec![
            CannedOperation {
                id: "1".to_string(),
                query: "query_1".to_string(),
                pretty_query: "pretty_query_1".to_string(),
                vars: HashMap::new(),
            },
            CannedOperation {
                id: "2".to_string(),
                query: "query_2".to_string(),
                pretty_query: "pretty_query_2".to_string(),
                vars: HashMap::new(),
            },
        ];

        let expected_content = indoc!(
            r#"
            {"query":"query_1","variables":{}}
            {"query":"query_2","variables":{}}
            "#
        );

        let res = canned_ops_json_lines(canned_ops);
        assert!(res.is_ok(), "expected String, got {res:?}");
        assert_eq!(res.unwrap(), expected_content);
    }

    #[tokio::test]
    async fn resolve_and_write_router_download_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("router-download.sh");

        let version = "v2.6.0";
        let url = format!("https://router.apollo.dev/download/nix/{version}");
        let expected_content = "router download script";

        let responses = &[(url.as_str(), "200", expected_content)];

        let mut ctx = MockContext::with_http_client(responses);
        let src = Source::Local {
            abs_path: PathBuf::new(),
        };

        let router_download = FileProvider::RouterDownloadScript(RouterDownloadScript {
            version: Field::Resolved(version.to_string()),
        });

        assert_resolve_and_write_success(
            router_download,
            &target,
            &src,
            &mut ctx,
            expected_content,
        )
        .await;
    }

    #[tokio::test]
    async fn resolve_and_write_router_download_not_found_error() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("router-download.sh");

        let version = "v22.12.6";
        let url = format!("https://router.apollo.dev/download/nix/{version}");
        let expected_err = format!("Unknown router version: {version}");

        let responses = &[(url.as_str(), "404", "Not found")];

        let mut ctx = MockContext::with_http_client(responses);
        let src = Source::Local {
            abs_path: PathBuf::new(),
        };

        let router_download = FileProvider::RouterDownloadScript(RouterDownloadScript {
            version: Field::Resolved(version.to_string()),
        });

        assert_resolve_and_write_error(router_download, &target, &src, &mut ctx, &expected_err)
            .await
    }

    #[tokio::test]
    async fn resolve_and_write_router_from_source_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("build-from-source.sh");

        let mut ctx = Context::new();
        let src = Source::Local {
            abs_path: PathBuf::new(),
        };

        let expected_content = indoc!(
            r#"
            mkdir router-source && \
            cd router-source && \
            git clone https://github.com/apollographql/router.git && \
            cd router && \
            git checkout git_ref && \
            rustup toolchain install 1.90.0 && \
            rustup run 1.90.0 cargo build --release && \
            cp ${CARGO_TARGET_DIR}/release/router ~/.cargo/bin/"#
        );
        let router_from_source = FileProvider::BuildRouterFromSource(BuildRouterFromSource {
            git_ref: Field::Resolved("git_ref".to_string()),
            rust_version: Field::Resolved("1.90.0".to_string()),
        });

        assert_resolve_and_write_success(
            router_from_source,
            &target,
            &src,
            &mut ctx,
            expected_content,
        )
        .await;
    }
}
