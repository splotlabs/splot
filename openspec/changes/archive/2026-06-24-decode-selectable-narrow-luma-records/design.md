## Context

The local decoder mission runtime path now consumes active CfL mode-info syntax and reaches the
`TX_MODE_SELECT` transform-record handoff. The first remaining live gate is a
luma-only SDP leaf at `r=8, c=24`, `BLOCK_4X32`, with `has_chroma = false`.
The existing selectable transform-record path already derives luma transform
records from AV2 §5.20.6.1/§5.20.6.3 and consumes §5.20.7.27 coefficient facts,
but its admission guard still rejects any leaf with `n4w < 2` or `n4h < 2`.

## Goals / Non-Goals

**Goals:**

- Admit the observed luma-only `BLOCK_4X32` selectable transform-record leaf.
- Keep luma transform-record derivation grounded in AV2 §5.20.6.1 and
  §5.20.6.3, and `LrTxSkip` derivation grounded in §5.20.7.24/§5.20.7.27.
- Preserve existing chroma guards so narrow luma-only support does not imply
  narrow chroma prediction or chroma residual support.
- Advance the live local decoder mission fail-closed frontier to the next honest unsupported
  diagnostic without output.

**Non-Goals:**

- No decoded `CurrFrame` or `CdefFrame` sample population.
- No CfL prediction, chroma reconstruction, loop-restoration filtering/output,
  reference refresh, raw/Y4M/hash success, or AVM/dav2d byte equality claim.
- No broad support for all luma-only SDP block shapes unless directly exercised
  and tested by the local path.

## Decisions

- Treat luma-only narrow admission separately from chroma-bearing admission.
  The first blocked leaf is `is_luma_part() == true` and `has_chroma == false`,
  so the safe change is to allow luma residual/transform-record handling for
  nonzero luma dimensions while retaining the existing chroma checks for
  chroma-bearing leaves.
- Keep the output frontier structured. If the new luma-only case parses, the
  runtime must still end in `decode/unsupported-feature` before sample-backed
  LR classification or filtering.
- Add focused tests at the private transform-record layer rather than a public
  API. This code is runtime-private and the proof should pin the observed block
  shape plus the live local decoder mission diagnostic movement.

## Risks / Trade-offs

- Broader-than-intended luma admission could consume syntax for unsupported
  partitions. Mitigation: gate the behavior to luma-only nonzero dimensions and
  keep transform partition and coefficient errors structured.
- The local stream may expose another immediately adjacent selectable
  transform-record subcase. Mitigation: allow the next fail-closed diagnostic to
  identify that subcase while proving the `BLOCK_4X32` gate is gone.
