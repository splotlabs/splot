# Change: multi-frame-header-writer

## Feature IDs

- `AV2-5.7-MULTI-FRAME-HEADER` (write: `todo` → `done`)
- `ENC-BITSTREAM-WRITER` (the seventh of the unwritten OBU-type body writers)

## Why

Continue moving the parser-modeled OBU types from `Unimplemented` to round-trippable in the
complete-OBU dispatch. `multi_frame_header_obu()` (§ 5.7) is the next target: a self-contained set of
fixed-width / `uvlc` fields plus the shared `seg_info()` (§ 5.4.9) — it reuses the existing
`write_seg_info`, so it is small and low-risk.

## What changes

- **Writer** (`crates/splot-core/src/write/multi_frame_header.rs`, new; additive, no model change):
  `write_multi_frame_header(writer, mfh)` — the inverse of `parse_multi_frame_header` (§ 5.7), field
  order preserved: `mfh_seq_header_id` / `mfh_id_minus_1` (`uvlc`), the gated `mfh_frame_size`
  (present-flag + width/height bit-widths and values), `mfh_deblocking_filter_update` + the four
  `mfh_apply_deblocking_filter` flags, and `mfh_seg_info_present_flag` + `mfh_ext_seg_flag` /
  `mfh_allow_seg_info_change` / `seg_info(mfh_ext_seg_flag ? 16 : 8)` (reusing `write_seg_info`).
  - **Reject-before-write** (scratch-writer; never panics): a `mfh_apply_deblocking_filter` array that
    is not all-`false` when `mfh_deblocking_filter_update` is clear (the parser leaves it `false`);
    the three segment-info `Option`s whose presence disagrees with the stored
    `mfh_seg_info_present_flag`; a frame-size bit width outside `1..=16`; and field-width rejects.
  - **Reproduce-verbatim** the parser-tolerated `mfh_seq_header_id` / `mfh_id_minus_1` values (the
    validator flags out-of-range ids but the parser preserves them).
- **Dispatch** (`write/dispatch.rs`): route `ParsedObu::MultiFrameHeader` to the new writer + the
  generic tail instead of `Unimplemented`; it carries no passthrough. Two types remain unwritten.
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalMultiFrameHeader { what }`.

## Validator impact

None.

## Non-goals

- No writers for the other two unwritten OBU types; no frame-header reuse semantics; no model change;
  no public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::multi_frame_header` + one `WriteError` variant + the
  dispatch arm, reusing `write_seg_info`).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (`AV2-5.7-MULTI-FRAME-HEADER` write `done` +
  `ENC-BITSTREAM-WRITER` note) + regenerated `docs/FEATURE-STATUS.md`.
