## 1. Tracking

- [x] 1.1 Add `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` to the implementation matrix.
- [x] 1.2 Add decoder support/status coverage for `ac0ej3-lr-source-read-frontier`.

## 2. Implementation

- [x] 2.1 Add crate-private source-read frontier state for active Wiener NS LR source blocks.
- [x] 2.2 Reuse `splot-recon` loop-restoration source selection/read primitives for supported output, tap, and luma-source coordinates.
- [x] 2.3 Move the live ac0ej3 runtime diagnostic from the source-bounds frontier to the source-read frontier.
- [x] 2.4 Keep source sample value reads, §7.20.3 filtering, decoded-frame allocation, reference refresh, hash, raw, and Y4M output unsupported.

## 3. Verification

- [x] 3.1 Add focused tests for source-read frontier derivation, tap/luma-source coverage, limit accounting, and transactional failures.
- [x] 3.2 Add or update the local ac0ej3 ignored CLI test to assert the new live diagnostic.
- [x] 3.3 Run `openspec validate decode-ac0ej3-lr-source-read-frontier --no-interactive`, focused decode/recon tests, feature/support checks, conformance, and `cargo xtask ci`.

## 4. PR Discipline

- [x] 4.1 Create a ready PR only; request Claude and Codex reviews, wait for both latest-head responses, and address actionable feedback before merge.
