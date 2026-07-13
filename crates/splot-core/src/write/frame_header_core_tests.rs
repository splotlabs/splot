// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
mod tests {
    use super::*;
    use crate::headers::frame::{
        FrameHeaderParseStatus, FrameSize,
    };

    use crate::test_bits::Bits;


    /// A representative non-single-picture sequence view (mirrors `info.rs::tests::base_seq`):
    /// OrderHintBits 4, NumRefFrames 8, no long-term ids, full refresh signaling, screen
    /// content forced off, 12-bit frame dimensions, 4096x2304 maximum, grain absent.
    fn base_seq() -> CoreSeqView {
        CoreSeqView::new_minimal_intra(4096, 2304).expect("4096x2304 is a valid maximum")
    }

    fn parse(
        data: &[u8],
        obu_type: ObuType,
        first_pic: bool,
        seq: &CoreSeqView,
    ) -> FrameHeaderCore {
        parse_core_body_for_test(data, obu_type, first_pic, seq, None).unwrap()
    }

    fn write_core(core: &FrameHeaderCore, seq: &CoreSeqView, first_pic: bool) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_frame_header_core(&mut writer, core, seq, None, first_pic).unwrap();
        writer.into_bytes()
    }

    /// Asserts `written` and `original` carry the same first `bits` bits, MSB-first. The
    /// `written` buffer zero-pads its final partial byte, while `original` may keep arbitrary
    /// payload bits after the header in that byte, so the comparison is bit-granular.
    fn assert_bits_equal(written: &[u8], original: &[u8], bits: u64, obu_type: ObuType) {
        let full_bytes = (bits / 8) as usize;
        assert_eq!(
            &written[..full_bytes],
            &original[..full_bytes],
            "{obu_type:?}: written whole bytes are not exact"
        );
        let rem = (bits % 8) as u32;
        if rem != 0 {
            let mask = 0xffu8 << (8 - rem);
            assert_eq!(
                written[full_bytes] & mask,
                original[full_bytes] & mask,
                "{obu_type:?}: written partial byte's header bits are not exact"
            );
        }
    }

    /// Parses `data` to an `IntraHeaderComplete` core, writes it back, and asserts byte-exact
    /// output and an equal reparse (the full parse -> write -> parse round-trip). Returns the
    /// parsed core for further assertions.
    fn assert_roundtrip(
        data: &[u8],
        obu_type: ObuType,
        first_pic: bool,
        seq: &CoreSeqView,
    ) -> FrameHeaderCore {
        let core = parse(data, obu_type, first_pic, seq);
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::IntraHeaderComplete,
            "fixture must parse to IntraHeaderComplete"
        );
        let written = write_core(&core, seq, first_pic);
        assert_bits_equal(&written, data, core.consumed_bits, obu_type);
        let reparsed = parse(&written, obu_type, first_pic, seq);
        assert_cores_equal(&reparsed, &core);
        core
    }

    /// Compares the two cores ignoring `consumed_bits` (the written buffer is exactly the
    /// frame header, while a fixture may carry trailing payload bytes).
    fn assert_cores_equal(a: &FrameHeaderCore, b: &FrameHeaderCore) {
        let mut a = a.clone();
        let mut b = b.clone();
        a.consumed_bits = 0;
        b.consumed_bits = 0;
        assert_eq!(a, b, "parse(write(core)) != core");
    }


    /// CLK, cur_mfh_id == 0, full non-lossless intra path with grain absent (the exact bytes
    /// from `info.rs::tests::frame_header_core_reads_direct_sequence_reference`).
    fn clk_direct_reference_bits() -> Bits {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(1); // seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(5, 4); // order_hint
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select = 0 -> TX_MODE_LARGEST
        bits.f(0, 2); // reduced_tx_set = 0
        bits
    }

    #[test]
    fn clk_non_lossless_round_trips() {
        let data = clk_direct_reference_bits().into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &base_seq());
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.refresh_frame_flags, Some((1 << 8) - 1));
        assert!(core.cur_mfh_id.is_zero());
    }

    #[test]
    fn olk_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(2); // seq_header_id
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(3, 4); // order_hint
        bits.f(0b0000_0101, 8); // refresh_frame_flags f(NumRefFrames == 8) direct
        bits.f(640 - 1, 12); // frame_width_minus_1
        bits.f(480 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(64, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::OpenLoopKey, true, &base_seq());
        assert_eq!(core.immediate_output_frame, Some(false));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
    }

    #[test]
    fn intra_only_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.bit(0); // frame_is_inter == 0 -> INTRA_ONLY_FRAME
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(7, 4); // order_hint
        bits.f(0b0000_0010, 8); // refresh_frame_flags f(NumRefFrames) (INTRA_ONLY direct arm)
        bits.f(320 - 1, 12); // frame_width_minus_1
        bits.f(240 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(100, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::RegularTileGroup, false, &base_seq());
        assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0010));
    }

    #[test]
    fn single_picture_key_round_trips() {
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        seq.ccso.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.f(9, 4); // order_hint
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(120, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(core.frame_size_override_flag, Some(false));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.implicit_output_frame, Some(false));
    }

    #[test]
    fn lossless_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(0, 4); // order_hint
        bits.f(256 - 1, 12); // frame_width_minus_1
        bits.f(256 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(0, 8); // base_q_idx == 0 -> lossless candidate
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &base_seq());
        assert!(core.lossless_info.as_ref().unwrap().coded_lossless);
    }

    #[test]
    fn grain_present_round_trips() {
        let mut seq = base_seq();
        seq.film_grain_params_present = Some(true);
        let mut bits = clk_direct_reference_bits();
        bits.bit(0); // apply_grain == 0
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &seq);
        assert!(!core.intra_tail.as_ref().unwrap().film_grain.apply_grain);
    }

    #[test]
    fn multi_tile_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(1); // seq_header_id
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(5, 4); // order_hint
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1 (TileColsLog2 -> 1)
        bits.bit(0); // stop incrementing cols
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(0, 1); // context_update_tile_id
        bits.f(0, 2); // tile_size_bytes_minus_1
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::ClosedLoopKey, true, &base_seq());
        let tile = core.tile_info.as_ref().unwrap();
        assert!(tile.tile_cols >= 2, "fixture should be multi-tile");
    }

    #[test]
    fn mfh_nonzero_round_trips() {
        let mfh_size = crate::hls::MfhFrameSize {
            width_bits: 12,
            height_bits: 12,
            width_minus_1: 1920 - 1,
            height_minus_1: 1080 - 1,
        };
        let record = crate::hls::MultiFrameHeaderRecord {
            mfh_id: crate::hls::MfhId::from_raw(1),
            mfh_seq_header_id: crate::headers::sequence::SequenceHeaderId::try_new(0).unwrap(),
            mfh_tlayer_id: crate::types::TemporalLayerId::from_bits(0),
            mfh_mlayer_id: crate::types::EmbeddedLayerId::from_bits(0),
            mfh_frame_size: Some(mfh_size),
            mfh_seg_info_present_flag: false,
            mfh_ext_seg_flag: None,
            mfh_allow_seg_info_change: None,
            mfh_segment_info: None,
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
            offset: crate::span::ByteOffset::new(0),
        };
        let seq = base_seq();
        let view = MfhFrameView::from_record(&record, &seq);

        let mut bits = Bits::default();
        bits.uvlc(1); // cur_mfh_id == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag == 0 (MFH default dims, no size bits)
        bits.f(7, 4); // order_hint
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();

        let core =
            parse_core_body_for_test(&data, ObuType::ClosedLoopKey, true, &seq, Some(&view))
                .unwrap();
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert_eq!(core.frame_size, Some(FrameSize::new(1920, 1080)));

        let mut writer = BitWriter::new();
        write_frame_header_core(&mut writer, &core, &seq, Some(&view), true).unwrap();
        let written = writer.into_bytes();
        assert_bits_equal(&written, &data, core.consumed_bits, ObuType::ClosedLoopKey);
        let reparsed =
            parse_core_body_for_test(&written, ObuType::ClosedLoopKey, true, &seq, Some(&view))
                .unwrap();
        assert_cores_equal(&reparsed, &core);
    }


    /// Builds a valid CLK intra core for mutation in the rejection tests.
    fn valid_core() -> (FrameHeaderCore, CoreSeqView) {
        let seq = base_seq();
        let data = clk_direct_reference_bits().into_bytes();
        let core = parse(&data, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        (core, seq)
    }

    /// Asserts the writer rejects `core` with `what`, leaving the buffer untouched. `first_pic`
    /// is the `FirstPictureInTU` the core was parsed with, threaded so the `starts_cvs`
    /// derivation matches (a genuine parsed core is accepted apart from the mutated field).
    fn assert_rejected_what(
        core: &FrameHeaderCore,
        seq: &CoreSeqView,
        first_pic: bool,
        what: &'static str,
    ) {
        let mut writer = BitWriter::new();
        let err = write_frame_header_core(&mut writer, core, seq, None, first_pic).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what },
            "expected reject {what}"
        );
        assert_eq!(writer.bit_len(), 0, "{what}: bits written on reject");
    }

    #[test]
    fn reject_non_intra_status() {
        for status in [
            FrameHeaderParseStatus::ActivationFieldsOnly,
            FrameHeaderParseStatus::ShowExistingFrameComplete,
            FrameHeaderParseStatus::StoppedInsideFilterParams,
            FrameHeaderParseStatus::StoppedInsideIntraTail,
            FrameHeaderParseStatus::StoppedInsideInterControl,
            FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: "AV2-5.18.2-FRAME-HEADER-INFO",
            },
            FrameHeaderParseStatus::StoppedBeforeWienerNsFilter {
                feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
            },
        ] {
            let (mut core, seq) = valid_core();
            core.status = status;
            assert_rejected_what(&core, &seq, true, "status");
        }
    }

    #[test]
    fn reject_missing_required_option() {
        let (mut core, seq) = valid_core();
        core.tile_info = None;
        assert_rejected_what(&core, &seq, true, "tile_info");

        let (mut core, seq) = valid_core();
        core.intra_tail = None;
        assert_rejected_what(&core, &seq, true, "intra_tail");

        let (mut core, seq) = valid_core();
        core.lr_params = None;
        assert_rejected_what(&core, &seq, true, "lr_params");

        let (mut core, seq) = valid_core();
        core.order_hint_lsb = None;
        assert_rejected_what(&core, &seq, true, "order_hint_lsb");
    }

    #[test]
    fn reject_lr_params_partial_set() {
        use crate::headers::frame::{FrameRestorationType, LrPartialParams, LrPlaneParams};
        let partial = LrPartialParams {
            uses_lr: true,
            planes: vec![
                LrPlaneParams {
                    restoration_type: FrameRestorationType::WienerNonsep,
                    frame_filters_on: true,
                    num_filter_classes: Some(6),
                    frame_filter_bank: None,
                },
                LrPlaneParams {
                    restoration_type: FrameRestorationType::None,
                    frame_filters_on: false,
                    num_filter_classes: None,
                    frame_filter_bank: None,
                },
                LrPlaneParams {
                    restoration_type: FrameRestorationType::None,
                    frame_filters_on: false,
                    num_filter_classes: None,
                    frame_filter_bank: None,
                },
            ],
            loop_restoration_size: [256, 32, 32],
        };

        let (mut core, seq) = valid_core();
        core.lr_params_partial = Some(partial);
        assert_rejected_what(&core, &seq, true, "lr_params_partial");
    }

    #[test]
    fn reject_show_existing_model() {
        let (mut core, seq) = valid_core();
        core.show_existing_frame = Some(true);
        assert_rejected_what(&core, &seq, true, "show_existing_frame");
    }

    #[test]
    fn reject_inferred_immediate_output_disagrees() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0); // implicit_output_frame (OLK: immediate inferred false, no bit)
        bits.bit(1); // frame_size_override_flag
        bits.f(3, 4); // order_hint
        bits.f(0, 8); // refresh_frame_flags direct
        bits.f(64 - 1, 12); // frame_width_minus_1
        bits.f(64 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.f(90, 8); // base_q_idx (single superblock -> no increment/context bits)
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let seq = base_seq();
        let mut core = parse(&data, ObuType::OpenLoopKey, true, &seq);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        core.immediate_output_frame = Some(true); // OLK infers false
        assert_rejected_what(&core, &seq, true, "immediate_output_frame");
    }

    #[test]
    fn reject_refresh_flags_arm_cannot_represent() {
        let mut seq = base_seq();
        seq.enable_short_refresh_frame_flags = true;
        seq.max_mlayer_id = 1; // CLK no longer takes the inferred all-frames arm
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(3, 4); // order_hint
        bits.f(2, 3); // frame_to_refresh f(CeilLog2(8) == 3) -> refresh == 1 << 2
        bits.f(64 - 1, 12);
        bits.f(64 - 1, 12);
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let mut core = parse(&data, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert_eq!(core.refresh_frame_flags, Some(1 << 2));
        core.refresh_frame_flags = Some(0b0000_0011);
        assert_rejected_what(&core, &seq, true, "refresh_frame_flags");
    }

    #[test]
    fn long_term_id_overflow_is_rejected_not_panicked() {
        let (mut core, seq) = valid_core();
        core.long_term_id = Some(i32::MAX);
        assert_rejected_what(&core, &seq, true, "long_term_id");
    }

    #[test]
    fn single_picture_open_loop_key_round_trips() {
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        seq.ccso.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.f(9, 4); // order_hint
        bits.f(0, 8); // refresh_frame_flags (OLK -> direct f(NumRefFrames == 8) arm)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(120, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::OpenLoopKey, true, &seq);
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.immediate_output_frame, Some(true));
    }


    /// A representative single-picture sequence (mirrors `single_picture_key_round_trips`),
    /// with `single_picture_header_flag` set on every sub-view that consults it.
    fn single_picture_seq() -> CoreSeqView {
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        seq.ccso.single_picture_header_flag = true;
        seq
    }

    /// The canonical single-picture CLK body bytes (the fixture from
    /// `single_picture_key_round_trips`), parsing to an `IntraHeaderComplete` core.
    fn single_picture_clk_bits() -> Bits {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.f(9, 4); // order_hint
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(120, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        bits
    }

    /// Builds a valid single-picture CLK intra core for mutation in the single-picture-gated
    /// rejection tests.
    fn valid_single_picture_core() -> (FrameHeaderCore, CoreSeqView) {
        let seq = single_picture_seq();
        let data = single_picture_clk_bits().into_bytes();
        let core = parse(&data, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        (core, seq)
    }

    #[test]
    fn reject_mismatched_inferred_frame_type() {
        let (mut core, seq) = valid_core();
        core.frame_type = Some(FrameType::IntraOnly);
        assert_rejected_what(&core, &seq, true, "frame_type");
    }

    #[test]
    fn reject_mfh_view_on_direct_reference() {
        let (core, seq) = valid_core();
        assert!(core.cur_mfh_id.is_zero());
        let record = crate::hls::MultiFrameHeaderRecord {
            mfh_id: crate::hls::MfhId::from_raw(1),
            mfh_seq_header_id: crate::headers::sequence::SequenceHeaderId::try_new(0).unwrap(),
            mfh_tlayer_id: crate::types::TemporalLayerId::from_bits(0),
            mfh_mlayer_id: crate::types::EmbeddedLayerId::from_bits(0),
            mfh_frame_size: None,
            mfh_seg_info_present_flag: false,
            mfh_ext_seg_flag: None,
            mfh_allow_seg_info_change: None,
            mfh_segment_info: None,
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
            offset: crate::span::ByteOffset::new(0),
        };
        let view = MfhFrameView::from_record(&record, &seq);
        let mut writer = BitWriter::new();
        let err = write_frame_header_core(&mut writer, &core, &seq, Some(&view), true).unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalFrameHeader { what: "mfh_record" });
        assert_eq!(writer.bit_len(), 0, "mfh_record: bits written on reject");
    }

    #[test]
    fn reject_show_existing_frame_none() {
        let (mut core, seq) = valid_core();
        core.show_existing_frame = None;
        assert_rejected_what(&core, &seq, true, "show_existing_frame");
    }

    #[test]
    fn reject_mirrored_allow_intrabc_disagrees() {
        let (mut core, seq) = valid_core();
        let mirrored = core.intrabc.as_ref().unwrap().allow_intrabc;
        core.allow_intrabc = Some(!mirrored);
        assert_rejected_what(&core, &seq, true, "allow_intrabc");
    }

    #[test]
    fn reject_forbidden_ref_long_term_id_mismatch() {
        let (mut core, seq) = valid_core();
        assert!(!core.forbidden_ref_long_term_id);
        core.forbidden_ref_long_term_id = true;
        assert_rejected_what(&core, &seq, true, "forbidden_ref_long_term_id");
    }

    #[test]
    fn reject_single_picture_output_inference_disagrees() {
        let (mut core, seq) = valid_single_picture_core();
        core.immediate_output_frame = Some(false);
        assert_rejected_what(&core, &seq, true, "immediate_output_frame");

        let (mut core, seq) = valid_single_picture_core();
        core.implicit_output_frame = Some(true);
        assert_rejected_what(&core, &seq, true, "implicit_output_frame");
    }

    #[test]
    fn reject_stale_long_term_id_on_no_bit_arms() {
        let (mut core, seq) = valid_single_picture_core();
        assert_eq!(core.long_term_id, None);
        core.long_term_id = Some(0);
        assert_rejected_what(&core, &seq, true, "long_term_id");

        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.bit(0); // frame_is_inter == 0 -> INTRA_ONLY_FRAME
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(7, 4); // order_hint
        bits.f(0b0000_0010, 8); // refresh_frame_flags
        bits.f(320 - 1, 12); // frame_width_minus_1
        bits.f(240 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(100, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let seq = base_seq();
        let mut core = parse(&data, ObuType::RegularTileGroup, false, &seq);
        assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
        assert_eq!(core.long_term_id, Some(-1));
        core.long_term_id = Some(0); // not the -1 sentinel
        assert_rejected_what(&core, &seq, false, "long_term_id");
    }

    #[test]
    fn reject_reached_qm_reset_true() {
        let (mut core, seq) = valid_core();
        assert!(!core.reached_qm_reset);
        core.reached_qm_reset = true;
        assert_rejected_what(&core, &seq, true, "reached_qm_reset");
    }


    #[test]
    fn reject_non_single_bridge_intra_model() {
        let (mut core, seq) = valid_core();
        assert!(!seq.single_picture_header_flag);
        core.obu_type = ObuType::BridgeFrame;
        core.is_bridge = true;
        core.bridge_frame_ref_idx = Some(0);
        core.starts_cvs = false;
        assert_rejected_what(&core, &seq, true, "bridge_unsupported");
    }

    #[test]
    fn reject_stale_bridge_frame_ref_idx_on_non_bridge() {
        let (mut core, seq) = valid_core();
        assert!(!core.is_bridge);
        core.bridge_frame_ref_idx = Some(3);
        assert_rejected_what(&core, &seq, true, "bridge_frame_ref_idx");
    }

    #[test]
    fn reject_stale_frame_to_show_map_idx() {
        let (mut core, seq) = valid_core();
        assert_eq!(core.frame_to_show_map_idx, None);
        core.frame_to_show_map_idx = Some(2);
        assert_rejected_what(&core, &seq, true, "frame_to_show_map_idx");
    }

    #[test]
    fn reject_stale_inter_control() {
        let (mut core, seq) = valid_core();
        assert!(core.inter.is_none());
        core.inter = Some(crate::headers::frame::InterControl::default());
        assert_rejected_what(&core, &seq, true, "inter");
    }

    #[test]
    fn reject_stale_sef_film_grain() {
        // sef_film_grain is the show-existing-frame film_grain_config, None on the intra path.
        // `FilmGrainConfig` is #[non_exhaustive], so
        // clone a real one from the intra tail's parsed film_grain rather than constructing it.
        let (mut core, seq) = valid_core();
        assert!(core.sef_film_grain.is_none());
        let grain = core.intra_tail.as_ref().unwrap().film_grain;
        core.sef_film_grain = Some(grain);
        assert_rejected_what(&core, &seq, true, "sef_film_grain");
    }

    #[test]
    fn reject_stale_sef_trailing_bits() {
        let (mut core, seq) = valid_core();
        assert!(core.sef_trailing_bits.is_none());
        core.sef_trailing_bits = Some(crate::headers::frame::SefTrailingBits::Valid);
        assert_rejected_what(&core, &seq, true, "sef_trailing_bits");
    }

    #[test]
    fn reject_stale_starts_cvs() {
        let (mut core, seq) = valid_core();
        assert_eq!(core.obu_type, ObuType::ClosedLoopKey);
        assert!(core.starts_cvs);
        core.starts_cvs = false;
        assert_rejected_what(&core, &seq, true, "starts_cvs");

        let data = clk_direct_reference_bits().into_bytes();
        let core = parse(&data, ObuType::ClosedLoopKey, false, &seq);
        assert!(!core.starts_cvs);
        let written = write_core(&core, &seq, false);
        assert_bits_equal(&written, &data, core.consumed_bits, ObuType::ClosedLoopKey);
        let reparsed = parse(&written, ObuType::ClosedLoopKey, false, &seq);
        assert_cores_equal(&reparsed, &core);
    }

    #[test]
    fn single_picture_sef_round_trips() {
        let seq = single_picture_seq();
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id
        bits.f(9, 4); // order_hint
        bits.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8) direct (RegularSef not CLK)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(120, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // tx_mode_select
        bits.f(0, 2); // reduced_tx_set
        let data = bits.into_bytes();
        let core = assert_roundtrip(&data, ObuType::RegularSef, true, &seq);
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.show_existing_frame, Some(false));
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.sef_film_grain, None);
        assert_eq!(core.frame_to_show_map_idx, None);
    }

    #[test]
    fn reject_single_picture_bridge() {
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(5, 3); // bridge_frame_ref_idx = 5 f(CeilLog2(8) == 3) — read before single-pic
        bits.bit(0); // bridge_frame_overwrite_flag = 0 (mirror :4423)
        bits.bit(0); // allow_intrabc = 0 (intrabc_params(), mirror :4571) -> STOP at bridge return
        let data = bits.into_bytes();
        let core =
            parse_core_body_for_test(&data, ObuType::BridgeFrame, true, &seq, None).unwrap();
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        assert!(core.is_bridge);
        let mut writer = BitWriter::new();
        let err = write_frame_header_core(&mut writer, &core, &seq, None, true).unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalFrameHeader { what: "status" });
        assert_eq!(writer.bit_len(), 0);
    }
}
