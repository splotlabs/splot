// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::{ExternalHlsError, ExternalHlsMode, ExternalHlsObjects};

fn external_options(bytes: &[u8]) -> ValidationOptions {
    ValidationOptions {
        external_hls: ExternalHlsMode::Objects(ExternalHlsObjects::from_annex_b(bytes).unwrap()),
    }
}

#[test]
fn external_sequence_header_contents_apply_when_a_frame_activates_the_key() {
    let mut stream = temporal_delimiter_obu();
    stream.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    stream.extend(annex_b_obu_with_header(&layer_obu_header(6, 2, 0, 0), &[]));
    let validator = Validator::new(false);

    for (limit, exceeds_maximum) in [(1, true), (2, false)] {
        let context = sequence_header_obu_for_xlayer(0, limit, 0);
        let options = external_options(&context);
        let report = validator.validate_bytes_with_options(&stream, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
            "{report}"
        );
        assert_eq!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
            exceeds_maximum,
            "{report}"
        );
        let reader_report = validator
            .validate_reader_with_options(stream.as_slice(), &options)
            .unwrap();
        assert_eq!(reader_report.to_string(), report.to_string());
    }
}

#[test]
fn external_sequence_header_has_an_exact_layer_key() {
    let mut stream = temporal_delimiter_obu();
    stream.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 0));
    let options = external_options(&sequence_header_obu_for_xlayer(0, 0, 0));
    let report = Validator::new(false).validate_bytes_with_options(&stream, &options);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "{report}"
    );
}

#[test]
fn in_band_sequence_header_replaces_external_definition() {
    let mut stream = td_and_seq_header(0, 2, 0);
    stream.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    stream.extend(annex_b_obu_with_header(&layer_obu_header(6, 2, 0, 0), &[]));
    let options = external_options(&sequence_header_obu_for_xlayer(0, 0, 0));
    let report = Validator::new(false).validate_bytes_with_options(&stream, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
        "{report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
        "{report}"
    );
}

#[test]
fn external_operating_point_set_supplies_count_until_replacement_or_reset() {
    let options = external_options(&global_ops_obu(false, 3, 2));
    let reference = brt_dependent_obu(31, 3, 3);
    let mut stream = temporal_delimiter_obu();
    stream.extend(&reference);
    stream.extend(global_ops_obu(false, 3, 3));
    stream.extend(&reference);
    stream.extend(global_ops_obu(false, 3, 0));
    stream.extend(&reference);

    let report = Validator::new(false).validate_bytes_with_options(&stream, &options);
    let counts: Vec<_> = report
        .errors()
        .filter(|d| d.rule_id == "brt/ops-count-mismatch")
        .collect();
    assert_eq!(counts.len(), 1, "{report}");
    assert!(counts[0].message.contains("supplied externally"));
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "brt/unavailable-operating-point-set")
            .count(),
        1,
        "{report}"
    );
}

#[test]
fn external_snapshot_checks_complete_objects_and_exact_unique_keys() {
    let first = sequence_header_obu_for_xlayer(0, 0, 0);
    let second = sequence_header_obu_for_xlayer(1, 0, 0);
    let ops = global_ops_obu(false, 3, 2);
    let combined = [second.as_slice(), ops.as_slice(), first.as_slice()].concat();
    let objects = ExternalHlsObjects::from_annex_b(&combined).unwrap();
    assert_eq!(
        objects
            .sequence_headers()
            .map(|(xlayer, id)| (xlayer.get(), id.get()))
            .collect::<Vec<_>>(),
        [(0, 0), (1, 0)]
    );
    assert_eq!(
        objects
            .operating_point_sets()
            .map(|(xlayer, id)| (xlayer.get(), id))
            .collect::<Vec<_>>(),
        [(31, 3)]
    );
    assert!(matches!(
        ExternalHlsObjects::from_annex_b(&[]),
        Err(ExternalHlsError::Empty)
    ));
    assert!(matches!(
        ExternalHlsObjects::from_annex_b(&[first.as_slice(), first.as_slice()].concat()),
        Err(ExternalHlsError::Duplicate { .. })
    ));
    assert!(matches!(
        ExternalHlsObjects::from_annex_b(&first[..first.len() - 1]),
        Err(ExternalHlsError::Syntax(_))
    ));
    assert!(matches!(
        ExternalHlsObjects::from_annex_b(&local_ops_obu(2, false, 0, 1, 2, false, 0)),
        Err(ExternalHlsError::InvalidObject(diagnostic)) if diagnostic.rule_id == "ops/local-reserved-bits-nonzero"
    ));
    assert!(matches!(
        ExternalHlsObjects::from_annex_b(&global_ops_obu(false, 3, 0)),
        Err(ExternalHlsError::Unsupported { .. })
    ));
    assert!(matches!(
        ExternalHlsObjects::from_annex_b(&global_ops_inherited_obu(0, 1, 0)),
        Err(ExternalHlsError::Unsupported { .. })
    ));
}

#[test]
fn external_sequence_header_replacement_checks_identity_within_the_coded_video_sequence() {
    let options = external_options(&sequence_header_obu_for_xlayer(0, 0, 0));
    for (frame_type, expected_finding) in [(4, false), (7, true)] {
        let mut stream = temporal_delimiter_obu();
        stream.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        stream.extend(td_and_seq_header(0, 2, 0));
        stream.extend(frame_obu_direct_seq_ref_layer(frame_type, 0, 0, 0, 0));
        let report = Validator::new(false).validate_bytes_with_options(&stream, &options);
        assert_eq!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            expected_finding,
            "{report}"
        );
    }
}

#[test]
fn external_snapshot_does_not_suppress_unrelated_random_access_findings() {
    let mut stream = td_and_seq(3);
    stream.extend(temporal_delimiter_obu());
    stream.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    for (id, expected_finding) in [(3, false), (4, true)] {
        let context = annex_b_obu(0x04, &sequence_header_payload_with_id(id, 1, 1));
        let options = external_options(&context);
        let report = Validator::new(false).validate_bytes_with_options(&stream, &options);
        assert_eq!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-at-random-access-point"),
            expected_finding,
            "{report}"
        );
    }
}

#[test]
fn external_sequence_header_supplies_state_for_avm_fixture() {
    use splot_core::annexb::parse_annex_b_obus;
    use splot_core::ivf::parse_ivf_partial;
    use splot_core::types::ObuType;

    let fixture = include_bytes!(
        "../../../../../tests/conformance/vectors/valid/syn-3frame-padded-filters-422-134x69-10bit-q150.ivf"
    );
    let input = parse_ivf_partial(fixture);
    assert!(input.error.is_none());
    let mut stream: Vec<_> = input
        .frames
        .into_iter()
        .flat_map(|frame| frame.payload.iter().copied())
        .collect();
    let mut start = 0;
    let sequence_range = parse_annex_b_obus(&stream)
        .unwrap()
        .into_iter()
        .find_map(|obu| {
            let end = usize::try_from(obu.offset.get() + u64::from(obu.size)).unwrap();
            let range = core::mem::replace(&mut start, end)..end;
            (obu.header.obu_type == ObuType::SequenceHeader).then_some(range)
        })
        .unwrap();
    let options = external_options(&stream[sequence_range.clone()]);
    stream.drain(sequence_range);

    let validator = Validator::new(false);
    let without_context = validator.validate_bytes(&stream);
    assert!(
        without_context
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "{without_context}"
    );
    let execution = validator.validate_reader_with_evidence(
        stream.as_slice(),
        &options,
        ValidationLimits {
            max_unit_bytes: 8192,
            max_obus: 64,
            max_diagnostics: 64,
        },
        || false,
    );
    assert!(matches!(
        execution.completion,
        ValidationCompletion::EndOfInput
    ));
    assert_eq!(execution.counts.errors, 0, "{}", execution.report);
}
