## Context

The existing LR runtime path already derives source-read/classified-Wiener
coordinates, limit-checks retained frame and `LrTxSkip` storage, allocates the
live LR storage shell, and can copy a complete retained `WienerNsLrTxSkipGrid`
into that shell. The remaining missing bridge is the live tile transform record:
AV2 §5.20.7.27 writes `LrTxSkip[row+i][col+j] = skip_flag || (eob == 0)` from
each parsed luma transform block, and §7.20.4 later reads that grid.

The local ac0ej3 key frame uses `TX_MODE_SELECT`, so its transform records also
depend on §5.20.6.1 `read_tx_size()` / `read_tx_partition()` before the
coefficient reader can know the exact transform extents. The current decoder
only has fixed-largest transform sizing in the runtime path.

## Goals / Non-Goals

**Goals:**

- Add a decoder-private handoff that converts parsed fixed-largest luma tile
  transform facts into `WienerNsLrTxSkipTransformRecord` values.
- Derive a retained `WienerNsLrTxSkipGrid` from those records and populate the
  live LR storage shell before the next unsupported diagnostic.
- Move the ac0ej3 mission fixture to a precise `TX_MODE_SELECT` transform-record
  frontier instead of the prior allocation-only live-storage diagnostic.
- Record the new partial support row in the implementation and decoder-support
  matrices.

**Non-Goals:**

- Implement §5.20.6.1 selectable transform partition syntax.
- Populate live `CurrFrame` or `CdefFrame` samples.
- Implement `FilterClass`, `SubclassLookup`, loop-restoration filtering/output,
  reference refresh, or full ac0ej3 oracle parity.

## Decisions

- Use the existing fixed-largest coefficient/tile facts rather than a new
  transform abstraction. This keeps the handoff grounded in the parser state the
  decoder already proves and avoids fabricating `LrTxSkip` defaults.
- Treat `TX_MODE_SELECT` as an explicit unsupported transform-record frontier.
  The alternative was to keep returning the generic live-storage diagnostic, but
  that hides the next real decoder brick and gives weaker progress information.
- Keep live frame samples unpopulated after successful fixed-largest tx-skip
  population. The PR should advance one runtime dependency at a time; frame
  sample retention and CDEF/current-frame handoff need separate proof.

## Risks / Trade-offs

- Fixed-largest support does not advance the local ac0ej3 fixture past transform
  records because ac0ej3 uses selectable transforms. Mitigation: the new
  diagnostic names that exact blocker and the matrix row records the limitation.
- Reading transform records from tile facts without reconstruction can drift from
  reconstruction paths. Mitigation: reuse existing coefficient block facts and
  focused tests for record geometry, retained-grid population, live-shell
  population, and ac0ej3 diagnostic movement.
