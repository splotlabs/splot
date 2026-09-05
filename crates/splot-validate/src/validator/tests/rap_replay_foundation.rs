// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn rap_replay_sequence_header_only_before_rap_is_flagged() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.spec_section.as_deref() == Some("7.3.8.1")),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_resend_in_rap_temporal_unit_passes() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent in the random access point's temporal unit
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_olk_random_access_point_is_flagged() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(OLK_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_ras_random_access_point_is_flagged() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(RAS_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_resend_after_reference_in_same_temporal_unit_is_flagged() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // reference precedes the resend
    data.extend(seq_obu(3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_no_random_access_point_does_not_fire() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_leading_temporal_unit_resend_does_not_qualify() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7));
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 7));
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
    assert!(
        report
            .errors()
            .filter(|d| d.rule_id == RAP_RULE)
            .all(|d| d.message.contains("seq_header_id 7")),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_non_leading_resend_after_rap_qualifies() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7));
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_deduplicates_per_object_per_random_access_point() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // RAP frame
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 3)); // same TU, same object
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report.errors().filter(|d| d.rule_id == RAP_RULE).count(),
        1,
        "report was: {report}"
    );
}

#[test]
fn rap_replay_distinct_random_access_points_each_report() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // RAP #1
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // RAP #2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report.errors().filter(|d| d.rule_id == RAP_RULE).count(),
        2,
        "report was: {report}"
    );
}

#[test]
fn rap_replay_suppressed_under_external_hls_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(3)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_multi_frame_header_reference_before_rap_is_flagged() {
    let mut data = td_and_seq(0);
    data.extend(multi_frame_header_obu(0)); // mfhId 1 -> seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("multi-frame header")),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_operating_point_set_referenced_only_before_rap_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2)); // OPS (31, 0) defined in TU0 only
    data.extend(seq_obu(3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // seq resent in the RAP TU -> seq replay silent
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK random access point
    data.extend(brt_dependent_obu(31, 0, 2)); // references OPS (31, 0), matching ops_cnt
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.spec_section.as_deref() == Some("7.3.8.1")
                && d.message.contains("operating point set")
                && d.message.contains("ops_id 0 for obu_xlayer_id 31")
        }),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "brt/unavailable-operating-point-set"),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_operating_point_set_resent_in_rap_temporal_unit_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2)); // OPS (31, 0) defined in TU0
    data.extend(seq_obu(3));
    data.extend(temporal_delimiter_obu());
    data.extend(global_ops_obu(false, 0, 2)); // OPS resent in the random access point's TU
    data.extend(seq_obu(3)); // seq resent in the RAP TU
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK random access point
    data.extend(brt_dependent_obu(31, 0, 2)); // references OPS (31, 0)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_rap_in_other_xlayer_does_not_govern_this_layers_reference() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> xlayer 0's CLK reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // xlayer 0 random access point
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "an xlayer-0 random access point must not govern an xlayer-1 reference \
         (§ 7.4.6 per-extended-layer random access); report was: {report}"
    );
}

#[test]
fn rap_replay_reference_governed_by_its_own_layers_rap_fires() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("seq_header_id 3")),
        "an xlayer-1 random access point governs the xlayer-1 reference; report was: \
         {report}"
    );
}

#[test]
fn rap_replay_rap_temporal_unit_with_leading_frame_promotes_own_resends() {
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent in the random access point's own temporal unit
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // random access point
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 3)); // + leading frame
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a random access point's own resends must qualify for that random access point \
         even when its temporal unit also carries a leading frame; report was: {report}"
    );
}

#[test]
fn rap_replay_same_tu_qualifying_resend_not_overwritten_by_leading_resend() {
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resend seq(3) -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK xlayer 0 -> random access point
    data.extend(seq_obu_layer(7, 0)); // qualifying resend of seq(7) at xlayer 0 (non-leading)
    data.extend(seq_obu_layer(7, 1)); // later resend of seq(7) at xlayer 1 (overwrites pre-fix)
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 1, 7)); // LEADING frame -> xlayer 1 leading
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7")),
        "a qualifying same-temporal-unit resend must not be discarded because a later \
         non-qualifying resend of the same object follows it; report was: {report}"
    );
}

#[test]
fn rap_replay_global_lcr_referenced_only_before_rap_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR id 5, xlayer 3 in map
    data.extend(sequence_header_obu_with_lcr(3, 5)); // seq(0) xlayer 3, seq_lcr_id 5
    data.extend(temporal_delimiter_obu());
    data.extend(sequence_header_obu_with_lcr(3, 5)); // seq resent (re-buffers the LCR ref)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0)); // CLK xlayer 3 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.spec_section.as_deref() == Some("7.3.8.1")
                && d.message.contains("global layer configuration record")
                && d.message.contains("lcr_global_config_record_id 5")
        }),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-layer-configuration-record"),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_global_lcr_resent_in_rap_temporal_unit_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None));
    data.extend(sequence_header_obu_with_lcr(3, 5));
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR resent in the RAP TU
    data.extend(sequence_header_obu_with_lcr(3, 5));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0)); // CLK xlayer 3 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_global_lcr_resent_only_in_post_rap_leading_tu_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR id 5, xlayer 3 in map
    data.extend(sequence_header_obu_with_lcr(3, 5)); // seq(0, seq_lcr_id 5)@xlayer 3
    data.extend(seq_obu_layer(9, 3)); // a no-LCR seq(9)@xlayer 3 for the RAP frame ref
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(9, 3)); // seq(9) resent -> the CLK's own reference satisfied
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 9)); // CLK@xlayer 3 -> RAP
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR resent in a LEADING TU
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 3, 9)); // LEADING frame -> xlayer 3 leading
    data.extend(temporal_delimiter_obu());
    data.extend(sequence_header_obu_with_lcr(3, 5)); // re-buffers the § 7.3.8.3 reference
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.spec_section.as_deref() == Some("7.3.8.1")
                && d.message.contains("global layer configuration record")
                && d.message.contains("lcr_global_config_record_id 5")
        }),
        "a global-HLS resend in a post-random-access leading temporal unit must not \
         satisfy a post-leading reference (§ 7.4.4); report was: {report}"
    );
}

#[test]
fn rap_replay_global_lcr_resent_in_post_rap_non_leading_tu_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None));
    data.extend(sequence_header_obu_with_lcr(3, 5));
    data.extend(seq_obu_layer(9, 3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(9, 3));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 9)); // CLK@xlayer 3 -> RAP
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR resent in a NON-leading TU
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 3, 9)); // REGULAR frame -> not leading
    data.extend(temporal_delimiter_obu());
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a global resend in a non-leading post-random-access temporal unit qualifies; \
         report was: {report}"
    );
}

#[test]
fn rap_replay_local_atlas_referenced_only_before_rap_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_obu(3, 2)); // local atlas, xlayer 3, atlas_segment_id 2
    data.extend(local_lcr_obu(3, 0, 1, Some(2))); // local LCR xlayer 3, local_atlas_id 2
    data.extend(temporal_delimiter_obu());
    data.extend(local_lcr_obu(3, 0, 1, Some(2))); // local LCR resent (re-buffers the ref)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0)); // CLK xlayer 3 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.spec_section.as_deref() == Some("7.3.8.1")
                && d.message.contains("local atlas segment")
                && d.message.contains("atlas_segment_id 2 for obu_xlayer_id 3")
        }),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "atlas/local-atlas-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_local_atlas_resent_in_rap_temporal_unit_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_obu(3, 2));
    data.extend(local_lcr_obu(3, 0, 1, Some(2)));
    data.extend(temporal_delimiter_obu());
    data.extend(atlas_obu(3, 2)); // atlas resent in the RAP TU
    data.extend(local_lcr_obu(3, 0, 1, Some(2)));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0)); // CLK xlayer 3 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_ops_only_external_hls_does_not_suppress_undeclared_seq_header() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // references seq(3), not resent
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(31, 0),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("seq_header_id 3")),
        "an OPS-only Provided set must not suppress an undeclared sequence-header \
         replay; report was: {report}"
    );
}

/// A § 5.4 sequence header OBU (seq 0) with `film_grain_params_present == 1`, so a
/// referencing output frame reads `apply_grain` and can reference a film-grain model.
fn film_grain_seq_obu() -> Vec<u8> {
    annex_b_obu(
        0x04,
        &frame_core_seq_payload(FrameCoreSeq {
            film_grain_params_present: true,
            ..FrameCoreSeq::base()
        }),
    )
}

/// A complete intra KEY frame (a § 7.4.1 random access point) that applies grain at
/// `fgm_id` through its § 5.18.2 intra tail, referencing in-band seq 0.
pub(in crate::validator::tests) fn clk_intra_frame_applying_grain(fgm_id: u8) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // immediate_output_frame == 1 (output frame -> apply_grain readable)
    fb.bit(0); // frame_size_override_flag == 0 (max dims 16x16)
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0); // §5.18.2 structure + loop-filter cluster
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    fb.bit(1); // apply_grain = 1
    fb.f(u32::from(fgm_id), 3); // fgm_id f(3)
    fb.f(0, 16); // grain_seed f(16) — complete film_grain_config()
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// `true` if a film-grain-family § 7.3.8.1 replay finding is present.
fn has_film_grain_rap(report: &ValidationReport) -> bool {
    report
        .errors()
        .any(|d| d.rule_id == RAP_RULE && d.message.contains("film grain model"))
}

#[test]
fn rap_replay_film_grain_only_before_rap_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(film_grain_seq_obu());
    data.extend(film_grain_obu_bytes(1 << 0, 0)); // defines slot 0 in TU0
    data.extend(temporal_delimiter_obu());
    data.extend(film_grain_seq_obu()); // seq resent -> its own replay is satisfied
    data.extend(clk_intra_frame_applying_grain(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_film_grain_rap(&report),
        "a film-grain model sent only before the random access point must fire the replay; \
         report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/film-grain-model-unavailable"),
        "the model is linearly available, so the linear check must stay silent; report was: \
         {report}"
    );
}

#[test]
fn rap_replay_film_grain_resent_in_rap_temporal_unit_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(film_grain_seq_obu());
    data.extend(film_grain_obu_bytes(1 << 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(film_grain_seq_obu());
    data.extend(film_grain_obu_bytes(1 << 0, 0)); // resent in the random access point's TU
    data.extend(clk_intra_frame_applying_grain(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_film_grain_rap(&report),
        "a film-grain model resent in the random access point's temporal unit must not fire \
         the replay; report was: {report}"
    );
}

#[test]
fn rap_replay_film_grain_disjoint_from_linear_unavailable() {
    let mut data = temporal_delimiter_obu();
    data.extend(film_grain_seq_obu());
    data.extend(temporal_delimiter_obu());
    data.extend(film_grain_seq_obu());
    data.extend(clk_intra_frame_applying_grain(0)); // references undefined slot 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/film-grain-model-unavailable"),
        "an undefined model must fire the linear check; report was: {report}"
    );
    assert!(
        !has_film_grain_rap(&report),
        "the replay must not fire for a model the linear check already owns; report was: \
         {report}"
    );
}

#[test]
fn rap_replay_film_grain_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet};
    let mut data = temporal_delimiter_obu();
    data.extend(film_grain_seq_obu());
    data.extend(film_grain_obu_bytes(1 << 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(film_grain_seq_obu());
    data.extend(clk_intra_frame_applying_grain(0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_film_grain_rap(&report),
        "a Provided external-HLS mode must suppress the film-grain replay; report was: {report}"
    );
}

/// `true` if a quantizer-matrix-family § 7.3.8.1 replay finding is present.
fn has_qm_rap(report: &ValidationReport) -> bool {
    report
        .errors()
        .any(|d| d.rule_id == RAP_RULE && d.message.contains("quantizer matrix level"))
}

/// A § 5.4 sequence header for the QM random-access tests: 4:2:0 (NumPlanes 3, matching the
/// `qm_chroma_info_present_flag == 1` QM OBUs) with `long_term_frame_id_bits != 0` (§ 6.4.6,
/// required for the RAS frame).
fn qm_rap_seq() -> FrameCoreSeq {
    FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    }
}

#[test]
fn rap_replay_qm_level_only_before_rap_is_flagged() {
    let seq = FrameCoreSeq {
        long_term_frame_id_bits: 4, // §6.4.6: the RAS requires long_term_frame_id_bits != 0
        max_mlayer_id: 1,           // != 0 -> the RAS refresh takes the explicit arm
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(qm_default_level_obu_chroma(0)); // TU1: level 0 at layer 0, QmMLayerId 0
    data.extend(temporal_delimiter_obu());
    data.extend(ras_frame_explicit_map_at_layer(1, 0, 1, 8, 1));
    data.extend(temporal_delimiter_obu());
    data.extend(intra_only_frame_with_qm_reference(0)); // TU3: references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_qm_rap(&report),
        "a QM level sent only before the random access point must fire the replay; report \
         was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "level 0 survives the cross-layer RAS reset, so the linear check must stay silent; \
         report was: {report}"
    );
}

#[test]
fn rap_replay_qm_level_resent_after_rap_passes() {
    let mut data = td_and_frame_core_seq(qm_rap_seq());
    data.extend(qm_default_level_obu_chroma(0)); // TU1
    data.extend(temporal_delimiter_obu());
    data.extend(ras_frame_confirmed_reset()); // TU2: RAP
    data.extend(temporal_delimiter_obu());
    data.extend(qm_default_level_obu_chroma(0)); // TU3: resent after the random access point
    data.extend(intra_only_frame_with_qm_reference(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_qm_rap(&report),
        "a QM level resent after the random access point must not fire the replay; report \
         was: {report}"
    );
}

#[test]
fn rap_replay_qm_reset_to_defaults_after_rap_counts_as_resend() {
    let mut data = td_and_frame_core_seq(qm_rap_seq());
    data.extend(qm_default_level_obu_chroma(0)); // TU1
    data.extend(temporal_delimiter_obu());
    data.extend(ras_frame_confirmed_reset()); // TU2: RAP
    data.extend(temporal_delimiter_obu());
    data.extend(qm_reset_obu_chroma()); // TU3: reset-to-defaults -> all levels (re)sent
    data.extend(intra_only_frame_with_qm_reference(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_qm_rap(&report),
        "a qm_bit_map == 0 reset-to-defaults after the random access point must satisfy the \
         replay for every level; report was: {report}"
    );
}

#[test]
fn rap_replay_qm_disjoint_from_linear_unavailable() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_frame_with_qm_reference(0)); // references undefined custom level 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "an undefined QM level must fire the linear check; report was: {report}"
    );
    assert!(
        !has_qm_rap(&report),
        "the replay must not fire for a level the linear check already owns; report was: \
         {report}"
    );
}

#[test]
fn rap_replay_qm_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet};
    let mut data = td_and_frame_core_seq(qm_rap_seq());
    data.extend(qm_default_level_obu_chroma(0));
    data.extend(temporal_delimiter_obu());
    data.extend(ras_frame_confirmed_reset());
    data.extend(temporal_delimiter_obu());
    data.extend(intra_only_frame_with_qm_reference(0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_qm_rap(&report),
        "a Provided external-HLS mode must suppress the QM replay; report was: {report}"
    );
}
