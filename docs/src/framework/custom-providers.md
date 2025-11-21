# Custom Providers

Custom providers allow you to create reusable file providers that execute custom commands to produce
files for use in your test plans. They enable you to encapsulate complex file generation logic into
reusable components that can be shared across multiple test plans, environments, and scenarios.

In this page we will cover how custom providers work, how to define them, how to declare them in
config files, and how to use them. For more detailed information on specific aspects of the
framework please see the relevant pages under the [Framework][0] section of the documentation.

## Custom Provider Definitions

A _Custom Provider Definition_ is a YAML file that defines a reusable custom provider. It specifies
the name, description, required variables, and the command to execute when the provider is used.

Custom provider definitions share structural similarities with [Scenario][1] configs but serve a
different purpose:

- **Purpose**: Custom provider definitions execute commands in order to produce file(s) that test
  plans can make use of, whereas scenario configs execute commands to perform test actions.
- **Restrictions**: Custom provider definitions cannot reference other custom providers within their
  command sections. Attempting to do so will result in a hard error during validation.
- **Templating**: Custom provider definitions are templated using arguments provided when the custom
  provider is referenced in a config file, rather than using variables from the test plan directly.

### Structure

A custom provider definition file contains the following top-level keys:

- `name`: The name of this custom provider definition.
- `description`: A brief, human readable description of the purpose and behaviour of this custom
  provider.
- `variable_definitions`: Declarations of the templating variables that this custom provider
  requires. These variables are provided as arguments when the custom provider is referenced in a
  config file.
  - Variable declarations require specifying both the variable name and a short description of how
    the variable is used.
  - Variable declarations also support an optional `default` field where you can specify a default
    scalar value to use if none is provided.
- `command`: A [Command Provider][2] that defines how the custom provider should be executed.
  - The command section can include `command`, `env_vars`, and `file_providers` as described in the
    [Command Providers][2] documentation.
  - The command should write its output to the path specified by the `$RTF_OUTPUT` environment
    variable, which RTF will set automatically.

### Example

The following is an example of a custom provider definition file:

```yaml
name: router-docker-compose
description: Generates a docker-compose.yml file for running the Apollo Router

variable_definitions:
  - name: graph_ref
    description: The Apollo graph reference to use
  - name: router_version
    description: The version of the router to use
    default: "latest"
  - name: build_router_from_source
    description: Whether to build the router from source
    default: "false"

command:
  name: generate-docker-compose.sh
  kind: relative_path
  path: scripts/generate-docker-compose.sh

env_vars:
  GRAPH_REF: "{{ graph_ref }}"
  ROUTER_VERSION: "{{ router_version }}"
  BUILD_FROM_SOURCE: "{{ build_router_from_source }}"
```

## Custom Provider Declarations

Custom provider definitions are loaded into config files through _Custom Provider Declarations_.
These declarations specify a source directory (either a local relative path or a GitHub repository)
and a mapping of provider names to definition files within that directory.

Custom provider declarations can be specified at three levels:

- **Test Plan level**: Custom providers declared in the test plan are available to the entire test
  plan, including both the scenario and environment configurations.
- **Environment level**: Custom providers declared in an environment config are only available
  within that environment's setup and teardown command sections.
- **Scenario level**: Custom providers declared in a scenario config are only available within that
  scenario's command section.

When the same custom provider name is declared at multiple levels, the most specific level takes
precedence (scenario/environment > test plan).

### Structure

A custom provider declaration contains:

- `source`: The source directory containing the custom provider definition files. This can be
  specified as either:
  - A local relative path using `kind: local` and `relative_path`
  - A GitHub repository using `kind: github` along with `org`, `repo`, `path`, and optionally
    `git_ref`
- `using`: A map of provider names to definition file paths. The provider name is what you will use
  in the `type` field when referencing the custom provider in file provider sections.

### Example: Local directory

```yaml
custom_providers:
  - kind: local
    relative_path: ../providers
    using:
      router-docker-compose: router-docker-compose.yaml
      my-custom-provider: my-custom-provider.yaml
```

### Example: GitHub repository

```yaml
custom_providers:
  - kind: github
    org: my-org
    repo: my-repo
    path: providers
    git_ref: main
    using:
      router-docker-compose: router-docker-compose.yaml
      my-custom-provider: my-custom-provider.yaml
```

## Using Custom Providers

Once you have declared custom providers in your config file, you can reference them in
`file_providers` sections using `kind: custom_provider` along with a `type` field that matches the
name from the declaration.

Any additional fields you provide beyond `name`, `env_var`, `kind`, and `type` will be passed as
arguments to the custom provider. These arguments must match the variable definitions specified in
the custom provider definition.

### Example

Given the custom provider declaration and definition shown above, you can use the custom provider in
a file provider section like this:

```yaml
file_providers:
  - name: router-docker-compose.yml
    env_var: ROUTER_DOCKER_COMPOSE
    kind: custom_provider
    type: router-docker-compose
    graph_ref: "my-graph@production"
    router_version: "v2.6.0"
    build_router_from_source: "false"
```

In this example:

- `type: router-docker-compose` matches the name in the `using` map of the custom provider
  declaration
- `graph_ref`, `router_version`, and `build_router_from_source` are passed as arguments to the
  custom provider and will be used to template the custom provider definition
- The custom provider will execute its command and write the output to the path specified by
  `$RTF_OUTPUT`, which RTF will then make available via the `ROUTER_DOCKER_COMPOSE` environment
  variable

## Responsibilities

To help clarify how custom providers work, here's what each component is responsible for:

### Custom Provider Definition

The custom provider definition file is responsible for:

- Defining the reusable provider logic (the command to execute)
- Declaring what variables/arguments the provider requires
- Specifying how those variables should be used (in env_vars, file_providers, etc.)

The definition file does not know where it will be used or what values will be provided for its
variables.

### Custom Provider Declaration

The custom provider declaration is responsible for:

- Loading custom provider definitions from a source directory (local or GitHub)
- Making those definitions available within a specific scope (test plan, environment, or scenario)
- Mapping provider names (used in `type` fields) to definition files

The declaration does not execute the provider; it only makes it available for use.

### Custom Provider (File Provider)

The custom provider file provider (specified with `kind: custom_provider`) is responsible for:

- Executing the custom provider definition's command
- Passing the provided arguments to the custom provider definition for templating
- Writing the output to the path specified by `$RTF_OUTPUT`
- Making the output available via the specified `env_var`

[0]: ./index.md
[1]: ./scenarios.md
[2]: ./command-providers.md
