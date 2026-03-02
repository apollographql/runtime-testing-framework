<!-- diataxis-type: tutorial -->

# Writing a command

In this guide, we'll look at how the `command` configuration works. We'll add environment variables
to the scenario command, then move the command script to its own file.

> **Prerequisites**
>
> - Completed the ["Writing a test plan"][0] tutorial
> - An `rtf-hello-world` directory in the state it was at the end of that guide

Your directory should look like this:

```
rtf-hello-world/
├── configs/
│   ├── environment.yaml
│   └── scenario.yaml
└── test-plan.yaml
```

## Environment variables

Any place a `command` can be specified, `env_vars` can also be specified. Environment variables
defined here are set before the command runs. Let's add one to `configs/scenario.yaml`, updating the
script to reference it:

```yaml
name: Script scenario
description: A script scenario
command:
  name: scenario.sh
  kind: inline
  content: |
    #!/usr/bin/env sh

    echo "$SCENARIO_ENV"
# --- Add an environment variable ---
env_vars:
  SCENARIO_ENV: scenario executed
# ------------------------------------
```

Run the [Test Plan][1] to confirm this works:

```bash
rtf run test-plan.yaml
```

Output:

```
environment setup
scenario executed
environment teardown
```

Remove the output before continuing:

```bash
rm -rf output/
```

## Inline command structure

Let's look at the three fields in the `command` config:

- `name` is the name of the file RTF will write the script to. You can see it in the output
  directory after a run.
- `kind` specifies how the file content is sourced. Using `inline` means the content is written
  directly in the config.
- `content` is required when `kind` is `inline` and contains the script. Scripts must include a
  [shebang][2] line so the OS knows how to execute them.

## Local command files

Defining command scripts inline works for short scripts but becomes hard to maintain as scripts
grow. You can save command scripts in separate files and refer to them using `kind: relative_path`.

Create a `scripts` directory and a scenario script:

```bash
mkdir scripts
touch scripts/scenario.sh
```

Add the following to `scripts/scenario.sh`:

```sh
#!/usr/bin/env sh

echo "Running scenario from an external file"
echo "$SCENARIO_ENV"
```

Now update `configs/scenario.yaml` to point to this file:

```yaml
name: Script scenario
description: A script scenario
command:
  name: scenario.sh
# --- Switch from inline to relative_path ---
  kind: relative_path
  path: ../scripts/scenario.sh
# -------------------------------------------
env_vars:
  SCENARIO_ENV: scenario executed
```

Two things changed: `kind` is now `relative_path`, and `content` is replaced by `path`. The `name`
field still controls what the file is called in the output.

> **Note** Paths are always relative to the file that defines them — here `configs/scenario.yaml` —
> not relative to where the CLI is run from.

Verify with the `--check` flag:

```bash
rtf template test-plan.yaml --check
```

The output is similar to this:

```yaml
name: Hello World
description: A test plan created as a guide for writing test plans
...
```

Now run the Test Plan:

```bash
rtf run test-plan.yaml
```

Output:

```
environment setup
Running scenario from an external file
scenario executed
environment teardown
```

The extra `echo` confirms we're running the external script. RTF also copies the script to the
output — examine it to confirm:

```bash
cat output/providers/scenario_providers/scenario.sh
```

Output:

```sh
#!/usr/bin/env sh

echo "Running scenario from an external file"
echo "$SCENARIO_ENV"
```

RTF copies `relative_path` files to the providers output directory and executes from there. This
guarantees a stable path regardless of where the CLI is invoked from.

> **Note** The `command` config in `environment.setup` and `environment.teardown` works in exactly
> the same way as shown here.

Remove the output before continuing:

```bash
rm -rf output/
```

## Next steps

In this guide, we covered how `command` config works — inline scripts, environment variables, and
pointing to external script files. Next, we'll use variables and overrides to customize the Scenario
at runtime.

[Using variables and overrides][3]

[0]: writing-a-test-plan.md
[1]: ../../reference/glossary.md
[2]: https://en.wikipedia.org/wiki/Shebang_%28Unix%29
[3]: using-variables-and-overrides.md
