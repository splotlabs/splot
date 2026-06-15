# Tasks

## Writer (additive — no model change)
- [x] `write/frame_segmentation.rs`: `write_segmentation_params` with an up-front
      `check_segmentation_encodable` (reject-before-write); `derive_seg_params` and
      `derive_seg_id_state` re-derive the parser's `(haveSegParams, allowChange,
      reuseSource)` triple and the `SegIdPreSkip` / `LastActiveSegId` state.
- [x] Fresh path reuses the shared § 5.4.9 `write_seg_info` (pre-validated by
      `check_seg_info_encodable` so it cannot reject mid-write).
- [x] Widen the `MfhSegView` re-export in `headers/frame/mod.rs`; register the module +
      re-export the writer in `write/mod.rs`. No model field / `WriteError` variant added.

## Tests and proof
- [x] Round-trip tests across every branch (disabled; enabled reuse-inferred /
      reuse-coded / fresh; MFH arm vs sequence arm vs zero fallback); one reject test per
      `NonCanonicalFrameHeader` path with `bit_len() == 0`, incl. constructed-model edges
      (hostile `max_segments`, derived-field disagreement); a round-trip property test that
      drives the parser on random bits + gating then re-emits and reparses.

## Matrix and docs
- [x] Advance the `segmentation_params()` portion of `write` on
      `AV2-5.18.7-SEGMENTATION-TILING` (write stays `partial` pending the filter children),
      with proof + the writer note. Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-segmentation --strict`
