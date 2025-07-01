# Command-Line Help for `rtf`

This document contains the help content for the `rtf` command-line program.

**Command Overview:**

* [`rtf`↴](#rtf)
* [`rtf run`↴](#rtf-run)
* [`rtf template`↴](#rtf-template)

## `rtf`

A swiss army knife for testing the Apollo Runtime

**Usage:** `rtf [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `run` — Check and run a test plan
* `template` — Template a test plan using provided values, outputting the resulting config to stdout

###### **Options:**

* `--value <VALUE>` — A single additional templationg value in the form "key=value"
* `--values <VALUES>` — Path to a JSON file containing additional template values



## `rtf run`

Check and run a test plan

**Usage:** `rtf run [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be executed

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for providers when they run

  Default value: `output`



## `rtf template`

Template a test plan using provided values, outputting the resulting config to stdout

**Usage:** `rtf template [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be templated

###### **Options:**

* `--check` — Run a static check of the resulting test plan after templating



