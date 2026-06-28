// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header EOF / activation-prefix unit tests (split from [`super`] for the source-line budget).

use super::*;

#[test]
fn frame_header_core_eof_at_order_hint() {
    // Enough bits for the prefix and output flags, but order_hint f(4) overruns.
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag
    // order_hint f(4) starts here but only padding bits remain.
    let data = bits.into_bytes();
    let err = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap_err();
    assert!(matches!(err, Error::UnexpectedEof { .. }));
}

#[test]
fn frame_header_core_eof_at_frame_size() {
    // Reaches frame_size() but the explicit width/height overruns the payload.
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag
    bits.f(0, 4); // order_hint
    // frame_width_minus_1 f(12) starts here but the payload ends early.
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
