// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Multistream decoder operation observation and MSDO-scoped checks.

use super::*;

/// Latest MSDO's per-layer § 6.6 PTL ceilings
/// (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md`).
/// A sequence activated by substream i belongs to sub_xlayer_id[i].
#[derive(Debug, Clone)]
pub(super) struct MsdoSubstreamMax {
    /// Per-layer ceiling. Duplicate sub_xlayer_id values retain the minimum
    /// in each PTL dimension: § 6.6 binds every declaration and requires no uniqueness.
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
#[allow(clippy::struct_field_names)]
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

/// § 7.3.8.2 non-RAP MSDO identity, resolved at TU completion because a later
/// CLK/OLK/RAS can make the TU a RAP. Compare full payload fingerprints, not only
/// CMVS key fields. At a RAP, update without comparison; elsewhere compare with
/// the immediately previous MSDO and advance even after a mismatch.
/// Source: docs/spec/av2/1.0.0/07-decoding-process.md § 7.3.8.2.
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
    /// Parses MSDO, records CMVS/IOP facts, payload identity and per-layer ceilings,
    /// then rechecks already-confirmed activations for MSDO-after-header ordering.
    /// Malformed payloads remain silent here; stateless MSDO syntax checks report them.
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
        self.annex_a_iop.note_msdo(
            msdo.num_streams(),
            msdo.multistream_profile_idc.get(),
            obu.offset,
        );

        self.msdo_identity
            .note_msdo(payload_fingerprint(obu.payload), obu.offset);

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
        let aggregate = MsdoAggregate {
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
        if !self
            .msdo_agreement_snapshots
            .iter()
            .any(|snapshot| snapshot.offset == obu.offset)
        {
            self.msdo_agreement_snapshots.push(MsdoWindowSnapshot {
                aggregate,
                offset: obu.offset,
                observed_tu_index: self.cvs.tu_index,
            });
        }
        self.msdo_substream_max = Some(MsdoSubstreamMax {
            ceilings,
            doh_constraint_flag: msdo.multistream_doh_constraint_flag,
            offset: obu.offset,
        });

        let xlayers: Vec<ExtendedLayerId> = self.frame_confirmed_xlayers.iter().copied().collect();
        for xlayer in xlayers {
            self.check_substream_max_ceilings(xlayer, options, report);
        }
    }

    /// Checks activated sequence PTL against the latest MSDO's § 6.6 ceilings.
    /// Only named substreams with frame-confirmed in-band activations participate;
    /// external sequence declarations suppress the check. Dedup includes CVS epoch,
    /// ceiling and value-space fingerprint so changed declarations re-emit.
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
        let Some(&ceiling) = substream_max.ceilings.get(&xlayer.get()) else {
            return;
        };
        let msdo_offset = substream_max.offset;
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
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

    /// Checks § 6.6 DOH flag agreement for one frame-confirmed in-band activation.
    /// The caller resolves CMVS membership at TU completion: eager activation-time
    /// checks cannot account for a later MSDO-less CLK ending the CMVS or a same-id
    /// CLK beginning one without reactivation. External sequence declarations
    /// suppress the check; dedup is per layer/header/CVS epoch.
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
