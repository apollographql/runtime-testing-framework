# Summary

[The Apollo Runtime Testing Framework](index.md)

# User Documentation

- [Understanding RTF](explanation/overview.md)

- [Tutorials](tutorials/index.md)
  - [Hello, world!](tutorials/hello-world.md)
  - [Writing test plans](tutorials/test-plans/index.md)
    - [Writing a test plan](tutorials/test-plans/writing-a-test-plan.md)
    - [Writing a command](tutorials/test-plans/writing-a-command.md)
    - [Writing a scenario](tutorials/test-plans/writing-a-scenario.md)
    - [Writing an environment](tutorials/test-plans/writing-an-environment.md)
    - [Using file providers](tutorials/test-plans/using-file-providers.md)
  - [Writing custom providers](tutorials/custom-providers/index.md)
    - [Writing a custom provider definition](tutorials/custom-providers/writing-a-custom-provider-definition.md)
    - [Using a custom provider](tutorials/custom-providers/using-a-custom-provider.md)

- [How-to guides](howto/cookbook.md)
  - [Troubleshooting]()

- [Reference](reference/index.md)
  - [Framework](reference/framework/index.md)
    - [Test Plans](reference/framework/test-plans.md)
    - [Environments](reference/framework/environments.md)
    - [Scenarios](reference/framework/scenarios.md)
    - [Command providers](reference/framework/command-providers.md)
    - [File providers](reference/framework/file-providers.md)
    - [Custom providers](reference/framework/custom-providers.md)
  - [CLI reference](reference/cli-help.md)
  - [Glossary](reference/glossary.md)

# Developer Documentation

- [Style guide](developer/style-guide.md)
- [Concepts and Architecture](developer/concepts-and-architecture.md)
- [Error handling](developer/error-handling.md)
- [Logging](developer/logging.md)
- [Testing](developer/testing/index.md)
  - [rtf-cli](developer/testing/rtf-cli.md)
  - [rtf-config](developer/testing/rtf-config.md)
  - [rtf-core](developer/testing/rtf-core.md)
  - [rtf-derive](developer/testing/rtf-derive.md)
- [PR checks](developer/pr-checks.md)
- [Parsing config files](developer/parsing-config-files.md)
- [Data structures](developer/data-structures/index.md)
  - [Templating fields](developer/data-structures/fields.md)
  - [File providers](developer/data-structures/file-providers.md)
  - [Command providers](developer/data-structures/command-providers.md)
  - [Config file formats](developer/data-structures/config-files.md)
- [Traits for working with config structs](developer/config-traits.md)
- [Use of IO in providers](developer/context.md)
- [CLI subcommand design](developer/cli-subcommand-design/index.md)
  - [Plumbing vs porcelain](developer/cli-subcommand-design/plumbing-vs-porcelain.md)
  - [No built-in magic](developer/cli-subcommand-design/no-built-in-magic.md)
  - [Global flags](developer/cli-subcommand-design/global-flags.md)
