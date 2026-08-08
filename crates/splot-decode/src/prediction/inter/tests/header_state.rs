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

    let error = super::super::validate_ras_reference_ids(&core, &reference, offset)
        .expect_err("RAS reference map");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::MissingInterControlRegion
        }
    ));
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
