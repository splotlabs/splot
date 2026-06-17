## 1. Spec + matrix

- [x] 1.1 Add the `RECON-GET-TX-CLASS` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `get-tx-class` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the row + feature id to the `reconstruction-transform-and-filters`
  coverage group in `xtask/src/decoder_conformance_coverage.rs`.
- [x] 1.4 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add the `tx_class(plane_tx_type)` `const fn` to `coefficient_scan.rs`
  (V_DCT/V_ADST/V_FLIPADST -> Vertical, H_DCT/H_ADST/H_FLIPADST -> Horizontal,
  else -> TwoD), reusing the existing `TransformClass` enum.
- [x] 2.2 Export `tx_class` in `lib.rs`; update the crate `//!` implements list +
  feature-id list.

## 3. Tests

- [x] 3.1 Exhaustive small-domain test pinning every vertical (10/12/14),
  horizontal (11/13/15), and 0..=9 value to its class plus two out-of-range
  inputs, and a `const` compile-time assertion.

## 4. Docs + gate

- [x] 4.1 Update `docs/DECODER-ROADMAP.md`.
- [x] 4.2 Regenerate the four generated status docs.
- [x] 4.3 `cargo xtask ci` green.
