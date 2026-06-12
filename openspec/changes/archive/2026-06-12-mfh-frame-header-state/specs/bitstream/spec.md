# bitstream delta: mfh-frame-header-state

Advances `AV2-5.7-MULTI-FRAME-HEADER`, `AV2-5.18.4-FRAME-SIZE`,
`AV2-5.18.7-SEGMENTATION-TILING` on the `cur_mfh_id > 0` path.

## ADDED Requirements

### Requirement: MFH-backed frame-header parsing

The frame-header core parser SHALL consume the resolved in-band
multi-frame header's parsed § 5.7 state on `cur_mfh_id > 0` paths: the
§ 5.18.4 default frame dimensions come from
`mfh_frame_width/height_minus_1[ cur_mfh_id ]` (with the § 5.7 omitted-size
inference to the sequence maxima), and § 5.18.7.1 `segmentation_params()`
parses its `mfh_seg_info_present_flag` / `mfh_ext_seg_flag` /
`mfh_allow_seg_info_change` gated arms. A frame whose referenced
multi-frame header is not resolvable in-band SHALL keep the existing
unsupported/Unknown routing rather than guessing field positions.

#### Scenario: MFH default dimensions

- **WHEN** an intra frame with `cur_mfh_id > 0` and
  `frame_size_override_flag == 0` references an in-band MFH carrying
  explicit dimensions
- **THEN** the parse continues through `tile_info()` with the MFH
  dimensions instead of stopping

#### Scenario: MFH-gated segmentation arms

- **WHEN** the referenced in-band MFH has `mfh_seg_info_present_flag == 1`
- **THEN** `segmentation_params()` parses the MFH-gated arm per § 5.18.7.1
  instead of stopping before it

#### Scenario: unresolvable MFH stays unsupported

- **WHEN** a frame references a `cur_mfh_id` with no resolvable in-band
  multi-frame header
- **THEN** the parse stops as before and dependent judgments stay Unknown

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
