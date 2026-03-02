<!-- diataxis-type: tutorial -->

# Using file providers

In this guide, we'll use File Providers to manage files that our environment and scenario configs
depend on. We'll move the `nginx` config from being inlined in docker compose to being managed by
RTF, then learn about the `inline`, `relative_path`, and `required` provider types.

> **Prerequisites**
>
> - Completed the ["Writing an environment"][0] tutorial
> - An `rtf-hello-world` directory in the state it was at the end of that guide

Your directory should look like this:

```bash
ls -R
```

Output:

```
configs         data            scripts         test-plan.yaml

./configs:
environment.yaml        scenario.yaml

./data:
compose-app.yaml

./scripts:
check-status-v2.sh      check-status.sh
```

## What are file providers?

We've already been using File Providers in the scenario configuration throughout the previous
guides. File Providers are RTF's way of referring to files and data that are required for the
environment and scenario to run successfully. The most commonly used File Providers are `inline` and
`relative_path`. They provide the file content either from the local filesystem or from within the
test plan configuration itself.

There are additional File Providers that, amongst other things, can pull data from APIs (such as
GraphOS specific providers). These will not be discussed in this guide but can be seen in the
[framework reference][1].

All File Providers follow the same basic principle — they create one or more files and place them on
a path for RTF to make use of. If you need to know that path (for your scenario docker command, for
example), RTF will assign an environment variable that contains the file's path. This is a
deliberate design choice that allows RTF to make changes to how it stores files obtained from
providers without breaking assumptions made about paths in user-written scripts.

## Adding an inline file

Let's see how File Providers can be used in our test plan config. In the
["Writing an environment" guide][0] we created an `nginx` config file by writing it inline in the
`compose-app.yaml` file. This is an antipattern and forces us to write a completely new
`compose-app.yaml` file if we just want to amend the `nginx` config. Let's fix that by using a file
provider to supply the config instead.

Let's update our `environment.yaml` file to make use of the `file_providers` key:

```yaml
name: Docker compose environment config
description: A docker compose environment config
env_vars:
  HELLO_MESSAGE: "Goodbye, World!"
compose_files:
  - name: docker-compose.yaml
    kind: inline
    content: |
      services:
        hello-world:
          image: nginx:alpine
          ports:
            - "8080:80"
# ---- Replace the inline config with a volume ----
          volumes:
            - ${NGINX_CONFIG}:/etc/nginx/conf.d/default.conf
# -------------------------------------------------
  - name: compose-app.yaml
    kind: relative_path
    path: ../data/compose-app.yaml
# ---- Add an inline file provider ----
file_providers:
  - name: nginx.conf
    env_var: NGINX_CONFIG
    kind: inline
    content: |
      server {
        listen 80;
        location / {
          proxy_pass http://app:8000;
        }
      }
# -------------------------------------
```

> **Note** `compose_files` also use a subset of File Providers. These do not set an environment
> variable that RTF can refer to since they are run using the `docker compose` `-f` flag.

Now, run the test plan:

```bash
rtf run test-plan.yaml
```

Output:

```
[+] up 3/3
 ✔ Network docker-compose-environment-config_default         Created     0.0s
 ✔ Container docker-compose-environment-config-app-1         Healthy     0.6s
 ✔ Container docker-compose-environment-config-hello-world-1 Healthy     0.6s
scenario env var value from test plan
Response: Goodbye, World!
HTTP status: 200
[+] down 3/3
 ✔ Container docker-compose-environment-config-app-1         Removed     10.1s
 ✔ Container docker-compose-environment-config-hello-world-1 Removed     0.1s
 ✔ Network docker-compose-environment-config_default         Removed     0.1s
```

This is the exact same output as we got before making this change. All we have done is refactored
where the config file is defined.

This works because RTF sets the path to the `nginx.conf` file it creates as an environment variable.
The docker compose file (which is also written inline to the test plan) uses this path to volume
mount the config file to the container.

Let's examine the providers output:

```bash
ls output/providers/setup_providers
```

Output:

```
compose-app.yaml        docker-compose.yaml     nginx.conf
```

```bash
cat output/providers/setup_providers/nginx.conf
```

Output:

```nginx
server {
  listen 80;
  location / {
    proxy_pass http://app:8000;
  }
}
```

We've successfully updated our test plan to write the `nginx.conf` file to the output with the
content we specified inline!

## File provider config structure

Now that we've seen a File Provider being defined, let's discuss how the config is structured. There
are three required fields:

1. `name` is the name the file will be saved with in the `providers` directory of the output.
2. `env_var` is the environment variable the file's path will be stored in. This is used by
   subsequent commands to refer to the file.
3. `kind` is used to set which kind of File Provider is being used. See the [framework reference][1]
   for details on all the providers available.

Each File Provider will have other fields that need to be defined, like `content` for `inline`.
These are specific to each provider type, and the `template` command will highlight any missing or
incorrectly defined keys.

## Adding a file from a relative path

Our `nginx` config is no longer defined inline to our docker compose file, but it _is_ still inline
in our environment config. Let's further reduce our inline dependencies by changing from using an
`inline` File Provider to using a `relative_path` File Provider. This allows us to define the
`nginx.conf` file in the local filesystem and direct RTF to use that file.

First, let's create the `nginx.conf` file:

```bash
touch data/nginx.conf
```

Then, add the config to that file:

```nginx
server {
  listen 80;
  location / {
    proxy_pass http://app:8000;
  }
}
```

Let's update `environment.yaml` to refer to this file, this time using the `relative_path` file
provider:

```yaml
name: Docker compose environment config
description: A docker compose environment config
env_vars:
  HELLO_MESSAGE: "Goodbye, World!"
compose_files:
  - name: docker-compose.yaml
    kind: inline
    content: |
      services:
        hello-world:
          image: nginx:alpine
          ports:
            - "8080:80"
          volumes:
            - ${NGINX_CONFIG}:/etc/nginx/conf.d/default.conf
  - name: compose-app.yaml
    kind: relative_path
    path: ../data/compose-app.yaml
file_providers:
  - name: nginx.conf
    env_var: NGINX_CONFIG
# --- Replace inline provider with relative_path ---
    kind: relative_path
    path: ../data/nginx.conf
# --------------------------------------------------
```

> **Note** The path is relative to the `environment.yaml` file!

Now, let's run the test plan:

```bash
rtf run test-plan.yaml
```

The output should be exactly as it was when we defined this file inline. Likewise, if we examine the
output directory:

```bash
ls output/providers/setup_providers
```

Output:

```
compose-app.yaml        docker-compose.yaml     nginx.conf
```

```bash
cat output/providers/setup_providers/nginx.conf
```

Output:

```nginx
server {
  listen 80;
  location / {
    proxy_pass http://app:8000;
  }
}
```

As expected, we also see the `nginx.conf` file with the same content as before.

## Required files

As discussed in previous sections of this guide, the environment and scenario configs are designed
to be reused. For config designed to be reused often, you might want to force the user of the test
plan to define a file, but not give them a default file to work with. For this use case, the
`required` File Provider is the perfect solution. Any test plan that tries to execute with a
`required` File Provider will fail and tell the user to specify the file themselves (normally this
is done using `overrides`). Let's walk through how `required` can be used:

Let's update our environment to require the user supply the `nginx.conf` file. We don't want to give
a base example because users might end up using that without thinking about what configuration they
actually need for their specific test. In other words, we don't want it to "just work" by design.
Let's add a `required` file to our `environment.yaml`:

```yaml
name: Docker compose environment config
description: A docker compose environment config
env_vars:
  HELLO_MESSAGE: "Goodbye, World!"
compose_files:
  - name: docker-compose.yaml
    kind: inline
    content: |
      services:
        hello-world:
          image: nginx:alpine
          ports:
            - "8080:80"
          volumes:
            - ${NGINX_CONFIG}:/etc/nginx/conf.d/default.conf
  - name: compose-app.yaml
    kind: relative_path
    path: ../data/compose-app.yaml
file_providers:
  - name: nginx.conf
    env_var: NGINX_CONFIG
# --- Replace relative_path provider with required ---
    kind: required
    message: Please specify an nginx config file
# --------------------------------------------------
```

Now, let's see what happens when we try to template this test plan:

```bash
rtf template test-plan.yaml --check
```

Output:

```
ERROR Static analysis checks failed
(environment.file_providers.NGINX_CONFIG) A required file has not been defined
Please specify an nginx config file
```

We get an error saying we haven't defined a required file, along with the message we put in the
`message` field. We'd get the same error if we tried `rtf run`.

To make this work, the test plan user should make use of overrides. Let's update `test-plan.yaml`:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: "scenario env var value from test plan"
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
  overrides:
    file_providers:
      - name: check-status.sh
        env_var: CHECK_STATUS_SCRIPT
        kind: relative_path
        path: scripts/check-status-v2.sh
environment:
  from:
    kind: local
    relative_path: configs/environment.yaml
# --- Override the nginx config ---
  overrides:
    file_providers:
      - name: nginx.conf
        env_var: NGINX_CONFIG
        kind: relative_path
        path: data/nginx.conf
# --------------------------------------
```

The override will match based on the `name` key. If we template now:

```bash
rtf template test-plan.yaml --check
```

You should see the full templated output. If you look at `environment.file_providers`, you can see
that `nginx.conf` now uses the config from the overrides. Our test plan no longer contains a
`required` file, so we no longer get that error. The `rtf run` command can also complete
successfully now.

## Next steps

You've now completed the test plan tutorials! You've worked through writing a test plan, scenario,
and environment from scratch, and learned how to use File Providers to manage external files.

To go further, explore these topics:

- [Custom providers][2] — learn how to write your own providers to pull data from external sources
- [Script-based test plans][3] — an alternative to docker-based scenarios and environments, for
  cases where docker isn't available
- [Framework reference][1] — the full reference for all config fields and provider types

[0]: writing-an-environment.md
[1]: ../../reference/framework/file-providers.md
[2]: ../custom-providers/index.md
[3]: ../script-test-plans/index.md
