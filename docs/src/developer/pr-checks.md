# PR checks

This page documents all the checks that run when raising a PR, how to run the corresponding checks locally
and how to resolve them.

All the tasks required to run these checks locally are defined using [mise tasks][0]. To run all the PR
checks locally:
```bash
mise run pr-all
```

## test
The GitHub action runs `cargo test`. It will run this for `stable`, `beta` and `nightly` rust builds.

To run locally
```bash
mise run pr-test
```
This will show details for all failing test cases.

## rust-fmt
The GitHub action runs the `cargo pr-format` alias defined in the `.cargo/config.toml` file. It checks
the code complies with
rust `fmt`.

To run locally
```bash
mise run pr-format
```
This will show details for all formatting issues.

## clippy
The GitHub action runs the `cargo pr-clippy` alias defined in the `.cargo/config.toml` file. It checks
the code complies with
rust `clippy`.

To run locally
```bash
mise run pr-clippy
```
This will show details for all clippy issues.

## rustdoc-links
The GitHub action runs `cargo doc` with additional flags to check that doc links work correctly.

To run locally
```bash
mise run pr-doclinks
```
This will show details for all doclink issues.
> **Note**: Most of the `cargo docs` output will be warnings. This might be quite noisey but won't
necessarily fail your PR checks.

## spell-check
This uses the [typos-cli crate][1] to check all the docstrings, YAML files and READMEs for typos. This
tool in particular has been chosen as the spell checker for RTF as it has been designed to reduce false
positives.

To run locally
```bash
mise run pr-spell-check
```
This will print all the typos in the repo to screen.

To fix all these typos automatically
```bash
mise run fix-spelling
```
This will automatically fix all the detected spelling errors locally.

Before committing to `main`, manually review the diff to check all spelling errors are actually errors and
have been fixed correctly.

If there are any detected spelling errors that should be valid, or files that should be ignored, then
[update the `.typos.toml` file][3] at the root of the repo.

## lint-markdown
This uses the [dprint crate][4] lint the markdown files and ensure consistent style and formatting.

To run locally
```bash
mise run lint-markdown
```
This will highlight any issues with the markdown files.

To fix all these issues automatically
```bash
mise run format-markdown
```
Before committing to `main`, manually review the diff to check all formatting fixes are desired.

Files can be excluded using the `dprint.json` file at the root of the repo.

    [0](https://mise.jdx.dev/tasks/)
    [1](https://github.com/crate-ci/typos)
    [2](https://github.com/crate-ci/typos?tab=readme-ov-file#install)
    [3](https://github.com/crate-ci/typos?tab=readme-ov-file#false-positives)
    [4](https://github.com/dprint/dprint)
