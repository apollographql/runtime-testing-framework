<!-- diataxis-type: reference -->

# Code style and idioms

This page contains a summary of the common code style elements and idioms we follow within this
codebase and notes on why we feel they are important. While we aim to keep this list up to date, it
should not be assumed to be exhaustive and should be treated as a living document to be updated as
we identify missing items or establish new ones.

As a staring point, we follow the official [Rust style guide][0] but there are a few places where we
deviate from the default advice for practical reasons.

As a general rule, we aim to optimise for being able to quickly and accurately read / navigate the
code we write without relying on IDE support or external tooling. This is so we are able to work
with the source code itself in contexts where the use of such tooling is not possible or would
otherwise require additional setup (e.g. diffs, GitHub web UI, GitHub issue). This may sound
pedantic, but it is based on years of experience of needing to be able to work with a codebase in
this way in time sensitive situations such as production incidents and release windows. When
additional tooling is available (which _is_ most of the time), none of the style and idiom
requirements here make their use more difficult. So we view it as being "worth it" to plan for the
times when we need the code to be clear and easy to understand without them.

## Imports

Rust allows organising [use statements][1] in a number of different ways, with `rustfmt` and IDE
tools such as `rust-analyzer` making "best effort" attempts to match the existing style within any
given file. Use of nightly `rustfmt` can provide more control over this but complicates the local
developer setup and is an easy thing to miss / forget, leading to failed CI checks and churn between
different setups.

As such, we use the following convention which all developer setups should work with automatically
without explicit configuration:

- All use statements for a given file are placed at the top of the file in a single block
- We always use nested imports rather than "line per import / module"
- `use super::` is only permitted within test modules to bring in the code under test

**Exceptions**

- Imports specific to a single function that would otherwise pollute the file-scope namespace may be
  placed at the start of that function body
  - e.g. use statements for enum variants, external API structs for building request payloads

**Reasoning**

- Placing use statements in a single block allows formatting tools to consistently organise imports.
  When using multiple blocks, the user defined blocks will typically be preserved even if they are
  not grouped according to the official style guide.
- Using nested imports reduces line noise at the top of the file and makes it easier to read the
  import list.

**Examples**

```rust
// Correct
use axum::{Router, routing::{get, post}};
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};

// Correct - bringing in enum variants within a function to reduce line noise
fn validate(payload: &SetStatusPayload) -> Result<()> {
    use SharedStatus::*;

    match (payload.status, payload.exit_code) {
        (Failed, None) => return Err(Error::MissingExitCode),
        (Failed, Some(0)) => return Err(Error::InvalidFailedExitCode),
        (Failed, Some(_)) => (),
        (Successful, Some(0)) => (),
        (_, Some(code)) => {
            return Err(Error::InvalidExitCode {
                status: payload.status,
                code,
            });
        }
        _ => (),
    };

    Ok(())
}

// Incorrect - multiple blocks
use axum::{Router, routing::{get, post}};

use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};

// Incorrect - multiple use statements for the same crate
use axum::Router;
use axum::routing::{get, post};
use tokio::net::TcpListener;
use tokio::sync::mpsc::unbounded_channel;

// Incorrect - top level use super;
use super::Result;
```

## Function returns

There must always be a blank line before implicit function returns.

**Exceptions**

- Single line functions do not require a blank line before the return as there is nothing to isolate
  the return value from.

**Reasoning**

- Having a blank line before function returns allows the reader's eye to quickly jump to value being
  returned and isolates it from the rest of the function so it can be easily read.

**Examples**

```rust
// Correct
fn prepare_resolution(cfg: &Config, payload: TriggerPayload) -> Result<(RepContext, RepTestPlan)> {
    let TriggerPayload {
        mut test_plan,
        relative_files,
        custom_providers,
    } = payload;

    let ctx = RepContext::new(cfg, relative_files, custom_providers);
    test_plan
        .check_templating_will_work(&HashMap::new(), &ctx)
        .map_err(ResolverError::TemplatingCheck)?;

    Ok((ctx, test_plan))
}

// Incorrect
fn prepare_resolution(cfg: &Config, payload: TriggerPayload) -> Result<(RepContext, RepTestPlan)> {
    let TriggerPayload {
        mut test_plan,
        relative_files,
        custom_providers,
    } = payload;

    let ctx = RepContext::new(cfg, relative_files, custom_providers);
    test_plan
        .check_templating_will_work(&HashMap::new(), &ctx)
        .map_err(ResolverError::TemplatingCheck)?;
    Ok((ctx, test_plan))
}
```

## Fully qualified paths

`use` statements should bring in the leaf item they are targeting rather than modules. Inline fully
qualified paths for types (e.g. `std::collections::HashMap`) should be avoided wherever possible.

> **NOTE**: Both Claude and Rust Analyzer have an annoying habit of using fully qualified paths.
> Claude does this somewhat randomly in the code it writes, while Rust Analyzer will tend to add
> imports when tab completing individual items, but not when inserting snippets (e.g. for trait
> methods).

When namespacing is required in order to resolving name collisions there are two accepted solutions:

1. Alias the import:
2. Import the module name for use as a single element path prefix:

**Exceptions**

- Free functions from the `io`, `fmt`, `fs` Standard library modules should always be used via their
  module name. Types may be imported directly where naming is unambiguous.
- When working with `Result` and `Error` types, the primary types for the file should be imported
  and used directly, with results and errors from other modules being qualified by their module
  name.
- `thiserror::Error` and `anyhow::Error` must always be fully qualified

**Reasoning**

- Fully qualified paths add significant line noise and result in larger diffs when code is
  refactored.
- Inline fully qualified paths also obscure the external dependencies of the file they are used in,
  as they can often end up removing an entire module or crate from the list of `use` statements at
  the top of the file.

**Examples**

```rust
// Correct
use rep_orchestrator_shared::status::Status as SharedStatus;
use std::io

fn example_1() -> SharedStatus { ... }
fn example_2() -> io::Result<()> { ... }


// Incorrect - ambiguous qualifier
use rep_orchestrator_shared::status;

// Incorrect - inline fully qualified paths
fn example_2() -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "foo"))
}
```

## IO handles in function arguments

A lot of the code we write in this repo makes use of API clients and / or context structs for
controlling interaction with IO related logic. Outside of top level code such as request handlers
and CLI commands, these structs must always be obtained through function parameters rather than
being constructed within the body of the function where they are used.

To aid in reading function signatures we always place these "IO handles" at the end of the function
arguments list.

**Exceptions**

- None

**Reasoning**

- Having a consistent position for these arguments at the end of the argument list allows the reader
  to quickly identify the non-IO handle related arguments that affect the logic of the function.
- Injecting IO logic via handles allows us to mock out IO behaviour in tests cleanly and without
  resorting to macros or external mocking crates.

**Examples**

```rust
// Correct
async fn init_test_run(name: &str, conn: &mut PgConnection) -> Result<TestRun> { ... }

impl Check for NamedFileProvider {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> { .. }
}

// Incorrect
async fn init_test_execution(
    name: &str,
    conn: &mut PgConnection,
    test_run_id: i32
) -> Result<TestExecution> { ... }
```

[0]: https://doc.rust-lang.org/beta/style-guide/index.html
[1]: https://doc.rust-lang.org/beta/style-guide/items.html#imports-use-statements
