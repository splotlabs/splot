// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coded-multistream-video-sequence boundary and deferred CMVS checks.

use super::*;

/// Three-state § 7.3.2 coded-multistream-video-sequence (CMVS) membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CmvsState {
    /// Definitively not inside a CMVS.
    #[default]
    Outside,
    /// Definitively inside a CMVS.
    Inside,
    /// Membership cannot be derived soundly from the modeled state; checks gated on
    /// the CMVS do not fire here (conservative under-approximation).
    Unknown,
}

/// Per-temporal-unit facts the [`CmvsTracker`] accumulates while observing a temporal
/// unit, then evaluates against the § 7.3.2 begin/end conditions when the temporal
/// unit completes.
#[derive(Debug, Default)]
pub(super) struct CmvsTuFacts {
    /// The temporal unit contains an `OBU_CLOSED_LOOP_KEY` for at least one extended
    /// layer (AV2 § 7.3.2 begin: "begins at a temporal unit that contains an OBU with
    /// obu_type equal to OBU_CLOSED_LOOP_KEY for at least one extended layer"; § 7.3.6:
    /// such a temporal unit begins a new coded video sequence for that extended layer).
    pub(super) has_clk: bool,
    /// The MSDO observation for this temporal unit, or `None` when no MSDO is present
    /// (AV2 § 7.3.2 conditions 1 and 2 turn on MSDO presence and key-field change).
    pub(super) msdo: Option<MsdoObservation>,
    /// A global layer configuration record OBU is present in this temporal unit. Used
    /// only to drive the conservative [`CmvsState::Unknown`] routing of the § 7.3.2
    /// condition-3 / end-condition-2 paths whose precise truth needs § 7.3.8
    /// activation state that is not yet modeled.
    pub(super) global_lcr_present: bool,
}

/// Minimal three-state § 7.3.2 CMVS begin/end tracker (AV2 v1.0.0 § 7.3.2).
///
/// The tracker accumulates per-temporal-unit facts ([`CmvsTuFacts`]) as OBUs are
/// observed and applies the § 7.3.2 begin/end conditions when a temporal unit
/// completes (at the global `OBU_TEMPORAL_DELIMITER` that ends it, or at the end of
/// the bitstream). It exposes three states ([`CmvsState`]); checks gated on the CMVS
/// (e.g. § 6.4.1 `monotonic_output_order_flag` agreement) fire only in
/// [`CmvsState::Inside`]. [`Self::state`] returns the membership *effective for OBUs
/// observed so far in the current temporal unit*: end condition 2 (an MSDO-less CLK
/// temporal unit that begins a new coded video sequence) is applied as soon as it is
/// decidable — at the CLK, since § 7.3.7 places the at-most-one MSDO before every coded
/// extended layer unit — so the stale `Inside` of the previous temporal unit does not
/// leak into activation-time checks for OBUs that already sit outside the CMVS.
/// The tracker is a sound under-approximation: every transition whose truth cannot be
/// derived from the modeled state (notably anything depending on exact § 7.3.8
/// global-LCR activation) routes to [`CmvsState::Unknown`], never to a spurious
/// `Inside`/`Outside`.
///
/// Each transition below carries the exact § 7.3.2 sentence it implements, because no
/// real multistream conformance vectors exist yet and the spec text is the only
/// oracle.
#[derive(Debug, Default)]
pub(super) struct CmvsTracker {
    /// Current CMVS membership.
    pub(super) state: CmvsState,
    /// Facts accumulated for the temporal unit currently being observed.
    pub(super) current_tu: CmvsTuFacts,
    /// § 6.4.1 monotonic-output-order disagreements emitted at sequence-header
    /// observation time while this temporal unit's CMVS membership is only
    /// *provisionally* [`CmvsState::Inside`] (committed `Inside`, but no CLK observed
    /// yet, so a later MSDO-less CLK could end the CMVS for this temporal unit —
    /// AV2 § 7.3.2 end condition 2, mirror `07-decoding-process.md` lines 335-341).
    /// They are flushed when the temporal unit completes ([`Self::complete_temporal_unit`]):
    /// emitted when the completed temporal unit is definitively `Inside`, dropped when a
    /// CLK turned it `Outside`/`Unknown`. Deferring avoids a false positive on the
    /// § 7.3.6-permitted same-CVS redefinition that immediately precedes the CLK that
    /// begins the new coded video sequence (mirror `07-decoding-process.md` lines
    /// 608-611).
    pub(super) pending_monotonic: Vec<Diagnostic>,
    /// Facts of the just-completed temporal unit, for the § 7.3.2 boundary-set-identity
    /// check resolved at the same temporal-unit-completion point (see
    /// [`ValidatorContext::resolve_deferred_cmvs_boundary`]). Set by
    /// [`Self::complete_temporal_unit`]; `None` before the first completion.
    pub(super) last_completed: Option<CmvsCompletedFacts>,
    /// The [`CvsTracker::tu_index`] of the temporal unit at which the *current* coded
    /// multistream video sequence began (§ 7.3.2), or `None` when no CMVS is active. A CMVS
    /// spans a contiguous run of temporal units; this records the index of its first one,
    /// captured by [`Self::complete_temporal_unit`] when a begin condition fires, carried
    /// across continuation temporal units, and cleared when the CMVS ends. Everything
    /// observed at `tu_index >= cmvs_start_tu_index` lies within the current CMVS — the
    /// window the § 6.8.2 agreement / DOH requirement and the § 6.6 MSDO DOH requirement
    /// scope their per-layer evaluation to (codex findings 3393129738 / 3393129745). A
    /// temporal unit is the atomic § 7.3.6 attribution unit (a CLK-bearing TU and all its
    /// pre-CLK HLS belong to the same new coded video sequence), so a TU-index lower bound
    /// avoids the pre-CLK / post-CLK generation ambiguity and is a sound
    /// under-approximation of CMVS membership.
    pub(super) cmvs_start_tu_index: Option<u64>,
    /// The [`CvsTracker::tu_index`] of the just-completed temporal unit itself, captured by
    /// [`Self::complete_temporal_unit`]. The § 7.3.2 boundary-set-identity check
    /// (`cmvs/boundary-set-mismatch`) needs the BOUNDARY temporal unit's own index — not the
    /// CMVS-window start — because end condition 2's divergence turns on whether THAT temporal
    /// unit "has an activated global layer configuration record", a property of the boundary
    /// temporal unit alone (codex finding 3393274375). `None` before the first completion.
    pub(super) last_completed_tu_index: Option<u64>,
}

/// The § 7.3.2 facts of a just-completed temporal unit, captured by
/// [`CmvsTracker::complete_temporal_unit`] for the boundary-set-identity check.
#[derive(Debug, Clone, Copy)]
pub(super) struct CmvsCompletedFacts {
    /// The committed CMVS membership *before* this temporal unit's begin/end conditions
    /// were applied — i.e. whether a CMVS was active when the temporal unit started.
    pub(super) was_inside_before: bool,
    /// The temporal unit contained an `OBU_CLOSED_LOOP_KEY` (§ 7.3.2 / § 7.3.6: begins a
    /// new coded video sequence for at least one extended layer).
    pub(super) has_clk: bool,
    /// An `OBU_MSDO` was present in the temporal unit.
    pub(super) msdo_present: bool,
    /// A global layer configuration record OBU was present in the temporal unit.
    pub(super) global_lcr_present: bool,
}

/// The disposition of the § 6.4.1 cross-layer monotonic-output-order agreement check at
/// a sequence-header observation, given the § 7.3.2 CMVS tracker state; see
/// [`CmvsTracker::monotonic_verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MonotonicVerdict {
    /// The tracker is not definitively inside a CMVS for the OBUs observed so far; the
    /// check does not fire.
    Skip,
    /// The tracker is definitively inside a CMVS; a disagreement is emitted eagerly.
    EmitNow,
    /// The tracker is only *provisionally* inside a CMVS (committed `Inside`, no CLK
    /// observed yet in this temporal unit, so a later MSDO-less CLK could end the
    /// CMVS); a disagreement is deferred to the temporal-unit flush.
    Defer,
}

impl CmvsTracker {
    /// Returns the § 7.3.2 CMVS membership *effective for OBUs observed so far in the
    /// current temporal unit*, applying the begin and end conditions as soon as they are
    /// decidable rather than only at temporal-unit completion.
    ///
    /// Consumed by the § 6.4.1 cross-xlayer `monotonic_output_order_flag` agreement
    /// check, which fires only in [`CmvsState::Inside`].
    ///
    /// The committed `self.state` reflects the previous completed temporal unit, so on its
    /// own it is stale for the temporal unit currently being observed in both directions.
    /// § 7.3.7 constrains the at-most-one MSDO of a temporal unit to precede every coded
    /// extended layer unit, so by the time any frame activation in the temporal unit runs,
    /// MSDO presence for the temporal unit is already final; the temporal unit's begin/end
    /// membership is therefore decidable at activation time once a CLK has been observed (a
    /// CLK lives inside a coded extended layer unit, so the MSDO, if any, was already seen).
    /// These two adjustments are mutually exclusive on MSDO presence, so they never both
    /// apply:
    ///
    /// - **Begin (CLK + MSDO present).** This temporal unit "contains an OBU with obu_type
    ///   equal to OBU_CLOSED_LOOP_KEY for at least one extended layer" and "an OBU with
    ///   obu_type equal to OBU_MSDO is present", so it begins a CMVS (§ 7.3.2 begin
    ///   condition 1 from a committed `Outside`, begin condition 2 / continuation from a
    ///   committed `Inside`). The result is [`CmvsState::Inside`] under every committed
    ///   state: from `Outside` (begin condition 1); from `Inside` (a changed MSDO begins a
    ///   new CMVS, an unchanged MSDO continues the active one — Inside either way); and from
    ///   `Unknown`, where both resolutions of the unknown (real `Outside` → begin condition
    ///   1, real `Inside` → continuation/begin condition 2) yield Inside. Without this
    ///   adjustment the committed `Outside`/`Unknown` would leak into activation-time checks
    ///   for OBUs that already sit inside the CMVS opened by this temporal unit.
    /// - **End (committed `Inside` + CLK + no MSDO).** This temporal unit "begins a new
    ///   coded video sequence for at least one extended layer but does not contain an OBU
    ///   with obu_type equal to OBU_MSDO" (§ 7.3.2 end condition 2), so it ends the active
    ///   CMVS — to [`CmvsState::Outside`], or to [`CmvsState::Unknown`] when a global LCR is
    ///   present (whose activation, and thus whether this is really an end, is not modeled).
    ///   Without this adjustment the stale `Inside` would leak into activation-time checks
    ///   for OBUs that already sit outside the CMVS.
    ///
    /// When no CLK has been observed yet in the temporal unit, neither adjustment applies
    /// and the committed state is returned unchanged.
    pub(super) fn state(&self) -> CmvsState {
        if self.current_tu.has_clk {
            // § 7.3.7: an MSDO, if any, precedes the CLK's coded extended layer unit, so
            // MSDO presence for this temporal unit is already final at activation time.
            if self.current_tu.msdo.is_some() {
                // § 7.3.2 begin: a CLK temporal unit with an MSDO present is inside the
                // CMVS it opens (or continues), regardless of the committed state.
                return CmvsState::Inside;
            }
            if matches!(self.state, CmvsState::Inside) {
                // § 7.3.2 end condition 2 is already decidable: an MSDO-less CLK temporal
                // unit begins a new coded video sequence but carries no OBU_MSDO.
                if self.current_tu.global_lcr_present {
                    return CmvsState::Unknown;
                }
                return CmvsState::Outside;
            }
        }
        self.state
    }

    /// The *committed* § 7.3.2 CMVS membership — the membership of the most recently
    /// completed temporal unit, ignoring any partial facts of the temporal unit currently
    /// being observed.
    ///
    /// After [`Self::complete_temporal_unit`] runs at a temporal-unit boundary, this is
    /// the just-completed temporal unit's final membership (the per-temporal-unit facts
    /// have been reset, so [`Self::state`] also returns the committed value — but this
    /// accessor names the intent). The deferred § 6.6 `msdo/doh-constraint-required`
    /// evaluation queries it at boundary resolution to decide whether the just-completed
    /// temporal unit's frame-confirmed activations sit inside a CMVS (see
    /// [`ValidatorContext::resolve_deferred_doh_constraint`]).
    pub(super) fn committed_inside(&self) -> bool {
        matches!(self.state, CmvsState::Inside)
    }

    /// The disposition of the § 6.4.1 cross-layer monotonic-output-order agreement check
    /// at a sequence-header observation, given the OBUs observed so far in the current
    /// temporal unit.
    ///
    /// The check fires only in [`CmvsState::Inside`]. When [`Self::state`] is `Inside`
    /// *and a CLK has already been observed* this temporal unit, the membership is final
    /// (§ 7.3.7 places the at-most-one MSDO before every coded extended layer unit, so a
    /// CLK temporal unit's begin/end membership is decided at the CLK), so a disagreement
    /// is emitted eagerly ([`MonotonicVerdict::EmitNow`]). When `state()` is `Inside` but
    /// *no CLK has been observed yet*, the verdict is only provisional: a later MSDO-less
    /// CLK in this temporal unit would end the CMVS (§ 7.3.2 end condition 2), placing a
    /// header observed before it — canonically a § 7.3.6-permitted same-CVS redefinition
    /// immediately preceding the CLK that begins the new coded video sequence (mirror
    /// `07-decoding-process.md` lines 608-611) — outside the CMVS. A disagreement is then
    /// deferred ([`MonotonicVerdict::Defer`]) and resolved at temporal-unit completion.
    /// Any non-`Inside` state skips the check ([`MonotonicVerdict::Skip`]).
    pub(super) fn monotonic_verdict(&self) -> MonotonicVerdict {
        match self.state() {
            CmvsState::Inside if self.current_tu.has_clk => MonotonicVerdict::EmitNow,
            CmvsState::Inside => MonotonicVerdict::Defer,
            CmvsState::Outside | CmvsState::Unknown => MonotonicVerdict::Skip,
        }
    }

    /// Queues a provisional-`Inside` § 6.4.1 monotonic-output-order disagreement for
    /// resolution at temporal-unit completion (see [`Self::monotonic_verdict`] /
    /// [`Self::complete_temporal_unit`]).
    pub(super) fn queue_provisional_monotonic(&mut self, diagnostic: Diagnostic) {
        self.pending_monotonic.push(diagnostic);
    }

    /// Records that the temporal unit being observed contains an
    /// `OBU_CLOSED_LOOP_KEY` for some extended layer (AV2 § 7.3.2 / § 7.3.6).
    pub(super) fn note_clk(&mut self) {
        self.current_tu.has_clk = true;
    }

    /// Records the MSDO observation for the temporal unit being observed
    /// (AV2 § 7.3.2 conditions 1 and 2). A temporal unit carries at most one MSDO
    /// (§ 7.3.7), so this is set at most once per temporal unit.
    pub(super) fn note_msdo(&mut self, observation: MsdoObservation) {
        self.current_tu.msdo = Some(observation);
    }

    /// Records that a global layer configuration record OBU is present in the temporal
    /// unit being observed (AV2 § 7.3.2 condition 3 / end condition 2). Whether it is
    /// *activated* needs § 7.3.8 activation state that is not modeled; the tracker
    /// therefore treats presence as an "activation cannot be ruled out" signal and
    /// routes the affected transitions to [`CmvsState::Unknown`].
    pub(super) fn note_global_lcr_present(&mut self) {
        self.current_tu.global_lcr_present = true;
    }

    /// The [`CvsTracker::tu_index`] of the temporal unit at which the current coded
    /// multistream video sequence began, or `None` when no CMVS is active. Observations
    /// (frame-confirmed activations, global-LCR OBUs) tagged with a `tu_index` at or after
    /// this value lie within the current CMVS; earlier ones belong to a prior CMVS and are
    /// excluded from the § 6.8.2 / § 6.6 DOH evaluations (codex findings 3393129738 /
    /// 3393129745).
    pub(super) fn current_cmvs_start_tu_index(&self) -> Option<u64> {
        self.cmvs_start_tu_index
    }

    /// The [`CvsTracker::tu_index`] of the just-completed (boundary) temporal unit, for the
    /// § 7.3.2 boundary-set check's "the BOUNDARY temporal unit has an activated global LCR"
    /// scoping (end condition 2 divergence). `None` before the first completion.
    pub(super) fn last_completed_tu_index(&self) -> Option<u64> {
        self.last_completed_tu_index
    }

    /// Completes the temporal unit being observed, applying the § 7.3.2 begin/end
    /// conditions, then resets the per-temporal-unit facts for the next one. Called at
    /// each temporal-unit boundary and at the end of the bitstream. `completed_tu_index` is
    /// the [`CvsTracker::tu_index`] of the just-completed temporal unit (captured before
    /// `advance_temporal_unit` bumps it), used to stamp the CMVS-window start when a begin
    /// condition fires.
    ///
    /// Provisional-`Inside` § 6.4.1 monotonic disagreements deferred during this temporal
    /// unit ([`Self::queue_provisional_monotonic`]) are resolved here against the
    /// temporal unit's final membership: emitted when the completed temporal unit is
    /// definitively [`CmvsState::Inside`], dropped when a CLK ended the CMVS
    /// ([`CmvsState::Outside`]/[`CmvsState::Unknown`], § 7.3.2 end condition 2).
    pub(super) fn complete_temporal_unit(
        &mut self,
        completed_tu_index: u64,
        report: &mut ValidationReport,
    ) {
        let facts = std::mem::take(&mut self.current_tu);
        let was_inside_before = matches!(self.state, CmvsState::Inside);
        self.last_completed = Some(CmvsCompletedFacts {
            was_inside_before,
            has_clk: facts.has_clk,
            msdo_present: facts.msdo.is_some(),
            global_lcr_present: facts.global_lcr_present,
        });
        let (next, window_action) = self.next_state(&facts);
        self.state = next;
        // The boundary temporal unit's own index, for the § 7.3.2 boundary-set check's
        // boundary-TU-scoped activated-global-LCR resolution (codex finding 3393274375).
        self.last_completed_tu_index = Some(completed_tu_index);
        // § 7.3.2 window bookkeeping: the live window for the *next* temporal unit. An Open
        // starts a fresh window at this temporal unit's index; a Keep carries the existing
        // start; a Close clears
        // it. Capturing this lower bound at the authoritative temporal-unit-completion
        // resolution lets the deferred § 6.8.2 / § 6.6 evaluations scope their per-layer
        // loops to observations made at or after this temporal unit (the current CMVS).
        match window_action {
            CmvsWindowAction::Open => self.cmvs_start_tu_index = Some(completed_tu_index),
            // Seed the start on the first-ever continuation if it was somehow never set.
            CmvsWindowAction::Keep => {
                self.cmvs_start_tu_index.get_or_insert(completed_tu_index);
            }
            CmvsWindowAction::Close => self.cmvs_start_tu_index = None,
        }
        let pending = std::mem::take(&mut self.pending_monotonic);
        if matches!(self.state, CmvsState::Inside) {
            for diagnostic in pending {
                report.push(diagnostic);
            }
        }
    }

    /// Whether the just-completed temporal unit is the § 7.3.2 boundary-set-identity
    /// divergence *candidate*: a temporal unit that, while a CMVS was active, begins a new
    /// coded video sequence (has a CLK) with no OBU_MSDO present but a global layer
    /// configuration record present. Under the MSDO-alone boundary rules such a temporal
    /// unit ENDS the CMVS (§ 7.3.2 end condition 2 fires — "does not contain an OBU_MSDO
    /// and does not have an activated global LCR", and there is no MSDO); under the
    /// MSDO+activated-global-LCR rules it does NOT end (the activated global LCR makes end
    /// condition 2 false), so the two boundary sets diverge here. Whether the global LCR is
    /// genuinely *activated* (making the divergence real and decidable) is confirmed by the
    /// caller against the association chain; this only reports the structural candidate.
    pub(super) fn last_completed_is_boundary_divergence_candidate(&self) -> bool {
        self.last_completed.is_some_and(|f| {
            f.was_inside_before && f.has_clk && !f.msdo_present && f.global_lcr_present
        })
    }

    /// Computes the § 7.3.2 CMVS state after a completed temporal unit with `facts`,
    /// given the current `self.state`, plus the [`CmvsWindowAction`] for the CMVS-window
    /// scoping. Begin conditions are evaluated before end conditions because a temporal
    /// unit that begins a new CMVS is the *earliest* end of the current one (§ 7.3.2 end
    /// condition 1). The window action drives the start-of-window bookkeeping in
    /// [`Self::complete_temporal_unit`].
    ///
    /// The window opens (a fresh lower bound) on *any* begin condition — including begin
    /// condition 3 (a CLK temporal unit activating a global LCR with no MSDO), which the
    /// membership state routes to [`CmvsState::Unknown`] because § 7.3.8 activation is not
    /// modeled. Opening the window there is sound: the § 6.8.2 DOH requirement and agreement
    /// resolve "an activated global LCR" from the association chain (`activated_global_lcr`),
    /// which IS decidable, so an LCR-only CMVS still needs a window for those checks to scope
    /// to; if the chain finds no activated global LCR, nothing fires regardless of the window.
    pub(super) fn next_state(&self, facts: &CmvsTuFacts) -> (CmvsState, CmvsWindowAction) {
        // AV2 § 7.3.2: "A coded multistream video sequence begins at a temporal unit
        // that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY for at least
        // one extended layer and satisfies one of the following conditions". Without a
        // CLK in the temporal unit, no begin condition can fire.
        if facts.has_clk {
            let currently_active = matches!(self.state, CmvsState::Inside);
            match facts.msdo {
                // AV2 § 7.3.2 begin condition 1: "No coded multistream video sequence is
                // currently active and an OBU with obu_type equal to OBU_MSDO is present."
                Some(_) if !currently_active => {
                    return (CmvsState::Inside, CmvsWindowAction::Open);
                }
                // AV2 § 7.3.2 begin condition 2: "A coded multistream video sequence is
                // currently active, an OBU with obu_type equal to OBU_MSDO is present,
                // and the value of multistream_profile_idc, multistream_level_idx,
                // multistream_tier, num_streams_minus_2, multistream_even_allocation_flag,
                // or multistream_large_picture_idc differs from the corresponding value
                // in the previous OBU_MSDO." A changed MSDO begins a new CMVS (which is
                // still Inside); an unchanged MSDO leaves the active CMVS intact.
                Some(MsdoObservation::Changed) => {
                    return (CmvsState::Inside, CmvsWindowAction::Open);
                }
                Some(MsdoObservation::First | MsdoObservation::Unchanged) => {
                    // Active CMVS (the `!currently_active` arm above already handled the
                    // inactive case for any MSDO), MSDO present but unchanged: this temporal
                    // unit neither begins a new CMVS (condition 2 needs a change) nor ends
                    // the current one (end condition 2 excludes an MSDO-accompanied CVS
                    // start), so the CMVS continues.
                    return (CmvsState::Inside, CmvsWindowAction::Keep);
                }
                None => {
                    // AV2 § 7.3.2 begin condition 3: "No coded multistream video sequence
                    // is currently active and a global layer configuration record is
                    // activated." Exact § 7.3.8 global-LCR activation is not modeled, so the
                    // membership cannot be soundly classified Inside — route to Unknown — but
                    // the window opens at this temporal unit so the chain-decidable LCR-only
                    // § 6.8.2 DOH/agreement checks can scope to it.
                    if facts.global_lcr_present && !currently_active {
                        return (CmvsState::Unknown, CmvsWindowAction::Open);
                    }
                }
            }
        }

        // AV2 § 7.3.2: "A coded multistream video sequence ends at the earliest of:"
        // (begin conditions above already handled end condition 1, "A temporal unit
        // that begins a new coded multistream video sequence as defined above").
        if matches!(self.state, CmvsState::Inside) {
            // AV2 § 7.3.2 end condition 2: "A temporal unit that begins a new coded
            // video sequence for at least one extended layer but does not contain an OBU
            // with obu_type equal to OBU_MSDO and does not have an activated global layer
            // configuration record." A CLK temporal unit (§ 7.3.6: begins a new coded
            // video sequence for its extended layer) with no MSDO ends the CMVS — unless
            // a global LCR is present, whose activation (and thus whether this is really
            // an end) is not modeled, so route that ambiguous case to Unknown.
            if facts.has_clk && facts.msdo.is_none() {
                if facts.global_lcr_present {
                    return (CmvsState::Unknown, CmvsWindowAction::Close);
                }
                return (CmvsState::Outside, CmvsWindowAction::Close);
            }
            // Otherwise the active CMVS continues across this temporal unit.
            return (CmvsState::Inside, CmvsWindowAction::Keep);
        }

        // No begin condition fired and the state is not Inside (so this temporal unit
        // contains no CLK — a CLK with an Inside committed state is handled by the
        // end-condition block above, and a CLK from Outside/Unknown would have matched a
        // begin arm). § 7.3.2 end conditions 1 and 2 both require a temporal unit that
        // "begins a new coded video sequence" (a CLK, § 7.3.6); with no CLK, NO end
        // condition can fire here, so an active window must be carried, not closed.
        //
        // - `Unknown` with an open window is an LCR-only CMVS (opened via begin condition 3,
        //   which the membership router cannot soundly classify Inside) whose end is still
        //   undecided: a non-CLK temporal unit cannot end it, so Keep preserves its window
        //   for the chain-decidable § 6.8.2 LCR-DOH / agreement checks to scope to later
        //   frame-confirmed activations (codex finding 3393274378). Without this the window
        //   closed prematurely and those later activations were skipped.
        // - `Outside`, or an `Unknown` whose window was already cleared (e.g. a divergence
        //   candidate that ended the CMVS at line ~3024 with Close), has no live window;
        //   Close keeps it cleared and avoids `complete_temporal_unit`'s Keep `get_or_insert`
        //   seeding a bogus window at this non-CLK temporal unit.
        if matches!(self.state, CmvsState::Unknown) && self.cmvs_start_tu_index.is_some() {
            return (self.state, CmvsWindowAction::Keep);
        }
        (self.state, CmvsWindowAction::Close)
    }
}

/// The § 7.3.2 CMVS-window action for a completed temporal unit, computed alongside the
/// membership state by [`CmvsTracker::next_state`] and applied by
/// [`CmvsTracker::complete_temporal_unit`]. The window is the [`CvsTracker::tu_index`]
/// lower bound the § 6.8.2 / § 6.6 deferred checks scope their per-layer loops to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CmvsWindowAction {
    /// A begin condition (1, 2, or 3) fired: open a fresh window at this temporal unit.
    Open,
    /// A continuation: keep the existing window start.
    Keep,
    /// An end condition / undecidable carry: close the window for the next temporal unit.
    Close,
}

impl ValidatorContext {
    /// Whether the § 7.3.7 / § 7.4.6 DOH constraints are active for the *just-completed*
    /// temporal unit at a global-temporal-delimiter boundary or at the end of the bitstream
    /// (round-5 F1 / round-6 F1). Resolves the LCR side against the GOVERNING CMVS window of
    /// the completed unit rather than the live window the [`CmvsTracker`](CmvsTracker) has
    /// just mutated.
    ///
    /// The base disjunction (see [`Self::doh_constraint_flag_active_in_window`]) is:
    /// `multistream_doh_constraint_flag` in the preceding MSDO equals 1, or
    /// `lcr_doh_constraint_flag` in the activated global LCR equals 1. Either source being
    /// absent contributes `false`, so when neither source declares the constraint the
    /// flag-gated checks stay silent.
    ///
    /// Per § 7.3.2 the completed unit is *contained* in whichever CMVS it belongs to:
    ///
    /// - When the unit BEGINS a new CMVS (begin condition 1, 2, or 3), it is the FIRST unit of
    ///   the NEW CMVS, whose window the tracker has just opened at this unit's index — the live
    ///   (post-completion) window `Some(start)`, used here.
    /// - When the unit only CONTINUES the CMVS, the window is unchanged either way.
    /// - When the unit ENDS the CMVS without beginning a new one (end condition 2 — a CLK with
    ///   no MSDO, no activated global LCR — Closes the live window to `None`), it is the LAST
    ///   unit of the ENDING CMVS, whose window was the pre-completion start. The live window is
    ///   `None`, so this falls back to `cmvs_window_before_completion`.
    ///
    /// So the governing window is the post-completion window when it is `Some`, else the
    /// pre-completion window — which is exactly the window of the CMVS that contains the
    /// completed unit. The MSDO side is window-independent: `msdo_substream_max` is last-wins
    /// live state that `complete_temporal_unit` does not clear, so it already reflects the MSDO
    /// that governed the completed unit (it must remain the preceding MSDO regardless of the
    /// LCR window).
    pub(super) fn doh_constraint_flag_active_for_completed_tu(
        &self,
        cmvs_window_before_completion: Option<u64>,
    ) -> bool {
        let governing_window = self
            .cmvs
            .current_cmvs_start_tu_index()
            .or(cmvs_window_before_completion);
        self.doh_constraint_flag_active_in_window(governing_window)
    }

    /// The base § 7.3.7 / § 7.4.6 DOH constraint-active disjunction (mirror lines 650-657 /
    /// 1316-1320), resolving the activated-global-LCR side against an explicit CMVS-window
    /// start (`None` → no window → no activated global LCR, via
    /// [`Self::activated_global_lcr_in_window`]). The MSDO side is window-independent (the
    /// live last-wins preceding MSDO, [`Self::msdo_substream_max`]; § 7.3.8.2 keeps a non-RAP
    /// MSDO identical to its predecessor). The DOH flag is active iff either source declares
    /// the constraint.
    pub(super) fn doh_constraint_flag_active_in_window(
        &self,
        cmvs_window_start: Option<u64>,
    ) -> bool {
        let msdo_flag = self
            .msdo_substream_max
            .as_ref()
            .is_some_and(|m| m.doh_constraint_flag);
        let lcr_flag = cmvs_window_start.is_some_and(|cmvs_start| {
            self.activated_global_lcr_in_window(cmvs_start)
                .is_some_and(|(_, record)| record.doh_constraint_flag)
        });
        msdo_flag || lcr_flag
    }

    /// Resolves the deferred § 6.6 `msdo/doh-constraint-required` evaluation for a
    /// just-completed temporal unit, at temporal-unit-completion time (a temporal
    /// delimiter boundary or the end-of-bitstream flush), *after* the [`CmvsTracker`] has
    /// applied the temporal unit's § 7.3.2 begin/end conditions.
    ///
    /// The requirement is scoped to a coded multistream *video* sequence, so the
    /// evaluation only fires when the completed temporal unit resolved to a definitive
    /// CMVS [`CmvsState::Inside`] ([`CmvsTracker::committed_inside`]). Deferring to this
    /// point — rather than evaluating eagerly at sequence-header activation as the
    /// original landing did — handles both arrival-order corner cases the eager check
    /// missed (codex findings 3392940061 and 3392940072):
    ///
    /// - A same-id header redefinition with `monotonic_output_order_flag == 0` at the top
    ///   of a temporal unit that a later MSDO-less CLK ENDS (§ 7.3.2 end condition 2) sits
    ///   *outside* the CMVS. The eager check, gated on the still-`Inside` committed state
    ///   at activation, fired a false positive; this deferred path sees the resolved
    ///   `Outside`/`Unknown` and does not.
    /// - A same-id CLK that re-references an already-frame-confirmed active header opens a
    ///   CMVS at the CLK without re-entering `on_sequence_activation` (the seq id is
    ///   unchanged and the layer was already confirmed), so the eager activation-time path
    ///   never ran; this deferred path re-examines the in-CMVS frame-confirmed activations
    ///   against the resolved membership, so the transition to `Inside` is caught.
    ///
    /// It re-runs [`Self::check_doh_constraint_required`] for the frame-confirmed extended
    /// layers activated *within the current CMVS window*
    /// ([`Self::frame_confirmed_xlayers_in_current_cmvs`]), NOT the whole-history
    /// `frame_confirmed_xlayers` accumulator — so a non-monotonic header left active from an
    /// earlier, already-ended coded video sequence outside this CMVS does not trip the § 6.6
    /// MSDO DOH requirement against this CMVS's MSDO (codex finding 3393129745, the same
    /// whole-history scope bug the § 6.8.2 LCR DOH check had). The
    /// `(xlayer, seq_header_id, cvs_epoch)` dedup inside that method keeps a resolved
    /// evaluation from re-spamming a diagnostic across successive temporal units of the same
    /// CMVS.
    pub(super) fn resolve_deferred_doh_constraint(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !self.cmvs.committed_inside() {
            return;
        }
        let xlayers = self.frame_confirmed_xlayers_in_current_cmvs();
        for xlayer in xlayers {
            self.check_doh_constraint_required(xlayer, options, report);
        }
    }

    /// Resolves the deferred § 6.8.2 MSDO↔global-LCR agreement (mirror
    /// `06-syntax-structures-semantics.md` lines 1646-1678) and the § 6.8.2 LCR
    /// DOH-constraint requirement (lines 1619-1621), at temporal-unit-completion time
    /// (a temporal-delimiter boundary or the end-of-bitstream flush), *after* the
    /// [`CmvsTracker`] has applied the temporal unit's § 7.3.2 begin/end conditions.
    ///
    /// The two checks have *different* presence preconditions (codex finding 3393129743):
    ///
    /// - The § 6.8.2 **agreement** constraints hold "when both an OBU with obu_type equal to
    ///   OBU_MSDO and an activated global layer configuration record OBU are present in the
    ///   same coded multistream video sequence" (mirror lines 1646-1648), so they fire only
    ///   when both an MSDO and an activated global LCR are present in the *current* CMVS.
    /// - The § 6.8.2 **LCR DOH** requirement (lines 1619-1621) is LCR-only — "when
    ///   monotonic_output_order_flag is equal to 0 in any activated sequence header of the
    ///   coded multistream video sequence, lcr_doh_constraint_flag shall be equal to 1". It
    ///   requires only an activated global LCR, *not* an MSDO: a global-LCR-only CMVS (legal
    ///   per the Annex A IOP2 Table A.4 rows, opened via § 7.3.2 begin condition 3) must
    ///   still satisfy it.
    ///
    /// Both gate on the association chain resolving an *activated* global LCR present in the
    /// current CMVS ([`Self::activated_global_lcr`], window-scoped) — an
    /// observed-but-never-activated global LCR, or one present only in an earlier CMVS,
    /// resolves nothing and triggers no diagnostic. The agreement additionally requires a
    /// recorded MSDO whose observation temporal unit lies within the current CMVS window
    /// (so a stale earlier-CMVS MSDO is not compared against this CMVS's global LCR). Gating
    /// on the chain-decidable `activated_global_lcr` rather than the conservative
    /// [`CmvsTracker::committed_inside`] is what lets the LCR-only requirement fire in the
    /// § 7.3.2 begin-condition-3 case the membership tracker routes to Unknown: the
    /// activation evidence is decidable from the association chain even when the tracker
    /// cannot soundly classify membership.
    ///
    /// In-band-only: when external HLS declares any sequence header the activation chain
    /// (and thus which global LCR, if any, is activated) is not reliably in-band, so the
    /// agreement is suppressed — the same `external_declares_sequence_header` gate the
    /// sibling agreement checks use. (The locally-decidable in-band §6.8.2 value-space
    /// checks are unaffected; they live on the stateless syntax path.)
    pub(super) fn resolve_deferred_lcr_msdo_agreement(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        // The activated global LCR present in the current CMVS gates BOTH checks. This
        // resolution is window-scoped (§ 6.8.2 "present in the same CMVS") and decided from
        // the frame-confirmed association chain, so it fires even for an LCR-only CMVS the
        // membership tracker routes to Unknown.
        let Some(cmvs_start) = self.cmvs.current_cmvs_start_tu_index() else {
            return;
        };
        // CMVS-window starts only advance (TU indices are monotonic and a new CMVS opens at a
        // later temporal unit), so an MSDO observed before the current window can never be
        // in-window again — drop it to keep the accumulator bounded. Correctness rests on the
        // in-window filter below, not this prune.
        self.msdo_agreement_snapshots
            .retain(|snapshot| snapshot.observed_tu_index >= cmvs_start);
        let Some((global_id, global)) = self.activated_global_lcr() else {
            return;
        };
        // Snapshot the global record so the borrow on `self` is released before pushing
        // diagnostics through `&mut self` dedup state.
        let global = global.clone();

        // § 6.8.2 agreement: EVERY MSDO present in this CMVS must agree with the activated
        // global LCR (mirror lines 1646-1648). Evaluate each accumulated MSDO whose
        // observation temporal unit lies within the current CMVS window — a stale MSDO
        // recorded only in an earlier CMVS (its observation temporal unit precedes this CMVS's
        // window) is excluded. A per-MSDO last-wins overwrite of the live `msdo_substream_max`
        // would let an earlier non-conforming MSDO escape when a later conforming one replaced
        // it before this resolution; iterating the accumulator closes that hole (codex finding
        // 3393274380). The `emitted_lcr_agreement` key carries each MSDO's offset, so distinct
        // MSDOs fire distinctly and a re-resolution across the CMVS's temporal units does not
        // re-spam. Snapshot the in-window MSDOs before the `&mut self` push loop so the borrow
        // on `self.msdo_agreement_snapshots` is released.
        let in_window_msdos: Vec<(MsdoAggregate, ByteOffset)> = self
            .msdo_agreement_snapshots
            .iter()
            .filter(|snapshot| snapshot.observed_tu_index >= cmvs_start)
            .map(|snapshot| (snapshot.aggregate.clone(), snapshot.offset))
            .collect();
        for (msdo, msdo_offset) in in_window_msdos {
            self.check_lcr_msdo_agreement(global_id, &global, &msdo, msdo_offset, report);
        }

        // § 6.8.2 LCR DOH requirement: LCR-only, runs regardless of MSDO presence.
        self.check_lcr_doh_constraint_required(global_id, &global, report);
    }

    /// Resolves the deferred § 7.3.2 boundary-set-identity check
    /// (`cmvs/boundary-set-mismatch`, mirror `07-decoding-process.md` line 351) at
    /// temporal-unit-completion time, *after* the [`CmvsTracker`] has applied the temporal
    /// unit's begin/end conditions.
    ///
    /// > It is a requirement of bitstream conformance that, in a coded multistream video
    /// > sequence in which both an OBU_MSDO and an activated global layer configuration
    /// > record are present, the set of coded multistream video sequence boundaries
    /// > obtained by applying the rules of this section using both the MSDO and the
    /// > activated global layer configuration record shall be identical to the set of
    /// > boundaries obtained by applying those rules using the MSDO alone.
    ///
    /// **Decidable-disagreement-only (lesson 12 — Unknown never fires).** The only place
    /// the two boundary sets can diverge is § 7.3.2 end condition 2: a temporal unit that
    /// begins a new coded video sequence (a CLK) with no OBU_MSDO ENDS the CMVS under the
    /// MSDO-alone rules, but does NOT end it when it "has an activated global layer
    /// configuration record". The [`CmvsTracker`] flags this structural candidate
    /// ([`CmvsTracker::last_completed_is_boundary_divergence_candidate`]); the divergence is
    /// real, and decidable, only when the chain confirms the global LCR present *in the
    /// boundary temporal unit itself* is genuinely *activated*
    /// ([`Self::activated_global_lcr_in_tu`], scoped to the boundary TU index — not the whole
    /// CMVS window, codex finding 3393274375) and the CMVS it ended contained an MSDO
    /// (`msdo_substream_max`). End condition 2's "does not have an activated global layer
    /// configuration record" is a property of the BOUNDARY temporal unit, so an activated
    /// global LCR present only earlier in the CMVS keeps end condition 2 true at this boundary
    /// (both rule sets end the CMVS here — no mismatch). When both hold, the MSDO-alone set
    /// has a boundary the MSDO+LCR set lacks, so the requirement is violated. When the
    /// boundary TU's global LCR is only *present* but not activated (the tracker routes that
    /// to Unknown), nothing fires. The diagnostic anchors at the activated global LCR (the
    /// disagreeing record) and is deduped by the CMVS's MSDO offset.
    ///
    /// Suppressed when external HLS declares any sequence header (the activation chain that
    /// decides whether the global LCR is activated is then not reliably in-band) — the same
    /// gate the § 6.8.2 agreement uses.
    pub(super) fn resolve_deferred_cmvs_boundary(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        if !self.cmvs.last_completed_is_boundary_divergence_candidate() {
            return;
        }
        // The boundary-identity requirement applies only when both an OBU_MSDO and an
        // activated global LCR are present in the CMVS (mirror line 351). The CMVS being
        // divergently-ended was definitively Inside, so it was opened by an MSDO; require a
        // recorded MSDO and a chain-confirmed activated global LCR.
        let Some(substream_max) = self.msdo_substream_max.as_ref() else {
            return;
        };
        let msdo_offset = substream_max.offset;
        // § 7.3.2 end condition 2 verbatim: a temporal unit "that begins a new coded video
        // sequence for at least one extended layer but does not contain an OBU with obu_type
        // equal to OBU_MSDO and does not have an activated global layer configuration record"
        // ends the CMVS. The MSDO-alone rules always end the CMVS at this CLK-with-no-MSDO
        // boundary TU; the MSDO+LCR rules end it too UNLESS the boundary TU "has an activated
        // global layer configuration record". The divergence — and the mismatch — therefore
        // exists ONLY when the BOUNDARY temporal unit itself has an activated global LCR. An
        // activated global LCR present only EARLIER in the CMVS does not make end condition 2
        // false at this boundary TU, so the resolution is scoped to the boundary temporal unit
        // (`activated_global_lcr_in_tu`), NOT the whole CMVS window (codex finding 3393274375):
        // when the boundary TU carries no activated global LCR, both rule sets end the CMVS
        // here and there is no mismatch.
        let Some(boundary_tu_index) = self.cmvs.last_completed_tu_index() else {
            return;
        };
        let Some((global_id, global)) = self.activated_global_lcr_in_tu(boundary_tu_index) else {
            return;
        };
        let global_offset = global.offset;
        if !self.emitted_cmvs_boundary.insert(msdo_offset) {
            return;
        }
        report.push(
            Diagnostic::error(
                "cmvs/boundary-set-mismatch",
                format!(
                    "§ 7.3.2: a temporal unit begins a new coded video sequence with no OBU_MSDO \
                     but with the activated global layer configuration record {global_id}, so it \
                     ends the coded multistream video sequence under the MSDO-alone boundary rules \
                     yet continues it under the MSDO-plus-global-LCR rules; § 7.3.2 requires the \
                     two boundary sets to be identical in a CMVS containing both an OBU_MSDO and an \
                     activated global LCR",
                ),
            )
            .with_spec_section("7.3.2")
            .with_byte_offset(global_offset),
        );
    }

    /// Emits `sequence-state/monotonic-output-order-mismatch` (AV2 § 6.4.1) when, with
    /// the § 7.3.2 CMVS tracker definitively *inside* a coded multistream video
    /// sequence, the sequence header just activated for `xlayer` disagrees on
    /// `monotonic_output_order_flag` with the active sequence header of any other
    /// extended layer.
    ///
    /// AV2 § 6.4.1 (mirror `06-syntax-structures-semantics.md` lines 324-325): "It is a
    /// requirement of bitstream conformance that in a coded multistream video sequence,
    /// all extended layers shall be associated with the same value of
    /// monotonic_output_order_flag." The requirement is scoped to a CMVS, so the check
    /// fires only in [`CmvsState::Inside`] — never in `Outside` or `Unknown`
    /// (conservative under-approximation; the CMVS tracker is the only oracle, as no
    /// real multistream conformance vectors exist). `byte_offset` locates the activating
    /// OBU.
    ///
    /// Both sides of the comparison use only *decidable* activations
    /// ([`Self::agreement_activation_for`]): a frame-confirmed activation, or the
    /// OBU-order fallback while it is the sole in-band candidate. § 7.3.6 permits
    /// "additional sequence header OBUs with a different seq_header_id ... not activated
    /// ... until referenced by a subsequent CLK frame header", so an extended layer
    /// whose only in-band header is an as-yet-unreferenced first-seen guess that a
    /// later frame can contradict is not yet associated with a flag — comparing against
    /// that guess would emit an error a retraction could not undo. The activating layer
    /// `xlayer` is likewise skipped until its own activation is decidable.
    ///
    /// The verdict is routed through [`CmvsTracker::monotonic_verdict`]: when the
    /// activation is observed at a sequence-header OBU *before* any CLK in the temporal
    /// unit, the committed `Inside` is only provisional (a later MSDO-less CLK could end
    /// the CMVS, § 7.3.2 end condition 2), so a disagreement is deferred and resolved at
    /// temporal-unit completion. This defers-and-drops the § 7.3.6-permitted same-CVS
    /// redefinition that immediately precedes the CLK beginning the new coded video
    /// sequence (mirror `07-decoding-process.md` lines 608-611), which the eager
    /// header-time emission would otherwise flag as a false positive.
    pub(super) fn check_monotonic_output_order_agreement(
        &mut self,
        xlayer: ExtendedLayerId,
        byte_offset: ByteOffset,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // An externally activated sequence header has an unmodeled
        // monotonic_output_order_flag, so the cross-layer comparison is unreliable. Only a
        // *declared* external sequence header suppresses it: `ExternalHlsSet` cannot
        // declare external MSDO/LCR objects, but the CmvsTracker enters Inside only on
        // definitive in-band evidence and missing external MSDO/LCR evidence errs toward
        // Outside/Unknown (false-negative-only), so undeclared external objects cannot
        // make Inside spurious — narrowing this gate to declares_any_sequence_header() (as
        // the sibling gates and validate_active_sequence_limits do) is sound.
        if external_declares_sequence_header(options) {
            return;
        }
        // § 6.4.1 scopes the agreement to a coded multistream video sequence. Decide
        // whether the check fires now, is deferred (provisional Inside), or is skipped.
        let verdict = self.cmvs.monotonic_verdict();
        if matches!(verdict, MonotonicVerdict::Skip) {
            return;
        }
        // The activating layer's flag, only when its activation is decidable (a
        // frame-confirmed reference, or the sole in-band candidate). A first-seen
        // OBU-order fallback that several headers could contradict is not yet an
        // association (§ 7.3.6).
        let Some((_, general)) = self.agreement_activation_for(xlayer) else {
            return;
        };
        let flag = general.monotonic_output_order_flag;
        let mut disagreements = Vec::new();
        for &other_xlayer in self.active_sequence_by_xlayer.keys() {
            if other_xlayer == xlayer {
                continue;
            }
            // Compare only against another extended layer whose activation is equally
            // decidable; an unconfirmed first-seen guess for the other layer is not yet
            // associated with a flag, so a disagreement against it could be retracted by
            // a later frame and must not be emitted.
            let Some((_, other_general)) = self.agreement_activation_for(other_xlayer) else {
                continue;
            };
            if other_general.monotonic_output_order_flag != flag {
                disagreements.push(
                    Diagnostic::error(
                        "sequence-state/monotonic-output-order-mismatch",
                        format!(
                            "obu_xlayer_id {} activates a sequence header with \
                             monotonic_output_order_flag {} but obu_xlayer_id {} is associated \
                             with monotonic_output_order_flag {} in the same coded multistream \
                             video sequence; all extended layers must agree",
                            xlayer.get(),
                            u8::from(flag),
                            other_xlayer.get(),
                            u8::from(other_general.monotonic_output_order_flag)
                        ),
                    )
                    .with_spec_section("6.4.1")
                    .with_byte_offset(byte_offset),
                );
            }
        }
        for diagnostic in disagreements {
            match verdict {
                MonotonicVerdict::EmitNow => report.push(diagnostic),
                MonotonicVerdict::Defer => self.cmvs.queue_provisional_monotonic(diagnostic),
                MonotonicVerdict::Skip => {}
            }
        }
    }
}
