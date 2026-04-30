<!-- diataxis-type: howto -->

# How to run a test coverage report

> **Prerequisites**
>
> - [`cargo-llvm-cov`][0] installed (included if you use [`mise`][1])
> - Valid API credentials for any ignored tests you want to include

> **Note**: There are no coverage targets for RTF. Reports are used to identify unexpected gaps, not
> to hit a percentage. High coverage does not mean the code is fully tested.

Clear previous coverage data:

```bash
cargo llvm-cov clean --workspace
```

Run all unignored tests:

```bash
cargo llvm-cov --no-report
```

Optionally, run the ignored tests to add their coverage. Supply only the credentials you have
available:

```bash
GITHUB_TOKEN="$GITHUB_TOKEN" cargo llvm-cov -p rtf-cli --no-report -- --ignored github::
APOLLO_KEY="$APOLLO_KEY" APOLLO_SUDO="true" cargo llvm-cov -p rtf-cli --no-report -- --ignored graphos::
```

Generate and open the report:

```bash
cargo llvm-cov report --open
```

Inspect the report manually for unexpected gaps. Two things to keep in mind:

- 100% coverage for an area doesn't mean it's fully tested — it means the code ran at least once. A
  single execution rarely covers all scenarios.
- Low coverage in some areas may be justified. Where there's a good reason, leaving lines uncovered
  is acceptable.

[0]: https://github.com/taiki-e/cargo-llvm-cov
[1]: https://mise.jdx.dev/getting-started.html
