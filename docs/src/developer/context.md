# Use of IO in providers

All IO that is run as part of provider logic _must_ be run using the [context][0] argument that is
passed to methods. This allows the CLI logic to control how providers are run as well as allowing us
to swap out real IO for mock implementations within tests.

If you are writing a new provider and need to perform IO that is not currently possible via the
existing [ResolutionContext][1] methods, you will need to first expose the functionality through
that trait and provide a default "live" implementation for the concrete [Context][2] struct that is
used by the CLI.

[0]: https://github.com/apollographql/runtime-testing-framework/blob/main/crates/rtf-config/src/context.rs
[1]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/context.rs#L35
[2]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/context.rs#L157
