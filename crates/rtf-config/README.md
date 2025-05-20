# Runtime Testing Framework - Config

Config file parsing for the Apollo Runtime Testing Framework.


## Config resolution

1.  Load the TestPlan
      -> At this point we should have either valid inline config or "pointers" to base files
2.  If we have base files, load them (Scenario & Environment) as serde_yaml::Mappings
      -> otherwise we have inline raw files as mappings
3.  Merge overrides mappings with their counterpart base files
4.  Parse the base file mappings into their raw form
5.  Try to resolve both base files (here we need to resolve setup and teardown independently)
6.  If Environment.setup has missing values, or if either of Scenario or Environment.teardown
    have missing values that would not be provided by Environment.setup, we error out
7.  Run the Environment setup
8.  Finish resolving the Scenario and Environment teardown
9.  Run the Scenario
10. Extract results
11. Run the Teardown
