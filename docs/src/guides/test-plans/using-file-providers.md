<!-- diataxis-type: tutorial -->

# Using file providers

This guide assumes you've completed the ["Writing an environment"](writing-an-environment.md) guide.
You should already have the files in a directory named `rtf-hello-world`. Your directory should be
in the state it was at the end of that guide:

```bash
$ ls -R
configs         scripts         test-plan.yaml

rtf-hello-world/configs:
environment.yaml        scenario.yaml

rtf-hello-world/scripts:
scenario.sh     setup.sh
```

## What are file providers?

We've already been using file providers in the `command` configuration throughout the previous
guides. File providers are rtf's way of referring to files and data that are required for the
environment setup, environment teardown, and scenario to run successfully. Commands make use of two
kinds of file providers, `inline` and `relative_path`. These are the most commonly used file
providers as they provide the file content either from the local filesystem or from within the test
plan configuration itself.

There are additional file providers that, amongst other things, can pull data from APIs (such as
GraphOS specific providers). These will not be discussed in this guide but can be seen in the
[framework reference](../../framework/file-providers.md).

All file providers follow the same basic principle - they create one or more files and place it on a
path for rtf to make use of. If you need to know that path (for your command script, for example),
rtf will assign an environment variable that contains the file's path. This is a deliberate design
choice that allows rtf to make changes to how it stores files obtained from providers without
breaking assumptions made about paths in user-written scripts.

## Adding an inline file

Let's see how file providers can be used in our test plan config. We'll add an inline file to our
scenario and just `cat` the output to the terminal. First, we need to make use of the
`file_providers` key in the `scenario.yaml`:

```yaml
name: Inline scenario config
description: An inline scenario config
variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
    default: "scenario executed with default value"
command:
  name: scenario.sh
  kind: relative_path
  path: ../scripts/scenario.sh
env_vars:
  SCENARIO_ENV: "{{ scenario_variable }}"
# --- Add a new file to the scenario ---
file_providers:
  - name: scenario.txt
    env_var: SCENARIO_TXT
    kind: inline
    content: |
      Some inline text content for our scenario
# --------------------------------------
```

> **Note** The environment setup and teardown configs can also define a `file_providers` section and
> it works in the exact same way as specified here. Files defined in the environment setup can also
> be referred to in the scenario and environment teardown using the same `env_var`. Similarly, files
> defined in the scenario will also be available to the teardown.

We haven't yet updated the scenario's command to make use of this, but let's look at what happens
when we run the test plan and examine the providers output:

```bash
$ rtf run test-plan.yaml
Using the override setup script
Environment setup complete. PROCESS_ID=2
Running scenario from an external file
scenario executed with test plan variable
Environment teardown complete. PROCESS_ID=2

$ ls output/providers
scenario.sh     scenario.txt    setup.sh        teardown.sh

$ cat output/providers/scenario.txt 
Some inline text content for our scenario
```

We've successfully updated our test plan to write a new `scenario.txt` file to the output with the
content we specified inline. Let's make use of this in our command script. Note that when specifying
file providers, we must set an `env_var`. This is the name of the environment variable that will be
set when this is run and it will contain the path to the file. Let's update our `scenario.sh`
script:

```sh
#!/usr/bin/env sh

echo "Running scenario from an external file"
# The SCENARIO_TXT environment variable is used to refer to the scenario.txt file's path
cat $SCENARIO_TXT
echo "$SCENARIO_ENV"
```

Now, if we run the test plan:

```bash
$ rtf run test-plan.yaml
Using the override setup script
Environment setup complete. PROCESS_ID=2
Running scenario from an external file
Some inline text content for our scenario
scenario executed with test plan variable
Environment teardown complete. PROCESS_ID=2
```

We can see the content from `scenario.txt` being printed to the terminal.

## File provider config structure

Now that we've seen a file provider being defined, let's discuss how the config is structured. There
are three required fields:

1. `name` is the name the file will be saved with in the `providers` directory of the output.
2. `env_var` is the environment variable the file's path will be stored in. This is used by
   subsequent commands to refer to the file.
3. `kind` is used to set which kind of file provider is being used. See the
   [file provider reference](../../framework/providers.md) for details on all the providers
   available.

Each file provider will have other fields that need to be defined, like `content` for `inline`.
These are specific to each provider type, and the `template` command will highlight any missing or
incorrectly defined keys.

## Adding a file from a relative path

Let's add a file from a relative path to the scenario config. The `file_provider` field can accept
as many files as you need via a list. Before adding the new file to the config, let's create the
file itself:

```bash
$ mkdir data
$ touch data/file.txt
```

Add the following content to `file.txt`:

```
More content from a file for our scenario
```

Let's update `scenario.yaml` to refer to this file, this time using the `relative_path` file
provider:

```yaml
name: Inline scenario config
description: An inline scenario config
variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
    default: "scenario executed with default value"
command:
  name: scenario.sh
  kind: relative_path
  path: ../scripts/scenario.sh
env_vars:
  SCENARIO_ENV: "{{ scenario_variable }}"
file_providers:
  - name: scenario.txt
    env_var: SCENARIO_TXT
    kind: inline
    content: |
      Some inline text content for our scenario
# --- Add a new relative_path provider ---
  - name: file.txt
    env_var: FILE_TXT
    kind: relative_path
    path: ../data/file.txt
# ----------------------------------------
```

> **Note** The path is relative to the `scenario.yaml` file!

Let's update our `scenario.sh` script too:

```sh
#!/usr/bin/env sh

echo "Running scenario from an external file"
cat $SCENARIO_TXT
# Add the cat command for file.txt
cat $FILE_TXT
echo "$SCENARIO_ENV"
```

Now, let's run the test plan:

```bash
$ rtf run test-plan.yaml
Using the override setup script
Environment setup complete. PROCESS_ID=2
Running scenario from an external file
Some inline text content for our scenario
More content from a file for our scenario
scenario executed with test plan variable
Environment teardown complete. PROCESS_ID=2

$ ls output/providers 
file.txt        scenario.sh     scenario.txt    setup.sh        teardown.sh
```

As expected, we also see the content of `file.txt` in our test plan execution. We can also see the
file itself in the output.

## Required files

As discussed in previous sections of this guide, the environment and scenario configs are designed
to be reused. For config designed to be reused often, you might want to force the user of the test
plan to define a file, but not give them a default file to work with. For this use case, the
`required` file provider is the perfect solution. Any test plan that tries to execute with a
`required` file provider will fail and tell the user to specify the file themselves (normally this
is done using `overrides`). Let's walk through how `required` can be used:

Let's say we need a configuration file for our environment setup. We don't want to give a base
example because users might end up using that without thinking about what configuration they
actually need for their specific test. In other words, we don't want it to "just work" by design.
Let's add a `required` file to our `environment.yaml`:

```yaml
name: Inline environment config
description: An inline environment config
setup:
  command:
    name: setup.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      PROCESS_ID="1"
      echo "Environment setup complete. PROCESS_ID=$PROCESS_ID"
      echo "{ \"process_id\": \"$PROCESS_ID\" }" > "$RTF_OUTPUT"
# --- Add a required file ---
  file_providers:
    - name: config.txt
      env_var: CONFIG
      kind: required
      message: Please specify a config file
# ---------------------------
  provides:
    - name: process_id
      description: The id of the process started in the environment setup
teardown:
  command:
    name: teardown.sh
    kind: inline
    content: |
      #!/usr/bin/env sh

      echo "Environment teardown complete. PROCESS_ID=$PROCESS_ID"
  env_vars:
    PROCESS_ID: "{{ process_id }}"
```

Now, let's see what happens when we try to template this test plan:

```bash
$ rtf template test-plan.yaml --check --var process_id="id"
ERROR (config.txt) a required file has not been defined.: Please specify a config file
```

We get an error saying we haven't defined a required file, along with the message we put in the
`message` field. We'd get the same error if we tried `rtf run`.

To make this work, the test plan user should make use of overrides. Let's update `test-plan.yaml`:

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
  overrides:
    setup:
      command:
        name: setup.sh
        kind: relative_path
        path: scripts/setup.sh
# --- Override the config.txt config ---
      file_providers:
        - name: config.txt
          env_var: CONFIG
          kind: inline
          content: |
            Config
# --------------------------------------
```

The override will match based on the `name` key. If we template now, we get:

```bash
$ rtf template test-plan.yaml --check --var process_id="id"
name: Hello World
description: A test plan created as a guide for writing test plans
variables:
  example_variable: variable
  scenario_variable: scenario executed with test plan variable
  process_id: id
matrix: {}
scenario:
  name: Inline scenario config
  description: An inline scenario config
  variable_definitions:
  - name: scenario_variable
    description: An example variable that the scenario expects to be defined
    default: scenario executed with default value
  command:
    name: scenario.sh
    kind: relative_path
    path: ../scripts/scenario.sh
    args: []
  env_vars:
    SCENARIO_ENV: scenario executed with test plan variable
  file_providers:
  - name: file.txt
    env_var: FILE_TXT
    kind: relative_path
    path: ../data/file.txt
  - name: scenario.txt
    env_var: SCENARIO_TXT
    kind: inline
    content: |
      Some inline text content for our scenario
environment:
  name: Inline environment config
  description: An inline environment config
  variable_definitions: []
  setup:
    command:
      name: setup.sh
      kind: relative_path
      path: scripts/setup.sh
      args: []
    env_vars: {}
    file_providers:
    - name: config.txt
      env_var: CONFIG
      kind: inline
      content: |
        Config
    provides:
    - name: process_id
      description: The id of the process started in the environment setup
      default: null
  teardown:
    command:
      name: teardown.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "Environment teardown complete. PROCESS_ID=$PROCESS_ID"
      args: []
    env_vars:
      PROCESS_ID: id
    file_providers: []
```

If you look at `setup.file_providers`, you can see that `config.txt` now uses the config from the
overrides. Our test plan no longer contains a `required` file, so we no longer get that error. The
`rtf run` command can also complete successfully now.

---

Congratulations! You've now completed the guide on how to write test plans.
