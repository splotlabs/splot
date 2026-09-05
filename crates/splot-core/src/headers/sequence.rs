// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 sequence-header syntax model (AV2 v1.0.0 § 5.4).

use crate::bitio::BitReader;
use crate::error::{Error, Result, SequenceHeaderErrorKind};
use crate::headers::frame::ceil_log2;
use crate::span::{BitOffset, ByteOffset};
use crate::tile::TileParamsInput;
use crate::types::{EmbeddedLayerId, TemporalLayerId};

mod child_configs;
mod layer_dependency;
mod profile;

pub use child_configs::{
    CdefOnSkipTxfm, DrlReorder, MAX_REF_FRAMES, SequenceFilterConfig, SequenceInterConfig,
    SequenceIntraConfig, SequencePartitionConfig, SequenceSccConfig, SequenceSegmentConfig,
    SequenceTileConfig, SequenceTqEntropyConfig, SuperblockSize, parse_sequence_filter_config,
    parse_sequence_inter_config, parse_sequence_intra_config, parse_sequence_partition_config,
    parse_sequence_scc_config, parse_sequence_segment_config, parse_sequence_tile_config,
    parse_sequence_transform_quant_entropy_config,
};
pub use layer_dependency::{
    MAX_NUM_MLAYERS, MAX_NUM_TLAYERS, MLayerDependencyMap, MLayerPresenceMap, TLayerDependencyMap,
};
pub use profile::ProfileIdc;

/// `MAX_SEQ_NUM` for `seq_header_id` validation (AV2 v1.0.0 § 6.4.1).
pub const MAX_SEQ_NUM: u32 = 16;

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
///
/// This covers `sequence_header_obu()` up to (but not including) the child config
/// structures `sequence_partition_config()` … `sequence_tile_config()`. The full
/// composite is [`SequenceHeader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceHeaderGeneral {
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
    /// `seq_cropping_window_present_flag` (`f(1)`); retained so the § 6.8.8
    /// `lcr_cropping_window_present_flag == seq_cropping_window_present_flag`
    /// equality can be checked exactly (a `false` flag infers all-zero offsets,
    /// otherwise indistinguishable from a present all-zero window).
    pub seq_cropping_window_present_flag: bool,
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
    /// Parsed `seq_decoder_model_info()` (§ 5.4.13), present when signalled.
    pub decoder_model_info: Option<SequenceDecoderModelInfo>,
    /// `mlayer_dependency_present_flag` (`f(1)`), read only when
    /// `max_mlayer_id > 0` and inferred `0` otherwise (AV2 § 5.4.1).
    pub mlayer_dependency_present_flag: bool,
    /// `tlayer_dependency_present_flag` (`f(1)`), read only when
    /// `max_tlayer_id > 0` and inferred `0` otherwise (AV2 § 5.4.1).
    pub tlayer_dependency_present_flag: bool,
    /// `multi_tlayer_dependency_map_present_flag` (`f(1)`), read only when
    /// `tlayer_dependency_present_flag` is set and `max_mlayer_id > 0`; inferred
    /// `0` otherwise (AV2 § 5.4.1).
    pub multi_tlayer_dependency_map_present_flag: bool,
    /// Derived `MLayerDependencyMap` (AV2 § 5.4.1 default fill plus signaled
    /// overrides).
    pub mlayer_dependency_map: MLayerDependencyMap,
    /// Derived `TLayerDependencyMap` (AV2 § 5.4.1 default fill plus signaled
    /// overrides, with embedded-layer-0 row replication when
    /// `multi_tlayer_dependency_map_present_flag == 0`).
    pub tlayer_dependency_map: TLayerDependencyMap,
}

/// Parses the general sequence-header syntax through dependency maps (AV2 § 5.4.1).
///
/// This parser stops immediately before the first child config structure
/// (`sequence_partition_config()`); the child structures are parsed by
/// [`parse_sequence_header`]. `seq_decoder_model_info()` (§ 5.4.13) is parsed
/// inline when present so the dependency-map bits that follow it are read at the
/// correct position.
///
/// # Errors
/// Returns typed [`Error`] values for EOF, malformed descriptors, or local § 6.4.1
/// conformance violations.
pub fn parse_sequence_header_general(reader: &mut BitReader<'_>) -> Result<SequenceHeaderGeneral> {
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
    let single_picture_header_flag = reader.read_flag()?;
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
        let still_picture = reader.read_flag()?;
        let max_tlayer_id = TemporalLayerId::from_bits(reader.read_bits_u8(2)?);
        let max_mlayer_id = EmbeddedLayerId::from_bits(reader.read_bits_u8(3)?);
        let seq_max_mlayer_count = if max_mlayer_id.get() > 0 {
            let n = ceil_log2(u32::from(max_mlayer_id.get()) + 1);
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
        let monotonic_output_order_flag = reader.read_flag()?;
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

    let (seq_cropping_window_present_flag, cropping_window) =
        parse_cropping_window(reader, max_frame_width, max_frame_height)?;

    let (
        seq_initial_display_delay_minus_1,
        decoder_model_info_present_flag,
        num_units_in_decoding_tick,
        seq_decoder_model_info_present_flag,
        decoder_model_info,
    ) = if single_picture_header_flag {
        (None, false, None, false, None)
    } else {
        let seq_initial_display_delay_present_flag = reader.read_flag()?;
        let seq_initial_display_delay_minus_1 = if seq_initial_display_delay_present_flag {
            Some(reader.read_bits_u8(4)?)
        } else {
            None
        };
        let decoder_model_info_present_flag = reader.read_flag()?;
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
            let seq_decoder_model_info_present_flag = reader.read_flag()?;
            let decoder_model_info = if seq_decoder_model_info_present_flag {
                Some(parse_sequence_decoder_model_info(reader)?)
            } else {
                None
            };
            (
                seq_initial_display_delay_minus_1,
                true,
                Some(num_units_in_decoding_tick),
                seq_decoder_model_info_present_flag,
                decoder_model_info,
            )
        } else {
            (seq_initial_display_delay_minus_1, false, None, false, None)
        }
    };

    let dependency_maps =
        layer_dependency::parse_dependency_maps(reader, max_tlayer_id, max_mlayer_id)?;

    Ok(SequenceHeaderGeneral {
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
        seq_cropping_window_present_flag,
        cropping_window,
        seq_initial_display_delay_minus_1,
        decoder_model_info_present_flag,
        num_units_in_decoding_tick,
        seq_decoder_model_info_present_flag,
        decoder_model_info,
        mlayer_dependency_present_flag: dependency_maps.mlayer_dependency_present_flag,
        tlayer_dependency_present_flag: dependency_maps.tlayer_dependency_present_flag,
        multi_tlayer_dependency_map_present_flag: dependency_maps
            .multi_tlayer_dependency_map_present_flag,
        mlayer_dependency_map: dependency_maps.mlayer_dependency_map,
        tlayer_dependency_map: dependency_maps.tlayer_dependency_map,
    })
}

/// Full `sequence_header_obu()` model: general fields plus the child configs that
/// follow them (AV2 v1.0.0 § 5.4.1). [`SequenceHeader::unimplemented_at`] records
/// the owning Feature ID when parsing stops at a bounded, table-dependent child.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceHeader {
    /// General §5.4.1 fields through the dependency maps.
    pub general: SequenceHeaderGeneral,
    /// `sequence_partition_config()` (§ 5.4.3).
    pub partition: Option<SequencePartitionConfig>,
    /// `sequence_segment_config()` (§ 5.4.4).
    pub segment: Option<SequenceSegmentConfig>,
    /// `sequence_intra_config()` (§ 5.4.5).
    pub intra: Option<SequenceIntraConfig>,
    /// `sequence_inter_config()` (§ 5.4.6).
    pub inter: Option<SequenceInterConfig>,
    /// `sequence_scc_config()` (§ 5.4.7).
    pub screen_content: Option<SequenceSccConfig>,
    /// `sequence_transform_quant_entropy_config()` (§ 5.4.8).
    pub transform_quant_entropy: Option<SequenceTqEntropyConfig>,
    /// `sequence_filter_config()` (§ 5.4.10).
    pub filter: Option<SequenceFilterConfig>,
    /// `sequence_tile_config()` (§ 5.4.2).
    pub tile: Option<SequenceTileConfig>,
    /// `film_grain_params_present` (§ 5.4.1), read after the child configs.
    pub film_grain_params_present: Option<bool>,
    /// Feature ID at which parsing stopped for a bounded, table-dependent child,
    /// or `None` if the whole sequence header was parsed.
    pub unimplemented_at: Option<&'static str>,
}

impl SequenceHeader {
    /// Returns `true` if the full sequence header (including `film_grain_params_present`) was parsed.
    #[must_use]
    pub const fn is_fully_parsed(&self) -> bool {
        self.unimplemented_at.is_none()
    }
}

/// Parses the full `sequence_header_obu()` syntax, including the child config
/// structures (AV2 v1.0.0 § 5.4.1).
///
/// `seg_info()` (§ 5.4.9) and the sequence `tile_params()` (§ 5.18.7.3) are parsed in
/// full, so a valid sequence header parses completely. The only remaining bounded
/// residual is a `seq_tile_info_present_flag = 1` header whose `seq_level_idx` is a
/// reserved (non-conformant) level with no defined tile bit layout: the returned
/// [`SequenceHeader`] then has [`SequenceHeader::unimplemented_at`] set to
/// `AV2-5.4.2-SEQUENCE-TILE-CONFIG` and the later children are `None`. The parser never
/// skips unknown payload bits.
///
/// # Errors
/// Returns typed [`Error`] values for EOF, malformed descriptors, or local § 6.4
/// conformance violations.
pub fn parse_sequence_header(reader: &mut BitReader<'_>) -> Result<SequenceHeader> {
    let general = parse_sequence_header_general(reader)?;
    let monochrome = general.chroma_format_idc.is_monochrome();
    let single_picture = general.single_picture_header_flag;

    let partition = parse_sequence_partition_config(reader, monochrome, single_picture)?;
    let seq_sb_size = partition.seq_sb_size();

    let segment = parse_sequence_segment_config(reader)?;
    let intra = parse_sequence_intra_config(reader, monochrome)?;
    let inter = parse_sequence_inter_config(reader, single_picture)?;
    let screen_content = parse_sequence_scc_config(reader, single_picture)?;
    let transform_quant_entropy =
        parse_sequence_transform_quant_entropy_config(reader, monochrome, single_picture)?;
    let filter = parse_sequence_filter_config(reader, single_picture, seq_sb_size)?;

    let tile_params_input = TileParamsInput {
        frame_width: general.max_frame_width.get(),
        frame_height: general.max_frame_height.get(),
        uniform_sb_size: seq_sb_size,
        sb_size: seq_sb_size,
        is_bridge: false,
        seq_tier: general.seq_tier,
        seq_level_idx: general.seq_level_idx,
    };
    let tile = parse_sequence_tile_config(reader, tile_params_input)?;

    let unimplemented_at = tile.unimplemented_at();
    let film_grain_params_present = if unimplemented_at.is_none() {
        Some(reader.read_flag()?)
    } else {
        None
    };

    Ok(SequenceHeader {
        general,
        partition: Some(partition),
        segment: Some(segment),
        intra: Some(intra),
        inter: Some(inter),
        screen_content: Some(screen_content),
        transform_quant_entropy: Some(transform_quant_entropy),
        filter: Some(filter),
        tile: Some(tile),
        film_grain_params_present,
        unimplemented_at,
    })
}

/// `timing_info()` (AV2 v1.0.0 § 5.4.12 / § 6.4.12).
///
/// `timing_info()` is not reached from `sequence_header_obu()`; it is referenced by
/// the content-interpretation OBU (`ci_timing_info_present_flag`). This parser is
/// modeled and tested directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingInfo {
    /// `num_units_in_display_tick` (must be greater than 0).
    pub num_units_in_display_tick: u32,
    /// `time_scale` (must be greater than 0).
    pub time_scale: u32,
    /// `equal_picture_interval`.
    pub equal_picture_interval: bool,
    /// `num_ticks_per_picture_minus_1`, present when `equal_picture_interval`.
    pub num_ticks_per_picture_minus_1: Option<u32>,
}

/// Parses `timing_info()` (AV2 v1.0.0 § 5.4.12) and enforces local § 6.4.12 ranges.
///
/// # Errors
/// Returns [`Error::InvalidSequenceHeader`] for zero `num_units_in_display_tick`,
/// zero `time_scale`, or an out-of-range `num_ticks_per_picture_minus_1`, and
/// [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_timing_info(reader: &mut BitReader<'_>) -> Result<TimingInfo> {
    let display_tick_offset = reader.byte_offset();
    let display_tick_bit_offset = reader.bit_offset();
    let num_units_in_display_tick = reader.read_bits(32)?;
    if num_units_in_display_tick == 0 {
        return Err(invalid_sequence_header(
            display_tick_offset,
            display_tick_bit_offset,
            SequenceHeaderErrorKind::TimingDisplayTickZero,
        ));
    }

    let time_scale_offset = reader.byte_offset();
    let time_scale_bit_offset = reader.bit_offset();
    let time_scale = reader.read_bits(32)?;
    if time_scale == 0 {
        return Err(invalid_sequence_header(
            time_scale_offset,
            time_scale_bit_offset,
            SequenceHeaderErrorKind::TimingTimeScaleZero,
        ));
    }

    let equal_picture_interval = reader.read_flag()?;
    let num_ticks_per_picture_minus_1 = if equal_picture_interval {
        let ticks_offset = reader.byte_offset();
        let ticks_bit_offset = reader.bit_offset();
        let value = reader.read_uvlc()?;
        if value == u32::MAX {
            return Err(invalid_sequence_header(
                ticks_offset,
                ticks_bit_offset,
                SequenceHeaderErrorKind::TimingNumTicksOutOfRange,
            ));
        }
        Some(value)
    } else {
        None
    };

    Ok(TimingInfo {
        num_units_in_display_tick,
        time_scale,
        equal_picture_interval,
        num_ticks_per_picture_minus_1,
    })
}

/// `seq_decoder_model_info()` (AV2 v1.0.0 § 5.4.13 / § 6.4.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceDecoderModelInfo {
    /// `decoder_buffer_delay`.
    pub decoder_buffer_delay: u32,
    /// `encoder_buffer_delay`.
    pub encoder_buffer_delay: u32,
    /// `low_delay_mode_flag`.
    pub low_delay_mode_flag: bool,
}

/// Parses `seq_decoder_model_info()` (AV2 v1.0.0 § 5.4.13).
///
/// Annex E buffering-model validation is intentionally out of scope.
///
/// # Errors
/// Returns descriptor errors or [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_decoder_model_info(
    reader: &mut BitReader<'_>,
) -> Result<SequenceDecoderModelInfo> {
    let decoder_buffer_delay = reader.read_uvlc()?;
    let encoder_buffer_delay = reader.read_uvlc()?;
    let low_delay_mode_flag = reader.read_flag()?;

    Ok(SequenceDecoderModelInfo {
        decoder_buffer_delay,
        encoder_buffer_delay,
        low_delay_mode_flag,
    })
}

/// Parses `seq_cropping_window_present_flag` and (when set) the four cropping
/// offsets (AV2 § 5.4.1). Returns the present flag alongside the window so the
/// validator can enforce the § 6.8.8 `lcr_cropping_window_present_flag ==
/// seq_cropping_window_present_flag` equality exactly — when the flag is `0` the
/// offsets are inferred to `0` (a default [`CroppingWindow`]), which would
/// otherwise be indistinguishable from a present-but-all-zero window.
fn parse_cropping_window(
    reader: &mut BitReader<'_>,
    max_frame_width: FrameWidth,
    max_frame_height: FrameHeight,
) -> Result<(bool, CroppingWindow)> {
    let seq_cropping_window_present_flag = reader.read_flag()?;
    if !seq_cropping_window_present_flag {
        return Ok((false, CroppingWindow::default()));
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
    Ok((
        true,
        CroppingWindow {
            left,
            right,
            top,
            bottom,
        },
    ))
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

#[cfg(test)]
mod proptests;
