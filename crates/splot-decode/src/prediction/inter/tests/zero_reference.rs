// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const ZERO_REFERENCE_INTRA_SEED: &[u8] = &[
    0x44, 0x4b, 0x49, 0x46, 0x00, 0x00, 0x20, 0x00, 0x41, 0x56, 0x30, 0x32, 0x40, 0x00, 0x40, 0x00,
    0x1e, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x2f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x08, 0x16, 0x04,
    0x80, 0x0a, 0x00, 0x55, 0x7f, 0xfc, 0x01, 0x80, 0x00, 0x00, 0x00, 0x00, 0x68, 0x26, 0x81, 0x00,
    0xe1, 0x04, 0xdc, 0x05, 0xa4, 0x15, 0x10, 0xf0, 0x11, 0x50, 0x00, 0x00, 0x05, 0x89, 0x81, 0xe1,
    0xf9, 0x3b, 0xd8, 0x01, 0x74, 0x14, 0x7b, 0x20, 0x0c, 0x35, 0xc0, 0x0c, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x08, 0x09, 0x1c, 0xf8, 0x49, 0x1d, 0x40, 0x00,
    0x00, 0x00, 0x40,
];

#[test]
fn zero_reference_inter_frame_decodes_when_every_block_is_intra() {
    let frame = decode_inter_frame_after_core_mutation(ZERO_REFERENCE_INTRA_SEED, |core| {
        let inter = core
            .inter
            .as_mut()
            .expect("fixture inter core has a control region");
        inter.num_total_refs = Some(0);
        inter.ref_frame_idx = RefIdxBuf::default();
    })
    .expect("a zero-reference inter frame may contain only intra-coded blocks");

    assert!(frame.y().samples().iter().all(|&sample| sample == 128));
    assert!(
        frame
            .u()
            .unwrap()
            .samples()
            .iter()
            .all(|&sample| sample == 128)
    );
    assert!(
        frame
            .v()
            .unwrap()
            .samples()
            .iter()
            .all(|&sample| sample == 128)
    );
}

#[test]
fn zero_reference_inter_frame_reaches_the_inter_block_boundary() {
    let error = decode_inter_frame_after_core_mutation(TWO_FRAME_INTER_FIXTURE, |core| {
        let inter = core
            .inter
            .as_mut()
            .expect("fixture inter core has a control region");
        inter.num_total_refs = Some(0);
        inter.ref_frame_idx = RefIdxBuf::default();
    })
    .expect_err("the fixture still codes an inter block without a reference");

    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed tile syntax must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
    assert!(matches!(
        &error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == crate::DecodeSourceIssueKind::TilePayloadParseError
            && issue.spec_section() == Some(super::super::SPEC_MODE_INFO)
            && issue.message()
                == "reference-list index 0 is outside the active 0-entry reference map"
    ));
}

#[test]
fn invalid_inter_reference_maps_are_typed_header_state_errors() {
    type Mutation = fn(&mut splot_core::headers::frame::InterControl);
    let mutations: [Mutation; 3] = [
        |inter| inter.num_total_refs = None,
        |inter| inter.num_total_refs = Some(8),
        |inter| {
            inter.num_total_refs = Some(0);
            inter.ref_frame_idx = [0].into_iter().collect();
        },
    ];
    for mutate in mutations {
        let error = decode_inter_frame_after_core_mutation(TWO_FRAME_INTER_FIXTURE, |core| {
            mutate(
                core.inter
                    .as_mut()
                    .expect("fixture inter core has a control region"),
            );
        })
        .expect_err("an invalid reference map must be rejected");

        assert!(matches!(
            error,
            DecodeError::HeaderState {
                source: DecodeHeaderStateError::InvalidInterReferenceMap,
            }
        ));
    }
}

#[test]
fn ccso_reference_slot_checks_all_reuse_modes_and_num_total_refs() {
    let offset = ByteOffset::new(74);
    assert_eq!(
        ccso_reference_slot(&[3], true, false, 0, offset, Some(2))
            .expect("in-range CCSO reference index resolves its slot"),
        Some(3)
    );
    assert_eq!(
        ccso_reference_slot(&[3], false, true, 0, offset, Some(2))
            .expect("block-reuse-only CCSO also resolves its slot"),
        Some(3)
    );
    assert_eq!(
        ccso_reference_slot(&[], false, false, 7, offset, Some(2))
            .expect("a plane without either reuse mode has no reference"),
        None
    );
    assert_eq!(
        ccso_reference_slot(&[3, 4, 5, 6, 7], true, false, 4, offset, Some(2))
            .expect("the last in-range CCSO reference index resolves its slot"),
        Some(7)
    );

    for ref_index in [5, 7] {
        let error = ccso_reference_slot(&[3, 4, 5, 6, 7], false, true, ref_index, offset, Some(2))
            .expect_err("block-reuse-only CCSO index must be less than NumTotalRefs");
        let DecodeError::MalformedSource { issue } = &error else {
            panic!("out-of-range CCSO reference index must be malformed source, got {error}");
        };
        assert_eq!(
            issue.kind(),
            crate::DecodeSourceIssueKind::FrameHeaderConformanceError
        );
        assert_eq!(issue.spec_section(), Some("6.17.7.8"));
        assert_eq!(issue.offset(), Some(offset));
        assert_eq!(issue.frame_index(), Some(2));
        assert_eq!(
            issue.message(),
            format!("ccso_ref_idx {ref_index} must be less than NumTotalRefs 5")
        );
        let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
            .expect("invalid CCSO reference index must remain user-reportable");
        assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
        assert!(matches!(
            report.details,
            crate::DecodeDiagnosticDetails::MalformedSource(_)
        ));
    }
}
