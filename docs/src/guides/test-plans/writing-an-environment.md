# Writing a new environment

This guide assumes you've completed the ["Writing a scenario"](writing-a-scenario.md) guide. You
should already have the files in a directory named `rtf-hello-world`. Your directory should be in
the state it was at the end of that guide.

```bash
$ ls -R
configs         scripts         test-plan.yaml

configs:
scenario.yaml

scripts:
scenario.sh
```

## Creating an environment file

Creating a separate environment file works exactly the same way and has the same benefits as
creating a separate scenario file outlined in the
["Writing a scenario" guide](writing-a-scenario.md#creating-a-scenario-file).

Let's update our test plan to specify the environment in a separate file:

```bash
touch configs/environment.yaml
```

Make sure the `environment.yaml` contains a copy of the environment config currently in your
`test-plan.yaml` file:

```yaml
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

Finally, update the `test-plan.yaml` file to use the new `environment.yaml` file:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: "scenario executed with test plan variable"
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
environment:
  from:
    kind: local
    relative_path: configs/environment.yaml
```

Let's verify this has made no material difference to the templated test plan:

```bash
$ rtf template test-plan.yaml --check
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: scenario executed with test plan variable
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
    SCENARIO_ENV: scenario executed with test plan variable
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

## Environment config structure

Now is a good time to review how the environment config is structured. The main difference compared
to the [scenario config](writing-a-scenario.md#scenario-config-structure) is that an environment can
run two commands using the `setup` and `teardown` keys. The `setup` key has an additional field
called `provides` which we'll explain in more detail in the
["Using provides" section](#using-provides). The fields in an environment config are:

- `name` (required) is used to give the environment an identifiable title. It can be any valid
  string. It has no impact on the execution of an environment.
- `description` (required) is used to give more information about the environment. It can be any
  valid string. It has no impact on the execution of an environment.
- `variable_definitions` (optional) are used to define which variables an environment requires to
  successfully execute. This works the same as it does for a scenario and is explained more in the
  ["using variables"](writing-a-scenario.md#using-variables) section of that guide.
- `setup` (required) is used to define the command that executes at the start of the `rtf run`
  command. It is intended to be used to create and configure the environment for the scenario to
  test. It requires the following keys:
  - `command` (required) is used to define what is executed when the environment setup is run. The
    ["Writing a command" guide](writing-a-command.md) explains commands in more detail.
  - `env_vars` (optional) is used to define the environment variables that are set when the
    `command` is executed.
  - `file_providers` (optional) is used to define the files and data that the environment setup
    depends on to execute. The ["Using file providers" guide](using-file-providers.md) explains how
    these are used in more detail. Custom providers, which can generate files dynamically, are
    covered in later guides.
  - `provides` (optional) is unique to the environment setup and is used to set variables that can
    only be known at runtime. This is explained more in the
    ["Using provides" section](#using-provides).
- `teardown` (required) is used to define the command that executes at the end of the `rtf run`
  command. It is intended to be used to collect results and shutdown the environment the scenario
  tested. It requires the following keys:
  - `command` (required) is used to define what is executed when the environment teardown is run.
    The ["Writing a command" guide](writing-a-command.md) explains commands in more detail.
  - `env_vars` (optional) is used to define the environment variables that are set when the
    `command` is executed.
  - `file_providers` (optional) is used to define the files and data that the environment teardown
    depends on to execute. The ["Using file providers" guide](using-file-providers.md) explains how
    these are used in more detail. Custom providers, which can generate files dynamically, are
    covered in later guides.

## Using provides

The `provides` key is used to take variables from the environment setup and make them available to
the scenario and environment teardown steps. The most common use case for this is when the
environment setup starts a process with some ID that can only be known at runtime. That ID is
required by the environment teardown so it can stop the process once the test is done. In practice,
this can be used to set any variable for the scenario and environment teardown to use.

Before we look at how `provides` works, let's update our environment setup command in
`environment.yaml`:

```yaml
name: Inline environment config
description: An inline environment config
setup:
# --- Update the setup command ---
  command:
    name: setup.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      PROCESS_ID="1"
      echo "Environment setup complete. PROCESS_ID=$PROCESS_ID"
# --------------------------------
teardown:
  command:
    name: setup.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      echo "environment teardown command executed"
```

The environment setup now creates a `PROCESS_ID` with a value of `1`. We'll want to be able to use
this in our teardown script. In preparation, let's update the teardown command so it can use a
`PROCESS_ID` environment variable that will eventually come from the setup:

```yaml
name: Inline environment config
description: An inline environment config
setup:
  command:
    name: setup.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      PROCESS_ID="1"
      echo "Environment setup complete. PROCESS_ID=$PROCESS_ID"
teardown:
# --- Update the teardown command ---
  command:
    name: teardown.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      echo "Environment teardown complete. PROCESS_ID=$PROCESS_ID"
  env_vars:
    PROCESS_ID: "not yet from setup"
# -----------------------------------
```

Let's verify this works:

```bash
$ rtf run test-plan.yaml
Environment setup complete. PROCESS_ID=1
Running scenario from an external file
scenario executed with test plan variable
Environment teardown complete. PROCESS_ID=not yet from setup
```

The `provides` key defines variables that are specified in the config in the exact same way that
`variables` are. Let's update the `environment.yaml`:

```yaml
name: Inline environment config
description: An inline environment config
setup:
  command:
    name: setup.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      PROCESS_ID="1"
      echo "Environment setup complete. PROCESS_ID=$PROCESS_ID"
# --- Add a provides ---
  provides:
    - name: process_id
      description: The id of the process started in the environment setup
# ----------------------
teardown:
  command:
    name: teardown.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      echo "Environment teardown complete. PROCESS_ID=$PROCESS_ID"
  env_vars:
# --- Use the process_id here ---
    PROCESS_ID: "{{ process_id }}"
# -------------------------------
```

> **Note** If you want to check this templates, you'll need to specify a variable for `process_id`
> using `--var process_id="dummy_id"`. If you don't do this, you'll get
> `ERROR (environment.teardown.env_vars.PROCESS_ID) unknown templating variable: process_id`.

If we try to run this, it won't work:

```bash
$ rtf run test-plan.yaml
Environment setup complete. PROCESS_ID=1
ERROR missing required output fields from environment setup: ["process_id"]
```

This is because we need to update our setup script to output the `process_id`. To do this, rtf has a
reserved file that output needs to be `echo`'d to called `RTF_OUTPUT`. Let's look at this in an
updated `environment.yaml`:

```yaml
name: Inline environment config
description: An inline environment config
setup:
  command:
    name: setup.sh
    kind: inline
# --- Add the missing echo to output command ---
    content: |
      #!/usr/bin/env sh

      PROCESS_ID="1"
      echo "Environment setup complete. PROCESS_ID=$PROCESS_ID"
      echo "{ \"process_id\": \"$PROCESS_ID\" }" > "$RTF_OUTPUT"
# ----------------------------------------------
  provides:
    - name: process_id
      description: The id of the process started in the environment setup
teardown:
  command:
    name: teardown.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      echo "Environment teardown complete. PROCESS_ID=$PROCESS_ID"
  env_vars:
    PROCESS_ID: "{{ process_id }}"
```

Now, if we run again, we'll see the `process_id` being successfully used in the teardown:

```bash
$ rtf run test-plan.yaml
Environment setup complete. PROCESS_ID=1
Running scenario from an external file
scenario executed with test plan variable
Environment teardown complete. PROCESS_ID=1
```

## Using overrides

One of the main benefits of defining environment (and scenario) config in a separate file is that it
can be reused across multiple test plans. There will be occasions where you want to reuse the
majority of what's defined in an environment config but make small edits. Instead of making a new
file with the edits, you can use `overrides`.

Before we look at overrides, let's create a new `setup.sh` script. We'll use this instead of the
current inline script in `environment.yaml`:

```bash
touch scripts/setup.sh
```

Add the following to the `setup.sh` file (this will make it obvious we've run the intended setup):

```sh
#!/usr/bin/env sh

PROCESS_ID="2"
echo "Using the override setup script"
echo "Environment setup complete. PROCESS_ID=$PROCESS_ID"
echo "{ \"process_id\": \"$PROCESS_ID\" }" > "$RTF_OUTPUT"
```

Overrides can be added to either the `environment` or `scenario` in the test plan config. Add this
to `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: "scenario executed with test plan variable"
  process_id: dummy
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
environment:
  from:
    kind: local
    relative_path: configs/environment.yaml
# --- Add an override for the setup command ---
  overrides:
    setup:
      command:
        name: setup.sh
        kind: relative_path
        path: scripts/setup.sh
# ---------------------------------------------
```

Before checking if this works, let's look at how it works. We only want to change the
`setup.command`, so only that segment of the config is required. rtf will merge the YAML on matching
keys before checking if it templates. If you run the `template` command now (pay attention to the
`--var` flag here, we need this because of the `provides` variable):

```bash
$ template rtf-hello-world/test-plan.yaml --check --var process_id=dummy`
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: scenario executed with test plan variable
  process_id: dummy
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
    SCENARIO_ENV: scenario executed with test plan variable
  file_providers: []
environment:
  name: Inline environment config
  description: An inline environment config
  variable_definitions: []
  setup:
    command:
      name: setup.sh
      kind: relative_path
      path: scripts/setup.sh
      args: []
    env_vars: {}
    file_providers: []
    provides:
    - name: process_id
      description: The id of the process started in the environment setup
      default: null
  teardown:
    command:
      name: teardown.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "Environment teardown complete. PROCESS_ID=$PROCESS_ID"
      args: []
    env_vars:
      PROCESS_ID: dummy
    file_providers: []
```

If you look closely at the templated test plan, the setup command now matches what we defined in the
`overrides`, while the rest of the environment config is unchanged. If we run the test plan:

```bash
$ rtf run test-plan.yaml
Using the override setup script
Environment setup complete. PROCESS_ID=2
Running scenario from an external file
scenario executed with test plan variable
Environment teardown complete. PROCESS_ID=2
```

This confirms we're successfully using a new `setup.sh` script for the environment without changing
any other config for the environment.

---

In this guide, we've covered moving environment config into its own file, using provides in the
setup, and using overrides in the test plan. Next, we'll guide you through how to use file
providers.

**Next:** [Using file providers](using-file-providers.md)
