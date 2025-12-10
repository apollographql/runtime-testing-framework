<!-- diataxis-type: tutorial -->

# Using a custom provider

This guide assumes you've completed the
["Writing a custom provider definition"](writing-a-custom-provider-definition.md) guide. You should
already have the files in a directory named `rtf-custom-provider`. Your directory should be in the
state it was at the end of that guide.

```bash
$ ls -R
my-provider.yaml scripts

./scripts:
generate.sh
```

You should also have no `output` directory. If you do, remove it before continuing:

```bash
rm -rf output/
```

## Creating a test plan

Now that we have a custom provider, let's use it in a test plan. Create an empty test plan file:

```bash
touch test-plan.yaml
```

Add the following content to `test-plan.yaml`:

```yaml
name: Custom Provider Test
description: A test plan that uses the env-generator custom provider
custom_providers:
  - kind: local
    relative_path: .
    using:
      env_generator: my-provider.yaml
scenario:
  inline:
    name: Empty scenario
    description: A scenario that does nothing
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh
environment:
  inline:
    name: Custom provider environment
    description: An environment that uses the env-generator custom provider
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "=== ENV_VARS.txt contents ==="
          source "$ENV_GENERATOR/ENV_VARS.txt"
          echo "PROJECT_NAME=$PROJECT_NAME"
          echo "LOG_LEVEL=$LOG_LEVEL"
          echo ""
          echo "=== base-config.txt contents ==="
          cat "$ENV_GENERATOR/base-config.txt"
      file_providers:
        - name: env-generator
          env_var: ENV_GENERATOR
          kind: custom_provider
          type: env_generator
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh
```

This test plan does the following:

- The `custom_providers` section registers the custom provider definition. Each entry specifies:
  - `kind: local`: indicates the provider definition is in the local filesystem.
  - `relative_path`: the directory containing the provider definition file.
  - `using`: a map of names to provider definition files. The name (`env_generator`) is used to
    reference the provider in file providers.
- The `scenario` is an empty command that does nothing. This is acceptable for testing the custom
  provider.
- The `environment.setup` uses the custom provider as a file provider. The `file_providers` entry
  specifies:
  - `name`: a name for this file provider instance.
  - `env_var`: the environment variable that will contain the path to the provider's output
    directory.
  - `kind: custom_provider`: indicates this is a custom provider.
  - `type`: the name registered in `custom_providers` (in this case, `env_generator`).
- The setup command sources the `ENV_VARS.txt` file using `$ENV_GENERATOR/ENV_VARS.txt` and prints
  the values it sets, then prints the contents of `base-config.txt`.
- The `teardown` is an empty command that does nothing.

## First template attempt

Let's check if the test plan templates correctly:

```bash
rtf template test-plan.yaml
```

This fails with an error:

```
ERROR (environment.setup.file_providers.ENV_GENERATOR.definition) missing required custom provider arguments: ["project_name"]
```

This error occurs because the `env-generator` custom provider has a `project_name` variable with no
default value. Notice that `log_level` is not in the error because it has a default value of
`"info"`.

## Adding arguments to the custom provider

To fix this, we need to provide the required `project_name` argument. Arguments are added as
additional fields on the file provider. Update the `file_providers` section in `test-plan.yaml`:

```yaml
name: Custom Provider Test
description: A test plan that uses the env-generator custom provider
custom_providers:
  - kind: local
    relative_path: .
    using:
      env_generator: my-provider.yaml
scenario:
  inline:
    name: Empty scenario
    description: A scenario that does nothing
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh
environment:
  inline:
    name: Custom provider environment
    description: An environment that uses the env-generator custom provider
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "=== ENV_VARS.txt contents ==="
          source "$ENV_GENERATOR/ENV_VARS.txt"
          echo "PROJECT_NAME=$PROJECT_NAME"
          echo "LOG_LEVEL=$LOG_LEVEL"
          echo ""
          echo "=== base-config.txt contents ==="
          cat "$ENV_GENERATOR/base-config.txt"
      file_providers:
        - name: env-generator
          env_var: ENV_GENERATOR
          kind: custom_provider
          type: env_generator
# --- Add the required argument ---
          project_name: my-test-project
# ---------------------------------
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh
```

Now run the template command again:

```bash
rtf template test-plan.yaml
```

This time the command succeeds and outputs the fully templated test plan. The custom provider's
arguments are resolved and used to template the provider definition.

## Running the test plan

Let's run the test plan to see the custom provider in action:

```bash
rtf run test-plan.yaml
```

The output shows:

```
Generated environment config in /path/to/rtf-custom-provider/output/providers/env-generator/RTF_OUTPUT
=== ENV_VARS.txt contents ===
PROJECT_NAME=my-test-project
LOG_LEVEL=info

=== base-config.txt contents ===
# Base configuration
# Project-specific values set via ENV_VARS.txt
```

This confirms:

1. The custom provider ran and generated its output files.
2. The `PROJECT_NAME` is set to the value we provided as an argument.
3. The `LOG_LEVEL` uses the default value of `"info"`.
4. The `base-config.txt` file is available to the setup command.

Remove the output directory before continuing:

```bash
rm -rf output/
```

## Overriding the default value

You can override the default `log_level` value by adding it as an argument. Update the
`file_providers` section:

```yaml
      file_providers:
        - name: env-generator
          env_var: ENV_GENERATOR
          kind: custom_provider
          type: env_generator
          project_name: my-test-project
# --- Override the default ---
          log_level: debug
# ----------------------------
```

Run the test plan again:

```bash
$ rtf run test-plan.yaml
Generated environment config in /path/to/rtf-custom-provider/output/providers/env-generator/RTF_OUTPUT
=== ENV_VARS.txt contents ===
PROJECT_NAME=my-test-project
LOG_LEVEL=debug

=== base-config.txt contents ===
# Base configuration
# Project-specific values set via ENV_VARS.txt
```

The `LOG_LEVEL` is now `"debug"` instead of the default `"info"`.

Remove the output directory before continuing:

```bash
rm -rf output/
```

## Using test plan variables for arguments

Instead of hardcoding values in the arguments, you can use test plan variables. This allows the same
test plan to be run with different configurations. Update `test-plan.yaml`:

```yaml
name: Custom Provider Test
description: A test plan that uses the env-generator custom provider
# --- Add variables ---
variables:
  project_name: my-variable-project
# ---------------------
custom_providers:
  - kind: local
    relative_path: .
    using:
      env_generator: my-provider.yaml
scenario:
  inline:
    name: Empty scenario
    description: A scenario that does nothing
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh
environment:
  inline:
    name: Custom provider environment
    description: An environment that uses the env-generator custom provider
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "=== ENV_VARS.txt contents ==="
          source "$ENV_GENERATOR/ENV_VARS.txt"
          echo "PROJECT_NAME=$PROJECT_NAME"
          echo "LOG_LEVEL=$LOG_LEVEL"
          echo ""
          echo "=== base-config.txt contents ==="
          cat "$ENV_GENERATOR/base-config.txt"
      file_providers:
        - name: env-generator
          env_var: ENV_GENERATOR
          kind: custom_provider
          type: env_generator
# --- Use templating for the argument value ---
          project_name: "{{ project_name }}"
          log_level: debug
# ---------------------------------------------
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh
```

Run the test plan:

```bash
$ rtf run test-plan.yaml
Generated environment config in /path/to/rtf-custom-provider/output/providers/env-generator/RTF_OUTPUT
=== ENV_VARS.txt contents ===
PROJECT_NAME=my-variable-project
LOG_LEVEL=debug

=== base-config.txt contents ===
# Base configuration
# Project-specific values set via ENV_VARS.txt
```

The `PROJECT_NAME` is now set from the test plan variable. You can also override this at runtime
using the `--var` flag:

```bash
$ rtf run test-plan.yaml --var project_name=runtime-override
Generated environment config in /path/to/rtf-custom-provider/output/providers/env-generator/RTF_OUTPUT
=== ENV_VARS.txt contents ===
PROJECT_NAME=runtime-override
LOG_LEVEL=debug

=== base-config.txt contents ===
# Base configuration
# Project-specific values set via ENV_VARS.txt
```

---

In this guide, we have covered how to use a custom provider in a test plan, pass arguments to it,
and use test plan variables for dynamic configuration.
