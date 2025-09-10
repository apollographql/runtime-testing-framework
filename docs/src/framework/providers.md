# File Providers

Available file providers:

- [Build Router From Source](#build-router-from-source)
- [GitHub File](#github-file)
- [GraphOS Canned Operations](#graphos-canned-operations)
- [GraphOS Supergraph Docker Compose](#graphos-supergraph-docker-compose)
- [GraphOS Supergraph Router URL Overrides](#graphos-supergraph-router-url-overrides)
- [GraphOS subgraph SDL](#graphos-subgraph-sdl)
- [GraphOS Supergraph SDL](#graphos-supergraph-sdl)
- [Inline File](#inline-file)
- [GraphOS Offline License](#graphos-offline-license)
- [Relative Path](#relative-path)
- [Required File](#required-file)
- [Resolved Values](#resolved-values)
- [Router Download Script](#router-download-script)
- [Merge YAML](#merge-yaml)

## Build Router From Source

A file provider used for building the Router from source at a specific git commit or reference.

```yaml
- name: "router-build.sh"
  env_var: ROUTER_BUILD_SCRIPT
  kind: build_router_from_source
  commit_ref: "some-ref"
  rust_version: "1.89.0"
```

### Fields

#### `commit_ref`

A git reference that can be passed to `git checkout`. This may be a full or partial commit hash,
branch name, or tag.

Defaults to `"main"` if unset.

#### `rust_version`

A Rust version string that can be passed to `rustup run {rust_version}`, such as `"1.78.0"`,
`"beta"`, or `"nightly"`.

Defaults to `"stable"` if unset.

## GitHub File

The user specifies a path to a file within a GitHub repository, optionally providing a specific ref
of the repository to pull the file from. If no ref is providing then the provider will pull the
version of the file found on the default branch.

```yaml
- name: "my-file.txt"
  env_var: MY_FILE
  kind: github_file
  org: "my-org"
  repo: "my-repo"
  path: "resources/test-data/my-file.txt"
  git_ref: "some-ref"
```

### Fields

#### `org`

The GitHub org for the repository containing the target file

#### `repo`

The GitHub repository containing the target file

#### `path`

The absolute path from the root of the repository to the target file

#### `git_ref`

An optional git reference to pull the file from. This may be a full or partial commit hash, branch
name, or tag.

Defaults to the mainline branch as specified in GitHub if unset.

## GraphOS Canned Operations

The user specifies the graph ref and parameters that should be used to generate canned GraphQL
requests based on operations data obtained from the GraphOS API.

```yaml
- name: canned_ops.json
  env_var: CANNED_OPS_FILE
  kind: graphos_canned_ops
  graph_ref: graph@variant
  top_n: 10
  skip_mutations: true
```

### Fields

#### `graph_ref`

The Apollo graph ref to pull operations for.

#### `top_n`

The number of operations to attempt to fetch.

Defaults to 20 if unset.

#### `skip_mutations`

Whether or not to include mutations in the returned operations.

Defaults to false if unset.

## GraphOS Supergraph Docker Compose

The user specifies the graph ref that should be used to fetch the supergraph SDL file from the
GraphOS API and generates a docker compose file. It runs a configurable number of subgraph services,
mocking based on the supergraph schema behind a loadbalancer.

```yaml
- name: "subgraph-compose.yaml"
  env_var: SUBGRAPH_COMPOSE
  kind: graphos_subgraph_docker_compose
  graph_ref: graph@variant
  image: ghcr.io/apollographql/runtime-testing-framework/router-scale-subgraph:main
  command:
  - -schema
  - /app/supergraph.graphql
  replicas: 5
  resource_limits:
    cpus: '0.5'
    memory: 1G
  resource_reservations:
   cpus: '0.1'
    memory: 512M
  mem_swappiness: 0
  loadbalancer:
    resource_limits:
      cpus: '0.5'
      memory: 1G
    resource_reservations:
      cpus: '0.1'
      memory: 512M
    mem_swappiness: 0
```

### Fields

#### `graph_ref`

The Apollo graph ref to pull the supergraph for.

#### `image`

The image the subgraph service runs.

Defaults to ghcr.io/apollographql/runtime-testing-framework/router-scale-subgraph:main if unset.

#### `command`

The command that subgraph server image runs.

Defaults to "-schema /app/supergraph.graphql" if unset.

#### `replicas`

The number of subgraph services containers running.

Defaults to 5 if unset.

#### `resource_limits`

The resource limits for the subgraph containers.

Defaults to cpus=0.5 and memory=1G if unset.

#### `resource_reservations`

The reserved resources for the subgraph containers.

Defaults to cpus=0.1 and memory=512M if unset.

#### `mem_swappiness`

Enable or disable memory swapping in the subgraph services.

Defaults to 0 (disabled) if unset.

#### `loadbalancer`

The configuration for the subgraph's loadbalancer.

## GraphOS Supergraph Router URL Overrides

The user specifies the graph ref that should be used to fetch subgraph SDL files from the GraphOS
API and generates a the override_subgraph_urls YAML snippet that can be merged into a router config
file

This should be used when generating the subgraph docker compose using
[GraphosSubgraphDockerCompose]. This will ensure the router subgraph urls map to the loadbalancer
url in that compose file.

```yaml
- name: subgraph-url-overrides.yaml
  env_var: SUBGRAPH_URL_OVERRIDES
  kind: graphos_subgraph_router_url_overrides
  graph_ref: graph@variant
  url_format: localhost
```

### Fields

#### `graph_ref`

The Apollo graph ref to pull the subgraphs for.

#### `url_format`

The format of the overrides url.

## GraphOS subgraph SDL

The user specifies the graph ref that should be used to fetch a subgraph SDL files from the GraphOS
API.

Note that this file proivider will output a directory of SDL schema files, one for each subgraph.

```yaml
- name: "subgraphs"
  env_var: SUBGRAPHS
  kind: graphos_subgraphs
  graph_ref: graph@variant
```

### Fields

#### `graph_ref`

The Apollo graph ref to pull subgraph SDL files for.

## GraphOS Supergraph SDL

The user specifies the ref that should be used to fetch a supergraph SDL file from the GraphOS API.

```yaml
- name: "supergraph.graphql"
  env_var: SUPERGRAPH
  kind: graphos_supergraph
  graph_ref: graph@variant
  with_subgraph_overrides: docker
```

### Fields

#### `graph_ref`

The Apollo graph ref to pull supergraph SDL for.

#### `with_subgraph_overrides`

Replace the supergraph's subgraph urls with overridden values for testing.

Defaults to null if unset.

## Inline File

The simplest form of file provider: the user specifies the contents of the file inline within their
config file.

```yaml
- name: "my-file.txt"
  env_var: MY_FILE
  kind: inline
  content: |
    my raw file content.
    specified inline within an RTF config file.
```

### Fields

#### `content`

The text to write out as the contents of the generated file.

## GraphOS Offline License

The user specifies the graph id that should be used to fetch an offline license from the GraphOS
API.

```yaml
- name: license.jwt
  env_var: LICENSE
  kind: graphos_offline_license
  graph_id: graph
```

### Fields

#### `graph_id`

The Apollo graph ref to pull an offline license for.

## Relative Path

A relative path from the containing config file to a target file that should be made available as
part of the test run. This provider works both with local files and files within GitHub if the
containing config file was pulled from a repository.

```yaml
- name: "my-file.txt"
  env_var: MY_FILE
  kind: relative_path
  path: "../../resources/test-data/my-file.txt"
```

### Fields

#### `path`

The relative path from the containing config file to the target file.

#### `src`

Set during TestPlan parsing as part of overrides. This should only ever be `Some` if this provider
was defined as part of an `overrides` section in the test plan.

## Required File

The only purpose of this file provider is to throw an error if it still exists when the file
providers are being checked. All definitions of a required file are expected to be replaced by user
defined file providers.

```yaml
- name: "router-config.yaml"
  env_var: ROUTER_CONFIG
  kind: required
  message: "you must specify a router config file to use"
```

### Fields

#### `message`

The error message to display to the user if this provider is not overwritten.

## Resolved Values

Returns the JSON string representation of the resolved values for the test plan being run.

```yaml
- name: "resolved-values.json"
  env_var: VALUES
  kind: resolved_values
```

## Router Download Script

Produces a POSIX shell script that can be run in order to download a target version of the Apollo
Router.

```yaml
- name: "router-download.sh"
  env_var: ROUTER_DOWNLOAD
  kind: router_download_script
  version: "v2.6.0"
```

### Fields

#### `version`

The version of the Apollo Router to download.

## Merge YAML

Merge the YAML output of two text based file providers into a single YAML file.

Matching keys in the overrides file will replace scalar values, concatenate arrays and merge keys
for maps.

```yaml
- name: router-config.yaml
  env_var: ROUTER_CONFIG
  kind: merge_yaml
  base:
    kind: relative_path
    path: "data/base-router-config.yaml"
  overrides:
    kind: graphos_subgraph_router_url_overrides
    graph_ref: "foo@bar"
    url_format: "docker"
```

### Fields

#### `base`

A base YAML file to start with.

#### `overrides`

An second YAML file to merge on top of the base file.
