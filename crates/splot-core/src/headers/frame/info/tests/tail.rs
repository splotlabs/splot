// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header EOF, activation-prefix and display-order-hint tests.

use super::*;

#[test]
fn frame_header_core_inter_mv_precision_eof_boundaries_are_truncation() {
    for (order_hint_bits, use_qtr_precision_mv, expected_bits) in [(4, None, 32), (11, Some(0), 40)]
    {
        let mut seq = base_seq();
        seq.order_hint_bits = order_hint_bits;
        seq.inter.explicit_ref_frame_map = true;
        seq.inter.enable_ref_frame_mvs = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(1); // frame_is_inter == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag
        bits.f(0, order_hint_bits); // order_hint
        bits.bit(1); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 3); // primary_ref_frame
        bits.f(0, 8); // refresh_frame_flags
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(1, 3); // num_total_refs
        bits.f(0, 3); // ref_frame_idx[0]
        bits.bit(0); // use_ref_frame_mvs
        bits.bit(0); // allow_intrabc
        if let Some(use_qtr_precision_mv) = use_qtr_precision_mv {
            bits.bit(use_qtr_precision_mv);
        }
        assert_eq!(bits.bit_len(), expected_bits);
        let data = bits.into_bytes();
        assert_eq!(data.len() * 8, expected_bits);

        let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &seq).unwrap();
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedInsideInterControl
        );
        assert!(core.status.is_truncated_in_modeled_region());
        let Some(inter) = core.inter.as_ref() else {
            panic!("partial inter facts were not preserved");
        };
        assert_eq!(inter.allow_screen_content_tools, Some(false));
        assert_eq!(inter.allow_intrabc, Some(false));
        assert_eq!(inter.force_integer_mv, None);
        assert_eq!(inter.mv_precision, None);
    }
}

#[test]
fn frame_header_core_eof_at_order_hint() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag
    let data = bits.into_bytes();
    let err = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap_err();
    assert!(matches!(err, Error::UnexpectedEof { .. }));
}

#[test]
fn frame_header_core_eof_at_frame_size() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag
    bits.f(0, 4); // order_hint
    let data = bits.into_bytes();
    let err = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap_err();
    assert!(matches!(err, Error::UnexpectedEof { .. }));
}

#[test]
fn frame_header_core_activation_prefix_mode_stops_at_prefix() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(1); // seq_header_id_in_frame_header
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let input = FrameHeaderParseInput {
        obu_type: ObuType::ClosedLoopKey,
        first_picture_in_tu: true,
        active_sequence: None,
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::ActivationPrefix,
    };
    let core = parse_frame_header_core(&mut reader, &input).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::ActivationFieldsOnly);
    assert_eq!(core.seq_header_id_in_frame_header, Some(1));
    assert_eq!(core.frame_type, None);
    assert_eq!(core.frame_size, None);
}

#[test]
fn frame_header_core_without_sequence_is_activation_fields_only() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(1); // seq_header_id_in_frame_header
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let input = FrameHeaderParseInput {
        obu_type: ObuType::ClosedLoopKey,
        first_picture_in_tu: true,
        active_sequence: None,
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    let core = parse_frame_header_core(&mut reader, &input).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::ActivationFieldsOnly);
    assert_eq!(
        core.referenced_sequence_header_id,
        SequenceHeaderId::try_new(1)
    );
    assert_eq!(core.frame_type, None);
}

#[test]
fn frame_header_core_eof_at_cur_mfh_id() {
    let mut reader = BitReader::new(&[], ByteOffset::new(0));
    let input = FrameHeaderParseInput {
        obu_type: ObuType::ClosedLoopKey,
        first_picture_in_tu: true,
        active_sequence: None,
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    assert!(matches!(
        parse_frame_header_core(&mut reader, &input),
        Err(Error::UnexpectedEof { .. })
    ));
}

#[test]
fn frame_header_core_inter_mfh_default_frame_size_uses_mfh_dims() {
    let seq = minimal_inter_seq_64();
    let mfh_size = Some(crate::hls::MfhFrameSize {
        width_bits: 6,
        height_bits: 6,
        width_minus_1: 48 - 1,
        height_minus_1: 32 - 1,
    });
    let record = mfh_record(mfh_size, None);
    let view = MfhFrameView::from_record(&record, &seq);

    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1, resolved from the MFH view.
    bits.bit(1); // frame_is_inter == 1
    bits.bit(1); // immediate_output_frame == 1 -> implicit_output_frame inferred 0
    bits.bit(0); // frame_size_override_flag == 0 -> default_dims from MFH
    bits.f(1, 4); // order_hint
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(0); // use_ref_frame_mvs = 0
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(1); // is_filter_switchable = 1
    bits.bit(0); // disable_cdf_update
    bits.f(90, 8); // quantization_params(): base_q_idx
    bits.bit(0); // segmentation_enabled = 0
    bits.bit(0); // setup_qm_params(): using_qmatrix = 0
    bits.bit(0); // delta_q_params(): delta_q_present = 0
    bits.bit(0); // allow_df_sub_pu
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(0); // apply_deblocking_filter[2]
    bits.bit(0); // apply_deblocking_filter[3]
    bits.bit(1); // read_tx_mode(): tx_mode_select = 1 -> TX_MODE_SELECT
    bits.bit(1); // frame_reference_mode(): reference_select = 1
    bits.bit(1); // skip_mode_params(): skip_mode_present = 1
    bits.f(2, 2); // reduced_tx_set
    bits.bit(0); // film_grain_config(): apply_grain = 0
    let data = bits.into_bytes();

    let (rv, roh, rw, rh) = one_valid_ref_64();
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh);
    let (core, _) = parse_body_with_ref(
        &data,
        ObuType::RegularTileGroup,
        false,
        &seq,
        Some(&view),
        &rs,
    )
    .unwrap();

    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
    assert_eq!(core.frame_size, Some(FrameSize::new(48, 32)));
}

#[test]
fn closed_loop_key_display_order_hint_is_not_extended_past_the_lsb_window() {
    let valid = [true];
    let hints = [68];
    let lsbs = [68];
    let implicit = [false];
    let immediate = [true];
    let widths = [64];
    let heights = [64];
    let reference_state = FrameReferenceStateView::from_slots(&valid, &hints, &widths, &heights)
        .with_single_layer_order_hint_state(&lsbs, &implicit, &immediate);
    assert_eq!(
        get_disp_order_hint(
            ObuType::ClosedLoopKey,
            Some(FrameType::Key),
            None,
            0,
            7,
            &reference_state,
        ),
        Some(0)
    );
    assert_eq!(
        get_disp_order_hint(
            ObuType::RegularTileGroup,
            Some(FrameType::Inter),
            None,
            0,
            7,
            &reference_state,
        ),
        Some(128)
    );
}
