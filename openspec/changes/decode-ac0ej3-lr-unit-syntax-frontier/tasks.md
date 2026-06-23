## 1. Tracking

- [x] 1.1 Add `DECODE-AC0EJ3-LR-UNIT-SYNTAX-FRONTIER` to the implementation matrix.
- [x] 1.2 Add decoder support/status coverage for `ac0ej3-lr-unit-syntax-frontier`.
- [x] 1.3 Update traversal/frontier notes so non-goals remain explicit.

## 2. Implementation

- [x] 2.1 Add `TileUseWienerNsCdf` row ownership and tests to the tile CDF subset.
- [x] 2.2 Implement narrow frame-level Wiener NS LR unit symbol consumption before partition traversal.
- [x] 2.3 Replace the current parsed-bank runtime frontier with a post-LR-unit structured unsupported diagnostic.

## 3. Verification

- [x] 3.1 Add focused positive and negative tests for LR unit symbol traversal.
- [x] 3.2 Add diagnostic identity and local ac0ej3 frontier tests.
- [x] 3.3 Run `openspec validate decode-ac0ej3-lr-unit-syntax-frontier --no-interactive`, feature/support checks, conformance, and `cargo xtask ci`.
