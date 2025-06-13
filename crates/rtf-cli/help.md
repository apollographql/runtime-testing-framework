# Command-Line Help for `rtf`

This document contains the help content for the `rtf` command-line program.

**Command Overview:**

* [`rtf`↴](#rtf)
* [`rtf run`↴](#rtf-run)
* [`rtf resolve`↴](#rtf-resolve)

## `rtf`

A swiss army knife for testing the Apollo Runtime

**Usage:** `rtf <COMMAND>`

###### **Subcommands:**

* `run` — Validate and run a test plan
* `resolve` — Resolve a test plan using provided values, outputting the resulting config to stdout



## `rtf run`

Validate and run a test plan

**Usage:** `rtf run [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be executed

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for providers when they run

  Default value: `output`



## `rtf resolve`

Resolve a test plan using provided values, outputting the resulting config to stdout

**Usage:** `rtf resolve [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be resolve

###### **Options:**

* `--values <VALUES>` — Additional values to use while templating, specified as a JSON object
* `--validate` — Run static validation of the resulting test plan after templating



