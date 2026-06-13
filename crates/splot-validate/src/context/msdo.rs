// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Multistream decoder operation observation and MSDO-scoped checks.

use super::*;

/// The § 6.6 sub-stream PTL ceilings of the most recently observed OBU_MSDO, indexed by
/// `sub_xlayer_id[i]` (AV2 v1.0.0 § 6.6, mirror `06-syntax-structures-semantics.md`
/// lines 1359-1378). A sequence header activated by the i-th independent sub-stream is
/// the header active for the extended layer whose `obu_xlayer_id` equals
/// `sub_xlayer_id[i]`; its `seq_profile_idc` / `seq_level_idx` / `seq_tier` must not
/// exceed the ceilings recorded here.
#[derive(Debug, Clone)]
pub(super) struct MsdoSubstreamMax {
    /// `sub_xlayer_id[i] → (sub_stream_max_profile[i], sub_stream_max_level[i],
    /// sub_stream_max_tier[i])`. § 6.6 imposes the ceiling "for each sequence header
    /// activated by the i-th independent sub-stream", i.e. for EACH i. The spec states no
    /// uniqueness requirement on `sub_xlayer_id` (see the proposal's roadmap-hygiene
    /// note), so two i values may name the same extended layer; a header activated by
    /// that layer must then satisfy both ceilings, so a duplicate `sub_xlayer_id` keeps
    /// the most restrictive (per-dimension minimum) maximum rather than letting a
    /// last-wins insert discard the tighter ceiling (recorded in `observe_msdo`; codex
    /// finding 3392940071).
    pub(super) ceilings: BTreeMap<u8, SubStreamCeiling>,
    /// `multistream_doh_constraint_flag` of the recorded MSDO, for the § 6.6
    /// DOH-constraint requirement (`msdo/doh-constraint-required`).
    pub(super) doh_constraint_flag: bool,
    /// Byte offset of the OBU_MSDO that declared these ceilings, for the diagnostic
    /// anchor when the violation is detected at sequence-header activation time.
    ///
    /// The § 6.8.2 MSDO↔global-LCR agreement uses the separate per-MSDO
    /// [`ValidatorContext::msdo_agreement_snapshots`] accumulator (it must evaluate EVERY
    /// MSDO in the CMVS, not just this live last-wins one), so the raw declaration-order
    /// aggregate / observation temporal unit are kept there rather than on this § 6.6 record.
    pub(super) offset: ByteOffset,
}

/// The § 6.6 MSDO aggregate fields and per-substream declaration-order entries kept for
/// the § 6.8.2 MSDO↔global-LCR agreement and the Table A.4 interoperability-point window
/// (AV2 § 5.6, mirror `06-syntax-structures-semantics.md` lines 1646-1673). Distinct from
/// the per-layer [`SubStreamCeiling`] merge: § 6.8.2 constraints 1/2/4 are per-declaration
/// (`num_streams_minus_2 + 1` entries, each carrying its `sub_xlayer_id[i]`), not the
/// most-restrictive per-layer view § 6.6 uses.
#[derive(Debug, Clone)]
pub(super) struct MsdoAggregate {
    /// `num_streams_minus_2 + 2` (AV2 § 5.6); the § 6.8.2 constraint-1 stream count and
    /// the Table A.3 extended-layer count (mirror lines 148-149).
    pub(super) num_streams: u32,
    /// `multistream_profile_idc` (AV2 § 5.6); the § 6.8.2 constraint-3 aggregate-profile
    /// value and the Table A.4 interoperability-point source (mirror lines 1659-1662).
    pub(super) profile_idc: u8,
    /// `multistream_level_idx` (AV2 § 5.6); § 6.8.2 constraint-3 level equality (line
    /// 1663).
    pub(super) level_idx: u8,
    /// `multistream_tier` (AV2 § 5.6); § 6.8.2 constraint-3 tier equality (line 1664).
    pub(super) tier: u8,
    /// `multistream_doh_constraint_flag` (AV2 § 5.6); § 6.8.2 constraint-5 DOH-flag equality
    /// (line 1673). Snapshotted with the rest of the declaration so the agreement check
    /// operates entirely on its `MsdoAggregate` argument rather than reaching back into the
    /// live `msdo_substream_max` (which a later same-CMVS MSDO could retarget).
    pub(super) doh_constraint_flag: bool,
    /// The per-declaration `sub_xlayer_id[i]` / `sub_stream_max_*[i]` entries in
    /// declaration order (`0..=num_streams_minus_2 + 1`), for § 6.8.2 constraints 2 and 4
    /// (lines 1651-1671).
    pub(super) sub_streams: Vec<MsdoSubStream>,
}

/// One accumulated OBU_MSDO snapshot for the § 6.8.2 MSDO↔global-LCR *agreement*
/// ([`ValidatorContext::msdo_agreement_snapshots`]). Distinct from the live last-wins
/// [`MsdoSubstreamMax`]: the agreement must hold for every MSDO present in the CMVS, so each
/// observed MSDO is retained (deduped by `offset`) and evaluated against the resolved
/// activated global LCR at deferred resolution.
#[derive(Debug, Clone)]
pub(super) struct MsdoWindowSnapshot {
    /// The § 6.8.2 aggregate / per-substream fields this MSDO declared.
    pub(super) aggregate: MsdoAggregate,
    /// Byte offset of the OBU_MSDO, for the diagnostic anchor and the dedup key.
    pub(super) offset: ByteOffset,
    /// The [`CvsTracker::tu_index`] at which this MSDO was observed, for the § 6.8.2 "present
    /// in the same CMVS" window filter (`>= cmvs_start_tu_index`).
    pub(super) observed_tu_index: u64,
}

/// One § 5.6 per-substream declaration (`sub_xlayer_id[i]` and the `sub_stream_max_*[i]`
/// PTL ceiling), kept in declaration order for the § 6.8.2 per-substream equality checks.
#[derive(Debug, Clone, Copy)]
pub(super) struct MsdoSubStream {
    /// `sub_xlayer_id[i]`.
    pub(super) sub_xlayer_id: u8,
    /// `sub_stream_max_profile[i]`.
    pub(super) max_profile: u8,
    /// `sub_stream_max_level[i]`.
    pub(super) max_level: u8,
    /// `sub_stream_max_tier[i]`.
    pub(super) max_tier: u8,
}

/// One sub-stream's § 6.6 PTL ceiling (`sub_stream_max_profile` / `sub_stream_max_level`
/// / `sub_stream_max_tier`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SubStreamCeiling {
    pub(super) max_profile: u8,
    pub(super) max_level: u8,
    pub(super) max_tier: u8,
}

/// Dedup key for the § 6.6 sub-stream PTL-ceiling findings: the activated header, its
/// coded-video-sequence epoch, the MSDO ceiling in force for the layer, and a
/// fingerprint of the activated header's checked value-space fields. A redefinition that
/// changes a checked field, or a new MSDO with a different ceiling, yields a distinct
/// key and re-emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SubstreamMaxFindingKey {
    pub(super) xlayer: ExtendedLayerId,
    pub(super) seq_header_id: SequenceHeaderId,
    pub(super) cvs_epoch: u64,
    pub(super) ceiling: SubStreamCeiling,
    pub(super) value_space: AnnexAValueSpaceFingerprint,
}

/// The six § 7.3.2 condition-2 key fields of a `multistream_decoder_operation_obu()`
/// (AV2 v1.0.0 § 7.3.2): a change in any of them at a coded-multistream-video-sequence
/// (CMVS) begin candidate begins a *new* CMVS while one is already active. Carrying
/// only these fields keeps the change comparison aligned with the exact spec list
/// ("the value of multistream_profile_idc, multistream_level_idx, multistream_tier,
/// num_streams_minus_2, multistream_even_allocation_flag, or
/// multistream_large_picture_idc differs from the corresponding value in the previous
/// OBU_MSDO"). `multistream_large_picture_idc` is `None` when
/// `multistream_even_allocation_flag` is set (§ 5.6), so the `Option` participates in
/// the comparison directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MsdoKeyFields {
    /// `multistream_profile_idc`.
    pub(super) profile_idc: u8,
    /// `multistream_level_idx`.
    pub(super) level_idx: u8,
    /// `multistream_tier`.
    pub(super) tier: u8,
    /// `num_streams_minus_2`.
    pub(super) num_streams_minus_2: u8,
    /// `multistream_even_allocation_flag`.
    pub(super) even_allocation_flag: bool,
    /// `multistream_large_picture_idc` (`None` under even allocation).
    pub(super) large_picture_idc: Option<u8>,
}

impl MsdoKeyFields {
    /// Projects the § 7.3.2 condition-2 key fields out of a parsed MSDO.
    pub(super) fn from_msdo(msdo: &MultistreamDecoderOperation) -> Self {
        Self {
            profile_idc: msdo.multistream_profile_idc.get(),
            level_idx: msdo.multistream_level_idx,
            tier: msdo.multistream_tier,
            num_streams_minus_2: msdo.num_streams_minus_2,
            even_allocation_flag: msdo.multistream_even_allocation_flag,
            large_picture_idc: msdo.multistream_large_picture_idc,
        }
    }
}

/// Outcome of observing one MSDO against the previous one (AV2 v1.0.0 § 7.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MsdoObservation {
    /// The first MSDO seen (no previous MSDO to compare against).
    First,
    /// An MSDO whose § 7.3.2 condition-2 key fields are unchanged from the previous
    /// OBU_MSDO ("an OBU with obu_type equal to OBU_MSDO that is not at a random
    /// access point shall be identical to the previous OBU_MSDO", § 7.3.8.2).
    Unchanged,
    /// An MSDO whose § 7.3.2 condition-2 key fields differ from the previous OBU_MSDO.
    Changed,
}

/// § 7.3.8.2 non-RAP MSDO-identity tracker (AV2 v1.0.0 § 7.3.8.2, mirror
/// `07-decoding-process.md` line 716): "an OBU with obu_type equal to OBU_MSDO that is
/// not at a random access point shall be identical to the previous OBU_MSDO."
///
/// § 7.3.7 places the at-most-one global MSDO before the frame OBUs of its temporal
/// unit, so whether the temporal unit is a random access point (§ 7.4.1: it contains a
/// CLK / OLK / RAS OBU) is only known once the temporal unit ends. The tracker therefore
/// buffers each temporal unit's MSDO payload fingerprint(s) and offset(s) plus the
/// temporal unit's accumulated random-access-point-ness, and resolves the identity rule
/// at temporal-unit completion ([`Self::complete_temporal_unit`]):
///
/// - A random-access-point temporal unit updates the reference fingerprint for each of
///   its MSDOs **without** a comparison ("at a random access point" is exempt).
/// - A non-random-access-point temporal unit compares each MSDO against the *previous*
///   OBU_MSDO and emits `msdo/non-rap-not-identical` (error) on a difference, then
///   advances the reference. With several MSDOs in one temporal unit the comparison is
///   pairwise-previous: the second MSDO compares against the first of the same temporal
///   unit (the spec's "the previous OBU_MSDO" is the immediately preceding one in
///   decode order). The reference advances per MSDO regardless of the verdict, so a
///   second identical MSDO after a flagged change is not re-flagged.
///
/// The full OBU payload fingerprint is used (the rule demands byte identity, "shall be
/// identical", not merely the § 7.3.2 condition-2 key fields).
#[derive(Debug, Default)]
pub(super) struct MsdoIdentityTracker {
    /// Payload fingerprint of the most recent OBU_MSDO resolved into the reference, or
    /// `None` until the first MSDO completes a temporal unit. The "previous OBU_MSDO"
    /// anchor for the next comparison.
    pub(super) previous: Option<u64>,
    /// MSDOs seen in the temporal unit currently being observed, in decode order:
    /// `(payload_fingerprint, offset)`.
    pub(super) current_tu: Vec<(u64, ByteOffset)>,
    /// Whether the temporal unit currently being observed is a § 7.4.1 random access
    /// point (contains a CLK / OLK / RAS OBU). Resolved at temporal-unit completion.
    pub(super) current_tu_is_rap: bool,
}

impl MsdoIdentityTracker {
    /// Buffers one parsed OBU_MSDO's payload fingerprint and offset for the temporal
    /// unit currently being observed (resolved at [`Self::complete_temporal_unit`]).
    pub(super) fn note_msdo(&mut self, fingerprint: u64, offset: ByteOffset) {
        self.current_tu.push((fingerprint, offset));
    }

    /// Marks the temporal unit currently being observed as a § 7.4.1 random access
    /// point (a CLK / OLK / RAS OBU was seen in it).
    pub(super) fn note_random_access_point(&mut self) {
        self.current_tu_is_rap = true;
    }

    /// Resolves the § 7.3.8.2 identity rule for the just-completed temporal unit and
    /// resets the per-temporal-unit working state. Called at each global temporal
    /// delimiter and once at end of stream for the final temporal unit (see
    /// [`ValidatorContext::finish`]).
    pub(super) fn complete_temporal_unit(&mut self, report: &mut ValidationReport) {
        let is_rap = self.current_tu_is_rap;
        for (fingerprint, offset) in std::mem::take(&mut self.current_tu) {
            if !is_rap
                && let Some(previous) = self.previous
                && previous != fingerprint
            {
                report.push(
                    Diagnostic::error(
                        "msdo/non-rap-not-identical",
                        "an OBU_MSDO in a temporal unit that is not a random access point \
                         (it contains no CLK / OLK / RAS OBU, § 7.4.1) differs from the previous \
                         OBU_MSDO; § 7.3.8.2 requires a non-random-access-point OBU_MSDO to be \
                         identical to the previous OBU_MSDO"
                            .to_string(),
                    )
                    .with_spec_section("7.3.8.2")
                    .with_byte_offset(offset),
                );
            }
            // The reference advances per MSDO regardless of the verdict (pairwise-previous
            // within the temporal unit; a RAP temporal unit advances without comparison).
            self.previous = Some(fingerprint);
        }
        self.current_tu_is_rap = false;
    }
}

/// Stateful § 5.6 MSDO observer (AV2 v1.0.0 § 5.6 / § 7.3.2).
///
/// The validator otherwise touches `OBU_MSDO` only for temporal-unit ordering
/// (`is_global_hls_prefix_obu`); this observer parses the payload and remembers the
/// last-seen MSDO's § 7.3.2 condition-2 key fields so the [`CmvsTracker`] can detect
/// the "differs from the corresponding value in the previous OBU_MSDO" condition. It
/// holds no diagnostics of its own.
#[derive(Debug, Default)]
pub(super) struct MsdoObserver {
    /// The § 7.3.2 condition-2 key fields of the most recently observed MSDO, or
    /// `None` until the first MSDO is seen.
    pub(super) last: Option<MsdoKeyFields>,
}

impl MsdoObserver {
    /// Records one parsed MSDO and reports how it relates to the previous one
    /// (AV2 v1.0.0 § 7.3.2 condition 2).
    pub(super) fn observe(&mut self, msdo: &MultistreamDecoderOperation) -> MsdoObservation {
        let fields = MsdoKeyFields::from_msdo(msdo);
        let outcome = match self.last {
            None => MsdoObservation::First,
            Some(previous) if previous == fields => MsdoObservation::Unchanged,
            Some(_) => MsdoObservation::Changed,
        };
        self.last = Some(fields);
        outcome
    }
}

impl ValidatorContext {
    /// Observes an `OBU_MSDO` (AV2 § 5.6): parses the payload, records its § 7.3.2
    /// condition-2 key fields in the stateful [`MsdoObserver`], forwards the
    /// observation to the [`CmvsTracker`] for the temporal unit currently being
    /// observed, records the § 6.6 sub-stream PTL ceilings for the agreement checks,
    /// buffers the § 7.3.8.2 identity fingerprint for this temporal unit, and re-runs
    /// the sub-stream PTL-ceiling agreement checks against every already
    /// frame-confirmed activation (the MSDO-arrives-after-the-header arrival order). A
    /// parse failure is silent — the structural MSDO syntax diagnostics are owned by
    /// the stateless check (AV2-5.6-MSDO), and the CMVS tracker treats an unparsable
    /// MSDO conservatively (no MSDO observation is recorded for the temporal unit, so
    /// no MSDO-driven begin condition fires).
    pub(super) fn observe_msdo(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(msdo) = parse_msdo(&mut reader) else {
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
        let observation = self.msdo.observe(&msdo);
        self.cmvs.note_msdo(observation);
        // AV2 § 7.3.2 / Annex A Table A.3: this OBU_MSDO sets MultiStreamDecoderMode == 1,
        // so num_streams_minus_2 + 2 is the declared Table A.3 extended-layer count and
        // multistream_profile_idc is the Table A.4 interoperability-point source for the
        // current temporal unit (committed to the right coded-video-sequence window at
        // temporal-unit completion).
        self.annex_a_iop.note_msdo(
            msdo.num_streams(),
            msdo.multistream_profile_idc.get(),
            obu.offset,
        );

        // AV2 § 7.3.8.2: buffer this MSDO's full-payload fingerprint and offset for the
        // temporal unit, resolved against the previous OBU_MSDO at temporal-unit end
        // (the TU's random-access-point-ness, § 7.4.1, is only known then). Multiple
        // MSDOs in one temporal unit each compare against their pairwise predecessor.
        self.msdo_identity
            .note_msdo(payload_fingerprint(obu.payload), obu.offset);

        // AV2 § 6.6: record the sub-stream PTL ceilings keyed by sub_xlayer_id, replacing
        // any earlier MSDO's ceilings. The live MSDO is the active multistream operation.
        //
        // § 6.6 states the ceiling constraints "for each sequence header activated by the
        // i-th independent sub-stream" — i.e. for EACH i in 0..=num_streams_minus_2+1.
        // The spec declares no uniqueness requirement on sub_xlayer_id (see the proposal's
        // roadmap-hygiene note), so two entries i and j may name the same extended layer.
        // A header activated by that layer is then "activated by the i-th sub-stream" for
        // BOTH i and j, so it must satisfy both ceilings; the effective per-layer ceiling
        // is the most restrictive (minimum) maximum per dimension. Merging by per-field
        // min on a duplicate sub_xlayer_id keeps that semantics; a plain last-wins insert
        // would silently discard the tighter of two declared ceilings (codex finding
        // 3392940071).
        let mut ceilings: BTreeMap<u8, SubStreamCeiling> = BTreeMap::new();
        for sub in msdo.sub_streams() {
            let declared = SubStreamCeiling {
                max_profile: sub.sub_stream_max_profile,
                max_level: sub.sub_stream_max_level,
                max_tier: sub.sub_stream_max_tier,
            };
            ceilings
                .entry(sub.sub_xlayer_id)
                .and_modify(|existing| {
                    existing.max_profile = existing.max_profile.min(declared.max_profile);
                    existing.max_level = existing.max_level.min(declared.max_level);
                    existing.max_tier = existing.max_tier.min(declared.max_tier);
                })
                .or_insert(declared);
        }
        // AV2 § 6.8.2: keep the raw declaration-order substream entries and the aggregate
        // PTL fields for the MSDO↔global-LCR agreement and the Table A.4 IOP window,
        // resolved at CMVS / coded-video-sequence boundaries.
        let aggregate = MsdoAggregate {
            num_streams: msdo.num_streams(),
            profile_idc: msdo.multistream_profile_idc.get(),
            level_idx: msdo.multistream_level_idx,
            tier: msdo.multistream_tier,
            doh_constraint_flag: msdo.multistream_doh_constraint_flag,
            sub_streams: msdo
                .sub_streams()
                .iter()
                .map(|sub| MsdoSubStream {
                    sub_xlayer_id: sub.sub_xlayer_id,
                    max_profile: sub.sub_stream_max_profile,
                    max_level: sub.sub_stream_max_level,
                    max_tier: sub.sub_stream_max_tier,
                })
                .collect(),
        };
        // AV2 § 6.8.2 agreement: accumulate this MSDO so EVERY MSDO present in the CMVS is
        // evaluated against the resolved activated global LCR — not just the live last-wins
        // one (codex finding 3393274380). Deduped by OBU offset so re-observing is idempotent.
        if !self
            .msdo_agreement_snapshots
            .iter()
            .any(|snapshot| snapshot.offset == obu.offset)
        {
            self.msdo_agreement_snapshots.push(MsdoWindowSnapshot {
                aggregate: aggregate.clone(),
                offset: obu.offset,
                observed_tu_index: self.cvs.tu_index,
            });
        }
        self.msdo_substream_max = Some(MsdoSubstreamMax {
            ceilings,
            doh_constraint_flag: msdo.multistream_doh_constraint_flag,
            offset: obu.offset,
        });

        // AV2 § 6.6: the MSDO-arrives-after-the-header arrival order — re-run the
        // sub-stream PTL-ceiling agreement against every extended layer with a
        // frame-confirmed activation, so a violation is flagged whether the MSDO precedes
        // or follows the activation. The activation-precedes-MSDO order is covered by the
        // calls in `on_sequence_activation`. The DOH-constraint check is NOT run here: it
        // is scoped to a coded multistream *video* sequence whose membership is not final
        // until the temporal unit completes, so it is deferred to
        // `resolve_deferred_doh_constraint` at temporal-unit boundary resolution (which
        // also covers the same-id CLK that opens a CMVS without re-activating a header,
        // codex finding 3392940072).
        let xlayers: Vec<ExtendedLayerId> = self.frame_confirmed_xlayers.iter().copied().collect();
        for xlayer in xlayers {
            self.check_substream_max_ceilings(xlayer, options, report);
        }
    }

    /// Emits the § 6.6 sub-stream PTL-ceiling errors
    /// (`msdo/substream-profile-exceeds-max` / `-level-` / `-tier-`) for the sequence
    /// header activated for `xlayer` against the most recently observed OBU_MSDO's
    /// `sub_stream_max_*` ceilings (mirror `06-syntax-structures-semantics.md` lines
    /// 1362-1378):
    ///
    /// > It is a requirement of bitstream conformance that seq_profile_idc /
    /// > seq_level_idx / seq_tier is less than or equal to sub_stream_max_profile[i] /
    /// > sub_stream_max_level[i] / sub_stream_max_tier[i] for each sequence header
    /// > activated by the i-th independent sub-stream.
    ///
    /// "Activated by the i-th independent sub-stream" is resolved through
    /// `sub_xlayer_id[i]`: the i-th sub-stream is the extended layer whose
    /// `obu_xlayer_id` equals `sub_xlayer_id[i]`, so the header active for `xlayer` is
    /// checked against the ceiling recorded under that key. An extended layer not named
    /// by any `sub_xlayer_id[i]` is not an independent sub-stream of this MSDO and has
    /// no ceiling — it is skipped.
    ///
    /// Gating mirrors the agreement checks exactly. The check fires only for a
    /// *frame-confirmed* activation (`frame_confirmed_xlayers`): the § 5.18.2
    /// `load_sequence_header` reference, never the OBU-order first-seen fallback (a
    /// § 7.3.6 staged-but-unactivated header could be superseded, and a ceiling error
    /// emitted against the guess could not be retracted). It is suppressed when external
    /// HLS declares a sequence header (`external_declares_sequence_header`): an
    /// out-of-band activated header carries unmodeled `seq_profile_idc` /
    /// `seq_level_idx` / `seq_tier`, so the in-band-ceiling comparison would be
    /// unreliable — the same split the OPS / § 6.4.1 agreement checks use. Locally the
    /// ceiling comes from the in-band MSDO and the activated header is the in-band one,
    /// so a Disabled-mode stream is fully checked.
    ///
    /// Idempotent across both arrival orders (activation re-confirmed by multiple
    /// frames; MSDO re-running it against already-confirmed layers) and across repeated
    /// activations of unchanged state via the [`SubstreamMaxFindingKey`] dedup, which
    /// carries the CVS epoch, the active ceiling, and a value-space fingerprint so a
    /// § 7.3.6 redefinition or a new MSDO ceiling re-emits.
    pub(super) fn check_substream_max_ceilings(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return;
        }
        let Some(substream_max) = self.msdo_substream_max.as_ref() else {
            return;
        };
        // The MSDO names sub-streams by obu_xlayer_id; only an extended layer named by a
        // sub_xlayer_id[i] is an independent sub-stream with a declared ceiling.
        let Some(&ceiling) = substream_max.ceilings.get(&xlayer.get()) else {
            return;
        };
        let msdo_offset = substream_max.offset;
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        // Anchor at the violating sequence header when its offset is known (matching
        // the annex-a/* value-space checks); the MSDO offset is the fallback for a
        // header whose defining OBU was not recorded.
        let anchor = self
            .sequence_header_offsets
            .get(&seq_header_id)
            .copied()
            .unwrap_or(msdo_offset);
        let epoch = self.cvs.cvs_generation_epoch(xlayer);
        let value_space = annex_a_value_space_fingerprint(&general);
        let key = SubstreamMaxFindingKey {
            xlayer,
            seq_header_id,
            cvs_epoch: epoch,
            ceiling,
            value_space,
        };
        if !self.emitted_substream_max.insert(key) {
            return;
        }

        let profile_idc = general.seq_profile_idc.get();
        let level_idx = general.seq_level_idx.get();
        let tier = u8::from(matches!(general.seq_tier, Tier::High));

        if profile_idc > ceiling.max_profile {
            report.push(
                Diagnostic::error(
                    "msdo/substream-profile-exceeds-max",
                    format!(
                        "the sequence header activated for sub-stream obu_xlayer_id {} has \
                         seq_profile_idc {profile_idc}, exceeding the MSDO's \
                         sub_stream_max_profile {} for that sub-stream (§ 6.6)",
                        xlayer.get(),
                        ceiling.max_profile,
                    ),
                )
                .with_spec_section("6.6")
                .with_byte_offset(anchor),
            );
        }
        if level_idx > ceiling.max_level {
            report.push(
                Diagnostic::error(
                    "msdo/substream-level-exceeds-max",
                    format!(
                        "the sequence header activated for sub-stream obu_xlayer_id {} has \
                         seq_level_idx {level_idx}, exceeding the MSDO's sub_stream_max_level \
                         {} for that sub-stream (§ 6.6)",
                        xlayer.get(),
                        ceiling.max_level,
                    ),
                )
                .with_spec_section("6.6")
                .with_byte_offset(anchor),
            );
        }
        if tier > ceiling.max_tier {
            report.push(
                Diagnostic::error(
                    "msdo/substream-tier-exceeds-max",
                    format!(
                        "the sequence header activated for sub-stream obu_xlayer_id {} has \
                         seq_tier {tier}, exceeding the MSDO's sub_stream_max_tier {} for that \
                         sub-stream (§ 6.6)",
                        xlayer.get(),
                        ceiling.max_tier,
                    ),
                )
                .with_spec_section("6.6")
                .with_byte_offset(anchor),
            );
        }
    }

    /// Emits `msdo/doh-constraint-required` (error, § 6.6) when, with the § 7.3.2 CMVS
    /// tracker definitively *inside* a coded multistream video sequence, the sequence
    /// header just activated for `xlayer` has `monotonic_output_order_flag == 0` while
    /// the recorded MSDO has `multistream_doh_constraint_flag == 0` (mirror
    /// `06-syntax-structures-semantics.md` lines 1391-1393):
    ///
    /// > It is a requirement of bitstream conformance that when
    /// > monotonic_output_order_flag is equal to 0 in any activated sequence header of
    /// > the coded multistream video sequence, multistream_doh_constraint_flag shall be
    /// > equal to 1.
    ///
    /// **CMVS membership is resolved by the deferred evaluation, not gated here.** § 6.6
    /// scopes the requirement to a coded multistream *video* sequence, but the temporal
    /// unit's CMVS membership is not final until its temporal unit completes: a same-id
    /// header redefinition at the top of a temporal unit that a later MSDO-less CLK ENDS
    /// (§ 7.3.2 end condition 2) sits *outside* the CMVS even though the committed state
    /// is still `Inside` when the header activates (codex finding 3392940061). And a
    /// same-id CLK that re-references an already-active header opens the CMVS at the CLK
    /// without re-entering `on_sequence_activation` (the seq id is unchanged and the layer
    /// was already frame-confirmed), so an eager activation-time check never sees the
    /// transition (codex finding 3392940072). Both are handled by routing this check
    /// through [`ValidatorContext::resolve_deferred_doh_constraint`], which runs at
    /// temporal-unit completion against the *resolved* membership and the then-current
    /// frame-confirmed activations — the same membership-resolution discipline the landed
    /// § 6.4.1 monotonic-output-order agreement check uses, snapshotting the active
    /// headers at resolution time. This method therefore performs the per-layer field
    /// comparison only; the caller guarantees the temporal unit resolved to a definitive
    /// CMVS `Inside`.
    ///
    /// Frame-confirmed activations only, suppressed under
    /// `external_declares_sequence_header`, and idempotent across temporal units and both
    /// arrival orders via the `(xlayer, seq_header_id, cvs_epoch)` dedup so a resolved
    /// evaluation does not re-spam per temporal unit.
    pub(super) fn check_doh_constraint_required(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return;
        }
        let Some(substream_max) = self.msdo_substream_max.as_ref() else {
            return;
        };
        let msdo_offset = substream_max.offset;
        // multistream_doh_constraint_flag == 1 already satisfies the requirement; the
        // flag travels with the recorded MSDO state (it is not a § 7.3.2 key field).
        if substream_max.doh_constraint_flag {
            return;
        }
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        if general.monotonic_output_order_flag {
            return;
        }
        let epoch = self.cvs.cvs_generation_epoch(xlayer);
        if !self
            .emitted_doh_constraint
            .insert((xlayer, seq_header_id, epoch))
        {
            return;
        }
        report.push(
            Diagnostic::error(
                "msdo/doh-constraint-required",
                format!(
                    "the sequence header activated for extended layer {} has \
                     monotonic_output_order_flag == 0 inside a coded multistream video sequence, \
                     but the MSDO's multistream_doh_constraint_flag == 0; § 6.6 requires \
                     multistream_doh_constraint_flag == 1 when any activated sequence header has \
                     monotonic_output_order_flag == 0",
                    xlayer.get(),
                ),
            )
            .with_spec_section("6.6")
            .with_byte_offset(msdo_offset),
        );
    }
}
