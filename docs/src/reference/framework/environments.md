<!-- diataxis-type: reference -->

# Environments

An Environment defines how RTF sets up and tears down the infrastructure required for a test. RTF
currently supports three environment kinds: **docker compose**, **kubernetes**, and **script**.
Depending on your use case you may find that you prefer (or need) to use one kind over another.

If you are unsure of where to start, we recommend the docker compose based environment as the
reasonable default choice that is suitable for most use cases; providing a flexible setup that can
be run both locally via the `rtf` CLI and remotely via the Orchestrator. The kubernetes environment
is intended for cases where you need to author native Kubernetes resources directly, while the
script-based environment is provided as a local-only fallback for situations where you need to
interact with the host machine executing the test plan.

For reference, the following table summarises the compatibility of each environment kind with
different RTF operations:

| Environment    | Template & check via CLI | Resolve via CLI | Run via CLI | Run via Orchestrator  |
| -------------- | ------------------------ | --------------- | ----------- | --------------------- |
| Docker compose | ✅                       | ✅              | ✅          | ✅ (via [kompose][0]) |
| Kubernetes     | ✅                       | ✅              | ❌          | ✅                    |
| Script         | ✅                       | ✅              | ✅          | ❌                    |

In terms of the trade offs being made between the different environment kinds: docker compose is the
only kind that is fully supported _everywhere_. It allows for a faster local development loop and
provides sensible defaults for deploying resources to a k8s cluster when needed, at the expense of
offering more limited control over how those deployments look. A kubernetes Environment can only be
executed under the [RTF Orchestrator][1] but allows for full control over what resources get applied
to the cluster. The script Environment is a "last resort" escape hatch for running tests on a local
machine when the environment itself can only be configured via locally executed commands outside of
RTF's control.

Regardless of which environment kind you use, the RTF configuration file you write will broadly have
the same structure. Below we cover both the shared config details utilised by all environments as
well as the configuration details unique to each kind.

> **Skipping the environment step entirely**
>
> If your Test Plan _doesn't_ require any environment to execute (e.g. you are using RTF's matrix
> and file provider features to parameterise a stand alone test suite) then you can instruct RTF to
> skip the environment stages of its execution flow by adding `skip: true` to your environment
> config file. This will take priority over any other configuration within the file:
>
> ```yaml
> name: skipped
> description: No environment needed
> skip: true
> ```

## Shared keys

The following keys apply to all three execution models.

- `name`: The name for this Environment configuration.
  - Uniqueness is not enforced by the `rtf` CLI but environments should have unique names that can
    be used to distinguish them.
- `description`: A brief, human-readable description of the behavior of the Environment.
  - Describe any prerequisites required to run this Environment, rather than placing them in
    comments or other files (such as a README).
- `variable_definitions`: Declarations of the templating variables supported by this Environment.
  - Variable declarations require specifying both the variable name and a short description of how
    the variable is used.
  - Variable declarations also support an optional `default` field where you can specify a default
    scalar value to use if none is provided within the [Test Plan][2].
  - If the same variable name is defined in both the Environment and Scenario used by a given Test
    Plan but with different defaults, each config file will fall back to its own default.
- `custom_providers`: Declarations for loading Custom Provider Definitions.
  - For full details on the structure of Custom Provider Declarations and Definitions see the
    [Custom Providers][3] page of the Framework documentation.

## Manifest file providers

The docker compose and kubernetes environments each accept a list of files that make up their
underlying infrastructure manifests (`compose_files` and `resources` respectively). These are
**not** arbitrary [File Providers][4], rather, a restricted subset of file provider kinds meant for
describing manifest content:

- [GitHub file][4] (`github_file`)
- [Inline file][4] (`inline`)
- [Inline directory][4] (`inline_dir`)
- [Relative dir][4] (`relative_dir`)
- [Relative path][4] (`relative_path`)
- [Required file][4] (`required`)
- [Templated file][4] (`templated`)

Note that there is no `env_var` field needed for manifest provider definitions: these resources are
used by RTF itself to bring up your test environment so it already has all of the information it
needs.

Declaring manifest providers is done at the top level of the config file like so:

```yaml
# docker compose
compose_files:
  - name: my-docker-compose.yaml
    kind: relative_path
    path: data/docker-compose.yaml

# kubernetes
resources:
  - name: my-manifest.yaml
    kind: relative_path
    path: data/my-manifest.yaml
```

### Referencing file providers within manifests

It is possible to reference the output of your file providers within your manifest files. The syntax
for this is the same as used by docker-compose for [variable interpolation][6], namely
`${MY_ENV_VAR}` and `$MY_ENV_VAR`. You must use the same environment variable strings as in your
file provider and environment variable declarations elsewhere within your Test Plan. Provided the
environment variable you reference is known to RTF, it will substitute the appropriate value before
deploying your environment.

#### Example

```yaml
# docker compose
services:
  nginx:
    image: nginx:alpine
    labels:
      rtf.io/file-providers: true
    command: sh -c "nginx -c ${NGINX_CONF_FILE};'"
    pull_policy: always

# kubernetes
apiVersion: apps/v1
kind: Deployment
metadata:
  name: nginx
  annotations:
    rtf.io/file-providers: "true"
spec:
  replicas: 1
  selector:
    matchLabels:
      app: nginx
  template:
    metadata:
      labels:
        app: nginx
    spec:
      containers:
        - name: nginx
          image: nginx:alpine
          args: ["sh", "-c", "nginx -c ${NGINX_CONF_FILE};'"]
          imagePullPolicy: Always
```

## Docker compose environment

A docker compose Environment brings the test infrastructure up and down using `docker compose`. RTF
writes the declared compose files and any additional file providers to a temporary directory before
invoking `docker compose up`, and runs `docker compose down` during teardown.

A docker compose environment is identified by the presence of the `compose_files` key.

### Docker compose keys

- `compose_files`: A list of named [manifest file providers](#manifest-file-providers) describing
  the compose files to start. This field is required.
- `project_name`: The docker compose project name. Optional; defaults to the environment `name` if
  not set.
- `env_vars`: Environment variables to pass to `docker compose up`.
- `file_providers`: Additional files the compose stack depends on, exposed to the stack as
  environment variables. Each entry is a [File Provider][4] with a required `name` and `env_var`
  field.

### Service labels

Services in the compose files can carry RTF-specific labels that control behaviour when the test
runs under the [RTF Orchestrator][1]. These labels have no effect when running locally with the
`rtf` CLI.

| Label                   | Value  | Effect                                                                                                                                                                                                                                                                                                                             |
| ----------------------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rtf.io/file-providers` | `true` | Mounts file provider output into the container. Required for services that read provider files at runtime.                                                                                                                                                                                                                         |
| `rtf.io/log-collection` | `true` | Container logs are uploaded to GCS after the run. Absent by default — logs are not collected unless opted in.                                                                                                                                                                                                                      |
| `rtf.io/otel`           | `true` | Injects RTF collector endpoints into the container as environment variables. The following variables are set automatically: `RTF_OTEL_COLLECTOR_GRPC` (gRPC endpoint, port 4317) and `RTF_OTEL_COLLECTOR_HTTP` (HTTP/protobuf endpoint, port 4318). Use these in your service's config instead of the standard `OTEL_*` variables. |

```yaml
services:
  router:
    image: my-router:latest
    labels:
      rtf.io/file-providers: true
      rtf.io/log-collection: true
      rtf.io/otel: true
```

### Full example

```yaml
name: docker-compose-environment
description: An example docker compose environment

variable_definitions:
  - name: message
    description: "A message to echo out"

project_name: docker-compose-env

env_vars:
  ECHO_MESSAGE: "{{ message }}"

compose_files:
  - name: compose.yaml
    kind: relative_path
    path: providers/compose.yaml

file_providers:
  - name: echo-server.py
    env_var: ECHO_SERVER_SCRIPT
    kind: relative_path
    path: providers/echo-server.py
```

## Kubernetes environment

A kubernetes Environment applies a set of native Kubernetes manifests directly, rather than relying
on RTF's docker compose to Kubernetes conversion (via [kompose][0]).

> This is intended as a more advanced operating model for users who are already familiar with
> authoring and debugging Kubernetes manifests. If you do not have experience with working directly
> with Kubernetes manifests then we advise that you do _not_ make use of this environment kind.

A kubernetes Environment can be templated, checked, and resolved locally like any other Environment,
but it can only be _executed_ under the [RTF Orchestrator][1] as shows in the
[capability table](#environments) above. While it is certainly possible to run Kubernetes workloads
locally under a tool like `kind` -- or apply the resulting manifests to a remote cluster that you
have access to -- this is not an execution model supported by RTF directly.

A kubernetes environment is identified by the presence of the `resources` key.

### Kubernetes keys

- `resources`: A list of named [manifest file providers](#manifest-file-providers) describing the
  Kubernetes resource manifests to apply. This field is required.
- `env_vars`: Environment variables made available to the environment's file providers.
- `file_providers`: Additional files the environment depends on, exposed to the environment's file
  providers as environment variables. Each entry is a [File Provider][4] with a required `name` and
  `env_var` field.

### Resource annotations

The same RTF-specific [docker compose service labels](#service-labels) can be applied to kubernetes
manifests, but they are set as **annotations** rather than labels:

> At present, only **`Deployment` resources** are supported for these annotation. If you think that
> you need this behaviour on other resource types, please reach out to the Runtime Readiness team in
> Slack to discuss your use case.

| Annotation              | Value  | Effect                                                                                                                                                                                                                                                                                                                             |
| ----------------------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rtf.io/file-providers` | `true` | Mounts file provider output into the container. Required for containers that read provider files at runtime.                                                                                                                                                                                                                       |
| `rtf.io/log-collection` | `true` | Container logs are uploaded to GCS after the run. Absent by default — logs are not collected unless opted in.                                                                                                                                                                                                                      |
| `rtf.io/otel`           | `true` | Injects RTF collector endpoints into the container as environment variables. The following variables are set automatically: `RTF_OTEL_COLLECTOR_GRPC` (gRPC endpoint, port 4317) and `RTF_OTEL_COLLECTOR_HTTP` (HTTP/protobuf endpoint, port 4318). Use these in your service's config instead of the standard `OTEL_*` variables. |

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: router
  annotations:
    rtf.io/file-providers: "true"
    rtf.io/log-collection: "true"
    rtf.io/otel: "true"
spec:
  replicas: 1
  selector:
    matchLabels:
      app: router
  template:
    metadata:
      labels:
        app: router
    spec:
      containers:
        - name: router
          image: my-router:latest
```

### Full example

```yaml
name: kubernetes-environment
description: An example Kubernetes environment

variable_definitions:
  - name: message
    description: "A message to echo out"

env_vars:
  ECHO_MESSAGE: "{{ message }}"

resources:
  - name: manifest.yaml
    kind: relative_path
    path: providers/manifest.yaml

file_providers:
  - name: echo-server.py
    env_var: ECHO_SERVER_SCRIPT
    kind: relative_path
    path: providers/echo-server.py
```

## Script environment

A script Environment defines setup and teardown commands that bracket the Scenario's execution. Each
phase is a [Command Provider][5]. This model is intended as a fallback for cases that cannot be
achieved using a docker compose or kubernetes Environment, and only runs locally via the `rtf` CLI.

### Script keys

- `setup`: A [Command Provider][5] that defines how the environment should be set up before the
  scenario is run.
- `teardown`: A [Command Provider][5] that defines how the environment should be torn down after the
  scenario is run.

### Full example

```yaml
name: example
description: An example description

variable_definitions:
  - name: my_variable
    description: "A description for my variable"
    default: "foo"

custom_providers:
  - kind: local
    relative_path: ./providers
    using:
      my_provider: my_provider.yaml

setup:
  command:
    name: my-setup-command.sh
    kind: relative_path
    path: scripts/setup.sh

  env_vars:
    MY_VARIABLE: "{{ my_variable }}"

teardown:
  command:
    name: my-teardown-command.sh
    kind: relative_path
    path: scripts/teardown.sh

  file_providers:
    - name: my-file.txt
      env_var: MY_FILE
      kind: inline
      content: My inline content
```

[0]: https://kompose.io/
[1]: ../../tutorials/running-with-the-orchestrator/index.md
[2]: ./test-plans.md
[3]: ./custom-providers.md
[4]: ./file-providers.md
[5]: ./command-providers.md
[6]: https://docs.docker.com/compose/how-tos/environment-variables/variable-interpolation/
