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
        // Bump the CVS generation only once per (xlayer, temporal unit): a redundant
        // CLK in the same temporal unit is the same § 7.3.6 boundary event, so it must
        // not advance the epoch (matches the idempotent `cvs_started_in_tu` insert).
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
    pub(super) fn defer_pre_cvs(
        &mut self,
        xlayer: ExtendedLayerId,
        diagnostic: Diagnostic,
        report: &mut ValidationReport,
    ) {
        debug_assert!(
            !xlayer.is_global(),
            "PreCvs deferral is for concrete extended layers only",
        );
        // Guard against a logic error rather than emit a stray diagnostic in release: a
        // global key must never reach the per-layer pending machinery. Dropping it here
        // matches the documented global under-report and cannot fire on the only caller
        // (which screens out global keys before calling).
        if xlayer.is_global() {
            let _ = report;
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
                // A surviving PreCvs entry means no CLK arrived for the layer this
                // temporal unit: the observations are in no coded video sequence, so the
                // comparison is dropped silently (it carries no `on_drop` replacement).
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
        // The match invalidates the comparison outright — no `on_drop` replacement is
        // emitted, since the random access point did not cross a coded video sequence
        // boundary (the pairing diagnostics never carry one anyway).
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
            // The just-completed temporal unit's index, captured before
            // `advance_temporal_unit` bumps `tu_index` to the next temporal unit. The Annex
            // A Table A.4 IOP commit (below) needs it to decide whether this temporal unit
            // begins a new coded video sequence relative to the open window.
            let completed_tu_index = self.cvs.tu_index;
            self.cvs.advance_temporal_unit(report);
            // AV2 § 6.16.7 / § 7.3.6: any deferred inference-presence diagnostic that
            // survived this temporal unit saw no CLK detach its earlier-temporal-unit
            // seed, so the seed stayed intra-CVS and the field inferred cleanly — drop
            // it silently (see TimecodeCvsState::pending_inference).
            self.drop_pending_timecode_inference();
            // AV2 § 7.3.2: the global temporal delimiter ends the just-observed
            // temporal unit, so the accumulated § 7.3.2 begin/end facts are evaluated
            // now, before the per-temporal-unit facts reset for the next unit. Any
            // deferred provisional-Inside § 6.4.1 monotonic disagreements are resolved
            // here against the completed temporal unit's final CMVS membership.
            // `completed_tu_index` (captured before advance_temporal_unit) is the
            // just-completed temporal unit's index, which stamps the CMVS-window start
            // (§ 7.3.2 scoping).
            //
            // The CMVS-window start of the *just-completed* temporal unit, captured BEFORE
            // `complete_temporal_unit` applies this unit's § 7.3.2 begin/end conditions and
            // mutates the live window (round-5 F1). The § 7.3.7 DOH flag for the completed unit
            // must be sampled against the CMVS that CONTAINS it: when this unit ENDS the CMVS
            // (end condition 2 — a CLK with no MSDO, no activated global LCR), it is the LAST
            // temporal unit of the ENDING CMVS, so its governing window is this pre-completion
            // start — not the live window the tracker is about to clear (see
            // [`Self::doh_constraint_flag_active_in_window`]).
            let cmvs_window_before_completion = self.cmvs.current_cmvs_start_tu_index();
            self.cmvs.complete_temporal_unit(completed_tu_index, report);
            // AV2 § 6.6: now that the just-completed temporal unit's CMVS membership is
            // resolved, evaluate the deferred `msdo/doh-constraint-required` check for its
            // frame-confirmed activations (see resolve_deferred_doh_constraint). Must run
            // after cmvs.complete_temporal_unit so the membership is final.
            self.resolve_deferred_doh_constraint(options, report);
            // AV2 § 6.8.2: with the just-completed temporal unit's CMVS membership
            // resolved, evaluate the deferred MSDO↔global-LCR agreement and the LCR
            // DOH-constraint requirement for its frame-confirmed activations (see
            // resolve_deferred_lcr_msdo_agreement).
            self.resolve_deferred_lcr_msdo_agreement(options, report);
            // AV2 § 7.3.2: evaluate the boundary-set-identity check for the just-completed
            // temporal unit (a CLK-without-MSDO with an activated global LCR diverges the
            // MSDO-alone and MSDO+LCR boundary sets; see resolve_deferred_cmvs_boundary).
            self.resolve_deferred_cmvs_boundary(options, report);
            // Annex A Table A.4: commit the just-completed temporal unit's IOP pending facts
            // to the right coded-video-sequence window — flushing and evaluating the prior
            // window first when this temporal unit begins a new coded video sequence (a CLK
            // in a temporal unit later than the open window's start, § 7.3.6).
            self.commit_annex_a_iop_pending(completed_tu_index, options, report);
            // AV2 § 7.3.8.2: the just-completed temporal unit's buffered OBU_MSDO(s) are
            // resolved against the previous OBU_MSDO now that the temporal unit's
            // § 7.4.1 random-access-point-ness is fully known.
            self.msdo_identity.complete_temporal_unit(report);
            // AV2 § 7.3.8.1: with the just-completed temporal unit's § 7.4.1 random-
            // access-point-ness and leading-frame-ness now known, resolve the buffered
            // HLS-availability replay references for it (suppressed under any external-HLS
            // Provided mode per the partial-declaration policy).
            self.complete_rap_replay_tu(completed_tu_index, options, report);
            self.frames_seen_in_tu.clear();
            // AV2 § 7.3.3 / § 7.3.4 / § 7.3.7: a coded frame unit does not span
            // temporal units, so the segmenter resolves its just-completed temporal
            // unit's still-open units' deferred (output-class-dependent) checks and
            // clears its per-temporal-unit state. The § 7.3.8.10 first-coded-frame-
            // unit CI counters likewise reset per temporal unit.
            self.frame_unit.reset_temporal_unit(report);
            // AV2 § 5.18.1 / § 7.3.7: a coded frame does not span temporal units, so the
            // per-coded-frame recorded first-header bits (for the § 6.17.1
            // frame_header_copy() bit-identity check) cannot pair across this boundary.
            // Clear them with the segmenter's per-temporal-unit state.
            self.frame_header_copy_record.clear();
            // AV2 § 7.3.6 / § 7.3.7 / § 7.4.6: resolve the just-completed temporal unit's
            // coded-extended-layer-unit constraints (output-frame presence, OrderHint
            // agreement, CLK/OLK first-unit and lowest-layer rules, all-leading-or-none) and
            // the flag-gated DOH OrderHint / OrderHintBits checks, then clear the per-TU
            // CELU state. The DOH flag must be recorded from the *just-completed* temporal
            // unit's activated global LCR / preceding MSDO before resolution. Runs after the
            // CMVS / activation resolution above so the activation chain is final. The LCR side
            // is sampled against the GOVERNING window of the completed unit (captured before
            // `complete_temporal_unit` cleared the live window, round-5 F1), so a CLK boundary
            // unit that ends the CMVS is still governed by the activated global LCR of the CMVS
            // that contained it.
            self.celu.set_doh_flag_active(
                self.doh_constraint_flag_active_for_completed_tu(cmvs_window_before_completion),
            );
            self.celu.reset_temporal_unit(report);
            // AV2 § 7.3.6 (round-6 F3): resolve the just-completed temporal unit's CIs against
            // the first-coded-extended-layer-unit-of-the-sequence presence rule (mirror lines
            // 560-562). Runs after the CLK boundary events of the temporal unit have been
            // applied (they are processed at the CLK OBU, earlier in the same temporal unit, so
            // `start_cvs_for_xlayer` already re-seeded the first-CELU state), so each CI's CVS
            // membership is final.
            self.resolve_ci_first_celu_for_tu(completed_tu_index, options, report);
            // AV2 § 7.3.7: clear the per-temporal-unit distinct-`obu_mlayer_id` sets so a
            // CLK in the next temporal unit re-attributes only that temporal unit's ids
            // to the new coded video sequence (see DistinctMlayerTracker::reset_cvs).
            self.distinct_mlayer.advance_temporal_unit();
        } else if obu.header.obu_type == ObuType::ClosedLoopKey {
            // AV2 § 7.3.6: this temporal unit begins a new coded video sequence for the
            // CLK's extended layer.
            self.start_cvs_for_xlayer(obu.header.extended_layer_id, report);
            self.observe_ci_rap(obu.header.extended_layer_id);
            // AV2 § 6.16.7 / § 6.16.10 / § 7.3.8.11 (finding 1, CLK re-pair). The
            // epoch-aware CI dedup deduplicates a CI re-sent in this RAP temporal unit
            // BEFORE the CLK (the lagging epoch could not tell it apart from an ordinary
            // identical repeat at CI-time, so the recheck was skipped). observe_ci_rap
            // has now advanced the epoch and dropped the stale pre-RAP timecode /
            // scan-type pairings; the CI re-sent in this temporal unit is the
            // § 7.3.8.11 authority for the new coded video sequence's pictures, so
            // re-pair the new epoch's observations against it now — once, with no
            // duplicate (the pre-RAP pairing was dropped, not reported).
            self.repair_post_rap_ci_pairings(obu.header.extended_layer_id, report);
            // AV2 § 7.3.2 / § 7.3.6: a CLK makes this temporal unit one that "contains
            // an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY for at least one
            // extended layer" (and begins a new coded video sequence for that layer).
            self.cmvs.note_clk();
            // AV2 § 7.3.6: a CLK also begins a new coded video sequence for the Annex A
            // Table A.4 IOP window. Recording it on the per-temporal-unit pending facts
            // means a same-temporal-unit pre-CLK OBU_MSDO/LCR is attributed to the NEW
            // coded video sequence when the temporal unit commits (lesson 8).
            self.annex_a_iop.note_clk();
            // AV2 § 7.4.1: a CLK makes the temporal unit a random access point.
            self.msdo_identity.note_random_access_point();
            // AV2 § 7.3.8.1 / § 7.4.6: the same random access point drives the HLS
            // availability replay (see RapReplayTracker), scoped to the CLK's own extended
            // layer — random access initiates per extended layer.
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        } else if obu.header.obu_type == ObuType::OpenLoopKey {
            // An OLK is NOT a § 7.3.6 CVS boundary during sequential decoding
            // (§ 7.4.4), but it IS a § 7.3.8.11 random access point that
            // re-initializes the extended layer's content interpretation
            // parameters to defaults.
            self.observe_ci_rap(obu.header.extended_layer_id);
            // AV2 § 6.16.7 / § 6.16.10 / § 7.3.8.11 (finding 1, OLK re-pair). Like
            // the CLK branch above, an OLK is a § 7.3.8.11 random access point, so a
            // CI re-sent identically in this RAP temporal unit BEFORE the OLK was
            // deduplicated by the epoch-aware CI guard (the lagging epoch could not
            // tell it apart from an ordinary identical repeat at CI-time). observe_ci_rap
            // has now advanced the epoch and dropped the stale pre-RAP timecode /
            // scan-type pairings; the CI re-sent in this temporal unit is the
            // § 7.3.8.11 authority for the new epoch's pictures, so re-pair the new
            // epoch's observations against it now — once, with no duplicate.
            self.repair_post_rap_ci_pairings(obu.header.extended_layer_id, report);
            // AV2 § 7.4.1: an OLK makes the temporal unit a random access point.
            self.msdo_identity.note_random_access_point();
            // AV2 § 7.3.8.1 / § 7.4.6: the same random access point drives the HLS
            // availability replay (see RapReplayTracker), scoped to the OLK's own extended
            // layer — random access initiates per extended layer.
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        } else if obu.header.obu_type == ObuType::RasFrame {
            // AV2 § 7.4.1: a RAS frame (OBU_RAS_FRAME) makes the temporal unit a random
            // access point. It is not a § 7.3.6 sequential-decoding CVS boundary, so it
            // touches only the § 7.3.8.2 identity tracker and § 7.3.8.1 replay tracker
            // here. The replay anchor is scoped to the RAS frame's own extended layer
            // (§ 7.4.6: random access initiates per extended layer).
            self.msdo_identity.note_random_access_point();
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        }
    }

    /// Starts a new coded video sequence for `xlayer` at the current temporal unit
    /// (AV2 § 7.3.6): drops the extended layer's CVS-scoped records (sequence
    /// fingerprints, content interpretations, active metadata, HDR baselines,
    /// scan-type observations) from earlier temporal units — same-temporal-unit
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
        // AV2 § 7.3.6 (mirror lines 560-562, round-6 F3): this CLK starts a new coded video
        // sequence for `xlayer` at this temporal unit, whose CELU is the "first coded extended
        // layer unit of the sequence". Reset the first-CELU CI presence state so the new
        // sequence judges its own first CELU. Idempotent within a temporal unit (a redundant
        // CLK in the same temporal unit is the same boundary event, so it must not drop CI
        // presence already recorded for this first CELU): only re-seed when the recorded first
        // CELU temporal unit differs from this one.
        let ci_state = self.ci_first_celu.entry(xlayer).or_default();
        if ci_state.first_celu_tu != Some(tu_index) {
            *ci_state = CiFirstCeluState {
                first_celu_tu: Some(tu_index),
                ..CiFirstCeluState::default()
            };
        }
        // AV2 § 7.3.6: "A new coded video sequence for an extended layer is defined to
        // start at each temporal unit that contains an OBU with obu_type equal to
        // OBU_CLOSED_LOOP_KEY ..." (mirror `07-decoding-process.md` lines 604–606). The
        // whole temporal unit containing this CLK lies in the NEW coded video sequence,
        // so an OPS buffer-delay baseline observed EARLIER in this same temporal unit
        // (before the CLK) belongs to the new coded video sequence, not the old one: its
        // stored CVS epoch is migrated to the layer's new epoch. A later OPS in this same
        // temporal unit then shares the migrated baseline's epoch and the § 6.10.5 error
        // tier compares them within one coded video sequence (the complementary case to
        // the deferred-error `on_drop` path: there the comparison's deferred error is
        // dropped/replaced; here the baseline was stored with no comparison pending).
        // Baselines from EARLIER temporal units genuinely belong to the old coded video
        // sequence and are left untouched. Only baselines keyed under this exact extended
        // layer are migrated; global-keyed (`GLOBAL_XLAYER_ID`) baselines keep the
        // documented `cvs_generation` approximation (re-stamping them could promote an
        // intentionally under-reported cross-CMVS advisory to an error). The migration
        // never compares; it only re-scopes, so it cannot itself emit a diagnostic.
        let migrated_epoch = self.cvs.cvs_generation_epoch(xlayer);
        for (key, baseline) in self.ops_buffer_delay_sums.iter_mut() {
            if key.xlayer == xlayer && baseline.tu_index == tu_index {
                baseline.scope.cvs_epoch = migrated_epoch;
            }
        }
        // The scan-type scopes flush before the content-interpretation pruning
        // below: the § 6.16.10 unestablished-CI warning for the ENDING coded video
        // sequence is evaluated against the records still present (mostly the
        // ending sequence's; a same-temporal-unit record that already joined the
        // new sequence may suppress the warning — an acceptable lenient
        // approximation for a warning-severity derived diagnostic).
        self.flush_scan_type_scope(xlayer, tu_index, report);
        if !xlayer.is_global() {
            self.flush_scan_type_scope(GLOBAL_XLAYER_ID, tu_index, report);
        }
        // § 6.16.7 timecode state: a CLK starts a new coded video sequence for THIS
        // extended layer at this temporal unit (§ 7.3.6). The prune is target-aware
        // (finding 2): it drops only the earlier-temporal-unit n_frames observations
        // and inference chains whose coded video sequence actually restarted — a CLK
        // for one extended layer leaves a global-bucket observation aimed at another
        // extended layer untouched. Same-temporal-unit observations joined the new
        // sequence and stay. (No flush: the timecode checks are eager, never deferred
        // to a flush.) Run BEFORE the content-interpretation migration below so the
        // re-pair afterwards sees the post-RAP observations only.
        self.prune_timecode_scope(xlayer, tu_index);
        // A deferred inference-presence diagnostic whose seed came from an earlier
        // temporal unit now fires: this CLK put the omitting timecode in a new coded
        // video sequence, detaching the seed (§ 7.3.6 / finding 2/3).
        self.emit_pending_timecode_inference(xlayer, report);
        self.sequence_fingerprints
            .retain(|(x, _), &mut (_, record_tu)| {
                !(*x == xlayer || x.is_global()) || record_tu >= tu_index
            });
        self.content_interpretations.retain(|(x, _), record| {
            !(*x == xlayer || x.is_global()) || record.tu_index >= tu_index
        });
        self.metadata.reset_cvs(xlayer, tu_index);
        // An HDR baseline joins the coded video sequence of every extended layer
        // its association touches; the CLK drops earlier-temporal-unit baselines
        // that touch its extended layer (a Universal record touches every layer,
        // mirroring the global-record pruning of the other stores). Pruning a
        // multi-xlayer record at any of its layers' boundaries only drops
        // comparisons, never inventing one.
        self.hdr_baselines.retain(|record| {
            !record.association.touches_xlayer(xlayer) || record.tu_index >= tu_index
        });
        // AV2 § 6.16.5 / § 6.16.6: the "first coded picture of that embedded layer
        // in the coded video sequence" state is per coded video sequence, so a CLK
        // that starts a new coded video sequence for `xlayer` clears the
        // first-picture-seen flags for all of its embedded layers — the next coded
        // picture in the new sequence is again a first coded picture. (A CLK is a
        // coded frame, so its own observe_obu re-sets the flag afterwards.) Records
        // keyed under GLOBAL_XLAYER_ID never enter this set (frame-bearing OBUs are
        // non-global).
        self.embedded_layer_first_picture_seen
            .retain(|(record_xlayer, _), _| *record_xlayer != xlayer);
        // AV2 § 6.4.1 / § 7.3.6: the distinct-`obu_mlayer_id` count is scoped to "the coded
        // video sequence associated with this sequence header", which starts AT this
        // temporal unit (mirror `07-decoding-process.md` lines 604-606). The
        // same-temporal-unit OBUs observed before this CLK — canonically the § 7.3.8.1
        // resent-at-RAP sequence header (forced to obu_mlayer_id 0) — belong to the NEW
        // coded video sequence, so reset_cvs *re-attributes* the boundary temporal unit's
        // seen ids to it (exact re-attribution, not the former whole-state drop). A
        // pending exceedance counted into the ENDING coded video sequence's set whose
        // members spanned an earlier temporal unit is still dropped at the temporal-unit
        // flush via the first_tu deferral (see count_distinct_mlayer /
        // CvsTracker::flush_completed_tu).
        //
        // The § 6.4.1 exceedance comparison on the re-seeded set is NOT run here: this
        // boundary event fires from observe_cvs_boundary_events BEFORE
        // observe_frame_bearing_obu parses the CLK's frame header and activates the
        // header the CLK *references* (mirror `06-syntax-structures-semantics.md` lines
        // 445-447 scope the count to "the coded video sequence associated with this
        // sequence header" — for the NEW coded video sequence that is the CLK-activated
        // header, not the still-active outgoing one). Comparing against the outgoing
        // header here is a wrong-header comparison (PR #41 false positive: outgoing max 1,
        // CLK-activated max 2, re-seeded set count 2). The re-seeded set is therefore
        // compared against the CLK-activated header in observe_frame_bearing_obu's
        // activation path via retroactive_distinct_mlayer_check (anchored at the CLK's
        // extension byte, the same anchor this removed check used). Conservative miss: if
        // the CLK's frame header is unparsable or its referenced header cannot be resolved
        // in-band, no activation happens and the re-seeded-set check is skipped — a sound
        // false negative, since the correct SeqMaxMlayerCnt is then unknown.
        self.distinct_mlayer.reset_cvs(xlayer, tu_index);
    }
}
