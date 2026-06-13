// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Stateful validator context for checks that depend on earlier OBUs.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::atlas_segment::parse_atlas_segment;
use splot_core::headers::buffer_removal_timing::parse_buffer_removal_timing;
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::film_grain::{
    FilmGrainObu, FilmGrainScalingPoint, MAX_FILM_GRAIN, parse_film_grain,
};
use splot_core::headers::frame::{
    CCSO_BAND_NUM, CcsoParams, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameHeaderPrefix, FrameReferenceStateView, FrameType, SetupQmParams,
    TileInfo, parse_frame_header_core, parse_frame_header_prefix,
};
use splot_core::headers::layer_config_record::{
    LayerConfigurationRecord, LcrAggregateInfo, LcrRepInfo, parse_layer_config_record,
};
use splot_core::headers::metadata::{
    MetadataPayload, MetadataScanType, MetadataTimecode, MetadataUnit, parse_metadata_group,
    parse_metadata_short,
};
use splot_core::headers::operating_point_set::{
    OperatingPointSet, OpsMlayerInfo, OpsMlayerSource, parse_operating_point_set,
};
use splot_core::headers::quantizer_matrix::{
    NUM_CUSTOM_QMS, QuantizerMatrixObu, parse_quantizer_matrix,
};
use splot_core::headers::sequence::{
    MAX_NUM_MLAYERS, MAX_SEQ_NUM, MLayerDependencyMap, SequenceHeader, SequenceHeaderGeneral,
    SequenceHeaderId, TLayerDependencyMap, Tier, TimingInfo, parse_sequence_header,
};
use splot_core::headers::tile_group::{
    FrameHeaderCopyOutcome, RecordedFrameHeaderBits, TileFramingDefect, TileGroupLayout,
    TileGroupStructureOutcome, parse_frame_header_copy, parse_tile_group_framing,
    parse_tile_group_prefix, parse_tile_group_structure,
};
use splot_core::hls::{
    MAX_MFH_NUM, MfhId, MultiFrameHeaderRecord, MultistreamDecoderOperation, parse_msdo,
    parse_multi_frame_header,
};
use splot_core::obu::finish_obu_payload;
use splot_core::span::{BitOffset, ByteOffset};
use splot_core::tile::{MAX_TILE_COLS, MAX_TILE_ROWS};
use splot_core::types::{
    EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId,
};

use crate::annex_a::{
    InteroperabilityPoint, MIN_FRAME_DIMENSION, config_idc_allows_profile, interoperability_point,
    is_defined_config_idc, is_reserved_level, is_reserved_profile, level_limits,
    profile_allows_chroma,
};
use crate::celu::{CeluRole, CodedExtendedLayerTracker, FrameFacts, Leadingness};
use crate::diagnostic::{Diagnostic, ValidationReport};
use crate::frame_unit::{FrameBoundary, FrameUnitSegmenter, SegRole, type_decided_output};
use crate::metadata_lifetime::{
    ActiveMetadataUnit, LAYER_CURRENT, LAYER_GLOBAL, LAYER_VALUES, MetadataLifetimeStore,
    PersistenceMode,
};
use crate::options::{ExternalHlsMode, ValidationOptions};
use crate::reference_state::{
    FrameRefUpdate, NUM_REF_FRAMES, ReferenceStateTracker, SlotState, is_key_or_switch, slot_facts,
};

mod annex_a_iop;
mod annex_a_value_space;
mod cmvs;
mod content_interpretation;
mod cvs;
mod film_grain;
mod frame_facts;
mod frame_header_checks;
mod frame_header_copy;
mod frame_header_diagnostics;
mod frame_headers;
mod hls;
mod lcr;
mod lcr_agreement;
mod lcr_sequence_agreement;
mod metadata;
mod msdo;
mod ops;
mod ops_buffer_delay;
mod quantizer_matrix;
mod rap_replay;
mod reference_frames;
mod scan_type;
mod sequence;
mod shared;
mod temporal_unit;
#[cfg(test)]
mod tests;
mod tile_groups;
mod timecode;

// This is a mechanical split of the former context.rs monolith: the submodules
// intentionally share this crate-private helper namespace via `use super::*`.
use self::annex_a_iop::*;
use self::annex_a_value_space::*;
use self::cmvs::*;
use self::content_interpretation::*;
use self::cvs::*;
use self::film_grain::*;
use self::frame_facts::*;
use self::frame_header_checks::*;
use self::frame_header_copy::*;
use self::frame_header_diagnostics::*;
use self::hls::*;
use self::lcr::*;
use self::lcr_agreement::*;
use self::metadata::*;
use self::msdo::*;
use self::ops::*;
use self::ops_buffer_delay::*;
use self::quantizer_matrix::*;
use self::rap_replay::*;
use self::scan_type::*;
use self::sequence::*;
use self::shared::*;
use self::temporal_unit::*;
use self::tile_groups::*;
use self::timecode::*;
// frame_headers, lcr_sequence_agreement, and reference_frames only add
// ValidatorContext impl methods; they are included by `mod` declarations above
// and export no standalone helper symbols for this glob list.

/// Stateful validator data derived from parseable high-level syntax OBUs.
#[derive(Debug, Default)]
pub(crate) struct ValidatorContext {
    sequence_headers: BTreeMap<SequenceHeaderId, SequenceHeader>,
    active_sequence_by_xlayer: BTreeMap<ExtendedLayerId, SequenceHeaderId>,
    /// Payload fingerprints for activated sequence headers, keyed by
    /// `(obu_xlayer_id, seq_header_id)`, used to detect non-bit-identical repeats
    /// of an activated sequence header within a coded video sequence (AV2 § 7.3.6).
    /// Each value is `(fingerprint, tu_index)` where `tu_index` is the temporal unit
    /// of the header's latest appearance; the [`CvsTracker`] boundary events scope
    /// the map to the exact coded video sequence.
    sequence_fingerprints: BTreeMap<(ExtendedLayerId, SequenceHeaderId), (u64, u64)>,
    /// Content-interpretation records keyed by `(obu_xlayer_id, obu_mlayer_id)`
    /// within the coded video sequence of the extended layer (exact § 7.3.6
    /// boundaries via [`CvsTracker`]), used for cross-embedded-layer timing
    /// consistency (AV2 § 6.4.12) and repeated-CI identity (AV2 § 6.14).
    content_interpretations: BTreeMap<ContentInterpretationKey, ContentInterpretationRecord>,
    /// Availability of in-band HLS objects, for reference checks (AV2 § 7.3.8).
    hls: HlsAvailabilityStore,
    /// Active in-band operating point sets, with non-monotonic reset/update semantics
    /// (AV2 § 6.10.1, § 7.3.8.5). Kept separate from [`HlsAvailabilityStore`] because
    /// OPS state, unlike the other HLS records, is explicitly resettable.
    ops: OpsAvailabilityStore,
    /// Quantizer-matrix `qm_bit_map` reset/level state (§ 6.12) plus per-level
    /// availability foundation for future frame-reference checks (§ 7.3.8).
    qm: QuantizerMatrixState,
    /// Film-grain `fgm_update_flags` slot state (§ 6.13) plus per-slot availability
    /// foundation for future frame-reference checks (§ 7.3.8).
    film_grain: FilmGrainState,
    /// Active metadata persistence / cancellation state (AV2 § 6.16.3); see
    /// [`MetadataLifetimeStore`]. Scoped to the coded video sequence via the
    /// [`CvsTracker`] CLK hook and to the coded frame for `NO_PERSISTENCE` via
    /// [`ValidatorContext::reset_coded_frame_window`].
    metadata: MetadataLifetimeStore,
    /// HDR CLL / MDCV content baselines per coded-video-sequence scope, for the
    /// § 6.16.5 / § 6.16.6 "shall have the same content" checks: each record
    /// carries its unit's bitstream-derived embedded-layer association (see
    /// [`HdrAssociation`]) and a new unit is compared against every baseline of
    /// the same metadata type whose association intersects it. Independent of the
    /// § 6.16.3 cancellation state in [`ValidatorContext::metadata`] — the
    /// same-content rule carries no cancel exception, so a unit re-signaled after a
    /// cancel is still compared against the earlier content.
    hdr_baselines: Vec<HdrBaselineRecord>,
    /// For each extended layer, the temporal unit of its most recent random
    /// access point (AV2 § 7.3.8.11: the content interpretation parameters are
    /// initialized to defaults "at each random access point of the extended
    /// layer (i.e., at each temporal unit containing an OBU in the extended
    /// layer with obu_type equal to OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY)").
    /// Scopes the § 6.16.10 Table 6.18 scan-type / CI pairings to the
    /// CI-parameter epoch; the CVS-scoped state (§ 6.14 repeated-CI identity,
    /// § 6.4.12 timing, § 7.3.6 stores) deliberately ignores it — an OLK does
    /// not start a new coded video sequence during sequential decoding
    /// (§ 7.4.4).
    ci_rap_started_in_tu: BTreeMap<ExtendedLayerId, u64>,
    /// The temporal unit in which [`ValidatorContext::repair_post_rap_ci_pairings`]
    /// last ran for each extended layer, so the § 7.3.8.11 RAP re-pair is idempotent
    /// within one temporal unit: a malformed temporal unit carrying two CLK/OLK random
    /// access points for the SAME extended layer would otherwise run the re-pair twice
    /// against the same post-epoch CI snapshot and duplicate every repaired diagnostic.
    /// The second CLK/OLK's `observe_ci_rap` keeps the epoch at this temporal unit and
    /// drops nothing new, so the guard short-circuits the redundant re-pair.
    repaired_post_rap_in_tu: BTreeMap<ExtendedLayerId, u64>,
    /// Scan-type metadata observations per coded-video-sequence scope, for the
    /// § 6.16.10 Table 6.18 consistency checks; see [`ScanTypeCvsState`]. Scoped to
    /// the coded video sequence via the [`CvsTracker`] CLK hook and flushed at the
    /// end of the bitstream (see [`ValidatorContext::finish`]).
    scan_type: ScanTypeCvsState,
    /// Timecode metadata state per coded-video-sequence scope, for the § 6.16.7
    /// inference-presence rules and the `ci_timing_info_present_flag`-gated n_frames
    /// bound; see [`TimecodeCvsState`]. Scoped to the coded video sequence via the
    /// [`CvsTracker`] CLK hook (the decoding-order inference chain resets at a CVS
    /// boundary).
    timecode: TimecodeCvsState,
    temporal_unit: TemporalUnitState,
    /// Exact coded-video-sequence boundary state (AV2 § 7.3.6); see [`CvsTracker`].
    cvs: CvsTracker,
    /// Extended layers that have produced a frame-bearing OBU since the most recent
    /// global temporal delimiter. Derives the per-extended-layer `FirstPictureInTU`
    /// — "the first frame unit in a coded extended layer unit in a temporal unit"
    /// (AV2 § 6.17.2) — for parsed frame headers (AV2 § 5.18.2 `startCVS`).
    frames_seen_in_tu: BTreeSet<ExtendedLayerId>,
    /// Already-emitted § 6.10.7 / § 6.8.9 layer-dependency agreement findings, so
    /// the activation-driven re-checks never duplicate a diagnostic for the same
    /// pairing (a different activated sequence header gets a distinct key).
    emitted_dependency_findings: BTreeSet<DependencyFindingKey>,
    /// Already-emitted § 6.8.5 PTL-ceiling findings, so the activation-driven re-checks
    /// never duplicate a diagnostic. The key carries the LCR snapshot's content and the
    /// activated header's compared PTL values, so a non-identical LCR redefinition or a
    /// same-id sequence-header reconfiguration re-emits while an identical re-evaluation
    /// is idempotent (mirrors [`SubstreamMaxFindingKey`]).
    emitted_lcr_ptl_findings: BTreeSet<LcrPtlFindingKey>,
    /// Already-emitted § 6.8.8 rep-info mismatch findings; see
    /// [`Self::emitted_lcr_ptl_findings`] for the dedup discipline.
    emitted_lcr_rep_info_findings: BTreeSet<LcrRepInfoFindingKey>,
    /// Already-emitted § 6.8.9 `lcr_max_expected_width/height` sequence-max bound findings;
    /// see [`Self::emitted_lcr_ptl_findings`] for the dedup discipline.
    emitted_lcr_expected_dims_findings: BTreeSet<LcrExpectedDimsFindingKey>,
    /// Extended layers whose active sequence header was confirmed by a parsed
    /// frame-header reference (the § 5.18.2 `load_sequence_header` path), as
    /// opposed to the first-seen OBU-order fallback. The § 6.10.7 / § 6.8.9
    /// agreement checks treat a fallback activation as decidable only while it
    /// is the sole in-band candidate (see `agreement_activation_for`).
    frame_confirmed_xlayers: BTreeSet<ExtendedLayerId>,
    /// § 6.4.1 LCR-association snapshots keyed by `(xlayer, seq_header_id)`:
    /// the header's `seq_lcr_id` resolved local-first-then-global against the
    /// LCRs present prior to the *latest observation* of that header, with the
    /// resolved record's embedded-layer maps as of that observation. § 6.4.1
    /// associates an LCR "present prior to this sequence header", so the
    /// § 6.8.9 agreement check must not re-resolve against the live store at a
    /// later activation (a post-header redefinition is not the associated
    /// record).
    lcr_associations: BTreeMap<(ExtendedLayerId, SequenceHeaderId), LcrAssociation>,
    /// Stateful § 5.6 MSDO observer (AV2 § 5.6 / § 7.3.2); see [`MsdoObserver`]. Feeds
    /// the [`CmvsTracker`] the § 7.3.2 condition-2 "differs from the previous OBU_MSDO"
    /// signal and holds no diagnostics of its own.
    msdo: MsdoObserver,
    /// Three-state § 7.3.2 coded-multistream-video-sequence begin/end tracker; see
    /// [`CmvsTracker`]. Scopes the § 6.4.1 cross-xlayer
    /// `monotonic_output_order_flag` check to a definitively-active CMVS. No
    /// `cmvs/*` diagnostics are emitted from the tracker itself.
    cmvs: CmvsTracker,
    /// Per-extended-layer distinct-`obu_mlayer_id` count within the current coded video
    /// sequence (AV2 § 6.4.1); see [`DistinctMlayerTracker`]. Reset at each § 7.3.6 CVS
    /// start and compared against the active sequence header's `SeqMaxMlayerCnt`.
    distinct_mlayer: DistinctMlayerTracker,
    /// For each extended layer with a frame-confirmed activation, the § 7.3.6 coded
    /// video sequence epoch ([`CvsTracker::cvs_epoch`]) in which that activation
    /// occurred. Used by `hls/multiple-active-sequence-headers` (AV2 § 7.3.6) to detect
    /// a second, different frame-confirmed activation within the *same* coded video
    /// sequence: a CLK that starts a new coded video sequence advances the epoch, so a
    /// re-activation across a CLK does not match. `None` is the implicit pre-first-CLK
    /// coded video sequence.
    frame_confirmed_activation_cvs: BTreeMap<ExtendedLayerId, Option<u64>>,
    /// For each extended layer, the [`CvsTracker::tu_index`] of its latest frame-confirmed
    /// sequence-header activation. Scopes the § 6.8.2 LCR DOH loop and the § 6.6 MSDO DOH
    /// loop to only the headers activated within the *current* CMVS window
    /// (`>= cmvs_start_tu_index`): both loops otherwise iterate the whole-history
    /// `frame_confirmed_xlayers` accumulator and would flag a non-monotonic header left
    /// active from an earlier, already-ended coded video sequence outside the current CMVS
    /// (codex finding 3393129745).
    frame_confirmed_activation_tu: BTreeMap<ExtendedLayerId, u64>,
    /// Last explicitly signalled `ops_decoder_buffer_delay + ops_encoder_buffer_delay`
    /// sum per `(obu_xlayer_id, ops_id, operating-point index)`, with the CVS epoch and
    /// § 6.10.1 reset generation in which it was observed, for the § 6.10.5
    /// buffer-delay sum-constancy checks. Annex E binds the delays per `(xId, opsID, op)`
    /// (`annex-e-decoder-model.md` lines 100–112), so the triple is the comparison key.
    /// Only explicitly signalled values enter this map; absent decoder-model info (and
    /// the Annex E resource-availability defaults) never write or clear an entry.
    ops_buffer_delay_sums: BTreeMap<OpsBufferDelayKey, BufferDelayBaseline>,
    /// Last explicitly signalled `decoder_buffer_delay + encoder_buffer_delay` sum of
    /// the frame-confirmed activated sequence header per extended layer, with the CVS
    /// epoch in which it was observed, for the § 6.4.13 cross-CVS advisory. Only
    /// frame-confirmed activations with explicit `seq_decoder_model_info()` populate
    /// this map; fallback-guess activations and headers without decoder-model info do
    /// not participate.
    seq_buffer_delay_sums: BTreeMap<ExtendedLayerId, SeqBufferDelayBaseline>,
    /// Already-emitted Annex A profile/level/tier value-space findings, keyed by
    /// `(xlayer, seq_header_id, cvs_generation_epoch, value_space_fingerprint)`, so the
    /// activation-driven re-checks emit each finding once per activated header per coded
    /// video sequence rather than per OBU (Annex A.2 Table A.1 / Annex A.4 Table A.7/A.9).
    /// The fingerprint ([`annex_a_value_space_fingerprint`]) covers the fields these checks
    /// inspect, so a § 7.3.6 same-`seq_header_id` redefinition that changes any checked
    /// field re-runs the checks instead of being suppressed by the prior activation's key.
    emitted_annex_a_value_space: BTreeSet<(
        ExtendedLayerId,
        SequenceHeaderId,
        u64,
        AnnexAValueSpaceFingerprint,
    )>,
    /// Byte offset of the OBU carrying each stored sequence header, keyed by
    /// `seq_header_id`, so the Annex A value-space diagnostics — emitted at activation
    /// time, which may be a frame OBU — anchor at the defining sequence-header OBU.
    sequence_header_offsets: BTreeMap<SequenceHeaderId, ByteOffset>,
    /// The most recently observed OBU_MSDO's `sub_xlayer_id[i] → (max_profile,
    /// max_level, max_tier)` mapping plus the MSDO's byte offset, recorded for the
    /// § 6.6 sub-stream PTL-ceiling agreement checks
    /// (`msdo/substream-profile-exceeds-max` / `-level-` / `-tier-`). `None` until the
    /// first parseable MSDO is seen. A later MSDO replaces it wholesale (§ 7.3.8.2
    /// requires a non-RAP MSDO to be identical to the previous one, so the live set is
    /// the active multistream operation). The recorded map is also the state the
    /// descoped Table A.4 IOP-presence machinery (`msdo-global-lcr-agreement`) will
    /// reuse.
    msdo_substream_max: Option<MsdoSubstreamMax>,
    /// Every OBU_MSDO observed, accumulated for the § 6.8.2 MSDO↔global-LCR *agreement*
    /// (mirror lines 1646-1648: the constraints hold "when both an OBU_MSDO and an activated
    /// global LCR ... are present in the same coded multistream video sequence"). The live
    /// `msdo_substream_max` is last-wins (correct for the § 6.6 ceiling check — the live MSDO
    /// is the active operation), but the agreement must hold for EVERY MSDO present in the
    /// CMVS: a per-MSDO last-wins overwrite would let an earlier non-conforming MSDO escape
    /// when a later conforming one replaced it before the deferred resolution (codex finding
    /// 3393274380). § 7.3.8.2's identity rule bounds this — a non-RAP MSDO must be identical
    /// to its predecessor, so distinct MSDOs only appear at RAPs — but distinct MSDOs CAN
    /// appear in one CMVS, so each is evaluated against the resolved LCR. Deduped by MSDO byte
    /// offset (one entry per OBU); the deferred resolution filters to the current CMVS window
    /// (`observed_tu_index >= cmvs_start`) and the `emitted_lcr_agreement` key (which carries
    /// the MSDO offset) keeps each per-MSDO disagreement to one emission.
    msdo_agreement_snapshots: Vec<MsdoWindowSnapshot>,
    /// Already-emitted § 6.6 sub-stream PTL-ceiling findings, keyed by
    /// `(xlayer, seq_header_id, cvs_generation_epoch, MSDO ceiling for the layer,
    /// activated-header value-space fingerprint)`, so the activation-driven and
    /// MSDO-arrival-driven re-checks emit each finding once per activated header per
    /// coded video sequence rather than per OBU. A § 7.3.6 same-`seq_header_id`
    /// redefinition that changes a checked PTL field, or a new MSDO that changes the
    /// ceiling, re-runs the checks (mirrors the `emitted_annex_a_value_space` dedup).
    emitted_substream_max: BTreeSet<SubstreamMaxFindingKey>,
    /// Already-emitted § 6.6 DOH-constraint findings (`msdo/doh-constraint-required`),
    /// keyed by `(xlayer, seq_header_id, cvs_generation_epoch)`, so the activation-
    /// driven and MSDO-arrival-driven re-checks emit once per activated header per coded
    /// video sequence rather than per OBU. The condition the rule checks
    /// (`monotonic_output_order_flag == 0` against `multistream_doh_constraint_flag ==
    /// 0`) is captured by the activated header id and CVS epoch — a redefinition keeping
    /// the same id and CVS but flipping the flag is a § 7.3.6 violation flagged
    /// elsewhere (`hls/repeated-sequence-header-not-identical`), so the triple key
    /// suffices.
    emitted_doh_constraint: BTreeSet<(ExtendedLayerId, SequenceHeaderId, u64)>,
    /// Per-temporal-unit § 7.3.8.2 MSDO-identity working state (current TU's RAP-ness
    /// and the fingerprint(s)+offset(s) of the MSDO(s) seen in this TU) plus the
    /// previous OBU_MSDO's payload fingerprint for the pairwise-previous identity
    /// comparison resolved at temporal-unit end. See [`MsdoIdentityTracker`].
    msdo_identity: MsdoIdentityTracker,
    /// In-band global layer configuration records by `lcr_global_config_record_id`, with
    /// the § 6.8.2 aggregate / per-substream PTL / DOH fields and the defining OBU offset
    /// (AV2 § 5.8.1). A redefinition of the same id overwrites the record. The
    /// [`HlsAvailabilityStore`] keeps only the `lcr_xlayer_map` for the § 7.3.8.3
    /// availability and § 6.4.1 association checks; this fuller record is what the § 6.8.2
    /// MSDO↔global-LCR agreement consumes once the chain has resolved an *activated*
    /// global LCR.
    global_lcr_records: BTreeMap<u8, GlobalLcrRecord>,
    /// Already-emitted § 6.8.2 MSDO↔global-LCR agreement findings, keyed by the resolved
    /// `(global_config_record_id, MSDO/header byte offset, global-LCR byte offset, rule id,
    /// field discriminant)`, so the deferred CMVS-resolution evaluation does not re-emit the
    /// same disagreement on every subsequent temporal unit of the CMVS. The MSDO and
    /// global-LCR offsets change when either record is redefined, so a genuine new
    /// disagreement re-emits. The field discriminant distinguishes the several
    /// `lcr/msdo-aggregate-mismatch` sub-fields (config / interop / level / tier) and each
    /// disagreeing `sub_xlayer_id`, so two distinct disagreements of the same rule are not
    /// collapsed.
    emitted_lcr_agreement: BTreeSet<(u8, ByteOffset, ByteOffset, &'static str, u32)>,
    /// Already-emitted `cmvs/boundary-set-mismatch` findings, keyed by the MSDO offset of
    /// the disagreeing CMVS, so the deferred resolution emits the boundary-set
    /// disagreement once per CMVS rather than per temporal unit.
    emitted_cmvs_boundary: BTreeSet<ByteOffset>,
    /// The Annex A Table A.4 interoperability-point presence window over the current coded
    /// (multistream-)video-sequence; see [`AnnexAIopTracker`]. Evaluated at each coded
    /// video sequence boundary and at the end of the bitstream.
    annex_a_iop: AnnexAIopTracker,
    /// § 7.3.8.1 random-access-point HLS availability replay; see [`RapReplayTracker`].
    /// Records each HLS object's most recent in-band (re)send temporal unit and buffers
    /// references that resolved linearly so that, at temporal-unit completion (when the
    /// unit's § 7.4.1 random-access-point-ness and leading-frame-ness are known), a
    /// reference at/after a random access point whose object was not (re)sent in or after
    /// that point's temporal unit fires `hls/unavailable-at-random-access-point`
    /// (mirror `07-decoding-process.md` lines 685-693).
    rap_replay: RapReplayTracker,
    /// Coded-frame-unit segmentation and the § 7.3.3 / § 7.3.4 / § 7.3.5
    /// presence-order checks plus the § 7.3.8.10 first-coded-frame-unit CI rule;
    /// see [`FrameUnitSegmenter`]. Reset at each global temporal delimiter and
    /// flushed at the end of the bitstream.
    frame_unit: FrameUnitSegmenter,
    /// Per coded frame (keyed by the segmenter's `(xlayer, mlayer, tlayer)` triple), the
    /// recorded bits of the *first* tile group's completed frame header, used to check a
    /// non-first tile group's `frame_header_copy()` bit-for-bit (AV2 § 5.18.1 / § 6.17.1).
    ///
    /// Set when a first tile group's `frame_header_info()` parses to completion
    /// ([`FrameHeaderParseStatus::IntraHeaderComplete`]); the recorded `NumFrameHeaderBits`
    /// is the first header's exact bit length. The segmenter is the boundary authority: a
    /// tile group reported as [`FrameBoundary::OpensNewUnit`] resets the triple's record
    /// (a new coded frame), a [`FrameBoundary::ContinuesUnit`] non-first tile group pairs
    /// against it, and a [`FrameBoundary::Ambiguous`] boundary drops the pairing AND poisons
    /// (removes) the record — the unreadable delimiter may have opened a new coded frame, so
    /// the recorded header can no longer pair until the next [`FrameBoundary::OpensNewUnit`]
    /// re-records. Cleared at each global temporal delimiter (a coded frame does not span
    /// temporal units).
    frame_header_copy_record:
        BTreeMap<(ExtendedLayerId, EmbeddedLayerId, TemporalLayerId), FrameHeaderCopyRecord>,
    /// Coded-extended-layer-unit constraints (§ 7.3.6) and the § 7.3.7 / § 7.4.6 DOH
    /// OrderHint / OrderHintBits checks; see [`CodedExtendedLayerTracker`]. Sits above
    /// the frame-unit segmenter (keyed per `obu_xlayer_id` across a temporal unit). Reset
    /// at each global temporal delimiter (resolving the per-CELU and DOH constraints) and
    /// flushed at the end of the bitstream.
    celu: CodedExtendedLayerTracker,
    /// For each `(xlayer, mlayer)`, the **temporal-unit index** of that embedded
    /// layer's first observed coded picture — the § 6.16.5 / § 6.16.6 "first coded
    /// picture of that embedded layer in the coded video sequence" state. An HDR CLL
    /// / MDCV metadata unit associated with an embedded layer whose first coded
    /// picture has already passed (an entry exists and the layer is not currently in
    /// its first coded frame unit) violates the "shall be indicated at the first
    /// coded picture" sentence (mirror `06-syntax-structures-semantics.md` lines
    /// 3687-3688 / 3736-3737).
    ///
    /// The recorded TU index disambiguates CVS membership: an entry from an *earlier*
    /// temporal unit may belong to a different coded video sequence (a CLK later in
    /// the current temporal unit starts a new CVS for its extended layer, § 7.3.6), so
    /// a first-coded-picture finding against an earlier-TU entry is *deferred* to the
    /// temporal-unit flush via [`CvsTracker::defer_or_emit`] — exactly as the HDR
    /// repeat-content check defers an earlier-TU baseline. The CLK hook still prunes
    /// the entries for its extended layer at the boundary so a new CVS re-establishes
    /// its own first-picture state.
    embedded_layer_first_picture_seen: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), u64>,
    /// Per extended layer, the § 7.3.6 "content interpretation present in any CELU shall also
    /// be present in the first CELU of the sequence" PRESENCE state (mirror lines 560-562,
    /// `07-decoding-process.md#s-7-3-6`). For each extended layer it records the embedded
    /// layers whose CI was observed in the FIRST coded extended layer unit of the current
    /// coded video sequence (the CELU in the CVS's first temporal unit, § 7.3.6) and dedups
    /// the diagnostic per embedded layer. A CI in a LATER CELU of the same CVS for an embedded
    /// layer the first CELU lacked fires `celu/content-interpretation-not-in-first-celu`. Reset
    /// per coded video sequence in [`Self::start_cvs_for_xlayer`]. The contents-identity half
    /// of the same sentence is owned by `content-interpretation/repeated-ci-not-identical`
    /// (§ 6.14); this state owns only the presence half. See [`CiFirstCeluState`].
    ci_first_celu: BTreeMap<ExtendedLayerId, CiFirstCeluState>,
    /// The `(xlayer, mlayer)` content-interpretation OBUs observed in the CURRENT temporal
    /// unit, each with the byte offset of its first appearance (the diagnostic anchor). Cleared
    /// at every temporal-unit boundary. Resolved against the per-CVS [`Self::ci_first_celu`]
    /// state at the boundary (and at the end of the bitstream): the whole temporal unit
    /// containing a CLK belongs to the new coded video sequence (§ 7.3.6), so a CI's CELU
    /// membership is final only once the temporal unit (with any CLK) is complete — the
    /// presence judgment is therefore deferred to the boundary rather than fired eagerly at
    /// CI-observation time (round-6 F3). See [`Self::resolve_ci_first_celu_for_tu`].
    ci_observed_in_tu: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), ByteOffset>,
    /// The § 7.23 reference-frame buffer state model, per extended layer (see
    /// [`ReferenceStateTracker`]). Updated at each completed frame's coded-frame
    /// boundary from the parsed `refresh_frame_flags` / `OrderHint` / dimensions
    /// (`ReferenceStateView::pending_ref_update` derives the grounded update kind), with
    /// honest all-slot poisoning whenever the refresh mask is unparsed and a grounded
    /// CLK reset at a new coded video sequence. The state is threaded into the
    /// frame-header parse input ([`FrameReferenceStateView`]) and gates the
    /// show-existing-frame slot-validity diagnostic (§ 6.17.2). Validator-derived: the
    /// buffers are written only in-band by the § 7.23 process, so the tracker never
    /// consults external HLS.
    reference_state: ReferenceStateTracker,
    /// The just-completed frame's pending § 7.23 update, deferred until the frame's
    /// coded-frame UNIT closes. § 7.23 runs at `decode_frame_wrapup` AFTER the frame is
    /// decoded, so the update must land before any *later* frame's reference checks read
    /// the buffer but after the current frame's own reference checks. The pending update
    /// (and its extended layer) is committed when the segmenter reports the next frame
    /// `OpensNewUnit` for that layer, or at the end-of-bitstream flush (no trailing
    /// delimiter). `None` between a commit and the next frame's parse.
    pending_ref_update: Option<(ExtendedLayerId, FrameRefUpdate)>,
}

impl ValidatorContext {
    /// Observes one parsed OBU, updating context and emitting stateful diagnostics.
    pub(crate) fn observe_obu(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // The per-extended-layer FirstPictureInTU as of *before* this OBU, captured
        // before `observe_frame_bearing_obu` marks the layer's frame seen. The
        // coded-frame-unit segmenter's output classification re-derives the same
        // frame-header fields the activation path used, so it must see the same
        // FirstPictureInTU value (AV2 § 6.17.2 / § 5.18.2 `startCVS`).
        let first_picture_in_tu = self.first_picture_in_tu(obu.header.extended_layer_id);

        // Temporal-unit and coded-video-sequence boundary events run first: a global
        // temporal delimiter completes the previous temporal unit (flushing deferred
        // CVS-scoped diagnostics) and a CLK starts a new coded video sequence for its
        // extended layer (AV2 § 7.3.6); see observe_cvs_boundary_events.
        self.observe_cvs_boundary_events(obu, options, report);

        self.temporal_unit.observe_obu(obu, report);

        // AV2 § 5.5 (mirror :1626-1630): temporal_delimiter_obu() clears QmProtected[level] =
        // 0 for every level. After this point a QM OBU sent earlier in a previous temporal
        // unit no longer protects its level from a CLK/OLK/SWITCH/RAS reset_qm() in this
        // temporal unit — the QmProtected discipline the § 7.3.8.9 availability check honors.
        if obu.header.obu_type == ObuType::TemporalDelimiter {
            self.qm.clear_qm_protected_at_temporal_delimiter();
        }

        // Annex A Table A.3: record this OBU's non-global obu_xlayer_id into the current
        // temporal unit's Table A.4 pending facts (the distinct extended-layer base count,
        // mirror lines 146-151). Recorded after the boundary events so a CLK's own xlayer
        // joins this temporal unit's facts, which the §7.3.6 per-temporal-unit attribution
        // assigns to the correct coded video sequence at temporal-unit completion.
        self.annex_a_iop.note_xlayer(obu.header.extended_layer_id);

        // AV2 § 7.3.8.1: a temporal unit carrying a LEADING_* frame OBU drops under
        // random access, so a resend inside it does not satisfy the availability replay.
        // The LEADING_* types (OBU_LEADING_TILE_GROUP / OBU_LEADING_SEF / OBU_LEADING_TIP)
        // are detectable from the OBU type alone — the sound type-detectable subset; a
        // non-LEADING_* leading frame (only knowable from inter-frame parsing) is the
        // documented under-approximation that leaves the unit qualifying.
        if matches!(
            obu.header.obu_type,
            ObuType::LeadingTileGroup | ObuType::LeadingSef | ObuType::LeadingTip
        ) {
            self.rap_replay
                .note_leading_frame(obu.header.extended_layer_id);
        }

        if obu.header.obu_type == ObuType::SequenceHeader {
            self.observe_sequence_header(obu, options, report);
        } else {
            // AV2 § 5.18.2: a frame header's load_sequence_header() runs at the start
            // of frame_header_info(), before the frame's own layer ids are
            // interpreted. So for a frame-bearing OBU, parse the prefix (best-effort)
            // and run the HLS reference + activation checks FIRST, then check the
            // active-sequence layer limits against the just-activated header. A parse
            // failure is silent, consistent with the multi-frame-header and
            // content-interpretation observers: the prefix is not-yet-validated
            // coverage, not a new error path.
            self.observe_frame_bearing_obu(obu, options, report);
            self.validate_active_sequence_limits(obu, options, report);
        }

        // AV2 § 6.4.1: count distinct obu_mlayer_id values per coded video sequence
        // against the active sequence header's SeqMaxMlayerCnt. Run after the
        // sequence-header / frame-bearing branch so a frame that activates a (more
        // permissive) header is counted against the just-activated header.
        self.count_distinct_mlayer(obu, options, report);

        match obu.header.obu_type {
            ObuType::ContentInterpretation => self.observe_content_interpretation(obu, report),
            ObuType::MultiFrameHeader => self.observe_multi_frame_header(obu, options, report),
            ObuType::LayerConfigurationRecord => {
                self.observe_layer_config_record(obu, options, report);
            }
            ObuType::AtlasSegment => self.observe_atlas_segment(obu),
            ObuType::OperatingPointSet => {
                self.observe_operating_point_set(obu, options, report);
            }
            ObuType::BufferRemovalTiming => {
                self.observe_buffer_removal_timing(obu, options, report);
            }
            ObuType::QuantizationMatrix => self.observe_quantizer_matrix(obu, report),
            ObuType::FilmGrain => self.observe_film_grain(obu, report),
            ObuType::MetadataShort | ObuType::MetadataGroup => {
                self.observe_metadata(obu, report);
            }
            ObuType::Msdo => self.observe_msdo(obu, options, report),
            _ => {}
        }

        // AV2 § 6.12 / § 6.13: both duplicate windows close at *any* coded frame,
        // including a SEF. The two families are scoped by their own verbatim
        // sentences, which both treat a SEF as a coded-frame boundary:
        //
        // - § 6.13 (film grain) scopes the duplicate-slot rule to the "same coded
        //   frame unit" and its NOTE permits reuse "in a subsequent coded frame
        //   unit". § 7.3.3 makes a single OBU_LEADING_SEF / OBU_REGULAR_SEF its own
        //   coded frame unit, so a SEF ends the current film-grain coded-frame-unit
        //   window — the next film-grain OBU belongs to a subsequent unit.
        // - § 6.12 (QM) scopes the duplicate-level rule to "between coded frames" and
        //   `QmSeen` to levels seen "since the last frame". § 7.3.3 lists a SEF as one
        //   of the two alternatives for "the coded frame" of a unit and states "Such a
        //   frame is associated with a decoded display order hint value, OrderHint",
        //   i.e. the spec calls a SEF a frame. So a SEF is a coded-frame boundary for
        //   `QmSeen` too.
        //
        // The two sentences therefore do not genuinely differ on the SEF boundary, so
        // `is_frame_bearing` (which includes a SEF) drives a single shared reset. The
        // reset is NOT at a temporal-unit boundary: a level / slot reused across a bare
        // temporal delimiter with no intervening frame is still a duplicate.
        if is_frame_bearing(obu.header.obu_type) {
            self.reset_coded_frame_window();
        }
        // AV2 § 6.16.3: NO_PERSISTENCE metadata is "Used only for the current
        // frame", so it expires at the coded frame of its frame unit (§ 7.3.5) —
        // including a SEF, which is the current displayed frame. This is the
        // coded-frame-unit-granular expiry the metadata-lifetime store's former
        // per-OBU TODO required; a SEF coded frame unit is its own unit, so its
        // NO_PERSISTENCE metadata expires at the SEF.
        if is_frame_bearing(obu.header.obu_type) {
            self.metadata.expire_no_persistence();
        }

        // AV2 § 7.3.3 / § 7.3.4 / § 7.3.5 / § 7.3.8.10: feed the coded-frame-unit
        // segmenter. The role (region classification, plus the output class and
        // is_first_tile_group / metadata_is_suffix facts) is computed from the
        // already-parsed state; an undecidable frame-header parse path yields an
        // unknown output class that routes the unit to Unknown (silent). The segmenter
        // returns each frame-bearing OBU's coded-frame-unit boundary signal — the CELU
        // tracker consumes it as the single source of truth for coded-frame-unit
        // boundaries (§ 7.3.6), rather than re-deriving them from frame-delimiter bits.
        let role = self.seg_role_for(obu, first_picture_in_tu);
        let boundary = self.frame_unit.observe(obu, role, report);

        // AV2 § 5.18.1 / § 6.17.1: record a completed first tile group's frame-header bits
        // and check a non-first tile group's frame_header_copy() bit-for-bit against them.
        // Keyed by the segmenter's per-coded-frame boundary signal (the authority) so the
        // pairing only ever joins a non-first tile group to ITS frame's first tile group.
        self.observe_frame_header_copy(obu, first_picture_in_tu, boundary, report);

        // AV2 § 7.3.6 / § 7.3.7 / § 7.4.6: feed the coded-extended-layer-unit tracker,
        // which sits above the frame-unit segmenter (keyed per obu_xlayer_id across the
        // temporal unit). The per-frame facts (output class, order_hint, leading-ness,
        // CLK/OLK identity) come from the same best-effort core parse; the coded-frame-unit
        // boundary comes from the segmenter (above). Any field the parse cannot reach is
        // left undecidable and routes the dependent judgment to silence; an Ambiguous
        // boundary poisons the embedded layer's unit-count-dependent judgments. The
        // per-frame OrderHintBits (from the active sequence header) is threaded separately
        // for the § 7.3.7 same-OrderHintBits-in-TU check, since it spans CELUs.
        let celu_role = if is_frame_bearing(obu.header.obu_type) {
            // A frame-bearing OBU: derive its CELU facts AND its OrderHintBits contribution from
            // ONE core parse + resolution (F4 — no double parse). The OrderHintBits is gated on
            // the SAME resolution decision the facts use: a frame whose referenced sequence
            // header did not resolve to the active header contributes no bits (None), rather
            // than the stale active header's bits, so the § 7.3.7 same-OrderHintBits-in-TU check
            // is not fed a wrong-bits value. (The CELU tracker filters global frame-bearing OBUs
            // in `observe`.)
            let (facts, bits) = self.frame_celu_facts(obu, first_picture_in_tu, boundary);
            // AV2 § 7.3.7 (mirror line 655): the same-OrderHintBits judgment is over frame
            // UNITS, not OBUs (F1). Feed the accumulator per frame-unit boundary, so a
            // continuation OBU (a non-first tile group of an already-counted coded frame) does
            // not contribute a redundant — and possibly unresolved-`None` — value that would
            // poison the whole temporal unit's bits judgment:
            //
            // - `OpensNewUnit` notes the unit's resolved bits (`Some` or, when the opener does
            //   not resolve to its active header, `None` — an unresolved opener still soundly
            //   poisons, since the unit it opens has unknowable bits);
            // - `ContinuesUnit` is skipped: the unit's bits came from its opener;
            // - `Ambiguous` notes `None`: the OBU might open a unit whose bits are unknowable,
            //   so it soundly poisons (the Unknown invariant).
            //
            // Global frame-bearing OBUs (obu_xlayer_id == GLOBAL_XLAYER_ID) are excluded BEFORE
            // the accumulator, mirroring the CELU tracker's non-global filter in
            // [`CodedExtendedLayerTracker::observe`] (round-5 F3). Such an OBU is invalid (a
            // frame-bearing OBU may not use the global xlayer — already diagnosed by
            // `obu-header/global-xlayer-allowed-types`) and is not part of any coded extended
            // layer unit (§ 7.3.6), so it never resolves an active sequence header and would
            // feed a spurious `None` that poisons the § 7.3.7 same-OrderHintBits judgment for
            // the valid CELUs in the temporal unit, suppressing a real bits mismatch.
            if !obu.header.extended_layer_id.is_global() {
                match facts.boundary {
                    FrameBoundary::OpensNewUnit => self.celu.note_order_hint_bits(bits, obu.offset),
                    FrameBoundary::ContinuesUnit => {}
                    FrameBoundary::Ambiguous => self.celu.note_order_hint_bits(None, obu.offset),
                }
            }
            CeluRole::Frame(facts)
        } else {
            self.celu_role_for(obu)
        };
        self.celu.observe(obu, celu_role, report);

        // AV2 § 7.23: maintain the per-extended-layer reference-frame buffer state. A
        // frame-bearing OBU that OPENS a new coded frame first commits the previous
        // frame's pending § 7.23 update (decode_frame_wrapup runs after that frame was
        // decoded, so its update must land before this frame's reference checks read the
        // buffer), then runs this frame's reference checks against the post-update buffer
        // and records this frame's own pending update. A non-frame OBU and a
        // continuation (non-first tile group) leave the pending update untouched.
        if is_frame_bearing(obu.header.obu_type) {
            self.observe_reference_state(obu, first_picture_in_tu, boundary, report);
        }

        // AV2 § 6.16.5 / § 6.16.6: mark this embedded layer's first coded picture
        // seen once its first coded frame (any frame-bearing OBU) is observed, for
        // the "shall be indicated at the first coded picture" check. A SEF is a
        // coded picture too. Record the *first* picture's temporal-unit index
        // (`or_insert`, not `insert`) so a later picture in the same CVS does not
        // overwrite it; the CLK hook prunes the entry at a new-CVS boundary.
        if is_frame_bearing(obu.header.obu_type) && !obu.header.extended_layer_id.is_global() {
            self.embedded_layer_first_picture_seen
                .entry((obu.header.extended_layer_id, obu.header.embedded_layer_id))
                .or_insert(self.cvs.tu_index);
        }
    }

    /// Flushes end-of-stream validator state. The end of the bitstream completes the
    /// final temporal unit (a coded video sequence continues "until the next closed
    /// random access point for that extended layer or the end of the bitstream",
    /// AV2 § 2), so the deferred cross-temporal-unit CVS-scoped diagnostics are
    /// flushed exactly as at a temporal-unit boundary — no CLK can arrive
    /// retroactively — and every scan-type scope retires, emitting the § 6.16.10
    /// unestablished-CI warning for scopes where no in-scope content interpretation
    /// ever established a non-zero `ci_scan_type_idc` (see
    /// [`ValidatorContext::flush_scan_type_scope`]). Called once by the validator
    /// after the last OBU.
    pub(crate) fn finish(&mut self, options: &ValidationOptions, report: &mut ValidationReport) {
        self.cvs.flush_completed_tu(report);
        // AV2 § 7.3.2 end condition 3: "The end of the bitstream." The final temporal
        // unit (which has no trailing global temporal delimiter) is completed here so
        // its § 7.3.2 begin/end facts are applied exactly as at an internal boundary.
        // Any deferred provisional-Inside § 6.4.1 monotonic disagreements that never saw
        // a CLK (the temporal unit stayed inside the CMVS until the end of the bitstream)
        // are emitted here. `cvs.tu_index` is the final temporal unit's index (no
        // advance_temporal_unit runs at the end of the bitstream), stamping the CMVS-window
        // start if this final temporal unit begins one.
        //
        // The CMVS-window start of the FINAL temporal unit, captured BEFORE
        // `complete_temporal_unit` applies this unit's § 7.3.2 begin/end conditions and
        // mutates the live window (round-6 F1, mirroring the internal-boundary capture). The
        // § 7.3.7 DOH flag for the final unit must be sampled against the CMVS that CONTAINS
        // it: when the end of the bitstream (end condition 3) ends a CMVS this final unit
        // CLOSED (a CLK with no MSDO, no activated global LCR — end condition 2), it is the
        // LAST temporal unit of the ENDING CMVS, so its governing window is this pre-completion
        // start — not the live window `complete_temporal_unit` is about to clear (see
        // [`Self::doh_constraint_flag_active_for_completed_tu`]).
        let cmvs_window_before_completion = self.cmvs.current_cmvs_start_tu_index();
        self.cmvs.complete_temporal_unit(self.cvs.tu_index, report);
        // AV2 § 6.6: resolve the deferred `msdo/doh-constraint-required` check for the
        // final temporal unit's frame-confirmed activations, exactly as an internal
        // boundary would (see resolve_deferred_doh_constraint).
        self.resolve_deferred_doh_constraint(options, report);
        // AV2 § 6.8.2: resolve the deferred MSDO↔global-LCR agreement and LCR DOH
        // requirement for the final temporal unit, exactly as an internal boundary would.
        self.resolve_deferred_lcr_msdo_agreement(options, report);
        // AV2 § 7.3.2: resolve the boundary-set-identity check for the final temporal
        // unit, exactly as an internal boundary would.
        self.resolve_deferred_cmvs_boundary(options, report);
        // Annex A Table A.4: commit the final temporal unit's IOP pending facts, then flush
        // and evaluate the final coded-(multistream-)video-sequence window — the end of the
        // bitstream ends the final coded video sequence (AV2 § 2 / § 7.3.2 end condition 3),
        // so its MSDO/LCR presence requirements are evaluated here.
        self.commit_annex_a_iop_pending(self.cvs.tu_index, options, report);
        self.flush_annex_a_iop_window(options, report);
        // AV2 § 7.3.8.2: the final temporal unit (which has no trailing global temporal
        // delimiter) is resolved here, exactly as an internal boundary would, so a
        // buffered final-TU OBU_MSDO is compared against the previous one.
        self.msdo_identity.complete_temporal_unit(report);
        // AV2 § 7.3.8.1: resolve the final temporal unit's buffered HLS-availability
        // replay references, exactly as an internal boundary would. `cvs.tu_index` is the
        // final temporal unit's index (no advance runs at the end of the bitstream).
        self.complete_rap_replay_tu(self.cvs.tu_index, options, report);
        let scope_keys: Vec<ExtendedLayerId> = self.scan_type.scopes.keys().copied().collect();
        for scope_key in scope_keys {
            self.flush_scan_type_scope(scope_key, u64::MAX, report);
        }
        // AV2 § 6.16.7 / § 7.3.6: the end of the bitstream ends the final coded video
        // sequence with no further CLK, so any deferred inference-presence diagnostic
        // whose earlier-temporal-unit seed survived stayed intra-CVS and inferred
        // cleanly — drop the survivors silently (see TimecodeCvsState::pending_inference).
        self.drop_pending_timecode_inference();
        // AV2 § 7.3.3 / § 7.3.4: the end of the bitstream ends the final temporal
        // unit (no trailing global temporal delimiter), so resolve its open coded
        // frame units' deferred checks exactly as a temporal-delimiter boundary would.
        self.frame_unit.finish(report);
        // AV2 § 7.3.6 / § 7.3.7 / § 7.4.6: the end of the bitstream ends the final
        // temporal unit, so resolve its coded-extended-layer-unit constraints and the
        // flag-gated DOH OrderHint / OrderHintBits checks exactly as an internal boundary
        // would. The DOH flag is recorded from the final temporal unit's activated global
        // LCR / preceding MSDO before resolution. The LCR side is sampled against the
        // GOVERNING window of the final unit (captured before `complete_temporal_unit` cleared
        // the live window, round-6 F1) — symmetric with the internal-boundary path — so a CLK
        // final unit that ends the CMVS at the end of the bitstream is still governed by the
        // activated global LCR of the CMVS that contained it (the MSDO side is window-
        // independent live last-wins state).
        self.celu.set_doh_flag_active(
            self.doh_constraint_flag_active_for_completed_tu(cmvs_window_before_completion),
        );
        self.celu.finish(report);
        // AV2 § 7.3.6 (round-6 F3): resolve the final temporal unit's CIs against the
        // first-coded-extended-layer-unit-of-the-sequence presence rule, exactly as an
        // internal boundary would. `cvs.tu_index` is the final temporal unit's index (no
        // advance runs at the end of the bitstream); its CLK boundary events were already
        // applied at the CLK OBU, so each CI's CVS membership is final.
        self.resolve_ci_first_celu_for_tu(self.cvs.tu_index, options, report);
        // AV2 § 7.23: the final frame has no following coded-frame boundary, so its
        // deferred reference-frame-update process runs at the end of the bitstream. This
        // commit keeps the modeled buffer consistent with the decoded state (no reference
        // check reads it after the end of the bitstream, but the flush is the symmetric
        // no-trailing-delimiter completion of the per-frame deferral).
        self.commit_pending_ref_update();
    }

    /// Returns the per-extended-layer `FirstPictureInTU` — "the first frame unit in
    /// a coded extended layer unit in a temporal unit" (AV2 § 6.17.2): `true` until
    /// a frame-bearing OBU for `xlayer` is observed in the current temporal unit.
    pub(crate) fn first_picture_in_tu(&self, xlayer: ExtendedLayerId) -> bool {
        !self.frames_seen_in_tu.contains(&xlayer)
    }
}
