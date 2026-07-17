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
  with access to the Orchestrator after the run completes.
- **Consistent environments**: every execution runs in an isolated namespace, giving far more
  consistent performance.
- **Parallel matrix execution**: matrix dimensions run as independent executions managed by the
  Orchestrator concurrently, leading to a significant speed up in wall-clock execution time.

## Accessing the Orchestrator

The Orchestrator is deployed at `https://api.rtf.apollographql.com` and is protected by Google Cloud
IAP. Access is managed in GCP by the Runtime Readiness team.

Once access is granted, you can authenticate with GCP using the following command:

```bash
gcloud auth application-default login
```

You can check whether or not you have access by attempting to hit the healthcheck endpoint on the
Orchestrator using the `rtf` CLI like so:

```bash
rtf remote request health
```

If you see `{"ok":true}` in your terminal then you are correctly authenticated. If you are unable to
reach the Orchestrator, reach out to the Runtime Readiness team in Slack and we will set up access
for you and your team.

> **Prerequisites**
>
> - you have completed the ["Writing test plans"][4] tutorial series
> - the `rtf` CLI installed
> - the `gcloud` CLI installed
> - you have authenticated with `gcloud auth application-default login`
> - you have validated your access to the Orchestrator using `rtf remote request health`

---

**Next:** [Making your test plan Orchestrator-ready][5]

[0]: ../../reference/glossary.md
[1]: ../../reference/framework/test-plans.md
[2]: ../../reference/framework/environments.md
[3]: ../../reference/framework/scenarios.md
[4]: ../test-plans/index.md
[5]: making-your-test-plan-orchestrator-ready.md
