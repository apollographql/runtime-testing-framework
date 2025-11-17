# Command-Line Help for `rtf`

This document contains the help content for the `rtf` command-line program.

**Command Overview:**

* [`rtf`↴](#rtf)
* [`rtf run`↴](#rtf-run)
* [`rtf expand-matrix`↴](#rtf-expand-matrix)
* [`rtf template`↴](#rtf-template)

## `rtf`

A swiss army knife for testing the Apollo Runtime

**Usage:** `rtf [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `run` — Check and run a test plan
* `expand-matrix` — Expand a test plan matrix into JSON
* `template` — Template a test plan using provided variables, outputting the resulting config to stdout

###### **Options:**

* `--var <VAR>` — A single additional templating variable in the form "key=value"
* `--vars <VARS>` — Path to a JSON file containing additional template variables
* `-v`, `--verbose` — Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`



## `rtf run`

Check and run a test plan

**Usage:** `rtf run [OPTIONS] [TEST_PLAN_PATH]`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test plan file that should be executed

###### **Options:**

* `--github <ORG/REPO/PATH>` — Execute a test plan file in GitHub instead of from a local path
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github
* `--outdir <OUTDIR>` — Output directory for providers when they run

  Default value: `output`



## `rtf expand-matrix`

Expand a test plan matrix into JSON

**Usage:** `rtf expand-matrix [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should have its matrix expanded

###### **Options:**

* `-c`, `--compact` — Return the expanded matrix JSON in compact form



## `rtf template`

Template a test plan using provided variables, outputting the resulting config to stdout

**Usage:** `rtf template [OPTIONS] [TEST_PLAN_PATH]`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be templated

###### **Options:**

* `--check` — Run a static check of the resulting test plan after templating
* `--github <ORG/REPO/PATH>` — Template a test plan file in GitHub instead of from a local path
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github



