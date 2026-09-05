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
    /// activation state that this tracker does not model.
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
    /// scope their per-layer evaluation to. A
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
    /// temporal unit alone. `None` before the first completion.
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
    /// Membership effective for the current temporal unit (§ 7.3.2). MSDO precedes every
    /// CELU (§ 7.3.7), so CLK + MSDO proves Inside. An MSDO-less CLK ends an active CMVS,
    /// unless global-LCR activation is unresolved (Unknown). Before CLK, retain prior state.
    pub(super) fn state(&self) -> CmvsState {
        if self.current_tu.has_clk {
            if self.current_tu.msdo.is_some() {
                return CmvsState::Inside;
            }
            if matches!(self.state, CmvsState::Inside) {
                if self.current_tu.global_lcr_present {
                    return CmvsState::Unknown;
                }
                return CmvsState::Outside;
            }
        }
        self.state
    }

    /// Whether the just-completed temporal unit was definitively inside a CMVS.
    pub(super) fn committed_inside(&self) -> bool {
        matches!(self.state, CmvsState::Inside)
    }

    /// Defer disagreements before CLK: a later MSDO-less CLK may end this temporal
    /// unit’s CMVS (§ 7.3.2). Emit only once Inside membership is final.
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
    /// excluded from the § 6.8.2 / § 6.6 DOH evaluations.
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
        self.last_completed_tu_index = Some(completed_tu_index);
        match window_action {
            CmvsWindowAction::Open => self.cmvs_start_tu_index = Some(completed_tu_index),
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
    /// membership state routes to [`CmvsState::Unknown`] because this tracker does not
    /// model § 7.3.8 activation. Opening the window there is sound: the § 6.8.2 DOH requirement and agreement
    /// resolve "an activated global LCR" from the association chain (`activated_global_lcr`),
    /// which IS decidable, so an LCR-only CMVS still needs a window for those checks to scope
    /// to; if the chain finds no activated global LCR, nothing fires regardless of the window.
    pub(super) fn next_state(&self, facts: &CmvsTuFacts) -> (CmvsState, CmvsWindowAction) {
        if facts.has_clk {
            let currently_active = matches!(self.state, CmvsState::Inside);
            match facts.msdo {
                Some(_) if !currently_active => {
                    return (CmvsState::Inside, CmvsWindowAction::Open);
                }
                Some(MsdoObservation::Changed) => {
                    return (CmvsState::Inside, CmvsWindowAction::Open);
                }
                Some(MsdoObservation::First | MsdoObservation::Unchanged) => {
                    return (CmvsState::Inside, CmvsWindowAction::Keep);
                }
                None => {
                    if facts.global_lcr_present && !currently_active {
                        return (CmvsState::Unknown, CmvsWindowAction::Open);
                    }
                }
            }
        }

        if matches!(self.state, CmvsState::Inside) {
            if facts.has_clk && facts.msdo.is_none() {
                if facts.global_lcr_present {
                    return (CmvsState::Unknown, CmvsWindowAction::Close);
                }
                return (CmvsState::Outside, CmvsWindowAction::Close);
            }
            return (CmvsState::Inside, CmvsWindowAction::Keep);
        }

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
    /// Uses the new CMVS window when this temporal unit opens one, otherwise the prior
    /// window if completion closed it. The preceding MSDO remains independent of the window.
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

    /// Checks § 6.6 DOH requirements after temporal-unit membership is final. Only
    /// frame-confirmed activations within the current CMVS count; earlier headers do not.
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

    /// Checks § 6.8.2 against the activated global LCR in the current CMVS. Agreement
    /// requires an MSDO in that window; the LCR DOH requirement also applies without MSDO.
    /// External sequence headers make the in-band association undecidable.
    pub(super) fn resolve_deferred_lcr_msdo_agreement(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        let Some(cmvs_start) = self.cmvs.current_cmvs_start_tu_index() else {
            return;
        };
        self.msdo_agreement_snapshots
            .retain(|snapshot| snapshot.observed_tu_index >= cmvs_start);
        let Some((global_id, global)) = self.activated_global_lcr() else {
            return;
        };
        let global = global.clone();

        let in_window_msdos: Vec<(MsdoAggregate, ByteOffset)> = self
            .msdo_agreement_snapshots
            .iter()
            .map(|snapshot| (snapshot.aggregate.clone(), snapshot.offset))
            .collect();
        for (msdo, msdo_offset) in in_window_msdos {
            self.check_lcr_msdo_agreement(global_id, &global, &msdo, msdo_offset, report);
        }

        self.check_lcr_doh_constraint_required(global_id, &global, report);
    }

    /// Checks § 7.3.2 boundary-set identity: an MSDO-less CLK ends the MSDO-only CMVS,
    /// but an activated global LCR in that boundary temporal unit prevents the same end.
    /// Earlier-window or unactivated LCRs do not prove a mismatch; external HLS suppresses it.
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
        let Some(substream_max) = self.msdo_substream_max.as_ref() else {
            return;
        };
        let msdo_offset = substream_max.offset;
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

    /// Checks § 6.4.1 agreement across decidably activated headers within a CMVS.
    /// Before CLK, defer disagreements until membership resolves at temporal-unit completion.
    pub(super) fn check_monotonic_output_order_agreement(
        &mut self,
        xlayer: ExtendedLayerId,
        byte_offset: ByteOffset,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        let verdict = self.cmvs.monotonic_verdict();
        if matches!(verdict, MonotonicVerdict::Skip) {
            return;
        }
        let Some((_, general)) = self.agreement_activation_for(xlayer) else {
            return;
        };
        let flag = general.monotonic_output_order_flag;
        let mut disagreements = Vec::new();
        for &other_xlayer in self.active_sequence_by_xlayer.keys() {
            if other_xlayer == xlayer {
                continue;
            }
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
