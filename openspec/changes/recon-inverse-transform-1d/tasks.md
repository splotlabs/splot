## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-INVERSE-TRANSFORM-1D` to the implementation matrix and decoder support matrix.

## 2. Reconstruction Implementation

- [x] 2.1 Add the `splot-recon -> splot-tables` dependency edge (Cargo.toml, `INTERNAL_DEP_RULES`, docs).
- [x] 2.2 Add `InverseTransform1dType` (§ 7.15.4.1 Table 7.1 kernel types; `IDT` excluded).
- [x] 2.3 Implement `inverse_transform_1d` per § 7.15.2.1: kernel matrix-multiply, § 4.8 `Round2`, § 7.15.2.1 `colTx`-dependent `Clip3`, with the exact length-4/length-32/`Fddt` dispatch and `i64` accumulation.
- [x] 2.4 Add typed `ReconError` variants for invalid length and length mismatch; export the public items and update docs.

## 3. Tests

- [x] 3.1 Add spec-exact vectors (DC flat field, single-coefficient kernel row, `Round2` downshift, both `colTx` clamp ranges).
- [x] 3.2 Add the `Fddt`-reverses-`Ddtx`, length-4 FDST-fallback, and length-32 DCT-for-every-type property tests plus the two typed-error cases.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency, concurrency, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-inverse-transform-1d --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
