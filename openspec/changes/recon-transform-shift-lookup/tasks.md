## 1. Spec + matrix

- [x] 1.1 Add the `RECON-TRANSFORM-SHIFT-LOOKUP` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `transform-shift-lookup` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the row + feature id to the `reconstruction-transform-and-filters`
  coverage group in `xtask/src/decoder_conformance_coverage.rs` and refresh its notes.
- [x] 1.4 Add the OpenSpec `decoder-support` delta for this change.

## 2. Implementation

- [x] 2.1 Add `crates/splot-recon/src/transform_params.rs` with the verbatim
  § 7.15.4 `Transform_Shift[25][2]` constant, the parallel § 9.2 `(log2W, log2H)`
  key table, and `transform_shift(log2_width, log2_height)`.
- [x] 2.2 Add the typed `ReconError::InvalidTransformShiftShape` variant + `Display` arm.
- [x] 2.3 Register the module + export `transform_shift` in `crates/splot-recon/src/lib.rs`
  and update the crate `//!` implemented/not-implemented lists and feature-id list.

## 3. Tests

- [x] 3.1 Per-shape table-faithfulness, key-uniqueness, independently-transcribed
  spec spot values, transpose-symmetry, and non-AV2-shape rejection tests.

## 4. Docs + gate

- [x] 4.1 Update `docs/DECODER-ROADMAP.md`.
- [x] 4.2 Regenerate the four generated status docs.
- [x] 4.3 `cargo xtask ci` green.
