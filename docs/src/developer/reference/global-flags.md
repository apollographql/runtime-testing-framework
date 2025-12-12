<!-- diataxis-type: reference -->

# Global flags

The CLI exposes the following global flags available to all subcommands:

## `--var`

Provides a single additional templating variable.

**Syntax:**

```
--var <KEY>=<VALUE>
```

**Example:**

```bash
rtf run test-plan.yaml --var 'message="hello, world"'
```

Multiple `--var` flags can be specified to override multiple variables:

```bash
rtf run test-plan.yaml \
  --var 'message="hello"' \
  --var 'subject="world"'
```

## `--vars`

Provides multiple templating variables from a JSON file.

**Syntax:**

```
--vars <PATH>
```

Where `<PATH>` is a path to a JSON file containing key-value pairs.

**Example:**

Given a file `variables.json`:

```json
{
  "message": "hello",
  "subject": "world"
}
```

```bash
rtf run test-plan.yaml --vars variables.json
```

## Precedence

Variables are resolved in the following order (later values override earlier):

1. Variables defined in the test plan's `variables` section
2. Variables provided via `--vars` JSON file
3. Variables provided via `--var` flags

## Related

- [Hello, world! tutorial][0] - demonstrates variable overrides
- [Test Plans reference][1] - documents the `variables` section

[0]: ../../tutorials/hello-world.md
[1]: ../../reference/framework/test-plans.md
