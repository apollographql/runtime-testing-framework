# Command-Line Help for `rtf`

This document contains the help content for the `rtf` command-line program.

**Command Overview:**

* [`rtf`↴](#rtf)
* [`rtf run`↴](#rtf-run)
* [`rtf expand-matrix`↴](#rtf-expand-matrix)
* [`rtf template`↴](#rtf-template)
* [`rtf custom-provider`↴](#rtf-custom-provider)
* [`rtf custom-provider template`↴](#rtf-custom-provider-template)
* [`rtf custom-provider run`↴](#rtf-custom-provider-run)
* [`rtf inline`↴](#rtf-inline)
* [`rtf inline all`↴](#rtf-inline-all)
* [`rtf inline relative-files`↴](#rtf-inline-relative-files)
* [`rtf resolve`↴](#rtf-resolve)
* [`rtf resolve scenario`↴](#rtf-resolve-scenario)
* [`rtf resolve environment`↴](#rtf-resolve-environment)
* [`rtf completion`↴](#rtf-completion)

## `rtf`

A swiss army knife for testing the Apollo Runtime

**Usage:** `rtf [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `run` — Check and run a test plan
* `expand-matrix` — Expand a test plan matrix into JSON
* `template` — Template a test plan using provided variables, outputting the resulting config to stdout
* `custom-provider` — Work directly with custom file provider definitions
* `inline` — Inline file providers in a test plan. Outputs the resulting test plan to the given directory
* `resolve` — Resolve file providers for a config file without executing it
* `completion` — 

###### **Options:**

* `--var <VAR>` — A single additional templating variable in the form "key=value"
* `--vars <VARS>` — Path to a JSON file containing additional template variables
* `-v`, `--verbose` — Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`



## `rtf run`

Check and run a test plan

**Usage:** `rtf run [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test plan file that should be executed. When using --github this must be in the format ORG/REPO/PATH

###### **Options:**

* `--github` — Execute a test plan file in GitHub instead of from a local path

  Default value: `false`
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

**Usage:** `rtf template [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test plan file that should be templated. When using --github this must be in the format ORG/REPO/PATH

###### **Options:**

* `--check` — Run a static check of the resulting test plan after templating
* `--github` — Template a test plan file from GitHub instead of from a local path

  Default value: `false`
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github



## `rtf custom-provider`

Work directly with custom file provider definitions

**Usage:** `rtf custom-provider <COMMAND>`

###### **Subcommands:**

* `template` — Template a custom provider definition, outputting the resulting config to stdout
* `run` — Execute a custom provider definition



## `rtf custom-provider template`

Template a custom provider definition, outputting the resulting config to stdout

**Usage:** `rtf custom-provider template [OPTIONS] <DEFINITION_PATH>`

###### **Arguments:**

* `<DEFINITION_PATH>` — Relative path to the custom provider definition file

###### **Options:**

* `--check` — Run a static check of the resulting test plan after templating



## `rtf custom-provider run`

Execute a custom provider definition

**Usage:** `rtf custom-provider run [OPTIONS] <DEFINITION_PATH>`

###### **Arguments:**

* `<DEFINITION_PATH>` — Relative path to the custom provider definition file

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for provider execution

  Default value: `output`



## `rtf inline`

Inline file providers in a test plan. Outputs the resulting test plan to the given directory

**Usage:** `rtf inline <COMMAND>`

###### **Subcommands:**

* `all` — Inline all file providers
* `relative-files` — Inline only relative file providers



## `rtf inline all`

Inline all file providers

**Usage:** `rtf inline all [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be inlined. When using --github this must be in the format ORG/REPO/PATH

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for inlined test plan

  Default value: `output`
* `--github` — Inline a test plan file from GitHub instead of from a local path

  Default value: `false`
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github



## `rtf inline relative-files`

Inline only relative file providers

**Usage:** `rtf inline relative-files [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test-plan.yaml file that should be inlined. When using --github this must be in the format ORG/REPO/PATH

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for inlined test plan

  Default value: `output`
* `--github` — Inline a test plan file from GitHub instead of from a local path

  Default value: `false`
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github



## `rtf resolve`

Resolve file providers for a config file without executing it

**Usage:** `rtf resolve <COMMAND>`

###### **Subcommands:**

* `scenario` — Resolve file providers for a standalone scenario config
* `environment` — Resolve file providers for a standalone environment config



## `rtf resolve scenario`

Resolve file providers for a standalone scenario config

**Usage:** `rtf resolve scenario [OPTIONS] <SCENARIO_PATH>`

###### **Arguments:**

* `<SCENARIO_PATH>` — Relative path to the scenario.yaml file

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for resolved providers and scenario.env

  Default value: `output`



## `rtf resolve environment`

Resolve file providers for a standalone environment config

**Usage:** `rtf resolve environment [OPTIONS] <ENVIRONMENT_PATH>`

###### **Arguments:**

* `<ENVIRONMENT_PATH>` — Relative path to the environment.yaml file

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for resolved providers and env files

  Default value: `output`



## `rtf completion`

**Usage:** `rtf completion [OPTIONS]`

###### **Options:**

* `-s`, `--shell <SHELL>`

  Possible values: `bash`, `elvish`, `fish`, `powershell`, `zsh`




