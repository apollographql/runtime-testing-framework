<!-- diataxis-type: tutorial -->

# Writing a test plan

Create an empty directory and make it your working directory:

```bash
mkdir rtf-hello-world
cd rtf-hello-world
```

Create an empty YAML file for the test plan:

```bash
touch test-plan.yaml
```

Add the following content to `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "scenario command executed"
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

This will all be explained as we progress through the guide, for now all you need to know is this is
the simplest test plan it is possible to write in rtf.

## Adding required fields

An rtf test plan has four required fields: `name`, `description`, `scenario`, and `environment`. The
sections below add each of these required fields and explain them in more detail.

### `name`

Add the `name` field to the `test-plan.yaml` file

```yaml
name: Hello World
```

`name` is used to give each test plan an identifiable title. It can be any valid string. The value
used for the `name` field has no impact on the execution of the test plan. This makes it easier to
work with the test plan programmatically.

### `description`

Add the `description` field to the `test-plan.yaml` file

```yaml
name: Hello World
description: Created as a guide for writing test plans
```

`description` is used to give more information about the test plan for future users. It can be any
valid string. The value used for the `description` field has no impact on the execution of the test
plan itself. This is a useful place to add links or reference materials and should be preferred over
adding that context to inline comments.

### `scenario`

The `scenario` is used to define the configuration and command that runs the actual testing logic in
the test plan. The `scenario` can be defined inline within the test plan or in its own file. In this
guide, we'll define the scenario inline. The guide on
[writing a new scenario](writing-a-scenario.md) covers how to define a scenario in a separate file.

Add the `scenario` field to the `test-plan.yaml` file.

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "scenario command executed"
```

This is the simplest `scenario` it is possible to define.

- The `inline` field is used to indicate the scenario will be defined in the test plan file.
- The `name` and `description` fields are required and used to identify the scenario and work the
  same as `name` and `description` in the test plan.
- `command` defines what will be executed when rtf executes the scenario. This is the simplest case
  which executes a single command. The guide on [writing a new scenario](writing-a-scenario.md)
  covers how to execute files or more complex scripts.

### `environment`

The `environment` is used to define the configuration and commands that setup the environment for
testing and tear it down after the test has completed. The `environment` can be defined inline
within the test plan or in its own file. In this guide, we'll define the environment inline. The
guide on [writing a new environment](writing-an-environment.md) covers how to define an environment
in a separate file.

Add the `environment` field to the `test-plan.yaml` file:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "scenario command executed"
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

This is the simplest `environment` it is possible to define.

- The `inline` field is used to indicate the environment will be defined in the test plan file.
- The `name` and `description` fields are required and used to identify the scenario and work the
  same as `name` and `description` in the test plan.
- The `setup` field defines what will happen during the environment setup phase of `rtf run`.
- The `teardown` field defines what will happen during the environment teardown phase of `rtf run`.
- `command` defines what will be executed when rtf executes the `setup` and `teardown`. This is the
  simplest case which executes a single command. The guide on
  [writing a new environment](writing-an-environment.md) covers how to execute files or more complex
  scripts.

## Checking the test plan

Now, let's check that the test plan has been defined correctly:

```bash
rtf template test-plan.yaml
```

This should result in the fully templated test plan being printed to the terminal:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
matrix: {}
scenario:
  name: Inline scenario config
  description: An inline scenario config
  variable_definitions: []
  command:
    name: scenario.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      echo "scenario command executed"
    args: []
  env_vars: {}
  file_providers: []
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

This highlights two optional fields for test plans that have not yet been used; `variables` and
`matrix`. These are discussed more below.

## Running the test plan

Before looking at `variables` and `matrix`, let's run the test plan:

```bash
rtf run test-plan.yaml
```

You should see output similar to this:

```
"environment setup command executed"
"scenario command executed"
"environment teardown command executed"
```

This also creates an `output` directory.

The `output` directory contains two files, `resolved-test-plan.yaml` and `test-plan-variables.json`.

- `resolved-test-plan.yaml` contains the fully resolved test plan config. This should be the same as
  what was shown in the `rtf template` command. This is a way to verify the test plan that ran to
  give you the output.
- `test-plan-variables.json` contains the variables used during the execution of the test plan. This
  is empty since no variables were set.

Remove the output directory before continuing (forgetting to do this will result in an error next
time `rtf run` is used):

```bash
rm -rf output/
```

> **Note** rtf is deliberately configured to not overwrite an existing output directory. This is so
> you cannot accidentally overwrite output you intend to keep. The `--output` flag can be used with
> `rtf run` to set a different output directory if you want to keep the existing output and run a
> new test.

## Setting variables

The `variables` field is used to set global variables that can be referenced in your scenario and/or
environment. Any variables set in the test plan config can be overridden using the `--var` and
`--vars` flags in the rtf CLI (see the
[modifying variables section of the hello world guide](../hello-world.md#modifying-variables) for
more information).

Let's add some example variables to `test-plan.yaml`. We are also going to update the scenario
command to use this variable. The ["Writing a command" section](writing-a-command.md) will explain
how this works, for now just add the configuration:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Add variables ---
variables:
  example_variable: example variable
# ------------------
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
# --- Update scenario ---
    variable_definitions:
      - name: example_variable
        description: An example variable
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh
        echo "$EXAMPLE_VARIABLE"
    env_vars:
      EXAMPLE_VARIABLE: "{{ example_variable }}"
# -----------------------
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

You can see the value being used by running the test plan again:

```bash
rtf run test-plan.yaml
```

Output:

```
environment setup command executed
example variable
environment teardown command executed
```

Now, the `test-plan-variables.json` file contains the variable that we set in the test plan:

```bash
cat output/test-plan-variables.json
```

Output:

```json
{
  "example_variable": "example variable"
}
```

## Using a matrix

The `matrix` field is used to create a matrix of variable dimensions to iterate over (see the
[matrix variables section of the hello world guide](../hello-world.md#matrix-variables) for more
information). A matrix can only be defined in the test plan config.

Let's add a matrix to and remove the `variables` from our `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Replace variables with a matrix ---
matrix:
  variant_names: "${example_variable}"
  dimensions:
    example_variable:
      - variable1
      - variable2
# ------------------------------------
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    variable_definitions:
      - name: example_variable
        description: An example variable
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh
        echo "$EXAMPLE_VARIABLE"
    env_vars:
      EXAMPLE_VARIABLE: "{{ example_variable }}"
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

Run the test plan again (make sure the `output` directory has been deleted after previous test
runs):

```bash
rtf run test-plan.yaml
```

The terminal output will look different this time - the environment and scenario commands are
executed twice. The `output` directory will also have a different structure:

```bash
ls output/
```

Output:

```
variable1        variable2
```

Let's look at each of those matrix directories to see the different variable values used per
execution:

```bash
cat output/variable1/test-plan-variables.json
```

Output:

```json
{
  "example_variable": "variable1"
}
```

```bash
cat output/variable2/test-plan-variables.json
```

Output:

```json
{
  "example_variable": "variable2"
}
```

The `example-variable`'s variable changes per execution.

---

In this guide we have covered writing the simplest possible test plan. Next, we will guide you
through how to write more powerful commands.

**Next:** [Writing a command](writing-a-command.md)
