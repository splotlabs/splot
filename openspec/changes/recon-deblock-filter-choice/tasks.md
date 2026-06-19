## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DEBLOCK-FILTER-CHOICE` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `deblock_filter_choice` and `DeblockFilterChoice` transcribing the § 7.17.7.2 cascade (zero-threshold return, secondDeriv estimate, threshold cascade, and the per-distance loop) over caller-resolved `s` / `t` lines, widths, thresholds, and the `Q_First` array.
- [x] 2.2 Transcribe the asymmetric directional-derivative gradient term exactly; keep the function total and panic-free (validated widths and line lengths, a window-bounded sample window, and a fixed-size `Q_First`); export the items and update the crate and module `//!` docs.

## 3. Tests

- [x] 3.1 Add hand-anchored deterministic tests (flat → full width, high curvature → 0, widths 1 and 3, zero thresholds → 0, invalid width and short line error).
- [x] 3.2 Add a property test comparing the function against an independent in-test re-trace of the spec pseudocode over varied samples, widths, and thresholds.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, source-lines, dependency-direction, and decoder-support checks, and an adversarial spec re-trace.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-deblock-filter-choice --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
