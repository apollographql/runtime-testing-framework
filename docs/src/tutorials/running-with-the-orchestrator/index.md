<!-- diataxis-type: tutorial -->

# Running test plans with the RTF Orchestrator Service

This section guides you through running RTF test plans using the [RTF Orchestrator Service][0]: a
managed, remote execution service for running RTF Test Plans at Apollo. Instead of running your
[Test Plan][1] locally with `rtf run`, you submit it to the Orchestrator which provisions an
isolated Kubernetes namespace for deploying your [Environment][2] before then executing your
[Scenario][3] and storing the results in GCS for you to retrieve.

## Why use the Orchestrator?

When you run a Test Plan with `rtf run`, RTF handles spinning up your test environment and running
your scenario on your local machine. When using scripted environments and scenarios this is
incredibly flexible but also highly susceptible to being affected by how that local machine is
configured. Running locally using a `docker compose` based environment and `docker` based scenario
helps with making things more reproducible, but you are still subject to the constraints of the
local machine you are running on.

In contrast, the Orchestrator provides a dedicated execution environment that handles running your
test plan in an isolated Kubernetes namespace. This provides several advantages:

- **Minimal local requirements**: the machine triggering the test run only needs to be able to
  submit the Test Plan to the Orchestrator, not run the services under test.
- **Output storage**: logs and output artifacts are uploaded to GCS and can be retrieved by anyone
  with access to the Orchestrator after the run completes. Output is only
  [retained for a limited time][9].
- **Consistent environments**: every execution runs in an isolated namespace, giving far more
  consistent performance.
- **Parallel matrix execution**: matrix dimensions run as independent executions managed by the
  Orchestrator concurrently, leading to a significant speed up in wall-clock execution time.

## Accessing the Orchestrator

The Orchestrator is deployed at [api.rtf.apollographql.com][6], with the [Orchestrator UI][7]
alongside it, and is protected by Google Cloud IAP. Access is managed by the Runtime Readiness team.

Before starting this tutorial, follow the [Getting set up with the Orchestrator][8] guide to request
access, authenticate with GCP, and check that you are able to reach the Orchestrator.

> **Prerequisites**
>
> - You have completed the ["Writing test plans"][4] tutorial series
> - The `rtf` CLI installed
> - The `gcloud` CLI installed
> - You have completed the [Getting set up with the Orchestrator][8] guide

---

**Next:** [Making your test plan Orchestrator-ready][5]

[0]: ../../reference/glossary.md#rtf-orchestrator-service
[1]: ../../reference/framework/test-plans.md
[2]: ../../reference/framework/environments.md
[3]: ../../reference/framework/scenarios.md
[4]: ../test-plans/index.md
[5]: making-your-test-plan-orchestrator-ready.md
[6]: https://api.rtf.apollographql.com
[7]: https://api.rtf.apollographql.com/ui
[8]: ../../howto/orchestrator/getting-set-up.md
[9]: ../../reference/orchestrator/limits-permissions-and-concepts.md#output-storage
