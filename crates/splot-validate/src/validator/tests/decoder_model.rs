// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

// --- Decoder-model buffer-delay sum constancy (§6.4.13 / §6.10.5) ---

#[test]
fn decoder_model_intra_cvs_ops_sum_change_is_error() {
    // A CLK frame starts a coded video sequence for xlayer 0 (§ 7.3.6), then the
    // same (obu_xlayer_id, ops_id, op) is redefined WITHIN that CVS (same temporal
    // unit, no OPS reset), both explicit, differing sum (30 -> 40) -> error
    // (§ 6.10.5). The CLK makes the stream genuinely intra-CVS — the error tier is
    // gated on a started CVS, so this is the canonical "same coded video sequence"
    // scenario the spec delta describes.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "an intra-CVS OPS buffer-delay sum change must be a single error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_before_first_clk_is_not_error() {
    // Two OPS redefinitions before any CLK: the OBUs lie in NO coded video sequence
    // (§ 7.3.6: a CVS starts at a CLK temporal unit), so the § 6.10.5 "video
    // sequence that includes one or more random access points" precondition is
    // unsatisfied and the error tier must not fire. The change spans no CVS or reset
    // boundary either, so the advisory stays silent too.
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30, no CVS yet
    data.extend(local_ops_obu_with_delays(2, false, 0, 25, 15)); // sum 40, still no CVS
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a pre-first-CLK OPS sum change is in no coded video sequence and must not \
         be an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "a pre-first-CLK OPS sum change spans no boundary and must not warn: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_with_late_clk_in_same_tu_is_not_error() {
    // Temporal-unit granularity (§ 7.3.6): the first OPS is in CVS 1 (TU1's CLK), the
    // second OPS sits in TU2 BEFORE TU2's own CLK. The CVS epoch is still 1 when the
    // second OPS is observed (the CLK comes later in TU2), but that CLK starts a NEW
    // coded video sequence for TU2, so the two OPS straddle a real CVS boundary and
    // the change is conforming under the per-CVS reading. The deferred error must be
    // dropped, never emitted — and the cross-CVS advisory fires in its place so the
    // genuinely cross-CVS change is not silently lost.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // TU1: starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, CVS 1
    data.extend(temporal_delimiter_obu()); // TU2 begins
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, epoch still 1
    data.extend(clk_frame_for_xlayer(0, 0)); // late CLK -> TU2 is CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a late same-TU CLK makes the change cross-CVS; the deferred error must be \
         dropped: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "the dropped deferred error must be replaced by the cross-CVS advisory, not \
         silently lost: {report}"
    );
}

#[test]
fn decoder_model_intra_cvs_ops_same_sum_is_not_flagged() {
    // Identical sums (different split, 10+20 vs 20+10) must not fire either tier.
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(2, false, 0, 20, 10)); // sum 30
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "an unchanged sum must not be flagged: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "an unchanged sum must not raise the advisory either: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_across_cvs_is_not_error_but_warns() {
    // A genuine CVS boundary at a temporal-unit edge (§ 7.3.6): TU1 holds CVS 1's
    // OPS, TU2's CLK starts CVS 2 and its OPS redefines the same triple with a
    // different sum. The two OPS sit in different coded video sequences, so the
    // change is conforming under the per-CVS reading: no error, only the cross-CVS
    // advisory (§ 6.4.13 / § 6.10.5). Both OPS are placed AFTER their CVS's CLK so
    // neither shares a temporal unit across the boundary.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // TU1: starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, CVS 1
    data.extend(temporal_delimiter_obu()); // TU2 begins
    data.extend(clk_frame_for_xlayer(0, 0)); // TU2: starts CVS 2 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a cross-CVS OPS sum change must not be an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a cross-CVS OPS sum change must raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_across_reset_is_not_error_but_warns() {
    // An OPS reset between the two definitions (same CVS) re-baselines the
    // constraint: no error, but the reset-spanning change raises the advisory.
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(2, true, 0, 25, 15)); // reset, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a reset-spanning OPS sum change must not be an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a reset-spanning OPS sum change must raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_redefinition_without_explicit_info_clears_baseline() {
    // The defining redefinition (ops_cnt > 0) omits ops_decoder_model_info() for the
    // op it covers. Per Annex E.1 (`annex-e-decoder-model.md` lines 25–27) the
    // previous parameters do not persist: the redefinition clears the stored
    // baseline for that triple rather than reusing it, so it neither compares against
    // the vanished value nor against the Annex E mode defaults. With nothing to
    // compare, no diagnostic of either tier is emitted.
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30, explicit
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "an absent-info redefinition clears the baseline and must not be compared: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "an absent-info redefinition must not raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_annex_e_defaults_are_never_compared() {
    // The default Annex E split (70000/20000, sum 90000) is a resource-availability
    // fallback, not a signalled value. A single explicit OPS whose sum equals that
    // default must not be compared against the default-bearing absent-info OPS.
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
    data.extend(local_ops_obu_with_delays(2, false, 0, 70_000, 20_000)); // explicit 90000
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "absent-info OPS using the Annex E defaults must not be compared: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "absent-info OPS must not raise the advisory against an explicit value: {report}"
    );
}

#[test]
fn decoder_model_seq_header_sum_change_across_cvs_warns() {
    // Two coded video sequences whose frame-confirmed activated sequence headers
    // carry explicit, differing seq_decoder_model_info() sums -> the § 6.4.13
    // advisory (warning). The seq-header tier has no error severity.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    )); // sum 0
    data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
    )); // sum 12
    data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "an activated seq-header sum change across a CLK must raise the advisory: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "the seq-header tier is advisory only, never an error: {report}"
    );
}

#[test]
fn decoder_model_seq_header_same_id_reconfiguration_across_cvs_warns() {
    // A same-seq_header_id reconfiguration is the canonical conforming way to change
    // activated-header parameters across a CVS boundary (legal at the boundary,
    // § 7.3.6). CVS 1 activates seq_header_id 0 with sum 0; CVS 2 re-sends the SAME
    // id 0 with a differing sum (12) and a CLK re-confirming it. The id never
    // changes, so the activation event's id-change short-circuit would skip it — the
    // advisory must still fire because it is evaluated on every frame-confirmed
    // activation at the (post-CLK) new CVS epoch.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    )); // id 0, sum 0
    data.extend(clk_frame_for_xlayer(0, 0)); // confirm id 0 + start CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 5, 7),
    )); // id 0 again, sum 12
    data.extend(clk_frame_for_xlayer(0, 0)); // re-confirm id 0 + start CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a same-id reconfiguration changing the sum across a CVS boundary must raise \
         the advisory: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "the seq-header tier is advisory only, never an error: {report}"
    );
}

#[test]
fn decoder_model_seq_header_without_info_never_warns() {
    // Consecutive CVSs whose activated headers omit seq_decoder_model_info() never
    // fire the advisory.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "headers without decoder-model info must not raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_seq_header_fallback_guess_activation_never_warns() {
    // With several in-band sequence headers and NO frame to confirm activation,
    // the first-seen activation is a fallback guess that must not participate in
    // the cross-CVS advisory (agreement_activation_for returns None).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    ));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "an unconfirmed fallback-guess activation must not participate: {report}"
    );
}

#[test]
fn decoder_model_external_hls_suppresses_both_ids() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // The exact intra-CVS error scenario, but with external HLS Provided: both the
    // error and the advisory must be suppressed.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    ));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40 (intra-CVS)
    data.extend(clk_frame_for_xlayer(0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
    ));
    data.extend(clk_frame_for_xlayer(0, 1));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new()
                .with_sequence_header_id(0)
                .with_sequence_header_id(1),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "external HLS must suppress the OPS error tier: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "external HLS must suppress both decoder-model advisories: {report}"
    );
}

#[test]
fn decoder_model_external_hls_without_seq_headers_still_suppresses_seq_advisory() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // The seq-header advisory's only previous suppression was the
    // external_declares_sequence_header early return, which is false when the
    // Provided set declares NO sequence header (only an operating point set here).
    // The blanket `ExternalHlsMode::Provided` guard must still suppress the seq tier,
    // matching design decision 5: a same-id reconfiguration across a CVS that would
    // otherwise warn must stay silent.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    ));
    data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
    )); // differing sum
    data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(31, 0),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "Provided external HLS without declared sequence headers must still suppress \
         the seq-header advisory: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "the seq-header tier never emits an error: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_across_targeted_reset_is_not_error_but_warns() {
    // A § 6.10.1 case-3 targeted reset (ops_reset_flag == 0, ops_cnt == 0) of OPS 0
    // between the two definitions re-baselines the constraint for that OPS alone,
    // exactly like a full reset: no error, but the reset-spanning sum change raises
    // the cross-CVS advisory. The CLK makes the stream genuinely intra-CVS so that
    // without the targeted-reset re-baselining the error tier WOULD fire.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu(0, false, 0, 0, 0, false, 0)); // targeted reset of OPS 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // redefine, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a targeted-reset-spanning OPS sum change must not be an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a targeted-reset-spanning OPS sum change must raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_targeted_reset_of_other_ops_still_errors() {
    // The intra-CVS error must still fire when the intervening targeted reset hits a
    // DIFFERENT OPS (here OPS 1): re-baselining is per-(obu_xlayer_id, opsID), so a
    // targeted reset of OPS 1 does not excuse a sum change of OPS 0 within the CVS.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // OPS 0, sum 30
    data.extend(local_ops_obu(0, false, 1, 0, 0, false, 0)); // targeted reset of OPS 1
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // OPS 0, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "a targeted reset of a different OPS must not excuse OPS 0's intra-CVS sum \
         change: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_dm_less_redefinition_clears_baseline_no_diagnostic() {
    // FINDING C (Annex E.1, mirror `annex-e-decoder-model.md` lines 25–27): "If the
    // new Operating Point Set OBU does not signal decoder model parameters for a
    // given operating point, the previous set of decoder model parameters does not
    // persist." explicit-30, then a redefinition of the SAME (xlayer, ops_id) that
    // OMITS ops_decoder_model_info() for that op (so it does not persist), then
    // explicit-40: the dm-less redefinition clears the baseline, so explicit-40 has
    // nothing to compare against -> NEITHER the error nor the advisory fires. (All in
    // one CVS so that without clearing the error tier WOULD fire.)
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, explicit
    data.extend(local_ops_obu(0, false, 0, 1, 0, false, 0)); // redefine OPS 0, no dm info
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, explicit
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a dm-less redefinition clears the baseline; explicit-40 must not error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "a cleared baseline must not be compared, so no advisory either: {report}"
    );
}

#[test]
fn decoder_model_ops_unrelated_redefinition_still_errors_within_cvs() {
    // FINDING C control: an UNRELATED other-OPS OBU between the two explicit
    // definitions of OPS 0 must NOT clear OPS 0's baseline, so the intra-CVS error
    // still fires. OPS 1 is defined dm-less between the two OPS 0 definitions; the
    // clearing is keyed on (xlayer, ops_id), so OPS 1's redefinition leaves OPS 0
    // untouched and explicit-30 vs explicit-40 of OPS 0 is still a single error.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // OPS 0, sum 30
    data.extend(local_ops_obu(0, false, 1, 1, 0, false, 0)); // unrelated OPS 1, no dm info
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // OPS 0, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "a dm-less redefinition of a DIFFERENT OPS must not clear OPS 0's baseline: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_seq_header_dm_less_activation_clears_baseline_no_warning() {
    // FINDING D (Annex E.1, mirror `annex-e-decoder-model.md` lines 24–25): "If the
    // new Sequence Header OBU does not signal decoder model parameters for an
    // extended layer, the previous set of decoder model parameters does not persist."
    // Three coded video sequences: CVS 1 activates an explicit-sum header, CVS 2
    // activates a header WITHOUT seq_decoder_model_info() (clearing the baseline),
    // CVS 3 activates an explicit header with a DIFFERENT sum. Because the dm-less
    // activation cleared the baseline, CVS 3 has no persistent previous parameter set
    // to compare against -> no cross-CVS advisory.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    )); // id 0, sum 0
    data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1))); // id 1, no dm info
    data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2 (clears baseline)
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(2, 5, 7),
    )); // id 2, sum 12
    data.extend(clk_frame_for_xlayer(0, 2)); // confirm + start CVS 3
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "a dm-less activation between the two explicit headers clears the baseline; \
         the later explicit sum must not raise the advisory: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "the seq-header tier never emits an error: {report}"
    );
}

#[test]
fn decoder_model_ops_cross_layer_local_reset_does_not_excuse_intra_cvs_error() {
    // FINDING B (§ 6.10.1 case 1, mirror `06-syntax-structures-semantics.md` lines
    // 2577–2578): a local reset resets "All OPS for the associated extended layer",
    // not all layers. xlayer 0 defines sum 30, xlayer 1 sends a LOCAL reset (which
    // resets only xlayer 1's OPS), then xlayer 0 redefines sum 40 within its own CVS.
    // No reset of xlayer 0 intervened, so the intra-CVS error must still fire — the
    // per-layer reset generation no longer lets xlayer 1's reset re-baseline xlayer 0.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // xlayer 0, sum 30
    data.extend(local_ops_obu(1, true, 0, 0, 0, false, 0)); // LOCAL reset of xlayer 1
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // xlayer 0, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "a local reset of an unrelated extended layer must not excuse xlayer 0's \
         intra-CVS sum change: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_global_reset_re_baselines_other_layers() {
    // FINDING B control (§ 6.10.1 case 1, mirror lines 2577–2578): a GLOBAL reset
    // resets "all layers if global", so it DOES re-baseline xlayer 0. xlayer 0 sum
    // 30, then a global (GLOBAL_XLAYER_ID = 31) reset, then xlayer 0 sum 40 within the
    // CVS: the global reset re-baselines the constraint, so no error, only the
    // reset-spanning advisory.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // xlayer 0, sum 30
    data.extend(local_ops_obu(31, true, 0, 0, 0, false, 0)); // GLOBAL reset (all layers)
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // xlayer 0, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a global reset re-baselines xlayer 0, so the change is not an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a global-reset-spanning sum change must raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_pre_clk_baseline_in_same_tu_migrates_to_new_cvs_error() {
    // FINDING A (§ 7.3.6, mirror `07-decoding-process.md` lines 604–606): "A new
    // coded video sequence for an extended layer is defined to start at each temporal
    // unit that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY ...". OPS
    // sum 30 is observed BEFORE the CLK, but the whole CLK temporal unit lies in the
    // NEW coded video sequence, so the baseline migrates to the new CVS epoch and the
    // post-CLK OPS sum 40 (same TU) is compared within ONE coded video sequence ->
    // the intra-CVS error fires (not merely the advisory).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, pre-CLK
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK -> whole TU is the new CVS
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, post-CLK, same TU
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "a pre-CLK baseline in the CLK's own TU migrates to the new CVS; the post-CLK \
         change is intra-CVS and must error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the migrated intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_both_definitions_pre_clk_in_same_tu_is_error() {
    // FINDING (round-3, § 7.3.6, mirror `07-decoding-process.md` lines 604–606): "A
    // new coded video sequence for an extended layer is defined to start at each
    // temporal unit that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY
    // ...". BOTH OPS definitions of the same (obu_xlayer_id, ops_id, op) occur BEFORE
    // the CLK in the SAME temporal unit, with no coded video sequence started yet for
    // the layer. The whole CLK temporal unit lies in the new coded video sequence, so
    // both observations are intra-CVS and the differing sum (30 -> 40) is the error
    // tier — the comparison is deferred PreCvs at the second OPS and emitted when the
    // CLK starts the layer's coded video sequence.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, pre-CLK
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, pre-CLK
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK later in same TU -> whole TU is the new CVS
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "both pre-CLK OPS definitions in the CLK's own temporal unit are intra-CVS; \
         the differing sum must be a single error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the deferred intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_both_definitions_pre_clk_no_clk_in_tu_stays_silent() {
    // The same [seq, OPS30, OPS40] pair as the round-3 case but with NO CLK in the
    // temporal unit (the temporal unit closes at the next temporal delimiter): the
    // observations are in no coded video sequence (§ 7.3.6), so the § 6.10.5
    // random-access-point precondition is unsatisfied and the deferred PreCvs error is
    // dropped silently — preserving the documented pre-first-CLK silence. The second
    // temporal delimiter completes the first temporal unit and triggers the silent
    // drop.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, no CVS yet
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, still no CVS
    data.extend(temporal_delimiter_obu()); // TU closes with no CLK for the layer
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a pre-first-CLK OPS sum change whose temporal unit closes with no CLK is in \
         no coded video sequence and must stay silent: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the dropped pre-CVS comparison spans no boundary and must not warn: {report}"
    );
}

#[test]
fn decoder_model_ops_multiple_pre_clk_changes_same_tu_error_per_change() {
    // Three pre-CLK definitions 30 -> 40 -> 50 of the same triple in the CLK's own
    // temporal unit. § 7.3.6 places all three in the new coded video sequence, so each
    // consecutive change is a distinct intra-CVS comparison: two PreCvs errors are
    // deferred (30 -> 40 at the second OPS, 40 -> 50 at the third) and both are emitted
    // when the CLK starts the layer's coded video sequence — exactly one diagnostic
    // per comparison, matching the eager mid-CVS path (one error per consecutive
    // change).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40
    data.extend(local_ops_obu_with_delays(0, false, 0, 30, 20)); // sum 50
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK later in same TU
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        2,
        "two consecutive intra-CVS sum changes must produce two errors, one per \
         comparison: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "intra-CVS changes must not raise the cross-CVS advisory: {report}"
    );
}
