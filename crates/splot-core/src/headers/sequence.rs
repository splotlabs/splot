// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 sequence-header syntax model (AV2 v1.0.0 § 5.4).

use crate::bitio::BitReader;
use crate::error::{Error, Result, SequenceHeaderErrorKind};
use crate::segment::{SegmentInfo, parse_seg_info};
use crate::span::{BitOffset, ByteOffset};
use crate::tile::{TileParams, TileParamsInput, parse_tile_layout};
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

/// Derived `MLayerDependencyMap[currLayer][refLayer]` (AV2 § 5.4.1): `true` when
/// embedded layer `currLayer` may depend on embedded layer `refLayer`
/// (`mlayer_dependency_map` "specifies the embedded layer dependencies", § 6.4.1).
///
/// The § 5.4.1 default fill is the lower-triangular pattern clipped to
/// `max_mlayer_id` (`refLayer <= currLayer && currLayer <= max_mlayer_id`). When
/// `mlayer_dependency_present_flag` is set, signaled bits replace rows
/// `1..=max_mlayer_id` (the diagonal is itself signaled and may be 0); row 0 keeps
/// the default. The mirror also spells this variable `MlayerDependencyMap` in
/// § 6.8.9 / § 6.10.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MLayerDependencyMap([[bool; MAX_NUM_MLAYERS]; MAX_NUM_MLAYERS]);

impl MLayerDependencyMap {
    /// Builds the § 5.4.1 default fill:
    /// `MLayerDependencyMap[currLayer][refLayer] = refLayer <= currLayer &&
    /// currLayer <= max_mlayer_id` over all `MAX_NUM_MLAYERS x MAX_NUM_MLAYERS`
    /// entries.
    #[must_use]
    pub fn default_for(max_mlayer_id: EmbeddedLayerId) -> Self {
        let max_mlayer = usize::from(max_mlayer_id.get());
        let mut map = [[false; MAX_NUM_MLAYERS]; MAX_NUM_MLAYERS];
        for (curr_layer, row) in map.iter_mut().enumerate() {
            for (ref_layer, entry) in row.iter_mut().enumerate() {
                *entry = ref_layer <= curr_layer && curr_layer <= max_mlayer;
            }
        }
        Self(map)
    }

    /// Returns `MLayerDependencyMap[curr][reference]`. Ids outside the 3-bit
    /// `obu_mlayer_id` range read as `false`.
    #[must_use]
    pub fn depends_on(&self, curr: EmbeddedLayerId, reference: EmbeddedLayerId) -> bool {
        self.0
            .get(usize::from(curr.get()))
            .and_then(|row| row.get(usize::from(reference.get())))
            .copied()
            .unwrap_or(false)
    }

    /// Stores a signaled `mlayer_dependency_map` bit; out-of-range indices (never
    /// produced by the parser, whose loop bounds come from the 3-bit
    /// `max_mlayer_id`) are ignored rather than panicking.
    fn set(&mut self, curr_layer: u8, ref_layer: u8, value: bool) {
        if let Some(entry) = self
            .0
            .get_mut(usize::from(curr_layer))
            .and_then(|row| row.get_mut(usize::from(ref_layer)))
        {
            *entry = value;
        }
    }
}

/// Derived `TLayerDependencyMap[mLayer][currTLayer][refTLayer]` (AV2 § 5.4.1):
/// `true` when, within embedded layer `mLayer`, temporal layer `currTLayer` may
/// depend on temporal layer `refTLayer` (`tlayer_dependency_map` "specifies the
/// temporal layer dependencies", § 6.4.1).
///
/// The § 5.4.1 default fill is `refTLayer <= currTLayer && currTLayer <=
/// max_tlayer_id && mLayer <= max_mlayer_id`. When
/// `tlayer_dependency_present_flag` is set, signaled bits replace rows
/// `currTLayer 1..=max_tlayer_id`; with `multi_tlayer_dependency_map_present_flag
/// == 0` the bits are signaled only for embedded layer 0 and replicated to
/// embedded layers `1..=max_mlayer_id` (§ 6.4.1: "the same values are used for
/// all embedded layers"). The mirror also spells this variable
/// `TlayerDependencyMap` in § 6.8.9 / § 6.10.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TLayerDependencyMap([[[bool; MAX_NUM_TLAYERS]; MAX_NUM_TLAYERS]; MAX_NUM_MLAYERS]);

impl TLayerDependencyMap {
    /// Builds the § 5.4.1 default fill:
    /// `TLayerDependencyMap[mLayer][currTLayer][refTLayer] = refTLayer <=
    /// currTLayer && currTLayer <= max_tlayer_id && mLayer <= max_mlayer_id` over
    /// all `MAX_NUM_MLAYERS x MAX_NUM_TLAYERS x MAX_NUM_TLAYERS` entries.
    #[must_use]
    pub fn default_for(max_tlayer_id: TemporalLayerId, max_mlayer_id: EmbeddedLayerId) -> Self {
        let max_tlayer = usize::from(max_tlayer_id.get());
        let max_mlayer = usize::from(max_mlayer_id.get());
        let mut map = [[[false; MAX_NUM_TLAYERS]; MAX_NUM_TLAYERS]; MAX_NUM_MLAYERS];
        for (m_layer, plane) in map.iter_mut().enumerate() {
            for (curr_tlayer, row) in plane.iter_mut().enumerate() {
                for (ref_tlayer, entry) in row.iter_mut().enumerate() {
                    *entry = ref_tlayer <= curr_tlayer
                        && curr_tlayer <= max_tlayer
                        && m_layer <= max_mlayer;
                }
            }
        }
        Self(map)
    }

    /// Returns `TLayerDependencyMap[mlayer][curr][reference]`. Ids outside the
    /// 3-bit `obu_mlayer_id` / 2-bit `obu_tlayer_id` ranges read as `false`.
    #[must_use]
    pub fn depends_on(
        &self,
        mlayer: EmbeddedLayerId,
        curr: TemporalLayerId,
        reference: TemporalLayerId,
    ) -> bool {
        self.0
            .get(usize::from(mlayer.get()))
            .and_then(|plane| plane.get(usize::from(curr.get())))
            .and_then(|row| row.get(usize::from(reference.get())))
            .copied()
            .unwrap_or(false)
    }

    /// Stores a signaled (or row-0-replicated) `tlayer_dependency_map` bit;
    /// out-of-range indices (never produced by the parser, whose loop bounds come
    /// from the 2-bit `max_tlayer_id` and 3-bit `max_mlayer_id`) are ignored
    /// rather than panicking.
    fn set(&mut self, m_layer: u8, curr_tlayer: u8, ref_tlayer: u8, value: bool) {
        if let Some(entry) = self
            .0
            .get_mut(usize::from(m_layer))
            .and_then(|plane| plane.get_mut(usize::from(curr_tlayer)))
            .and_then(|row| row.get_mut(usize::from(ref_tlayer)))
        {
            *entry = value;
        }
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
            // AV2 § 5.4.1: when present, seq_decoder_model_info() (§ 5.4.13) is
            // parsed here, before the dependency-map bits.
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

    let dependency_maps = parse_dependency_maps(reader, max_tlayer_id, max_mlayer_id)?;

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

// AV2 § 3 constants used by the sequence-header child config parsers. Values are
// taken from the AV2 v1.0.0 symbol table; they control bit positions and so are
// modeled explicitly rather than inlined as magic numbers.
/// `MOTION_MODES`: number of motion modes (AV2 § 3).
const MOTION_MODES: usize = 5;
/// `INTERINTRA`: first signalled motion-mode index (AV2 § 3).
const INTERINTRA: usize = 1;
/// `DELTAWARP`: delta-warp motion-mode index (AV2 § 3).
const DELTAWARP: usize = 3;
/// `MAX_REF_MV_STACK_SIZE` (AV2 § 3); `ns(MAX_REF_MV_STACK_SIZE - 1)` width.
const MAX_REF_MV_STACK_SIZE: u32 = 6;
/// `MAX_REF_BV_STACK_SIZE` (AV2 § 3); `ns(MAX_REF_BV_STACK_SIZE - 1)` width.
const MAX_REF_BV_STACK_SIZE: u32 = 4;
/// `SELECT_SCREEN_CONTENT_TOOLS` (AV2 § 3).
const SELECT_SCREEN_CONTENT_TOOLS: u8 = 2;
/// `SELECT_INTEGER_MV` (AV2 § 3).
const SELECT_INTEGER_MV: u8 = 2;

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

    // AV2 § 5.4.2: tile_params(max_frame_width_minus_1 + 1, max_frame_height_minus_1 + 1,
    // seqSbSize, seqSbSize, 0) with the sequence tier/level.
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

    if let Some(feature) = tile.unimplemented_at() {
        return Ok(SequenceHeader {
            general,
            partition: Some(partition),
            segment: Some(segment),
            intra: Some(intra),
            inter: Some(inter),
            screen_content: Some(screen_content),
            transform_quant_entropy: Some(transform_quant_entropy),
            filter: Some(filter),
            tile: Some(tile),
            film_grain_params_present: None,
            unimplemented_at: Some(feature),
        });
    }

    let film_grain_params_present = reader.read_bit()? != 0;

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
        film_grain_params_present: Some(film_grain_params_present),
        unimplemented_at: None,
    })
}

/// Sequence superblock size, derived per `get_seq_sb_size()` (AV2 § 5.18.7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperblockSize {
    /// `BLOCK_64X64`.
    Block64x64,
    /// `BLOCK_128X128`.
    Block128x128,
    /// `BLOCK_256X256`.
    Block256x256,
}

/// `sequence_partition_config()` (AV2 v1.0.0 § 5.4.3 / § 6.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencePartitionConfig {
    /// `use_256x256_superblock`.
    pub use_256x256_superblock: bool,
    /// `use_128x128_superblock` (inferred `false` when 256×256 superblocks are used).
    pub use_128x128_superblock: bool,
    /// `enable_sdp` (inferred `0` for monochrome).
    pub enable_sdp: bool,
    /// `enable_extended_sdp` (inferred `0` unless SDP is enabled and not a single picture).
    pub enable_extended_sdp: bool,
    /// `enable_ext_partitions`.
    pub enable_ext_partitions: bool,
    /// `enable_uneven_4way_partitions` (inferred `0` unless extended partitions are enabled).
    pub enable_uneven_4way_partitions: bool,
    /// `reduce_pb_aspect_ratio`.
    pub reduce_pb_aspect_ratio: bool,
    /// `MaxPbAspectRatio` (inferred `8` unless `reduce_pb_aspect_ratio`).
    pub max_pb_aspect_ratio: u32,
}

impl SequencePartitionConfig {
    /// Returns `get_seq_sb_size()` (AV2 § 5.18.7.6).
    #[must_use]
    pub const fn seq_sb_size(&self) -> SuperblockSize {
        if self.use_256x256_superblock {
            SuperblockSize::Block256x256
        } else if self.use_128x128_superblock {
            SuperblockSize::Block128x128
        } else {
            SuperblockSize::Block64x64
        }
    }
}

/// Parses `sequence_partition_config()` (AV2 v1.0.0 § 5.4.3).
///
/// `monochrome` is `Monochrome` and `single_picture` is `single_picture_header_flag`
/// from the general header; both gate conditional fields.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_partition_config(
    reader: &mut BitReader<'_>,
    monochrome: bool,
    single_picture: bool,
) -> Result<SequencePartitionConfig> {
    let use_256x256_superblock = reader.read_bit()? != 0;
    let use_128x128_superblock = if use_256x256_superblock {
        false
    } else {
        reader.read_bit()? != 0
    };
    let enable_sdp = if monochrome {
        false
    } else {
        reader.read_bit()? != 0
    };
    let enable_extended_sdp = if enable_sdp && !single_picture {
        reader.read_bit()? != 0
    } else {
        false
    };
    let enable_ext_partitions = reader.read_bit()? != 0;
    let enable_uneven_4way_partitions = if enable_ext_partitions {
        reader.read_bit()? != 0
    } else {
        false
    };
    let reduce_pb_aspect_ratio = reader.read_bit()? != 0;
    let max_pb_aspect_ratio = if reduce_pb_aspect_ratio {
        let max_pb_aspect_ratio_log2_minus_1 = reader.read_bits_u8(1)?;
        1u32 << (u32::from(max_pb_aspect_ratio_log2_minus_1) + 1)
    } else {
        8
    };

    Ok(SequencePartitionConfig {
        use_256x256_superblock,
        use_128x128_superblock,
        enable_sdp,
        enable_extended_sdp,
        enable_ext_partitions,
        enable_uneven_4way_partitions,
        reduce_pb_aspect_ratio,
        max_pb_aspect_ratio,
    })
}

/// `sequence_segment_config()` (AV2 v1.0.0 § 5.4.4 / § 6.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceSegmentConfig {
    /// `enable_ext_seg`.
    pub enable_ext_seg: bool,
    /// `MaxSegments` (`16` when extended segmentation is enabled, else `8`).
    pub max_segments: u8,
    /// `seq_seg_info_present_flag`.
    pub seq_seg_info_present_flag: bool,
    /// `seq_allow_seg_info_change`, present when segment info is signalled.
    pub seq_allow_seg_info_change: Option<bool>,
    /// Parsed `seg_info(MaxSegments)` (§ 5.4.9), present when segment info is signalled.
    pub segment_info: Option<SegmentInfo>,
}

/// Parses `sequence_segment_config()` (AV2 § 5.4.4), including `seg_info(MaxSegments)`
/// (§ 5.4.9) when `seq_seg_info_present_flag` is set.
///
/// # Errors
/// Returns descriptor errors or [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_segment_config(reader: &mut BitReader<'_>) -> Result<SequenceSegmentConfig> {
    let enable_ext_seg = reader.read_bit()? != 0;
    let max_segments = if enable_ext_seg { 16 } else { 8 };
    let seq_seg_info_present_flag = reader.read_bit()? != 0;
    let (seq_allow_seg_info_change, segment_info) = if seq_seg_info_present_flag {
        let allow = reader.read_bit()? != 0;
        // AV2 § 5.4.4: ( SeqFeatureEnabled, SeqFeatureData ) = seg_info( MaxSegments ).
        let info = parse_seg_info(reader, max_segments)?;
        (Some(allow), Some(info))
    } else {
        (None, None)
    };

    Ok(SequenceSegmentConfig {
        enable_ext_seg,
        max_segments,
        seq_seg_info_present_flag,
        seq_allow_seg_info_change,
        segment_info,
    })
}

/// `sequence_intra_config()` (AV2 v1.0.0 § 5.4.5 / § 6.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceIntraConfig {
    /// `enable_dip`.
    pub enable_dip: bool,
    /// `enable_intra_edge_filter`.
    pub enable_intra_edge_filter: bool,
    /// `enable_mrls`.
    pub enable_mrls: bool,
    /// `enable_cfl_intra`.
    pub enable_cfl_intra: bool,
    /// `cfl_ds_filter_index` (inferred `0` for monochrome).
    pub cfl_ds_filter_index: u8,
    /// `enable_mhccp`.
    pub enable_mhccp: bool,
    /// `enable_ibp`.
    pub enable_ibp: bool,
}

/// Parses `sequence_intra_config()` (AV2 v1.0.0 § 5.4.5).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_intra_config(
    reader: &mut BitReader<'_>,
    monochrome: bool,
) -> Result<SequenceIntraConfig> {
    let enable_dip = reader.read_bit()? != 0;
    let enable_intra_edge_filter = reader.read_bit()? != 0;
    let enable_mrls = reader.read_bit()? != 0;
    let enable_cfl_intra = reader.read_bit()? != 0;
    let cfl_ds_filter_index = if monochrome {
        0
    } else {
        reader.read_bits_u8(2)?
    };
    let enable_mhccp = reader.read_bit()? != 0;
    let enable_ibp = reader.read_bit()? != 0;

    Ok(SequenceIntraConfig {
        enable_dip,
        enable_intra_edge_filter,
        enable_mrls,
        enable_cfl_intra,
        cfl_ds_filter_index,
        enable_mhccp,
        enable_ibp,
    })
}

/// DRL reordering mode derived from `disable_drl_reorder` / `constrain_drl_reorder` (AV2 § 5.4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrlReorder {
    /// `DRL_REORDER_DISABLED`.
    Disabled,
    /// `DRL_REORDER_CONSTRAINT`.
    Constraint,
    /// `DRL_REORDER_ALWAYS`.
    Always,
}

/// `sequence_inter_config()` (AV2 v1.0.0 § 5.4.6 / § 6.4.6).
///
/// Fields not read on the `single_picture_header_flag` branch carry the spec's
/// inferred values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceInterConfig {
    /// `seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]`; index `0` (`SIMPLE`) is always `0`.
    pub seq_enabled_motion_modes: [bool; MOTION_MODES],
    /// `seq_frame_motion_modes_present_flag`.
    pub seq_frame_motion_modes_present_flag: bool,
    /// `enable_six_param_warp_delta`.
    pub enable_six_param_warp_delta: bool,
    /// `enable_masked_compound`.
    pub enable_masked_compound: bool,
    /// `enable_ref_frame_mvs`.
    pub enable_ref_frame_mvs: bool,
    /// `reduced_ref_frame_mvs_mode`.
    pub reduced_ref_frame_mvs_mode: bool,
    /// `OrderHintBits`.
    pub order_hint_bits: u8,
    /// `enable_refmvbank`.
    pub enable_refmvbank: bool,
    /// `DrlReorder`.
    pub drl_reorder: DrlReorder,
    /// `explicit_ref_frame_map`.
    pub explicit_ref_frame_map: bool,
    /// `NumRefFrames`.
    pub num_ref_frames: u8,
    /// `long_term_frame_id_bits`.
    pub long_term_frame_id_bits: u8,
    /// `seq_max_drl_bits_minus_1` (inferred `0` for single pictures).
    pub seq_max_drl_bits_minus_1: u32,
    /// `allow_frame_max_drl_bits` (inferred `0` for single pictures).
    pub allow_frame_max_drl_bits: bool,
    /// `seq_max_bvp_drl_bits_minus_1`.
    pub seq_max_bvp_drl_bits_minus_1: u32,
    /// `allow_frame_max_bvp_drl_bits`.
    pub allow_frame_max_bvp_drl_bits: bool,
    /// `num_same_ref_compound` (inferred `0` for single pictures).
    pub num_same_ref_compound: u8,
    /// `enable_tip`.
    pub enable_tip: bool,
    /// `EnableTipOutput`.
    pub enable_tip_output: bool,
    /// `enable_tip_hole_fill`.
    pub enable_tip_hole_fill: bool,
    /// `enable_mv_traj`.
    pub enable_mv_traj: bool,
    /// `enable_bawp`.
    pub enable_bawp: bool,
    /// `enable_cwp` (inferred `0` for single pictures).
    pub enable_cwp: bool,
    /// `enable_imp_msk_bld`.
    pub enable_imp_msk_bld: bool,
    /// `enable_df_sub_pu` (inferred `0` for single pictures).
    pub enable_df_sub_pu: bool,
    /// `enable_tip_explicit_qp`.
    pub enable_tip_explicit_qp: bool,
    /// `enable_opfl_refine` (`REFINE_NONE` = `0` for single pictures).
    pub enable_opfl_refine: u8,
    /// `enable_refinemv` (inferred `0` for single pictures).
    pub enable_refinemv: bool,
    /// `enable_tip_refinemv`.
    pub enable_tip_refinemv: bool,
    /// `enable_bru` (inferred `0` for single pictures).
    pub enable_bru: bool,
    /// `enable_adaptive_mvd` (inferred `0` for single pictures).
    pub enable_adaptive_mvd: bool,
    /// `enable_mvd_sign_derive` (inferred `0` for single pictures).
    pub enable_mvd_sign_derive: bool,
    /// `enable_flex_mvres` (inferred `0` for single pictures).
    pub enable_flex_mvres: bool,
    /// `enable_global_motion`.
    pub enable_global_motion: bool,
    /// `enable_short_refresh_frame_flags` (inferred `0` for single pictures).
    pub enable_short_refresh_frame_flags: bool,
}

fn read_drl_reorder(reader: &mut BitReader<'_>) -> Result<DrlReorder> {
    let disable_drl_reorder = reader.read_bit()? != 0;
    if disable_drl_reorder {
        Ok(DrlReorder::Disabled)
    } else {
        let constrain_drl_reorder = reader.read_bit()? != 0;
        Ok(if constrain_drl_reorder {
            DrlReorder::Constraint
        } else {
            DrlReorder::Always
        })
    }
}

/// Parses `sequence_inter_config()` (AV2 v1.0.0 § 5.4.6).
///
/// This reads sequence-level inter tool flags only; it does not model motion
/// estimation, reference management, or any decoding process.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] or descriptor errors if the payload is malformed.
pub fn parse_sequence_inter_config(
    reader: &mut BitReader<'_>,
    single_picture: bool,
) -> Result<SequenceInterConfig> {
    let mut config = SequenceInterConfig {
        seq_enabled_motion_modes: [false; MOTION_MODES],
        seq_frame_motion_modes_present_flag: false,
        enable_six_param_warp_delta: false,
        enable_masked_compound: false,
        enable_ref_frame_mvs: false,
        reduced_ref_frame_mvs_mode: false,
        order_hint_bits: 0,
        enable_refmvbank: false,
        drl_reorder: DrlReorder::Disabled,
        explicit_ref_frame_map: false,
        num_ref_frames: 2,
        long_term_frame_id_bits: 0,
        seq_max_drl_bits_minus_1: 0,
        allow_frame_max_drl_bits: false,
        seq_max_bvp_drl_bits_minus_1: 0,
        allow_frame_max_bvp_drl_bits: false,
        num_same_ref_compound: 0,
        enable_tip: false,
        enable_tip_output: false,
        enable_tip_hole_fill: false,
        enable_mv_traj: false,
        enable_bawp: false,
        enable_cwp: false,
        enable_imp_msk_bld: false,
        enable_df_sub_pu: false,
        enable_tip_explicit_qp: false,
        enable_opfl_refine: 0,
        enable_refinemv: false,
        enable_tip_refinemv: false,
        enable_bru: false,
        enable_adaptive_mvd: false,
        enable_mvd_sign_derive: false,
        enable_flex_mvres: false,
        enable_global_motion: false,
        enable_short_refresh_frame_flags: false,
    };

    if single_picture {
        // single_picture_header_flag branch: only a small set of flags is signalled.
        config.enable_refmvbank = reader.read_bit()? != 0;
        config.drl_reorder = read_drl_reorder(reader)?;
        config.seq_max_bvp_drl_bits_minus_1 = reader.read_ns(MAX_REF_BV_STACK_SIZE - 1)?;
        config.allow_frame_max_bvp_drl_bits = reader.read_bit()? != 0;
        config.enable_bawp = reader.read_bit()? != 0;
        // NumRefFrames = 2, long_term_frame_id_bits = 0 (inferred above).
        return Ok(config);
    }

    let mut motion_mode_enabled = false;
    for mode in INTERINTRA..MOTION_MODES {
        let enabled = reader.read_bit()? != 0;
        config.seq_enabled_motion_modes[mode] = enabled;
        motion_mode_enabled |= enabled;
    }
    config.seq_frame_motion_modes_present_flag = if motion_mode_enabled {
        reader.read_bit()? != 0
    } else {
        false
    };
    config.enable_six_param_warp_delta = if config.seq_enabled_motion_modes[DELTAWARP] {
        reader.read_bit()? != 0
    } else {
        false
    };
    config.enable_masked_compound = reader.read_bit()? != 0;
    config.enable_ref_frame_mvs = reader.read_bit()? != 0;
    config.reduced_ref_frame_mvs_mode = if config.enable_ref_frame_mvs {
        reader.read_bit()? != 0
    } else {
        false
    };
    let order_hint_bits_minus_1 = reader.read_bits_u8(4)?;
    config.order_hint_bits = order_hint_bits_minus_1 + 1;
    config.enable_refmvbank = reader.read_bit()? != 0;
    config.drl_reorder = read_drl_reorder(reader)?;
    config.explicit_ref_frame_map = reader.read_bit()? != 0;
    let explicit_num_ref_frames = reader.read_bit()? != 0;
    config.num_ref_frames = if explicit_num_ref_frames {
        reader.read_bits_u8(4)? + 1
    } else {
        8
    };
    config.long_term_frame_id_bits = reader.read_bits_u8(3)?;
    config.seq_max_drl_bits_minus_1 = reader.read_ns(MAX_REF_MV_STACK_SIZE - 1)?;
    config.allow_frame_max_drl_bits = reader.read_bit()? != 0;
    config.seq_max_bvp_drl_bits_minus_1 = reader.read_ns(MAX_REF_BV_STACK_SIZE - 1)?;
    config.allow_frame_max_bvp_drl_bits = reader.read_bit()? != 0;
    config.num_same_ref_compound = reader.read_bits_u8(2)?;
    config.enable_tip = reader.read_bit()? != 0;
    if config.enable_tip {
        let disable_tip_output = reader.read_bit()? != 0;
        config.enable_tip_output = !disable_tip_output;
        config.enable_tip_hole_fill = reader.read_bit()? != 0;
    }
    config.enable_mv_traj = reader.read_bit()? != 0;
    config.enable_bawp = reader.read_bit()? != 0;
    config.enable_cwp = reader.read_bit()? != 0;
    config.enable_imp_msk_bld = reader.read_bit()? != 0;
    config.enable_df_sub_pu = reader.read_bit()? != 0;
    config.enable_tip_explicit_qp = if config.enable_tip_output && config.enable_df_sub_pu {
        reader.read_bit()? != 0
    } else {
        false
    };
    config.enable_opfl_refine = reader.read_bits_u8(2)?;
    config.enable_refinemv = reader.read_bit()? != 0;
    config.enable_tip_refinemv =
        if config.enable_tip && (config.enable_opfl_refine != 0 || config.enable_refinemv) {
            reader.read_bit()? != 0
        } else {
            false
        };
    config.enable_bru = reader.read_bit()? != 0;
    config.enable_adaptive_mvd = reader.read_bit()? != 0;
    config.enable_mvd_sign_derive = reader.read_bit()? != 0;
    config.enable_flex_mvres = reader.read_bit()? != 0;
    // single_picture_header_flag is false on this branch, so enable_global_motion is signalled.
    config.enable_global_motion = reader.read_bit()? != 0;
    config.enable_short_refresh_frame_flags = reader.read_bit()? != 0;

    Ok(config)
}

/// `sequence_scc_config()` (AV2 v1.0.0 § 5.4.7 / § 6.4.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceSccConfig {
    /// `seq_force_screen_content_tools` (`SELECT_SCREEN_CONTENT_TOOLS` = 2 when chosen).
    pub seq_force_screen_content_tools: u8,
    /// `seq_force_integer_mv` (`SELECT_INTEGER_MV` = 2 when chosen).
    pub seq_force_integer_mv: u8,
}

/// Parses `sequence_scc_config()` (AV2 v1.0.0 § 5.4.7).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_scc_config(
    reader: &mut BitReader<'_>,
    single_picture: bool,
) -> Result<SequenceSccConfig> {
    if single_picture {
        return Ok(SequenceSccConfig {
            seq_force_screen_content_tools: SELECT_SCREEN_CONTENT_TOOLS,
            seq_force_integer_mv: SELECT_INTEGER_MV,
        });
    }

    let seq_choose_screen_content_tools = reader.read_bit()? != 0;
    let seq_force_screen_content_tools = if seq_choose_screen_content_tools {
        SELECT_SCREEN_CONTENT_TOOLS
    } else {
        reader.read_bits_u8(1)?
    };

    let seq_force_integer_mv = if seq_force_screen_content_tools > 0 {
        let seq_choose_integer_mv = reader.read_bit()? != 0;
        if seq_choose_integer_mv {
            SELECT_INTEGER_MV
        } else {
            reader.read_bits_u8(1)?
        }
    } else {
        SELECT_INTEGER_MV
    };

    Ok(SequenceSccConfig {
        seq_force_screen_content_tools,
        seq_force_integer_mv,
    })
}

/// `sequence_transform_quant_entropy_config()` (AV2 v1.0.0 § 5.4.8 / § 6.4.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceTqEntropyConfig {
    /// `enable_fsc`.
    pub enable_fsc: bool,
    /// `enable_idtx_intra` (inferred `1` when `enable_fsc`).
    pub enable_idtx_intra: bool,
    /// `enable_intra_ist`.
    pub enable_intra_ist: bool,
    /// `enable_inter_ist`.
    pub enable_inter_ist: bool,
    /// `enable_chroma_dctonly` (inferred `0` for monochrome).
    pub enable_chroma_dctonly: bool,
    /// `enable_inter_ddt` (only signalled for non-single-picture headers).
    pub enable_inter_ddt: bool,
    /// `reduced_tx_part_set`.
    pub reduced_tx_part_set: bool,
    /// `enable_cctx` (inferred `0` for monochrome).
    pub enable_cctx: bool,
    /// `enable_tcq`.
    pub enable_tcq: bool,
    /// `choose_tcq_per_frame`.
    pub choose_tcq_per_frame: bool,
    /// `enable_parity_hiding`.
    pub enable_parity_hiding: bool,
    /// `enable_avg_cdf` (inferred `1` for single pictures).
    pub enable_avg_cdf: bool,
    /// `avg_cdf_type` (inferred `1` for single pictures, `0` when averaging disabled).
    pub avg_cdf_type: u8,
    /// `separate_uv_delta_q` (inferred `0` for monochrome).
    pub separate_uv_delta_q: bool,
    /// `equal_ac_dc_q`.
    pub equal_ac_dc_q: bool,
    /// `base_y_dc_delta_q` (raw 5-bit field, only present when `!equal_ac_dc_q`).
    pub base_y_dc_delta_q: u8,
    /// `y_dc_delta_q_enabled`.
    pub y_dc_delta_q_enabled: bool,
    /// `base_uv_dc_delta_q` (chroma only; the raw 5-bit field when `!equal_ac_dc_q`,
    /// otherwise mirrored from `base_uv_ac_delta_q` per AV2 § 5.4.8).
    pub base_uv_dc_delta_q: u8,
    /// `uv_dc_delta_q_enabled`.
    pub uv_dc_delta_q_enabled: bool,
    /// `base_uv_ac_delta_q` (raw 5-bit field, chroma only).
    pub base_uv_ac_delta_q: u8,
    /// `uv_ac_delta_q_enabled`.
    pub uv_ac_delta_q_enabled: bool,
}

/// Parses `sequence_transform_quant_entropy_config()` (AV2 v1.0.0 § 5.4.8).
///
/// Only sequence-level transform/quant/entropy tool flags are read; no transform,
/// quantizer, or entropy coder is implemented.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_transform_quant_entropy_config(
    reader: &mut BitReader<'_>,
    monochrome: bool,
    single_picture: bool,
) -> Result<SequenceTqEntropyConfig> {
    let enable_fsc = reader.read_bit()? != 0;
    let enable_idtx_intra = if enable_fsc {
        true
    } else {
        reader.read_bit()? != 0
    };
    let enable_intra_ist = reader.read_bit()? != 0;
    let enable_inter_ist = reader.read_bit()? != 0;
    let enable_chroma_dctonly = if monochrome {
        false
    } else {
        reader.read_bit()? != 0
    };
    let enable_inter_ddt = if single_picture {
        false
    } else {
        reader.read_bit()? != 0
    };
    let reduced_tx_part_set = reader.read_bit()? != 0;
    let enable_cctx = if monochrome {
        false
    } else {
        reader.read_bit()? != 0
    };
    let enable_tcq = reader.read_bit()? != 0;
    let choose_tcq_per_frame = if enable_tcq && !single_picture {
        reader.read_bit()? != 0
    } else {
        false
    };
    // AV2 § 5.4.8: enable_parity_hiding is inferred 0 only when
    // (enable_tcq && !choose_tcq_per_frame); in every other case (including
    // !enable_tcq) the spec reads the f(1) flag in the else branch.
    let enable_parity_hiding = if enable_tcq && !choose_tcq_per_frame {
        false
    } else {
        reader.read_bit()? != 0
    };
    let (enable_avg_cdf, avg_cdf_type) = if single_picture {
        (true, 1)
    } else {
        let enable_avg_cdf = reader.read_bit()? != 0;
        let avg_cdf_type = if enable_avg_cdf {
            reader.read_bits_u8(1)?
        } else {
            0
        };
        (enable_avg_cdf, avg_cdf_type)
    };
    let separate_uv_delta_q = if monochrome {
        false
    } else {
        reader.read_bit()? != 0
    };

    let equal_ac_dc_q = reader.read_bit()? != 0;
    let mut base_y_dc_delta_q = 0;
    let mut y_dc_delta_q_enabled = false;
    if !equal_ac_dc_q {
        base_y_dc_delta_q = reader.read_bits_u8(5)?;
        y_dc_delta_q_enabled = reader.read_bit()? != 0;
    }
    let mut base_uv_dc_delta_q = 0;
    let mut uv_dc_delta_q_enabled = false;
    let mut base_uv_ac_delta_q = 0;
    let mut uv_ac_delta_q_enabled = false;
    if !monochrome {
        if !equal_ac_dc_q {
            base_uv_dc_delta_q = reader.read_bits_u8(5)?;
            uv_dc_delta_q_enabled = reader.read_bit()? != 0;
        }
        base_uv_ac_delta_q = reader.read_bits_u8(5)?;
        uv_ac_delta_q_enabled = reader.read_bit()? != 0;
        if equal_ac_dc_q {
            // AV2 § 5.4.8: when equal_ac_dc_q, BaseUVDcDeltaQ = BaseUVAcDeltaQ.
            // base_uv_dc_delta_q is not signalled here, so mirror the AC field.
            base_uv_dc_delta_q = base_uv_ac_delta_q;
        }
    }

    Ok(SequenceTqEntropyConfig {
        enable_fsc,
        enable_idtx_intra,
        enable_intra_ist,
        enable_inter_ist,
        enable_chroma_dctonly,
        enable_inter_ddt,
        reduced_tx_part_set,
        enable_cctx,
        enable_tcq,
        choose_tcq_per_frame,
        enable_parity_hiding,
        enable_avg_cdf,
        avg_cdf_type,
        separate_uv_delta_q,
        equal_ac_dc_q,
        base_y_dc_delta_q,
        y_dc_delta_q_enabled,
        base_uv_dc_delta_q,
        uv_dc_delta_q_enabled,
        base_uv_ac_delta_q,
        uv_ac_delta_q_enabled,
    })
}

/// `CdefOnSkipTxfm` mode derived in `sequence_filter_config()` (AV2 § 5.4.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdefOnSkipTxfm {
    /// `CDEF_ON_SKIP_TXFM_ADAPTIVE`.
    Adaptive,
    /// `CDEF_ON_SKIP_TXFM_ALWAYS_ON`.
    AlwaysOn,
    /// `CDEF_ON_SKIP_TXFM_DISABLED`.
    Disabled,
}

/// `sequence_filter_config()` (AV2 v1.0.0 § 5.4.10 / § 6.4.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceFilterConfig {
    /// `disable_loopfilters_across_tiles`.
    pub disable_loopfilters_across_tiles: bool,
    /// `enable_cdef`.
    pub enable_cdef: bool,
    /// `enable_gdf`.
    pub enable_gdf: bool,
    /// `gdf_unit_matches_sb_size` (only signalled when GDF is on and superblocks are 64×64).
    pub gdf_unit_matches_sb_size: bool,
    /// `enable_restoration`.
    pub enable_restoration: bool,
    /// `lr_tools_disable[0][RESTORE_PC_WIENER]`.
    pub lr_pc_wiener_disabled: bool,
    /// `lr_tools_disable[0][RESTORE_WIENER_NONSEP]`.
    pub lr_wiener_nonsep_disabled: bool,
    /// `lr_tools_disable[1][RESTORE_PC_WIENER]` (inferred `1`/`true` when restoration is enabled).
    pub lr_uv_pc_wiener_disabled: bool,
    /// `lr_tools_uv_present`.
    pub lr_tools_uv_present: bool,
    /// `lr_tools_disable[1][RESTORE_WIENER_NONSEP]`.
    pub lr_uv_wiener_nonsep_disabled: bool,
    /// `enable_ccso`.
    pub enable_ccso: bool,
    /// `ccso_unit_matches_sb_size`.
    pub ccso_unit_matches_sb_size: bool,
    /// `CdefOnSkipTxfm`.
    pub cdef_on_skip_txfm: CdefOnSkipTxfm,
    /// `df_par_bits_minus_2`.
    pub df_par_bits_minus_2: u8,
}

/// Parses `sequence_filter_config()` (AV2 v1.0.0 § 5.4.10).
///
/// `seq_sb_size` is `get_seq_sb_size()` from the partition config; it gates
/// `gdf_unit_matches_sb_size`. Only sequence-level filter tool flags are read.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_filter_config(
    reader: &mut BitReader<'_>,
    single_picture: bool,
    seq_sb_size: SuperblockSize,
) -> Result<SequenceFilterConfig> {
    let disable_loopfilters_across_tiles = reader.read_bit()? != 0;
    let enable_cdef = reader.read_bit()? != 0;
    let enable_gdf = reader.read_bit()? != 0;
    let gdf_unit_matches_sb_size = if enable_gdf && seq_sb_size == SuperblockSize::Block64x64 {
        reader.read_bit()? != 0
    } else {
        false
    };
    let enable_restoration = reader.read_bit()? != 0;
    let mut lr_pc_wiener_disabled = false;
    let mut lr_wiener_nonsep_disabled = false;
    let mut lr_tools_uv_present = false;
    let mut lr_uv_wiener_nonsep_disabled = false;
    // AV2 § 5.4.10: lr_tools_disable[1][RESTORE_PC_WIENER] is inferred to 1 when
    // restoration is enabled (it is never signalled).
    let lr_uv_pc_wiener_disabled = enable_restoration;
    if enable_restoration {
        lr_pc_wiener_disabled = reader.read_bit()? != 0;
        lr_wiener_nonsep_disabled = reader.read_bit()? != 0;
        lr_tools_uv_present = reader.read_bit()? != 0;
        lr_uv_wiener_nonsep_disabled = if lr_tools_uv_present {
            reader.read_bit()? != 0
        } else {
            // lr_tools_disable[1][RESTORE_WIENER_NONSEP] = lr_tools_disable[0][RESTORE_WIENER_NONSEP].
            lr_wiener_nonsep_disabled
        };
    }
    let enable_ccso = reader.read_bit()? != 0;
    let ccso_unit_matches_sb_size = if enable_ccso {
        reader.read_bit()? != 0
    } else {
        false
    };
    let cdef_on_skip_txfm = if single_picture {
        CdefOnSkipTxfm::Adaptive
    } else {
        let cdef_on_skip_txfm_always_on = reader.read_bit()? != 0;
        if cdef_on_skip_txfm_always_on {
            CdefOnSkipTxfm::AlwaysOn
        } else {
            let cdef_on_skip_txfm_disabled = reader.read_bit()? != 0;
            if cdef_on_skip_txfm_disabled {
                CdefOnSkipTxfm::Disabled
            } else {
                CdefOnSkipTxfm::Adaptive
            }
        }
    };
    let df_par_bits_minus_2 = reader.read_bits_u8(2)?;

    Ok(SequenceFilterConfig {
        disable_loopfilters_across_tiles,
        enable_cdef,
        enable_gdf,
        gdf_unit_matches_sb_size,
        enable_restoration,
        lr_pc_wiener_disabled,
        lr_wiener_nonsep_disabled,
        lr_uv_pc_wiener_disabled,
        lr_tools_uv_present,
        lr_uv_wiener_nonsep_disabled,
        enable_ccso,
        ccso_unit_matches_sb_size,
        cdef_on_skip_txfm,
        df_par_bits_minus_2,
    })
}

/// `sequence_tile_config()` (AV2 v1.0.0 § 5.4.2 / § 6.4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceTileConfig {
    /// `seq_tile_info_present_flag`.
    pub seq_tile_info_present_flag: bool,
    /// `allow_tile_info_change`, present when tile info is signalled.
    pub allow_tile_info_change: Option<bool>,
    /// Parsed `tile_params()` (§ 5.18.7.3), present when tile info is signalled and the
    /// sequence level is not reserved.
    pub params: Option<TileParams>,
    /// `SeqSbColStarts[0..SeqTileCols]` (AV2 § 5.4.2: the `sbColStarts` returned by the
    /// `tile_params()` call at mirror
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2` :654-656). Recorded at parse
    /// time so the frame § 5.18.7.4 non-uniform `reuse_tile_params()` branch can rebuild
    /// the layout. Empty when tile info is not signalled or the level is reserved (no
    /// parsed [`Self::params`]). Bounded by `MAX_TILE_COLS`.
    pub seq_sb_col_starts: Vec<u32>,
    /// `SeqSbRowStarts[0..SeqTileRows]` (AV2 § 5.4.2; the `sbRowStarts` companion of
    /// [`Self::seq_sb_col_starts`]). Bounded by `MAX_TILE_ROWS`.
    pub seq_sb_row_starts: Vec<u32>,
}

impl SequenceTileConfig {
    /// Returns the owning Feature ID if tile info is present but `tile_params()` could
    /// not be parsed because `seq_level_idx` is a reserved (non-conformant) level with
    /// no defined tile bit layout. `None` for any valid sequence header.
    #[must_use]
    pub const fn unimplemented_at(&self) -> Option<&'static str> {
        if self.seq_tile_info_present_flag && self.params.is_none() {
            Some("AV2-5.4.2-SEQUENCE-TILE-CONFIG")
        } else {
            None
        }
    }
}

/// Parses `sequence_tile_config()` (AV2 § 5.4.2), including `tile_params()`
/// (§ 5.18.7.3) when `seq_tile_info_present_flag` is set.
///
/// `tile_params_input` carries the frame dimensions, superblock size, tier, and level
/// that `tile_params()` needs; the caller builds it from the parsed general header and
/// partition config (`is_bridge` is `false` at the sequence call site). A reserved
/// `seq_level_idx` has no defined tile bit layout, so the params are left `None` (a
/// bounded residual reported by [`SequenceTileConfig::unimplemented_at`]).
///
/// The `tile_params()` call at AV2 § 5.4.2 returns `SeqSbColStarts` / `SeqSbRowStarts`
/// (mirror `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2` :654-656); these are
/// retained on [`SequenceTileConfig`] for the frame § 5.18.7.4 non-uniform
/// `reuse_tile_params()` branch.
///
/// # Errors
/// Returns descriptor errors or [`Error::UnexpectedEof`] if the payload ends mid-field.
pub fn parse_sequence_tile_config(
    reader: &mut BitReader<'_>,
    tile_params_input: TileParamsInput,
) -> Result<SequenceTileConfig> {
    let seq_tile_info_present_flag = reader.read_bit()? != 0;
    if !seq_tile_info_present_flag {
        return Ok(SequenceTileConfig {
            seq_tile_info_present_flag: false,
            allow_tile_info_change: None,
            params: None,
            seq_sb_col_starts: Vec::new(),
            seq_sb_row_starts: Vec::new(),
        });
    }

    let allow_tile_info_change = reader.read_bit()? != 0;
    // AV2 § 5.4.2: tile_params(max_frame_width_minus_1 + 1, max_frame_height_minus_1 + 1,
    // seqSbSize, seqSbSize, 0). A reserved seq_level_idx has no defined layout, so it is
    // surfaced as a bounded residual rather than a hard parse error. The full layout
    // keeps SeqSbColStarts / SeqSbRowStarts for the § 5.18.7.4 reuse path.
    let (params, seq_sb_col_starts, seq_sb_row_starts) =
        match parse_tile_layout(reader, tile_params_input) {
            Ok(layout) => (
                Some(layout.params),
                layout.sb_col_starts,
                layout.sb_row_starts,
            ),
            Err(Error::Unimplemented { .. }) => (None, Vec::new(), Vec::new()),
            Err(error) => return Err(error),
        };

    Ok(SequenceTileConfig {
        seq_tile_info_present_flag: true,
        allow_tile_info_change: Some(allow_tile_info_change),
        params,
        seq_sb_col_starts,
        seq_sb_row_starts,
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

    let equal_picture_interval = reader.read_bit()? != 0;
    let num_ticks_per_picture_minus_1 = if equal_picture_interval {
        let ticks_offset = reader.byte_offset();
        let ticks_bit_offset = reader.bit_offset();
        let value = reader.read_uvlc()?;
        // AV2 § 6.4.12 bounds num_ticks_per_picture_minus_1 to (1 << 32) - 2. The
        // uvlc() descriptor already caps values at (1 << 32) - 2, so this guard is
        // defensive; it can only fire if the descriptor contract changes.
        if u64::from(value) > (1u64 << 32) - 2 {
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
    let low_delay_mode_flag = reader.read_bit()? != 0;

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
    let seq_cropping_window_present_flag = reader.read_bit()? != 0;
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

/// Parsed § 5.4.1 dependency-map flags plus the two derived maps.
struct DependencyMaps {
    /// `mlayer_dependency_present_flag`, inferred `0` when `max_mlayer_id == 0`.
    mlayer_dependency_present_flag: bool,
    /// `tlayer_dependency_present_flag`, inferred `0` when `max_tlayer_id == 0`.
    tlayer_dependency_present_flag: bool,
    /// `multi_tlayer_dependency_map_present_flag`, inferred `0` when not read.
    multi_tlayer_dependency_map_present_flag: bool,
    /// Derived `MLayerDependencyMap`.
    mlayer_dependency_map: MLayerDependencyMap,
    /// Derived `TLayerDependencyMap`.
    tlayer_dependency_map: TLayerDependencyMap,
}

/// Parses the § 5.4.1 dependency-map region: both maps receive their
/// unconditional default fill first, then the signaled override bits (when
/// present) replace entries following the spec's loop structure exactly.
fn parse_dependency_maps(
    reader: &mut BitReader<'_>,
    max_tlayer_id: TemporalLayerId,
    max_mlayer_id: EmbeddedLayerId,
) -> Result<DependencyMaps> {
    let mut mlayer_dependency_map = MLayerDependencyMap::default_for(max_mlayer_id);
    let mut tlayer_dependency_map = TLayerDependencyMap::default_for(max_tlayer_id, max_mlayer_id);

    let mut mlayer_dependency_present_flag = false;
    if max_mlayer_id.get() > 0 {
        mlayer_dependency_present_flag = reader.read_bit()? != 0;
        if mlayer_dependency_present_flag {
            for curr_layer in 1..=max_mlayer_id.get() {
                // AV2 § 5.4.1: refLayer iterates from currLayer down to 0, so the
                // diagonal bit is signaled first (and may be 0).
                for ref_layer in (0..=curr_layer).rev() {
                    let bit = reader.read_bit()? != 0;
                    mlayer_dependency_map.set(curr_layer, ref_layer, bit);
                }
            }
        }
    }

    let mut tlayer_dependency_present_flag = false;
    let mut multi_tlayer_dependency_map_present_flag = false;
    if max_tlayer_id.get() > 0 {
        tlayer_dependency_present_flag = reader.read_bit()? != 0;
        if tlayer_dependency_present_flag {
            multi_tlayer_dependency_map_present_flag = if max_mlayer_id.get() > 0 {
                reader.read_bit()? != 0
            } else {
                false
            };
            for m_layer in 0..=max_mlayer_id.get() {
                for curr_tlayer in 1..=max_tlayer_id.get() {
                    for ref_tlayer in (0..=curr_tlayer).rev() {
                        if multi_tlayer_dependency_map_present_flag || m_layer == 0 {
                            let bit = reader.read_bit()? != 0;
                            tlayer_dependency_map.set(m_layer, curr_tlayer, ref_tlayer, bit);
                        } else {
                            // AV2 § 5.4.1: with the multi flag clear, embedded
                            // layers 1..=max_mlayer_id copy embedded layer 0's
                            // signaled values (not the default fill).
                            let bit = tlayer_dependency_map.depends_on(
                                EmbeddedLayerId::from_bits(0),
                                TemporalLayerId::from_bits(curr_tlayer),
                                TemporalLayerId::from_bits(ref_tlayer),
                            );
                            tlayer_dependency_map.set(m_layer, curr_tlayer, ref_tlayer, bit);
                        }
                    }
                }
            }
        }
    }

    Ok(DependencyMaps {
        mlayer_dependency_present_flag,
        tlayer_dependency_present_flag,
        multi_tlayer_dependency_map_present_flag,
        mlayer_dependency_map,
        tlayer_dependency_map,
    })
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
        assert!(!header.seq_cropping_window_present_flag);
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
        assert!(!header.seq_cropping_window_present_flag);
        assert_eq!(header.seq_initial_display_delay_minus_1, Some(2));
        assert!(!header.decoder_model_info_present_flag);
        assert_eq!(header.num_units_in_decoding_tick, None);
        assert!(!header.seq_decoder_model_info_present_flag);
        assert!(!header.mlayer_dependency_present_flag);
        assert!(!header.tlayer_dependency_present_flag);
        assert!(!header.multi_tlayer_dependency_map_present_flag);
    }

    /// Appends the general non-single-picture sequence-header fields up to (but
    /// not including) the § 5.4.1 dependency-map region. `max_tlayer_id` /
    /// `max_mlayer_id` select the layer bounds; every other optional feature is
    /// disabled and `seq_max_mlayer_cnt_minus_1` is coded as `max_mlayer_id`.
    fn push_general_header_until_dependency_maps(
        bits: &mut Bits,
        max_tlayer_id: u32,
        max_mlayer_id: u32,
    ) {
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx (<= 3 -> seq_tier inferred Main)
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(max_tlayer_id, 2); // max_tlayer_id
        bits.f(max_mlayer_id, 3); // max_mlayer_id
        if max_mlayer_id > 0 {
            // n = CeilLog2(max_mlayer_id + 1)
            let n = u32::BITS - max_mlayer_id.leading_zeros();
            bits.f(max_mlayer_id, n); // seq_max_mlayer_cnt_minus_1
        }
        bits.bit(0); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
    }

    fn mlayer(value: u8) -> EmbeddedLayerId {
        EmbeddedLayerId::from_bits(value)
    }

    fn tlayer(value: u8) -> TemporalLayerId {
        TemporalLayerId::from_bits(value)
    }

    #[test]
    fn dependency_maps_default_fill_when_flags_absent() {
        let mut bits = Bits::default();
        push_general_header_until_dependency_maps(&mut bits, 2, 1);
        bits.bit(0); // mlayer_dependency_present_flag
        bits.bit(0); // tlayer_dependency_present_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header_general(&mut reader).unwrap();
        assert!(!header.mlayer_dependency_present_flag);
        assert!(!header.tlayer_dependency_present_flag);
        assert!(!header.multi_tlayer_dependency_map_present_flag);

        // § 5.4.1 default: refLayer <= currLayer && currLayer <= max_mlayer_id.
        let m_map = header.mlayer_dependency_map;
        assert!(m_map.depends_on(mlayer(0), mlayer(0)));
        assert!(m_map.depends_on(mlayer(1), mlayer(0)));
        assert!(m_map.depends_on(mlayer(1), mlayer(1)));
        assert!(!m_map.depends_on(mlayer(0), mlayer(1))); // refLayer > currLayer
        assert!(!m_map.depends_on(mlayer(2), mlayer(0))); // currLayer 2 > max_mlayer_id 1
        assert!(!m_map.depends_on(mlayer(2), mlayer(2)));

        // § 5.4.1 default: refTLayer <= currTLayer && currTLayer <= max_tlayer_id
        // && mLayer <= max_mlayer_id.
        let t_map = header.tlayer_dependency_map;
        assert!(t_map.depends_on(mlayer(0), tlayer(2), tlayer(0)));
        assert!(t_map.depends_on(mlayer(1), tlayer(2), tlayer(0)));
        assert!(t_map.depends_on(mlayer(1), tlayer(2), tlayer(2)));
        assert!(!t_map.depends_on(mlayer(0), tlayer(1), tlayer(2))); // refTLayer > currTLayer
        for reference in 0..MAX_NUM_TLAYERS as u8 {
            // currTLayer 3 > max_tlayer_id 2: the whole row is false.
            assert!(!t_map.depends_on(mlayer(0), tlayer(3), tlayer(reference)));
        }
        assert!(!t_map.depends_on(mlayer(2), tlayer(1), tlayer(0))); // mLayer 2 > max_mlayer_id 1
    }

    #[test]
    fn mlayer_dependency_override_bit_order() {
        // max_tlayer_id = 0 so no tlayer_dependency_present_flag bit is read.
        let mut bits = Bits::default();
        push_general_header_until_dependency_maps(&mut bits, 0, 2);
        bits.bit(1); // mlayer_dependency_present_flag
        // § 5.4.1: refLayer runs from currLayer down to 0. The bit patterns are
        // asymmetric so an ascending-order parse would produce different maps.
        bits.bit(0); // currLayer 1: [1][1] (diagonal, signaled, zero here)
        bits.bit(1); // currLayer 1: [1][0]
        bits.bit(1); // currLayer 2: [2][2]
        bits.bit(1); // currLayer 2: [2][1]
        bits.bit(0); // currLayer 2: [2][0]
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header_general(&mut reader).unwrap();
        assert!(header.mlayer_dependency_present_flag);
        assert!(!header.tlayer_dependency_present_flag);

        let m_map = header.mlayer_dependency_map;
        assert!(!m_map.depends_on(mlayer(1), mlayer(1))); // signaled diagonal zero
        assert!(m_map.depends_on(mlayer(1), mlayer(0)));
        assert!(m_map.depends_on(mlayer(2), mlayer(2)));
        assert!(m_map.depends_on(mlayer(2), mlayer(1)));
        assert!(!m_map.depends_on(mlayer(2), mlayer(0))); // proves descending order
        assert!(m_map.depends_on(mlayer(0), mlayer(0))); // row 0 keeps the default
        // max_tlayer_id = 0: the tlayer map keeps its default fill.
        assert!(
            header
                .tlayer_dependency_map
                .depends_on(mlayer(2), tlayer(0), tlayer(0))
        );
    }

    #[test]
    fn tlayer_dependency_row0_replication() {
        let mut bits = Bits::default();
        push_general_header_until_dependency_maps(&mut bits, 1, 1);
        bits.bit(0); // mlayer_dependency_present_flag
        bits.bit(1); // tlayer_dependency_present_flag
        bits.bit(0); // multi_tlayer_dependency_map_present_flag
        // Bits are signaled only for mLayer 0; refTLayer descends from currTLayer.
        bits.bit(1); // mLayer 0, currTLayer 1: [0][1][1]
        bits.bit(0); // mLayer 0, currTLayer 1: [0][1][0] (default would be 1)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header_general(&mut reader).unwrap();
        assert!(header.tlayer_dependency_present_flag);
        assert!(!header.multi_tlayer_dependency_map_present_flag);

        let t_map = header.tlayer_dependency_map;
        assert!(t_map.depends_on(mlayer(0), tlayer(1), tlayer(1)));
        assert!(!t_map.depends_on(mlayer(0), tlayer(1), tlayer(0)));
        // § 5.4.1: mLayer 1 copies mLayer 0's signaled values, not the defaults
        // (the default for [1][1][0] would be true).
        assert!(t_map.depends_on(mlayer(1), tlayer(1), tlayer(1)));
        assert!(!t_map.depends_on(mlayer(1), tlayer(1), tlayer(0)));
        // mLayer 2 > max_mlayer_id stays at the (all-false) default.
        assert!(!t_map.depends_on(mlayer(2), tlayer(1), tlayer(1)));
    }

    #[test]
    fn tlayer_dependency_multi_rows_signaled() {
        let mut bits = Bits::default();
        push_general_header_until_dependency_maps(&mut bits, 1, 1);
        bits.bit(0); // mlayer_dependency_present_flag
        bits.bit(1); // tlayer_dependency_present_flag
        bits.bit(1); // multi_tlayer_dependency_map_present_flag
        // Distinct per-mLayer rows prove each embedded layer is signaled.
        bits.bit(1); // mLayer 0, currTLayer 1: [0][1][1]
        bits.bit(0); // mLayer 0, currTLayer 1: [0][1][0]
        bits.bit(0); // mLayer 1, currTLayer 1: [1][1][1]
        bits.bit(1); // mLayer 1, currTLayer 1: [1][1][0]
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header_general(&mut reader).unwrap();
        assert!(header.tlayer_dependency_present_flag);
        assert!(header.multi_tlayer_dependency_map_present_flag);

        let t_map = header.tlayer_dependency_map;
        assert!(t_map.depends_on(mlayer(0), tlayer(1), tlayer(1)));
        assert!(!t_map.depends_on(mlayer(0), tlayer(1), tlayer(0)));
        assert!(!t_map.depends_on(mlayer(1), tlayer(1), tlayer(1)));
        assert!(t_map.depends_on(mlayer(1), tlayer(1), tlayer(0)));
    }

    #[test]
    fn single_picture_header_collapses_maps() {
        let data = valid_single_picture_prefix();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header_general(&mut reader).unwrap();
        assert!(!header.mlayer_dependency_present_flag);
        assert!(!header.tlayer_dependency_present_flag);
        assert!(!header.multi_tlayer_dependency_map_present_flag);
        // max_tlayer_id = max_mlayer_id = 0: only the [0][0] / [0][0][0] entries
        // of the § 5.4.1 default fill survive the clipping.
        let m_map = header.mlayer_dependency_map;
        assert!(m_map.depends_on(mlayer(0), mlayer(0)));
        assert!(!m_map.depends_on(mlayer(1), mlayer(0)));
        assert!(!m_map.depends_on(mlayer(1), mlayer(1)));
        let t_map = header.tlayer_dependency_map;
        assert!(t_map.depends_on(mlayer(0), tlayer(0), tlayer(0)));
        assert!(!t_map.depends_on(mlayer(0), tlayer(1), tlayer(0)));
        assert!(!t_map.depends_on(mlayer(1), tlayer(0), tlayer(0)));
    }

    #[test]
    fn dependency_map_truncation_reports_eof() {
        // End the stream right after multi_tlayer_dependency_map_present_flag:
        // with max ids 3/7 the 72 signaled tlayer bits overrun the zero-bit byte
        // padding, so the parser must report EOF (never panic).
        let mut bits = Bits::default();
        push_general_header_until_dependency_maps(&mut bits, 3, 7);
        bits.bit(0); // mlayer_dependency_present_flag
        bits.bit(1); // tlayer_dependency_present_flag
        bits.bit(1); // multi_tlayer_dependency_map_present_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_sequence_header_general(&mut reader),
            Err(Error::UnexpectedEof { .. })
        ));
        // Every shorter prefix truncates earlier in the header and must also
        // fail without panicking.
        for len in 0..data.len() {
            let mut reader = BitReader::new(&data[..len], ByteOffset::new(0));
            assert!(parse_sequence_header_general(&mut reader).is_err());
        }
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

    /// Appends a complete, minimal still-picture `sequence_header_obu()` (general
    /// fields through `film_grain_params_present`) with chroma format 4:2:0 (not
    /// monochrome). All tool flags are `0` except where a fixed value is required.
    fn push_still_picture_header(bits: &mut Bits) {
        push_still_picture_header_until_tile(bits, 0, false);
        // sequence_tile_config
        bits.bit(0); // seq_tile_info_present_flag (fully parsed)
        // film_grain_params_present
        bits.bit(0);
    }

    #[test]
    fn parses_full_still_picture_sequence_header() {
        let mut bits = Bits::default();
        push_still_picture_header(&mut bits);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header(&mut reader).unwrap();
        assert!(header.is_fully_parsed());
        assert_eq!(header.unimplemented_at, None);
        assert_eq!(header.film_grain_params_present, Some(false));
        let partition = header.partition.unwrap();
        assert_eq!(partition.seq_sb_size(), SuperblockSize::Block64x64);
        assert_eq!(partition.max_pb_aspect_ratio, 8);
        let inter = header.inter.unwrap();
        assert_eq!(inter.drl_reorder, DrlReorder::Disabled);
        assert_eq!(inter.num_ref_frames, 2);
        assert_eq!(inter.order_hint_bits, 0);
        let scc = header.screen_content.unwrap();
        assert_eq!(scc.seq_force_screen_content_tools, 2);
        assert_eq!(scc.seq_force_integer_mv, 2);
        assert!(header.tile.unwrap().allow_tile_info_change.is_none());
    }

    #[test]
    fn sequence_partition_config_reads_inferred_values() {
        let mut bits = Bits::default();
        bits.bit(1); // use_256x256_superblock (use_128x128 not read)
        bits.bit(1); // enable_sdp (not monochrome)
        bits.bit(1); // enable_extended_sdp (sdp && !single picture)
        bits.bit(1); // enable_ext_partitions
        bits.bit(1); // enable_uneven_4way_partitions
        bits.bit(1); // reduce_pb_aspect_ratio
        bits.bit(0); // max_pb_aspect_ratio_log2_minus_1 -> MaxPbAspectRatio = 2
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let partition = parse_sequence_partition_config(&mut reader, false, false).unwrap();
        assert!(partition.use_256x256_superblock);
        assert!(!partition.use_128x128_superblock);
        assert_eq!(partition.seq_sb_size(), SuperblockSize::Block256x256);
        assert!(partition.enable_extended_sdp);
        assert!(partition.enable_uneven_4way_partitions);
        assert_eq!(partition.max_pb_aspect_ratio, 2);
    }

    #[test]
    fn sequence_partition_config_infers_128x128_superblock() {
        let mut bits = Bits::default();
        bits.bit(0); // use_256x256_superblock
        bits.bit(1); // use_128x128_superblock
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio -> MaxPbAspectRatio = 8
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let partition = parse_sequence_partition_config(&mut reader, false, false).unwrap();
        assert_eq!(partition.seq_sb_size(), SuperblockSize::Block128x128);
        assert_eq!(partition.max_pb_aspect_ratio, 8);
    }

    #[test]
    fn sequence_intra_config_infers_cfl_filter_for_monochrome() {
        let mut bits = Bits::default();
        // Seven flags, no cfl_ds_filter_index because monochrome.
        for _ in 0..4 {
            bits.bit(0);
        }
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let intra = parse_sequence_intra_config(&mut reader, true).unwrap();
        assert_eq!(intra.cfl_ds_filter_index, 0);
        // Exactly six bits consumed (no 2-bit cfl_ds_filter_index).
        assert_eq!(reader.bit_offset().get(), 6);
    }

    #[test]
    fn sequence_intra_config_reads_cfl_filter_when_chroma_present() {
        let mut bits = Bits::default();
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(1); // enable_cfl_intra
        bits.f(2, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let intra = parse_sequence_intra_config(&mut reader, false).unwrap();
        assert!(intra.enable_cfl_intra);
        assert_eq!(intra.cfl_ds_filter_index, 2);
    }

    #[test]
    fn sequence_inter_config_still_picture_branch_has_no_order_hints() {
        let mut bits = Bits::default();
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.bit(0); // enable_bawp
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let inter = parse_sequence_inter_config(&mut reader, true).unwrap();
        assert_eq!(inter.order_hint_bits, 0);
        assert_eq!(inter.num_ref_frames, 2);
        assert_eq!(inter.drl_reorder, DrlReorder::Disabled);
        assert!(inter.seq_enabled_motion_modes.iter().all(|&m| !m));
    }

    #[test]
    fn sequence_scc_config_single_picture_uses_select_values() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let scc = parse_sequence_scc_config(&mut reader, true).unwrap();
        assert_eq!(scc.seq_force_screen_content_tools, 2);
        assert_eq!(scc.seq_force_integer_mv, 2);
    }

    #[test]
    fn sequence_filter_config_reads_tool_flags_without_filtering() {
        let mut bits = Bits::default();
        bits.bit(1); // disable_loopfilters_across_tiles
        bits.bit(1); // enable_cdef
        bits.bit(0); // enable_gdf (no gdf_unit_matches_sb_size since BLOCK_64X64 only matters with gdf)
        bits.bit(1); // enable_restoration
        bits.bit(1); // lr_tools_disable[0][RESTORE_PC_WIENER]
        bits.bit(0); // lr_tools_disable[0][RESTORE_WIENER_NONSEP]
        bits.bit(1); // lr_tools_uv_present
        bits.bit(1); // lr_tools_disable[1][RESTORE_WIENER_NONSEP]
        bits.bit(0); // enable_ccso
        bits.f(2, 2); // df_par_bits_minus_2
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let filter =
            parse_sequence_filter_config(&mut reader, true, SuperblockSize::Block128x128).unwrap();
        assert!(filter.disable_loopfilters_across_tiles);
        assert!(filter.enable_cdef);
        assert!(filter.enable_restoration);
        assert!(filter.lr_pc_wiener_disabled);
        assert!(filter.lr_tools_uv_present);
        assert!(filter.lr_uv_wiener_nonsep_disabled);
        // Inferred lr_tools_disable[1][RESTORE_PC_WIENER] = 1 when restoration is on.
        assert!(filter.lr_uv_pc_wiener_disabled);
        assert_eq!(filter.cdef_on_skip_txfm, CdefOnSkipTxfm::Adaptive);
        assert_eq!(filter.df_par_bits_minus_2, 2);
    }

    #[test]
    fn sequence_filter_config_infers_no_uv_pc_wiener_without_restoration() {
        let mut bits = Bits::default();
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration -> restoration block skipped
        bits.bit(0); // enable_ccso
        bits.f(0, 2); // df_par_bits_minus_2
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let filter =
            parse_sequence_filter_config(&mut reader, true, SuperblockSize::Block64x64).unwrap();
        assert!(!filter.enable_restoration);
        assert!(!filter.lr_uv_pc_wiener_disabled);
    }

    #[test]
    fn sequence_tq_config_mirrors_uv_dc_delta_when_equal() {
        let mut bits = Bits::default();
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly (not monochrome)
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q
        bits.f(19, 5); // base_uv_ac_delta_q
        bits.bit(1); // uv_ac_delta_q_enabled
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let tq = parse_sequence_transform_quant_entropy_config(&mut reader, false, true).unwrap();
        assert!(tq.equal_ac_dc_q);
        assert_eq!(tq.base_uv_ac_delta_q, 19);
        // AV2 § 5.4.8: BaseUVDcDeltaQ = BaseUVAcDeltaQ when equal_ac_dc_q.
        assert_eq!(tq.base_uv_dc_delta_q, 19);
        assert!(!tq.uv_dc_delta_q_enabled);
    }

    #[test]
    fn sequence_filter_config_reads_gdf_unit_flag_for_64x64() {
        let mut bits = Bits::default();
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(1); // enable_gdf
        bits.bit(1); // gdf_unit_matches_sb_size (because seqSbSize == BLOCK_64X64)
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.bit(0); // cdef_on_skip_txfm_always_on
        bits.bit(0); // cdef_on_skip_txfm_disabled -> Adaptive
        bits.f(0, 2); // df_par_bits_minus_2
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let filter =
            parse_sequence_filter_config(&mut reader, false, SuperblockSize::Block64x64).unwrap();
        assert!(filter.enable_gdf);
        assert!(filter.gdf_unit_matches_sb_size);
        assert_eq!(filter.cdef_on_skip_txfm, CdefOnSkipTxfm::Adaptive);
    }

    #[test]
    fn sequence_timing_rejects_zero_display_tick() {
        let mut bits = Bits::default();
        bits.f(0, 32); // num_units_in_display_tick = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_timing_info(&mut reader),
            Err(Error::InvalidSequenceHeader {
                kind: SequenceHeaderErrorKind::TimingDisplayTickZero,
                ..
            })
        ));
    }

    #[test]
    fn sequence_timing_rejects_zero_time_scale() {
        let mut bits = Bits::default();
        bits.f(1, 32); // num_units_in_display_tick
        bits.f(0, 32); // time_scale = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_timing_info(&mut reader),
            Err(Error::InvalidSequenceHeader {
                kind: SequenceHeaderErrorKind::TimingTimeScaleZero,
                ..
            })
        ));
    }

    #[test]
    fn sequence_timing_parses_equal_picture_interval() {
        let mut bits = Bits::default();
        bits.f(1000, 32); // num_units_in_display_tick
        bits.f(60000, 32); // time_scale
        bits.bit(1); // equal_picture_interval
        bits.uvlc(5); // num_ticks_per_picture_minus_1
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let timing = parse_timing_info(&mut reader).unwrap();
        assert_eq!(timing.num_units_in_display_tick, 1000);
        assert_eq!(timing.time_scale, 60000);
        assert!(timing.equal_picture_interval);
        assert_eq!(timing.num_ticks_per_picture_minus_1, Some(5));
    }

    #[test]
    fn sequence_decoder_model_info_parses_delays() {
        let mut bits = Bits::default();
        bits.uvlc(7); // decoder_buffer_delay
        bits.uvlc(9); // encoder_buffer_delay
        bits.bit(1); // low_delay_mode_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_sequence_decoder_model_info(&mut reader).unwrap();
        assert_eq!(info.decoder_buffer_delay, 7);
        assert_eq!(info.encoder_buffer_delay, 9);
        assert!(info.low_delay_mode_flag);
    }

    #[test]
    fn sequence_segment_config_parses_seg_info() {
        let mut bits = Bits::default();
        bits.bit(0); // enable_ext_seg -> MaxSegments = 8
        bits.bit(1); // seq_seg_info_present_flag
        bits.bit(0); // seq_allow_seg_info_change
        for _ in 0..(8 * crate::segment::SEG_LVL_MAX) {
            bits.bit(0); // seg_info(8): all features disabled
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let segment = parse_sequence_segment_config(&mut reader).unwrap();
        assert_eq!(segment.max_segments, 8);
        assert!(segment.seq_seg_info_present_flag);
        assert_eq!(segment.seq_allow_seg_info_change, Some(false));
        let info = segment
            .segment_info
            .expect("segment info is parsed when present");
        assert_eq!(info.num_segments, 8);
    }

    #[test]
    fn sequence_segment_config_absent_has_no_segment_info() {
        let mut bits = Bits::default();
        bits.bit(1); // enable_ext_seg -> MaxSegments = 16
        bits.bit(0); // seq_seg_info_present_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let segment = parse_sequence_segment_config(&mut reader).unwrap();
        assert_eq!(segment.max_segments, 16);
        assert!(!segment.seq_seg_info_present_flag);
        assert_eq!(segment.segment_info, None);
    }

    #[test]
    fn sequence_header_composite_parses_segment_info() {
        // A header with seq_seg_info_present_flag = 1 now parses fully (no bound).
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 0, true);
        bits.bit(0); // seq_tile_info_present_flag (tile absent)
        bits.bit(0); // film_grain_params_present
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header(&mut reader).unwrap();
        assert!(header.is_fully_parsed());
        assert_eq!(header.unimplemented_at, None);
        let segment = header.segment.unwrap();
        assert!(segment.seq_seg_info_present_flag);
        let info = segment.segment_info.expect("segment info present");
        assert_eq!(info.num_segments, 8);
        assert_eq!(header.film_grain_params_present, Some(false));
    }

    #[test]
    fn sequence_header_composite_parses_tile_params() {
        // A header with seq_tile_info_present_flag = 1 now parses tile_params fully.
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 0, false);
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        bits.bit(1); // uniform_tile_spacing_flag (16x8 frame -> single tile, no increments)
        bits.bit(0); // film_grain_params_present
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header(&mut reader).unwrap();
        assert!(header.is_fully_parsed());
        assert_eq!(header.unimplemented_at, None);
        let tile = header.tile.unwrap();
        assert!(tile.seq_tile_info_present_flag);
        assert_eq!(tile.allow_tile_info_change, Some(false));
        let params = tile.params.expect("tile params parsed for a valid level");
        assert!(params.uniform_spacing);
        assert_eq!(params.tile_cols, 1);
        assert_eq!(params.tile_rows, 1);
        // § 5.4.2 records SeqSbColStarts / SeqSbRowStarts; a single-tile layout starts
        // at superblock 0 on both axes.
        assert_eq!(tile.seq_sb_col_starts, vec![0]);
        assert_eq!(tile.seq_sb_row_starts, vec![0]);
        assert_eq!(header.film_grain_params_present, Some(false));
    }

    #[test]
    fn sequence_tile_config_records_non_uniform_start_arrays() {
        // § 5.4.2 tile_params(maxFrameWidth, maxFrameHeight, seqSbSize, seqSbSize, 0)
        // on a 128x8 grid (BLOCK_64X64 -> sbCols = 2, sbRows = 1), non-uniform: two
        // 1-superblock columns. The recorded SeqSbColStarts / SeqSbRowStarts are the
        // ones the § 5.18.7.4 non-uniform reuse branch consumes.
        let input = TileParamsInput {
            frame_width: 128,
            frame_height: 8,
            uniform_sb_size: SuperblockSize::Block64x64,
            sb_size: SuperblockSize::Block64x64,
            is_bridge: false,
            seq_tier: Tier::Main,
            seq_level_idx: LevelIdx::from_bits(0),
        };
        let mut bits = Bits::default();
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(1); // allow_tile_info_change
        bits.bit(0); // uniform_tile_spacing_flag = 0
        bits.bit(0); // ns(2) width_in_sbs_minus_1 = 0 -> first column 1 sb wide
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let tile = parse_sequence_tile_config(&mut reader, input).unwrap();
        assert!(tile.seq_tile_info_present_flag);
        assert_eq!(tile.allow_tile_info_change, Some(true));
        let params = tile.params.expect("non-reserved level parses tile params");
        assert!(!params.uniform_spacing);
        assert_eq!(params.tile_cols, 2);
        assert_eq!(params.tile_rows, 1);
        assert_eq!(params.sb_cols, 2);
        assert_eq!(params.sb_rows, 1);
        // Two 1-superblock columns -> starts [0, 1]; the single row starts at 0.
        assert_eq!(tile.seq_sb_col_starts, vec![0, 1]);
        assert_eq!(tile.seq_sb_row_starts, vec![0]);
    }

    #[test]
    fn sequence_tile_config_reserved_level_records_empty_start_arrays() {
        // A reserved seq_level_idx has no defined tile bit layout; params is None and the
        // start arrays stay empty (the bounded residual).
        let input = TileParamsInput {
            frame_width: 128,
            frame_height: 8,
            uniform_sb_size: SuperblockSize::Block64x64,
            sb_size: SuperblockSize::Block64x64,
            is_bridge: false,
            seq_tier: Tier::Main,
            seq_level_idx: LevelIdx::from_bits(22),
        };
        let mut bits = Bits::default();
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let tile = parse_sequence_tile_config(&mut reader, input).unwrap();
        assert!(tile.params.is_none());
        assert!(tile.seq_sb_col_starts.is_empty());
        assert!(tile.seq_sb_row_starts.is_empty());
        assert_eq!(
            tile.unimplemented_at(),
            Some("AV2-5.4.2-SEQUENCE-TILE-CONFIG")
        );
    }

    #[test]
    fn sequence_header_composite_bounds_at_reserved_level_tile_params() {
        // seq_level_idx 22 is a reserved level with no defined tile bit layout, so a
        // seq_tile_info_present header bounds at tile_params (the only residual bound).
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 22, false);
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let header = parse_sequence_header(&mut reader).unwrap();
        assert_eq!(
            header.unimplemented_at,
            Some("AV2-5.4.2-SEQUENCE-TILE-CONFIG")
        );
        assert!(header.filter.is_some());
        assert!(header.tile.unwrap().params.is_none());
        assert_eq!(header.film_grain_params_present, None);
    }

    /// Appends a still-picture `sequence_header_obu()` up to (but not including)
    /// `sequence_tile_config()`. Mirrors the parser field-for-field. `seq_level_idx`
    /// selects the level (single-picture headers never code `seq_tier`, so the level is
    /// free to vary). When `segment_info_present`, an all-disabled `seg_info(8)` is
    /// appended after the segment-config flags.
    fn push_still_picture_header_until_tile(
        bits: &mut Bits,
        seq_level_idx: u32,
        segment_info_present: bool,
    ) {
        // general (single_picture_header_flag = 1, chroma 4:2:0)
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(seq_level_idx, 5); // seq_level_idx (single picture -> no seq_tier)
        bits.uvlc(0); // chroma_format_idc = CHROMA_FORMAT_420
        bits.uvlc(0); // bit_depth_idc
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        // sequence_partition_config (not monochrome, single picture)
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock -> seqSbSize = BLOCK_64X64
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        // sequence_segment_config
        bits.bit(0); // enable_ext_seg -> MaxSegments = 8
        bits.bit(u8::from(segment_info_present)); // seq_seg_info_present_flag
        if segment_info_present {
            bits.bit(0); // seq_allow_seg_info_change
            for _ in 0..(8 * crate::segment::SEG_LVL_MAX) {
                bits.bit(0); // seg_info(8): 8 segments x SEG_LVL_MAX features, all disabled
            }
        }
        // sequence_intra_config (not monochrome)
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        bits.f(0, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        // sequence_inter_config (single_picture_header_flag branch)
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder -> DRL_REORDER_DISABLED
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.bit(0); // enable_bawp
        // sequence_scc_config (single picture -> no signalled bits)
        // sequence_transform_quant_entropy_config (not monochrome, single picture)
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q -> skip y/uv dc delta reads
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        // sequence_filter_config (single picture, seqSbSize = BLOCK_64X64)
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.f(0, 2); // df_par_bits_minus_2
    }

    #[test]
    fn sequence_header_child_payload_eof_never_panics() {
        // Truncate the full still-picture header at every byte boundary; parsing
        // must return an error, never panic.
        let mut bits = Bits::default();
        push_still_picture_header(&mut bits);
        let full = bits.into_bytes();
        for len in 0..full.len() {
            let mut reader = BitReader::new(&full[..len], ByteOffset::new(0));
            let _ = parse_sequence_header(&mut reader);
        }
    }

    #[test]
    fn dispatch_round_trips_full_sequence_header_with_trailing_bits() {
        use crate::obu::{
            ParsedObu, PayloadStatus, dispatch_obu_payload, read_obu_header_from_slice,
        };
        let mut bits = Bits::default();
        push_still_picture_header(&mut bits);
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(1); // trailing_one_bit
        let payload = bits.into_bytes();
        let header = read_obu_header_from_slice(&[0x04], ByteOffset::new(0)).unwrap();
        let status = dispatch_obu_payload(header, &payload, ByteOffset::new(1)).unwrap();
        assert!(matches!(
            status,
            PayloadStatus::Parsed(ParsedObu::SequenceHeader(ref h)) if h.is_fully_parsed()
        ));
    }

    #[test]
    fn dispatch_rejects_sequence_header_nonzero_obu_extension_flag() {
        use crate::obu::{dispatch_obu_payload, read_obu_header_from_slice};
        let mut bits = Bits::default();
        push_still_picture_header(&mut bits);
        bits.bit(1); // obu_extension_flag = 1 -> conformance violation (AV2 § 6.2.1)
        bits.bit(1); // trailing_one_bit (would be valid, but the flag is already bad)
        let payload = bits.into_bytes();
        let header = read_obu_header_from_slice(&[0x04], ByteOffset::new(0)).unwrap();
        assert!(matches!(
            dispatch_obu_payload(header, &payload, ByteOffset::new(1)),
            Err(Error::InvalidObuExtension { .. })
        ));
    }

    #[test]
    fn dispatch_rejects_sequence_header_bad_trailing_bits() {
        use crate::error::TrailingBitsErrorKind;
        use crate::obu::{dispatch_obu_payload, read_obu_header_from_slice};
        let mut bits = Bits::default();
        push_still_picture_header(&mut bits);
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(0); // malformed trailing_one_bit (must be 1)
        let payload = bits.into_bytes();
        let header = read_obu_header_from_slice(&[0x04], ByteOffset::new(0)).unwrap();
        assert!(matches!(
            dispatch_obu_payload(header, &payload, ByteOffset::new(1)),
            Err(Error::InvalidTrailingBits {
                kind: TrailingBitsErrorKind::MissingOneBit,
                ..
            })
        ));
    }

    #[test]
    fn dispatch_reports_bounded_sequence_header_as_unimplemented() {
        use crate::obu::{PayloadStatus, dispatch_obu_payload, read_obu_header_from_slice};
        // A reserved seq_level_idx (22) with tile info present is the only residual
        // bounded sequence-header case: tile_params has no defined layout for it.
        let mut bits = Bits::default();
        push_still_picture_header_until_tile(&mut bits, 22, false);
        bits.bit(1); // seq_tile_info_present_flag -> bounded at tile_params (reserved level)
        bits.bit(0); // allow_tile_info_change
        let payload = bits.into_bytes();
        let header = read_obu_header_from_slice(&[0x04], ByteOffset::new(0)).unwrap();
        assert!(matches!(
            dispatch_obu_payload(header, &payload, ByteOffset::new(1)),
            Ok(PayloadStatus::Unimplemented {
                feature: "AV2-5.4.2-SEQUENCE-TILE-CONFIG",
                ..
            })
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

        /// The full `sequence_header_obu()` walk (general + all child configs) must
        /// never panic on arbitrary input (CLAUDE.md § 8 no-panic requirement).
        #[test]
        fn parse_sequence_header_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_sequence_header(&mut reader);
        }

        /// `timing_info()` parsing must never panic on arbitrary input.
        #[test]
        fn parse_timing_info_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_timing_info(&mut reader);
        }
    }
}
