<!-- diataxis-type: reference -->

# Custom providers

Custom Providers are reusable file providers that execute commands to produce files. They
encapsulate file generation logic for sharing across test plans, environments, and scenarios.

> An example of a valid Custom Provider Definition is provided in the [Full example](#full-example)
> section below.

## Custom provider definitions

A _Custom Provider Definition_ is a YAML file that defines a reusable custom provider. They execute
commands to produce one or more files for other RTF config files to use, whereas Scenarios execute
test actions. Custom provider definitions cannot reference other custom providers. They are
templated using the arguments provided to the custom provider in a configuration file.

### Top level keys

- `name`: The name for this Custom Provider Definition.
  - Uniqueness is not enforced by the `rtf` CLI but it is worthwhile ensuring that the custom
    providers you write each have unique names that can be used to distinguish them.
- `description`: A brief, human readable description of the behaviour of the Custom Provider.
  - If there are any pre-requisites to using this Custom Provider it is best to call them out here
    rather than in comments or other files (such as a README).
- `variable_definitions`: Declarations of the templating variables supported by this Custom
  Provider.
  - Variable declarations require specifying both the variable name and a short description of how
    the variable is used.
  - Variable declarations also support an optional `default` field where you can specify a default
    scalar value to use if none is provided when the Custom Provider is invoked.
- `command`: See [Command Provider][1].
- `env_vars`: See [Command Provider][1].
- `file_providers`: See [Command Provider][1].

### Full example

The following is a minimal "kitchen sink" example of the structure of a valid Custom Provider
Definition file.

```yaml
name: example
description: An example Custom Provider that generates a configuration file

variable_definitions:
  - name: config_name
    description: "The name to use in the generated configuration"
  - name: config_value
    description: "The value to include in the configuration"
    default: "default_value"

command:
  name: generate-config.sh
  kind: relative_path
  path: scripts/generate-config.sh

env_vars:
  CONFIG_NAME: "{{ config_name }}"
  CONFIG_VALUE: "{{ config_value }}"

file_providers:
  - name: template.txt
    env_var: TEMPLATE_FILE
    kind: inline
    content: |
      some inline file content
```

## Custom provider declarations

Custom provider definitions are loaded into config files through _Custom Provider Declarations_.
These declarations specify a source directory (either a local relative path or a GitHub repository)
and a mapping of provider names to definition files within that directory. The provider names are
the names referenced when using the custom providers.

Custom provider declarations can be specified at three levels:

- **Test Plan level**: Custom providers declared in the test plan are only available to any
  overrides defined within the test plan itself. Unlike variables, custom providers DO NOT become
  available globally. If referencing a custom provider directly in an environment or scenario
  config, the declaration for that provider MUST be in that config file.
- **Environment level**: Custom providers declared in an environment config are only available
  within that environment's setup and teardown command sections.
- **Scenario level**: Custom providers declared in a scenario config are only available within that
  scenario's command section.

### Structure

- `kind`: The source directory containing the custom provider definition files. This can be
  specified as either:
  - A local relative path using `kind: local` and `relative_path`
  - A GitHub repository using `kind: github` along with `org`, `repo`, `path`, and optionally
    `git_ref`
- `using`: A map of provider names to definition file paths. The provider name is what you will use
  in the `type` field when referencing the custom provider in file provider sections.

### Local path example

```yaml
custom_providers:
  - kind: local
    relative_path: ./providers
    using:
      my_provider: my_provider.yaml
      another_provider: another_provider.yaml
```

### GitHub repository example

The `git_ref` field is optional and should be used if you want to target a branch that is not the
repository default.

> You _must_ have a valid GitHub access token with permissions to interact with your chosen
> repository exported as `GITHUB_TOKEN` in your shell environment for this method to work. See
> [here][2] for GitHub's documentation on how to create and manage access tokens.

```yaml
custom_providers:
  - kind: github
    org: my-org
    repo: my-repo
    path: path/to/providers
    # git_ref: main
    using:
      my_provider: my_provider.yaml
      another_provider: another_provider.yaml
```

## Using custom providers

Custom providers appear in the `file_providers` list with `kind: custom_provider`.

### Structure

- `name`: The name for the output file produced by this custom provider. This is common to all file
  providers.
- `env_var`: The environment variable that will be set to the path of the output file. This is
  common to all file providers.
- `kind`: Must be `custom_provider`.
- `type`: The name of the custom provider to use. This must match a name from the `using` map in a
  Custom Provider Declaration.
- Additional keys are passed as arguments to the custom provider and should match the
  `variable_definitions` in the Custom Provider Definition. Arguments can be literal values or
  template strings referencing variables from the config file.

### Example

The following example uses a custom provider named `my_provider` (which must be declared in the
config file's `custom_providers` section) to generate a configuration file:

```yaml
file_providers:
  - name: generated-config.yaml
    env_var: GENERATED_CONFIG
    kind: custom_provider
    type: my_provider
    config_name: "my-service"
    config_value: "{{ service_value }}"
```

In this example, `config_name` and `config_value` are the arguments to `my_provider`.

[0]: ./index.md
[1]: ./command-providers.md
[2]: https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens
