## 1. Tracking

- [x] 1.1 Add `DECODE-AC0EJ3-LR-SOURCE-BOUNDS-FRONTIER` to the implementation matrix.
- [x] 1.2 Add decoder support/status coverage for `ac0ej3-lr-source-bounds-frontier`.

## 2. Implementation

- [x] 2.1 Add crate-private active LR source-block/source-bound facts to the root frontier.
- [x] 2.2 Add Wiener NS per-unit filter syntax CDF rows and consume §5.20.10.6 before source-bound retention.
- [x] 2.3 Derive source-bound facts from supported Wiener NS LR units.
- [x] 2.4 Derive tile-clamped source bounds when loop filters are disabled across tiles.
- [x] 2.5 Move the live ac0ej3 runtime diagnostic to the source-bounds frontier.

## 3. Verification

- [x] 3.1 Add focused tests for active source blocks, stripe bounds, inactive units, CDF defaults, tile-clamped bounds, and §5.20.10.6 merged-unit bank reuse ordering.
- [x] 3.2 Run `openspec validate decode-ac0ej3-lr-source-bounds-frontier --no-interactive`,
      focused decode tests, feature/support checks, conformance, and `cargo xtask ci`.
