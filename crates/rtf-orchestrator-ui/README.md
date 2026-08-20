# RTF Orchestrator UI

The server-rendered (Askama + htmx) web UI served by the orchestrator, for viewing test run and
execution status, browsing registered test plans, and triggering new runs.

## Testing

Run tests as usual with `cargo test`. Most Askama template rendering (`src/view/`) is unit-tested by
asserting on the view structs directly; a small set of full-page renders are instead covered by
[`insta`][0] snapshot tests, to catch unintended rendering regressions without hand-writing
per-field assertions against the HTML output.

Install the [`cargo-insta`][1] CLI to review snapshot changes:

```bash
cargo install cargo-insta
```

After a change affects a snapshot:

```bash
cargo insta test    # run tests and collect any changed snapshots
cargo insta review  # interactively accept or reject each diff
```

Accepted snapshots live alongside their test module under `src/view/snapshots/*.snap` and are
committed to the repo.

[0]: https://insta.rs
[1]: https://insta.rs/docs/cli/
