<!-- diataxis-type: reference -->

# Test Plans

The Test Plan is the top-level entry point for RTF. It defines variables, matrix dimensions, and
references to Scenario and Environment configurations.

> An example of a valid `test-plan.yaml` is provided in the [Full example](#full-example) section
> below.

## Top level keys

- `name`: The name for this Test Plan.
  - Uniqueness is not enforced by the `rtf` CLI but test plans should have unique names that can be
    used to distinguish them.
- `description`: A brief, human readable description of the behaviour of the Test Plan.
  - If there are any pre-requisites to running this Test Plan it is best to call them out here
    rather than in comments or other files (such as a README).
- `variables`: Key value pairs for templating the Test Plan where the variables are all scalar.
  - Scalar here is defined to be a number, string or boolean.
- `matrix`: Dimensions specified as key value pairs for templating the Test Plan where the variables
  are arrays of scalars.
  - Each matrix entry must have a consistent type for the variables array. Mixing different scalar
    variables will result in an error when the Test Plan is run.
  - An optional `variant_names` key can be provided to customise the names of the output directories
    used by each variant.
- `custom_providers`: Declarations for loading Custom Provider Definitions.
  - For full details on the structure of Custom Provider Declarations and Definitions see the
    [Custom Providers][5] page of the Framework documentation.
- `scenario`: A [Config Spec](#config-specs) for the scenario to be run.
  - For full details on the structure of a Scenario see the [Scenario][0] page of the Framework
    documentation.
- `environment`: A [Config Spec](#config-specs) for the test environment to provision.
  - For full details on the structure of an Environment see the [Environment][1] page of the
    Framework documentation.

## Config Specs

Both the `scenario` and `environment` keys map to a structure known as a _Config Spec_, which tells
RTF how to find the appropriate configuration for that aspect of the Test Plan. Config Specs support
three source types:

1. `inline`: Embed configuration directly in the Test Plan
2. `local`: Reference a local file by relative path
3. `github`: Fetch from a GitHub repository

The `local` and `github` types support optional `overrides` that merge on top of the base
configuration before the Test Plan is templated and checked.

### Inline configuration

To provide configuration inline, add an `inline` key under the relevant top level `scenario` or
`environment` key and provide the config file contents underneath.

```yaml
scenario:
  inline:
    # your scenario.yaml configuration goes here
```

```yaml
environment:
  inline:
    # your environment.yaml configuration goes here
```

### From a local file

To use a local file as a base, add a `from` key under the relevant top level `scenario` or
`environment` key, specifying the `kind` as `local` and the relative path _from the test-plan.yaml
file_ under the `relative_path` key.

To define _overrides_, add the `overrides` key at the same indentation level as `from` and then add
your override configuration under that key. The structure here is _not_ required to parse as a full
config file so you are free to only specify the keys that you need.

> For details on how overrides are applied, see [Applying Overrides](#applying-overrides) below.

```yaml
scenario:
  from:
    kind: local
    relative_path: ../my-scenario.yaml
  overrides:
    # overrides go here
```

```yaml
environment:
  from:
    kind: local
    relative_path: ../my-environment.yaml
  overrides:
    # overrides go here
```

### From a remote file in GitHub

To fetch from a GitHub repository, add a `from` key under the relevant top level `scenario` or
`environment` key, specifying the `kind` as `github` along with details for the `org`, `repo` and
`path` to the file relative to the root of the repository. It is also possible to optionally provide
a `git_ref` to pull the file from. If this is not specified then RTF will default to pulling from
the mainline branch.

> You _must_ have a valid GitHub access token with permissions to interact with your chosen
> repository exported as `GITHUB_TOKEN` in your shell environment for this method to work. See
> [here][2] for GitHub's documentation on how to create and manage access tokens.

As with using a [local file](#from-a-local-file), _overrides_ can be defined by adding the
`overrides` key at the same indentation level as `from` and then adding your override configuration
under that key. The structure here is _not_ required to parse as a full config file so you are free
to only specify the keys that you need.

> For details on how overrides are applied, see [Applying Overrides](#applying-overrides) below.

```yaml
scenario:
  from:
    kind: github
    org: my-org
    repo: my-repo
    # git_ref: testing-branch
    path: test-plans/example/my-scenario.yaml
  overrides:
    # overrides go here
```

```yaml
environment:
  from:
    kind: github
    org: my-org
    repo: my-repo
    # git_ref: testing-branch
    path: test-plans/example/my-environment.yaml
  overrides:
    # overrides go here
```

## Applying overrides

The `overrides` section of a _Config Spec_ is merged with the configuration file provided under the
`from` key as raw YAML before the configuration is parsed by RTF.

The merging strategy used is as follows:

- For maps, keys from overrides merge on top of matching keys in the base. If a key exists in both,
  RTF recursively merges the values; otherwise, RTF inserts the override key directly.
- For arrays, override values are appended to base values.
- For differing types or scalars, RTF replaces the base value with the override.

Once the overrides have been applied and the resulting config file is successfully parsed, all
arrays are then sorted and deduplicated based on an appropriate key in order to support replacing
array elements:

- For [File providers][3] the key used is `env_var`.
- For variable declarations the key used is `name`.

## A note on relative paths

There are several places within RTF config files that you be required to specify relative paths to
other files on disk. In order to help with reasoning about how to provide these paths, RTF has
strict semantics about how such relative paths are resolved.

Namely, they are _always_ resolved relative to the file the path is written in. At first glance this
may sound obvious but the important aspect to remember is around writing `overrides` in your
`test-plan.yaml`.

When a base config file is loaded by RTF, all pre-existing relative paths will be resolved relative
to the path that the config file was loaded from (this is also true when pulling remote files from
GitHub). When `overrides` are then applied from the Test Plan, any new relative paths found there
are resolved relative to the location of the `test-plan.yaml` file, _not_ the location of the base
config file.

## Working with matrices

The `matrix` key expands to multiple test plan variants via the [cartesian product][4] of its
dimensions.

For example, the following matrix:

```yaml
matrix:
  dimensions:
    a: ["foo", "bar"]
    b: [1, 2, 3]
```

Expands to six test plans (known as "variants") covering each of the possible combinations of values
for `a` and `b`.

Each variant is run individually and writes its output to its own subdirectory, named
`matrix_variant_$n` by default. The order in which variants are run is deterministic: dimensions are
ordered alphanumerically and the cartesian product is formed from the user provided ordering of
values for each dimension (as shown below).

- a=foo b=1 (matrix_variant_1)
- a=foo b=2 (matrix_variant_2)
- a=foo b=3 (matrix_variant_3)
- a=bar b=1 (matrix_variant_4)
- a=bar b=2 (matrix_variant_5)
- a=bar b=3 (matrix_variant_6)

### Naming variants

To customize output subdirectory names, specify the `matrix.variant_names` key in your test plan
with a template string for generating the variant names:

```yaml
matrix:
  variant_names: "${a}_${b}"
  dimensions:
    a: ["foo", "bar"]
    b: [1, 2, 3]
```

When doing so, the ordering for variants remains the same but the output directory names are
generated using the template provided:

- a=foo b=1 (foo_1)
- a=foo b=2 (foo_2)
- a=foo b=3 (foo_3)
- a=bar b=1 (bar_1)
- a=bar b=2 (bar_2)
- a=bar b=3 (bar_3)

The syntax used for template strings involves placing _matrix dimension names_ inside of `${}` along
with static string content in order to generate a unique name for each variant. The resulting string
is then slugified to remove whitespace and slashes.

### Combining related variables

The following initial matrix expands out to four variants covering different crate revisions for
inclusion in a Rust build as shown below:

```yaml
matrix:
  dimensions:
    federation_rev: [ "v2.6.2", "v2.7.0" ]
    compiler_rev: [ "apollo-compiler@1.28.0", "apollo-compiler@1.30.0" ]

# Produces the following variants:
# - federation_rev: v2.6.2
#   compiler_rev: apollo-compiler@1.28.0
# 
# - federation_rev: v2.6.2
#   compiler_rev: apollo-compiler@1.30.0
#
# - federation_rev: v2.7.0
#   compiler_rev: apollo-compiler@1.28.0
#
# - federation_rev: v2.7.0
#   compiler_rev: apollo-compiler@1.30.0
```

If each value of `federation_rev` only has a single matching `compiler_rev`, two of the resulting
four variants are invalid. Adding further matrix dimensions makes the problem worse, by adding more
undesirable variants.

To fix this, use `matrix.compound` to define a named _compound dimension_ that groups several
variables together. Each entry in the compound dimension must contain the same set of variables, and
only those explicit groups of values will be used to construct the resulting matrix variants:

```yaml
matrix:
  # The dimensions key must always be present, even if it is an empty map
  dimensions: {}

  compound:
    crate_revisions:
      - federation_rev: v2.6.2
        compiler_rev: apollo-compiler@1.28.0

      - federation_rev: v2.7.0
        compiler_rev: apollo-compiler@1.30.0

# Produces the following variants:
# - federation_rev: v2.6.2
#   compiler_rev: apollo-compiler@1.28.0
# 
# - federation_rev: v2.7.0
#   compiler_rev: apollo-compiler@1.30.0
```

Now only the valid revision pairs are produced. The group name (`crate_revisions` here) exists only
to identify the group within the `compound` map and allow for runtime overriding of the dimension;
it is not templated into the test plan itself. You can add further dimensions to the matrix as
normal while preserving the correct compound combinations:

```yaml
matrix:
  dimensions:
    graph_ref: [ "graph_1@prod", "graph_2@dev" ]

  compound:
    crate_revisions:
      - federation_rev: v2.6.2
        compiler_rev: apollo-compiler@1.28.0

      - federation_rev: v2.7.0
        compiler_rev: apollo-compiler@1.30.0

# Produces the following variants:
# - graph_ref: "graph_1@prod"
#   federation_rev: v2.6.2
#   compiler_rev: apollo-compiler@1.28.0
# 
# - graph_ref: "graph_1@prod"
#   federation_rev: v2.7.0
#   compiler_rev: apollo-compiler@1.30.0
#
# - graph_ref: "graph_2@dev"
#   federation_rev: v2.6.2
#   compiler_rev: apollo-compiler@1.28.0
# 
# - graph_ref: "graph_2@dev"
#   federation_rev: v2.7.0
#   compiler_rev: apollo-compiler@1.30.0
```

> When working with older test plans you may encounter the `matrix.include` key, which is a
> deprecated alias for a single compound group named `include`. `matrix.include: [...]` behaves
> exactly like `matrix.compound: { include:
> [...] }` and is still supported for backwards
> compatibility purposes, but if you see it in a test plan you're working with, you should migrate
> it to use `matrix.compound` instead.

### Combining multiple compound dimensions

Compound dimensions interact with one another in the way you would expect: with each compound
dimension contributing blocks of values to the expanded set of variants rather than individual ones
(as with a normal matrix dimension). If in the above example we found that we needed to work with
the graph name and variant as individual variables, we could express that using a second compound
dimension like so:

```yaml
matrix:
  dimensions: {}

  compound:
    graph_ref:
      - graph_name: graph_1
        variant: prod

      - graph_name: graph_2
        variant: dev

    crate_revisions:
      - federation_rev: v2.6.2
        compiler_rev: apollo-compiler@1.28.0

      - federation_rev: v2.7.0
        compiler_rev: apollo-compiler@1.30.0

# Produces the following variants:
# - graph_name: "graph_1"
#   variant: "prod"
#   federation_rev: v2.6.2
#   compiler_rev: apollo-compiler@1.28.0
# 
# - graph_name: "graph_1"
#   variant: "prod"
#   federation_rev: v2.7.0
#   compiler_rev: apollo-compiler@1.30.0
#
# - graph_name: "graph_2"
#   variant: "dev"
#   federation_rev: v2.6.2
#   compiler_rev: apollo-compiler@1.28.0
# 
# - graph_name: "graph_2"
#   variant: "dev"
#   federation_rev: v2.7.0
#   compiler_rev: apollo-compiler@1.30.0
```

Producing the same number of variants as before, but now with the ability to reference graph name
and variant directly.

## Full example

The following is a minimal "kitchen sink" example of the structure of a valid `test-plan.yaml`.

```yaml
name: example
description: An example description

variables:
  foo: "A value for foo"

matrix:
  variant_names: "${bar}_${baz}_${a}_${b}"
  dimensions:
    bar: [1, 2, 3]
    baz: [true, false]

  compound:
    extra:
      - a: 4
        b: 5
      - a: 6
        b: 7

custom_providers:
  - kind: local
    relative_path: ./providers
    using:
      my_provider: my_provider.yaml

scenario:
  inline:
    name: An inline scenario
    description: A description for the inline scenario

    variable_definitions:
      - name: foo
        description: "A description for foo"

    command:
      name: my-test.sh
      kind: relative_path
      path: scripts/my-test.sh

    env_vars:
      FOO: "{{ foo }}"

environment:
  from:
    kind: local
    relative_path: environment.yaml

  overrides:
    file_providers:
      - name: my-additional-file.txt
        env_var: ADDITIONAL_FILE
        kind: inline
        content: |
          An additional file that wasn't present in the original environment.yaml
```

[0]: ./scenarios.md
[1]: ./environments.md
[2]: https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens
[3]: ./file-providers.md
[4]: https://en.wikipedia.org/wiki/Cartesian_product
[5]: ./custom-providers.md
