<!-- diataxis-type: reference -->

# Environments

An Environment defines how RTF sets up and tears down the infrastructure required for a test. There
are two execution models: **docker compose** and **script**. Docker compose is the recommended
approach; script-based environments are available as a fallback for cases that docker compose cannot
handle.

## Shared keys

The following keys apply to both execution models.

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
    scalar value to use if none is provided within the [Test Plan][0].
  - If the same variable name is defined in both the Environment and Scenario used by a given Test
    Plan but with different defaults, each config file will fall back to its own default.
- `custom_providers`: Declarations for loading Custom Provider Definitions.
  - For full details on the structure of Custom Provider Declarations and Definitions see the
    [Custom Providers][1] page of the Framework documentation.

## Docker compose environment

A docker compose Environment brings the test infrastructure up and down using `docker compose`. RTF
writes the declared compose files and any additional file providers to a temporary directory before
invoking `docker compose up`, and runs `docker compose down` during teardown.

### Docker compose keys

- `compose_files`: A list of named compose file providers describing the compose files to start.
  This field is required.
  - Each entry is a [File Provider][2] with a required `name` field. For single-file providers,
    `name` is used as the output filename; for directory providers it is used as the directory name.
- `project_name`: The docker compose project name. Optional; defaults to the environment `name` if
  not set.
- `env_vars`: Environment variables to pass to `docker compose up`.
- `file_providers`: Additional files the compose stack depends on, exposed to the stack as
  environment variables. Each entry is a [File Provider][2] with a required `name` and `env_var`
  field.

### Service labels

Services in the compose files can carry RTF-specific labels that control behaviour when the test
runs inside the [REP cluster][3]. These labels have no effect when running locally with the RTF CLI.

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
description: Setup and teardown a docker compose environment

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

## Script environment

A script Environment defines setup and teardown commands that bracket a Scenario's execution. Each
phase is a [Command Provider][5]. This model is intended as a fallback for cases that cannot be
achieved using a docker compose Environment.

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

[0]: ./test-plans.md
[1]: ./custom-providers.md
[2]: ./file-providers.md
[3]: ../../developer/explanation/concepts-and-architecture.md#rtf-and-rep
[4]: https://opentelemetry.io/docs/kubernetes/operator/automatic/
[5]: ./command-providers.md
