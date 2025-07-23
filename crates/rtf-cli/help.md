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

* `--value <VALUE>` — A single additional templating value in the form "key=value"
* `--values <VALUES>` — Path to a JSON file containing additional template values
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



## `rtf template`

Template a test plan using provided values, outputting the resulting config to stdout

**Usage:** `rtf template [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be templated

###### **Options:**

* `--check` — Run a static check of the resulting test plan after templating



