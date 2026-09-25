# example-router-test

An example of running a local router load test against a real GraphOS graph with the
[`rtf` CLI][rtf]. Read the warnings below before using it.

## ⚠️ This is not a performance test

This test plan shows how the pieces fit together; its results are not a meaningful measure of router
performance. The router, subgraph-mock and k6 all share one machine and compete for its CPU and
memory, nothing is resource-limited or isolated from other workloads, and a single short run says
nothing about variance. Don't compare or publish numbers from it.

## ⚠️ This sends data to GraphOS

This test plan is **not** isolated from GraphOS. The router sends usage reports and traces for every
request to `graph_ref`, so the load shows up in that variant's Studio metrics (tagged with client
name `rtf-example-router-test`). Use a non-production variant.

## Prerequisites

### The `rtf` CLI

Install `rtf` from source (needs a [Rust toolchain][rust-install]), following the
[runtime-testing-framework README][rtf-install]:

```bash
git clone git@github.com:apollographql/runtime-testing-framework.git
cd runtime-testing-framework
cargo install --path crates/rtf-cli
rtf --help
```

The [RTF docs][rtf-docs] explain test plans, scenarios and environments; the
[Hello, world! tutorial][rtf-hello-world] is a good first run.

### Docker and Docker Compose

[Docker][docker-install] and [Docker Compose][compose-install] v2 (`docker compose`) installed
locally, with the Docker daemon running. Docker Desktop includes both. Every component, k6 included,
runs in a container, so nothing else needs installing.

### `APOLLO_KEY`

A valid `APOLLO_KEY` for the graph in `graph_ref`: a [graph API key][graph-api-key] (not a personal
key) with the [Graph Admin role][graph-api-key-roles], which is the default for new graph API keys.

It's read from the shell running `rtf` and used twice:

- `rtf` uses it to fetch the supergraph schema and the top operations from GraphOS.
- The router uses it (with `APOLLO_GRAPH_REF`) to fetch its license and to report usage.

## How it works

`rtf run` resolves the file providers, starts the environment with Docker Compose, runs the k6
scenario against it, then tears everything down.

- **Supergraph schema** (`environment.yaml`): pulled from GraphOS, with every subgraph URL rewritten
  to point at subgraph-mock and connector URLs rewritten to an unresolvable `.invalid` host.
- **Router** (`data/router-config.yaml`, `data/docker-compose.yaml`): loads the rewritten schema
  from a file (`-s`), so it never fetches one from Uplink.
- **subgraph-mock** (`data/subgraph-config.yaml`): serves the whole supergraph with generated
  responses. Each subgraph is routed to `/<subgraph name>`, so per-subgraph overrides apply.
- **k6** (`scenario.yaml`, `scripts/load-test.js`): pulls the top `top_n` operations from GraphOS
  and sends them round-robin, ramping linearly to `rps` over `warmup_duration`, then holding `rps`
  for `duration`.

k6 prints an end-of-test summary with latency, throughput, HTTP failures (`http_req_failed`) and
GraphQL errors (`graphql_errors`). Errors are reported, never fail the run. Connectors aren't
mocked, so fields resolved through `@connect` show up as GraphQL errors.

## Running

From the repo root:

```bash
APOLLO_KEY="<graph_api_key>" rtf run test-plans/example-router-test/test-plan.yaml \
  --var 'graph_ref=<graph@variant>'
```

To use a locally built subgraph-mock:

```bash
docker build -t local-subgraph-mock .
APOLLO_KEY="<graph_api_key>" rtf run test-plans/example-router-test/test-plan.yaml \
  --var 'graph_ref=<graph@variant>' --var 'subgraph_image=local-subgraph-mock'
```

### Variables

Pass these with `--var 'name=value'`:

| Variable          | Default                                     | Description                                          |
| ----------------- | ------------------------------------------- | ---------------------------------------------------- |
| `graph_ref`       | (required)                                  | GraphOS graph ref the router runs as                 |
| `rps`             | `10`                                        | Target requests per second                           |
| `duration`        | `60s`                                       | Constant-rate duration, after the warmup             |
| `warmup_duration` | `10s`                                       | Linear ramp from 0 to `rps`                          |
| `top_n`           | `10`                                        | Number of operations pulled from GraphOS             |
| `max_vus`         | `50`                                        | VUs allocated for each k6 scenario (warmup and load) |
| `router_tag`      | `v2.17.0`                                   | Router image tag                                     |
| `router_image`    | `ghcr.io/apollographql/router`              | Router image, without the tag                        |
| `subgraph_image`  | `ghcr.io/apollographql/subgraph-mock:0.2.1` | subgraph-mock image and tag                          |

If k6 reports `dropped_iterations`, raise `max_vus` or lower `rps`.

[rtf]: https://github.com/apollographql/runtime-testing-framework
[rtf-install]: https://github.com/apollographql/runtime-testing-framework#contributing-to-rtf-as-an-apollo-engineer
[rtf-docs]: https://apollographql.github.io/runtime-testing-framework
[rtf-hello-world]: https://apollographql.github.io/runtime-testing-framework/tutorials/hello-world.html
[graph-api-key]: https://www.apollographql.com/docs/graphos/platform/access-management/api-keys/graph-api-keys#create-a-graph-api-key
[graph-api-key-roles]: https://www.apollographql.com/docs/graphos/platform/access-management/member-roles#graph-api-key-roles
[docker-install]: https://docs.docker.com/get-started/get-docker/
[compose-install]: https://docs.docker.com/compose/install/
[rust-install]: https://www.rust-lang.org/tools/install
