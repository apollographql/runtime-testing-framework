<!-- diataxis-type: reference -->

# Logging Reference

This page covers when and how to use different log levels in the Runtime Testing Framework. The
project uses the [`tracing`](https://tracing.rs/) crate for structured logging.

For the design rationale behind logging choices, see
[Logging Philosophy](../explanation/logging-philosophy.md).

## Log levels overview

The Runtime Testing Framework uses five log levels, from most to least verbose:

| Level   | Verbosity Flag | Purpose                                        | Audience                                 |
| ------- | -------------- | ---------------------------------------------- | ---------------------------------------- |
| `TRACE` | `-vvv`         | Extremely detailed execution flow              | Framework developers debugging           |
| `DEBUG` | `-vv`          | Detailed diagnostic information                | Developers and end users troubleshooting |
| `INFO`  | `-v`           | High-level progress indicators                 | End users                                |
| `WARN`  | (default)      | Potentially problematic situations             | End users                                |
| `ERROR` | (always shown) | Error conditions that prevent normal operation | End users                                |

## When to use each level

### ERROR level

Use `error!` for conditions that prevent the application from continuing normal operation or cause
significant functionality to fail.

**Examples:**

- Failed to initialize logging system
- Missing required configuration
- Network requests that fail completely
- File I/O errors that prevent core functionality

**Avoid using ERROR for:**

- Temporary failures that will be retried
- Optional operations that fail
- Expected validation failures

### WARN level

Use `warn!` for situations that are unusual or potentially problematic but don't prevent the
operation from continuing. WARN is the default log level shown to end users, so these messages
should provide useful context about what users should look for if errors occur later in the process.

**Examples:**

- Deprecated features being used
- Non-critical parsing failures
- Missing optional data
- Retrying failed operations

### INFO level

Use `info!` for high-level progress indicators that help users understand what the application is
doing.

**Examples:**

- Major phase transitions (loading, executing, completing)
- Processing of user-provided inputs
- Successful completion of significant operations
- Progress indicators for long-running operations

### DEBUG level

Use `debug!` for detailed diagnostic information that helps developers understand the internal
workings and troubleshoot issues.

**Examples:**

- File system operations (creating directories, writing files)
- Detailed processing steps
- Configuration values being used
- Internal state changes

### TRACE level

Use `trace!` for extremely detailed execution flow information, typically for debugging complex
logic or data flow issues.

> **Warning**: Do not log potentially sensitive data in the `trace` logs. Assume all data supplied
> by the user or from sources specified by the user could contain sensitive data. For this reason,
> we do not log raw API responses or the contents of a file.

**Examples:**

- The URL of an API being called. Do not log the response or parameters used to call the API since
  these could be sensitive.
- Fine-grained execution flow
- Performance-sensitive debugging information

## Logging best practices

### Use structured fields

Take advantage of
[tracing's structured logging capabilities](https://docs.rs/tracing/latest/tracing/#recording-fields)
by including relevant context as fields:

```rust
// Good: structured fields for easy filtering and analysis
info!(%graph_id, %variant, "pulling supergraph details");
warn!(error = %e, "failed to parse configuration");

// Avoid: embedding everything in the message
info!("pulling supergraph details for graph_id={} variant={}", graph_id, variant);
```

### Common field naming conventions

Use these prefixes to control how values are formatted in log output:

- Use `%` prefix for Display formatting (human-readable): `%graph_id`, `%error`
- Use `?` prefix for Debug formatting (developer-oriented): `?config_object`, `?response_data`
- Use `error = %e` for error context
- Use descriptive field names: `operation_count`, `file_path`, `duration_ms`

**Examples:**

```rust
// Good field names
debug!(file_path = %path, size_bytes = file_size, "reading configuration file");
warn!(retry_count = attempts, max_retries = MAX_ATTEMPTS, "operation failed, retrying");

// Avoid generic or unclear names
debug!(thing = %path, num = file_size, "reading file");
```

### Error Context

Always include relevant context when logging errors:

```rust
// Good: includes context about what failed
error!(path = %config_path, "failed to read configuration file: {e}");

// Avoid: generic error without context
error!("file operation failed: {e}");
```

## Configuration

The logging level can be controlled in several ways:

### Command-line verbosity flags

Use these flags to control the overall log level:

- No flags: WARN level (default - only warnings and errors)
- `-v`: INFO level (includes progress indicators)
- `-vv`: DEBUG level (includes detailed diagnostic information)
- `-vvv`: TRACE level (includes extremely detailed execution flow)

### Environment Variable

Set `APOLLO_RTF_LOG` for fine-grained control over specific modules:

```bash
APOLLO_RTF_LOG=rtf_core=debug,rtf_cli=info cargo run
```

### Per-module Filtering

You can set different log levels for different parts of the codebase:

```bash
APOLLO_RTF_LOG=warn,rtf_core::graphos=debug cargo run
```

> **Note**: When both command-line flags and environment variables are used, the environment
> variable takes precedence for the modules it specifies, while the command-line flag sets the
> default level for other modules.

## Quick reference

### When to use each level

- **ERROR**: Operation cannot continue, user needs to take action
- **WARN**: Something unusual happened, but operation continues (default visibility)
- **INFO**: High-level progress updates, what RTF is currently doing
- **DEBUG**: Detailed diagnostic information for troubleshooting
- **TRACE**: Extremely detailed execution flow for debugging

### Common patterns

```rust
// Error with context
error!(path = %config_path, "failed to read configuration file: {e}");

// Progress indication
info!(test_count = tests.len(), "executing test suite");

// Debug with structured data
debug!(endpoint = %url, method = "POST", "making API request");

// Warning about potential issues
warn!(feature = "deprecated_option", "using deprecated configuration option");
```

### Verbosity flags

- Default: `WARN` and `ERROR` only
- `-v`: Add `INFO` messages
- `-vv`: Add `DEBUG` messages
- `-vvv`: Add `TRACE` messages
