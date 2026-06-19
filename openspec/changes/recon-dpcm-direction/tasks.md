## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DPCM-DIRECTION` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `dpcm_direction(use_dpcm, mode_is_v_pred) -> Option<DpcmDirection>` to `transform_params.rs`, mapping the § 7.15.4 cases (None / Vertical for V_PRED / Horizontal otherwise).
- [x] 2.2 Keep it a total `const fn` taking caller-resolved scalars (no frame state or prediction-mode enum); export it and update the module and crate docs (which listed this as a future row).

## 3. Tests

- [x] 3.1 Add the four-spec-case mapping test (three pinned at compile time as `const` spec contracts).
- [x] 3.2 Add an integration test driving the § 7.15.4 outer transform through a lossless IDTX block so the selected Vertical direction produces the per-column cumulative sum and None leaves the residual flat.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency-direction, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-dpcm-direction --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
