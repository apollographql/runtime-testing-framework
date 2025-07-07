# Command providers

[Command providers][0] support running a subset of [file providers][1] in order
to obtain an executable file that can be used as one of the environment setup,
environment teardown or scenario commands. Within config files they are wrapped
in a [CommandSpec][2] which allows the user to provide command line arguments.

Each command section supports specifying environment variables and file providers
that will be made available when the command executes.


  [0]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/providers/command.rs#L363
  [1]: ./file-providers.md
  [2]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/providers/command.rs#L300
