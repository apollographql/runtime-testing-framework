<!-- diataxis-type: howto -->

# Working with common RTF patterns

This page is a collection of short configuration snippets and strategies for working with RTF Test
Plans to achieve specific goals. For an introductory overview of how to work with RTF please refer
to the [getting started guide][0]. For a technical reference on RTF as a whole please refer to the
[framework section of the docs][1].

## Only running a stage from a Test Plan

**Problem**: You only want to run a single stage of a Test Plan rather than the entire thing.

**Solution**: RTF's `run` command supports limiting execution to one of the three stages:

- Environment setup
- Scenario
- Environment teardown

To do so, simply add the appropriate flag to your use of `rtf run`:

```bash
# Only the environment setup
rtf run --environment-up <test-plan>

# Only the scenario
rtf run --scenario <test-plan>

# Only the environment teardown
rtf run --environment-down <test-plan>
```

**Discussion**: All other `rtf run` flags behave normally in combination with these flags, but it is
an error to specify multiple at the same time. If your Test Plan includes a matrix this will still
result in an execution per matrix variant. To limit execution to a single variant you should make
use of the `--var` flag to pin variables to a single value (see below).

## Limiting a matrix based Test Plan to run a single dimension

**Problem**: Your Test Plan contains a matrix but you would like to run it for a single dimension.

**Solution**: You can use the command line `--var` flag to replace individual matrix dimensions with
scalar variables:

```bash
rtf run --var 'router_version=v2.5.0' --var 'graph_ref=foo@prod' test-plan.yaml
```

Alternatively, if you are happy to edit the Test Plan itself, you can always comment out the matrix
definition and replace it with variable definitions like so:

```yaml
variables:
  router_cpu_limit: "4"
  # New variables to replace the matrix dimensions
  router_version: "v2.5.0"
  graph_ref: "foo@prod"

# Commented out matrix dimensions
#
# matrix:
#   dimensions:
#     router_version:
#       - "v2.5.0"
#       - "v2.6.1"
# 
#     graph_ref:
#       - "foo@prod"
#       - "bar@production"
```

**Discussion**: The use of command line arguments is recommended over editing the test plan file
directly as it prevents the common issue of accidentally committing a modified test plan that now no
longer runs the originally intended set of dimensions.

## Running multiple iterations of a Test Plan

**Problem**: You have a test plan that you would like to run multiple times in order to collect
results that can be analyzed statistically.

**Solution**: RTF's [matrix][2] feature can be used with a placeholder dimension that will be
expanded over to produce `n` copies of a given test plan (or dimensions within an existing matrix)
by providing a series of unique values for the dimension:

```yaml
matrix:
  variant_names: "iteration_${n}"
  dimensions:
    n: [0, 1, 2, 3, 4]
```

This also works if you have an existing matrix:

```yaml
matrix:
  variant_names: "${my_dimension}_${n}"
  dimensions:
    n: [0, 1, 2, 3, 4]
    my_dimension: ["foo", "bar"]
```

**Discussion**: You _must_ include your iteration variable within your `variant_names` template in
order to produce unique variant names, as each original variant you are wanting to repeat will have
the same values other than this.

Using this approach is encouraged over simply running the Test Plan multiple times manually as it
allows RTF to cache provider data internally as it runs in order to reduce network calls and work
required to generate the output of each provider.

## Determining how many variants of Test Plan will be run by a given matrix

**Problem**: You have written a Test Plan that defines a non-trivial matrix and you want to work out
something like the expected running time or other properties related to the number of variants being
run.

**Solution**: The `rtf expand-matrix` subcommand can be used to output the fully expanded variables
for each variant in the order they will be executed by `rtf run`:

```yaml
# Example matrix setup
matrix:
  variant_names: "${color}_${fruit}_${name}_${count}"
  dimensions:
    name: ["foo", "bar"]
    count: [1, 2, 3]
  include:
    - fruit: apple
      color: red
    - fruit: pear
      color: green
```

```bash
rtf expand-matrix test-plan.yaml
```

Output:

```json
{
  "variants": [
    {
      "name": "red_apple_foo_1",
      "variables": {
        "color": "red",
        "count": 1,
        "fruit": "apple",
        "name": "foo"
      }
    },
    ...
  ]
}
```

[jq][3] can be used to count the variants like so:

```bash
rtf expand-matrix test-plan.yaml | jq '.variants | length'
```

Output:

```text
12
```

**Discussion**: This will work even for Test Plans without a matrix by returning the single
"variant" representing the top level variables for the Test Plan.

The variants returned by `rtf expand-matrix` are computed using the Test Plan as it is written.
Specifying additional matrix dimensions via the `--vars` flag on `rtf run` will alter the number of
variants.

## Previewing matrix variant names

**Problem**: You are specifying a custom matrix variant name template using `matrix.variant_names`
and you want to check that it will produce the expected directory names.

**Solution**: The `rtf expand-matrix` command can be used alongside [jq][3] to output a list of
matrix variant names as a shell one-liner:

```yaml
# Example matrix setup
matrix:
  variant_names: "${color}_${fruit}_${name}_${count}"
  dimensions:
    name: ["foo", "bar"]
    count: [1, 2, 3]
  include:
    - fruit: apple
      color: red
    - fruit: pear
      color: green
```

```bash
rtf expand-matrix test-plan.yaml |
    jq -r '.variants | map(.name) | join("\n")'
```

Output:

```text
red_apple_foo_1
green_pear_foo_1
red_apple_bar_1
green_pear_bar_1
red_apple_foo_2
green_pear_foo_2
red_apple_bar_2
green_pear_bar_2
red_apple_foo_3
green_pear_foo_3
red_apple_bar_3
green_pear_bar_3
```

**Discussion**: The variants returned by `rtf expand-matrix` are computed using the Test Plan as it
is written. Specifying additional matrix dimensions via the `--vars` flag on `rtf run` will alter
the number of variants which in turn may result in a previously valid `variant_names` template
becoming invalid if it now produces non-unique names.

## Conditional YAML merging

**Problem**: You want to use a variable to conditionally decide which YAML snippets get merged into
your YAML files.

**Solution**: Use a conditional file provider to apply the correct YAML snippets based on the
variable's value. Here, we use the example of optionally merging DataDog telemetry configuration
into router configuration if the `telemetry_backend` variable is set to `"datadog"`:

```yaml
file_providers:
  - name: router-config.yaml
    env_var: ROUTER_CONFIG
    kind: conditional
    cases:
      # DataDog: include telemetry exporter config
      - where: { var: telemetry_backend, eq: datadog }
        kind: merge_yaml
        base:
          kind: relative_path
          path: data/router-config.yaml
        overrides:
          - kind: relative_path
            path: data/datadog-telemetry-overlay.yaml
          - kind: graphos_subgraph_router_url_overrides
            graph_ref: "{{ graph_ref }}"
            url_format: "docker"

      # Local: no exporter config needed
      - where: { var: telemetry_backend, eq: local }
        kind: merge_yaml
        base:
          kind: relative_path
          path: data/router-config.yaml
        overrides:
          kind: graphos_subgraph_router_url_overrides
          graph_ref: "{{ graph_ref }}"
          url_format: "docker"
```

This allows you to run the same test plan with different telemetry backends:

```bash
# Run with local telemetry
rtf run test-plan.yaml -v "telemetry_backend=local"

# Run with DataDog telemetry
rtf run test-plan.yaml -v "telemetry_backend=datadog"
```

**Discussion**: This pattern combines `kind: conditional` with `kind: merge_yaml` to dynamically
compose configuration files based on variable values. Each case applies different overlays while
sharing the same base configuration.

Key considerations:

- The first matching `where` clause is used; order cases from most to least specific
- All cases must produce valid output—RTF validates during static analysis
- Base configuration should omit sections that overlays will provide to avoid merge conflicts
- Multiple overlays can be chained as an array, merging in sequence

This pattern is useful when:

- Different deployment targets require different configuration snippets
- Feature flags should toggle configuration sections
- Environment-specific settings need conditional inclusion

The telemetry example above demonstrates this by conditionally including DataDog exporter
configuration only when `telemetry_backend=datadog`, while both cases share the same base router
config and subgraph URL overrides.

[0]: ../tutorials/index.md
[1]: ../reference/framework/index.md
[2]: ../reference/framework/test-plans.md#working-with-matrices
[3]: https://jqlang.org/
