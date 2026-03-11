---
name: bump-deps
description: Update Rust toolchain, Cargo deps, mise tools, and align CI versions
---

Perform a full dependency bump for this Rust project. Work through each phase below in order. Each
phase ends with a commit — only commit if `mise run pr-all` passes cleanly.

## Phase 1 — Rust toolchain

1. Run `rustup check` to find the latest stable version.
2. Update `rust = "..."` in `.config/mise/mise.toml`.
3. Update both `rust:<version>` image tags in `toolbox/Dockerfile` to match.
4. Run `mise install`.
5. Run `mise run pr-all`.
6. Commit: `Update Rust toolchain to <version> in mise and Dockerfile`

## Phase 2 — mise tool versions

For each tool in the `[tools]` section of `.config/mise/mise.toml` (excluding `rust`, which was
handled in Phase 1):

1. Find the latest version:
   - Cargo tools: `mise exec -- cargo search <crate-name> --limit 1`
   - Other tools (dprint, mdbook, etc.): `mise ls-remote <tool> | tail -5`
2. Update all versions in `.config/mise/mise.toml`.
3. Run `mise install`.
4. Run `mise run pr-all`.
5. Commit: `Upgrade mise tools to latest versions`

Use the actual version number, not `"latest"`.

If `pr-all` fails, or the tool upgrade causes changes to other files, revert that tool's version
bump. Commit the passing upgrades, then flag the reverted tool in your summary for follow-up.

## Phase 3 — Cargo dependencies

Prerequisite: `cargo upgrade` must be available via mise (cargo-edit). If it is not in `mise.toml`,
add it with the current pinned version before continuing.

1. Run `mise exec -- cargo upgrade --incompatible allow` to bump version constraints in all
   `Cargo.toml` files.
2. Run `cargo update` to sync `Cargo.lock`.
3. Run `mise run pr-all`.
4. Commit: `Upgrade Cargo workspace dependencies to latest`

If `pr-all` fails, identify the breaking package, revert only that package's version
(`mise exec -- cargo upgrade --package <name> --precise <old-version>`), re-run `pr-all`, commit the
passing upgrades, then flag the reverted package in your summary for follow-up.

## Phase 4 — GitHub Actions alignment

Check `.github/workflows/*.yaml` for any tool versions that are now out of sync with mise:

1. Search for hardcoded version strings matching the tools updated in Phase 2.
2. Update them to match. Common locations:
   - `crate-ci/typos@v<version>` action refs
   - Inline `mise_toml:` blocks that specify tool versions
3. Commit: `Align GitHub Actions tool versions with mise`

Note: The Rust version in CI is already read dynamically from `mise.toml` — do not hardcode it.

## Summary

After all phases, report:

- What changed in each phase
- Any tools or packages skipped due to breaking changes (flagged for follow-up)
- Final commit log (`git log --oneline -5`)
