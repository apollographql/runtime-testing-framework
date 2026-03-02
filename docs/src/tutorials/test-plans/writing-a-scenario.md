<!-- diataxis-type: tutorial -->

# Writing a new scenario

In this guide, we'll take the inline scenario from the ["Writing a test plan"][0] tutorial and
improve it step by step. We'll connect it to the docker compose environment, move the docker command
into a script using file providers, extract the scenario into its own file, use variables to pass
values into the scenario, and use overrides to swap out scenario config without modifying the base
file.

> **Prerequisites**
>
> - Completed the ["Writing a test plan"][0] tutorial
> - An `rtf-hello-world` directory containing `test-plan.yaml` with the exact content shown below
>
> We're going to remove the variables and matrix added in the final step of the "Writing a test
> plan" guide before continuing.

Your `test-plan.yaml` file should contain:

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
      command: echo "hello world"
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

## Changing the docker command

> **Note** RTF runs the scenario container by parameterizing `docker run` from the `docker` config.
> For details on `docker run`, Docker images, and networking, refer to the
> [Docker documentation][1].

The scenario container in the test plan does not interact with the docker compose environment, it
just echos "hello world". In a real test scenario, we would want the scenario container to call a
service in the docker compose stack.

Let's update our `command` to call the `hello-world` service we start in the docker compose
environment.

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
# --- Update the docker scenario command ---
      command: wget -qO- http://hello-world:80
# ------------------------------------------
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

We are using `wget` to make the HTTP request because the `alpine` container already has this
installed.

> **Note** We are using `http://hello-world:80` as the endpoint. This works because RTF
> automatically adds a docker scenario container to the docker compose environment network using the
> `--net` flag. This avoids the need to map the docker compose services to localhost.

Now, if we run the test plan:

```bash
rtf run test-plan.yaml
```

Output:

```
[+] up 2/2
 ✔ Network inline-docker-compose-environment-config_default         Created      0.0s
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Healthy      0.7s
<!DOCTYPE html>
<html>
<head>
<title>Welcome to nginx!</title>
<style>
html { color-scheme: light dark; }
body { width: 35em; margin: 0 auto;
font-family: Tahoma, Verdana, Arial, sans-serif; }
</style>
</head>
<body>
<h1>Welcome to nginx!</h1>
<p>If you see this page, the nginx web server is successfully installed and
working. Further configuration is required.</p>

<p>For online documentation and support please refer to
<a href="http://nginx.org/">nginx.org</a>.<br/>
Commercial support is available at
<a href="http://nginx.com/">nginx.com</a>.</p>

<p><em>Thank you for using nginx.</em></p>
</body>
</html>
[+] down 2/2
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Removed      0.1s
 ✔ Network inline-docker-compose-environment-config_default         Removed      0.1s
```

Your scenario container is now talking to the docker compose service! The `wget` output confirms the
two containers are on the same network.

## Running a script as a docker command

Specifying commands like this works for a simple one line command, but is not useful for anything
more complex. It is possible to execute a script in the `command` for more complex use cases
instead. Let's move the `wget` command into a file instead of specifying it inline and wrap it in
some additional logic so we just get the HTTP status code in the terminal output.

```bash
mkdir scripts
touch scripts/check-status.sh
```

Then add the command to the `check-status.sh` file

```sh
#!/bin/sh
response=$(wget -S -O /dev/null http://hello-world:80 2>&1)
status=$(echo "$response" | grep "HTTP/" | grep -o "[0-9][0-9][0-9]")
echo "HTTP status: $status"
```

Next, update the `test-plan.yaml` file so the scenario executes the `check-status.sh` file:

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
# --- Update the docker scenario command ---
      command: sh "$CHECK_STATUS_SCRIPT"
    file_providers:
      - name: check-status.sh
        env_var: CHECK_STATUS_SCRIPT
        kind: relative_path
        path: scripts/check-status.sh
# ------------------------------------------
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

> **Note** File providers won't be executable directly so you need to make sure to run using
> `sh $CHECK_STATUS_SCRIPT`, not `./$CHECK_STATUS_SCRIPT`.

We have not explained how File Providers work yet. This is covered more in the
["Using file providers" guide][2].

Let's run the test plan:

```bash
rtf run test-plan.yaml
```

Output:

```
[+] up 2/2
 ✔ Network inline-docker-compose-environment-config_default         Created      0.0s
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Healthy      0.7s
HTTP status: 200
[+] down 2/2
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Removed      0.1s
 ✔ Network inline-docker-compose-environment-config_default         Removed      0.1s
```

We now get a much more readable output from the scenario container instead of the verbose `nginx`
response. You now have a scenario running a script and reporting a clean HTTP 200!

## Creating a scenario file

Writing the scenario inline like this works for simple test plans. However, more comprehensive
scenarios quickly become harder to read and maintain. It is possible to create a separate file to
store your scenario config and update your test plan to refer to that file. As well as
maintainability benefits this also means the same scenario can be reused in multiple test plans. It
is also possible to update the base scenario (and environment) config using overrides in the test
plan. We cover how to use overrides in the [Using overrides][3] section below.

Let's walkthrough how to create a scenario config. First, create a `configs` directory and an empty
YAML file inside it:

```bash
mkdir configs
touch configs/scenario.yaml
```

We want to move the scenario config so it is no longer defined inline in the test plan config. Copy
the scenario config from the test plan and add to the `scenario.yaml` file:

```yaml
name: Docker scenario config
description: A docker scenario config
docker:
  image: alpine
  tag: latest
  command: sh "$CHECK_STATUS_SCRIPT"
file_providers:
  - name: check-status.sh
    env_var: CHECK_STATUS_SCRIPT
    kind: relative_path
    path: scripts/check-status.sh
```

Now, we need to update the test plan so it uses the scenario config in the `scenario.yaml` file. To
do this, we need to delete the `inline` key (and the scenario config nested beneath it) and switch
to using the `from` key:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
# --- Replace the inline scenario ---
  from:
    kind: local
    relative_path: configs/scenario.yaml
# -----------------------------------
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

Now check the test plan templates:

```bash
rtf template test-plan.yaml
```

This will result in YAML being printed to the terminal, showing the scenario config has been inlined
from the external file. This is exactly what we had when the scenario was defined inline - the
template command inlines external configs ahead of execution.

When writing configs with a lot of files on relative paths, it is good practice to use the `--check`
flag when running the `template` command. This will check that files on relative paths can be found.
Let's see what happens when we run with that flag:

```bash
rtf template test-plan.yaml --check
```

Output:

```
ERROR Static analysis checks failed
(scenario.CHECK_STATUS_SCRIPT) The requested file did not exist
provided path was file:///path/to/configs/scripts/check-status.sh
```

When copying over our inline scenario config we forgot to account for the fact that our
`scenario.yaml` file is on a different path to our `test-plan.yaml` file. Relative paths are always
relative to the file they are defined in. Let's fix our mistake in the `scenario.yaml` file:

```yaml
name: Docker scenario config
description: A docker scenario config
docker:
  image: alpine
  tag: latest
  command: sh "$CHECK_STATUS_SCRIPT"
file_providers:
  - name: check-status.sh
    env_var: CHECK_STATUS_SCRIPT
    kind: relative_path
# --- Update the path to the scenario script ---
    path: ../scripts/check-status.sh
# ----------------------------------------------
```

If we run the `template` command again with the `--check` flag you should see the config YAML
printed to your terminal:

```bash
rtf template test-plan.yaml --check
```

The output is similar to this:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
...
```

The `from` key can have two variables, `local` or `github`. In this case, we are using `local`. This
will import the scenario config from the file on the relative path defined in the `relative_path`
key.

The `github` key allows the scenario config to be imported from a GitHub repo. It is not discussed
in detail in this guide, please refer to the [framework reference docs][4] for more detail.

You now have a reusable scenario in its own file! See the [framework reference docs][4] for the full
Scenario config structure.

## Using variables

We saw how to set variables in the test plan in the ["Writing a test plan" guide][5]. Let's look at
this in more detail. To use variables in a scenario, we need to define them in the `variables`
field.

Declaring the variables here declares a contract between the scenario and test plan and lists the
variables that must be specified for the scenario to complete. The value of the variable can be set
using a default in the scenario, in the test plan or provided via the CLI. If the variable is used
by the scenario and not set via any of those methods, it will cause the test plan execution to fail.
If variables are defined for usage in the scenario but not defined in the `variables` field then the
test plan will fail to template.

Let's see that in action. We are going to add an environment variable to the scenario and use a
variable to set its value. Update `scenario.yaml`:

```yaml
name: Docker scenario config
description: A docker scenario config
# --- Add a variables section ---
variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
# -------------------------------
docker:
  image: alpine
  tag: latest
  command: sh "$CHECK_STATUS_SCRIPT"
# --- Use the scenario_variable in the environment variables ---
env_vars:
  SCENARIO_ENV: "{{ scenario_variable }}"
# --------------------------------------------------------------
file_providers:
  - name: check-status.sh
    env_var: CHECK_STATUS_SCRIPT
    kind: relative_path
    path: ../scripts/check-status.sh
```

Let's explain how this works. The variables each have a `name` and `description`. The `name` is the
variable's identifier and is used in the template string. The `description` is there to give more
information about how and why the variable is used. Variables are templated into the config with the
`"{{ ... }}"` syntax, where `...` is replaced by the variable's `name`.

> **Note** The double curly braces and space either side of the variable name are important here. If
> the template string does not match this exactly, then rtf will error and call out there is a
> malformed template string.

To show this is added to the scenario container's environment, let's also add a line to the
`check-status.sh` script to echo the environment variable's value.

```sh
#!/bin/sh
# New line to echo the SCENARIO_ENV
echo $SCENARIO_ENV
response=$(wget -S -O /dev/null http://hello-world:80 2>&1)
status=$(echo "$response" | grep "HTTP/" | grep -o "[0-9][0-9][0-9]")
echo "HTTP status: $status"
```

Let's attempt to template the test plan:

```bash
rtf template test-plan.yaml
```

Output:

```
ERROR Templating failed
(scenario.env_vars.SCENARIO_ENV) Missing template variables definition. Make sure the variable is defined in the scenario or environment config variable definitions
  - scenario_variable: "An example variable that the scenario expects to be defined"
```

We have successfully defined a variable and where it should be used. However, we have not specified
what value it should actually have. If we attempted to run this test plan we would see the same
error. Let's define a default for this variable:

```yaml
name: Docker scenario config
description: A docker scenario config
variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
# --- Set a default for this variable ---
    default: "default scenario env var value"
# ---------------------------------------
docker:
  image: alpine
  tag: latest
  command: sh "$CHECK_STATUS_SCRIPT"
env_vars:
  SCENARIO_ENV: "{{ scenario_variable }}"
file_providers:
  - name: check-status.sh
    env_var: CHECK_STATUS_SCRIPT
    kind: relative_path
    path: ../scripts/check-status.sh
```

Let's run this and see what happens:

```bash
rtf run test-plan.yaml
```

Output:

```
[+] up 2/2
 ✔ Network inline-docker-compose-environment-config_default         Created      0.0s
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Healthy      0.7s
default scenario env var value
HTTP status: 200
[+] down 2/2
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Removed      0.2s
 ✔ Network inline-docker-compose-environment-config_default         Removed      0.1s
```

The scenario environment variable `echo` statement uses the default variable. We can override this
variable by setting a different value in the test plan (this will take precedence over a default).
Update `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
# --- Add a new value for scenario_variable ---
variables:
  scenario_variable: "scenario env var value from test plan"
# ---------------------------------------------
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
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

Now if we run:

```bash
rtf run test-plan.yaml
```

Output:

```
[+] up 2/2
 ✔ Network inline-docker-compose-environment-config_default         Created      0.0s
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Healthy      0.7s
scenario env var value from test plan
HTTP status: 200
[+] down 2/2
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Removed      0.2s
 ✔ Network inline-docker-compose-environment-config_default         Removed      0.1s
```

We can see that the variable from the test plan has overridden the default. Similarly, if the
variable is specified from the CLI it will override both the test plan variable and default:

```bash
rtf run test-plan.yaml --var scenario_variable="scenario env var value from cli variable"
```

Output:

```
[+] up 2/2
 ✔ Network inline-docker-compose-environment-config_default         Created      0.0s
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Healthy      0.7s
scenario env var value from cli variable
HTTP status: 200
[+] down 2/2
 ✔ Container inline-docker-compose-environment-config-hello-world-1 Removed      0.2s
 ✔ Network inline-docker-compose-environment-config_default         Removed      0.1s
```

You now know how to pass variables at all three levels: scenario default, test plan, and CLI. Each
level overrides the one before it.

## Using overrides

One of the main benefits of defining scenario (and environment) config in a separate file is that it
can be reused across multiple test plans. There will be occasions where you want to reuse the
majority of what's defined in a scenario config but make small edits. Instead of making a new file
with the edits, you can use `overrides`.

Before we add the override to our test plan, let's create an updated script for the scenario to run
that prints the response as well as the HTTP status code:

```bash
touch scripts/check-status-v2.sh
```

Add the following to `check-status-v2.sh`:

```sh
#!/bin/sh
echo $SCENARIO_ENV
body=$(wget -qO- http://hello-world:80)
echo "Response: $body"
status=$(wget -S -O /dev/null http://hello-world:80 2>&1 | grep "HTTP/" | grep -o "[0-9][0-9][0-9]")
echo "HTTP status: $status"
```

Overrides can be added to either the `environment` or `scenario` in the test plan config. Add this
to `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: "scenario env var value from test plan"
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
# --- Add a new overrides section ---
  overrides:
    file_providers:
      - name: check-status.sh
        env_var: CHECK_STATUS_SCRIPT
        kind: relative_path
        path: scripts/check-status-v2.sh
# ------------------------------------
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

Before checking if this works, let's look at how it works. We only want to change the
`CHECK_STATUS_SCRIPT`, so only that segment of the config is required. RTF will merge the YAML on
matching keys before checking if it templates. If you run the `template` command now:

```bash
rtf template test-plan.yaml --check
```

You should see that the scenario `CHECK_STATUS_SCRIPT` now matches what we defined in the
`overrides`, while the rest of the scenario config is unchanged. If we run the test plan:

```bash
rtf run test-plan.yaml
```

Output:

```
[+] up 2/2
 ✔ Network docker-compose-environment-config_default         Created               0.0s
 ✔ Container docker-compose-environment-config-hello-world-1 Healthy               0.7s
scenario env var value from test plan
Response: <!DOCTYPE html>
<html>
<head>
<title>Welcome to nginx!</title>
<style>
html { color-scheme: light dark; }
body { width: 35em; margin: 0 auto;
font-family: Tahoma, Verdana, Arial, sans-serif; }
</style>
</head>
<body>
<h1>Welcome to nginx!</h1>
<p>If you see this page, the nginx web server is successfully installed and
working. Further configuration is required.</p>

<p>For online documentation and support please refer to
<a href="http://nginx.org/">nginx.org</a>.<br/>
Commercial support is available at
<a href="http://nginx.com/">nginx.com</a>.</p>

<p><em>Thank you for using nginx.</em></p>
</body>
</html>
HTTP status: 200
[+] down 2/2
 ✔ Container docker-compose-environment-config-hello-world-1 Removed               0.1s
 ✔ Network docker-compose-environment-config_default         Removed               0.1s
```

We can now see the response being printed in the terminal output again. This confirms we're
successfully using the new `check-status-v2.sh` script for the scenario without changing any other
config for the scenario.

## Next steps

In this guide, we moved a scenario into its own file, used file providers to run a script, wired up
variables with defaults that can be overridden at runtime, and used overrides to swap out config
without touching the base file. Next, we'll walk through writing an environment config.

[Writing an environment][6]

[0]: writing-a-test-plan.md
[1]: https://docs.docker.com/reference/cli/docker/container/run/
[2]: using-file-providers.md
[3]: #using-overrides
[4]: ../../reference/framework/index.md
[5]: writing-a-test-plan.md#setting-variables
[6]: writing-an-environment.md
