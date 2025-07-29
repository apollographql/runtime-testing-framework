# PR checks

This page documents all the checks that run when raising a PR, how to run the corresponding checks locally
and how to resolve them.

## test
This runs `cargo test`. It will run this for `stable`, `beta` and `nightly` rust builds.

To run and fix locally, run `cargo test` and address any issues that are highlighted.

## rust-fmt
Runs the `cargo pr-format` alias defined in the `.cargo/config.toml` file. It checks the code complies with
rust `fmt`.

To run and fix locally, run `cargo pr-format` and address any issues that are highlighted.

## clippy
Runs the `cargo pr-clippy` alias defined in the `.cargo/config.toml` file. It checks the code complies with
rust `clippy`.

To run and fix locally, run `cargo pr-clippy` and address any issues that are highlighted.

## rustdoc-links
Checks that doc links work correctly.

To run and fix locally, run `RUSTDOCFLAGS="-D rustdoc::broken-intra-doc-links" cargo doc --all-features
--workspace --no-deps` and address any issues that are highlighted.

## spell-check
This uses the [typos-cli crate][0] to check all the docstrings, YAML files
and READMEs for typos. This tool in particular has been chosen as the spell checker for RTF as it has been
designed to reduce false positives.

If using mise, then the `typos-cli` should already be installed, else, [install][1] the cli and then run
```bash
typos
```
This will print all the typos in the repo to screen.

To fix, run
```bash
typos --write-changes
```
This will automatically fix all the detected spelling errors locally.

Before committing to `main`, manually review the diff to check all spelling errors are actually errors and
have been fixed correctly.

If there are any detected spelling errors that should be valid, or files that should be ignored, then
[update the `.typos.toml` file][2] at the
root of the repo.

    [0](https://github.com/crate-ci/typos)
    [1](https://github.com/crate-ci/typos?tab=readme-ov-file#install)
    [2](https://github.com/crate-ci/typos?tab=readme-ov-file#false-positives)
