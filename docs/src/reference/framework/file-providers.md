<!-- diataxis-type: reference -->

# File Providers

Available file providers:

- [Build Router from source](#build-router-from-source)
- [Conditional](#conditional)
- [Custom provider](#custom-provider)
- [From command](#from-command)
- [GitHub directory](#github-directory)
- [GitHub file](#github-file)
- [GraphOS canned operations](#graphos-canned-operations)
- [GraphOS canned operations by ID](#graphos-canned-operations-by-id)
- [GraphOS supergraph Router URL overrides](#graphos-supergraph-router-url-overrides)
- [GraphOS subgraph SDL](#graphos-subgraph-sdl)
- [GraphOS subgraph names](#graphos-subgraph-names)
- [GraphOS supergraph SDL](#graphos-supergraph-sdl)
- [Inline file](#inline-file)
- [Inline directory](#inline-directory)
- [Merge YAML](#merge-yaml)
- [GraphOS offline license](#graphos-offline-license)
- [Relative dir](#relative-dir)
- [Relative path](#relative-path)
- [Required file](#required-file)
- [Router download script](#router-download-script)
- [Templated file](#templated-file)

## Build Router from source

`kind: build_router_from_source`

A shell script that builds the Apollo Router from source at a specific git reference when run.

The script clones the Router repository, builds it using the requested Rust version, profile and
features, and then copies the resulting `router` binary into `~/.cargo/bin`. Running the script
requires `git` and `rustup` to be installed and the `CARGO_TARGET_DIR` environment variable to be
set.

```yaml
- name: "router-build.sh"
  env_var: ROUTER_BUILD_SCRIPT
  kind: build_router_from_source
  git_ref: "some-ref"
  rust_version: "1.89.0"
  profile: "release"
  features: "default"
```

<details>
<summary>Fields</summary>

### `features`

A comma separated list of cargo features to build the Router with.

Defaults to `"default"` if unset.

### `git_ref` (required)

The git reference of the Router repository to build: a full or partial commit hash, a branch name or
a tag.

### `profile`

The cargo profile to build the Router with.

Defaults to `"release"` if unset.

### `rust_version`

The Rust toolchain to build the Router with, such as `"1.78.0"`, `"beta"` or `"nightly"`.

Defaults to `"stable"` if unset.

</details>

## Conditional

`kind: conditional`

One of an ordered list of file providers, chosen based on the value of your templating variables.
Each case has a "where" clause, and the first case whose "where" clause holds is used as this file
provider.

### Writing where clauses

The "where" clause on each case is a simple comparison against a single templating variable. You
must include the `var` key, giving the name of a templating variable that is defined in the test
plan containing this provider. You can then check that the variable is equal (`eq`) or not equal
(`ne`) to a given scalar value.

If none of the "where" clauses hold, RTF will report an error when templating your config.

```yaml
- name: conditional_config.json
  env_var: CONDITIONAL_CONFIG
  kind: conditional
  cases:
    - where: { var: test_type, eq: load }
      kind: relative_path
      path: data/config-load.json

    - where: { var: test_type, eq: ramp }
      kind: relative_path
      path: data/config-ramp.json
```

<details>
<summary>Fields</summary>

### `cases` (required)

The cases to check, in order. Each case is a file provider with an additional `where` key.

<details>
<summary>Fields</summary>

#### `where` (required)

The clause that must hold for this case to be used.

<details>
<summary>Fields</summary>

##### `var` (required)

The name of the templating variable to check.

</details>

<details>
<summary>Variants</summary>

##### `eq`

The variable must be equal to the given scalar value.

##### `ne`

The variable must not be equal to the given scalar value.

</details>

</details>

<details>
<summary>Variants</summary>

- Any [file provider](#file-providers)

</details>

</details>

## Custom provider

`kind: custom_provider`

Run a custom provider to produce a file or directory of files. The environment variable will be set
to the path of the custom provider's output.

The custom provider to run is selected by its `type`, which must match the name of a custom provider
definition available to the containing config file. All other keys are passed to the custom provider
as arguments, and are used to set the templating variables declared in its definition.

```yaml
- name: "router-docker-compose"
  env_var: ROUTER_DOCKER_COMPOSE
  kind: custom_provider
  type: "router-docker-compose"
  graph_ref: "graph@variant"
  router_version: "v2.x.y"
  build_router_from_source: "false"
```

<details>
<summary>Fields</summary>

### `type` (required)

The name of the custom provider definition to run.

</details>

## From command

`kind: from_command`

A file or directory of files produced by running a command provider. The environment variable will
be set to the path of the command's output.

As with all other command providers, you can provide both environment variables and other file
providers as inputs to the command being run. Your command should write its output to the path given
by the `$RTF_OUTPUT` environment variable, either as a single file or as a directory containing
multiple files.

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

<details>
<summary>Fields</summary>

### `command` (required)

The command to be run

<details>
<summary>Fields</summary>

#### `args`

Arguments to the command

#### `name` (required)

The name of the command to run

</details>

<details>
<summary>Variants</summary>

- [Inline file](#inline-file)
- [Relative path](#relative-path)
- [Required file](#required-file)

</details>

### `env_vars`

Environment variables to set

### `file_providers`

File providers to run and make available prior to execution

<details>
<summary>Fields</summary>

#### `env_var` (required)

The environment variable to place the absolute path to this providers output

#### `name` (required)

The name to use for the output produced by this provider

This can be either a file or a directory depending on the file provider.

</details>

<details>
<summary>Variants</summary>

- Any [file provider](#file-providers)

</details>

</details>

## GitHub directory

`kind: github_dir`

A directory of files pulled from a GitHub repository. The environment variable will be set to the
path of the directory itself. All files within that directory will need to be referenced using a
combination of this environment variable and their path relative to `path`.

Only the files listed under `files` are included and there is no way to wildcard multiple files. If
no `git_ref` is provided, the files are pulled from the default branch.

Using this file provider requires the `GITHUB_TOKEN` environment variable to be set.

```yaml
- name: "my-data"
  env_var: MY_DATA
  kind: github_dir
  org: "my-org"
  repo: "my-repo"
  git_ref: "some-ref"
  path: "resources/test-data"
  files:
    - "my-file.txt"
    - "nested/my-nested-file.json"
```

<details>
<summary>Fields</summary>

### `files` (required)

The paths of the files to include, relative to `path`.

### `git_ref`

The git reference to pull the files from: a full or partial commit hash, a branch name or a tag.

Defaults to the repository's default branch if unset.

### `org` (required)

The GitHub organization that owns the repository.

### `path` (required)

The path to the directory from the root of the repository.

### `repo` (required)

The name of the repository containing the directory.

</details>

## GitHub file

`kind: github_file`

A file pulled from a GitHub repository.

If no `git_ref` is provided, the file is pulled from the default branch.

Using this file provider requires the `GITHUB_TOKEN` environment variable to be set.

```yaml
- name: "my-file.txt"
  env_var: MY_FILE
  kind: github_file
  org: "my-org"
  repo: "my-repo"
  path: "resources/test-data/my-file.txt"
  git_ref: "some-ref"
```

<details>
<summary>Fields</summary>

### `git_ref`

The git reference to pull the file from: a full or partial commit hash, a branch name or a tag.

Defaults to the repository's default branch if unset.

### `org` (required)

The GitHub organization that owns the repository.

### `path` (required)

The path to the file from the root of the repository.

### `repo` (required)

The name of the repository containing the file.

</details>

## GraphOS canned operations

`kind: graphos_canned_ops`

A set of canned GraphQL requests built from the most frequently used operations for a graph ref, as
reported by the GraphOS API. The environment variable will be set to the path of a file containing
one JSON request body (with `query` and `variables` keys) per line.

Using this file provider requires the `APOLLO_KEY` environment variable to be set.

```yaml
- name: canned_ops.json
  env_var: CANNED_OPS_FILE
  kind: graphos_canned_ops
  graph_ref: graph@variant
  top_n: 10
  skip_mutations: true
  time_range: 7d
```

<details>
<summary>Fields</summary>

### `graph_ref` (required)

The graph ref (`graph_id@variant`) to fetch operations for.

### `skip_mutations`

Whether to exclude mutations from the fetched operations.

Defaults to false if unset.

### `time_range`

How far back to look for operations.

Accepts duration strings such as `"30d"`, `"7d"` or `"12h"`.

Defaults to `"30d"` if unset.

### `top_n`

The maximum number of operations to fetch.

Defaults to 20 if unset.

</details>

## GraphOS canned operations by ID

`kind: graphos_canned_ops_by_id`

A set of canned GraphQL requests built from specific operations for a graph ref, fetched from the
GraphOS API by operation ID. The environment variable will be set to the path of a file containing
one JSON request body (with `query` and `variables` keys) per line.

Using this file provider requires the `APOLLO_KEY` environment variable to be set.

```yaml
- name: canned_ops.json
  env_var: CANNED_OPS_FILE
  kind: graphos_canned_ops_by_id
  graph_ref: graph@variant
  operations:
    kind: inline
    content: |
      5b1f8a2a1bd4be697559013a23fcbcb9186afe77
      3f56aa92aad650bbfc7ba481cbe029aba2f6c5f4
      50b77d7351052abd84dcd2c2ccb63eff2fa2f94c
```

<details>
<summary>Fields</summary>

### `graph_ref` (required)

The graph ref (`graph_id@variant`) to fetch operations for.

### `operations` (required)

A text file provider for a file containing the GraphOS operation IDs of the operations you want to
fetch, one per line.

<details>
<summary>Variants</summary>

- [GitHub file](#github-file)
- [Inline file](#inline-file)
- [Relative path](#relative-path)
- [Required file](#required-file)
- [Templated file](#templated-file)

</details>

</details>

## GraphOS supergraph Router URL overrides

`kind: graphos_subgraph_router_url_overrides`

A Router config snippet that overrides the URL of each subgraph in a graph ref, using the subgraphs
fetched from the GraphOS API. The environment variable will be set to the path of a YAML file
containing an `override_subgraph_url` section that you can merge into your Router config file (for
example, using a `merge_yaml` file provider).

Use this file provider when you want subgraph requests to be sent to mock servers rather than the
real subgraphs defined in the supergraph, which is typically what you want when testing against real
graphs.

Using this file provider requires the `APOLLO_KEY` environment variable to be set.

```yaml
- name: subgraph-url-overrides.yaml
  env_var: SUBGRAPH_URL_OVERRIDES
  kind: graphos_subgraph_router_url_overrides
  graph_ref: graph@variant
  url_format: localhost
```

<details>
<summary>Fields</summary>

### `graph_ref` (required)

The graph ref (`graph_id@variant`) to fetch the subgraphs for.

### `url_format`

The format to use for the overridden subgraph URLs.

Defaults to `localhost` if unset.

<details>
<summary>Variants</summary>

- `localhost`: Subgraph URLs are set to `http://localhost:{port}`, where the port is `4001` for the
  first subgraph and increases by one for each subsequent subgraph. Connector URLs are set to
  `http://localhost:3000`.
- `docker`: Subgraph URLs are set to `http://loadbalancer:8080`. Connector URLs are set to
  `http://connector:3000`.

#### `custom`

Subgraph and connector URLs are built from a custom URL format.

<details>
<summary>Fields</summary>

##### `add_subgraph_route`

Whether to append a `/{subgraph_name}` route to the URL for each subgraph.

Defaults to false if unset.

##### `base_port`

The port to send subgraph requests to.

Defaults to `8080` if unset.

##### `base_url`

The base URL (without a port) to send subgraph requests to.

Defaults to `http://loadbalancer` if unset.

##### `custom_subgraph_urls`

A map of subgraph names to the URL to use for that subgraph, for any subgraphs that do not fit the
format described by the other fields.

##### `increment_port`

Whether to give each subgraph its own port, starting from `base_port`.

Defaults to false if unset.

</details>

</details>

</details>

## GraphOS subgraph SDL

`kind: graphos_subgraphs`

A directory containing the SDL of each subgraph in a graph ref, fetched from the GraphOS API. The
environment variable will be set to the path of the directory itself, which will contain one
`{subgraph_name}.graphql` file for each subgraph.

Using this file provider requires the `APOLLO_KEY` environment variable to be set.

```yaml
- name: "subgraphs"
  env_var: SUBGRAPHS
  kind: graphos_subgraphs
  graph_ref: graph@variant
```

<details>
<summary>Fields</summary>

### `graph_ref` (required)

The graph ref (`graph_id@variant`) to fetch the subgraph SDL files for.

</details>

## GraphOS subgraph names

`kind: graphos_subgraph_names`

The names of each subgraph in a graph ref, fetched from the GraphOS API. The environment variable
will be set to the path of a file containing one subgraph name per line.

Using this file provider requires the `APOLLO_KEY` environment variable to be set.

```yaml
- name: "subgraph_names"
  env_var: SUBGRAPH_NAMES
  kind: graphos_subgraph_names
  graph_ref: graph@variant
```

<details>
<summary>Fields</summary>

### `graph_ref` (required)

The graph ref (`graph_id@variant`) to fetch the subgraph names for.

</details>

## GraphOS supergraph SDL

`kind: graphos_supergraph`

The supergraph SDL for a graph ref, fetched from the GraphOS API.

The subgraph and connector URLs within the supergraph can optionally be rewritten so that requests
are routed to mock servers running as part of your test.

Using this file provider requires the `APOLLO_KEY` environment variable to be set.

```yaml
- name: "supergraph.graphql"
  env_var: SUPERGRAPH
  kind: graphos_supergraph
  graph_ref: graph@variant
  with_subgraph_overrides: docker
```

<details>
<summary>Fields</summary>

### `graph_ref` (required)

The graph ref (`graph_id@variant`) to fetch the supergraph SDL for.

### `with_connector_overrides`

Replace the connector URLs within the supergraph using the given URL format.

If unset, the connector URLs are left unchanged.

<details>
<summary>Variants</summary>

- `localhost`: Subgraph URLs are set to `http://localhost:{port}`, where the port is `4001` for the
  first subgraph and increases by one for each subsequent subgraph. Connector URLs are set to
  `http://localhost:3000`.
- `docker`: Subgraph URLs are set to `http://loadbalancer:8080`. Connector URLs are set to
  `http://connector:3000`.

#### `custom`

Subgraph and connector URLs are built from a custom URL format.

<details>
<summary>Fields</summary>

##### `add_subgraph_route`

Whether to append a `/{subgraph_name}` route to the URL for each subgraph.

Defaults to false if unset.

##### `base_port`

The port to send subgraph requests to.

Defaults to `8080` if unset.

##### `base_url`

The base URL (without a port) to send subgraph requests to.

Defaults to `http://loadbalancer` if unset.

##### `custom_subgraph_urls`

A map of subgraph names to the URL to use for that subgraph, for any subgraphs that do not fit the
format described by the other fields.

##### `increment_port`

Whether to give each subgraph its own port, starting from `base_port`.

Defaults to false if unset.

</details>

</details>

### `with_fed3_compat`

Rewrite the supergraph so a Router 3 build accepts it. A graph composed before the September 2025
GraphQL spec is otherwise rejected at startup.

Defaults to false if unset.

### `with_subgraph_overrides`

Replace the subgraph URLs within the supergraph using the given URL format.

If unset, the subgraph URLs are left unchanged.

<details>
<summary>Variants</summary>

- `localhost`: Subgraph URLs are set to `http://localhost:{port}`, where the port is `4001` for the
  first subgraph and increases by one for each subsequent subgraph. Connector URLs are set to
  `http://localhost:3000`.
- `docker`: Subgraph URLs are set to `http://loadbalancer:8080`. Connector URLs are set to
  `http://connector:3000`.

#### `custom`

Subgraph and connector URLs are built from a custom URL format.

<details>
<summary>Fields</summary>

##### `add_subgraph_route`

Whether to append a `/{subgraph_name}` route to the URL for each subgraph.

Defaults to false if unset.

##### `base_port`

The port to send subgraph requests to.

Defaults to `8080` if unset.

##### `base_url`

The base URL (without a port) to send subgraph requests to.

Defaults to `http://loadbalancer` if unset.

##### `custom_subgraph_urls`

A map of subgraph names to the URL to use for that subgraph, for any subgraphs that do not fit the
format described by the other fields.

##### `increment_port`

Whether to give each subgraph its own port, starting from `base_port`.

Defaults to false if unset.

</details>

</details>

</details>

## Inline file

`kind: inline`

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

<details>
<summary>Fields</summary>

### `content` (required)

The text to write out as the contents of the generated file.

</details>

## Inline directory

`kind: inline_dir`

An inline representation of a directory of files. The environment variable will be set to the path
of the directory itself. All files within that directory will need to be referenced using a
combination of this environment variable and its `path`.

This file provider primarily exists so that other file providers that produce a directory of files
can be converted into their inline representations.

If, as a user of RTF, you need to specify multiple inline files, we _strongly_ advise you use an
`inline` file provider for each file and that you DO NOT use this file provider.

```yaml
- name: "my-directory"
  env_var: MY_DIRECTORY
  kind: inline_dir
  files:
    - path: file1.txt
      content: |
        content for file1
    - path: nested/file2.txt
      content: |
        content for file2
```

<details>
<summary>Fields</summary>

### `files` (required)

A list of inline files stored in the directory

<details>
<summary>Fields</summary>

#### `content` (required)

The text to write out as the contents of the generated file.

#### `path` (required)

The relative path to the generated file within the directory

</details>

</details>

## Merge YAML

`kind: merge_yaml`

A single YAML file produced by merging the output of one or more file providers on top of a base
YAML file.

Where a key is present in both files, scalar values from the overrides replace those in the base,
arrays are concatenated and maps are merged.

When merging a single overrides file, you can specify its file provider directly under the
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

When merging multiple overrides files, specify their file providers as an array in the order you
want them to be merged:

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

<details>
<summary>Fields</summary>

### `base` (required)

The file provider for the base YAML file.

<details>
<summary>Variants</summary>

- [GitHub file](#github-file)
- [GraphOS supergraph Router URL overrides](#graphos-supergraph-router-url-overrides)
- [Inline file](#inline-file)
- [Relative path](#relative-path)
- [Required file](#required-file)
- [Templated file](#templated-file)

</details>

### `overrides` (required)

One or more file providers for the YAML files to merge on top of the base file, in order.

</details>

## GraphOS offline license

`kind: offline_graphos_license`

An offline license for a graph, fetched from the GraphOS API.

Using this file provider requires the `APOLLO_KEY` environment variable to be set.

```yaml
- name: license.jwt
  env_var: LICENSE
  kind: offline_graphos_license
  graph_id: graph
```

<details>
<summary>Fields</summary>

### `graph_id` (required)

The ID of the graph to fetch an offline license for.

</details>

## Relative dir

`kind: relative_dir`

A directory of files referenced by its path relative to the containing config file. The environment
variable will be set to the path of the directory itself. All files within that directory will need
to be referenced using a combination of this environment variable and their path relative to `path`.

Only the files listed under `files` are included and there is no way to wildcard multiple files.

This works both for local config files and for config files pulled from a GitHub repository, in
which case the files are pulled from the same repository.

```yaml
- name: "my-data"
  env_var: MY_DATA
  kind: relative_dir
  path: "../../resources/test-data"
  files:
    - "my-file.txt"
    - "nested/my-nested-file.json"
```

<details>
<summary>Fields</summary>

### `files` (required)

The paths of the files to include, relative to `path`.

### `path` (required)

The path to the directory, relative to the containing config file.

</details>

## Relative path

`kind: relative_path`

A file referenced by its path relative to the containing config file.

This works both for local config files and for config files pulled from a GitHub repository, in
which case the file is pulled from the same repository.

```yaml
- name: "my-file.txt"
  env_var: MY_FILE
  kind: relative_path
  path: "../../resources/test-data/my-file.txt"
```

<details>
<summary>Fields</summary>

### `path` (required)

The path to the file, relative to the containing config file.

</details>

## Required file

`kind: required`

A placeholder for a file that must be provided by whoever uses your config file, such as a test plan
using your scenario or environment. If the placeholder has not been overridden with another file
provider by the time RTF checks your config, RTF will report an error containing the `message` you
provide.

```yaml
- name: "router-config.yaml"
  env_var: ROUTER_CONFIG
  kind: required
  message: "you must specify a router config file to use"
```

<details>
<summary>Fields</summary>

### `message` (required)

The error message to report if this file has not been provided.

</details>

## Router download script

`kind: router_download_script`

A POSIX shell script that downloads a specific version of the Apollo Router when run.

```yaml
- name: "router-download.sh"
  env_var: ROUTER_DOWNLOAD
  kind: router_download_script
  version: "v2.6.0"
```

<details>
<summary>Fields</summary>

### `version` (required)

The version of the Apollo Router to download, such as `"v2.6.0"`.

</details>

## Templated file

`kind: templated`

A file whose contents are specified inline within your config file, with any `${variable}` patterns
replaced by the value of the matching templating variable for the current run.

```yaml
- name: config.json
  env_var: CONFIG_FILE
  kind: templated
  content: |
    { "endpoint": "${router_url}" }
```

<details>
<summary>Fields</summary>

### `content` (required)

The text to write out as the contents of the generated file, which may contain `${variable}`
patterns.

</details>
