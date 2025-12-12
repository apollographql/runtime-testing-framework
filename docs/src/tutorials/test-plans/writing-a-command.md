<!-- diataxis-type: tutorial -->

# Writing a command

This guide assumes you've completed the ["Writing a test plan"](writing-a-test-plan.md) guide. You
should already have a `test-plan.yaml` file in a directory named `rtf-hello-world`. We're going to
remove the variables and matrix added in the final step of the "Writing a test plan" guide. Your
`test-plan.yaml` file should contain:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "scenario command executed"
environment:
  inline:
    name: Inline environment config
    description: An inline environment config
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment setup command executed"
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment teardown command executed"
```

## Environment variables

Wherever it's possible to specify a `command`, it's also possible to specify `env_vars`. Any
environment variables defined in the test plan config will be made available to the command when it
is being executed. Let's try to use an environment variable in our scenario command:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    command:
      name: scenario.sh
      kind: inline
      content: |
        #!/usr/bin/env sh

        echo "$SCENARIO_ENV"
# --- Add an environment variable ---
    env_vars:
        SCENARIO_ENV: scenario command executed
# -----------------------------------
environment:
  inline:
    name: Inline environment config
    description: An inline environment config
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment setup command executed"
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment teardown command executed"
```

We're now hoping to echo the exact same message, but this time using an environment variable. If we
run the test plan:

```bash
rtf run test-plan.yaml
```

You should see output similar to this:

```
environment setup command executed
scenario command executed
environment teardown command executed
```

This works exactly as we hoped!

## Inline command file

Let's look at the structure of the `command` configuration. We have three fields: `name`, `kind`,
and `content`.

- `name` is the name of the file that the scenario will execute. We will be able to see that file in
  the output when we execute the test plan.
- `kind` is used to specify how the file content will be sourced. In this case, we are writing the
  file content `inline` to the test plan config. The next section will discuss how we can refer to
  separate file.
- `content` is a key specifically required when the `kind` is `inline` and is used to define the
  file's content.

> **Note** Under the hood, the `command` is using a subset of file providers. These are discussed
> more in the ["Using file providers" guide](using-file-providers.md)

## Local command file

Defining command files inline makes sense for very simple shell scripts but would quickly become
unmanageable for more complex files. You can also save your command scripts in a separate file and
refer to that file in your command config.

Let's start by creating a `scenario.sh` file in a `scripts` subdirectory:

```bash
mkdir scripts
touch scripts/scenario.sh
```

Make sure the following content is in the `scenario.sh` script:

```sh
#!/usr/bin/env sh

echo "Running scenario from an external file"
echo "$SCENARIO_ENV"
```

Finally, let's update our `test-plan.yaml` to point to this file, instead of using the inline
content:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
scenario:
  inline:
    name: Inline scenario config
    description: An inline scenario config
    command:
      name: scenario.sh
# --- Update from inline to relative_path ---
      kind: relative_path
      path: scripts/scenario.sh
# -------------------------------------------
    env_vars:
      SCENARIO_ENV: scenario command executed
environment:
  inline:
    name: Inline environment config
    description: An inline environment config
    setup:
      command:
        name: setup.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment setup command executed"
    teardown:
      command:
        name: teardown.sh
        kind: inline
        content: |
          #!/usr/bin/env sh

          echo "environment teardown command executed"
```

There are two updates to note here. First, we've changed `kind` from `inline` to `relative_path`.
This updates our test plan to look for the command file at a relative path. The `content` key is no
longer required for this `kind`; instead, the `path` key is used to define where the file is stored.
The `name` doesn't have to match the filename on the `path` - it just sets what rtf will store the
file as in the `providers` directory.

> **Note** The path is relative to the `test-plan.yaml` file's location, not relative to where the
> CLI is run from. This is so that the test plan can refer to files in the directory structure in a
> predictable way.

Let's run the test plan again:

```bash
rtf run test-plan.yaml
```

Output:

```
environment setup command executed
Running scenario from an external file
scenario command executed
environment teardown command executed
```

The extra echo confirms we've used the new `scenario.sh` file. You should also see that the
`scenario.sh` file in the `output/providers/scenario_providers` directory matches what was defined
in the `scenario.sh` file we just created. Copying files in this way is intentional behavior for the
`relative_path` file provider, other providers have files generated by rtf directly, this is one of
the few that references files created before rtf runs. It guarantees the path where the file will be
located so rtf can execute successfully:

```bash
cat output/providers/scenario_providers/scenario.sh
```

Output:

```sh
#!/usr/bin/env sh

echo "Running scenario from an external file"
echo "$SCENARIO_ENV"
```

We have only updated the `command` for the `scenario` in this guide. The `command` in
`environment.setup` and `environment.teardown` works in exactly the same way.

---

In this guide, we've covered writing more powerful commands both inline and in other files. Next,
we'll guide you through how to write a scenario config.

**Next:** [Writing a scenario](writing-a-scenario.md)
