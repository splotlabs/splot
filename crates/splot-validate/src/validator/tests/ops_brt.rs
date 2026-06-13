// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn ops_local_reserved_bits_nonzero_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu(2, false, 0, 1, 0b10, false, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "ops/local-reserved-bits-nonzero"),
        "report was: {report}"
    );
}

#[test]
fn ops_mlayer_info_idc_reserved_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_idc_obu(3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "ops/mlayer-info-idc-reserved"),
        "report was: {report}"
    );
}

#[test]
fn ops_ptl_reserved_bits_nonzero_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu(2, false, 0, 1, 0, true, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "ops/ptl-reserved-bits-nonzero"),
        "report was: {report}"
    );
}

#[test]
fn ops_payload_size_mismatch_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "ops/payload-size-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn ops_inherited_op_index_out_of_range_is_flagged() {
    // Same-OPS reference (embedded_ops_id == ops_id 0): op_index 5 >= ops_cnt 1
    // and >= j (layer 1).
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_inherited_obu(0, 0, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "ops/inherited-op-index-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn ops_cross_ops_inherited_op_index_out_of_range_is_flagged() {
    // OPS 0 inherits from a different, already-defined OPS 1 (ops_cnt 1) at
    // op_index 5, which is out of range — exercises the cross-OPS resolution
    // against the prior active OPS state.
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 1, 1)); // define OPS 1 (ops_cnt 1)
    data.extend(global_ops_inherited_obu(0, 1, 5)); // OPS 0 inherits OPS 1 op 5
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "ops/inherited-op-index-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn ops_cross_ops_inherited_op_index_in_range_is_not_flagged() {
    // OPS 0 inherits from OPS 1 (ops_cnt 3) at op_index 2, which is in range, so
    // the cross-OPS bound check must not flag it.
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 1, 3)); // define OPS 1 (ops_cnt 3)
    data.extend(global_ops_inherited_obu(0, 1, 2)); // OPS 0 inherits OPS 1 op 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/inherited-op-index-out-of-range"),
        0,
        "an in-range cross-OPS inheritance must not be flagged: {report}"
    );
}

#[test]
fn ops_reset_removes_active_ops() {
    // Define OPS 0 (cnt 2), reference it (resolves), reset it, reference again.
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2));
    data.extend(brt_dependent_obu(31, 0, 2)); // matches active ops_cnt 2
    data.extend(global_ops_obu(true, 0, 0)); // reset (cnt 0)
    data.extend(brt_dependent_obu(31, 0, 2)); // now unavailable
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "brt/unavailable-operating-point-set"),
        1,
        "expected exactly the post-reset BRT to be unavailable: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "brt/ops-count-mismatch"),
        0,
        "the pre-reset BRT must resolve and match: {report}"
    );
}

#[test]
fn ops_update_changes_active_ops_count() {
    // Define OPS 0 (cnt 2), match a BRT, then update to cnt 3 and re-reference.
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2));
    data.extend(brt_dependent_obu(31, 0, 2)); // matches cnt 2
    data.extend(global_ops_obu(false, 0, 3)); // update -> cnt 3
    data.extend(brt_dependent_obu(31, 0, 2)); // now mismatches cnt 3
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "brt/ops-count-mismatch"),
        1,
        "only the post-update BRT should mismatch: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "brt/unavailable-operating-point-set"),
        0,
        "the OPS stays available across the update: {report}"
    );
}

#[test]
fn brt_ops_count_mismatch_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2));
    data.extend(brt_dependent_obu(31, 0, 3)); // br_ops_cnt 3 != ops_cnt 2
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "brt/ops-count-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn brt_missing_ops_is_flagged_when_external_hls_disabled() {
    let mut data = temporal_delimiter_obu();
    data.extend(brt_dependent_obu(31, 5, 1)); // no OPS defined
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "brt/unavailable-operating-point-set"),
        "report was: {report}"
    );
}

#[test]
fn brt_missing_ops_is_not_hard_error_when_external_ops_declared() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(brt_dependent_obu(31, 5, 1)); // no in-band OPS (31, 5)
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(31, 5),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert_eq!(
        ops_error_count(&report, "brt/unavailable-operating-point-set"),
        0,
        "a declared external OPS must suppress the hard missing-OPS error: {report}"
    );
}

#[test]
fn brt_missing_ops_is_flagged_when_external_hls_lacks_the_ops() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(brt_dependent_obu(31, 5, 1)); // references OPS (31, 5)
    // External HLS is provided but only declares a sequence header and a
    // different OPS, so OPS (31, 5) is still unavailable and must be flagged.
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new()
                .with_sequence_header_id(0)
                .with_operating_point_set(31, 4),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "brt/unavailable-operating-point-set"),
        "an external HLS set that does not declare the referenced OPS must still flag it: \
         {report}"
    );
}

#[test]
fn ops_malformed_payload_is_flagged() {
    // A global OPS header claims ops_cnt=1 but the payload ends immediately, so
    // the header fields cannot be read. The OPS syntax check must report it.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(18, 0, 0, 31),
        &[0x01],
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "bitstream/parse-error"),
        "a malformed OPS payload must be reported: {report}"
    );
}

#[test]
fn brt_malformed_payload_is_flagged() {
    // br_ops_dependent_flag=1, br_ops_id=0, br_ops_cnt=1 fills the single payload
    // byte, so the per-op flag read runs past the input. The BRT syntax check
    // must report it.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(15, 0, 0, 31),
        &[0x81],
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "bitstream/parse-error"),
        "a malformed BRT payload must be reported: {report}"
    );
}

#[test]
fn global_brt_before_coded_layers_is_accepted() {
    // A global BRT before any coded extended layer unit raises no ordering error.
    let mut data = temporal_delimiter_obu();
    data.extend(brt_extended_layer_obu(31)); // global BRT, extended-layer form
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // a coded-layer OBU
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "obu-order/global-hls-after-coded-layer"),
        0,
        "a global BRT before coded layers must be accepted: {report}"
    );
}

#[test]
fn global_brt_after_coded_layer_is_not_flagged() {
    // § 7.3.7 does not list BRT among global prefix OBUs and § 7.3.3/§ 7.3.4 place
    // it in coded frame units, so a global BRT after a coded layer is left
    // unclassified rather than flagged (sound-over-complete; see
    // is_global_hls_prefix_obu).
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // coded-layer OBU
    data.extend(brt_extended_layer_obu(31)); // global BRT after the coded layer
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "obu-order/global-hls-after-coded-layer"),
        0,
        "a global BRT after a coded layer is not flagged in this phase: {report}"
    );
}

#[test]
fn local_brt_follows_coded_layer_classification() {
    // A local BRT is a coded extended layer OBU (§ 7.3.3/§ 7.3.4): it starts the
    // coded-layer phase, so a later global OPS prefix is flagged out of order.
    let mut data = temporal_delimiter_obu();
    data.extend(brt_extended_layer_obu(2)); // local BRT -> coded extended layer unit
    data.extend(global_ops_obu(false, 0, 1)); // global OPS prefix after a coded layer
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/global-hls-after-coded-layer"),
        "a local BRT must start the coded-layer phase: {report}"
    );
}
