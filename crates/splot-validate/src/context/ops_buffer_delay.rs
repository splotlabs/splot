// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! OPS and sequence-header decoder buffer-delay checks.

use super::*;

/// Comparison key for the § 6.10.5 operating-point buffer-delay sum-constancy check:
/// the `(obu_xlayer_id, opsID, op)` triple Annex E binds the delays to
/// (`annex-e-decoder-model.md` lines 100–112).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct OpsBufferDelayKey {
    pub(super) xlayer: ExtendedLayerId,
    pub(super) ops_id: u8,
    pub(super) op_index: u8,
}

/// The boundary scope of one § 6.10.5 buffer-delay observation: the per-extended-layer
/// CVS epoch, the per-extended-layer § 6.10.1 effective reset generation (global resets
/// plus that layer's local resets), and the per-OPS targeted-reset generation. Two
/// observations share the same scope — and so are subject to the error tier rather than
/// the cross-boundary advisory — only when all three match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BufferDelayScope {
    /// [`CvsTracker::cvs_generation_epoch`] of the OPS's extended layer (or the
    /// multistream-wide global epoch) at the observation.
    pub(super) cvs_epoch: u64,
    /// [`OpsAvailabilityStore::effective_reset_generation`] of the observation's
    /// extended layer (global resets plus that layer's local resets), including the
    /// defining OPS's own `ops_reset_flag`. Per-layer (round-2): a reset of an unrelated
    /// extended layer no longer changes this scope.
    pub(super) reset_generation: u64,
    /// [`OpsAvailabilityStore::targeted_reset_generation`] for the observation's
    /// `(obu_xlayer_id, opsID)`. A § 6.10.1 case-3 targeted reset of this OPS bumps it,
    /// re-baselining the comparison for exactly this OPS without disturbing any other.
    pub(super) targeted_reset_generation: u64,
}

/// One stored operating-point buffer-delay baseline (§ 6.10.5): the last explicitly
/// signalled sum together with the boundary scope and temporal unit that scope the
/// error-tier comparison.
#[derive(Debug, Clone, Copy)]
pub(super) struct BufferDelayBaseline {
    /// `ops_decoder_buffer_delay + ops_encoder_buffer_delay`, summed as `u64` so the
    /// two `u32` `uvlc()` values cannot overflow the comparison.
    pub(super) sum: u64,
    /// The CVS / reset / targeted-reset scope at the baseline observation.
    pub(super) scope: BufferDelayScope,
    /// [`CvsTracker::tu_index`] of the baseline observation. A CVS boundary is
    /// temporal-unit-granular (§ 7.3.6), so a baseline observed in an earlier temporal
    /// unit may be split into a different coded video sequence by a CLK later in the
    /// current temporal unit; the error-tier comparison is therefore routed through
    /// [`CvsTracker::defer_or_emit`] on this index.
    pub(super) tu_index: u64,
}

/// Builds the § 6.10.5 operating-point cross-boundary advisory
/// (`decoder-model/buffer-delay-sum-changed-across-cvs`, severity `warning`) for a
/// change of the explicitly signalled buffer-delay sum from `previous_sum` to `sum` for
/// the `(obu_xlayer_id, opsID, op)` triple `key`. Shared by the eager cross-boundary
/// check (`check_ops_buffer_delay_cross_cvs`) and the deferred-error replacement path
/// (`check_ops_buffer_delay_sums`), where it is the `on_drop` diagnostic emitted when a
/// late CLK reveals the deferred intra-CVS error to be a genuine cross-CVS change.
pub(super) fn ops_buffer_delay_cross_cvs_warning(
    key: OpsBufferDelayKey,
    previous_sum: u64,
    sum: u64,
    offset: ByteOffset,
) -> Diagnostic {
    Diagnostic::warning(
        "decoder-model/buffer-delay-sum-changed-across-cvs",
        format!(
            "operating point set {} operating point {} for obu_xlayer_id {} changes its \
             ops_decoder_buffer_delay + ops_encoder_buffer_delay sum from {} to {} across \
             a coded-video-sequence or OPS-reset boundary; the § 6.4.13 / § 6.10.5 \
             \"video sequence\" scope is unspecified, so this finding is advisory under \
             the broad reading",
            key.ops_id,
            key.op_index,
            key.xlayer.get(),
            previous_sum,
            sum,
        ),
    )
    // The finding cites the § 6.10.5 OPS variant of the sum-constancy sentence
    // (§ 6.4.13 is the sequence-header variant, emitted from
    // `check_seq_buffer_delay_sum`).
    .with_spec_section("6.10.5")
    .with_byte_offset(offset)
}

/// Builds the § 6.10.5 operating-point intra-CVS error
/// (`decoder-model/buffer-delay-sum-changed`, severity `error`) for a change of the
/// explicitly signalled buffer-delay sum from `previous_sum` to `sum` for the
/// `(obu_xlayer_id, opsID, op)` triple `key`, observed within one coded video sequence
/// with no intervening OPS reset. Shared by the eager/deferred same-epoch path and the
/// pre-first-CLK same-temporal-unit path (`check_ops_buffer_delay_sums`), which differ
/// only in how the comparison is routed across the § 7.3.6 temporal-unit-granular CVS
/// boundary, not in the diagnostic text.
pub(super) fn ops_buffer_delay_intra_cvs_error(
    key: OpsBufferDelayKey,
    previous_sum: u64,
    sum: u64,
    offset: ByteOffset,
) -> Diagnostic {
    Diagnostic::error(
        "decoder-model/buffer-delay-sum-changed",
        format!(
            "operating point set {} operating point {} for obu_xlayer_id {} changes its \
             ops_decoder_buffer_delay + ops_encoder_buffer_delay sum from {} to {} within \
             one coded video sequence with no intervening OPS reset; § 6.10.5 requires the \
             sum be kept constant",
            key.ops_id,
            key.op_index,
            key.xlayer.get(),
            previous_sum,
            sum,
        ),
    )
    .with_spec_section("6.10.5")
    .with_byte_offset(offset)
}

/// One stored activated-sequence-header buffer-delay baseline (§ 6.4.13): the last
/// explicitly signalled sum of a frame-confirmed activated header for an extended
/// layer, with the CVS epoch that scopes the cross-CVS advisory.
#[derive(Debug, Clone, Copy)]
pub(super) struct SeqBufferDelayBaseline {
    /// `decoder_buffer_delay + encoder_buffer_delay`, summed as `u64`.
    pub(super) sum: u64,
    /// [`CvsTracker::cvs_generation_epoch`] of the extended layer at the baseline
    /// observation.
    pub(super) cvs_epoch: u64,
    /// `seq_header_id` of the header that established the baseline, cited in the
    /// advisory message.
    pub(super) seq_header_id: SequenceHeaderId,
}

impl ValidatorContext {
    pub(super) fn check_ops_buffer_delay_sums(
        &mut self,
        obu: &ObuEnvelope<'_>,
        ops: &OperatingPointSet,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // External HLS may legitimately supply differing decoder-model parameters, so
        // both tiers are suppressed under any Provided mode (precedent: the
        // sequence-state checks and `check_ops_entries_against_active`).
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }

        // The effective reset generation scoping this OBU's own values, for this OBU's
        // own extended layer (a reset of an unrelated layer no longer re-baselines this
        // one — round-2 per-layer scoping). When this OPS carries ops_reset_flag, its
        // reset has not been applied yet (the local checks run first), so account for it
        // here: the reset it carries (whether it bumps the global counter for a global
        // OBU or the local counter for a local one) raises this same layer's effective
        // generation by exactly 1, so a value defined by a resetting OPS is in a fresh
        // reset generation and is never the same-generation continuation of an earlier
        // baseline.
        let effective_reset_gen =
            self.ops.effective_reset_generation(ops.xlayer_id) + u64::from(ops.reset_flag);
        // A § 6.10.1 case-3 targeted reset (ops_cnt == 0) carries no operating-point
        // payloads, so it never reaches this loop; the count therefore already reflects
        // every targeted reset of this OPS that preceded this defining OBU. The defining
        // OBU itself (case 4, ops_cnt > 0) does not bump it, so no in-flight adjustment is
        // needed here (unlike effective_reset_gen, which must add this OBU's reset_flag).
        let scope = BufferDelayScope {
            cvs_epoch: self.cvs.cvs_generation_epoch(ops.xlayer_id),
            reset_generation: effective_reset_gen,
            targeted_reset_generation: self
                .ops
                .targeted_reset_generation(ops.xlayer_id, ops.ops_id),
        };
        let cvs_started = self.cvs.cvs_started(ops.xlayer_id);
        let tu_index = self.cvs.tu_index;

        // Annex E.1: "If the new Operating Point Set OBU does not signal decoder model
        // parameters for a given operating point, the previous set of decoder model
        // parameters does not persist." (mirror `annex-e-decoder-model.md` lines 25–27.)
        // A defining OPS (ops_cnt > 0) supplies a complete new definition of this
        // (obu_xlayer_id, ops_id): every op triple of the key whose new payload omits
        // ops_decoder_model_info(), and every op index the new definition no longer
        // covers (op >= ops_cnt), loses its previously signalled parameters, so its
        // baseline is cleared — never compared against a later explicit value. Clearing
        // (not a default-value comparison) keeps the Annex E mode defaults out of every
        // comparison. A non-defining OPS (ops_cnt == 0: § 6.10.1 case 1/3 reset) carries
        // no payloads and never reaches here; its re-baselining is already handled by the
        // reset and targeted-reset generations. Runs before the comparison loop so an
        // op the redefinition drops cannot be compared.
        if ops.ops_cnt > 0 {
            let ops_explicitly_carries: BTreeSet<u8> = ops
                .payloads
                .iter()
                .filter(|payload| payload.decoder_model_info.is_some())
                .map(|payload| payload.index)
                .collect();
            self.ops_buffer_delay_sums.retain(|key, _| {
                key.xlayer != ops.xlayer_id
                    || key.ops_id != ops.ops_id
                    || ops_explicitly_carries.contains(&key.op_index)
            });
        }

        for payload in &ops.payloads {
            let Some(info) = &payload.decoder_model_info else {
                // Absent ops_decoder_model_info() for this op: per Annex E.1 the previous
                // parameters did not persist (the redefinition's clearing above already
                // dropped any baseline for this triple). Contributes no new signalled
                // value (§ 6.10.5 compares signalled values only).
                continue;
            };
            let sum = u64::from(info.decoder_buffer_delay) + u64::from(info.encoder_buffer_delay);
            let key = OpsBufferDelayKey {
                xlayer: ops.xlayer_id,
                ops_id: ops.ops_id,
                op_index: payload.index,
            };
            // Copied out so the error-tier diagnostic can be routed through
            // `&mut self.cvs.defer_or_emit` without holding a borrow of the map.
            let previous = self.ops_buffer_delay_sums.get(&key).copied();

            // Error tier: same triple, same boundary scope (CVS epoch, reset generation,
            // and per-OPS targeted-reset generation all match), differing sum. There are
            // two routings, both intra-CVS comparisons keyed by the § 7.3.6 boundary, and
            // they are mutually exclusive on `cvs_started`:
            //
            // 1. A coded video sequence has already started for the scope (`cvs_started`):
            //    the comparison is deferred to temporal-unit granularity. A same-temporal-
            //    unit baseline is emitted eagerly (a CVS boundary cannot fall inside a
            //    temporal unit); an earlier-temporal-unit baseline is deferred and, if a
            //    CLK later in this temporal unit splits it into a different coded video
            //    sequence, dropped and replaced by the cross-boundary advisory.
            //
            // 2. No coded video sequence has started yet (`!cvs_started`) but BOTH
            //    observations are in the current temporal unit (`previous.tu_index ==
            //    tu_index`): the pre-first-CLK silence path. Per § 7.3.6 a CLK later in
            //    this temporal unit pulls both observations into the new coded video
            //    sequence (which contains the CLK), making the change intra-CVS — so the
            //    error is deferred PreCvs and emitted on that CLK. If the temporal unit
            //    closes first with no CLK for the layer, the observations are in no coded
            //    video sequence and the comparison is dropped silently (preserving the
            //    documented pre-first-CLK silence). Global-keyed observations keep the
            //    documented cross-CMVS under-report and are not deferred here (a global
            //    CLK does not migrate global baselines in `start_cvs_for_xlayer`, so
            //    deferring would emit a comparison the eager path never re-baselines).
            if let Some(previous) = previous
                && previous.scope == scope
                && previous.sum != sum
            {
                let diagnostic =
                    ops_buffer_delay_intra_cvs_error(key, previous.sum, sum, obu.offset);
                if cvs_started {
                    // When the error is deferred (the baseline came from an earlier
                    // temporal unit) and then dropped because a late CLK starts a new
                    // coded video sequence in this temporal unit, the comparison was
                    // genuinely cross-CVS: emit the cross-boundary advisory in the
                    // error's place so the change is not silently lost (§ 7.3.6
                    // temporal-unit-granular CVS boundary).
                    let on_drop =
                        ops_buffer_delay_cross_cvs_warning(key, previous.sum, sum, obu.offset);
                    self.cvs.defer_or_emit_with_replacement(
                        ops.xlayer_id,
                        previous.tu_index,
                        diagnostic,
                        Some(on_drop),
                        report,
                    );
                } else if previous.tu_index == tu_index && !ops.xlayer_id.is_global() {
                    self.cvs.defer_pre_cvs(ops.xlayer_id, diagnostic, report);
                }
                // The remaining `!cvs_started` shapes are intentionally silent: an
                // earlier-temporal-unit pre-CLK baseline (no CLK has ever started a coded
                // video sequence and the observations span a temporal-unit boundary) and
                // every global-keyed pre-CLK pair stay in the documented pre-first-CLK /
                // cross-CMVS silence.
            }

            // The cross-boundary advisory compares the latest explicit sum against the
            // stored baseline regardless of CVS/reset epoch; run it before overwriting.
            // It reads the baseline from the map directly (the error path's `&mut cvs`
            // borrow above has already ended).
            self.check_ops_buffer_delay_cross_cvs(obu, key, sum, scope, report);

            self.ops_buffer_delay_sums.insert(
                key,
                BufferDelayBaseline {
                    sum,
                    scope,
                    tu_index,
                },
            );
        }
    }

    /// Emits the § 6.10.5 cross-boundary advisory
    /// (`decoder-model/buffer-delay-sum-changed-across-cvs`, severity `warning`) when
    /// the explicitly signalled operating-point buffer-delay sum changes for the same
    /// `(obu_xlayer_id, opsID, op)` triple across a coded-video-sequence or § 6.10.1
    /// OPS-reset boundary. Such a change is conforming under the per-CVS reading of the
    /// § 6.10.5 "video sequence" scope (each CVS re-baselines), so it must stay a
    /// warning: the scope is unspecified and this finding asserts only the broad
    /// (whole-sub-bitstream) reading. The same-CVS, same-reset-generation case is the
    /// error tier (`check_ops_buffer_delay_sums`) and is intentionally not re-reported
    /// here.
    pub(super) fn check_ops_buffer_delay_cross_cvs(
        &self,
        obu: &ObuEnvelope<'_>,
        key: OpsBufferDelayKey,
        sum: u64,
        scope: BufferDelayScope,
        report: &mut ValidationReport,
    ) {
        let Some(previous) = self.ops_buffer_delay_sums.get(&key) else {
            return;
        };
        // The advisory covers a CVS, OPS-reset, or targeted-reset boundary-spanning
        // change. A change sharing the full boundary scope (CVS epoch, reset generation,
        // and per-OPS targeted-reset generation) is the error tier's domain
        // (`check_ops_buffer_delay_sums`), and an unchanged sum is conforming under
        // every reading; both are excluded here.
        if previous.scope == scope || previous.sum == sum {
            return;
        }
        report.push(ops_buffer_delay_cross_cvs_warning(
            key,
            previous.sum,
            sum,
            obu.offset,
        ));
    }
    pub(super) fn check_seq_buffer_delay_sum(
        &mut self,
        xlayer: ExtendedLayerId,
        activating_offset: ByteOffset,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // External HLS may legitimately supply differing decoder-model parameters, so
        // the advisory is suppressed under any Provided mode — the same blanket guard
        // the OPS tier uses, independent of whether the provided set declares a
        // sequence header.
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        // Frame-confirmed activation only; a guessed activation is never compared (its
        // delays could be contradicted by a later frame).
        let Some((seq_header_id, general)) = self.agreement_activation_for(xlayer) else {
            return;
        };
        let Some(info) = general.decoder_model_info else {
            // Annex E.1: "If the new Sequence Header OBU does not signal decoder model
            // parameters for an extended layer, the previous set of decoder model
            // parameters does not persist." (mirror `annex-e-decoder-model.md` lines
            // 24–25.) A frame-confirmed activation of a header WITHOUT explicit
            // seq_decoder_model_info() therefore clears this layer's baseline so a later
            // explicit header is not compared against vanished parameters. Clearing —
            // never a default-value comparison — keeps the Annex E mode defaults out of
            // comparisons.
            self.seq_buffer_delay_sums.remove(&xlayer);
            return;
        };
        let sum = u64::from(info.decoder_buffer_delay) + u64::from(info.encoder_buffer_delay);
        let cvs_epoch = self.cvs.cvs_generation_epoch(xlayer);

        if let Some(previous) = self.seq_buffer_delay_sums.get(&xlayer)
            && previous.cvs_epoch != cvs_epoch
            && previous.sum != sum
        {
            report.push(
                Diagnostic::warning(
                    "decoder-model/buffer-delay-sum-changed-across-cvs",
                    format!(
                        "the activated sequence header for extended layer {} changes its \
                         decoder_buffer_delay + encoder_buffer_delay sum from {} (sequence header \
                         {}) to {} (sequence header {}) across a coded-video-sequence boundary; \
                         the § 6.4.13 \"video sequence\" scope is unspecified, so this finding is \
                         advisory under the broad reading",
                        xlayer.get(),
                        previous.sum,
                        previous.seq_header_id.get(),
                        sum,
                        seq_header_id.get(),
                    ),
                )
                .with_spec_section("6.4.13")
                .with_byte_offset(activating_offset),
            );
        }

        self.seq_buffer_delay_sums.insert(
            xlayer,
            SeqBufferDelayBaseline {
                sum,
                cvs_epoch,
                seq_header_id,
            },
        );
    }

    /// Observes a buffer removal timing OBU. For the OPS-dependent form (§ 5.12,
    /// § 6.11), resolves `(obu_xlayer_id, br_ops_id)` against the active OPS state: an
    /// unavailable OPS under external-HLS-disabled mode is `brt/unavailable-operating-
    /// point-set`, and a `br_ops_cnt` differing from the active `ops_cnt` is
    /// `brt/ops-count-mismatch`. The extended-layer form has nothing to resolve here.
    pub(super) fn observe_buffer_removal_timing(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(brt) = parse_buffer_removal_timing(&mut reader) else {
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

        let Some((br_ops_id, br_ops_cnt)) = brt.ops_reference() else {
            return;
        };
        let xlayer = obu.header.extended_layer_id;

        if let Some(record) = self.ops.get(xlayer, br_ops_id) {
            // Capture the fields the diagnostic needs before the mutable replay-buffer
            // call below borrows `self` (the record itself borrows `self.ops`).
            let record_ops_cnt = record.ops_cnt;
            let record_offset = record.offset;
            // AV2 § 7.3.8.1: the OPS resolved in-band (linear availability held, so the
            // `brt/unavailable-operating-point-set` check did not fire), so buffer this
            // § 7.3.8.5 reference for the random-access-point availability replay.
            self.note_rap_reference(
                RapHlsKey::OperatingPointSet {
                    xlayer: xlayer.get(),
                    ops_id: br_ops_id,
                },
                xlayer,
                obu.offset,
            );
            if br_ops_cnt != record_ops_cnt {
                report.push(
                    Diagnostic::error(
                        "brt/ops-count-mismatch",
                        format!(
                            "OBU_BUFFER_REMOVAL_TIMING references ops_id {} for obu_xlayer_id \
                             {} with br_ops_cnt {}, but the active operating point set (defined \
                             at byte {}) has ops_cnt {}",
                            br_ops_id,
                            xlayer.get(),
                            br_ops_cnt,
                            record_offset,
                            record_ops_cnt
                        ),
                    )
                    .with_spec_section("6.11")
                    .with_byte_offset(obu.offset),
                );
            }
        } else {
            // AV2 § 7.3.8.5: the referenced OPS must be available in-band or by
            // external means. Suppress the hard error only when the caller has
            // explicitly declared this `(obu_xlayer_id, ops_id)` as external HLS;
            // a generic external-HLS mode that declares other objects (e.g. only
            // sequence headers) does not make this OPS available.
            let external_ops_declared = match &options.external_hls {
                ExternalHlsMode::Provided(set) => {
                    set.has_operating_point_set(xlayer.get(), br_ops_id)
                }
                ExternalHlsMode::Disabled => false,
            };
            if !external_ops_declared {
                report.push(
                    Diagnostic::error(
                        "brt/unavailable-operating-point-set",
                        format!(
                            "OBU_BUFFER_REMOVAL_TIMING references ops_id {} for obu_xlayer_id \
                             {}, but no operating point set with that id is available in-band \
                             or declared as external HLS",
                            br_ops_id,
                            xlayer.get()
                        ),
                    )
                    .with_spec_section("7.3.8.5")
                    .with_byte_offset(obu.offset),
                );
            }
        }
    }
}
