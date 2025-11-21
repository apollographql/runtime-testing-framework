# Writing a new scenario

This guide assumes you have completed the ["Writing a test plan"](writing-a-test-plan.md) and
["Writing a command"](writing-a-command.md) guides. You should already have a `test-plan.yaml` file
in a directory named `rtf-hello-world`. Your `test-plan.yaml` file should contain:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    command:
      name: scenario.sh
      kind: relative_path
      path: scripts/scenario.sh
    env_vars:
      SCENARIO_ENV: scenario command executed
environment:
  inline:
    name: Inline environment config
    description: An inline environment config
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment setup command executed"
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment teardown command executed"
```

## Creating a scenario file

Writing the scenario inline like this works for simple test plans. However, more comprehensive
scenarios quickly become harder to read and maintain. It is possible to create a separate file to
store your scenario config and update your test plan to refer to that file. As well as
maintainability benefits this also means the same scenario can be reused in multiple test plans. It
is also possible to update the base scenario (and environment) config using overrides in the test
plan. We cover how to use overrides in the
["Writing an environment" guide](writing-an-environment.md#using-overrides).

Let's walkthrough how to create a scenario config. First, create a `configs` directory and an empty
YAML file inside it:

```bash
mkdir configs
touch configs/scenario.yaml
```

We want to move the scenario config so it is no longer defined inline in the test plan config. Copy
the scenario config from the test plan and add to the `scenario.yaml` file:

```yaml
name: Inline scenario config
description: An inline scenario config
command:
    name: scenario.sh
    kind: relative_path
    path: scripts/scenario.sh
env_vars:
    SCENARIO_ENV: scenario command executed
```

Now, we need to update the test plan so it uses the scenario config in the `scenario.yaml` file. To
do this, we need to delete the `inline` key (and the scenario config nested beneath it) and switch
to using the `from` key:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
# --- Replace the inline scenario ---
  from:
    kind: local
    relative_path: configs/scenario.yaml
# -----------------------------------
environment:
  inline:
    name: Inline environment config
    description: An inline environment config
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment setup command executed"
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment teardown command executed"
```

Now check the test plan templates:

```bash
rtf template test-plan.yaml
```

This will result in the following YAML being printed to the terminal:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables: {}
matrix: {}
scenario:
  name: Inline scenario config
  description: An inline scenario config
  variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
    default: scenario executed with default value
  command:
    name: scenario.sh
    kind: relative_path
    path: ../scripts/scenario.sh
    args: []
  env_vars:
    SCENARIO_ENV: scenario executed with default value
  file_providers:
  - name: file.txt
    env_var: FILE_TXT
    kind: relative_path
    path: ../data/file.txt
  - name: scenario.txt
    env_var: SCENARIO_TXT
    kind: inline
    content: |
      Some inline text content for our scenario
environment:
  name: Inline environment config
  description: An inline environment config
  variable_definitions: []
  setup:
    command:
      name: setup.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "environment setup command executed"
      args: []
    env_vars: {}
    file_providers: []
    provides: []
  teardown:
    command:
      name: setup.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "environment teardown command executed"
      args: []
    env_vars: {}
    file_providers: []
```

This is the exact same test plan as we had when the scenario was defined inline. In fact, the
template command is inlining the config ahead of execution.

When writing configs with a lot of files on relative paths, it is good practice to use the `--check`
flag when running the `template` command. This will check that files on relative paths can be found.
Let's see what happens when we run with that flag:

```bash
$ rtf template test-plan.yaml --check
ERROR (command.command_provider) the requested file did not exist.: provided path was Resolved("scripts/scenario.sh")
```

When copying over our inline scenario config we forgot to account for the fact that our
`scenario.yaml` file is on a different path to our `test-plan.yaml` file. Relative paths are always
relative to the file they are defined in. Let's fix our mistake in the `scenario.yaml` file:

```yaml
name: Inline scenario config
description: An inline scenario config
command:
  name: scenario.sh
  kind: relative_path
# --- Update the path to the scenario script ---
  path: ../scripts/scenario.sh
# ----------------------------------------------
env_vars:
  SCENARIO_ENV: scenario command executed
```

If we run the `template` command again with the `--check` flag you should see the config YAML
printed to your terminal:

```bash
$ rtf template test-plan.yaml --check
name: Hello World
description: A test plan created as a guide for writing test plans
...
```

The `from` key can have two variables, `local` or `github`. In this case, we are using `local`. This
will import the scenario config from the file on the relative path defined in the `relative_path`
key.

The `github` key allows the scenario config to be imported from a GitHub repo. It is not discussed
in detail in this guide, please refer to the [framework reference docs](../../framework/index.md)
reference for more detail.

## Scenario config structure

Let's take a moment to review how scenario config is structured. There are six fields that can be
defined in the scenario

- `name` (required) is used to give the scenario an identifiable title. It can be any valid string.
  It has no impact on the execution of a scenario.
- `description` (required) is used to give more information about the scenario. It can be any valid
  string. It has no impact on the execution of a scenario.
- `variable_definitions` (optional) are used to define which variables a scenario requires to
  successfully execute. The ["using variables"](#using-variables) section explains this in more
  detail.
- `command` (required) is used to define what is executed when the scenario is run. The
  ["Writing a command" guide](writing-a-command.md) explains commands in more detail.
- `env_vars` (optional) is used to define the environment variables that are set when the `command`
  is executed.
- `file_providers` (optional) is used to define the files and data that the scenario depends on to
  execute. The ["Using file providers" guide](using-file-providers.md) explains how these are used
  in more detail. Custom providers, which can generate files dynamically, are covered in later
  guides.

## Using variables

We saw how to set variables in the test plan in the
["Writing a test plan" guide](writing-a-test-plan.md#setting-variables). However, the variable we
set was not used anywhere in the scenario or environment. To use variables in a scenario, we need to
define them in the `variables` field.

Declaring the variables here declares a contract between the scenario and test plan and lists the
variables that must be specified for the scenario to complete. The value of the variable can be set
using a default in the scenario, in the test plan or provided via the CLI. If the variable is used
by the scenario and not set via any of those methods, it will cause the test plan execution to fail.
If variables are defined for usage in the scenario but not defined in the `variables` field then the
test plan will fail to template.

Let's see that in action. We are going to replace the static environment variable variable with one
defined using a variable. Update `scenario.yaml`:

```yaml
name: Inline scenario config
description: An inline scenario config
# --- Add a variables section ---
variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
# ----------------------------
command:
  name: scenario.sh
  kind: relative_path
  path: ../scripts/scenario.sh
# --- Use the scenario_variable in the environment variables ---
env_vars:
  SCENARIO_ENV: "{{ scenario_variable }}"
# -----------------------------------------------------------
```

Let's explain how this works. The variables each have a `name` and `description`. The `name` is the
variable's identifier and is used in the template string. The `description` is there to give more
information about how and why the variable is used. Variables are templated into the config with the
`"{{ ... }}"` syntax, where `...` is replaced by the variable's `name`.

> **Note** The double curly braces and space either side of the variable name are important here. If
> the template string does not match this exactly, then rtf will error and call out there is a
> malformed template string.

Let's attempt to template the test plan:

```bash
$ rtf template test-plan.yaml
ERROR (scenario) missing template variables: scenario_variable
```

We have successfully defined a variable and where it should be used. However, we have not specified
what value it should actually have. If we attempted to run this test plan we would see the same
error. Let's define a default for this variable:

```yaml
name: Inline scenario config
description: An inline scenario config
variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
# --- Set a default for this variable ---
    default: "scenario executed with default value"
# ------------------------------------
command:
  name: scenario.sh
  kind: relative_path
  path: ../scripts/scenario.sh
env_vars:
  SCENARIO_ENV: "{{ scenario_variable }}"
```

Lets run this and see what happens:

```bash
$ rtf run test-plan.yaml
"environment setup command executed"
Running scenario from an external file
scenario executed with default value
"environment teardown command executed"
```

The second scenario `echo` statement uses the default variable. We can override this variable by
setting a different value in the test plan (this will take presence over a default). Update
`test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Add a new value for scenario_variable ---
variable_definitions:
  scenario_variable: "scenario executed with test plan variable"
# ------------------------------------------
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml

environment:
  inline:
    name: Inline environment config
    description: An inline environment config
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment setup command executed"
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment teardown command executed"
```

Now if we run:

```bash
$ run test-plan.yaml
"environment setup command executed"
Running scenario from an external file
scenario executed with test plan variable
"environment teardown command executed"
```

We can see that the variable from the test plan has overridden the default. Similarly, if the
variable is specified from the CLI it will override both the test plan variable and default:

```bash
$ run test-plan.yaml --var scenario_variable="scenario executed with cli variable"
"environment setup command executed"
Running scenario from an external file
scenario executed with cli variable
"environment teardown command executed"
```

---

In this guide, we've covered moving scenario config into its own file and using variables. Next,
we'll guide you through how to write an environment config.

**Next:** [Writing an environment](writing-an-environment.md)
