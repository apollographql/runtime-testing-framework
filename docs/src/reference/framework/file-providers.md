<!-- diataxis-type: reference -->

# File Providers

Available file providers:

- [Build Router from source](#build-router-from-source)
- [Conditional](#conditional)
- [Custom provider](#custom-provider)
- [From command](#from-command)
- [GitHub file](#github-file)
- [Fields common to all GraphOS providers](#fields-common-to-all-graphos-providers)
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

A file provider used for building the Router from source at a specific git commit or reference. A
profile and list of features can optionally be provided.

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

### `git_ref`

A git reference that can be passed to `git checkout`. This may be a full or partial commit hash,
branch name, or tag.

### `rust_version`

A Rust version string that can be passed to `rustup run {rust_version}`, such as `"1.78.0"`,
`"beta"`, or `"nightly"`.

Defaults to `"stable"` if unset.

### `profile`

The profile to build the Router with.

Defaults to `"release"` if unset.

### `features`

Comma separated list of features to build the Router with.

Defaults to `"default"` if unset.

</details>

## Conditional

Conditionally run a file provider from an ordered list based on simple "where" clauses that make use
of the provided templating variables. The first case with a "where" clause that holds will be run as
the output of this provider.

### Writing where clauses

The "where" clause on each case is a simple comparison against a single templating variable. You
must include the `var` key which accepts a string variable name that is required to be defined
within the test plan containing this provider. You may then assert that the variable is equal (`eq`)
or not equal (`ne`) to a given scalar value.

If none of the provider where clauses match, this provider will error during static analysis checks.

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

### `cases`

The ordered list of cases to be checked against the variables used for templating the test plan.

</details>

## Custom provider

Use a custom provider to execute a command and produce a set of files.

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

### `ty`

The type of custom provider to use. This is the name of the custom provider to use.

</details>

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

<details>
<summary>Fields</summary>

### `command`

The command to be run

<details>
<summary>Fields</summary>

#### `name`

The name of the command to run

<details>
<summary>Variants</summary>

- [Inline file](#inline-file)
- [Relative path](#relative-path)
- [Required file](#required-file)

</details>

#### `args`

Arguments to the command

</details>

### `env_vars`

Environment variables to set

### `file_providers`

File providers to run and make available prior to execution

</details>

## GitHub file

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

<details>
<summary>Fields</summary>

### `org`

The GitHub org for the repository containing the target file

### `repo`

The GitHub repository containing the target file

### `path`

The absolute path from the root of the repository to the target file

### `git_ref`

An optional git reference to pull the file from. This may be a full or partial commit hash, branch
name, or tag.

Defaults to the mainline branch as specified in GitHub if unset.

</details>

## Fields common to all GraphOS providers

Every file provider that resolves a `graph_ref` against the GraphOS API also accepts an optional
`graphos_env` field naming which declared [GraphOS environment][0] the request should be routed
through.

Omitting `graphos_env` (or setting it to `"default"`) falls through to the implicit production
environment synthesized from the `APOLLO_KEY` env var. Other values must correspond to entries in
the Test Plan's top-level `graphos_environments` block.

```yaml
# Uses the implicit default (prod) environment.
- name: expedia-supergraph.graphql
  kind: graphos_supergraph
  graph_ref: ExpediaInc-8789@prod

# Uses a declared non-default environment.
- name: engine-supergraph.graphql
  kind: graphos_supergraph
  graph_ref: engine-ed9f6f25068608ef@prod
  graphos_env: apollo_staging
```

## GraphOS canned operations

The user specifies the graph ref and parameters that should be used to generate canned GraphQL
requests based on operations data obtained from the GraphOS API.

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

### `graph_ref`

The Apollo graph ref to pull operations for.

### `top_n`

The number of operations to attempt to fetch.

Defaults to 20 if unset.

### `skip_mutations`

Whether or not to include mutations in the returned operations.

Defaults to false if unset.

### `time_range`

How far back to query for operations.

Accepts duration strings like "30d", "7d", "12h". Defaults to "30d" if unset.

</details>

## GraphOS canned operations by ID

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

<details>
<summary>Fields</summary>

### `graph_ref`

The Apollo graph ref to pull operations for.

### `operation_ids`

Operation IDs from the Apollo studio API for the operations you want to work with as queried from an
`OperationInsightsListItem` in the Studio graphQL API.

</details>

## GraphOS supergraph Router URL overrides

The user specifies the graph ref that should be used to fetch subgraph SDL files from the GraphOS
API and generates a the override_subgraph_urls YAML snippet that can be merged into a router config
file.

This should be used whenever subgraph requests need to be mapped to a mock server instead of hitting
the real subgraph as defined in the supergraph, which is typically desirable behavior when working
with real graphs.

```yaml
- name: subgraph-url-overrides.yaml
  env_var: SUBGRAPH_URL_OVERRIDES
  kind: graphos_subgraph_router_url_overrides
  graph_ref: graph@variant
  url_format: localhost
```

<details>
<summary>Fields</summary>

### `graph_ref`

The Apollo graph ref to pull the subgraphs for.

### `url_format`

The format of the overrides url.

<details>
<summary>Variants</summary>

- `localhost`: Overrides to `http://localhost:<port>` for each subgraph. Port is defined as
  `4001 + n` where `n` is the nth subgraph, starting at 0.
- `docker`: Overrides to `http://loadbalancer:8080`.

#### `custom`

Accepts a custom formatting configuration that will define the URLs.

<details>
<summary>Fields</summary>

##### `base_url`

The base URL to route subgraph requests to.

Defaults to the [UrlFormat::Docker] format if not set.

##### `base_port`

The base port that the subgraph requests should use

Defaults to the [UrlFormat::Docker] port if not set.

##### `increment_port`

Whether or not to increment the port number from the base for each subgraph.

Defaults to false if unset.

##### `add_subgraph_route`

Whether or not to include a `/{subgraph_name}` route for each subgraph.

Defaults to false if unset.

##### `custom_subgraph_urls`

Custom subgraph URL overrides for routes that do not fit the structure built by the above
parameters.

</details>

</details>

</details>

## GraphOS subgraph SDL

The user specifies the graph ref that should be used to fetch a subgraph SDL files from the GraphOS
API.

Note that this file provider will output a directory of SDL schema files, one for each subgraph.

```yaml
- name: "subgraphs"
  env_var: SUBGRAPHS
  kind: graphos_subgraphs
  graph_ref: graph@variant
```

<details>
<summary>Fields</summary>

### `graph_ref`

The Apollo graph ref to pull subgraph SDL files for.

</details>

## GraphOS subgraph names

The user specifies the graph ref that should be used to fetch the names of subgraphs in the
supergraph from the GraphOS API.

This file provider will output a newline-delimited file of the subgraph names.

```yaml
- name: "subgraph_names"
  env_var: SUBGRAPH_NAMES
  kind: graphos_subgraph_names
  graph_ref: graph@variant
```

<details>
<summary>Fields</summary>

### `graph_ref`

The Apollo graph ref to pull subgraph names for.

</details>

## GraphOS supergraph SDL

The user specifies the ref that should be used to fetch a supergraph SDL file from the GraphOS API.

```yaml
- name: "supergraph.graphql"
  env_var: SUPERGRAPH
  kind: graphos_supergraph
  graph_ref: graph@variant
  with_subgraph_overrides: docker
```

<details>
<summary>Fields</summary>

### `graph_ref`

The Apollo graph ref to pull supergraph SDL for.

### `with_subgraph_overrides`

Replace the supergraph's subgraph urls with overridden values for testing.

Defaults to null if unset.

### `with_connector_overrides`

Replace the supergraph's connector urls with overridden values for testing.

Defaults to null if unset.

</details>

## Inline file

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

### `content`

The text to write out as the contents of the generated file.

</details>

## Inline directory

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

### `files`

A list of inline files stored in the directory

</details>

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

<details>
<summary>Fields</summary>

### `base`

A base YAML file to start with.

<details>
<summary>Variants</summary>

- [GitHub file](#github-file)
- [GraphOS supergraph Router URL overrides](#graphos-supergraph-router-url-overrides)
- [Inline file](#inline-file)
- [Relative path](#relative-path)
- [Required file](#required-file)
- [Templated file](#templated-file)

</details>

### `overrides`

One or more YAML files to merge on top of the base file in sequence.

</details>

## GraphOS offline license

The user specifies the graph id that should be used to fetch an offline license from the GraphOS
API.

```yaml
- name: license.jwt
  env_var: LICENSE
  kind: graphos_offline_license
  graph_id: graph
```

<details>
<summary>Fields</summary>

### `graph_id`

The Apollo graph id to pull an offline license for.

</details>

## Relative dir

A relative path from the containing config file to a target directory and a list of files that
should be made available as part of the test run. This provider works both with local directories
and directories within GitHub if the containing config file was pulled from a repository.

The specified environment variable for this provider will point to the location of the directory
itself. Relative paths under the specified directory will be maintained and in order to provided
deterministic locations for each included file.

Only the specified files will be included and there is no way to wildcard multiple files.

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

### `path`

The relative path from the containing config file to the target directory.

### `files`

The file paths under this directory that should be included.

</details>

## Relative path

A relative path from the containing config file to a target file that should be made available as
part of the test run. This provider works both with local files and files within GitHub if the
containing config file was pulled from a repository.

```yaml
- name: "my-file.txt"
  env_var: MY_FILE
  kind: relative_path
  path: "../../resources/test-data/my-file.txt"
```

<details>
<summary>Fields</summary>

### `path`

The relative path from the containing config file to the target file.

</details>

## Required file

The only purpose of this file provider is to throw an error if it still exists when the file
providers are being checked. All definitions of a required file are expected to be replaced by user
defined file providers.

```yaml
- name: "router-config.yaml"
  env_var: ROUTER_CONFIG
  kind: required
  message: "you must specify a router config file to use"
```

<details>
<summary>Fields</summary>

### `message`

The error message to display to the user if this provider is not overwritten.

</details>

## Router download script

Produces a POSIX shell script that can be run in order to download a target version of the Apollo
Router.

```yaml
- name: "router-download.sh"
  env_var: ROUTER_DOWNLOAD
  kind: router_download_script
  version: "v2.6.0"
```

<details>
<summary>Fields</summary>

### `version`

The version of the Apollo Router to download.

</details>

## Templated file

Write a file whose content is an inline string with `${variable}` patterns interpolated from the RTF
template variables defined for the current run.

```yaml
- name: config.json
  env_var: CONFIG_FILE
  kind: templated
  content: |
    { "endpoint": "${router_url}" }
```

<details>
<summary>Fields</summary>

### `content`

The file content with optional `${variable}` interpolation patterns.

</details>

[0]: ./graphos-environments.md
