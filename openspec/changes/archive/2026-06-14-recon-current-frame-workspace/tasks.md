## 1. OpenSpec and Planning

- [x] 1.1 Validate proposal, design, spec delta, tasks, and planning log before
      creating an implementation branch.
- [x] 1.2 Record planning subagent findings, PR #113 Codex review carry-forward
      status, and PR #101 concurrency boundary in `agent-log.md`.
- [x] 1.3 Add `RECON-CURRENT-FRAME-WORKSPACE` matrix/status/docs entries without
      claiming runtime decode output.

## 2. Workspace Implementation

- [x] 2.1 Add `splot-recon` workspace module, typed errors, crate docs, and
      public exports.
- [x] 2.2 Implement checked workspace allocation from `DecodedFrameInfo` with
      fallible plane buffers, fill-sample validation, and no scheduler state.
- [x] 2.3 Implement bounded plane metadata, sample access, fill, and rectangular
      write helpers.
- [x] 2.4 Implement edge extraction and square DC prediction write helpers using
      the existing `RECON-INTRA-DC-SQUARE-PREDICTION` primitive.
- [x] 2.5 Implement freeze into immutable `DecodedFrame<T>` through existing
      plane/frame validation paths.

## 3. Tests and Documentation

- [x] 3.1 Add positive unit tests for allocation, writes, edge extraction, square
      DC prediction writes, freeze, hash, Y4M, and reference-store interop.
- [x] 3.2 Add negative unit tests for overflow/allocation planning, missing
      chroma planes, bounds, shape mismatch, unsupported sample type, and
      out-of-range samples.
- [x] 3.3 Update decoder support docs, implementation matrix, generated status
      docs, and testing/roadmap text as needed.

## 4. Review and Gates

- [x] 4.1 Run focused `splot-recon` tests, source-line checks, feature-status
      checks, dependency/concurrency checks, and OpenSpec validation.
- [x] 4.2 Run required subagent review, fix or explicitly record findings, and
      keep the PR #113 review carry-forward closed.
- [x] 4.3 Prepare archive handoff. After archive, continue the mission process:
      run `cargo xtask ci`, create a ready PR, trigger Codex review, and wait
      for the latest-head completed review before any merge.
