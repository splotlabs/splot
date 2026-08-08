// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn missing_inter_header_regions_are_typed_header_state_errors() {
    use DecodeHeaderStateError::{
        MissingDisplayOrderHint, MissingFrameSize, MissingInterControlRegion, MissingInterTail,
        ZeroFrameSize,
    };
    type MutationCase = (fn(&mut FrameHeaderCore), DecodeHeaderStateError);
    let cases: [MutationCase; 7] = [
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
