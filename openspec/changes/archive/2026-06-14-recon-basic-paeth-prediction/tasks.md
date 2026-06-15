## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add or update `RECON-INTRA-BASIC-PAETH-PREDICTION` in the implementation matrix and decoder support matrix.
- [x] 1.3 Run planning/spec/API subagents and record findings in `agent-log.md`.

## 2. Reconstruction Implementation

- [x] 2.1 Add a new `splot-recon` basic/PAETH intra prediction module without growing `intra.rs` over the source-line budget.
- [x] 2.2 Add allocation-free caller-owned rectangular PAETH prediction APIs over prepared left/above/top-left samples.
- [x] 2.3 Add typed validation for edge lengths, edge sample ranges, unsupported sample storage, output stride, output length, and missing workspace edges.
- [x] 2.4 Add current-frame workspace PAETH prediction helpers that remain scheduler-free and avoid AV2 availability policy.

## 3. Tests

- [x] 3.1 Add PAETH tests covering left, above, and top-left candidate selection.
- [x] 3.2 Add invalid input tests for edge lengths, sample range, storage type, stride, output length, and missing workspace edges.
- [x] 3.3 Add workspace tests for in-storage PAETH prediction and typed boundary failure.
- [x] 3.4 Run focused `splot-recon` tests plus dependency and concurrency checks.

## 4. Documentation And Review

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [x] 4.2 Run mandatory review subagents and record sign-offs/findings in `agent-log.md`.
- [x] 4.3 Run `openspec validate recon-basic-paeth-prediction --strict`, archive the change, and run required local gates before commit/PR.
