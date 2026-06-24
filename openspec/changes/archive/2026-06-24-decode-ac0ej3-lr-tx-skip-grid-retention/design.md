## Context

The ac0ej3 LR path has progressed to a live fail-closed storage-retention gate:
the runtime derives active-bit-depth `CurrFrame`/`CdefFrame` buffer sizes and the
frame-wide `LrTxSkip` grid dimensions, then stops before any unpopulated storage
is used. Existing storage-backed classified-Wiener helpers can read from a
bounded `WienerNsLrTxSkipGrid`, but the decoder has no value-backed way to build
that grid from parsed transform-block facts.

The AV2 v1.0.0 spec defines the grid write in §5.20.7.25
`store_tx_info()`: for luma, each covered transform 4x4 cell stores
`LrTxSkip[row + i][col + j] = skip_flag || (eob == 0)`. The classified-Wiener
lookup in §7.20.4 later reads `LrTxSkip[y >> 2][x >> 2]`. This change creates the
decoder-local bridge between those facts without claiming live tile reconstruction
or filtering.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-AC0EJ3-LR-TX-SKIP-GRID-RETENTION` as a partial ac0ej3 LR feature.
- Provide a private transform-record helper that writes boolean `LrTxSkip` values
  into the existing bounded grid representation.
- Reject records that overflow the grid, leave holes, or otherwise cannot prove
  every returned value came from parsed transform state.
- Keep the live ac0ej3 diagnostic fail-closed before decoded sample population,
  `FilterClass` grid retention, LR filtering, output, and reference refresh.

**Non-Goals:**

- No broad parse-only traversal of the full ac0ej3 1920x1080 key tile.
- No decoded `CurrFrame`/`CdefFrame` sample allocation or zero-filled frame
  construction.
- No live classifier call, `FilterClass` grid persistence, `SubclassLookup`,
  chroma Wiener NS filtering, 10-bit output, AVM/dav2d byte equality, or
  successful ac0ej3 decode.

## Decisions

- Keep the helper in `runtime_minimal/wienerns_lr.rs`. It is only needed by the
  decoder's private LR frontier and can reuse `WienerNsLrTxSkipGrid` without
  widening public APIs or adding crate dependencies.
- Use an intermediate `Vec<Option<u8>>` while records are applied. `None` means a
  cell has not been populated by parsed transform syntax; the final grid is built
  only after every cell is `Some(0|1)`. This avoids sentinel values that could be
  mistaken for decoded data.
- Model only the normative luma grid write. Chroma planes do not write
  `LrTxSkip`, and broader transform metadata (`DeblockingTxSizes`, `TxColBase`,
  `TxRowBase`) remains outside this feature.
- Surface malformed helper inputs as structured reconstruction errors. The helper
  is private and test-facing for this brick, so existing `ReconError` variants are
  enough: invalid bounds for out-of-grid records and buffer mismatch for missing
  cells.

## Risks / Trade-offs

- [Risk] A helper-only brick can look more complete than the live runtime.
  Mitigation: feature/matrix/support notes explicitly state that live tile
  parsing still does not populate the grid and ac0ej3 remains fail-closed.
- [Risk] Future transform traversal may need packed or bitset storage for memory
  efficiency. Mitigation: this helper returns the current byte-per-value grid
  used by the classifier; packing can be introduced behind the same complete-grid
  contract later.
- [Risk] Overlapping transform records could hide a caller bug. Mitigation: the
  helper treats repeated writes of the same value as idempotent and conflicting
  writes as invalid bounds/contract failure in tests when that policy is wired.
