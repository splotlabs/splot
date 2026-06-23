## Context

`DECODE-AC0EJ3-LR-UNIT-SELECTIONS-FRONTIER` retained the syntax result of
AV2 §5.20.10.5 `use_wiener_ns` for covered Wiener NS LR units. The local ac0ej3
stream also reaches §5.20.10.6 entropy-coded per-unit Wiener NS filter syntax on
chroma planes before `read_lr()` is complete. The next runtime handoff needs
that syntax consumed, then block-level §7.20.1 facts: for an active unit, which
loop-restore blocks are covered, what current-plane sample rectangle each block
uses, and what luma source/stripe bounds later feed §7.20.2 source selection.

## Goals / Non-Goals

**Goals:**

- Consume the required §5.20.10.6 entropy-coded per-unit Wiener NS filter syntax
  before retaining source-bound facts, without exposing or applying the decoded
  coefficients.
- Derive active block source-bound facts from the consumed LR unit selections
  for the supported root frontier.
- Preserve syntax-order LR-unit selection state and aggregate active counters.
- Keep derivation transactional: if source-bound derivation fails, LR CDF
  mutations are not committed.
- Move the local ac0ej3 fail-closed diagnostic to the new source-bounds row.

**Non-Goals:**

- Reading `CurrFrame` / `CdefFrame` samples, applying §7.20.2 source reads, or
  applying §7.20.3 Wiener NS filtering.
- SDP PlaneStart/PlaneEnd traversal, switchable LR, PC-Wiener, chroma filter
  reconstruction/correctness, temporal/reference Wiener state, 10-bit output,
  or successful ac0ej3 decode.
- Adding public APIs or dependencies.

## Decisions

- Store active source-bound facts in `TileLoopRestorationRootFrontier`, next to
  the LR-unit selections that produced them. This keeps the state at the
  traversal/runtime handoff and avoids importing `splot-recon` types into the
  tile-payload parser layer.
- Add the §9.3 Wiener NS length, UV-symmetry, and base CDF rows to the tile CDF
  subset and consume the §5.20.10.6 per-unit syntax transactionally before
  source-bound retention. The coefficient values remain an internal syntax
  detail for this frontier; §7.20.3 filtering is still unsupported.
- Derive source-bound facts only when the root LR frontier explicitly asks to
  retain them. The normal partition planners still read LR syntax without
  allocating a source-bound vector.
- Carry `disable_loopfilters_across_tiles` from the sequence filter config into
  traversal frame facts. When it is set, retained source bounds are clamped to
  the tile MI range; otherwise they use the frame-wide luma extent, matching
  AV2 §7.20.1.
- Bound retained active block facts with `MaxLumaSamplesPerFrame`. The vector is
  a pre-output planning artifact and must remain subject to decode limits.

## Risks / Trade-offs

- The frontier still does not apply loop restoration. Mitigation: the row stays
  partial and the diagnostic explicitly names source reads/filtering as the next
  unsupported step.
- The source-bound vector can contain many 4x4 blocks. Mitigation: it is retained
  only for the root LR frontier and is checked against existing decode limits.
- The derivation still stops at bounds and does not read source samples.
  Mitigation: source reads and filtering remain explicitly outside this row.
