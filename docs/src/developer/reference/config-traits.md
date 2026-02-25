<!-- diataxis-type: reference -->

# Traits for working with config structs

We have a set of five traits that are used to provide the shared behaviour needed to parse, validate
and execute rtf config files:

- [Template][0]: used to locate and resolve templatable [fields][1] within larger config data
  structures.
- [Check][2]: used to run side-effect free static analysis checks on config files before they are
  executed.
- [AsUtf8FileContent][3]: used by providers that return a single string data file to define how
  their content is generated.
- [ResolveFileContent][4]: used by providers that return multiple files to define how their content
  is generated.
- [ResolveAndWrite][5]: used by providers to define how they generate their content and write it out
  to the user specified output directory.

All [File Providers][6] are required to implement the `Template` and `Check` traits and _either_ the
`AsUtf8FileContent` trait, the `ResolveFileContent` trait, or the `ResolveAndWrite` trait. You
should prefer implementing `AsUtf8FileContent` where possible as it will handle some of the
boilerplate logic for you.

> This is possible whenever your provider is writing out a single utf8 encoded file as its output.

[0]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/templating.rs#L45
[1]: ./data-structures/fields.md
[2]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/checks.rs#L47
[3]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/providers/file/mod.rs#L90
[4]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/providers/file/mod.rs#L92
[5]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/providers/file/mod.rs#L118
[6]: ./data-structures/file-providers.md
