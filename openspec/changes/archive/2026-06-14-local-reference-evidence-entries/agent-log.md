# Agent Log: local-reference-evidence-entries

## Orchestrator Plan

Objective: convert already-recorded local AVM/dav2d raw output agreement for
two committed decoder fixtures into checked, portable metadata in
`docs/LOCAL-REFERENCE-EVIDENCE.toml`.

Scope:

- OpenSpec capability: `decoder-support`.
- Feature IDs: `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST` for the manifest
  checker/schema and `RECON-HASH-INPUT-SERIALIZATION` for the evidence target.
- Decoder support rows: `local-reference-evidence-manifest` and
  `deterministic-frame-hash`.
- No AVM/dav2d execution from repo code, tests, scripts, `xtask`, CI, or build.
- No crate or dependency graph change.
- No runtime decode, reconstruction, decoded-frame SHA-256 digest, or Y4M
  output.
- PR #101 concurrency model remains unchanged; this metadata-only change must
  not introduce direct Rayon, crossbeam, global worker pools, ad-hoc threads, or
  queues.

## Planning Subagents

### @architect / Singer the 3rd

Agent id: `019ec604-e4aa-7893-a5be-23580b6e87cf`

Prompt: assess scope, touched files, risks, dependency/concurrency implications,
and blockers for adding checked local-reference evidence entries.

Status: complete. Conditional sign-off after documenting the metadata-only
boundary, fixing the initial placeholder spec delta shape, and keeping docs and
matrix text consistent with the manifest entries. No dependency or concurrency
changes required.

### @spec-reader / Pauli the 3rd

Agent id: `019ec605-00ae-7e81-a90f-47abf8fd9b73`

Prompt: inspect decoder-support specs and recommend the delta needed to add
real evidence entries without overclaiming runtime support.

Status: complete. Required the delta to modify the existing
`decoder-support` portable local-reference evidence manifest requirement rather
than adding a duplicate requirement. The delta spec now uses
`## MODIFIED Requirements` and preserves the full existing requirement with the
new scenarios appended.

### @reference-oracle with @avm-reader-runner / @dav2d-reader-runner / Carson the 3rd

Agent id: `019ec605-37e0-7982-8e37-437c9c578790`

Prompt: verify, without running AVM/dav2d or inspecting outside the repo,
whether archived agent logs contain enough portable AVM/dav2d evidence to
populate manifest entries for the 8-bit and 10-bit intra fixtures.

Status: complete. Confirmed the archived free-form evidence is sufficient only
for conservative raw MD5 reference-output metadata. Recommended recording exact
tool revisions and digest values while stating that exact local version output
and command arguments were not archived.

### @security-reviewer / Euclid the 3rd

Agent id: `019ec605-53eb-77f0-80e4-87cee37abcd8`

Prompt: assess security and repository-boundary risks of adding non-executable
AVM/dav2d digest metadata.

Status: complete. Signed off on non-executable metadata after stale docs and
matrix wording were updated so the manifest entries do not imply runtime decode,
reconstruction, hash computation, Y4M output, or external decoder invocation.

## PR #113 Review Carry-forward

The next decoder-mission PR must not regress feedback from PR #113.

- `discussion_r3409248110` flagged duplicated Annex B/IVF parser logic in the
  first byte planner. Current `main` drives `splot-core` streaming primitives
  (`AnnexBObuCursor` and `IvfFrameCursor`) from `splot-decode`, so parser logic
  remains single-sourced.
- Codex review `pullrequestreview-4492663492` flagged unsupported-structure
  precedence before later `max_obus` failures. Current `main` records
  `first_unsupported` in `crates/splot-decode/src/byte_stream.rs` and has
  focused regression coverage.
- The same review flagged IVF cursor retry stability for fatal first-frame
  header errors. Current `main` keeps public cursor retry behavior stable and
  covers it in `crates/splot-core/src/ivf.rs`.
- The same review flagged `decode_plan_bytes` valid seed prefixing. Current CI
  writes flag-prefixed seeds for `decode_plan_bytes` so byte zero is preserved
  for the planner input.
- The same review flagged stale `DecodeContext` docs. Current
  `crates/splot-decode/src/context.rs` documents raw Annex B/IVF byte planning.

This change does not edit those runtime, parser, fuzz, or CI surfaces. It will
carry these points in the PR description so review traceability is explicit.

## Local Evidence Source

Archived source:
`openspec/changes/archive/2026-06-13-decoder-roadmap-matrix-boundary/agent-log.md`

Recorded metadata:

- AVM commit: `f6f0b9c8914f38be39a953c0a9aa6a2e4050717c`.
- dav2d commit: `f4f96cb06bb3cd3f31e29e1f190f1c0e373ab352`.
- Built AVM binaries observed locally: `avmenc`, `avmdec`, `dump_obu`,
  `decode_to_md5`.
- Built dav2d binary observed locally: `dav2d`.
- Raw MD5 agreement:
  - `tests/conformance/vectors/valid/syn-key-intra-64x64.ivf`:
    `f2d45ae552bebe211f3156daf0a7fcf6`.
  - `tests/conformance/vectors/valid/syn-intra-64x64-10bit.ivf`:
    `6c9c31585f56bcc7ca40cfbb319f7bb5`.

No fresh AVM or dav2d command has been run for this change.

## Manifest Entries Added

- `lref-avm-dav2d-syn-key-intra-64x64` records raw reference-output MD5
  agreement for `tests/conformance/vectors/valid/syn-key-intra-64x64.ivf`.
- `lref-avm-dav2d-syn-intra-64x64-10bit` records raw reference-output MD5
  agreement for `tests/conformance/vectors/valid/syn-intra-64x64-10bit.ivf`.

Both entries are non-executable metadata and use repo-relative fixture paths,
fixture byte length, fixture SHA-256, upstream reference revisions, sanitized
command summaries, raw MD5 digest metadata, and equality assertions.

## Verification

- `cargo xtask check-reference-evidence`: passed.
- `cargo xtask check-decoder-support`: passed.
- `cargo xtask check-feature-status`: passed.
- `openspec validate local-reference-evidence-entries --strict`: passed.
- `openspec validate --all --no-interactive`: passed.
- `git diff --check`: passed.
- `cargo xtask ci`: passed.

Post-archive verification:

- `cargo xtask check-reference-evidence`: passed.
- `cargo xtask check-decoder-support`: passed.
- `cargo xtask check-feature-status`: passed.
- `openspec validate --all --no-interactive`: passed.
- `git diff --check`: passed.
- `cargo xtask ci`: passed.

## Final Review Subagents

### @spec-conformance-reviewer / Gauss the 3rd

Agent id: `019ec60c-1f75-7c00-b74e-777034092ba3`

Status: complete. No findings. Confirmed manifest entries match archived
AVM/dav2d commits and raw MD5 values, remain path-free and non-executable, and
label the digests as reference raw output rather than `splot-dfh-sha256-v1`.
Also confirmed `deterministic-frame-hash` remains `partial` and does not claim
runtime decode, reconstruction, hash computation, or Y4M support.

### @encoder-impact-reviewer / Pascal the 3rd

Agent id: `019ec60c-343b-70b2-a181-185cd5ab2e0e`

Status: complete. No findings. Confirmed the entries do not shape runtime APIs,
do not alter Cargo/dependency/runtime files, and leave decoder/encoder support
boundaries unchanged.

### @reviewer / Volta the 3rd

Agent id: `019ec60c-0b8b-7133-9007-a4d9f7a38b01`

Status: complete. No blocking findings. Verified both manifest entries are
coherent, fixture paths are tracked, SHA-256 and byte lengths match, digest
IDs/assertions validate, and MD5/tool revision values match the archived agent
log. Confirmed docs and matrices keep `deterministic-frame-hash` as `partial`
and do not overclaim `splot` runtime decode/hash/Y4M support.

### @security-reviewer / Wegener the 3rd

Agent id: `019ec60c-5c06-75f0-b808-c399f6b061b0`

Status: complete. No security findings. Confirmed the diff adds only
docs/matrix/OpenSpec metadata; no AVM/dav2d source, snippets, binaries,
submodules, dependencies, wrappers, build probes, scripts, CI jobs, xtask
implementations, or mandatory repo test paths were added. Also confirmed
manifest `command_summary`, `version_summary`, and `output_scope` fields are
portable and scoped to reference raw decoder MD5 rather than
`splot-dfh-sha256-v1`.
