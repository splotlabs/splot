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
    /// used by the § 7.3.6 pruning when the unit's targeting is not derivable.
    pub(super) scope_xlayer: ExtendedLayerId,
    /// The unit's § 6.16.3 layer targeting, when derivable from the bitstream.
    /// The n_frames bound pairs this timecode only with a content
    /// interpretation OBU for a layer it targets (see
    /// [`HdrAssociation::includes_embedded_pair`]). `None` when the targeting is not
    /// bitstream-derivable (LAYER_UNSPECIFIED, etc., see [`derive_hdr_association`]),
    /// in which case the n_frames bound compares NOTHING (the spec leaves the layer
    /// association unspecified, so no CI's rate binds this timecode — see
    /// [`timecode_ci_in_scope`]).
    pub(super) targeting: Option<HdrAssociation>,
    /// CI identities already emitted against eagerly in this observation's own TU.
    /// RAP repair skips these pairs only, allowing other CIs whose stale pre-RAP
    /// comparison was deferred and dropped to be re-paired.
    pub(super) eagerly_emitted: BTreeSet<ContentInterpretationKey>,
}

impl TimecodeObservation {
    /// Whether this observation belongs to the coded video sequence of extended layer
    /// `xlayer` — i.e. a § 7.3.6 CVS restart for `xlayer` should drop it.
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
#[derive(Debug)]
pub(super) struct TimecodeInferenceEntry {
    /// The previous set's literally-coded field presence (no OR with any inferred
    /// predecessor state — see the chain population in
    /// [`ValidatorContext::check_timecode_consistency`]).
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
    /// bitstream. The chain entry is reset on a § 7.3.6 CLK only
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
    /// [`PendingTimecodeInference::belongs_to_cvs_of`]. A
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

/// Timecode inference and rate checks (§ 6.16.7,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-7`).
/// Inference keys the carrying (xlayer, mlayer); it never crosses embedded layers.
/// For unspecified targeting this is the only concrete stream scope available.
/// Rate observations retain targeting for late CI pairing. CVS restarts prune
/// only targeted layers, using carrying scope when targeting is underivable.
#[derive(Debug, Default)]
pub(super) struct TimecodeCvsState {
    /// Previous literal field presence and targeting per carrying (xlayer, mlayer).
    /// The previous TU distinguishes a same-TU seed from one a later CLK invalidates.
    pub(super) inference: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), TimecodeInferenceEntry>,
    /// n_frames observations, flat and self-describing (each carries its
    /// carrying-`obu_xlayer_id` scope, § 7.3.8.11 epoch tu, and § 6.16.3 targeting),
    /// for the CI-after re-check of the n_frames bound and the target-aware § 7.3.6
    /// pruning.
    pub(super) observations: Vec<TimecodeObservation>,
    /// Omissions seeded by an earlier-TU present value: emit only if a later CLK
    /// restarts a targeted CVS and invalidates that seed; otherwise discard at TU end.
    /// This needs exact targeting rather than CvsTracker's per-layer PreCvs key.
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
    /// bitstream. The deferred diagnostic fires only when a CLK restarts the
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
    /// target-aware test as [`TimecodeObservation::belongs_to_cvs_of`]. A
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
        Some(association) => association.includes_embedded_pair(ci_xlayer, ci_mlayer),
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
        let ticks_minus_1 = u64::from(timing.num_ticks_per_picture_minus_1.unwrap_or(0));
        (ticks_minus_1 + 1) * u64::from(timing.num_units_in_display_tick)
    } else {
        u64::from(timing.num_units_in_display_tick)
    };
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
    .with_byte_offset(metadata_offset)
}

impl ValidatorContext {
    /// Drops pre-boundary observations and inference seeds targeted by this CLK
    /// (§ 7.3.6). Same-TU seeds survive; unrelated-layer CLKs do not prune global
    /// LAYER_VALUES entries targeting elsewhere. Underivable targeting uses carrying
    /// scope, with global scope conservatively touching every layer.
    pub(super) fn prune_timecode_scope(&mut self, clk_xlayer: ExtendedLayerId, keep_from_tu: u64) {
        self.timecode.observations.retain(|observation| {
            observation.tu_index >= keep_from_tu || !observation.belongs_to_cvs_of(clk_xlayer)
        });
        self.timecode.inference.retain(|_, entry| {
            entry.prev_tu >= keep_from_tu || !entry.belongs_to_cvs_of(clk_xlayer)
        });
    }

    /// Emits pending omissions whose earlier-TU seed this layer's CLK invalidates.
    /// Target-aware matching preserves entries for unrelated layers; the TU flush
    /// discards survivors. Underivable targeting uses the carrying scope.
    pub(super) fn emit_pending_timecode_inference(
        &mut self,
        xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        for entry in self
            .timecode
            .pending_inference
            .extract_if(.., |entry| entry.belongs_to_cvs_of(xlayer))
        {
            report.push(entry.diagnostic);
        }
    }

    /// Drops the deferred § 6.16.7 inference-presence diagnostics that survived the
    /// just-completed temporal unit with no CVS boundary: their earlier-temporal-unit
    /// seed stayed in the same coded video sequence (§ 7.3.6), so the field infers
    /// cleanly and the diagnostic is silently discarded. See
    /// [`TimecodeCvsState::pending_inference`].
    pub(super) fn drop_pending_timecode_inference(&mut self) {
        self.timecode.pending_inference.clear();
    }

    /// Checks § 6.16.7 inference presence and n_frames < maxPicPerSecond.
    /// Presence means literally coded in the immediate predecessor, not inferred:
    /// consecutive omissions cannot seed each other. This interpretation follows
    /// "shall have been present" in the mirror; an AVM differential may revisit it.
    /// Rate checks use targeted CIs with timing at/after their § 7.3.8.11 epoch;
    /// underivable targeting establishes no rate bound. Late CIs recheck observations.
    /// Both diagnostics anchor at the timecode OBU and ignore external-HLS mode.
    pub(super) fn check_timecode_consistency(
        &mut self,
        obu: &ObuEnvelope<'_>,
        timecode: &MetadataTimecode,
        targeting: Option<HdrAssociation>,
        report: &mut ValidationReport,
    ) {
        let scope_xlayer = obu.header.extended_layer_id;
        // TODO(spec: AV2-5.17.7-METADATA-TIMECODE): a group-form LAYER_VALUES timecode
        let inference_key = (scope_xlayer, obu.header.embedded_layer_id);
        let tu_index = self.cvs.tu_index;

        let this = TimecodeFieldPresence::of(timecode);
        let prev = self.timecode.inference.insert(
            inference_key,
            TimecodeInferenceEntry {
                presence: this,
                prev_tu: tu_index,
                scope_xlayer,
                targeting: targeting.clone(),
            },
        );
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
                None => report.push(diagnostic),
                Some(entry) if !entry.presence.field(field) => report.push(diagnostic),
                Some(entry) if entry.prev_tu == tu_index => {}
                Some(_) => self
                    .timecode
                    .pending_inference
                    .push(PendingTimecodeInference {
                        xlayer: scope_xlayer,
                        targeting: targeting.clone(),
                        diagnostic,
                    }),
            }
        }

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
                );
                if record.tu_index == tu_index {
                    eagerly_emitted.insert((*ci_xlayer, *ci_mlayer));
                }
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
            }
        }

        self.timecode.observations.push(TimecodeObservation {
            n_frames: timecode.n_frames,
            offset: obu.offset,
            tu_index,
            scope_xlayer,
            targeting,
            eagerly_emitted,
        });
    }

    /// Rechecks rate bounds when CI arrives after timecode. Only targeted observations
    /// at/after the CI's RAP epoch participate; absent CI timing decides nothing.
    /// RAP repair skips pairs already in eagerly_emitted, not whole observations,
    /// so another CI whose stale comparison was dropped can still pair.
    /// Diagnostics anchor at the timecode metadata OBU.
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
        for observation in self.timecode.observations.iter().filter(|observation| {
            observation.tu_index >= epoch
                && !(repair
                    && observation
                        .eagerly_emitted
                        .contains(&(ci_xlayer, ci_mlayer)))
                && u64::from(observation.n_frames) >= max_pic
                && timecode_ci_in_scope(observation.targeting.as_ref(), ci_xlayer, ci_mlayer)
        }) {
            let diagnostic = timecode_n_frames_error(
                observation.n_frames,
                max_pic,
                ci_xlayer,
                ci_mlayer,
                ci_offset,
                observation.offset,
            );
            self.cvs
                .defer_or_emit(ci_xlayer, observation.tu_index, diagnostic, report);
        }
    }
}
