# Design: frame-header-writer-segmentation

## Context

`segmentation_params()` (§ 5.18.7.1) reads `segmentation_enabled`, then — when enabled —
derives `(haveSegParams, allowChange, reuseSource)` from the sequence header or the resolved
MFH record, codes `reuse_seg_info` only when `allowChange`, and on the fresh path reads the
`seg_info(MaxSegments)` body (§ 5.4.9, the shared `write_seg_info` already exists). It then
*derives* `segmentation_update_map` (inferred `1` on the intra path),
`segmentation_temporal_update` (inferred `0`), and `SegIdPreSkip` / `LastActiveSegId` from the
feature table — none of which are coded.

## Decisions

- **Additive — derivations, not a model extension.** The principled split from #4b / #4c:
  those surfaced bits that affect *layout / downstream parsing* and were not recoverable. Every
  read-but-not-stored value here is instead either a **derivation** the parser recomputes from
  the feature table (`SegIdPreSkip`, `LastActiveSegId`) or an **inferred constant** on the
  intra path (`reuse_seg_info` when `allowChange == 0`, `segmentation_update_map = 1`,
  `segmentation_temporal_update = 0`). The writer re-derives each and rejects a disagreeing
  model, so no `SegmentationParams` field is added.
- **Re-derive the parser's branch triple.** `derive_seg_params(seg, mfh)` reproduces the
  parser's `(haveSegParams, allowChange, reuseSource)` selection exactly — the MFH arm (built
  by the caller only when `cur_mfh_id > 0 && mfh_seg_info_present_flag`) takes priority over the
  sequence arm, which takes priority over the zero fallback — so the writer signals or infers
  `reuse_seg_info` from the same inputs the parser used and validates the reuse `features` copy
  against the same source.
- **Reuse the § 5.4.9 body writer, pre-validated.** The fresh path reconstructs the
  `SegmentInfo` and calls the shared `write_seg_info`; `check_segmentation_encodable` runs
  `check_seg_info_encodable` first so the shared writer cannot reject mid-write (no partial
  buffer).
- **No panic on constructed models.** The `SegIdPreSkip` / `LastActiveSegId` re-derivation
  clamps its segment count at `min(max_segments, MAX_SEGMENTS)` so a hostile `max_segments`
  cannot index out of `features`; the index fits in `u8` because `i < MAX_SEGMENTS == 16`. All
  validation runs in `check_segmentation_encodable` before the first `writer.write_*`, so a
  rejected model leaves `writer.bit_len() == 0`.

## Testing

Round-trip via the public parser across every branch (disabled; enabled with `reuse_seg_info`
inferred or coded; the fresh `seg_info()` body; the MFH arm, the sequence arm, and the zero
fallback for the reuse source). One reject test per `NonCanonicalFrameHeader` path (asserting
`bit_len() == 0`), including the constructed-model edges (a hostile `max_segments`, each
derived/inferred field disagreeing with its re-derivation). A round-trip property test that
parses random bits + gating then re-emits and reparses.
