// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 atlas segment info OBU syntax model (AV2 v1.0.0 § 5.9).
//!
//! `atlas_segment_info_obu()` codes the `atlas_segment_id`, an
//! `ats_atlas_segment_mode_idc` selecting one of five layouts (enhanced, basic,
//! single, multistream, multistream-with-alpha), and a label-segment table mapping
//! atlas-segment indices to ids. This parser reads the full § 5.9 syntax; it never
//! skips payload bits. Range checks that prevent unsafe loops — the mode value and the
//! per-mode segment / region counts — are enforced as typed errors so `splot-validate`
//! reports them rather than risking an unbounded parse. Cross-OBU atlas availability
//! (AV2 § 7.3.8.4) is checked in `splot-validate`.

use crate::bitio::BitReader;
use crate::error::{AtlasSegmentErrorKind, Error, Result};

/// `MAX_NUM_ATLAS_SEGMENTS` (AV2 § 3): the conformance bound on atlas segment counts
/// (AV2 § 6.9.6) and the safety bound on the segment loops.
const MAX_NUM_ATLAS_SEGMENTS: u32 = 256;
/// `MAX_ATLAS_COLS` (AV2 § 3): the conformance bound on atlas region columns
/// (AV2 § 6.9.3.1).
const MAX_ATLAS_COLS: u32 = 64;
/// `MAX_ATLAS_ROWS` (AV2 § 3): the conformance bound on atlas region rows
/// (AV2 § 6.9.3.1).
const MAX_ATLAS_ROWS: u32 = 64;

/// `ats_atlas_segment_mode_idc` (AV2 v1.0.0 § 6.9, Table 6.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtlasSegmentMode {
    /// `ENHANCED_ATLAS` (0): region grid plus region-to-segment mapping.
    Enhanced,
    /// `BASIC_ATLAS` (1): explicit per-segment rectangles.
    Basic,
    /// `SINGLE_ATLAS` (2): one segment with nominal dimensions.
    Single,
    /// `MULTISTREAM_ATLAS` (3): per-segment input-stream composition.
    Multistream,
    /// `MULTISTREAM_ALPHA_ATLAS` (4): multistream plus per-segment alpha flags.
    MultistreamAlpha,
}

impl AtlasSegmentMode {
    /// Maps an `ats_atlas_segment_mode_idc` value to a mode, returning `None` for the
    /// out-of-range values (`> 4`) that have no defined syntax (AV2 § 6.9).
    #[must_use]
    pub const fn from_idc(idc: u32) -> Option<Self> {
        match idc {
            0 => Some(Self::Enhanced),
            1 => Some(Self::Basic),
            2 => Some(Self::Single),
            3 => Some(Self::Multistream),
            4 => Some(Self::MultistreamAlpha),
            _ => None,
        }
    }

    /// Returns the `ats_atlas_segment_mode_idc` value for this mode.
    #[must_use]
    pub const fn idc(self) -> u32 {
        match self {
            Self::Enhanced => 0,
            Self::Basic => 1,
            Self::Single => 2,
            Self::Multistream => 3,
            Self::MultistreamAlpha => 4,
        }
    }
}

/// Parsed `atlas_segment_info_obu()` syntax (AV2 v1.0.0 § 5.9).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasSegment {
    /// `atlas_segment_id[obu_xlayer_id]` (`f(3)`): the atlas id (= `xAId`).
    pub atlas_segment_id: u8,
    /// `ats_atlas_segment_mode_idc` mapped to its mode.
    pub mode: AtlasSegmentMode,
    /// `numSegments`: the number of atlas segments derived from the mode.
    pub num_segments: u32,
    /// The per-mode parsed information.
    pub mode_info: AtlasModeInfo,
    /// `ats_label_segment_info()` (AV2 § 5.9.1): the segment-id assignment.
    pub label: AtlasLabelSegmentInfo,
}

/// The per-mode body of an atlas segment info OBU (AV2 v1.0.0 § 5.9).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AtlasModeInfo {
    /// `ats_enhanced_atlas_info()` (AV2 § 5.9.2).
    Enhanced(AtlasEnhancedInfo),
    /// `ats_basic_info()` (AV2 § 5.9.5).
    Basic(AtlasBasicInfo),
    /// The `SINGLE_ATLAS` nominal dimensions (AV2 § 5.9).
    Single(AtlasSingleInfo),
    /// `ats_multistream_info()` (AV2 § 5.9.3).
    Multistream(AtlasMultistreamInfo),
    /// `ats_multistream_with_alpha_info()` (AV2 § 5.9.4).
    MultistreamAlpha(AtlasMultistreamInfo),
}

/// The `SINGLE_ATLAS` nominal dimensions (AV2 v1.0.0 § 5.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasSingleInfo {
    /// `ats_nominal_width_minus_1[xAId]` (`uvlc()`).
    pub nominal_width_minus_1: u32,
    /// `ats_nominal_height_minus_1[xAId]` (`uvlc()`).
    pub nominal_height_minus_1: u32,
}

/// `ats_enhanced_atlas_info(xAId)` (AV2 v1.0.0 § 5.9.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasEnhancedInfo {
    /// `ats_region_info(xAId)` (AV2 § 5.9.2.1).
    pub region: AtlasRegionInfo,
    /// `ats_region_to_segment_mapping(xAId)` (AV2 § 5.9.2.2).
    pub mapping: AtlasRegionToSegmentMapping,
}

/// `ats_region_info(xAId)` (AV2 v1.0.0 § 5.9.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasRegionInfo {
    /// `ats_num_region_columns_minus_1[xAId]` (`uvlc()`).
    pub num_region_columns_minus_1: u32,
    /// `ats_num_region_rows_minus_1[xAId]` (`uvlc()`).
    pub num_region_rows_minus_1: u32,
    /// `ats_uniform_spacing_flag[xAId]` (`f(1)`).
    pub uniform_spacing: bool,
    /// `ats_column_width_minus_1[xAId][]` (`uvlc()`), present when not uniform.
    pub column_widths_minus_1: Vec<u32>,
    /// `ats_row_height_minus_1[xAId][]` (`uvlc()`), present when not uniform.
    pub row_heights_minus_1: Vec<u32>,
    /// `ats_region_width_minus_1[xAId]` (`uvlc()`), present when uniform.
    pub region_width_minus_1: Option<u32>,
    /// `ats_region_height_minus_1[xAId]` (`uvlc()`), present when uniform.
    pub region_height_minus_1: Option<u32>,
    /// `NumRegionsInAtlas[xAId]` (derived).
    pub num_regions_in_atlas: u32,
}

/// `ats_region_to_segment_mapping(xAId)` (AV2 v1.0.0 § 5.9.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasRegionToSegmentMapping {
    /// `ats_single_region_per_atlas_segment_flag[xAId]` (`f(1)`).
    pub single_region_per_atlas_segment: bool,
    /// `ats_num_atlas_segments_minus_1[xAId]` (coded, or derived when single-region).
    pub num_atlas_segments_minus_1: u32,
    /// One entry per atlas segment, present when not single-region-per-segment.
    pub segments: Vec<AtlasSegmentRegion>,
}

/// One atlas segment's region rectangle in `ats_region_to_segment_mapping()`
/// (AV2 v1.0.0 § 5.9.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasSegmentRegion {
    /// `ats_top_left_region_column[xAId][i]` (`uvlc()`).
    pub top_left_region_column: u32,
    /// `ats_top_left_region_row[xAId][i]` (`uvlc()`).
    pub top_left_region_row: u32,
    /// `ats_bottom_right_region_column_off[xAId][i]` (`uvlc()`).
    pub bottom_right_region_column_off: u32,
    /// `ats_bottom_right_region_row_off[xAId][i]` (`uvlc()`).
    pub bottom_right_region_row_off: u32,
}

/// `ats_basic_info(xAId)` (AV2 v1.0.0 § 5.9.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasBasicInfo {
    /// `ats_stream_id_present[xAId]` (`f(1)`).
    pub stream_id_present: bool,
    /// `ats_width[xAId]` (`uvlc()`).
    pub width: u32,
    /// `ats_height[xAId]` (`uvlc()`).
    pub height: u32,
    /// `ats_num_atlas_segments_minus_1[xAId]` (`uvlc()`).
    pub num_atlas_segments_minus_1: u32,
    /// One entry per atlas segment.
    pub segments: Vec<AtlasBasicSegment>,
}

/// One segment of `ats_basic_info()` (AV2 v1.0.0 § 5.9.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasBasicSegment {
    /// `ats_input_stream_id[xAId][i]` (`f(5)`), present when `ats_stream_id_present`.
    pub input_stream_id: Option<u8>,
    /// `ats_segment_top_left_pos_x[xAId][i]` (`uvlc()`).
    pub top_left_pos_x: u32,
    /// `ats_segment_top_left_pos_y[xAId][i]` (`uvlc()`).
    pub top_left_pos_y: u32,
    /// `ats_segment_width[xAId][i]` (`uvlc()`).
    pub width: u32,
    /// `ats_segment_height[xAId][i]` (`uvlc()`).
    pub height: u32,
}

/// `ats_multistream_info()` / `ats_multistream_with_alpha_info()`
/// (AV2 v1.0.0 § 5.9.3 / § 5.9.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasMultistreamInfo {
    /// `ats_msi_width[xlayerId][xAId]` (`uvlc()`).
    pub width: u32,
    /// `ats_msi_height[xlayerId][xAId]` (`uvlc()`).
    pub height: u32,
    /// `ats_msi_num_atlas_segments_minus_1[xlayerId][xAId]` (`uvlc()`).
    pub num_atlas_segments_minus_1: u32,
    /// `ats_msi_alpha_segments_present_flag` (`f(1)`); `Some` only for the alpha
    /// variant (AV2 § 5.9.4).
    pub alpha_segments_present: Option<bool>,
    /// `(ats_msi_background_red_value, _green_value, _blue_value)` (`f(8)` each),
    /// present when `ats_msi_background_info_present_flag`.
    pub background: Option<(u8, u8, u8)>,
    /// One entry per atlas segment.
    pub segments: Vec<AtlasMultistreamSegment>,
}

/// One segment of `ats_multistream_info()` (AV2 v1.0.0 § 5.9.3 / § 5.9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasMultistreamSegment {
    /// `ats_msi_input_stream_id[xlayerId][xAId][i]` (`f(5)`).
    pub input_stream_id: u8,
    /// `ats_msi_segment_top_left_pos_x[xlayerId][xAId][i]` (`uvlc()`).
    pub top_left_pos_x: u32,
    /// `ats_msi_segment_top_left_pos_y[xlayerId][xAId][i]` (`uvlc()`).
    pub top_left_pos_y: u32,
    /// `ats_msi_segment_width[xlayerId][xAId][i]` (`uvlc()`).
    pub width: u32,
    /// `ats_msi_segment_height[xlayerId][xAId][i]` (`uvlc()`).
    pub height: u32,
    /// `ats_msi_alpha_segment_flag[xlayerId][xAId][i]` (`f(1)`); inferred `false` when
    /// not present (AV2 § 6.9.5).
    pub alpha_segment_flag: bool,
}

/// `ats_label_segment_info(xlayerId, xAId, numSegments)` (AV2 v1.0.0 § 5.9.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasLabelSegmentInfo {
    /// `ats_signaled_atlas_segment_ids_flag[xlayerId][xAId]` (`f(1)`).
    pub signaled_atlas_segment_ids: bool,
    /// `AtlasSegmentIndexToID[xlayerId][xAId][i]`: the explicit `ats_atlas_segment_id`
    /// values when signaled, otherwise the segment indices.
    pub segment_ids: Vec<u8>,
}

/// Parses `atlas_segment_info_obu()` (AV2 v1.0.0 § 5.9).
///
/// The full § 5.9 syntax is read; the parser never skips payload bits. The mode value
/// and the per-mode segment / region counts are range-checked before any loop, so a
/// malformed count cannot drive an unbounded parse.
///
/// # Errors
/// Returns descriptor errors (`uvlc`), [`Error::InvalidAtlasSegment`] for an
/// out-of-range mode or segment / region count, or [`Error::UnexpectedEof`] if the
/// payload ends mid-field.
pub fn parse_atlas_segment(reader: &mut BitReader<'_>) -> Result<AtlasSegment> {
    let atlas_segment_id = reader.read_bits_u8(3)?;

    let mode_offset = reader.byte_offset();
    let mode_bit_offset = reader.bit_offset();
    let mode_idc = reader.read_uvlc()?;
    let Some(mode) = AtlasSegmentMode::from_idc(mode_idc) else {
        return Err(Error::InvalidAtlasSegment {
            offset: mode_offset,
            bit_offset: mode_bit_offset,
            kind: AtlasSegmentErrorKind::ModeOutOfRange,
        });
    };

    let (mode_info, num_segments) = match mode {
        AtlasSegmentMode::Enhanced => {
            let (info, num) = parse_ats_enhanced_atlas_info(reader)?;
            (AtlasModeInfo::Enhanced(info), num)
        }
        AtlasSegmentMode::Basic => {
            let (info, num) = parse_ats_basic_info(reader)?;
            (AtlasModeInfo::Basic(info), num)
        }
        AtlasSegmentMode::Single => {
            let info = AtlasSingleInfo {
                nominal_width_minus_1: reader.read_uvlc()?,
                nominal_height_minus_1: reader.read_uvlc()?,
            };
            (AtlasModeInfo::Single(info), 1)
        }
        AtlasSegmentMode::Multistream => {
            let (info, num) = parse_ats_multistream_info(reader, false)?;
            (AtlasModeInfo::Multistream(info), num)
        }
        AtlasSegmentMode::MultistreamAlpha => {
            let (info, num) = parse_ats_multistream_info(reader, true)?;
            (AtlasModeInfo::MultistreamAlpha(info), num)
        }
    };

    // numSegments drives the label loop. This is unreachable via the per-mode parsers
    // (each caps its count: the single-region path checks num_regions_in_atlas, the
    // others check minus_1 >= MAX_NUM_ATLAS_SEGMENTS); kept as a safety net in case a
    // future mode derives numSegments differently.
    if num_segments > MAX_NUM_ATLAS_SEGMENTS {
        return Err(Error::InvalidAtlasSegment {
            offset: reader.byte_offset(),
            bit_offset: reader.bit_offset(),
            kind: AtlasSegmentErrorKind::SegmentCountOutOfRange,
        });
    }

    let label = parse_ats_label_segment_info(reader, num_segments)?;

    Ok(AtlasSegment {
        atlas_segment_id,
        mode,
        num_segments,
        mode_info,
        label,
    })
}

/// Parses `ats_label_segment_info(xlayerId, xAId, numSegments)` (AV2 v1.0.0 § 5.9.1).
fn parse_ats_label_segment_info(
    reader: &mut BitReader<'_>,
    num_segments: u32,
) -> Result<AtlasLabelSegmentInfo> {
    let signaled_atlas_segment_ids = reader.read_bit()? != 0;
    let mut segment_ids = Vec::new();
    if signaled_atlas_segment_ids {
        for _ in 0..num_segments {
            segment_ids.push(reader.read_bits_u8(8)?);
        }
    } else {
        // AV2 § 5.9.1: AtlasSegmentIndexToID[i] = i. numSegments is bounded by
        // MAX_NUM_ATLAS_SEGMENTS, so each index fits in u8.
        for i in 0..num_segments {
            segment_ids.push(u8::try_from(i).unwrap_or(u8::MAX));
        }
    }
    Ok(AtlasLabelSegmentInfo {
        signaled_atlas_segment_ids,
        segment_ids,
    })
}

/// Parses `ats_enhanced_atlas_info(xAId)` (AV2 v1.0.0 § 5.9.2), returning the info and
/// `numSegments`.
fn parse_ats_enhanced_atlas_info(reader: &mut BitReader<'_>) -> Result<(AtlasEnhancedInfo, u32)> {
    let region = parse_ats_region_info(reader)?;
    let mapping = parse_ats_region_to_segment_mapping(reader, region.num_regions_in_atlas)?;
    let num_segments = mapping.num_atlas_segments_minus_1.saturating_add(1);
    Ok((AtlasEnhancedInfo { region, mapping }, num_segments))
}

/// Parses `ats_region_info(xAId)` (AV2 v1.0.0 § 5.9.2.1).
fn parse_ats_region_info(reader: &mut BitReader<'_>) -> Result<AtlasRegionInfo> {
    let dim_offset = reader.byte_offset();
    let dim_bit_offset = reader.bit_offset();
    let num_region_columns_minus_1 = reader.read_uvlc()?;
    let num_region_rows_minus_1 = reader.read_uvlc()?;
    if num_region_columns_minus_1 >= MAX_ATLAS_COLS || num_region_rows_minus_1 >= MAX_ATLAS_ROWS {
        return Err(Error::InvalidAtlasSegment {
            offset: dim_offset,
            bit_offset: dim_bit_offset,
            kind: AtlasSegmentErrorKind::RegionDimensionOutOfRange,
        });
    }

    let uniform_spacing = reader.read_bit()? != 0;
    let mut column_widths_minus_1 = Vec::new();
    let mut row_heights_minus_1 = Vec::new();
    let mut region_width_minus_1 = None;
    let mut region_height_minus_1 = None;
    if uniform_spacing {
        region_width_minus_1 = Some(reader.read_uvlc()?);
        region_height_minus_1 = Some(reader.read_uvlc()?);
    } else {
        for _ in 0..=num_region_columns_minus_1 {
            column_widths_minus_1.push(reader.read_uvlc()?);
        }
        for _ in 0..=num_region_rows_minus_1 {
            row_heights_minus_1.push(reader.read_uvlc()?);
        }
    }

    // Bounded by MAX_ATLAS_COLS * MAX_ATLAS_ROWS, so the product cannot overflow u32.
    let num_regions_in_atlas =
        (num_region_columns_minus_1 + 1).saturating_mul(num_region_rows_minus_1 + 1);

    Ok(AtlasRegionInfo {
        num_region_columns_minus_1,
        num_region_rows_minus_1,
        uniform_spacing,
        column_widths_minus_1,
        row_heights_minus_1,
        region_width_minus_1,
        region_height_minus_1,
        num_regions_in_atlas,
    })
}

/// Parses `ats_region_to_segment_mapping(xAId)` (AV2 v1.0.0 § 5.9.2.2).
fn parse_ats_region_to_segment_mapping(
    reader: &mut BitReader<'_>,
    num_regions_in_atlas: u32,
) -> Result<AtlasRegionToSegmentMapping> {
    let single_region_per_atlas_segment = reader.read_bit()? != 0;
    let mut segments = Vec::new();
    let num_atlas_segments_minus_1 = if single_region_per_atlas_segment {
        if num_regions_in_atlas > MAX_NUM_ATLAS_SEGMENTS {
            return Err(Error::InvalidAtlasSegment {
                offset: reader.byte_offset(),
                bit_offset: reader.bit_offset(),
                kind: AtlasSegmentErrorKind::SegmentCountOutOfRange,
            });
        }
        num_regions_in_atlas.saturating_sub(1)
    } else {
        let minus_1 = reader.read_uvlc()?;
        if minus_1 >= MAX_NUM_ATLAS_SEGMENTS {
            return Err(Error::InvalidAtlasSegment {
                offset: reader.byte_offset(),
                bit_offset: reader.bit_offset(),
                kind: AtlasSegmentErrorKind::SegmentCountOutOfRange,
            });
        }
        for _ in 0..=minus_1 {
            segments.push(AtlasSegmentRegion {
                top_left_region_column: reader.read_uvlc()?,
                top_left_region_row: reader.read_uvlc()?,
                bottom_right_region_column_off: reader.read_uvlc()?,
                bottom_right_region_row_off: reader.read_uvlc()?,
            });
        }
        minus_1
    };

    Ok(AtlasRegionToSegmentMapping {
        single_region_per_atlas_segment,
        num_atlas_segments_minus_1,
        segments,
    })
}

/// Parses `ats_basic_info(xAId)` (AV2 v1.0.0 § 5.9.5), returning the info and
/// `numSegments`.
fn parse_ats_basic_info(reader: &mut BitReader<'_>) -> Result<(AtlasBasicInfo, u32)> {
    let stream_id_present = reader.read_bit()? != 0;
    let width = reader.read_uvlc()?;
    let height = reader.read_uvlc()?;
    let count_offset = reader.byte_offset();
    let count_bit_offset = reader.bit_offset();
    let num_atlas_segments_minus_1 = reader.read_uvlc()?;
    if num_atlas_segments_minus_1 >= MAX_NUM_ATLAS_SEGMENTS {
        return Err(Error::InvalidAtlasSegment {
            offset: count_offset,
            bit_offset: count_bit_offset,
            kind: AtlasSegmentErrorKind::SegmentCountOutOfRange,
        });
    }

    let mut segments = Vec::new();
    for _ in 0..=num_atlas_segments_minus_1 {
        let input_stream_id = if stream_id_present {
            Some(reader.read_bits_u8(5)?)
        } else {
            None
        };
        segments.push(AtlasBasicSegment {
            input_stream_id,
            top_left_pos_x: reader.read_uvlc()?,
            top_left_pos_y: reader.read_uvlc()?,
            width: reader.read_uvlc()?,
            height: reader.read_uvlc()?,
        });
    }

    let num_segments = num_atlas_segments_minus_1 + 1;
    Ok((
        AtlasBasicInfo {
            stream_id_present,
            width,
            height,
            num_atlas_segments_minus_1,
            segments,
        },
        num_segments,
    ))
}

/// Parses `ats_multistream_info()` / `ats_multistream_with_alpha_info()`
/// (AV2 v1.0.0 § 5.9.3 / § 5.9.4), returning the info and `numSegments`.
fn parse_ats_multistream_info(
    reader: &mut BitReader<'_>,
    with_alpha: bool,
) -> Result<(AtlasMultistreamInfo, u32)> {
    let width = reader.read_uvlc()?;
    let height = reader.read_uvlc()?;
    let count_offset = reader.byte_offset();
    let count_bit_offset = reader.bit_offset();
    let num_atlas_segments_minus_1 = reader.read_uvlc()?;
    if num_atlas_segments_minus_1 >= MAX_NUM_ATLAS_SEGMENTS {
        return Err(Error::InvalidAtlasSegment {
            offset: count_offset,
            bit_offset: count_bit_offset,
            kind: AtlasSegmentErrorKind::SegmentCountOutOfRange,
        });
    }

    let alpha_segments_present = if with_alpha {
        Some(reader.read_bit()? != 0)
    } else {
        None
    };
    let background_info_present = reader.read_bit()? != 0;
    let background = if background_info_present {
        Some((
            reader.read_bits_u8(8)?,
            reader.read_bits_u8(8)?,
            reader.read_bits_u8(8)?,
        ))
    } else {
        None
    };

    let mut segments = Vec::new();
    for i in 0..=num_atlas_segments_minus_1 {
        let input_stream_id = reader.read_bits_u8(5)?;
        let top_left_pos_x = reader.read_uvlc()?;
        let top_left_pos_y = reader.read_uvlc()?;
        let seg_width = reader.read_uvlc()?;
        let seg_height = reader.read_uvlc()?;
        // AV2 § 5.9.4: the per-segment alpha flag is coded only for the alpha variant
        // and not for the last segment, which is inferred 0 (AV2 § 6.9.5).
        let alpha_segment_flag =
            if alpha_segments_present == Some(true) && i != num_atlas_segments_minus_1 {
                reader.read_bit()? != 0
            } else {
                false
            };
        segments.push(AtlasMultistreamSegment {
            input_stream_id,
            top_left_pos_x,
            top_left_pos_y,
            width: seg_width,
            height: seg_height,
            alpha_segment_flag,
        });
    }

    let num_segments = num_atlas_segments_minus_1 + 1;
    Ok((
        AtlasMultistreamInfo {
            width,
            height,
            num_atlas_segments_minus_1,
            alpha_segments_present,
            background,
            segments,
        },
        num_segments,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    /// MSB-first bit writer for building atlas payloads in tests.
    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    fn parse(bytes: &[u8]) -> Result<AtlasSegment> {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_atlas_segment(&mut reader)
    }

    #[test]
    fn parses_single_atlas() {
        let mut bits = Bits::default();
        bits.f(3, 3); // atlas_segment_id = 3
        bits.uvlc(2); // mode_idc = SINGLE_ATLAS
        bits.uvlc(1919); // ats_nominal_width_minus_1
        bits.uvlc(1079); // ats_nominal_height_minus_1
        bits.bit(0); // ats_signaled_atlas_segment_ids_flag = 0
        let record = parse(&bits.into_bytes()).unwrap();
        assert_eq!(record.atlas_segment_id, 3);
        assert_eq!(record.mode, AtlasSegmentMode::Single);
        assert_eq!(record.num_segments, 1);
        let AtlasModeInfo::Single(single) = record.mode_info else {
            panic!("expected single mode info");
        };
        assert_eq!(single.nominal_width_minus_1, 1919);
        assert_eq!(single.nominal_height_minus_1, 1079);
        assert!(!record.label.signaled_atlas_segment_ids);
        assert_eq!(record.label.segment_ids, vec![0]);
    }

    #[test]
    fn parses_basic_atlas_with_signaled_ids() {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(1); // mode_idc = BASIC_ATLAS
        bits.bit(1); // ats_stream_id_present
        bits.uvlc(640); // ats_width
        bits.uvlc(480); // ats_height
        bits.uvlc(1); // ats_num_atlas_segments_minus_1 = 1 -> 2 segments
        for _ in 0..2 {
            bits.f(5, 5); // ats_input_stream_id
            bits.uvlc(0); // top_left_pos_x
            bits.uvlc(0); // top_left_pos_y
            bits.uvlc(100); // width
            bits.uvlc(100); // height
        }
        bits.bit(1); // ats_signaled_atlas_segment_ids_flag = 1
        bits.f(10, 8); // ats_atlas_segment_id[0]
        bits.f(20, 8); // ats_atlas_segment_id[1]
        let record = parse(&bits.into_bytes()).unwrap();
        assert_eq!(record.mode, AtlasSegmentMode::Basic);
        assert_eq!(record.num_segments, 2);
        let AtlasModeInfo::Basic(basic) = &record.mode_info else {
            panic!("expected basic mode info");
        };
        assert!(basic.stream_id_present);
        assert_eq!(basic.segments.len(), 2);
        assert_eq!(basic.segments[0].input_stream_id, Some(5));
        assert!(record.label.signaled_atlas_segment_ids);
        assert_eq!(record.label.segment_ids, vec![10, 20]);
    }

    #[test]
    fn parses_enhanced_atlas_uniform_single_region() {
        let mut bits = Bits::default();
        bits.f(1, 3); // atlas_segment_id
        bits.uvlc(0); // mode_idc = ENHANCED_ATLAS
        // ats_region_info:
        bits.uvlc(0); // num_region_columns_minus_1 = 0
        bits.uvlc(0); // num_region_rows_minus_1 = 0 -> NumRegionsInAtlas = 1
        bits.bit(1); // ats_uniform_spacing_flag
        bits.uvlc(63); // ats_region_width_minus_1
        bits.uvlc(63); // ats_region_height_minus_1
        // ats_region_to_segment_mapping:
        bits.bit(1); // ats_single_region_per_atlas_segment_flag -> numSegments = 1
        bits.bit(0); // ats_signaled_atlas_segment_ids_flag
        let record = parse(&bits.into_bytes()).unwrap();
        assert_eq!(record.mode, AtlasSegmentMode::Enhanced);
        assert_eq!(record.num_segments, 1);
        let AtlasModeInfo::Enhanced(enhanced) = &record.mode_info else {
            panic!("expected enhanced mode info");
        };
        assert_eq!(enhanced.region.num_regions_in_atlas, 1);
        assert!(enhanced.region.uniform_spacing);
        assert!(enhanced.mapping.single_region_per_atlas_segment);
        assert_eq!(enhanced.mapping.num_atlas_segments_minus_1, 0);
    }

    #[test]
    fn parses_multistream_atlas() {
        let mut bits = Bits::default();
        bits.f(2, 3); // atlas_segment_id
        bits.uvlc(3); // mode_idc = MULTISTREAM_ATLAS
        bits.uvlc(3840); // ats_msi_width
        bits.uvlc(2160); // ats_msi_height
        bits.uvlc(0); // ats_msi_num_atlas_segments_minus_1 = 0 -> 1 segment
        bits.bit(0); // ats_msi_background_info_present_flag
        // segment 0:
        bits.f(1, 5); // ats_msi_input_stream_id
        bits.uvlc(0); // pos_x
        bits.uvlc(0); // pos_y
        bits.uvlc(1920); // width
        bits.uvlc(1080); // height
        bits.bit(0); // ats_signaled_atlas_segment_ids_flag
        let record = parse(&bits.into_bytes()).unwrap();
        assert_eq!(record.mode, AtlasSegmentMode::Multistream);
        assert_eq!(record.num_segments, 1);
        let AtlasModeInfo::Multistream(msi) = &record.mode_info else {
            panic!("expected multistream mode info");
        };
        assert_eq!(msi.width, 3840);
        assert_eq!(msi.alpha_segments_present, None);
        assert_eq!(msi.segments.len(), 1);
        assert_eq!(msi.segments[0].input_stream_id, 1);
        assert!(!msi.segments[0].alpha_segment_flag);
    }

    #[test]
    fn parses_multistream_alpha_atlas() {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(4); // mode_idc = MULTISTREAM_ALPHA_ATLAS
        bits.uvlc(100); // ats_msi_width
        bits.uvlc(100); // ats_msi_height
        bits.uvlc(1); // ats_msi_num_atlas_segments_minus_1 = 1 -> 2 segments
        bits.bit(1); // ats_msi_alpha_segments_present_flag
        bits.bit(1); // ats_msi_background_info_present_flag
        bits.f(255, 8); // red
        bits.f(0, 8); // green
        bits.f(0, 8); // blue
        // segment 0 (not last -> alpha flag coded):
        bits.f(0, 5); // input_stream_id
        bits.uvlc(0);
        bits.uvlc(0);
        bits.uvlc(50);
        bits.uvlc(50);
        bits.bit(1); // ats_msi_alpha_segment_flag[0]
        // segment 1 (last -> alpha flag inferred 0):
        bits.f(1, 5);
        bits.uvlc(50);
        bits.uvlc(0);
        bits.uvlc(50);
        bits.uvlc(50);
        bits.bit(0); // ats_signaled_atlas_segment_ids_flag
        let record = parse(&bits.into_bytes()).unwrap();
        assert_eq!(record.mode, AtlasSegmentMode::MultistreamAlpha);
        assert_eq!(record.num_segments, 2);
        let AtlasModeInfo::MultistreamAlpha(msi) = &record.mode_info else {
            panic!("expected multistream-alpha mode info");
        };
        assert_eq!(msi.alpha_segments_present, Some(true));
        assert_eq!(msi.background, Some((255, 0, 0)));
        assert!(msi.segments[0].alpha_segment_flag);
        assert!(!msi.segments[1].alpha_segment_flag);
        assert_eq!(record.label.segment_ids, vec![0, 1]);
    }

    #[test]
    fn rejects_mode_out_of_range() {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(5); // mode_idc = 5 -> out of range
        assert!(matches!(
            parse(&bits.into_bytes()),
            Err(Error::InvalidAtlasSegment {
                kind: AtlasSegmentErrorKind::ModeOutOfRange,
                ..
            })
        ));
    }

    #[test]
    fn rejects_segment_count_out_of_range() {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(1); // mode_idc = BASIC_ATLAS
        bits.bit(0); // stream_id_present
        bits.uvlc(10); // width
        bits.uvlc(10); // height
        bits.uvlc(MAX_NUM_ATLAS_SEGMENTS); // num_atlas_segments_minus_1 == 256 -> out of range
        assert!(matches!(
            parse(&bits.into_bytes()),
            Err(Error::InvalidAtlasSegment {
                kind: AtlasSegmentErrorKind::SegmentCountOutOfRange,
                ..
            })
        ));
    }

    #[test]
    fn rejects_region_dimension_out_of_range() {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(0); // mode_idc = ENHANCED_ATLAS
        bits.uvlc(MAX_ATLAS_COLS); // num_region_columns_minus_1 == 64 -> out of range
        bits.uvlc(0);
        assert!(matches!(
            parse(&bits.into_bytes()),
            Err(Error::InvalidAtlasSegment {
                kind: AtlasSegmentErrorKind::RegionDimensionOutOfRange,
                ..
            })
        ));
    }

    #[test]
    fn reports_eof_in_mode_body() {
        // Mode SINGLE but no nominal dimensions follow.
        let mut bits = Bits::default();
        bits.f(0, 3);
        bits.uvlc(2);
        assert!(matches!(
            parse(&bits.into_bytes()),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn reports_eof_in_header() {
        assert!(matches!(parse(&[]), Err(Error::UnexpectedEof { .. })));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// The atlas-segment parser must never panic on arbitrary input.
        #[test]
        fn atlas_segment_parser_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_atlas_segment(&mut reader);
        }
    }
}
