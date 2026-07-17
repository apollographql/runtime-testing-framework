# Command-Line Help for `rtf`

This document contains the help content for the `rtf` command-line program.

**Command Overview:**

* [`rtf`↴](#rtf)
* [`rtf run`↴](#rtf-run)
* [`rtf docs`↴](#rtf-docs)
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
* [`rtf json-schemas`↴](#rtf-json-schemas)
* [`rtf remote`↴](#rtf-remote)
* [`rtf remote prepare`↴](#rtf-remote-prepare)
* [`rtf remote request`↴](#rtf-remote-request)
* [`rtf remote run`↴](#rtf-remote-run)
* [`rtf remote ci-run`↴](#rtf-remote-ci-run)
* [`rtf remote execution-output`↴](#rtf-remote-execution-output)
* [`rtf remote run-output`↴](#rtf-remote-run-output)
* [`rtf version`↴](#rtf-version)

## `rtf`

A swiss army knife for testing the Apollo Runtime

**Usage:** `rtf [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `run` — Check and run a test plan
* `docs` — Open the RTF documentation in your browser
* `expand-matrix` — Expand a test plan matrix into JSON
* `template` — Template a test plan using provided variables, outputting the resulting config to stdout
* `custom-provider` — Work directly with custom file provider definitions
* `inline` — Inline file providers in a test plan. Outputs the resulting test plan to the given directory
* `resolve` — Resolve file providers for a config file without executing it
* `completion` — Write a shell completion file to STDOUT for the given shell
* `json-schemas` — Output json schemas for environment configuration
* `remote` — Interactions with remote RTF service
* `version` — Display CLI version and exit

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

* `--environment-up` — Only run the environment setup
* `--environment-down` — Only run the environment teardown
* `--scenario` — Only run the environment scenario
* `--github` — Execute a test plan file in GitHub instead of from a local path

  Default value: `false`
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github
* `--outdir <OUTDIR>` — Output directory for providers when they run

  Default value: `output`
* `--force` — Force removal of an existing output directory before running

  Default value: `false`



## `rtf docs`

Open the RTF documentation in your browser

**Usage:** `rtf docs [SEARCH_TERM]...`

###### **Arguments:**

* `<SEARCH_TERM>` — An optional search term to search for within the docs



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
* `--force` — Force removal of an existing output directory before running

  Default value: `false`



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
* `--force` — Force removal of an existing output directory before running

  Default value: `false`
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
* `--force` — Force removal of an existing output directory before running

  Default value: `false`
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
* `--force` — Force removal of an existing output directory before running

  Default value: `false`



## `rtf resolve environment`

Resolve file providers for a standalone environment config

**Usage:** `rtf resolve environment [OPTIONS] <ENVIRONMENT_PATH>`

###### **Arguments:**

* `<ENVIRONMENT_PATH>` — Relative path to the environment.yaml file

###### **Options:**

* `--outdir <OUTDIR>` — Output directory for resolved providers and env files

  Default value: `output`
* `--force` — Force removal of an existing output directory before running

  Default value: `false`



## `rtf completion`

Write a shell completion file to STDOUT for the given shell

**Usage:** `rtf completion [OPTIONS]`

###### **Options:**

* `-s`, `--shell <SHELL>` — The shell to generate completions for (defaults to identifying from the environment)

  Possible values: `bash`, `elvish`, `fish`, `powershell`, `zsh`




## `rtf json-schemas`

Output json schemas for environment configuration

**Usage:** `rtf json-schemas <CONFIG>`

###### **Arguments:**

* `<CONFIG>`

  Possible values: `test-plan`, `environment`, `scenario`




## `rtf remote`

Interactions with remote RTF service

**Usage:** `rtf remote <COMMAND>`

###### **Subcommands:**

* `prepare` — Prepare a test plan for remote execution by the RTF service. Outputs an RTF service compatible JSON payload with inlined relative files and custom providers
* `request` — Send an IAP-authenticated HTTP request to the RTF service
* `run` — Trigger a test run using the RTF service
* `ci-run` — Trigger a test run using the RTF service and poll for the result
* `execution-output` — Pull output for a single test execution
* `run-output` — Pull output for all executions within a given test run



## `rtf remote prepare`

Prepare a test plan for remote execution by the RTF service. Outputs an RTF service compatible JSON payload with inlined relative files and custom providers

**Usage:** `rtf remote prepare [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test plan file. When using --github this must be in the format ORG/REPO/PATH

###### **Options:**

* `--github` — Prepare a test plan file from GitHub instead of from a local path

  Default value: `false`
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github



## `rtf remote request`

Send an IAP-authenticated HTTP request to the RTF service.

The response body is written to stdout on success.

**Usage:** `rtf remote request [OPTIONS] <PATH>`

###### **Arguments:**

* `<PATH>` — Path on the orchestrator to request (e.g. `/health`)

###### **Options:**

* `-X`, `--method <METHOD>` — HTTP method

  Default value: `GET`
* `-b`, `--body <BODY>` — Request body as a literal string
* `--orchestrator-url <ORCHESTRATOR_URL>` — Override the RTF service base URL



## `rtf remote run`

Trigger a test run using the RTF service.

The output of this command will be the test run id and a link to the RTF UI to view the status

**Usage:** `rtf remote run [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test plan file. When using --github this must be in the format ORG/REPO/PATH

###### **Options:**

* `--github` — Prepare a test plan file from GitHub instead of from a local path

  Default value: `false`
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github



## `rtf remote ci-run`

Trigger a test run using the RTF service and poll for the result.

The output of this command is aimed at being usable in CI runs and is non-interactive.

**Usage:** `rtf remote ci-run [OPTIONS] <TEST_PLAN_PATH>`

###### **Arguments:**

* `<TEST_PLAN_PATH>` — Relative path to the test plan file. When using --github this must be in the format ORG/REPO/PATH

###### **Options:**

* `--github` — Prepare a test plan file from GitHub instead of from a local path

  Default value: `false`
* `--ref <GIT_REF>` — Optional git ref to pull files from when using --github
* `--poll-interval-seconds <POLL_INTERVAL_SECONDS>`

  Default value: `10`



## `rtf remote execution-output`

Pull output for a single test execution

**Usage:** `rtf remote execution-output [OPTIONS] <ID>`

###### **Arguments:**

* `<ID>` — ID of the orchestrator test execution you wish to pull output for

###### **Options:**

* `--outdir <OUTDIR>` — Directory to place output in

  Default value: `output`
* `--force` — Force removal of an existing output directory before running

  Default value: `false`



## `rtf remote run-output`

Pull output for all executions within a given test run

**Usage:** `rtf remote run-output [OPTIONS] <ID>`

###### **Arguments:**

* `<ID>` — ID of the orchestrator test run you wish to pull output for

###### **Options:**

* `--outdir <OUTDIR>` — Directory to place output in

  Default value: `output`
* `--force` — Force removal of an existing output directory before running

  Default value: `false`



## `rtf version`

Display CLI version and exit

**Usage:** `rtf version`



