## Context

The local decoder mission stream reaches active two-class luma Wiener NS loop
restoration in the leading closed-loop key frame. The current fail-closed path
derives and limit-checks the live 10-bit `CurrFrame`/`CdefFrame` storage
footprint and frame-wide `LrTxSkip[y >> 2][x >> 2]` grid shape, then stops as
`DECODE-LR-RUNTIME-STORAGE-RETENTION`.

The next boundary must be value-honest: §7.20.2/§7.20.4 source reads need real
decoded samples and real `LrTxSkip` values, but the current tile
reconstruction path has not populated those values for the local decoder mission key frame.

## Goals / Non-Goals

**Goals:**
- Add `DECODE-LR-LIVE-STORAGE-ALLOCATION` as the live fail-closed
  frontier after storage-footprint planning.
- Allocate private storage shells for the two active-bit-depth frame buffers
  and the `LrTxSkip` grid.
- Represent missing samples explicitly so storage-backed classification cannot
  accidentally consume zero-filled or fabricated values.
- Preserve limit checks before allocation and keep low storage limits returning
  `decode/resource-limit`.

**Non-Goals:**
- No decoded sample population from tile reconstruction.
- No real transform-record handoff into the `LrTxSkip` grid.
- No `FilterClass` grid retention, `SubclassLookup`, §7.20.3 filtering,
  output, reference refresh, or successful local decoder mission decode.

## Decisions

1. Use decoder-owned storage shells instead of `DecodedFrame<T>` for this
   frontier.

   `DecodedFrame<T>` validates complete sample planes and therefore can only
   represent populated frame data. The new shells track the same AV2-derived
   shapes but store `Option`-backed cells, making the unpopulated state explicit
   until a later tile-reconstruction brick writes real values.

2. Keep the dense `WienerNsLrTxSkipGrid` value-backed.

   The existing grid is the correct input for storage-backed §7.20.4
   classification because it rejects holes and non-boolean values. This change
   adds a separate live allocation shell for incomplete `LrTxSkip` state rather
   than weakening the value-backed grid contract.

3. Reuse the current storage-footprint derivation as the allocation preflight.

   `derive_wienerns_lr_runtime_storage_retention_frontier` already grounds the
   active bit depth in AV2 §6.4.1 and the frame dimensions in §6.17.4.1, then
   applies the decode limits. Allocation should happen only after that preflight
   succeeds.

## Risks / Trade-offs

- [Risk] Allocating unpopulated cells could be mistaken for output-ready
  storage. → The shell exposes population counts and the live diagnostic states
  that decoded samples and `LrTxSkip` values are absent; storage-backed
  classification is not called.
- [Risk] Actual `Option` storage is larger than the byte-budget accounting.
  → The budget remains an AV2 sample-storage budget used for admission and
  limit checks; this private fail-closed shell is a temporary runtime boundary,
  not the final packed representation.
- [Risk] Adding another LR helper to an already-large module increases file
  size. → Keep the helper private and compact; split the module only if the hard
  source-line cap is approached.
