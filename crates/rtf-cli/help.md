# Command-Line Help for `rtf`

This document contains the help content for the `rtf` command-line program.

**Command Overview:**

* [`rtf`↴](#rtf)
* [`rtf run`↴](#rtf-run)

## `rtf`

A swiss army knife for testing the Apollo Runtime

**Usage:** `rtf <COMMAND>`

###### **Subcommands:**

* `run` — Validate and run a test plan



## `rtf run`

Validate and run a test plan

**Usage:** `rtf run [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be executed

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for providers when they run

  Default value: `output`



