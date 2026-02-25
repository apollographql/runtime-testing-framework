<!-- diataxis-type: reference -->

# Config file formats

The [rtf-config][0] crate provides parsers for the three config files used by RTF: `TestPlanConfig`,
`EnvironmentConfig` and `ScenarioConfig`.

## Scenario config

The simplest of the three config file formats is the scenario config which simply provides a way for
the user to pair [templating][1] variables with a [command][2].

## Environment config

The environment config file defines how RTF sets up and tears down the test environment. There are
two execution models: **docker compose** and **script**.

### Docker compose environment

A docker compose environment uses a list of docker compose files to bring the environment up and
down using `docker compose`. Additional environment variables can be passed to `docker compose up`,
and any extra files the compose stack depends on can be declared in a list of File Providers, which
exposes them to the stack as environment variables.

### Script environment

A script environment defines a pair of command sections: one for setting up the environment before
the test is run and another for tearing it down after the test is complete.

## Test plan config

The test plan config file is where the user defines the variables they wish to use for templating
along with the scenario and environment configurations, either provided inline or as references to
external files.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config
[1]: ./fields.md
[2]: ./command-providers.md
