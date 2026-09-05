// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Quantizer-matrix state and frame-reference checks.

use super::*;

/// Per-level quantizer-matrix availability, recorded when a QM OBU specifies a level
/// (AV2 § 6.12 / § 7.3.8 foundation). Frame-reference checks use the stored layer
/// identity and data shape; duplicate-level diagnostics cite the conflicting definition.
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

/// QM availability and duplicate checks (§ 6.12 / § 7.3.8.9). The coded-frame
/// window resets independently of availability. Resent levels are protected for
/// the temporal unit; confirmed resets clear unprotected levels and prior poison.
/// Unconfirmed resets poison unprotected levels until a confirmed reset or resend.
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

    /// Applies the unconditional CLK/OLK reset to unprotected levels (§ 5.18.2).
    /// Cleared levels become definitively unavailable, removing any prior poison.
    pub(super) fn reset_qm_availability_for_key(&mut self) {
        for level in 0..NUM_CUSTOM_QMS {
            if (self.qm_protected >> level) & 1 == 0 {
                self.available[level] = None;
                self.availability_poisoned &= !(1u16 << level);
            }
        }
    }

    /// Applies a confirmed SWITCH/RAS reset (§ 5.18.2). Unprotected levels reset when
    /// their defining mlayer is absent or its presence map includes this frame’s layer.
    /// Without a presence map the latter case is undecidable and retains availability.
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
                None => true,
                Some(record) if record.mlayer_id.is_none() => true,
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

    /// Marks unprotected availability unknown when the SWITCH/RAS reset is unconfirmed.
    /// Keep records for later regrounding; protected levels survive either reset outcome.
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
    if !setup_qm.using_qmatrix {
        return Vec::new();
    }
    let num_planes: u8 = if active_sequence.general.chroma_format_idc.is_monochrome() {
        1
    } else {
        3
    };
    let qm_num = usize::from(setup_qm.pic_qm_num_minus_1) + 1;
    let mut referenced = [false; NUM_CUSTOM_QMS];
    for set in setup_qm.levels.iter().take(qm_num) {
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
    let availability_decidable = matches!(options.external_hls, ExternalHlsMode::Disabled);
    let mut replay_levels = Vec::new();
    for (level, _) in referenced
        .iter()
        .enumerate()
        .filter(|(_, referenced)| **referenced)
    {
        if qm_state.availability_poisoned(level) {
            continue;
        }
        let Some(record) = qm_state.available[level] else {
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
    /// **Unconfirmed-effect discipline (§ 7.23 staging gate, applied to QM
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
            ObuType::ClosedLoopKey | ObuType::OpenLoopKey if first_picture_in_tu => {
                self.qm.reset_qm_availability_for_key();
            }
            ObuType::RasFrame | ObuType::Switch => {
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
                    Some(core)
                        if obu.header.obu_type == ObuType::Switch
                            && core.restricted_prediction_switch == Some(false) => {}
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

        let overlap = self.qm.seen_levels_since_coded_frame & qm.qm_bit_map;
        for level in 0..NUM_CUSTOM_QMS {
            if overlap & (1 << level) == 0 {
                continue;
            }
            let prior = match self.qm.available[level] {
                Some(record) => format!(
                    " (previously specified by a quantizer matrix OBU at embedded layer {}, \
                     temporal layer {}, data_present={}, num_planes={})",
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
            for record in &mut self.qm.available {
                *record = Some(QmLevelRecord {
                    mlayer_id: None,
                    tlayer_id: None,
                    data_present: false,
                    num_planes: qm.num_planes,
                });
            }
            self.qm.availability_poisoned = 0;
            self.qm.qm_protected = (1u16 << NUM_CUSTOM_QMS) - 1;
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
                self.qm.availability_poisoned &= !(1u16 << index);
                self.qm.qm_protected |= 1u16 << index;
                self.rap_replay.note_resend(
                    RapHlsKey::QmLevel { level: level.level },
                    obu.header.extended_layer_id,
                );
            }
        }
    }
}
