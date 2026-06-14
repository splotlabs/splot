# Agent Log: reference-evidence-cross-checks

## Orchestrator Plan

Objective: strengthen the checked local-reference evidence metadata contract by
making decoder-support matrix pointers resolve back to manifest entries, and by
requiring reciprocal row references.

Scope:

- OpenSpec capability: `decoder-support`.
- Feature ID: `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST`.
- Touched area: `xtask` validation, focused tests, OpenSpec archive.
- No AVM/dav2d execution from repo code, tests, scripts, `xtask`, CI, or build.
- No crate or dependency graph change.
- No runtime decode, reconstruction, hash computation, or Y4M output.
- PR #101 concurrency model remains unchanged; this automation-only change must
  not introduce direct Rayon, crossbeam, global worker pools, ad-hoc threads, or
  queues.

## Planning Subagents

### @architect

Agent id: `019ec61b-5254-7413-b941-c674697b3fb4`

Prompt: assess architecture and implementation risks for reciprocal
decoder-support matrix to local-reference evidence manifest pointer checks.

Status: complete. No architecture blockers after implementation fixes. Confirmed
the architecture is sound with manifest parsing in `reference_evidence.rs`, a
crate-internal checked evidence index, and reciprocal validation in
`check-decoder-support` before status drift comparison. Confirmed the PR #101
concurrency boundary and AVM/dav2d local-only boundary are preserved. Initial
findings on a stale malformed-pointer test failure and source-line growth were
resolved by fixing the test setup and splitting new link tests into
`xtask/src/decoder_support/link_tests.rs`; `xtask/src/decoder_support.rs` is now
under the 1000-line soft budget.

### @spec-reader

Agent id: `019ec61b-6b30-7a43-ab5c-2451889002cb`

Prompt: verify the `decoder-support` delta spec preserves the full existing
requirement and adds testable reciprocal pointer behavior without overclaiming
runtime support.

Status: complete. No blockers. Confirmed the delta uses `MODIFIED
Requirements`, preserves the full existing portable evidence manifest
requirement, and adds a testable reciprocal pointer scenario. Wording findings
were applied: the design now describes only row-to-manifest reciprocal drift,
and the spec says the check does not require `splot` deterministic frame-hash
computation.

### @api-designer

Agent id: `019ec61b-82e3-7923-a3a7-2c7e64b82723`

Prompt: recommend the smallest xtask-internal helper/API shape for resolving
canonical manifest pointers and reciprocal row references.

Status: complete. Recommended the implemented shape: keep `Manifest` and
`Evidence` private, expose a narrow checked `ReferenceEvidenceIndex` and
`canonical_evidence_pointer_id`, validate links inside `check-decoder-support`,
ignore free-form prose, and test valid, missing-ID, non-reciprocal, and
malformed canonical pointers.

### @security-reviewer

Agent id: `019ec61b-9933-7560-8e88-40e423b60f16`

Prompt: assess repository-boundary and security risks of adding metadata-only
cross-checks, including local path leakage and external decoder execution.

Status: complete. No security blockers. Confirmed the change is metadata-only
and should not add new process execution. The implementation adds no new
`Command::new`, no AVM/dav2d/ffmpeg/network execution, and no concurrency
primitive usage. Recommended malformed pointer and concurrency/process scans
were run.

## Implementation Notes

- Added `ReferenceEvidenceIndex` and
  `load_checked_reference_evidence_index` in `xtask/src/reference_evidence.rs`.
- Added `canonical_evidence_pointer_id` so the manifest pointer prefix remains
  single-sourced.
- Added `validate_local_reference_evidence_links` in
  `xtask/src/decoder_support.rs`; `check-decoder-support` now validates
  canonical manifest pointers before status drift comparison.
- Added focused link tests in `xtask/src/decoder_support/link_tests.rs`.

## Verification

- `openspec validate reference-evidence-cross-checks --strict`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo test -p xtask decoder_support --locked`: passed.
- `cargo test -p xtask reference_evidence --locked`: passed.
- `cargo xtask check-reference-evidence`: passed.
- `cargo xtask check-decoder-support`: passed.
- `cargo xtask check-feature-status`: passed.
- `cargo xtask check-concurrency-policy`: passed.
- `cargo xtask check-source-lines`: passed; new/changed files stay under the
  1000-line soft budget.
- `openspec validate --all --no-interactive`: passed.
- `git diff --check`: passed.
- Process/concurrency scan found no new `Command::new`, no decoder execution,
  and no new Rayon/crossbeam/thread/queue usage.
- `cargo xtask ci`: passed.

Post-archive verification:

- `cargo test -p xtask decoder_support --locked`: passed.
- `cargo test -p xtask reference_evidence --locked`: passed.
- `cargo xtask check-reference-evidence`: passed.
- `cargo xtask check-decoder-support`: passed.
- `cargo xtask check-feature-status`: passed.
- `cargo xtask check-source-lines`: passed; new/changed files stay under the
  1000-line soft budget.
- `cargo xtask check-concurrency-policy`: passed.
- `openspec validate --all --no-interactive`: passed.
- `git diff --check`: passed after removing archive-generated trailing blank
  line at EOF in the synced active spec.
- `cargo xtask ci`: passed.

## Final Review Subagents

### @reviewer / Hegel the 3rd

Agent id: `019ec623-d188-7393-9c74-28e11f247f62`

Status: complete. No findings. Confirmed reciprocal pointer validation is
correct, canonical manifest pointers are resolved only by ID, missing IDs and
non-reciprocal links fail, tests cover intended cases, changed Rust files are
under the source-line soft budget, and the worktree shape matches scope.

### @security-reviewer / Newton the 3rd

Agent id: `019ec623-ef24-7b21-ad32-d85424bf388c`

Status: complete. No security findings. Confirmed no new AVM/dav2d/ffmpeg or
network execution, no new process execution beyond the existing `git ls-files`
fixture tracking check, no pointer path/shell interpretation, no dependency,
script, CI, or runtime changes, and no PR #101 concurrency-model impact.

### @spec-conformance-reviewer / Locke the 3rd

Agent id: `019ec624-0b00-77d0-8b76-a76fb287fbed`

Status: complete. No spec-conformance findings. Confirmed the delta is
archive-shaped as a full modified requirement, preserves existing scenarios,
adds the reciprocal pointer scenario accurately, and does not overclaim decoder
runtime, reconstruction, deterministic hash, Y4M, AV2 conformance, or
AVM/dav2d/ffmpeg execution behavior.

### @encoder-impact-reviewer / Beauvoir the 3rd

Agent id: `019ec624-2205-76e0-9064-1a8d37692ded`

Status: complete. No findings. Confirmed the change is confined to xtask
evidence validation, does not touch `splot-decode`, `splot-recon`,
`splot-encode`, runtime APIs, Cargo manifests, or lockfiles, and helps future
decoder/encoder evidence hygiene without shaping runtime APIs.
