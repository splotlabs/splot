// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn ops_deferred_check_fires_on_frame_activation_change() {
    // The OPS is conformant under the initially active header 0 (whose signaled
    // map clears MLayerDependencyMap[1][0]) — silent at observation. A CLK then
    // re-activates xlayer 0 to header 1 (default maps), and the frame-driven
    // activation hook must evaluate the stored OPS maps against the new header
    // and emit exactly one finding.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_mlayer_dep_cleared(0),
    ));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1)));
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/mlayer-dependency-missing"),
        1,
        "the frame-driven re-activation must evaluate stored OPS maps; report was: {report}"
    );
}

#[test]
fn ops_disagreement_reemitted_after_sequence_header_redefinition() {
    // The OPS disagrees with header 0 (flagged once). Re-sending header 0 with
    // changed agreement inputs (max_tlayer_id 1 -> 0 changes the default
    // TLayerDependencyMap) invalidates the id's dedup keys and re-fires the
    // checks: the still-disagreeing mlayer map is reported against the
    // redefined content too.
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1)));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/mlayer-dependency-missing"),
        2,
        "a same-id redefinition must re-fire the agreement checks; report was: {report}"
    );
}

#[test]
fn lcr_local_tlayer_dependency_missing_is_flagged() {
    // Local LCR × tlayer map: lcr_tlayer_map[0][0][0] includes temporal layer 1
    // without temporal layer 0 against the default TLayerDependencyMap[0][1][0]. The
    // CLK frame referencing seq id 0 frame-confirms the xlayer-0 activation.
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b1, &[0b10]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/tlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn lcr_global_mlayer_dependency_missing_is_flagged() {
    // Global LCR × mlayer map: lcr_mlayer_map[1][3] includes embedded layer 1
    // without embedded layer 0 against the default MLayerDependencyMap[1][0]. A CLK
    // frame on xlayer 3 referencing seq id 0 frame-confirms the activation.
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu_with_embedded(5, 3, 0b10, &[0b1]));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(1, 0, 0, 3),
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/mlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn lcr_local_record_takes_precedence_over_global() {
    // § 6.4.1 resolution order: with both a dependency-closed local LCR and a
    // violating global LCR carrying the same id, the local record is the
    // associated one, so no finding may be emitted.
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b11, &[0b11, 0b11]));
    data.extend(global_lcr_obu_with_embedded(5, 0, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "the local LCR resolves first (§ 6.4.1); report was: {report}"
    );
}

#[test]
fn lcr_unresolved_nonzero_seq_lcr_id_is_not_dependency_checked() {
    // seq_lcr_id != 0 resolving to no in-band LCR: the § 7.3.8.3 availability
    // diagnostic owns the case; no dependency finding can exist without maps.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "hls/unavailable-layer-configuration-record"),
        "report was: {report}"
    );
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "an unresolved seq_lcr_id must not be dependency-checked; report was: {report}"
    );
}

#[test]
fn lcr_after_sequence_header_is_not_paired() {
    // § 6.4.1 associates a sequence header only with an LCR "present prior to
    // this sequence header"; a later-arriving violating LCR must not be
    // retroactively paired with the earlier activation (the § 7.3.8.3
    // availability diagnostic already owns this stream).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "a later LCR must not pair with an earlier activation; report was: {report}"
    );
}

#[test]
fn lcr_redefined_without_embedded_info_is_not_checked() {
    // A redefinition of local LCR 5 without embedded-layer info replaces the
    // stored maps wholesale; the activation must see the latest (map-less)
    // definition, not the superseded violating maps.
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(local_lcr_obu(0, 0, 5, None)); // redefinition, no embedded info
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "stale maps from a superseded definition must not be checked; report was: {report}"
    );
}

#[test]
fn lcr_dependency_check_suppressed_under_external_hls_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // The § 6.8.9 closure pairs the in-band activated header against the LCR its
    // seq_lcr_id resolves to under § 6.4.1 (local-first). A Provided declaration is
    // PARTIAL (`ExternalHlsMode::Provided` — it cannot enumerate external LCRs), so an
    // unmodeled external *local* LCR with this seq_lcr_id could win the resolution
    // ahead of the in-band record; the in-band association may not be the activated
    // one, so the check is suppressed under ANY Provided mode (even an empty set) to
    // avoid a false positive — the same local-first-shadowing reasoning as the
    // lcr/global-xlayer-map-missing-xlayer gate. The stream WOULD fire under Disabled
    // (the trailing CLK frame frame-confirms the activation), confirming the
    // suppression is the only reason it is silent here.
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    // Sanity: under Disabled this in-band violation fires.
    let baseline = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&baseline, "lcr/mlayer-dependency-missing"),
        "the in-band violation must fire under Disabled; report was: {baseline}"
    );
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "any Provided external HLS must suppress the association-dependent LCR \
         dependency check (an unmodeled external local LCR could shadow the in-band \
         association); report was: {report}"
    );
}

#[test]
fn lcr_dependency_uses_strict_frame_confirmation() {
    // Finding-1 regression (codex 3393669703): the § 6.8.5 / § 6.8.8 / § 6.8.9 LCR
    // agreement checks must use the STRICT frame-confirmed gate — never the
    // sole-in-band-header OBU-order fallback — so they fire only against a frame-loaded
    // activation, matching the Annex A value-space precedent. A sole staged header with
    // NO frame is a guess (§ 7.3.6 permits staging), so the dependency check stays
    // silent until a frame confirms the activation.
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    // Sole staged header, NO frame: strict gate keeps the check silent.
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "a sole staged header (no frame) must not fire the LCR dependency check via the \
         sole-header fallback; report was: {report}"
    );
    // Adding a frame that loads the staged header confirms the activation -> fires.
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/mlayer-dependency-missing"),
        "the frame-confirmed activation must fire the LCR dependency check; report was: \
         {report}"
    );
}

#[test]
fn lcr_ptl_uses_strict_frame_confirmation() {
    // Finding-1 regression for § 6.8.5: a sole staged header (no frame) is silent;
    // the frame-confirmed activation fires.
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8, // > lcr_max_level_idx 4
        tier: 0,
        max_mlayer_id: 0,
    }));
    assert!(
        !has_error(
            &Validator::new(false).validate_bytes(&data),
            "lcr/ptl-level-exceeds-max"
        ),
        "a sole staged header (no frame) must not fire the § 6.8.5 ceiling via the \
         sole-header fallback"
    );
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    assert!(
        has_error(
            &Validator::new(false).validate_bytes(&data),
            "lcr/ptl-level-exceeds-max"
        ),
        "the frame-confirmed activation must fire the § 6.8.5 ceiling"
    );
}

#[test]
fn lcr_agreement_silent_when_external_header_could_be_the_activator() {
    // Finding-1 regression (codex 3393669703), the worst case the strict gate guards:
    // an external sequence header is DECLARED, and an in-band header is staged but NO
    // frame has loaded it. The OBU-order sole-header fallback would guess the staged
    // in-band header is active and fire the LCR checks against it — but the real
    // activated header could be the external one, so firing would be a false positive.
    // The checks must stay silent. (They also stay silent WITH a confirming frame here,
    // because any Provided mode suppresses the association-dependent LCR checks per the
    // partial-declaration policy — both paths are silent, neither fires against a guess.)
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(9)),
    };
    // No frame: the strict gate alone keeps it silent (no activation to fire against).
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "with an external header declared and no frame, the LCR check must not fire \
         against a guessed in-band activation; report was: {report}"
    );
    // Even with a confirming frame the Provided gate suppresses the check.
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "under any Provided mode the association-dependent LCR check stays suppressed; \
         report was: {report}"
    );
}

#[test]
fn lcr_repeated_sequence_header_pairs_with_now_present_lcr() {
    // § 6.4.1 associates "this sequence header" with an LCR present prior to
    // it: the violating LCR arrives after the first header but before the
    // bit-identical repeat, so the repeat's association must be evaluated and
    // flagged exactly once. The trailing CLK frame frame-confirms the activation.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "lcr/mlayer-dependency-missing"),
        1,
        "the repeated header must pair with the now-present LCR; report was: {report}"
    );
}

#[test]
fn lcr_association_snapshotted_at_header_observation() {
    // § 6.4.1 associates the header with the LCR present prior to *that
    // header*: the dependency-closed LCR precedes the header, the violating
    // redefinition follows it, and the frame-driven activation must check the
    // header-observation snapshot (the closed maps), not the live store.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1))); // id 0
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b11, &[0b11, 0b11]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(1, 5, 1, 1), // id 1, seq_lcr_id 5
    ));
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1])); // redefinition
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1)); // activates id 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "the post-header redefinition must not be paired with header 1; report was: {report}"
    );
}
