# RTF Orchestrator UI

The server-rendered (Askama + htmx + Alpine.js) web UI served by the orchestrator, for viewing test
run and execution status, browsing registered test plans, and triggering new runs.

## Libraries

| Library            | Used for                                                               | Docs                              |
| ------------------ | ---------------------------------------------------------------------- | --------------------------------- |
| Askama `0.16`      | Server-side HTML templates (`templates/`, structs in `src/templates/`) | [Book][2] · [API][3]              |
| htmx `2.0.4`       | Partial page updates from the server, e.g. run status polling          | [Docs][4] · [Reference][5]        |
| Alpine.js `3.14.9` | Client-side interactivity, e.g. the test plan page's trigger form      | [Start here][6] · [Directives][7] |
| Chart.js `4.4.4`   | Charts on the dashboard and test plan pages                            | [Docs][8]                         |

The JS libraries are vendored into `static/` (see the header comment in each `*.min.js` file for its
source) and embedded into the binary, so the UI makes no requests to third-party CDNs. To upgrade
one, replace the file with the new release and update its header comment and the version above.

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

### JS unit tests

The trigger form's model (`static/trigger-model.js`) is unit tested with Node's built-in
[test runner][9], with no npm dependencies. Node is installed by mise:

```bash
mise run pr-js-test
```

The tests load `tests/js/fixtures/test-plan-details.json` as the page data. A Rust test fails if
that fixture no longer matches what the server embeds in the page; after an intentional change,
regenerate it and check the JS tests still pass:

```bash
UPDATE_JS_FIXTURES=1 cargo test -p rtf-orchestrator-ui js_data_matches_the_js_test_fixture
mise run pr-js-test
```

[0]: https://insta.rs
[1]: https://insta.rs/docs/cli/
[2]: https://askama.rs/en/stable/
[3]: https://docs.rs/askama/0.16.1/askama/
[4]: https://htmx.org/docs/
[5]: https://htmx.org/reference/
[6]: https://alpinejs.dev/start-here
[7]: https://alpinejs.dev/directives/data
[8]: https://www.chartjs.org/docs/4.4.4/
[9]: https://nodejs.org/api/test.html
