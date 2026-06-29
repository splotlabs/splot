// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the [`super`] AV2 § 5.4 sequence-header model.

use super::*;

#[test]
fn profile_idc_round_trips_every_5_bit_value() {
    for value in 0u8..=31 {
        assert_eq!(ProfileIdc::from_bits(value).get(), value);
    }
}

#[test]
fn profile_idc_classifies_table_a1_values() {
    assert_eq!(ProfileIdc::from_bits(0), ProfileIdc::Main420Ip0);
    assert_eq!(ProfileIdc::from_bits(1), ProfileIdc::Main420Ip1);
    assert_eq!(ProfileIdc::from_bits(2), ProfileIdc::Main420Ip2);
    assert_eq!(ProfileIdc::from_bits(3), ProfileIdc::Main422Ip1);
    assert_eq!(ProfileIdc::from_bits(4), ProfileIdc::Main444Ip1);
    assert_eq!(ProfileIdc::from_bits(31), ProfileIdc::Configurable);
    for value in 5u8..=30 {
        assert_eq!(ProfileIdc::from_bits(value), ProfileIdc::Reserved(value));
        assert!(ProfileIdc::from_bits(value).is_reserved());
        assert!(!ProfileIdc::from_bits(value).is_configurable());
    }
    assert!(ProfileIdc::from_bits(31).is_configurable());
    assert!(!ProfileIdc::from_bits(0).is_reserved());
}

#[test]
fn profile_idc_ord_matches_raw_value_order() {
    let mut ids: Vec<ProfileIdc> = (0u8..=31).rev().map(ProfileIdc::from_bits).collect();
    ids.sort();
    let sorted: Vec<u8> = ids.iter().map(|p| p.get()).collect();
    assert_eq!(sorted, (0u8..=31).collect::<Vec<_>>());
}

#[test]
fn profile_idc_identity_is_canonical_under_misconstruction() {
    let configurable_via_reserved = ProfileIdc::Reserved(31);
    assert_eq!(configurable_via_reserved, ProfileIdc::Configurable);
    assert!(configurable_via_reserved.is_configurable());
    assert!(!configurable_via_reserved.is_reserved());
    assert_eq!(ProfileIdc::Reserved(0), ProfileIdc::Main420Ip0);
    assert!(ProfileIdc::Reserved(2) < ProfileIdc::Main422Ip1);
    assert_eq!(ProfileIdc::from_bits(0xE0), ProfileIdc::Main420Ip0);
}

use crate::test_bits::Bits;

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

    let m_map = header.mlayer_dependency_map;
    assert!(m_map.depends_on(mlayer(0), mlayer(0)));
    assert!(m_map.depends_on(mlayer(1), mlayer(0)));
    assert!(m_map.depends_on(mlayer(1), mlayer(1)));
    assert!(!m_map.depends_on(mlayer(0), mlayer(1))); // refLayer > currLayer
    assert!(!m_map.depends_on(mlayer(2), mlayer(0))); // currLayer 2 > max_mlayer_id 1
    assert!(!m_map.depends_on(mlayer(2), mlayer(2)));

    let t_map = header.tlayer_dependency_map;
    assert!(t_map.depends_on(mlayer(0), tlayer(2), tlayer(0)));
    assert!(t_map.depends_on(mlayer(1), tlayer(2), tlayer(0)));
    assert!(t_map.depends_on(mlayer(1), tlayer(2), tlayer(2)));
    assert!(!t_map.depends_on(mlayer(0), tlayer(1), tlayer(2))); // refTLayer > currTLayer
    for reference in 0..MAX_NUM_TLAYERS as u8 {
        assert!(!t_map.depends_on(mlayer(0), tlayer(3), tlayer(reference)));
    }
    assert!(!t_map.depends_on(mlayer(2), tlayer(1), tlayer(0))); // mLayer 2 > max_mlayer_id 1
}

#[test]
fn mlayer_dependency_override_bit_order() {
    let mut bits = Bits::default();
    push_general_header_until_dependency_maps(&mut bits, 0, 2);
    bits.bit(1); // mlayer_dependency_present_flag
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
    assert!(
        header
            .tlayer_dependency_map
            .depends_on(mlayer(2), tlayer(0), tlayer(0))
    );
}

#[test]
fn mlayer_presence_map_closes_transitive_dependency() {
    let mut dep = MLayerDependencyMap::default_for(mlayer(2));
    dep.set(1, 1, false); // signaled diagonal zero (irrelevant to presence — reflexive)
    dep.set(2, 0, false); // clear the DIRECT 2 -> 0 edge
    assert!(!dep.depends_on(mlayer(2), mlayer(0)));
    assert!(dep.depends_on(mlayer(2), mlayer(1)));
    assert!(dep.depends_on(mlayer(1), mlayer(0)));

    let presence = dep.presence_map();
    assert!(presence.is_present(mlayer(0), mlayer(0)));
    assert!(presence.is_present(mlayer(1), mlayer(1)));
    assert!(presence.is_present(mlayer(2), mlayer(2)));
    assert!(presence.is_present(mlayer(1), mlayer(0)));
    assert!(presence.is_present(mlayer(2), mlayer(1)));
    assert!(presence.is_present(mlayer(2), mlayer(0)));
    assert!(!presence.is_present(mlayer(0), mlayer(1)));
    assert!(!presence.is_present(mlayer(0), mlayer(2)));
    assert!(!presence.is_present(mlayer(1), mlayer(2)));
}

#[test]
fn tlayer_dependency_row0_replication() {
    let mut bits = Bits::default();
    push_general_header_until_dependency_maps(&mut bits, 1, 1);
    bits.bit(0); // mlayer_dependency_present_flag
    bits.bit(1); // tlayer_dependency_present_flag
    bits.bit(0); // multi_tlayer_dependency_map_present_flag
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
    assert!(t_map.depends_on(mlayer(1), tlayer(1), tlayer(1)));
    assert!(!t_map.depends_on(mlayer(1), tlayer(1), tlayer(0)));
    assert!(!t_map.depends_on(mlayer(2), tlayer(1), tlayer(1)));
}

#[test]
fn tlayer_dependency_multi_rows_signaled() {
    let mut bits = Bits::default();
    push_general_header_until_dependency_maps(&mut bits, 1, 1);
    bits.bit(0); // mlayer_dependency_present_flag
    bits.bit(1); // tlayer_dependency_present_flag
    bits.bit(1); // multi_tlayer_dependency_map_present_flag
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
    bits.bit(0); // seq_tile_info_present_flag (fully parsed)
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
    for _ in 0..4 {
        bits.bit(0);
    }
    bits.bit(0); // enable_mhccp
    bits.bit(0); // enable_ibp
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let intra = parse_sequence_intra_config(&mut reader, true).unwrap();
    assert_eq!(intra.cfl_ds_filter_index, 0);
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
    assert_eq!(tile.seq_sb_col_starts, vec![0]);
    assert_eq!(tile.seq_sb_row_starts, vec![0]);
    assert_eq!(header.film_grain_params_present, Some(false));
}

#[test]
fn sequence_tile_config_records_non_uniform_start_arrays() {
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
    assert_eq!(tile.seq_sb_col_starts, vec![0, 1]);
    assert_eq!(tile.seq_sb_row_starts, vec![0]);
}

#[test]
fn sequence_tile_config_reserved_level_records_empty_start_arrays() {
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
    bits.bit(0); // use_256x256_superblock
    bits.bit(0); // use_128x128_superblock -> seqSbSize = BLOCK_64X64
    bits.bit(0); // enable_sdp
    bits.bit(0); // enable_ext_partitions
    bits.bit(0); // reduce_pb_aspect_ratio
    bits.bit(0); // enable_ext_seg -> MaxSegments = 8
    bits.bit(u8::from(segment_info_present)); // seq_seg_info_present_flag
    if segment_info_present {
        bits.bit(0); // seq_allow_seg_info_change
        for _ in 0..(8 * crate::segment::SEG_LVL_MAX) {
            bits.bit(0); // seg_info(8): 8 segments x SEG_LVL_MAX features, all disabled
        }
    }
    bits.bit(0); // enable_dip
    bits.bit(0); // enable_intra_edge_filter
    bits.bit(0); // enable_mrls
    bits.bit(0); // enable_cfl_intra
    bits.f(0, 2); // cfl_ds_filter_index
    bits.bit(0); // enable_mhccp
    bits.bit(0); // enable_ibp
    bits.bit(0); // enable_refmvbank
    bits.bit(1); // disable_drl_reorder -> DRL_REORDER_DISABLED
    bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
    bits.bit(0); // allow_frame_max_bvp_drl_bits
    bits.bit(0); // enable_bawp
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
    bits.bit(0); // disable_loopfilters_across_tiles
    bits.bit(0); // enable_cdef
    bits.bit(0); // enable_gdf
    bits.bit(0); // enable_restoration
    bits.bit(0); // enable_ccso
    bits.f(0, 2); // df_par_bits_minus_2
}

#[test]
fn sequence_header_child_payload_eof_never_panics() {
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
    use crate::obu::{ParsedObu, PayloadStatus, dispatch_obu_payload, read_obu_header_from_slice};
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
