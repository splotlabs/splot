// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! LCR agreement checks against MSDO state and association snapshots.

use super::*;

/// A fingerprint of the sequence-header fields the § 6.8.5 LCR PTL-ceiling and § 6.8.8
/// LCR rep-info agreement checks ([`ValidatorContext::check_lcr_ptl_ceilings`] /
/// [`ValidatorContext::check_lcr_rep_info_agreement`]) compare against the activated LCR
/// **but** that the [`AnnexAValueSpaceFingerprint`] does not already track. The Annex A
/// fingerprint covers profile / chroma / bit-depth / tier / level (the § 6.8.5 PTL
/// operands plus the § 6.8.8 format-info operands), so this fingerprint covers the
/// remainder both checks read: `seq_max_mlayer_cnt_minus_1 + 1` (the § 6.8.5
/// mlayer-count ceiling operand), `max_frame_width/height_minus_1 + 1`, and the
/// cropping window (present flag + the four offsets, the § 6.8.8 rep-info operands).
///
/// A § 7.3.6 same-`seq_header_id` redefinition that changes only these fields does not
/// move the value-space fingerprint, so without this it would not widen
/// `layers_to_check` in [`ValidatorContext::observe_sequence_header`] — leaving other
/// extended layers with this id active unre-checked against their LCRs. Detecting a
/// change here folds those layers into the recheck (the `lcr/ptl-*` and
/// `lcr/rep-info-mismatch` dedup keys keep the re-runs idempotent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LcrAgreementValueFingerprint {
    /// `SeqMaxMlayerCnt` (`seq_max_mlayer_cnt_minus_1 + 1`), the § 6.8.5 mlayer-count
    /// ceiling operand.
    pub(super) max_mlayer_count: u8,
    /// `max_frame_width_minus_1 + 1`, the § 6.8.8 `lcr_max_pic_width` operand.
    pub(super) max_frame_width: u32,
    /// `max_frame_height_minus_1 + 1`, the § 6.8.8 `lcr_max_pic_height` operand.
    pub(super) max_frame_height: u32,
    /// `seq_cropping_window_present_flag`, the § 6.8.8 cropping-present operand.
    pub(super) cropping_present: bool,
    /// `seq_cropping_win_{left,right,top,bottom}_offset`, the § 6.8.8 cropping offsets
    /// (inferred to 0 when the window is absent).
    pub(super) cropping_offsets: (u32, u32, u32, u32),
}

/// Projects the LCR-agreement dedup fingerprint out of an activated sequence header's
/// general fields (see [`LcrAgreementValueFingerprint`]).
pub(super) fn lcr_agreement_value_fingerprint(
    general: &SequenceHeaderGeneral,
) -> LcrAgreementValueFingerprint {
    LcrAgreementValueFingerprint {
        max_mlayer_count: general.seq_max_mlayer_count.get(),
        max_frame_width: general.max_frame_width.get(),
        max_frame_height: general.max_frame_height.get(),
        cropping_present: general.seq_cropping_window_present_flag,
        cropping_offsets: (
            general.cropping_window.left,
            general.cropping_window.right,
            general.cropping_window.top,
            general.cropping_window.bottom,
        ),
    }
}

impl ValidatorContext {
    /// Resolves "the activated global layer configuration record of the coded multistream
    /// video sequence" (AV2 § 6.8.2 / § 7.3.2) from the existing § 6.4.1 association chain:
    /// a *frame-confirmed* activated sequence header's `seq_lcr_id` resolves
    /// local-first-then-global (see [`Self::snapshot_lcr_association`]); an association that
    /// landed on a global record names an activated global LCR. Returns its
    /// `lcr_global_config_record_id` and the [`GlobalLcrRecord`] snapshotted at association
    /// time, or `None` when no frame-confirmed activation resolves one within the current
    /// CMVS — the Unknown state the § 6.8.2 agreement, the § 6.8.2 DOH requirement, and the
    /// Table A.4 global-LCR arms all treat as "no activated global LCR" (never firing).
    ///
    /// Only frame-confirmed activations are consulted (`agreement_activation_for`): a
    /// staged-but-unreferenced header is not yet an activation (§ 7.3.6), and an
    /// observed-but-never-activated global LCR therefore satisfies nothing. The
    /// associations are scanned in ascending `obu_xlayer_id` order, so the first global
    /// association found is deterministic; the § 6.8.2 "all extended layers reference the
    /// same activated global LCR" rule (lines 1550-1551) that would reconcile divergent
    /// resolutions is a separate, out-of-scope residual, so this takes the first resolved
    /// record and the agreement checks compare the MSDO against it.
    ///
    /// Two correctness properties of this resolution:
    ///
    /// - **Association-time snapshot.** The record returned is the [`GlobalLcrRecord`]
    ///   cloned into the association at the header's latest observation, NOT a live
    ///   `global_lcr_records` lookup. A same-id global-LCR redefinition *after* the header
    ///   associated therefore cannot retarget the agreement at the later revision (the same
    ///   discipline the § 6.8.9 dependency path uses for its embedded maps).
    /// - **Present in this CMVS.** The § 6.8.2 agreement and the boundary-identity check
    ///   apply only when an activated global LCR is "present in the same coded multistream
    ///   video sequence". The snapshotted record's observation temporal unit
    ///   (`observed_tu_index`) must lie within the current CMVS window
    ///   (`>= cmvs_start_tu_index`); a record activated by a still-resolvable association
    ///   but observed only in an earlier CMVS is excluded, so it does not leak into a later
    ///   MSDO-only CMVS's evaluation. When no CMVS window is open (`None`) nothing is
    ///   present, so this returns `None`.
    pub(super) fn activated_global_lcr(&self) -> Option<(u8, &GlobalLcrRecord)> {
        let cmvs_start = self.cmvs.current_cmvs_start_tu_index()?;
        self.activated_global_lcr_in_window(cmvs_start)
    }

    /// As [`Self::activated_global_lcr`], but resolves against an explicit CMVS-window start.
    /// [`Self::activated_global_lcr`] passes the live window; this seam keeps the window start
    /// an explicit parameter for callers that resolve against a non-live window.
    pub(super) fn activated_global_lcr_in_window(
        &self,
        cmvs_start: u64,
    ) -> Option<(u8, &GlobalLcrRecord)> {
        self.activated_global_lcr_where(|_xlayer, record| record.observed_tu_index >= cmvs_start)
    }

    /// As [`Self::activated_global_lcr_in_window`], but scopes the activated global LCR to one
    /// that is frame-confirmed-ACTIVATED in a SINGLE boundary temporal unit
    /// (`frame_confirmed_activation_tu[xlayer] == boundary_tu_index`) rather than present
    /// anywhere in the CMVS window. The § 7.3.2 boundary-set check needs this: end condition 2's
    /// divergence turns on whether the BOUNDARY temporal unit itself "has an activated global
    /// layer configuration record". An activated global LCR activated only EARLIER in the CMVS
    /// (its association still chain-resolvable, but its activation temporal unit precedes the
    /// boundary TU) does NOT make end condition 2 false at a later CLK boundary TU that carries
    /// no activation of its own — both rule sets end the CMVS there, so there is no mismatch.
    /// The scope is the *activation* temporal unit, not the global
    /// record's observation temporal unit, because a same-id CLK re-references an already-active
    /// header (re-activating in the boundary TU) without re-sending its sequence header — so the
    /// association snapshot keeps the global record's earlier observation timestamp while the
    /// activation is genuinely in the boundary TU.
    pub(super) fn activated_global_lcr_in_tu(
        &self,
        boundary_tu_index: u64,
    ) -> Option<(u8, &GlobalLcrRecord)> {
        self.activated_global_lcr_where(|xlayer, _record| {
            self.frame_confirmed_activation_tu
                .get(&xlayer)
                .is_some_and(|&tu| tu == boundary_tu_index)
        })
    }

    /// Resolves "the activated global layer configuration record" from the § 6.4.1
    /// association chain (see [`Self::activated_global_lcr`]), returning the first
    /// frame-confirmed activation whose `(xlayer, associated global record)` satisfies
    /// `accept`. The callers supply the scope predicate (whole-CMVS-window by record
    /// observation, or single boundary TU by the xlayer's activation temporal unit).
    /// Associations are scanned in ascending `obu_xlayer_id` order, so the first accepted
    /// record is deterministic.
    pub(super) fn activated_global_lcr_where(
        &self,
        accept: impl Fn(ExtendedLayerId, &GlobalLcrRecord) -> bool,
    ) -> Option<(u8, &GlobalLcrRecord)> {
        for &xlayer in &self.frame_confirmed_xlayers {
            let Some((seq_header_id, _)) = self.agreement_activation_for(xlayer) else {
                continue;
            };
            let Some(association) = self.lcr_associations.get(&(xlayer, seq_header_id)) else {
                continue;
            };
            if !association.lcr_is_global {
                continue;
            }
            let Some(record) = association.global_record.as_ref() else {
                continue;
            };
            if accept(xlayer, record) {
                return Some((association.lcr_id, record));
            }
        }
        None
    }

    /// Emits the § 6.8.2 MSDO↔global-LCR agreement diagnostics (mirror
    /// `06-syntax-structures-semantics.md` lines 1646-1673) for the active MSDO `msdo`
    /// (declared at `msdo_offset`) against the activated global LCR `global` (id
    /// `global_id`). The caller guarantees CMVS membership is resolved `Inside` and both
    /// records are present. Each diagnostic anchors at the most informative OBU — the
    /// disagreeing record — and is deduped by `(global_id, msdo_offset, global.offset,
    /// rule)` so the deferred resolution does not re-spam across the CMVS's temporal units.
    ///
    /// The constraints, in spec order:
    /// 1. `num_streams_minus_2 + 2 == LcrMaxNumXLayerCount` (line 1650).
    /// 2. every `sub_xlayer_id[i]` is in `LcrXLayerID[]` (lines 1651-1652).
    /// 3. when `lcr_aggregate_info_present_flag == 1` (lines 1657-1664):
    ///    `multistream_profile_idc` consistent with `lcr_config_idc` (Annex A.3 Table A.6),
    ///    its interoperability point equal to `lcr_max_interop` (Table A.1),
    ///    `multistream_level_idx == lcr_aggregate_level_idx`, and
    ///    `multistream_tier == lcr_max_tier_flag`.
    /// 4. when `lcr_seq_profile_tier_level_info_present_flag == 1` (lines 1666-1671): for
    ///    each i, `sub_stream_max_profile/level/tier[i] ==
    ///    lcr_seq_profile_idc/lcr_max_level_idx/lcr_tier_flag[sub_xlayer_id[i]]` (exact
    ///    equality — unlike the § 6.6 sub-stream ceilings, which are `<=`).
    /// 5. `multistream_doh_constraint_flag == lcr_doh_constraint_flag` (line 1673).
    pub(super) fn check_lcr_msdo_agreement(
        &mut self,
        global_id: u8,
        global: &GlobalLcrRecord,
        msdo: &MsdoAggregate,
        msdo_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if msdo.sub_streams.len() != global.xlayer_ids.len() {
            self.push_lcr_agreement(
                "lcr/msdo-stream-count-mismatch",
                global_id,
                msdo_offset,
                global.offset,
                msdo_offset,
                0,
                format!(
                    "§ 6.8.2: the OBU_MSDO declares num_streams_minus_2 + 2 = {} but the activated \
                     global layer configuration record {global_id} has LcrMaxNumXLayerCount = {} \
                     (the set-bit count of lcr_xlayer_map); they must be equal",
                    msdo.sub_streams.len(),
                    global.xlayer_ids.len(),
                ),
                report,
            );
        }

        for sub in &msdo.sub_streams {
            if !global.xlayer_ids.contains(&sub.sub_xlayer_id) {
                self.push_lcr_agreement(
                    "lcr/msdo-sub-xlayer-not-in-lcr",
                    global_id,
                    msdo_offset,
                    global.offset,
                    msdo_offset,
                    u32::from(sub.sub_xlayer_id),
                    format!(
                        "§ 6.8.2: the OBU_MSDO names sub_xlayer_id {} but it is not a set bit of \
                         the activated global layer configuration record {global_id}'s \
                         lcr_xlayer_map (LcrXLayerID[]); every sub_xlayer_id must be in LcrXLayerID[]",
                        sub.sub_xlayer_id,
                    ),
                    report,
                );
            }
        }

        if let Some(agg) = global.aggregate_info {
            self.check_lcr_aggregate_agreement(
                global_id,
                agg,
                msdo,
                msdo_offset,
                global.offset,
                report,
            );
        }

        if global.seq_ptl_present {
            self.check_lcr_substream_ptl_agreement(global_id, global, msdo, msdo_offset, report);
        }

        let msdo_doh = msdo.doh_constraint_flag;
        if msdo_doh != global.doh_constraint_flag {
            self.push_lcr_agreement(
                "lcr/msdo-doh-flag-mismatch",
                global_id,
                msdo_offset,
                global.offset,
                msdo_offset,
                0,
                format!(
                    "§ 6.8.2: multistream_doh_constraint_flag ({}) differs from the activated \
                     global layer configuration record {global_id}'s lcr_doh_constraint_flag ({}); \
                     they must be equal",
                    u8::from(msdo_doh),
                    u8::from(global.doh_constraint_flag),
                ),
                report,
            );
        }
    }

    /// § 6.8.2 constraint 3 (mirror lines 1657-1664): the aggregate-info agreement, each
    /// disagreeing field named in the `lcr/msdo-aggregate-mismatch` message. Anchored at
    /// the OBU_MSDO (the disagreeing aggregate-profile/level/tier values it declares).
    pub(super) fn check_lcr_aggregate_agreement(
        &mut self,
        global_id: u8,
        agg: LcrAggregateInfo,
        msdo: &MsdoAggregate,
        msdo_offset: ByteOffset,
        global_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if is_defined_config_idc(agg.config_idc)
            && !config_idc_allows_profile(agg.config_idc, msdo.profile_idc)
        {
            self.push_lcr_agreement(
                "lcr/msdo-aggregate-mismatch",
                global_id,
                msdo_offset,
                global_offset,
                msdo_offset,
                100,
                format!(
                    "§ 6.8.2: multistream_profile_idc ({}) is not consistent with the activated \
                     global layer configuration record {global_id}'s lcr_config_idc ({}) per \
                     Annex A.3 Table A.6",
                    msdo.profile_idc, agg.config_idc,
                ),
                report,
            );
        }

        if let Some(iop) = interoperability_point(msdo.profile_idc)
            && iop.value() != agg.max_interop
        {
            self.push_lcr_agreement(
                "lcr/msdo-aggregate-mismatch",
                global_id,
                msdo_offset,
                global_offset,
                msdo_offset,
                101,
                format!(
                    "§ 6.8.2: the interoperability point ({}) of multistream_profile_idc ({}) per \
                     Annex A.2 Table A.1 differs from the activated global layer configuration \
                     record {global_id}'s lcr_max_interop ({})",
                    iop.value(),
                    msdo.profile_idc,
                    agg.max_interop,
                ),
                report,
            );
        }

        if msdo.level_idx != agg.aggregate_level_idx {
            self.push_lcr_agreement(
                "lcr/msdo-aggregate-mismatch",
                global_id,
                msdo_offset,
                global_offset,
                msdo_offset,
                102,
                format!(
                    "§ 6.8.2: multistream_level_idx ({}) differs from the activated global layer \
                     configuration record {global_id}'s lcr_aggregate_level_idx ({})",
                    msdo.level_idx, agg.aggregate_level_idx,
                ),
                report,
            );
        }

        let lcr_tier = u8::from(agg.max_tier_flag);
        if msdo.tier != lcr_tier {
            self.push_lcr_agreement(
                "lcr/msdo-aggregate-mismatch",
                global_id,
                msdo_offset,
                global_offset,
                msdo_offset,
                103,
                format!(
                    "§ 6.8.2: multistream_tier ({}) differs from the activated global layer \
                     configuration record {global_id}'s lcr_max_tier_flag ({lcr_tier})",
                    msdo.tier,
                ),
                report,
            );
        }
    }

    /// § 6.8.2 constraint 4 (mirror lines 1666-1671): for each i in
    /// `0..=num_streams_minus_2 + 1`, the MSDO's `sub_stream_max_*[i]` equals the global
    /// LCR's `lcr_*[sub_xlayer_id[i]]` — exact equality (unlike the § 6.6 `<=` ceilings).
    /// An i whose `sub_xlayer_id[i]` is not in the global LCR's per-xlayer PTL map is
    /// skipped here: it is already flagged by constraint 2
    /// (`lcr/msdo-sub-xlayer-not-in-lcr`), so re-reporting the absent PTL entry would be
    /// redundant. The diagnostic anchors at the OBU_MSDO (the `sub_stream_max_*` values).
    pub(super) fn check_lcr_substream_ptl_agreement(
        &mut self,
        global_id: u8,
        global: &GlobalLcrRecord,
        msdo: &MsdoAggregate,
        msdo_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        for sub in &msdo.sub_streams {
            let Some(ptl) = global.seq_ptl_by_xlayer.get(&sub.sub_xlayer_id) else {
                continue;
            };
            if sub.max_profile != ptl.seq_profile_idc
                || sub.max_level != ptl.max_level_idx
                || sub.max_tier != ptl.tier_flag
            {
                self.push_lcr_agreement(
                    "lcr/msdo-substream-ptl-mismatch",
                    global_id,
                    msdo_offset,
                    global.offset,
                    msdo_offset,
                    u32::from(sub.sub_xlayer_id),
                    format!(
                        "§ 6.8.2: for sub_xlayer_id {}, the OBU_MSDO's (sub_stream_max_profile, \
                         sub_stream_max_level, sub_stream_max_tier) = ({}, {}, {}) must equal the \
                         activated global layer configuration record {global_id}'s \
                         (lcr_seq_profile_idc, lcr_max_level_idx, lcr_tier_flag) = ({}, {}, {})",
                        sub.sub_xlayer_id,
                        sub.max_profile,
                        sub.max_level,
                        sub.max_tier,
                        ptl.seq_profile_idc,
                        ptl.max_level_idx,
                        ptl.tier_flag,
                    ),
                    report,
                );
            }
        }
    }

    /// Emits `lcr/doh-constraint-required` (error, § 6.8.2, mirror lines 1619-1621) when any
    /// sequence header activated within the *current CMVS* has
    /// `monotonic_output_order_flag == 0` while the activated global LCR's
    /// `lcr_doh_constraint_flag == 0`. The same deferred-resolution mechanism as
    /// `msdo/doh-constraint-required`, but the constrained flag is the global LCR's, not the
    /// MSDO's. Deduped by `(global_id, global.offset, global.offset, rule)`. Anchored at the
    /// activating header (the disagreeing record) when its offset is known, else the global
    /// LCR OBU.
    ///
    /// The loop is scoped to [`Self::frame_confirmed_xlayers_in_current_cmvs`] — headers
    /// whose latest activation lies within the current CMVS window — NOT the whole-history
    /// `frame_confirmed_xlayers` accumulator, so a non-monotonic header left active from an
    /// earlier, already-ended coded video sequence outside this CMVS is not flagged against
    /// this CMVS's global LCR.
    pub(super) fn check_lcr_doh_constraint_required(
        &mut self,
        global_id: u8,
        global: &GlobalLcrRecord,
        report: &mut ValidationReport,
    ) {
        if global.doh_constraint_flag {
            return;
        }
        let xlayers = self.frame_confirmed_xlayers_in_current_cmvs();
        for xlayer in xlayers {
            let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
                continue;
            };
            if general.monotonic_output_order_flag {
                continue;
            }
            let anchor = self
                .sequence_header_offsets
                .get(&seq_header_id)
                .copied()
                .unwrap_or(global.offset);
            self.push_lcr_agreement(
                "lcr/doh-constraint-required",
                global_id,
                anchor,
                global.offset,
                anchor,
                u32::from(xlayer.get()),
                format!(
                    "§ 6.8.2: the sequence header activated for extended layer {} has \
                     monotonic_output_order_flag == 0 inside a coded multistream video sequence, \
                     but the activated global layer configuration record {global_id}'s \
                     lcr_doh_constraint_flag == 0; § 6.8.2 requires lcr_doh_constraint_flag == 1 \
                     when any activated sequence header has monotonic_output_order_flag == 0",
                    xlayer.get(),
                ),
                report,
            );
        }
    }

    /// Pushes one § 6.8.2 MSDO↔global-LCR agreement diagnostic (error, spec section
    /// `6.8.2`) anchored at `anchor`, deduped by `(global_id, key_a, global_offset, rule,
    /// field)` so the deferred CMVS resolution does not re-spam it across the CMVS's
    /// temporal units. `key_a` is the MSDO offset for the agreement constraints (a new MSDO
    /// re-emits) and the activating-header offset for the DOH requirement (each disagreeing
    /// header fires once). `field` distinguishes the several sub-fields of a shared rule
    /// (the four `lcr/msdo-aggregate-mismatch` arms, each disagreeing `sub_xlayer_id`) so
    /// two distinct disagreements are not collapsed into one.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_lcr_agreement(
        &mut self,
        rule_id: &'static str,
        global_id: u8,
        key_a: ByteOffset,
        global_offset: ByteOffset,
        anchor: ByteOffset,
        field: u32,
        message: String,
        report: &mut ValidationReport,
    ) {
        if !self
            .emitted_lcr_agreement
            .insert((global_id, key_a, global_offset, rule_id, field))
        {
            return;
        }
        report.push(
            Diagnostic::error(rule_id, message)
                .with_spec_section("6.8.2")
                .with_byte_offset(anchor),
        );
    }

    /// Takes the § 6.4.1 LCR-association snapshot for one observed sequence
    /// header: `seq_lcr_id == 0` or an unresolved reference clears any previous
    /// snapshot (the latest observation of an id defines its association);
    /// otherwise the local-first-then-global resolution against the LCRs present
    /// prior to this header is stored, with the resolved record's embedded-layer
    /// maps as of this observation.
    pub(super) fn snapshot_lcr_association(
        &mut self,
        xlayer: ExtendedLayerId,
        seq_header_id: SequenceHeaderId,
        seq_lcr_id: u8,
    ) {
        let key = (xlayer, seq_header_id);
        if seq_lcr_id == 0 {
            self.lcr_associations.remove(&key);
            return;
        }
        let association = if self.hls.has_local_lcr(xlayer, seq_lcr_id) {
            Some(LcrAssociation {
                lcr_is_global: false,
                lcr_id: seq_lcr_id,
                maps: self
                    .hls
                    .local_lcr_embedded
                    .get(&(xlayer, seq_lcr_id))
                    .cloned(),
                global_record: None,
                ptl: self.hls.local_lcr_ptl.get(&(xlayer, seq_lcr_id)).copied(),
                rep_info: self
                    .hls
                    .local_lcr_rep_info
                    .get(&(xlayer, seq_lcr_id))
                    .copied(),
            })
        } else if self.hls.global_lcr_xlayer_map(seq_lcr_id).is_some() {
            Some(LcrAssociation {
                lcr_is_global: true,
                lcr_id: seq_lcr_id,
                maps: self
                    .hls
                    .global_lcr_embedded
                    .get(&(seq_lcr_id, xlayer))
                    .cloned(),
                global_record: self.global_lcr_records.get(&seq_lcr_id).cloned(),
                ptl: self.hls.global_lcr_ptl.get(&(seq_lcr_id, xlayer)).copied(),
                rep_info: self
                    .hls
                    .global_lcr_rep_info
                    .get(&(seq_lcr_id, xlayer))
                    .copied(),
            })
        } else {
            None
        };
        match association {
            Some(association) => {
                self.lcr_associations.insert(key, association);
            }
            None => {
                self.lcr_associations.remove(&key);
            }
        }
    }
}
