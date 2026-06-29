// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! High-level-syntax (HLS) OBU payload parsers beyond the sequence header:
//! the Multi Stream Decoder Operation OBU (AV2 v1.0.0 § 5.6) and the
//! Multi-Frame Header OBU (AV2 v1.0.0 § 5.7).
//!
//! These parsers read syntax only; they maintain no decoder state and perform no
//! reconstruction. Local conformance ranges (`num_streams_minus_2 <= 2`, sequence
//! and multi-frame id ranges) are validated by `splot-validate`.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::sequence::{MAX_SEQ_NUM, ProfileIdc, SequenceHeaderId};
use crate::segment::{SegmentInfo, parse_seg_info};
use crate::span::ByteOffset;
use crate::types::{EmbeddedLayerId, TemporalLayerId};

/// `MAX_MFH_NUM` (AV2 v1.0.0 § 3): maximum number of multi-frame headers.
pub const MAX_MFH_NUM: u32 = 16;

/// A `cur_mfh_id` / `mfhId` multi-frame-header identifier (AV2 v1.0.0 § 5.7 / § 6.17).
///
/// The value `0` is the special "no multi-frame header" case: a frame header with
/// `cur_mfh_id == 0` references a sequence header directly via
/// `seq_header_id_in_frame_header`. Stored multi-frame headers use
/// `mfhId = mfh_id_minus_1 + 1`, which conformance bounds to `1..MAX_MFH_NUM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MfhId(u32);

impl MfhId {
    /// Wraps a raw `cur_mfh_id` / `mfhId` value as read from the bitstream. The value
    /// may be out of range; callers gate on [`MfhId::in_range`].
    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// The `cur_mfh_id == 0` "reference a sequence header directly" value.
    #[must_use]
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Returns `true` if this is the `cur_mfh_id == 0` direct-reference value.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Returns `true` if the value can name a valid multi-frame header
    /// (`< MAX_MFH_NUM`; AV2 § 5.7).
    #[must_use]
    pub const fn in_range(self) -> bool {
        self.0 < MAX_MFH_NUM
    }
}

/// HLS availability record for a parsed multi-frame header OBU (AV2 v1.0.0 § 5.7 /
/// § 7.3.8.7), consumed by the frame-header `cur_mfh_id` reference check.
///
/// Beyond the availability identity (`mfh_id` / `mfh_seq_header_id` / layer ids) this
/// also carries the parsed § 5.7 state a `cur_mfh_id > 0` frame header consumes at
/// § 5.18.2: the frame-size payload (for the § 5.18.4.1 default dimensions),
/// the segment-info gating flags and the parsed `MfhFeatureEnabled` / `MfhFeatureData`
/// (for the § 5.18.7.1 MFH-gated `segmentation_params()` arm), and
/// `mfh_deblocking_filter_update` (groundwork; deblocking parsing itself lands with
/// the frame-filtering change). Keeping this state in `splot-core` lets the validator
/// pass a complete view into the core parse without `splot-core` depending on any
/// `splot-validate` type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiFrameHeaderRecord {
    /// `mfhId = mfh_id_minus_1 + 1` (in range, `< MAX_MFH_NUM`).
    pub mfh_id: MfhId,
    /// `mfh_seq_header_id`: the sequence header this multi-frame header references.
    pub mfh_seq_header_id: SequenceHeaderId,
    /// `MfhTLayerId[mfhId]`: the multi-frame header OBU's `obu_tlayer_id`.
    pub mfh_tlayer_id: TemporalLayerId,
    /// `MfhMLayerId[mfhId]`: the multi-frame header OBU's `obu_mlayer_id`.
    pub mfh_mlayer_id: EmbeddedLayerId,
    /// `mfh_frame_size_present_flag` payload, when present (AV2 § 5.7). `None` is the
    /// omitted-size case: § 5.18.2 (:4101) infers the dimensions to the sequence
    /// maxima when this is absent, applied at § 5.18.4.1 consumption.
    pub mfh_frame_size: Option<MfhFrameSize>,
    /// `mfh_seg_info_present_flag` (AV2 § 5.7): gates the § 5.18.7.1 MFH arm.
    pub mfh_seg_info_present_flag: bool,
    /// `mfh_ext_seg_flag`, present when `mfh_seg_info_present_flag` (AV2 § 5.7); the
    /// § 5.18.7.1 `mfh_ext_seg_flag == enable_ext_seg` `haveSegParams` test.
    pub mfh_ext_seg_flag: Option<bool>,
    /// `mfh_allow_seg_info_change`, present when `mfh_seg_info_present_flag`
    /// (AV2 § 5.7); the § 5.18.7.1 `allowChange` input.
    pub mfh_allow_seg_info_change: Option<bool>,
    /// Parsed `seg_info(mfh_ext_seg_flag ? 16 : 8)` (`MfhFeatureEnabled[mfhId]` /
    /// `MfhFeatureData[mfhId]`, AV2 § 5.7 / § 5.4.9), present when
    /// `mfh_seg_info_present_flag`. Reused by the § 5.18.7.1 `reuse_seg_info` arm.
    pub mfh_segment_info: Option<SegmentInfo>,
    /// `mfh_deblocking_filter_update[mfhId]` (AV2 § 5.7): selects the § 5.18.5.2
    /// `cur_mfh_id > 0` deblocking arm, where `apply_deblocking_filter` is copied from
    /// the MFH instead of read from the frame header.
    pub mfh_deblocking_filter_update: bool,
    /// `mfh_apply_deblocking_filter[mfhId][0..4]` (AV2 § 5.7): copied into
    /// `apply_deblocking_filter[i]` by § 5.18.5.2 (mirror :5951-5965) when
    /// `mfh_deblocking_filter_update` is set, gated by `NumPlanes`. All `false` when no
    /// update was signalled.
    pub mfh_apply_deblocking_filter: [bool; 4],
    /// Source byte offset of the multi-frame header OBU that produced this record.
    pub offset: ByteOffset,
}

/// Maximum number of sub-streams an MSDO can signal: `num_streams_minus_2` is
/// `f(3)` (0..=7), so `num_streams_minus_2 + 2` is at most 9.
const MAX_SUB_STREAMS: usize = 9;

/// One per-substream entry of `multistream_decoder_operation_obu()` (AV2 § 5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubStreamConfig {
    /// `sub_xlayer_id[i]`.
    pub sub_xlayer_id: u8,
    /// `sub_stream_max_profile[i]`.
    pub sub_stream_max_profile: u8,
    /// `sub_stream_max_level[i]`.
    pub sub_stream_max_level: u8,
    /// `sub_stream_max_tier[i]`.
    pub sub_stream_max_tier: u8,
}

impl SubStreamConfig {
    const ZERO: Self = Self {
        sub_xlayer_id: 0,
        sub_stream_max_profile: 0,
        sub_stream_max_level: 0,
        sub_stream_max_tier: 0,
    };
}

/// Parsed `multistream_decoder_operation_obu()` (AV2 v1.0.0 § 5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MultistreamDecoderOperation {
    /// `num_streams_minus_2` (`f(3)`). Conformance requires this to be `<= 2`.
    pub num_streams_minus_2: u8,
    /// `multistream_profile_idc` (Annex A Table A.1 value space, shared with
    /// `seq_profile_idc`).
    pub multistream_profile_idc: ProfileIdc,
    /// `multistream_level_idx`.
    pub multistream_level_idx: u8,
    /// `multistream_tier`.
    pub multistream_tier: u8,
    /// `multistream_even_allocation_flag`.
    pub multistream_even_allocation_flag: bool,
    /// `multistream_large_picture_idc`, present when allocation is not even.
    pub multistream_large_picture_idc: Option<u8>,
    /// Number of valid entries in [`MultistreamDecoderOperation::sub_streams`].
    pub sub_stream_count: u8,
    /// Per-substream entries (`num_streams_minus_2 + 2` of them).
    pub sub_streams: [SubStreamConfig; MAX_SUB_STREAMS],
    /// `multistream_doh_constraint_flag`.
    pub multistream_doh_constraint_flag: bool,
}

impl MultistreamDecoderOperation {
    /// Returns the number of independent streams (`num_streams_minus_2 + 2`).
    #[must_use]
    pub const fn num_streams(&self) -> u32 {
        self.num_streams_minus_2 as u32 + 2
    }

    /// Returns the parsed sub-stream entries.
    #[must_use]
    pub fn sub_streams(&self) -> &[SubStreamConfig] {
        &self.sub_streams[..self.sub_stream_count as usize]
    }
}

/// Parses `multistream_decoder_operation_obu()` (AV2 v1.0.0 § 5.6).
///
/// The per-substream loop runs `num_streams_minus_2 + 2` times exactly as signalled;
/// the `num_streams_minus_2 <= 2` conformance bound (§ 6.6) is enforced by the
/// validator, not by truncating the parse.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the
/// payload ends mid-field.
pub fn parse_msdo(reader: &mut BitReader<'_>) -> Result<MultistreamDecoderOperation> {
    let num_streams_minus_2 = reader.read_bits_u8(3)?;
    let multistream_profile_idc = ProfileIdc::from_bits(reader.read_bits_u8(5)?);
    let multistream_level_idx = reader.read_bits_u8(5)?;
    let multistream_tier = reader.read_bits_u8(1)?;
    let multistream_even_allocation_flag = reader.read_flag()?;
    let multistream_large_picture_idc = if multistream_even_allocation_flag {
        None
    } else {
        Some(reader.read_bits_u8(3)?)
    };

    let mut sub_streams = [SubStreamConfig::ZERO; MAX_SUB_STREAMS];
    let count = usize::from(num_streams_minus_2) + 2;
    for entry in sub_streams.iter_mut().take(count) {
        *entry = SubStreamConfig {
            sub_xlayer_id: reader.read_bits_u8(5)?,
            sub_stream_max_profile: reader.read_bits_u8(5)?,
            sub_stream_max_level: reader.read_bits_u8(5)?,
            sub_stream_max_tier: reader.read_bits_u8(1)?,
        };
    }

    let multistream_doh_constraint_flag = reader.read_flag()?;

    Ok(MultistreamDecoderOperation {
        num_streams_minus_2,
        multistream_profile_idc,
        multistream_level_idx,
        multistream_tier,
        multistream_even_allocation_flag,
        multistream_large_picture_idc,
        sub_stream_count: count as u8,
        sub_streams,
        multistream_doh_constraint_flag,
    })
}

/// `mfh_frame_size_present_flag` payload of `multi_frame_header_obu()` (AV2 § 5.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MfhFrameSize {
    /// `mfh_frame_width_bits_minus_1 + 1`.
    pub width_bits: u8,
    /// `mfh_frame_height_bits_minus_1 + 1`.
    pub height_bits: u8,
    /// `mfh_frame_width_minus_1`.
    pub width_minus_1: u32,
    /// `mfh_frame_height_minus_1`.
    pub height_minus_1: u32,
}

/// Parsed `multi_frame_header_obu()` syntax (AV2 v1.0.0 § 5.7).
///
/// Frame-header reuse semantics are out of scope for this phase; this captures the
/// syntax fields needed for HLS availability and future frame references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MultiFrameHeader {
    /// `mfh_seq_header_id`.
    pub mfh_seq_header_id: u32,
    /// `mfh_id_minus_1` (`mfhId = mfh_id_minus_1 + 1`).
    pub mfh_id_minus_1: u32,
    /// `mfh_frame_size_present_flag` payload, when present.
    pub mfh_frame_size: Option<MfhFrameSize>,
    /// `mfh_deblocking_filter_update`.
    pub mfh_deblocking_filter_update: bool,
    /// `mfh_apply_deblocking_filter[0..4]` (all `false` unless an update was signalled).
    pub mfh_apply_deblocking_filter: [bool; 4],
    /// `mfh_seg_info_present_flag`.
    pub mfh_seg_info_present_flag: bool,
    /// `mfh_ext_seg_flag`, present when segment info is signalled.
    pub mfh_ext_seg_flag: Option<bool>,
    /// `mfh_allow_seg_info_change`, present when segment info is signalled.
    pub mfh_allow_seg_info_change: Option<bool>,
    /// Parsed `seg_info(mfh_ext_seg_flag ? 16 : 8)` (§ 5.4.9), present when segment
    /// info is signalled.
    pub segment_info: Option<SegmentInfo>,
}

impl MultiFrameHeader {
    /// Returns `mfhId = mfh_id_minus_1 + 1`.
    #[must_use]
    pub const fn mfh_id(&self) -> u64 {
        self.mfh_id_minus_1 as u64 + 1
    }

    /// Returns `true` if `mfh_seq_header_id` is within `MAX_SEQ_NUM` (AV2 § 6.4.1).
    #[must_use]
    pub const fn seq_header_id_in_range(&self) -> bool {
        self.mfh_seq_header_id < MAX_SEQ_NUM
    }

    /// Returns `true` if `mfhId` is within `MAX_MFH_NUM` (AV2 § 5.7).
    #[must_use]
    pub const fn mfh_id_in_range(&self) -> bool {
        self.mfh_id() < MAX_MFH_NUM as u64
    }
}

/// Parses `multi_frame_header_obu()` syntax in full (AV2 § 5.7), including
/// `seg_info(mfh_ext_seg_flag ? 16 : 8)` (§ 5.4.9) when `mfh_seg_info_present_flag` is
/// set.
///
/// # Errors
/// Returns descriptor errors or
/// [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload ends
/// mid-field.
pub fn parse_multi_frame_header(reader: &mut BitReader<'_>) -> Result<MultiFrameHeader> {
    let mfh_seq_header_id = reader.read_uvlc()?;
    let mfh_id_minus_1 = reader.read_uvlc()?;

    let mfh_frame_size_present_flag = reader.read_flag()?;
    let mfh_frame_size = if mfh_frame_size_present_flag {
        let width_bits = reader.read_bits_u8(4)? + 1;
        let height_bits = reader.read_bits_u8(4)? + 1;
        let width_minus_1 = reader.read_bits(u32::from(width_bits))?;
        let height_minus_1 = reader.read_bits(u32::from(height_bits))?;
        Some(MfhFrameSize {
            width_bits,
            height_bits,
            width_minus_1,
            height_minus_1,
        })
    } else {
        None
    };

    let mfh_deblocking_filter_update = reader.read_flag()?;
    let mut mfh_apply_deblocking_filter = [false; 4];
    if mfh_deblocking_filter_update {
        for flag in &mut mfh_apply_deblocking_filter {
            *flag = reader.read_flag()?;
        }
    }

    let mfh_seg_info_present_flag = reader.read_flag()?;
    let (mfh_ext_seg_flag, mfh_allow_seg_info_change, segment_info) = if mfh_seg_info_present_flag {
        let ext_seg = reader.read_flag()?;
        let allow_change = reader.read_flag()?;
        let info = parse_seg_info(reader, if ext_seg { 16 } else { 8 })?;
        (Some(ext_seg), Some(allow_change), Some(info))
    } else {
        (None, None, None)
    };

    Ok(MultiFrameHeader {
        mfh_seq_header_id,
        mfh_id_minus_1,
        mfh_frame_size,
        mfh_deblocking_filter_update,
        mfh_apply_deblocking_filter,
        mfh_seg_info_present_flag,
        mfh_ext_seg_flag,
        mfh_allow_seg_info_change,
        segment_info,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn msdo_bits(num_streams_minus_2: u32) -> Bits {
        let mut bits = Bits::default();
        bits.f(num_streams_minus_2, 3); // num_streams_minus_2
        bits.f(0, 5); // multistream_profile_idc
        bits.f(0, 5); // multistream_level_idx
        bits.bit(0); // multistream_tier
        bits.bit(1); // multistream_even_allocation_flag (no large_picture_idc)
        for i in 0..(num_streams_minus_2 + 2) {
            bits.f(i & 0x1F, 5); // sub_xlayer_id
            bits.f(0, 5); // sub_stream_max_profile
            bits.f(0, 5); // sub_stream_max_level
            bits.bit(0); // sub_stream_max_tier
        }
        bits.bit(0); // multistream_doh_constraint_flag
        bits
    }

    #[test]
    fn parses_minimal_msdo() {
        let data = msdo_bits(0).into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let msdo = parse_msdo(&mut reader).unwrap();
        assert_eq!(msdo.num_streams_minus_2, 0);
        assert_eq!(msdo.num_streams(), 2);
        assert_eq!(msdo.sub_streams().len(), 2);
        assert!(msdo.multistream_even_allocation_flag);
        assert_eq!(msdo.multistream_large_picture_idc, None);
    }

    #[test]
    fn parses_msdo_uneven_allocation_reads_large_picture_idc() {
        let mut bits = Bits::default();
        bits.f(1, 3); // num_streams_minus_2 = 1 -> 3 streams
        bits.f(2, 5); // multistream_profile_idc
        bits.f(4, 5); // multistream_level_idx
        bits.bit(1); // multistream_tier
        bits.bit(0); // multistream_even_allocation_flag = 0
        bits.f(5, 3); // multistream_large_picture_idc
        for _ in 0..3 {
            bits.f(0, 5);
            bits.f(0, 5);
            bits.f(0, 5);
            bits.bit(0);
        }
        bits.bit(1); // multistream_doh_constraint_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let msdo = parse_msdo(&mut reader).unwrap();
        assert_eq!(msdo.num_streams(), 3);
        assert_eq!(msdo.multistream_large_picture_idc, Some(5));
        assert_eq!(msdo.multistream_tier, 1);
        assert!(msdo.multistream_doh_constraint_flag);
    }

    #[test]
    fn parses_msdo_with_too_many_streams_for_validator_to_flag() {
        let data = msdo_bits(5).into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let msdo = parse_msdo(&mut reader).unwrap();
        assert_eq!(msdo.num_streams_minus_2, 5);
        assert_eq!(msdo.num_streams(), 7);
        assert_eq!(msdo.sub_streams().len(), 7);
    }

    #[test]
    fn msdo_reports_eof() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_msdo(&mut reader),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn parses_minimal_mfh_without_segment_info() {
        let mut bits = Bits::default();
        bits.uvlc(0); // mfh_seq_header_id
        bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mfh = parse_multi_frame_header(&mut reader).unwrap();
        assert_eq!(mfh.mfh_seq_header_id, 0);
        assert_eq!(mfh.mfh_id(), 1);
        assert!(mfh.seq_header_id_in_range());
        assert!(mfh.mfh_id_in_range());
        assert_eq!(mfh.mfh_frame_size, None);
        assert!(!mfh.mfh_deblocking_filter_update);
        assert_eq!(mfh.segment_info, None);
    }

    #[test]
    fn parses_mfh_with_frame_size_and_deblocking() {
        let mut bits = Bits::default();
        bits.uvlc(3); // mfh_seq_header_id
        bits.uvlc(2); // mfh_id_minus_1 -> mfhId = 3
        bits.bit(1); // mfh_frame_size_present_flag
        bits.f(3, 4); // mfh_frame_width_bits_minus_1 -> 4 bits
        bits.f(3, 4); // mfh_frame_height_bits_minus_1 -> 4 bits
        bits.f(15, 4); // mfh_frame_width_minus_1
        bits.f(7, 4); // mfh_frame_height_minus_1
        bits.bit(1); // mfh_deblocking_filter_update
        bits.bit(1); // mfh_apply_deblocking_filter[0]
        bits.bit(0); // [1]
        bits.bit(1); // [2]
        bits.bit(0); // [3]
        bits.bit(0); // mfh_seg_info_present_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mfh = parse_multi_frame_header(&mut reader).unwrap();
        assert_eq!(mfh.mfh_seq_header_id, 3);
        assert_eq!(mfh.mfh_id(), 3);
        let size = mfh.mfh_frame_size.unwrap();
        assert_eq!(size.width_minus_1, 15);
        assert_eq!(size.height_minus_1, 7);
        assert_eq!(mfh.mfh_apply_deblocking_filter, [true, false, true, false]);
        assert_eq!(mfh.segment_info, None);
    }

    #[test]
    fn mfh_with_segment_info_is_parsed() {
        let mut bits = Bits::default();
        bits.uvlc(0); // mfh_seq_header_id
        bits.uvlc(0); // mfh_id_minus_1
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(1); // mfh_seg_info_present_flag
        bits.bit(0); // mfh_ext_seg_flag -> MaxSegments = 8
        bits.bit(1); // mfh_allow_seg_info_change
        for _ in 0..(8 * 3) {
            bits.bit(0); // seg_info(8): all features disabled
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mfh = parse_multi_frame_header(&mut reader).unwrap();
        assert!(mfh.mfh_seg_info_present_flag);
        assert_eq!(mfh.mfh_ext_seg_flag, Some(false));
        assert_eq!(mfh.mfh_allow_seg_info_change, Some(true));
        let info = mfh.segment_info.expect("segment info parsed when present");
        assert_eq!(info.num_segments, 8);
    }

    #[test]
    fn mfh_with_ext_segment_info_uses_sixteen_segments() {
        let mut bits = Bits::default();
        bits.uvlc(0); // mfh_seq_header_id
        bits.uvlc(0); // mfh_id_minus_1
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(1); // mfh_seg_info_present_flag
        bits.bit(1); // mfh_ext_seg_flag -> MaxSegments = 16
        bits.bit(0); // mfh_allow_seg_info_change
        for _ in 0..(16 * 3) {
            bits.bit(0); // seg_info(16): all features disabled
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mfh = parse_multi_frame_header(&mut reader).unwrap();
        assert_eq!(mfh.mfh_ext_seg_flag, Some(true));
        let info = mfh.segment_info.expect("segment info parsed when present");
        assert_eq!(info.num_segments, 16);
    }

    #[test]
    fn mfh_out_of_range_ids_are_detectable() {
        let mut bits = Bits::default();
        bits.uvlc(16); // mfh_seq_header_id
        bits.uvlc(16); // mfh_id_minus_1 -> mfhId = 17 (>= MAX_MFH_NUM)
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mfh = parse_multi_frame_header(&mut reader).unwrap();
        assert!(!mfh.seq_header_id_in_range());
        assert!(!mfh.mfh_id_in_range());
    }

    #[test]
    fn mfh_id_models_zero_and_range() {
        assert!(MfhId::zero().is_zero());
        assert!(MfhId::zero().in_range());
        assert_eq!(MfhId::from_raw(3).get(), 3);
        assert!(!MfhId::from_raw(3).is_zero());
        assert!(MfhId::from_raw(MAX_MFH_NUM - 1).in_range());
        assert!(!MfhId::from_raw(MAX_MFH_NUM).in_range());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// HLS payload parsers must never panic on arbitrary input.
        #[test]
        fn hls_parsers_never_panic(data in proptest::collection::vec(any::<u8>(), 0..128)) {
            let mut msdo_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_msdo(&mut msdo_reader);

            let mut mfh_reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_multi_frame_header(&mut mfh_reader);
        }
    }
}
