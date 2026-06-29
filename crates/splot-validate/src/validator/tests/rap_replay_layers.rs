// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn rap_replay_mfh_reference_blanket_suppressed_under_any_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = td_and_seq(0);
    data.extend(multi_frame_header_obu(0)); // mfhId 1 -> seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // references the MFH, not resent
    let baseline = Validator::new(false).validate_bytes(&data);
    assert!(
        baseline
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("multi-frame header")),
        "the MFH replay must fire under Disabled; report was: {baseline}"
    );
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(31, 0),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("multi-frame header")),
        "an inexpressible MFH kind must be blanket-suppressed under any Provided mode; \
         report was: {report}"
    );
}

#[test]
fn rap_replay_global_reference_in_post_rap_leading_tu_is_moot() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2)); // OPS (31, 0) defined in TU0 only
    data.extend(seq_obu(3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // seq resent in the RAP TU -> seq replay silent
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK random access point (xlayer 0)
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 3)); // leading frame -> TU2 leading
    data.extend(brt_dependent_obu(31, 0, 2)); // global BRT references OPS (31, 0)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a global reference in a post-random-access leading temporal unit is moot \
         (§ 7.3.8.1 drops the whole temporal unit); report was: {report}"
    );
}

#[test]
fn rap_replay_global_reference_in_non_leading_tu_still_fires() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2));
    data.extend(seq_obu(3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 3)); // regular -> TU2 not leading
    data.extend(brt_dependent_obu(31, 0, 2));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| { d.rule_id == RAP_RULE && d.message.contains("operating point set") }),
        "a global OPS reference in a decoded (non-leading) post-random-access temporal \
         unit still fires; report was: {report}"
    );
}

#[test]
fn rap_replay_global_resend_in_mixed_leading_tu_not_visible_to_earlier_anchor() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1010, None)); // global LCR id 5; xlayers 1 and 3 in map
    data.extend(sequence_header_obu_with_lcr(3, 5)); // seq(0, seq_lcr_id 5)@xlayer 3
    data.extend(seq_obu_layer(9, 3)); // a no-LCR seq(9)@xlayer 3 for the RAP frame ref
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(9, 3)); // seq(9) resent -> CLK's own reference satisfied
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 9)); // CLK@xlayer 3 -> R3 = TU1
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b1010, None)); // global LCR resent in the mixed TU
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 9)); // CLK@xlayer 1 -> RAP for xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 3, 9)); // LEADING frame -> xlayer 3 leading
    data.extend(temporal_delimiter_obu());
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.message.contains("global layer configuration record")
                && d.message.contains("lcr_global_config_record_id 5")
        }),
        "a global resend in a mixed leading temporal unit must not be visible to an \
         EARLIER anchor whose start drops that temporal unit; report was: {report}"
    );
}

#[test]
fn rap_replay_global_resend_in_mixed_leading_tu_visible_to_anchor_at_that_tu() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b0010, None)); // global LCR id 5; xlayer 1 in map
    data.extend(seq_obu_layer(9, 1)); // a no-LCR seq(9)@xlayer 1 for the RAP frame ref
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b0010, None)); // global LCR resent in the RAP's own TU
    data.extend(seq_obu_layer(9, 1)); // seq(9) resent -> CLK's own seq reference satisfied
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 9)); // CLK@xlayer 1 -> RAP at TU1
    data.extend(sequence_header_obu_with_lcr(1, 5)); // seq_lcr_id 5 @xlayer 1 -> § 7.3.8.3 ref
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 3, 9)); // LEADING frame -> xlayer 3 leading
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a global resend in the random access point's own (mixed leading) temporal unit \
         is visible to that anchor (clause (a)); report was: {report}"
    );
}

#[test]
fn rap_replay_resend_by_undecodable_other_layer_does_not_satisfy_reference() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(7, 1)); // resend by xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 7)); // regular xlayer-1 frame
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| { d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7") }),
        "a resend by an other layer that is not decoded under start-at-M's-anchor must \
         not satisfy M's reference (§ 7.4.6 sender-decodability); report was: {report}"
    );
}

#[test]
fn rap_replay_resend_by_decodable_other_layer_satisfies_reference() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(7, 1)); // seq(7) resent by xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 7)); // CLK@xlayer 1 -> RAP for xlayer 1
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a resend by a layer that random-accesses within [R, S.tu] is decoded under \
         start-at-R and satisfies the reference (§ 7.4.6); report was: {report}"
    );
}

#[test]
fn rap_replay_leading_tu_redefinition_does_not_invalidate_available_rap_version() {
    let mut data = td_and_seq(7);
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7)); // resent in the random access point's own temporal unit
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 7)); // CLK random access point at TU1
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7)); // "redefined" in a leading temporal unit
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 7)); // -> TU2 leading
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a leading-temporal-unit redefinition must NOT invalidate an available random-\
         access-point version (the availability check answers availability, not the \
         § 7.4.4 content divergence); report was: {report}"
    );
}

#[test]
fn rap_replay_leading_tu_redefinition_with_no_rap_version_still_fires() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK random access point at TU1
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7)); // resent only in a leading temporal unit
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 7)); // -> TU2 leading
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| { d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7") }),
        "a leading-temporal-unit (re)send must not satisfy a random-access reference when \
         no random-access-point version exists; report was: {report}"
    );
}

#[test]
fn rap_replay_resend_in_rap_tu_by_undecodable_other_layer_does_not_satisfy() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0 = TU1
    data.extend(seq_obu_layer(7, 1)); // seq(7) resent in TU1 ONLY by xlayer 1 (no RAP there)
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| { d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7") }),
        "a resend in the random access point's own temporal unit by a layer that does NOT \
         random-access there is not decoded under start-at-that-random-access-point \
         (§ 7.4.6); report was: {report}"
    );
}

#[test]
fn rap_replay_resend_in_rap_tu_by_layer_that_random_accesses_there_satisfies() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0 = TU1
    data.extend(seq_obu_layer(7, 1)); // seq(7) resent in TU1 by xlayer 1, before its CLK
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 7)); // CLK@xlayer 1 -> RAP for xlayer 1 at TU1
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a resend in the random access point's own temporal unit by a layer that DOES \
         random-access there is decoded and satisfies the reference; report was: {report}"
    );
}

#[test]
fn rap_replay_reference_must_satisfy_every_governing_anchor_not_just_newest() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the first CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R_a = TU2
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the second CLK's own reference is satisfied at R_b
    data.extend(seq_obu(7)); // seq(7) resent (clause (a) for R_b, invisible to R_a)
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R_b = TU3
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 3)); // + leading frame
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.message.contains("seq_header_id 7")
                && d.message.contains("random access point at temporal unit 2")
        }),
        "a reference satisfied by the newest anchor must still fire against an OLDER \
         governing anchor it cannot satisfy, naming that older start point (§ 7.3.8.1 \
         'any random access point'); report was: {report}"
    );
    assert!(
        !report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.message.contains("seq_header_id 7")
                && d.message.contains("random access point at temporal unit 3")
        }),
        "the newest anchor is satisfied and must not fire; report was: {report}"
    );
}

#[test]
fn rap_replay_non_leading_decodable_resend_covers_every_governing_anchor() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> first CLK's own reference satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R_a = TU1
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> second CLK's own reference satisfied
    data.extend(seq_obu(7)); // seq(7) resent in a NON-leading random-access temporal unit
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R_b = TU2 (non-leading)
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a non-leading decodable resend covers every governing anchor in [R_a, S.tu]; \
         report was: {report}"
    );
}

#[test]
fn rap_replay_sender_rap_in_post_anchor_leading_tu_does_not_enable_layer() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0 = TU2
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(3, 1)); // seq(3) resent by xlayer 1 (satisfies the CLK below)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 3)); // CLK@xlayer 1 -> RAP for xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 1, 3)); // + LEADING frame@xlayer 1
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(7, 1)); // resend by xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 7)); // regular xlayer-1 frame
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7")),
        "a sender layer whose only random access point sits in a post-anchor leading \
         temporal unit is not enabled under the earlier anchor, so its later resend cannot \
         satisfy the reference (§ 7.4.6 + § 7.3.8.1 leading drop); report was: {report}"
    );
}

#[test]
fn rap_replay_sender_rap_in_post_anchor_non_leading_tu_enables_layer() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0 = TU2
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(3, 1)); // seq(3) resent by xlayer 1 (satisfies the CLK below)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 3)); // CLK@xlayer 1 -> RAP for xlayer 1
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(7, 1)); // resend by xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 7)); // regular xlayer-1 frame
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a sender layer whose random access point temporal unit is non-leading is enabled \
         under the earlier anchor, so its later resend satisfies the reference (§ 7.4.6); \
         report was: {report}"
    );
}
