<!-- diataxis-type: howto -->

# Getting set up with the Orchestrator

> **Note**: This page describes Apollo internal processes. Concrete details are maintained in
> internal Confluence documentation linked below.

This guide covers getting access to the [RTF Orchestrator Service][0], running Test Plans with it
locally and from CI, and diagnosing failed runs. For a step-by-step introduction, see the
["Running with the Orchestrator"][1] tutorial.

## Authenticating with the Orchestrator service

Access to the Orchestrator is managed by the Runtime Readiness team. See
[requesting Orchestrator access][2] for how to request it.

Once access is granted, authenticate with GCP using the `gcloud` CLI:

```bash
gcloud auth application-default login
```

Verify your access and identity:

```bash
rtf remote request whoami
```

The first time you call the Orchestrator, the `rtf` CLI will open a browser window to complete the
sign-in process. The resulting token is cached locally for later requests so you should only have to
do this once.

## Keeping the CLI up to date

The `rtf` CLI can be installed from the root of the RTF repository using the following command:

```bash
cargo install --path crates/rtf-cli
```

Very occasionally, changes to the Orchestrator API will require you to install a newer build of the
CLI. If `rtf remote` commands start failing, please try pulling latest on `main` and reinstalling.
If the errors persist, please reach out to the Runtime Readiness team in
`#proj-runtime-testing-framework` to report the issue.

## Running a Test Plan under the Orchestrator

Choose the command based on where the Test Plan lives and whether you want to wait for the result:

| Test Plan                     | Trigger and return     | Trigger and wait for the result |
| ----------------------------- | ---------------------- | ------------------------------- |
| Local file or GitHub path     | `rtf remote run`       | `rtf remote ci-run`             |
| [Known Test Plan][11] by UUID | `rtf remote run-known` | `rtf remote ci-run-known`       |

`run` and `run-known` print the Test Run ID and a link to the run in the Orchestrator UI. `ci-run`
and `ci-run-known` poll until the run completes and exit non-zero if it doesn't succeed. As the
names imply, these commands are intended for programmatic use in CI jobs but they can also be useful
if you are triggering runs locally from a script or using an agent. That way, the `rtf` CLI handles
polling and error reporting for you.

As with the local `rtf run` command, Test Plan variables can be set using the `--var <key>=<value>`
and `--vars <file.json>` flags:

```bash
rtf remote ci-run test-plan.yaml --var router_version=v2.18.0
```

See [Using `rtf remote`][3] for further details on how each `rtf` subcommand works.

## Understanding Orchestrator workload scheduling

When running Test Plans under the Orchestrator each Test Run executes inside of a single
[Workload Pool][12] containing one or more Kubernetes clusters. The pool in which a given workload
is run depends on both how the Test Plan was submitted to the Orchestrator and (optionally) server
side configuration for Known Test Plans.

### The default pool

If the run was initiated using `rtf remote run` or `rtf remote ci-run` then it will always schedule
inside of the default pool. This is a shared environment where pods from your Environment and
Scenario will be spread over the available nodes from all clusters present in the pool. For any
individual execution your Environment and Scenario resources will be scheduled within a single
cluster, but no guarantee is given that all executions will run on the _same_ cluster.

Given the shared nature of this pool it is primarily intended for use cases such as functional
testing, short running CI tests and iterating on new Test Plans. It is _not_ appropriate for heavier
workloads such as performance testing or workloads that are sensitive to noisy neighbours.

### Pinned pools

All other workload pools are configured for specific purposes and require an Admin to pin your Test
Plan to that particular pool. This is only possible for Known Test Plans that have been registered
with the Orchestrator (also an Admin action).

Depending on the configuration of the pool in question, it may also be possible to request that your
tests run on a [Dedicated Cluster][13] for increased stability and limited control over how the
cluster is configured before your run starts. See [Orchestrator internal processes][4] for further
details on how to get your Test Plan pinned to a particular pool or request that a new pool be
provisioned.

> In order for your Known Test Plan to be associated with your server side configuration, you _must_
> trigger runs using `rtf remote run-known`, `rtf remote ci-run-known` or directly from the UI.
> Triggering using other methods will treat that run as being anonymous, resulting in your Test Plan
> being run in the default pool.

### Rate limits and Test Executions

All workload pools run under a set of [rate limits][5] that are configured per-pool. These limits
are enforced per-user with automation users being limited per-repo they are running test plans for.
As such, you should prefer writing Test Plans that make use of RTF's matrix feature over triggering
multiple single-execution runs in a loop. Loops will hit your per-user rate limit faster in addition
to resulting in poorer workload scheduling behaviour from the Orchestrator as we are unable to cache
file provider data as efficiently.

## Running from CI

The only supported way to run Test Plans under the Orchestrator from CI is the
[`rtf-orchestrator-run-known`][6] GitHub Action. Calling `rtf remote` commands directly from a
GitHub Actions workflow won't work, as the runner isn't authenticated with the Orchestrator.

The action triggers a [Known Test Plan][11] as an [Automation User][14] for your repository, waits
for the run to complete, and fails the job if the run doesn't succeed. Before you can use it, your
repository needs to be set up to authenticate with the Orchestrator. See
[setting up CI for the Orchestrator][7] for how to request this.

Once your repository is set up, add the action to a workflow:

```yaml
jobs:
  run-test-plan:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write
    steps:
      - uses: actions/checkout@v4
      - uses: apollographql/release-tooling/rtf-orchestrator-run-known@<commit-sha>
        with:
          test_plan_id: <test-plan-uuid>
          git_ref: <git-ref>     # optional
          vars_file: vars.json   # optional
```

## Using private Docker images in Test Plans

Public Docker images should be usable in Test Plans without any further setup. Images that are not
public must be published to Apollo's internal Docker registry first. See
[publishing private images for the Orchestrator][8].

## Diagnosing a failed run

### Fetching the output

The `rtf` CLI can be used to fetch the scenario log and output zipfile for either a single
execution, or all executions within a parent run:

```bash
rtf remote execution-output <execution-id>
rtf remote run-output <run-id>
```

The run and execution pages in the [Orchestrator UI][9] provide links to both of these outputs as
well as further links to each execution's logs in GCP and its metrics in Grafana. The scenario log
and output zipfile should be sufficient for successful runs, but we recommend making use of GCP logs
and Grafana for digging into failures or manual investigation into more complex setups over
complicating your test plan to export larger amounts of data in your output zipfile.

### `UNRUNNABLE: timed out waiting for deployments`

**Cause:** an Environment service failed to become healthy. This is commonly due to invalid service
configuration (such as a router config option that the router version doesn't support) or attempting
to use a Docker image that the Orchestrator doesn't have access to.

**Solution:** check the failing service's container logs via the GCP logs link on the execution
page. To validate configuration before triggering, run the Test Plan locally with `rtf run`.

### A service fails to find config or data files on startup

**Cause:** the files are likely not present in the container.

**Solution:** supply them with [File Providers][10] and add the `rtf.io/file-providers: "true"`
label to the service. Do not mount provider paths with explicit volumes.

## Cancelling an in-flight run

You can cancel an in-flight run using the `cancel` button on its summary page in the Orchestrator
UI. Alternatively, you can use the `rtf` CLI to perform the same action like so:

```bash
rtf remote request -X POST -d '{"id": "<run-id>"}' admin/purge-run
```

You are able to cancel runs that you initiated and runs of Known Test Plans you own. For anything
else you will need to request that an Admin cancel the run. See
[Orchestrator internal processes][4].

[0]: ../../reference/glossary.md#rtf-orchestrator-service
[1]: ../../tutorials/running-with-the-orchestrator/index.md
[2]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/2146893825/GCP+Access+-+Release+Tooling
[3]: ../../reference/orchestrator/rtf-remote.md
[4]: ../../reference/orchestrator/internal-processes.md
[5]: ../../reference/orchestrator/limits-permissions-and-concepts.md#rate-limits
[6]: https://github.com/apollographql/release-tooling/tree/main/rtf-orchestrator-run-known
[7]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/2139947048/Onboarding+To+Our+Tooling
[8]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/2139553811/Build+Publish
[9]: orchestrator-ui.md
[10]: ../../reference/framework/file-providers.md
[11]: ../../reference/glossary.md#known-test-plan
[12]: ../../reference/glossary.md#workload-pool
[13]: ../../reference/glossary.md#dedicated-cluster
[14]: ../../reference/glossary.md#automation-user
