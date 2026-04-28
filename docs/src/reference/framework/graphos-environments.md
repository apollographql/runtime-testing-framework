<!-- diataxis-type: reference -->

# GraphOS Environments

A test plan can target any GraphOS environment that rtf knows about by setting the `graphos_env`
field on a [GraphOS file provider][0]. rtf ships with a closed list of known environments — adding a
new one is an engine change, not a YAML change.

## Known environments

| Name      | URL                                                         | API key env var      |
| --------- | ----------------------------------------------------------- | -------------------- |
| `default` | `https://graphql.api.apollographql.com/api/graphql`         | `APOLLO_KEY`         |
| `staging` | `https://graphql-staging.api.apollographql.com/api/graphql` | `APOLLO_KEY_STAGING` |

For each entry above, rtf reads the listed env var from its process environment at startup.
Environments whose API key env var is not set are simply unavailable — any provider that references
them will fail at check time with a targeted error message naming the missing env var.

For most plans, exporting `APOLLO_KEY` (the prod key, obtained via the standard SHERIFF flow) is
enough. Plans that target Apollo's internal `engine@prod` graph also need `APOLLO_KEY_STAGING`
exported (separate SHERIFF ticket for staging access).

## Selecting an environment from a file provider

Every [GraphOS file provider][0] accepts an optional `graphos_env` field. Omitting it (or setting it
to `"default"`) routes to the production GraphOS instance.

```yaml
# Pulls a customer graph from the default (prod) environment.
- name: customer-supergraph.graphql
  kind: graphos_supergraph
  graph_ref: customer-graph@prod

# Pulls Apollo's engine@prod supergraph from staging GraphOS.
- name: engine-supergraph.graphql
  kind: graphos_supergraph
  graph_ref: engine@prod
  graphos_env: staging
```

`graphos_env` is a `Field<String>` so it can be templated from matrix variables. A single matrix can
fan out across graphs that live in different environments by pairing each `graph_ref` with the
appropriate `graphos_env` via `matrix.include`:

```yaml
matrix:
  dimensions: {}
  include:
    - graph_ref: customer-graph@prod
      graphos_env: default
    - graph_ref: engine@prod
      graphos_env: staging
```

Values that don't match a known environment name produce an `UnknownGraphosEnv` error at check time,
listing the valid values.

## Adding a new known environment

A new environment is added in code by extending the `KNOWN_GRAPHOS_ENVS` list in
`crates/rtf-integrations/src/lib.rs` with its name, URL, env var, and `sudo` flag. The environment
becomes available in YAML the moment a new rtf release is cut — no plan-side schema migration
required.

The set is intentionally engine-side rather than user-declared so that the rep-orchestrator can
provision the corresponding secrets at deploy time without coordinating with arbitrary
plan-author-declared env var names.

[0]: ./file-providers.md
