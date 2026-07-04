## 1. Tracking

- [x] 1.1 Add `DECODE-LR-SOURCE-READ-FRONTIER` to the implementation matrix.
- [x] 1.2 Add decoder support/status coverage for `lr-source-read-frontier`.

## 2. Implementation

- [x] 2.1 Add crate-private source-read frontier state for active Wiener NS LR source blocks.
- [x] 2.2 Reuse `splot-recon` loop-restoration source selection/read primitives for supported output, tap, and luma-source coordinates.
- [x] 2.3 Move the live local decoder mission runtime diagnostic from the source-bounds frontier to the classified-Wiener boundary that precedes source reads for its two-class luma bank.
- [x] 2.4 Keep source sample value reads, §7.20.3 filtering, decoded-frame allocation, reference refresh, hash, raw, and Y4M output unsupported.

## 3. Verification

- [x] 3.1 Add focused tests for source-read frontier derivation, tap/luma-source coverage, limit accounting, classified-Wiener ordering, and transactional failures.
- [x] 3.2 Add or update the local decoder mission ignored CLI test to assert the new live diagnostic.
- [x] 3.3 Run `openspec validate decode-lr-source-read-frontier --no-interactive`, focused decode/recon tests, feature/support checks, conformance, and `cargo xtask ci`.

## 4. PR Discipline

- [x] 4.1 Create a ready PR only; request Claude and Codex reviews, wait for both latest-head responses, and address actionable feedback before merge.
