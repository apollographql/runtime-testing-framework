# File providers

[File providers][0] are the primary way that rtf exposes useful functionality to users. Each
provider is defined as a struct that can be parsed from a YAML snippet within a larger config file,
with fields that serve as inputs to business logic from the [rtf-core][1] crate. Running a file
provider will generate one or more resources within a user designated output directory which can
then be referenced by the commands being run as part of a test plan.

Within config files, file providers are always wrapped in a [NamedFileProvider][2] that adds
filename and environment variable fields along with logic for running the provider and outputting
the new resources in the correct location on the filesystem.

[0]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/providers/file/mod.rs#L224
[1]: https://github.com/apollographql/runtime-testing-framework/tree/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-core
[2]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/providers/file/mod.rs#L201
