## 1. Tracking And Spec Mapping

- [x] 1.1 Add §5.20.6.1/§5.20.6.3 transform-size citation coverage to `docs/SPEC-MAPPING.md`.
- [x] 1.2 Add implementation-matrix and decoder-support rows for `DECODE-SELECTABLE-TRANSFORM-RECORDS`.
- [x] 1.3 Update the existing LR handoff row notes so they point at the selectable transform-record follow-on instead of claiming the local decoder mission stream still stops there.

## 2. Selectable Transform Records

- [x] 2.1 Add decoder-private data structures/helpers for supported selectable luma transform extents, middle/scan-order flags, and record coverage.
- [x] 2.2 Parse supported AV2 §5.20.6.1/§5.20.6.3 `TX_MODE_SELECT` transform records for the local decoder mission intra LR path.
- [x] 2.3 Feed selectable luma transform extents into the existing coefficient reader and `WienerNsLrTxSkipTransformRecord` derivation.
- [x] 2.4 Keep unsupported selectable transform syntax fail-closed with structured diagnostics and no partial live `LrTxSkip` population.

## 3. Tests And Verification

- [x] 3.1 Add focused unit tests for selectable transform geometry, live `LrTxSkip` population, and unsupported/incomplete-grid failures.
- [x] 3.2 Move the local decoder mission CLI regression past `unsupported_wienerns_lr_tx_mode_select_transform_records` to the next decoded-sample prerequisite.
- [x] 3.3 Run OpenSpec validation, feature/decoder-support checks, focused decoder/CLI tests, and `cargo xtask ci`.
