## Context

The current ac0ej3 LR runtime path can parse frame-level Wiener NS LR syntax,
derive active source/classification dependencies, allocate live LR storage, and
populate live `LrTxSkip` storage when luma transform records are fixed-largest.
The local mission stream reaches that handoff but uses `TX_MODE_SELECT`, so the
runtime correctly stops at `unsupported_wienerns_lr_tx_mode_select_transform_records`.

AV2 §5.20.6.1 and §5.20.6.3 define the missing syntax surface:
`read_tx_size()` selects between a block-wide transform and
`read_tx_partition()` for selectable transforms; `read_tx_partition()` writes
`LumaTxSizes`, middle flags, and scan-order flags through `set_tx_size`.
The LR path only needs the resulting luma transform extents and coefficient
`eob` facts to derive §5.20.7.27 `LrTxSkip`.

## Goals / Non-Goals

**Goals:**

- Add a decoder-private selectable transform-record reader for the supported
  ac0ej3 intra LR path.
- Preserve the exact §5.20.6.1/§5.20.6.3 transform extents used when reading
  luma coefficients, then convert them into existing
  `WienerNsLrTxSkipTransformRecord` values.
- Populate live LR `LrTxSkip` storage for the local ac0ej3 stream before the
  next unsupported decoded-sample diagnostic.
- Record the new partial support row in the implementation and decoder-support
  matrices, and add the transform-size citation surface to
  `docs/SPEC-MAPPING.md`.

**Non-Goals:**

- Populate live `CurrFrame` or `CdefFrame` samples.
- Derive or retain `FilterClass`/`SubclassLookup`.
- Apply loop-restoration filters, emit 10-bit output, refresh references, or
  claim AVM/dav2d equality for ac0ej3.
- Broaden unrelated transform-type, FSC, CCTX, IDTX, IST, inter, or encoder
  behavior.

## Decisions

- Model selectable transform records inside the decoder runtime instead of
  widening public APIs. The data is only needed to advance the private ac0ej3 LR
  frontier and should not become a stable external contract.
- Reuse existing coefficient readers after transform-size derivation. This keeps
  bit consumption guarded by the same §5.20.7.27 residual parser used by the
  fixed-largest handoff and prevents a second, divergent coefficient path.
- Store only the facts required for LR handoff: row/column, transform extent,
  `eob`, and effective skip. The decoded coefficient values remain an input to
  later reconstruction work and are intentionally not exposed through this
  frontier.
- Fail closed for unsupported transform partition shapes or syntax contexts.
  The parser may support the partition forms needed by ac0ej3, but it must not
  fabricate a full `LrTxSkip` grid when a selectable transform branch is outside
  the implemented subset.

## Risks / Trade-offs

- Selectable transform syntax is recursive enough to hide off-by-one geometry
  bugs. Mitigation: test record coverage, incomplete-grid rejection, and the
  local ac0ej3 diagnostic movement through the existing live-grid population
  helper.
- Parsing transform records without frame reconstruction can drift from future
  reconstruction state. Mitigation: keep the handoff decoder-private and feed
  the same luma transform sizes into the coefficient reader that later
  reconstruction will consume.
- A larger PR can become too broad if it tries to implement filtering or output.
  Mitigation: stop immediately after live `LrTxSkip` population and keep the
  next diagnostic explicit.
