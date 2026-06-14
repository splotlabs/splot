## 1. Planning And Boundaries

- [x] 1.1 Validate the OpenSpec proposal, design, spec delta, tasks, and agent log for `recon-intra-dc-prediction`.
- [x] 1.2 Record orchestrator, spec-reader, API-designer, reference-boundary, and backlog-auditor findings in `agent-log.md`.
- [x] 1.3 Confirm active unrelated OpenSpec changes are not modified and AVM/dav2d remain local-only/non-executable.

## 2. Square DC Prediction Implementation

- [x] 2.1 Add `splot-recon` square DC prediction types and public exports for `RECON-INTRA-DC-SQUARE-PREDICTION`.
- [x] 2.2 Implement square DC prediction for both-edge, left-only, above-only, and no-edge cases using AV2 §7.13.2.10 source-backed rounding.
- [x] 2.3 Add typed `ReconError` variants for invalid block size, edge length mismatch, prediction sample range, sample storage conversion, output shape, and prediction allocation failure.
- [x] 2.4 Keep the implementation scheduler-free with no new dependencies, no decode/CLI wiring, and no `splot-*` dependency graph change.

## 3. Tests

- [x] 3.1 Add positive unit tests for both-edge square DC, left-only, above-only, and no-edge prediction.
- [x] 3.2 Add negative tests for unsupported log2 size, missing or wrong-length edge samples, sample type/bit-depth mismatch, out-of-range edge samples, output shape, storage conversion, and checked allocation behavior.
- [x] 3.3 Add tests proving output block geometry and sample storage remain stable and visible to future frame/workspace integration.

## 4. Documentation And Status

- [x] 4.1 Add `RECON-INTRA-DC-SQUARE-PREDICTION` to `docs/IMPLEMENTATION-MATRIX.toml` with self-contained proof.
- [x] 4.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` and `docs/DECODER-ROADMAP.md` so square DC prediction is supported while full scalar intra reconstruction remains incomplete.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.
- [x] 4.4 Confirm no AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers, scripts, CI jobs, required `xtask` commands, or mandatory tests were added.

## 5. Validation And Review

- [x] 5.1 Run `openspec validate recon-intra-dc-prediction --strict`.
- [x] 5.2 Run focused `splot-recon` tests and lints.
- [x] 5.3 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-concurrency-policy`, and `cargo xtask check-dependency-direction`.
- [x] 5.4 Run `cargo xtask ci`.
- [x] 5.5 Complete required review-agent passes, fix or document every finding, and update `agent-log.md`.

## 6. Archive And PR

- [x] 6.1 Archive the completed OpenSpec change and verify the delta folded into `openspec/specs/`.
- [ ] 6.2 Commit, push, and open a ready PR. Do not create a draft PR unless directly requested.
- [ ] 6.3 Wait for CI green and latest-head Codex review completion before any merge action; an `eyes` reaction is only in-progress acknowledgement.
