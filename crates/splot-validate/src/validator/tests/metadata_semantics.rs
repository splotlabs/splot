// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Builds a `metadata_decoded_frame_hash()` short OBU payload (type 5) with a single
/// frame hash (per_plane 0) and the given reserved bit.
pub(in crate::validator::tests) fn frame_hash_payload(reserved: u8) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 4); // hash_type = 0 (MD5)
    bits.bit(0); // per_plane = 0 -> single frame_hash
    bits.bit(0); // has_grain
    bits.bit(0); // is_monochrome
    bits.bit(reserved); // reserved
    for _ in 0..16 {
        bits.f(0, 8); // frame_hash bytes
    }
    bits.align();
    metadata_short_payload(0x00, 5, &bits.into_bytes())
}

#[test]
fn metadata_decoded_frame_hash_reserved_nonzero_is_warned() {
    let report =
        Validator::new(false).validate_bytes(&global_metadata_short_stream(&frame_hash_payload(1)));
    assert!(
        has_warning(&report, "metadata/decoded-frame-hash-reserved-nonzero"),
        "report was: {report}"
    );
}

#[test]
fn metadata_decoded_frame_hash_reserved_zero_is_silent() {
    let report =
        Validator::new(false).validate_bytes(&global_metadata_short_stream(&frame_hash_payload(0)));
    assert!(
        !has_warning(&report, "metadata/decoded-frame-hash-reserved-nonzero"),
        "report was: {report}"
    );
}

#[test]
fn metadata_scan_type_pic_struct_reserved_is_flagged() {
    let payload = metadata_short_payload(0x00, 8, &[0x68]);
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        has_error(&report, "metadata/scan-type-pic-struct-reserved"),
        "report was: {report}"
    );
}

#[test]
fn metadata_valid_short_is_accepted() {
    let payload = [0x08, 0x04, 0x80]; // cancel=1, type=4
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("metadata/")),
        "report was: {report}"
    );
}

/// A metadata group payload with one non-cancel unit of reserved
/// `metadata_type` 0 (no unit payload bytes) and the given `muh_layer_idc` /
/// `muh_persistence_idc`. `muh_layer_idc` must not be `LAYER_VALUES` (3),
/// which would add layer-map bytes.
pub(in crate::validator::tests) fn group_unit_payload(
    layer_idc: u8,
    persistence_idc: u8,
) -> Vec<u8> {
    assert_ne!(layer_idc, 3, "LAYER_VALUES would require layer-map bytes");
    vec![
        0x00, // is_suffix=0, necessity=0, application_id=0
        0x00, // metadata_unit_cnt_minus_1 = 0
        0x00, // metadata_type = 0 (Reserved -> UnknownRaw, no unit bytes)
        0x06, // muh_header_size = 3, cancel = 0
        0x00, // muh_payload_size = 0
        (layer_idc << 5) | ((persistence_idc & 0x07) << 2),
        0x00, // muh_priority lo 6 bits + muh_reserved_zero_2bits
        0x80, // OBU trailing byte
    ]
}

#[test]
fn metadata_persistence_reserved_idc_warns() {
    let short = metadata_short_payload(0x04, 1, &[0x12, 0x34, 0x56, 0x78]);
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&short));
    assert!(
        has_warning(&report, "metadata/persistence-idc-reserved"),
        "report was: {report}"
    );
    assert!(
        report.is_conformant(),
        "a reserved value is a warning, not an error: {report}"
    );

    let report = Validator::new(false)
        .validate_bytes(&global_metadata_group_stream(&group_unit_payload(0, 4)));
    assert!(
        has_warning(&report, "metadata/persistence-idc-reserved"),
        "report was: {report}"
    );
    assert!(report.is_conformant(), "report was: {report}");
}

#[test]
fn metadata_persistence_defined_idc_is_accepted() {
    let short = metadata_short_payload(0x01, 1, &[0x12, 0x34, 0x56, 0x78]);
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&short));
    assert!(
        !has_warning(&report, "metadata/persistence-idc-reserved"),
        "report was: {report}"
    );

    let report = Validator::new(false)
        .validate_bytes(&global_metadata_group_stream(&group_unit_payload(0, 1)));
    assert!(
        !has_warning(&report, "metadata/persistence-idc-reserved"),
        "report was: {report}"
    );

    let cancelled = [0x0C, 0x01, 0x80];
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&cancelled));
    assert!(
        !has_warning(&report, "metadata/persistence-idc-reserved"),
        "a cancel unit must not warn about its persistence bits: {report}"
    );
}

#[test]
fn metadata_group_layer_idc_reserved_warns() {
    let report = Validator::new(false)
        .validate_bytes(&global_metadata_group_stream(&group_unit_payload(4, 1)));
    assert!(
        has_warning(&report, "metadata/group-layer-idc-reserved"),
        "report was: {report}"
    );
    assert!(report.is_conformant(), "report was: {report}");
}

#[test]
fn metadata_group_layer_idc_defined_values_accepted() {
    let report = Validator::new(false)
        .validate_bytes(&global_metadata_group_stream(&group_unit_payload(2, 1)));
    assert!(
        !has_warning(&report, "metadata/group-layer-idc-reserved"),
        "report was: {report}"
    );
}

/// A short metadata OBU with an extension header at `obu_xlayer_id == xlayer`
/// (tlayer / mlayer 0) carrying `metadata_short_payload(first, metadata_type,
/// unit)`.
pub(in crate::validator::tests) fn metadata_short_obu_at(
    xlayer: u8,
    first: u8,
    metadata_type: u8,
    unit: &[u8],
) -> Vec<u8> {
    annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, xlayer),
        &metadata_short_payload(first, metadata_type, unit),
    )
}

#[test]
fn cancel_unknown_type_emits_nothing() {
    let report =
        Validator::new(false).validate_bytes(&global_metadata_short_stream(&[0x08, 0x05, 0x80]));
    assert!(report.is_conformant(), "report was: {report}");
    assert_eq!(report.warnings().count(), 0, "report was: {report}");
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule_id.starts_with("metadata/")),
        "report was: {report}"
    );
}

#[test]
fn metadata_truncated_observer_is_silent() {
    use splot_core::annexb::parse_annex_b_obus;

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &[0x00, 0x01],
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(9, 0, 0, 31),
        &[0x00],
    ));

    let validation = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&validation, "metadata/unit-payload-underflow"),
        "the validator must preserve the malformed short-unit error: {validation}"
    );
    assert!(
        validation
            .errors()
            .any(|diagnostic| diagnostic.rule_id == "bitstream/parse-error"),
        "the validator must preserve the truncated group parse error: {validation}"
    );
    assert!(
        !validation.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.rule_id.as_str(),
                "metadata/hdr-cll-repeat-content-differs"
                    | "metadata/hdr-mdcv-repeat-content-differs"
                    | "metadata/hdr-cll-first-coded-picture"
                    | "metadata/hdr-mdcv-first-coded-picture"
            )
        }),
        "malformed metadata must not create stateful semantic false positives: {validation}"
    );

    let obus = parse_annex_b_obus(&data).unwrap_or_default();
    assert_eq!(obus.len(), 3, "the test stream must parse into three OBUs");
    let options = ValidationOptions::default();
    let mut observer_report = ValidationReport::new();
    let mut context = ValidatorContext::default();
    for obu in &obus {
        context.observe_obu(obu, &options, &mut observer_report);
    }
    assert!(
        observer_report.diagnostics.is_empty(),
        "the state observer must leave malformed-payload reporting to the validator checks: \
         {observer_report}"
    );
}

/// A 24-byte `metadata_hdr_mdcv()` unit (§ 5.17.6) with fixed chromaticities
/// and the given `luminance_min`.
pub(in crate::validator::tests) fn hdr_mdcv_unit(luminance_min: u32) -> Vec<u8> {
    let mut unit = Vec::new();
    for v in [10u16, 20, 30, 40, 50, 60, 70, 80] {
        unit.extend_from_slice(&v.to_be_bytes());
    }
    unit.extend_from_slice(&1_000_000u32.to_be_bytes());
    unit.extend_from_slice(&luminance_min.to_be_bytes());
    unit
}

#[test]
fn hdr_cll_repeat_same_content_accepted() {
    let unit = [0x12, 0x34, 0x56, 0x78];
    let mut data = global_metadata_short_stream(&metadata_short_payload(0x11, 1, &unit));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(0x11, 1, &unit),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "report was: {report}"
    );
}

#[test]
fn hdr_cll_repeat_content_differs_flagged() {
    let mut data =
        global_metadata_short_stream(&metadata_short_payload(0x11, 1, &[0x12, 0x34, 0x56, 0x78]));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(0x11, 1, &[0x99, 0x99, 0x56, 0x78]),
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(0x01, 1, &[0x00, 0x01, 0x56, 0x78]),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "metadata/hdr-cll-repeat-content-differs")
            .count(),
        1,
        "exactly the intersecting differing repeat must be flagged: {report}"
    );
}

#[test]
fn hdr_mdcv_repeat_content_differs_flagged() {
    let mut data =
        global_metadata_short_stream(&metadata_short_payload(0x11, 2, &hdr_mdcv_unit(5)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(0x11, 2, &hdr_mdcv_unit(9)),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/hdr-mdcv-repeat-content-differs"),
        "report was: {report}"
    );
}

#[test]
fn hdr_cll_after_cvs_restart_new_content_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x12, 0x34, 0x56, 0x78]));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x99, 0x99, 0x56, 0x78]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "report was: {report}"
    );
}

#[test]
fn hdr_cll_global_then_current_layer_differing_content_flagged() {
    let mut data =
        global_metadata_short_stream(&metadata_short_payload(0x11, 1, &[0x12, 0x34, 0x56, 0x78]));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(metadata_short_obu_at(0, 0x21, 1, &[0x99, 0x99, 0x56, 0x78]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "a LAYER_CURRENT unit shares its embedded layer with the global \
         LAYER_GLOBAL baseline; report was: {report}"
    );
}

#[test]
fn hdr_cll_global_then_current_layer_same_content_accepted() {
    let unit = [0x12, 0x34, 0x56, 0x78];
    let mut data = global_metadata_short_stream(&metadata_short_payload(0x11, 1, &unit));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(metadata_short_obu_at(0, 0x21, 1, &unit));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "report was: {report}"
    );
}

#[test]
fn hdr_cll_unspecified_layer_targeting_not_compared() {
    let mut data =
        global_metadata_short_stream(&metadata_short_payload(0x01, 1, &[0x12, 0x34, 0x56, 0x78]));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(0x01, 1, &[0x99, 0x99, 0x56, 0x78]),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "LAYER_UNSPECIFIED units have no derivable association; report was: {report}"
    );
}

#[test]
fn hdr_cll_cross_mode_deferral_dropped_at_clk() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(0x11, 1, &[0x12, 0x34, 0x56, 0x78]),
    ));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(temporal_delimiter_obu());
    data.extend(metadata_short_obu_at(0, 0x21, 1, &[0x99, 0x99, 0x56, 0x78]));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "the CLK puts the two units in different coded video sequences; \
         report was: {report}"
    );
}

/// A global (xlayer 31) group metadata OBU with one HDR CLL unit using
/// `muh_layer_idc == LAYER_VALUES` explicit targeting: `muh_xlayer_map` is
/// `xlayer_map` and one `muh_mlayer_map` byte (embedded layer 1) is emitted per
/// selected extended layer (AV2 § 5.17.3).
pub(in crate::validator::tests) fn global_group_cll_obu(
    xlayer_map: u32,
    content: [u8; 4],
) -> Vec<u8> {
    let map_count = xlayer_map.count_ones() as u8; // <= 32
    let mut payload = vec![
        0x00,                         // is_suffix=0, necessity=0, application_id=0
        0x00,                         // metadata_unit_cnt_minus_1 = 0
        0x01,                         // metadata_type = HdrCll
        (1 + 2 + 4 + map_count) << 1, // cancel = 0
        0x04,                         // muh_payload_size = 4
        0x64,                         // layer_idc=3 (LAYER_VALUES), persistence=1 (BASIC)
        0x00,                         // priority lo + reserved bits
    ];
    payload.extend_from_slice(&xlayer_map.to_be_bytes()); // muh_xlayer_map
    payload.extend(std::iter::repeat_n(0b0000_0010u8, usize::from(map_count)));
    payload.extend_from_slice(&content); // metadata_hdr_cll()
    payload.push(0x80); // OBU trailing byte
    annex_b_obu_with_header(&layer_obu_header(9, 0, 0, 31), &payload)
}

#[test]
fn hdr_cll_global_group_disjoint_layer_targeting_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_group_cll_obu(0b01, [0x12, 0x34, 0x56, 0x78]));
    data.extend(global_group_cll_obu(0b10, [0x99, 0x99, 0x56, 0x78]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "disjoint single-xlayer global targeting must not be compared; \
         report was: {report}"
    );

    let mut data = temporal_delimiter_obu();
    data.extend(global_group_cll_obu(0b0011, [0x12, 0x34, 0x56, 0x78]));
    data.extend(global_group_cll_obu(0b1100, [0x99, 0x99, 0x56, 0x78]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "disjoint multi-xlayer global targeting must not be compared; \
         report was: {report}"
    );
}

#[test]
fn hdr_cll_global_group_overlapping_layer_targeting_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_group_cll_obu(0b01, [0x12, 0x34, 0x56, 0x78]));
    data.extend(global_group_cll_obu(0b11, [0x99, 0x99, 0x56, 0x78]));
    let report = Validator::new(false).validate_bytes(&data);
    let finding = report
        .errors()
        .find(|d| d.rule_id == "metadata/hdr-cll-repeat-content-differs");
    assert!(
        finding.is_some_and(|finding| finding
            .message
            .contains("obu_xlayer_id 0 / obu_mlayer_id 1")),
        "overlapping global LAYER_VALUES targeting must be compared and the \
         finding must name a shared embedded layer; report was: {report}"
    );
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "metadata/hdr-cll-repeat-content-differs")
            .count(),
        1,
        "one differing baseline yields one finding; report was: {report}"
    );
}

#[test]
fn hdr_cll_cross_tu_repeat_differs_flagged_at_flush() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x12, 0x34, 0x56, 0x78]));
    data.extend(temporal_delimiter_obu());
    data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x99, 0x99, 0x56, 0x78]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
        "a cross-temporal-unit repeat without a CLK stays in the same coded \
         video sequence and must be flagged; report was: {report}"
    );
}
