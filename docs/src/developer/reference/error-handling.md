<!-- diataxis-type: reference -->

# Error handling

Wherever possible we aim to provide users with as much debugging information as possible when rtf
encounters an problem that prevents continuing execution (as opposed to early exiting with the first
error encountered). To support this, the [error][0] module in the `rtf-config` crate provides a
generic API for gathering and reporting multiple errors to the user.

The general idea is to create an [ErrorBuilder][1] whenever you are running batch logic such as
templating, validation and static analysis checks. So long as there are no side effects to the logic
being run, you should try to make use of an error builder to collect related errors where possible.

See the implementation of [try_template][2] for the `CommandSection` struct for an example of what
this looks like in practice.

[0]: https://github.com/apollographql/runtime-testing-framework/blob/main/crates/rtf-config/src/error.rs
[1]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/error.rs#L94
[2]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/providers/command.rs#L181
