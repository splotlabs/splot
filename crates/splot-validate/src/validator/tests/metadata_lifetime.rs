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

/// Parses `data` as Annex B and observes every OBU through a fresh
/// [`ValidatorContext`], returning the context and the collected report. The
/// § 6.16.3 lifetime-store semantics gate no validate_bytes diagnostic (they
/// are decoder-applicability rules, not conformance requirements), so the
/// store scenarios assert state through the context's query surface.
pub(in crate::validator::tests) fn context_after_observing(
    data: &[u8],
) -> (ValidatorContext, ValidationReport) {
    use splot_core::annexb::parse_annex_b_obus;
    let obus = parse_annex_b_obus(data).unwrap_or_default();
    assert!(!obus.is_empty(), "the test stream must parse into OBUs");
    let options = ValidationOptions::default();
    let mut report = ValidationReport::new();
    let mut context = ValidatorContext::default();
    for obu in &obus {
        context.observe_obu(obu, &options, &mut report);
    }
    (context, report)
}

#[test]
fn basic_persistence_cancel_clears_active() {
    use splot_core::types::ExtendedLayerId;
    let x0 = ExtendedLayerId::from_bits(0);

    let mut data = temporal_delimiter_obu();
    data.extend(metadata_short_obu_at(0, 0x01, 1, &[0x12, 0x34, 0x56, 0x78])); // BASIC
    let (context, _) = context_after_observing(&data);
    assert_eq!(context.active_metadata_units(x0, 1).len(), 1);

    data.extend(metadata_short_obu_at(0, 0x08, 1, &[])); // cancel, type 1
    let (context, _) = context_after_observing(&data);
    assert!(
        context.active_metadata_units(x0, 1).is_empty(),
        "a cancel unit must clear the extended layer's BASIC record"
    );
}

#[test]
fn global_persistence_ignores_cancel() {
    use crate::metadata_lifetime::PersistenceMode;
    use splot_core::types::ExtendedLayerId;
    let x0 = ExtendedLayerId::from_bits(0);

    let mut data = temporal_delimiter_obu();
    data.extend(metadata_short_obu_at(0, 0x00, 1, &[0x12, 0x34, 0x56, 0x78])); // GLOBAL
    data.extend(metadata_short_obu_at(0, 0x08, 1, &[])); // cancel, type 1
    let (context, _) = context_after_observing(&data);
    let units = context.active_metadata_units(x0, 1);
    assert_eq!(
        units.len(),
        1,
        "a GLOBAL_PERSISTENCE record must survive the cancel"
    );
    assert_eq!(units[0].persistence, PersistenceMode::Global);
}

#[test]
fn global_persistence_overwrites_prior_global() {
    use splot_core::headers::metadata::MetadataPayload;
    use splot_core::types::ExtendedLayerId;
    let x0 = ExtendedLayerId::from_bits(0);

    let mut data = temporal_delimiter_obu();
    data.extend(metadata_short_obu_at(0, 0x00, 1, &[0x12, 0x34, 0x56, 0x78]));
    let second_obu_offset = data.len() as u64 + 1;
    data.extend(metadata_short_obu_at(0, 0x00, 1, &[0x99, 0x99, 0x56, 0x78]));
    let (context, _) = context_after_observing(&data);
    let units = context.active_metadata_units(x0, 1);
    assert_eq!(units.len(), 1, "the later GLOBAL record must overwrite");
    assert!(
        matches!(
            units[0].payload,
            MetadataPayload::HdrCll(cll) if cll.max_cll == 0x9999
        ),
        "the surviving record must carry the later content"
    );
    assert_eq!(
        units[0].offset.get(),
        second_obu_offset,
        "the surviving record must locate the later observation"
    );
}

#[test]
fn global_xlayer_cancel_clears_all_layers() {
    use splot_core::types::ExtendedLayerId;
    let x0 = ExtendedLayerId::from_bits(0);
    let x1 = ExtendedLayerId::from_bits(1);

    let mut data = temporal_delimiter_obu();
    data.extend(metadata_short_obu_at(0, 0x01, 1, &[0x12, 0x34, 0x56, 0x78]));
    data.extend(metadata_short_obu_at(1, 0x01, 1, &[0x12, 0x34, 0x56, 0x78]));
    data.extend(metadata_short_obu_at(31, 0x08, 1, &[])); // global cancel, type 1
    let (context, _) = context_after_observing(&data);
    assert!(context.active_metadata_units(x0, 1).is_empty());
    assert!(context.active_metadata_units(x1, 1).is_empty());
}

#[test]
fn no_persistence_expires_at_coded_frame() {
    use splot_core::types::ExtendedLayerId;
    let x0 = ExtendedLayerId::from_bits(0);

    let mut data = temporal_delimiter_obu();
    data.extend(metadata_short_obu_at(0, 0x02, 1, &[0x12, 0x34, 0x56, 0x78])); // NO
    data.extend(metadata_short_obu_at(0, 0x01, 0, &[0xDE, 0xAD])); // BASIC, type 0
    let (context, _) = context_after_observing(&data);
    assert_eq!(context.active_metadata_units(x0, 1).len(), 1);
    assert_eq!(context.active_metadata_units(x0, 0).len(), 1);

    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));
    let (context, _) = context_after_observing(&data);
    assert!(
        context.active_metadata_units(x0, 1).is_empty(),
        "a NO_PERSISTENCE record must expire at the coded frame"
    );
    assert_eq!(
        context.active_metadata_units(x0, 0).len(),
        1,
        "a BASIC record must survive the coded frame"
    );
}

#[test]
fn cvs_restart_drops_active_metadata() {
    use splot_core::headers::metadata::MetadataPayload;
    use splot_core::types::ExtendedLayerId;
    let x0 = ExtendedLayerId::from_bits(0);

    let mut data = temporal_delimiter_obu();
    data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x12, 0x34, 0x56, 0x78])); // layer_idc 1
    data.extend(temporal_delimiter_obu());
    data.extend(metadata_short_obu_at(0, 0x21, 1, &[0x99, 0x99, 0x56, 0x78])); // layer_idc 2

    let (context, _) = context_after_observing(&data);
    assert_eq!(context.active_metadata_units(x0, 1).len(), 2);

    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    let (context, _) = context_after_observing(&data);
    let units = context.active_metadata_units(x0, 1);
    assert_eq!(
        units.len(),
        1,
        "the CLK must drop earlier-temporal-unit records only"
    );
    assert!(
        matches!(
            units[0].payload,
            MetadataPayload::HdrCll(cll) if cll.max_cll == 0x9999
        ),
        "the same-temporal-unit record joins the new coded video sequence"
    );
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

/// Sequence-header payload (id 0, the given `max_tlayer_id`, max_mlayer_id 2)
/// with a signaled § 5.4.1 mlayer dependency map: rows 1..=2 in descending
/// refLayer order yield `[1][1]=1, [1][0]=0; [2][2]=1, [2][1]=0, [2][0]=1`.
/// Row `[1][0]` differs from the lower-triangular default fill, proving the
/// signaled map (not the default) is consulted. With `max_tlayer_id > 0` the
/// `tlayer_dependency_present_flag` bit is cleared, so `TLayerDependencyMap`
/// keeps the § 5.4.1 default fill (`refTLayer <= currTLayer`).
pub(in crate::validator::tests) fn sequence_header_payload_with_mlayer_deps(
    max_tlayer_id: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id
    bits.f(0, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(max_tlayer_id, 2); // max_tlayer_id
    bits.f(2, 3); // max_mlayer_id = 2
    bits.f(0, 2); // seq_max_mlayer_cnt_minus_1 (CeilLog2(3) = 2 bits)
    bits.bit(1); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1
    bits.f(7, 4); // max_frame_height_minus_1
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    bits.bit(1); // mlayer_dependency_present_flag (max_mlayer_id > 0)
    bits.bit(1);
    bits.bit(0);
    bits.bit(1);
    bits.bit(0);
    bits.bit(1);
    if max_tlayer_id > 0 {
        bits.bit(0); // tlayer_dependency_present_flag -> default fill
    }
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}

#[test]
fn propagation_respects_dependency_maps() {
    use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};
    let x0 = ExtendedLayerId::from_bits(0);
    let t0 = TemporalLayerId::from_bits(0);

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_mlayer_deps(0),
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(9, 0, 0, 0),
        &[
            0x00,        // is_suffix=0, necessity=0, application_id=0
            0x00,        // metadata_unit_cnt_minus_1 = 0
            0x01,        // metadata_type = HdrCll
            0x08,        // muh_header_size = 4, cancel = 0
            0x04,        // muh_payload_size = 4
            0x64,        // layer_idc=3 (LAYER_VALUES), persistence=1 (BASIC)
            0x00,        // priority lo + reserved bits
            0b0000_0110, // muh_mlayer_map: embedded layers 1 and 2
            0x12,
            0x34,
            0x56,
            0x78, // metadata_hdr_cll
            0x80, // OBU trailing byte
        ],
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(9, 0, 0, 0),
        &[0x00, 0x00, 0x00, 0x06, 0x00, 0x44, 0x00, 0x80],
    ));
    let (context, _) = context_after_observing(&data);

    let values_units = context.active_metadata_units(x0, 1);
    assert_eq!(values_units.len(), 1);
    let record = &values_units[0];
    assert!(context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(0), t0));
    assert!(context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(2), t0));
    assert!(
        !context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(1), t0),
        "the SIGNALED map, not the default fill, must be consulted"
    );

    let current_units = context.active_metadata_units(x0, 0);
    assert_eq!(current_units.len(), 1);
    let record = &current_units[0];
    assert!(!context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(2), t0));
    assert!(context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(0), t0));
}

#[test]
fn propagation_temporal_axis_and_explicit_indication_boundary() {
    use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};
    let x0 = ExtendedLayerId::from_bits(0);
    let m0 = EmbeddedLayerId::from_bits(0);
    let t0 = TemporalLayerId::from_bits(0);
    let t1 = TemporalLayerId::from_bits(1);

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_mlayer_deps(1),
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 1, 0, 0),
        &metadata_short_payload(0x01, 1, &[0x12, 0x34, 0x56, 0x78]),
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(9, 0, 0, 0),
        &[
            0x00,        // is_suffix=0, necessity=0, application_id=0
            0x00,        // metadata_unit_cnt_minus_1 = 0
            0x00,        // metadata_type = 0 (Reserved -> UnknownRaw, no unit bytes)
            0x08,        // muh_header_size = 4, cancel = 0
            0x00,        // muh_payload_size = 0
            0x64,        // layer_idc=3 (LAYER_VALUES), persistence=1 (BASIC)
            0x00,        // priority lo + reserved bits
            0b0000_0001, // muh_mlayer_map: embedded layer 0 only
            0x80,        // OBU trailing byte
        ],
    ));
    let (context, _) = context_after_observing(&data);

    let cll_units = context.active_metadata_units(x0, 1);
    assert_eq!(cll_units.len(), 1);
    let record = &cll_units[0];
    assert!(context.metadata_applies_to(x0, record, m0, t1));
    assert!(
        !context.metadata_applies_to(x0, record, m0, t0),
        "TLayerDependencyMap[K][C][T] must be consulted as [K][target][source]"
    );

    let values_units = context.active_metadata_units(x0, 0);
    assert_eq!(values_units.len(), 1);
    let record = &values_units[0];
    assert!(!context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(2), t0));
    assert!(context.metadata_applies_to(x0, record, m0, t0));
}

#[test]
fn propagation_requires_explicit_per_target_map_bit() {
    use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};
    let x0 = ExtendedLayerId::from_bits(0);
    let t0 = TemporalLayerId::from_bits(0);

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 2)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(9, 0, 0, 0),
        &[
            0x00,        // is_suffix=0, necessity=0, application_id=0
            0x00,        // metadata_unit_cnt_minus_1 = 0
            0x01,        // metadata_type = HdrCll
            0x08,        // muh_header_size = 4, cancel = 0
            0x04,        // muh_payload_size = 4
            0x64,        // layer_idc=3 (LAYER_VALUES), persistence=1 (BASIC)
            0x00,        // priority lo + reserved bits
            0b0000_0101, // muh_mlayer_map: embedded layers 0 and 2, NOT 1
            0x12,
            0x34,
            0x56,
            0x78, // metadata_hdr_cll
            0x80, // OBU trailing byte
        ],
    ));
    let (context, _) = context_after_observing(&data);

    let units = context.active_metadata_units(x0, 1);
    assert_eq!(units.len(), 1);
    let record = &units[0];
    assert!(context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(0), t0));
    assert!(
        context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(2), t0),
        "bit 2 of muh_mlayer_map is set and MLayerDependencyMap[2][0] is 1"
    );
    assert!(
        !context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(1), t0),
        "bit 1 of muh_mlayer_map is clear, so embedded layer 1 is never \
         explicitly signaled (§ 6.16.3) despite MLayerDependencyMap[1][0] == 1"
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

#[test]
fn metadata_truncated_observer_is_silent() {
    use splot_core::annexb::parse_annex_b_obus;
    use splot_core::types::GLOBAL_XLAYER_ID;

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &[0x00, 0x01],
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(9, 0, 0, 31),
        &[0x00],
    ));
    let obus = parse_annex_b_obus(&data).unwrap_or_default();
    assert_eq!(obus.len(), 3, "the test stream must parse into 3 OBUs");

    let options = ValidationOptions::default();
    let mut report = ValidationReport::new();
    let mut context = ValidatorContext::default();
    for obu in &obus {
        context.observe_obu(obu, &options, &mut report);
    }
    assert!(
        report.diagnostics.is_empty(),
        "the observer must not report parse failures: {report}"
    );
    assert!(
        context
            .active_metadata_units(GLOBAL_XLAYER_ID, 1)
            .is_empty()
    );
}
