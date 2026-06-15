# Change: frame-header-writer-segmentation

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.7-SEGMENTATION-TILING` (advances the `segmentation_params()` portion of its
  `write` stage; the row stays `partial` until the filter-param children land)

## Why

Fifth slice (#4e) of the frame-header writer (intra path). It inverts the § 5.18.7.1
`segmentation_params()` parser
([`crate::headers::frame::parse_segmentation_params`]). Like #4d, this slice is
**additive — no model change**: every read-but-not-stored value here is either a
**derivation** the parser recomputes from the feature table (`SegIdPreSkip`,
`LastActiveSegId`) or an **inferred constant** on the intra path
(`reuse_seg_info` when `allowChange == 0`, `segmentation_update_map = 1`,
`segmentation_temporal_update = 0`), so the writer re-derives and validates rather than
storing extra bits.

## What changes

- **Writer** (`crates/splot-core/src/write/frame_segmentation.rs`):
  `write_segmentation_params(writer, params, seg, mfh)` — the exact inverse of the intra
  `segmentation_params()` parse, validating the whole model up front
  (`check_segmentation_encodable`, reject-before-write).
- **Field order** (the parser's § 5.18.7.1 read order): `segmentation_enabled` `f(1)`
  always; when enabled, `reuse_seg_info` `f(1)` **only on the `allowChange` path**
  (otherwise inferred `= haveSegParams`, no bit); then on the fresh path the
  `seg_info(MaxSegments)` body, **reusing the shared § 5.4.9 `write_seg_info`**
  (`AV2-5.4.9-SEGMENT-INFO`). The reuse path writes no feature bits.
- **Re-derived, never coded** (rejected on mismatch): the inferred `reuse_seg_info`; the
  reuse `features` table (validated against the § 5.18.7.1 reuse source — `MfhFeatureData`
  on the MFH arm, `SeqFeatureData` on the sequence arm, all-disabled when absent); the
  intra-inferred `segmentation_update_map` / `segmentation_temporal_update`; and the
  `SegIdPreSkip` / `LastActiveSegId` table derivation (clamped at `MAX_SEGMENTS` so a
  hostile `max_segments` cannot index out of `features`).
- **Visibility only** outside `write/`: the existing `MfhSegView` re-export in
  `headers/frame/mod.rs` is widened so the writer can name the parser's resolved-MFH input.
  No model field and no new `WriteError` variant (reuses
  `WriteError::NonCanonicalFrameHeader`).

## Validator impact

None. No new diagnostics; the validator is unchanged.

## Non-goals

- No filter / restoration / CCSO writers (`lr_params`, `ccso_params`, `gdf_params`,
  `cdef_params`) — later #4f/#4g slices.
- No composing `write_frame_header`.

## Impact

- Crate: `crates/splot-core` (additive `write` module + a visibility-only re-export widen).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
