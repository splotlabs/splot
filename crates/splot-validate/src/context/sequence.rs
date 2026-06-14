// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Sequence-header observation, activation, and per-CVS sequence state.

use super::*;

/// Per-extended-layer distinct-`obu_mlayer_id` accumulator for the § 6.4.1
/// `SeqMaxMlayerCnt` count (AV2 v1.0.0 § 6.4.1).
///
/// § 6.4.1 (mirror `06-syntax-structures-semantics.md` lines 445-447) requires "the
/// number of distinct values of obu_mlayer_id present in the coded video sequence
/// associated with this sequence header is less than or equal to SeqMaxMlayerCnt", and
/// the § 6.4.1 NOTE (lines 450-452) adds that "the counting applies to all OBUs, even
/// if they are not layer-specific".
///
/// **Attribution (design decision 4, conservative under-approximation).** The count is
/// scoped to *one extended layer's* coded video sequence (§ 7.3.6), but a global
/// (`GLOBAL_XLAYER_ID`) OBU belongs to no single extended layer's CVS, so attributing
/// its `obu_mlayer_id` to a specific extended layer is ambiguous. Only OBUs whose
/// `obu_xlayer_id` names a concrete extended layer are counted under that layer; the
/// `obu_mlayer_id` of a global OBU is left uncounted. This can only *under*-count, so a
/// reported exceedance is always real. The forced-`obu_mlayer_id == 0` of a per-layer
/// `OBU_SEQUENCE_HEADER` (§ 6.2.2) is itself a concrete-`obu_xlayer_id` OBU and is
/// counted, matching the NOTE's "a sequence containing only embedded layer 1 will count
/// as two layers" example.
/// TODO(spec: AV2-6.4-SEQUENCE-HEADER-SEMANTICS): attribute global-`obu_xlayer_id`
/// OBUs (e.g. a global LCR) to the per-extended-layer CVS counts once the § 6.2.2 /
/// § 7.3.6 global-OBU-to-CVS association is modeled.
#[derive(Debug, Default)]
pub(super) struct DistinctMlayerTracker {
    /// For each extended layer, the distinct `obu_mlayer_id` values counted in its
    /// current coded video sequence, and whether the § 6.4.1 exceedance has already
    /// been reported for that coded video sequence (emit once per CVS).
    pub(super) per_xlayer: BTreeMap<ExtendedLayerId, DistinctMlayerState>,
    /// For each extended layer, the distinct `obu_mlayer_id` values observed so far in
    /// the *current temporal unit* (cleared at each temporal-unit advance, analogous to
    /// [`ValidatorContext::frames_seen_in_tu`]). § 7.3.6 (mirror
    /// `07-decoding-process.md` lines 604-606) starts a new coded video sequence AT the
    /// temporal unit containing the CLK, so every id of that temporal unit — including a
    /// § 7.3.8.1 resent-at-RAP sequence header observed before the CLK — belongs to the
    /// NEW coded video sequence. [`Self::reset_cvs`] re-seeds the new coded video
    /// sequence from this per-temporal-unit set so those pre-CLK ids are attributed to
    /// the new CVS (exact re-attribution, not the former whole-state drop).
    pub(super) per_xlayer_tu: BTreeMap<ExtendedLayerId, BTreeSet<EmbeddedLayerId>>,
}

/// One extended layer's distinct-`obu_mlayer_id` count state within its current coded
/// video sequence; see [`DistinctMlayerTracker`].
#[derive(Debug, Default)]
pub(super) struct DistinctMlayerState {
    /// The distinct `obu_mlayer_id` values seen so far in this coded video sequence.
    pub(super) seen: BTreeSet<EmbeddedLayerId>,
    /// The temporal unit in which the first `obu_mlayer_id` of this coded video sequence
    /// was counted, or `None` until the first count. The exceedance baseline for
    /// [`CvsTracker::defer_or_emit`]: when the whole accumulated set was first observed in
    /// the temporal unit of the OBU that triggers the exceedance, all members share one
    /// coded video sequence regardless of a later same-temporal-unit CLK (§ 7.3.6: a CVS
    /// starts *at* a temporal unit, so same-TU OBUs join the same new CVS), and the
    /// diagnostic is emitted eagerly; when the set spans an earlier temporal unit, a CLK
    /// later in the current temporal unit could split it across two coded video sequences,
    /// so the diagnostic is deferred and dropped by [`CvsTracker::flush_completed_tu`] if
    /// such a CLK arrives.
    pub(super) first_tu: Option<u64>,
    /// `true` once `sequence-state/distinct-mlayer-count-exceeds-seq-max` has been
    /// emitted for this coded video sequence (the check emits once per CVS).
    pub(super) reported: bool,
}

impl DistinctMlayerTracker {
    /// Resets `xlayer`'s distinct-`obu_mlayer_id` count at a § 7.3.6 coded-video-sequence
    /// start (CLK), *re-attributing* the same-temporal-unit ids observed before the CLK
    /// to the new coded video sequence rather than dropping them.
    ///
    /// § 7.3.6 (mirror `07-decoding-process.md` lines 604-606): the new coded video
    /// sequence starts AT the temporal unit containing the CLK, so every id of that
    /// temporal unit — canonically the § 7.3.8.1 resent-at-RAP sequence header, forced to
    /// `obu_mlayer_id == 0` (the § 6.4.1 NOTE, mirror
    /// `06-syntax-structures-semantics.md` lines 450-452) — belongs to the new coded
    /// video sequence and must count toward `SeqMaxMlayerCnt`. The new state is therefore
    /// re-seeded from the current temporal unit's seen set (`per_xlayer_tu`) with
    /// `first_tu == tu_index` (the boundary temporal unit, so an exceedance within the new
    /// coded video sequence counted in this temporal unit emits eagerly). The once-per-CVS
    /// `reported` flag is carried only when the old set's first temporal unit *was* this
    /// boundary temporal unit — meaning the old set was entirely in the boundary temporal
    /// unit and thus is the same coded video sequence — so an exceedance already reported
    /// among the pre-CLK ids is not re-reported; when the old set spanned an earlier
    /// temporal unit, its (deferred) exceedance belonged to the ending coded video
    /// sequence and the new coded video sequence starts unreported.
    ///
    /// Only re-seeds the state; the § 6.4.1 exceedance comparison runs *after* the CLK's
    /// frame header activates the new coded video sequence's referenced sequence header
    /// (see [`ValidatorContext::observe_frame_bearing_obu`]'s activation path), where the
    /// correct `SeqMaxMlayerCnt` is available. A set whose pre-CLK members already exceed
    /// `SeqMaxMlayerCnt` cannot be re-surfaced by [`Self::observe`] (it never re-yields an
    /// already-seen id), so the activation path runs [`Self::current_count`] to read the
    /// re-seeded set back out.
    pub(super) fn reset_cvs(&mut self, xlayer: ExtendedLayerId, tu_index: u64) {
        let prior_first_tu = self.per_xlayer.get(&xlayer).and_then(|s| s.first_tu);
        let prior_reported = self.per_xlayer.get(&xlayer).is_some_and(|s| s.reported);
        let tu_seen = self.per_xlayer_tu.get(&xlayer).cloned().unwrap_or_default();
        if tu_seen.is_empty() {
            // No id of this extended layer was observed in the boundary temporal unit:
            // the new coded video sequence genuinely starts empty.
            self.per_xlayer.remove(&xlayer);
            return;
        }
        // Carry `reported` only when the ending set was entirely within this boundary
        // temporal unit (its first counted id is this temporal unit) — then it is the same
        // coded video sequence as the re-seeded set and an already-emitted exceedance must
        // not repeat.
        let reported = prior_reported && prior_first_tu == Some(tu_index);
        let state = DistinctMlayerState {
            seen: tu_seen,
            first_tu: Some(tu_index),
            reported,
        };
        self.per_xlayer.insert(xlayer, state);
    }

    /// Clears the per-temporal-unit seen sets at a global `OBU_TEMPORAL_DELIMITER`
    /// (AV2 § 7.3.7), so the next temporal unit's re-attribution at a CLK
    /// ([`Self::reset_cvs`]) starts from an empty per-temporal-unit set.
    pub(super) fn advance_temporal_unit(&mut self) {
        self.per_xlayer_tu.clear();
    }

    /// Records `mlayer` under `xlayer`'s current coded video sequence at temporal unit
    /// `tu_index` and returns `(new_distinct_count, first_tu)` when this `obu_mlayer_id`
    /// was not already counted *and* the exceedance has not yet been reported for this
    /// coded video sequence; otherwise `None`. `first_tu` is the temporal unit of the
    /// set's first counted OBU (the [`CvsTracker::defer_or_emit`] baseline). The caller
    /// compares the returned count against `SeqMaxMlayerCnt` and, on the first exceedance,
    /// marks the coded video sequence reported via [`Self::mark_reported`].
    ///
    /// The id is always recorded in the per-temporal-unit set
    /// ([`DistinctMlayerTracker::per_xlayer_tu`]) regardless of the once-per-CVS report
    /// state, so [`Self::reset_cvs`] can re-attribute every pre-CLK id of the boundary
    /// temporal unit to the new coded video sequence.
    pub(super) fn observe(
        &mut self,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
        tu_index: u64,
    ) -> Option<(usize, u64)> {
        self.per_xlayer_tu.entry(xlayer).or_default().insert(mlayer);
        let state = self.per_xlayer.entry(xlayer).or_default();
        if state.reported {
            return None;
        }
        let first_tu = *state.first_tu.get_or_insert(tu_index);
        if state.seen.insert(mlayer) {
            Some((state.seen.len(), first_tu))
        } else {
            None
        }
    }

    /// Marks `xlayer`'s current coded video sequence as having reported the § 6.4.1
    /// exceedance, suppressing further reports until the next CVS reset.
    pub(super) fn mark_reported(&mut self, xlayer: ExtendedLayerId) {
        self.per_xlayer.entry(xlayer).or_default().reported = true;
    }

    /// Returns the distinct `obu_mlayer_id` count accumulated so far in `xlayer`'s
    /// current coded video sequence and the set's first-counted temporal unit, or `None`
    /// when nothing has been counted yet or the exceedance was already reported for this
    /// coded video sequence (emit once per CVS). Read-only: it does not record a new id,
    /// so it surfaces a count that [`Self::observe`] cannot re-yield (its already-seen ids
    /// return `None`). Used by the activation-path retroactive check, which compares a
    /// count accumulated *before* a sequence header became active for the extended layer.
    pub(super) fn current_count(&self, xlayer: ExtendedLayerId) -> Option<(usize, u64)> {
        let state = self.per_xlayer.get(&xlayer)?;
        if state.reported {
            return None;
        }
        let first_tu = state.first_tu?;
        if state.seen.is_empty() {
            return None;
        }
        Some((state.seen.len(), first_tu))
    }
}

impl ValidatorContext {
    /// Returns the activated in-band sequence header's general fields for `xlayer`,
    /// if any. The fields are copied out so callers can keep mutating `self`.
    pub(super) fn active_general_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> Option<(SequenceHeaderId, SequenceHeaderGeneral)> {
        let id = *self.active_sequence_by_xlayer.get(&xlayer)?;
        let header = self.sequence_headers.get(&id)?;
        Some((id, header.general))
    }

    /// The frame-confirmed extended layers whose latest activation lies within the *current*
    /// CMVS window — i.e. whose most recent frame-confirmed sequence-header activation
    /// happened at a temporal unit at or after the CMVS-window start
    /// (`cmvs_start_tu_index`). Returns an empty vector when no CMVS window is open.
    ///
    /// The § 6.8.2 LCR DOH requirement and the § 6.6 MSDO DOH requirement scope their
    /// per-layer evaluation to this set instead of the whole-history `frame_confirmed_xlayers`
    /// accumulator, so a non-monotonic header left active from an earlier, already-ended
    /// coded video sequence outside the current CMVS does not trigger a diagnostic against
    /// this CMVS's MSDO / global LCR (codex finding 3393129745). The § 7.3.2 CMVS spans
    /// specific temporal units, so a temporal-unit lower bound is the right scope.
    pub(super) fn frame_confirmed_xlayers_in_current_cmvs(&self) -> Vec<ExtendedLayerId> {
        let Some(cmvs_start) = self.cmvs.current_cmvs_start_tu_index() else {
            return Vec::new();
        };
        self.frame_confirmed_xlayers
            .iter()
            .copied()
            .filter(|xlayer| {
                self.frame_confirmed_activation_tu
                    .get(xlayer)
                    .is_some_and(|&tu| tu >= cmvs_start)
            })
            .collect()
    }

    /// The activated sequence header usable for the § 6.10.7 / § 6.8.9 agreement
    /// checks: the in-band active header for `xlayer`, but only when the
    /// activation is *decidable* — confirmed by a parsed frame-header reference
    /// (§ 5.18.2 `load_sequence_header`), or the OBU-order fallback while it is
    /// the sole in-band sequence header (any frame must then reference it or
    /// trip the availability checks). With several in-band candidates and no
    /// frame yet, the first-seen fallback is a guess a later frame can
    /// contradict, and an agreement error emitted against the guess could not be
    /// retracted — so the checks defer to frame-driven activation instead.
    pub(super) fn agreement_activation_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> Option<(SequenceHeaderId, SequenceHeaderGeneral)> {
        let resolved = self.active_general_for(xlayer)?;
        if self.frame_confirmed_xlayers.contains(&xlayer) || self.sequence_headers.len() == 1 {
            Some(resolved)
        } else {
            None
        }
    }

    /// The activated sequence header for `xlayer`, but *only* when a parsed
    /// frame-header reference confirmed it (§ 5.18.2 `load_sequence_header`) — the
    /// strict variant of [`Self::agreement_activation_for`] that does *not* admit the
    /// sole-in-band-header OBU-order fallback.
    ///
    /// The fallback (`sequence_headers.len() == 1`) is a guess: § 7.3.6 permits staging a
    /// header before any frame activates one, and with external HLS declared the *real*
    /// activated header could be the external one (the in-band staged header may never be
    /// referenced). Checks that fire unconditionally on a violation (the Annex A
    /// value-space check, and the § 6.8.5 / § 6.8.8 / § 6.8.9 LCR-agreement checks)
    /// therefore use this strict gate so they never emit against a fallback guess; they
    /// re-enter the moment a frame confirms the activation. Contrast the OPS / § 6.8.2
    /// resolutions that tolerate the fallback because they emit nothing without an
    /// OPS/global-LCR present and are otherwise suppressed under external HLS.
    pub(super) fn frame_confirmed_activation_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> Option<(SequenceHeaderId, SequenceHeaderGeneral)> {
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return None;
        }
        self.active_general_for(xlayer)
    }

    /// Runs the § 6.10.7 / § 6.8.9 agreement checks that become decidable when a
    /// sequence header is newly activated (or re-activated to a different id) for
    /// `xlayer`: the stored explicit maps of active OPS records describing the
    /// layer (its local bucket plus global-OPS entries), and the § 6.8.9 pairing
    /// through the activated header's `seq_lcr_id`. The dedup keys make repeated
    /// activation idempotent.
    pub(super) fn on_sequence_activation(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // Annex A.2 / Annex A.4 profile and level/tier value-space facts are intrinsic
        // to the in-band sequence header just activated for this layer, locally
        // decidable regardless of any external HLS the caller declares (an externally
        // activated header would carry its own out-of-band values, but the active header
        // recorded here is always the in-band one resolved by the § 5.18.2
        // load_sequence_header path). This is a *header-only* check — it reads nothing
        // from the § 6.4.1 LCR association — so it runs *before* the external-HLS early
        // return below and is never suppressed under a Provided mode (contrast the LCR
        // agreement checks below, which read an association an unmodeled external LCR
        // could shadow). The check gates on a *frame-confirmed* activation
        // (`frame_confirmed_xlayers`), so a fallback-guess staged header that no frame
        // has loaded does not fire (§ 7.3.6 allows staged-but-unactivated headers).
        self.check_annex_a_value_space(xlayer, report);
        // AV2 § 6.8.5 / § 6.8.8 / § 6.8.9: the activated LCR's PTL ceilings, rep-info
        // equality, and dependency-map closure against the sequence header activated for
        // this layer. Unlike the header-only Annex A check above, each of these is
        // *association-dependent*: it pairs the in-band header against the LCR its
        // `seq_lcr_id` resolves to under § 6.4.1 (local-LCR-first, then global). Under a
        // Provided external-HLS mode an unmodeled external *local* LCR with the same
        // `seq_lcr_id` could win that resolution ahead of the in-band record, so the
        // association the validator paired may not be the one a real decoder uses — the
        // in-band "violation" would then be a false positive against the wrong operand
        // (zero-false-positive principle, AGENTS.md § 7). Each check therefore restores
        // its own "suppress under any Provided mode" gate (see the per-check rationale and
        // `check_seq_lcr_reference`'s lcr/global-xlayer-map-missing-xlayer gate, which
        // suppresses on the identical local-first-shadowing reasoning). They use the
        // strict `frame_confirmed_activation_for` gate (no sole-in-band-header fallback),
        // matching the Annex A value-space precedent: a check that fires unconditionally
        // on a violation must never emit against a guessed activation, least of all when
        // an external header could be the real one.
        self.check_lcr_dependency_agreement(xlayer, options, report);
        self.check_lcr_ptl_ceilings(xlayer, options, report);
        self.check_lcr_rep_info_agreement(xlayer, options, report);
        self.check_lcr_expected_dims_bounds(xlayer, options, report);
        if external_declares_sequence_header(options) {
            return;
        }
        // NB: the § 6.4.13 cross-CVS buffer-delay advisory is NOT run here. It is
        // evaluated from the frame path (`observe_frame_bearing_obu`) on every
        // frame-confirmed activation, because this activation event can fire from the
        // sequence-header-observation path before the temporal unit's CLK has advanced
        // the CVS epoch — comparing at that stale epoch would overwrite the baseline and
        // miss a same-id reconfiguration across the boundary.
        let mut pending: Vec<(ByteOffset, u8, Vec<OpsExplicitEntry>)> = Vec::new();
        for bucket in [xlayer, GLOBAL_XLAYER_ID] {
            for record in self.ops.records_for(bucket) {
                let relevant: Vec<OpsExplicitEntry> = record
                    .explicit_entries
                    .iter()
                    .filter(|entry| entry.xlayer_id == xlayer)
                    .cloned()
                    .collect();
                if !relevant.is_empty() {
                    pending.push((record.offset, record.ops_id, relevant));
                }
            }
        }
        for (offset, ops_id, entries) in pending {
            self.check_ops_entries_against_active(offset, ops_id, &entries, options, report);
        }
        // AV2 § 6.6: the activation-precedes-MSDO arrival order for the sub-stream
        // PTL-ceiling agreement check (the MSDO-precedes-activation order is covered by
        // the re-check loop in `observe_msdo`). It gates on the in-band MSDO state and a
        // frame-confirmed activation, and is suppressed when external HLS declares a
        // sequence header. The DOH-constraint check is NOT run here; its CMVS membership
        // is only final at temporal-unit completion, so it is deferred to
        // `resolve_deferred_doh_constraint` (see that method and check_doh_constraint_required).
        self.check_substream_max_ceilings(xlayer, options, report);
        // Annex A Table A.4: record this frame-confirmed activation's interoperability
        // point, embedded-layer count, and activated-global-LCR span into the current
        // temporal unit's IOP pending facts (committed to the right coded-video-sequence
        // window at temporal-unit completion). Suppressed under any Provided external HLS
        // (in-band presence counting is unsound when an external header may shadow the
        // in-band one — the same gate the window evaluation uses).
        self.note_annex_a_iop_activation(xlayer, options);
    }

    pub(super) fn observe_sequence_header(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Gate availability and activation on the same validation the
        // SequenceHeaderSyntax check applies: the full sequence_header_obu() parse,
        // accepting a bounded-but-Ok parse (one that stops at an unimplemented child
        // config), and — for a fully parsed header — a valid §5.2.1 payload tail
        // (obu_extension_flag + trailing_bits). A header that fails its child configs
        // or its tail is malformed and is NOT recorded as available, so a later MFH
        // cannot resolve against it (AV2 § 7.3.8.6).
        let Ok(sequence_header) = parse_sequence_header(&mut reader) else {
            return;
        };
        if sequence_header.is_fully_parsed()
            && finish_obu_payload(
                &mut reader,
                obu.payload,
                obu.header.obu_type.is_extensible_obu(),
            )
            .is_err()
        {
            return;
        }
        let general = sequence_header.general;

        // A conformant sequence header must be base-layer and non-global (AV2 §6.2.2);
        // sequence_header_can_activate() captures exactly that layer-id validity. A
        // header that violates it is malformed (flagged by the stateless §6.2.2
        // checks) and is neither available (§7.3.8.6) nor activatable, so a later MFH
        // cannot resolve against it.
        if !sequence_header_can_activate(obu) {
            return;
        }

        // Record in-band availability (AV2 § 7.3.8.6): a well-formed sequence header
        // included in the bitstream makes its seq_header_id available to later
        // references.
        self.hls
            .record_sequence_header(u32::from(general.seq_header_id.get()));
        // AV2 § 7.3.8.1: note this in-band (re)send (by the sequence header's own extended
        // layer) for the random-access-point availability replay (resolved at temporal-unit
        // completion). The seq_header_id namespace is global (§ 7.3.8.6), so availability is
        // object-keyed; the sending layer drives only the resend's leading / random-access
        // qualification.
        self.rap_replay.note_resend(
            RapHlsKey::SequenceHeader(u32::from(general.seq_header_id.get())),
            obu.header.extended_layer_id,
        );

        // AV2 § 6.4.1 / § 7.3.8.3 / § 7.3.8.6: when seq_lcr_id != 0, the referenced
        // layer configuration record must be available (local-then-global resolution),
        // and a referenced global LCR must include this header's xlayer in its map.
        self.check_seq_lcr_reference(obu, general.seq_lcr_id.get(), options, report);
        // AV2 § 7.3.8.1: when seq_lcr_id resolved to an in-band LCR (the linear
        // § 7.3.8.3 availability check above did not fire), buffer that § 7.3.8.3
        // reference for the random-access-point availability replay, governed by this
        // sequence header's own extended layer.
        self.note_seq_lcr_rap_reference(obu, general.seq_lcr_id.get());

        let seq_header_id = general.seq_header_id;
        let xlayer = obu.header.extended_layer_id;
        let fingerprint = payload_fingerprint(obu.payload);

        // AV2 § 7.3.6: "Within a particular coded video sequence of an extended
        // layer, it is allowed to send redundant copies of the activated
        // sequence_header_obu, but the contents must be bit-identical each time the
        // activated sequence header appears." Compare a payload fingerprint, not
        // parsed fields, since inferred values can hide syntax differences.
        // Fingerprints are scoped per extended layer to the exact coded video
        // sequence: a CLK boundary event drops records from earlier temporal units
        // (see start_cvs_for_xlayer), and a comparison against an earlier temporal
        // unit's record is deferred to the temporal-unit flush (see
        // CvsTracker::defer_or_emit).
        //
        // NOTE: the fingerprint key is (xlayer, seq_header_id); cross-xlayer identity
        // for the same seq_header_id is not yet enforced.
        //
        // Re-scoped under rap-availability-replay: the §7.3.8.1 random-access-point
        // availability this change lands does NOT enable a cross-xlayer identity check.
        // §7.3.8.6 / §6.4.1 model the sequence-header memory as "stored in an area of
        // memory indexed by seq_header_id"
        // (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1, line 641) — a
        // GLOBAL seq_header_id namespace with no extended-layer qualifier — so the
        // availability store is already keyed by seq_header_id alone (cross-xlayer), and
        // the §7.3.8.1 replay key (RapHlsKey::SequenceHeader) is likewise global. The
        // OUTSTANDING gap is the §7.3.6 *bit-identity* comparison, whose fingerprint map
        // is keyed per (xlayer, seq_header_id): two extended layers sending the same
        // seq_header_id with DIFFERENT payloads overwrite the one global memory slot, but
        // §7.3.6's bit-identity sentence scopes "redundant copies ... bit-identical" to a
        // coded video sequence OF AN EXTENDED LAYER (mirror #s-7-3-6), so promoting the
        // fingerprint key to a global seq_header_id namespace needs a cross-extended-layer
        // content baseline and a cross-CVS scope distinct from the current per-layer §7.3.6
        // pruning. That belongs to AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT (the §7.3.6 owner),
        // not to this §7.3.8.1 availability change; this change introduces no cross-xlayer
        // content state that would make it decidable here.
        // TODO(spec: AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT): enforce cross-extended-layer
        // bit-identity of a shared seq_header_id against the global save_sequence_header
        // memory slot (mirror lines 640-641), with a cross-CVS content baseline.
        let tu_index = self.cvs.tu_index;
        match self.sequence_fingerprints.entry((xlayer, seq_header_id)) {
            Entry::Vacant(slot) => {
                slot.insert((fingerprint, tu_index));
            }
            Entry::Occupied(mut slot) => {
                let (stored_fingerprint, stored_tu) = *slot.get();
                if stored_fingerprint != fingerprint {
                    let diagnostic = Diagnostic::error(
                        "hls/repeated-sequence-header-not-identical",
                        format!(
                            "activated sequence header seq_header_id {} for obu_xlayer_id {} \
                             is repeated with different payload bytes",
                            seq_header_id.get(),
                            xlayer.get()
                        ),
                    )
                    .with_spec_section("7.3.6")
                    .with_byte_offset(obu.offset);
                    self.cvs
                        .defer_or_emit(xlayer, stored_tu, diagnostic, report);
                }
                // Refresh the record to this latest appearance: § 7.3.6 starts a new
                // coded video sequence *at the temporal unit*, so a copy re-sent in a
                // CLK temporal unit must survive the CLK pruning as the new coded
                // video sequence's baseline (a non-identical repeat also becomes the
                // new baseline after being routed above).
                slot.insert((fingerprint, tu_index));
            }
        }

        // Store the latest well-formed header per seq_header_id, so a reconfiguration
        // (a later sequence header reusing the id with different layer limits) is the
        // one used for max_tlayer_id / max_mlayer_id checks once a frame header
        // activates it. A non-identical repeat within a CVS is still flagged above. The
        // full header (not just its general fields) is retained so the frame-header
        // core parser can read OrderHintBits / NumRefFrames / dimensions from its inter
        // and screen-content configs (AV2 § 5.18.2).
        let new_general = sequence_header.general;
        self.sequence_header_offsets
            .insert(seq_header_id, obu.offset);
        let previous_header = self.sequence_headers.insert(seq_header_id, sequence_header);
        // The active sequence header for an extended layer defaults to the first one
        // seen in OBU order; a parsed CLK/OLK frame header overrides this with the
        // sequence header it references (see observe_frame_bearing_obu), which is the
        // exact AV2 § 5.18.2 activation point for the paths the skeleton parses.
        self.active_sequence_by_xlayer
            .entry(xlayer)
            .or_insert(seq_header_id);

        // § 6.4.1 associates "this sequence header" with the LCR present prior to
        // it, so every observation (first sighting, bit-identical repeat, or
        // redefinition) re-takes the association snapshot — an LCR that arrived
        // between two sightings pairs with the later one.
        self.snapshot_lcr_association(xlayer, seq_header_id, new_general.seq_lcr_id.get());

        // § 6.10.7 / § 6.8.9 bind whatever content the activated id currently
        // carries, and a same-id reconfiguration (legal at a coded-video-sequence
        // boundary, § 7.3.6) changes that content without an activation-id change.
        // When the agreement inputs (dependency maps, seq_lcr_id) of an
        // already-stored header are redefined, invalidate the id's dedup keys and
        // re-run the checks for every extended layer it is active for. The
        // observed header's own layer is re-run whenever this header is its
        // active one (covering the first sighting and the repeat-after-LCR case;
        // the dedup keys keep re-runs idempotent).
        let agreement_inputs_changed = previous_header.as_ref().is_some_and(|previous| {
            let old = previous.general;
            old.mlayer_dependency_map != new_general.mlayer_dependency_map
                || old.tlayer_dependency_map != new_general.tlayer_dependency_map
                || old.seq_lcr_id != new_general.seq_lcr_id
        });
        // § 7.3.6 also permits a same-`seq_header_id` redefinition that changes only the
        // Annex A value-space fields (profile / chroma / bit-depth / tier / level). Those
        // are not agreement inputs, so they do not appear in `agreement_inputs_changed`,
        // yet they are active for *every* extended layer that references this id — a
        // redefinition flipping the level to a reserved value must re-run the Annex A
        // value-space check for all of them, not just the activating layer. Detect the
        // value-space fingerprint change separately and fold the same active-layer set
        // into the recheck below (the fingerprint in the
        // `emitted_annex_a_value_space` dedup key keeps the re-runs idempotent and only
        // re-emits when a field actually changed).
        let annex_a_value_space_changed = previous_header.as_ref().is_some_and(|previous| {
            annex_a_value_space_fingerprint(&previous.general)
                != annex_a_value_space_fingerprint(&new_general)
        });
        // § 7.3.6 likewise permits a same-`seq_header_id` redefinition that changes only
        // the § 6.8.5 / § 6.8.8 LCR-agreement operands the Annex A fingerprint does not
        // track — `SeqMaxMlayerCnt`, the frame dimensions, and the cropping window. Those
        // are not agreement inputs and not in the value-space fingerprint, yet they are
        // active for *every* extended layer referencing this id, so a redefinition flipping
        // (say) max_frame_width to disagree with the activated LCR must re-run the LCR
        // checks for all of them, not just the activating layer. Detect this fingerprint
        // change separately and fold the same active-layer set into the recheck below (the
        // `lcr/ptl-*` and `lcr/rep-info-mismatch` dedup keys keep the re-runs idempotent and
        // only re-emit when a checked field actually changed).
        let lcr_agreement_values_changed = previous_header.as_ref().is_some_and(|previous| {
            lcr_agreement_value_fingerprint(&previous.general)
                != lcr_agreement_value_fingerprint(&new_general)
        });
        let mut layers_to_check = BTreeSet::new();
        if self.active_sequence_by_xlayer.get(&xlayer) == Some(&seq_header_id) {
            layers_to_check.insert(xlayer);
        }
        if agreement_inputs_changed || annex_a_value_space_changed || lcr_agreement_values_changed {
            // Re-run every extended layer this id is active for: the agreement checks
            // (when their inputs changed), the Annex A value-space check (when its
            // fingerprint changed), and/or the § 6.8.5 / § 6.8.8 LCR-agreement checks (when
            // the LCR-agreement fingerprint changed) must see the redefinition on all
            // referencing layers.
            layers_to_check.extend(
                self.active_sequence_by_xlayer
                    .iter()
                    .filter(|(_, id)| **id == seq_header_id)
                    .map(|(layer, _)| *layer),
            );
        }
        if agreement_inputs_changed {
            // Invalidate the agreement-check dedup keys for this id so the re-run above
            // re-emits (the Annex A dedup key already carries the value-space fingerprint
            // and re-emits on its own when a checked field changed).
            self.emitted_dependency_findings
                .retain(|key| key.seq_header_id() != seq_header_id);
        }
        // AV2 § 6.4.1: a distinct-obu_mlayer_id count accumulated before any active
        // sequence header for an extended layer is only checkable once a header activates
        // and its SeqMaxMlayerCnt becomes available. The eager per-OBU check cannot see
        // it (no active header at count time, and the activating header's own already-seen
        // obu_mlayer_id == 0 yields nothing new), so the activation path compares it
        // retroactively. Suppressed under caller-provided external HLS for the same reason
        // as the eager check: an out-of-band header may carry a SeqMaxMlayerCnt this
        // validator does not model.
        let external_hls_suppresses = matches!(
            &options.external_hls,
            ExternalHlsMode::Provided(set) if set.declares_any_sequence_header()
        );
        for layer in layers_to_check {
            self.on_sequence_activation(layer, options, report);
            // AV2 § 6.4.1: a sequence-header observation that (re)activates this layer's
            // header — the first-seen OBU-order activation, or a same-id reconfiguration
            // — must agree on monotonic_output_order_flag with the other extended layers
            // when definitively inside a § 7.3.2 CMVS. Located at the sequence-header OBU.
            self.check_monotonic_output_order_agreement(layer, obu.offset, options, report);
            if !external_hls_suppresses {
                // Anchor the retroactive exceedance to the activating OBU using the same
                // offset idiom as the eager check (obu.offset + 1, bit 0).
                let byte_offset = obu.offset.saturating_add(1);
                self.retroactive_distinct_mlayer_check(layer, byte_offset, report);
            }
        }
    }

    pub(super) fn validate_active_sequence_limits(
        &self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !requires_active_sequence(obu) {
            return;
        }

        // When external HLS declares any sequence header, an externally-provided
        // sequence header may be the active one for this extended layer (AV2
        // § 7.3.8.1: external HLS objects "remain available ... until superseded"),
        // with layer limits this validator does not model. The in-band
        // active-sequence-limit checks (missing active header and tlayer/mlayer
        // limits) are therefore unreliable and suppressed, so the validator never
        // rejects a conformant external-HLS stream. An empty external set declares no
        // sequence header that could be active, so it does NOT suppress (the missing
        // active header is still an error). Exact enforcement needs external
        // sequence-header activation and layer limits (AV2-5.18-FRAME-HEADER).
        if let ExternalHlsMode::Provided(set) = &options.external_hls
            && set.declares_any_sequence_header()
        {
            return;
        }

        let Some(seq_header_id) = self
            .active_sequence_by_xlayer
            .get(&obu.header.extended_layer_id)
        else {
            report.push(sequence_state_error(
                "sequence-state/no-active-sequence-header",
                "7.3.8",
                obu,
                None,
                format!(
                    "{} uses obu_xlayer_id {} before an active sequence header is available",
                    obu.header.obu_type.spec_name(),
                    obu.header.extended_layer_id.get()
                ),
            ));
            return;
        };

        // Invariant: sequence_headers and active_sequence_by_xlayer are updated
        // together in observe_sequence_header(). This guard only becomes reachable
        // if a future sequence-header eviction policy removes stored headers.
        let Some(sequence_header) = self.sequence_headers.get(seq_header_id) else {
            report.push(sequence_state_error(
                "sequence-state/unknown-sequence-header-id",
                "7.3.8",
                obu,
                None,
                format!(
                    "active seq_header_id {} for obu_xlayer_id {} is unavailable",
                    seq_header_id.get(),
                    obu.header.extended_layer_id.get()
                ),
            ));
            return;
        };

        // AV2 § 6.2.2 NOTE (mirror `06-syntax-structures-semantics.md` lines 197-198): the
        // limits "apply after a sequence header OBU is activated", so they are scoped to the
        // *activated* header's window. For a frame-confirmed layer use the limits snapshotted
        // at the latest § 5.18.2 `load_sequence_header` activation, not the live store: a
        // § 7.3.6 same-`seq_header_id` redefinition (legal only at a coded-video-sequence
        // boundary) overwrites the store the moment it is sent but does not re-activate until
        // its confirming frame, so an OBU between the redefinition and that frame is still in
        // the prior activation's window and must be bounded by the prior (e.g. looser) limits.
        // A fallback-only layer (no frame confirmation) has no snapshot and keeps reading the
        // live store, preserving the eager pre-frame behavior the `active_sequence_header_*`
        // tests rely on.
        let (max_tlayer_id, max_mlayer_id) = self
            .frame_confirmed_activated_limits
            .get(&obu.header.extended_layer_id)
            .copied()
            .unwrap_or((
                sequence_header.general.max_tlayer_id,
                sequence_header.general.max_mlayer_id,
            ));

        if obu.header.temporal_layer_id > max_tlayer_id {
            report.push(sequence_state_error(
                "sequence-state/tlayer-exceeds-max",
                "6.2.2",
                obu,
                Some(BitOffset::from_bits(6)),
                format!(
                    "obu_tlayer_id {} exceeds active sequence max_tlayer_id {}",
                    obu.header.temporal_layer_id.get(),
                    max_tlayer_id.get()
                ),
            ));
        }

        if obu.header.embedded_layer_id > max_mlayer_id {
            let byte_offset = obu.offset.saturating_add(1);
            report.push(
                Diagnostic::error(
                    "sequence-state/mlayer-exceeds-max",
                    format!(
                        "obu_mlayer_id {} exceeds active sequence max_mlayer_id {}",
                        obu.header.embedded_layer_id.get(),
                        max_mlayer_id.get()
                    ),
                )
                .with_spec_section("6.2.2")
                .with_byte_offset(byte_offset)
                .with_bit_offset(BitOffset::from_bits(0)),
            );
        }
    }

    /// Counts the distinct `obu_mlayer_id` values present in `obu`'s extended layer's
    /// current coded video sequence and emits
    /// `sequence-state/distinct-mlayer-count-exceeds-seq-max` (AV2 § 6.4.1) the first
    /// time the count exceeds the active sequence header's `SeqMaxMlayerCnt`.
    ///
    /// § 6.4.1 requires "the number of distinct values of obu_mlayer_id present in the
    /// coded video sequence associated with this sequence header is less than or equal
    /// to SeqMaxMlayerCnt" (mirror `06-syntax-structures-semantics.md` lines 445-447).
    /// Only OBUs carrying a concrete `obu_xlayer_id` are counted (global OBUs cannot be
    /// unambiguously attributed to one extended layer's coded video sequence; see
    /// [`DistinctMlayerTracker`] for the conservative attribution reading and the
    /// associated spec TODO). The comparison uses the extended layer's active sequence
    /// header, so a layer with no active header yet — or whose active header is supplied
    /// out of band — is skipped rather than guessed.
    ///
    /// § 7.3.6 starts a new coded video sequence *at* the temporal unit containing the
    /// CLK, so an OBU of this extended layer observed earlier in the temporal unit that a
    /// later CLK begins a coded video sequence already belongs to the *new* coded video
    /// sequence. The validator cannot know a CLK is still coming when it counts that OBU,
    /// so an exceedance whose accumulated set spans an earlier temporal unit is routed
    /// through [`CvsTracker::defer_or_emit`]: deferred, then dropped by
    /// [`CvsTracker::flush_completed_tu`] when a CLK started a coded video sequence for
    /// this extended layer in the temporal unit (the set straddled the boundary), and
    /// emitted otherwise. An exceedance whose set is entirely within the current temporal
    /// unit is in one coded video sequence regardless of a later CLK and is emitted eagerly.
    pub(super) fn count_distinct_mlayer(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // A global OBU belongs to no single extended layer's coded video sequence; its
        // obu_mlayer_id is left uncounted (sound under-approximation).
        if obu.header.extended_layer_id.is_global() {
            return;
        }

        // When external HLS declares any sequence header, the active header for this
        // extended layer may be supplied out of band with a SeqMaxMlayerCnt this
        // validator does not model, so the in-band count is unreliable and suppressed
        // (mirrors validate_active_sequence_limits' external-HLS gate).
        if let ExternalHlsMode::Provided(set) = &options.external_hls
            && set.declares_any_sequence_header()
        {
            return;
        }

        let xlayer = obu.header.extended_layer_id;
        let tu_index = self.cvs.tu_index;
        let Some((new_count, first_tu)) =
            self.distinct_mlayer
                .observe(xlayer, obu.header.embedded_layer_id, tu_index)
        else {
            return;
        };

        // Compare against the active sequence header's SeqMaxMlayerCnt; with no active
        // in-band header for this extended layer yet (pre-first-activation edge), there
        // is no header to associate the count with, so the check is skipped. The count
        // accumulated before the first activation is compared retroactively by
        // [`Self::retroactive_distinct_mlayer_check`] when a header becomes active.
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        // The obu_mlayer_id lives in the extension byte that follows the OBU header byte,
        // so the diagnostic is anchored there (matching the § 6.2.2 mlayer-exceeds-max
        // idiom: obu.offset + 1, bit 0).
        let byte_offset = obu.offset.saturating_add(1);
        self.emit_distinct_mlayer_exceedance(
            xlayer,
            (new_count, first_tu),
            (seq_header_id, general.seq_max_mlayer_count.get()),
            byte_offset,
            report,
        );
    }

    /// Retroactively compares `xlayer`'s already-accumulated distinct-`obu_mlayer_id`
    /// count against the `SeqMaxMlayerCnt` of the sequence header that just became active
    /// for it (AV2 § 6.4.1). OBUs arriving before any active sequence header for an
    /// extended layer accumulate a distinct count that [`Self::count_distinct_mlayer`]
    /// never compares (it has no header to associate the count with, and the activating
    /// sequence header's own already-seen `obu_mlayer_id == 0` makes [`DistinctMlayerTracker::observe`]
    /// yield `None`). Once a header activates, its `SeqMaxMlayerCnt` is available, so the
    /// pre-activation count is compared here. The diagnostic is anchored to the activating
    /// OBU's `byte_offset` and routed/deduplicated identically to the eager check
    /// (emit once per CVS via [`DistinctMlayerTracker::mark_reported`]). Called from the
    /// sequence-activation path; the external-HLS gate is applied by the caller.
    pub(super) fn retroactive_distinct_mlayer_check(
        &mut self,
        xlayer: ExtendedLayerId,
        byte_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        let Some((count, first_tu)) = self.distinct_mlayer.current_count(xlayer) else {
            return;
        };
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        self.emit_distinct_mlayer_exceedance(
            xlayer,
            (count, first_tu),
            (seq_header_id, general.seq_max_mlayer_count.get()),
            byte_offset,
            report,
        );
    }

    /// Emits `sequence-state/distinct-mlayer-count-exceeds-seq-max` (AV2 § 6.4.1) when the
    /// distinct count exceeds the active header's `SeqMaxMlayerCnt`, routing the diagnostic
    /// through the § 7.3.6 boundary logic and marking `xlayer`'s coded video sequence
    /// reported (emit once per CVS). Shared by the eager [`Self::count_distinct_mlayer`]
    /// and the activation-path [`Self::retroactive_distinct_mlayer_check`].
    /// `count_and_first_tu` is the distinct count and the set's first-counted temporal unit
    /// (the [`CvsTracker::defer_or_emit`] deferral baseline): a set confined to the current
    /// temporal unit emits eagerly; a set spanning an earlier temporal unit is deferred and
    /// dropped if a CLK begins a new coded video sequence for this extended layer in this
    /// temporal unit (the pre-CLK members then belong to the new coded video sequence, not
    /// the exceeding old one). `active_header` is the activated header's id and its
    /// `SeqMaxMlayerCnt`.
    pub(super) fn emit_distinct_mlayer_exceedance(
        &mut self,
        xlayer: ExtendedLayerId,
        count_and_first_tu: (usize, u64),
        active_header: (SequenceHeaderId, u8),
        byte_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        let (count, first_tu) = count_and_first_tu;
        let (seq_header_id, max_mlayer_cnt) = active_header;
        let max = usize::from(max_mlayer_cnt);
        if count <= max {
            return;
        }
        let diagnostic = Diagnostic::error(
            "sequence-state/distinct-mlayer-count-exceeds-seq-max",
            format!(
                "the coded video sequence for obu_xlayer_id {} carries {} distinct \
                 obu_mlayer_id values, exceeding SeqMaxMlayerCnt {} of the active \
                 sequence header {}",
                xlayer.get(),
                count,
                max,
                seq_header_id.get()
            ),
        )
        .with_spec_section("6.4.1")
        .with_byte_offset(byte_offset)
        .with_bit_offset(BitOffset::from_bits(0));
        self.cvs.defer_or_emit(xlayer, first_tu, diagnostic, report);
        self.distinct_mlayer.mark_reported(xlayer);
    }
}
