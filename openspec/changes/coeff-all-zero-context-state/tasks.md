## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE` to the implementation matrix.
- [x] 1.2 Add `coeff-all-zero-context-state` to the decoder support matrix.
- [x] 1.3 Add the OpenSpec decoder-support delta.

## 2. Implementation

- [x] 2.1 Add a crate-private coefficient-loop foundation module that derives
  luma and V `all_zero` contexts from `TileCoeffContextState`.
- [x] 2.2 Wire the minimal block-symbol frontier to allocate coefficient context
  state from the tile work-unit and use the state-backed context reducers.
- [x] 2.3 Keep the existing fixture trace and output unchanged.

## 3. Verification

- [x] 3.1 Add focused unit tests for state-backed context reductions and
  pathological bounds.
- [x] 3.2 Run focused decoder tests and the full CI gate.
- [x] 3.3 Regenerate status/coverage docs.
