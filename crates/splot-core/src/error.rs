// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Typed error model for `splot-core`.
//!
//! Library code never panics on malformed input; every failure is one of these
//! variants. Recognized-but-unmodeled functionality returns
//! [`Error::Unimplemented`] rather than `todo!()`/`unimplemented!()`.

use core::fmt;

use thiserror::Error;

use crate::span::{BitOffset, ByteOffset};

/// Specific ways `trailing_bits(nbBits)` can violate AV2 § 6.2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailingBitsErrorKind {
    /// `trailing_bits` was asked to parse zero bits.
    Empty,
    /// The required `trailing_one_bit` was not equal to `1`.
    MissingOneBit,
    /// A `trailing_zero_bit` was not equal to `0`.
    ZeroBitNotZero,
}

impl fmt::Display for TrailingBitsErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "nbBits must be greater than zero",
            Self::MissingOneBit => "trailing_one_bit must be equal to 1",
            Self::ZeroBitNotZero => "trailing_zero_bit must be equal to 0",
        };
        f.write_str(message)
    }
}

/// Specific ways `byte_alignment()` can violate AV2 § 6.2.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteAlignmentErrorKind {
    /// A byte-alignment `zero_bit` was not equal to `0`.
    ZeroBitNotZero,
}

impl fmt::Display for ByteAlignmentErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ZeroBitNotZero => "zero_bit must be equal to 0",
        };
        f.write_str(message)
    }
}

/// Specific locally decidable `sequence_header_obu()` violations from AV2 § 6.4.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceHeaderErrorKind {
    /// `seq_header_id` is not less than `MAX_SEQ_NUM`.
    SeqHeaderIdOutOfRange,
    /// `chroma_format_idc` is not in Table 6.2.
    ChromaFormatOutOfRange,
    /// `bit_depth_idc` is not in Table 6.3.
    BitDepthOutOfRange,
    /// `seq_max_mlayer_cnt_minus_1` is greater than `max_mlayer_id`.
    SeqMaxMlayerCountOutOfRange,
    /// `seq_cropping_win_left_offset` is greater than `max_frame_width_minus_1`.
    CropLeftOutOfRange,
    /// `seq_cropping_win_right_offset` is greater than `max_frame_width_minus_1`.
    CropRightOutOfRange,
    /// `seq_cropping_win_top_offset` is greater than `max_frame_height_minus_1`.
    CropTopOutOfRange,
    /// `seq_cropping_win_bottom_offset` is greater than `max_frame_height_minus_1`.
    CropBottomOutOfRange,
    /// `num_units_in_decoding_tick` is zero.
    TimingNumUnitsZero,
    /// `num_units_in_display_tick` is zero (AV2 § 6.4.12).
    TimingDisplayTickZero,
    /// `time_scale` is zero (AV2 § 6.4.12).
    TimingTimeScaleZero,
    /// `num_ticks_per_picture_minus_1` exceeds `(1 << 32) - 2` (AV2 § 6.4.12).
    TimingNumTicksOutOfRange,
}

impl fmt::Display for SequenceHeaderErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::SeqHeaderIdOutOfRange => "seq_header_id must be less than MAX_SEQ_NUM",
            Self::ChromaFormatOutOfRange => "chroma_format_idc must be less than or equal to 3",
            Self::BitDepthOutOfRange => "bit_depth_idc must be less than or equal to 1",
            Self::SeqMaxMlayerCountOutOfRange => {
                "seq_max_mlayer_cnt_minus_1 must be less than or equal to max_mlayer_id"
            }
            Self::CropLeftOutOfRange => {
                "seq_cropping_win_left_offset must be less than or equal to max_frame_width_minus_1"
            }
            Self::CropRightOutOfRange => {
                "seq_cropping_win_right_offset must be less than or equal to max_frame_width_minus_1"
            }
            Self::CropTopOutOfRange => {
                "seq_cropping_win_top_offset must be less than or equal to max_frame_height_minus_1"
            }
            Self::CropBottomOutOfRange => {
                "seq_cropping_win_bottom_offset must be less than or equal to max_frame_height_minus_1"
            }
            Self::TimingNumUnitsZero => "num_units_in_decoding_tick must be greater than 0",
            Self::TimingDisplayTickZero => "num_units_in_display_tick must be greater than 0",
            Self::TimingTimeScaleZero => "time_scale must be greater than 0",
            Self::TimingNumTicksOutOfRange => {
                "num_ticks_per_picture_minus_1 must not exceed (1 << 32) - 2"
            }
        };
        f.write_str(message)
    }
}

/// Specific structural violations of `layer_config_record_obu()` (AV2 § 5.8 / § 6.8)
/// that prevent further parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerConfigRecordErrorKind {
    /// The bits parsed for `lcr_global_payload(n, sz)` exceeded the declared
    /// `sz * 8` payload bits (AV2 § 5.8.5: `RemainingLcrPayloadBits` would be
    /// negative).
    PayloadSizeOverflow,
}

impl fmt::Display for LayerConfigRecordErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::PayloadSizeOverflow => {
                "lcr_global_payload parsed content exceeds the declared lcr_data_size * 8 bits"
            }
        };
        f.write_str(message)
    }
}

/// Specific structural violations of `atlas_segment_info_obu()` (AV2 § 5.9 / § 6.9)
/// that prevent further parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtlasSegmentErrorKind {
    /// `ats_atlas_segment_mode_idc` is greater than 4 (AV2 § 6.9, Table 6.11): no
    /// per-mode syntax is defined, so parsing cannot continue.
    ModeOutOfRange,
    /// A region-grid dimension (`ats_num_region_columns_minus_1` /
    /// `ats_num_region_rows_minus_1`) reaches `MAX_ATLAS_COLS` / `MAX_ATLAS_ROWS`
    /// (AV2 § 6.9.3.1), which would drive an out-of-range loop.
    RegionDimensionOutOfRange,
    /// A segment count (`numSegments` / `ats_num_atlas_segments_minus_1` /
    /// `ats_msi_num_atlas_segments_minus_1`) reaches `MAX_NUM_ATLAS_SEGMENTS`
    /// (AV2 § 6.9.6), which would drive an out-of-range loop.
    SegmentCountOutOfRange,
}

impl fmt::Display for AtlasSegmentErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ModeOutOfRange => "ats_atlas_segment_mode_idc must be less than or equal to 4",
            Self::RegionDimensionOutOfRange => {
                "atlas region columns/rows must be less than MAX_ATLAS_COLS / MAX_ATLAS_ROWS"
            }
            Self::SegmentCountOutOfRange => {
                "atlas segment count must be less than or equal to MAX_NUM_ATLAS_SEGMENTS"
            }
        };
        f.write_str(message)
    }
}

/// Specific structural violations of `padding_obu()` (AV2 § 5.16 / § 6.15) that
/// prevent further parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaddingErrorKind {
    /// A non-empty padding payload is entirely zero. AV2 § 5.16 / § 6.15 require at
    /// least one non-zero byte (the `trailing_bits()` byte) when any payload is present.
    AllZeroPayload,
    /// The bytes from the last non-zero payload byte through the payload end are not a
    /// valid `trailing_bits()` pattern (AV2 § 5.2.3 / § 6.2.3).
    InvalidTrailingBits,
}

impl fmt::Display for PaddingErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::AllZeroPayload => {
                "a non-empty padding OBU payload must contain at least one non-zero byte"
            }
            Self::InvalidTrailingBits => {
                "padding OBU trailing_bits() must start with trailing_one_bit followed by zeros"
            }
        };
        f.write_str(message)
    }
}

/// Locally decidable violations of AV2 § 6.17.9 global-motion state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalMotionErrorKind {
    /// `NumTotalRefs` exceeds `REFS_PER_FRAME` or the supplied map length.
    ReferenceCountOutOfRange,
    /// A logical reference maps outside the modeled reference-frame buffer.
    ReferenceSlotOutOfRange,
    /// `our_ref != NumTotalRefs` selected a restricted reference (§ 6.17.9.1).
    OurReferenceRestricted,
    /// The selected saved `their_ref` is restricted (§ 6.17.9.1).
    SavedReferenceRestricted,
    /// An order hint does not fit the signed AV2 derivation domain.
    OrderHintOutOfRange,
}

impl fmt::Display for GlobalMotionErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ReferenceCountOutOfRange => {
                "global-motion reference count must fit REFS_PER_FRAME and ref_frame_idx"
            }
            Self::ReferenceSlotOutOfRange => {
                "global-motion reference slot must exist in the modeled reference state"
            }
            Self::OurReferenceRestricted => "OrderHints[our_ref] must not equal RESTRICTED_OH",
            Self::SavedReferenceRestricted => {
                "SavedOrderHints[refIdx][their_ref] must not equal RESTRICTED_OH"
            }
            Self::OrderHintOutOfRange => {
                "global-motion order hints must fit the AV2 signed derivation domain"
            }
        };
        f.write_str(message)
    }
}

/// Specific caller-supplied CDF row violations for AV2 § 8.2.6 symbol decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolCdfErrorKind {
    /// The row length is not `N + 1` for a supported `N` in `2..=8`.
    UnsupportedLength {
        /// Actual row length.
        len: usize,
    },
    /// A cumulative probability entry is outside the supported AV2 coding range.
    ProbabilityOutOfRange {
        /// Offending CDF index.
        index: usize,
        /// Offending value.
        value: i32,
    },
    /// A cumulative probability entry is smaller than its predecessor. AV2
    /// § 8.2.6 adaptation can drive adjacent entries equal, so equal adjacent
    /// entries are accepted and only a strict decrease is rejected.
    DecreasingCumulative {
        /// Previous cumulative CDF index.
        previous_index: usize,
        /// Offending CDF index.
        index: usize,
    },
    /// `cdf[N - 1]` is not a valid `Para_Adjustment_List` row.
    AdaptationRateOutOfRange {
        /// Offending CDF index.
        index: usize,
        /// Offending value.
        value: i32,
    },
    /// `cdf[N]` is not in the AV2 capped use-count range `0..=32`.
    CountOutOfRange {
        /// Offending CDF index.
        index: usize,
        /// Offending value.
        value: i32,
    },
}

impl fmt::Display for SymbolCdfErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedLength { len } => {
                write!(f, "CDF length {len} is not supported; expected 3..=9")
            }
            Self::ProbabilityOutOfRange { index, value } => write!(
                f,
                "CDF cumulative entry {index} has value {value}, expected 1..=32767"
            ),
            Self::DecreasingCumulative {
                previous_index,
                index,
            } => write!(
                f,
                "CDF cumulative entry {index} must not be less than entry {previous_index}"
            ),
            Self::AdaptationRateOutOfRange { index, value } => write!(
                f,
                "CDF adaptation-rate entry {index} has value {value}, expected 0..=124"
            ),
            Self::CountOutOfRange { index, value } => write!(
                f,
                "CDF use-count entry {index} has value {value}, expected 0..=32"
            ),
        }
    }
}

/// Specific symbol decoder state violations for AV2 § 8.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolDecoderErrorKind {
    /// The tile payload size cannot be represented by the signed `SymbolMaxBits` state.
    PayloadTooLarge {
        /// Payload length in bytes.
        bytes: usize,
    },
    /// `read_literal(n)` was asked to return more than 32 bits.
    LiteralWidthTooLarge {
        /// Requested literal width.
        requested: u32,
        /// Maximum literal width returned as a `u32`.
        max: u32,
    },
    /// `exit_symbol()` was invoked when `SymbolMaxBits < -14`.
    SymbolMaxBitsTooSmall {
        /// Current signed `SymbolMaxBits` value.
        symbol_max_bits: i64,
    },
    /// The arithmetic interval collapsed before renormalization.
    InvalidArithmeticRange,
    /// The computed trailing bit position was outside the bounded tile payload.
    TrailingBitOutOfRange {
        /// Relative bit position inside the tile payload.
        bit_position: u64,
    },
    /// The computed padding end position was outside the bounded tile payload.
    PaddingEndOutOfRange {
        /// Relative bit position inside the tile payload.
        bit_position: u64,
    },
    /// `paddingEndPosition` was not byte-aligned.
    PaddingEndNotByteAligned {
        /// Relative bit position inside the tile payload.
        bit_position: u64,
    },
    /// The required `exit_symbol()` trailing bit was not equal to `1`.
    MissingTrailingOneBit,
    /// An `exit_symbol()` padding bit after the trailing one was not equal to `0`.
    NonZeroPaddingBit,
}

impl fmt::Display for SymbolDecoderErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { bytes } => {
                write!(
                    f,
                    "tile payload of {bytes} byte(s) is too large for SymbolMaxBits"
                )
            }
            Self::LiteralWidthTooLarge { requested, max } => {
                write!(
                    f,
                    "read_literal({requested}) exceeds the {max}-bit return width"
                )
            }
            Self::SymbolMaxBitsTooSmall { symbol_max_bits } => write!(
                f,
                "SymbolMaxBits is {symbol_max_bits}, but exit_symbol() requires at least -14"
            ),
            Self::InvalidArithmeticRange => f.write_str("symbol arithmetic interval collapsed"),
            Self::TrailingBitOutOfRange { bit_position } => write!(
                f,
                "trailingBitPosition {bit_position} is outside the tile payload"
            ),
            Self::PaddingEndOutOfRange { bit_position } => write!(
                f,
                "paddingEndPosition {bit_position} is outside the tile payload"
            ),
            Self::PaddingEndNotByteAligned { bit_position } => {
                write!(f, "paddingEndPosition {bit_position} is not byte-aligned")
            }
            Self::MissingTrailingOneBit => {
                f.write_str("exit_symbol() trailing bit must be equal to 1")
            }
            Self::NonZeroPaddingBit => {
                f.write_str("exit_symbol() padding bits after the trailing bit must be zero")
            }
        }
    }
}

/// Specific conformance violations of `tile_params()` (AV2 § 6.17.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileParamsErrorKind {
    /// `TileCols` exceeds `MAX_TILE_COLS`.
    TileColsOutOfRange,
    /// `TileRows` exceeds `MAX_TILE_ROWS`.
    TileRowsOutOfRange,
}

impl fmt::Display for TileParamsErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::TileColsOutOfRange => "TileCols must be less than or equal to MAX_TILE_COLS",
            Self::TileRowsOutOfRange => "TileRows must be less than or equal to MAX_TILE_ROWS",
        };
        f.write_str(message)
    }
}

/// Specific structural violations of the metadata OBUs (AV2 § 5.17 / § 6.16) that
/// prevent further parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataErrorKind {
    /// A metadata unit's declared payload size is too small to hold the parsed content:
    /// either `obuPayloadSize - 2 - Leb128Bytes` underflows for a short OBU (§ 5.17.2),
    /// or the child syntax would read past `metadataPayloadSize` so
    /// `remainingMuPayloadBits` would be negative (§ 6.16.1).
    UnitPayloadUnderflow,
    /// `metadata_unit_cnt_minus_1` is not less than 16383 (AV2 § 6.16.3).
    GroupUnitCountTooLarge,
    /// A metadata group unit's `headerRemainingBytes` would go negative: `muh_header_size`
    /// does not account for `Leb128Bytes`, the fixed header fields, and the layer maps
    /// (AV2 § 5.17.3 / § 6.16.3).
    GroupHeaderUnderflow,
}

impl fmt::Display for MetadataErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnitPayloadUnderflow => {
                "metadata unit payload size is too small for the parsed content"
            }
            Self::GroupUnitCountTooLarge => "metadata_unit_cnt_minus_1 must be less than 16383",
            Self::GroupHeaderUnderflow => {
                "muh_header_size is too small for the metadata unit header fields"
            }
        };
        f.write_str(message)
    }
}

/// Errors produced while parsing AV2 bitstreams.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// A feature defined by the AV2 spec is recognized but not yet modeled.
    #[error("unimplemented AV2 feature: {feature}")]
    Unimplemented {
        /// Short, stable name of the missing feature.
        feature: &'static str,
    },

    /// A bit-read requested more bits than the reader supports for the target.
    #[error("cannot read {requested} bits (maximum is {max})")]
    BitWidthTooLarge {
        /// Number of bits requested.
        requested: u32,
        /// Maximum number of bits supported for this read.
        max: u32,
    },

    /// A byte-read requested more bytes than the reader supports for the target.
    #[error("cannot read {requested} little-endian byte(s) (maximum is {max})")]
    ByteWidthTooLarge {
        /// Number of bytes requested.
        requested: u32,
        /// Maximum number of bytes supported for this read.
        max: u32,
    },

    /// The input ended before a complete syntax element could be read.
    #[error("unexpected end of input at byte {offset}: needed {needed} more byte(s)")]
    UnexpectedEof {
        /// Offset at which more data was required.
        offset: ByteOffset,
        /// Number of additional bytes required.
        needed: usize,
    },

    /// A LEB128 value violated AV2 § 4.11.6.
    #[error("invalid LEB128 at byte {offset}: {message}")]
    InvalidLeb128 {
        /// Offset of the start of the LEB128 value.
        offset: ByteOffset,
        /// Human-readable reason.
        message: String,
    },

    /// A `uvlc()` descriptor violated AV2 § 4.11.3.
    #[error("invalid uvlc() at byte {offset}.{bit_offset}: {message}")]
    InvalidUvlc {
        /// Offset of the offending bit.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidUvlc::offset`].
        bit_offset: BitOffset,
        /// Human-readable reason.
        message: String,
    },

    /// An `ns(n)` descriptor was requested with an invalid parameter.
    #[error("invalid ns(n) at byte {offset}.{bit_offset}: {message}")]
    InvalidNs {
        /// Offset of the descriptor request.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidNs::offset`].
        bit_offset: BitOffset,
        /// Human-readable reason.
        message: String,
    },

    /// An `rg(n)` descriptor violated AV2 § 4.11.10 (it must never return a value
    /// less than 0, i.e. its unary prefix must terminate within 32 bits).
    #[error("invalid rg(n) at byte {offset}.{bit_offset}: {message}")]
    InvalidRg {
        /// Offset of the start of the descriptor.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidRg::offset`].
        bit_offset: BitOffset,
        /// Human-readable reason.
        message: String,
    },

    /// A `quantizer_matrix_obu()` / `user_defined_qm()` value violated AV2 § 5.13 /
    /// § 6.4.11 (for example a `quant_delta` outside the conformant `-128..=127`
    /// range).
    #[error("invalid quantizer matrix at byte {offset}.{bit_offset}: {message}")]
    InvalidQuantizerMatrix {
        /// Offset of the offending value.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidQuantizerMatrix::offset`].
        bit_offset: BitOffset,
        /// Human-readable reason.
        message: String,
    },

    /// An OBU header violated AV2 § 5.2.2.
    #[error("invalid OBU header at byte {offset}: {message}")]
    InvalidObuHeader {
        /// Offset of the start of the OBU header.
        offset: ByteOffset,
        /// Human-readable reason.
        message: String,
    },

    /// `trailing_bits(nbBits)` violated AV2 § 6.2.3.
    #[error("invalid trailing_bits() at byte {offset}.{bit_offset}: {kind}")]
    InvalidTrailingBits {
        /// Offset of the offending bit.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidTrailingBits::offset`].
        bit_offset: BitOffset,
        /// Specific trailing-bits violation.
        kind: TrailingBitsErrorKind,
    },

    /// `byte_alignment()` violated AV2 § 6.2.4.
    #[error("invalid byte_alignment() at byte {offset}.{bit_offset}: {kind}")]
    InvalidByteAlignment {
        /// Offset of the offending bit.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidByteAlignment::offset`].
        bit_offset: BitOffset,
        /// Specific byte-alignment violation.
        kind: ByteAlignmentErrorKind,
    },

    /// `sequence_header_obu()` violated AV2 § 6.4.1.
    #[error("invalid sequence_header_obu() at byte {offset}.{bit_offset}: {kind}")]
    InvalidSequenceHeader {
        /// Offset of the offending syntax element.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidSequenceHeader::offset`].
        bit_offset: BitOffset,
        /// Specific sequence-header violation.
        kind: SequenceHeaderErrorKind,
    },

    /// `tile_params()` violated the tile-count limits in AV2 § 6.17.7.2.
    #[error("invalid tile_params() at byte {offset}.{bit_offset}: {kind}")]
    InvalidTileParams {
        /// Offset at which the derived tile count exceeded its conformance limit.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidTileParams::offset`].
        bit_offset: BitOffset,
        /// Specific tile-parameter violation.
        kind: TileParamsErrorKind,
    },

    /// `global_motion_params()` violated AV2 § 6.17.9.
    #[error("invalid global_motion_params() at byte {offset}.{bit_offset}: {kind}")]
    InvalidGlobalMotion {
        /// Offset of the offending syntax element or derived state.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidGlobalMotion::offset`].
        bit_offset: BitOffset,
        /// Specific global-motion violation.
        kind: GlobalMotionErrorKind,
    },

    /// A declared OBU size was structurally invalid (for example, zero).
    #[error("OBU size out of range at byte {offset}: {size}")]
    ObuSizeOutOfRange {
        /// Offset of the OBU length prefix.
        offset: ByteOffset,
        /// The offending declared size.
        size: u64,
    },

    /// `obu_extension_flag` was non-zero, violating AV2 § 6.2.1.
    #[error(
        "invalid obu_extension_flag at byte {offset}.{bit_offset}: must be 0 in this specification version (§ 6.2.1)"
    )]
    InvalidObuExtension {
        /// Offset of the `obu_extension_flag` bit.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidObuExtension::offset`].
        bit_offset: BitOffset,
    },

    /// A declared OBU payload extends beyond the available input.
    #[error(
        "OBU payload out of range at byte {offset}: size {size} exceeds {remaining} remaining byte(s)"
    )]
    ObuPayloadOutOfRange {
        /// Offset of the OBU (its header).
        offset: ByteOffset,
        /// Declared OBU size in bytes.
        size: u32,
        /// Bytes actually remaining in the input.
        remaining: usize,
    },

    /// `layer_config_record_obu()` violated AV2 § 5.8 / § 6.8 in a way that prevents
    /// further parsing.
    #[error("invalid layer_config_record_obu() at byte {offset}.{bit_offset}: {kind}")]
    InvalidLayerConfigRecord {
        /// Offset of the offending syntax element.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidLayerConfigRecord::offset`].
        bit_offset: BitOffset,
        /// Specific layer-config-record violation.
        kind: LayerConfigRecordErrorKind,
    },

    /// `atlas_segment_info_obu()` violated AV2 § 5.9 / § 6.9 in a way that prevents
    /// further parsing.
    #[error("invalid atlas_segment_info_obu() at byte {offset}.{bit_offset}: {kind}")]
    InvalidAtlasSegment {
        /// Offset of the offending syntax element.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidAtlasSegment::offset`].
        bit_offset: BitOffset,
        /// Specific atlas-segment violation.
        kind: AtlasSegmentErrorKind,
    },

    /// `padding_obu()` violated AV2 § 5.16 / § 6.15 in a way that prevents further
    /// parsing.
    #[error("invalid padding_obu() at byte {offset}.{bit_offset}: {kind}")]
    InvalidPadding {
        /// Offset of the offending syntax element.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidPadding::offset`].
        bit_offset: BitOffset,
        /// Specific padding violation.
        kind: PaddingErrorKind,
    },

    /// A metadata OBU (`metadata_short_obu()` / `metadata_group_obu()`) violated AV2
    /// § 5.17 / § 6.16 in a way that prevents further parsing.
    #[error("invalid metadata OBU at byte {offset}.{bit_offset}: {kind}")]
    InvalidMetadata {
        /// Offset of the offending syntax element.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidMetadata::offset`].
        bit_offset: BitOffset,
        /// Specific metadata violation.
        kind: MetadataErrorKind,
    },

    /// A caller-supplied CDF row violated the constraints enforced before AV2 § 8.2.6.
    #[error("invalid symbol CDF at byte {offset}.{bit_offset}: {kind}")]
    InvalidSymbolCdf {
        /// Offset of the symbol decoder when the row was rejected.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidSymbolCdf::offset`].
        bit_offset: BitOffset,
        /// Specific CDF-row violation.
        kind: SymbolCdfErrorKind,
    },

    /// The AV2 § 8.2 symbol decoder reached a malformed or unsupported local state.
    #[error("invalid symbol decoder state at byte {offset}.{bit_offset}: {kind}")]
    InvalidSymbolDecoderState {
        /// Offset of the offending bit or decoder state.
        offset: ByteOffset,
        /// Bit offset within [`Self::InvalidSymbolDecoderState::offset`].
        bit_offset: BitOffset,
        /// Specific symbol-decoder violation.
        kind: SymbolDecoderErrorKind,
    },
}

/// Convenience alias for results produced by `splot-core`.
pub type Result<T> = core::result::Result<T, Error>;
