# Summary

[The Apollo Runtime Testing Framework](index.md)

# User Documentation

- [Overview](overview.md)

- [Getting started](guides/index.md)
  - [Hello, world!](guides/hello-world.md)

- [The framework](framework/index.md)
  - [Test Plans](framework/test-plans.md)
  - [Environments](framework/environments.md)
  - [Scenarios](framework/scenarios.md)
  - [Command providers](framework/command-providers.md)
  - [File providers](framework/file-providers.md)
  - [Custom providers](framework/custom-providers.md)

- [Writing test plans](guides/test-plans/index.md)
  - [Writing a new test plan](guides/test-plans/writing-a-test-plan.md)
  - [Writing a command](guides/test-plans/writing-a-command.md)
  - [Writing a new scenario](guides/test-plans/writing-a-scenario.md)
  - [Writing a new environment](guides/test-plans/writing-an-environment.md)
  - [Using file providers](guides/test-plans/using-file-providers.md)

- [Troubleshooting]()

- [Cookbook](cookbook.md)

- [Command Line Help](cli-help.md)

- [Glossary](glossary.md)

# Developer Documentation

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
