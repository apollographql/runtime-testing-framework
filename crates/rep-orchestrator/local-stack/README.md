# rep-orchestrator local stack

Runs the orchestrator on a local `kind-rtf-mgmt` cluster via Tilt. PostgreSQL is deployed directly
into the cluster — no external database required.

## Prerequisites

Install via Homebrew if missing:

```
docker  kind  kubectl  helm  tilt  rtf
```

## Steps

### 1. Create clusters

From `crates/rep-orchestrator/`:

```bash
make cluster-setup
```

Creates `kind-rtf-mgmt` and `kind-rtf-workload`. Safe to re-run — skips existing clusters.

### 2. Start the stack

```bash
make cluster-up
```

Tilt will:

1. Deploy PostgreSQL into the cluster
2. Build the dev image (first run compiles all of Rust — this takes a few minutes)
3. Deploy the orchestrator via Helm, wired to the local postgres service
4. Forward ports `8035` (orchestrator) and `5432` (postgres) to localhost

Open the Tilt UI at `http://localhost:10350` to monitor resource health.

**Prod mode** (uses `docker/prod/Dockerfile`):

```bash
make cluster-up-prod
```

Note: the GCP access token expires after ~1 hour. Restart Tilt to refresh it.

### 3. Wait for healthy

In the Tilt UI (or terminal output), wait until both `postgres` and `rep-orchestrator` show green.
The orchestrator readiness probe hits `/health` every 5 seconds.

### 4. Verify

```bash
curl http://localhost:8035/health
```

This only confirms the service is up. To also verify the database connection:

```bash
curl http://localhost:8035/test-run/00000000-0000-0000-0000-000000000000/status
```

Expect a `NOT FOUND` error — the run doesn't exist, but the handler must query the database to
determine that, so a `NOT FOUND` confirms the connection is working. A 500 indicates a database
problem.

To run the full integration test suite against this stack (from `crates/rep-orchestrator/`):

```bash
make integration-tests
```

### 5. Live reload (dev mode only)

Edit any `.rs` file under `crates/`. Tilt syncs the change into the running container and
`cargo-watch` recompiles — no image rebuild required.

### 6. Tear down

Stop Tilt with `Ctrl-C`, then:

```bash
make cluster-down
```

Deletes both kind clusters.
