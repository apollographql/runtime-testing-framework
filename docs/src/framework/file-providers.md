# File Providers

Available file providers:

- [Build Router From Source](#build-router-from-source)
- [From command](#from-command)
- [GitHub File](#github-file)
- [GraphOS Canned Operations](#graphos-canned-operations)
- [GraphOS Canned Operations by ID](#graphos-canned-operations-by-id)
- [GraphOS Subgraph Docker Compose](#graphos-subgraph-docker-compose)
- [GraphOS Supergraph Router URL Overrides](#graphos-supergraph-router-url-overrides)
- [GraphOS Subgraph SDL](#graphos-subgraph-sdl)
- [GraphOS Subgraph Names](#graphos-subgraph-names)
- [GraphOS Supergraph SDL](#graphos-supergraph-sdl)
- [Inline File](#inline-file)
- [Merge YAML](#merge-yaml)
- [GraphOS Offline License](#graphos-offline-license)
- [Relative Path](#relative-path)
- [Required File](#required-file)
- [Resolved Values](#resolved-values)
- [Router Download Script](#router-download-script)

## Build Router From Source

A file provider used for building the Router from source at a specific git commit or reference.

```yaml
- name: "router-build.sh"
  env_var: ROUTER_BUILD_SCRIPT
  kind: build_router_from_source
  git_ref: "some-ref"
  rust_version: "1.89.0"
```

### Fields

#### `git_ref`

A git reference that can be passed to `git checkout`. This may be a full or partial commit hash,
branch name, or tag.

#### `rust_version`

A Rust version string that can be passed to `rustup run {rust_version}`, such as `"1.78.0"`,
`"beta"`, or `"nightly"`.

Defaults to `"stable"` if unset.

## From command

Run a command provider and use its output as a file provider resource.

As with all other command providers, you can provide both environment variables and other file
providers as inputs to the command being executed. RTF will use the contents of the `$RTF_OUTPUT`
path as the output of this provider, supporting both writing a single file to that path and creating
a directory at that path containing multiple files.

```yaml
- name: vegeta-ops.json
  env_var: VEGETA_OPS
  kind: from_command
  command:
    name: format-for-vegeta.sh
    kind: relative_path
    path: scripts/format-for-vegeta.sh
  env_vars:
    ROUTER_URL: "http://127.0.0.1:4000/"
  file_providers:
    - name: canned_ops.json
      env_var: CANNED_OPS_FILE
      kind: graphos_canned_ops
      graph_ref: "my@graph"
      top_n: 20
      skip_mutations: true
```

### format-for-vegeta.sh

```bash
#!/usr/bin/env sh
while read -r req; do
  if [[ "$OSTYPE" == "darwin"* ]]; then
    encoded=$(echo "$req" | base64 -b 0)
  else
    encoded=$(echo "$req" | base64 -w 0)
  fi
   
  jq -nc \
    --arg body "$encoded" \
    --arg url "$ROUTER_URL" \
    '{
      "body": $body,
      "header": { "Content-type": ["application/json"] },
      "method": "POST",
      "url": $url
    }' >> "$RTF_OUTPUT"
done <"$CANNED_OPS_FILE"
```

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

## GraphOS Canned Operations by ID

The user specifies the graph ref and parameters that should be used to generate canned GraphQL
requests based on operations data obtained from the GraphOS API.

```yaml
- name: canned_ops.json
  env_var: CANNED_OPS_FILE
  kind: graphos_canned_ops_by_id
  graph_ref: graph@variant
  operation_ids:
    - 5b1f8a2a1bd4be697559013a23fcbcb9186afe77
    - 3f56aa92aad650bbfc7ba481cbe029aba2f6c5f4
    - 50b77d7351052abd84dcd2c2ccb63eff2fa2f94c
```

### Fields

#### `graph_ref`

The Apollo graph ref to pull operations for.

#### `operation_ids`

Operation IDs from the Apollo studio API for the operations you want to work with as queried from an
`OperationInsightsListItem` in the Studio graphQL API.

## GraphOS Subgraph Docker Compose

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

## GraphOS Subgraph SDL

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

## GraphOS Subgraph Names

The user specifies the graph ref that should be used to fetch the names of subgraphs in the
supergraph from the GraphOS API.

This file proivider will output a newline-delimited file of the subgraph names.

```yaml
- name: "subgraph_names"
  env_var: SUBGRAPH_NAMES
  kind: graphos_subgraph_names
  graph_ref: graph@variant
```

### Fields

#### `graph_ref`

The Apollo graph ref to pull subgraph names for.

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

## Merge YAML

Merge the YAML output of text based file providers into a single YAML file.

Matching keys in the overrides file will replace scalar values, concatenate arrays and merge keys
for maps.

When merging a single overrides file the overrides provider can be specified directly under the
`overrides` key:

```yaml
- name: router-config.yaml
  env_var: ROUTER_CONFIG
  kind: merge_yaml
  base:
    kind: relative_path
    path: "data/base-router-config.yaml"
  overrides:
    kind: relative_path
    path: "../my-overrides.yaml"
```

When merging multiple overrides files, specify the providers in the order you want to merge them as
an array:

```yaml
- name: router-config.yaml
  env_var: ROUTER_CONFIG
  kind: merge_yaml
  base:
    kind: relative_path
    path: "data/base-router-config.yaml"
  overrides:
    - kind: relative_path
      path: "../my-overrides.yaml"
    - kind: relative_path
      path: "../my-other-overrides.yaml"
```

### Fields

#### `base`

A base YAML file to start with.

#### `overrides`

One or more YAML files to merge on top of the base file in sequence.

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

The Apollo graph id to pull an offline license for.

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
