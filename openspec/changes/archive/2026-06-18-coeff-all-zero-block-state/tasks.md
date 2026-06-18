## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ALL-ZERO-BLOCK-STATE` to the implementation matrix.
- [x] 1.2 Add `coeff-all-zero-block-state` to the decoder support matrix.
- [x] 1.3 Add the OpenSpec decoder-support delta.

## 2. Implementation

- [x] 2.1 Extend transform coefficient block state with zeroed `Quant[]` storage
  and checked accessors.
- [x] 2.2 Add a crate-private all-zero coefficient-block state helper in
  `coeff_loop.rs`.
- [x] 2.3 Wire the minimal block-symbol frontier to apply all-zero state effects
  after the luma and V all-zero symbol reads.
- [x] 2.4 Keep the existing fixture trace and output unchanged.

## 3. Verification

- [x] 3.1 Add focused unit tests for `Quant[]` state, all-zero context writes,
  typed failures, and minimal trace no-output-change behavior.
- [x] 3.2 Run focused decoder tests and the full CI gate.
- [x] 3.3 Regenerate status/coverage docs.
