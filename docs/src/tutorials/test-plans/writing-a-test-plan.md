<!-- diataxis-type: tutorial -->

# Writing a test plan

In this guide, we'll write a docker-based [Test Plan][0] from scratch. We'll cover the four required
fields, run the Test Plan, then add variables and a matrix to parameterize runs across multiple
configurations.

> **Prerequisites**
>
> - RTF CLI installed and available in your terminal
> - Docker and Docker Compose available in your terminal

Create an empty directory and make it your working directory:

```bash
mkdir rtf-hello-world
cd rtf-hello-world
```

Create an empty YAML file for the Test Plan:

```bash
touch test-plan.yaml
```

Add the following content to `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    docker:
      image: alpine
      tag: latest
      command: echo "hello world!"
environment:
  inline:
    name: Inline docker compose environment config
    description: An inline docker compose environment config
    compose_files:
      - name: docker-compose.yaml
        kind: inline
        content: |
          services:
            hello-world:
              image: nginx:alpine
              ports:
                - "8080:80"
```

This will all be explained as we progress through the guide, for now all you need to know is this is
the most basic docker based [Test Plan][0] it is possible to write in RTF.

## Adding required fields

An RTF Test Plan has four required fields: `name`, `description`, `scenario`, and `environment`. The
sections below add each of these required fields and explain them in more detail.

### `name`

Add the `name` field to the `test-plan.yaml` file

```yaml
name: Hello World
```

`name` is used to give each Test Plan an identifiable title. It can be any valid string. The value
used for the `name` field has no impact on the execution of the Test Plan. This makes it easier to
work with the Test Plan programmatically.

### `description`

Add the `description` field to the `test-plan.yaml` file

```yaml
name: Hello World
description: Created as a guide for writing test plans
```

`description` is used to give more information about the Test Plan for future users. It can be any
valid string. The value used for the `description` field has no impact on the execution of the Test
Plan itself. This is a useful place to add links or reference materials and should be preferred over
adding that context to inline comments.

### `scenario`

The `scenario` is used to define the configuration and command that runs the actual testing logic in
the Test Plan. The `scenario` can be defined inline within the Test Plan or in its own file. In this
guide, we'll define the [Scenario][0] inline. The guide on [writing a new Scenario][1] covers how to
define a Scenario in a separate file.

We'll use a `docker` based Scenario — the recommended approach in RTF. If `docker` isn't an option,
see [script based Scenarios][2]. We'll cover more advanced Scenario configuration in the guide on
[writing a new Scenario][1].

> **Note** RTF runs the scenario by passing the `image`, `tag`, and `command` config fields as
> arguments to `docker run`. For details on `docker run` and Docker images, refer to the
> [Docker documentation][3].

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline docker scenario config
    description: An inline docker scenario config
    docker:
      image: alpine
      tag: latest
      command: echo "hello world!"
```

- The `inline` field is used to indicate the Scenario will be defined in the Test Plan file.
- The `name` and `description` fields are required and used to identify the Scenario and work the
  same as `name` and `description` in the Test Plan.
- The `docker` field is used to define the container the Scenario will run
  - `image` is the name of the container image. Note that you will need to ensure that wherever you
    are running `docker` from is authenticated to pull the image.
  - `tag` defines the image tag that should be pulled.
  - `command` optionally sets the command the container runs.

### `environment`

The `environment` is used to define the configuration and commands that set up the [Environment][0]
for testing and tear it down after the test has completed. The `environment` can be defined inline
within the Test Plan or in its own file. In this guide, we'll define the Environment inline. The
guide on [writing a new Environment][4] covers how to define an Environment in a separate file.

We'll use a `docker compose` based Environment — the recommended approach in RTF. If
`docker compose` isn't an option, see [script based Environments][2]. We'll cover more advanced
Environment configuration in the guide on [writing a new Environment][4].

> **Note** RTF manages the environment by running `docker compose up` and `docker compose down`,
> passing the resolved compose files as `-f` arguments. For details on Docker Compose files and
> options, refer to the [Docker Compose documentation][5].

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline docker scenario config
    description: An inline docker scenario config
    docker:
      image: alpine
      tag: latest
      command: echo "hello world!"
environment:
  inline:
    name: Inline docker compose environment config
    description: An inline docker compose environment config
    compose_files:
      - name: docker-compose.yaml
        kind: inline
        content: |
          services:
            hello-world:
              image: nginx:alpine
              ports:
                - "8080:80"
```

- The `inline` field is used to indicate the Environment will be defined in the Test Plan file.
- The `name` and `description` fields are required and used to identify the Environment and work the
  same as `name` and `description` in the Test Plan.
- The `compose_files` array defines a list of `docker compose` files the Environment will run.

The service we are running in the `docker-compose.yaml` test service is a simple web server. We will
show how to connect the docker container that runs in the scenario to the service running in the
environment in the ["Writing a scenario" guide][1].

## Checking the Test Plan

Now, let's check that the Test Plan has been defined correctly:

```bash
rtf template test-plan.yaml
```

The output is similar to this:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables: {}
matrix:
  variant_names: null
  dimensions: {}
  include: []
custom_providers: []
scenario:
  name: Inline scenario config
  description: An inline scenario config
  variable_definitions: []
  custom_providers: []
  docker:
    image: alpine
    tag: latest
    command: echo "hello world!"
  env_vars: {}
  file_providers: []
environment:
  name: Inline docker compose environment config
  description: An inline docker compose environment config
  variable_definitions: []
  custom_providers: []
  project_name: null
  compose_files:
  - name: docker-compose.yaml
    kind: inline
    content: |
      services:
        hello-world:
          image: nginx:alpine
          ports:
            - "8080:80"
  file_providers: []
  env_vars: {}
```

The Test Plan templates successfully! This also highlights two optional fields — `variables` and
`matrix` — that have not yet been used. These are discussed more below.

## Running the Test Plan

Before looking at `variables` and `matrix`, let's run the Test Plan:

```bash
rtf run test-plan.yaml
```

The output is similar to this:

```
[+] up 2/2
 ✔ Network inline-docker-compose-environment-config_default         Created      0.0s
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Healthy      0.7s
hello world!
[+] down 2/2
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Removed      0.1s
 ✔ Network inline-docker-compose-environment-config_default         Removed      0.1s
```

Your first Test Plan ran successfully! The `hello world!` output confirms the scenario container ran
the command we configured.

This also creates an `output` directory.

The `output` directory contains a `providers` directory and two files: `resolved-test-plan.yaml` and
`test-plan-variables.json`.

- `resolved-test-plan.yaml` contains the fully resolved Test Plan config. This should be the same as
  what was shown in the `rtf template` command. This is a way to verify the Test Plan that ran to
  give you the output.
- `test-plan-variables.json` contains the variables used during the execution of the Test Plan. This
  is empty since no variables were set.

Remove the output directory before continuing (forgetting to do this will result in an error next
time `rtf run` is used):

```bash
rm -rf output/
```

> **Note** RTF is deliberately configured to not overwrite an existing output directory. This is so
> you cannot accidentally overwrite output you intend to keep. The `--output` flag can be used with
> `rtf run` to set a different output directory if you want to keep the existing output and run a
> new test.

## Setting variables

The `variables` field is used to set global variables that can be referenced in your scenario and/or
environment. Any variables set in the Test Plan config can be overridden using the `--var` and
`--vars` flags in the RTF CLI (see the [modifying variables section of the hello world guide][6] for
more information).

Let's add some example variables to `test-plan.yaml`. We are also going to update the Scenario
command to use this variable. The ["Writing a Scenario" section][1] will explain how this works, for
now just add the configuration:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Add variables ---
variables:
  example_variable: example variable
# ------------------
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
# --- Update scenario ---
    variable_definitions:
      - name: example_variable
        description: An example variable
    env_vars:
      EXAMPLE_VARIABLE: "{{ example_variable }}"
    docker:
      image: alpine
      tag: latest
      command: echo "$EXAMPLE_VARIABLE"
# -----------------------
environment:
  inline:
    name: Inline docker compose environment config
    description: An inline docker compose environment config
    compose_files:
      - name: docker-compose.yaml
        kind: inline
        content: |
          services:
            hello-world:
              image: nginx:alpine
              ports:
                - "8080:80"
```

You can see the value being used by running the Test Plan again:

```bash
rtf run test-plan.yaml
```

Output:

```
[+] up 2/2
 ✔ Network inline-docker-compose-environment-config_default         Created      0.0s
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Healthy      0.7s
example variable
[+] down 2/2
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Removed      0.1s
 ✔ Network inline-docker-compose-environment-config_default         Removed      0.1s
```

Now, the `test-plan-variables.json` file contains the variable that we set in the Test Plan:

```bash
cat output/test-plan-variables.json
```

Output:

```json
{
  "example_variable": "example variable"
}
```

## Using a matrix

The `matrix` field is used to create a matrix of variable dimensions to iterate over (see the
[matrix variables section of the hello world guide][7] for more information). A matrix can only be
defined in the Test Plan config.

Let's add a matrix to and remove the `variables` from our `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Replace variables with a matrix ---
matrix:
  variant_names: "${example_variable}"
  dimensions:
    example_variable:
      - variable1
      - variable2
# ------------------------------------
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    variable_definitions:
      - name: example_variable
        description: An example variable
    env_vars:
      EXAMPLE_VARIABLE: "{{ example_variable }}"
    docker:
      image: alpine
      tag: latest
      command: echo "$EXAMPLE_VARIABLE"
environment:
  inline:
    name: Inline docker compose environment config
    description: An inline docker compose environment config
    compose_files:
      - name: docker-compose.yaml
        kind: inline
        content: |
          services:
            hello-world:
              image: nginx:alpine
              ports:
                - "8080:80"
```

Run the Test Plan again (make sure the `output` directory has been deleted after previous test
runs):

```bash
rtf run test-plan.yaml
```

The terminal output will look different this time - the environment and scenario commands are
executed twice. The `output` directory will also have a different structure:

```bash
ls output/
```

Output:

```
variable1        variable2
```

Let's look at each of those matrix directories to see the different variable values used per
execution:

```bash
cat output/variable1/test-plan-variables.json
```

Output:

```json
{
  "example_variable": "variable1"
}
```

```bash
cat output/variable2/test-plan-variables.json
```

Output:

```json
{
  "example_variable": "variable2"
}
```

The `example_variable`'s variable changes per execution. You've run your first matrix — the scenario
executed twice, once for each value in the `example_variable` dimension!

## Next steps

In this guide, we covered writing a docker-based Test Plan with inline scenario and environment
configs, and used variables and a matrix to parameterize runs. Next, we'll walk through how to write
scenarios in more detail.

[Writing a scenario][1]

[0]: ../../reference/glossary.md
[1]: writing-a-scenario.md
[2]: ../script-test-plans/index.md
[3]: https://docs.docker.com/reference/cli/docker/container/run/
[4]: writing-an-environment.md
[5]: https://docs.docker.com/compose/
[6]: ../hello-world.md#modifying-variables
[7]: ../hello-world.md#matrix-variables
