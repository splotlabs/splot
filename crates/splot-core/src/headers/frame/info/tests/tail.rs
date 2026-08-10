// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header EOF / activation-prefix unit tests (split from [`super`] for the source-line budget).

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
