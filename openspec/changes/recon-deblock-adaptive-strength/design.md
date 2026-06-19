## Context

`RECON-DEBLOCK-SAMPLE-FILTER` left `q_thr` caller-resolved and
`RECON-DEBLOCK-FILTER-MAX-WIDTH` covered the widths. § 7.17.5 derives `qThr` and
`side` from the § 7.17.6 filter level; its arithmetic is a self-contained slice
once the level and the `Side_Thresholds` value are caller-resolved.

## Goals / Non-Goals

**Goals:** a total derivation for the § 7.17.5 `(qThr, side)` plus the `qInd` index
helper, reusing the existing `quantizer_value`.

**Non-Goals:** the § 7.17.6 filter-level selection (segment/qindex state), the
§ 7.17.7.2 filter choice, and any runtime wiring.

## Decisions

- **Split the `qInd` index out as a `const fn`.** `Side_Thresholds` is in
  `splot-core`'s § 9.2 tables, so the caller does the lookup; exposing
  `deblock_side_threshold_index` gives the caller a tested helper for the
  `Clip3(0, MAX_SIDE_TABLE - 1, lvl - 24 * (BitDepth - 8))` arithmetic without
  the table.
- **Reuse `quantizer_value` for `get_q`.** `qThr = Round2(get_q(lvl, 0),
  QUANT_TABLE_BITS) >> 6` calls the already-shipped § 7.14.2 quantizer lookup, so
  the strength function is a regular (non-`const`) `fn` composing it with the
  `Round2` and the shift.
- **Caller-resolved `side_threshold`.** The strength function takes the pre-indexed
  `Side_Thresholds[qInd]` value, matching the deblock module's
  caller-resolves-values contract.

## Risks / Trade-offs

- The `qThr` test composes `quantizer_value` (so it is not a fully hand-computed
  anchor), but `quantizer_value` is independently tested, so the strength test
  pins the § 7.17.5 composition (the `Round2(_, 3) >> 6`) on top of it; the `side`
  arithmetic is hand-pinned exactly. It is loaded ahead of its runtime caller,
  matching the established pattern.
