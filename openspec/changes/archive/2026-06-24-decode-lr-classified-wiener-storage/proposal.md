## Why

The local decoder mission decode path can now derive §7.20.4 classified-Wiener values when a
caller supplies source samples and `LrTxSkip` values, but that still bypasses
the real storage boundary. The next brick needs to prove the decoder can drive
the classifier from decoded current/CDEF frame views and a retained `LrTxSkip`
grid without claiming the live runtime has those buffers yet.

Feature ID: `DECODE-LR-CLASSIFIED-WIENER-STORAGE`.

## What Changes

- Add a tracked local decoder mission decoder frontier for storage-backed classified-Wiener
  classification.
- Make `pc_wiener_classify` able to propagate `LrTxSkip` storage lookup
  failures, matching real decoder storage rather than an infallible synthetic
  callback.
- Add decoder-owned storage adapters for frame-backed luma source reads and
  bounded `LrTxSkip` grid lookups.
- Keep live local decoder mission fail-closed with a diagnostic that names the remaining
  runtime retention/filtering boundary.
- Update implementation and decoder support matrix entries with focused proof.
- Non-goals: no loop-restoration filtering, no chroma Wiener NS filtering, no
  10-bit output allocation, no reference refresh, and no AVM/dav2d equality
  claim for local decoder mission.

## Capabilities

### New Capabilities

- `lr-classified-wiener-storage`: Tracks the fail-closed runtime frontier
  where AV2 §7.20.4 luma classified-Wiener classification can read real decoded
  frame storage plus `LrTxSkip` storage, while live local decoder mission still stops before
  those retained values are applied to loop restoration.

### Modified Capabilities

- None.

## Impact

- `crates/splot-recon/src/pc_wiener.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-decode/src/runtime_minimal/wienerns_lr.rs`
- `crates/splot-decode/src/runtime_minimal/inter/lr_source_read_tests.rs`
- `crates/splot-decode/src/runtime_minimal.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
