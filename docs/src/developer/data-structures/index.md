# Data structures

The [rtf-config][0] crate provides a number of data structures for working with
user written YAML config files, forming an overall tree structure that is used
to execute commands in the [CLI][1].

The following pages provide a brief overview of the various data data structures
we use and how they relate to one another, starting from the leaves of the tree
and working up to the [TestPlanConfig][2] struct that acts as the root.


  [0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-config
  [1]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-cli
  [2]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/formats/test_plan.rs#L23
