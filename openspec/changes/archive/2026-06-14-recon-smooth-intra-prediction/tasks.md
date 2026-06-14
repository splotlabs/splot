## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Record planning subagent findings and PR #113 review carry-forward in `agent-log.md`.
- [x] 1.3 Add or update `RECON-INTRA-SMOOTH-PREDICTION` in the implementation matrix and decoder support matrix.

## 2. Reconstruction Implementation

- [x] 2.1 Add a new scheduler-free `splot-recon` smooth intra prediction module without changing crate dependency direction.
- [x] 2.2 Add allocation-free caller-owned rectangular smooth prediction APIs over prepared left/above edge samples and explicit smooth mode selection.
- [x] 2.3 Add typed validation for edge lengths, edge sample ranges, unsupported sample storage, computed output range, output stride, output length, and missing workspace prepared edges.
- [x] 2.4 Add current-frame workspace smooth prediction helpers that remain scheduler-free and avoid AV2 availability/fallback policy.

## 3. Tests

- [x] 3.1 Add smooth prediction tests covering `SMOOTH_PRED`, `SMOOTH_V_PRED`, and `SMOOTH_H_PRED` with non-uniform edges.
- [x] 3.2 Add invalid input tests for edge lengths, sample range, storage type, computed output range, stride, output length, and missing workspace prepared edges.
- [x] 3.3 Add workspace tests for in-storage smooth prediction and typed boundary failure.
- [x] 3.4 Run focused `splot-recon` tests plus dependency and concurrency checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [x] 4.2 Run mandatory review subagents and record sign-offs/findings in `agent-log.md`.
- [x] 4.3 Run `openspec validate recon-smooth-intra-prediction --strict`, archive the change, and run required local gates before commit/PR.
- [ ] 4.4 Create a ready PR only; do not create a draft PR.
- [ ] 4.5 After the final commit, request Codex review and wait for completed latest-head review before merge.
