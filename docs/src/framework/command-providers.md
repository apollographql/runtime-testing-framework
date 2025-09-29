# Command providers

---

- [The command section](#the-command-section)
  - [Inline scripts](#inline-scripts)
  - [Relative paths](#relative-paths)
  - [Required commands](#required-commands)
- [The env vars section](#the-env-vars-section)
- [The file providers section](#the-file-providers-section)
  - [A note on providers as resources](#a-note-on-providers-as-resources)

---

_Command Providers_ are the core executable element making up RTF Test Plans. They allow you, the
user, to specify how a given command should be run and what resources it needs in order to do so.
Both the [Environment][0] and [Scenario][1] configuration files are simply ways of defining Command
Providers with known semantics for RTF to execute alongside templating values that can be used to
customise how that command is run.

The configuration for a Command Provider consists of three top level sections:

1. The command itself.
2. Environment variables that should be set before the command is run.
3. A set of [File Providers][2] that should be run and made available before the command is run.

The details of each section are outlined below.

> For full examples of what command sections look like inside of RTF test plans, please see the
> examples found in the [Environment][3] and [Scenario][4] pages.

## The command section

You can define your command in two ways:

1. As an inline script that will be written to disk and made executable.
2. As a relative path to an existing executable script.

Both strategies will result in the appropriate utf-8 encoded text file being written to disk and
made executable before being executed as a subprocess by RTF. As such, you _must_ include an
appropriate [shebang][5] line at the top of your script in order for it to run correctly.

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

It is also possible to instead mark that the command is _required_ as an override specified in the
user's Test Plan. This is primarily used as part of a Scenario or Environment configuration where
you want to set up supporting resources and data around an arbitrary user specified command.

In each of the three options you will need to provide the `name` of the command as well as a `kind`
the specifies which strategy you want to use to define your command.

### Inline scripts

To provide your command as an inline script simply specify the kind as `inline` and provide your
script contents under the `content` key.

It is worth familiarising yourself with YAML's support for multiline strings in order to ensure that
you have the correct indentation within your scripts. [This site][6] serves as a nice minimal
reference for the relevant YAML syntax.

```yaml
command:
  name: my-shell-script.sh
  kind: inline
  content: |
    #!/usr/bin/env sh
    echo "Hello from RTF"
```

### Relative paths

To use a pre-existing script as your command simply specify the kind as `relative_path` and provide
the relative path to your script under the `path` key. (See [here][7] for details on how relative
paths are handled by RTF).

```yaml
command:
  name: my-shell-script.sh
  kind: relative_path
  path: ../scripts/my-shell-script.sh
```

### Required commands

To mark a command as required, but not specified by default, you can use the `required` kind which
supports providing an accompanying `message` to inform the user of how they should define their own
command. If the user fails to provide an override for the command in their [Test Plan][8], RTF will
error at the templating stage of execution and print your error message as the reason for the
failure.

```yaml
command:
  name: my-command
  kind: required
  message: "This command must be provided in the test plan explicitly"
```

## The env vars section

Environment variables are defined simply as key value pairs under the `env_vars` key. Values may be
templated using the `"{{ my_value }}"` syntax using any scalar value (not just strings). The
environment variables explicitly defined under this key will be merged with the environment
available to RTF itself before your command is executed.

```yaml
# values:
#   my_string_env_var: "bar"
#   my_integer_env_var: 42

env_vars:
  FOO: "foo"
  BAR: "{{ my_string_env_var }}"
  BAZ: "{{ my_integer_env_var }}"
```

## The file providers section

Under the `file_providers` key you may specify any number of _File Providers_ as resources that will
be made available to your command prior to execution. The [File Providers][2] page covers the
specifics of each of the built-in file providers within RTF so here we will focus instead on their
shared structure and semantics.

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

### A note on providers as resources

You must ensure that _every_ resource that you need to make available to your _Command Provider_ is
specified via a _File Provider_. RTF will only guarantee that the absolute paths it places in
provider environment variables are correct. You must not attempt to construct relative paths between
resources or from command scripts themselves as RTF can (and will) break such relative path
relationships without warning.

To help with managing the resources you need for your commands, RTF internally caches and reuses
file providers that share identical keys, so you are free to duplicate providers between different
_Command Providers_. Allowing you to share resources between different _Command Providers_ without
worrying about the providers running multiple times.

[0]: ./environments.md
[1]: ./scenarios.md
[2]: ./file-providers.md
[3]: ./environments.md#full-example
[4]: ./scenarios.md#full-example
[5]: https://en.wikipedia.org/wiki/Shebang_%28Unix%29
[6]: https://yaml-multiline.info/
[7]: ./test-plans.md#a-note-on-relative-paths
[8]: ./test-plans.md
