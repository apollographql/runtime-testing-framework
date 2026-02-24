---
name: review-rtf-docs
description: Review RTF documentation against the style guide and Diataxis framework
argument-hint: [--all | --section <path> | --file <path>]
---

You are reviewing RTF documentation. Work through the steps below in order.

## Excluded files

Never review these files, even if explicitly targeted:

- `docs/src/SUMMARY.md`
- `docs/src/developer/reference/style-guide.md`

## Step 1: Determine scope

Parse $ARGUMENTS to determine which files to review:

**Default (no arguments):** Collect all changed markdown files on the current branch relative to
`main`. Run both of these and combine the results:

`git diff --name-only main...HEAD "$(git rev-parse --show-toplevel)/docs/src/"`

`git diff --name-only --cached "$(git rev-parse --show-toplevel)/docs/src/"`

Remove excluded files from the combined results. If no files remain, tell the user:

> No documentation changes detected on this branch. Use `--all`, `--section <path>`, or
> `--file <path>` to review specific docs.

Then stop.

**`--all`:** Find all markdown files under `docs/src/` (excluding the two files above).

**`--section <path>`:** Find all markdown files under `docs/src/<path>/` (excluding the two files
above).

**`--file <path>`:** Use the single file `docs/src/<path>`. Verify it exists and is not excluded.

## Step 2: Read the style guide

Read the full style guide so you can apply it during review:

`cat docs/src/developer/reference/style-guide.md`

## Step 3: Review files (batched)

Review files in batches of up to 8 at a time. For each batch: read all files in the batch, produce
their reviews, then move to the next batch.

Read each file's full content regardless of what changed — do not limit review to diff lines.

For each file, output a review section using the format below. Only report issues that are actually
present. Do not invent violations or pad findings.

---

### `<path relative to docs/src/>`

**Declared type:** `<type>` _(or `[ERROR] No diataxis-type declaration found`)_

<findings>

---

### Severity levels

- `[ERROR]` — must be fixed: missing/invalid `diataxis-type` declaration, content clearly does not
  match declared type
- `[WARN]` — style guide violation that should be corrected
- `[SUGGESTION]` — non-blocking (e.g., a better-fitting Diataxis type when content partially matches
  but another type is a stronger fit)

### Diataxis declaration checks

These are mechanical checks not covered by the style guide prose:

1. **Presence:** The very first line must be `<!-- diataxis-type: <type> -->`. If missing or not on
   line 1: `[ERROR] Missing diataxis-type declaration — must be the first line`

2. **Valid value:** Valid values are `tutorial`, `howto`, `reference`, `explanation`. Anything else:
   `[ERROR] Invalid diataxis-type value: "<value>"`

3. **Type match:** Using the type definitions and guidelines in the style guide you read in Step 2,
   evaluate whether the content matches the declared type.
   - Clearly does not match:
     `[ERROR] Content does not match declared type. Suggested type: <alternative> — <reason>`
   - Declared type fits but another is a stronger fit:
     `[SUGGESTION] Consider type "<alternative>" — <reason>`

### Style guide checks

Apply every rule in the style guide you read in Step 2. Do not re-derive the rules — use them
directly as written.

## Step 4: Summary

After all batches are complete, output a summary:

```
## Review summary

Files reviewed: N
Files with errors: N   (must fix)
Files with warnings: N (should fix)
Files clean: N

### Errors
- `<file>`: <brief description of each error>

### Warnings
- `<file>`: <N> warning(s) — <brief description>
```

If all files are clean, say so clearly.
