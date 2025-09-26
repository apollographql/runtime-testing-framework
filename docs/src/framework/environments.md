# Environments

---

- [Top level keys](#top-level-keys)
- [Full example](#full-example)

---

As outlined in the [Test Plan][0] overview, your Environment configuration is one of the two main
pieces of configuration needed in order to run tests under RTF. Conceptually, an Environment
configuration is a pair of [Command providers][1] that are run either side of the test scenario you
want to run in order to setup and teardown the resources required for running the test.

In this page we will cover the available keys within an Environment and outline the structure and
semantics of each. For more detailed information on specific aspects of the framework please see the
relevant pages under the [Framework][2] section of the documentation.

> An example of a valid `environment.yaml` is provided in the [Full example](#full-example) section
> below.

## Top level keys

- `name`: The name for this Environment configuration.
  - Uniqueness is not enforced by the `rtf` CLI but it is worthwhile ensuring that the environments
    you write each have unique names that can be used to distinguish them.
- `description`: A brief, human readable description of the behaviour of the Environment.
  - If there are any pre-requesites to running this Environment it is best to call them out here
    rather than in comments or other files (such as a README).
- `values`: Declarations of the templating values supported by this Environment.
  - Value declarations require specifying both the value name and a short description of how the
    value is used.
  - Value declarations also support an optional `default` field where you can specify a default
    scalar value to use if none is provided within the [Test Plan][0].
  - If the same value name is defined in both the Environment and Scenario used by a given Test Plan
    but with different defaults, each config file will fall back to its own default.
- `setup`: A [Command Provider][1] that defines how the environment should be set up before the
  scenario is run.
- `teardown`: A [Command Provider][1] that defines how the environment should be torn down after the
  scenario is run.

> We are aware that having the same key name for value declarations in the Scenario / Environment
> config files and the value _definitions_ in the Test plan config file can be confusing. We are
> considering renaming one or both of the keys to make the distinction clearer.
>
> Please see [RR-352][3] for more details.

## Full example

The following is a minimal "kitchen sink" example of the structure of a valid `environment.yaml`.

```yaml
name: example
description: An example description

values:
  - name: my_value
    description: "A description for my value"
    default: "foo"

setup:
  command:
    name: my-setup-command.sh
    kind: relative_path
    path: scripts/setup.sh

  env_vars:
    MY_VALUE: "{{ my_value }}"

teardown:
  command:
    name: my-teardown-command.sh
    kind: relative_path
    path: scripts/teardown.sh

  file_providers:
    - name: my-file.txt
      env_var: MY_FILE
      kind: inline
      content: My inline content
```

[0]: ./test-plans.md
[1]: ./command-providers.md
[2]: ./index.md
[3]: https://apollographql.atlassian.net/browse/RR-352
