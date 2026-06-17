## 1. Spec + matrix

- [x] 1.1 Add the `RECON-GET-TRANSFORM-1D-TYPE` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `get-transform-1d-type` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the row + feature id to the `reconstruction-transform-and-filters`
  coverage group in `xtask/src/decoder_conformance_coverage.rs` and refresh its notes.
- [x] 1.4 Add the OpenSpec `decoder-support` delta for this change.

## 2. Implementation

- [x] 2.1 Add the verbatim § 7.15.4 `Transform_1d_Type[16][2]` constant, the
  `TransformPass` enum, and `get_transform_1d_type(...)` (with the `useDdt`
  substitution) to `crates/splot-recon/src/transform_params.rs`.
- [x] 2.2 Add the typed `ReconError::InvalidPlaneTxType` variant + `Display` arm.
- [x] 2.3 Export `get_transform_1d_type` + `TransformPass` in
  `crates/splot-recon/src/lib.rs` and update the crate `//!` lists + feature ids.

## 3. Tests

- [x] 3.1 Per-`PlaneTxType` base-table faithfulness (both passes), the `useDdt`
  substitution (eligible and ineligible cases), and the out-of-range rejection;
  plus a compile-time const-eval check.

## 4. Docs + gate

- [x] 4.1 Update `docs/DECODER-ROADMAP.md`.
- [x] 4.2 Regenerate the four generated status docs.
- [x] 4.3 `cargo xtask ci` green.
