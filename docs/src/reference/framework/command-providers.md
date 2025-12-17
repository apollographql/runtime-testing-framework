<!-- diataxis-type: reference -->

# Command providers

Command Providers are the core executable element of RTF Test Plans. They specify how a command runs
and what resources it needs. [Environment][0] and [Scenario][1] configurations are Command Providers
with defined execution semantics.

The configuration for a Command Provider consists of three top level sections:

1. The command itself.
2. Environment variables that should be set before the command is run.
3. A set of [File Providers][2] that should be run and made available before the command is run.

The details of each section are outlined below.

> Full examples appear in the [Environment][3] and [Scenario][4] pages.

## The command section

Commands can be defined in two ways:

1. As an inline script that will be written to disk and made executable.
2. As a relative path to an existing executable script.

Both strategies result in the appropriate UTF-8 encoded text file being written to disk and made
executable before being executed as a subprocess by RTF. Scripts _must_ include an appropriate
[shebang][5] line at the top in order to run correctly.

> ⚠️ **At this time, RTF does not support directly executing binaries via command providers**
>
> If the command you wish to execute is simply a pre-existing binary, you should provide an **inline
> wrapper script** that ensures that the binary in question is available on the PATH before calling
> the binary with the appropriate arguments:
>
> ```bash
> #!/usr/bin/env sh
> if ! which "$YOUR_BINARY" > /dev/null 2>&1; then
>   echo "ERROR: $YOUR_BINARY is not available on the path"
>   exit 1
> fi
>
> "$YOUR_BINARY" # arguments to the binary
> ```

It is also possible to mark that the command is _required_ as an override specified in the Test
Plan. This is primarily used as part of a Scenario or Environment configuration to set up supporting
resources and data around an arbitrary user-specified command.

In each of the three options, the `name` of the command must be provided along with a `kind` that
specifies which strategy is used to define the command.

### Inline scripts

To provide a command as an inline script, specify the kind as `inline` and provide the script
contents under the `content` key.

For multiline script content, see [YAML multiline string syntax][6].

```yaml
command:
  name: my-shell-script.sh
  kind: inline
  content: |
    #!/usr/bin/env sh
    echo "Hello from RTF"
```

### Relative paths

To use a pre-existing script, specify the kind as `relative_path` and provide the relative path to
the script under the `path` key. (See [here][7] for details on how relative paths are handled by
RTF).

```yaml
command:
  name: my-shell-script.sh
  kind: relative_path
  path: ../scripts/my-shell-script.sh
```

### Required commands

To mark a command as required but not specified by default, use the `required` kind with an
accompanying `message` to inform users how to define their own command. If an override for the
command is not provided in the [Test Plan][8], RTF will error at the templating stage of execution
and print the error message as the reason for the failure.

```yaml
command:
  name: my-command
  kind: required
  message: "This command must be provided in the test plan explicitly"
```

## The env vars section

Environment variables are defined simply as key value pairs under the `env_vars` key. Variables may
be templated using the `"{{ my_variable }}"` syntax using any scalar value (not just strings). The
environment variables explicitly defined under this key will be merged with the environment
available to RTF itself before your command is executed.

```yaml
# variables:
#   my_string_env_var: "bar"
#   my_integer_env_var: 42

env_vars:
  FOO: "foo"
  BAR: "{{ my_string_env_var }}"
  BAZ: "{{ my_integer_env_var }}"
```

## The file providers section

The `file_providers` key accepts any number of File Providers as resources made available before
command execution. See [File Providers][2] for provider-specific details. This section covers shared
structure and semantics.

When defined under a _Command Provider_ the following shared keys are added to the variant specific
keys defined by each file provider:

- `name`: the name for this specific provider that will be used to report any errors encountered
  during execution.
- `env_var`: the environment variable to set containing the absolute path to the resources created
  by the provider.
  - Depending on the provider this may either be a single file or a directory of files.
  - See the relevant documentation for each provider to learn more about the structure of their
    outputs.
- `kind`: the variant "kind" which then sets the expected keys required for the rest of the provider
  block.

```yaml
file_providers:
  - name: my-inline-file.txt
    env_var: MY_INLINE_FILE
    content: "some file content"

  - name: supergraph.graphql
    env_var: SUPERGRAPH_SCHEMA
    kind: graphos_supergraph
    graph_ref: "foo@bar"
    with_subgraph_overrides: docker
```

> **Note**: Every resource needed by a Command Provider must be specified via a File Provider. RTF
> only guarantees paths set in provider environment variables. Do not construct relative paths
> between resources or from command scripts.

RTF internally caches and reuses file providers that share identical keys, so you are free to
duplicate providers between different Command Providers. This allows sharing resources without
providers running multiple times.

[0]: ./environments.md
[1]: ./scenarios.md
[2]: ./file-providers.md
[3]: ./environments.md#full-example
[4]: ./scenarios.md#full-example
[5]: https://en.wikipedia.org/wiki/Shebang_%28Unix%29
[6]: https://yaml-multiline.info/
[7]: ./test-plans.md#a-note-on-relative-paths
[8]: ./test-plans.md
