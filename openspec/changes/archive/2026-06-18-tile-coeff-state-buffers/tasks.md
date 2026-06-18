## 1. Spec + matrix

- [x] 1.1 Add `DECODE-TILE-COEFF-STATE-BUFFERS` to
  `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `tile-coeff-state-buffers` row to
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Link the new row and Feature ID from decoder conformance coverage.
- [x] 1.4 Keep roadmap/status notes honest that `coeffs()` and reconstruction are
  still unwired.

## 2. Implementation

- [x] 2.1 Add `crates/splot-decode/src/tile_payload/coeff_state.rs` with bounded
  transform-block `Level[]` / `QuantSign[]` state.
- [x] 2.2 Add tile-neighbour above/left level and DC-context line state for three
  planes.
- [x] 2.3 Add checked end-of-`coeffs()` update and block-context reset helpers.
- [x] 2.4 Wire the module into `tile_payload.rs` without changing runtime decode
  behavior.

## 3. Tests

- [x] 3.1 Cover valid transform-block allocation, read/write views, and
  zero-initialization.
- [x] 3.2 Cover invalid dimensions, coordinate/plane errors, and allocation-size
  accounting.
- [x] 3.3 Cover §5.20.7.27 above/left level/DC context updates and reset ranges,
  including bounded pathological counts.

## 4. Gate

- [x] 4.1 Regenerate generated status docs.
- [x] 4.2 Run OpenSpec validation and feature-status checks.
- [x] 4.3 Run targeted `splot-decode` tests.
- [x] 4.4 Run `cargo xtask ci`.
