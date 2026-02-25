<!-- diataxis-type: reference -->

# Scenarios

A Scenario defines the test command to execute within a Test Plan. It is a [Command Provider][0]
that runs between the Environment's setup and teardown phases.

## Top level keys

- `name`: The name for this Scenario configuration.
  - Uniqueness is not enforced by the `rtf` CLI but scenarios should have unique names that can be
    used to distinguish them.
- `description`: A brief, human readable description of the behaviour of the Scenario.
  - If there are any pre-requisites to running this Scenario it is best to call them out here rather
    than in comments or other files (such as a README).
- `variable_definitions`: Declarations of the templating variables supported by this Scenario.
  - Variable declarations require specifying both the variable name and a short description of how
    the variable is used.
  - Variable declarations also support an optional `default` field where you can specify a default
    scalar value to use if none is provided within the [Test Plan][1].
  - If the same variable name is defined in both the Environment and Scenario used by a given Test
    Plan but with different defaults, each config file will fall back to its own default.
- `custom_providers`: Declarations for loading Custom Provider Definitions.
  - For full details on the structure of Custom Provider Declarations and Definitions see the
    [Custom Providers][2] page of the Framework documentation.

The remaining keys depend on the command variant:

## Docker command

Runs the scenario inside a Docker container. The container has access to the output of any
`file_providers`, with their paths remapped to `/output/…` inside the container.

- `docker`: Configuration for the Docker image and command to run.
  - `image`: The Docker image to use (e.g. `alpine`, `ghcr.io/my-org/my-image`). Supports
    templating.
  - `tag`: The image tag to pull. Defaults to `latest` if omitted. Supports templating.
  - `command`: The command executed under `sh -c` inside the container. Supports templating.
- `env_vars`: Environment variables passed into the container. Values support templating.
- `file_providers`: See [File Providers][3]. It is possible to run a script from a file provider as
  your command (see the [full example](#full-example) below). File providers won't be executable
  directly so you need to make sure to run using `sh $MY_SCRIPT`, not `./$MY_SCRIPT`.

### Full Example

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

docker:
  image: alpine
  tag: "3.18"
  command: "sh $MY_SCRIPT"

env_vars:
  MY_VARIABLE: "{{ my_variable }}"

file_providers:
  - name: my-script.sh
    env_var: MY_SCRIPT
    kind: relative_path
    path: scripts/scenario.sh
```

## Script command

Runs the scenario as a script or binary executed directly on the host. See [Command Provider][0] for
the full set of supported command kinds.

- `command`: See [Command Provider][0].
- `env_vars`: Environment variables passed to the command. Values support templating.
- `file_providers`: See [Command Provider][0].

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

command:
  name: my-scenario-command.sh
  kind: relative_path
  path: scripts/scenario.sh
  args: ["a", "b"]

env_vars:
  MY_VARIABLE: "{{ my_variable }}"

file_providers:
  - name: my-file.txt
    env_var: MY_FILE
    kind: inline
    content: My inline content
```

[0]: ./command-providers.md
[1]: ./test-plans.md
[2]: ./custom-providers.md
[3]: ./file-providers.md
