## 1. Tracking

- [x] 1.1 Add `DECODE-AC0EJ3-LR-UNIT-SELECTIONS-FRONTIER` to the implementation matrix.
- [x] 1.2 Add decoder support/status coverage for `ac0ej3-lr-unit-selections-frontier`.

## 2. Implementation

- [x] 2.1 Add a crate-private LR-unit selection record to the root frontier.
- [x] 2.2 Record plane and absolute unit coordinates while consuming supported
      frame-level Wiener NS LR-unit symbols.
- [x] 2.3 Preserve existing aggregate counters, inactive helper, runtime active
      unit diagnostic, and resource-limit behavior.

## 3. Verification

- [x] 3.1 Add focused tests for inactive, active, and multi-unit selection state.
- [x] 3.2 Run `openspec validate decode-ac0ej3-lr-unit-selections-frontier --no-interactive`,
      focused decode tests, feature/support checks, conformance, and `cargo xtask ci`.
