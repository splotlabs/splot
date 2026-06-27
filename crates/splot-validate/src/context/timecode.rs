// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Timecode metadata consistency checks.

use super::*;

/// One observed `metadata_timecode()` unit's n_frames within its
/// coded-video-sequence scope (AV2 § 6.16.7), kept so a content interpretation that
/// arrives *after* the timecode (and establishes its `ci_timing_info_present_flag` /
/// timing) can re-evaluate the n_frames bound — the same arrival-order ambiguity the
/// § 6.16.10 scan-type / CI pairing handles (see [`ScanTypeObservation`]).
#[derive(Debug)]
pub(super) struct TimecodeObservation {
    /// The observed `n_frames` value (AV2 § 6.16.7, `f(9)`).
    pub(super) n_frames: u16,
    /// Source byte offset of the carrying metadata OBU (the diagnostic anchor — the
    /// offending timecode metadata OBU).
    pub(super) offset: ByteOffset,
    /// Temporal unit ([`CvsTracker::tu_index`]) of the observation, for the exact
    /// § 7.3.6 CVS scoping and the § 7.3.8.11 CI-parameter epoch filter.
    pub(super) tu_index: u64,
    /// The carrying OBU's `obu_xlayer_id` ([`GLOBAL_XLAYER_ID`] for a global OBU),
    /// used by the § 7.3.6 pruning when the unit's targeting is not derivable (finding
    /// 2 / finding 4).
    pub(super) scope_xlayer: ExtendedLayerId,
    /// The unit's § 6.16.3 layer targeting, when derivable from the bitstream
    /// (finding 4): the n_frames bound pairs this timecode only with a content
    /// interpretation OBU for a layer it targets (see
    /// [`HdrAssociation::associated_with_ci`]). `None` when the targeting is not
    /// bitstream-derivable (LAYER_UNSPECIFIED, etc., see [`derive_hdr_association`]),
    /// in which case the n_frames bound compares NOTHING (the spec leaves the layer
    /// association unspecified, so no CI's rate binds this timecode — see
    /// [`timecode_ci_in_scope`]).
    pub(super) targeting: Option<HdrAssociation>,
    /// The content-interpretation identities `(obu_xlayer_id, obu_mlayer_id)` whose
    /// n_frames bound this observation already paired-and-emitted *eagerly* against, in
    /// its OWN temporal unit, at observation time (round-7 finding 2). A CI key lands
    /// here when, at [`ValidatorContext::record_metadata_timecode_state`], that
    /// already-recorded in-scope CI in this temporal unit decided the bound and the
    /// diagnostic was emitted (not deferred) — i.e. an identical CI was re-sent BEFORE
    /// the timecode in the same § 7.3.8.11 RAP temporal unit. The § 7.3.8.11 RAP re-pair
    /// ([`ValidatorContext::repair_post_rap_ci_pairings`]) skips only the
    /// `(observation, CI)` *pairs* recorded here, not the whole observation: a multi-layer
    /// stream can pair one observation with several CIs in opposite orderings relative to
    /// the metadata, so an eager emission against one CI must not suppress the re-pair of
    /// a different CI whose eager pairing was DEFERRED against a stale pre-RAP record (and
    /// dropped at the RAP). The set is empty for an observation that emitted nothing
    /// eagerly, and re-pairing covers every not-yet-emitted post-epoch pairing.
    pub(super) eagerly_emitted: BTreeSet<ContentInterpretationKey>,
}

impl TimecodeObservation {
    /// Whether this observation belongs to the coded video sequence of extended layer
    /// `xlayer` — i.e. a § 7.3.6 CVS restart for `xlayer` should drop it (finding 2).
    /// A derivable targeting decides it exactly (the layers the timecode describes); an
    /// underivable targeting (which compares nothing for the bound) falls back to the
    /// carrying `obu_xlayer_id` scope, with a global carrying scope touching every
    /// layer (the documented harmless any-CLK approximation for an inert observation).
    pub(super) fn belongs_to_cvs_of(&self, xlayer: ExtendedLayerId) -> bool {
        match &self.targeting {
            Some(association) => association.touches_xlayer(xlayer),
            None => self.scope_xlayer.is_global() || self.scope_xlayer == xlayer,
        }
    }
}

/// An entry of the § 6.16.7 inference-presence chain, keyed in
/// [`TimecodeCvsState::inference`] by the carrying OBU's `(obu_xlayer_id,
/// obu_mlayer_id)`: the previous set's literal field presence, the temporal unit
/// that set was carried in, and that set's § 6.16.3 targeting.
#[derive(Debug, Clone)]
pub(super) struct TimecodeInferenceEntry {
    /// The previous set's literally-coded field presence (no OR with any inferred
    /// predecessor state — see the chain population in
    /// [`ValidatorContext::record_metadata_timecode_state`]).
    pub(super) presence: TimecodeFieldPresence,
    /// The temporal unit the previous set was carried in, so the § 7.3.6 CVS
    /// boundary can tell an intra-CVS predecessor (same/later temporal unit) from
    /// one that belongs to the ending coded video sequence (earlier temporal unit).
    pub(super) prev_tu: u64,
    /// The carrying OBU's `obu_xlayer_id` ([`GLOBAL_XLAYER_ID`] for a global OBU)
    /// of the set that wrote this entry — the fallback CVS scope when its targeting
    /// is not bitstream-derivable.
    pub(super) scope_xlayer: ExtendedLayerId,
    /// The previous set's § 6.16.3 layer targeting, when derivable from the
    /// bitstream (round-7 finding 1). The chain entry is reset on a § 7.3.6 CLK only
    /// when that CLK restarts the coded video sequence of a layer the previous set
    /// actually targets, mirroring [`TimecodeObservation::belongs_to_cvs_of`] and
    /// [`PendingTimecodeInference::belongs_to_cvs_of`] — so a global `LAYER_VALUES`
    /// chain aimed at one extended layer survives a CLK for an unrelated layer rather
    /// than dropping on every CLK. `None` falls back to the carrying `obu_xlayer_id`
    /// scope (a global carrying scope touching every layer, the documented any-CLK
    /// approximation).
    pub(super) targeting: Option<HdrAssociation>,
}

impl TimecodeInferenceEntry {
    /// Whether a § 7.3.6 CVS restart for extended layer `xlayer` detaches this chain
    /// entry's previous set — the same target-aware test as
    /// [`TimecodeObservation::belongs_to_cvs_of`] and
    /// [`PendingTimecodeInference::belongs_to_cvs_of`] (round-7 finding 1). A
    /// derivable targeting decides it exactly (the layers the previous set
    /// describes); an underivable targeting falls back to the carrying
    /// `obu_xlayer_id` scope, with a global carrying scope touching every layer (the
    /// documented harmless any-CLK approximation).
    pub(super) fn belongs_to_cvs_of(&self, xlayer: ExtendedLayerId) -> bool {
        match &self.targeting {
            Some(association) => association.touches_xlayer(xlayer),
            None => self.scope_xlayer.is_global() || self.scope_xlayer == xlayer,
        }
    }
}

/// Per coded-video-sequence-scope timecode state (AV2 § 6.16.7).
///
/// Two § 6.16.7 facts are decidable from metadata alone and tracked here, each with
/// the keying the per-layer § 6.16.3 semantics demand:
///
/// - **Inference-presence** ([`inference`], the mirror's "When seconds_value
///   \[minutes_value, hours_value\] is not present, its value is inferred to be equal
///   to the value of \[that element\] for the previous set of clock timestamp syntax
///   elements **in decoding order**, and it is required that such a previous
///   \[element\] shall have been present",
///   `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-7`, lines
///   3873-3893). The chain is keyed by the carrying OBU's concrete
///   `(obu_xlayer_id, obu_mlayer_id)` (finding 3): § 6.16.3 marks
///   METADATA_TYPE_TIMECODE as layer-specific (Table 6.17, "Y"), so a timecode on
///   embedded layer `(x, m0)` is NOT the "previous set" of one on `(x, m1)` and must
///   not seed its inference. For a timecode whose targeting is unspecified
///   (`LAYER_UNSPECIFIED`, § 6.16.3 lines 3520-3521: "does not specify to what layers
///   the metadata applies to"), the chain still keys by the carrying OBU's own
///   `(obu_xlayer_id, obu_mlayer_id)` — the only concrete scope the bitstream pins
///   down (finding 4, documented sound choice: the "previous set in decoding order"
///   is read as the previous set carried at the same physical stream scope, which is
///   always derivable and never compares across unrelated targets).
/// - **n_frames bound re-check** ([`observations`]): observed timecodes' n_frames,
///   kept so a later content interpretation can re-evaluate the bound (the eager
///   metadata-time direction reads the already-stored CI timing). Each observation
///   carries its carrying-OBU `obu_xlayer_id` scope and its § 6.16.3 `targeting`, so
///   the § 7.3.6 per-extended-layer CVS pruning drops an observation only when a CLK
///   restarts the coded video sequence of a layer the observation actually targets
///   (finding 2 — a CLK for one extended layer no longer prunes a global-bucket
///   observation aimed at another).
///
/// Both facts reset at the § 7.3.6 per-extended-layer CVS boundary (a CLK starts a
/// new coded video sequence, breaking the decoding-order inference chain) via the
/// [`ValidatorContext::prune_timecode_scope`] call sites in
/// [`ValidatorContext::start_cvs_for_xlayer`].
#[derive(Debug, Default)]
pub(super) struct TimecodeCvsState {
    /// Inference-presence state per carrying-OBU `(obu_xlayer_id, obu_mlayer_id)`:
    /// the previous-set field presence, the temporal unit of that previous set, and
    /// the § 6.16.3 targeting of the set that wrote it (finding 3). `None`-keyed
    /// entries do not exist — every timecode has a concrete carrying scope. The
    /// temporal unit lets the § 7.3.6 CVS boundary reset the chain (a previous set
    /// from an earlier temporal unit belongs to the ending coded video sequence and
    /// no longer seeds the new one); the targeting makes that reset target-aware so a
    /// CLK for an unrelated extended layer no longer drops a global `LAYER_VALUES`
    /// chain aimed at a different layer (round-7 finding 1, mirroring
    /// [`TimecodeObservation::belongs_to_cvs_of`]).
    pub(super) inference: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), TimecodeInferenceEntry>,
    /// n_frames observations, flat and self-describing (each carries its
    /// carrying-`obu_xlayer_id` scope, § 7.3.8.11 epoch tu, and § 6.16.3 targeting),
    /// for the CI-after re-check of the n_frames bound and the target-aware § 7.3.6
    /// pruning (finding 2).
    pub(super) observations: Vec<TimecodeObservation>,
    /// Inference-presence diagnostics whose firing depends on whether a § 7.3.6
    /// CVS boundary is crossed later in the current temporal unit (AV2 § 6.16.7).
    ///
    /// A timecode that omits a field, seeded only by a *present* value from a
    /// previous set in an **earlier** temporal unit, sits in the same coded video
    /// sequence as that seed *unless* a CLK later in this temporal unit starts a
    /// new coded video sequence (§ 7.3.6: the whole temporal unit containing a CLK
    /// joins the new sequence). If that happens, the seed belongs to the ending
    /// sequence, no source remains for the inference, and the diagnostic fires; if
    /// the temporal unit completes with no such boundary, the seed is intra-CVS and
    /// the field infers cleanly. The decision is therefore deferred to the temporal
    /// unit's resolution: [`ValidatorContext::emit_pending_timecode_inference`]
    /// emits matching entries on a CVS start, and
    /// [`ValidatorContext::drop_pending_timecode_inference`] drops the survivors
    /// silently at the temporal-unit flush. This mirrors the
    /// [`PendingPolarity::PreCvs`] machinery, but is kept dedicated to the timecode
    /// state because it keys the carrying OBU's exact `(obu_xlayer_id,
    /// obu_mlayer_id)`, which the per-layer [`CvsTracker::defer_pre_cvs`] path does
    /// not model.
    pub(super) pending_inference: Vec<PendingTimecodeInference>,
}

/// A § 6.16.7 inference-presence diagnostic deferred until the current temporal
/// unit's § 7.3.6 CVS scope is resolved (see [`TimecodeCvsState::pending_inference`]).
#[derive(Debug)]
pub(super) struct PendingTimecodeInference {
    /// The carrying OBU's `obu_xlayer_id` of the omitting timecode ([`GLOBAL_XLAYER_ID`]
    /// for a global OBU), the fallback CVS scope when the targeting is not derivable.
    pub(super) xlayer: ExtendedLayerId,
    /// The omitting timecode's § 6.16.3 layer targeting, when derivable from the
    /// bitstream (finding 2). The deferred diagnostic fires only when a CLK restarts the
    /// coded video sequence of a layer this timecode actually targets — mirroring
    /// [`TimecodeObservation::belongs_to_cvs_of`] — so a global `LAYER_VALUES` timecode
    /// aimed at one extended layer is left pending by an unrelated layer's CLK rather
    /// than firing on every CLK. `None` falls back to the carrying `obu_xlayer_id`
    /// scope (a global carrying scope touching every layer, the documented any-CLK
    /// approximation).
    pub(super) targeting: Option<HdrAssociation>,
    /// The inference-without-previous diagnostic to emit if the seed turns out to
    /// belong to the ending coded video sequence.
    pub(super) diagnostic: Diagnostic,
}

impl PendingTimecodeInference {
    /// Whether a § 7.3.6 CVS restart for extended layer `xlayer` detaches this
    /// deferred timecode's earlier-temporal-unit inference seed — the same
    /// target-aware test as [`TimecodeObservation::belongs_to_cvs_of`] (finding 2). A
    /// derivable targeting decides it exactly (the layers the timecode describes); an
    /// underivable targeting falls back to the carrying `obu_xlayer_id` scope, with a
    /// global carrying scope touching every layer (the documented harmless any-CLK
    /// approximation, matching the eager-fire path of [`Self`] for a missing seed).
    pub(super) fn belongs_to_cvs_of(&self, xlayer: ExtendedLayerId) -> bool {
        match &self.targeting {
            Some(association) => association.touches_xlayer(xlayer),
            None => self.xlayer.is_global() || self.xlayer == xlayer,
        }
    }
}

/// Whether each clock-timestamp field carried a *present* value in a
/// `metadata_timecode()` set (AV2 § 6.16.7). A field present in the previous set in
/// decoding order satisfies the inference's "such a previous \[element\] shall have
/// been present" requirement for the next set that omits it.
#[derive(Debug, Clone, Copy)]
pub(super) struct TimecodeFieldPresence {
    pub(super) seconds: bool,
    pub(super) minutes: bool,
    pub(super) hours: bool,
}

impl TimecodeFieldPresence {
    /// Records the present fields of a parsed timecode (each `Option` is `Some` when
    /// the field was coded, per the § 5.17.7 presence flags).
    pub(super) fn of(timecode: &MetadataTimecode) -> Self {
        Self {
            seconds: timecode.seconds_value.is_some(),
            minutes: timecode.minutes_value.is_some(),
            hours: timecode.hours_value.is_some(),
        }
    }

    /// Whether the named clock-timestamp field (`"seconds_value"`,
    /// `"minutes_value"`, or `"hours_value"`) carried a present value.
    pub(super) fn field(self, name: &str) -> bool {
        match name {
            "seconds_value" => self.seconds,
            "minutes_value" => self.minutes,
            "hours_value" => self.hours,
            _ => false,
        }
    }
}

pub(super) fn timecode_ci_in_scope(
    targeting: Option<&HdrAssociation>,
    ci_xlayer: ExtendedLayerId,
    ci_mlayer: EmbeddedLayerId,
) -> bool {
    match targeting {
        Some(association) => association.associated_with_ci(ci_xlayer, ci_mlayer),
        // Underivable targeting (LAYER_UNSPECIFIED, ...): the spec does not say which
        // layers the metadata applies to, so no CI's rate can be soundly bound to it.
        None => false,
    }
}

/// `maxPicPerSecond` for the § 6.16.7 n_frames bound: `ceil(time_scale /
/// TicksPerPicture)`, where `TicksPerPicture` equals
/// `(num_ticks_per_picture_minus_1 + 1) * num_units_in_display_tick` when
/// `equal_picture_interval`, else `num_units_in_display_tick` (mirror lines
/// 3833-3837, 3865-3867). Both `time_scale` and `num_units_in_display_tick` are
/// guaranteed `> 0` by the § 6.4.12 timing-info parser, so `TicksPerPicture >= 1`,
/// the result is `>= 1`, and the division never panics.
pub(super) fn max_pic_per_second(timing: &TimingInfo) -> u64 {
    let ticks_per_picture = if timing.equal_picture_interval {
        // num_ticks_per_picture_minus_1 is Some when equal_picture_interval; treat an
        // unexpected None as 0 (TicksPerPicture == num_units_in_display_tick) — a
        // conservative fallback that never panics and never under-counts the bound.
        let ticks_minus_1 = u64::from(timing.num_ticks_per_picture_minus_1.unwrap_or(0));
        (ticks_minus_1 + 1) * u64::from(timing.num_units_in_display_tick)
    } else {
        u64::from(timing.num_units_in_display_tick)
    };
    // ceil(time_scale / ticks_per_picture) for positive integers.
    let time_scale = u64::from(timing.time_scale);
    time_scale.div_ceil(ticks_per_picture)
}

/// Builds the § 6.16.7 n_frames-exceeds-rate diagnostic
/// (`metadata/timecode-n-frames-exceeds-rate`), anchored at the offending timecode
/// metadata OBU.
pub(super) fn timecode_n_frames_error(
    n_frames: u16,
    max_pic_per_second: u64,
    ci_xlayer: ExtendedLayerId,
    ci_mlayer: EmbeddedLayerId,
    ci_offset: ByteOffset,
    metadata_offset: ByteOffset,
    at: ByteOffset,
) -> Diagnostic {
    Diagnostic::error(
        "metadata/timecode-n-frames-exceeds-rate",
        format!(
            "n_frames {n_frames} (timecode metadata at byte {metadata_offset}) must be less than \
             maxPicPerSecond {max_pic_per_second} = ceil(time_scale / TicksPerPicture), which the \
             content interpretation timing_info() for obu_xlayer_id {} / obu_mlayer_id {} (at byte \
             {ci_offset}) establishes with ci_timing_info_present_flag 1",
            ci_xlayer.get(),
            ci_mlayer.get(),
        ),
    )
    .with_spec_section("6.16.7")
    .with_byte_offset(at)
}

impl ValidatorContext {
    /// Prunes the § 6.16.7 timecode state at a § 7.3.6 CVS boundary: a CLK for
    /// `clk_xlayer` starts a new coded video sequence for THAT extended layer at
    /// `keep_from_tu` (mirror `07-decoding-process.md` lines 604-606, "A new coded
    /// video sequence for an extended layer is defined to start ... in the coded
    /// extended layer unit corresponding to the extended layer").
    ///
    /// § 7.3.6 CVS boundaries are per extended layer (finding 2), so this prunes only
    /// the state whose coded video sequence actually restarted:
    ///
    /// - **n_frames observations**: an observation belongs to the coded video sequences
    ///   of the extended layers it targets (its § 6.16.3 `targeting`), so it is dropped
    ///   only when `clk_xlayer` is one of them and it predates `keep_from_tu`. An
    ///   observation whose targeting is not bitstream-derivable (`None`) never fires the
    ///   bound (see [`timecode_ci_in_scope`]); it is keyed by its carrying
    ///   `obu_xlayer_id` scope and dropped when that scope's CVS restarts (a global
    ///   carrying scope keeps the documented any-CLK approximation, harmless because it
    ///   compares nothing). A global LAYER_VALUES observation aimed at extended layer 1
    ///   therefore survives a CLK for extended layer 0 and is still in scope for layer
    ///   1's later n_frames re-checks.
    /// - **inference chain**: each `(obu_xlayer_id, obu_mlayer_id)` entry whose previous
    ///   set both belongs to a coded video sequence `clk_xlayer` restarts — the
    ///   target-aware [`TimecodeInferenceEntry::belongs_to_cvs_of`] test, matching the
    ///   n_frames-observation pruning above (round-7 finding 1) — and predates
    ///   `keep_from_tu` is reset (the seed belongs to the ending coded video sequence; a
    ///   same-temporal-unit predecessor joined the new sequence and still seeds it). Pre-
    ///   fix the entry was dropped whenever its carrying `obu_xlayer_id` matched
    ///   `clk_xlayer` or was global, so a global `LAYER_VALUES` chain aimed at one
    ///   extended layer was reset by an unrelated layer's CLK; the targeting now spares
    ///   it, just as it does the matching observation and pending-inference entries.
    pub(super) fn prune_timecode_scope(&mut self, clk_xlayer: ExtendedLayerId, keep_from_tu: u64) {
        self.timecode.observations.retain(|observation| {
            // Keep observations at/after the boundary, and observations whose coded
            // video sequence did NOT restart at this CLK (the CLK is for a different
            // extended layer than any the observation belongs to).
            observation.tu_index >= keep_from_tu || !observation.belongs_to_cvs_of(clk_xlayer)
        });
        self.timecode.inference.retain(|_, entry| {
            // Keep entries whose previous set is at/after the boundary, and entries
            // whose coded video sequence did NOT restart at this CLK (the CLK is for a
            // different extended layer than any the previous set targets) — the same
            // target-aware test as the observation pruning above (round-7 finding 1),
            // replacing the pre-fix carrying-scope-only `xlayer == clk_xlayer ||
            // is_global` predicate that dropped a global LAYER_VALUES chain on any CLK.
            entry.prev_tu >= keep_from_tu || !entry.belongs_to_cvs_of(clk_xlayer)
        });
    }

    /// Emits the deferred § 6.16.7 inference-presence diagnostics whose seed now
    /// belongs to an ending coded video sequence because a CLK started a new coded
    /// video sequence for `xlayer` at this temporal unit (§ 7.3.6). A pending entry
    /// fires when the CLK restarts the coded video sequence of a layer the omitting
    /// timecode actually targets — the target-aware
    /// [`PendingTimecodeInference::belongs_to_cvs_of`] test, mirroring the
    /// observation pruning in [`Self::prune_timecode_scope`] (finding 2). § 7.3.6 CVS
    /// boundaries are per extended layer, so a CLK for one extended layer detaches the
    /// seed of a timecode carried on (or, for a global `LAYER_VALUES` timecode,
    /// targeting) that extended layer only; a CLK for an UNRELATED extended layer
    /// leaves a global timecode aimed at a different layer pending (pre-fix any global
    /// carrying scope fired on every CLK, a false positive). A global timecode with no
    /// derivable targeting keeps the documented any-CLK approximation (its
    /// `obu_xlayer_id` is global). Survivors are left for
    /// [`Self::drop_pending_timecode_inference`] at the temporal-unit flush. See
    /// [`TimecodeCvsState::pending_inference`].
    pub(super) fn emit_pending_timecode_inference(
        &mut self,
        xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        let mut retained = Vec::with_capacity(self.timecode.pending_inference.len());
        for entry in std::mem::take(&mut self.timecode.pending_inference) {
            if entry.belongs_to_cvs_of(xlayer) {
                report.push(entry.diagnostic);
            } else {
                retained.push(entry);
            }
        }
        self.timecode.pending_inference = retained;
    }

    /// Drops the deferred § 6.16.7 inference-presence diagnostics that survived the
    /// just-completed temporal unit with no CVS boundary: their earlier-temporal-unit
    /// seed stayed in the same coded video sequence (§ 7.3.6), so the field infers
    /// cleanly and the diagnostic is silently discarded. See
    /// [`TimecodeCvsState::pending_inference`].
    pub(super) fn drop_pending_timecode_inference(&mut self) {
        self.timecode.pending_inference.clear();
    }

    /// Checks the locally-decidable § 6.16.7 timecode rules for one
    /// `metadata_timecode()` unit
    /// (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-7`):
    ///
    /// 1. **Inference-presence** (lines 3873-3893): for each of `seconds_value`,
    ///    `minutes_value`, `hours_value` that is *not present* in this set, the mirror
    ///    infers its value from "the previous set of clock timestamp syntax elements in
    ///    decoding order, and it is required that such a previous \[element\] shall have
    ///    been present". When no previous set in this CVS scope carried that field, the
    ///    inference has no source, so `metadata/timecode-inferred-without-previous`
    ///    (error) is emitted naming the field.
    ///
    ///    **Interpretation choice — literal "present" reading (documented):** the
    ///    mirror requires, of an omitted field, that "such a previous seconds_value
    ///    \[minutes_value, hours_value\] shall have been present" (lines 3873-3893,
    ///    `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-7`). "Present"
    ///    is read literally as the syntax element having been *coded in the immediate
    ///    predecessor set in decoding order* — i.e. the previous set's own presence
    ///    flags. An *inferred* value in the previous set therefore does NOT make the
    ///    element "present" for the next set: a set that omits a field, followed by
    ///    another set that also omits it, fires the diagnostic on the second omission
    ///    too (the chain never seeds itself from an inference). The lenient chained-
    ///    inference reading — where the first omitted-but-inferred value would then
    ///    count as "present" and satisfy the next omitting set — was rejected for
    ///    lacking textual support: the sentence speaks of the element having "been
    ///    present", not of a value having been *established* (whether by presence or by
    ///    inference). AVM differential testing may revisit this if the reference
    ///    decoder treats a propagated inferred value as satisfying the requirement.
    ///    [`TimecodeFieldPresence`] therefore records each set's own literal field
    ///    presence only, never an OR with the predecessor's inferred state.
    /// 2. **n_frames bound** (lines 3865-3867): "When ci_timing_info_present_flag is
    ///    equal to 1, n_frames shall be less than maxPicPerSecond". The
    ///    `ci_timing_info_present_flag` is the content interpretation OBU's flag
    ///    associated with the timecode's extended layer (annex-e-decoder-model.md line
    ///    293: "ci_timing_info_present_flag equal to 1 in the content interpretation OBU
    ///    associated with this extended layer"); a present `timing_info()` in an
    ///    in-scope content interpretation is exactly that flag set. The bound is checked
    ///    against every in-scope content-interpretation record at/after the timecode
    ///    layer's § 7.3.8.11 CI-parameter epoch (the same epoch filter the § 6.16.10
    ///    scan-type / CI pairing applies); a content interpretation arriving *after* the
    ///    timecode re-evaluates instead (see
    ///    [`ValidatorContext::recheck_timecode_n_frames_after_ci`]). "In scope" is the
    ///    unit's § 6.16.3 layer targeting (`targeting`): when the targeting is
    ///    bitstream-derivable, only the CIs of the layers the timecode describes pair
    ///    with it, so a global `LAYER_VALUES` timecode naming only some layers does not
    ///    pair with an untargeted layer's CI (finding 4, see [`timecode_ci_in_scope`]);
    ///    an underivable targeting falls back to the `obu_xlayer_id` scope.
    ///
    /// Both diagnostics anchor at the offending timecode metadata OBU. These are
    /// metadata-local facts, so they are not gated by [`ValidationOptions`]'
    /// external-HLS mode.
    pub(super) fn check_timecode_consistency(
        &mut self,
        obu: &ObuEnvelope<'_>,
        timecode: &MetadataTimecode,
        targeting: Option<HdrAssociation>,
        report: &mut ValidationReport,
    ) {
        let scope_xlayer = obu.header.extended_layer_id;
        // The inference chain is keyed by the carrying OBU's concrete
        // `(obu_xlayer_id, obu_mlayer_id)` (finding 3): METADATA_TYPE_TIMECODE is
        // layer-specific (§ 6.16.3 Table 6.17), so a timecode on embedded layer
        // `(x, m0)` is not the "previous set in decoding order" of one on `(x, m1)` and
        // must not seed its inference. For unspecified targeting the carrying OBU's own
        // pair is still the soundest concrete scope (finding 4, documented).
        // TODO(spec: AV2-5.17.7-METADATA-TIMECODE): a group-form LAYER_VALUES timecode
        // carries GLOBAL_XLAYER_ID, so two groups targeting disjoint layer sets share
        // this carrying-pair key and a present value from one set can seed an omitted
        // field aimed at another -- a known false-negative (never a false positive);
        // keying chains per derived target set would close it.
        let inference_key = (scope_xlayer, obu.header.embedded_layer_id);
        let tu_index = self.cvs.tu_index;

        // 1. Inference-presence (decoding-order, per carrying-layer scope). The
        // "previous set in decoding order" is the immediate predecessor for THIS
        // carrying layer; its presence is read literally (round-1 finding): an inferred
        // value in the predecessor does NOT make the element present, so the chain never
        // seeds itself from an inference.
        let prev = self.timecode.inference.get(&inference_key).cloned();
        // Record this set's own literal field presence as the new previous set (no OR
        // with the predecessor's inferred state), carrying its § 6.16.3 targeting and
        // carrying scope so the § 7.3.6 chain reset is target-aware (round-7 finding 1).
        // Also append the n_frames observation (likewise carrying the targeting and the
        // carrying scope) for the CI-after re-check and the target-aware § 7.3.6 pruning.
        let this = TimecodeFieldPresence::of(timecode);
        self.timecode.inference.insert(
            inference_key,
            TimecodeInferenceEntry {
                presence: this,
                prev_tu: tu_index,
                scope_xlayer,
                targeting: targeting.clone(),
            },
        );
        // For each absent field, a previous *present* value (literally coded in the
        // immediate predecessor set in decoding order) is required.
        for (present, field) in [
            (timecode.seconds_value.is_some(), "seconds_value"),
            (timecode.minutes_value.is_some(), "minutes_value"),
            (timecode.hours_value.is_some(), "hours_value"),
        ] {
            if present {
                continue;
            }
            let diagnostic = Diagnostic::error(
                "metadata/timecode-inferred-without-previous",
                format!(
                    "{field} is not present and is inferred from the previous set of clock \
                     timestamp syntax elements in decoding order, but no previous timecode \
                     in the coded video sequence carried a present {field}"
                ),
            )
            .with_spec_section("6.16.7")
            .with_byte_offset(obu.offset);
            match &prev {
                // No previous present value in scope — the inference has no source
                // regardless of any later § 7.3.6 boundary, so fire eagerly.
                None => report.push(diagnostic),
                Some(entry) if !entry.presence.field(field) => report.push(diagnostic),
                // A present predecessor in THIS temporal unit always shares the coded
                // video sequence (§ 7.3.6 sequences start at temporal units, never
                // inside one), so it seeds the inference cleanly — silent.
                Some(entry) if entry.prev_tu == tu_index => {}
                // A present predecessor in an EARLIER temporal unit seeds only if no CLK
                // later in this temporal unit starts a new coded video sequence
                // (finding 2 / § 7.3.6). Defer the decision to the temporal unit's
                // resolution: emit on a matching CVS start, drop silently otherwise.
                Some(_) => self
                    .timecode
                    .pending_inference
                    .push(PendingTimecodeInference {
                        xlayer: scope_xlayer,
                        // Carry the § 6.16.3 targeting so emit_pending_timecode_inference
                        // fires only on a CLK for a layer this timecode targets (finding
                        // 2), mirroring the n_frames observation's target-aware pruning.
                        targeting: targeting.clone(),
                        diagnostic,
                    }),
            }
        }

        // 2. n_frames bound against the already-observed in-scope content
        // interpretations (a later CI re-evaluates via
        // recheck_timecode_n_frames_after_ci). The § 6.16.3 targeting scopes the
        // pairing to the CIs of the layers this timecode describes; an underivable
        // targeting compares nothing (finding 4, see timecode_ci_in_scope).
        //
        // `eagerly_emitted` collects the CI identities whose same-temporal-unit in-scope
        // bound was decided HERE and emitted (not deferred) — i.e. an identical CI was
        // re-sent BEFORE this timecode in the same § 7.3.8.11 RAP temporal unit. The RAP
        // re-pair (repair_post_rap_ci_pairings) skips exactly those `(observation, CI)`
        // pairs so the diagnostic is not emitted twice (round-7 finding 2), while still
        // re-pairing any OTHER CI for this observation. A pairing DEFERRED against an
        // earlier-temporal-unit (stale pre-RAP) CI does NOT enter the set: that deferred
        // diagnostic is dropped at the RAP, so the re-pair must still cover it.
        let mut eagerly_emitted = BTreeSet::new();
        for ((ci_xlayer, ci_mlayer), record) in &self.content_interpretations {
            if !timecode_ci_in_scope(targeting.as_ref(), *ci_xlayer, *ci_mlayer) {
                continue;
            }
            if record.tu_index < self.ci_rap_epoch(*ci_xlayer) {
                continue;
            }
            let Some(timing) = record.content.timing_info else {
                continue;
            };
            let max_pic = max_pic_per_second(&timing);
            if u64::from(timecode.n_frames) >= max_pic {
                let diagnostic = timecode_n_frames_error(
                    timecode.n_frames,
                    max_pic,
                    *ci_xlayer,
                    *ci_mlayer,
                    record.offset,
                    obu.offset,
                    obu.offset,
                );
                // defer_or_emit emits eagerly iff the CI is in this temporal unit; a
                // same-temporal-unit emission is the round-7 finding 2 case to skip in
                // the RAP re-pair, keyed by the CI's identity so only this exact pairing
                // is skipped.
                if record.tu_index == tu_index {
                    eagerly_emitted.insert((*ci_xlayer, *ci_mlayer));
                }
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
            }
        }

        // Append the n_frames observation (carrying the § 6.16.3 targeting and the
        // carrying scope) for the CI-after re-check and the target-aware § 7.3.6 pruning,
        // tagged with whether its bound was already emitted eagerly above (round-7
        // finding 2). Pushed after the loop so `eagerly_emitted` is final.
        self.timecode.observations.push(TimecodeObservation {
            n_frames: timecode.n_frames,
            offset: obu.offset,
            tu_index,
            scope_xlayer,
            targeting,
            eagerly_emitted,
        });
    }

    /// Re-evaluates the § 6.16.7 n_frames bound of the stored timecode observations
    /// against a newly observed content-interpretation record — the content
    /// interpretation may arrive after the timecode metadata it constrains (the same
    /// arrival-order handling as
    /// [`ValidatorContext::recheck_scan_type_after_ci`]). Only a content
    /// interpretation with a present `timing_info()` (i.e.
    /// `ci_timing_info_present_flag == 1`) establishes the bound; observations from a
    /// temporal unit before the CI layer's § 7.3.8.11 random access point are skipped
    /// (their pictures' content interpretation parameters belong to the previous
    /// epoch). The diagnostic anchors at the offending timecode metadata OBU.
    ///
    /// `repair` flags the call as the § 7.3.8.11 RAP re-pair from
    /// [`Self::repair_post_rap_ci_pairings`] (round-7 finding 2). The eager
    /// CI-after-timecode caller passes `false`; the RAP re-pair passes `true`, which
    /// skips an `(observation, CI)` pair that already paired-and-emitted eagerly against
    /// this in-scope same-temporal-unit CI at observation time (the
    /// [`TimecodeObservation::eagerly_emitted`] set contains the CI's identity —
    /// populated when an identical CI was already recorded BEFORE the observation in the
    /// same RAP temporal unit, so the eager observation-time pairing emitted directly).
    /// Re-pairing such a pair would duplicate the diagnostic; the skip is per-CI, so a
    /// DIFFERENT CI for the same observation — whose eager pairing was instead DEFERRED
    /// against a stale pre-RAP CI (and dropped by `observe_ci_rap` at the RAP) — still
    /// gets re-paired.
    pub(super) fn recheck_timecode_n_frames_after_ci(
        &mut self,
        ci_xlayer: ExtendedLayerId,
        ci_mlayer: EmbeddedLayerId,
        content: &ContentInterpretation,
        ci_offset: ByteOffset,
        repair: bool,
        report: &mut ValidationReport,
    ) {
        let Some(timing) = content.timing_info else {
            return;
        };
        let max_pic = max_pic_per_second(&timing);
        let epoch = self.ci_rap_epoch(ci_xlayer);
        // The observations are a single flat list now; the § 6.16.3 targeting decides
        // which of them this CI's layer can bind, so an untargeted layer's CI cannot
        // pair with an observation aimed elsewhere, and an underivable-targeting
        // observation binds to nothing (finding 4, see timecode_ci_in_scope). The
        // § 7.3.8.11 epoch filter (tu_index >= epoch) drops observations whose pictures
        // belong to a previous content-interpretation-parameter epoch. The RAP re-pair
        // additionally skips an observation already paired-and-emitted eagerly against
        // THIS CI at observation time (round-7 finding 2), keyed by the CI's identity so
        // an eager emission against a different CI does not suppress this one. Snapshot
        // first to avoid borrowing self twice.
        let violations: Vec<(u16, ByteOffset, u64)> = self
            .timecode
            .observations
            .iter()
            .filter(|observation| {
                observation.tu_index >= epoch
                    && !(repair
                        && observation
                            .eagerly_emitted
                            .contains(&(ci_xlayer, ci_mlayer)))
                    && u64::from(observation.n_frames) >= max_pic
                    && timecode_ci_in_scope(observation.targeting.as_ref(), ci_xlayer, ci_mlayer)
            })
            .map(|observation| {
                (
                    observation.n_frames,
                    observation.offset,
                    observation.tu_index,
                )
            })
            .collect();
        for (n_frames, metadata_offset, observation_tu) in violations {
            let diagnostic = timecode_n_frames_error(
                n_frames,
                max_pic,
                ci_xlayer,
                ci_mlayer,
                ci_offset,
                metadata_offset,
                // Anchor at the offending timecode metadata OBU (the message also
                // names it), not the CI OBU.
                metadata_offset,
            );
            self.cvs
                .defer_or_emit(ci_xlayer, observation_tu, diagnostic, report);
        }
    }
}
