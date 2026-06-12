# Proposal: thread multi-frame-header state into the frame-header parse

## Feature IDs

- `AV2-5.7-MULTI-FRAME-HEADER` (the threading: parsed fields reach consumers)
- `AV2-5.18.4-FRAME-SIZE` (MFH-path default dimensions, § 5.18.4 / mirror :5767)
- `AV2-5.18.7-SEGMENTATION-TILING` (MFH-gated segmentation_params arms,
  § 5.18.7.1 / mirror :6266)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the stops' removal advances the core parse)

## Why

The § 5.7 parser already models `mfh_frame_size`, `mfh_seg_info_present_flag`,
`mfh_ext_seg_flag`, `mfh_allow_seg_info_change`, `mfh_deblocking_filter_update`
(splot-core `hls.rs`), but the validator's availability record
(`MultiFrameHeaderRecord`) drops them and the core parse's MFH input is
reserved plumbing. Three `cur_mfh_id > 0` stops exist solely because the
already-parsed state never reaches the parse: frame_size default dims
(`info.rs` ~607/663), the segmentation_params hard stop (~698), and the
future deblocking gate. PR #53 already resolves which in-band MFH record
governs a frame (`frame_core_against_referenced_header`).

## What Changes

1. **Widen the threading**: the validator's MFH record (or a parallel parse
   view) carries the parsed § 5.7 fields needed by § 5.18.2; the resolved
   record is passed into the core parse (the `mfh_record` input wired by
   PR #53), replacing the reserved stub.
2. **Delete stop (1)**: with `cur_mfh_id > 0` and
   `frame_size_override_flag == 0`, default dimensions come from
   `mfh_frame_width/height_minus_1[ cur_mfh_id ]` (mirror :5767-5769;
   inferred to the sequence maxima when the MFH omitted them, mirror :4101).
3. **Delete stop (2)**: `segmentation_params()` consults
   `mfh_seg_info_present_flag` / `mfh_ext_seg_flag` /
   `mfh_allow_seg_info_change` (§ 5.18.7.1, mirror :6266 ff.) — parse the
   MFH-gated arms exactly per the mirror.
4. **Stop (3) groundwork only**: `mfh_deblocking_filter_update` is threaded
   and recorded; deblocking parsing itself stays with
   frame-filtering-deblocking-gdf-cdef (next change) — no new stop removed
   there, the residual note names it.
5. Frames whose MFH record is NOT resolvable in-band keep the existing
   Unknown routing (the PR #53 resolution guard is the gate; external-HLS
   Provided suppression unchanged).
6. Matrix rows advance with proof; inspect surfaces any newly parsed
   MFH-path fields exactly as the core path does.

## Non-goals

- Deblocking/GDF/CDEF parsing (§ 5.18.5.2, § 5.18.7.9-.10 — next change).
- MFH-gated inter-path parsing (`AV2-5.18.2-FRAME-HEADER-INFO` inter region
  stays stopped).
- New validator diagnostics beyond what richer parses make decidable
  (existing checks simply see more decided facts on MFH paths).

## Acceptance criteria

- [ ] The two stops are gone: an MFH-backed intra frame with
  `frame_size_override_flag == 0` parses through tile_info, and
  segmentation_params parses its MFH-gated arms; positive/negative/EOF
  tests per parser change, both with and without each MFH flag.
- [ ] Unresolvable-MFH frames still route to Unknown (no guessing).
- [ ] Matrix proof on all four rows; `cargo xtask ci` green.
