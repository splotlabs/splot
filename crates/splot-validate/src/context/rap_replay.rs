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
/// multi-frame headers, an `(obu_xlayer_id, ops_id)` for operating point sets. Only
/// families with a concrete, parsed reference site participate; film-grain / quantizer-
/// matrix references await frame-header parsing (named residual on
/// AV2-7.3.8-HLS-AVAILABILITY).
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
        }
    }
}

/// One in-band (re)send of an HLS object, recorded as a § 7.3.8.1 replay *event*.
///
/// The anchor-relative visibility predicate (see [`RapReplayTracker`]) needs each
/// (re)send's temporal unit, its sending extended layer, and whether that temporal unit
/// turned out to carry leading frames — facts that decide whether the (re)send is visible
/// *under a decode that starts at a given random access point R*. A single global last-good
/// scalar cannot answer this: a (re)send that is visible when starting at one random access
/// point can be invisible when starting at an earlier one (it sits in a strictly-later
/// temporal unit that drops leading frames, or in a layer that does not yet decode under
/// that start). So the tracker stores the events and replays them per anchor.
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

/// One reference buffered for § 7.3.8.1 replay resolution at temporal-unit completion.
///
/// Buffered only when the reference resolved linearly (the object was available in-band
/// at reference time, so the linear `hls/unavailable-*` check did not fire — the two
/// predicates are disjoint by construction) and external HLS did not suppress it.
///
/// The before-reference same-temporal-unit (re)send senders are captured *eagerly* (in-band
/// order) so a (re)send that follows the reference does not retroactively satisfy
/// "available ... prior to being referenced" (matching the linear checks' intra-temporal-
/// unit ordering). Their visibility (leading-ness, random-access-point-ness, § 7.4.6
/// sender-decodability) is resolved at temporal-unit completion against the reference's
/// governing random access point, when this unit's per-extended-layer facts are fully
/// known.
#[derive(Debug, Clone)]
pub(super) struct RapPendingReference {
    /// The referenced object.
    pub(super) key: RapHlsKey,
    /// The governing extended layer for this reference: the referencing OBU's
    /// `obu_xlayer_id`. § 7.4 random access initiates *per extended layer* (§ 7.4.6
    /// Multistream Random Access, mirror `07-decoding-process.md` lines 1314-1318: "a
    /// temporal unit may be a random access point for some extended layers but not for
    /// others" and "the decoder shall not decode coded extended layer units for an
    /// extended layer until a random access point for that extended layer is
    /// encountered"), so a reference answers to *its own* layer's most recent random
    /// access point. [`GLOBAL_XLAYER_ID`] references (e.g. a global-layer
    /// buffer-removal-timing OBU) are governed by the global anchor — the most recent
    /// random access point across *any* extended layer — since a global-layer HLS OBU is
    /// decoded by whichever layer first random-accesses at that temporal unit, so the
    /// referenced object must be available at any random access point a decoder might
    /// start from.
    pub(super) governing_xlayer: ExtendedLayerId,
    /// The object's (re)send events recorded in the *completed* prior temporal units
    /// (object-keyed, cross-extended-layer — § 7.3.8.6 models the sequence-header memory as
    /// a global `seq_header_id` namespace), snapshotted at reference time so a later resend
    /// cannot retroactively satisfy the reference. Their per-anchor visibility is resolved
    /// at completion.
    pub(super) promoted_events: Vec<RapResendEvent>,
    /// The extended layers that (re)sent this object *earlier in this temporal unit*
    /// (before this reference, in-band order); empty if it was not resent before the
    /// reference. The before-reference resend counts when *any* of these senders is visible
    /// under the governing random access point. Their leading-ness / random-access-point-
    /// ness / § 7.4.6 sender-decodability is deferred to temporal-unit completion (see
    /// [`RapReplayTracker::complete_temporal_unit`]).
    pub(super) this_tu_resend_xlayers: BTreeSet<ExtendedLayerId>,
    /// Byte offset of the referencing OBU, where the diagnostic is anchored.
    pub(super) offset: ByteOffset,
}

///
/// **Event pruning.** Per-anchor visibility means the per-object last-good scalar is
/// replaced by stored (re)send *events*; the per-layer / any-layer random-access-point
/// histories back the governing-anchor scan and clause (c)/(a) sender-decodability. All are
/// pruned of entries strictly below the anchor floor — the *earliest* retained random access
/// point, since under the every-anchor rule (finding 2) a future reference can be governed by
/// any random access point that has occurred. An entry below the earliest retained anchor can
/// never affect a future verdict, so dropping it preserves every event a future reference
/// could see; see [`RapReplayTracker::anchor_floor`] for the bound (state is held to the
/// random access points in the live window, small for real streams — correctness over a
/// tighter memory bound).
#[derive(Debug, Default)]
pub(super) struct RapReplayTracker {
    /// Per object, every visible-candidate in-band (re)send event recorded in *completed*
    /// temporal units (object-keyed, cross-extended-layer — § 7.3.8.6 models the sequence-
    /// header memory as a global `seq_header_id` namespace). Anchor-relative visibility (see
    /// the type docs) replays these per reference against its governing random access point;
    /// a single scalar cannot, because a (re)send visible when starting at one random access
    /// point may be invisible when starting at an earlier one. Pruned of events older than
    /// every current anchor's floor.
    pub(super) resend_events: BTreeMap<RapHlsKey, Vec<RapResendEvent>>,
    /// Objects (re)sent in the temporal unit currently being observed, mapped to the *set*
    /// of extended layers that sent each (eager, in-band order). Used both to snapshot the
    /// before-reference resends for the current unit and to append this unit's resend events
    /// into [`Self::resend_events`] at completion (whose visibility needs each sending
    /// layer's leading / random-access state). Cleared per unit. When an object is resent by
    /// several layers in one unit, *all* senders are retained: the object becomes available
    /// for a random access point if *any* of them is visible under that start (§ 7.3.8.1 is
    /// a per-object availability question, so one visible send suffices).
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
    /// Per extended layer, the temporal-unit index of its most recent random access point
    /// completed so far. Tracked for diagnostics/pruning context; § 7.3.8.1 satisfaction is
    /// resolved against *every* governing anchor (see [`Self::governing_rap_tus`] and
    /// [`Self::complete_temporal_unit`]), not just the most recent one, so this scalar is no
    /// longer the satisfaction anchor.
    pub(super) most_recent_rap_tu: BTreeMap<ExtendedLayerId, u64>,
    /// The temporal-unit index of the most recent random access point across *any*
    /// extended layer, or `None` before any random access point. Retained as the most-recent
    /// global anchor for context; like [`Self::most_recent_rap_tu`] it is not the sole
    /// satisfaction anchor — [`GLOBAL_XLAYER_ID`] references must be satisfied at *every*
    /// random access point a decoder might start from (every entry of [`Self::rap_history_any`]
    /// at or before the reference), per § 7.3.8.1's "any random access point".
    pub(super) most_recent_rap_tu_any: Option<u64>,
    /// Per extended layer, the set of temporal units at which that layer had a § 7.4.1
    /// random access point. Two roles. (1) The *governing anchors* of a layer reference:
    /// § 7.3.8.1 requires availability "if decoding process starts at **any** random access
    /// point", so a reference from layer `L` must be satisfied under every `L`-random-access
    /// point `R` with `R <= refTU` (finding 2), not only the most recent — see
    /// [`Self::governing_rap_tus`]. (2) § 7.4.6 sender-decodability — clause (c)/(a) of the
    /// visibility predicate asks whether a (re)send's sending layer had a random access point
    /// in the closed interval `[R, S.tu]` whose own temporal unit is decoded under start-at-`R`
    /// (so its coded extended layer units are decoded by `S.tu` under that decode). A
    /// `BTreeMap` keyed by temporal unit makes both queries range scans; the `bool` value
    /// records whether the random-access-point temporal unit carried a LEADING_* frame in any
    /// layer — a strictly-post-`R` such unit drops under start-at-`R` (§ 7.3.8.1), so its
    /// random access point does not let the layer decode from `R` (see
    /// [`Self::sender_decodable_at`]). Pruned of entries strictly below the anchor floor (see
    /// [`Self::anchor_floor`]).
    pub(super) rap_history: BTreeMap<ExtendedLayerId, BTreeMap<u64, bool>>,
    /// The set of temporal units that were a § 7.4.1 random access point for *any* extended
    /// layer (the union of [`Self::rap_history`]'s value sets). These are the governing
    /// anchors of a [`GLOBAL_XLAYER_ID`] reference: a global-layer HLS OBU is decoded by
    /// whichever layer first random-accesses at a temporal unit, so the object it references
    /// must be available at *every* such start point at or before the reference (§ 7.3.8.1
    /// "any random access point", finding 2). Maintained explicitly (rather than recomputed
    /// from [`Self::rap_history`]) so the per-reference anchor scan is a single range query.
    /// Keyed by temporal unit; the `bool` value mirrors [`Self::rap_history`]'s
    /// (whether that random-access-point temporal unit carried a LEADING_* frame in any
    /// layer) so both histories share a value type, though a global reference's senders are
    /// always decodable (see [`Self::sender_decodable_at`]) and never consult it. Pruned of
    /// entries strictly below the anchor floor (see [`Self::anchor_floor`]).
    pub(super) rap_history_any: BTreeMap<u64, bool>,
    /// Already-emitted `(object, random-access-point temporal unit)` findings, so one
    /// dangling object reports once per random access point even across several
    /// referencing frames in or after it (proposal dedup requirement).
    pub(super) emitted: BTreeSet<(RapHlsKey, u64)>,
    /// A permanently-empty random-access-point history, returned by [`Self::governing_rap_tus`]
    /// for a layer with no recorded random access point. Held as a field (rather than a
    /// per-call temporary) so the returned `range(..)` iterator can borrow it.
    pub(super) empty_rap_history: BTreeMap<u64, bool>,
}

impl RapReplayTracker {
    /// Records an in-band (re)send of `key` by extended layer `xlayer` in the temporal
    /// unit currently being observed (§ 7.3.8.1 / § 7.3.7: global HLS precedes the unit's
    /// frame OBUs, so this runs before any reference in the same unit). The sending layer
    /// is retained so its leading / random-access qualification can be resolved at
    /// completion.
    pub(super) fn note_resend(&mut self, key: RapHlsKey, xlayer: ExtendedLayerId) {
        // Accumulate every sender in this unit (not last-writer-wins): a qualifying resend
        // must not be lost when a later non-qualifying (leading, non-random-access) resend
        // of the same object follows it in the same unit — § 7.3.8.1 availability holds if
        // *any* same-unit send qualifies.
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
        // The senders of this object earlier in this unit (before this reference, in-band
        // order). Their visibility is deferred: the random access point's own unit is always
        // decoded, so a before-reference resend in it counts even when the unit is leading.
        // The full set (not just one sender) is captured so a visible resend is not lost
        // behind a later non-visible one.
        let this_tu_resend_xlayers = self.resent_this_tu.get(&key).cloned().unwrap_or_default();
        self.pending_this_tu.push(RapPendingReference {
            key,
            governing_xlayer,
            promoted_events,
            this_tu_resend_xlayers,
            offset,
        });
    }

    /// The governing random access points for a reference from `governing_xlayer` made in
    /// temporal unit `ref_tu`: **every** random access point `R <= ref_tu` a decoder might
    /// start from, smallest first (finding 2). § 7.3.8.1 requires the referenced HLS OBU to
    /// be available "if decoding process starts at **any** random access point", so a single
    /// most-recent anchor is insufficient — a (re)send visible to the newest anchor can be
    /// invisible to an older one (e.g. a clause-(a) resend in a temporal unit that also
    /// carries leading frames drops under start-at-the-older-anchor).
    ///
    /// A reference from a concrete layer `L` answers to `L`'s own random access points
    /// (§ 7.4.6 per-extended-layer random access: a decoder cannot decode `L`'s coded
    /// extended layer units until `L` itself random-accesses); a [`GLOBAL_XLAYER_ID`]
    /// reference answers to the random access points across *any* layer (a global-layer HLS
    /// OBU is decoded by whichever layer first random-accesses there). Empty when no random
    /// access point at or before `ref_tu` governs the reference yet (decoding from the
    /// bitstream start needs no resend).
    pub(super) fn governing_rap_tus(
        &self,
        governing_xlayer: ExtendedLayerId,
        ref_tu: u64,
    ) -> impl Iterator<Item = u64> + '_ {
        // `..=ref_tu`: a random access point strictly after the reference cannot be a start
        // point the reference is decoded from. Ascending order is intentional — the caller
        // reports the smallest (earliest) violated start point, which is the most actionable.
        // A governing anchor `R` is a start point a decoder uses; it is itself always decoded
        // (§ 7.4.1), so its own leading-ness never disqualifies it — only the temporal-unit
        // keys matter here (leading-ness gates *senders* reached from `R`, in
        // [`Self::sender_decodable_at`]). For a global reference the keys come from the
        // any-layer history; for a layer reference from that layer's per-anchor history.
        let history: &BTreeMap<u64, bool> = if governing_xlayer.is_global() {
            &self.rap_history_any
        } else {
            // No history for an unseen layer == no governing anchor.
            self.rap_history
                .get(&governing_xlayer)
                .unwrap_or(&self.empty_rap_history)
        };
        history.range(..=ref_tu).map(|(&tu, _)| tu)
    }

    /// § 7.4.6 sender-decodability — clause (c) of the visibility predicate. `true` when a
    /// (re)send by `sending_xlayer` at temporal unit `send_tu` is decoded under a decode
    /// that starts at random access point `rap_tu`.
    ///
    /// A [`GLOBAL_XLAYER_ID`] send is decoded by whichever layer first random-accesses at
    /// its temporal unit, so it is decodable whenever that temporal unit is decoded (its
    /// leading-ness and `send_tu == rap_tu` exemptions are handled by clauses (a)/(b)). A
    /// concrete sending layer's coded extended layer units begin decoding at that layer's
    /// first random access point at or after `rap_tu` (§ 7.4.6: "the decoder shall not
    /// decode coded extended layer units for an extended layer until a random access point
    /// for that extended layer is encountered"), so the send is decoded iff the layer had a
    /// random access point `T` in the closed interval `[rap_tu, send_tu]` **whose own temporal
    /// unit is itself decoded under start-at-`rap_tu`** (round-5 finding). `T`'s temporal unit
    /// is decoded under start-at-`rap_tu` exactly when it is the start unit (`T == rap_tu`,
    /// always decoded — § 7.4.1) or it carries no leading frame in any layer (a strictly-later
    /// leading temporal unit drops wholesale under start-at-`rap_tu`, § 7.3.8.1, taking the
    /// random access point sitting in it with it — so the layer does not random-access on that
    /// decode path and `T` cannot enable it). This grounds out without further sender checks:
    /// the enabling random access point's own visibility is exactly "its temporal unit is
    /// decoded", because a layer random-accessing *at* a decoded temporal unit is decodable
    /// from there by definition (§ 7.4.1).
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

    /// Anchor-relative visibility (the model). `true` when (re)send event `event` is visible
    /// under a decode that starts at random access point `rap_tu` (§ 7.3.8.1 / § 7.4.6):
    ///
    /// - clause (a): the (re)send is in the random access point's own temporal unit
    ///   (`event.tu == rap_tu`, always decoded — § 7.4.1) AND its sending layer is decoded
    ///   under start-at-`rap_tu` (§ 7.4.6 sender-decodability, see
    ///   [`Self::sender_decodable_at`]); OR
    /// - clauses (b) + (c): a strictly-later (re)send is visible only when its temporal unit
    ///   carries no leading frame (§ 7.3.8.1 drops leading temporal units) and its sending
    ///   layer is decoded under start-at-`rap_tu` (§ 7.4.6 — see [`Self::sender_decodable_at`]).
    ///
    /// Clause (a)'s sender-decodability requirement is finding 1: even in the random access
    /// point's own temporal unit, a (re)send carried by a *non-global* layer that has no
    /// random access point in that temporal unit is not decoded under start-at-`rap_tu` —
    /// § 7.4.6: "the decoder shall not decode coded extended layer units for an extended layer
    /// until a random access point for that extended layer is encountered". For
    /// `event.tu == rap_tu` the closed-interval test `[rap_tu, rap_tu]` reduces to "the sending
    /// layer has its own random access point at `rap_tu`" (or the sender is global, decoded by
    /// whichever layer first random-accesses there), which covers the design sketch's "the
    /// sending layer IS the anchor's layer" case.
    pub(super) fn event_visible_at(&self, event: RapResendEvent, rap_tu: u64) -> bool {
        if event.tu == rap_tu {
            return self.sender_decodable_at(event.sending_xlayer, event.tu, rap_tu);
        }
        event.tu > rap_tu
            && !event.tu_has_any_leading
            && self.sender_decodable_at(event.sending_xlayer, event.tu, rap_tu)
    }

    /// The smallest random access point any *future* reference could be governed by: the
    /// minimum over every retained random-access-point history entry (`None` before any
    /// random access point). This is the earliest entry of [`Self::rap_history_any`], since
    /// that set is the union of the per-layer histories; equivalently, the global minimum
    /// first random access point still retained.
    ///
    /// **Why the *earliest* retained anchor, not the most recent (finding 2).** Under the
    /// every-anchor rule a future reference (at a temporal unit strictly after the current
    /// one) can be governed by *any* random access point that has occurred — every retained
    /// `R` is `<= refTU` for any future `refTU`. So the smallest governing anchor a future
    /// reference might use is the earliest retained anchor, and no event or history entry at
    /// or after it is dead. An event `S` strictly below this floor *is* dead: no retained
    /// anchor `R <= S.tu` exists, so clause (a)'s `S.tu == R` and clause (b)'s `S.tu > R` both
    /// fail for every retained (and therefore every future-governing) `R`. A history entry
    /// `T` strictly below the floor is dead too: it can serve only sender-decodability
    /// `range(R..=S.tu)` with `R >= floor`, which never scans below the floor. Because the
    /// earliest anchor itself is never pruned (it is a candidate governing anchor as long as
    /// it is retained), this floor advances only when no reference can ever again need the
    /// earliest anchor; in practice retained state is bounded by the random access points in
    /// the live window (streams have few per window). Correctness — never silencing a real
    /// violation for an older anchor — takes priority over a tighter memory bound.
    pub(super) fn anchor_floor(&self) -> Option<u64> {
        self.rap_history_any.keys().next().copied()
    }

    /// Resolves the § 7.3.8.1 replay rule for the just-completed temporal unit `tu_index`
    /// and resets the per-temporal-unit working state, returning the diagnostics to emit
    /// each paired with its dangling object's [`RapHlsKey`] (so the caller can apply the
    /// per-kind external-HLS suppression policy — see `complete_rap_replay_tu`).
    ///
    /// Order matters and is sound regardless of intra-unit OBU order: append this unit's
    /// (re)send events, advance the per-extended-layer / global random-access-point anchors
    /// and per-layer / any-layer random-access-point histories, then replay this unit's
    /// buffered references against *every* governing random access point (§ 7.3.8.1 "any
    /// random access point", finding 2) under the anchor-relative visibility predicate (which
    /// now sees this unit when it is itself a random access point), and finally prune state
    /// below the anchor floor.
    pub(super) fn complete_temporal_unit(&mut self, tu_index: u64) -> Vec<(RapHlsKey, Diagnostic)> {
        let tu_has_any_leading = !self.current_tu_leading_xlayers.is_empty();
        // Append this unit's resends as events, one per sending layer (all senders, not
        // last-writer-wins): per-anchor visibility filters them, so § 7.3.8.1's per-object
        // "any visible send suffices" is preserved even when a non-visible layer also
        // resends the same object here.
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
        // Advance the per-extended-layer and global random-access-point anchors and record
        // the per-layer / any-layer random-access-point histories (the governing anchors for
        // later references, finding 2, and § 7.4.6 sender-decodability). Each entry carries
        // this temporal unit's `tu_has_any_leading` so sender-decodability can tell whether a
        // random access point's own temporal unit is decoded under an earlier start
        // (round-5 finding; see [`Self::sender_decodable_at`]). The any-layer history
        // (governing GLOBAL_XLAYER_ID references) records this temporal unit whenever *any*
        // layer random-accesses here.
        if !self.current_tu_rap_xlayers.is_empty() {
            self.most_recent_rap_tu_any = Some(tu_index);
            self.rap_history_any.insert(tu_index, tu_has_any_leading);
            for &xlayer in &self.current_tu_rap_xlayers {
                self.most_recent_rap_tu.insert(xlayer, tu_index);
                self.rap_history
                    .entry(xlayer)
                    .or_default()
                    .insert(tu_index, tu_has_any_leading);
            }
        }

        let mut diagnostics = Vec::new();
        for pending in std::mem::take(&mut self.pending_this_tu) {
            // § 7.3.8.1 requires availability "if decoding process starts at ANY random access
            // point". So this reference must be satisfied under *every* governing anchor a
            // decoder might start from — every `R <= tu_index` random-accessing the reference's
            // governing layer (any layer for a global reference), not merely the most recent
            // (finding 2). A clause-(a) resend in a temporal unit that also carries leading
            // frames satisfies the newest anchor (that unit is its own start) yet is invisible
            // to an older anchor (under which the unit drops), so the most-recent anchor alone
            // can hide a real violation. The anchors are scanned smallest-first; the first
            // unsatisfied one is reported (the earliest violated start point is the most
            // actionable). Collected up front so the borrow of `self` ends before the
            // `self.emitted` mutation below (the anchor count per window is small).
            let governing_anchors: Vec<u64> = self
                .governing_rap_tus(pending.governing_xlayer, tu_index)
                .collect();
            // No random access point governs this reference yet (decoding from the bitstream
            // start needs no resend).
            for rap_tu in governing_anchors {
                // Moot when this reference's own temporal unit drops under start-at-rap_tu: a
                // strictly-later temporal unit carrying any leading frame is dropped wholesale
                // (§ 7.3.8.1), taking this reference with it — for a global referencing OBU
                // (e.g. a buffer-removal-timing OBU) just as for a frame-bearing one. The
                // random access point's own temporal unit (tu_index == rap_tu) is always
                // decoded (§ 7.4.1), so it is keyed to the governing anchor here.
                let reference_unit_drops = tu_index > rap_tu && tu_has_any_leading;
                if reference_unit_drops {
                    continue;
                }
                // Visible from the completed-unit events, or from a before-reference resend in
                // this unit (its event carries this unit's `tu_index` / leading-ness — built
                // here so the before-reference senders evaluate against the same predicate).
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
        // Prune events and random-access-point history strictly below the anchor floor (the
        // earliest retained random access point; see [`Self::anchor_floor`]). Such an entry
        // can never affect a future verdict: no retained — hence no future-governing — anchor
        // `R <= entry.tu` exists, so clause (a)'s `S.tu == R` and clause (b)'s `S.tu > R` both
        // fail, and sender-decodability `range(R..=S.tu)` (with `R >= floor`) never scans
        // below the floor. Pruning `rap_history_any` below its own minimum is a no-op (the
        // floor is that minimum); it is included only to keep the floor invariant explicit.
        // Under the every-anchor rule the floor advances only when the earliest anchor is no
        // longer a candidate governing anchor, so retained state is bounded by the random
        // access points in the live window — small for real streams (correctness over a
        // tighter bound).
        if let Some(floor) = self.anchor_floor() {
            for events in self.resend_events.values_mut() {
                events.retain(|event| event.tu >= floor);
            }
            self.resend_events.retain(|_, events| !events.is_empty());
            for history in self.rap_history.values_mut() {
                *history = history.split_off(&floor);
            }
            self.rap_history.retain(|_, history| !history.is_empty());
            self.rap_history_any = self.rap_history_any.split_off(&floor);
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
/// a kind the set cannot express ([`RapHlsKey::MultiFrameHeader`], and — once wired —
/// LCRs / atlas segments), any `Provided` mode keeps the blanket suppression, since such
/// an OBU may exist externally without being (or being expressible as) declared.
pub(super) fn rap_replay_suppressed_by_external_hls(
    key: RapHlsKey,
    external_hls: &ExternalHlsMode,
) -> bool {
    let ExternalHlsMode::Provided(set) = external_hls else {
        // Disabled: the caller asserts no external provision, so nothing is suppressed.
        return false;
    };
    match key {
        // Declarable kinds: authoritative exact-key match.
        RapHlsKey::SequenceHeader(id) => set.has_sequence_header(id),
        RapHlsKey::OperatingPointSet { xlayer, ops_id } => {
            set.has_operating_point_set(xlayer, ops_id)
        }
        // Inexpressible kinds: any Provided mode suppresses (partial-declaration policy).
        RapHlsKey::MultiFrameHeader(_)
        | RapHlsKey::LayerConfigurationRecord { .. }
        | RapHlsKey::Atlas { .. } => true,
    }
}

impl ValidatorContext {
    /// Buffers a linearly-resolved § 7.3.8.1 HLS reference for the random-access-point
    /// availability replay, governed by the referencing OBU's extended layer `xlayer`
    /// (resolved at temporal-unit completion; see [`RapReplayTracker`]). § 7.4 random
    /// access initiates per extended layer (§ 7.4.6), so a reference answers to its own
    /// layer's most recent random access point (a [`GLOBAL_XLAYER_ID`] reference answers
    /// to the global anchor). The caller buffers only references whose object was available
    /// in-band at reference time and not suppressed by external HLS, keeping the replay
    /// predicate disjoint from the linear `hls/unavailable-*` checks.
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

    /// Resolves the § 7.3.8.1 random-access-point HLS-availability replay for the
    /// just-completed temporal unit `completed_tu_index` and emits any replay
    /// diagnostics, gated on the partial-declaration external-HLS suppression policy.
    ///
    /// **External-HLS suppression (PR #49 policy, refined per-key — finding 3).**
    /// § 7.3.8.1's external-means escape — "When HLS OBUs are provided through external
    /// means, they remain available to the decoding process until superseded" — means an
    /// externally-provided object need not be resent at a random access point. The
    /// suppression under `ExternalHlsMode::Provided` is *per referenced key*, because
    /// [`ExternalHlsSet`] is authoritative for the kinds it can express:
    ///
    /// - For an externally-*declarable* kind — sequence headers
    ///   ([`ExternalHlsSet::with_sequence_header_id`]) and operating point sets
    ///   ([`ExternalHlsSet::with_operating_point_set`]) — the replay is suppressed only
    ///   when the *exact* referenced key is declared external. The caller's declaration is
    ///   authoritative for these kinds: a Provided set that does NOT list this
    ///   `seq_header_id` (resp. `(obu_xlayer_id, ops_id)`) is asserting it is not external,
    ///   so an in-band-only object dangling at a random access point still fires.
    /// - For a kind the set *cannot* express — multi-frame headers, LCRs, atlas segments —
    ///   any Provided mode keeps the blanket suppression: such an OBU MAY exist externally
    ///   unenumerated (`ExternalHlsMode::Provided` is a *partial* declaration), so firing
    ///   could be a false positive (zero-false-positive principle, AGENTS.md § 7).
    ///
    /// The default `Disabled` mode (the caller asserts no external provision) lets every
    /// replay fire. The pending references for the unit are always drained inside
    /// [`RapReplayTracker::complete_temporal_unit`], so the per-unit working state resets
    /// cleanly regardless of suppression.
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
            // Unresolved in-band: the linear `hls/unavailable-layer-configuration-record`
            // check owns this; do not replay (disjointness).
            return;
        };
        self.note_rap_reference(key, xlayer, obu.offset);
    }
}
