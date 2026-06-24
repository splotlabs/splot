## 1. Narrow Luma Admission

- [x] 1.1 Split selectable transform-record block-shape admission so luma-only nonzero leaves are allowed while unsupported chroma-bearing narrow leaves remain fail-closed.
- [x] 1.2 Add focused transform-record tests that pin the observed luma-only `BLOCK_4X32` case and preserve the chroma/no-fabrication guard.

## 2. ac0ej3 Runtime Handoff

- [x] 2.1 Verify the local ac0ej3 probe advances past `unsupported_wienerns_lr_selectable_transform_records_block_shape`.
- [x] 2.2 Update structured diagnostics or CLI regression expectations to name the new live frontier without claiming output.

## 3. Tracking And Verification

- [x] 3.1 Add `DECODE-AC0EJ3-SELECTABLE-NARROW-LUMA-RECORDS` to the implementation and decoder-support matrices, update spec mapping if needed, and regenerate generated status docs.
- [x] 3.2 Run focused tests, the local ac0ej3 decode probe, `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, and `cargo xtask ci`.
