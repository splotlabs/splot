## 1. OpenSpec Planning

- [x] 1.1 Create proposal, design, decoder-support delta, and agent log for `decode-stream-state-planner`.
- [x] 1.2 Validate the OpenSpec change with `openspec validate decode-stream-state-planner --strict` before creating a feature branch.

## 2. Stream Planner Implementation

- [x] 2.1 Add `splot-core` as the only new `splot-decode` dependency and keep dependency-direction checks valid.
- [x] 2.2 Add `crates/splot-decode/src/stream_plan.rs` with parsed-input planner types, base-layer selection, ordered plan metadata, IVF source context, and no raw payload exposure.
- [x] 2.3 Wire `DecodeContext::plan_stream` through the existing context-owned runtime model without direct Rayon/crossbeam/global-pool use.
- [x] 2.4 Extend `DecodeError` with typed local errors for source malformation, unsupported structures, and decode limit failures.

## 3. Tests

- [x] 3.1 Add unit tests for raw Annex B planning order, base-layer acceptance, frame-candidate counting, and metadata-only output.
- [x] 3.2 Add unit tests for IVF planning order, frame index/PTS/offset preservation, and warning retention.
- [x] 3.3 Add negative tests for malformed raw/IVF source transactionality, invalid xlayer scope, non-base layers, unsupported OBU types, and parser errors.
- [x] 3.4 Add limit tests for `max_input_bytes`, `max_obus`, `max_ivf_frame_records`, and `max_frames_to_decode`.
- [x] 3.5 Add deterministic planning tests across `ThreadCount::Auto`, `--threads 1`, and a fixed positive thread count.

## 4. Documentation And Matrix Sync

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` to describe the parsed stream planner, concurrency ownership, and raw-byte/fuzz deferral.
- [x] 4.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` and `docs/IMPLEMENTATION-MATRIX.toml` with `DECODE-STREAM-STATE-PLANNER`, proof commands, and non-goals.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md` if their inputs changed.

## 5. Verification And Review

- [x] 5.1 Run `openspec validate decode-stream-state-planner --strict` and `openspec validate --all --no-interactive`.
- [x] 5.2 Run focused Rust tests plus `cargo xtask check-dependency-direction`, `cargo xtask check-concurrency-policy`, `cargo xtask check-decoder-support`, `cargo xtask feature-status`, and `cargo xtask check-feature-status`.
- [x] 5.3 Run mandatory subagent review passes, fix or record every finding, and update `agent-log.md`.
- [x] 5.4 Run `cargo xtask ci`, archive the OpenSpec change, rerun gates, commit, push, and open a ready PR.
