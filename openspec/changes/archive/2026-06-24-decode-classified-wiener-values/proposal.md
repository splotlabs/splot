## Why

The local decoder mission decode path currently reaches AV2 §7.20.4 classified-Wiener coordinate
derivation, then stops before source sample values, `LrTxSkip` values, and
`FilterClass` can be derived. The next decoder brick needs to prove value-backed
classification plumbing without pretending the runtime already has 10-bit
current/CDEF frame storage or an `LrTxSkip` grid.

Feature ID: `DECODE-LR-CLASSIFIED-WIENER-VALUES`.

## What Changes

- Add a tracked local decoder mission decoder frontier for value-backed classified-Wiener
  classification.
- Wire the minimal decoder helpers to call `splot-recon::pc_wiener_classify`
  through caller-supplied source-sample and `LrTxSkip` value interfaces.
- Retain a fail-closed live local decoder mission diagnostic that names the remaining missing
  runtime storage instead of claiming real frame-buffer reads.
- Update decoder support and implementation matrix entries with focused proof.
- Non-goals: no broad AV2 decode, no loop-restoration filtering, no chroma Wiener
  NS filtering, no 10-bit output allocation, no reference refresh, no AVM/dav2d
  equality claim for local decoder mission.

## Capabilities

### New Capabilities

- `lr-classified-wiener-values`: Tracks the fail-closed runtime frontier
  for value-backed AV2 §7.20.4 luma classified-Wiener classification and the
  remaining current/CDEF frame plus `LrTxSkip` storage blocker.

### Modified Capabilities

- None.

## Impact

- `crates/splot-decode/src/runtime_minimal.rs`
- `crates/splot-decode/src/runtime_minimal/wienerns_lr.rs`
- `crates/splot-decode/src/runtime_minimal/inter/lr_source_read_tests.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
