// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `atlas_segment_info_obu()` writer (AV2 v1.0.0 § 5.9,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-9`) — the inverse of
//! [`crate::headers::atlas_segment::parse_atlas_segment`].
//!
//! The OBU codes `atlas_segment_id` `f(3)`, then `ats_atlas_segment_mode_idc`
//! `uvlc()` selecting one of five per-mode bodies, and closes with
//! `ats_label_segment_info()` (§ 5.9.1). The five modes are:
//!
//! - `ENHANCED_ATLAS` (idc 0): `ats_enhanced_atlas_info()` (§ 5.9.2) — `ats_region_info()`
//!   (§ 5.9.2.1, a region grid with uniform or explicit column/row dimensions) then
//!   `ats_region_to_segment_mapping()` (§ 5.9.2.2, a single-region-per-segment flag
//!   plus, when not single-region, an explicit per-segment region-rectangle list).
//! - `BASIC_ATLAS` (idc 1): `ats_basic_info()` (§ 5.9.5) — explicit per-segment
//!   rectangles with an optional per-segment `ats_input_stream_id`.
//! - `SINGLE_ATLAS` (idc 2): the nominal width/height pair (§ 5.9), one segment.
//! - `MULTISTREAM_ATLAS` (idc 3) / `MULTISTREAM_ALPHA_ATLAS` (idc 4):
//!   `ats_multistream_info()` / `ats_multistream_with_alpha_info()` (§ 5.9.3 / § 5.9.4)
//!   — per-segment input-stream composition, optional background, and (alpha variant
//!   only) a per-segment alpha flag.
//!
//! ## The derived `numSegments`
//!
//! `numSegments` is **not** a wire field: the parser derives it from the mode body
//! (`1` for `SINGLE_ATLAS`, `num_atlas_segments_minus_1 + 1` for the explicit-count
//! modes, `mapping.num_atlas_segments_minus_1 + 1` for `ENHANCED_ATLAS`, where for the
//! single-region path `num_atlas_segments_minus_1 = num_regions_in_atlas - 1`). The
//! writer re-derives it the same way from the mode body and rejects a stored
//! `AtlasSegment::num_segments` that disagrees, so the parse-context value round-trips
//! without being written. It also drives the `ats_label_segment_info()` id loop length,
//! which must match `label.segment_ids.len()`.
//!
//! ## Tolerated values reproduced verbatim
//!
//! § 6.9.2 notes that `ats_atlas_segment_id` and `ats_signaled_atlas_segment_ids_flag`
//! are descriptive id-assignment elements with no bitstream-conformance requirement, so
//! any `atlas_segment_id` (`f(3)`) and any signaled `segment_ids` (`f(8)` each) the
//! parser preserves are reproduced exactly, never rejected.
//!
//! `OBU_ATLAS_SEGMENT` is an **extensible** OBU type (§ 5.2.1), so the OBU tail is the
//! dispatch's generic extensible tail (`obu_extension_flag = 0` then `trailing_bits()`);
//! this writer emits the body, not the tail.

use crate::headers::atlas_segment::{
    AtlasBasicInfo, AtlasBasicSegment, AtlasEnhancedInfo, AtlasLabelSegmentInfo, AtlasModeInfo,
    AtlasMultistreamInfo, AtlasMultistreamSegment, AtlasRegionInfo, AtlasRegionToSegmentMapping,
    AtlasSegment, AtlasSegmentMode, AtlasSegmentRegion, AtlasSingleInfo,
};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `atlas_segment_id` is `f(3)`.
const ATLAS_SEGMENT_ID_BITS: u32 = 3;
/// `ats_input_stream_id` / `ats_msi_input_stream_id` are `f(5)`.
const INPUT_STREAM_ID_BITS: u32 = 5;
/// `ats_atlas_segment_id` (the signaled label id) is `f(8)`.
const LABEL_SEGMENT_ID_BITS: u32 = 8;
/// `ats_msi_background_red/green/blue_value` are `f(8)` each.
const BACKGROUND_F8: u32 = 8;
/// `MAX_NUM_ATLAS_SEGMENTS` (AV2 § 3): the conformance bound the parser enforces on the
/// per-mode segment count (`num_atlas_segments_minus_1 < MAX_NUM_ATLAS_SEGMENTS`, and
/// the derived `numSegments <= MAX_NUM_ATLAS_SEGMENTS`).
const MAX_NUM_ATLAS_SEGMENTS: u32 = 256;
/// `MAX_ATLAS_COLS` (AV2 § 3): the conformance bound the parser enforces on
/// `ats_num_region_columns_minus_1` (`< MAX_ATLAS_COLS`).
const MAX_ATLAS_COLS: u32 = 64;
/// `MAX_ATLAS_ROWS` (AV2 § 3): the conformance bound the parser enforces on
/// `ats_num_region_rows_minus_1` (`< MAX_ATLAS_ROWS`).
const MAX_ATLAS_ROWS: u32 = 64;

/// Writes an `atlas_segment_info_obu()` body (AV2 v1.0.0 § 5.9), the inverse of
/// [`crate::headers::atlas_segment::parse_atlas_segment`]. The OBU header and the
/// extensible OBU tail are the dispatch's job ([`crate::write::write_complete_obu`]);
/// this writes the typed body only.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU
///   payload begins on a byte boundary).
/// - [`WriteError::NonCanonicalAtlasSegment`] for a constructed model the § 5.9 parser
///   could never produce, so it would not round-trip. The `what` label names the
///   offending field:
///   - `"mode_info_variant"`: the [`AtlasModeInfo`] variant disagrees with `mode` (the
///     parser builds the variant from the mode, so a mismatch is parser-unproducible).
///   - `"num_segments"`: the stored `num_segments` disagrees with the value the writer
///     re-derives from the mode body (the parse-context derivation), or it exceeds
///     `MAX_NUM_ATLAS_SEGMENTS` (the parser's bound).
///   - `"region_dimension"`: `ats_num_region_columns_minus_1 >= MAX_ATLAS_COLS` or
///     `ats_num_region_rows_minus_1 >= MAX_ATLAS_ROWS` (the parser rejects these).
///   - `"region_uniform_dims"`: the `region_width_minus_1` / `region_height_minus_1`
///     uniform pair or the `column_widths_minus_1` / `row_heights_minus_1` explicit
///     lists do not match `uniform_spacing` and the region counts (the parser reads
///     exactly one of the two forms, sized by the counts).
///   - `"num_regions_in_atlas"`: the stored `num_regions_in_atlas` disagrees with
///     `(columns_minus_1 + 1) * (rows_minus_1 + 1)` (the parser's derivation).
///   - `"single_region_segments"`: a single-region-per-segment mapping carries an
///     explicit `segments` list (the parser leaves it empty and derives the count from
///     `num_regions_in_atlas`), or its stored `num_atlas_segments_minus_1` disagrees
///     with `num_regions_in_atlas - 1`.
///   - `"segment_count"`: a per-mode `num_atlas_segments_minus_1 >= MAX_NUM_ATLAS_SEGMENTS`
///     (the parser's bound), or a `segments` `Vec` whose length disagrees with
///     `num_atlas_segments_minus_1 + 1`.
///   - `"stream_id_gate"`: a basic-mode segment's `input_stream_id` presence disagrees
///     with `stream_id_present` (the parser reads it iff the flag is set).
///   - `"alpha_segments_gate"`: the multistream `alpha_segments_present` presence
///     disagrees with the alpha-vs-non-alpha mode (`Some` only for `MULTISTREAM_ALPHA`),
///     or a non-last/last per-segment `alpha_segment_flag` disagrees with the § 6.9.5
///     inference (the last segment's flag is inferred `false` and coded for none of the
///     non-alpha variant's segments).
///   - `"label_segment_count"`: `label.segment_ids.len()` disagrees with the derived
///     `numSegments`.
///   - `"label_unsignaled_ids"`: an unsignaled label (`signaled_atlas_segment_ids ==
///     false`) does not carry the inferred identity ids (`segment_ids[i] == i`), which
///     the parser fills.
/// - [`WriteError::ValueTooWide`] / [`WriteError::ValueOutOfRange`] from the primitive
///   writers for a field value outside its descriptor's domain.
///
/// All checks run before any bit reaches `writer` (the body is drafted into a scratch
/// and appended only on full success), so a rejected model leaves `writer` unchanged and
/// the writer never panics.
pub fn write_atlas_segment(writer: &mut BitWriter, atlas: &AtlasSegment) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    if !mode_matches_info(atlas.mode, &atlas.mode_info) {
        return Err(non_canonical("mode_info_variant"));
    }

    let derived_num_segments = derive_num_segments(&atlas.mode_info)?;
    if atlas.num_segments != derived_num_segments {
        return Err(non_canonical("num_segments"));
    }
    if derived_num_segments > MAX_NUM_ATLAS_SEGMENTS {
        return Err(non_canonical("num_segments"));
    }

    let mut scratch = BitWriter::new();
    scratch.write_bits_u8(atlas.atlas_segment_id, ATLAS_SEGMENT_ID_BITS)?;
    scratch.write_uvlc(atlas.mode.idc())?;

    match &atlas.mode_info {
        AtlasModeInfo::Enhanced(info) => write_enhanced_atlas_info(&mut scratch, info)?,
        AtlasModeInfo::Basic(info) => write_basic_info(&mut scratch, info)?,
        AtlasModeInfo::Single(info) => write_single_info(&mut scratch, *info)?,
        AtlasModeInfo::Multistream(info) => write_multistream_info(&mut scratch, info, false)?,
        AtlasModeInfo::MultistreamAlpha(info) => write_multistream_info(&mut scratch, info, true)?,
    }

    write_label_segment_info(&mut scratch, &atlas.label, derived_num_segments)?;

    writer.append(&scratch)
}

/// Returns `true` when `info`'s variant is the one the § 5.9 parser builds for `mode`.
fn mode_matches_info(mode: AtlasSegmentMode, info: &AtlasModeInfo) -> bool {
    matches!(
        (mode, info),
        (AtlasSegmentMode::Enhanced, AtlasModeInfo::Enhanced(_))
            | (AtlasSegmentMode::Basic, AtlasModeInfo::Basic(_))
            | (AtlasSegmentMode::Single, AtlasModeInfo::Single(_))
            | (AtlasSegmentMode::Multistream, AtlasModeInfo::Multistream(_))
            | (
                AtlasSegmentMode::MultistreamAlpha,
                AtlasModeInfo::MultistreamAlpha(_)
            )
    )
}

/// Re-derives `numSegments` from the mode body exactly as `parse_atlas_segment` does:
/// `1` for `SINGLE_ATLAS`, `num_atlas_segments_minus_1 + 1` for the explicit-count modes,
/// and `mapping.num_atlas_segments_minus_1 + 1` for `ENHANCED_ATLAS`.
///
/// # Errors
/// Returns [`WriteError::NonCanonicalAtlasSegment`] with `what == "segment_count"` if a
/// per-mode `num_atlas_segments_minus_1` is `>= MAX_NUM_ATLAS_SEGMENTS` (the parser's
/// bound, which also keeps the `+ 1` from overflowing `u32`).
fn derive_num_segments(info: &AtlasModeInfo) -> WriteResult<u32> {
    let minus_1 = match info {
        AtlasModeInfo::Single(_) => return Ok(1),
        AtlasModeInfo::Enhanced(info) => info.mapping.num_atlas_segments_minus_1,
        AtlasModeInfo::Basic(info) => info.num_atlas_segments_minus_1,
        AtlasModeInfo::Multistream(info) | AtlasModeInfo::MultistreamAlpha(info) => {
            info.num_atlas_segments_minus_1
        }
    };
    if minus_1 >= MAX_NUM_ATLAS_SEGMENTS {
        return Err(non_canonical("segment_count"));
    }
    Ok(minus_1 + 1)
}

/// Writes the `SINGLE_ATLAS` nominal dimensions (AV2 v1.0.0 § 5.9).
fn write_single_info(scratch: &mut BitWriter, info: AtlasSingleInfo) -> WriteResult<()> {
    scratch.write_uvlc(info.nominal_width_minus_1)?;
    scratch.write_uvlc(info.nominal_height_minus_1)
}

/// Writes `ats_enhanced_atlas_info(xAId)` (AV2 v1.0.0 § 5.9.2): `ats_region_info()`
/// then `ats_region_to_segment_mapping()`.
fn write_enhanced_atlas_info(scratch: &mut BitWriter, info: &AtlasEnhancedInfo) -> WriteResult<()> {
    write_region_info(scratch, &info.region)?;
    write_region_to_segment_mapping(scratch, &info.region, &info.mapping)
}

/// Writes `ats_region_info(xAId)` (AV2 v1.0.0 § 5.9.2.1): the column/row counts, the
/// uniform-spacing flag, then either the uniform region width/height pair or the explicit
/// per-column / per-row dimension lists.
fn write_region_info(scratch: &mut BitWriter, region: &AtlasRegionInfo) -> WriteResult<()> {
    if region.num_region_columns_minus_1 >= MAX_ATLAS_COLS
        || region.num_region_rows_minus_1 >= MAX_ATLAS_ROWS
    {
        return Err(non_canonical("region_dimension"));
    }
    let columns = region.num_region_columns_minus_1 + 1;
    let rows = region.num_region_rows_minus_1 + 1;

    if region.num_regions_in_atlas != columns.saturating_mul(rows) {
        return Err(non_canonical("num_regions_in_atlas"));
    }

    if region.uniform_spacing {
        if region.region_width_minus_1.is_none()
            || region.region_height_minus_1.is_none()
            || !region.column_widths_minus_1.is_empty()
            || !region.row_heights_minus_1.is_empty()
        {
            return Err(non_canonical("region_uniform_dims"));
        }
    } else if region.region_width_minus_1.is_some()
        || region.region_height_minus_1.is_some()
        || region.column_widths_minus_1.len() != columns as usize
        || region.row_heights_minus_1.len() != rows as usize
    {
        return Err(non_canonical("region_uniform_dims"));
    }

    scratch.write_uvlc(region.num_region_columns_minus_1)?;
    scratch.write_uvlc(region.num_region_rows_minus_1)?;
    scratch.write_flag(region.uniform_spacing)?;
    if region.uniform_spacing {
        let width = region
            .region_width_minus_1
            .ok_or_else(|| non_canonical("region_uniform_dims"))?;
        let height = region
            .region_height_minus_1
            .ok_or_else(|| non_canonical("region_uniform_dims"))?;
        scratch.write_uvlc(width)?;
        scratch.write_uvlc(height)?;
    } else {
        for &width in &region.column_widths_minus_1 {
            scratch.write_uvlc(width)?;
        }
        for &height in &region.row_heights_minus_1 {
            scratch.write_uvlc(height)?;
        }
    }
    Ok(())
}

/// Writes `ats_region_to_segment_mapping(xAId)` (AV2 v1.0.0 § 5.9.2.2): the
/// single-region-per-segment flag and, when not single-region, the coded
/// `ats_num_atlas_segments_minus_1` followed by the per-segment region rectangles.
fn write_region_to_segment_mapping(
    scratch: &mut BitWriter,
    region: &AtlasRegionInfo,
    mapping: &AtlasRegionToSegmentMapping,
) -> WriteResult<()> {
    scratch.write_flag(mapping.single_region_per_atlas_segment)?;
    if mapping.single_region_per_atlas_segment {
        if !mapping.segments.is_empty() {
            return Err(non_canonical("single_region_segments"));
        }
        if mapping.num_atlas_segments_minus_1 != region.num_regions_in_atlas.saturating_sub(1) {
            return Err(non_canonical("single_region_segments"));
        }
    } else {
        if mapping.num_atlas_segments_minus_1 >= MAX_NUM_ATLAS_SEGMENTS {
            return Err(non_canonical("segment_count"));
        }
        let count = mapping.num_atlas_segments_minus_1 as usize + 1;
        if mapping.segments.len() != count {
            return Err(non_canonical("segment_count"));
        }
        scratch.write_uvlc(mapping.num_atlas_segments_minus_1)?;
        for segment in &mapping.segments {
            write_segment_region(scratch, segment)?;
        }
    }
    Ok(())
}

/// Writes one `ats_region_to_segment_mapping()` per-segment region rectangle
/// (AV2 v1.0.0 § 5.9.2.2).
fn write_segment_region(scratch: &mut BitWriter, segment: &AtlasSegmentRegion) -> WriteResult<()> {
    scratch.write_uvlc(segment.top_left_region_column)?;
    scratch.write_uvlc(segment.top_left_region_row)?;
    scratch.write_uvlc(segment.bottom_right_region_column_off)?;
    scratch.write_uvlc(segment.bottom_right_region_row_off)
}

/// Writes `ats_basic_info(xAId)` (AV2 v1.0.0 § 5.9.5): the stream-id-present flag, the
/// atlas width/height, the coded `ats_num_atlas_segments_minus_1`, then the per-segment
/// rectangles (each with an optional `ats_input_stream_id`).
fn write_basic_info(scratch: &mut BitWriter, info: &AtlasBasicInfo) -> WriteResult<()> {
    if info.num_atlas_segments_minus_1 >= MAX_NUM_ATLAS_SEGMENTS {
        return Err(non_canonical("segment_count"));
    }
    let count = info.num_atlas_segments_minus_1 as usize + 1;
    if info.segments.len() != count {
        return Err(non_canonical("segment_count"));
    }

    scratch.write_flag(info.stream_id_present)?;
    scratch.write_uvlc(info.width)?;
    scratch.write_uvlc(info.height)?;
    scratch.write_uvlc(info.num_atlas_segments_minus_1)?;
    for segment in &info.segments {
        write_basic_segment(scratch, info.stream_id_present, segment)?;
    }
    Ok(())
}

/// Writes one `ats_basic_info()` segment (AV2 v1.0.0 § 5.9.5). `stream_id_present`
/// gates the optional `ats_input_stream_id` `f(5)`.
fn write_basic_segment(
    scratch: &mut BitWriter,
    stream_id_present: bool,
    segment: &AtlasBasicSegment,
) -> WriteResult<()> {
    if stream_id_present {
        let stream_id = segment
            .input_stream_id
            .ok_or_else(|| non_canonical("stream_id_gate"))?;
        scratch.write_bits_u8(stream_id, INPUT_STREAM_ID_BITS)?;
    } else if segment.input_stream_id.is_some() {
        return Err(non_canonical("stream_id_gate"));
    }
    scratch.write_uvlc(segment.top_left_pos_x)?;
    scratch.write_uvlc(segment.top_left_pos_y)?;
    scratch.write_uvlc(segment.width)?;
    scratch.write_uvlc(segment.height)
}

/// Writes `ats_multistream_info()` / `ats_multistream_with_alpha_info()`
/// (AV2 v1.0.0 § 5.9.3 / § 5.9.4): the atlas width/height, the coded
/// `ats_msi_num_atlas_segments_minus_1`, the alpha-segments-present flag (alpha variant
/// only), the optional background, then the per-segment composition (each with a
/// conditionally-coded alpha flag).
fn write_multistream_info(
    scratch: &mut BitWriter,
    info: &AtlasMultistreamInfo,
    with_alpha: bool,
) -> WriteResult<()> {
    if info.alpha_segments_present.is_some() != with_alpha {
        return Err(non_canonical("alpha_segments_gate"));
    }
    if info.num_atlas_segments_minus_1 >= MAX_NUM_ATLAS_SEGMENTS {
        return Err(non_canonical("segment_count"));
    }
    let count = info.num_atlas_segments_minus_1 as usize + 1;
    if info.segments.len() != count {
        return Err(non_canonical("segment_count"));
    }

    scratch.write_uvlc(info.width)?;
    scratch.write_uvlc(info.height)?;
    scratch.write_uvlc(info.num_atlas_segments_minus_1)?;
    if let Some(alpha_present) = info.alpha_segments_present {
        scratch.write_flag(alpha_present)?;
    }
    scratch.write_flag(info.background.is_some())?;
    if let Some((red, green, blue)) = info.background {
        scratch.write_bits_u8(red, BACKGROUND_F8)?;
        scratch.write_bits_u8(green, BACKGROUND_F8)?;
        scratch.write_bits_u8(blue, BACKGROUND_F8)?;
    }

    let alpha_coded = info.alpha_segments_present == Some(true);
    let last_index = info.num_atlas_segments_minus_1;
    for (i, segment) in info.segments.iter().enumerate() {
        let is_last = i as u32 == last_index;
        let codes_alpha = alpha_coded && !is_last;
        if !codes_alpha && segment.alpha_segment_flag {
            return Err(non_canonical("alpha_segments_gate"));
        }
        write_multistream_segment(scratch, segment, codes_alpha)?;
    }
    Ok(())
}

/// Writes one `ats_multistream_info()` segment (AV2 v1.0.0 § 5.9.3 / § 5.9.4).
/// `codes_alpha` is `true` only for a non-last segment of the alpha variant, in which
/// case the per-segment `ats_msi_alpha_segment_flag` `f(1)` follows the rectangle.
fn write_multistream_segment(
    scratch: &mut BitWriter,
    segment: &AtlasMultistreamSegment,
    codes_alpha: bool,
) -> WriteResult<()> {
    scratch.write_bits_u8(segment.input_stream_id, INPUT_STREAM_ID_BITS)?;
    scratch.write_uvlc(segment.top_left_pos_x)?;
    scratch.write_uvlc(segment.top_left_pos_y)?;
    scratch.write_uvlc(segment.width)?;
    scratch.write_uvlc(segment.height)?;
    if codes_alpha {
        scratch.write_flag(segment.alpha_segment_flag)?;
    }
    Ok(())
}

/// Writes `ats_label_segment_info(xlayerId, xAId, numSegments)` (AV2 v1.0.0 § 5.9.1):
/// `ats_signaled_atlas_segment_ids_flag` `f(1)` then, when signaled, `numSegments`
/// explicit `ats_atlas_segment_id` `f(8)` values; when not signaled the ids are the
/// inferred identity indices (`segment_ids[i] == i`) and no bits are coded.
fn write_label_segment_info(
    scratch: &mut BitWriter,
    label: &AtlasLabelSegmentInfo,
    num_segments: u32,
) -> WriteResult<()> {
    if label.segment_ids.len() != num_segments as usize {
        return Err(non_canonical("label_segment_count"));
    }

    scratch.write_flag(label.signaled_atlas_segment_ids)?;
    if label.signaled_atlas_segment_ids {
        for &id in &label.segment_ids {
            scratch.write_bits_u8(id, LABEL_SEGMENT_ID_BITS)?;
        }
    } else {
        for (i, &id) in label.segment_ids.iter().enumerate() {
            let expected = u8::try_from(i).unwrap_or(u8::MAX);
            if id != expected {
                return Err(non_canonical("label_unsignaled_ids"));
            }
        }
    }
    Ok(())
}

/// Helper constructing the atlas-segment-specific non-canonical reject with a stable
/// `what`.
fn non_canonical(what: &'static str) -> WriteError {
    WriteError::NonCanonicalAtlasSegment { what }
}

#[cfg(test)]
include!("atlas_segment_tests.rs");
