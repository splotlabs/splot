## 1. Source Implementation

- [x] 1.1 Add `DECODE-BYTE-STREAM-PLANNER` to `docs/IMPLEMENTATION-MATRIX.toml`
  with planned source modules, scope, and proof commands.
- [x] 1.2 Add internal bounded Annex B/IVF traversal in
  `crates/splot-decode/src/byte_stream.rs` using public `splot-core`
  primitives and no new dependencies.
- [x] 1.3 Add `DecodeContext::plan_bytes(&[u8], DecodeOptions)` that runs the
  byte planner inside the context-owned `WorkerPool`.
- [x] 1.4 Keep the existing parsed-input `DecodeContext::plan_stream` behavior
  and CLI unsupported behavior unchanged.

## 2. Tests and Fuzzing

- [x] 2.1 Add positive raw Annex B and IVF tests proving `plan_bytes` matches
  representative parsed-input plans and preserves offsets/source metadata.
- [x] 2.2 Add negative and EOF/parser-edge tests for malformed raw Annex B, IVF
  container errors, IVF frame payload parse errors, unsupported structures, and
  transactional no-partial-plan behavior.
- [x] 2.3 Add limit tests proving `max_input_bytes`, `max_obus`,
  `max_ivf_frame_records`, and `max_frames_to_decode` are enforced through the
  byte planner.
- [x] 2.4 Add thread-policy tests proving deterministic `plan_bytes` output for
  `ThreadCount::Auto`, `1`, and a fixed non-zero count.
- [x] 2.5 Add `fuzz/fuzz_targets/decode_plan_bytes.rs` and wire it into
  `fuzz/Cargo.toml` without requiring AVM/dav2d or new external dependencies.

## 3. Documentation and Matrices

- [x] 3.1 Update `docs/DECODER-ROADMAP.md`, `docs/TESTING.md`, and
  `docs/SPEC-MAPPING.md` for the byte-consuming planner and fuzz target.
- [x] 3.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` and regenerate/check
  `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 3.3 Run and record `cargo xtask feature-status` and
  `cargo xtask check-feature-status`.
- [x] 3.4 Keep `openspec/changes/decode-byte-stream-planner/agent-log.md`
  updated with agent outputs, implementation notes, tests, and review findings.

## 4. Review and Gates

- [x] 4.1 Run OpenSpec validation for this change and all specs.
- [x] 4.2 Run focused Rust checks for `splot-decode`, fuzz metadata, decoder
  support, dependency direction, and concurrency policy.
- [x] 4.3 Run `cargo xtask ci` before PR.
- [x] 4.4 Invoke implementation/test/documentation/review agents and resolve all
  findings.
- [ ] 4.5 Open a ready PR, request Codex review, and do not merge until Codex
  gives explicit approval/thumbs-up or final review sign-off.
