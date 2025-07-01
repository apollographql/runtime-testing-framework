# Runtime Testing Framework - Config

Config file parsing for the Apollo Runtime Testing Framework.


## Design

This crate provides parsers for the three related config files used in RTF,
along with parsers for the various `provider` fragments that are used to
expose the rest of the framework to users through those config files.

The parsers for `TestPlanConfig`, `EnvironmentConfig` and `ScenarioConfig`
live in the `formats` module and all expose a similar API for how they
operate. Each config file is templated in three stages:

  1. Loading and parsing a given YAML file into a "raw" form that is allowed
     to contain limited templating via Helm-style scalar values.
  2. Applying a provided scalar values map to fill in any templated fields.
  3. Running all providers to generate their requested data.

For each of these stages we check for any errors or inconsistencies and report
all known errors to the user as a batch operation. Each of the config file
structs provides an API for running partial checks and templating so that
end users are able to efficiently debug and iterate on their config files.


## Config resolution & execution

The full resolution and execution of a test plan has the following flow. As
mentioned above, each step has implicit "check and report errors" behaviour
as part of its execution:

1. Load and template the `TestPlan` file.
2. Load the `Scenario` and `Environment` files as raw YAML.
3. Merge any overrides from the `TestPlan` into the `Scenario` and `Environment` files.
4. Template the `Environment` setup section and check that the output it provides is sufficient
   to finish templating both the `Scenario` and the rest of the `Environment` configuration.
5. Run the environment setup and finish templating the `Scenario` and `Environment` teardown.
6. Run the `Scenario` and extract test output.
7. Run the `Environment` teardown.
