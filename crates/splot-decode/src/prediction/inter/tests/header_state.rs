// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::SefTrailingBits;
use splot_core::write::{BitWriter, write_annexb_obu};

fn repack_first_sef_payload(payload: &[u8]) -> (Vec<u8>, usize) {
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let mut header = parsed.header.expect("SEF fixture has an IVF header");
    header.frame_count = u32::try_from(parsed.frames.len()).expect("fixture frame count fits u32");
    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    let mut replaced_frame_index = None;
    for (frame_index, frame) in parsed.frames.iter().enumerate() {
        let mut frame_writer = BitWriter::new();
        for envelope in &frame.obus {
            let obu_payload = if replaced_frame_index.is_none() && envelope.header.obu_type.is_sef()
            {
                replaced_frame_index = Some(frame_index);
                payload
            } else {
                envelope.payload
            };
            write_annexb_obu(&mut frame_writer, &envelope.header, obu_payload)
                .expect("repack fixture OBU");
        }
        write_repacked_ivf_frame(&mut bytes, frame.frame.pts, &frame_writer.into_bytes());
    }
    (
        bytes,
        replaced_frame_index.expect("SEF fixture contains a SEF OBU"),
    )
}

#[test]
fn missing_inter_header_regions_are_typed_header_state_errors() {
    use DecodeHeaderStateError::{
        IncompleteInterFrame, MissingDisplayOrderHint, MissingFrameSize, MissingInterControlRegion,
        MissingInterTail, ZeroFrameSize,
    };
    type MutationCase = (fn(&mut FrameHeaderCore), DecodeHeaderStateError);
    let cases: [MutationCase; 8] = [
        (
            |core| core.status = FrameHeaderParseStatus::CoreFieldsOnly,
            IncompleteInterFrame,
        ),
        (|core| core.inter = None, MissingInterControlRegion),
        (|core| core.inter_tail = None, MissingInterTail),
        (
            |core| core.inter.as_mut().unwrap().interpolation_filter = None,
            DecodeHeaderStateError::MissingInterpolationFilter,
        ),
        (|core| core.order_hint = None, MissingDisplayOrderHint),
        (|core| core.frame_size = None, MissingFrameSize),
        (
            |core| {
                core.frame_size = Some(splot_core::headers::frame::FrameSize::new(0, 64));
            },
            ZeroFrameSize,
        ),
        (
            |core| {
                core.frame_size = Some(splot_core::headers::frame::FrameSize::new(64, 0));
            },
            ZeroFrameSize,
        ),
    ];
    for (mutate, expected) in cases {
        let error = decode_inter_frame_after_core_mutation(TWO_FRAME_INTER_FIXTURE, mutate)
            .expect_err("header state");
        assert!(matches!(error, DecodeError::HeaderState { source } if source == expected));
    }
}

#[test]
fn truncated_inter_header_is_a_malformed_source_diagnostic() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.status = FrameHeaderParseStatus::StoppedInsideInterControl;

    let error = super::super::validate_inter_frame_parse(&core, offset, Some(3))
        .expect_err("truncated inter header");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.2.1"));
    assert_eq!(issue.frame_index(), Some(3));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated inter header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn inter_parser_coverage_preserves_feature_id() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    let cases = [
        (
            FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
            },
            "AV2-5.18.7-SEGMENTATION-TILING",
            "5.18.2",
        ),
        (
            FrameHeaderParseStatus::StoppedBeforeWienerNsFilter {
                feature_id: "lr_temporal_reference_filter_match",
            },
            "lr_temporal_reference_filter_match",
            "5.18.7.11",
        ),
    ];
    for (status, expected_reason, expected_section) in cases {
        core.status = status;
        let error = super::super::validate_inter_frame_parse(&core, offset, Some(4))
            .expect_err("inter parser coverage stop");
        let DecodeError::UnsupportedFeature { unsupported } = error else {
            panic!("expected unsupported feature, got {error}");
        };
        assert_eq!(unsupported.reason(), expected_reason);
        assert_eq!(unsupported.spec_section(), expected_section);
    }
}

#[test]
fn complete_intra_tile_group_remains_reportable() {
    let (_, mut core) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    core.obu_type = splot_core::types::ObuType::RegularTileGroup;
    core.frame_type = Some(splot_core::headers::frame::FrameType::IntraOnly);
    let offset = ByteOffset::new(74);

    let error = super::super::validate_inter_frame_parse(&core, offset, Some(4))
        .expect_err("intra-only tile-group routing");
    let DecodeError::UnsupportedFeature { unsupported } = &error else {
        panic!("expected unsupported feature, got {error}");
    };
    assert_eq!(unsupported.reason(), "unsupported_tile_boundary");
    assert_eq!(unsupported.spec_section(), "5.18.2");
    assert_eq!(unsupported.byte_offset(), Some(offset));
    assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_some());
}

#[test]
fn block_reference_sample_failures_are_typed_reference_state_errors() {
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    let Err(error) = super::super::hold_reference_pair(&reference, 0, None) else {
        panic!("an empty reference slot must fail closed");
    };
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::MissingFrame { slot: 0 }
        }
    ));

    let Err(error) = super::super::hold_reference_pair(&reference, 1, None) else {
        panic!("a reference beyond the active store must fail closed");
    };
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::SlotOutOfRange {
                slot: 1,
                slot_count: 1
            }
        }
    ));

    let frame = crate::test_support::decoded_frame(4, 4);
    let (slot, _writer) = crate::pipeline::inflight::RefFrameSlot::pending(frame.info())
        .expect("pending reference slot");
    reference
        .store
        .put(splot_recon::ReferenceSlot::new(0).expect("slot zero"), slot)
        .expect("store pending reference");
    let Err(error) = super::super::hold_reference_pair(&reference, 0, None) else {
        panic!("an unpublished reference slot must fail closed");
    };
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::ReferenceSamplesUnavailable { slot: 0 }
        }
    ));
}

#[test]
fn ras_missing_reference_map_is_a_typed_header_state_error() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    core.inter = None;
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::validate_ras_reference_ids(&core, &reference, offset, None)
        .expect_err("RAS reference map");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::MissingInterControlRegion
        }
    ));
}

#[test]
fn truncated_inter_frame_prefix_is_a_malformed_source_diagnostic() {
    const TRUNCATED_PREFIX: &[u8] = &[0];
    let (sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let parsed = parse_ivf_fixture(TWO_FRAME_INTER_FIXTURE, "inter");
    let mut envelope = parsed.frames[1]
        .obus
        .iter()
        .find(|envelope| envelope.header.obu_type == splot_core::types::ObuType::RegularTileGroup)
        .copied()
        .expect("inter tile group");
    envelope.payload = TRUNCATED_PREFIX;
    envelope.size = u32::from(envelope.header.header_size_bytes) + 1;
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(1))
            .expect_err("truncated frame-header prefix");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.18.2"));
    assert_eq!(issue.offset(), Some(envelope.offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert!(issue.message().contains("unexpected end of input"));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated frame header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn missing_tile_group_prefix_is_a_malformed_source_diagnostic() {
    let (sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let parsed = parse_ivf_fixture(TWO_FRAME_INTER_FIXTURE, "inter");
    let mut envelope = parsed.frames[1]
        .obus
        .iter()
        .find(|envelope| envelope.header.obu_type == splot_core::types::ObuType::RegularTileGroup)
        .copied()
        .expect("inter tile group");
    envelope.payload = &[];
    envelope.size = u32::from(envelope.header.header_size_bytes);
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(1))
            .expect_err("missing tile-group prefix");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.19"));
    assert_eq!(issue.offset(), Some(envelope.offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert_eq!(
        issue.message(),
        "tile group payload ends before is_first_tile_group"
    );
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("missing tile-group prefix must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn malformed_sef_frame_header_is_a_malformed_source_diagnostic() {
    let (bytes, frame_index) = repack_first_sef_payload(&[]);
    let options = DecodeOptions::default();
    let context = decode_context();
    let plan = context
        .plan_bytes(&bytes, options)
        .expect("plan malformed SEF");

    let result = context
        .pool()
        .install(|| decode_frames_from_plan(&bytes, &options, &plan));
    let Err(error) = result else {
        panic!("malformed SEF frame header decoded successfully");
    };
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.18.2"));
    assert_eq!(issue.frame_index(), Some(frame_index));
    assert!(issue.message().contains("unexpected end of input"));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed SEF frame header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn truncated_tip_activation_prefix_is_a_malformed_source_diagnostic() {
    let (sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let parsed = parse_ivf_fixture(TWO_FRAME_INTER_FIXTURE, "inter");
    let mut envelope = parsed.frames[1]
        .obus
        .iter()
        .find(|envelope| envelope.header.obu_type == splot_core::types::ObuType::RegularTileGroup)
        .copied()
        .expect("inter tile group");
    envelope.header.obu_type = splot_core::types::ObuType::RegularTip;
    envelope.payload = &[];
    envelope.size = u32::from(envelope.header.header_size_bytes);
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(1))
            .expect_err("truncated TIP activation prefix");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.18.2"));
    assert_eq!(issue.offset(), Some(envelope.offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert!(issue.message().contains("unexpected end of input"));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated TIP activation prefix must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn sef_eof_inside_film_grain_is_a_malformed_source_diagnostic() {
    let (mut sequence, _) = fixture_sequence_and_key_core(SEF_FAMILIES_FIXTURE);
    sequence.film_grain_params_present = Some(true);
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let (frame_index, mut envelope) = parsed
        .frames
        .iter()
        .enumerate()
        .find_map(|(frame_index, frame)| {
            frame
                .obus
                .iter()
                .find(|envelope| envelope.header.obu_type.is_sef())
                .copied()
                .map(|envelope| (frame_index, envelope))
        })
        .expect("SEF OBU");
    let mut payload = BitWriter::new();
    payload.write_uvlc(0).expect("cur_mfh_id");
    payload.write_uvlc(0).expect("seq_header_id");
    payload.write_bits(6, 3).expect("frame_to_show_map_idx");
    payload.write_flag(true).expect("derive_sef_order_hint");
    payload.write_flag(true).expect("apply_grain");
    payload.write_bits(2, 3).expect("fgm_id");
    payload.write_bits(0, 8).expect("partial grain_seed");
    let payload = payload.into_bytes();
    envelope.payload = &payload;
    envelope.size = u32::from(envelope.header.header_size_bytes)
        + u32::try_from(payload.len()).expect("payload length fits u32");
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::parse_inter_frame_activation(
        envelope,
        &sequence,
        &reference,
        true,
        Some(frame_index),
    )
    .expect_err("SEF truncated inside film grain");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.18.2"));
    assert_eq!(issue.offset(), Some(envelope.offset));
    assert_eq!(issue.frame_index(), Some(frame_index));
    assert_eq!(
        issue.message(),
        "show-existing frame header ends inside film_grain_config()"
    );
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated SEF frame header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn malformed_sef_trailing_bits_are_a_malformed_source_diagnostic() {
    let mut payload = BitWriter::new();
    payload.write_uvlc(0).expect("cur_mfh_id");
    payload.write_uvlc(0).expect("seq_header_id");
    payload.write_bits(0, 3).expect("frame_to_show_map_idx");
    payload.write_flag(true).expect("derive_sef_order_hint");
    payload.write_bit(1).expect("trailing_one_bit");
    payload.write_bit(1).expect("nonzero trailing_zero_bit");
    let (bytes, frame_index) = repack_first_sef_payload(&payload.into_bytes());
    let options = DecodeOptions::default();
    let context = decode_context();
    let plan = context
        .plan_bytes(&bytes, options)
        .expect("plan malformed SEF trailing bits");

    let result = context
        .pool()
        .install(|| decode_frames_from_plan(&bytes, &options, &plan));
    let Err(error) = result else {
        panic!("SEF with malformed trailing bits decoded successfully");
    };
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.2.3"));
    assert_eq!(issue.frame_index(), Some(frame_index));
    assert!(issue.message().contains("trailing_zero_bit"));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed SEF trailing bits must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn sef_reference_slot_out_of_range_is_a_malformed_source_diagnostic() {
    let (mut sequence, _) = fixture_sequence_and_key_core(SEF_FAMILIES_FIXTURE);
    sequence
        .inter
        .as_mut()
        .expect("sequence inter config")
        .num_ref_frames = 3;
    let order_hint_bits = u32::from(
        sequence
            .inter
            .as_ref()
            .expect("sequence inter config")
            .order_hint_bits,
    );
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let (frame_index, sef_envelope) = parsed
        .frames
        .iter()
        .enumerate()
        .find_map(|(frame_index, frame)| {
            frame
                .obus
                .iter()
                .find(|envelope| envelope.header.obu_type.is_sef())
                .copied()
                .map(|envelope| (frame_index, envelope))
        })
        .expect("SEF OBU");
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid = vec![false; 3];
    reference.ref_order_hint = vec![0; 3];
    for derive_sef_order_hint in [false, true] {
        let mut envelope = sef_envelope;
        let mut payload = BitWriter::new();
        payload.write_uvlc(0).expect("cur_mfh_id");
        payload.write_uvlc(0).expect("seq_header_id");
        payload.write_bits(3, 2).expect("frame_to_show_map_idx");
        payload
            .write_flag(derive_sef_order_hint)
            .expect("derive_sef_order_hint");
        if !derive_sef_order_hint {
            payload
                .write_bits(0, order_hint_bits)
                .expect("display_order_hint");
        }
        payload.write_bit(1).expect("trailing_one_bit");
        let payload = payload.into_bytes();
        envelope.payload = &payload;
        envelope.size = u32::from(envelope.header.header_size_bytes)
            + u32::try_from(payload.len()).expect("payload length fits u32");

        let error = super::super::parse_validated_inter_frame_core_with_mfh(
            envelope,
            &sequence,
            &reference,
            true,
            None,
            Some(frame_index),
        )
        .expect_err("out-of-range SEF reference slot");
        let DecodeError::MalformedSource { issue } = &error else {
            panic!("expected malformed source, got {error}");
        };
        assert_eq!(issue.spec_section(), Some("6.17.2"));
        assert_eq!(issue.frame_index(), Some(frame_index));
        assert_eq!(
            issue.message(),
            "show-existing-frame reference slot 3 is outside the active 3-slot buffer"
        );
    }
}

#[test]
fn empty_sef_trailing_bits_use_payload_conformance_section() {
    let (sequence, _) = fixture_sequence_and_key_core(SEF_FAMILIES_FIXTURE);
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let envelope = parsed
        .frames
        .iter()
        .flat_map(|frame| &frame.obus)
        .find(|envelope| envelope.header.obu_type.is_sef())
        .copied()
        .expect("SEF OBU");
    let num_ref_frames = usize::from(
        sequence
            .inter
            .as_ref()
            .expect("sequence inter config")
            .num_ref_frames,
    );
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid = vec![false; num_ref_frames];
    reference.ref_order_hint = vec![0; num_ref_frames];
    let mut core =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(2))
            .expect("complete SEF state");
    core.sef_trailing_bits = Some(SefTrailingBits::Empty);

    let error = super::super::validate_sef_frame_core(&core, &reference, envelope.offset, Some(2))
        .expect_err("empty SEF trailing bits");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.2.1"));
}

fn tip_output_core_for_validation() -> (FrameHeaderCore, ByteOffset) {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.status = FrameHeaderParseStatus::InterHeaderComplete;
    core.obu_type = splot_core::types::ObuType::RegularTip;
    core.frame_is_intra = Some(false);
    core.inter.as_mut().expect("inter control").tip_frame_mode = Some(TipFrameMode::AsOutput);
    (core, offset)
}

#[test]
fn tip_output_quantization_missing_control_is_a_typed_header_state_error() {
    let (mut sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    sequence
        .inter
        .as_mut()
        .expect("fixture has inter sequence config")
        .enable_tip_explicit_qp = false;
    core.quantization_params = None;
    core.inter = None;
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error =
        super::super::infer_tip_output_quantization(&mut core, &sequence, &reference, offset, None)
            .expect_err("missing TIP inter control");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::MissingInterControlRegion,
        }
    ));
}

#[test]
fn tip_output_without_reference_pair_is_a_malformed_source_diagnostic() {
    let (mut sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let (mut core, offset) = tip_output_core_for_validation();
    sequence
        .inter
        .as_mut()
        .expect("fixture has inter sequence config")
        .enable_tip_explicit_qp = false;
    core.order_hint_lsb = Some(10);
    core.order_hint = Some(10);
    core.quantization_params = None;
    core.inter.as_mut().expect("inter control").ref_frame_idx = [0, 1].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid = vec![true; 2];
    reference.ref_order_hint = vec![10; 2];

    let error = super::super::infer_tip_output_quantization(
        &mut core,
        &sequence,
        &reference,
        offset,
        Some(6),
    )
    .expect_err("TIP reference pair");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("7.10.1"));
    assert_eq!(issue.frame_index(), Some(6));
    assert!(issue.message().contains("past/future reference pair"));
}

#[test]
fn tip_output_missing_reference_quantizer_is_a_typed_reference_state_error() {
    let (mut sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let (mut core, offset) = tip_output_core_for_validation();
    sequence
        .inter
        .as_mut()
        .expect("fixture has inter sequence config")
        .enable_tip_explicit_qp = false;
    core.order_hint_lsb = Some(10);
    core.order_hint = Some(10);
    core.quantization_params = None;
    core.inter.as_mut().expect("inter control").ref_frame_idx = [0, 1].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid = vec![true; 2];
    reference.ref_order_hint = vec![9, 12];

    let error =
        super::super::infer_tip_output_quantization(&mut core, &sequence, &reference, offset, None)
            .expect_err("TIP reference quantizer metadata");
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::error::DecodeReferenceStateError::MissingQuantizerMetadata { slot: 0 },
        }
    ));
}

#[test]
fn truncated_tip_output_header_is_a_malformed_source_diagnostic() {
    let (mut core, offset) = tip_output_core_for_validation();
    core.status = FrameHeaderParseStatus::StoppedInsideInterControl;

    let error = super::super::validate_tip_output_frame_parse(&core, offset, Some(3))
        .expect_err("truncated TIP-output header");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.2.1"));
    assert_eq!(issue.frame_index(), Some(3));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated TIP-output header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn non_output_tip_mode_is_a_malformed_source_diagnostic() {
    let (mut core, offset) = tip_output_core_for_validation();
    core.inter.as_mut().expect("inter control").tip_frame_mode = Some(TipFrameMode::Disabled);

    let error = super::super::validate_tip_output_frame_parse(&core, offset, Some(4))
        .expect_err("non-output TIP mode");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.17.2"));
    assert_eq!(issue.frame_index(), Some(4));
}

#[test]
fn single_picture_tip_obu_is_a_malformed_source_diagnostic() {
    let (sequence, _) = fixture_sequence_and_key_core(SINGLE_PICTURE_BRIDGE_FIXTURE);
    assert!(sequence.general.single_picture_header_flag);
    let parsed = parse_ivf_fixture(SINGLE_PICTURE_BRIDGE_FIXTURE, "single-picture bridge");
    let (frame_index, mut envelope) = parsed
        .frames
        .iter()
        .enumerate()
        .find_map(|(frame_index, frame)| {
            frame
                .obus
                .iter()
                .find(|envelope| {
                    envelope.header.obu_type == splot_core::types::ObuType::ClosedLoopKey
                })
                .copied()
                .map(|envelope| (frame_index, envelope))
        })
        .expect("closed-loop-key OBU");
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    reader.read_bit().expect("is_first_tile_group");
    let mut payload = BitWriter::new();
    while reader.remaining_bits() != 0 {
        payload
            .write_bit(reader.read_bit().expect("key payload bit"))
            .expect("TIP payload bit");
    }
    let payload = payload.into_bytes();
    envelope.header.obu_type = splot_core::types::ObuType::RegularTip;
    envelope.payload = &payload;
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    let error = super::super::parse_validated_inter_frame_core_with_mfh(
        envelope,
        &sequence,
        &reference,
        true,
        None,
        Some(frame_index),
    )
    .expect_err("TIP OBU under a single-picture sequence");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.17.2"));
    assert_eq!(issue.frame_index(), Some(frame_index));
}

#[test]
fn tip_output_parser_coverage_preserves_feature_id() {
    let (mut core, offset) = tip_output_core_for_validation();
    core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
        feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
    };

    let error = super::super::validate_tip_output_frame_parse(&core, offset, Some(5))
        .expect_err("TIP-output parser coverage stop");
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("expected unsupported feature, got {error}");
    };
    assert_eq!(unsupported.reason(), "AV2-5.18.7-SEGMENTATION-TILING");
    assert_eq!(unsupported.spec_section(), "5.18.2");
}

#[test]
fn impossible_tip_output_state_is_a_typed_header_state_error() {
    let (mut core, _offset) = tip_output_core_for_validation();
    core.frame_is_intra = Some(true);

    let error = super::super::validate_tip_output_frame_core(&core)
        .expect_err("incomplete TIP-output state");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::IncompleteTipOutput
        }
    ));
}

#[test]
fn impossible_tip_output_runtime_inputs_are_typed_state_errors() {
    type MutationCase = (
        fn(&mut SequenceHeader, &mut FrameHeaderCore),
        DecodeHeaderStateError,
    );
    let (sequence, core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    let cases: [MutationCase; 4] = [
        (
            |_, core| core.frame_size = None,
            DecodeHeaderStateError::MissingFrameSize,
        ),
        (
            |_, core| core.inter = None,
            DecodeHeaderStateError::MissingInterControlRegion,
        ),
        (
            |sequence, _| sequence.partition = None,
            DecodeHeaderStateError::IncompleteTipOutput,
        ),
        (
            |_, core| core.order_hint = None,
            DecodeHeaderStateError::MissingDisplayOrderHint,
        ),
    ];

    for (mutate, expected) in cases {
        let mut sequence = sequence.clone();
        let mut core = core.clone();
        mutate(&mut sequence, &mut core);
        let error = super::super::block::tip::reconstruct_output(
            &mut super::super::InterDecodeScratch::default(),
            &sequence,
            &core,
            &reference,
            splot_recon::BitDepth::Eight,
            offset,
        )
        .expect_err("TIP-output runtime state");
        assert!(matches!(error, DecodeError::HeaderState { source } if source == expected));
    }
}

#[test]
fn unpublished_tip_output_motion_field_is_a_typed_reference_state_error() {
    let (sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.inter.as_mut().expect("inter control").ref_frame_idx = [0].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_motion_fields = vec![Some(super::super::MotionFieldHandle::pending())];

    let error = super::super::block::tip::reconstruct_output(
        &mut super::super::InterDecodeScratch::default(),
        &sequence,
        &core,
        &reference,
        splot_recon::BitDepth::Eight,
        offset,
    )
    .expect_err("unpublished TIP-output motion field");
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::error::DecodeReferenceStateError::MissingMotionFieldPublication,
        }
    ));
}

#[test]
fn impossible_sef_state_is_a_typed_header_state_error() {
    let (sequence, _) = fixture_sequence_and_key_core(SEF_FAMILIES_FIXTURE);
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let envelope = parsed
        .frames
        .iter()
        .flat_map(|frame| &frame.obus)
        .find(|envelope| envelope.header.obu_type.is_sef())
        .copied()
        .expect("SEF OBU");
    let num_ref_frames = usize::from(
        sequence
            .inter
            .as_ref()
            .expect("sequence inter config")
            .num_ref_frames,
    );
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid = vec![false; num_ref_frames];
    reference.ref_order_hint = vec![0; num_ref_frames];
    let mut core =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(2))
            .expect("complete SEF state");
    core.immediate_output_frame = None;

    let error = super::super::validate_sef_frame_core(&core, &reference, envelope.offset, Some(2))
        .expect_err("incomplete SEF state");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::IncompleteShowExistingFrame
        }
    ));
}

#[test]
fn ras_unlisted_long_term_reference_is_a_malformed_source_diagnostic() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    core.ref_long_term_ids = vec![3];
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [0].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_long_term_id = vec![Some(5)];

    let error = super::super::validate_ras_reference_ids(&core, &reference, offset, Some(1))
        .expect_err("unlisted RAS reference");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("6.17.2"));
    assert_eq!(issue.offset(), Some(offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert_eq!(
        issue.message(),
        "RAS reference slot 0 has RefLongTermId 5, which is absent from the frame's listed \
         long-term IDs [3]"
    );
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed RAS input must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);

    core.ref_long_term_ids = vec![5];
    super::super::validate_ras_reference_ids(&core, &reference, offset, Some(1))
        .expect("listed RAS reference");
}

#[test]
fn ras_out_of_range_reference_slot_is_a_malformed_source_diagnostic() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [1].into_iter().collect();
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::validate_ras_reference_ids(&core, &reference, offset, Some(1))
        .expect_err("out-of-range RAS reference slot");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("6.17.2"));
    assert_eq!(issue.offset(), Some(offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert_eq!(
        issue.message(),
        "RAS reference slot 1 is outside the active reference map of 0 slots"
    );
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("out-of-range RAS slot must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn ras_slot_conformance_precedes_ccso_reference_reuse() {
    let (sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [1].into_iter().collect();
    let ccso = core.ccso_params.as_mut().expect("CCSO state");
    ccso.planes
        .push(splot_core::headers::frame::CcsoPlaneParams {
            ccso_planes: true,
            reuse_ccso: true,
            sb_reuse_ccso: false,
            ccso_ref_idx: Some(0),
            ccso_bo_only: None,
            ccso_scale_idx: None,
            ccso_quant_idx: None,
            ccso_ext_filter: None,
            ccso_edge_clf: None,
            ccso_max_band_log2: None,
            ccso_offset_idx: Vec::new(),
        });
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::validate_and_resolve_inter_frame_core(
        &mut core,
        &sequence,
        &reference,
        offset,
        Some(1),
    )
    .expect_err("RAS conformance must precede CCSO reuse");
    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn ras_slot_conformance_precedes_parser_coverage() {
    let (sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
        feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
    };
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [1].into_iter().collect();
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::validate_and_resolve_inter_frame_core(
        &mut core,
        &sequence,
        &reference,
        offset,
        Some(1),
    )
    .expect_err("RAS conformance must precede parser coverage");
    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn out_of_range_primary_reference_is_a_malformed_source_diagnostic() {
    let error = decode_inter_frame_after_core_mutation(TWO_FRAME_INTER_FIXTURE, |core| {
        let inter = core.inter.as_mut().unwrap();
        inter.signal_primary_ref_frame = Some(true);
        inter.primary_ref_frame = Some(6);
        inter.disable_cross_frame_cdf_init = Some(true);
        inter.ref_frame_idx = [0].into_iter().collect();
        inter.num_total_refs = Some(1);
    })
    .expect_err("primary reference must be inside the active map");

    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("6.17"));
    assert!(issue.offset().is_some());
    assert_eq!(issue.frame_index(), Some(1));
    assert_eq!(
        issue.message(),
        "primary reference index 6 is outside the active 1-entry map"
    );

    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed frame-header input must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}
