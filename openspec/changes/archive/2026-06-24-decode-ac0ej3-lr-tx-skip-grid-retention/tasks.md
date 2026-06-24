## 1. Implementation

- [x] 1.1 Add decoder-local transform-record types and a complete-grid `LrTxSkip` retention helper.
- [x] 1.2 Keep the live ac0ej3 diagnostic fail-closed without claiming tile-populated grid values.

## 2. Tests

- [x] 2.1 Add focused positive and negative tests for grid derivation, missing-cell rejection, and out-of-range record rejection.
- [x] 2.2 Verify the local ac0ej3 diagnostic still reaches the current unsupported runtime gate.

## 3. Tracking And Validation

- [x] 3.1 Add `DECODE-AC0EJ3-LR-TX-SKIP-GRID-RETENTION` to the implementation matrix and decoder support matrix/status.
- [x] 3.2 Run OpenSpec, feature-status, decoder-support, focused tests, and the repository CI gate.
