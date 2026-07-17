<!-- diataxis-type: explanation -->

# Orchestrator Helm chart versioning policy

This document records the decisions behind how we version and deploy the `rep-orchestrator` Helm
chart for the [RTF Orchestrator Service][0], the rationale for the phased approach, and the options
that were considered and rejected.

**When a phase is completed, update the [Current phase][1] section to reflect actual state. The
phase descriptions below are a permanent historical record of what was done and why — they should
not be edited after the fact.**

## Current phase

> **Phase 1 + 2 — chart/image link fixed; production uses `edge` for fast iteration**
>
> The chart is published with version `0.0.0+<git-sha>` and `appVersion` set to the git SHA. A
> pinned chart version guarantees a pinned image via the `appVersion` fallback in the Deployment
> template. Production overrides `image.tag: edge` and `pullPolicy: Always` in the ArgoCD
> `valuesObject`, so pods pick up the latest image on every rollout without a chart bump PR.
> Production is still updated by manually opening a PR on `runtime-environment-provisioner` to bump
> `targetRevision`.

## Design intent

The chart and image are published together on every code merge by design. The intent is that
updating the Helm chart is the canonical way to update the Orchestrator — a chart version change
implies a software change, and a software change produces a new chart version. This keeps deployment
straightforward: there is one thing to bump (the chart), and it brings everything with it.

## Options considered

Before settling on the phased approach, three alternatives were evaluated:

### Option A — rolling tag (`prod` / `latest`)

Publish the chart under a mutable tag and point ArgoCD at it so it auto-tracks.

**Rejected.** ArgoCD's OCI Helm source requires an exact `targetRevision` string; it cannot track a
mutable tag or a semver range. It would pull the tag once at install time and never update. Making
this work would require ArgoCD Image Updater, an additional operational dependency. More
fundamentally, with no staging environment between `main` and production, a mutable tag means any
bad merge silently redeploys with no human checkpoint — unacceptable as we approach live consumers.

### Option B — immediate move to semver

Replace `0.0.0+sha` with incrementing semver (`0.1.0`, `0.1.1`, …) from the outset.

**Rejected for now.** ArgoCD OCI still requires an exact version; semver ranges do not work, so we
gain no auto-tracking benefit. During early active development (several merges per day) we would
accumulate hundreds of patch versions with no semantic distinction between them.

### Option C — SHA pinning with phased improvement (chosen)

Keep SHA-based pinning but fix its problems in layers, making the process progressively more
automated and semantically correct as the service matures.

## Phase history

### Phase 0 — Baseline

**State:**

- Chart published as `0.0.0+<git-sha>` on every merge to `main` that touches `crates/**`.
- Image published with tags: full git SHA, `main-<short-sha>`, and mutable `edge`.
- `values.yaml` hardcodes `image.tag: edge` — every chart version deploys whatever `edge` points to
  at rollout time, regardless of the chart's own SHA.
- Production updated by manually opening a PR on `runtime-environment-provisioner` to bump
  `targetRevision`.

**Problems that motivated moving to Phase 1:**

- **Chart and image are decoupled.** Pinning a chart version does not guarantee a pinned image. The
  SHA in the chart version identifies the templates, not the running binary, contradicting the
  lockstep intent.
- **`0.0.0+sha` is semantically unordered.** Semver build metadata (the `+` segment) has no defined
  precedence — `0.0.0+abc` and `0.0.0+def` are equal. This rules out any tooling that relies on
  version ordering (including Renovate).
- **Manual PRs at high velocity.** With several merges per day, repeatedly bumping `targetRevision`
  by hand is a significant source of toil.

---

### Phase 1 — fix the chart/image version link

**What changed:**

- The `helm package` step passes `--app-version ${{ github.sha }}`, embedding the git SHA into
  `Chart.appVersion`.
- The Helm `Deployment` template uses `appVersion` as the fallback image tag:
  ```yaml
  image: "{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}"
  ```
- `values.yaml` clears `image.tag` (previously hardcoded to `edge`) and sets
  `pullPolicy: IfNotPresent` as the chart default. SHA tags are immutable; `Always` is wasteful when
  the image cannot have changed.

**Result:** A pinned chart version guarantees a pinned image. `image.tag` remains overridable via
values, which is used in Phase 2.

---

### Phase 2 — production uses `edge` for fast iteration

**What changed:**

- The ArgoCD `valuesObject` in `runtime-environment-provisioner` adds:
  ```yaml
  image:
    tag: edge
    pullPolicy: Always
  ```

**Result:** Production picks up the latest image on every pod restart or rollout without requiring a
chart bump PR. With several merges per day and no live consumers at this stage, this removes the
manual PR toil during the highest-velocity period of development.

**Trade-off:** The `edge` override is a known, intentional deviation from the lockstep intent. The
deployed image is not pinned. This is acceptable while there are no consumers; it must be removed
before the service takes on production traffic (Phase 3).

> **Note:** Phases 1 and 2 are a single implementation effort, listed separately because Phase 1 is
> a change in this repo and Phase 2 is a values change in `runtime-environment-provisioner`.

---

### Phase 3 — remove `edge` override (enforce lockstep in production)

**Trigger:** The service has external consumers and a bad deploy has real consequences.

**What changed:**

- The `image.tag: edge` and `pullPolicy: Always` overrides are removed from the production
  `valuesObject` in `runtime-environment-provisioner`.
- The chart's `appVersion` (the git SHA baked in at publish time) becomes the live image tag.
- Deploying a new image now requires a chart bump PR to `runtime-environment-provisioner`.

**Result:** The lockstep intent is fully enforced in production. Every code change produces a new
chart version; bumping to that version in `runtime-environment-provisioner` is the single operation
that updates the running Orchestrator.

---

## Summary

| Phase | Image tag in production      | Lockstep enforced?         | Automation                                             |
| ----- | ---------------------------- | -------------------------- | ------------------------------------------------------ |
| 0     | `edge` (hardcoded in values) | No                         | None — manual PRs to `runtime-environment-provisioner` |
| 1 + 2 | `edge` (values override)     | No — intentional deviation | None — manual PRs to `runtime-environment-provisioner` |
| 3     | Git SHA (`appVersion`)       | Yes                        | None — manual PRs to `runtime-environment-provisioner` |

[0]: ../../reference/glossary.md
[1]: #current-phase
