## Context

`DECODE-LR-TX-SKIP-GRID-RETENTION` proves a decoder-local dense
`WienerNsLrTxSkipGrid` from caller-provided transform records. The live local decoder mission
frontier then allocates `Option`-backed storage shells for `CurrFrame`,
`CdefFrame`, and `LrTxSkip`, but the live `LrTxSkip` shell remains value-free.

This change connects those two private pieces only at the storage boundary. It
does not move the live runtime past decoded sample population because real tile
transform records and 10-bit reconstructed frame samples are still not wired
into this pre-decode LR frontier.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-LR-LIVE-TX-SKIP-GRID` as a distinct partial local decoder mission LR
  feature.
- Populate the live `LrTxSkip` shell from a complete
  `WienerNsLrTxSkipGrid` without changing values.
- Reject dimension mismatches and attempted re-population before mutation.
- Keep tests focused on exact population, guard behavior, and the live local decoder mission
  fail-closed diagnostic.

**Non-Goals:**

- Do not derive live transform records from tile traversal.
- Do not populate `CurrFrame` or `CdefFrame` samples.
- Do not derive or retain `FilterClass` or `SubclassLookup`.
- Do not apply loop restoration, produce 10-bit output, refresh references, or
  claim AVM/dav2d byte equality.

## Decisions

1. Keep population private to `runtime_minimal::wienerns_lr`.

   The live storage type is already private proof state. Exposing a public API
   would overstate runtime decode support and widen the contract before real
   tile handoff exists.

2. Copy from the dense grid through checked dimensions.

   The dense helper owns completeness and value derivation from §5.20.7.24
   transform facts. The live shell should only prove that an already-complete
   grid can be retained in the allocated `Option` slots without defaults.

3. Fail before mutation on invalid input.

   Dimension mismatches and re-population attempts return structured
   reconstruction errors before any live slot is changed, preserving the
   fail-closed invariant used by the current diagnostic.

## Risks / Trade-offs

- [Risk] This is still a prerequisite brick and does not make local decoder mission decode.
  -> Mitigation: matrix, support row, diagnostics, and tests state that live
  tile-populated records, decoded samples, filtering, output, and reference
  refresh remain unsupported.
- [Risk] Copying a dense grid adds transient duplication.
  -> Mitigation: the storage is private proof state; a later brick can replace
  the `Option` shell with packed presence/value buffers once production
  population is wired.
