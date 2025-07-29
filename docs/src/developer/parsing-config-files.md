# Parsing config files

The [rtf-config][0] crate provides parsers for the [three config files][1] used by RTF, along with
parsers for the [various][2] [provider][3] fragments that are used to expose the rest of the
framework to users through those config files.

The parsers for `TestPlanConfig`, `EnvironmentConfig` and `ScenarioConfig` live in the [formats][4]
module. Each exposes a similar API for how they are parsed from YAML data files, templated,
validated and executed. The shared behaviour used to do this is defined in a [set of traits][5] that
provide tree-walk based methods for traversing the nested data structures obtained from parsing user
written config files.

## Config resolution & execution

The full resolution and execution of a test plan has the following flow:

1. Load and parse the user specified `TestPlan` file.
2. Locate and load any required `Secenario` and `Environment` files defined in `from` directives as
   raw YAML. If the test plan defines overrides for either section then deep merge before parsing
   into concrete structs.
3. Check that the test plan contains all of the required values for templating to be possible. If it
   doesn't then early exit reporting the missing values.
4. Template the environment _setup_ section before running static analysis checks.
5. If all checks pass, run the setup command and use the provided output to finish templating the
   _scenario_ and environment _teardown_ sections.
6. Run static analysis checks for both sections. If any checks fail for either section then early
   exit.
7. Run the test scenario.
8. Run the environment teardown.

> For each of these stages we check for any errors or inconsistencies and report all known errors to
> the user as a batch operation. Each of the config file structs provides an API for running partial
> checks and templating so that end users are able to efficiently debug and iterate on their config
> files.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-config
[1]: ./data-structures/config-files.md
[2]: ./data-structures/file-providers.md
[3]: ./data-structures/command-providers.md
[4]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-config/src/formats
[5]: ./config-traits.md
