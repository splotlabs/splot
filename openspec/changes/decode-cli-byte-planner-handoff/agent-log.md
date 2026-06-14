# Agent Log: decode-cli-byte-planner-handoff

## Orchestrator Plan

- Branch: `codex/decode-cli-planner-handoff`.
- Change: `decode-cli-byte-planner-handoff`.
- Goal-owned PR only; unrelated user PRs do not block this work.
- Ready PRs only. Do not create a draft PR unless Bartosz explicitly asks for
  one.
- Merge gate: request Codex review after every pushed head update and wait for
  explicit latest-head no-findings/thumbs-up/final sign-off. Treat `eyes` as
  in-progress only. Green CI alone is not approval.
- Incorporate the PR #101 concurrency model: CLI decode must construct
  `DecodeContext` and call `DecodeContext::plan_bytes`; any future parallel
  decode work must run through the context-owned `splot_parallel::WorkerPool`.
  No direct Rayon, crossbeam, global pool, ad-hoc threads, or queues.
- AVM/dav2d boundary: this slice must not locate, build, run, wrap, depend on,
  or commit metadata from AVM/dav2d. It is planner/diagnostic handoff only, not
  decoded-output evidence.

## Agents Invoked

| Agent | Role | Objective | Output |
|---|---|---|---|
| @architect | Planning subagent | Design CLI handoff architecture and concurrency boundary. | Complete: plan-first CLI handoff through `DecodeContext::plan_bytes`; no direct pool use or new concurrency dependency in CLI. |
| @spec-reader | Planning subagent | Extract pinned AV2 spec citations and diagnostic requirements. | Complete: `decode/malformed-source` for parser/container failures, `decode/resource-limit` for planner limits, `decode/unsupported-feature` for planner unsupported and runtime deferral with distinct matrix metadata. |
| @api-designer | Planning subagent | Recommend CLI/library diagnostic API shape and tests. | Complete: keep diagnostics library-owned, keep serde JSON CLI-owned, add stable source issue strings, and cover text/JSON/no-touch behavior. |
| @reference-oracle | Reference subagent | Decide whether local AVM/dav2d evidence is needed and audit boundary. | Complete: not applicable. No AVM/dav2d commands, path probing, metadata, wrappers, tests, scripts, deps, or CI hooks needed for this planner/diagnostic slice. |

## PR #113 Review Carry-Forward

- Codex review URL:
  <https://github.com/splotlabs/splot/pull/113#pullrequestreview-4492663492>.
- `discussion_r3409278210`: byte planning must preserve earlier unsupported
  structures when a later OBU limit is exceeded. Current `main` already records
  and returns the first unsupported structure in `byte_stream.rs`; this PR keeps
  the non-regression covered.
- `discussion_r3409278211`: IVF `next_frame_record()` must not advance cursor
  state on fatal first-frame-header errors. Current `main` no longer marks the
  cursor finished for that error; this PR does not weaken it.
- `discussion_r3409278212`: `decode_plan_bytes` fuzz seeds must preserve valid
  fixture paths despite the leading limit-policy byte. Current CI seed logic
  writes prefixed `decode_plan_bytes` seeds; this PR does not change it.
- `discussion_r3409278215`: `DecodeContext` docs must mention raw-byte
  planning. Current docs do; this PR keeps CLI use aligned with those docs.

## PR #116 Codex Review Follow-Up

- Codex review URL:
  <https://github.com/splotlabs/splot/pull/116#pullrequestreview-4492824905>.
- `discussion_r3409441530`: `splot decode` must enforce finite
  `max_input_bytes` before reading an entire input into memory. This follow-up
  adds a decode-only bounded file reader that rejects oversized regular files
  from metadata and otherwise reads at most `limit + 1` bytes before emitting
  the existing `decode/resource-limit` diagnostic.
- `discussion_r3409441531`: Annex B parser/container failures must not be
  mis-cited to OBU syntax `§ 5.2.1`. This follow-up leaves malformed-source
  `spec_section` unset when the source issue cannot be attributed to one precise
  AV2 section; policy-only `max_input_bytes` also leaves `spec_section` unset.

## Implementation Notes

- Use `DecodeContext::new(DecodeRuntimeConfig::new(args.threads))` and then
  `DecodeContext::plan_bytes(&bytes, DecodeOptions::default())`.
- Enforce `DecodeOptions::default().limits().max_input_bytes()` before
  constructing the full CLI input buffer. Oversized inputs are reported through
  the same library-owned `decode/resource-limit` adapter as planner limits.
- Do not write output artifacts in this slice. Output path resolution exists
  only for CLI argument validation and future artifact selection.
- Missing input remains an operational read error with exit code `2`, not a
  `decode/*` diagnostic.
- Runtime deferral after successful planning must not be described as decode
  success. Wording: byte stream planning succeeded; runtime decode/output
  remains unsupported.
- Reference evidence: not applicable. Proof is self-contained via CLI/library
  tests, diagnostic-registry checks, decoder-support checks, feature-status
  checks, and existing byte-planner/fuzz build coverage.

## Tests and Checks

- `openspec validate decode-cli-byte-planner-handoff --no-interactive`: passed.
- `openspec validate --all --no-interactive`: passed.
- `cargo test -p splot-decode --locked`: passed.
- `cargo test -p splot-cli --test decode_cli --locked`: passed.
- `cargo xtask check-diagnostic-registry`: passed; decoder registry has 3
  emitted IDs.
- `cargo xtask check-decoder-support`: passed.
- `cargo xtask feature-status`: passed.
- `cargo xtask check-feature-status`: passed after regenerating
  `docs/SPEC-COVERAGE.md`.
- `cargo xtask check-concurrency-policy`: passed.
- `cargo xtask check-dependency-direction`: passed.
- `cargo xtask ci`: passed.

## Review Sign-offs

- Pending.
