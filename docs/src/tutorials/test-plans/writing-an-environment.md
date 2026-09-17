<!-- diataxis-type: tutorial -->

# Writing a new environment

In this guide, we'll take the inline environment from the ["Writing a scenario"][0] tutorial and
move it into its own file. We'll then extend it by adding a second compose service, and use RTF
environment variables to configure the docker compose stack.

> **Note** RTF manages the environment by parameterizing `docker compose up` and
> `docker compose down` from the `compose_files` config. For details on Docker Compose files and
> multi-file composition, refer to the [Docker Compose documentation][1].

> **Prerequisites**
>
> - Completed the ["Writing a scenario"][0] tutorial
> - An `rtf-hello-world` directory in the state it was at the end of that guide

Your directory should look like this:

```bash
configs         scripts         test-plan.yaml

./configs:
scenario.yaml

./scripts:
check-status-v2.sh      check-status.sh
```

## Creating an environment file

Creating a separate environment file works exactly the same way and has the same benefits as
creating a separate scenario file outlined in the ["Writing a scenario" guide][2].

Let's update our test plan to specify the environment in a separate file:

```bash
touch configs/environment.yaml
```

Make sure the `environment.yaml` contains a copy of the environment config currently in your
`test-plan.yaml` file:

```yaml
name: Docker compose environment config
description: A docker compose environment config
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

Finally, update the `test-plan.yaml` file to use the new `environment.yaml` file:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: "scenario executed with test plan variable"
scenario:
  from:
    kind: local
    relative_path: configs/scenario.yaml
environment:
  from:
    kind: local
    relative_path: configs/environment.yaml
```

Let's verify this has made no material difference to the templated test plan:

```bash
rtf template test-plan.yaml --check
```

The output is similar to this:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  scenario_variable: scenario env var value from test plan
matrix:
  variant_names: null
  dimensions: {}
  compound: {}
custom_providers: []
scenario:
  ...
environment:
  ...
```

You now have a reusable environment in its own file! See the [framework reference docs][3] for the
full Environment config structure.

## Adding additional compose files

RTF supports [multiple compose files][5] via the `compose_files` key — each file is passed to
`docker compose up` using the `-f` flag.

Let's create a new compose file.

```bash
mkdir data
touch data/compose-app.yaml
```

We are going to create a python web server that returns `Hello, World!` when called. Add the
following content to the `compose-app.yaml` file:

```yaml
services:
  app:
    image: python:alpine
    environment:
      HELLO_MESSAGE: Hello, World!
    ports:
      - 8000:8000
    command:
      - python
      - -c
      - |
        from http.server import HTTPServer, BaseHTTPRequestHandler
        import os
        class H(BaseHTTPRequestHandler):
            def do_GET(self):
                msg = os.environ.get('HELLO_MESSAGE').encode()
                self.send_response(200)
                self.send_header('Content-Type', 'text/plain')
                self.end_headers()
                self.wfile.write(msg)
            def log_message(self, *a):
                pass
        HTTPServer(('', 8000), H).serve_forever()
```

Let's add this to our `compose_files` in `environment.yaml`:

```yaml
name: Docker compose environment config
description: A docker compose environment config
compose_files:
  - name: docker-compose.yaml
    kind: inline
    content: |
      services:
        hello-world:
          image: nginx:alpine
          ports:
            - "8080:80"
# --- Add the compose-app.yaml file ---
  - name: compose-app.yaml
    kind: relative_path
    path: ../data/compose-app.yaml
# -------------------------------------
```

> **Note** The path to the `compose-app.yaml` file is relative to the `environment.yaml` file its
> path is defined in.

We have not explained File Providers yet and will cover them in detail in the
["Using file providers" guide][5].

Let's run the test plan

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
Response: <removed for brevity>
HTTP status: 200
[+] down 3/3
 ✔ Container docker-compose-environment-config-app-1         Removed     10.1s
 ✔ Container docker-compose-environment-config-hello-world-1 Removed     0.1s
 ✔ Network docker-compose-environment-config_default         Removed     0.1s
```

In addition to the `docker-compose-environment-config-hello-world-1` container that was running
before, we now can also see the `docker-compose-environment-config-app-1` starting up. You now have
an additional service running from a new compose file! Let's update our test to actually use it!

## Using environment variables

We have a running `nginx` server and our new `python` web server. We are going to update the `nginx`
config so that when we call `nginx`, it forwards our request to our new service. We are going to do
this by updating the docker compose configuration in our `environment.yaml` file:

```yaml
name: Docker compose environment config
description: A docker compose environment config
compose_files:
  - name: docker-compose.yaml
    kind: inline
    content: |
      services:
        hello-world:
          image: nginx:alpine
          ports:
            - "8080:80"
# -------- Add nginx config ---------
          configs:
            - source: nginx_conf
              target: /etc/nginx/conf.d/default.conf
      configs:
        nginx_conf:
          content: |
            server {
              listen 80;
              location / {
                proxy_pass http://app:8000;
              }
            }
# -----------------------------------
  - name: compose-app.yaml
    kind: relative_path
    path: ../data/compose-app.yaml
```

Let's run the test plan:

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
Response: Hello, World!
HTTP status: 200
[+] down 3/3
 ✔ Container docker-compose-environment-config-app-1         Removed     10.1s
 ✔ Container docker-compose-environment-config-hello-world-1 Removed     0.1s
 ✔ Network docker-compose-environment-config_default         Removed     0.1s
```

> **Note** Inlining configuration to docker compose like this is an antipattern in RTF, as it
> requires you to supply a completely new docker compose YAML file if you want to update any part of
> its configuration. There is a better way to do this, using File Providers. We cover how to do this
> in the ["Using file providers" guide][5].

Notice that we now also get `Response: Hello, World!`. Now, everything in our test is connecting as
expected! Let's verify this by changing the response message using an environment variable.

First, let's update our `environment.yaml` to define an environment variable for the docker compose
stack to use. We are not going to set this environment variable using RTF variables, instead we are
just going to hardcode it for ease. The ["Writing a scenario" guide][0] contains an example of
setting environment variables using RTF variables. Update `environment.yaml`:

```yaml
name: Docker compose environment config
description: A docker compose environment config
# ------ Add an environment variable ------
env_vars:
  HELLO_MESSAGE: "Goodbye, World!"
# -----------------------------------------
compose_files:
  - name: docker-compose.yaml
    kind: inline
    content: |
      services:
        hello-world:
          image: nginx:alpine
          ports:
            - "8080:80"
          configs:
            - source: nginx_conf
              target: /etc/nginx/conf.d/default.conf
      configs:
        nginx_conf:
          content: |
            server {
              listen 80;
              location / {
                proxy_pass http://app:8000;
              }
            }
  - name: compose-app.yaml
    kind: relative_path
    path: ../data/compose-app.yaml
```

We also need to update `compose-app.yaml` to use this environment variable, instead of the hardcoded
value:

```yaml
services:
  app:
    image: python:alpine
    environment:
# ---- Replace hardcoded message with one from the env var ----
      HELLO_MESSAGE: ${HELLO_MESSAGE}
# ------------------------------------------------------------
    ports:
      - 8000:8000
    command:
      - python
      - -c
      - |
        from http.server import HTTPServer, BaseHTTPRequestHandler
        import os
        class H(BaseHTTPRequestHandler):
            def do_GET(self):
                msg = os.environ.get('HELLO_MESSAGE').encode()
                self.send_response(200)
                self.send_header('Content-Type', 'text/plain')
                self.end_headers()
                self.wfile.write(msg)
            def log_message(self, *a):
                pass
        HTTPServer(('', 8000), H).serve_forever()
```

> **Note** This is making use of [docker compose variable interpolation][6].

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

Our test plan has successfully used an environment variable from RTF to set configuration in our
docker compose environment!

## Next steps

In this guide, we moved environment config into its own file, added an additional compose service,
and used RTF environment variables to configure the docker compose stack. Next, we'll guide you
through how to use file providers.

[Using file providers][5]

[0]: writing-a-scenario.md
[1]: https://docs.docker.com/compose/
[2]: writing-a-scenario.md#creating-a-scenario-file
[3]: ../../reference/framework/index.md
[4]: https://docs.docker.com/reference/cli/docker/compose/#use--f-to-specify-the-name-and-path-of-one-or-more-compose-files
[5]: using-file-providers.md
[6]: https://docs.docker.com/compose/how-tos/environment-variables/variable-interpolation/#ways-to-set-variables-with-interpolation
