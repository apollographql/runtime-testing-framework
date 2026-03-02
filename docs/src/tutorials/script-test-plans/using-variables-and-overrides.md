<!-- diataxis-type: tutorial -->

# Using variables and overrides

In this guide, we'll add a variable to the Scenario and use an override to swap the scenario command
without modifying the base config. Both features work identically to the [docker tutorial][0] — this
guide shows them applied to a script-based [Test Plan][1].

> **Prerequisites**
>
> - Completed the ["Writing a command"][2] tutorial
> - An `rtf-hello-world` directory in the state it was at the end of that guide

Your directory should look like this:

```
rtf-hello-world/
├── configs/
│   ├── environment.yaml
│   └── scenario.yaml
├── scripts/
│   └── scenario.sh
└── test-plan.yaml
```

## Using variables

Variables allow a Test Plan to pass values into Scenario and Environment configs at runtime. See the
[Using variables][3] section of the docker tutorial for a full explanation.

Let's add a variable to `configs/scenario.yaml`:

```yaml
name: Script scenario
description: A script scenario
# --- Add a variable definition ---
variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
    default: "scenario executed with default value"
# ---------------------------------
command:
  name: scenario.sh
  kind: relative_path
  path: ../scripts/scenario.sh
# --- Use the variable ---
env_vars:
  SCENARIO_ENV: "{{ scenario_variable }}"
# ------------------------
```

Set a value for it in `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Add a variable value ---
variables:
  scenario_variable: "scenario executed with test plan variable"
# ----------------------------
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
environment:
  from:
    kind: local
    relative_path: configs/environment.yaml
```

Run the Test Plan:

```bash
rtf run test-plan.yaml
```

Output:

```
environment setup
Running scenario from an external file
scenario executed with test plan variable
environment teardown
```

The value from `test-plan.yaml` overrides the default in `scenario.yaml`. You can also pass it at
the CLI:

```bash
rtf run test-plan.yaml --var scenario_variable="scenario executed with CLI variable"
```

Output:

```
environment setup
Running scenario from an external file
scenario executed with CLI variable
environment teardown
```

## Using overrides

Overrides let you replace specific parts of a Scenario or Environment config in the Test Plan
without editing the base file. See the [Using overrides][4] section of the docker tutorial for a
full explanation.

Let's create a second scenario script to override with:

```bash
touch scripts/scenario-v2.sh
```

Add the following to `scripts/scenario-v2.sh`:

```sh
#!/usr/bin/env sh

echo "Running scenario from override script"
echo "$SCENARIO_ENV"
```

Add an override for the scenario command in `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: "scenario executed with test plan variable"
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
# --- Add an override for the command ---
  overrides:
    command:
      name: scenario.sh
      kind: relative_path
      path: scripts/scenario-v2.sh
# ---------------------------------------
environment:
  from:
    kind: local
    relative_path: configs/environment.yaml
```

Run the Test Plan:

```bash
rtf run test-plan.yaml
```

Output:

```
environment setup
Running scenario from override script
scenario executed with test plan variable
environment teardown
```

The override replaces only the `command` — the `variable_definitions` and `env_vars` from
`scenario.yaml` are unchanged, so `SCENARIO_ENV` still receives its value from `scenario_variable`.

## Next steps

You've now completed the script-based test plan tutorials. To go further:

- [Framework reference][5] — full reference for all config fields
- [Using file providers][6] — manage files that your Environment and Scenario depend on

[0]: ../test-plans/writing-a-scenario.md
[1]: ../../reference/glossary.md
[2]: writing-a-command.md
[3]: ../test-plans/writing-a-scenario.md#using-variables
[4]: ../test-plans/writing-a-scenario.md#using-overrides
[5]: ../../reference/framework/index.md
[6]: ../test-plans/using-file-providers.md
