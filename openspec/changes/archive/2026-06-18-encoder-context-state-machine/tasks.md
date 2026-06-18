## 1. API and state model

- [x] 1.1 Add public lifecycle state/status types and typed invalid-state errors.
- [x] 1.2 Implement accepting, draining, finished, and failed transitions in `Context`.
- [x] 1.3 Preserve bounded input/output queue accounting without retaining borrowed sample data or emitting fake packets.
- [x] 1.4 Update `splot-cli` encode handling so lifecycle success is still reported as not implemented.

## 2. Tests and fuzzing

- [x] 2.1 Replace unimplemented-operation tests with deterministic lifecycle tests for receive-before-input, backpressure, flush, repeated flush, send-after-flush, end-of-stream, and failed state.
- [x] 2.2 Add a bounded `encoder_context_state_machine_bytes` fuzz target over lifecycle command sequences.
- [x] 2.3 Run targeted encoder and fuzz build/smoke checks.

## 3. Documentation and status

- [x] 3.1 Add the `ENC-CONTEXT-STATE-MACHINE` implementation-matrix row and regenerate status views.
- [x] 3.2 Sync the encoder API spec delta into `openspec/specs/encoder-api/spec.md`.
- [x] 3.3 Update encoder goal, roadmap, and gap audit wording so the lifecycle status is honest.

## 4. Validation, review, and archive

- [x] 4.1 Run OpenSpec validation, feature-status checks, formatting, and `cargo xtask ci`.
- [x] 4.2 Obtain local correctness/spec, security/zero-copy, determinism/concurrency, and test/evidence review reports and address findings.
- [x] 4.3 Archive the OpenSpec change after implementation and rerun validation/CI.
- [ ] 4.4 Open a PR with the Flight Manifest, proof commands, local reviewer decisions, and final GitHub Claude/Codex review checklist.
