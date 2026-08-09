// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coded-video-sequence boundary tracking.

use super::*;

/// Exact coded-video-sequence boundary tracker (AV2 § 7.3.6,
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3-6`).
///
/// "A new coded video sequence for an extended layer is defined to start at each
/// temporal unit that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY in
/// the coded extended layer unit corresponding to the extended layer" (§ 7.3.6).
/// Two consequences drive this design:
///
/// - The boundary event is per extended layer and per temporal unit: the first
///   `OBU_CLOSED_LOOP_KEY` observed for an `obu_xlayer_id` within a temporal unit
///   starts the new coded video sequence (later CLKs in the same temporal unit are
///   idempotent). The raw OBU header suffices — § 7.3.6 is stated at the OBU level,
///   so an unparsable CLK payload still starts the sequence. OLK / RAS OBUs do NOT
///   start one during sequential decoding (§ 2 "Coded video sequence"; § 7.4.4:
///   "During sequential decoding, the process does not start a new coded video
///   sequence for the extended layer").
/// - The new sequence starts *at the temporal unit*, so OBUs of the same extended
///   layer earlier in that temporal unit (e.g. the sequence header preceding the
///   activating CLK) already belong to the NEW coded video sequence — and the
///   validator cannot know a CLK is still coming when it observes them. CVS-scoped
///   comparisons whose baseline record came from an *earlier* temporal unit are
///   therefore deferred and flushed when the temporal unit completes: dropped when
///   the record's extended layer started a coded video sequence in that temporal
///   unit, emitted otherwise. Same-temporal-unit comparisons are always within one
///   coded video sequence and are emitted eagerly.
///
/// Records keyed under `GLOBAL_XLAYER_ID` have no single owning extended layer; as a
/// documented approximation their deferred diagnostics are dropped when ANY extended
/// layer started a coded video sequence in the completed temporal unit (sound — it
/// only drops comparisons, never inventing one).
#[derive(Debug, Default)]
pub(super) struct CvsTracker {
    /// Index of the current temporal unit; incremented at each global
    /// `OBU_TEMPORAL_DELIMITER` (AV2 § 7.3.7).
    pub(super) tu_index: u64,
    /// For each extended layer, the temporal unit in which its most recent coded
    /// video sequence started (§ 7.3.6 CLK boundary events).
    pub(super) cvs_started_in_tu: BTreeMap<ExtendedLayerId, u64>,
    /// Monotonic count of coded-video-sequence starts across the whole bitstream,
    /// incremented once per § 7.3.6 CLK boundary event (idempotent within a temporal
    /// unit). A global (`GLOBAL_XLAYER_ID`) scope, which spans the whole multistream
    /// and has no single owning extended layer, uses this counter as its CVS epoch:
    /// any CLK in any extended layer bumps it, so two observations sharing this value
    /// are guaranteed to lie within one coded video sequence of every layer (sound —
    /// it only adds boundaries, never removes one). See [`CvsTracker::cvs_generation_epoch`].
    pub(super) cvs_generation: u64,
    /// For each extended layer, the [`CvsTracker::cvs_generation`] value at which its
    /// most recent coded video sequence started — the per-layer CVS epoch used to
    /// scope the § 6.10.5 buffer-delay sum-constancy comparison.
    pub(super) cvs_generation_for: BTreeMap<ExtendedLayerId, u64>,
    /// Deferred cross-temporal-unit CVS-scoped diagnostics, tagged with the extended
    /// layer that scopes the comparison; flushed when the temporal unit completes. Each
    /// entry may carry an optional `on_drop` replacement diagnostic emitted in place of
    /// the primary when the primary is dropped because a § 7.3.6 coded-video-sequence
    /// boundary was crossed (the comparison was genuinely cross-CVS after all). The
    /// mechanism is rule-id-agnostic: a caller that wants no replacement passes `None`.
    pub(super) pending_cross_tu: Vec<PendingCrossTu>,
}

/// The flush polarity of a deferred [`PendingCrossTu`] comparison. The two § 7.3.6
/// boundary events (`CvsTracker::start_cvs` and `CvsTracker::flush_completed_tu`) handle
/// a pending entry oppositely depending on which side of the CLK boundary the comparison
/// is sound on; the polarity selects which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingPolarity {
    /// The comparison's baseline is in an EARLIER temporal unit, so the comparison is an
    /// intra-CVS assertion that a same-temporal-unit CLK could falsify. Emit the primary
    /// when the temporal unit completes without a CVS start for the layer; on a CVS start
    /// drop the primary and emit any `on_drop` replacement (the comparison spanned the
    /// § 7.3.6 boundary after all). This is the deferred-error polarity.
    CvsScoped,
    /// Both observations are in the CURRENT temporal unit but no coded video sequence has
    /// started for the layer yet (the pre-first-CLK silence path). Per § 7.3.6 the whole
    /// temporal unit containing a CLK belongs to the NEW coded video sequence, so a CLK
    /// later in this temporal unit pulls BOTH observations into one coded video sequence
    /// and the change is intra-CVS — emit the primary on the CVS start. If the temporal
    /// unit instead completes with no CLK for the layer, the observations remain in no
    /// coded video sequence (the § 6.10.5 random-access-point precondition is
    /// unsatisfied), so drop the primary silently. This is the inverse of `CvsScoped`.
    PreCvs,
}

/// One deferred cross-temporal-unit CVS-scoped diagnostic (see
/// [`CvsTracker::pending_cross_tu`]).
#[derive(Debug)]
pub(super) struct PendingCrossTu {
    /// The extended layer scoping the comparison; `GLOBAL_XLAYER_ID` for a record with
    /// no single owning extended layer.
    pub(super) xlayer: ExtendedLayerId,
    /// Which § 7.3.6 boundary event emits this entry's primary (see [`PendingPolarity`]).
    pub(super) polarity: PendingPolarity,
    /// The primary diagnostic. For [`PendingPolarity::CvsScoped`] it is emitted when the
    /// temporal unit completes without a CVS start for the layer; for
    /// [`PendingPolarity::PreCvs`] it is emitted on a CVS start for the layer.
    pub(super) primary: Diagnostic,
    /// The replacement emitted instead of `primary` when `primary` is dropped because a
    /// coded-video-sequence boundary was crossed (the comparison spanned the boundary).
    /// Used only by [`PendingPolarity::CvsScoped`]; a [`PendingPolarity::PreCvs`] entry
    /// that is dropped (its temporal unit closed with no CLK) leaves the observations in
    /// no coded video sequence, so its dropped comparison must simply vanish (`None`).
    pub(super) on_drop: Option<Diagnostic>,
}

impl CvsTracker {
    /// Records a § 7.3.6 boundary event: a CLK OBU for `xlayer` starts a new coded
    /// video sequence at the current temporal unit. Pending deferred diagnostics for
    /// `xlayer` are resolved by their [`PendingPolarity`]:
    ///
    /// - [`PendingPolarity::CvsScoped`]: the comparison spanned this CVS boundary, so the
    ///   primary is dropped and any `on_drop` replacement is emitted in its place.
    /// - [`PendingPolarity::PreCvs`]: per § 7.3.6 the whole temporal unit containing this
    ///   CLK belongs to the new coded video sequence, so both observations are now
    ///   intra-CVS — the primary is emitted.
    ///
    /// Both pending kinds are recorded only in the current temporal unit (a `CvsScoped`
    /// entry whose baseline came from an earlier temporal unit, a `PreCvs` entry whose
    /// observations are both in this temporal unit) and are flushed at the temporal-unit
    /// boundary, so every entry present here is necessarily tagged to this temporal unit.
    /// Idempotent within a temporal unit.
    pub(super) fn start_cvs(&mut self, xlayer: ExtendedLayerId, report: &mut ValidationReport) {
        if self.cvs_started_in_tu.get(&xlayer) != Some(&self.tu_index) {
            self.cvs_generation += 1;
            self.cvs_generation_for.insert(xlayer, self.cvs_generation);
        }
        self.cvs_started_in_tu.insert(xlayer, self.tu_index);
        let mut retained = Vec::with_capacity(self.pending_cross_tu.len());
        for entry in std::mem::take(&mut self.pending_cross_tu) {
            if entry.xlayer == xlayer {
                match entry.polarity {
                    PendingPolarity::CvsScoped => {
                        if let Some(replacement) = entry.on_drop {
                            report.push(replacement);
                        }
                    }
                    PendingPolarity::PreCvs => report.push(entry.primary),
                }
            } else {
                retained.push(entry);
            }
        }
        self.pending_cross_tu = retained;
    }

    /// The temporal unit in which `xlayer`'s current coded video sequence started, or
    /// `None` when no CLK boundary event has been observed for it yet (the implicit
    /// coded video sequence that began at the start of the bitstream, § 7.3.6). Two
    /// events sharing a CVS epoch are in the same coded video sequence; `None` (no CLK)
    /// is distinct from `Some(0)` (a CLK in the first temporal unit), so a re-activation
    /// across a first-temporal-unit CLK is correctly treated as a new coded video
    /// sequence.
    pub(super) fn cvs_epoch(&self, xlayer: ExtendedLayerId) -> Option<u64> {
        self.cvs_started_in_tu.get(&xlayer).copied()
    }

    /// The generation-counter CVS epoch scoping the § 6.10.5 / § 6.4.13 buffer-delay
    /// comparisons for `xlayer`: the [`CvsTracker::cvs_generation`] at which the layer's
    /// current coded video sequence started, or — for the multistream-wide
    /// `GLOBAL_XLAYER_ID` scope — the running generation counter so any CLK in any layer
    /// changes the epoch. Returns 0 before any CLK has been observed. Distinct from
    /// [`CvsTracker::cvs_epoch`], which returns the temporal-unit index used by the
    /// § 7.3.6 single-active-sequence-header check.
    pub(super) fn cvs_generation_epoch(&self, xlayer: ExtendedLayerId) -> u64 {
        if xlayer.is_global() {
            self.cvs_generation
        } else {
            self.cvs_generation_for.get(&xlayer).copied().unwrap_or(0)
        }
    }

    /// Whether a coded video sequence has started for `xlayer` — i.e. its
    /// [`CvsTracker::cvs_generation_epoch`] is non-zero (§ 7.3.6: a CVS "is defined to
    /// start at each temporal unit that contains an OBU with obu_type equal to
    /// OBU_CLOSED_LOOP_KEY"). For the multistream-wide `GLOBAL_XLAYER_ID` scope this is
    /// true once any extended layer has started a coded video sequence. Before the first
    /// CLK the layer's OBUs lie in no coded video sequence at all, so the intra-CVS
    /// error tier (whose constraint binds only "within one coded video sequence") must
    /// not compare them.
    pub(super) fn cvs_started(&self, xlayer: ExtendedLayerId) -> bool {
        self.cvs_generation_epoch(xlayer) > 0
    }

    /// Routes a CVS-scoped comparison diagnostic. `record_tu` is the temporal unit
    /// of the baseline record being compared against: a same-temporal-unit baseline
    /// is always in the same coded video sequence (§ 7.3.6: a coded video sequence
    /// starts at a temporal unit, never inside one), so the diagnostic is emitted
    /// eagerly; a baseline from an earlier temporal unit is deferred, because a CLK
    /// later in the current temporal unit would put the baseline and the new
    /// observation in different coded video sequences.
    pub(super) fn defer_or_emit(
        &mut self,
        xlayer: ExtendedLayerId,
        record_tu: u64,
        diagnostic: Diagnostic,
        report: &mut ValidationReport,
    ) {
        self.defer_or_emit_with_replacement(xlayer, record_tu, diagnostic, None, report);
    }

    /// Like [`CvsTracker::defer_or_emit`], but a deferred primary that is later dropped
    /// because a coded-video-sequence boundary was crossed is replaced by `on_drop`
    /// (when `Some`). When the primary is emitted eagerly (same temporal unit) or
    /// flushed unchanged at the completed temporal unit, `on_drop` is discarded — the
    /// comparison stayed within one coded video sequence. The mechanism stays
    /// rule-id-agnostic; the caller decides what the cross-boundary replacement says.
    pub(super) fn defer_or_emit_with_replacement(
        &mut self,
        xlayer: ExtendedLayerId,
        record_tu: u64,
        diagnostic: Diagnostic,
        on_drop: Option<Diagnostic>,
        report: &mut ValidationReport,
    ) {
        if record_tu == self.tu_index {
            report.push(diagnostic);
        } else {
            self.pending_cross_tu.push(PendingCrossTu {
                xlayer,
                polarity: PendingPolarity::CvsScoped,
                primary: diagnostic,
                on_drop,
            });
        }
    }

    /// Records a [`PendingPolarity::PreCvs`] comparison: both observations are in the
    /// current temporal unit, but no coded video sequence has started for `xlayer` yet
    /// (the pre-first-CLK silence path). Per § 7.3.6 a CLK later in this temporal unit
    /// pulls both observations into one coded video sequence, so the captured `diagnostic`
    /// is emitted on the next [`CvsTracker::start_cvs`] for `xlayer`; if the temporal unit
    /// closes first with no CLK for the layer, [`CvsTracker::flush_completed_tu`] drops it
    /// silently (the observations are in no coded video sequence). The caller must have
    /// already established `cvs_started(xlayer) == false`; `xlayer` must be a concrete
    /// extended layer (global keys keep the documented cross-CMVS under-report and are not
    /// deferred here).
    pub(super) fn defer_pre_cvs(&mut self, xlayer: ExtendedLayerId, diagnostic: Diagnostic) {
        debug_assert!(
            !xlayer.is_global(),
            "PreCvs deferral is for concrete extended layers only",
        );
        if xlayer.is_global() {
            return;
        }
        self.pending_cross_tu.push(PendingCrossTu {
            xlayer,
            polarity: PendingPolarity::PreCvs,
            primary: diagnostic,
            on_drop: None,
        });
    }

    /// Flushes the deferred diagnostics of the just-completed temporal unit, resolving
    /// each entry by its [`PendingPolarity`]:
    ///
    /// - [`PendingPolarity::CvsScoped`]: the primary is dropped when its extended layer
    ///   started a new coded video sequence in this temporal unit (the compared records
    ///   then sit in different coded video sequences, § 7.3.6) and any `on_drop`
    ///   replacement is emitted in its place; otherwise the primary is emitted.
    /// - [`PendingPolarity::PreCvs`]: a CVS start would already have emitted and removed
    ///   the entry in [`CvsTracker::start_cvs`], so any `PreCvs` entry surviving to this
    ///   flush is one whose temporal unit closed with no CLK for the layer — its two
    ///   observations remain in no coded video sequence (the § 6.10.5 random-access-point
    ///   precondition is unsatisfied), so it is dropped silently (pre-first-CLK silence).
    ///
    /// An entry tagged with `GLOBAL_XLAYER_ID` scopes records with no single owning
    /// extended layer and treats "started a coded video sequence in this temporal unit"
    /// as ANY extended layer having done so (documented approximation, sound: it only
    /// drops comparisons).
    pub(super) fn flush_completed_tu(&mut self, report: &mut ValidationReport) {
        let tu_index = self.tu_index;
        let any_started_this_tu = self.cvs_started_in_tu.values().any(|&tu| tu == tu_index);
        for entry in std::mem::take(&mut self.pending_cross_tu) {
            match entry.polarity {
                PendingPolarity::CvsScoped => {
                    let started_this_tu = if entry.xlayer.is_global() {
                        any_started_this_tu
                    } else {
                        self.cvs_started_in_tu.get(&entry.xlayer) == Some(&tu_index)
                    };
                    if started_this_tu {
                        if let Some(replacement) = entry.on_drop {
                            report.push(replacement);
                        }
                    } else {
                        report.push(entry.primary);
                    }
                }
                PendingPolarity::PreCvs => {}
            }
        }
    }

    /// Completes the current temporal unit at a global `OBU_TEMPORAL_DELIMITER`
    /// (AV2 § 7.3.7): flushes the deferred diagnostics, then advances `tu_index`.
    pub(super) fn advance_temporal_unit(&mut self, report: &mut ValidationReport) {
        self.flush_completed_tu(report);
        self.tu_index += 1;
    }

    /// Drops pending deferred diagnostics carrying one of exactly `rule_ids`
    /// that are tagged with `xlayer` — or with `GLOBAL_XLAYER_ID`: a
    /// global-bucket comparison has no single owning extended layer, so dropping
    /// it at any layer's epoch event is the same documented sound approximation
    /// as [`CvsTracker::flush_completed_tu`] (it only drops comparisons). Used
    /// for the § 6.16.10 Table 6.18 pairing rules, whose deferred diagnostics a
    /// § 7.3.8.11 random access point invalidates without ending the coded video
    /// sequence (AV2 § 7.4.4); every other pending diagnostic is CVS-scoped and
    /// must survive.
    pub(super) fn drop_pending_for_rules(&mut self, xlayer: ExtendedLayerId, rule_ids: &[&str]) {
        self.pending_cross_tu.retain(|entry| {
            !((entry.xlayer == xlayer || entry.xlayer.is_global())
                && rule_ids.contains(&entry.primary.rule_id.as_str()))
        });
    }
}

impl ValidatorContext {
    /// Tracks the boundary events that scope the coded-video-sequence comparison
    /// state (sequence-header fingerprints, AV2 § 7.3.6, and content-interpretation
    /// records, § 6.4.12 / § 6.14).
    ///
    /// A global `OBU_TEMPORAL_DELIMITER` completes the current temporal unit: the
    /// deferred cross-temporal-unit diagnostics are flushed (see [`CvsTracker`]) and
    /// the per-temporal-unit frame set resets. An `OBU_CLOSED_LOOP_KEY` starts a new
    /// coded video sequence for its extended layer at the *current* temporal unit
    /// (AV2 § 7.3.6: "A new coded video sequence for an extended layer is defined to
    /// start at each temporal unit that contains an OBU with obu_type equal to
    /// OBU_CLOSED_LOOP_KEY in the coded extended layer unit corresponding to the
    /// extended layer"); see `start_cvs_for_xlayer`. This models the exact § 7.3.6
    /// boundary for sequential decoding from the raw OBU headers alone — no
    /// random-access state is needed. The § 7.4.4 treat-as-new-CVS behavior when a
    /// decoder *initiates* decoding at an OLK applies only to random-access
    /// decoding, not to the sequential decoding a bitstream validator models
    /// ("During sequential decoding, the process does not start a new coded video
    /// sequence for the extended layer", § 7.4.4), so OLK / RAS OBUs are
    /// deliberately not CVS boundary events here. An OLK (like a CLK) is,
    /// however, a § 7.3.8.11 random access point that unconditionally
    /// re-initializes its extended layer's content interpretation parameters, so
    /// both record the CI-parameter epoch (see `observe_ci_rap`). Frame-header
    /// activation drives the *active* sequence header (see
    /// `observe_frame_bearing_obu`); these events drive the fingerprint /
    /// content-interpretation scope.
    ///
    /// NB: the §6.12/§6.13 quantizer-matrix / film-grain duplicate windows are
    /// deliberately NOT reset here. Those windows close at a *coded frame*, not at a
    /// temporal-unit or CVS boundary, so a QM level / film-grain slot reused across
    /// a temporal delimiter with no intervening frame is still a duplicate (see
    /// reset_coded_frame_window, called from the frame-bearing branch).
    pub(super) fn observe_cvs_boundary_events(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if obu.header.obu_type == ObuType::TemporalDelimiter
            && obu.header.extended_layer_id.is_global()
        {
            let completed_tu_index = self.cvs.tu_index;
            self.cvs.advance_temporal_unit(report);
            self.drop_pending_timecode_inference();
            let cmvs_window_before_completion = self.cmvs.current_cmvs_start_tu_index();
            self.cmvs.complete_temporal_unit(completed_tu_index, report);
            self.resolve_deferred_doh_constraint(options, report);
            self.resolve_deferred_lcr_msdo_agreement(options, report);
            self.resolve_deferred_cmvs_boundary(options, report);
            self.commit_annex_a_iop_pending(completed_tu_index, options, report);
            self.msdo_identity.complete_temporal_unit(report);
            self.complete_rap_replay_tu(completed_tu_index, options, report);
            self.frames_seen_in_tu.clear();
            self.frame_unit.reset_temporal_unit(report);
            self.frame_header_copy_record.clear();
            self.celu.set_doh_flag_active(
                self.doh_constraint_flag_active_for_completed_tu(cmvs_window_before_completion),
            );
            self.celu.reset_temporal_unit(report);
            self.resolve_ci_first_celu_for_tu(completed_tu_index, options, report);
            self.distinct_mlayer.advance_temporal_unit();
        } else if obu.header.obu_type == ObuType::ClosedLoopKey {
            self.start_cvs_for_xlayer(obu.header.extended_layer_id, report);
            self.observe_ci_rap(obu.header.extended_layer_id);
            self.repair_post_rap_ci_pairings(obu.header.extended_layer_id, report);
            self.cmvs.note_clk();
            self.annex_a_iop.note_clk();
            self.msdo_identity.note_random_access_point();
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        } else if obu.header.obu_type == ObuType::OpenLoopKey {
            self.observe_ci_rap(obu.header.extended_layer_id);
            self.repair_post_rap_ci_pairings(obu.header.extended_layer_id, report);
            self.msdo_identity.note_random_access_point();
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        } else if obu.header.obu_type == ObuType::RasFrame {
            self.msdo_identity.note_random_access_point();
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        }
    }

    /// Starts a new coded video sequence for `xlayer` at the current temporal unit
    /// (AV2 § 7.3.6): drops the extended layer's CVS-scoped records (sequence
    /// fingerprints, content interpretations, HDR baselines, scan-type observations)
    /// from earlier temporal units — same-temporal-unit
    /// records, e.g. the sequence header preceding the activating CLK, joined the
    /// new coded video sequence and stay — and its pending deferred diagnostics.
    /// Records keyed under `GLOBAL_XLAYER_ID` belong to no single extended layer,
    /// so they are pruned at every boundary event as a documented approximation
    /// (§ 6.16.10-style "current CVS" scoping for layer-nonspecific global records
    /// has no single owner; their deferred diagnostics are filtered at the
    /// temporal-unit flush instead, see [`CvsTracker::flush_completed_tu`]).
    /// Idempotent within a temporal unit.
    pub(super) fn start_cvs_for_xlayer(
        &mut self,
        xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        self.cvs.start_cvs(xlayer, report);
        let tu_index = self.cvs.tu_index;
        let ci_state = self.ci_first_celu.entry(xlayer).or_default();
        if ci_state.first_celu_tu != Some(tu_index) {
            *ci_state = CiFirstCeluState {
                first_celu_tu: Some(tu_index),
                ..CiFirstCeluState::default()
            };
        }
        let migrated_epoch = self.cvs.cvs_generation_epoch(xlayer);
        for (key, baseline) in &mut self.ops_buffer_delay_sums {
            if key.xlayer == xlayer && baseline.tu_index == tu_index {
                baseline.scope.cvs_epoch = migrated_epoch;
            }
        }
        self.flush_scan_type_scope(xlayer, tu_index, report);
        if !xlayer.is_global() {
            self.flush_scan_type_scope(GLOBAL_XLAYER_ID, tu_index, report);
        }
        self.prune_timecode_scope(xlayer, tu_index);
        self.emit_pending_timecode_inference(xlayer, report);
        self.sequence_fingerprints
            .retain(|(x, _), &mut (_, record_tu)| {
                !(*x == xlayer || x.is_global()) || record_tu >= tu_index
            });
        self.content_interpretations.retain(|(x, _), record| {
            !(*x == xlayer || x.is_global()) || record.tu_index >= tu_index
        });
        self.hdr_baselines.retain(|record| {
            !record.association.touches_xlayer(xlayer) || record.tu_index >= tu_index
        });
        self.embedded_layer_first_picture_seen
            .retain(|(record_xlayer, _), _| *record_xlayer != xlayer);
        self.distinct_mlayer.reset_cvs(xlayer, tu_index);
    }
}
