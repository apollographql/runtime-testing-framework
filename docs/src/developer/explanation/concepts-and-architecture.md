<!-- diataxis-type: explanation -->

# Concepts and architecture

This page provides a high-level overview of RTF and REP's architecture and the key concepts that
inform their design.

## Crates

RTF is organized as a [Cargo workspace][0] with crates stored in the `crates` directory. Each crate
has its own README file explaining its purpose at the crate's root.

The `rtf-config` crate is the heart of RTF. It handles:

- **Parsing** - YAML config files ([Test Plans][1], [Environments][1], [Scenarios][1]) are parsed
  into strongly typed Rust structs
- **Templating** - Variable substitution using the `{{ variable }}` syntax
- **Validation** - Static analysis checks before execution
- **Providers** - Both File Providers and Command Providers live here

The crate exposes a [ResolutionContext][2] trait that abstracts all IO operations, enabling
testability and CLI control over execution.

The `rep-orchestrator` crate is a server-side orchestration layer for REP. It manages the lifecycle
of test runs and individual test executions across two Kubernetes clusters: a management cluster
(Argo workflows for environment provisioning) and a workload cluster (scenario jobs). It uses the
`rep-orchestrator-shared` crate for types shared between it and the `rep-orchestrator-cli` which
submits updates to the clusters.

The `rtf-integrations` crate provides the `rtf-config` crate with clients to make various HTTP
requests.

## Data flow

When a user runs `rtf run test-plan.yaml`, the following flow occurs:

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                              rtf run                                    │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  1. Load Test Plan                                                      │
│     - Parse test-plan.yaml                                              │
│     - Load custom provider definitions                                  │
│     - Resolve scenario/environment references (local or GitHub)         │
│     - Apply any overrides                                               │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  2. Template Test Plan                                                  │
│     - Substitute variables into test plan.                              │
│     - Run static analysis checks                                        │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  3. Execute Environment Setup                                           │
│     - Resolve file providers                                            │
│     - Run setup command                                                 │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  4. Execute Scenario                                                    │
│     - Resolve file providers                                            │
│     - Run scenario command                                              │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  5. Execute Environment Teardown                                        │
│     - Resolve file providers                                            │
│     - Run teardown command                                              │
└─────────────────────────────────────────────────────────────────────────┘
```

## RTF and REP

RTF and REP are related but distinct systems that serve different execution contexts:

- **RTF CLI** (`rtf`) is a local command-line tool. A developer runs it directly to perform actions
  against test plans on the same system the CLI is hosted on.
- **REP (Runtime Environment Provisioner) Orchestrator Service** is a server-side system. It
  receives Test Plans over HTTP, manages their execution in a REP provisioned cluster
  asynchronously, and reports results back to callers via status endpoints.

The handoff point between the two systems is the `RepPayload` — a resolved Test Plan produced by
`rtf rep prepare` and submitted to REP via `POST /test-run/trigger`. REP does not replace the RTF
CLI; they are complementary tools for different execution contexts.

### REP data flow

When a caller triggers a test run via REP:

```text
┌─────────────────────────────────────────────────────────────────────────┐
│  POST /test-run/trigger (RepPayload)                                    │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  1. Create Test Run                                                     │
│     - Test run record created in DB (status: Initialising)              │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  2. Resolve                                                             │
│     - Resolver task picks up the run                                    │
│     - Test plan resolved into individual executions (status: Resolving) │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  3. Provision                                                           │
│     - Event loop provisions the environment (status: Provisioning)      │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  4. Run                                                                 │
│     - Scenario job dispatched (status: Running)                         │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  5. Complete                                                            │
│     - Event loop monitors for terminal status                           │
│       (Successful / Failed)                                             │
│     - Environment teardown run                                          │
│     - Test run marked complete                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

## Key abstractions

### Providers

Providers are the primary extension point in RTF. They come in two forms:

- **File Providers** - Generate files that are made available to commands via environment variables.
  Examples include `relative_path` (reading a file from a specific relative path),
  `graphos_supergraph` (fetch from GraphOS), and `merge_yaml` (a utility to merge YAML from multiple
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

See [Use of IO in Providers][3] for more details.

## Design principles

RTF follows several key design principles:

- **Composition over embedding** - RTF composes with external tools rather than embedding them. See
  the [Overview][4] for more on this philosophy.

- **Plumbing and porcelain** - Commands are split into low-level "plumbing" (like `template`) and
  high-level "porcelain" (like `run`). See [Plumbing vs Porcelain][5].

- **No built-in magic** - Commands don't have special inline logic. See [No Built-in Magic][6].

- **Fail fast with good errors** - RTF validates early and reports all known errors in batch rather
  than failing on the first error. See [Error Handling][7].

[0]: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
[1]: ../../reference/glossary.md
[2]: https://github.com/apollographql/runtime-testing-framework/blob/main/crates/rtf-config/src/context.rs
[3]: context.md
[4]: ../../explanation/overview.md
[5]: cli-design/plumbing-vs-porcelain.md
[6]: cli-design/no-built-in-magic.md
[7]: ../reference/error-handling.md
