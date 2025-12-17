<!-- diataxis-type: reference -->

# Config file formats

The [rtf-config][0] crate provides parsers for the three config files used by RTF: `TestPlanConfig`,
`EnvironmentConfig` and `ScenarioConfig`.

## Scenario Config

The simplest of the three config file formats is the scenario config which simply provides a way for
the user to pair [templating][1] variables with a [command][2].

## Environment Config

The environment config file defines a pair of command sections: one for setting up the environment
before the test is run and another for tearing it down after the test is complete. The setup section
also requires the user to declare the structure of additional JSON variables that will be provided
by the setup command for it to pass data along to the scenario and teardown sections.

## Test plan config

The test plan config file is where the user defines the variables they wish to use for templating
along with the scenario and environment configurations, either provided inline or as references to
external files.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config
[1]: ./fields.md
[2]: ./command-providers.md
