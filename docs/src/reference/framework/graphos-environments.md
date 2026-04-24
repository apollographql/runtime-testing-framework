<!-- diataxis-type: reference -->

# GraphOS Environments

A Test Plan can target multiple GraphOS environments in a single run — for example, customer graphs
living in production GraphOS alongside Apollo's own `engine@prod` graph living in staging GraphOS.
Each environment is declared at the top level of the Test Plan and referenced by name from
individual [GraphOS file providers][0] via their `graphos_env` field.

## The implicit `default` environment

If a Test Plan does not declare a `graphos_environments` block, RTF synthesizes a single `default`
environment that points at the production GraphOS endpoint
(`https://graphql.api.apollographql.com/api/graphql`) and reads its API key from the `APOLLO_KEY`
env var. The optional `APOLLO_SUDO` env var — when set to `"true"` or `"1"` — configures the
`default` environment to include the `apollo-sudo: true` header on every request.

This means Test Plans that only target prod graphs need no `graphos_environments` block at all, and
can continue to be authored exactly as they have been.

## Declaring additional environments

To target GraphOS endpoints other than prod, add a `graphos_environments` block to the top level of
the `test-plan.yaml`. Each entry is a map keyed by a name of your choosing, with the following
fields:

```yaml
graphos_environments:
  apollo_staging:
    url: https://graphql-staging.api.apollographql.com/api/graphql
    api_key_env_var: APOLLO_KEY_STAGING
    sudo: false
```

<details>
<summary>Fields</summary>

### `url`

The GraphOS API endpoint for this environment. Required.

### `api_key_env_var`

The name of the environment variable that RTF should read to obtain this environment's API key.
Required.

When RTF starts, it reads each declared environment's `api_key_env_var` from the process environment
and registers a platform client under the environment's name. Environments whose `api_key_env_var`
is not set are skipped — any provider that references them will fail at check time with a clear
error message identifying both the environment and the expected env var.

### `sudo`

If `true`, requests made on behalf of this environment include the `apollo-sudo: true` header.
Defaults to `false`.

</details>

## Using an environment from a file provider

Any [GraphOS file provider][0] accepts an optional `graphos_env` field identifying which declared
environment should satisfy its `graph_ref`. Providers that don't specify a `graphos_env` fall
through to the implicit `default` environment.

```yaml
# Pulls the Expedia@prod supergraph from the default (prod) environment —
# no graphos_env needed.
- name: expedia-supergraph.graphql
  env_var: EXPEDIA_SUPERGRAPH
  kind: graphos_supergraph
  graph_ref: ExpediaInc-8789@prod

# Pulls Apollo's engine@prod supergraph from the declared apollo_staging
# environment.
- name: engine-supergraph.graphql
  env_var: ENGINE_SUPERGRAPH
  kind: graphos_supergraph
  graph_ref: engine-ed9f6f25068608ef@prod
  graphos_env: apollo_staging
```

Because `graphos_env` is a regular string field, it can be templated from matrix variables — a
single matrix can fan out across graphs that live in different environments by pairing each
`graph_ref` with the appropriate `graphos_env` via `matrix.include`:

```yaml
matrix:
  dimensions: {}
  include:
    - graph_ref: ExpediaInc-8789@prod
      graphos_env: default
    - graph_ref: engine-ed9f6f25068608ef@prod
      graphos_env: apollo_staging
```

## Runtime contract

The rtf process needs the API key for every environment the test plan actually exercises. For the
mixed-graph example above, that means both `APOLLO_KEY` (for `ExpediaInc-8789@prod`) and
`APOLLO_KEY_STAGING` (for `engine-ed9f6f25068608ef@prod`) must be exported in the shell that runs
`rtf`.

Environments whose API key is absent at runtime produce a `MissingGraphOsApiKey` error at check
time. The error message names both the referenced environment and the expected env var, so
misspelled environment names and unset credentials produce clearly different diagnostics.

[0]: ./file-providers.md
