<!-- diataxis-type: explanation -->

# Concepts and Architecture

This page provides a high-level overview of RTF's architecture and the key concepts that inform its
design.

## Crates

RTF is organized as a [Cargo workspace][0] with crates stored in the `crates` directory. Each crate
has its own README file explaining its purpose at the crate's root.

The `rtf-config` crate is the heart of RTF. It handles:

- **Parsing** - YAML config files (Test Plans, Environments, Scenarios) are parsed into strongly
  typed Rust structs
- **Templating** - Variable substitution using the `{{ variable }}` syntax
- **Validation** - Static analysis checks before execution
- **Providers** - Both file providers and command providers live here

The crate exposes a [ResolutionContext][1] trait that abstracts all IO operations, enabling
testability and CLI control over execution.

The `rtf-integrations` crate provides the `rtf-config` crate with clients to make various HTTP
requests.

The other crates sit above these crates, providing business logic and presentation layers for RTF.

## Data flow

When a user runs `rtf run test-plan.yaml`, the following flow occurs:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              rtf run                                     │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  1. Load Test Plan                                                       │
│     - Parse test-plan.yaml                                               │
│     - Load custom provider definitions                                   │
│     - Resolve scenario/environment references (local or GitHub)          │
│     - Apply any overrides                                                │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  2. Template Environment Setup                                           │
│     - Substitute variables into setup section                            │
│     - Run static analysis checks                                         │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  3. Execute Environment Setup                                            │
│     - Resolve file providers                                             │
│     - Run setup command                                                  │
│     - Capture "provides" output for later templating                     │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  4. Template Scenario + Environment Teardown                             │
│     - Use setup output to finish templating                              │
│     - Run static analysis checks                                         │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  5. Execute Scenario                                                     │
│     - Resolve file providers                                             │
│     - Run scenario command                                               │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  6. Execute Environment Teardown                                         │
│     - Resolve file providers                                             │
│     - Run teardown command                                               │
└─────────────────────────────────────────────────────────────────────────┘
```

## Key abstractions

### Providers

Providers are the primary extension point in RTF. They come in two forms:

- **File Providers** - Generate files that are made available to commands via environment variables.
  Examples include `relative_path` (reading a file from a specific relative path),
  `graphos_supergraph` (fetch from GraphOS), and `merge_yaml` (a utility to merge yaml from multiple
  sources). Custom Providers allow users to define their own file providers using YAML definitions
  that execute commands to produce files.

- **Command Providers** - Define executable commands with their environment variables and file
  provider dependencies. Environments and Scenarios are both command providers with defined
  execution semantics.

### The Template trait

The `Template` trait enables recursive traversal of config structs to find and resolve templatable
fields. It is typically derived using `#[derive(Template)]` from `rtf-derive`.

### The Field type

The `Field<T>` enum is the mechanism that enables templating within config structs. It wraps scalar
types (strings, numbers, booleans) and can exist in one of two states:

```rust
pub enum Field<T> {
    Pending(String),   // Contains a variable name to be resolved
    Resolved(T),       // Contains the final value
}
```

When RTF parses a YAML config file, any value matching the `"{{ variable_name }}"` pattern is
deserialized as `Field::Pending("variable_name")`. Values without this pattern become
`Field::Resolved(value)` immediately.

For example, given this YAML:

```yaml
graph_ref: "{{ graph }}"
top_n: 20
```

The `graph_ref` field parses as `Field::Pending("graph")` while `top_n` parses as
`Field::Resolved(20)`.

During templating, the `Template` trait's `try_template` method walks the config struct and resolves
each pending field by looking up its variable name in the `TemplateContext`. Once resolved, the
field transitions from `Pending` to `Resolved` and can be used during execution.

This design provides several benefits:

- **Type safety** - The generic parameter `T` ensures variables resolve to the correct type
- **Validation** - Pending fields are detected before execution, enabling early error reporting
- **Traceability** - The path to each field is tracked, producing clear error messages like
  `ERROR (environment.setup.file_providers[0].graph_ref) unknown templating variable: graph`

### ResolutionContext

All IO in providers must go through the `ResolutionContext` trait. This abstraction:

- Enables mocking in tests
- Gives the CLI control over execution
- Provides a consistent interface for file operations, HTTP requests, and command execution

See [Use of IO in Providers](context.md) for more details.

## Design principles

RTF follows several key design principles:

1. **Composition over embedding** - RTF composes with external tools rather than embedding them. See
   the [Overview](../../explanation/overview.md) for more on this philosophy.

2. **Plumbing and porcelain** - Commands are split into low-level "plumbing" (like `template`) and
   high-level "porcelain" (like `run`). See
   [Plumbing vs Porcelain](cli-design/plumbing-vs-porcelain.md).

3. **No built-in magic** - Commands don't have special inline logic. See
   [No Built-in Magic](cli-design/no-built-in-magic.md).

4. **Fail fast with good errors** - RTF validates early and reports all known errors in batch rather
   than failing on the first error. See [Error Handling](../reference/error-handling.md).

[0]: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
[1]: https://github.com/apollographql/runtime-testing-framework/blob/main/crates/rtf-config/src/context.rs
