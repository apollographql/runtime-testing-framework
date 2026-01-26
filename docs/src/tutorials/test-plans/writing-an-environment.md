<!-- diataxis-type: tutorial -->

# Writing a new environment

This guide assumes you've completed the ["Writing a scenario"][0] guide. You should already have the
files in a directory named `rtf-hello-world`. Your directory should be in the state it was at the
end of that guide.

```bash
ls -R
configs         scripts         test-plan.yaml

configs:
scenario.yaml

scripts:
scenario.sh
```

## Creating an environment file

Creating a separate environment file works exactly the same way and has the same benefits as
creating a separate scenario file outlined in the ["Writing a scenario" guide][1].

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
    name: teardown.sh
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
rtf template test-plan.yaml --check
```

You should see output similar to this:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: scenario executed with test plan variable
matrix: {}
scenario:
  name: Inline scenario config
  ...
environment:
  name: Inline environment config
  ...
```

## Environment config structure

Now is a good time to review how the environment config is structured. The main difference compared
to the [scenario config][2] is that an environment can run two commands using the `setup` and
`teardown` keys. The fields in an environment config are:

- `name` (required) is used to give the environment an identifiable title. It can be any valid
  string. It has no impact on the execution of an environment.
- `description` (required) is used to give more information about the environment. It can be any
  valid string. It has no impact on the execution of an environment.
- `variable_definitions` (optional) are used to define which variables an environment requires to
  successfully execute. This works the same as it does for a scenario and is explained more in the
  ["using variables"][3] section of that guide.
- `setup` (required) is used to define the command that executes at the start of the `rtf run`
  command. It is intended to be used to create and configure the environment for the scenario to
  test. It requires the following keys:
  - `command` (required) is used to define what is executed when the environment setup is run. The
    ["Writing a command" guide][4] explains commands in more detail.
  - `env_vars` (optional) is used to define the environment variables that are set when the
    `command` is executed.
  - `file_providers` (optional) is used to define the files and data that the environment setup
    depends on to execute. The ["Using file providers" guide][5] explains how these are used in more
    detail.
- `teardown` (required) is used to define the command that executes at the end of the `rtf run`
  command. It is intended to be used to collect results and shutdown the environment the scenario
  tested. It requires the following keys:
  - `command` (required) is used to define what is executed when the environment teardown is run.
    The ["Writing a command" guide][4] explains commands in more detail.
  - `env_vars` (optional) is used to define the environment variables that are set when the
    `command` is executed.
  - `file_providers` (optional) is used to define the files and data that the environment teardown
    depends on to execute. The ["Using file providers" guide][5] explains how these are used in more
    detail.

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

echo "Using the override setup script"
echo "Environment setup complete.
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
keys before checking if it templates. If you run the `template` command now:

```bash
rtf template test-plan.yaml --check
```

You should see that the setup command now matches what we defined in the `overrides`, while the rest
of the environment config is unchanged. If we run the test plan:

```bash
rtf run test-plan.yaml
```

Output:

```
Using the override setup script
Environment setup complete.
Running scenario from an external file
scenario executed with test plan variable
Environment teardown complete.
```

This confirms we're successfully using a new `setup.sh` script for the environment without changing
any other config for the environment.

---

In this guide, we've covered moving environment config into its own file and using overrides in the
test plan. Next, we'll guide you through how to use file providers.

**Next:** [Using file providers][5]

[0]: writing-a-scenario.md
[1]: writing-a-scenario.md#creating-a-scenario-file
[2]: writing-a-scenario.md#scenario-config-structure
[3]: writing-a-scenario.md#using-variables
[4]: writing-a-command.md
[5]: using-file-providers.md
