<!-- diataxis-type: tutorial -->

# Making your test plan Orchestrator-ready

In this guide, we'll take the Test Plan from the ["Writing test plans"][0] tutorial series and
updated it so it can be run under the RTF Orchestrator. To do this we will be adding `rtf.io` docker
compose labels that tell the Orchestrator how to handle deploying and interactingh with your
services.

> **Prerequisites**
>
> - Completed the ["Writing test plans"][0] tutorial series
> - An `rtf-hello-world` directory in the state it was at the end of that series

## Restrictions on Orchestrator Test Plans

The Orchestrator requires that Test Plans submitted to it are written using `docker compose` based
Environments and `docker` based scenarios: script based Test Plans are not supported. Enforcing this
allows the Orchestrator to convert your docker compose based environment into Kubernetes resources
using [kompose][1] which are then patched with [kustomize][2] according to the labels detailed
below.

The Test Plan you built in the ["Writing test plans"][0] series already uses a docker compose based
environment and docker based scenario, so all we need to add is the appropriate labels and it will
be ready to run!

## The `rtf.io` labels

The Orchestrator uses docker compose service labels in the `rtf.io` namespace to determine how it
should handle your services during deployment and execution. These labels have no effect when you
run a Test Plan locally with `rtf run`: they exist solely to allow the Orchestrator to replicate the
execution behaviour of `rtf run` in Kubernetes where we can't rely on shared local filesystem.

Currently there are three labels available:

| Label                   | Value  | Effect                                                                                                                                                                                                                                                                                                                             |
| ----------------------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rtf.io/file-providers` | `true` | Mounts file provider output into the container. Required for services that read provider files at runtime.                                                                                                                                                                                                                         |
| `rtf.io/log-collection` | `true` | Container logs are uploaded to GCS after the run. Absent by default, logs are not collected unless opted in.                                                                                                                                                                                                                       |
| `rtf.io/otel`           | `true` | Injects RTF collector endpoints into the container as environment variables. The following variables are set automatically: `RTF_OTEL_COLLECTOR_GRPC` (gRPC endpoint, port 4317) and `RTF_OTEL_COLLECTOR_HTTP` (HTTP/protobuf endpoint, port 4318). Use these in your service's config instead of the standard `OTEL_*` variables. |

### Adding log collection

When set to `"true"` on a service, the `rtf.io/log-collection` label will instruct the orchestrator
to collect that service's container logs after the Scenario completes and includes them in the
output zip file that gets pushed to GCS under `output/logs/`.

Let's add it to the `hello-world` service in our `environment.yaml`:

```yaml
name: Docker compose environment config
description: A docker compose environment config
compose_files:
  - name: docker-compose.yaml
    kind: inline
    content: |
      services:
        hello-world:
          image: nginx:alpine
          ports:
            - "8080:80"
# -------- Add rtf.io labels --------
          labels:
            rtf.io/log-collection: "true"
# -----------------------------------
```

> **Note** Docker compose label values must be strings. Unquoted `true` is interpreted as a boolean
> and will not match the expected string value `"true"`. Always quote the value.

Remember: there is no automatic log collection. You must add the `rtf.io/log-collection` label to
each service you want to collect logs from.

### Accessing file provider output

Any service that needs to access the output from RTF [file providers][3] needs to be annotated with
the `rtf.io/file-providers: "true"` label. This instructs the Orchestrator to add an init container
that will resolve and mount the required file provider output into the container via a shared
volume. As with `rtf run` the environment variables you specify in your test plan will contain the
correct absolute path for the requested resources, regardless of whether you run locally or under
the Orchestrator.

```yaml
services:
  router:
    image: "ghcr.io/apollographql/router:v2.14.0"
    labels:
      rtf.io/log-collection: "true"
      rtf.io/file-providers: "true"
    # The SUPERGRAPH_SCHEMA and ROUTER_CONFIG environmment variables here
    # are set according to the file providers that have been added using
    # the label.
    command: -s ${SUPERGRAPH_SCHEMA} -c ${ROUTER_CONFIG}
```

> **Note** A service using `rtf.io/file-providers` must not attempt to define explicit volume mounts
> for individual provider paths. Doing so will cause the Orchestrator to reject your Test Plan as
> invalid.

## Verifying the test plan

As with local execution, we run `rtf template` to confirm the updated test plan still parses
correctly:

```bash
rtf template test-plan.yaml --check
```

The output should be similar to:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: scenario env var value from test plan
matrix:
  variant_names: null
  dimensions: {}
  include: []
custom_providers: []
scenario:
  ...
environment:
  ...
```

Your test plan is now ready to submit to the orchestrator.

## Next steps

In this guide, we added `rtf.io` labels to our docker compose environment so the Orchestrator knows
how to handle our services. Next, we'll submit the Test Plan to the Orchestrator for execution and
look at how we monitor the status of the execution before fetching the results.

[Running and fetching results][4]

[0]: ../test-plans/index.md
[1]: https://kompose.io/
[2]: https://kustomize.io/
[3]: ../../reference/framework/file-providers.md
[4]: running-and-fetching-results.md
