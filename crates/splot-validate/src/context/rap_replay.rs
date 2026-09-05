// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Random-access-point HLS availability replay.

use super::*;

/// Identity of one referenceable HLS object family + key, for the § 7.3.8.1
/// random-access-point availability replay (AV2 v1.0.0 § 7.3.8.1, mirror
/// `07-decoding-process.md` lines 685-693).
///
/// The key is whatever uniquely names the object within its family at the reference
/// site: a `seq_header_id` for sequence headers, a `cur_mfh_id` (as `mfhId`) for
/// multi-frame headers, an `(obu_xlayer_id, ops_id)` for operating point sets, an `fgm_id`
/// slot for film-grain models, a custom quantizer-matrix level for quantizer matrices. The
/// quantizer-matrix and film-grain references are recorded from the parsed frame header; the
/// replay tracks the OBU *send* (its temporal facts) and is disjoint from the linear
/// availability state, so the quantizer matrix's reset/poison discipline (which governs only
/// the linear check) does not affect replay soundness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RapHlsKey {
    /// Sequence header `seq_header_id` (§ 7.3.8.6), referenced by a frame header's
    /// `seq_header_id_in_frame_header` or a multi-frame header's `mfh_seq_header_id`.
    SequenceHeader(u32),
    /// Multi-frame header `mfhId` (§ 7.3.8.7), referenced by a frame header's
    /// `cur_mfh_id`.
    MultiFrameHeader(u32),
    /// Operating point set `(obu_xlayer_id, ops_id)` (§ 7.3.8.5), referenced by a
    /// buffer-removal-timing OBU's `br_ops_id`.
    OperatingPointSet { xlayer: u8, ops_id: u8 },
    /// Layer configuration record (§ 7.3.8.3). When `xlayer == GLOBAL_XLAYER_ID` (31) the
    /// `id` is a global LCR's `lcr_global_config_record_id`, referenced by a local LCR's
    /// `lcr_global_id` or by a sequence header's `seq_lcr_id` that resolves to a global
    /// record; otherwise the `id` is a local LCR's `lcr_local_id` in that extended layer,
    /// referenced by a sequence header's `seq_lcr_id` that resolves to a local record.
    /// Matches the linear LCR availability stores' keying.
    LayerConfigurationRecord { xlayer: u8, id: u8 },
    /// Local atlas segment OBU `(obu_xlayer_id, atlas_segment_id)` (§ 7.3.8.4), referenced
    /// by a local LCR's `lcr_local_atlas_id`. Only *local* atlas segments participate: a
    /// global atlas "can be available" (§ 7.3.8.4 is permissive, not "shall"), so — like
    /// the linear checks — it is excluded from the replay.
    Atlas { xlayer: u8, id: u8 },
    /// Film-grain model slot `fgm_id` (§ 7.3.8.8), defined by a film-grain OBU's
    /// `fgm_update_flags` and referenced by a frame `film_grain_config()` with
    /// `apply_grain == 1`. Film-grain availability is monotonic (no reset), so a model sent
    /// only before a random access point and not resent is unavailable from that start — the
    /// case the linear monotonic `frame-header/film-grain-model-unavailable` under-reports.
    FilmGrain { slot: u8 },
    /// Custom quantizer-matrix level (§ 7.3.8.9), made available by a quantizer-matrix OBU
    /// (a `qm_bit_map` bit, or every level on a `qm_bit_map == 0` reset-to-defaults) and
    /// referenced by a frame `setup_qm_params()` with `using_qmatrix == 1`. The replay
    /// records the OBU *send* — its temporal facts — and is disjoint from the linear
    /// availability/poison state: a level not (re)sent in or after a random access point is
    /// unavailable from that decode start regardless of reset_qm() (no level is available
    /// from a decode start without a quantizer-matrix OBU send at or after it; § 6.12 makes
    /// even a default matrix an in-OBU field).
    QmLevel { level: u8 },
}

impl RapHlsKey {
    /// The human-readable family name used in the replay diagnostic message.
    pub(super) fn family(self) -> &'static str {
        match self {
            Self::SequenceHeader(_) => "sequence header",
            Self::MultiFrameHeader(_) => "multi-frame header",
            Self::OperatingPointSet { .. } => "operating point set",
            Self::LayerConfigurationRecord { xlayer, .. } if xlayer == GLOBAL_XLAYER_ID.get() => {
                "global layer configuration record"
            }
            Self::LayerConfigurationRecord { .. } => "local layer configuration record",
            Self::Atlas { .. } => "local atlas segment",
            Self::FilmGrain { .. } => "film grain model",
            Self::QmLevel { .. } => "quantizer matrix level",
        }
    }

    /// The spec subsection citing this family's availability requirement, appended to
    /// the § 7.3.8.1 general citation in the diagnostic message.
    pub(super) fn family_section(self) -> &'static str {
        match self {
            Self::SequenceHeader(_) => "7.3.8.6",
            Self::MultiFrameHeader(_) => "7.3.8.7",
            Self::OperatingPointSet { .. } => "7.3.8.5",
            Self::LayerConfigurationRecord { .. } => "7.3.8.3",
            Self::Atlas { .. } => "7.3.8.4",
            Self::FilmGrain { .. } => "7.3.8.8",
            Self::QmLevel { .. } => "7.3.8.9",
        }
    }

    /// A short identifier of the referenced object for the diagnostic message.
    pub(super) fn describe(self) -> String {
        match self {
            Self::SequenceHeader(id) => format!("seq_header_id {id}"),
            Self::MultiFrameHeader(id) => format!("mfhId {id}"),
            Self::OperatingPointSet { xlayer, ops_id } => {
                format!("ops_id {ops_id} for obu_xlayer_id {xlayer}")
            }
            Self::LayerConfigurationRecord { xlayer, id } if xlayer == GLOBAL_XLAYER_ID.get() => {
                format!("lcr_global_config_record_id {id}")
            }
            Self::LayerConfigurationRecord { xlayer, id } => {
                format!("lcr_local_id {id} for obu_xlayer_id {xlayer}")
            }
            Self::Atlas { xlayer, id } => {
                format!("atlas_segment_id {id} for obu_xlayer_id {xlayer}")
            }
            Self::FilmGrain { slot } => format!("fgm_id {slot}"),
            Self::QmLevel { level } => format!("custom quantizer matrix level {level}"),
        }
    }
}

/// One completed-TU resend for anchor-relative visibility. TU, sender and
/// leading-frame status decide whether a decoder starting at a given RAP sees it;
/// a single last-good timestamp cannot answer every anchor.
#[derive(Debug, Clone, Copy)]
pub(super) struct RapResendEvent {
    /// The temporal-unit index in which the object was (re)sent.
    pub(super) tu: u64,
    /// The extended layer whose coded extended layer unit carried the (re)send. A
    /// [`GLOBAL_XLAYER_ID`] send has no single owning layer (it is decoded by whichever
    /// layer first random-accesses there). § 7.4.6 sender-decodability uses this to decide
    /// whether the (re)send's layer is decoded under a given random-access start.
    pub(super) sending_xlayer: ExtendedLayerId,
    /// Whether the sending temporal unit carried a LEADING_* frame OBU in *any* layer
    /// (resolved at temporal-unit completion). § 7.3.8.1: a decode starting at an earlier
    /// random access point "drops any temporal units containing leading frames", so a
    /// strictly-later (re)send in a leading temporal unit is not visible under that start.
    pub(super) tu_has_any_leading: bool,
}

/// Linearly-resolved reference buffered until TU completion; externally suppressed
/// or unavailable objects do not enter replay. Snapshot resends before the
/// reference so a later send cannot retroactively satisfy it. Completion resolves
/// leading-frame status and per-layer sender decodability.
#[derive(Debug, Clone)]
pub(super) struct RapPendingReference {
    /// The referenced object.
    pub(super) key: RapHlsKey,
    /// Referencing layer: every prior RAP in this layer governs the reference.
    /// Global references use every layer's RAPs, since any layer can decode global HLS.
    pub(super) governing_xlayer: ExtendedLayerId,
    /// The object's (re)send events recorded in the *completed* prior temporal units
    /// (object-keyed, cross-extended-layer — § 7.3.8.6 models the sequence-header memory as
    /// a global `seq_header_id` namespace), snapshotted at reference time so a later resend
    /// cannot retroactively satisfy the reference. Their per-anchor visibility is resolved
    /// at completion.
    pub(super) promoted_events: Vec<RapResendEvent>,
    /// Layers that resent this object before the reference in this TU. Any sender
    /// visible under the governing RAP satisfies it; visibility resolves at TU end.
    pub(super) this_tu_resend_xlayers: BTreeSet<ExtendedLayerId>,
    /// Byte offset of the referencing OBU, where the diagnostic is anchored.
    pub(super) offset: ByteOffset,
}

/// § 7.3.8.1 availability under every prior random-access start, per extended
/// layer (§ 7.4.6; docs/spec/av2/1.0.0/07-decoding-process.md).
/// Keep resend events because visibility depends on the chosen anchor: a later
/// leading-frame TU drops, and its sending layer may not yet be decodable.
/// Only events before the earliest retained anchor can be discarded. Histories
/// retain every anchor; correctness takes precedence over a tighter memory bound.
#[derive(Debug, Default)]
pub(super) struct RapReplayTracker {
    /// Completed-TU resends per object, shared across layers (sequence ids are a
    /// global namespace). Events older than every possible anchor are pruned.
    pub(super) resend_events: BTreeMap<RapHlsKey, Vec<RapResendEvent>>,
    /// Current-TU senders per object, snapshotted before each reference. Keep every
    /// sender: one visible send suffices. Completion promotes events and clears this.
    pub(super) resent_this_tu: BTreeMap<RapHlsKey, BTreeSet<ExtendedLayerId>>,
    /// References buffered in the temporal unit currently being observed, resolved at
    /// completion (see [`Self::complete_temporal_unit`]).
    pub(super) pending_this_tu: Vec<RapPendingReference>,
    /// Extended layers for which the temporal unit currently being observed is a § 7.4.1
    /// random access point (a CLK / OLK / RAS OBU in that layer's coded extended layer
    /// unit). Resolved at completion.
    pub(super) current_tu_rap_xlayers: BTreeSet<ExtendedLayerId>,
    /// Extended layers whose coded extended layer unit in the temporal unit currently
    /// being observed contains a LEADING_* frame OBU (§ 7.3.8.1: such units drop under
    /// random access, so their resends do not qualify — unless the unit is itself that
    /// layer's random access point).
    pub(super) current_tu_leading_xlayers: BTreeSet<ExtendedLayerId>,
    /// All RAP TUs per layer, mapped to whether any layer carried a leading frame.
    /// Provides every governing anchor and sender-decodability range queries.
    /// A later leading TU drops under an earlier start and cannot enable its layer.
    pub(super) rap_history: BTreeMap<ExtendedLayerId, BTreeMap<u64, bool>>,
    /// Union of per-layer RAP TUs, used for global references and the anchor floor.
    /// Values share the per-layer history type; global anchor queries need only keys.
    /// Histories only grow, so none of their entries can precede this union's minimum.
    pub(super) rap_history_any: BTreeMap<u64, bool>,
    /// Already-emitted `(object, random-access-point temporal unit)` findings, so one
    /// dangling object reports once per random access point even across several
    /// referencing frames in or after it (proposal dedup requirement).
    pub(super) emitted: BTreeSet<(RapHlsKey, u64)>,
}

impl RapReplayTracker {
    /// Records an in-band (re)send of `key` by extended layer `xlayer` in the temporal
    /// unit currently being observed (§ 7.3.8.1 / § 7.3.7: global HLS precedes the unit's
    /// frame OBUs, so this runs before any reference in the same unit). The sending layer
    /// is retained so its leading / random-access qualification can be resolved at
    /// completion.
    pub(super) fn note_resend(&mut self, key: RapHlsKey, xlayer: ExtendedLayerId) {
        self.resent_this_tu.entry(key).or_default().insert(xlayer);
    }

    /// Marks the temporal unit currently being observed as a § 7.4.1 random access point
    /// for extended layer `xlayer` (a CLK / OLK / RAS OBU in that layer's coded extended
    /// layer unit).
    pub(super) fn note_random_access_point(&mut self, xlayer: ExtendedLayerId) {
        self.current_tu_rap_xlayers.insert(xlayer);
    }

    /// Marks extended layer `xlayer`'s coded extended layer unit in the temporal unit
    /// currently being observed as containing a LEADING_* frame OBU (§ 7.3.8.1).
    pub(super) fn note_leading_frame(&mut self, xlayer: ExtendedLayerId) {
        self.current_tu_leading_xlayers.insert(xlayer);
    }

    /// Buffers a linearly-resolved reference to `key` from the OBU at `offset` in the
    /// temporal unit currently being observed. `governing_xlayer` is the referencing
    /// OBU's `obu_xlayer_id` (the layer whose random access point governs this reference;
    /// a [`GLOBAL_XLAYER_ID`] reference is governed by the global anchor). The object's
    /// completed-unit (re)send events and the senders that resent it *before this reference*
    /// in this unit are snapshotted eagerly (in-band order, so a later resend cannot
    /// retroactively satisfy the reference); their anchor-relative visibility is resolved at
    /// temporal-unit completion (see [`Self::complete_temporal_unit`]), once this unit's
    /// per-extended-layer leading-ness and random-access-point-ness are known.
    pub(super) fn note_reference(
        &mut self,
        key: RapHlsKey,
        governing_xlayer: ExtendedLayerId,
        offset: ByteOffset,
    ) {
        let promoted_events = self.resend_events.get(&key).cloned().unwrap_or_default();
        let this_tu_resend_xlayers = self.resent_this_tu.get(&key).cloned().unwrap_or_default();
        self.pending_this_tu.push(RapPendingReference {
            key,
            governing_xlayer,
            promoted_events,
            this_tu_resend_xlayers,
            offset,
        });
    }

    /// Every governing RAP at/before ref_tu, ascending (§ 7.3.8.1 "any").
    /// Concrete references use their layer's RAPs (§ 7.4.6); global references use
    /// the union. No prior RAP means no resend requirement from the bitstream start.
    pub(super) fn governing_rap_tus(
        &self,
        governing_xlayer: ExtendedLayerId,
        ref_tu: u64,
    ) -> impl Iterator<Item = u64> + '_ {
        let history = if governing_xlayer.is_global() {
            Some(&self.rap_history_any)
        } else {
            self.rap_history.get(&governing_xlayer)
        };
        history
            .into_iter()
            .flat_map(move |history| history.range(..=ref_tu).map(|(&tu, _)| tu))
    }

    /// § 7.4.6: global sends need no layer gate. A concrete sender requires a RAP
    /// in [rap_tu, send_tu] whose TU survives this decode: the anchor TU itself, or
    /// a later TU without leading frames. A RAP in a dropped TU cannot enable a layer.
    pub(super) fn sender_decodable_at(
        &self,
        sending_xlayer: ExtendedLayerId,
        send_tu: u64,
        rap_tu: u64,
    ) -> bool {
        if sending_xlayer.is_global() {
            return true;
        }
        self.rap_history
            .get(&sending_xlayer)
            .is_some_and(|history| {
                history
                    .range(rap_tu..=send_tu)
                    .any(|(&rap_t, &rap_t_has_any_leading)| {
                        rap_t == rap_tu || !rap_t_has_any_leading
                    })
            })
    }

    /// A resend is visible at the anchor or in a later non-leading TU, provided its
    /// sending layer is decodable (§ 7.3.8.1 / § 7.4.6). Even at the anchor itself,
    /// a non-global sender needs its own RAP there.
    pub(super) fn event_visible_at(&self, event: RapResendEvent, rap_tu: u64) -> bool {
        if event.tu == rap_tu {
            return self.sender_decodable_at(event.sending_xlayer, event.tu, rap_tu);
        }
        event.tu > rap_tu
            && !event.tu_has_any_leading
            && self.sender_decodable_at(event.sending_xlayer, event.tu, rap_tu)
    }

    /// Earliest retained RAP, from the union of all histories. Every prior RAP can
    /// still govern a future reference; no later floor is safe. Resends below this
    /// floor cannot satisfy any retained anchor.
    pub(super) fn anchor_floor(&self) -> Option<u64> {
        self.rap_history_any.keys().next().copied()
    }

    /// Promotes resends and RAP facts, then resolves buffered references against
    /// every governing anchor. Reference-time snapshots preserve before-reference
    /// ordering. Returns diagnostics with object keys for external-HLS suppression,
    /// prunes obsolete resend events, and resets current-TU facts.
    pub(super) fn complete_temporal_unit(&mut self, tu_index: u64) -> Vec<(RapHlsKey, Diagnostic)> {
        let tu_has_any_leading = !self.current_tu_leading_xlayers.is_empty();
        for (key, xlayers) in std::mem::take(&mut self.resent_this_tu) {
            let events = self.resend_events.entry(key).or_default();
            for sending_xlayer in xlayers {
                events.push(RapResendEvent {
                    tu: tu_index,
                    sending_xlayer,
                    tu_has_any_leading,
                });
            }
        }
        if !self.current_tu_rap_xlayers.is_empty() {
            self.rap_history_any.insert(tu_index, tu_has_any_leading);
            for &xlayer in &self.current_tu_rap_xlayers {
                self.rap_history
                    .entry(xlayer)
                    .or_default()
                    .insert(tu_index, tu_has_any_leading);
            }
        }

        let mut diagnostics = Vec::new();
        for pending in std::mem::take(&mut self.pending_this_tu) {
            let governing_anchors: Vec<u64> = self
                .governing_rap_tus(pending.governing_xlayer, tu_index)
                .collect();
            for rap_tu in governing_anchors {
                let reference_unit_drops = tu_index > rap_tu && tu_has_any_leading;
                if reference_unit_drops {
                    continue;
                }
                let satisfied = pending
                    .promoted_events
                    .iter()
                    .any(|&event| self.event_visible_at(event, rap_tu))
                    || pending
                        .this_tu_resend_xlayers
                        .iter()
                        .any(|&sending_xlayer| {
                            self.event_visible_at(
                                RapResendEvent {
                                    tu: tu_index,
                                    sending_xlayer,
                                    tu_has_any_leading,
                                },
                                rap_tu,
                            )
                        });
                if satisfied {
                    continue;
                }
                if !self.emitted.insert((pending.key, rap_tu)) {
                    continue;
                }
                diagnostics.push((
                    pending.key,
                    rap_replay_unavailable(pending.key, rap_tu, pending.offset),
                ));
            }
        }
        if let Some(floor) = self.anchor_floor() {
            for events in self.resend_events.values_mut() {
                events.retain(|event| event.tu >= floor);
            }
            self.resend_events.retain(|_, events| !events.is_empty());
        }
        self.current_tu_rap_xlayers.clear();
        self.current_tu_leading_xlayers.clear();
        diagnostics
    }
}

/// Builds the `hls/unavailable-at-random-access-point` replay diagnostic (AV2 v1.0.0
/// § 7.3.8.1, mirror `07-decoding-process.md` lines 685-693), anchored at the dangling
/// reference. The general § 7.3.8.1 rule is the cited section; the family's own
/// availability subsection is named in the message.
pub(super) fn rap_replay_unavailable(
    key: RapHlsKey,
    rap_tu: u64,
    offset: ByteOffset,
) -> Diagnostic {
    Diagnostic::error(
        "hls/unavailable-at-random-access-point",
        format!(
            "the referenced {} ({}, § {}) was last sent before the random access point at \
             temporal unit {rap_tu} and not resent in or after it; § 7.3.8.1 requires an HLS \
             OBU referenced at a random access point to be resent in the random access point's \
             temporal unit (or provided through external means), since decoding may start there \
             and drop temporal units carrying leading frames",
            key.family(),
            key.describe(),
            key.family_section(),
        ),
    )
    .with_spec_section("7.3.8.1")
    .with_byte_offset(offset)
}

/// Whether a § 7.3.8.1 replay finding for `key` is suppressed by `external_hls` (finding
/// 3 — per-key external-HLS suppression). See `complete_rap_replay_tu` for the policy.
///
/// For an externally-*declarable* kind ([`RapHlsKey::SequenceHeader`],
/// [`RapHlsKey::OperatingPointSet`]) the caller's [`crate::options::ExternalHlsSet`] is
/// authoritative: suppress only when the *exact* referenced key is declared external. For
/// a kind the set cannot express ([`RapHlsKey::MultiFrameHeader`], LCRs, atlas segments,
/// film-grain models, and quantizer-matrix levels), any `Provided` mode keeps the blanket
/// suppression, since such an OBU may exist externally without being (or being expressible
/// as) declared.
pub(super) fn rap_replay_suppressed_by_external_hls(
    key: RapHlsKey,
    external_hls: &ExternalHlsMode,
) -> bool {
    let ExternalHlsMode::Provided(set) = external_hls else {
        return false;
    };
    match key {
        RapHlsKey::SequenceHeader(id) => set.has_sequence_header(id),
        RapHlsKey::OperatingPointSet { xlayer, ops_id } => {
            set.has_operating_point_set(xlayer, ops_id)
        }
        RapHlsKey::MultiFrameHeader(_)
        | RapHlsKey::LayerConfigurationRecord { .. }
        | RapHlsKey::Atlas { .. }
        | RapHlsKey::FilmGrain { .. }
        | RapHlsKey::QmLevel { .. } => true,
    }
}

impl ValidatorContext {
    /// Buffers a linearly-resolved § 7.3.8.1 HLS reference for the random-access-point
    /// availability replay, governed by the referencing OBU's extended layer `xlayer`
    /// (resolved at temporal-unit completion; see [`RapReplayTracker`]). § 7.4 random
    /// access initiates per extended layer (§ 7.4.6), so a reference answers to every prior
    /// random access point in its own layer (a [`GLOBAL_XLAYER_ID`] reference answers to
    /// every prior random access point across all layers). The caller buffers only references
    /// whose object was available in-band at reference time and not suppressed by external
    /// HLS, keeping the replay predicate disjoint from the linear `hls/unavailable-*` checks.
    pub(super) fn note_rap_reference(
        &mut self,
        key: RapHlsKey,
        xlayer: ExtendedLayerId,
        offset: ByteOffset,
    ) {
        self.rap_replay.note_reference(key, xlayer, offset);
    }

    /// Buffers a frame-bearing OBU's in-band-resolved § 7.3.8.1 HLS references for the
    /// random-access-point availability replay (AV2 § 7.3.8.6 / § 7.3.8.7), governed by
    /// the frame's extended layer `xlayer`.
    ///
    /// `resolved` is the in-band sequence-header id the frame activates (`None` when the
    /// reference was out of range, external, or unavailable — those cases are owned by the
    /// linear checks and are not replayed). A `cur_mfh_id > 0` that resolves to an in-band
    /// multi-frame header is the frame's § 7.3.8.7 MFH reference; the sequence header it
    /// further references is the same `resolved`.
    pub(super) fn note_frame_rap_references(
        &mut self,
        prefix: &FrameHeaderPrefix,
        resolved: Option<SequenceHeaderId>,
        xlayer: ExtendedLayerId,
        offset: ByteOffset,
    ) {
        if !prefix.cur_mfh_id.is_zero()
            && prefix.cur_mfh_id.in_range()
            && self.hls.multi_frame_header(prefix.cur_mfh_id).is_some()
        {
            self.note_rap_reference(
                RapHlsKey::MultiFrameHeader(prefix.cur_mfh_id.get()),
                xlayer,
                offset,
            );
        }
        if let Some(seq_id) = resolved {
            self.note_rap_reference(
                RapHlsKey::SequenceHeader(u32::from(seq_id.get())),
                xlayer,
                offset,
            );
        }
    }

    /// Resolves and drains this TU's replay references, then applies per-key external
    /// suppression. Provided declarations are authoritative for sequence/OPS keys;
    /// other families remain blanket-suppressed because the partial declaration API
    /// cannot express them. Disabled mode permits all findings (§ 7.3.8.1).
    pub(super) fn complete_rap_replay_tu(
        &mut self,
        completed_tu_index: u64,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let diagnostics = self.rap_replay.complete_temporal_unit(completed_tu_index);
        for (key, diagnostic) in diagnostics {
            if rap_replay_suppressed_by_external_hls(key, &options.external_hls) {
                continue;
            }
            report.push(diagnostic);
        }
    }

    /// Buffers a sequence header's `seq_lcr_id` § 7.3.8.3 reference for the random-access-
    /// point availability replay, but only when it resolved to an in-band LCR (so the
    /// linear § 7.3.8.3 availability check did not fire — keeping the replay predicate
    /// disjoint). Mirrors [`Self::check_seq_lcr_reference`]'s § 6.4.1 resolution order
    /// (local LCR in this extended layer first, then global LCR). The reference is governed
    /// by the sequence header's own extended layer.
    pub(super) fn note_seq_lcr_rap_reference(&mut self, obu: &ObuEnvelope<'_>, seq_lcr_id: u8) {
        if seq_lcr_id == 0 {
            return;
        }
        let xlayer = obu.header.extended_layer_id;
        let key = if self.hls.has_local_lcr(xlayer, seq_lcr_id) {
            RapHlsKey::LayerConfigurationRecord {
                xlayer: xlayer.get(),
                id: seq_lcr_id,
            }
        } else if self.hls.global_lcr_xlayer_map(seq_lcr_id).is_some() {
            RapHlsKey::LayerConfigurationRecord {
                xlayer: GLOBAL_XLAYER_ID.get(),
                id: seq_lcr_id,
            }
        } else {
            return;
        };
        self.note_rap_reference(key, xlayer, obu.offset);
    }
}
