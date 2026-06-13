// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Layer-configuration-record observation and association state.

use super::*;

/// One in-band global layer configuration record's § 6.8.2 agreement fields (AV2
/// § 5.8.1): the aggregate info, per-substream PTL info indexed by `obu_xlayer_id`, the
/// DOH-constraint flag, and the defining OBU's byte offset for the diagnostic anchor.
/// Stored alongside the [`HlsAvailabilityStore`]'s `lcr_xlayer_map` (which the
/// availability / association chain consumes) so the MSDO↔global-LCR agreement can read
/// the full record of whichever global LCR the chain resolved as *activated*.
#[derive(Debug, Clone)]
pub(super) struct GlobalLcrRecord {
    /// `LcrMaxNumXLayerCount` = the set-bit count of `lcr_xlayer_map` (AV2 § 5.8.1,
    /// mirror `06-syntax-structures-semantics.md` lines 382-384): the § 6.8.2 constraint-1
    /// stream count and the Table A.3 extended-layer count under an activated global LCR.
    pub(super) max_num_xlayer_count: u32,
    /// `LcrXLayerID[]` = the set-bit indices of `lcr_xlayer_map`, ascending (AV2 § 5.8.1):
    /// the § 6.8.2 constraint-2 membership set.
    pub(super) xlayer_ids: BTreeSet<u8>,
    /// `lcr_aggregate_info()` when `lcr_aggregate_info_present_flag == 1` (§ 6.8.2
    /// constraint 3, lines 1657-1664).
    pub(super) aggregate_info: Option<LcrAggregateInfo>,
    /// `lcr_seq_profile_idc[i]` / `lcr_max_level_idx[i]` / `lcr_tier_flag[i]` indexed by
    /// `obu_xlayer_id` (i), present when `lcr_seq_profile_tier_level_info_present_flag ==
    /// 1` (§ 6.8.2 constraint 4 (the "1." numbered as constraint 4), lines 1666-1671).
    pub(super) seq_ptl_by_xlayer: BTreeMap<u8, LcrSeqPtl>,
    /// `lcr_seq_profile_tier_level_info_present_flag`.
    pub(super) seq_ptl_present: bool,
    /// `lcr_doh_constraint_flag` (§ 6.8.2 constraint 5 and the § 6.8.2 DOH requirement,
    /// lines 1619-1621 / 1673).
    pub(super) doh_constraint_flag: bool,
    /// Byte offset of the OBU that defined this record, for the diagnostic anchor.
    pub(super) offset: ByteOffset,
    /// The [`CvsTracker::tu_index`] at which this record's defining OBU was observed (a
    /// redefinition restamps it). The § 6.8.2 agreement applies only when the global LCR is
    /// "present in the same coded multistream video sequence" (mirror lines 1646-1648): the
    /// snapshot of this record taken at association time carries its observation temporal
    /// unit, and the deferred resolution requires that temporal unit to lie within the
    /// current CMVS window (`>= cmvs_start_tu_index`) so a record observed only in an
    /// earlier CMVS does not leak into a later MSDO-only CMVS's evaluation (codex finding
    /// 3393129738).
    pub(super) observed_tu_index: u64,
}

/// One global LCR's `lcr_seq_profile_tier_level_info(i)` PTL ceiling, indexed by
/// `obu_xlayer_id` in [`GlobalLcrRecord::seq_ptl_by_xlayer`] (AV2 § 5.8.4).
#[derive(Debug, Clone, Copy)]
pub(super) struct LcrSeqPtl {
    /// `lcr_seq_profile_idc[i]`.
    pub(super) seq_profile_idc: u8,
    /// `lcr_max_level_idx[i]`.
    pub(super) max_level_idx: u8,
    /// `lcr_tier_flag[i]` as `0`/`1`.
    pub(super) tier_flag: u8,
}

/// The § 6.4.1 LCR association of one observed sequence header; see
/// [`ValidatorContext::lcr_associations`].
#[derive(Debug, Clone)]
pub(super) struct LcrAssociation {
    /// `true` when the association resolved to a global LCR (no local record
    /// with the id existed in the header's extended layer at observation).
    pub(super) lcr_is_global: bool,
    /// The associated record's id (the header's `seq_lcr_id`).
    pub(super) lcr_id: u8,
    /// The record's § 5.8.8 embedded-layer maps at observation time; `None`
    /// when it carried no embedded-layer info (§ 6.8.9 binds "if present").
    pub(super) maps: Option<LcrEmbeddedMaps>,
    /// The full § 6.8.2 agreement fields of the associated *global* LCR as observed at
    /// association time (a clone of the [`GlobalLcrRecord`] present prior to this header),
    /// or `None` when the association is local. The § 6.8.2 MSDO↔global-LCR agreement and
    /// DOH requirement read this snapshot rather than the live `global_lcr_records` map, so
    /// a same-id global-LCR redefinition *after* this header associated does not retarget
    /// the agreement at the later revision (codex finding 3393129741) — mirroring the
    /// existing § 6.8.9 dependency path, which also snapshots its associated maps.
    pub(super) global_record: Option<GlobalLcrRecord>,
    /// The § 5.8.4 PTL declared maxima of the associated LCR for this extended layer,
    /// snapshotted at association time for the § 6.8.5 ceiling checks. A *local*
    /// association reads the local record's `lcr_seq_profile_tier_level_info(xlayerId)`
    /// (the § 6.8.5 keying record — the sentence names the local LCR); a *global*
    /// association reads the global record's `lcr_seq_profile_tier_level_info(i)` for
    /// this xlayer. `None` when the associated record carried no PTL info for the layer
    /// (§ 6.8.5 "when ... present in an activated LCR"; absent PTL compares nothing).
    /// Snapshotting (rather than a live lookup) keeps the comparison pinned to the
    /// record revision this header associated to, exactly like [`Self::global_record`]
    /// and [`Self::maps`].
    pub(super) ptl: Option<LcrPtlSnapshot>,
    /// The § 5.8.7 rep info of the associated LCR for this extended layer, snapshotted
    /// at association time for the § 6.8.8 equality checks (local record's
    /// `lcr_rep_info(0, xId)` for a local association, the global payload's
    /// `lcr_rep_info(1, xId)` for a global one). `None` when the associated record
    /// carried no rep info for the layer (absent rep-info compares nothing).
    pub(super) rep_info: Option<LcrRepInfoSnapshot>,
}

/// The § 5.8.8 embedded-layer maps of one LCR `lcr_xlayer_info` entry, retained for
/// the § 6.8.9 dependency-map agreement checks, plus the defining LCR OBU's byte
/// offset — the § 6.8.9 diagnostic points at the LCR OBU, not at the activating
/// sequence header or frame.
#[derive(Debug, Clone)]
pub(super) struct LcrEmbeddedMaps {
    /// `lcr_mlayer_map[isGlobal][xId]`.
    pub(super) mlayer_map: u8,
    /// `(embedded layer index, lcr_tlayer_map[isGlobal][xId][j])` pairs, in
    /// ascending set-bit order of `lcr_mlayer_map`.
    pub(super) tlayer_maps: Vec<(u8, u8)>,
    /// `(embedded layer index j, lcr_max_expected_width[..][j], lcr_max_expected_height)`
    /// per embedded layer (AV2 § 5.8.8 / § 6.8.9), present only when
    /// `lcr_same_sh_max_resolution_flag == 0` (otherwise the width/height default to the
    /// sequence maxima and the § 6.8.9 sequence-max bound is satisfied trivially, so a
    /// `None` width/height entry is omitted from the bound check). Retained for the § 6.8.9
    /// `lcr_max_expected_width/height <= max_frame_width/height_minus_1 + 1` bound against
    /// the activated sequence header.
    pub(super) max_expected: Vec<(u8, Option<u32>, Option<u32>)>,
    /// Byte offset of the defining LCR OBU.
    pub(super) offset: ByteOffset,
}

/// One LCR's `lcr_seq_profile_tier_level_info(i)` declared maxima (AV2 § 5.8.4 /
/// § 6.8.5), snapshotted for the § 6.8.5 PTL-ceiling agreement plus the defining LCR
/// OBU's byte offset (the diagnostic anchors at the LCR OBU when more informative than
/// the activating header). All four maxima are the LCR-declared ceilings the activated
/// sequence header's PTL must not exceed (`<=`, equality passes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LcrPtlSnapshot {
    /// `lcr_seq_profile_idc[i]`.
    pub(super) seq_profile_idc: u8,
    /// `lcr_max_level_idx[i]`.
    pub(super) max_level_idx: u8,
    /// `lcr_tier_flag[i]` as `0`/`1`.
    pub(super) tier_flag: u8,
    /// `lcr_max_mlayer_count[i]`.
    pub(super) max_mlayer_count: u8,
    /// Byte offset of the defining LCR OBU.
    pub(super) offset: ByteOffset,
}

/// One LCR `lcr_rep_info(isGlobal, xId)` entry's representation info (AV2 § 5.8.7 /
/// § 6.8.8), snapshotted for the § 6.8.8 rep-info equality agreement plus the defining
/// LCR OBU's byte offset. `format` / `cropping` mirror the parsed `Option`s: a missing
/// `lcr_format_info_present_flag` / `lcr_cropping_window_present_flag` leaves the
/// corresponding field `None`, and the § 6.8.8 comparisons that gate on those flags
/// compare nothing when absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LcrRepInfoSnapshot {
    /// `lcr_max_pic_width[isGlobal][xId]` (always present).
    pub(super) max_pic_width: u32,
    /// `lcr_max_pic_height[isGlobal][xId]` (always present).
    pub(super) max_pic_height: u32,
    /// `(lcr_bit_depth_idc, lcr_chroma_format_idc)`, present when
    /// `lcr_format_info_present_flag == 1`.
    pub(super) format: Option<(u32, u32)>,
    /// The four `lcr_cropping_win_*_offset` values, present when
    /// `lcr_cropping_window_present_flag == 1` (the present flag itself is the
    /// `Option::is_some`). Order: `(left, right, top, bottom)`.
    pub(super) cropping: Option<(u32, u32, u32, u32)>,
    /// Byte offset of the defining LCR OBU.
    pub(super) offset: ByteOffset,
}

/// Which dependency map a § 6.10.7 / § 6.8.9 agreement finding concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum DependencyMapKind {
    /// `MLayerDependencyMap` closure of an embedded-layer bitmask.
    Mlayer,
    /// `TLayerDependencyMap` closure of the temporal-layer bitmask signalled for
    /// embedded layer `mlayer`.
    Tlayer { mlayer: u8 },
}

/// Dedup key for an emitted layer-dependency agreement finding: the same
/// `(violating OBU, entry, activated sequence header, map)` pairing fires at most
/// once even when re-activation re-runs the checks. A different activated
/// sequence-header *id*, or a different defining OPS/LCR OBU (distinguished by
/// its byte offset), gets a distinct key; a same-id sequence-header
/// *redefinition* instead invalidates the id's keys (see
/// `observe_sequence_header`) so the re-fired checks can report against the new
/// content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum DependencyFindingKey {
    /// A § 6.10.7 finding, keyed by the OPS OBU's offset and entry coordinates.
    Ops {
        ops_offset: ByteOffset,
        payload_index: u8,
        entry_xlayer: ExtendedLayerId,
        seq_header_id: SequenceHeaderId,
        map: DependencyMapKind,
    },
    /// A § 6.8.9 finding, keyed by the activated pairing coordinates and the
    /// defining LCR OBU's offset (a redefined LCR is a distinct violating OBU).
    Lcr {
        xlayer: ExtendedLayerId,
        seq_header_id: SequenceHeaderId,
        lcr_is_global: bool,
        lcr_id: u8,
        lcr_offset: ByteOffset,
        map: DependencyMapKind,
    },
}

impl DependencyFindingKey {
    /// The activated sequence header this finding was paired with.
    pub(super) fn seq_header_id(self) -> SequenceHeaderId {
        match self {
            Self::Ops { seq_header_id, .. } | Self::Lcr { seq_header_id, .. } => seq_header_id,
        }
    }
}

/// Which § 6.8.5 PTL ceiling a finding constrains; part of [`LcrPtlFindingKey`] so the
/// four sub-rules are deduped independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum LcrPtlField {
    /// `lcr/ptl-profile-exceeds-max`.
    Profile,
    /// `lcr/ptl-level-exceeds-max`.
    Level,
    /// `lcr/ptl-tier-exceeds-max`.
    Tier,
    /// `lcr/ptl-mlayer-count-exceeds-max`.
    MlayerCount,
}

/// Dedup key for an emitted § 6.8.5 PTL-ceiling finding: the activated pairing
/// coordinates, the defining LCR OBU's offset (a redefined LCR is a distinct violating
/// OBU), the ceiling sub-field, and a content fingerprint of both the LCR-declared
/// maximum and the activated header's value. A non-identical LCR redefinition (new
/// offset / changed maximum) or a same-id sequence-header reconfiguration (changed
/// header value) yields a distinct key and re-emits; an identical re-evaluation is
/// idempotent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LcrPtlFindingKey {
    pub(super) xlayer: ExtendedLayerId,
    pub(super) seq_header_id: SequenceHeaderId,
    pub(super) lcr_is_global: bool,
    pub(super) lcr_id: u8,
    pub(super) lcr_offset: ByteOffset,
    pub(super) field: LcrPtlField,
    /// The LCR-declared maximum in force (a redefinition with a new offset already
    /// yields a distinct key; this is kept for content symmetry with the header value).
    pub(super) lcr_max: u32,
    /// The activated header's compared value.
    pub(super) header_value: u32,
}

/// Which § 6.8.8 rep-info field a finding constrains; part of [`LcrRepInfoFindingKey`]
/// so the sub-fields are deduped independently and named in the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum LcrRepInfoField {
    /// `lcr_max_pic_width` vs `max_frame_width_minus_1 + 1`.
    Width,
    /// `lcr_max_pic_height` vs `max_frame_height_minus_1 + 1`.
    Height,
    /// `lcr_bit_depth_idc` vs `bit_depth_idc`.
    BitDepth,
    /// `lcr_chroma_format_idc` vs `chroma_format_idc`.
    ChromaFormat,
    /// `lcr_cropping_window_present_flag` vs `seq_cropping_window_present_flag`.
    CroppingPresent,
    /// `lcr_cropping_win_left_offset` vs `seq_cropping_win_left_offset`.
    CropLeft,
    /// `lcr_cropping_win_right_offset` vs `seq_cropping_win_right_offset`.
    CropRight,
    /// `lcr_cropping_win_top_offset` vs `seq_cropping_win_top_offset`.
    CropTop,
    /// `lcr_cropping_win_bottom_offset` vs `seq_cropping_win_bottom_offset`.
    CropBottom,
}

/// Dedup key for an emitted § 6.8.8 rep-info mismatch finding; see
/// [`LcrPtlFindingKey`] for the dedup discipline. The LCR value and header value fold a
/// content change into a distinct key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LcrRepInfoFindingKey {
    pub(super) xlayer: ExtendedLayerId,
    pub(super) seq_header_id: SequenceHeaderId,
    pub(super) lcr_is_global: bool,
    pub(super) lcr_id: u8,
    pub(super) lcr_offset: ByteOffset,
    pub(super) field: LcrRepInfoField,
    pub(super) lcr_value: u64,
    pub(super) header_value: u64,
}

/// Dedup key for an emitted § 6.8.9 `lcr_max_expected_width/height` sequence-max bound
/// finding; see [`LcrPtlFindingKey`] for the dedup discipline. The embedded-layer index,
/// the LCR-declared expected dimension, and the activated header maximum fold a content
/// change into a distinct key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LcrExpectedDimsFindingKey {
    pub(super) xlayer: ExtendedLayerId,
    pub(super) seq_header_id: SequenceHeaderId,
    pub(super) lcr_is_global: bool,
    pub(super) lcr_id: u8,
    pub(super) lcr_offset: ByteOffset,
    /// The embedded layer index `j` the bound constrains.
    pub(super) mlayer_index: u8,
    /// `true` for the width bound, `false` for the height bound.
    pub(super) is_width: bool,
    /// The LCR-declared `lcr_max_expected_width/height[..][j]`.
    pub(super) lcr_value: u32,
    /// The activated header's `max_frame_width/height_minus_1 + 1`.
    pub(super) header_max: u32,
}

/// Scans an 8-bit embedded-layer bitmask for the first § 6.10.7 / § 6.8.9 closure
/// violation under `MLayerDependencyMap`: a set bit `cMId` for which the map
/// requires a dependency `rMId < cMId` (`MLayerDependencyMap[cMId][rMId] == 1`)
/// whose bit is not set. Returns the violating `(cMId, rMId)` pair.
pub(super) fn mlayer_closure_violation(mask: u8, m_map: &MLayerDependencyMap) -> Option<(u8, u8)> {
    for curr in 0u8..8 {
        if mask & (1u8 << curr) == 0 {
            continue;
        }
        for reference in 0..curr {
            if m_map.depends_on(
                EmbeddedLayerId::from_bits(curr),
                EmbeddedLayerId::from_bits(reference),
            ) && mask & (1u8 << reference) == 0
            {
                return Some((curr, reference));
            }
        }
    }
    None
}

/// Scans one embedded layer's 4-bit temporal-layer bitmask for the first § 6.10.7
/// / § 6.8.9 closure violation under `TLayerDependencyMap[mlayer]` — the same
/// shape as [`mlayer_closure_violation`]. Returns the violating `(cTId, rTId)`
/// pair.
pub(super) fn tlayer_closure_violation(
    mlayer: u8,
    mask: u8,
    t_map: &TLayerDependencyMap,
) -> Option<(u8, u8)> {
    for curr in 0u8..4 {
        if mask & (1u8 << curr) == 0 {
            continue;
        }
        for reference in 0..curr {
            if t_map.depends_on(
                EmbeddedLayerId::from_bits(mlayer),
                TemporalLayerId::from_bits(curr),
                TemporalLayerId::from_bits(reference),
            ) && mask & (1u8 << reference) == 0
            {
                return Some((curr, reference));
            }
        }
    }
    None
}

/// Builds an [`LcrRepInfoSnapshot`] from a parsed `lcr_rep_info()` and the defining LCR
/// OBU's byte offset (AV2 § 5.8.7), mapping the parsed `format_info` / `cropping_window`
/// `Option`s straight through — a missing `lcr_format_info_present_flag` /
/// `lcr_cropping_window_present_flag` leaves the snapshot field `None`, and the § 6.8.8
/// comparisons gated on those flags compare nothing when absent.
pub(super) fn rep_info_snapshot(rep_info: &LcrRepInfo, offset: ByteOffset) -> LcrRepInfoSnapshot {
    LcrRepInfoSnapshot {
        max_pic_width: rep_info.max_pic_width,
        max_pic_height: rep_info.max_pic_height,
        format: rep_info
            .format_info
            .map(|f| (f.bit_depth_idc, f.chroma_format_idc)),
        cropping: rep_info
            .cropping_window
            .map(|c| (c.left_offset, c.right_offset, c.top_offset, c.bottom_offset)),
        offset,
    }
}

impl ValidatorContext {
    /// Observes a layer configuration record OBU: records its in-band availability and
    /// checks a local record's references to a global LCR (AV2 § 7.3.8.3) and a local
    /// atlas segment OBU (AV2 § 7.3.8.4). Availability is recorded only after a
    /// successful parse and a valid § 5.2.1 payload tail, mirroring the sequence-header
    /// and multi-frame-header observers.
    pub(super) fn observe_layer_config_record(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures and syntax diagnostics are handled by the stateless
        // LayerConfigRecordSyntax check; here we only act on a successful parse.
        let Ok(record) = parse_layer_config_record(&mut reader, obu.header.extended_layer_id)
        else {
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

        let xlayer = obu.header.extended_layer_id;
        let external_disabled = matches!(options.external_hls, ExternalHlsMode::Disabled);
        match record {
            LayerConfigurationRecord::Global(info) => {
                // AV2 § 7.3.2 condition 3 / end condition 2: a global layer
                // configuration record OBU is present in this temporal unit. Whether it
                // is *activated* needs § 7.3.8 activation state the validator does not
                // model, so the CMVS tracker only treats this as an "activation cannot
                // be ruled out" signal and routes the affected boundary transitions to
                // CmvsState::Unknown rather than guessing.
                self.cmvs.note_global_lcr_present();
                // Annex A Table A.4: a global LCR OBU is present in this temporal unit
                // (raw presence; the *activated*-global-LCR distinction needed by the
                // Table A.4 global-LCR arms is resolved from the association chain at the
                // window flush, not here).
                self.annex_a_iop.note_global_lcr(obu.offset);
                // AV2 § 7.3.8.3: record the global LCR's id and xlayer map for later
                // local-LCR and sequence-header references.
                self.hls
                    .record_global_lcr(info.global_config_record_id, info.xlayer_map);
                // AV2 § 7.3.8.1: note this global LCR's in-band (re)send (global extended
                // layer) for the random-access-point availability replay, so a local LCR's
                // lcr_global_id or a sequence header's seq_lcr_id referencing it at a
                // random access point must find it resent there.
                self.rap_replay.note_resend(
                    RapHlsKey::LayerConfigurationRecord {
                        xlayer: GLOBAL_XLAYER_ID.get(),
                        id: info.global_config_record_id,
                    },
                    obu.header.extended_layer_id,
                );
                // AV2 § 6.8.2: keep the full aggregate / per-substream PTL / DOH fields of
                // this global LCR (keyed by id, redefinition overwrites) so the
                // MSDO↔global-LCR agreement can read whichever record the association chain
                // later resolves as *activated*. LcrXLayerID[] / LcrMaxNumXLayerCount come
                // from the set bits of lcr_xlayer_map (§ 5.8.1, mirror lines 382-384).
                let xlayer_ids: BTreeSet<u8> = (0u8..31)
                    .filter(|i| info.xlayer_map & (1 << i) != 0)
                    .collect();
                let mut seq_ptl_by_xlayer: BTreeMap<u8, LcrSeqPtl> = BTreeMap::new();
                for ptl in &info.seq_ptl_infos {
                    seq_ptl_by_xlayer.insert(
                        ptl.xlayer_id,
                        LcrSeqPtl {
                            seq_profile_idc: ptl.seq_profile_idc,
                            max_level_idx: ptl.max_level_idx,
                            tier_flag: u8::from(ptl.tier_flag),
                        },
                    );
                }
                self.global_lcr_records.insert(
                    info.global_config_record_id,
                    GlobalLcrRecord {
                        max_num_xlayer_count: info.xlayer_map.count_ones(),
                        xlayer_ids,
                        aggregate_info: info.aggregate_info,
                        seq_ptl_by_xlayer,
                        seq_ptl_present: info.seq_ptl_info_present,
                        doh_constraint_flag: info.doh_constraint_flag,
                        offset: obu.offset,
                        // The temporal unit of this observation, for the § 6.8.2 "present in
                        // the same CMVS" window check (codex finding 3393129738). A temporal
                        // unit is atomic for § 7.3.6 attribution, so this is unambiguous even
                        // for a global LCR observed before the CLK of its own temporal unit.
                        // Stamped on every (re)definition.
                        observed_tu_index: self.cvs.tu_index,
                    },
                );
                // AV2 § 6.8.9: retain each payload's embedded-layer maps for the
                // dependency-map agreement checks. A redefinition replaces the maps
                // wholesale so a dropped payload cannot leave stale entries.
                self.hls
                    .clear_global_lcr_embedded(info.global_config_record_id);
                // AV2 § 6.8.5/§ 6.8.8: retain this global LCR's per-xlayer PTL declared
                // maxima and rep info for the ceiling / equality agreement checks. A
                // redefinition clears and re-records so a dropped PTL/rep-info cannot
                // leave stale entries.
                self.hls
                    .clear_global_lcr_extras(info.global_config_record_id);
                // § 5.8.4: lcr_seq_profile_tier_level_info(i) is present per xlayer in
                // the map only when lcr_seq_profile_tier_level_info_present_flag == 1.
                for ptl in &info.seq_ptl_infos {
                    self.hls.record_global_lcr_ptl(
                        info.global_config_record_id,
                        ExtendedLayerId::from_bits(ptl.xlayer_id),
                        LcrPtlSnapshot {
                            seq_profile_idc: ptl.seq_profile_idc,
                            max_level_idx: ptl.max_level_idx,
                            tier_flag: u8::from(ptl.tier_flag),
                            max_mlayer_count: ptl.max_mlayer_count,
                            offset: obu.offset,
                        },
                    );
                }
                for payload in &info.payloads {
                    let xlayer_id = ExtendedLayerId::from_bits(payload.xlayer_id);
                    if let Some(embedded) = &payload.xlayer_info.embedded_layer_info {
                        self.hls.record_global_lcr_embedded(
                            info.global_config_record_id,
                            xlayer_id,
                            LcrEmbeddedMaps {
                                mlayer_map: embedded.mlayer_map,
                                tlayer_maps: embedded
                                    .layers
                                    .iter()
                                    .map(|layer| (layer.mlayer_index, layer.tlayer_map))
                                    .collect(),
                                max_expected: embedded
                                    .layers
                                    .iter()
                                    .map(|layer| {
                                        (
                                            layer.mlayer_index,
                                            layer.max_expected_width,
                                            layer.max_expected_height,
                                        )
                                    })
                                    .collect(),
                                offset: obu.offset,
                            },
                        );
                    }
                    // § 5.8.7: lcr_rep_info(1, xId) is present only when its flag is set.
                    if let Some(rep_info) = &payload.xlayer_info.rep_info {
                        self.hls.record_global_lcr_rep_info(
                            info.global_config_record_id,
                            xlayer_id,
                            rep_info_snapshot(rep_info, obu.offset),
                        );
                    }
                }
            }
            LayerConfigurationRecord::Local(info) => {
                // Annex A Table A.4: a local LCR OBU is present in this temporal unit (the
                // IOP1 `!e && m` / IOP2 LCR arms can be satisfied by a local LCR).
                self.annex_a_iop.note_local_lcr();
                // AV2 § 7.3.8.3: a local LCR's lcr_global_id (when non-zero) must
                // resolve to an available global LCR.
                if info.global_id != 0 {
                    if self.hls.global_lcr_xlayer_map(info.global_id).is_some() {
                        // Resolved in-band (linear check did not fire) -> buffer the
                        // § 7.3.8.3 reference for the random-access-point availability
                        // replay, governed by this local LCR's own extended layer.
                        self.note_rap_reference(
                            RapHlsKey::LayerConfigurationRecord {
                                xlayer: GLOBAL_XLAYER_ID.get(),
                                id: info.global_id,
                            },
                            xlayer,
                            obu.offset,
                        );
                    } else if external_disabled {
                        report.push(
                            Diagnostic::error(
                                "lcr/global-lcr-unavailable",
                                format!(
                                    "local layer configuration record for obu_xlayer_id {} \
                                     references lcr_global_id {}, but no global layer \
                                     configuration record with that id is available in-band \
                                     (external HLS is disabled)",
                                    xlayer.get(),
                                    info.global_id
                                ),
                            )
                            .with_spec_section("7.3.8.3")
                            .with_byte_offset(obu.offset),
                        );
                    }
                }
                // AV2 § 7.3.8.4: a local LCR's lcr_local_atlas_id must resolve to an
                // available local atlas segment OBU in the same extended layer.
                if let Some(atlas_id) = info.local_atlas_id {
                    if self.hls.has_local_atlas(xlayer, atlas_id) {
                        // Resolved in-band (linear check did not fire) -> buffer the
                        // § 7.3.8.4 *local* atlas reference for the replay (a global atlas
                        // "can be available" and is excluded, matching the linear check).
                        self.note_rap_reference(
                            RapHlsKey::Atlas {
                                xlayer: xlayer.get(),
                                id: atlas_id,
                            },
                            xlayer,
                            obu.offset,
                        );
                    } else if external_disabled {
                        report.push(
                            Diagnostic::error(
                                "atlas/local-atlas-unavailable",
                                format!(
                                    "local layer configuration record for obu_xlayer_id {} \
                                     references lcr_local_atlas_id {}, but no local atlas segment \
                                     OBU with that id is available in-band for that extended layer \
                                     (external HLS is disabled)",
                                    xlayer.get(),
                                    atlas_id
                                ),
                            )
                            .with_spec_section("7.3.8.4")
                            .with_byte_offset(obu.offset),
                        );
                    }
                }
                self.hls.record_local_lcr(xlayer, info.local_id);
                // AV2 § 7.3.8.1: note this local LCR's in-band (re)send (its own extended
                // layer) for the random-access-point availability replay, so a sequence
                // header's seq_lcr_id resolving to it at a random access point must find it
                // resent there.
                self.rap_replay.note_resend(
                    RapHlsKey::LayerConfigurationRecord {
                        xlayer: xlayer.get(),
                        id: info.local_id,
                    },
                    xlayer,
                );
                // AV2 § 6.8.9: retain the embedded-layer maps for the dependency-map
                // agreement checks. A redefinition replaces the maps wholesale so a
                // re-sent record without embedded info cannot leave stale entries.
                self.hls.clear_local_lcr_embedded(xlayer, info.local_id);
                // AV2 § 6.8.5/§ 6.8.8: retain this local LCR's PTL declared maxima and
                // rep info for the ceiling / equality agreement checks (the § 6.8.5
                // sentences key the ceiling on the local LCR). Cleared first so a
                // re-sent record that drops them cannot leave stale entries.
                self.hls.clear_local_lcr_extras(xlayer, info.local_id);
                if let Some(embedded) = &info.xlayer_info.embedded_layer_info {
                    self.hls.record_local_lcr_embedded(
                        xlayer,
                        info.local_id,
                        LcrEmbeddedMaps {
                            mlayer_map: embedded.mlayer_map,
                            tlayer_maps: embedded
                                .layers
                                .iter()
                                .map(|layer| (layer.mlayer_index, layer.tlayer_map))
                                .collect(),
                            max_expected: embedded
                                .layers
                                .iter()
                                .map(|layer| {
                                    (
                                        layer.mlayer_index,
                                        layer.max_expected_width,
                                        layer.max_expected_height,
                                    )
                                })
                                .collect(),
                            offset: obu.offset,
                        },
                    );
                }
                // § 5.8.4: lcr_seq_profile_tier_level_info(xlayerId) is present only when
                // lcr_profile_tier_level_info_present_flag[xlayerId] == 1.
                if let Some(ptl) = &info.seq_ptl_info {
                    self.hls.record_local_lcr_ptl(
                        xlayer,
                        info.local_id,
                        LcrPtlSnapshot {
                            seq_profile_idc: ptl.seq_profile_idc,
                            max_level_idx: ptl.max_level_idx,
                            tier_flag: u8::from(ptl.tier_flag),
                            max_mlayer_count: ptl.max_mlayer_count,
                            offset: obu.offset,
                        },
                    );
                }
                // § 5.8.7: lcr_rep_info(0, xId) is present only when its flag is set.
                if let Some(rep_info) = &info.xlayer_info.rep_info {
                    self.hls.record_local_lcr_rep_info(
                        xlayer,
                        info.local_id,
                        rep_info_snapshot(rep_info, obu.offset),
                    );
                }
            }
            // `LayerConfigurationRecord` is `#[non_exhaustive]`; only global and local
            // scopes exist in AV2 v1.0.0, so any future variant is ignored here.
            _ => {}
        }

        // Deliberately NO § 6.8.9 re-evaluation here: § 6.4.1 associates a sequence
        // header only with an LCR "present prior to this sequence header" (or
        // provided externally), so a later-arriving LCR must not be retroactively
        // paired with an earlier activation — the agreement checks run only from
        // on_sequence_activation. An LCR redefinition between activations is
        // likewise evaluated at the next activation event only (sound over
        // complete).
    }

    /// Resolves a sequence header's `seq_lcr_id` reference (AV2 § 6.4.1 / § 7.3.8.3 /
    /// § 7.3.8.6): when non-zero it must resolve to an available local LCR (same
    /// xlayer, `lcr_local_id == seq_lcr_id`) or, failing that, an available global LCR
    /// (`lcr_global_config_record_id == seq_lcr_id`) whose `lcr_xlayer_map` includes
    /// this header's xlayer. Availability diagnostics are gated on external HLS being
    /// disabled (an externally-provided LCR is not modeled).
    pub(super) fn check_seq_lcr_reference(
        &self,
        obu: &ObuEnvelope<'_>,
        seq_lcr_id: u8,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if seq_lcr_id == 0 {
            // AV2 § 6.4.1: seq_lcr_id == 0 means no LCR is associated.
            return;
        }
        let xlayer = obu.header.extended_layer_id;

        // Resolution order (AV2 § 6.4.1): a local LCR in this xlayer first, then a
        // global LCR.
        if self.hls.has_local_lcr(xlayer, seq_lcr_id) {
            return;
        }
        if let Some(xlayer_map) = self.hls.global_lcr_xlayer_map(seq_lcr_id) {
            // AV2 § 6.4.1: the activated global LCR's lcr_xlayer_map must include the
            // sequence header's obu_xlayer_id. Suppressed under any Provided external-HLS
            // mode: a Provided declaration is *partial* (`ExternalHlsMode::Provided` —
            // unenumerated external LCRs MAY exist), so an externally-provided local LCR
            // with this seq_lcr_id could resolve ahead of this in-band global by the
            // local-first § 6.4.1 order, making the global's map irrelevant; flagging it
            // would be a false positive. This is the same local-first-shadowing reasoning
            // that suppresses the § 6.8.5 / § 6.8.8 / § 6.8.9 association-dependent
            // agreement checks (`check_lcr_dependency_agreement` and friends) — consistent
            // with the unavailable branch below and the multi-frame-header precedent.
            let xlayer_bit = xlayer.get();
            if matches!(options.external_hls, ExternalHlsMode::Disabled)
                && xlayer_bit < 31
                && xlayer_map & (1u32 << xlayer_bit) == 0
            {
                report.push(
                    Diagnostic::error(
                        "lcr/global-xlayer-map-missing-xlayer",
                        format!(
                            "sequence header for obu_xlayer_id {} references global layer \
                             configuration record {seq_lcr_id} via seq_lcr_id, but that xlayer is \
                             not set in its lcr_xlayer_map (0x{xlayer_map:08x})",
                            xlayer.get()
                        ),
                    )
                    .with_spec_section("6.4.1")
                    .with_byte_offset(obu.offset),
                );
            }
            return;
        }

        // Unresolved: neither a local nor a global LCR with this id is available.
        if matches!(options.external_hls, ExternalHlsMode::Disabled) {
            report.push(
                Diagnostic::error(
                    "hls/unavailable-layer-configuration-record",
                    format!(
                        "sequence header references seq_lcr_id {seq_lcr_id}, but no local layer \
                         configuration record in obu_xlayer_id {} and no global layer configuration \
                         record with that id is available in-band (external HLS is disabled)",
                        xlayer.get()
                    ),
                )
                .with_spec_section("7.3.8.3")
                .with_byte_offset(obu.offset),
            );
        }
    }
}
