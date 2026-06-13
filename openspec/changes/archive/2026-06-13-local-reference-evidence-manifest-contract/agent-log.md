# Agent Log: local-reference-evidence-manifest-contract

## Orchestrator Plan

Objective: define and gate a portable local-reference evidence manifest for
future decoder fixtures and local AVM/dav2d comparison metadata, without
changing the crate dependency graph or adding external decoder integration.

Reason for selecting this slice: runtime decoder crate scaffolding still needs
explicit maintainer approval, but the mission can advance through docs,
OpenSpec, and automation that make future local evidence portable and
self-contained.

Feature ID: `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST`.

## Planning Agents

### @architect / Bernoulli

- Agent ID: `019ec1a1-c5f2-74d2-a26c-92f8ce87379e`
- Objective: assess a PR-sized docs/OpenSpec/automation slice while dependency
  graph changes remain unapproved.
- Output: recommended a contract plus offline portability/schema gate, primary
  Feature ID `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST`, decoder-support row
  `local-reference-evidence-manifest`, a new `xtask/src/reference_evidence.rs`
  module, and wiring through `cargo xtask check-decoder-support`. Non-goals:
  no new crates, dependencies, fixtures, AVM/dav2d invocation, runners,
  wrappers, scripts, CI jobs, or runtime decode/hash/Y4M support.

### @spec-reader / Ohm

- Agent ID: `019ec1a1-c8cf-7d93-bccb-44f6c9c08fd0`
- Objective: identify AV2 citations and wording for the manifest contract.
- Output: confirmed the manifest envelope should cite no AV2 section. Future
  hash evidence may cite § 6.16.13 and § 7.21.1 / § 7.21.2 when recording
  decoded-output hash metadata; § 5.17.12 applies only to AV2
  `METADATA_TYPE_DECODED_FRAME_HASH` metadata interop fields. Avoid wording
  that claims runtime decode support, CI reruns of AVM/dav2d, AV2 conformance,
  or an AV2 SHA-256 hash type.

### @api-designer / Confucius

- Agent ID: `019ec1a1-cb7a-7bc0-aecf-47b7a4ba599c`
- Objective: recommend the manifest schema and checker behavior.
- Output: recommended a structured TOML manifest with `manifest_version`,
  evidence IDs, feature and decoder-support row references, fixture identity,
  reference-run metadata, output digest metadata, and assertions. The checker
  should reject local paths, executable paths, shell command composition,
  malformed hashes, duplicate IDs, missing fixture files, stale fixture hashes,
  and broken cross-references without running external tools.

### @reference-oracle / Chandrasekhar

- Agent ID: `019ec1a1-cf01-72a3-bffa-443794aead36`
- Objective: determine whether fresh local AVM/dav2d evidence is needed for
  this slice and define allowed/forbidden evidence boundary.
- Output: confirmed no fresh AVM/dav2d evidence is needed. Committed manifests
  may record future local evidence as portable metadata only: tool identity,
  upstream revisions, repo-relative fixture identity, sanitized command
  summaries, decoded hashes, comparison notes, dates, caveats, and provenance.
  Forbidden: any committed source, binaries, wrappers, scripts, CI hooks,
  dependency, build probe, runnable test, local paths, or claim that local
  metadata proves `splot decode`.

## Local Reference Boundary

No AVM or dav2d command was run for this change. No AVM/dav2d source, snippets,
binaries, submodules, dependencies, build probes, wrappers, CI jobs, required
scripts, `xtask` runners that execute external decoders, or mandatory tests are
added by this change.

The checker introduced by this change is intended to parse committed metadata
and committed fixture bytes only. It must be usable on machines with no external
decoder installed.

## Implementation Notes

- Added `docs/LOCAL-REFERENCE-EVIDENCE.toml` as the empty versioned manifest
  skeleton for future decoder local-reference metadata.
- Added `xtask/src/reference_evidence.rs` and the focused
  `cargo xtask check-reference-evidence` command.
- Wired `cargo xtask check-decoder-support` to run the reference-evidence
  checker, so `cargo xtask ci` covers the manifest through the existing decoder
  support gate.
- Updated `docs/DECODER-ROADMAP.md`, `docs/CONFORMANCE.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, and `docs/IMPLEMENTATION-MATRIX.toml`
  without claiming runtime decode, reconstruction, deterministic hash, Y4M, or
  live AVM/dav2d execution support.

## Verification

- `cargo fmt --all -- --check`
- `cargo test -p xtask reference_evidence --locked`
- `cargo xtask check-reference-evidence`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `openspec validate local-reference-evidence-manifest-contract --strict`
- `openspec validate --all --no-interactive`
- `git diff --check`
- `cargo xtask ci`

## Review Agents

### reviewer / Kepler

- Agent ID: `019ec1b7-536a-7223-9578-b986988c5d27`
- Findings:
  - `P2`: fixture validation did not prove the path was a committed in-repo
    file; untracked files and symlinked parent directories could pass locally.
  - `P2`: `command_summary` allowed relative executable/path-like fragments
    such as `./build/avmdec` or `tools/dav2d`.
  - `P3`: duplicate `reference_run.id` values were not rejected.
- Resolution:
  - Added fixture path component walking that rejects any symlink component,
    canonical root containment checks, and git-tracked fixture enforcement when
    the checker runs inside a Git worktree.
  - Added command-summary relative path fragment rejection.
  - Added `reference_run.id` uniqueness validation.
  - Added regression tests for each case in `xtask/src/reference_evidence/tests.rs`.

### security-reviewer / Bacon

- Agent ID: `019ec1b7-56e8-7ee2-b251-e9f00983a16a`
- Findings:
  - `P2`: fixture paths could escape the repo through symlinked parent
    components.
  - `P3`: local path detection missed colon-prefixed forms such as
    `cwd:/Users/me/avmdec`.
  - `P3`: digest equality assertions could be tautological.
- Resolution:
  - Added symlink component walking and canonical root containment checks.
  - Added colon-fragment scanning for local path detection while preserving
    non-file URLs.
  - Added digest assertion checks requiring distinct digest IDs and distinct
    reference-run IDs.
  - Added regression tests for intermediate symlink, colon-prefixed path,
    duplicate run ID, and tautological digest assertion cases.

### spec-conformance-reviewer / Meitner

- Agent ID: `019ec1b7-5b1b-7d53-9e08-b306f570ec5a`
- Findings: none. Confirmed the change keeps the manifest separate from the
  conformance corpus and avoids AV2/runtime decode overclaims.

### encoder-impact-reviewer / Descartes

- Agent ID: `019ec1b7-5ee8-7ca0-8d3e-b4a4edb00a19`
- Findings: none. Confirmed no encoder-facing crate, encoder research doc,
  dependency graph, or reconstruction API impact.

## Post-Review Verification

- `cargo fmt --all -- --check`
- `cargo test -p xtask reference_evidence --locked`
- `cargo xtask check-reference-evidence`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `openspec validate local-reference-evidence-manifest-contract --strict`
- `openspec validate --all --no-interactive`
- `git diff --check`
- `cargo xtask check-source-lines`
- `cargo xtask ci`

## Archive

- Ran `openspec archive local-reference-evidence-manifest-contract --yes`.
- Archive path:
  `openspec/changes/archive/2026-06-13-local-reference-evidence-manifest-contract/`.
- Synced the `decoder-support` delta into
  `openspec/specs/decoder-support/spec.md`.

## Post-Archive Verification

- `cargo test -p xtask reference_evidence --locked`
- `cargo xtask check-reference-evidence`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `openspec validate --all --no-interactive`
- `git diff --check`
- `cargo fmt --all -- --check`
- `cargo xtask ci`
