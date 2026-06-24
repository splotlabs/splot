// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Annex A interoperability-point presence-window checks.

use super::*;

/// The Annex A Table A.4 interoperability-point OBU-presence tracker (AV2 v1.0.0 Annex A.2
/// Table A.4, mirror `annex-a-profiles-levels-and-tiers.md` lines 178-201), scoped to a
/// coded (multistream-)video-sequence window.
///
/// The window spans the whole coded video sequence and is evaluated at its end — the start
/// of the next coded video sequence (a CLK in a *later* temporal unit, § 7.3.6) or the end
/// of the bitstream. Per-temporal-unit observations accumulate in [`Self::pending`]; at
/// temporal-unit completion they are committed to the right window (lesson 8): a temporal
/// unit that begins a new coded video sequence (has a CLK while a window opened in an
/// earlier temporal unit is still open) first flushes the prior window, then seeds a fresh
/// window from *this* temporal unit's pending facts — so an OBU_MSDO (or any HLS) observed
/// BEFORE the CLK in the CLK-bearing temporal unit belongs to the NEW coded video sequence,
/// not the prior one (§ 7.3.6: the new coded video sequence starts at the temporal unit
/// containing the CLK).
///
/// The presence-requirement evaluation needs frame-confirmed activation state that is only
/// final at temporal-unit completion (which sequence headers are activated, whether a
/// global LCR is *activated*, and the MSDO's `multistream_profile_idc`), so the window's
/// interoperability point, extended/embedded-layer counts, and activated-global-LCR flag
/// are resolved from the live context at flush time, not accumulated per-OBU.
#[derive(Debug, Default)]
pub(super) struct AnnexAIopTracker {
    /// The currently-open coded-video-sequence window, or `None` before the first
    /// observation.
    pub(super) window: Option<AnnexAIopWindow>,
    /// The temporal unit currently being observed, committed to the window at temporal-unit
    /// completion (see [`Self::commit_pending`]).
    pub(super) pending: TuIopFacts,
}

/// One temporal unit's Annex A Table A.4 facts, accumulated as OBUs are observed and
/// committed to the [`AnnexAIopTracker`]'s window when the temporal unit completes.
#[derive(Debug, Default, Clone)]
pub(super) struct TuIopFacts {
    /// Distinct non-global `obu_xlayer_id` values observed in this temporal unit (Table A.3
    /// extended-layer base count, mirror lines 146-151).
    pub(super) distinct_xlayers: BTreeSet<ExtendedLayerId>,
    /// The largest `num_streams_minus_2 + 2` of any OBU_MSDO in this temporal unit, with
    /// the OBU offset, when present.
    pub(super) msdo: Option<(u32, ByteOffset)>,
    /// `multistream_profile_idc` of the OBU_MSDO in this temporal unit (the Table A.4 IOP
    /// source when an MSDO is present), when present.
    pub(super) msdo_profile_idc: Option<u8>,
    /// A local layer configuration record OBU was present in this temporal unit.
    pub(super) local_lcr_present: bool,
    /// A global layer configuration record OBU was present in this temporal unit (raw
    /// presence; activation is resolved separately at flush).
    pub(super) global_lcr_present: bool,
    /// This temporal unit contains an `OBU_CLOSED_LOOP_KEY` for at least one extended layer
    /// (§ 7.3.6: begins a new coded video sequence for that layer).
    pub(super) has_clk: bool,
    /// The interoperability point agreed by the *frame-confirmed* sequence headers activated
    /// in this temporal unit, when no MSDO IOP overrides it (the MSDO's
    /// `multistream_profile_idc` is the IOP source when an MSDO is present, mirror lines
    /// 1659-1662). `None` until an activation with a table-mapped profile occurs.
    pub(super) iop: Option<AnnexAIopState>,
    /// The maximum `seq_max_mlayer_cnt_minus_1 + 1` across frame-confirmed activated headers
    /// in this temporal unit (Table A.3 "Number of Embedded Layers", mirror lines 152-153).
    pub(super) max_embedded_layers: u32,
    /// `LcrMaxNumXLayerCount` of an *activated* global LCR resolved from a frame-confirmed
    /// activation in this temporal unit, when one resolves (the Table A.3 declared
    /// extended-layer count under an activated global LCR, mirror lines 149-150, and the
    /// signal the Table A.4 global-LCR arms require — only an activated global LCR counts).
    pub(super) activated_global_count: Option<u32>,
    /// Byte offset of the latest evidence-bearing OBU in this temporal unit, for the
    /// diagnostic anchor.
    pub(super) anchor_offset: Option<ByteOffset>,
}

/// The interoperability-point state of an Annex A IOP window: a single agreed IOP, or
/// `Mixed` when activated profiles disagree (the Table A.4 row is then not determinable,
/// so the check is skipped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnnexAIopState {
    /// A single agreed interoperability point.
    Single(InteroperabilityPoint),
    /// Activated profiles disagree on the interoperability point; skip the check.
    Mixed,
}

/// One coded-(multistream-)video-sequence window's accumulated Annex A Table A.4 evidence.
#[derive(Debug, Clone)]
pub(super) struct AnnexAIopWindow {
    /// Distinct non-global `obu_xlayer_id` values observed across the window (Table A.3
    /// extended-layer base count).
    pub(super) distinct_xlayers: BTreeSet<ExtendedLayerId>,
    /// The largest `num_streams_minus_2 + 2` of any OBU_MSDO in the window, when present —
    /// the declared Table A.3 extended-layer count under `MultiStreamDecoderMode == 1`
    /// (mirror lines 148-149).
    pub(super) msdo_num_streams: Option<u32>,
    /// `multistream_profile_idc` of an OBU_MSDO in the window, the Table A.4 interoperability
    /// point source when an MSDO is present (mirror lines 1659-1662). `None` when no MSDO is
    /// in the window.
    pub(super) msdo_profile_idc: Option<u8>,
    /// `true` if an `OBU_MSDO` occurred in the window.
    pub(super) msdo_present: bool,
    /// `true` if a local `OBU_LAYER_CONFIGURATION_RECORD` was present in the window.
    pub(super) local_lcr_present: bool,
    /// `LcrMaxNumXLayerCount` of an *activated* global LCR resolved in the window, when one
    /// resolved. Only an activated global LCR satisfies the Table A.4 global-LCR arms and
    /// contributes the Table A.3 declared extended-layer count; an observed-but-unactivated
    /// global LCR leaves this `None`.
    pub(super) activated_global_count: Option<u32>,
    /// The interoperability point agreed by the window's frame-confirmed activated headers,
    /// or `None`/`Mixed` when undecidable. Overridden by the MSDO's `multistream_profile_idc`
    /// at evaluation when an MSDO is present (mirror lines 1659-1662).
    pub(super) iop: Option<AnnexAIopState>,
    /// The maximum `seq_max_mlayer_cnt_minus_1 + 1` across the window's activated headers
    /// (Table A.3 "Number of Embedded Layers", mirror lines 152-153).
    pub(super) max_embedded_layers: u32,
    /// Byte offset anchoring the window's diagnostic — the latest evidence-bearing OBU.
    pub(super) anchor_offset: ByteOffset,
    /// The [`CvsTracker::tu_index`] of the temporal unit in which this window's coded video
    /// sequence began (the temporal unit carrying its CLKs), or `None` for leading evidence
    /// before the first CLK. A CLK in a *later* temporal unit begins the next coded video
    /// sequence and flushes this window (§ 7.3.6).
    pub(super) cvs_start_tu: Option<u64>,
}

impl Default for AnnexAIopWindow {
    fn default() -> Self {
        Self {
            distinct_xlayers: BTreeSet::new(),
            msdo_num_streams: None,
            msdo_profile_idc: None,
            msdo_present: false,
            local_lcr_present: false,
            activated_global_count: None,
            iop: None,
            max_embedded_layers: 0,
            anchor_offset: ByteOffset::new(0),
            cvs_start_tu: None,
        }
    }
}

impl AnnexAIopTracker {
    /// Records a non-global `obu_xlayer_id` observed in the current temporal unit (Table
    /// A.3 extended-layer base count).
    pub(super) fn note_xlayer(&mut self, xlayer: ExtendedLayerId) {
        if !xlayer.is_global() {
            self.pending.distinct_xlayers.insert(xlayer);
        }
    }

    /// Records an OBU_MSDO observed in the current temporal unit: its declared substream
    /// count, `multistream_profile_idc` (the Table A.4 IOP source), and OBU offset.
    pub(super) fn note_msdo(&mut self, num_streams: u32, profile_idc: u8, offset: ByteOffset) {
        let best = self
            .pending
            .msdo
            .map_or(num_streams, |(prev, _)| prev.max(num_streams));
        self.pending.msdo = Some((best, offset));
        self.pending.msdo_profile_idc = Some(profile_idc);
    }

    /// Records that a global LCR OBU was present in the current temporal unit.
    pub(super) fn note_global_lcr(&mut self, offset: ByteOffset) {
        self.pending.global_lcr_present = true;
        self.pending.anchor_offset = Some(offset);
    }

    /// Records that a local LCR OBU was present in the current temporal unit.
    pub(super) fn note_local_lcr(&mut self) {
        self.pending.local_lcr_present = true;
    }

    /// Records that the current temporal unit contains an `OBU_CLOSED_LOOP_KEY`.
    pub(super) fn note_clk(&mut self) {
        self.pending.has_clk = true;
    }

    /// Records a frame-confirmed sequence-header activation in the current temporal unit:
    /// its profile's interoperability point (Annex A.2 Table A.1), its embedded-layer count
    /// (`seq_max_mlayer_cnt_minus_1 + 1`), the `LcrMaxNumXLayerCount` of the *activated*
    /// global LCR it resolves (if any — only an activated global LCR is recorded here), and
    /// the activating OBU offset. A reserved / Configurable profile leaves the IOP unset
    /// (its interoperability point is not table-determined); two activations disagreeing on
    /// the IOP mark the window [`AnnexAIopState::Mixed`] and the Table A.4 check is then
    /// skipped (multistream profile-agreement is out of scope here).
    pub(super) fn note_activation(
        &mut self,
        profile_idc: u8,
        embedded_layers: u32,
        activated_global_count: Option<u32>,
        offset: ByteOffset,
    ) {
        self.pending.max_embedded_layers = self.pending.max_embedded_layers.max(embedded_layers);
        self.pending.anchor_offset = Some(offset);
        if let Some(count) = activated_global_count {
            self.pending.activated_global_count =
                Some(self.pending.activated_global_count.unwrap_or(0).max(count));
        }
        if let Some(iop) = interoperability_point(profile_idc) {
            self.pending.iop = Some(match self.pending.iop {
                None => AnnexAIopState::Single(iop),
                Some(AnnexAIopState::Single(existing)) if existing == iop => {
                    AnnexAIopState::Single(existing)
                }
                Some(_) => AnnexAIopState::Mixed,
            });
        }
    }

    /// Whether committing the current temporal unit's pending facts begins a NEW coded
    /// video sequence relative to the open window — a CLK in this temporal unit while a
    /// window whose coded video sequence began in an *earlier* temporal unit is open. A CLK
    /// in the same temporal unit the window's coded video sequence began in (a second
    /// extended layer's CLK within one multistream random-access temporal unit) continues
    /// the same window; leading evidence with no recorded coded-video-sequence start
    /// (`cvs_start_tu == None`) is absorbed by the first coded video sequence.
    pub(super) fn pending_starts_new_cvs(&self, tu_index: u64) -> bool {
        self.pending.has_clk
            && matches!(
                self.window.as_ref().and_then(|w| w.cvs_start_tu),
                Some(start) if start != tu_index
            )
    }

    /// Merges the current temporal unit's pending facts into `window` (the same coded video
    /// sequence continues across this temporal unit), recording the coded-video-sequence
    /// start temporal unit when this temporal unit carries the window's CLK.
    pub(super) fn merge_pending_into(
        window: &mut AnnexAIopWindow,
        pending: &TuIopFacts,
        tu_index: u64,
    ) {
        window
            .distinct_xlayers
            .extend(pending.distinct_xlayers.iter().copied());
        if let Some((num_streams, offset)) = pending.msdo {
            window.msdo_present = true;
            window.msdo_num_streams = Some(window.msdo_num_streams.unwrap_or(0).max(num_streams));
            window.anchor_offset = offset;
        }
        if let Some(profile) = pending.msdo_profile_idc {
            window.msdo_profile_idc = Some(profile);
        }
        window.local_lcr_present |= pending.local_lcr_present;
        if let Some(count) = pending.activated_global_count {
            window.activated_global_count =
                Some(window.activated_global_count.unwrap_or(0).max(count));
        }
        window.max_embedded_layers = window.max_embedded_layers.max(pending.max_embedded_layers);
        if let Some(offset) = pending.anchor_offset {
            window.anchor_offset = offset;
        }
        // Combine this temporal unit's IOP into the window's: a single agreed IOP carries
        // through; a disagreement marks the window Mixed (the Table A.4 row is then not
        // determinable, so the check is skipped).
        window.iop = match (window.iop, pending.iop) {
            (None, p) => p,
            (w, None) => w,
            (Some(AnnexAIopState::Single(a)), Some(AnnexAIopState::Single(b))) if a == b => {
                Some(AnnexAIopState::Single(a))
            }
            _ => Some(AnnexAIopState::Mixed),
        };
        if pending.has_clk {
            window.cvs_start_tu.get_or_insert(tu_index);
        }
    }

    /// Builds a fresh window from a temporal unit's pending facts (a temporal unit that
    /// begins a new coded video sequence). The new window's coded-video-sequence start is
    /// this temporal unit.
    pub(super) fn window_from_pending(pending: &TuIopFacts, tu_index: u64) -> AnnexAIopWindow {
        let mut window = AnnexAIopWindow::default();
        Self::merge_pending_into(&mut window, pending, tu_index);
        window
    }
}

/// The Table A.3 "Number of Extended Layers" for an [`AnnexAIopWindow`] (mirror lines
/// 146-151), in the mirror's exact definition order — a *declared* count takes precedence
/// over the observed coded structure:
///
/// 1. `MultiStreamDecoderMode == 1` (an OBU_MSDO is present): `num_streams_minus_2 + 2`
///    (mirror lines 148-149), regardless of how many distinct `obu_xlayer_id` materialize.
/// 2. else, an *activated* global LCR (`window.activated_global_count` resolved):
///    `LcrMaxNumXLayerCount` (mirror lines 149-150).
/// 3. else: the distinct non-global `obu_xlayer_id` count actually present, at least 1
///    (mirror lines 150-151; Table A.3 "For a coded video sequence, this value is equal to
///    1").
///
/// `window.activated_global_count` is `None` when no activated global LCR resolves, so an
/// observed-but-unactivated global LCR does not contribute a declared count (it falls
/// through to the observed distinct count).
pub(super) fn annex_a_extended_layers(window: &AnnexAIopWindow) -> u32 {
    if let Some(num_streams) = window.msdo_num_streams {
        return num_streams;
    }
    if let Some(count) = window.activated_global_count {
        return count;
    }
    (window.distinct_xlayers.len() as u32).max(1)
}

/// Builds an Annex A Table A.4 interoperability-point presence diagnostic (error, spec
/// section `A.2`, anchored at `offset`).
pub(super) fn annex_a_iop_error(
    rule_id: &'static str,
    offset: ByteOffset,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("A.2")
        .with_byte_offset(offset)
}

impl ValidatorContext {
    /// Records the frame-confirmed sequence-header activation for `xlayer` into the Annex A
    /// Table A.4 IOP pending facts: the header's profile (for the interoperability point),
    /// its embedded-layer count (`seq_max_mlayer_cnt_minus_1 + 1`), and the
    /// `LcrMaxNumXLayerCount` of the *activated* global LCR its `seq_lcr_id` resolves to
    /// (only an activated global LCR is recorded — the Table A.4 global-LCR arms require an
    /// activated record, lesson 10). Only a frame-confirmed activation is recorded (a staged
    /// fallback guess could be contradicted by a later frame, § 7.3.6). Suppressed under any
    /// Provided external HLS (`matches!(.., Provided(_))`): an external header may shadow the
    /// in-band one, so in-band presence counting is unsound — the same gate the window
    /// evaluation uses (`evaluate_annex_a_iop_window`).
    pub(super) fn note_annex_a_iop_activation(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
    ) {
        if matches!(options.external_hls, ExternalHlsMode::Provided(_)) {
            return;
        }
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return;
        }
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        let offset = self
            .sequence_header_offsets
            .get(&seq_header_id)
            .copied()
            .unwrap_or(ByteOffset::new(0));
        // The activated global LCR span for this layer, if its association resolved a
        // global record. Only an *activated* (associated) global LCR contributes the
        // Table A.3 declared count / satisfies the Table A.4 global-LCR arms.
        //
        // Read `LcrMaxNumXLayerCount` from the association-time snapshot
        // (`association.global_record`), NOT a live `global_lcr_records` lookup, exactly like
        // the § 6.8.2 agreement path (`activated_global_lcr_in_window`). A same-id global-LCR
        // redefinition *after* this header associated otherwise retargets the count to the
        // later revision's `lcr_xlayer_map`; the snapshot keeps the Table A.4 layer accounting
        // pinned to the revision this header actually associated to.
        let activated_global_count = self
            .lcr_associations
            .get(&(xlayer, seq_header_id))
            .filter(|a| a.lcr_is_global)
            .and_then(|a| a.global_record.as_ref())
            .map(|record| record.max_num_xlayer_count);
        self.annex_a_iop.note_activation(
            general.seq_profile_idc.get(),
            u32::from(general.seq_max_mlayer_count.get()),
            activated_global_count,
            offset,
        );
    }

    /// Commits the just-completed temporal unit's Annex A Table A.4 IOP pending facts to
    /// the right coded-(multistream-)video-sequence window (AV2 § 7.3.6 per-temporal-unit
    /// attribution, lesson 8). `completed_tu_index` is the temporal unit's index.
    ///
    /// When this temporal unit begins a NEW coded video sequence (it has a CLK and the open
    /// window's coded video sequence began in an *earlier* temporal unit), the prior window
    /// is first flushed and evaluated, then a fresh window is seeded from this temporal
    /// unit's pending facts — so a same-temporal-unit pre-CLK OBU_MSDO/LCR belongs to the
    /// NEW coded video sequence (§ 7.3.6: the new coded video sequence starts at the
    /// temporal unit containing the CLK). Otherwise the pending facts merge into the open
    /// window (the same coded video sequence continues across this temporal unit). The
    /// pending facts reset for the next temporal unit.
    pub(super) fn commit_annex_a_iop_pending(
        &mut self,
        completed_tu_index: u64,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if self.annex_a_iop.pending_starts_new_cvs(completed_tu_index) {
            // This temporal unit begins the next coded video sequence: flush+evaluate the
            // ending window, then seed a fresh one from this temporal unit's pending facts.
            self.flush_annex_a_iop_window(options, report);
            let pending = std::mem::take(&mut self.annex_a_iop.pending);
            self.annex_a_iop.window = Some(AnnexAIopTracker::window_from_pending(
                &pending,
                completed_tu_index,
            ));
        } else {
            // The same coded video sequence continues (or leading evidence before the first
            // CLK): merge this temporal unit's pending facts into the open window.
            let pending = std::mem::take(&mut self.annex_a_iop.pending);
            let window = self
                .annex_a_iop
                .window
                .get_or_insert_with(AnnexAIopWindow::default);
            AnnexAIopTracker::merge_pending_into(window, &pending, completed_tu_index);
        }
        // Both branches above `std::mem::take` `self.annex_a_iop.pending`, leaving it at
        // `TuIopFacts::default()`, so an explicit `reset_pending()` here would be a no-op.
    }

    /// Takes the current Annex A Table A.4 IOP window and evaluates its MSDO/LCR
    /// interoperability-point presence requirements, resetting the window for the next coded
    /// video sequence. Suppressed (the window is taken but no diagnostic is emitted) under
    /// any Provided external HLS, which makes in-band presence counting unsound.
    pub(super) fn flush_annex_a_iop_window(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let Some(window) = self.annex_a_iop.window.take() else {
            return;
        };
        if matches!(options.external_hls, ExternalHlsMode::Provided(_)) {
            return;
        }
        self.evaluate_annex_a_iop_window(&window, report);
    }

    /// Emits the Annex A Table A.4 MSDO/LCR interoperability-point presence diagnostics for
    /// one coded-(multistream-)video-sequence `window` (AV2 v1.0.0 Annex A.2 Table A.4,
    /// mirror `annex-a-profiles-levels-and-tiers.md` lines 178-201).
    ///
    /// The interoperability point is taken from the window's OBU_MSDO `multistream_profile_idc`
    /// when an MSDO is present (mirror lines 1659-1662), else from the window's
    /// frame-confirmed activated headers (lesson; see [`AnnexAIopWindow::iop`]). A window with
    /// no decidable single interoperability point (no in-band profile, a reserved /
    /// Configurable profile whose IOP is not table-determined, or mixed IOPs across layers) is
    /// a no-op — the Table A.4 row is not determinable.
    ///
    /// `E = "Number of Extended Layers > 1"` and `M = "Number of Embedded Layers > 1"` are the
    /// Table A.3 counts ([`annex_a_extended_layers`] in declared precedence, mirror lines
    /// 146-151; the embedded-layer maximum, lines 152-153). The Table A.4 rows, by IOP:
    ///
    /// - IOP0 (lines 183-185): MSDO prohibited when `!E`, required when `E`.
    /// - IOP1 (lines 187-191): `!E && !M` -> MSDO prohibited; `E && !M` -> MSDO required;
    ///   `!E && M` -> MSDO prohibited and a local LCR required. (`E && M` exceeds IOP1's Table
    ///   A.3 layer budget and has no Table A.4 row.)
    /// - IOP2 (lines 193-201): `!E && !M` -> MSDO prohibited; `E && !M` -> MSDO **or** an
    ///   activated global LCR required (either satisfies); `!E && M` -> MSDO prohibited and an
    ///   LCR (local or activated global) required; `E && M` -> (MSDO **and** local LCR) **or**
    ///   an activated global LCR required.
    ///
    /// Only an *activated* global LCR ([`AnnexAIopWindow::activated_global_count`], resolved
    /// via the association chain) satisfies the global-LCR arms (lesson 10); an
    /// observed-but-unactivated global LCR does not.
    pub(super) fn evaluate_annex_a_iop_window(
        &self,
        window: &AnnexAIopWindow,
        report: &mut ValidationReport,
    ) {
        // The MSDO's multistream_profile_idc determines the IOP when an MSDO is present
        // (mirror lines 1659-1662); otherwise the activated headers' agreed IOP is used.
        let iop = match window.msdo_profile_idc {
            Some(profile) => match interoperability_point(profile) {
                Some(iop) => iop,
                // Reserved / Configurable multistream_profile_idc: IOP not table-determined.
                None => return,
            },
            None => match window.iop {
                Some(AnnexAIopState::Single(iop)) => iop,
                // No in-band profile, or activated profiles disagree: row not determinable.
                _ => return,
            },
        };
        let extended_layers = annex_a_extended_layers(window);
        let e = extended_layers > 1;
        let m = window.max_embedded_layers.max(1) > 1;
        let offset = window.anchor_offset;
        let global_lcr = window.activated_global_count.is_some();
        // AV2 Annex A Table A.3 (mirror lines 125-170): the per-IOP layer budget. Checked
        // before the Table A.4 presence rules — an IOP1 window with both E and M exceeds the
        // budget and has no Table A.4 row, so the budget bound is the only constraint on it.
        self.emit_iop_layer_budget(
            iop,
            extended_layers,
            window.max_embedded_layers.max(1),
            e,
            m,
            offset,
            report,
        );
        match iop {
            InteroperabilityPoint::Iop0 => {
                // Rows 1-2 (lines 183-185): embedded layers are N/A.
                self.emit_iop_msdo_presence(e, window, extended_layers, offset, report);
            }
            InteroperabilityPoint::Iop1 => {
                if !m {
                    // Rows 3-4 (lines 187-189): MSDO prohibited (!E) / required (E).
                    self.emit_iop_msdo_presence(e, window, extended_layers, offset, report);
                } else if !e {
                    // Row 5 (line 191): !E && M -> MSDO prohibited; local LCR required.
                    self.emit_msdo_prohibited(window, offset, report);
                    self.emit_iop1_local_lcr_required(window, offset, report);
                }
                // E && M: no Table A.4 row (outside IOP1's layer budget); see the TODO.
            }
            InteroperabilityPoint::Iop2 => {
                self.evaluate_iop2(e, m, window, global_lcr, extended_layers, offset, report);
            }
        }
    }

    /// AV2 Annex A Table A.3 layer-budget bound (mirror lines 125-170) for the window's
    /// interoperability point. Table A.3 caps, per IOP, the Number of Extended Layers, the
    /// Number of Embedded Layers, and whether the Extended-and-Embedded *combination*
    /// (`E && M`, both counts > 1) is permitted:
    ///
    /// - IOP0 (line 130): extended 1-4, embedded 1, combination 0.
    /// - IOP1 (line 132): extended 1-4, embedded 1-2, combination 0.
    /// - IOP2 (line 134): extended 1-4, embedded 1-3, combination 0 or 1.
    ///
    /// **Zero-false-positive.** `extended_layers`
    /// ([`annex_a_extended_layers`]) and `embedded_layers` (`max_embedded_layers`) are
    /// conservative LOWER bounds — they under-count when activations are missing, never
    /// over-count — so a count that exceeds its IOP limit is a proven violation. The IOP is
    /// table-determined here (the caller already returned for a reserved / Configurable /
    /// disagreeing profile). The Table A.3 "Number of Layers" (sum of embedded counts across
    /// singlestreams) bound is not tracked and stays a named residual.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_iop_layer_budget(
        &self,
        iop: InteroperabilityPoint,
        extended_layers: u32,
        embedded_layers: u32,
        e: bool,
        m: bool,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        // (max extended layers, max embedded layers, is the E && M combination permitted).
        let (max_extended, max_embedded, combination_allowed) = match iop {
            InteroperabilityPoint::Iop0 => (4, 1, false),
            InteroperabilityPoint::Iop1 => (4, 2, false),
            InteroperabilityPoint::Iop2 => (4, 3, true),
        };
        let mut violations = Vec::new();
        if extended_layers > max_extended {
            violations.push(format!(
                "{extended_layers} extended layers exceed the maximum {max_extended}"
            ));
        }
        if embedded_layers > max_embedded {
            violations.push(format!(
                "{embedded_layers} embedded layers exceed the maximum {max_embedded}"
            ));
        }
        if e && m && !combination_allowed {
            violations.push(
                "more than one extended layer and more than one embedded layer (the \
                 Extended-and-Embedded combination is not permitted)"
                    .to_owned(),
            );
        }
        if !violations.is_empty() {
            report.push(annex_a_iop_error(
                "annex-a/layer-budget-exceeds-iop",
                offset,
                format!(
                    "Annex A Table A.3: interoperability point {} layer budget exceeded: {}",
                    iop.value(),
                    violations.join("; ")
                ),
            ));
        }
    }

    /// Table A.4 IOP0 rows and IOP1 `!M` rows: MSDO required when `E`, prohibited when `!E`.
    pub(super) fn emit_iop_msdo_presence(
        &self,
        e: bool,
        window: &AnnexAIopWindow,
        extended_layers: u32,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if e {
            if !window.msdo_present {
                report.push(annex_a_iop_error(
                    "annex-a/msdo-required-for-iop",
                    offset,
                    format!(
                        "Annex A Table A.4: the coded video sequence has more than one extended \
                         layer ({extended_layers}) but contains no OBU_MSDO, which the activated \
                         profile's interoperability point requires"
                    ),
                ));
            }
        } else {
            self.emit_msdo_prohibited(window, offset, report);
        }
    }

    /// Table A.4 IOP2 rows (mirror lines 193-201). `global_lcr` is whether an *activated*
    /// global LCR is present in the window (only an activated one satisfies the global-LCR
    /// arms).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn evaluate_iop2(
        &self,
        e: bool,
        m: bool,
        window: &AnnexAIopWindow,
        global_lcr: bool,
        extended_layers: u32,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        match (e, m) {
            // Row "2 N N" (line 193): MSDO prohibited.
            (false, false) => self.emit_msdo_prohibited(window, offset, report),
            // Row "2 Y N" (line 195): MSDO or an activated global LCR required (either
            // satisfies); MSDO is not prohibited here.
            (true, false) => {
                if !window.msdo_present && !global_lcr {
                    report.push(annex_a_iop_error(
                        "annex-a/msdo-required-for-iop",
                        offset,
                        format!(
                            "Annex A Table A.4: interoperability point 2 with more than one \
                             extended layer ({extended_layers}) requires an OBU_MSDO or an \
                             activated global OBU_LAYER_CONFIGURATION_RECORD, but neither is \
                             present in the coded video sequence"
                        ),
                    ));
                }
            }
            // Row "2 N Y" (line 197): MSDO prohibited; LCR (local or activated global)
            // required.
            (false, true) => {
                self.emit_msdo_prohibited(window, offset, report);
                if !global_lcr && !window.local_lcr_present {
                    report.push(annex_a_iop_error(
                        "annex-a/lcr-required-for-iop",
                        offset,
                        "Annex A Table A.4: interoperability point 2 with more than one embedded \
                         layer requires a local or activated global \
                         OBU_LAYER_CONFIGURATION_RECORD, but none is present in the coded video \
                         sequence"
                            .to_owned(),
                    ));
                }
            }
            // Row "2 Y Y" (lines 199-200): (MSDO and local LCR) or an activated global LCR
            // required.
            (true, true) => {
                let satisfied = (window.msdo_present && window.local_lcr_present) || global_lcr;
                if !satisfied {
                    report.push(annex_a_iop_error(
                        "annex-a/lcr-required-for-iop",
                        offset,
                        "Annex A Table A.4: interoperability point 2 with more than one extended \
                         layer and more than one embedded layer requires either an OBU_MSDO plus a \
                         local OBU_LAYER_CONFIGURATION_RECORD, or an activated global \
                         OBU_LAYER_CONFIGURATION_RECORD, but neither combination is present in the \
                         coded video sequence"
                            .to_owned(),
                    ));
                }
            }
        }
    }

    /// Emits `annex-a/msdo-prohibited-for-iop` when an MSDO is present in a window whose
    /// Table A.4 row prohibits one.
    ///
    /// This is the documented *defensive* arm. Under the Table A.3 "Number of Extended
    /// Layers" definition ([`annex_a_extended_layers`], declared precedence), a present
    /// OBU_MSDO declares `num_streams_minus_2 + 2 >= 2`, so `E = extended_layers > 1` is
    /// always true when `msdo_present` is true. Every Table A.4 "MSDO Prohibited" row
    /// requires `E` to be false (`E == 1`), so a caller reaching this method with `!E`
    /// already has `!msdo_present`, and this body never fires in-band today. The genuine
    /// violation the prohibition rows would catch — an MSDO declaring substreams that never
    /// materialize as distinct extended layers — is the declared-vs-observed reconciliation
    /// owned by the § 6.6 sub-stream change, not this presence window. The id stays emitted
    /// (and registered) so a future declared-vs-observed model can reach it.
    pub(super) fn emit_msdo_prohibited(
        &self,
        window: &AnnexAIopWindow,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if window.msdo_present {
            report.push(annex_a_iop_error(
                "annex-a/msdo-prohibited-for-iop",
                offset,
                "Annex A Table A.4: the coded video sequence does not have more than one extended \
                 layer, so an OBU_MSDO is prohibited for the activated profile's interoperability \
                 point"
                    .to_owned(),
            ));
        }
    }

    /// Emits `annex-a/lcr-required-for-iop` for the IOP1 `!E && M` "Required (Local)" row
    /// (mirror line 191) when no local LCR is present in the window.
    pub(super) fn emit_iop1_local_lcr_required(
        &self,
        window: &AnnexAIopWindow,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if !window.local_lcr_present {
            report.push(annex_a_iop_error(
                "annex-a/lcr-required-for-iop",
                offset,
                "Annex A Table A.4: interoperability point 1 with more than one embedded layer \
                 requires a local OBU_LAYER_CONFIGURATION_RECORD, but none is present in the coded \
                 video sequence"
                    .to_owned(),
            ));
        }
    }
}
