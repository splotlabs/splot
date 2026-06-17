// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Typed errors for the AV2 bitstream writer (`ENC-BITSTREAM-WRITER`).
//!
//! The writer is the inverse of the [`crate::bitio::BitReader`] descriptors. A
//! [`WriteError`] is raised when a model value cannot be encoded by the requested
//! AV2 descriptor — for example a value too large for a fixed field, or a width
//! outside a descriptor's domain. These are *encoder-side* programming errors
//! (the caller asked for an impossible encoding), distinct from the parser's
//! conformance/EOF [`crate::error::Error`] variants, so the writer carries its own
//! self-contained error type and never touches the parser error model.

use thiserror::Error;

/// An AV2 bitstream-writer descriptor could not encode the requested value.
///
/// Every variant corresponds to a precondition of the matching
/// [`crate::bitio::BitReader`] descriptor: the writer rejects exactly the values
/// the reader could never have produced, so the round-trip property
/// `read(write(x)) == x` holds for every value the writer accepts.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WriteError {
    /// A fixed-width write requested more bits than the descriptor allows
    /// (`f(n)`/`su(n)`/`rg(n)` accept `n <= 32`).
    #[error("bit width {requested} exceeds the maximum of {max}")]
    BitWidthTooLarge {
        /// The requested width, in bits.
        requested: u32,
        /// The maximum width the descriptor permits, in bits.
        max: u32,
    },

    /// A little-endian write requested more bytes than the descriptor allows
    /// (`le(n) -> u64` accepts `n <= 8`).
    #[error("byte width {requested} exceeds the maximum of {max}")]
    ByteWidthTooLarge {
        /// The requested width, in bytes.
        requested: u32,
        /// The maximum width the descriptor permits, in bytes.
        max: u32,
    },

    /// A descriptor that requires a positive width was given zero (e.g. `ns(0)`).
    #[error("the {descriptor} descriptor requires a width greater than zero")]
    ZeroWidth {
        /// The AV2 descriptor name (`"ns"`).
        descriptor: &'static str,
    },

    /// A value does not fit in the requested fixed field width.
    #[error("value {value} does not fit in {width_bits} bit(s)")]
    ValueTooWide {
        /// The offending value.
        value: u64,
        /// The field width that cannot hold it, in bits.
        width_bits: u32,
    },

    /// A value lies outside the range the descriptor can encode (`su(n)` signed
    /// range, `ns(n)` `0..n`, `uvlc`/`svlc` conformance bound, or `rg(n)` whose
    /// unary prefix would not terminate within 32 bits).
    #[error("the {descriptor} descriptor cannot encode value {value}")]
    ValueOutOfRange {
        /// The AV2 descriptor name (`"su"`, `"ns"`, `"uvlc"`, `"svlc"`, `"rg"`).
        descriptor: &'static str,
        /// The offending value, widened to `i64` so both signed and unsigned
        /// descriptors share one variant.
        value: i64,
    },

    /// `trailing_bits(0)` was requested. The parser rejects an empty trailing-bits
    /// field (AV2 v1.0.0 § 5.2.3,
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-3`, always writes at least
    /// the `trailing_one_bit`), so the writer never produces one.
    #[error("trailing_bits requires at least one bit")]
    EmptyTrailingBits,

    /// An [`ObuHeader`](crate::obu::ObuHeader)'s `has_header_extension` flag
    /// disagrees with its `header_size_bytes` (the flag is `true` iff the header is
    /// two bytes). Such a header could never have been produced by the parser.
    #[error(
        "OBU header extension flag ({flag}) is inconsistent with header_size_bytes ({size_bytes})"
    )]
    InconsistentHeader {
        /// The header's `has_header_extension` flag.
        flag: bool,
        /// The header's `header_size_bytes`.
        size_bytes: u8,
    },

    /// A no-extension [`ObuHeader`](crate::obu::ObuHeader) carries layer ids that the
    /// AV2 v1.0.0 § 5.2.2 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2`)
    /// parser could never infer (it derives `obu_mlayer_id = 0` and `obu_xlayer_id`
    /// to `GLOBAL_XLAYER_ID` for the global-scope types or `0` otherwise). Such ids are unrepresentable without the extension byte, so the
    /// writer rejects them rather than silently dropping them.
    #[error(
        "no-extension OBU header has non-inferable layer ids (mlayer {embedded}, xlayer {extended})"
    )]
    NonInferableLayerIds {
        /// The header's `embedded_layer_id` (`obu_mlayer_id`).
        embedded: u8,
        /// The header's `extended_layer_id` (`obu_xlayer_id`).
        extended: u8,
    },

    /// An Annex B OBU's total byte count (`header_size_bytes + payload.len()`)
    /// exceeds the LEB128 `u32` size domain (AV2 v1.0.0 § 4.11.6,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-6`).
    #[error("OBU size {total} exceeds the u32 leb128 domain")]
    ObuTooLarge {
        /// The computed total byte count that does not fit in a `u32`.
        total: u64,
    },

    /// A byte-granular framer (e.g. `write_annexb_obu`) was given a writer that is
    /// not on a byte boundary; the bytes it emits would be mis-positioned. The error
    /// is returned before any byte is written.
    #[error("writer is not byte-aligned")]
    WriterNotByteAligned,

    /// An [`ObuHeader`](crate::obu::ObuHeader)'s `obu_type` is a non-canonical
    /// `ObuType::Reserved(raw)` whose raw value the § 5.2.2 parser maps to a
    /// different variant on reparse (e.g. `Reserved(1)` reparses as a named type), so
    /// writing it would break `read(write(x)) == x`. Rejected before any byte.
    #[error("non-canonical obu_type with raw value {raw}")]
    NonCanonicalObuType {
        /// The header's `obu_type.raw()`.
        raw: u8,
    },

    /// A derived sequence-header value is inconsistent with the flags/fields that the
    /// AV2 v1.0.0 § 5.4.1 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`)
    /// parser would re-derive from on reparse, so the model could never have been
    /// produced by `parse_sequence_header_general` and writing it would break
    /// `read(write(x)) == x`. Examples: a `seq_tier == High` whose conditional gate
    /// (`seq_level_idx > 3 && !single_picture_header_flag`) is false; a single-picture
    /// header carrying a non-inferred constant; an `Option` field whose presence
    /// disagrees with its gating present-flag; a dependency map that does not match the
    /// § 5.4.1 default-fill / signaled-override re-derivation; or a cropping window that
    /// is non-default while `seq_cropping_window_present_flag == 0`. Rejected before any
    /// bit is written.
    #[error("non-canonical {what}: model value cannot be reproduced by the §5.4 parser")]
    NonCanonicalSequenceValue {
        /// A short, stable label for the offending field (e.g. `"seq_tier"`,
        /// `"mlayer_dependency_map"`).
        what: &'static str,
    },

    /// A [`SequenceHeader`](crate::headers::sequence::SequenceHeader) the AV2 v1.0.0
    /// § 5.4.2 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2`) parser could not
    /// fully parse: `seq_tile_info_present_flag == 1` while `seq_level_idx` is a reserved
    /// (non-conformant) level with no defined `tile_params()` (§ 5.18.7.3) bit layout, so
    /// the parser left a bounded residual (`SequenceHeader::unimplemented_at` /
    /// `SequenceTileConfig::unimplemented_at`) and never modeled the tile bits or any
    /// payload after them. The un-modeled tail cannot be re-emitted, so the writer rejects
    /// the whole header before writing any bit rather than producing a truncated stream.
    #[error("sequence header is not fully parsed (stopped at {feature})")]
    UnwritableSequenceHeader {
        /// The owning Feature ID at which the parser stopped (e.g.
        /// `"AV2-5.4.2-SEQUENCE-TILE-CONFIG"`).
        feature: &'static str,
    },

    /// A frame-header value is inconsistent with what the AV2 v1.0.0 § 5.18.2
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`) parser would re-derive on
    /// reparse, so the model could never have been produced by the frame-header parser and
    /// writing it would break `read(write(x)) == x`. Examples: an `is_*` / `startCVS` flag
    /// that disagrees with the `obu_type` derivation, a bridge frame carrying a non-zero
    /// `cur_mfh_id` (the parser infers `0`), or a `seq_header_id_in_frame_header` /
    /// `referenced_sequence_header_id` whose presence disagrees with the `cur_mfh_id == 0`
    /// gate. Rejected before any bit is written.
    #[error("non-canonical {what}: frame-header value cannot be reproduced by the §5.18 parser")]
    NonCanonicalFrameHeader {
        /// A short, stable label for the offending field (e.g. `"is_bridge"`,
        /// `"cur_mfh_id"`).
        what: &'static str,
    },

    /// A § 5.19 `tile_group_obu()` structure value either could not have been produced by
    /// `parse_tile_group_structure` (AV2 v1.0.0 § 5.19,
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`) — so writing it would break
    /// `read(write(x)) == x` — or is a § 6.18 conformance violation the writer refuses to emit.
    /// Non-reproducible examples: a non-`Complete` (truncated) structure, a degenerate
    /// `NumTiles == 0` layout, a `tg_start` / `tg_end` outside `f(tileBits)`, or a tile-range/flag
    /// combination the parser's inference could not produce. Conformance refusals (the § 5.19 parser
    /// tolerates these — it reads the `f(tileBits)` values without enforcing § 6.18 — but the writer
    /// will not emit a stream `splot validate` would reject): an inverted `tg_end < tg_start` range,
    /// or a `tg_end >= NumTiles` out-of-range index (a non-power-of-two grid has spare `f(tileBits)`
    /// codes above the last tile). Rejected before any bit is written.
    ///
    /// The same variant also covers § 5.20.1 `tile_group_payload()` framing
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`) the parser could not have produced: a
    /// defective framing, a bridge (unframeable `tile_size == 0`) framing, a tile-data count/length
    /// mismatch, a `TileSizeBytes` outside `1..=4`, a zero-size tile, or a `tile_size - 1` outside
    /// `le(TileSizeBytes)`.
    #[error(
        "non-canonical {what}: tile-group value cannot be reproduced by the §5.19/§5.20.1 parser"
    )]
    NonCanonicalTileGroup {
        /// A short, stable label for the offending field (e.g. `"incomplete_structure"`,
        /// `"tg_range"`, `"tile_data_len"`).
        what: &'static str,
    },

    /// A metadata-OBU value is inconsistent with what the AV2 v1.0.0 § 5.17
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17`) / § 6.16 parsers would
    /// re-derive on reparse, so the model could never have been produced by the metadata
    /// parsers and writing it would break `read(write(x)) == x`. Examples: a `metadata_type`
    /// that disagrees with its payload variant, a passthrough byte count that disagrees with
    /// the modeled `payload_len`, a `muh_*` field whose presence disagrees with
    /// `muh_cancel_flag`, a `muh_header_size` that cannot account for the bytes it must cover,
    /// or a stored `metadata_type_leb128_bytes` that cannot encode the value. Rejected before
    /// any bit is written.
    #[error("non-canonical {what}: metadata value cannot be reproduced by the §5.17 parser")]
    NonCanonicalMetadata {
        /// A short, stable label for the offending field (e.g. `"type_payload_mismatch"`,
        /// `"passthrough_len"`, `"muh_header_size"`).
        what: &'static str,
    },

    /// A buffer-removal-timing value is inconsistent with what the AV2 v1.0.0 § 5.12
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-12`) parser would re-derive on reparse, so
    /// the model could never have been produced by `parse_buffer_removal_timing` and writing it would
    /// break `read(write(x)) == x`. Examples: an `op_times` length that disagrees with `br_ops_cnt`,
    /// a per-operating-point `index` that disagrees with its position, or a `br_time_op` presence that
    /// disagrees with `br_decoder_model_present_op_flag`. Rejected before any bit is written.
    #[error(
        "non-canonical {what}: buffer-removal-timing value cannot be reproduced by the §5.12 parser"
    )]
    NonCanonicalBufferRemovalTiming {
        /// A short, stable label for the offending field (e.g. `"op_count"`, `"op_index"`,
        /// `"op_decoder_model_flag"`, `"passthrough"`).
        what: &'static str,
    },

    /// A multistream-decoder-operation value is inconsistent with what the AV2 v1.0.0 § 5.6
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-6`) parser would re-derive on reparse, so
    /// the model could never have been produced by `parse_msdo` and writing it would break
    /// `read(write(x)) == x`. Examples: a `multistream_large_picture_idc` presence that disagrees
    /// with `multistream_even_allocation_flag`, a `sub_stream_count` that disagrees with
    /// `num_streams_minus_2 + 2`, or a non-zero unused `sub_streams` slot. Rejected before any bit.
    #[error(
        "non-canonical {what}: multistream-decoder-operation value cannot be reproduced by the §5.6 parser"
    )]
    NonCanonicalMsdo {
        /// A short, stable label for the offending field (e.g. `"large_picture_idc_flag"`,
        /// `"sub_stream_count"`, `"unused_sub_stream"`, `"passthrough"`).
        what: &'static str,
    },

    /// An operating-point-set value is inconsistent with what the AV2 v1.0.0 § 5.10 / § 5.11
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-10`, and the § 5.11.1-§ 5.11.5 child
    /// structures) parser would re-derive on reparse, so the model could never have been produced by
    /// `parse_operating_point_set` and writing it would break `read(write(x)) == x`. Examples: an
    /// `obu_xlayer_id` that disagrees with the stored `xlayer_id`; a header `Option`/present-flag whose
    /// presence disagrees with the `ops_cnt > 0` (and global-vs-local) gate; a `payloads` length or a
    /// per-payload `index` that disagrees with `ops_cnt`; a gated payload `Option`
    /// (`op_intent`/`aggregate_info`/`color_info`/per-entry `ptl_info`/the color triple) whose presence
    /// disagrees with its flag; an `xlayer_entries` set/order that does not match the `ops_xlayer_map`
    /// bits; an mlayer source that disagrees with `ops_mlayer_info_idc`; an `ops_mlayer_info()`
    /// `tlayer_maps` set that does not match `ops_mlayer_map`; or a declared `ops_data_size` that
    /// disagrees with the re-derived `opsBytes`. Rejected before any bit is written.
    #[error(
        "non-canonical {what}: operating-point-set value cannot be reproduced by the §5.10/§5.11 parser"
    )]
    NonCanonicalOperatingPointSet {
        /// A short, stable label for the offending field (e.g. `"xlayer_id"`, `"payload_count"`,
        /// `"xlayer_entries"`, `"global_mlayer_source"`, `"ops_data_size"`, `"passthrough"`).
        what: &'static str,
    },

    /// A multi-frame-header value is inconsistent with what the AV2 v1.0.0 § 5.7
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-7`) parser would re-derive on reparse, so
    /// the model could never have been produced by `parse_multi_frame_header` and writing it would
    /// break `read(write(x)) == x`. Examples: a `mfh_apply_deblocking_filter[i]` that is `true`
    /// while `mfh_deblocking_filter_update` is `false` (the parser leaves the array all-`false`
    /// without an update); a stored `width_bits` / `height_bits` outside `1..=16` (it is
    /// `mfh_frame_*_bits_minus_1 + 1` with the `minus_1` an `f(4)` value); or a
    /// `mfh_seg_info_present_flag` that disagrees with the presence of the three segment-info
    /// `Option`s (`mfh_ext_seg_flag` / `mfh_allow_seg_info_change` / `segment_info`). Rejected
    /// before any bit is written.
    ///
    /// A `mfh_seq_header_id` / `mfh_id_minus_1` out of its § 6.x conformance range — which the
    /// validator flags but the parser preserves verbatim — is **not** rejected; the writer
    /// reproduces it exactly.
    #[error(
        "non-canonical {what}: multi-frame-header value cannot be reproduced by the §5.7 parser"
    )]
    NonCanonicalMultiFrameHeader {
        /// A short, stable label for the offending field (e.g. `"deblocking_apply_forced_false"`,
        /// `"frame_width_bits"`, `"frame_height_bits"`, `"seg_info_present_flag"`,
        /// `"passthrough"`).
        what: &'static str,
    },

    /// A content-interpretation value is inconsistent with what the AV2 v1.0.0 § 5.15
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-15`) parser would re-derive on reparse, so
    /// the model could never have been produced by `parse_content_interpretation` and writing it
    /// would break `read(write(x)) == x`. Examples: a `ColorDescription::primaries` presence that
    /// disagrees with `ci_color_description_idc == 0`; a progressive (`ci_scan_type_idc == 1`) chroma
    /// position whose `bottom` differs from its `top` (the parser infers them equal and codes no
    /// bottom); an `AspectRatioInfo::extended_sar` presence that disagrees with
    /// `ci_aspect_ratio_idc == 255`; or a `num_ticks_per_picture_minus_1` presence that disagrees
    /// with `equal_picture_interval`. Rejected before any bit is written.
    ///
    /// A reserved-but-tolerated value the parser preserves verbatim — a reserved
    /// `ci_color_description_idc` (`6..=127`), a reserved `ci_aspect_ratio_idc` (`17..=254`), a
    /// reserved `ci_scan_type_idc`, or a non-zero `ci_reserved_2bit` — is **not** rejected; the writer
    /// reproduces it exactly.
    #[error(
        "non-canonical {what}: content-interpretation value cannot be reproduced by the §5.15 parser"
    )]
    NonCanonicalContentInterpretation {
        /// A short, stable label for the offending field (e.g. `"color_primaries_idc"`,
        /// `"chroma_bottom_progressive"`, `"extended_sar_idc"`, `"timing_num_ticks_gate"`,
        /// `"passthrough"`).
        what: &'static str,
    },

    /// An atlas-segment value is inconsistent with what the AV2 v1.0.0 § 5.9
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-9`, and the § 5.9.1 – § 5.9.5 child
    /// structures) parser would re-derive on reparse, so the model could never have been produced by
    /// `parse_atlas_segment` and writing it would break `read(write(x)) == x`. Examples: an
    /// [`AtlasModeInfo`](crate::headers::atlas_segment::AtlasModeInfo) variant that disagrees with
    /// `mode`; a stored `num_segments` that disagrees with the value re-derived from the mode body (or
    /// that exceeds the § 6.9.6 bound); a region column/row count outside the § 6.9.3.1 bound; a
    /// uniform-vs-explicit region-dimension shape that disagrees with `ats_uniform_spacing_flag` and
    /// the region counts; a `NumRegionsInAtlas` that disagrees with the count product; a
    /// single-region-per-segment mapping carrying an explicit segment list; a per-mode
    /// `num_atlas_segments_minus_1` outside the bound or a count-vs-`Vec`-length mismatch; an
    /// `ats_input_stream_id` presence that disagrees with `ats_stream_id_present`; an
    /// `alpha_segments_present` or per-segment `alpha_segment_flag` that disagrees with the
    /// alpha-vs-non-alpha mode and the § 6.9.5 last-segment inference; a `segment_ids` length that
    /// disagrees with `numSegments`; or an unsignaled label that is not the inferred identity ids.
    /// Rejected before any bit is written.
    ///
    /// A § 6.9.2 descriptive id-assignment element the parser preserves verbatim — any
    /// `ats_atlas_segment_id`, any `ats_signaled_atlas_segment_ids_flag`, or any signaled
    /// `segment_ids` value — is **not** rejected; the writer reproduces it exactly.
    #[error("non-canonical {what}: atlas-segment value cannot be reproduced by the §5.9 parser")]
    NonCanonicalAtlasSegment {
        /// A short, stable label for the offending field (e.g. `"mode_info_variant"`,
        /// `"num_segments"`, `"region_dimension"`, `"region_uniform_dims"`,
        /// `"num_regions_in_atlas"`, `"single_region_segments"`, `"segment_count"`,
        /// `"stream_id_gate"`, `"alpha_segments_gate"`, `"label_segment_count"`,
        /// `"label_unsignaled_ids"`, `"passthrough"`).
        what: &'static str,
    },

    /// A layer-configuration-record value is inconsistent with what the AV2 v1.0.0 § 5.8
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-8`, and the § 5.8.1 – § 5.8.9 child
    /// structures) parser would re-derive on reparse, so the model could never have been produced by
    /// `parse_layer_config_record` and writing it would break `read(write(x)) == x`. Examples: a
    /// `Global` / `Local` record variant — or a local record's `xlayer_id` — that disagrees with the
    /// OBU header's `obu_xlayer_id` (the parser selects the variant and fills the local ids from it,
    /// so the dispatch threads it in and a disagreement is parser-unproducible); a local
    /// `lcr_seq_profile_tier_level_info` whose `xlayer_id` disagrees with the record's `xlayer_id`
    /// (the parser passes the one `xId` into both); a
    /// `lcr_global_atlas_id_present_flag` (or `lcr_local_atlas_id_present_flag`) that disagrees with the
    /// presence of `global_atlas_id` / `local_atlas_id`, or a non-zero `reserved_zero_3bits` retained
    /// alongside a present atlas id (the parser codes one or the other, forcing the reserved field to
    /// `0` when an atlas id is read); a `seq_ptl_infos` or `payloads` list whose length or per-element
    /// `xlayer_id` disagrees with the `lcr_xlayer_map` set-bit ids the loops iterate; an
    /// `lcr_aggregate_info_present_flag` / `lcr_seq_profile_tier_level_info_present_flag` /
    /// `lcr_global_payload_present_flag` that disagrees with its section's presence; an
    /// `lcr_global_payload` whose written content bits plus `remaining_payload_bits` do not equal
    /// `lcr_data_size * 8`, or a `num_dependent_xlayer_map` presence that disagrees with
    /// `lcr_dependent_xlayers_flag && n > 0`; both an embedded-layer info and the else-branch atlas
    /// reference set at once, or the else-branch atlas reference present without `isGlobal &&
    /// lcr_global_atlas_id_present_flag`; an `lcr_embedded_layer_info.layers` set whose length or
    /// per-element `mlayer_index` disagrees with the `lcr_mlayer_map` set bits; a per-embedded-layer
    /// atlas / `lcr_auxiliary_type` / `lcr_view_id` / `lcr_dependent_layer_map` / max-expected-resolution
    /// `Option` whose presence disagrees with its gate; or an `lcr_xlayer_color_info` primaries presence
    /// that disagrees with `layer_color_description_idc == 0`. Rejected before any bit is written.
    ///
    /// The four `lcr_xlayer_info` present flags and the `lcr_format_info` / `lcr_cropping_window`
    /// present flags are not stored in the model — the writer derives each from its `Option`'s
    /// presence — so no flag-vs-`Option` disagreement is representable there.
    ///
    /// A § 6.8 reserved-zero field the parser preserves verbatim — any `lcr_*_reserved_zero_3bits` /
    /// `lcr_*_reserved_zero_5bits` / `lsptli_reserved_2bits`, or an out-of-conformance-range
    /// `lcr_global_config_record_id` / `lcr_local_id` — is **not** rejected; the writer reproduces it
    /// exactly within the field's descriptor domain.
    #[error(
        "non-canonical {what}: layer-config-record value cannot be reproduced by the §5.8 parser"
    )]
    NonCanonicalLayerConfigRecord {
        /// A short, stable label for the offending field (e.g. `"xlayer_scope"`,
        /// `"local_xlayer_id"`, `"local_ptl_xlayer_id"`, `"global_atlas_id_gate"`,
        /// `"atlas_reserved_3bits"`, `"aggregate_info_gate"`, `"seq_ptl_info_count"`,
        /// `"seq_ptl_xlayer_id"`, `"payload_count"`, `"payload_xlayer_id"`, `"payload_size"`,
        /// `"num_dependent_gate"`, `"local_ptl_gate"`, `"embedded_atlas_exclusive"`,
        /// `"xlayer_atlas_gate"`, `"mlayer_layer_count"`, `"mlayer_index"`, `"embedded_atlas_gate"`,
        /// `"aux_type_gate"`, `"view_id_gate"`, `"dependent_layer_map_gate"`, `"max_expected_gate"`,
        /// `"color_primaries_gate"`, `"passthrough"`).
        what: &'static str,
    },

    /// A film-grain value is inconsistent with what the AV2 v1.0.0 § 5.14
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-14`) / § 5.18.10.2
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10-2`) parsers would re-derive on reparse, so
    /// the model could never have been produced by `parse_film_grain` / `parse_film_grain_model` and
    /// writing it would break `read(write(x)) == x`. The model is lossy versus the wire format — it
    /// stores only the cumulative scaling-point `value` / `scaling` and the de-biased AR `coeffs`, not
    /// the wire bit-widths (`point_value_increment_bits_minus_1`, `point_scaling_bits_minus_5`,
    /// `bits_per_ar_coeff_*_minus_5`) — so the writer re-derives a minimal in-range width per array (a
    /// canonicalization, like leb128-minimal); byte-exactness is not guaranteed, but semantic
    /// round-trip is (the widths are not in the model's `PartialEq`). Examples: a `sub_x` / `sub_y` /
    /// `monochrome` that disagrees with re-deriving them from `chroma_idc`; a `models` set whose slots
    /// do not match the `update_flags` set bits in ascending order; a count-vs-`Vec`-length mismatch
    /// (`num_y_points` / `num_cb_points` / `num_cr_points` / the AR-coeff lengths); a gated-`Option`
    /// presence-vs-gate mismatch (`cb_mult` / `cb_offset` vs `num_cb_points > 0`, etc.); a
    /// `chroma_scaling_from_luma` / `mc_identity` forced false by the parser but stored `true`;
    /// non-monotonic scaling points; or a scaling increment / scaling value / AR coeff that fits no
    /// in-range bit width. Rejected before any bit is written.
    #[error("non-canonical {what}: film-grain value cannot be reproduced by the §5.14 parser")]
    NonCanonicalFilmGrain {
        /// A short, stable label for the offending field (e.g. `"chroma_subsampling"`,
        /// `"slot_update_flags"`, `"num_y_points_len"`, `"cb_mult_gate"`,
        /// `"monochrome_chroma_scaling"`, `"non_monotonic_points"`, `"point_increment_width"`,
        /// `"passthrough"`).
        what: &'static str,
    },

    /// A quantizer-matrix value is inconsistent with what the AV2 v1.0.0 § 5.13 / § 5.4.11
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-13`,
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-11`) parser would re-derive on reparse, so the
    /// model could never have been produced by `parse_quantizer_matrix` and writing it would break
    /// `read(write(x)) == x`. The model is lossy versus the wire format — it stores only the *decoded*
    /// `UserDefinedQmPlane::values` (each coefficient `1..=255`), not the wire `quant_delta`s or the
    /// optional `qm_8x8_is_symmetric` / `qm_4x8_is_transpose_of_8x4` / `qm_copy_from_previous_plane` /
    /// coefficient-repeat compressions — so the writer canonicalizes to the long form (every skip flag
    /// `0`, one `svlc()` delta per cell in 2D diagonal scan order, recomputing the minimal in-range
    /// `quant_delta`); byte-exactness is not guaranteed, but semantic round-trip is. Examples: a
    /// `num_planes` that disagrees with `qm_chroma_info_present_flag`; a `levels` set whose length or
    /// per-element `level` does not match the `qm_bit_map` set bits in ascending order; an `is_default`
    /// flag that disagrees with the `matrices` `Option`; a `matrices` set whose length or order does not
    /// match `Fundamental_Tx_Size`; a plane count, width / height, or value-count mismatch; or a
    /// coefficient of `0` (the parser never decodes a `0` — `quant2 == 0` is the repeat sentinel, not a
    /// stored coefficient — so it is unrepresentable). Rejected before any bit is written.
    #[error(
        "non-canonical {what}: quantizer-matrix value cannot be reproduced by the §5.13 parser"
    )]
    NonCanonicalQuantizationMatrix {
        /// A short, stable label for the offending field (e.g. `"num_planes"`, `"level_count"`,
        /// `"level_index"`, `"is_default_gate"`, `"transform_count"`, `"transform_order"`,
        /// `"plane_count"`, `"plane_dimensions"`, `"plane_value_count"`, `"coefficient_zero"`,
        /// `"passthrough"`).
        what: &'static str,
    },

    /// A writer for this OBU payload type does not exist yet — an honest stub returned by the
    /// complete-OBU dispatch for the `ParsedObu` variants whose body writer has not landed. Distinct
    /// from a non-canonical reject: the model is fine, the writer is simply not implemented.
    #[error("no writer implemented for {feature}")]
    Unimplemented {
        /// The matrix Feature ID of the unimplemented OBU type (e.g. `"AV2-5.15-CONTENT-INTERPRETATION"`).
        feature: &'static str,
    },

    /// An [`ObuHeader`](crate::obu::ObuHeader)'s `obu_type` does not select the
    /// [`ParsedObu`](crate::obu::ParsedObu) payload variant it was paired with in the complete-OBU
    /// writer (e.g. a `SequenceHeader` header with a `Padding` payload). The § 5.2.1 OBU dispatch
    /// routes a single `obu_type` to exactly one payload syntax, so such a pair could never have come
    /// from parsing one OBU; writing it would reparse as the header's type and break
    /// `read(write(x)) == x`. Rejected before any bit is written.
    #[error("OBU header type does not select the {payload} payload")]
    ObuTypePayloadMismatch {
        /// The mispaired payload's syntax name ([`ParsedObu::syntax_name`](crate::obu::ParsedObu::syntax_name)).
        payload: &'static str,
    },
}

/// Result alias for [`crate::write::BitWriter`] operations.
pub type WriteResult<T> = core::result::Result<T, WriteError>;
