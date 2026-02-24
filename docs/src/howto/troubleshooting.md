<!-- diataxis-type: howto -->

# Troubleshooting

This page covers common issues encountered when using RTF and how to resolve them.

## Output directory already exists

**Symptom:**

```text
ERROR /path/to/output already exists and is non-empty
```

**Cause:** RTF refuses to overwrite existing output directories to prevent accidental data loss.

**Solution:** Either remove the existing directory or specify a different output directory:

```bash
rm -rf output
rtf run test-plan.yaml
```

Or:

```bash
rtf run test-plan.yaml --outdir=new_output
```

## Missing template variable

**Symptom:**

```text
ERROR Templating failed
(scenario.env_vars.MY_VAR) Missing template variables definition. Make sure the variable is defined in the scenario or environment config variable definitions
  - my_variable: "Description of the variable"
```

**Cause:** A config file references a variable that isn't defined in the [Test Plan's][0]
`variables` section and has no default value.

**Solution:** Either define the variable in the Test Plan:

```yaml
variables:
  my_variable: "some value"
```

Or add a default value in the scenario/environment's `variable_definitions`:

```yaml
variable_definitions:
  - name: my_variable
    description: "Description of the variable"
    default: "default value"
```

## Static analysis check failed - required file

**Symptom:**

```text
ERROR Static analysis checks failed
(MY_FILE) A required file has not been defined
You must provide a file for MY_FILE
```

**Cause:** A `required` [File Provider][0] exists that must be replaced with an actual provider in
the Test Plan's overrides.

**Solution:** Add an override in the Test Plan that replaces the required provider:

```yaml
environment:
  from:
    kind: local
    relative_path: ./environment.yaml
  overrides:
    file_providers:
      - name: my-file
        env_var: MY_FILE
        kind: inline
        content: "actual content"
```

## Static analysis check failed - file not found

**Symptom:**

```text
ERROR Static analysis checks failed
(scenario.command.command_provider) The requested file did not exist
provided path was file:///path/to/missing-script.sh
```

**Cause:** A `relative_path` [Command Provider][0] or File Provider references a file that doesn't
exist.

**Solution:**

1. Verify the file exists at the specified path
2. Remember that paths are relative to the config file containing them, not the Test Plan
3. If using overrides, paths in the override are relative to the Test Plan file

## Command execution failed

**Symptom:**

```text
ERROR Unable to execute the my-script.sh command: "/path/to/my-script.sh" failed to terminate successfully
```

**Cause:** The command exited with a non-zero status code.

**Solution:** Check the command output above the error for clues. Common causes include:

1. **Script logic errors** - The script encountered an error during execution
2. **Missing dependencies** - A command used within the script isn't available

## Interpreter not found

**Symptom:**

```text
ERROR Unable to execute the my-script.sh command: No such file or directory (os error 2)
```

**Cause:** The script's shebang references an interpreter that doesn't exist.

**Solution:** Use a portable shebang:

```bash
#!/usr/bin/env bash
```

Avoid hardcoded paths like `#!/bin/bash` which may not exist on all systems.

## GitHub authentication failed

**Symptom:**

```text
ERROR malformed scenario config section
ERROR Unable to load and resolve test plan: HTTP status client error (401 Unauthorized) for url (https://api.github.com/repos/org/repo/contents/path/to/file.yaml)
```

**Cause:** The `GITHUB_TOKEN` environment variable is not set or the token lacks required
permissions.

**Solution:** Export a valid GitHub personal access token:

```bash
export GITHUB_TOKEN="ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
```

The token needs read access to the repositories referenced in the Test Plan.

## GraphOS authentication failed

**Symptom:**

```text
ERROR errors returned when running graphql operation
ERROR Unable to resolve and write SUPERGRAPH file: unable to fetch details for graph@variant: graphql errors returned when running operation: ["HTTP fetch failed from 'kotlin': 406: Not Acceptable", "Invalid credentials provided"]
```

**Cause:** The `APOLLO_KEY` environment variable is not set or is invalid.

**Solution:** Export a valid Apollo API key:

```bash
export APOLLO_KEY="service:my-graph:xxxxxxxxxxxxxxxxxxxx"
```

## Matrix variant names are not unique

**Symptom:**

```text
ERROR The provided variant_names template produced duplicate names: ["duplicate_name"]
```

**Cause:** The `matrix.variant_names` template produces duplicate names for different variants.

**Solution:** Ensure the template includes enough dimensions to produce unique names:

```yaml
matrix:
  variant_names: "${dim1}_${dim2}"  # Include all varying dimensions
  dimensions:
    dim1: ["a", "b"]
    dim2: [1, 2]
```

Use `rtf expand-matrix` to check how the matrix dimensions will be named.

## Debugging tips

### Use verbose logging

Add `-v` flags to increase log verbosity:

```bash
rtf run test-plan.yaml -v      # INFO level
rtf run test-plan.yaml -vv     # DEBUG level
rtf run test-plan.yaml -vvv    # TRACE level
```

### Preview templated config

Use `rtf template` to see the fully resolved config before running:

```bash
rtf template test-plan.yaml
```

Add `--check` to also run static analysis:

```bash
rtf template test-plan.yaml --check
```

### Inspect matrix expansion

Use `rtf expand-matrix` to see all variants that will be run:

```bash
rtf expand-matrix test-plan.yaml
```

### Check File Provider resolution

The output directory contains a `providers/` subdirectory with all resolved File Provider outputs.
Inspect these files to verify providers produced the expected content.

[0]: ../reference/glossary.md
