## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-TX-CLASS-DERIVE` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `coeff-tx-class-derive`.

## 2. Implementation

- [x] 2.1 Add a crate-private decode-local `PlaneTxType -> CoeffTransformClass` helper.
- [x] 2.2 Add a max-level handoff that derives `txClass` from `PlaneTxType` before delegating to the existing max-level path.
- [x] 2.3 Add focused tests for class mapping and max-level handoff equivalence.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, and the Rust acceptance gate.
