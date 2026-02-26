<!-- diataxis-type: tutorial -->

# Writing a test plan

In this guide, we'll write a script-based [Test Plan][0] from scratch. The Test Plan structure is
identical to the [docker tutorial][1] — the differences are in the Scenario and Environment configs,
which will use `command`s instead of `docker`.

Create a working directory:

```bash
mkdir rtf-hello-world
cd rtf-hello-world
mkdir configs
```

Create the three config files:

```bash
touch test-plan.yaml configs/scenario.yaml configs/environment.yaml
```

Add the following to `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
environment:
  from:
    kind: local
    relative_path: configs/environment.yaml
```

Add the following to `configs/scenario.yaml`:

```yaml
name: Script scenario
description: A script scenario
command:
  name: scenario.sh
  kind: inline
  content: |
    #!/usr/bin/env sh

    echo "scenario executed"
```

Add the following to `configs/environment.yaml`:

```yaml
name: Script environment
description: A script environment
setup:
  command:
    name: setup.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      echo "environment setup"
teardown:
  command:
    name: teardown.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      echo "environment teardown"
```

## Scenario config

A script [Scenario][0] uses `command:` instead of `docker:`. The `command:` defines a shell script
that runs directly on the host. We cover `command:` in detail in the [Writing a command][2] guide.

## Environment config

A script [Environment][0] uses `setup:` and `teardown:` instead of `compose_files:`. Each phase
defines a `command:` that runs a shell script on the host, bracketing the Scenario's execution. See
the [framework reference][3] for the full Environment config structure.

## Checking the Test Plan

The `rtf template` command works the same way as in the docker tutorial. Run it to confirm the Test
Plan templates correctly:

```bash
rtf template test-plan.yaml --check
```

## Running the Test Plan

```bash
rtf run test-plan.yaml
```

Output:

```
environment setup
scenario executed
environment teardown
```

RTF runs the Environment setup command, then the Scenario, then the Environment teardown — in that
order. Remove the output directory before the next run:

```bash
rm -rf output/
```

## Variables and matrix

The `variables` and `matrix` fields work identically in script-based Test Plans. See the
[Setting variables][4] and [Using a matrix][5] sections of the docker tutorial for full
walkthroughs.

## Next steps

In this guide, we wrote a script-based Test Plan and ran it successfully. Next, we'll look at the
`command` configuration in more detail.

[Writing a command][2]

[0]: ../../reference/glossary.md
[1]: ../test-plans/writing-a-test-plan.md
[2]: writing-a-command.md
[3]: ../../reference/framework/environments.md
[4]: ../test-plans/writing-a-test-plan.md#setting-variables
[5]: ../test-plans/writing-a-test-plan.md#using-a-matrix
