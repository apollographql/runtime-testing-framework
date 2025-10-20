# Writing a test plan

---

- [Getting started](#getting-started)
- [Adding required fields](#adding-required-fields)
  - [`name`](#name)
  - [`description`](#description)
  - [`scenario`](#scenario)
  - [`environment`](#environment)
- [Checking the test plan](#checking-the-test-plan)
- [Running the test plan](#running-the-test-plan)
- [Setting values](#setting-values)
- [Using a matrix](#using-a-matrix)

---

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
        name: setup.sh
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
values:
  example_value: "value"
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
        name: setup.sh
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
values:
  example_value: value
matrix: {}
scenario:
  name: Inline scenario config
  description: An inline scenario config
  values: []
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
  values: []
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

This highlights two optional fields for test plans that have not yet been used; `values` and
`matrix`. These are discussed more below.

## Running the test plan

Before looking at `values` and `matrix`, let's run the test plan:

```bash
rtf run test-plan.yaml
```

This results in the following terminal output and an `output` directory:

```bash
"environment setup command executed"
"scenario command executed"
"environment teardown command executed"
```

The `output` directory contains two files, `resolved-test-plan.yaml` and `test-plan-values.json`.

- `resolved-test-plan.yaml` contains the fully resolved test plan config. This should be the same as
  what was shown in the `rtf template` command. This is a way to sanity check the test plan that ran
  to give you the output.
- `test-plan-values.json` contains the values used during the execution of the test plan. This is
  empty since no values were set.

Remove the output directory before continuing (forgetting to do this will result in an error next
time `rtf run` is used):

```bash
rm -rf output/
```

> **Note** rtf is deliberately configured to not overwrite an existing output directory. This is so
> you cannot accidentally overwrite output you intend to keep. The `--output` flag can be used with
> `rtf run` to set a different output directory if you want to keep the existing output and run a
> new test.

## Setting values

The `values` field is used to set global values that can be referenced in your scenario and/or
environment. Any values set in the test plan config can be overridden using the `--value` and
`--values` flags in the rtf CLI (see the
[modifying values section of the hello world guide](../hello-world.md#modifying-values) for more
information).

Let's add some example values to `test-plan.yaml`. We are also going to update the scenario command
to use this value. The ["Writing a command" section](writing-a-command.md) will explain how this
works, for now just add the configuration:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Add values ---
values:
  example_value: example value
# ------------------
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
# --- Update scenario ---
    values:
      - name: example_value
        description: An example value
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh
        echo "$EXAMPLE_VALUE"
    env_vars:
      EXAMPLE_VALUE: "{{ example_value }}"
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
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment teardown command executed"
```

You can see the value being used by running the test plan again:

```bash
$ rtf run test-plan.yaml
environment setup command executed
example value
environment teardown command executed
```

Now, the `test-plan-values.json` file contains the value that we set in the test plan:

```bash
$ cat output/test-plan-values.json 
{
  "example_value": "example value"
}
```

## Using a matrix

The `matrix` field is used to create a matrix of values to iterate over (see the
[matrix values section of the hello world guide](../hello-world.md#matrix-values) for more
information). A matrix can only be defined in the test plan config.

Let's add a matrix to and remove the `values` from our `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Replace values with a matrix ---
matrix:
  variant_names: "${example_value}"
  dimensions:
    example_value:
      - value1
      - value2
# ------------------------------------
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    values:
      - name: example_value
        description: An example value
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh
        echo "$EXAMPLE_VALUE"
    env_vars:
      EXAMPLE_VALUE: "{{ example_value }}"
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
        name: setup.sh
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
$ ls output/
value1        value2
```

Let's look at each of those matrix directories to see the different values used per execution:

```bash
$ cat output/value1/test-plan-values.json 
{
  "example_value": "value1"
}

$ cat output/value2/test-plan-values.json
{
  "example_value": "value2"
}
```

The `example-value`'s value changes per execution.

---

In this guide we have covered writing the simplest possible test plan. Next, we will guide you
through how to write more powerful commands.

**Next:** [Writing a command](writing-a-command.md)
