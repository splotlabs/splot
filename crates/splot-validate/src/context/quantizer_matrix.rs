// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Quantizer-matrix state and frame-reference checks.

use super::*;

/// Per-level quantizer-matrix availability, recorded when a QM OBU specifies a level
/// (AV2 § 6.12 / § 7.3.8 foundation). Kept for future frame-reference checks; this
/// phase reads it only to cite the conflicting definition in a duplicate-level
/// diagnostic.
#[derive(Debug, Clone, Copy)]
pub(super) struct QmLevelRecord {
    /// `QmMLayerId[level]` (`None` models the spec's `-1` for a reset).
    pub(super) mlayer_id: Option<u8>,
    /// `QmTLayerId[level]` (`None` models the spec's `-1` for a reset).
    pub(super) tlayer_id: Option<u8>,
    /// `QmDataPresent[level]`: `true` for user-defined data, `false` for a default.
    pub(super) data_present: bool,
    /// `QmNumPlanes[level]`.
    pub(super) num_planes: u8,
}

/// Quantizer-matrix validator state (AV2 § 6.12).
///
/// The window fields (`seen_levels_since_coded_frame`, `qm_obu_seen_since_coded_frame`)
/// reset at each coded-frame boundary (see [`ValidatorContext::reset_coded_frame_window`])
/// and drive the § 6.12 duplicate-reset / duplicate-level checks. The `available`
/// array is monotonic per-level HLS state, foundation for the deferred frame
/// quantization-reference checks (`using_qmatrix` / `qm_*`, § 7.3.8 / § 6.17.6).
#[derive(Debug, Default)]
pub(super) struct QuantizerMatrixState {
    /// Levels (`qm_bit_map` bits) specified by a QM OBU since the last coded frame.
    pub(super) seen_levels_since_coded_frame: u16,
    /// `true` once any QM OBU has been observed since the last coded frame. A
    /// `qm_bit_map == 0` reset is only conformant as the first QM OBU in the window.
    pub(super) qm_obu_seen_since_coded_frame: bool,
    /// Per-level availability for frame-reference validation (AV2 § 7.3.8.9). A `Some`
    /// record means a QM OBU made the level available; `None` means no available record
    /// (never defined, or cleared by a `reset_qm()` for an unprotected level).
    pub(super) available: [Option<QmLevelRecord>; NUM_CUSTOM_QMS],
    /// `QmProtected[level]` (AV2 § 6.12 mirror :3134-3138 / § 5.5 / § 5.13): a bitmap of the
    /// levels protected from `reset_qm()` because a QM OBU (re)sent them in the *current*
    /// temporal unit. Cleared to 0 for every level at a temporal delimiter (§ 5.5 mirror
    /// :1626-1630); a QM OBU sets the bit for the levels it sends (§ 5.13 mirror
    /// :3010/:3033). `reset_qm()` (§ 5.18.2 mirror :4106-4108 / :4278-4286) clears
    /// `available[level]` only for *unprotected* levels, so a level re-sent this temporal
    /// unit survives a CLK/OLK/SWITCH/RAS reset (the QmProtected discipline).
    pub(super) qm_protected: u16,
    /// Per-level *poison* bitmap: a level whose `reset_qm()` was triggered by a SWITCH /
    /// RAS frame whose parse never **reached** (or could not **confirm**) the § 5.18.2
    /// reset call site (mirror :4279-4283) — a truncated header, an unresolvable core, or a
    /// SWITCH whose `restricted_prediction_switch` gate the parse never read. The reset's
    /// *availability* effect is then UNKNOWN (it might or might not have cleared the level),
    /// so neither the `available` record (would falsely under-report) nor a clear-to-`None`
    /// (would falsely *fire* `frame-header/qm-level-unavailable`) is sound. A poisoned level
    /// DROPS the § 7.3.8.9 availability judgment (stays silent), exactly like an
    /// externally-suppressed level. Re-grounded by a QM OBU re-sending the level (definitely
    /// available again) or by a *confirmed* reset (definitely cleared — see
    /// [`Self::reset_qm_availability_for_key`] /
    /// [`Self::reset_qm_availability_for_switch_or_ras`]); persists across temporal
    /// delimiters (it is HLS availability state, not the § 6.12 coded-frame window).
    /// Protected levels (`qm_protected` set) are never poisoned: a reset cannot touch them,
    /// so their availability stays known.
    pub(super) availability_poisoned: u16,
}

impl QuantizerMatrixState {
    /// Clears the §6.12 "between coded frames" window at a coded-frame boundary.
    pub(super) fn reset_coded_frame_window(&mut self) {
        self.seen_levels_since_coded_frame = 0;
        self.qm_obu_seen_since_coded_frame = false;
    }

    /// Clears `QmProtected[level] = 0` for every level at a temporal delimiter
    /// (AV2 § 5.5, mirror :1626-1630).
    pub(super) fn clear_qm_protected_at_temporal_delimiter(&mut self) {
        self.qm_protected = 0;
    }

    /// Applies the AV2 § 5.18.2 `reset_qm()` *availability* effect for a CLK / OLK (the
    /// unconditional `needsReset = 1` arm, mirror :5348-5360): every **unprotected** level's
    /// availability record is cleared to the spec defaults (`QmDataPresent = 0`,
    /// `QmMLayerId = QmTLayerId = -1`), which the validator models as the level becoming
    /// *unavailable* (`available[level] = None`). A level protected by a QM OBU re-sent in
    /// the current temporal unit (`qm_protected` bit set) is left untouched.
    ///
    /// This is the fully-decidable arm: a CLK / OLK resets *every* unprotected level
    /// regardless of layer (`needsReset = 1`), so no `MLayerPresenceMap` state is needed.
    ///
    /// A CLK / OLK reset is always *confirmed* (it is decidable from `obu_type` +
    /// `FirstPictureInTU` alone), so it also clears any prior `availability_poisoned` bit for
    /// the cleared level: the level is now definitively unavailable, re-grounding the
    /// § 7.3.8.9 judgment.
    pub(super) fn reset_qm_availability_for_key(&mut self) {
        for level in 0..NUM_CUSTOM_QMS {
            if (self.qm_protected >> level) & 1 == 0 {
                self.available[level] = None;
                self.availability_poisoned &= !(1u16 << level);
            }
        }
    }

    /// Applies the AV2 § 5.18.2 `reset_qm()` *availability* effect for a SWITCH (with
    /// `restricted_prediction_switch == 1`) or RAS frame (mirror :4278-4286 / :5350-5354).
    ///
    /// For these frame types `needsReset = QmMLayerId[level] == -1 ||
    /// MLayerPresenceMap[QmMLayerId[level]][obu_mlayer_id]` (mirror :5350-5352). Both arms are
    /// modeled: a level whose recorded `QmMLayerId == -1` (the record's `mlayer_id == None`,
    /// set by a `qm_bit_map == 0` reset QM OBU, or by a prior `reset_qm`) is unconditionally
    /// reset; a level with a recorded `QmMLayerId == m` (`mlayer_id == Some(m)`) resets when
    /// `MLayerPresenceMap[m][obu_mlayer_id] == 1` — i.e. the current frame's embedded layer is
    /// (transitively) present whenever the level's defining layer is, so the level cannot
    /// survive into this frame. `presence` is the § 5.4.1 [`MLayerPresenceMap`] of the frame's
    /// activated sequence header; when it is `None` (the activated header is unavailable) the
    /// presence arm cannot be decided, so the level is left available (never falsely cleared —
    /// the zero-false-positive direction for an availability reset). Protected levels
    /// (`qm_protected` set) are untouched.
    ///
    /// This is the *confirmed*-reset path: the caller only invokes it once the frame's core
    /// parse has reached the § 5.18.2 reset call site (mirror :4283) — a resolved RAS core,
    /// or a SWITCH core with `restricted_prediction_switch == 1`
    /// ([`ValidatorContext::apply_qm_reset_for_frame`]). A level it clears therefore also
    /// re-grounds out of any prior poison (`availability_poisoned` bit cleared): the level is
    /// now definitively unavailable.
    pub(super) fn reset_qm_availability_for_switch_or_ras(
        &mut self,
        obu_mlayer_id: EmbeddedLayerId,
        presence: Option<&MLayerPresenceMap>,
    ) {
        for level in 0..NUM_CUSTOM_QMS {
            if (self.qm_protected >> level) & 1 != 0 {
                continue;
            }
            let needs_reset = match self.available[level] {
                // No available record: a reset is a no-op on the record but still re-grounds
                // any poison, exactly as the `QmMLayerId == -1` arm did before.
                None => true,
                // `QmMLayerId == -1`: unconditionally reset (mirror :5351).
                Some(record) if record.mlayer_id.is_none() => true,
                // `QmMLayerId == m`: reset iff MLayerPresenceMap[m][obu_mlayer_id] (mirror
                // :5352). Undecidable without the activated header's presence map -> leave
                // available (no false unavailability).
                Some(record) => record.mlayer_id.is_some_and(|m| {
                    presence
                        .is_some_and(|p| p.is_present(EmbeddedLayerId::from_bits(m), obu_mlayer_id))
                }),
            };
            if needs_reset {
                self.available[level] = None;
                self.availability_poisoned &= !(1u16 << level);
            }
        }
    }

    /// Poisons the *availability* state for a SWITCH / RAS frame whose `reset_qm()` effect
    /// the validator cannot CONFIRM (AV2 § 5.18.2 mirror :4279-4283): the frame's core parse
    /// never reached the reset call site (a truncated header, an unresolvable core, or a
    /// SWITCH whose `restricted_prediction_switch` gate the parse never read). The reset
    /// might or might not have fired, so each unprotected level's availability becomes
    /// *unknown* — the § 7.3.8.9 judgment must DROP rather than fire or under-report.
    ///
    /// Only the `availability_poisoned` bit is set; the `available` record is left untouched
    /// (a poisoned level is dropped regardless of its record, and preserving the record lets
    /// a confirmed reset / resend re-ground precisely). Protected levels (`qm_protected`
    /// set) survive any reset, so their availability stays known and is never poisoned —
    /// symmetric with the protected-level skip in the confirmed-reset methods.
    pub(super) fn poison_qm_availability_for_unconfirmed_reset(&mut self) {
        for level in 0..NUM_CUSTOM_QMS {
            if (self.qm_protected >> level) & 1 == 0 {
                self.availability_poisoned |= 1u16 << level;
            }
        }
    }

    /// Whether `level`'s availability is poisoned (unknown) by an unconfirmed SWITCH / RAS
    /// reset. The § 7.3.8.9 availability check drops its judgment for a poisoned level. A
    /// `level >= NUM_CUSTOM_QMS` (never a custom slot) is never poisoned.
    pub(super) fn availability_poisoned(&self, level: usize) -> bool {
        level < NUM_CUSTOM_QMS && (self.availability_poisoned >> level) & 1 != 0
    }
}

/// Emits the locally decidable § 6.17.6.2 custom-QM plane-count diagnostics and the
/// § 7.3.8.9 quantizer-matrix *availability* diagnostic for a parsed frame
/// `setup_qm_params()` (AV2 v1.0.0 § 6.17.6.2,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-6-2`; § 7.3.8.9,
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3-8-9`).
///
/// Each `qm_y[i]` / `qm_u[i]` / `qm_v[i]` less than `NUM_CUSTOM_QMS` references a custom
/// quantizer-matrix slot. Two requirements are checked:
///
/// - **§ 6.17.6.2 plane count.** A referenced custom slot's `QmNumPlanes` must equal the
///   active sequence's `NumPlanes`. Only slots with a recorded QM OBU state are checked;
///   a slot with no record is owned by the availability check below.
/// - **§ 7.3.8.9 availability.** "When using_qmatrix is equal to 1 in a frame header, the
///   quantization matrix levels referenced by qm_y, qm_u, and qm_v shall be available to
///   the decoding process, by inclusion of a quantization matrix OBU in the bitstream or by
///   provision through external means." A referenced custom slot with NO available record
///   (`qm_state.available[level] == None`) is unavailable.
///
/// **External-means suppression (zero-false-positive discipline).**
/// [`ExternalHlsSet`](crate::options::ExternalHlsSet) cannot express quantizer-matrix OBUs
/// (only sequence headers and operating point sets), so under any
/// [`ExternalHlsMode::Provided`] the levels MAY be supplied externally without being listed
/// — exactly the inexpressible-kind case the blanket "any Provided suppresses" policy covers
/// (matching the film-grain availability check). The availability diagnostic therefore fires
/// only under [`ExternalHlsMode::Disabled`]. The `available[]` state honors the § 5.5 /
/// § 5.13 / § 5.18.2 QmProtected `reset_qm()` discipline (a CLK / OLK at FirstPictureInTU, or
/// a RAS / restricted SWITCH, clears unprotected levels), so a level reset out of a previous
/// temporal unit and not re-sent in the current one is correctly judged unavailable.
///
/// **Unconfirmed-reset poison.** A SWITCH / RAS frame whose core parse did not CONFIRM it
/// reached the § 5.18.2 `reset_qm()` call site (mirror :4283) — a truncated header, an
/// unresolvable core, or a SWITCH whose `restricted_prediction_switch` gate was never read —
/// POISONS the unprotected levels' availability instead of clearing them
/// ([`ValidatorContext::apply_qm_reset_for_frame`]). A poisoned level
/// ([`QuantizerMatrixState::availability_poisoned`]) DROPS its § 7.3.8.9 judgment (neither
/// the false-fire of a clear-to-`None` nor the stale "available" of a skip), exactly like an
/// externally-suppressed level, until a QM OBU re-sends it or a later confirmed reset grounds
/// it. This is the QM-availability analogue of the § 7.23 unconfirmed-effect staging gate.
///
/// The § 6.17.6.2 layer-dependency constraints
/// (MLayerDependencyMap[obu_mlayer_id][QmMLayerId[level]] == 1 and the TLayerDependencyMap
/// analogue) now land here for every referenced custom level with a recorded layer identity
/// (QmMLayerId >= 0): frame-header/qm-mlayer-dependency-missing /
/// frame-header/qm-tlayer-dependency-missing, mirroring the film-grain dependency checks.
///
/// Returns the referenced custom levels that were linearly available in-band under
/// external-disabled (so the linear `frame-header/qm-level-unavailable` did NOT fire) — the
/// caller buffers each as a § 7.3.8.1 random-access-point replay reference, keeping the
/// replay predicate disjoint from this linear check. Empty when no replay reference applies
/// (no using_qmatrix, external-HLS provided, or every referenced level unavailable/poisoned).
pub(super) fn frame_qm_reference_checks(
    setup_qm: &SetupQmParams,
    active_sequence: &SequenceHeader,
    qm_state: &QuantizerMatrixState,
    options: &ValidationOptions,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) -> Vec<u8> {
    // The qm_y/qm_u/qm_v syntax (and its conformance bullets) exists only when
    // using_qmatrix == 1 (AV2 § 5.18.6.2).
    if !setup_qm.using_qmatrix {
        return Vec::new();
    }
    // AV2 § 6.4.1: NumPlanes = Monochrome ? 1 : 3.
    let num_planes: u8 = if active_sequence.general.chroma_format_idc.is_monochrome() {
        1
    } else {
        3
    };
    let qm_num = usize::from(setup_qm.pic_qm_num_minus_1) + 1;
    // Distinct referenced custom slots: qm_uv_same_as_y / shared-UV copies and
    // repeated levels across the qmNum sets reference the same slot, which violates
    // (or satisfies) § 6.17.6.2 / § 7.3.8.9 once, not once per syntax element.
    let mut referenced = [false; NUM_CUSTOM_QMS];
    for set in setup_qm.levels.iter().take(qm_num) {
        // qm_y[i] is always present; qm_u[i] / qm_v[i] exist only when NumPlanes > 1
        // (AV2 § 5.18.6.2) — the parsed zeroed placeholders for a monochrome
        // sequence are not bitstream references.
        if let Some(slot) = referenced.get_mut(usize::from(set.qm_y)) {
            *slot = true;
        }
        if num_planes > 1 {
            if let Some(slot) = referenced.get_mut(usize::from(set.qm_u)) {
                *slot = true;
            }
            if let Some(slot) = referenced.get_mut(usize::from(set.qm_v)) {
                *slot = true;
            }
        }
    }
    // AV2 § 7.3.8.9 availability fires only when external HLS cannot supply the levels
    // (ExternalHlsSet cannot express QM OBUs, so any Provided mode means the levels MAY be
    // external — the inexpressible-kind blanket suppression).
    let availability_decidable = matches!(options.external_hls, ExternalHlsMode::Disabled);
    // The referenced custom levels that resolved linearly-available (under Disabled), buffered
    // by the caller for the § 7.3.8.1 random-access-point replay — disjoint from the linear
    // checks below (an unavailable/poisoned level is not replayed).
    let mut replay_levels = Vec::new();
    for (level, _) in referenced
        .iter()
        .enumerate()
        .filter(|(_, referenced)| **referenced)
    {
        // A level POISONED by a SWITCH / RAS reset whose effect the validator could not
        // confirm (a truncated header that never reached the § 5.18.2 reset call site, or a
        // SWITCH whose restricted_prediction_switch gate the parse never read) has UNKNOWN
        // availability: the reset may or may not have cleared it. Drop both the availability
        // and the plane-count judgments — guessing either way would be a false positive
        // (clear-to-fire) or a false negative (stale "available"). The poison is lifted by a
        // QM OBU re-sending the level, or by a later confirmed reset
        // (QuantizerMatrixState::availability_poisoned).
        if qm_state.availability_poisoned(level) {
            continue;
        }
        let Some(record) = qm_state.available[level] else {
            // AV2 § 7.3.8.9: the referenced custom level has no available record (no QM OBU
            // ever defined it, or a reset_qm() cleared it and it was not re-sent in the
            // current temporal unit). Decidable only under external-disabled; a poisoned /
            // externally-suppressed state stays silent (no guessing).
            if availability_decidable {
                report.push(frame_header_error(
                    "frame-header/qm-level-unavailable",
                    "7.3.8.9",
                    obu,
                    format!(
                        "setup_qm_params() has using_qmatrix == 1 and references custom \
                         quantizer matrix level {level}, but no quantizer matrix OBU has made \
                         that level available (§7.3.8.9: the referenced QM levels must be \
                         available, by inclusion of a QM OBU or external means)"
                    ),
                ));
            }
            continue;
        };
        if record.num_planes != num_planes {
            report.push(frame_header_error(
                "frame-header/qm-plane-count-mismatch",
                "6.17.6.2",
                obu,
                format!(
                    "setup_qm_params() references custom quantizer matrix level {level}, whose \
                     recorded QmNumPlanes {} differs from the active sequence's NumPlanes \
                     {num_planes}",
                    record.num_planes
                ),
            ));
        }
        // AV2 § 6.17.6.2 (mirror :5413-5419): when a referenced custom level's defining QM OBU
        // recorded a layer identity (QmMLayerId[level] >= 0, i.e. record.mlayer_id == Some(m)),
        // the frame's embedded/temporal layer must DEPEND on that defining layer —
        // MLayerDependencyMap[obu_mlayer_id][QmMLayerId[level]] == 1 and
        // TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][QmTLayerId[level]] == 1. A level
        // reset to defaults (QmMLayerId == -1, record.mlayer_id == None) has no defining layer
        // and is not subject to the constraint. Decidable from the recorded layer identity, the
        // frame's obu_mlayer_id/obu_tlayer_id, and the activated header's § 5.4.1 maps — the
        // proven pattern of frame-header/film-grain-{mlayer,tlayer}-dependency-missing.
        //
        // Suppressed under any Provided external-HLS mode (gated on `availability_decidable`,
        // i.e. Disabled), exactly like the film-grain dependency check: QM OBUs cannot be
        // expressed by `ExternalHlsSet`, so the level's recorded layer identity MAY be supplied
        // externally, and the activated sequence header (whose § 5.4.1 maps this check reads)
        // MAY itself be external with different maps — either makes the in-band join unsound
        // (Codex P2). Only the external-disabled case is decidable from the bitstream alone.
        if availability_decidable && let Some(qm_mlayer) = record.mlayer_id {
            let general = &active_sequence.general;
            let frame_mlayer = obu.header.embedded_layer_id;
            if !general
                .mlayer_dependency_map
                .depends_on(frame_mlayer, EmbeddedLayerId::from_bits(qm_mlayer))
            {
                report.push(frame_header_error(
                    "frame-header/qm-mlayer-dependency-missing",
                    "6.17.6.2",
                    obu,
                    format!(
                        "setup_qm_params() at obu_mlayer_id {fm} references custom quantizer \
                         matrix level {level} defined at embedded layer {qm_mlayer}, but the \
                         active sequence header's MLayerDependencyMap[{fm}][{qm_mlayer}] is 0 \
                         (§ 6.17.6.2)",
                        fm = frame_mlayer.get(),
                    ),
                ));
            }
            // The § 6.17.6.2 TLayerDependencyMap constraint is gated on the SAME QmMLayerId >= 0
            // condition (mirror :5417); QmTLayerId is recorded with QmMLayerId, so a Some
            // mlayer_id implies a Some tlayer_id (a reset clears both to -1 together).
            if let Some(qm_tlayer) = record.tlayer_id {
                let frame_tlayer = obu.header.temporal_layer_id;
                if !general.tlayer_dependency_map.depends_on(
                    frame_mlayer,
                    frame_tlayer,
                    TemporalLayerId::from_bits(qm_tlayer),
                ) {
                    report.push(frame_header_error(
                        "frame-header/qm-tlayer-dependency-missing",
                        "6.17.6.2",
                        obu,
                        format!(
                            "setup_qm_params() at obu_tlayer_id {ft} references custom quantizer \
                             matrix level {level} defined at temporal layer {qm_tlayer}, but the \
                             active sequence header's \
                             TLayerDependencyMap[{fm}][{ft}][{qm_tlayer}] is 0 (§ 6.17.6.2)",
                            ft = frame_tlayer.get(),
                            fm = frame_mlayer.get(),
                        ),
                    ));
                }
            }
        }
        // The level is linearly available in-band: buffer it for the § 7.3.8.1 replay (only
        // under Disabled — under any Provided mode the level MAY be external, and the QM
        // family is inexpressible by ExternalHlsSet, so the replay would suppress anyway).
        if availability_decidable {
            replay_levels.push(level as u8);
        }
    }
    replay_levels
}

impl ValidatorContext {
    /// Applies a frame header's `reset_qm()` *availability* effect to the per-level QM
    /// availability state (AV2 § 7.3.8.9, mirror :847-858; § 5.18.2 mirror :4106-4108 /
    /// :4278-4286; `reset_qm()` mirror :5346-5363).
    ///
    /// "Quantization matrix levels from previous temporal units are reset at the first OBU
    /// in a temporal unit with obu_type equal to OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY
    /// or OBU_SWITCH or OBU_RAS_FRAME (the QmProtected array is used to avoid the reset of
    /// levels sent in the current temporal unit). … If obu_type is equal to OBU_SWITCH, the
    /// reset only applies if restricted_prediction_switch is equal to 1."
    ///
    /// - CLK / OLK: `keyFrame && FirstPictureInTU` (a CLK / OLK *is* a KEY frame). The reset
    ///   is decidable from obu_type + `FirstPictureInTU` alone, with the unconditional
    ///   `needsReset = 1` arm (every unprotected level cleared). Always confirmed.
    /// - RAS: always resets *once the parse reaches the reset call site*. The spec's
    ///   `reset_qm()` call (mirror :4283) sits AFTER `restricted_prediction_switch`
    ///   (mirror :4209), `num_key_ref_frames` (mirror :4244-4257), and the output flags — so
    ///   the validator must not assume the reset on the `obu_type` alone. A RAS core that
    ///   resolves to `Some` necessarily read through those fields and entered the inter path
    ///   (`finish_inter_control`, mirror :4351+), which is past :4283, so a `Some` RAS core
    ///   *confirms* the reset; a `None` core (truncated before the bit, unresolvable
    ///   sequence, or a malformed prefix) leaves the reset UNCONFIRMED.
    /// - SWITCH: resets only when the parsed `restricted_prediction_switch == 1`; the bit is
    ///   read on the SWITCH / RAS arm of the core parse and recorded on the core. A `Some`
    ///   core with the bit `0` *confirms NO reset* (nothing to do); a `None` core leaves the
    ///   reset UNCONFIRMED (the gate bit was never read, so the spec might or might not have
    ///   cleared the levels).
    ///
    /// The SWITCH / RAS arm uses the partial `needsReset` model (only the
    /// `QmMLayerId == -1` arm is provable; see
    /// [`QuantizerMatrixState::reset_qm_availability_for_switch_or_ras`]).
    ///
    /// **Unconfirmed-effect discipline (the PR #63 § 7.23 staging gate, applied to QM
    /// availability).** When the RAS / SWITCH reset is UNCONFIRMED, the validator must
    /// neither clear the level to `None` (that would falsely *fire*
    /// `frame-header/qm-level-unavailable` for a level the reset may have left available) nor
    /// silently skip it (that would falsely assert the level is *still available* when the
    /// reset may have cleared it). Both directions guess. The sound treatment is to POISON
    /// the level's availability ([`QuantizerMatrixState::poison_qm_availability_for_unconfirmed_reset`]):
    /// the § 7.3.8.9 judgment DROPS until the level is re-grounded by a QM OBU re-sending it,
    /// or by a later *confirmed* reset that grounds it as definitively unavailable.
    pub(super) fn apply_qm_reset_for_frame(
        &mut self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        resolved: Option<SequenceHeaderId>,
    ) {
        match obu.header.obu_type {
            // CLK / OLK reset_qm() runs at `keyFrame && FirstPictureInTU` (mirror :4106 /
            // :4279-4283). It is decidable from `obu_type` + `FirstPictureInTU` ALONE — no
            // sequence-dependent read precedes it — so it clears regardless of whether the
            // frame's referenced sequence header resolves (codex F1(a)).
            ObuType::ClosedLoopKey | ObuType::OpenLoopKey if first_picture_in_tu => {
                self.qm.reset_qm_availability_for_key();
            }
            // RAS / SWITCH reset_qm() sits at mirror :4279-4283, PAST sequence-dependent reads
            // (`restricted_prediction_switch`, `num_key_ref_frames` / `ref_long_term_id[i]`),
            // so confirming it needs the parsed bits. An unresolvable reference
            // (`resolved == None`) cannot prove the reset fired -> POISON, never skip (codex
            // F1(b)). A resolvable reference confirms from the parsed `reached_qm_reset` fact:
            // the parse passes the :4283 call site with the trigger met. The fact survives a
            // facts-preserving inter-control truncation (`StoppedInsideInterControl` keeps the
            // core), so a RAS / restricted SWITCH that reaches the reset and then truncates
            // inside the inter region (e.g. EOF in `ref_frame_idx`) still CONFIRMS — it no
            // longer requires the whole core parse to complete (codex F2). A parse that stops
            // BEFORE the call site leaves the fact `false` -> the reset stays unconfirmed ->
            // poison. A SWITCH with `restricted_prediction_switch == 0` never sets the fact
            // (its reset_qm() trigger is false), which is the confirmed NO-reset case the spec
            // gate describes — the parse reaches the inter region but `reset_qm()` did not run.
            ObuType::RasFrame | ObuType::Switch => {
                // The § 5.18.2 SWITCH/RAS reset_qm() presence arm reads
                // MLayerPresenceMap[QmMLayerId[level]][obu_mlayer_id] (mirror :5352), where the
                // presence map is the § 5.4.1 reflexive-transitive closure of the frame's
                // ACTIVATED sequence header's MLayerDependencyMap. Derive it once (owned, so the
                // immutable `sequence_headers` borrow ends before the mutable `self.qm` reset).
                let presence = resolved
                    .and_then(|seq_id| self.sequence_headers.get(&seq_id))
                    .map(|header| header.general.mlayer_dependency_map.presence_map());
                let obu_mlayer_id = obu.header.embedded_layer_id;
                match resolved.and_then(|seq_id| {
                    self.frame_core_against_resolved_header(obu, first_picture_in_tu, seq_id)
                }) {
                    Some(core) if core.reached_qm_reset => {
                        self.qm.reset_qm_availability_for_switch_or_ras(
                            obu_mlayer_id,
                            presence.as_ref(),
                        );
                    }
                    // A resolvable SWITCH whose parse reached the reset point but whose
                    // `restricted_prediction_switch == 0` (so `reached_qm_reset == false`)
                    // confirms NO reset — nothing to do.
                    Some(core)
                        if obu.header.obu_type == ObuType::Switch
                            && core.restricted_prediction_switch == Some(false) => {}
                    // Unresolvable, or a resolvable core that stopped before the reset call
                    // site (the SWITCH gate bit unread, or a RAS truncated mid-prefix /
                    // mid-`ref_long_term_id`): the reset is unconfirmed.
                    _ => self.qm.poison_qm_availability_for_unconfirmed_reset(),
                }
            }
            _ => {}
        }
    }

    /// Observes a quantizer matrix OBU (§ 5.13), running the locally-checkable § 6.12
    /// duplicate-reset / duplicate-level diagnostics and recording per-level
    /// availability. A parse failure or malformed payload tail is silent (the OBU is
    /// not-yet-validated coverage in that case), consistent with the OPS observer.
    pub(super) fn observe_quantizer_matrix(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(qm) = parse_quantizer_matrix(&mut reader) else {
            return;
        };
        if finish_obu_payload(
            &mut reader,
            obu.payload,
            obu.header.obu_type.is_extensible_obu(),
        )
        .is_err()
        {
            return;
        }
        // Emit diagnostics against the window/availability captured before this OBU,
        // then fold this OBU into the state.
        self.emit_quantizer_matrix_diagnostics(obu, &qm, report);
        self.check_quantizer_matrix(obu, &qm);
    }

    /// Emits the § 6.12 quantizer-matrix diagnostics for `qm`, reading the window and
    /// per-level availability captured before this OBU.
    pub(super) fn emit_quantizer_matrix_diagnostics(
        &self,
        obu: &ObuEnvelope<'_>,
        qm: &QuantizerMatrixObu,
        report: &mut ValidationReport,
    ) {
        if qm.qm_bit_map == 0 {
            // AV2 § 6.12: only the first quantizer matrix OBU between coded frames may
            // have qm_bit_map == 0.
            if self.qm.qm_obu_seen_since_coded_frame {
                report.push(
                    Diagnostic::error(
                        "qm/duplicate-reset-between-frames",
                        "a quantizer matrix OBU with qm_bit_map == 0 is only permitted as the \
                         first quantizer matrix OBU between coded frames",
                    )
                    .with_spec_section("6.12")
                    .with_byte_offset(obu.offset),
                );
            }
            return;
        }

        // AV2 § 6.12: the same quantizer matrix level must not be specified twice
        // between coded frames.
        let overlap = self.qm.seen_levels_since_coded_frame & qm.qm_bit_map;
        for level in 0..NUM_CUSTOM_QMS {
            if overlap & (1 << level) == 0 {
                continue;
            }
            let prior = match self.qm.available[level] {
                Some(record) => format!(
                    " (previously specified by a quantizer matrix OBU at embedded layer {}, \
                     temporal layer {}, data_present={}, num_planes={})",
                    // QmMLayerId / QmTLayerId are -1 in the spec for a reset.
                    record
                        .mlayer_id
                        .map_or_else(|| "-1".to_owned(), |m| m.to_string()),
                    record
                        .tlayer_id
                        .map_or_else(|| "-1".to_owned(), |t| t.to_string()),
                    record.data_present,
                    record.num_planes,
                ),
                None => String::new(),
            };
            report.push(
                Diagnostic::error(
                    "qm/duplicate-level-between-frames",
                    format!(
                        "quantizer matrix level {level} is specified twice between coded \
                         frames{prior}"
                    ),
                )
                .with_spec_section("6.12")
                .with_byte_offset(obu.offset),
            );
        }
    }

    /// Updates the §6.12 window and per-level availability after the diagnostics.
    pub(super) fn check_quantizer_matrix(
        &mut self,
        obu: &ObuEnvelope<'_>,
        qm: &QuantizerMatrixObu,
    ) {
        self.qm.qm_obu_seen_since_coded_frame = true;
        if qm.qm_bit_map == 0 {
            // AV2 § 5.13 reset path (mirror :3006-3018): every custom level returns to its
            // defaults (QmDataPresent = 0, QmMLayerId = QmTLayerId = -1, QmNumPlanes =
            // numPlanes) and `QmProtected[level] = 1` for every level. The reset makes the
            // level "available as default" (a QM OBU was sent), so the availability record is
            // a default record (not `None`); a frame-reference check after a reset must not
            // see stale layer/data state from a previously defined matrix.
            for record in &mut self.qm.available {
                *record = Some(QmLevelRecord {
                    mlayer_id: None,
                    tlayer_id: None,
                    data_present: false,
                    num_planes: qm.num_planes,
                });
            }
            // A QM OBU re-grounds availability: every level is now definitely available (as a
            // default), so any prior unconfirmed-reset poison is lifted for all levels.
            self.qm.availability_poisoned = 0;
            // AV2 § 5.13 mirror :3010: QmProtected[level] = 1 for every level.
            self.qm.qm_protected = (1u16 << NUM_CUSTOM_QMS) - 1;
            // AV2 § 7.3.8.1 replay: a reset-to-defaults makes EVERY custom level available
            // (as a default), so record a (re)send for all of them — a later frame reference
            // to any level is satisfied at a random access point only if some QM OBU (this
            // reset included) sent it in or after that start.
            for level in 0..NUM_CUSTOM_QMS {
                self.rap_replay.note_resend(
                    RapHlsKey::QmLevel { level: level as u8 },
                    obu.header.extended_layer_id,
                );
            }
            return;
        }
        self.qm.seen_levels_since_coded_frame |= qm.qm_bit_map;
        for level in &qm.levels {
            let index = level.level as usize;
            if index < NUM_CUSTOM_QMS {
                self.qm.available[index] = Some(QmLevelRecord {
                    mlayer_id: Some(obu.header.embedded_layer_id.get()),
                    tlayer_id: Some(obu.header.temporal_layer_id.get()),
                    data_present: !level.is_default,
                    num_planes: qm.num_planes,
                });
                // A QM OBU re-sending this level re-grounds its availability (definitely
                // available again), lifting any prior unconfirmed-reset poison for it.
                self.qm.availability_poisoned &= !(1u16 << index);
                // AV2 § 5.13 mirror :3033: QmProtected[level] = 1 for each sent level, so a
                // level (re)sent in this temporal unit survives a later reset_qm().
                self.qm.qm_protected |= 1u16 << index;
                // AV2 § 7.3.8.1 replay: record the (re)send of this custom level.
                self.rap_replay.note_resend(
                    RapHlsKey::QmLevel { level: level.level },
                    obu.header.extended_layer_id,
                );
            }
        }
    }
}
