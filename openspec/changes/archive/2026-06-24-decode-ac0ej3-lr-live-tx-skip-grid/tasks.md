## 1. Live Storage

- [x] 1.1 Add `DECODE-AC0EJ3-LR-LIVE-TX-SKIP-GRID` constants and live
  tx-skip population helpers.
- [x] 1.2 Preserve fail-before-mutation behavior for dimension mismatches and
  re-population attempts.

## 2. Tests

- [x] 2.1 Add focused live-storage tests for successful grid population,
  dimension mismatch, and re-population rejection.
- [x] 2.2 Keep the live ac0ej3 diagnostic fail-closed and update focused CLI
  assertions only if the diagnostic reason changes.

## 3. Tracking

- [x] 3.1 Add the implementation matrix row, decoder support row, and generated
  support/status updates for `ac0ej3-lr-live-tx-skip-grid`.
- [x] 3.2 Validate OpenSpec and feature/support tracking.

## 4. Verification

- [x] 4.1 Run focused decode tests for live storage and tx-skip retention.
- [x] 4.2 Run `cargo xtask conformance`, `cargo xtask check-fixtures`, and
  `cargo xtask ci`.
