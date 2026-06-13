// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn monotonic_output_order_disagreement_inside_cmvs_is_flagged() {
    // § 6.4.1: inside an MSDO-begun CMVS, extended layers 0 (monotonic 1) and 1
    // (monotonic 0) disagree on monotonic_output_order_flag — flagged.
    let data = cmvs_two_layer_stream(true, false);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "report was: {report}"
    );
}

/// A single temporal unit that *opens* a § 7.3.2 CMVS (begin condition 1: a CLK
/// temporal unit with an MSDO present and no CMVS yet active) and frame-confirms both
/// extended layers WITHIN that same opening temporal unit. xlayer 0's CLK references
/// seq 0 (`monotonic_x0`); xlayer 1's CLK references seq 1 (`monotonic_x1`). The CMVS
/// membership is decidable at the CLK (§ 7.3.7: the at-most-one MSDO precedes every
/// coded extended layer unit), so the cross-layer agreement check sees `Inside` when
/// the second CLK activates — the begin direction of the boundary that the two-TU
/// `cmvs_two_layer_stream` does not exercise.
pub(in crate::validator::tests) fn cmvs_two_layer_single_tu_stream(
    monotonic_x0: bool,
    monotonic_x1: bool,
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu(); // single temporal unit
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> opens the CMVS
    data.extend(seq_header_obu_monotonic(0, 0, monotonic_x0)); // xlayer 0 seq 0
    data.extend(seq_header_obu_monotonic(1, 1, monotonic_x1)); // xlayer 1 seq 1
    // Both activations are CLK frame headers in this same opening temporal unit: the
    // first frame-confirms xlayer 0 (and, as the CLK, begins the CMVS), the second
    // frame-confirms xlayer 1 — the disagreement fires at the second activation.
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1, ref seq 1
    data
}

#[test]
fn monotonic_output_order_disagreement_in_cmvs_opening_tu_is_flagged() {
    // § 6.4.1 / § 7.3.2: two extended layers activating disagreeing
    // monotonic_output_order_flag values WITHIN the CMVS-opening temporal unit (MSDO +
    // CLKs + activations, a single temporal unit). § 7.3.7 makes the begin condition
    // decidable at the CLK, so the tracker reports `Inside` at the second activation
    // and the disagreement fires — the begin direction of the boundary (without it the
    // committed `Outside` of the previous temporal unit would stale-leak and the check
    // would miss this opening-temporal-unit disagreement).
    let data = cmvs_two_layer_single_tu_stream(true, false);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "report was: {report}"
    );
}

#[test]
fn monotonic_output_order_agreement_in_cmvs_opening_tu_is_conforming() {
    // § 6.4.1: both extended layers agree (monotonic 1) within the CMVS-opening
    // temporal unit — no diagnostic. Guards the begin-direction adjustment against a
    // false positive on a conforming single-temporal-unit CMVS.
    let data = cmvs_two_layer_single_tu_stream(true, true);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "agreeing flags in the CMVS-opening temporal unit must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_agreement_inside_cmvs_is_conforming() {
    // § 6.4.1: inside the same CMVS, both extended layers agree (monotonic 1) — no
    // diagnostic.
    let data = cmvs_two_layer_stream(true, true);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "agreeing flags inside a CMVS must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_outside_cmvs_is_not_flagged() {
    // § 6.4.1: the agreement requirement is scoped to a coded multistream video
    // sequence. With no MSDO and no global LCR, the CMVS tracker stays Outside, so
    // disagreeing monotonic_output_order_flag values across two extended layers do
    // not fire.
    let mut data = temporal_delimiter_obu();
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 monotonic 1
    data.extend(seq_header_obu_monotonic(1, 1, false)); // xlayer 1 monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // activate xlayer 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // activate xlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "disagreement outside any CMVS must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_in_unknown_cmvs_is_not_flagged() {
    // § 6.4.1 / § 7.3.2: a CLK temporal unit with a global LCR present but no MSDO
    // routes the CMVS tracker to Unknown (begin condition 3 needs an *activated*
    // global LCR, which is not modeled). The agreement check fires only in Inside,
    // so a disagreement while Unknown must not fire (conservative under-approximation).
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(global_lcr_obu(0, 0b11, None)); // global LCR (xlayers 0, 1), no MSDO
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 monotonic 1
    data.extend(seq_header_obu_monotonic(1, 1, false)); // xlayer 1 monotonic 0
    data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> Unknown (LCR present, no MSDO)
    data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Unknown
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // re-activate xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "disagreement while the CMVS tracker is Unknown must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_in_cmvs_ending_tu_is_not_flagged() {
    // § 7.3.2 end condition 2 / § 7.3.7: a temporal unit that begins a new coded
    // video sequence (a CLK) but contains no MSDO and no activated global LCR ENDS
    // the active CMVS — that temporal unit is outside the CMVS. § 7.3.7 places the
    // optional MSDO before every coded extended layer unit, so MSDO absence is
    // already decidable at the CLK. A CLK in such a temporal unit that activates a
    // header disagreeing on monotonic_output_order_flag must therefore NOT fire,
    // even though the previous temporal unit left the tracker Inside.
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> begins the CMVS
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
    data.extend(annex_b_obu(0x04, &seq_header_payload_monotonic(1, false))); // seq 1 monotonic 0 (available)
    // CLK xlayer 0 referencing seq 0 frame-confirms xlayer 0 and begins the CMVS, so
    // xlayer 0 is a decidable association (isolating the end-of-CMVS state downgrade
    // from the fallback-guess gate).
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
    // Temporal unit 2: a CLK for xlayer 1 with NO MSDO ends the CMVS (end cond. 2);
    // it activates seq 1 (monotonic 0), disagreeing with xlayer 0's seq 0 (monotonic 1).
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1, ref seq 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "an MSDO-less CLK temporal unit ends the CMVS; a disagreement activated there \
         is outside the CMVS and must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_unreferenced_extra_header_inside_cmvs_is_not_flagged() {
    // § 7.3.6: an additional sequence header with a different seq_header_id that no
    // frame references "is not activated and has no effect on the decoding process".
    // Inside a CMVS, xlayer 1 carries an extra never-referenced header (seq 2,
    // monotonic 0) before the header (seq 1, monotonic 1) its frame actually
    // references; xlayer 0 (seq 0, monotonic 1) agrees with the *referenced* xlayer-1
    // header, so the unreferenced disagreeing guess must not fire the check.
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
    // xlayer 1 sends the unreferenced extra header (seq 2, monotonic 0) FIRST, then
    // its referenced header (seq 1, monotonic 1). The OBU-order fallback for xlayer 1
    // is the never-activated seq 2.
    data.extend(seq_header_obu_monotonic(1, 2, false)); // xlayer 1 extra, unreferenced
    data.extend(seq_header_obu_monotonic(1, 1, true)); // xlayer 1 referenced header
    data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> begins the CMVS
    data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Inside
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame-confirm xlayer 0 (seq 0)
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // frame-confirm xlayer 1 (seq 1)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "an unreferenced extra header with a differing flag must not fire (§ 7.3.6 \
         leaves it unactivated); report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // § 6.4.1: under caller-provided external HLS that declares a sequence header (the
    // in-use seq 0), an externally-activated sequence header has an unmodeled
    // monotonic_output_order_flag, so the cross-layer comparison is unreliable and
    // suppressed even on the otherwise-firing inside-CMVS disagreement stream. This is
    // the positive coverage for the narrowed (declares_any_sequence_header()) gate's
    // suppression branch (FIX 3 test 5).
    let data = cmvs_two_layer_stream(true, false);
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "external HLS declaring a sequence header must suppress the § 6.4.1 monotonic \
         agreement check; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_under_empty_external_hls_is_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // FIX 3 (Codex finding 4). `ExternalHlsSet::new()` declares an external channel
    // that declares NO sequence header, so it cannot supply an out-of-band active
    // sequence header. The § 6.4.1 monotonic gate must narrow to
    // declares_any_sequence_header() (as validate_active_sequence_limits and the
    // distinct-mlayer gate do), so an empty set must NOT suppress the inside-CMVS
    // disagreement. The stream fires mid-CMVS (TU2 has no CLK, so it stays Inside),
    // exercising the gate rather than the FIX 1 deferral drop.
    let data = cmvs_two_layer_stream(true, false);
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "an empty external set declares no sequence header and must not suppress the \
         § 6.4.1 monotonic agreement check; report was: {report}"
    );
}

/// Builds the first two temporal units shared by the
/// `monotonic_output_order_*_provisional_*` tests: TU1 opens a CMVS (MSDO + CLK,
/// begin condition 1) and frame-confirms xlayer 0 to seq 0 (`monotonic 1`); xlayer 1
/// carries seq 1 (`monotonic 1`). TU2 frame-confirms xlayer 1 to seq 1. Both layers
/// agree on `monotonic_output_order_flag == 1` and the CMVS is committed `Inside`
/// after TU2. The caller appends a TU3 whose shape exercises the provisional-Inside
/// deferral.
pub(in crate::validator::tests) fn cmvs_provisional_inside_prefix() -> Vec<u8> {
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> opens the CMVS
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
    // CLK xlayer 0 referencing seq 0 begins the CMVS and frame-confirms xlayer 0
    // (kept before xlayer 1's header so coded extended layer units stay ascending).
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_monotonic(1, 1, true)); // xlayer 1 seq 1 monotonic 1
    data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS committed Inside
    // Frame-confirm xlayer 1 (ref seq 1); both layers now agree (monotonic 1).
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1));
    data
}

#[test]
fn monotonic_output_order_provisional_inside_clk_ending_tu_is_not_flagged() {
    // FIX 1 false-positive regression (Codex "Defer CMVS agreement until pre-CLK
    // headers are scoped"). § 7.3.2 end condition 2 (mirror
    // `07-decoding-process.md` lines 335-341): a temporal unit that begins a new
    // coded video sequence for an extended layer but contains no OBU_MSDO and no
    // activated global LCR ENDS the CMVS, so it sits OUTSIDE. When a same-id
    // reconfiguration of seq 0 (now monotonic 0) is observed at the *top* of such a
    // temporal unit, the CLK that ends the CMVS has not yet been observed, so the
    // committed `Inside` is provisional. The agreement check must defer its
    // header-time verdict and drop it once the CLK confirms the temporal unit ended
    // the CMVS — emitting at header time would be a false positive on a conformant
    // redefinition (§ 7.3.6 permits redefinition when a new CVS follows, mirror
    // `07-decoding-process.md` lines 608-611).
    let mut data = cmvs_provisional_inside_prefix();
    data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
    // Same-id reconfiguration of seq 0 with the disagreeing flag, observed BEFORE the
    // CLK that ends the CMVS for this temporal unit.
    data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
    // CLK xlayer 0 referencing seq 0: an MSDO-less CLK temporal unit ends the CMVS.
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "a header redefinition at the top of a CMVS-ending CLK temporal unit is outside \
         the CMVS once the CLK is seen; the provisional header-time verdict must be \
         dropped; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_provisional_inside_mid_cmvs_redefinition_is_flagged() {
    // FIX 1 deferral-still-emits guard. Same TU3 shape as the false-positive case but
    // the temporal unit stays *inside* the CMVS (a non-CLK frame replaces the CLK), so
    // the deferred header-time verdict must be emitted at temporal-unit flush. A
    // mid-CMVS redefinition that disagrees on monotonic_output_order_flag is a genuine
    // § 6.4.1 violation.
    let mut data = cmvs_provisional_inside_prefix();
    data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO, no CLK)
    // Same-id reconfiguration of seq 0 with the disagreeing flag; no CLK follows, so
    // the temporal unit stays inside the CMVS.
    data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
    // A non-CLK frame for xlayer 0 keeps the CMVS Inside across this temporal unit.
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "a mid-CMVS redefinition disagreeing on monotonic_output_order_flag must be \
         emitted at temporal-unit flush; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_provisional_inside_flushes_at_end_of_bitstream() {
    // FIX 1 end-of-bitstream flush. The disagreeing redefinition is the last OBU: the
    // temporal unit never receives a CLK, so it stays inside the CMVS (§ 7.3.2 end
    // condition 3 closes the CMVS only at the end of the bitstream). The deferred
    // verdict must be emitted when `finish` flushes the final temporal unit.
    let mut data = cmvs_provisional_inside_prefix();
    data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
    // Same-id reconfiguration disagreeing on the flag, with no following frame at all.
    data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "a disagreeing redefinition with no following CLK stays inside the CMVS and must \
         be emitted at the end-of-bitstream flush; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_provisional_inside_unknown_clk_is_not_flagged() {
    // FIX 1 Unknown guard. A CLK temporal unit with an activated-global-LCR candidate
    // present but no MSDO routes the CMVS tracker to `Unknown` (§ 7.3.2 end condition 2
    // needs "no activated global layer configuration record"; activation is not
    // modeled). The check fires only on `Inside`, so the deferred header-time verdict
    // for a redefinition at the top of such a temporal unit must be dropped.
    let mut data = cmvs_provisional_inside_prefix();
    data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
    data.extend(global_lcr_obu(0, 0b11, None)); // global LCR (xlayers 0, 1), no MSDO
    // Same-id reconfiguration disagreeing on the flag, observed before the CLK.
    data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
    // CLK xlayer 0 with the global LCR present and no MSDO -> tracker goes Unknown.
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "a CLK temporal unit with a global LCR and no MSDO routes the tracker to \
         Unknown; the provisional verdict must be dropped; report was: {report}"
    );
}
