# Agent Log: decode-byte-stream-review-fixes

## Orchestrator

- Model/effort: Codex GPT-5 local orchestration.
- Objective: address the four Codex review comments from PR #113 review
  `4492663492` in a follow-up PR before continuing the CLI byte-planner
  handoff.
- Process correction: PR #113 was merged before the Codex review for head
  `3066f4d` was complete. This change records the review findings and fixes
  them in the next PR. Future PRs must not merge on `eyes`, green CI, or a
  completed workflow alone; the latest head needs explicit Codex
  no-findings/thumbs-up/final sign-off or direct maintainer instruction.

## Review Findings Being Addressed

1. P2 `discussion_r3409278210`: preserve earlier unsupported-structure errors
   when later bytes would exceed `max_obus`.
2. P3 `discussion_r3409278211`: keep `IvfFrameCursor` state unchanged on
   fatal frame-header errors.
3. P3 `discussion_r3409278212`: preserve fixture bytes for
   `decode_plan_bytes` fuzz seeds.
4. P3 `discussion_r3409278215`: update `DecodeContext` docs for raw-byte
   planning.

## Subagents

### `@reference-oracle`: Lagrange the 3rd

- Agent id: `019ec554-675f-7420-b41d-a58780009c41`.
- Task: decide whether local AVM/dav2d evidence is needed for the pending CLI
  byte-planner handoff.
- Output reused here: no AVM/dav2d evidence is needed for planner/CLI handoff
  slices that do not reconstruct pixels, decode tile payloads, compute hashes,
  write Y4M, compare decoded output, or claim decoder conformance.

### `@architect`: Fermat the 3rd

- Agent id: `019ec553-b1d3-7dd1-8efb-5764f4534d66`.
- Task: inspect the planned CLI handoff design.
- Relevant output: keep decoder work within `DecodeContext` and
  `splot_parallel::WorkerPool`, no direct Rayon/crossbeam/thread/queue use.

### `@spec-reader`: Noether the 3rd

- Agent id: `019ec553-d51e-7602-a1eb-e810c2d05fa5`.
- Task: identify citations and matrix rows for the CLI follow-up.
- Relevant output: byte traversal/malformed raw Annex B cites §4.11.6,
  Annex B.2/B.3, §5.2.1, §5.2.2, and §6.2.1; unsupported OBU/layer behavior
  cites §5.2.1 and §6.2.2. IVF remains a non-normative container.

### `@api-designer`: Helmholtz the 3rd

- Agent id: `019ec553-f8e1-7b92-91ce-0cd05654db59`.
- Task: inspect future CLI diagnostic API shape.
- Relevant output: keep serialization in CLI, keep `splot-decode` diagnostics
  library-owned, and avoid new dependencies.

## Implementation Notes

- Synced the archived `decode-byte-stream-planner` delta into
  `openspec/specs/decoder-support/spec.md`.
- Moved the completed change to
  `openspec/changes/archive/2026-06-14-decode-byte-stream-planner/`.
- Updated `parse_bounded_annex_b_at` to classify retained OBU prefixes through
  `plan_stream`, preserving unsupported-structure errors before later tail
  traversal limits can mask them.
- Updated `IvfFrameCursor::next_frame_record()` so fatal first-frame-header
  errors leave cursor state unchanged and can be retried.
- Added `decode_plan_bytes` prefixed CI fuzz corpus seeds so byte zero remains
  the fuzz target's limit flag and the fixture bytes remain intact payloads.
- Updated `DecodeContext` docs to describe byte-consuming plan support without
  claiming decode output support.
- Addressed PR #114 review feedback on head `10748b2`: replaced the temporary
  full-prefix `plan_stream` recheck with a single-OBU unsupported-structure
  classifier, records the first unsupported structure during traversal, and
  uses that recorded unsupported result only to preserve precedence over later
  traversal limits. Later malformed parser errors still flow through
  `plan_stream` as `MalformedSource`.
- Updated `DecodeStreamInput` docs to describe the raw-byte planner handoff
  without leaving the earlier "future raw-byte planner" wording in public API
  docs.
- Updated the archived review-fix design record after PR #114 review so it no
  longer describes the superseded full-prefix replanning algorithm or the stale
  unsupported-before-malformed precedence.
- Addressed the PR #114 IVF cross-frame review finding by making `plan_stream`
  surface IVF frame payload parse errors before classifying any frame OBUs, and
  added a regression test for unsupported frame 0 plus malformed frame 1.
- Addressed the follow-up PR #114 parsed-IVF ordering review by checking parsed
  IVF OBU and frame-candidate limits in source order before later frame payload
  errors, while still deferring earlier unsupported structures until later
  malformed payloads have been observed.
- Addressed the follow-up PR #114 byte-IVF limit review by keeping
  `max_ivf_frame_records` as a typed `DecodeError::Limit` even when an earlier
  IVF frame payload recorded an unsupported OBU.

## Tests and Checks

- `openspec validate decode-byte-stream-review-fixes --strict`
- `openspec validate --all --no-interactive`
- `cargo test -p splot-core ivf --locked`
- `cargo test -p splot-decode --locked`
- `cargo xtask check-fuzz-targets`
- `cargo xtask check-dependency-direction`
- `cargo xtask check-concurrency-policy`
- `cargo xtask check-decoder-support`
- `cargo xtask check-diagnostic-registry`
- `cargo xtask check-feature-status`
- `cargo xtask feature-status`
- `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`
- `cargo clippy -p splot-core -p splot-decode --all-targets --locked -- -D warnings`
- `git diff --check`
- `cargo xtask ci`
- `cargo test -p splot-decode unsupported_prefix_is_reported_before_later_obu_limit --locked`
- `cargo test -p splot-core frame_cursor_retry_preserves_truncated_initial_frame_header_error --locked`
- `cargo test -p splot-decode malformed_suffix_is_reported_after_unsupported_prefix --locked`
- `cargo test -p splot-decode malformed_later_ivf_payload_wins_over_earlier_unsupported_obu --locked`
- `cargo test -p splot-decode parsed_ivf_obu_limits_win_before_later_payload_errors --locked`
- `cargo test -p splot-decode byte_ivf_record_limit_wins_over_earlier_unsupported_obu --locked`

## Review Sign-offs

### `@reviewer`: Boyle the 3rd

- Agent id: `019ec55d-6082-7ed1-a111-c42ce6a374b0`.
- Result: no findings. Checked unsupported-prefix ordering, IVF retry
  behavior, fuzz seed shape, `DecodeContext` docs, no CLI handoff, no output
  overclaim, and archived OpenSpec file movement.

### `@security-reviewer`: Hume the 3rd

- Agent id: `019ec55d-80d5-7c00-bfbd-8c706eda8b0a`.
- Result: no findings. Explicitly confirmed: No AVM/dav2d source, snippets,
  binaries, submodules, deps, build probes, wrappers, CI jobs, required
  scripts, required xtask commands, or mandatory tests were added.

### `@spec-conformance-reviewer`: Descartes the 3rd

- Agent id: `019ec55d-9c0a-7692-b698-8d590859f59a`.
- Result: no findings. Checked Annex B.2/B.3, §5.2.1, §5.2.2, §6.2.2, §7.1,
  non-normative IVF handling, unsupported metadata, and OpenSpec sync/archive.

### `@encoder-impact-reviewer`: James the 3rd

- Agent id: `019ec55d-ba4d-7162-8f9e-8437b86858b5`.
- Result: no findings. Confirmed future encoder-grade boundary and PR #101
  concurrency model remain respected.

## Local Reference Boundary

No AVM or dav2d evidence is used or needed for this change. It only fixes
planner behavior, cursor state, fuzz seed coverage, and docs. It does not
decode tile payloads, reconstruct pixels, compute decoded-frame hashes, write
Y4M, compare decoded output, or claim AV2 decoder conformance/parity.
