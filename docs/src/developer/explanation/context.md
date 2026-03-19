<!-- diataxis-type: explanation -->

# Use of IO in providers

All IO that is run as part of provider logic _must_ be run using the [context][0] argument that is
passed to methods. This allows the caller to control how providers are run as well as allowing us to
swap out real IO for mock implementations within tests.

If you are writing a new provider and need to perform IO that is not currently possible via the
existing [ResolutionContext][1] methods, you will need to first expose the functionality through
that trait and provide a default "live" implementation for the concrete [Context][2] struct that is
used by the CLI.

## Concrete implementations

### `Context` (RTF CLI)

[`Context`][2] is the concrete implementation used by the RTF CLI. It performs real IO: reading
files from the filesystem, executing commands, making HTTP requests to GraphOS and GitHub. When the
CLI runs `rtf run`, it constructs a `Context` and passes it through to all provider logic.

### `RepContext` (REP)

[`RepContext`][3] is the concrete implementation used by `rep-orchestrator`. It wraps an inner
`Context` but overrides the file IO behaviour: instead of reading from the filesystem, it reads from
an in-memory map of file contents that was pre-bundled by `rtf rep prepare` into the `RepPayload`.

This is a deliberate design constraint. REP is a server — it has no access to the filesystem paths
that existed on the developer's machine when the test plan was prepared. The `RepPayload` carries
everything the server needs, and `RepContext` enforces that only that content is accessible.

As a consequence, any `ResolutionContext` methods that would touch the filesystem directly are
implemented as panics:

```rust
fn read_path_to_string(&self, _path: impl AsRef<Path>) -> io::Result<String> {
    panic!("attempt to read path to string")
}
```

This is intentional. We should not be attempting to use the filesystem in the ways these methods
expose during REP execution. Attempting to use the filesystem via those methods is a bug in the
architecture, not something to handle gracefully.

HTTP and API client operations (`platform_client`, `github_client`, `http_client`) are delegated to
the inner `Context`.

[0]: https://github.com/apollographql/runtime-testing-framework/blob/main/crates/rtf-config/src/context.rs
[1]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/context.rs#L35
[2]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/context.rs#L157
[3]: https://github.com/apollographql/runtime-testing-framework/blob/main/crates/rep-orchestrator/src/context.rs
