// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 sequence-header syntax model (AV2 v1.0.0 § 5.4).

use crate::bitio::BitReader;
use crate::error::{Error, Result, SequenceHeaderErrorKind};
use crate::span::{BitOffset, ByteOffset};
use crate::types::{EmbeddedLayerId, TemporalLayerId};

/// `MAX_SEQ_NUM` for `seq_header_id` validation (AV2 v1.0.0 § 6.4.1).
pub const MAX_SEQ_NUM: u32 = 16;
/// `MAX_NUM_TLAYERS` used by sequence-header dependency maps (AV2 § 5.4.1).
pub const MAX_NUM_TLAYERS: usize = 4;
/// `MAX_NUM_MLAYERS` used by sequence-header dependency maps (AV2 § 5.4.1).
pub const MAX_NUM_MLAYERS: usize = 8;

/// `seq_header_id` (AV2 v1.0.0 § 5.4.1 / § 6.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SequenceHeaderId(u8);

impl SequenceHeaderId {
    /// Creates a sequence-header id if `value < MAX_SEQ_NUM`.
    #[must_use]
    pub const fn try_new(value: u32) -> Option<Self> {
        if value < MAX_SEQ_NUM {
            Some(Self(value as u8))
        } else {
            None
        }
    }

    /// Returns the raw `seq_header_id` value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// `seq_profile_idc` (AV2 v1.0.0 § 5.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileIdc(u8);

impl ProfileIdc {
    /// Creates a profile id from the 5-bit `seq_profile_idc` field.
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw `seq_profile_idc` value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// `seq_level_idx` (AV2 v1.0.0 § 5.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LevelIdx(u8);

impl LevelIdx {
    /// Creates a level index from the 5-bit `seq_level_idx` field.
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw `seq_level_idx` value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// `seq_tier` (AV2 v1.0.0 § 5.4.1 / § 6.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Main tier (`seq_tier == 0`).
    Main,
    /// High tier (`seq_tier == 1`).
    High,
}

impl Tier {
    /// Creates a tier from the 1-bit `seq_tier` field.
    #[must_use]
    pub const fn from_bit(bit: u8) -> Self {
        if bit == 0 { Self::Main } else { Self::High }
    }
}

/// `chroma_format_idc` values from AV2 Table 6.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChromaFormatIdc {
    /// `CHROMA_FORMAT_420`.
    Yuv420,
    /// `CHROMA_FORMAT_400`.
    Monochrome,
    /// `CHROMA_FORMAT_444`.
    Yuv444,
    /// `CHROMA_FORMAT_422`.
    Yuv422,
}

impl ChromaFormatIdc {
    /// Creates a chroma format from `chroma_format_idc`.
    #[must_use]
    pub const fn try_new(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Yuv420),
            1 => Some(Self::Monochrome),
            2 => Some(Self::Yuv444),
            3 => Some(Self::Yuv422),
            _ => None,
        }
    }

    /// Returns the raw `chroma_format_idc` value.
    #[must_use]
    pub const fn get(self) -> u8 {
        match self {
            Self::Yuv420 => 0,
            Self::Monochrome => 1,
            Self::Yuv444 => 2,
            Self::Yuv422 => 3,
        }
    }

    /// Returns `true` for `CHROMA_FORMAT_400`.
    #[must_use]
    pub const fn is_monochrome(self) -> bool {
        matches!(self, Self::Monochrome)
    }
}

/// `bit_depth_idc` values from AV2 Table 6.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BitDepthIdc {
    /// 10-bit samples (`bit_depth_idc == 0`).
    Ten,
    /// 8-bit samples (`bit_depth_idc == 1`).
    Eight,
}

impl BitDepthIdc {
    /// Creates a bit-depth id from `bit_depth_idc`.
    #[must_use]
    pub const fn try_new(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Ten),
            1 => Some(Self::Eight),
            _ => None,
        }
    }

    /// Returns the raw `bit_depth_idc` value.
    #[must_use]
    pub const fn get(self) -> u8 {
        match self {
            Self::Ten => 0,
            Self::Eight => 1,
        }
    }

    /// Returns the sample bit depth derived by AV2 Table 6.3.
    #[must_use]
    pub const fn bit_depth(self) -> u8 {
        match self {
            Self::Ten => 10,
            Self::Eight => 8,
        }
    }
}

/// `seq_lcr_id` (AV2 v1.0.0 § 5.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LcrId(u8);

impl LcrId {
    /// Creates an LCR id from the 3-bit `seq_lcr_id` field.
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw `seq_lcr_id` value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Number of bits used for a frame dimension syntax element (AV2 § 5.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameDimensionBits(u8);

impl FrameDimensionBits {
    /// Creates the bit count from `frame_*_bits_minus_1`.
    #[must_use]
    pub const fn from_minus_1(value: u8) -> Self {
        Self(value + 1)
    }

    /// Returns the number of bits.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Maximum frame width derived from `max_frame_width_minus_1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameWidth(u32);

impl FrameWidth {
    /// Creates a frame width from `max_frame_width_minus_1`.
    #[must_use]
    pub const fn from_minus_1(value: u32) -> Self {
        Self(value + 1)
    }

    /// Returns the frame width in pixels.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Returns `max_frame_width_minus_1`.
    #[must_use]
    pub const fn minus_1(self) -> u32 {
        self.0 - 1
    }
}

/// Maximum frame height derived from `max_frame_height_minus_1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameHeight(u32);

impl FrameHeight {
    /// Creates a frame height from `max_frame_height_minus_1`.
    #[must_use]
    pub const fn from_minus_1(value: u32) -> Self {
        Self(value + 1)
    }

    /// Returns the frame height in pixels.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Returns `max_frame_height_minus_1`.
    #[must_use]
    pub const fn minus_1(self) -> u32 {
        self.0 - 1
    }
}

/// Sequence cropping window offsets (AV2 v1.0.0 § 5.4.1 / § 6.4.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CroppingWindow {
    /// `seq_cropping_win_left_offset`.
    pub left: u32,
    /// `seq_cropping_win_right_offset`.
    pub right: u32,
    /// `seq_cropping_win_top_offset`.
    pub top: u32,
    /// `seq_cropping_win_bottom_offset`.
    pub bottom: u32,
}

/// `SeqMaxMlayerCnt`, derived from `seq_max_mlayer_cnt_minus_1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EmbeddedLayerCount(u8);

impl EmbeddedLayerCount {
    /// Creates a count if `seq_max_mlayer_cnt_minus_1 <= max_mlayer_id`.
    #[must_use]
    pub const fn try_from_minus_1(minus_1: u32, max_mlayer_id: EmbeddedLayerId) -> Option<Self> {
        if minus_1 <= max_mlayer_id.get() as u32 {
            Some(Self((minus_1 + 1) as u8))
        } else {
            None
        }
    }

    /// Creates a count for the inferred single-layer case.
    #[must_use]
    pub const fn one() -> Self {
        Self(1)
    }

    /// Returns the embedded-layer count.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Parsed general sequence-header fields through AV2 § 5.4.1 dependency maps.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceHeader {
    /// `seq_header_id`.
    pub seq_header_id: SequenceHeaderId,
    /// `seq_profile_idc`.
    pub seq_profile_idc: ProfileIdc,
    /// `single_picture_header_flag`.
    pub single_picture_header_flag: bool,
    /// `seq_level_idx`.
    pub seq_level_idx: LevelIdx,
    /// `seq_tier`.
    pub seq_tier: Tier,
    /// `chroma_format_idc`.
    pub chroma_format_idc: ChromaFormatIdc,
    /// `bit_depth_idc`.
    pub bit_depth_idc: BitDepthIdc,
    /// `seq_lcr_id`.
    pub seq_lcr_id: LcrId,
    /// `still_picture`.
    pub still_picture: bool,
    /// `max_tlayer_id`.
    pub max_tlayer_id: TemporalLayerId,
    /// `max_mlayer_id`.
    pub max_mlayer_id: EmbeddedLayerId,
    /// `SeqMaxMlayerCnt`.
    pub seq_max_mlayer_count: EmbeddedLayerCount,
    /// `monotonic_output_order_flag`.
    pub monotonic_output_order_flag: bool,
    /// `frame_width_bits_minus_1 + 1`.
    pub frame_width_bits: FrameDimensionBits,
    /// `frame_height_bits_minus_1 + 1`.
    pub frame_height_bits: FrameDimensionBits,
    /// `max_frame_width_minus_1 + 1`.
    pub max_frame_width: FrameWidth,
    /// `max_frame_height_minus_1 + 1`.
    pub max_frame_height: FrameHeight,
    /// Cropping offsets inferred or parsed from the sequence header.
    pub cropping_window: CroppingWindow,
    /// `seq_initial_display_delay_minus_1`, if present.
    pub seq_initial_display_delay_minus_1: Option<u8>,
    /// `decoder_model_info_present_flag`.
    pub decoder_model_info_present_flag: bool,
    /// `num_units_in_decoding_tick`, if decoder model information is present.
    pub num_units_in_decoding_tick: Option<u32>,
    /// `seq_decoder_model_info_present_flag`.
    pub seq_decoder_model_info_present_flag: bool,
}

/// Parses the general sequence-header syntax through dependency maps (AV2 § 5.4.1).
///
/// This parser intentionally stops immediately before unimplemented child syntax.
/// Child syntax structures are represented by explicit stubs until their matrix
/// rows are implemented. When `seq_decoder_model_info()` is present, the
/// remaining sequence-header tail stays opaque rather than being treated as a
/// conformance error.
///
/// # Errors
/// Returns typed [`Error`] values for EOF, malformed descriptors, or local § 6.4.1
/// conformance violations.
pub fn parse_sequence_header_general(reader: &mut BitReader<'_>) -> Result<SequenceHeader> {
    let seq_header_id_offset = reader.byte_offset();
    let seq_header_id_bit_offset = reader.bit_offset();
    let seq_header_id_raw = reader.read_uvlc()?;
    let seq_header_id = SequenceHeaderId::try_new(seq_header_id_raw).ok_or_else(|| {
        invalid_sequence_header(
            seq_header_id_offset,
            seq_header_id_bit_offset,
            SequenceHeaderErrorKind::SeqHeaderIdOutOfRange,
        )
    })?;

    let seq_profile_idc = ProfileIdc::from_bits(reader.read_bits_u8(5)?);
    let single_picture_header_flag = reader.read_bit()? != 0;
    let seq_level_idx = LevelIdx::from_bits(reader.read_bits_u8(5)?);
    let seq_tier = if seq_level_idx.get() > 3 && !single_picture_header_flag {
        Tier::from_bit(reader.read_bit()?)
    } else {
        Tier::Main
    };

    let chroma_offset = reader.byte_offset();
    let chroma_bit_offset = reader.bit_offset();
    let chroma_raw = reader.read_uvlc()?;
    let chroma_format_idc = ChromaFormatIdc::try_new(chroma_raw).ok_or_else(|| {
        invalid_sequence_header(
            chroma_offset,
            chroma_bit_offset,
            SequenceHeaderErrorKind::ChromaFormatOutOfRange,
        )
    })?;

    let bit_depth_offset = reader.byte_offset();
    let bit_depth_bit_offset = reader.bit_offset();
    let bit_depth_raw = reader.read_uvlc()?;
    let bit_depth_idc = BitDepthIdc::try_new(bit_depth_raw).ok_or_else(|| {
        invalid_sequence_header(
            bit_depth_offset,
            bit_depth_bit_offset,
            SequenceHeaderErrorKind::BitDepthOutOfRange,
        )
    })?;

    let (
        seq_lcr_id,
        still_picture,
        max_tlayer_id,
        max_mlayer_id,
        seq_max_mlayer_count,
        monotonic_output_order_flag,
    ) = if single_picture_header_flag {
        (
            LcrId::from_bits(0),
            true,
            TemporalLayerId::from_bits(0),
            EmbeddedLayerId::from_bits(0),
            EmbeddedLayerCount::one(),
            true,
        )
    } else {
        let seq_lcr_id = LcrId::from_bits(reader.read_bits_u8(3)?);
        let still_picture = reader.read_bit()? != 0;
        let max_tlayer_id = TemporalLayerId::from_bits(reader.read_bits_u8(2)?);
        let max_mlayer_id = EmbeddedLayerId::from_bits(reader.read_bits_u8(3)?);
        let seq_max_mlayer_count = if max_mlayer_id.get() > 0 {
            let n = ceil_log2_u32(u32::from(max_mlayer_id.get()) + 1);
            let count_offset = reader.byte_offset();
            let count_bit_offset = reader.bit_offset();
            let seq_max_mlayer_cnt_minus_1 = reader.read_bits(n)?;
            EmbeddedLayerCount::try_from_minus_1(seq_max_mlayer_cnt_minus_1, max_mlayer_id)
                .ok_or_else(|| {
                    invalid_sequence_header(
                        count_offset,
                        count_bit_offset,
                        SequenceHeaderErrorKind::SeqMaxMlayerCountOutOfRange,
                    )
                })?
        } else {
            EmbeddedLayerCount::one()
        };
        let monotonic_output_order_flag = reader.read_bit()? != 0;
        (
            seq_lcr_id,
            still_picture,
            max_tlayer_id,
            max_mlayer_id,
            seq_max_mlayer_count,
            monotonic_output_order_flag,
        )
    };

    let frame_width_bits = FrameDimensionBits::from_minus_1(reader.read_bits_u8(4)?);
    let frame_height_bits = FrameDimensionBits::from_minus_1(reader.read_bits_u8(4)?);
    let max_frame_width_minus_1 = reader.read_bits(u32::from(frame_width_bits.get()))?;
    let max_frame_height_minus_1 = reader.read_bits(u32::from(frame_height_bits.get()))?;
    let max_frame_width = FrameWidth::from_minus_1(max_frame_width_minus_1);
    let max_frame_height = FrameHeight::from_minus_1(max_frame_height_minus_1);

    let cropping_window = parse_cropping_window(reader, max_frame_width, max_frame_height)?;

    let (
        seq_initial_display_delay_minus_1,
        decoder_model_info_present_flag,
        num_units_in_decoding_tick,
        seq_decoder_model_info_present_flag,
    ) = if single_picture_header_flag {
        (None, false, None, false)
    } else {
        let seq_initial_display_delay_present_flag = reader.read_bit()? != 0;
        let seq_initial_display_delay_minus_1 = if seq_initial_display_delay_present_flag {
            Some(reader.read_bits_u8(4)?)
        } else {
            None
        };
        let decoder_model_info_present_flag = reader.read_bit()? != 0;
        if decoder_model_info_present_flag {
            let num_units_offset = reader.byte_offset();
            let num_units_bit_offset = reader.bit_offset();
            let num_units_in_decoding_tick = reader.read_bits(32)?;
            if num_units_in_decoding_tick == 0 {
                return Err(invalid_sequence_header(
                    num_units_offset,
                    num_units_bit_offset,
                    SequenceHeaderErrorKind::TimingNumUnitsZero,
                ));
            }
            let seq_decoder_model_info_present_flag = reader.read_bit()? != 0;
            (
                seq_initial_display_delay_minus_1,
                true,
                Some(num_units_in_decoding_tick),
                seq_decoder_model_info_present_flag,
            )
        } else {
            (seq_initial_display_delay_minus_1, false, None, false)
        }
    };

    if seq_decoder_model_info_present_flag {
        // TODO(spec: AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO): parse
        // seq_decoder_model_info() before validating the following dependency
        // maps. Until then, keep the remaining payload tail opaque.
    } else {
        parse_dependency_map_bits(reader, max_tlayer_id, max_mlayer_id)?;
    }

    Ok(SequenceHeader {
        seq_header_id,
        seq_profile_idc,
        single_picture_header_flag,
        seq_level_idx,
        seq_tier,
        chroma_format_idc,
        bit_depth_idc,
        seq_lcr_id,
        still_picture,
        max_tlayer_id,
        max_mlayer_id,
        seq_max_mlayer_count,
        monotonic_output_order_flag,
        frame_width_bits,
        frame_height_bits,
        max_frame_width,
        max_frame_height,
        cropping_window,
        seq_initial_display_delay_minus_1,
        decoder_model_info_present_flag,
        num_units_in_decoding_tick,
        seq_decoder_model_info_present_flag,
    })
}

/// Stub for `sequence_tile_config()` (AV2 § 5.4.2).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_tile_config(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.2-SEQUENCE-TILE-CONFIG): parse sequence_tile_config().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.2-SEQUENCE-TILE-CONFIG",
    })
}

/// Stub for `sequence_partition_config()` (AV2 § 5.4.3).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_partition_config(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.3-SEQUENCE-PARTITION-CONFIG): parse sequence_partition_config().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.3-SEQUENCE-PARTITION-CONFIG",
    })
}

/// Stub for `sequence_segment_config()` (AV2 § 5.4.4).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_segment_config(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG): parse sequence_segment_config().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG",
    })
}

/// Stub for `sequence_intra_config()` (AV2 § 5.4.5).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_intra_config(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.5-SEQUENCE-INTRA-CONFIG): parse sequence_intra_config().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.5-SEQUENCE-INTRA-CONFIG",
    })
}

/// Stub for `sequence_inter_config()` (AV2 § 5.4.6).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_inter_config(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.6-SEQUENCE-INTER-CONFIG): parse sequence_inter_config().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.6-SEQUENCE-INTER-CONFIG",
    })
}

/// Stub for `sequence_scc_config()` (AV2 § 5.4.7).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_scc_config(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.7-SEQUENCE-SCC-CONFIG): parse sequence_scc_config().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.7-SEQUENCE-SCC-CONFIG",
    })
}

/// Stub for `sequence_transform_quant_entropy_config()` (AV2 § 5.4.8).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_transform_quant_entropy_config(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG): parse sequence_transform_quant_entropy_config().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG",
    })
}

/// Stub for `seg_info()` (AV2 § 5.4.9).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_segment_info(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.9-SEGMENT-INFO): parse seg_info().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.9-SEGMENT-INFO",
    })
}

/// Stub for `sequence_filter_config()` (AV2 § 5.4.10).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_filter_config(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.10-SEQUENCE-FILTER-CONFIG): parse sequence_filter_config().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.10-SEQUENCE-FILTER-CONFIG",
    })
}

/// Stub for `user_defined_qm()` (AV2 § 5.4.11).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_user_qm(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.11-USER-QM): parse user_defined_qm().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.11-USER-QM",
    })
}

/// Stub for `timing_info()` (AV2 § 5.4.12).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_timing_info(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.12-TIMING-INFO): parse timing_info().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.12-TIMING-INFO",
    })
}

/// Stub for `seq_decoder_model_info()` (AV2 § 5.4.13).
///
/// # Errors
/// Always returns [`Error::Unimplemented`].
pub fn parse_sequence_decoder_model_info(_reader: &mut BitReader<'_>) -> Result<()> {
    // TODO(spec: AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO): parse seq_decoder_model_info().
    Err(Error::Unimplemented {
        feature: "AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO",
    })
}

fn parse_cropping_window(
    reader: &mut BitReader<'_>,
    max_frame_width: FrameWidth,
    max_frame_height: FrameHeight,
) -> Result<CroppingWindow> {
    let seq_cropping_window_present_flag = reader.read_bit()? != 0;
    if !seq_cropping_window_present_flag {
        return Ok(CroppingWindow::default());
    }

    let left = read_checked_crop(
        reader,
        max_frame_width.minus_1(),
        SequenceHeaderErrorKind::CropLeftOutOfRange,
    )?;
    let right = read_checked_crop(
        reader,
        max_frame_width.minus_1(),
        SequenceHeaderErrorKind::CropRightOutOfRange,
    )?;
    let top = read_checked_crop(
        reader,
        max_frame_height.minus_1(),
        SequenceHeaderErrorKind::CropTopOutOfRange,
    )?;
    let bottom = read_checked_crop(
        reader,
        max_frame_height.minus_1(),
        SequenceHeaderErrorKind::CropBottomOutOfRange,
    )?;
    Ok(CroppingWindow {
        left,
        right,
        top,
        bottom,
    })
}

fn read_checked_crop(
    reader: &mut BitReader<'_>,
    max_minus_1: u32,
    kind: SequenceHeaderErrorKind,
) -> Result<u32> {
    let offset = reader.byte_offset();
    let bit_offset = reader.bit_offset();
    let value = reader.read_uvlc()?;
    if value > max_minus_1 {
        return Err(invalid_sequence_header(offset, bit_offset, kind));
    }
    Ok(value)
}

fn parse_dependency_map_bits(
    reader: &mut BitReader<'_>,
    max_tlayer_id: TemporalLayerId,
    max_mlayer_id: EmbeddedLayerId,
) -> Result<()> {
    if max_mlayer_id.get() > 0 {
        let mlayer_dependency_present_flag = reader.read_bit()? != 0;
        if mlayer_dependency_present_flag {
            for curr_layer in 1..=max_mlayer_id.get() {
                for _ref_layer in (0..=curr_layer).rev() {
                    let _ = reader.read_bit()?;
                }
            }
        }
    }

    if max_tlayer_id.get() > 0 {
        let tlayer_dependency_present_flag = reader.read_bit()? != 0;
        if tlayer_dependency_present_flag {
            let multi_tlayer_dependency_map_present_flag = if max_mlayer_id.get() > 0 {
                reader.read_bit()? != 0
            } else {
                false
            };
            for m_layer in 0..=max_mlayer_id.get() {
                for curr_tlayer in 1..=max_tlayer_id.get() {
                    for _ref_tlayer in (0..=curr_tlayer).rev() {
                        if multi_tlayer_dependency_map_present_flag || m_layer == 0 {
                            let _ = reader.read_bit()?;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn invalid_sequence_header(
    offset: ByteOffset,
    bit_offset: BitOffset,
    kind: SequenceHeaderErrorKind,
) -> Error {
    Error::InvalidSequenceHeader {
        offset,
        bit_offset,
        kind,
    }
}

fn ceil_log2_u32(value: u32) -> u32 {
    if value <= 1 {
        0
    } else {
        u32::BITS - (value - 1).leading_zeros()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

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

    fn valid_single_picture_prefix() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.into_bytes()
    }

    #[test]
    fn parses_single_picture_general_sequence_header() {
        let data = valid_single_picture_prefix();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header_general(&mut reader).unwrap();
        assert_eq!(header.seq_header_id.get(), 0);
        assert!(header.single_picture_header_flag);
        assert_eq!(header.seq_tier, Tier::Main);
        assert_eq!(header.chroma_format_idc, ChromaFormatIdc::Yuv420);
        assert_eq!(header.bit_depth_idc.bit_depth(), 10);
        assert_eq!(header.seq_lcr_id.get(), 0);
        assert!(header.still_picture);
        assert_eq!(header.max_tlayer_id.get(), 0);
        assert_eq!(header.max_mlayer_id.get(), 0);
        assert_eq!(header.seq_max_mlayer_count.get(), 1);
        assert!(header.monotonic_output_order_flag);
        assert_eq!(header.frame_width_bits.get(), 4);
        assert_eq!(header.frame_height_bits.get(), 4);
        assert_eq!(header.max_frame_width.get(), 16);
        assert_eq!(header.max_frame_height.get(), 8);
        assert_eq!(header.cropping_window, CroppingWindow::default());
    }

    #[test]
    fn parses_non_single_picture_general_sequence_header() {
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(1, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(2, 5); // seq_level_idx; seq_tier inferred Main because level <= 3
        bits.uvlc(2); // chroma_format_idc = CHROMA_FORMAT_444
        bits.uvlc(1); // bit_depth_idc = 8-bit
        bits.f(5, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(2, 2); // max_tlayer_id
        bits.f(0, 3); // max_mlayer_id
        bits.bit(0); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(1); // seq_initial_display_delay_present_flag
        bits.f(2, 4); // seq_initial_display_delay_minus_1
        bits.bit(0); // decoder_model_info_present_flag
        bits.bit(0); // tlayer_dependency_present_flag

        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header_general(&mut reader).unwrap();
        assert_eq!(header.seq_header_id.get(), 0);
        assert_eq!(header.seq_profile_idc.get(), 1);
        assert!(!header.single_picture_header_flag);
        assert_eq!(header.seq_level_idx.get(), 2);
        assert_eq!(header.seq_tier, Tier::Main);
        assert_eq!(header.chroma_format_idc, ChromaFormatIdc::Yuv444);
        assert_eq!(header.bit_depth_idc, BitDepthIdc::Eight);
        assert_eq!(header.seq_lcr_id.get(), 5);
        assert!(!header.still_picture);
        assert_eq!(header.max_tlayer_id.get(), 2);
        assert_eq!(header.max_mlayer_id.get(), 0);
        assert_eq!(header.seq_max_mlayer_count.get(), 1);
        assert!(!header.monotonic_output_order_flag);
        assert_eq!(header.seq_initial_display_delay_minus_1, Some(2));
        assert!(!header.decoder_model_info_present_flag);
        assert_eq!(header.num_units_in_decoding_tick, None);
        assert!(!header.seq_decoder_model_info_present_flag);
    }

    #[test]
    fn rejects_seq_header_id_out_of_range() {
        let mut bits = Bits::default();
        bits.uvlc(MAX_SEQ_NUM);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_sequence_header_general(&mut reader),
            Err(Error::InvalidSequenceHeader {
                kind: SequenceHeaderErrorKind::SeqHeaderIdOutOfRange,
                ..
            })
        ));
    }

    #[test]
    fn rejects_chroma_format_out_of_range() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.f(0, 5);
        bits.bit(1);
        bits.f(0, 5);
        bits.uvlc(4);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_sequence_header_general(&mut reader),
            Err(Error::InvalidSequenceHeader {
                kind: SequenceHeaderErrorKind::ChromaFormatOutOfRange,
                ..
            })
        ));
    }

    #[test]
    fn rejects_bit_depth_out_of_range() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.f(0, 5);
        bits.bit(1);
        bits.f(0, 5);
        bits.uvlc(0);
        bits.uvlc(2);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_sequence_header_general(&mut reader),
            Err(Error::InvalidSequenceHeader {
                kind: SequenceHeaderErrorKind::BitDepthOutOfRange,
                ..
            })
        ));
    }

    #[test]
    fn rejects_seq_max_mlayer_count_out_of_range() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.f(0, 5);
        bits.bit(0);
        bits.f(0, 5);
        bits.uvlc(0);
        bits.uvlc(0);
        bits.f(0, 3);
        bits.bit(0);
        bits.f(0, 2);
        bits.f(2, 3);
        bits.f(3, 2);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_sequence_header_general(&mut reader),
            Err(Error::InvalidSequenceHeader {
                kind: SequenceHeaderErrorKind::SeqMaxMlayerCountOutOfRange,
                ..
            })
        ));
    }

    #[test]
    fn rejects_crop_offset_out_of_range() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.f(0, 5);
        bits.bit(1);
        bits.f(0, 5);
        bits.uvlc(0);
        bits.uvlc(0);
        bits.f(3, 4);
        bits.f(3, 4);
        bits.f(15, 4);
        bits.f(7, 4);
        bits.bit(1);
        bits.uvlc(16);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_sequence_header_general(&mut reader),
            Err(Error::InvalidSequenceHeader {
                kind: SequenceHeaderErrorKind::CropLeftOutOfRange,
                ..
            })
        ));
    }

    #[test]
    fn rejects_zero_num_units_in_decoding_tick() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.f(0, 5);
        bits.bit(0);
        bits.f(0, 5);
        bits.uvlc(0);
        bits.uvlc(0);
        bits.f(0, 3);
        bits.bit(0);
        bits.f(0, 2);
        bits.f(0, 3);
        bits.bit(1);
        bits.f(3, 4);
        bits.f(3, 4);
        bits.f(15, 4);
        bits.f(7, 4);
        bits.bit(0);
        bits.bit(0);
        bits.bit(1);
        bits.f(0, 32);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_sequence_header_general(&mut reader),
            Err(Error::InvalidSequenceHeader {
                kind: SequenceHeaderErrorKind::TimingNumUnitsZero,
                ..
            })
        ));
    }

    #[test]
    fn reports_eof() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_sequence_header_general(&mut reader),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// `sequence_header_obu()` general parsing must never panic on arbitrary input.
        #[test]
        fn parse_sequence_header_general_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..128),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_sequence_header_general(&mut reader);
        }
    }
}
