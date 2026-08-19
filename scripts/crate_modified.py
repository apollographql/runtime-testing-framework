#!/usr/bin/env python3
"""Exit 0 if any of --crates were modified relative to --mainline, else 1.

"Modified" means: the crate's own files changed, OR any local path
dependency (dependencies/dev-dependencies/build-dependencies, including
`workspace = true` refs) changed, transitively. A change to the root
Cargo.toml or Cargo.lock always counts as "everything changed".

Local dependency resolution comes from `cargo metadata --no-deps`.

Pass --working-tree to check currently staged/unstaged changes instead
"""

import argparse
import json
import logging
import subprocess
import sys
from pathlib import Path

log = logging.getLogger("crate_modified")


def run_git(root, *args):
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True, cwd=root
    ).stdout


def changed_files(root, mainline, working_tree):
    if working_tree:
        out = run_git(root, "diff", "--no-renames", "--name-only", "HEAD")
    else:
        out = run_git(root, "diff", "--no-renames", "--name-only", f"{mainline}...HEAD")
    return [line for line in out.splitlines() if line]


def under_dir(file_path, dir_path):
    fp, dp = Path(file_path).parts, Path(dir_path).parts
    return fp[: len(dp)] == dp


def cargo_metadata(root):
    try:
        out = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            check=True, capture_output=True, text=True, cwd=root,
        ).stdout
    except subprocess.CalledProcessError as e:
        sys.exit(f"error: `cargo metadata` failed:\n{e.stderr}")
    return json.loads(out)


class Workspace:
    def __init__(self, root):
        self.root = root.resolve()
        meta = cargo_metadata(self.root)
        workspace_root = Path(meta["workspace_root"]).resolve()
        packages = meta["packages"]

        name_by_dir = {}
        self.crates = {}
        for pkg in packages:
            pkg_dir = Path(pkg["manifest_path"]).resolve().parent
            name_by_dir[pkg_dir] = pkg["name"]
            self.crates[pkg["name"]] = {
                "dir": pkg_dir.relative_to(workspace_root).as_posix(),
                "deps": set(),
            }

        for pkg in packages:
            for dep in pkg["dependencies"]:
                if dep.get("path") is None:
                    continue
                dep_crate = name_by_dir.get(Path(dep["path"]).resolve())
                if dep_crate:
                    self.crates[pkg["name"]]["deps"].add(dep_crate)


def find_modifications(files, crate_name, crates):
    modifications = []
    visited = set()

    def visit(name, path_stack):
        if name in path_stack or name in visited:
            return

        visited.add(name)
        crate = crates[name]

        for f in files:
            if under_dir(f, crate["dir"]):
                modifications.append((name, f))
        for dep in sorted(crate["deps"]):
            visit(dep, path_stack + [name])

    visit(crate_name, [])

    return modifications


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crates", required=True, help="comma-separated crate names")
    parser.add_argument("--mainline", default="origin/main", help="ref to diff against (default: origin/main)")
    parser.add_argument(
        "--working-tree",
        action="store_true",
        help="use currently staged/unstaged changes (git diff HEAD) instead of diffing against --mainline; for manual local verification",
    )
    parser.add_argument("--root", default=".", type=Path, help="workspace root (default: cwd)")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format="%(message)s")

    root = args.root.resolve()
    targets = [c.strip() for c in args.crates.split(",") if c.strip()]
    ws = Workspace(root)

    for name in targets:
        if name not in ws.crates:
            sys.exit(f"error: unknown crate {name!r}")

    files = changed_files(root, args.mainline, args.working_tree)

    diff_source = "working tree" if args.working_tree else args.mainline
    log.debug("changed files vs %s:\n  %s", diff_source, "\n  ".join(files) or "(none)")

    root_hits = [f for f in files if f in ("Cargo.toml", "Cargo.lock")]
    if root_hits:
        for f in root_hits:
            log.info("[MODIFIED] all crates -> root manifest changed: %s", f)
        sys.exit(0)

    any_modified = False

    for name in targets:
        modifications = find_modifications(files, name, ws.crates)
        if modifications:
            any_modified = True
            for via, f in modifications:
                via_note = "" if via == name else f" (via local dependency {via})"
                log.info("[ MODIFIED ] %s%s: %s", name, via_note, f)
        else:
            log.info("[UNMODIFIED] %s: no relevant changes", name)

    sys.exit(0 if any_modified else 1)


if __name__ == "__main__":
    main()
