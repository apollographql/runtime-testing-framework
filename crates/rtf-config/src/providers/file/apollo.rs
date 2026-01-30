// Unit tests for the parsing and validation of the providers in this file
// are part of the suite of tests in the mod.rs file
use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    inlining,
    providers::{
        self,
        file::{AsUtf8FileContent, DirFile, InlineDir, ResolveFileContent},
    },
    templating::Field,
};
use indoc::indoc;
use itertools::Itertools;
use reqwest::StatusCode;
use rtf_derive::Template;
use rtf_integrations::{
    HttpClient,
    graphos::supergraph::{
        Subgraph, SupergraphDetails,
        operations::{
            canned_operations::{CannedOperation, canned_ops_for_ids, top_studio_canned_ops},
            fetch_offline_license,
        },
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::warn;

/// # GraphOS supergraph SDL
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
    /// Replace the supergraph's connector urls with overridden values for testing.
    ///
    /// Defaults to null if unset.
    #[serde(default)]
    #[template(skip)]
    pub with_connector_overrides: Option<UrlFormat>,
}

impl GraphosSupergraph {
    fn content_from_details(&self, mut sg: Arc<SupergraphDetails>) -> String {
        if let Some(url_format) = &self.with_subgraph_overrides {
            let sg = Arc::make_mut(&mut sg);
            let subgraph_urls = url_format.urls_for_subgraphs(&sg.subgraphs);
            sg.rewrite_subgraph_urls(&subgraph_urls)
                .expect("unable to rewrite subgraph URLs");
        }

        if let Some(connector_format) = &self.with_connector_overrides {
            let sg = Arc::make_mut(&mut sg);
            let connector_url = connector_format.connector_base_url();
            sg.rewrite_connector_urls(&connector_url)
                .expect("unable to rewrite connector URLs");
        }

        sg.supergraph_sdl.clone()
    }
}
impl AsUtf8FileContent for GraphosSupergraph {
    async fn try_get_file_content(
        &self,
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
/// Note that this file provider will output a directory of SDL schema files, one for each
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
    fn content_from_details(&self, sg: Arc<SupergraphDetails>, dir: &Path) -> Vec<DirFile> {
        let contents: Vec<_> = sg
            .subgraphs
            .clone()
            .into_iter()
            .map(|sg| DirFile {
                path: dir.join(sg.name).with_extension("graphql"),
                content: sg.sdl,
            })
            .collect();

        contents
    }

    pub(crate) async fn inline(&self, ctx: &impl ResolutionContext) -> inlining::Result<InlineDir> {
        // The target in try_get_all_file_contents is used to prefix the actual file paths
        // We are not interested in that here so we set a new PathBuf so we just get the file name
        // as <subgraph_name>.graphql
        let files = self.try_get_all_file_contents(PathBuf::new(), ctx).await?;

        Ok(InlineDir { files })
    }
}

impl ResolveFileContent for GraphosSubgraphs {
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<DirFile>> {
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
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS subgraph names
///
/// The user specifies the graph ref that should be used to fetch the names of
/// subgraphs in the supergraph from the GraphOS API.
///
/// This file provider will output a newline-delimited file of the subgraph names.
///
/// ```yaml
/// - name: "subgraph_names"
///   env_var: SUBGRAPH_NAMES
///   kind: graphos_subgraph_names
///   graph_ref: graph@variant
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct GraphosSubgraphNames {
    /// The Apollo graph ref to pull subgraph names for.
    pub graph_ref: Field<String>,
}

impl AsUtf8FileContent for GraphosSubgraphNames {
    async fn try_get_file_content(
        &self,
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

        let contents = subgraphs.into_iter().map(|sg| sg.name).join("\n");

        Ok(contents)
    }
}

impl Check for GraphosSubgraphNames {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS supergraph Router URL overrides
///
/// The user specifies the graph ref that should be used to fetch subgraph
/// SDL files from the GraphOS API and generates a the override_subgraph_urls
/// YAML snippet that can be merged into a router config file.
///
/// This should be used whenever subgraph requests need to be mapped to a mock
/// server instead of hitting the real subgraph as defined in the supergraph,
/// which is typically desirable behavior when working with real graphs.
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
    pub url_format: UrlFormat,
}

fn default_url_format() -> UrlFormat {
    UrlFormat::Localhost
}

/// The allowed url formats for the [GraphosSubgraphRouterUrlOverrides]
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(rename_all = "snake_case")]
pub enum UrlFormat {
    /// Overrides to `http://localhost:<port>` for each subgraph.
    ///
    /// Port is defined as `4001 + n` where `n` is the nth subgraph, starting at 0.
    Localhost,
    /// Overrides to `http://loadbalancer:8080`.
    Docker,
    /// Accepts a custom formatting configuration that will define the URLs.
    Custom(CustomUrlFormat),
}

impl UrlFormat {
    fn urls_for_subgraphs(&self, subgraphs: &[Subgraph]) -> HashMap<String, String> {
        let mut subgraph_urls: HashMap<String, String> = HashMap::new();

        match self {
            UrlFormat::Localhost => {
                let base_port = 4001;
                for (offset, sg) in subgraphs.iter().enumerate() {
                    let port = base_port + offset;

                    let url = Self::format(&sg.name, "http://localhost", port, false);
                    subgraph_urls.insert(sg.name.clone(), url);
                }
            }
            // The docker compose generated by rtf puts all subgraph containers behind a loadbalancer
            UrlFormat::Docker => {
                let url = docker_base_url().as_resolved().to_owned();
                let port = *docker_port().as_resolved();

                for sg in subgraphs {
                    subgraph_urls
                        .insert(sg.name.clone(), Self::format(&sg.name, &url, port, false));
                }
            }
            UrlFormat::Custom(config) => {
                let base_url = config.base_url.as_resolved();
                let base_port = *config.base_port.as_resolved();
                let add_subgraph_route = *config.add_subgraph_route.as_resolved();
                let custom_subgraph_urls: HashMap<String, String> = config
                    .custom_subgraph_urls
                    .iter()
                    .map(|(k, v)| (k.to_owned(), v.as_resolved().to_owned()))
                    .collect();

                if *config.increment_port.as_resolved() {
                    for (offset, sg) in subgraphs.iter().enumerate() {
                        let url =
                            custom_subgraph_urls
                                .get(&sg.name)
                                .cloned()
                                .unwrap_or_else(|| {
                                    Self::validate_and_format(
                                        &custom_subgraph_urls,
                                        &sg.name,
                                        base_url,
                                        base_port + offset,
                                        add_subgraph_route,
                                    )
                                });

                        subgraph_urls.insert(sg.name.clone(), url);
                    }
                } else {
                    for sg in subgraphs {
                        let url =
                            custom_subgraph_urls
                                .get(&sg.name)
                                .cloned()
                                .unwrap_or_else(|| {
                                    Self::validate_and_format(
                                        &custom_subgraph_urls,
                                        &sg.name,
                                        base_url,
                                        base_port,
                                        add_subgraph_route,
                                    )
                                });

                        subgraph_urls.insert(sg.name.clone(), url);
                    }
                };
            }
        };

        subgraph_urls
    }

    fn validate_and_format(
        custom_subgraph_urls: &HashMap<String, String>,
        name: &str,
        base_url: &str,
        port: usize,
        add_subgraph_route: bool,
    ) -> String {
        let generated = Self::format(name, base_url, port, add_subgraph_route);
        if let Some((ovr_name, _url)) = custom_subgraph_urls
            .iter()
            .find(|(_ovr_name, url)| **url == generated)
        {
            warn!(
                "Generated subgraph URL ({generated}) for {name} collides with the custom override for {ovr_name}. Traffic for both subgraphs will be routed to the same URL."
            );
        }
        generated
    }

    fn format(name: &str, base_url: &str, port: usize, add_subgraph_route: bool) -> String {
        if add_subgraph_route {
            format!("{base_url}:{port}/{name}")
        } else {
            format!("{base_url}:{port}")
        }
    }

    /// Generate the base URL for connector mock services.
    ///
    /// Unlike subgraphs which may have different URLs per subgraph, connectors
    /// all point to the same connector-mock service.
    fn connector_base_url(&self) -> String {
        match self {
            UrlFormat::Localhost => "http://localhost:3000".to_string(),
            UrlFormat::Docker => "http://connector:3000".to_string(),
            UrlFormat::Custom(config) => {
                let base_url = config.base_url.as_resolved();
                let base_port = *config.base_port.as_resolved();
                format!("{base_url}:{base_port}")
            }
        }
    }
}

/// Defines a custom URL format to route subgraph requests to.
///
/// The `base_url` and `base_port` components define the base subgraph URL.
/// These produce a subgraph URL of `{base_url}:{base_port}` for all subgraph names as the default.
///
/// If `increment_port` is set to true, each subgraph will get a distinct port number
/// generated via auto-incrementing the `base_port` for each known subgraph. This is useful
/// for running `n` subgraph servers for `n` named subgraphs in the supergraph.
///
/// To use a single subgraph server with defined per-subgraph behavior, set `add_subgraph_route`
/// to true. This will append a route with the subgraph's name to the base URL, producing the form
/// `{base_url}:{base_port}/{subgraph_name}` for each subgraph.
///
/// ```yaml
/// - name: subgraph-url-overrides.yaml
///   env_var: SUBGRAPH_URL_OVERRIDES
///   kind: graphos_subgraph_router_url_overrides
///   graph_ref: graph@variant
///   url_format:
///     base_url: "http://loadbalancer"
///     base_port: "8080"
///     increment_port: false
///     add_subgraph_route: false
///     custom_subgraph_urls:
///       my_subgraph: "http://my-subgraph-instance:8081"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct CustomUrlFormat {
    /// The base URL to route subgraph requests to.
    ///
    /// Defaults to the [UrlFormat::Docker] format if not set.
    #[serde(default = "docker_base_url")]
    base_url: Field<String>,
    /// The base port that the subgraph requests should use
    ///
    /// Defaults to the [UrlFormat::Docker] port if not set.
    #[serde(default = "docker_port")]
    base_port: Field<usize>,
    /// Whether or not to increment the port number from the base for each subgraph.
    ///
    /// Defaults to false if unset.
    #[serde(default)]
    increment_port: Field<bool>,
    /// Whether or not to include a `/{subgraph_name}` route for each subgraph.
    ///
    /// Defaults to false if unset.
    #[serde(default)]
    add_subgraph_route: Field<bool>,
    /// Custom subgraph URL overrides for routes that do not fit the structure built by the above parameters.
    #[serde(default)]
    custom_subgraph_urls: HashMap<String, Field<String>>,
}

fn docker_base_url() -> Field<String> {
    Field::Resolved("http://loadbalancer".to_owned())
}

fn docker_port() -> Field<usize> {
    Field::Resolved(8080)
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
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        validate_graph_ref_and_client(self.graph_ref.as_resolved(), path, ctx)
    }
}

/// # GraphOS canned operations
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
///   time_range: 7d
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
    /// How far back to query for operations.
    ///
    /// Accepts duration strings like "30d", "7d", "12h".
    /// Defaults to "30d" if unset.
    #[serde(default = "default_time_range")]
    pub time_range: Field<String>,
}

fn default_top_n() -> Field<usize> {
    Field::Resolved(20)
}

fn default_time_range() -> Field<String> {
    Field::Resolved("30d".to_string())
}

impl AsUtf8FileContent for GraphosCannedOps {
    async fn try_get_file_content(
        &self,
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
        let from_seconds = -(humantime::parse_duration(self.time_range.as_resolved())
            .expect("validated time_range")
            .as_secs() as i64);
        let canned_ops = top_studio_canned_ops(
            &details,
            *self.top_n.as_resolved(),
            *self.skip_mutations.as_resolved(),
            from_seconds,
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
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();
        errs.append(validate_graph_ref_and_client(
            self.graph_ref.as_resolved(),
            path,
            ctx,
        ));

        if let Err(e) = humantime::parse_duration(self.time_range.as_resolved()) {
            errs.push(checks::ErrorKind::InvalidDuration, e.to_string(), path);
        }

        errs.into_result(())
    }
}

/// # GraphOS canned operations by ID
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

/// # GraphOS offline license
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

/// # Router download script
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
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}

/// # Build Router from source
///
/// A file provider used for building the Router from source at a specific git commit
/// or reference. A profile and list of features can optionally be provided.
///
/// ```yaml
/// - name: "router-build.sh"
///   env_var: ROUTER_BUILD_SCRIPT
///   kind: build_router_from_source
///   git_ref: "some-ref"
///   rust_version: "1.89.0"
///   profile: "release"
///   features: "default"
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

    /// The profile to build the Router with.
    ///
    /// Defaults to `"release"` if unset.
    #[serde(default = "default_profile")]
    pub(crate) profile: Field<String>,

    /// Comma separated list of features to build the Router with.
    ///
    /// Defaults to `"default"` if unset.
    #[serde(default = "default_features")]
    pub(crate) features: Field<String>,
}

fn default_rust_version() -> Field<String> {
    Field::Resolved("stable".to_string())
}

fn default_profile() -> Field<String> {
    Field::Resolved("release".to_string())
}

fn default_features() -> Field<String> {
    Field::Resolved("default".to_string())
}

impl AsUtf8FileContent for BuildRouterFromSource {
    async fn try_get_file_content(
        &self,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let commit_ref = self.git_ref.as_resolved();
        let rust_version = self.rust_version.as_resolved();
        let profile = self.profile.as_resolved();
        let features = self.features.as_resolved();

        let install_script = format!(
            indoc!(
                r#"mkdir router-source && \
                cd router-source && \
                git clone https://github.com/apollographql/router.git && \
                cd router && \
                git checkout {} && \
                rustup toolchain install {} && \
                rustup run {} cargo build --profile {} --features {} && \
                cp ${{CARGO_TARGET_DIR}}/{}/router ~/.cargo/bin/"#
            ),
            commit_ref, rust_version, rust_version, profile, features, profile
        );

        Ok(install_script)
    }
}

impl Check for BuildRouterFromSource {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
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
        inlining::InlineMode,
        mock_context::MockContext,
        providers::file::{
            FileProvider, InlineFile,
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
            with_connector_overrides: None,
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

    /// Create a GraphOS Subgraph Names
    fn subgraph_names_fp(graph_ref: &str) -> FileProvider {
        FileProvider::GraphosSubgraphNames(subgraph_names(graph_ref))
    }
    fn subgraph_names(graph_ref: &str) -> GraphosSubgraphNames {
        GraphosSubgraphNames {
            graph_ref: Field::Resolved(graph_ref.to_string()),
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
            time_range: Field::Resolved("30d".to_string()),
        })
    }

    /// Create a GraphOS Canned Ops with a specific time_range
    fn canned_ops_with_time_range(time_range: &str) -> GraphosCannedOps {
        GraphosCannedOps {
            graph_ref: Field::Resolved("graph@variant".to_string()),
            top_n: Field::Resolved(20),
            skip_mutations: Field::Resolved(true),
            time_range: Field::Resolved(time_range.to_string()),
        }
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
    #[test_case(subgraphs_overrides_fp("graph@variant"), true, &[]; "subgraphs overrides success")]
    #[test_case(subgraphs_overrides_fp("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "subgraphs overrides invalid ref")]
    #[test_case(subgraphs_overrides_fp("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "subgraphs overrides missing key")]
    #[test_case(subgraphs_overrides_fp("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "subgraphs overrides invalid ref and missing key")]
    #[test_case(subgraph_names_fp("graph@variant"), true, &[]; "subgraph names success")]
    #[test_case(subgraph_names_fp("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "subgraph names invalid ref")]
    #[test_case(subgraph_names_fp("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "subgraph names missing key")]
    #[test_case(subgraph_names_fp("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "subgraph names invalid ref and missing key")]
    #[test_case(canned_ops("graph@variant"), true, &[]; "canned ops success")]
    #[test_case(canned_ops("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "canned ops invalid ref")]
    #[test_case(canned_ops("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "canned ops missing key")]
    #[test_case(canned_ops("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "canned ops invalid ref and missing key")]
    #[test_case(canned_ops_by_id("graph@variant"), true, &[]; "canned ops by id success")]
    #[test_case(canned_ops_by_id("not a valid ref"), true, &[ErrorKind::InvalidGraphRef]; "canned ops by id invalid ref")]
    #[test_case(canned_ops_by_id("graph@variant"), false, &[ErrorKind::MissingGraphOsApiKey]; "canned ops by id missing key")]
    #[test_case(canned_ops_by_id("not a valid ref"), false, &[ErrorKind::InvalidGraphRef, ErrorKind::MissingGraphOsApiKey]; "canned ops by id invalid ref and missing key")]
    #[test]
    fn graphos_providers_check_graph_ref(
        fp: FileProvider,
        with_platform_config: bool,
        expected_err_kinds: &[ErrorKind],
    ) {
        let mut ctx = Context::new();
        if with_platform_config {
            ctx.with_platform_config("dummy_key", false, false);
        }

        if !expected_err_kinds.is_empty() {
            assert_check_errors(fp, &ctx, expected_err_kinds);
        } else {
            let res = fp.try_check(&mut Vec::new(), &ctx);
            assert!(res.is_ok(), "expected check to succeed, got {res:?}");
        }
    }

    #[test_case("30d"; "days")]
    #[test_case("7d"; "week")]
    #[test_case("12h"; "hours")]
    #[test_case("30m"; "minutes")]
    #[test_case("1d 12h"; "compound")]
    #[test]
    fn canned_ops_check_valid_time_range(time_range: &str) {
        let canned_ops = canned_ops_with_time_range(time_range);

        let mut ctx = Context::new();
        ctx.with_platform_config("dummy_key", false, false);

        let res = canned_ops.try_check(&mut Vec::new(), &ctx);
        assert!(
            res.is_ok(),
            "expected check to succeed for '{time_range}', got {res:?}"
        );
    }

    #[test_case("not a duration"; "invalid string")]
    #[test_case("30"; "missing unit")]
    #[test_case("-7d"; "negative duration")]
    #[test]
    fn canned_ops_check_invalid_time_range(time_range: &str) {
        let canned_ops = canned_ops_with_time_range(time_range);

        let mut ctx = Context::new();
        ctx.with_platform_config("dummy_key", false, false);

        assert_check_errors(canned_ops, &ctx, &[ErrorKind::InvalidDuration]);
    }

    #[test]
    fn canned_ops_time_range_defaults_to_30d() {
        let yaml = indoc!(
            r#"
            kind: graphos_canned_ops
            graph_ref: graph@variant
            "#
        );

        let provider: FileProvider = serde_yaml::from_str(yaml).expect("valid yaml");
        let FileProvider::GraphosCannedOps(canned_ops) = provider else {
            panic!("expected GraphosCannedOps variant");
        };

        assert_eq!(canned_ops.time_range.as_resolved(), "30d");
    }

    #[test]
    fn offline_license_check_success() {
        let offline = OfflineGraphosLicense {
            graph_id: Field::Resolved("graph".to_string()),
        };

        let mut ctx = Context::new();
        ctx.with_platform_config("dummy_key", false, false);

        let res = offline.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn offline_license_check_missing_api_key() {
        let offline = OfflineGraphosLicense {
            graph_id: Field::Resolved("graph".to_string()),
        };

        let ctx = Context::new();

        assert_check_errors(offline, &ctx, &[ErrorKind::MissingGraphOsApiKey]);
    }

    #[test]
    fn router_download_script_check_success() {
        let router_download = RouterDownloadScript {
            version: Field::Resolved("v2.0.0".to_string()),
        };

        let ctx = Context::new();

        let res = router_download.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn build_router_from_source_check_success() {
        let build_from_source = BuildRouterFromSource {
            git_ref: Field::Resolved("ref".to_string()),
            rust_version: Field::Resolved("1.90.0".to_string()),
            profile: Field::Resolved("release".to_string()),
            features: Field::Resolved("default".to_string()),
        };

        let ctx = Context::new();

        let res = build_from_source.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[tokio::test]
    async fn supergraph_resolve_success() {
        let details = supergraph_details();
        let supergraph = supergraph("graph@variant");

        let expected_content = supergraph_sdl();
        let res = supergraph.content_from_details(details);
        assert_eq!(res, expected_content)
    }

    #[tokio::test]
    async fn supergraph_resolve_with_docker_overrides_success() {
        let details = supergraph_details();
        let supergraph = GraphosSupergraph {
            graph_ref: Field::Resolved("graph@variant".to_string()),
            with_subgraph_overrides: Some(UrlFormat::Docker),
            with_connector_overrides: None,
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
    async fn supergraph_resolve_with_custom_overrides_success() {
        let details = supergraph_details();
        let supergraph = GraphosSupergraph {
            graph_ref: Field::Resolved("graph@variant".to_string()),
            with_subgraph_overrides: Some(UrlFormat::Custom(CustomUrlFormat {
                base_url: Field::Resolved("http://my-subgraph".to_string()),
                base_port: Field::Resolved(8000),
                increment_port: Field::Resolved(false),
                add_subgraph_route: Field::Resolved(false),
                custom_subgraph_urls: vec![(
                    "bar".to_string(),
                    Field::Resolved("http://my-other-subgraph:8081".to_string()),
                )]
                .into_iter()
                .collect(),
            })),
            with_connector_overrides: None,
        };

        // The supergraph file indentation is transformed so that assert_eq is not possible
        // Instead, we check that the foo and bar url references are as expected
        let expected_foo_url = r#"FOO @join__graph(name: "foo", url: "http://my-subgraph:8000")"#;
        let expected_bar_url =
            r#"BAR @join__graph(name: "bar", url: "http://my-other-subgraph:8081")"#;

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
    async fn supergraph_resolve_with_custom_override_collision_success() {
        let details = supergraph_details();
        let supergraph = GraphosSupergraph {
            graph_ref: Field::Resolved("graph@variant".to_string()),
            with_subgraph_overrides: Some(UrlFormat::Custom(CustomUrlFormat {
                base_url: Field::Resolved("http://my-subgraph".to_string()),
                base_port: Field::Resolved(8000),
                increment_port: Field::Resolved(false),
                add_subgraph_route: Field::Resolved(false),
                custom_subgraph_urls: vec![(
                    "bar".to_string(),
                    Field::Resolved("http://my-subgraph:8000".to_string()),
                )]
                .into_iter()
                .collect(),
            })),
            with_connector_overrides: None,
        };

        // The supergraph file indentation is transformed so that assert_eq is not possible
        // Instead, we check that the foo and bar url references are as expected
        let expected_foo_url = r#"FOO @join__graph(name: "foo", url: "http://my-subgraph:8000")"#;
        let expected_bar_url = r#"BAR @join__graph(name: "bar", url: "http://my-subgraph:8000")"#;

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
    async fn supergraph_resolve_with_localhost_overrides_success() {
        let details = supergraph_details();
        let supergraph = GraphosSupergraph {
            graph_ref: Field::Resolved("graph@variant".to_string()),
            with_subgraph_overrides: Some(UrlFormat::Localhost),
            with_connector_overrides: None,
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
    async fn subgraphs_resolve_success() {
        let details = supergraph_details();
        let subgraphs = subgraphs("graph@variant");

        let base_path = Path::new("subgraphs");

        let expected_content: Vec<DirFile> = vec![
            DirFile {
                path: base_path.join("foo.graphql"),
                content: subgraph_foo().to_string(),
            },
            DirFile {
                path: base_path.join("bar.graphql"),
                content: subgraph_bar().to_string(),
            },
        ];

        let res = subgraphs.content_from_details(details, base_path);
        assert_eq!(res, expected_content)
    }

    #[tokio::test]
    async fn subgraph_url_overrides_resolve_success() {
        let details = supergraph_details();
        let subgraphs_overrides = subgraphs_overrides("graph@variant");

        let expected_content = "override_subgraph_url:\n  bar: http://loadbalancer:8080\n  foo: http://loadbalancer:8080\n";

        let res = subgraphs_overrides.content_from_details(details);
        assert!(res.is_ok(), "expected String, got {res:?}");
        assert_eq!(res.unwrap(), expected_content);
    }

    #[test]
    fn canned_ops_format_json_correctly() {
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
    async fn router_download_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("router-download.sh");

        let version = "v2.6.0";
        let url = format!("https://router.apollo.dev/download/nix/{version}");
        let expected_content = "router download script";

        let responses = &[(url.as_str(), "200", expected_content)];

        let mut ctx = MockContext::with_http_client(responses);
        let router_download = FileProvider::RouterDownloadScript(RouterDownloadScript {
            version: Field::Resolved(version.to_string()),
        });

        assert_resolve_and_write_success(router_download, &target, &mut ctx, expected_content)
            .await;
    }

    #[tokio::test]
    async fn router_download_resolve_and_write_not_found_error() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("router-download.sh");

        let version = "v22.12.6";
        let url = format!("https://router.apollo.dev/download/nix/{version}");
        let expected_err = format!("Unknown router version: {version}");

        let responses = &[(url.as_str(), "404", "Not found")];

        let mut ctx = MockContext::with_http_client(responses);

        let router_download = FileProvider::RouterDownloadScript(RouterDownloadScript {
            version: Field::Resolved(version.to_string()),
        });

        assert_resolve_and_write_error(router_download, &target, &mut ctx, &expected_err).await
    }

    #[tokio::test]
    async fn router_from_source_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("build-from-source.sh");

        let mut ctx = Context::new();

        let expected_content = indoc!(
            r#"
            mkdir router-source && \
            cd router-source && \
            git clone https://github.com/apollographql/router.git && \
            cd router && \
            git checkout git_ref && \
            rustup toolchain install 1.90.0 && \
            rustup run 1.90.0 cargo build --profile release --features default && \
            cp ${CARGO_TARGET_DIR}/release/router ~/.cargo/bin/"#
        );
        let router_from_source = FileProvider::BuildRouterFromSource(BuildRouterFromSource {
            git_ref: Field::Resolved("git_ref".to_string()),
            rust_version: Field::Resolved("1.90.0".to_string()),
            profile: Field::Resolved("release".to_string()),
            features: Field::Resolved("default".to_string()),
        });

        assert_resolve_and_write_success(router_from_source, &target, &mut ctx, expected_content)
            .await;
    }

    #[test]
    fn connector_base_url_localhost() {
        let url_format = UrlFormat::Localhost;
        assert_eq!(url_format.connector_base_url(), "http://localhost:3000");
    }

    #[test]
    fn connector_base_url_docker() {
        let url_format = UrlFormat::Docker;
        assert_eq!(url_format.connector_base_url(), "http://connector:3000");
    }

    #[test]
    fn connector_base_url_custom() {
        let url_format = UrlFormat::Custom(CustomUrlFormat {
            base_url: Field::Resolved("http://my-connector".to_string()),
            base_port: Field::Resolved(4000),
            increment_port: Field::Resolved(false),
            add_subgraph_route: Field::Resolved(false),
            custom_subgraph_urls: HashMap::new(),
        });
        assert_eq!(url_format.connector_base_url(), "http://my-connector:4000");
    }

    #[tokio::test]
    async fn router_download_script_inline_all_files_success() {
        let version = "v2.6.0";
        let url = format!("https://router.apollo.dev/download/nix/{version}");
        let expected_content = "router download script content";

        let responses = &[(url.as_str(), "200", expected_content)];
        let ctx = MockContext::with_http_client(responses);

        let mut provider = FileProvider::RouterDownloadScript(RouterDownloadScript {
            version: Field::Resolved(version.to_string()),
        });

        let res = provider.inline(&InlineMode::All, &ctx).await;
        assert!(res.is_ok(), "expected provider to inline, got {res:?}");

        let expected_inline_provider = FileProvider::Inline(InlineFile {
            content: expected_content.to_string(),
        });
        assert_eq!(
            provider, expected_inline_provider,
            "expected inline file provider with router download script content"
        );
    }

    #[tokio::test]
    async fn build_router_from_source_inline_all_files_success() {
        let ctx = Context::new();

        let expected_content = indoc!(
            r#"
            mkdir router-source && \
            cd router-source && \
            git clone https://github.com/apollographql/router.git && \
            cd router && \
            git checkout git_ref && \
            rustup toolchain install 1.90.0 && \
            rustup run 1.90.0 cargo build --profile release --features default && \
            cp ${CARGO_TARGET_DIR}/release/router ~/.cargo/bin/"#
        );

        let mut provider = FileProvider::BuildRouterFromSource(BuildRouterFromSource {
            git_ref: Field::Resolved("git_ref".to_string()),
            rust_version: Field::Resolved("1.90.0".to_string()),
            profile: Field::Resolved("release".to_string()),
            features: Field::Resolved("default".to_string()),
        });

        let res = provider.inline(&InlineMode::All, &ctx).await;
        assert!(res.is_ok(), "expected provider to inline, got {res:?}");

        let expected_inline_provider = FileProvider::Inline(InlineFile {
            content: expected_content.to_string(),
        });
        assert_eq!(
            provider, expected_inline_provider,
            "expected inline file provider with build script content"
        );
    }
}
