// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit and property tests for `crate::write::seq_config`, split into a sibling file and
// `include!`d so the writer source stays under the advisory source-line limit. All
// `super::*` references resolve to the `seq_config` module that includes this file.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::sequence::{
        parse_sequence_inter_config, parse_sequence_intra_config, parse_sequence_partition_config,
        parse_sequence_scc_config, parse_sequence_segment_config,
        parse_sequence_transform_quant_entropy_config,
    };
    use crate::segment::SegmentFeature;
    use crate::span::ByteOffset;

    /// MSB-first bit builder mirroring the `Bits` helper in `headers::sequence`'s own
    /// tests; reuses the same hand-built, spec-grounded fixture style and adds an `ns`
    /// encoder (the inverse of [`BitReader::read_ns`]) for the inter-config fixtures.
    #[derive(Default, Clone)]
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
        /// Encodes `ns(n)`, the inverse of [`BitReader::read_ns`].
        fn ns(&mut self, value: u32, n: u32) {
            let w = u32::BITS - n.leading_zeros();
            let m = (1u64 << w) - u64::from(n);
            let value = u64::from(value);
            if value < m {
                self.f(value as u32, w - 1);
            } else {
                let t = value + m;
                self.f((t >> 1) as u32, w - 1);
                self.bit((t & 1) as u8);
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

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    // ------------------------------------------------------------------
    // § 5.4.3 partition
    // ------------------------------------------------------------------

    fn parse_partition(bytes: &[u8], mono: bool, single: bool) -> SequencePartitionConfig {
        parse_sequence_partition_config(&mut reader(bytes), mono, single).unwrap()
    }

    fn assert_partition_roundtrip(config: &SequencePartitionConfig, mono: bool, single: bool) {
        let mut w = BitWriter::new();
        write_sequence_partition_config(&mut w, config, mono, single).unwrap();
        let bytes = w.into_bytes();
        let reparsed = parse_partition(&bytes, mono, single);
        assert_eq!(&reparsed, config, "parse(write(partition)) != partition");
        let mut w2 = BitWriter::new();
        write_sequence_partition_config(&mut w2, &reparsed, mono, single).unwrap();
        assert_eq!(w2.into_bytes(), bytes, "partition write not idempotent");
    }

    #[test]
    fn partition_byte_exact_all_off() {
        // !256, !128, !sdp(mono), ext_part=0, reduce=0 -> 4 bits: 0 0 0 0.
        let mut bits = Bits::default();
        bits.bit(0); // use_256x256
        bits.bit(0); // use_128x128
        // monochrome -> no enable_sdp bit
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        let data = bits.into_bytes();
        let config = parse_partition(&data, true, true);
        let mut w = BitWriter::new();
        write_sequence_partition_config(&mut w, &config, true, true).unwrap();
        assert_eq!(w.into_bytes(), data, "partition prefix not byte-exact");
        assert_partition_roundtrip(&config, true, true);
    }

    #[test]
    fn partition_all_conditionals_present() {
        // Non-mono, non-single, with sdp + extended_sdp + ext_partitions + uneven + reduce.
        let mut bits = Bits::default();
        bits.bit(0); // use_256x256
        bits.bit(1); // use_128x128
        bits.bit(1); // enable_sdp
        bits.bit(1); // enable_extended_sdp (sdp && !single)
        bits.bit(1); // enable_ext_partitions
        bits.bit(1); // enable_uneven_4way
        bits.bit(1); // reduce_pb_aspect_ratio
        bits.bit(1); // log2_minus_1 = 1 -> MaxPbAspectRatio 4
        let data = bits.into_bytes();
        let config = parse_partition(&data, false, false);
        assert!(config.use_128x128_superblock);
        assert!(config.enable_extended_sdp);
        assert_eq!(config.max_pb_aspect_ratio, 4);
        assert_partition_roundtrip(&config, false, false);
    }

    #[test]
    fn partition_256_implies_no_128() {
        let mut bits = Bits::default();
        bits.bit(1); // use_256x256 -> no use_128x128 bit
        bits.bit(1); // enable_sdp
        // single -> no enable_extended_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce
        let data = bits.into_bytes();
        let config = parse_partition(&data, false, true);
        assert!(config.use_256x256_superblock);
        assert!(!config.use_128x128_superblock);
        assert_eq!(config.max_pb_aspect_ratio, 8);
        assert_partition_roundtrip(&config, false, true);
    }

    #[test]
    fn partition_rejects_128_with_256() {
        let config = SequencePartitionConfig {
            use_256x256_superblock: true,
            use_128x128_superblock: true,
            enable_sdp: false,
            enable_extended_sdp: false,
            enable_ext_partitions: false,
            enable_uneven_4way_partitions: false,
            reduce_pb_aspect_ratio: false,
            max_pb_aspect_ratio: 8,
        };
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_partition_config(&mut w, &config, false, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "use_128x128_superblock"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn partition_rejects_sdp_when_monochrome() {
        let mut config = parse_partition(
            &Bits {
                bits: vec![0, 0, 0, 0],
            }
            .into_bytes(),
            true,
            true,
        );
        config.enable_sdp = true;
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_partition_config(&mut w, &config, true, true),
            Err(WriteError::NonCanonicalSequenceValue { what: "enable_sdp" })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn partition_rejects_extended_sdp_when_single() {
        // sdp on, single -> extended_sdp must be inferred false.
        let mut bits = Bits::default();
        bits.bit(0);
        bits.bit(0);
        bits.bit(1); // enable_sdp
        bits.bit(0);
        bits.bit(0);
        let mut config = parse_partition(&bits.into_bytes(), false, true);
        config.enable_extended_sdp = true;
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_partition_config(&mut w, &config, false, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "enable_extended_sdp"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn partition_rejects_uneven_without_ext_partitions() {
        let mut config = parse_partition(
            &Bits {
                bits: vec![0, 0, 0, 0],
            }
            .into_bytes(),
            true,
            true,
        );
        config.enable_uneven_4way_partitions = true;
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_partition_config(&mut w, &config, true, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "enable_uneven_4way_partitions"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn partition_rejects_bad_aspect_ratio_when_reduce() {
        let mut bits = Bits::default();
        bits.bit(0);
        bits.bit(0);
        bits.bit(0); // ext_part
        bits.bit(1); // reduce
        bits.bit(0); // log2_minus_1 = 0 -> ratio 2
        let mut config = parse_partition(&bits.into_bytes(), true, true);
        assert_eq!(config.max_pb_aspect_ratio, 2);
        config.max_pb_aspect_ratio = 16; // not 2 or 4
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_partition_config(&mut w, &config, true, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "max_pb_aspect_ratio"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn partition_rejects_non_eight_aspect_when_not_reduce() {
        let mut config = parse_partition(
            &Bits {
                bits: vec![0, 0, 0, 0],
            }
            .into_bytes(),
            true,
            true,
        );
        config.max_pb_aspect_ratio = 3; // !reduce but not 8
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_partition_config(&mut w, &config, true, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "max_pb_aspect_ratio"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    // ------------------------------------------------------------------
    // § 5.4.4 segment (+ seg_info)
    // ------------------------------------------------------------------

    fn parse_segment(bytes: &[u8]) -> SequenceSegmentConfig {
        parse_sequence_segment_config(&mut reader(bytes)).unwrap()
    }

    fn assert_segment_roundtrip(config: &SequenceSegmentConfig) {
        let mut w = BitWriter::new();
        write_sequence_segment_config(&mut w, config).unwrap();
        let bytes = w.into_bytes();
        let reparsed = parse_segment(&bytes);
        assert_eq!(&reparsed, config, "parse(write(segment)) != segment");
        let mut w2 = BitWriter::new();
        write_sequence_segment_config(&mut w2, &reparsed).unwrap();
        assert_eq!(w2.into_bytes(), bytes, "segment write not idempotent");
    }

    #[test]
    fn segment_no_info_byte_exact() {
        // enable_ext_seg=0, seq_seg_info_present_flag=0 -> 2 bits.
        let mut bits = Bits::default();
        bits.bit(0);
        bits.bit(0);
        let data = bits.into_bytes();
        let config = parse_segment(&data);
        assert_eq!(config.max_segments, 8);
        let mut w = BitWriter::new();
        write_sequence_segment_config(&mut w, &config).unwrap();
        assert_eq!(w.into_bytes(), data, "segment prefix not byte-exact");
        assert_segment_roundtrip(&config);
    }

    #[test]
    fn segment_with_info_round_trips() {
        // enable_ext_seg=1 (MaxSegments 16), present=1, allow=1, then 16*3 disabled bits.
        let mut bits = Bits::default();
        bits.bit(1); // enable_ext_seg -> 16 segments
        bits.bit(1); // seq_seg_info_present_flag
        bits.bit(1); // seq_allow_seg_info_change
        for _ in 0..(16 * 3) {
            bits.bit(0); // all features disabled
        }
        let config = parse_segment(&bits.into_bytes());
        assert_eq!(config.max_segments, 16);
        assert_eq!(config.seq_allow_seg_info_change, Some(true));
        assert!(config.segment_info.is_some());
        assert_segment_roundtrip(&config);
    }

    #[test]
    fn segment_with_enabled_feature_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // enable_ext_seg -> 8 segments
        bits.bit(1); // present
        bits.bit(0); // allow
        bits.bit(1); // seg0 feat0 enabled
        bits.f(50, 10); // su(10) = 50
        bits.bit(0); // seg0 feat1
        bits.bit(0); // seg0 feat2
        for _ in 0..(7 * 3) {
            bits.bit(0); // segments 1..8 disabled
        }
        let config = parse_segment(&bits.into_bytes());
        let info = config.segment_info.unwrap();
        assert_eq!(info.features[0][0].data, 50);
        assert_segment_roundtrip(&config);
    }

    #[test]
    fn segment_rejects_wrong_max_segments() {
        let mut config = parse_segment(&Bits { bits: vec![0, 0] }.into_bytes());
        config.max_segments = 16; // enable_ext_seg is false -> derived 8
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_segment_config(&mut w, &config),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "max_segments"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn segment_rejects_option_flag_mismatch() {
        let mut config = parse_segment(&Bits { bits: vec![0, 0] }.into_bytes());
        // present flag clear but allow Option set.
        config.seq_allow_seg_info_change = Some(true);
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_segment_config(&mut w, &config),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seq_seg_info_present_flag"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn segment_propagates_seg_info_rejection() {
        let mut bits = Bits::default();
        bits.bit(0);
        bits.bit(1);
        bits.bit(0);
        for _ in 0..(8 * 3) {
            bits.bit(0);
        }
        let mut config = parse_segment(&bits.into_bytes());
        // Corrupt a disabled feature so write_seg_info rejects.
        if let Some(info) = config.segment_info.as_mut() {
            info.features[0][0] = SegmentFeature {
                enabled: false,
                data: 9,
            };
        }
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_segment_config(&mut w, &config),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_disabled_data"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    // ------------------------------------------------------------------
    // § 5.4.5 intra
    // ------------------------------------------------------------------

    fn parse_intra(bytes: &[u8], mono: bool) -> SequenceIntraConfig {
        parse_sequence_intra_config(&mut reader(bytes), mono).unwrap()
    }

    fn assert_intra_roundtrip(config: &SequenceIntraConfig, mono: bool) {
        let mut w = BitWriter::new();
        write_sequence_intra_config(&mut w, config, mono).unwrap();
        let bytes = w.into_bytes();
        let reparsed = parse_intra(&bytes, mono);
        assert_eq!(&reparsed, config, "parse(write(intra)) != intra");
        let mut w2 = BitWriter::new();
        write_sequence_intra_config(&mut w2, &reparsed, mono).unwrap();
        assert_eq!(w2.into_bytes(), bytes, "intra write not idempotent");
    }

    #[test]
    fn intra_non_mono_byte_exact() {
        // 4 bits + f(2) + 2 bits = 8 bits = exactly one byte.
        let mut bits = Bits::default();
        bits.bit(1); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(1); // enable_mrls
        bits.bit(1); // enable_cfl_intra
        bits.f(2, 2); // cfl_ds_filter_index = 2
        bits.bit(0); // enable_mhccp
        bits.bit(1); // enable_ibp
        let data = bits.into_bytes();
        let config = parse_intra(&data, false);
        assert_eq!(config.cfl_ds_filter_index, 2);
        let mut w = BitWriter::new();
        write_sequence_intra_config(&mut w, &config, false).unwrap();
        assert_eq!(w.into_bytes(), data, "intra not byte-exact");
        assert_intra_roundtrip(&config, false);
    }

    #[test]
    fn intra_mono_skips_cfl_index() {
        let mut bits = Bits::default();
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(0); // enable_cfl_intra (no cfl_ds_filter_index when mono)
        bits.bit(0);
        bits.bit(0);
        let config = parse_intra(&bits.into_bytes(), true);
        assert_eq!(config.cfl_ds_filter_index, 0);
        assert_intra_roundtrip(&config, true);
    }

    #[test]
    fn intra_rejects_cfl_index_when_monochrome() {
        let mut config = parse_intra(
            &Bits {
                bits: vec![0, 0, 0, 0, 0, 0],
            }
            .into_bytes(),
            true,
        );
        config.cfl_ds_filter_index = 2;
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_intra_config(&mut w, &config, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "cfl_ds_filter_index"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    // ------------------------------------------------------------------
    // § 5.4.6 inter
    // ------------------------------------------------------------------

    fn parse_inter(bytes: &[u8], single: bool) -> SequenceInterConfig {
        parse_sequence_inter_config(&mut reader(bytes), single).unwrap()
    }

    fn assert_inter_roundtrip(config: &SequenceInterConfig, single: bool) {
        let mut w = BitWriter::new();
        write_sequence_inter_config(&mut w, config, single).unwrap();
        let bytes = w.into_bytes();
        let reparsed = parse_inter(&bytes, single);
        assert_eq!(&reparsed, config, "parse(write(inter)) != inter");
        let mut w2 = BitWriter::new();
        write_sequence_inter_config(&mut w2, &reparsed, single).unwrap();
        assert_eq!(w2.into_bytes(), bytes, "inter write not idempotent");
    }

    #[test]
    fn inter_single_picture_round_trips() {
        // single branch: enable_refmvbank, drl_reorder, ns(3), allow_bvp, enable_bawp.
        let mut bits = Bits::default();
        bits.bit(1); // enable_refmvbank
        bits.bit(0); // disable_drl_reorder = 0
        bits.bit(1); // constrain_drl_reorder = 1 -> Constraint
        bits.ns(2, MAX_REF_BV_STACK_SIZE - 1); // seq_max_bvp_drl_bits_minus_1 = 2 (ns(3))
        bits.bit(1); // allow_frame_max_bvp_drl_bits
        bits.bit(1); // enable_bawp
        let config = parse_inter(&bits.into_bytes(), true);
        assert!(config.enable_refmvbank);
        assert_eq!(config.drl_reorder, DrlReorder::Constraint);
        assert_eq!(config.seq_max_bvp_drl_bits_minus_1, 2);
        assert_eq!(config.num_ref_frames, 2); // inferred
        assert_inter_roundtrip(&config, true);
    }

    #[test]
    fn inter_full_branch_round_trips() {
        let mut bits = Bits::default();
        // seq_enabled_motion_modes[1..5] (4 bits): enable DELTAWARP (index 3 -> 3rd bit).
        bits.bit(0); // [1]
        bits.bit(0); // [2]
        bits.bit(1); // [3] DELTAWARP
        bits.bit(0); // [4]
        bits.bit(1); // seq_frame_motion_modes_present_flag (motionModeEnabled)
        bits.bit(1); // enable_six_param_warp_delta (DELTAWARP enabled)
        bits.bit(1); // enable_masked_compound
        bits.bit(1); // enable_ref_frame_mvs
        bits.bit(1); // reduced_ref_frame_mvs_mode (ref_frame_mvs enabled)
        bits.f(5, 4); // order_hint_bits_minus_1 = 5 -> order_hint_bits 6
        bits.bit(1); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder = 1 -> Disabled
        bits.bit(1); // explicit_ref_frame_map
        bits.bit(1); // explicit_num_ref_frames
        bits.f(4, 4); // num_ref_frames_minus_1 = 4 -> 5
        bits.f(3, 3); // long_term_frame_id_bits = 3
        bits.ns(3, MAX_REF_MV_STACK_SIZE - 1); // seq_max_drl_bits_minus_1 = 3 (ns(5))
        bits.bit(1); // allow_frame_max_drl_bits
        bits.ns(1, MAX_REF_BV_STACK_SIZE - 1); // seq_max_bvp_drl_bits_minus_1 = 1 (ns(3))
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.f(2, 2); // num_same_ref_compound = 2
        bits.bit(1); // enable_tip
        bits.bit(0); // disable_tip_output -> EnableTipOutput = 1
        bits.bit(1); // enable_tip_hole_fill
        bits.bit(1); // enable_mv_traj
        bits.bit(0); // enable_bawp
        bits.bit(1); // enable_cwp
        bits.bit(0); // enable_imp_msk_bld
        bits.bit(1); // enable_df_sub_pu
        bits.bit(1); // enable_tip_explicit_qp (EnableTipOutput && enable_df_sub_pu)
        bits.f(1, 2); // enable_opfl_refine = 1
        bits.bit(0); // enable_refinemv
        bits.bit(1); // enable_tip_refinemv (enable_tip && opfl_refine != 0)
        bits.bit(0); // enable_bru
        bits.bit(1); // enable_adaptive_mvd
        bits.bit(0); // enable_mvd_sign_derive
        bits.bit(1); // enable_flex_mvres
        bits.bit(1); // enable_global_motion
        bits.bit(0); // enable_short_refresh_frame_flags
        let config = parse_inter(&bits.into_bytes(), false);
        assert!(config.seq_enabled_motion_modes[DELTAWARP]);
        assert!(config.enable_six_param_warp_delta);
        assert_eq!(config.order_hint_bits, 6);
        assert_eq!(config.num_ref_frames, 5);
        assert_eq!(config.seq_max_drl_bits_minus_1, 3);
        assert!(config.enable_tip_output);
        assert!(config.enable_tip_explicit_qp);
        assert!(config.enable_tip_refinemv);
        assert_inter_roundtrip(&config, false);
    }

    #[test]
    fn inter_full_branch_inferred_num_ref_frames() {
        // explicit_num_ref_frames = 0 -> NumRefFrames inferred 8; no minus_1 bits.
        let mut bits = Bits::default();
        for _ in 0..4 {
            bits.bit(0); // motion modes off
        }
        // motionModeEnabled false -> no seq_frame_motion_modes_present_flag bit
        // DELTAWARP off -> no enable_six_param_warp_delta bit
        bits.bit(0); // enable_masked_compound
        bits.bit(0); // enable_ref_frame_mvs (false -> no reduced bit)
        bits.f(0, 4); // order_hint_bits_minus_1 = 0 -> 1
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder -> Disabled
        bits.bit(0); // explicit_ref_frame_map
        bits.bit(0); // explicit_num_ref_frames = 0 -> NumRefFrames 8
        bits.f(0, 3); // long_term_frame_id_bits
        bits.ns(0, MAX_REF_MV_STACK_SIZE - 1);
        bits.bit(0); // allow_frame_max_drl_bits
        bits.ns(0, MAX_REF_BV_STACK_SIZE - 1);
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.f(0, 2); // num_same_ref_compound
        bits.bit(0); // enable_tip (false -> no tip sub-bits)
        bits.bit(0); // enable_mv_traj
        bits.bit(0); // enable_bawp
        bits.bit(0); // enable_cwp
        bits.bit(0); // enable_imp_msk_bld
        bits.bit(0); // enable_df_sub_pu (EnableTipOutput false -> no tip_explicit_qp)
        bits.f(0, 2); // enable_opfl_refine
        bits.bit(0); // enable_refinemv (enable_tip false -> no tip_refinemv)
        bits.bit(0); // enable_bru
        bits.bit(0); // enable_adaptive_mvd
        bits.bit(0); // enable_mvd_sign_derive
        bits.bit(0); // enable_flex_mvres
        bits.bit(0); // enable_global_motion
        bits.bit(0); // enable_short_refresh_frame_flags
        let config = parse_inter(&bits.into_bytes(), false);
        assert_eq!(config.num_ref_frames, 8);
        assert_eq!(config.order_hint_bits, 1);
        assert_inter_roundtrip(&config, false);
    }

    #[test]
    fn inter_rejects_simple_motion_mode_set() {
        let mut config = parse_inter(&inter_single_fixture(), true);
        config.seq_enabled_motion_modes[0] = true;
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_inter_config(&mut w, &config, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seq_enabled_motion_modes_simple"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn inter_rejects_single_picture_non_inferred() {
        let mut config = parse_inter(&inter_single_fixture(), true);
        config.enable_global_motion = true; // inferred false for single picture
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_inter_config(&mut w, &config, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "single_picture_inter_inferred"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn inter_rejects_zero_order_hint_bits() {
        let mut config = inter_full_default();
        config.order_hint_bits = 0;
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_inter_config(&mut w, &config, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "order_hint_bits"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn inter_rejects_ns_out_of_range() {
        let mut config = inter_full_default();
        config.seq_max_drl_bits_minus_1 = MAX_REF_MV_STACK_SIZE - 1; // == 5, out of ns(5)
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_inter_config(&mut w, &config, false),
            Err(WriteError::ValueOutOfRange {
                descriptor: "ns",
                ..
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn inter_rejects_order_hint_field_overflow() {
        let mut config = inter_full_default();
        config.order_hint_bits = 18; // minus_1 = 17 needs 5 bits, f(4) overflows
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_inter_config(&mut w, &config, false),
            Err(WriteError::ValueTooWide { width_bits: 4, .. })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn inter_rejects_gated_off_non_default() {
        // A field the parser infers behind a DISABLED gate (and does not read) must be
        // rejected before any bit — a stored non-default would shift the rest of the
        // stream and break read(write(x)) == x. (Codex review on PR #142.)
        fn rejects(mutate: impl FnOnce(&mut SequenceInterConfig), what: &'static str) {
            let mut config = inter_full_default();
            mutate(&mut config);
            let mut w = BitWriter::new();
            let err = write_sequence_inter_config(&mut w, &config, false);
            assert!(
                matches!(err, Err(WriteError::NonCanonicalSequenceValue { what: got }) if got == what),
                "expected NonCanonicalSequenceValue({what})"
            );
            assert_eq!(w.bit_len(), 0);
        }
        rejects(
            |c| c.seq_frame_motion_modes_present_flag = true,
            "seq_frame_motion_modes_present_flag",
        );
        rejects(
            |c| c.enable_six_param_warp_delta = true,
            "enable_six_param_warp_delta",
        );
        rejects(
            |c| c.reduced_ref_frame_mvs_mode = true,
            "reduced_ref_frame_mvs_mode",
        );
        // Codex's exact example: enable_tip = false but the tip subfields are set.
        rejects(
            |c| {
                c.enable_tip_output = true;
                c.enable_df_sub_pu = true;
                c.enable_tip_explicit_qp = true;
            },
            "enable_tip_subfields",
        );
        rejects(|c| c.enable_tip_refinemv = true, "enable_tip_refinemv");
    }

    fn inter_single_fixture() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder -> Disabled
        bits.ns(0, MAX_REF_BV_STACK_SIZE - 1);
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.bit(0); // enable_bawp
        bits.into_bytes()
    }

    fn inter_full_default() -> SequenceInterConfig {
        // A parser-reachable full-branch config (all-zero motion modes, inferred refs).
        parse_inter(&inter_full_zero_fixture(), false)
    }

    fn inter_full_zero_fixture() -> Vec<u8> {
        let mut bits = Bits::default();
        for _ in 0..4 {
            bits.bit(0);
        }
        bits.bit(0); // enable_masked_compound
        bits.bit(0); // enable_ref_frame_mvs
        bits.f(0, 4); // order_hint_bits_minus_1
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder
        bits.bit(0); // explicit_ref_frame_map
        bits.bit(0); // explicit_num_ref_frames
        bits.f(0, 3); // long_term_frame_id_bits
        bits.ns(0, MAX_REF_MV_STACK_SIZE - 1);
        bits.bit(0);
        bits.ns(0, MAX_REF_BV_STACK_SIZE - 1);
        bits.bit(0);
        bits.f(0, 2); // num_same_ref_compound
        bits.bit(0); // enable_tip
        bits.bit(0); // enable_mv_traj
        bits.bit(0); // enable_bawp
        bits.bit(0); // enable_cwp
        bits.bit(0); // enable_imp_msk_bld
        bits.bit(0); // enable_df_sub_pu
        bits.f(0, 2); // enable_opfl_refine
        bits.bit(0); // enable_refinemv
        bits.bit(0); // enable_bru
        bits.bit(0); // enable_adaptive_mvd
        bits.bit(0); // enable_mvd_sign_derive
        bits.bit(0); // enable_flex_mvres
        bits.bit(0); // enable_global_motion
        bits.bit(0); // enable_short_refresh_frame_flags
        bits.into_bytes()
    }

    // ------------------------------------------------------------------
    // § 5.4.7 scc
    // ------------------------------------------------------------------

    fn parse_scc(bytes: &[u8], single: bool) -> SequenceSccConfig {
        parse_sequence_scc_config(&mut reader(bytes), single).unwrap()
    }

    fn assert_scc_roundtrip(config: &SequenceSccConfig, single: bool) {
        let mut w = BitWriter::new();
        write_sequence_scc_config(&mut w, config, single).unwrap();
        let bytes = w.into_bytes();
        let reparsed = parse_scc(&bytes, single);
        assert_eq!(&reparsed, config, "parse(write(scc)) != scc");
        let mut w2 = BitWriter::new();
        write_sequence_scc_config(&mut w2, &reparsed, single).unwrap();
        assert_eq!(w2.into_bytes(), bytes, "scc write not idempotent");
    }

    #[test]
    fn scc_single_picture_no_bits() {
        let config = parse_scc(&[], true);
        assert_eq!(
            config.seq_force_screen_content_tools,
            SELECT_SCREEN_CONTENT_TOOLS
        );
        let mut w = BitWriter::new();
        write_sequence_scc_config(&mut w, &config, true).unwrap();
        assert_eq!(w.bit_len(), 0, "single-picture scc writes no bits");
        assert_scc_roundtrip(&config, true);
    }

    #[test]
    fn scc_choose_both() {
        let mut bits = Bits::default();
        bits.bit(1); // seq_choose_screen_content_tools -> force = 2
        // force > 0 -> seq_choose_integer_mv
        bits.bit(1); // seq_choose_integer_mv -> integer_mv = 2
        let config = parse_scc(&bits.into_bytes(), false);
        assert_eq!(config.seq_force_screen_content_tools, 2);
        assert_eq!(config.seq_force_integer_mv, 2);
        assert_scc_roundtrip(&config, false);
    }

    #[test]
    fn scc_explicit_force_zero() {
        let mut bits = Bits::default();
        bits.bit(0); // !choose -> explicit force
        bits.bit(0); // seq_force_screen_content_tools = 0 -> no integer_mv (inferred 2)
        let config = parse_scc(&bits.into_bytes(), false);
        assert_eq!(config.seq_force_screen_content_tools, 0);
        assert_eq!(config.seq_force_integer_mv, SELECT_INTEGER_MV);
        assert_scc_roundtrip(&config, false);
    }

    #[test]
    fn scc_explicit_force_one_explicit_imv() {
        let mut bits = Bits::default();
        bits.bit(0); // !choose
        bits.bit(1); // force = 1
        bits.bit(0); // !choose_integer_mv
        bits.bit(1); // integer_mv = 1
        let config = parse_scc(&bits.into_bytes(), false);
        assert_eq!(config.seq_force_screen_content_tools, 1);
        assert_eq!(config.seq_force_integer_mv, 1);
        assert_scc_roundtrip(&config, false);
    }

    #[test]
    fn scc_rejects_single_picture_non_inferred() {
        let mut config = parse_scc(&[], true);
        config.seq_force_screen_content_tools = 1;
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_scc_config(&mut w, &config, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "single_picture_scc_inferred"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn scc_rejects_force_out_of_range() {
        let mut config = parse_scc(&Bits { bits: vec![1, 1] }.into_bytes(), false);
        config.seq_force_screen_content_tools = 3; // > 2
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_scc_config(&mut w, &config, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seq_force_screen_content_tools"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn scc_rejects_integer_mv_non_default_when_force_zero() {
        let mut config = parse_scc(&Bits { bits: vec![0, 0] }.into_bytes(), false);
        assert_eq!(config.seq_force_screen_content_tools, 0);
        config.seq_force_integer_mv = 1; // force 0 infers integer_mv = 2
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_scc_config(&mut w, &config, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seq_force_integer_mv"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    // ------------------------------------------------------------------
    // § 5.4.8 transform/quant/entropy
    // ------------------------------------------------------------------

    fn parse_tq(bytes: &[u8], mono: bool, single: bool) -> SequenceTqEntropyConfig {
        parse_sequence_transform_quant_entropy_config(&mut reader(bytes), mono, single).unwrap()
    }

    fn assert_tq_roundtrip(config: &SequenceTqEntropyConfig, mono: bool, single: bool) {
        let mut w = BitWriter::new();
        write_sequence_transform_quant_entropy_config(&mut w, config, mono, single).unwrap();
        let bytes = w.into_bytes();
        let reparsed = parse_tq(&bytes, mono, single);
        assert_eq!(&reparsed, config, "parse(write(tq)) != tq");
        let mut w2 = BitWriter::new();
        write_sequence_transform_quant_entropy_config(&mut w2, &reparsed, mono, single).unwrap();
        assert_eq!(w2.into_bytes(), bytes, "tq write not idempotent");
    }

    /// Non-mono, non-single, !equal_ac_dc_q (signals all delta-q blocks).
    #[test]
    fn tq_full_unequal_acdc_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // enable_fsc -> enable_idtx_intra bit follows
        bits.bit(1); // enable_idtx_intra
        bits.bit(1); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(1); // enable_chroma_dctonly (!mono)
        bits.bit(1); // enable_inter_ddt (!single)
        bits.bit(0); // reduced_tx_part_set
        bits.bit(1); // enable_cctx (!mono)
        bits.bit(1); // enable_tcq
        bits.bit(1); // choose_tcq_per_frame (tcq && !single)
        bits.bit(1); // enable_parity_hiding (!(tcq && !choose))
        bits.bit(1); // enable_avg_cdf (!single)
        bits.bit(1); // avg_cdf_type (enable_avg_cdf)
        bits.bit(1); // separate_uv_delta_q (!mono)
        bits.bit(0); // equal_ac_dc_q = 0
        bits.f(7, 5); // base_y_dc_delta_q
        bits.bit(1); // y_dc_delta_q_enabled
        bits.f(9, 5); // base_uv_dc_delta_q
        bits.bit(0); // uv_dc_delta_q_enabled
        bits.f(11, 5); // base_uv_ac_delta_q
        bits.bit(1); // uv_ac_delta_q_enabled
        let config = parse_tq(&bits.into_bytes(), false, false);
        assert!(!config.equal_ac_dc_q);
        assert_eq!(config.base_y_dc_delta_q, 7);
        assert_eq!(config.base_uv_dc_delta_q, 9);
        assert_eq!(config.base_uv_ac_delta_q, 11);
        assert_tq_roundtrip(&config, false, false);
    }

    /// Non-mono, equal_ac_dc_q (mirrors base_uv_dc from base_uv_ac).
    #[test]
    fn tq_equal_acdc_mirrors_uv_dc() {
        let mut bits = Bits::default();
        bits.bit(1); // enable_fsc -> enable_idtx_intra inferred 1
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // enable_inter_ddt
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq (false -> parity_hiding bit read)
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // enable_avg_cdf (false -> avg_cdf_type inferred 0)
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q = 1 (no base_y block)
        bits.f(13, 5); // base_uv_ac_delta_q
        bits.bit(1); // uv_ac_delta_q_enabled
        let config = parse_tq(&bits.into_bytes(), false, false);
        assert!(config.equal_ac_dc_q);
        assert!(config.enable_idtx_intra);
        assert_eq!(config.base_uv_ac_delta_q, 13);
        assert_eq!(config.base_uv_dc_delta_q, 13); // mirrored
        assert_tq_roundtrip(&config, false, false);
    }

    /// Monochrome single-picture: chroma fields and many gates collapse.
    #[test]
    fn tq_mono_single_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // enable_fsc
        bits.bit(1); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        // mono -> no enable_chroma_dctonly
        // single -> no enable_inter_ddt
        bits.bit(1); // reduced_tx_part_set
        // mono -> no enable_cctx
        bits.bit(1); // enable_tcq
        // tcq && single -> choose_tcq_per_frame inferred 0; (tcq && !choose) -> parity inferred 0
        // single -> enable_avg_cdf inferred (1,1)
        // mono -> no separate_uv_delta_q
        bits.bit(0); // equal_ac_dc_q = 0
        bits.f(3, 5); // base_y_dc_delta_q
        bits.bit(1); // y_dc_delta_q_enabled
        // mono -> no chroma delta-q block
        let config = parse_tq(&bits.into_bytes(), true, true);
        assert!(config.enable_avg_cdf);
        assert_eq!(config.avg_cdf_type, 1);
        assert!(!config.choose_tcq_per_frame);
        assert!(!config.enable_parity_hiding);
        assert_eq!(config.base_y_dc_delta_q, 3);
        assert_tq_roundtrip(&config, true, true);
    }

    #[test]
    fn tq_rejects_idtx_intra_when_fsc() {
        let mut config = parse_tq(&tq_min_fixture(), false, false);
        config.enable_fsc = true;
        config.enable_idtx_intra = false; // must be true when fsc
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_transform_quant_entropy_config(&mut w, &config, false, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "enable_idtx_intra"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn tq_rejects_chroma_field_when_monochrome() {
        let mut config = parse_tq(&tq_mono_fixture(), true, false);
        config.enable_cctx = true; // inferred false for mono
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_transform_quant_entropy_config(&mut w, &config, true, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "monochrome_chroma_fields"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn tq_rejects_inter_ddt_when_single() {
        let mut config = parse_tq(&tq_single_fixture(), false, true);
        config.enable_inter_ddt = true; // inferred false for single
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_transform_quant_entropy_config(&mut w, &config, false, true),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "enable_inter_ddt"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn tq_rejects_parity_hiding_when_tcq_no_choose() {
        let mut config = parse_tq(&tq_tcq_fixture(), false, false);
        // tcq && !choose -> parity inferred 0.
        config.enable_tcq = true;
        config.choose_tcq_per_frame = false;
        config.enable_parity_hiding = true;
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_transform_quant_entropy_config(&mut w, &config, false, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "enable_parity_hiding"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn tq_rejects_base_y_set_when_equal_acdc() {
        let mut config = parse_tq(&tq_equal_fixture(), false, false);
        assert!(config.equal_ac_dc_q);
        config.base_y_dc_delta_q = 5; // inferred 0 when equal
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_transform_quant_entropy_config(&mut w, &config, false, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "base_y_dc_delta_q"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn tq_rejects_uv_dc_not_mirrored_when_equal_acdc() {
        let mut config = parse_tq(&tq_equal_fixture(), false, false);
        // equal_ac_dc_q && !mono: base_uv_dc must mirror base_uv_ac.
        config.base_uv_dc_delta_q = config.base_uv_ac_delta_q.wrapping_add(1) & 0x1F;
        if config.base_uv_dc_delta_q == config.base_uv_ac_delta_q {
            config.base_uv_dc_delta_q = (config.base_uv_ac_delta_q + 2) & 0x1F;
        }
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_transform_quant_entropy_config(&mut w, &config, false, false),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "base_uv_dc_delta_q"
            })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    #[test]
    fn tq_rejects_base_y_field_overflow() {
        let mut config = parse_tq(&tq_min_fixture(), false, false);
        assert!(!config.equal_ac_dc_q);
        config.base_y_dc_delta_q = 32; // f(5) overflow
        let mut w = BitWriter::new();
        assert!(matches!(
            write_sequence_transform_quant_entropy_config(&mut w, &config, false, false),
            Err(WriteError::ValueTooWide { width_bits: 5, .. })
        ));
        assert_eq!(w.bit_len(), 0);
    }

    /// Minimal non-mono, non-single, !equal_ac_dc_q fixture.
    fn tq_min_fixture() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // enable_inter_ddt
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // enable_avg_cdf
        bits.bit(0); // separate_uv_delta_q
        bits.bit(0); // equal_ac_dc_q = 0
        bits.f(0, 5); // base_y_dc_delta_q
        bits.bit(0); // y_dc_delta_q_enabled
        bits.f(0, 5); // base_uv_dc_delta_q
        bits.bit(0); // uv_dc_delta_q_enabled
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        bits.into_bytes()
    }

    fn tq_mono_fixture() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        // mono: no chroma_dctonly
        bits.bit(0); // enable_inter_ddt (!single)
        bits.bit(0); // reduced_tx_part_set
        // mono: no cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // enable_avg_cdf (!single)
        // mono: no separate_uv_delta_q
        bits.bit(0); // equal_ac_dc_q
        bits.f(0, 5); // base_y_dc_delta_q
        bits.bit(0); // y_dc_delta_q_enabled
        bits.into_bytes()
    }

    fn tq_single_fixture() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly (!mono)
        // single: no enable_inter_ddt
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx (!mono)
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        // single: enable_avg_cdf inferred (1,1)
        bits.bit(0); // separate_uv_delta_q (!mono)
        bits.bit(1); // equal_ac_dc_q
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        bits.into_bytes()
    }

    fn tq_tcq_fixture() -> Vec<u8> {
        // tcq on, choose off path, so parity inferred 0.
        let mut bits = Bits::default();
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // enable_inter_ddt
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(1); // enable_tcq
        bits.bit(0); // choose_tcq_per_frame (tcq && !single)
        // (tcq && !choose) -> enable_parity_hiding inferred 0
        bits.bit(0); // enable_avg_cdf
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        bits.into_bytes()
    }

    fn tq_equal_fixture() -> Vec<u8> {
        // Non-mono, non-single, equal_ac_dc_q = 1 with non-zero uv_ac.
        let mut bits = Bits::default();
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // enable_inter_ddt
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // enable_avg_cdf
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q
        bits.f(10, 5); // base_uv_ac_delta_q
        bits.bit(1); // uv_ac_delta_q_enabled
        bits.into_bytes()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::sequence::{
        parse_sequence_inter_config, parse_sequence_intra_config, parse_sequence_partition_config,
        parse_sequence_scc_config, parse_sequence_segment_config,
        parse_sequence_transform_quant_entropy_config,
    };
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    proptest! {
        // --- § 5.4.3 partition ---

        /// Every parser-reachable partition config round-trips and is byte-stable.
        #[test]
        fn roundtrip_partition(
            bytes in proptest::collection::vec(any::<u8>(), 0..4),
            mono in any::<bool>(),
            single in any::<bool>(),
        ) {
            if let Ok(config) = parse_sequence_partition_config(&mut reader(&bytes), mono, single) {
                let mut w = BitWriter::new();
                write_sequence_partition_config(&mut w, &config, mono, single).unwrap();
                let out = w.into_bytes();
                let reparsed =
                    parse_sequence_partition_config(&mut reader(&out), mono, single).unwrap();
                prop_assert_eq!(&reparsed, &config);
                let mut w2 = BitWriter::new();
                write_sequence_partition_config(&mut w2, &reparsed, mono, single).unwrap();
                prop_assert_eq!(w2.into_bytes(), out);
            }
        }

        /// The partition writer never panics on a parsed model.
        #[test]
        fn partition_never_panics(
            bytes in proptest::collection::vec(any::<u8>(), 0..8),
            mono in any::<bool>(),
            single in any::<bool>(),
        ) {
            if let Ok(config) = parse_sequence_partition_config(&mut reader(&bytes), mono, single) {
                let mut w = BitWriter::new();
                let _ = write_sequence_partition_config(&mut w, &config, mono, single);
            }
        }

        // --- § 5.4.4 segment ---

        #[test]
        fn roundtrip_segment(bytes in proptest::collection::vec(any::<u8>(), 0..16)) {
            if let Ok(config) = parse_sequence_segment_config(&mut reader(&bytes)) {
                let mut w = BitWriter::new();
                write_sequence_segment_config(&mut w, &config).unwrap();
                let out = w.into_bytes();
                let reparsed = parse_sequence_segment_config(&mut reader(&out)).unwrap();
                prop_assert_eq!(&reparsed, &config);
                let mut w2 = BitWriter::new();
                write_sequence_segment_config(&mut w2, &reparsed).unwrap();
                prop_assert_eq!(w2.into_bytes(), out);
            }
        }

        #[test]
        fn segment_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..16)) {
            if let Ok(config) = parse_sequence_segment_config(&mut reader(&bytes)) {
                let mut w = BitWriter::new();
                let _ = write_sequence_segment_config(&mut w, &config);
            }
        }

        // --- § 5.4.5 intra ---

        #[test]
        fn roundtrip_intra(
            bytes in proptest::collection::vec(any::<u8>(), 0..4),
            mono in any::<bool>(),
        ) {
            if let Ok(config) = parse_sequence_intra_config(&mut reader(&bytes), mono) {
                let mut w = BitWriter::new();
                write_sequence_intra_config(&mut w, &config, mono).unwrap();
                let out = w.into_bytes();
                let reparsed = parse_sequence_intra_config(&mut reader(&out), mono).unwrap();
                prop_assert_eq!(&reparsed, &config);
                let mut w2 = BitWriter::new();
                write_sequence_intra_config(&mut w2, &reparsed, mono).unwrap();
                prop_assert_eq!(w2.into_bytes(), out);
            }
        }

        #[test]
        fn intra_never_panics(
            bytes in proptest::collection::vec(any::<u8>(), 0..4),
            mono in any::<bool>(),
        ) {
            if let Ok(config) = parse_sequence_intra_config(&mut reader(&bytes), mono) {
                let mut w = BitWriter::new();
                let _ = write_sequence_intra_config(&mut w, &config, mono);
            }
        }

        // --- § 5.4.6 inter ---

        #[test]
        fn roundtrip_inter(
            bytes in proptest::collection::vec(any::<u8>(), 0..16),
            single in any::<bool>(),
        ) {
            if let Ok(config) = parse_sequence_inter_config(&mut reader(&bytes), single) {
                let mut w = BitWriter::new();
                write_sequence_inter_config(&mut w, &config, single).unwrap();
                let out = w.into_bytes();
                let reparsed = parse_sequence_inter_config(&mut reader(&out), single).unwrap();
                prop_assert_eq!(&reparsed, &config);
                let mut w2 = BitWriter::new();
                write_sequence_inter_config(&mut w2, &reparsed, single).unwrap();
                prop_assert_eq!(w2.into_bytes(), out);
            }
        }

        #[test]
        fn inter_never_panics(
            bytes in proptest::collection::vec(any::<u8>(), 0..16),
            single in any::<bool>(),
        ) {
            if let Ok(config) = parse_sequence_inter_config(&mut reader(&bytes), single) {
                let mut w = BitWriter::new();
                let _ = write_sequence_inter_config(&mut w, &config, single);
            }
        }

        // --- § 5.4.7 scc ---

        #[test]
        fn roundtrip_scc(
            bytes in proptest::collection::vec(any::<u8>(), 0..4),
            single in any::<bool>(),
        ) {
            if let Ok(config) = parse_sequence_scc_config(&mut reader(&bytes), single) {
                let mut w = BitWriter::new();
                write_sequence_scc_config(&mut w, &config, single).unwrap();
                let out = w.into_bytes();
                let reparsed = parse_sequence_scc_config(&mut reader(&out), single).unwrap();
                prop_assert_eq!(&reparsed, &config);
                let mut w2 = BitWriter::new();
                write_sequence_scc_config(&mut w2, &reparsed, single).unwrap();
                prop_assert_eq!(w2.into_bytes(), out);
            }
        }

        #[test]
        fn scc_never_panics(
            bytes in proptest::collection::vec(any::<u8>(), 0..4),
            single in any::<bool>(),
        ) {
            if let Ok(config) = parse_sequence_scc_config(&mut reader(&bytes), single) {
                let mut w = BitWriter::new();
                let _ = write_sequence_scc_config(&mut w, &config, single);
            }
        }

        // --- § 5.4.8 transform/quant/entropy ---

        #[test]
        fn roundtrip_tq(
            bytes in proptest::collection::vec(any::<u8>(), 0..12),
            mono in any::<bool>(),
            single in any::<bool>(),
        ) {
            if let Ok(config) =
                parse_sequence_transform_quant_entropy_config(&mut reader(&bytes), mono, single)
            {
                let mut w = BitWriter::new();
                write_sequence_transform_quant_entropy_config(&mut w, &config, mono, single)
                    .unwrap();
                let out = w.into_bytes();
                let reparsed = parse_sequence_transform_quant_entropy_config(
                    &mut reader(&out),
                    mono,
                    single,
                )
                .unwrap();
                prop_assert_eq!(&reparsed, &config);
                let mut w2 = BitWriter::new();
                write_sequence_transform_quant_entropy_config(&mut w2, &reparsed, mono, single)
                    .unwrap();
                prop_assert_eq!(w2.into_bytes(), out);
            }
        }

        #[test]
        fn tq_never_panics(
            bytes in proptest::collection::vec(any::<u8>(), 0..12),
            mono in any::<bool>(),
            single in any::<bool>(),
        ) {
            if let Ok(config) =
                parse_sequence_transform_quant_entropy_config(&mut reader(&bytes), mono, single)
            {
                let mut w = BitWriter::new();
                let _ = write_sequence_transform_quant_entropy_config(&mut w, &config, mono, single);
            }
        }
    }
}
