# Scenarios

As outlined in the [Test Plan][0] overview, your Scenario configuration is one of the two main
pieces of configuration needed in order to run tests under RTF. Conceptually, a Scenario simply a
[Command providers][1] with enforced semantics within the overall execution of the Test Plan, being
run inbetween the Environment's setup and teardown commands.

In this page we will cover the available keys within a Scenario and outline the structure and
semantics of each. For more detailed information on specific aspects of the framework please see the
relevant pages under the [Framework][2] section of the documentation.

> An example of a valid `scenario.yaml` is provided in the [Full example](#full-example) section
> below.

## Top level keys

- `name`: The name for this Scenario configuration.
  - Uniqueness is not enforced by the `rtf` CLI but it is worthwhile ensuring that the scenarios you
    write each have unique names that can be used to distinguish them.
- `description`: A brief, human readable description of the behaviour of the Scenario.
  - If there are any pre-requesites to running this Scenario it is best to call them out here rather
    than in comments or other files (such as a README).
- `variable_definitions`: Declarations of the templating variables supported by this Scenario.
  - Variable declarations require specifying both the variable name and a short description of how
    the variable is used.
  - Variable declarations also support an optional `default` field where you can specify a default
    scalar value to use if none is provided within the [Test Plan][0].
  - If the same variable name is defined in both the Environment and Scenario used by a given Test
    Plan but with different defaults, each config file will fall back to its own default.
- `command`: See [Command Provider][1].
- `env_vars`: See [Command Provider][1].
- `file_providers`: See [Command Provider][1].

## Full example

The following is a minimal "kitchen sink" example of the structure of a valid `scenario.yaml`.

```yaml
name: example
description: An example description

variable_definitions:
  - name: my_variable
    description: "A description for my variable"
    default: "foo"

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

[0]: ./test-plans.md
[1]: ./command-providers.md
[2]: ./index.md
