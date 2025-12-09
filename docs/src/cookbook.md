<!-- diataxis-type: howto -->

# Cookbook

_Common patterns and idioms for working with RTF_

This page is a collection of short configuration snippets and strategies for working with RTF Test
Plans to achieve specific goals. For an introductory overview of how to work with RTF please refer
to the [getting started guide][0]. For a technical reference on RTF as a whole please refer to the
[framework section of the docs][1].

## Running multiple iterations of a Test Plan

**Problem**: You have a test plan that you would like to run multiple times in order to collect
results that can be analysed statistically.

**Solution**: RTF's [matrix][2] feature can be used with a dummy dimension that will be expanded
over to produce `n` copies of a given test plan (or dimensions within an existing matrix) by
providing a series of unique values for the dimension:

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

## Determining how many variants of test plan will be run by a given matrix

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
$ rtf expand-matrix test-plan.yaml
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
$ rtf test-plan.yaml | jq '.variants | length'
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
$ rtf expand-matrix test-plan.yaml |
    jq -r '.variants | map(.name) | join("\n")'
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

[0]: ./src/guides/index.md
[1]: ./framework/index.md
[2]: ./framework/test-plans.md#working-with-matrices
[3]: https://jqlang.org/
