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
    FrameHeaderCopyOutcome, RecordedFrameHeaderBits, TileGroupLayout, TileGroupStructureOutcome,
    parse_frame_header_copy, parse_tile_group_prefix, parse_tile_group_structure,
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

/// Maximum conformant `num_*_points` for a film-grain scaling function
/// (AV2 v1.0.0 § 6.17.10.2).
const MAX_FILM_GRAIN_SCALING_POINTS: u8 = 14;

/// Smallest `seq_level_idx` at which the High tier (`seq_tier == 1`) may be signaled
/// (Annex A.4 Table A.7 maps LevelIdx 4 to level 4.0, mirror line 281; the Table A.9
/// NOTE, mirror lines 436-437, restricts High tier to "level 4.0 and above").
const HIGH_TIER_MIN_LEVEL_IDX: u8 = 4;

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
        BTreeMap<(ExtendedLayerId, EmbeddedLayerId, TemporalLayerId), RecordedFrameHeaderBits>,
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

/// One in-band global layer configuration record's § 6.8.2 agreement fields (AV2
/// § 5.8.1): the aggregate info, per-substream PTL info indexed by `obu_xlayer_id`, the
/// DOH-constraint flag, and the defining OBU's byte offset for the diagnostic anchor.
/// Stored alongside the [`HlsAvailabilityStore`]'s `lcr_xlayer_map` (which the
/// availability / association chain consumes) so the MSDO↔global-LCR agreement can read
/// the full record of whichever global LCR the chain resolved as *activated*.
#[derive(Debug, Clone)]
struct GlobalLcrRecord {
    /// `LcrMaxNumXLayerCount` = the set-bit count of `lcr_xlayer_map` (AV2 § 5.8.1,
    /// mirror `06-syntax-structures-semantics.md` lines 382-384): the § 6.8.2 constraint-1
    /// stream count and the Table A.3 extended-layer count under an activated global LCR.
    max_num_xlayer_count: u32,
    /// `LcrXLayerID[]` = the set-bit indices of `lcr_xlayer_map`, ascending (AV2 § 5.8.1):
    /// the § 6.8.2 constraint-2 membership set.
    xlayer_ids: BTreeSet<u8>,
    /// `lcr_aggregate_info()` when `lcr_aggregate_info_present_flag == 1` (§ 6.8.2
    /// constraint 3, lines 1657-1664).
    aggregate_info: Option<LcrAggregateInfo>,
    /// `lcr_seq_profile_idc[i]` / `lcr_max_level_idx[i]` / `lcr_tier_flag[i]` indexed by
    /// `obu_xlayer_id` (i), present when `lcr_seq_profile_tier_level_info_present_flag ==
    /// 1` (§ 6.8.2 constraint 4 (the "1." numbered as constraint 4), lines 1666-1671).
    seq_ptl_by_xlayer: BTreeMap<u8, LcrSeqPtl>,
    /// `lcr_seq_profile_tier_level_info_present_flag`.
    seq_ptl_present: bool,
    /// `lcr_doh_constraint_flag` (§ 6.8.2 constraint 5 and the § 6.8.2 DOH requirement,
    /// lines 1619-1621 / 1673).
    doh_constraint_flag: bool,
    /// Byte offset of the OBU that defined this record, for the diagnostic anchor.
    offset: ByteOffset,
    /// The [`CvsTracker::tu_index`] at which this record's defining OBU was observed (a
    /// redefinition restamps it). The § 6.8.2 agreement applies only when the global LCR is
    /// "present in the same coded multistream video sequence" (mirror lines 1646-1648): the
    /// snapshot of this record taken at association time carries its observation temporal
    /// unit, and the deferred resolution requires that temporal unit to lie within the
    /// current CMVS window (`>= cmvs_start_tu_index`) so a record observed only in an
    /// earlier CMVS does not leak into a later MSDO-only CMVS's evaluation (codex finding
    /// 3393129738).
    observed_tu_index: u64,
}

/// One global LCR's `lcr_seq_profile_tier_level_info(i)` PTL ceiling, indexed by
/// `obu_xlayer_id` in [`GlobalLcrRecord::seq_ptl_by_xlayer`] (AV2 § 5.8.4).
#[derive(Debug, Clone, Copy)]
struct LcrSeqPtl {
    /// `lcr_seq_profile_idc[i]`.
    seq_profile_idc: u8,
    /// `lcr_max_level_idx[i]`.
    max_level_idx: u8,
    /// `lcr_tier_flag[i]` as `0`/`1`.
    tier_flag: u8,
}

/// The Annex A Table A.4 interoperability-point OBU-presence tracker (AV2 v1.0.0 Annex A.2
/// Table A.4, mirror `annex-a-profiles-levels-and-tiers.md` lines 178-201), scoped to a
/// coded (multistream-)video-sequence window.
///
/// The window spans the whole coded video sequence and is evaluated at its end — the start
/// of the next coded video sequence (a CLK in a *later* temporal unit, § 7.3.6) or the end
/// of the bitstream. Per-temporal-unit observations accumulate in [`Self::pending`]; at
/// temporal-unit completion they are committed to the right window (lesson 8): a temporal
/// unit that begins a new coded video sequence (has a CLK while a window opened in an
/// earlier temporal unit is still open) first flushes the prior window, then seeds a fresh
/// window from *this* temporal unit's pending facts — so an OBU_MSDO (or any HLS) observed
/// BEFORE the CLK in the CLK-bearing temporal unit belongs to the NEW coded video sequence,
/// not the prior one (§ 7.3.6: the new coded video sequence starts at the temporal unit
/// containing the CLK).
///
/// The presence-requirement evaluation needs frame-confirmed activation state that is only
/// final at temporal-unit completion (which sequence headers are activated, whether a
/// global LCR is *activated*, and the MSDO's `multistream_profile_idc`), so the window's
/// interoperability point, extended/embedded-layer counts, and activated-global-LCR flag
/// are resolved from the live context at flush time, not accumulated per-OBU.
#[derive(Debug, Default)]
struct AnnexAIopTracker {
    /// The currently-open coded-video-sequence window, or `None` before the first
    /// observation.
    window: Option<AnnexAIopWindow>,
    /// The temporal unit currently being observed, committed to the window at temporal-unit
    /// completion (see [`Self::commit_pending`]).
    pending: TuIopFacts,
}

/// One temporal unit's Annex A Table A.4 facts, accumulated as OBUs are observed and
/// committed to the [`AnnexAIopTracker`]'s window when the temporal unit completes.
#[derive(Debug, Default, Clone)]
struct TuIopFacts {
    /// Distinct non-global `obu_xlayer_id` values observed in this temporal unit (Table A.3
    /// extended-layer base count, mirror lines 146-151).
    distinct_xlayers: BTreeSet<ExtendedLayerId>,
    /// The largest `num_streams_minus_2 + 2` of any OBU_MSDO in this temporal unit, with
    /// the OBU offset, when present.
    msdo: Option<(u32, ByteOffset)>,
    /// `multistream_profile_idc` of the OBU_MSDO in this temporal unit (the Table A.4 IOP
    /// source when an MSDO is present), when present.
    msdo_profile_idc: Option<u8>,
    /// A local layer configuration record OBU was present in this temporal unit.
    local_lcr_present: bool,
    /// A global layer configuration record OBU was present in this temporal unit (raw
    /// presence; activation is resolved separately at flush).
    global_lcr_present: bool,
    /// This temporal unit contains an `OBU_CLOSED_LOOP_KEY` for at least one extended layer
    /// (§ 7.3.6: begins a new coded video sequence for that layer).
    has_clk: bool,
    /// The interoperability point agreed by the *frame-confirmed* sequence headers activated
    /// in this temporal unit, when no MSDO IOP overrides it (the MSDO's
    /// `multistream_profile_idc` is the IOP source when an MSDO is present, mirror lines
    /// 1659-1662). `None` until an activation with a table-mapped profile occurs.
    iop: Option<AnnexAIopState>,
    /// The maximum `seq_max_mlayer_cnt_minus_1 + 1` across frame-confirmed activated headers
    /// in this temporal unit (Table A.3 "Number of Embedded Layers", mirror lines 152-153).
    max_embedded_layers: u32,
    /// `LcrMaxNumXLayerCount` of an *activated* global LCR resolved from a frame-confirmed
    /// activation in this temporal unit, when one resolves (the Table A.3 declared
    /// extended-layer count under an activated global LCR, mirror lines 149-150, and the
    /// signal the Table A.4 global-LCR arms require — only an activated global LCR counts).
    activated_global_count: Option<u32>,
    /// Byte offset of the latest evidence-bearing OBU in this temporal unit, for the
    /// diagnostic anchor.
    anchor_offset: Option<ByteOffset>,
}

/// The interoperability-point state of an Annex A IOP window: a single agreed IOP, or
/// `Mixed` when activated profiles disagree (the Table A.4 row is then not determinable,
/// so the check is skipped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnnexAIopState {
    /// A single agreed interoperability point.
    Single(InteroperabilityPoint),
    /// Activated profiles disagree on the interoperability point; skip the check.
    Mixed,
}

/// One coded-(multistream-)video-sequence window's accumulated Annex A Table A.4 evidence.
#[derive(Debug, Clone)]
struct AnnexAIopWindow {
    /// Distinct non-global `obu_xlayer_id` values observed across the window (Table A.3
    /// extended-layer base count).
    distinct_xlayers: BTreeSet<ExtendedLayerId>,
    /// The largest `num_streams_minus_2 + 2` of any OBU_MSDO in the window, when present —
    /// the declared Table A.3 extended-layer count under `MultiStreamDecoderMode == 1`
    /// (mirror lines 148-149).
    msdo_num_streams: Option<u32>,
    /// `multistream_profile_idc` of an OBU_MSDO in the window, the Table A.4 interoperability
    /// point source when an MSDO is present (mirror lines 1659-1662). `None` when no MSDO is
    /// in the window.
    msdo_profile_idc: Option<u8>,
    /// `true` if an `OBU_MSDO` occurred in the window.
    msdo_present: bool,
    /// `true` if a local `OBU_LAYER_CONFIGURATION_RECORD` was present in the window.
    local_lcr_present: bool,
    /// `LcrMaxNumXLayerCount` of an *activated* global LCR resolved in the window, when one
    /// resolved. Only an activated global LCR satisfies the Table A.4 global-LCR arms and
    /// contributes the Table A.3 declared extended-layer count; an observed-but-unactivated
    /// global LCR leaves this `None`.
    activated_global_count: Option<u32>,
    /// The interoperability point agreed by the window's frame-confirmed activated headers,
    /// or `None`/`Mixed` when undecidable. Overridden by the MSDO's `multistream_profile_idc`
    /// at evaluation when an MSDO is present (mirror lines 1659-1662).
    iop: Option<AnnexAIopState>,
    /// The maximum `seq_max_mlayer_cnt_minus_1 + 1` across the window's activated headers
    /// (Table A.3 "Number of Embedded Layers", mirror lines 152-153).
    max_embedded_layers: u32,
    /// Byte offset anchoring the window's diagnostic — the latest evidence-bearing OBU.
    anchor_offset: ByteOffset,
    /// The [`CvsTracker::tu_index`] of the temporal unit in which this window's coded video
    /// sequence began (the temporal unit carrying its CLKs), or `None` for leading evidence
    /// before the first CLK. A CLK in a *later* temporal unit begins the next coded video
    /// sequence and flushes this window (§ 7.3.6).
    cvs_start_tu: Option<u64>,
}

impl Default for AnnexAIopWindow {
    fn default() -> Self {
        Self {
            distinct_xlayers: BTreeSet::new(),
            msdo_num_streams: None,
            msdo_profile_idc: None,
            msdo_present: false,
            local_lcr_present: false,
            activated_global_count: None,
            iop: None,
            max_embedded_layers: 0,
            anchor_offset: ByteOffset::new(0),
            cvs_start_tu: None,
        }
    }
}

impl AnnexAIopTracker {
    /// Records a non-global `obu_xlayer_id` observed in the current temporal unit (Table
    /// A.3 extended-layer base count).
    fn note_xlayer(&mut self, xlayer: ExtendedLayerId) {
        if !xlayer.is_global() {
            self.pending.distinct_xlayers.insert(xlayer);
        }
    }

    /// Records an OBU_MSDO observed in the current temporal unit: its declared substream
    /// count, `multistream_profile_idc` (the Table A.4 IOP source), and OBU offset.
    fn note_msdo(&mut self, num_streams: u32, profile_idc: u8, offset: ByteOffset) {
        let best = self
            .pending
            .msdo
            .map_or(num_streams, |(prev, _)| prev.max(num_streams));
        self.pending.msdo = Some((best, offset));
        self.pending.msdo_profile_idc = Some(profile_idc);
    }

    /// Records that a global LCR OBU was present in the current temporal unit.
    fn note_global_lcr(&mut self, offset: ByteOffset) {
        self.pending.global_lcr_present = true;
        self.pending.anchor_offset = Some(offset);
    }

    /// Records that a local LCR OBU was present in the current temporal unit.
    fn note_local_lcr(&mut self) {
        self.pending.local_lcr_present = true;
    }

    /// Records that the current temporal unit contains an `OBU_CLOSED_LOOP_KEY`.
    fn note_clk(&mut self) {
        self.pending.has_clk = true;
    }

    /// Records a frame-confirmed sequence-header activation in the current temporal unit:
    /// its profile's interoperability point (Annex A.2 Table A.1), its embedded-layer count
    /// (`seq_max_mlayer_cnt_minus_1 + 1`), the `LcrMaxNumXLayerCount` of the *activated*
    /// global LCR it resolves (if any — only an activated global LCR is recorded here), and
    /// the activating OBU offset. A reserved / Configurable profile leaves the IOP unset
    /// (its interoperability point is not table-determined); two activations disagreeing on
    /// the IOP mark the window [`AnnexAIopState::Mixed`] and the Table A.4 check is then
    /// skipped (multistream profile-agreement is out of scope here).
    fn note_activation(
        &mut self,
        profile_idc: u8,
        embedded_layers: u32,
        activated_global_count: Option<u32>,
        offset: ByteOffset,
    ) {
        self.pending.max_embedded_layers = self.pending.max_embedded_layers.max(embedded_layers);
        self.pending.anchor_offset = Some(offset);
        if let Some(count) = activated_global_count {
            self.pending.activated_global_count =
                Some(self.pending.activated_global_count.unwrap_or(0).max(count));
        }
        if let Some(iop) = interoperability_point(profile_idc) {
            self.pending.iop = Some(match self.pending.iop {
                None => AnnexAIopState::Single(iop),
                Some(AnnexAIopState::Single(existing)) if existing == iop => {
                    AnnexAIopState::Single(existing)
                }
                Some(_) => AnnexAIopState::Mixed,
            });
        }
    }

    /// Whether committing the current temporal unit's pending facts begins a NEW coded
    /// video sequence relative to the open window — a CLK in this temporal unit while a
    /// window whose coded video sequence began in an *earlier* temporal unit is open. A CLK
    /// in the same temporal unit the window's coded video sequence began in (a second
    /// extended layer's CLK within one multistream random-access temporal unit) continues
    /// the same window; leading evidence with no recorded coded-video-sequence start
    /// (`cvs_start_tu == None`) is absorbed by the first coded video sequence.
    fn pending_starts_new_cvs(&self, tu_index: u64) -> bool {
        self.pending.has_clk
            && matches!(
                self.window.as_ref().and_then(|w| w.cvs_start_tu),
                Some(start) if start != tu_index
            )
    }

    /// Merges the current temporal unit's pending facts into `window` (the same coded video
    /// sequence continues across this temporal unit), recording the coded-video-sequence
    /// start temporal unit when this temporal unit carries the window's CLK.
    fn merge_pending_into(window: &mut AnnexAIopWindow, pending: &TuIopFacts, tu_index: u64) {
        window
            .distinct_xlayers
            .extend(pending.distinct_xlayers.iter().copied());
        if let Some((num_streams, offset)) = pending.msdo {
            window.msdo_present = true;
            window.msdo_num_streams = Some(window.msdo_num_streams.unwrap_or(0).max(num_streams));
            window.anchor_offset = offset;
        }
        if let Some(profile) = pending.msdo_profile_idc {
            window.msdo_profile_idc = Some(profile);
        }
        window.local_lcr_present |= pending.local_lcr_present;
        if let Some(count) = pending.activated_global_count {
            window.activated_global_count =
                Some(window.activated_global_count.unwrap_or(0).max(count));
        }
        window.max_embedded_layers = window.max_embedded_layers.max(pending.max_embedded_layers);
        if let Some(offset) = pending.anchor_offset {
            window.anchor_offset = offset;
        }
        // Combine this temporal unit's IOP into the window's: a single agreed IOP carries
        // through; a disagreement marks the window Mixed (the Table A.4 row is then not
        // determinable, so the check is skipped).
        window.iop = match (window.iop, pending.iop) {
            (None, p) => p,
            (w, None) => w,
            (Some(AnnexAIopState::Single(a)), Some(AnnexAIopState::Single(b))) if a == b => {
                Some(AnnexAIopState::Single(a))
            }
            _ => Some(AnnexAIopState::Mixed),
        };
        if pending.has_clk {
            window.cvs_start_tu.get_or_insert(tu_index);
        }
    }

    /// Builds a fresh window from a temporal unit's pending facts (a temporal unit that
    /// begins a new coded video sequence). The new window's coded-video-sequence start is
    /// this temporal unit.
    fn window_from_pending(pending: &TuIopFacts, tu_index: u64) -> AnnexAIopWindow {
        let mut window = AnnexAIopWindow::default();
        Self::merge_pending_into(&mut window, pending, tu_index);
        window
    }
}

/// The Table A.3 "Number of Extended Layers" for an [`AnnexAIopWindow`] (mirror lines
/// 146-151), in the mirror's exact definition order — a *declared* count takes precedence
/// over the observed coded structure:
///
/// 1. `MultiStreamDecoderMode == 1` (an OBU_MSDO is present): `num_streams_minus_2 + 2`
///    (mirror lines 148-149), regardless of how many distinct `obu_xlayer_id` materialize.
/// 2. else, an *activated* global LCR (`window.activated_global_count` resolved):
///    `LcrMaxNumXLayerCount` (mirror lines 149-150).
/// 3. else: the distinct non-global `obu_xlayer_id` count actually present, at least 1
///    (mirror lines 150-151; Table A.3 "For a coded video sequence, this value is equal to
///    1").
///
/// `window.activated_global_count` is `None` when no activated global LCR resolves, so an
/// observed-but-unactivated global LCR does not contribute a declared count (it falls
/// through to the observed distinct count).
fn annex_a_extended_layers(window: &AnnexAIopWindow) -> u32 {
    if let Some(num_streams) = window.msdo_num_streams {
        return num_streams;
    }
    if let Some(count) = window.activated_global_count {
        return count;
    }
    (window.distinct_xlayers.len() as u32).max(1)
}

/// The § 6.6 sub-stream PTL ceilings of the most recently observed OBU_MSDO, indexed by
/// `sub_xlayer_id[i]` (AV2 v1.0.0 § 6.6, mirror `06-syntax-structures-semantics.md`
/// lines 1359-1378). A sequence header activated by the i-th independent sub-stream is
/// the header active for the extended layer whose `obu_xlayer_id` equals
/// `sub_xlayer_id[i]`; its `seq_profile_idc` / `seq_level_idx` / `seq_tier` must not
/// exceed the ceilings recorded here.
#[derive(Debug, Clone)]
struct MsdoSubstreamMax {
    /// `sub_xlayer_id[i] → (sub_stream_max_profile[i], sub_stream_max_level[i],
    /// sub_stream_max_tier[i])`. § 6.6 imposes the ceiling "for each sequence header
    /// activated by the i-th independent sub-stream", i.e. for EACH i. The spec states no
    /// uniqueness requirement on `sub_xlayer_id` (see the proposal's roadmap-hygiene
    /// note), so two i values may name the same extended layer; a header activated by
    /// that layer must then satisfy both ceilings, so a duplicate `sub_xlayer_id` keeps
    /// the most restrictive (per-dimension minimum) maximum rather than letting a
    /// last-wins insert discard the tighter ceiling (recorded in `observe_msdo`; codex
    /// finding 3392940071).
    ceilings: BTreeMap<u8, SubStreamCeiling>,
    /// `multistream_doh_constraint_flag` of the recorded MSDO, for the § 6.6
    /// DOH-constraint requirement (`msdo/doh-constraint-required`).
    doh_constraint_flag: bool,
    /// Byte offset of the OBU_MSDO that declared these ceilings, for the diagnostic
    /// anchor when the violation is detected at sequence-header activation time.
    ///
    /// The § 6.8.2 MSDO↔global-LCR agreement uses the separate per-MSDO
    /// [`ValidatorContext::msdo_agreement_snapshots`] accumulator (it must evaluate EVERY
    /// MSDO in the CMVS, not just this live last-wins one), so the raw declaration-order
    /// aggregate / observation temporal unit are kept there rather than on this § 6.6 record.
    offset: ByteOffset,
}

/// The § 6.6 MSDO aggregate fields and per-substream declaration-order entries kept for
/// the § 6.8.2 MSDO↔global-LCR agreement and the Table A.4 interoperability-point window
/// (AV2 § 5.6, mirror `06-syntax-structures-semantics.md` lines 1646-1673). Distinct from
/// the per-layer [`SubStreamCeiling`] merge: § 6.8.2 constraints 1/2/4 are per-declaration
/// (`num_streams_minus_2 + 1` entries, each carrying its `sub_xlayer_id[i]`), not the
/// most-restrictive per-layer view § 6.6 uses.
#[derive(Debug, Clone)]
struct MsdoAggregate {
    /// `num_streams_minus_2 + 2` (AV2 § 5.6); the § 6.8.2 constraint-1 stream count and
    /// the Table A.3 extended-layer count (mirror lines 148-149).
    num_streams: u32,
    /// `multistream_profile_idc` (AV2 § 5.6); the § 6.8.2 constraint-3 aggregate-profile
    /// value and the Table A.4 interoperability-point source (mirror lines 1659-1662).
    profile_idc: u8,
    /// `multistream_level_idx` (AV2 § 5.6); § 6.8.2 constraint-3 level equality (line
    /// 1663).
    level_idx: u8,
    /// `multistream_tier` (AV2 § 5.6); § 6.8.2 constraint-3 tier equality (line 1664).
    tier: u8,
    /// `multistream_doh_constraint_flag` (AV2 § 5.6); § 6.8.2 constraint-5 DOH-flag equality
    /// (line 1673). Snapshotted with the rest of the declaration so the agreement check
    /// operates entirely on its `MsdoAggregate` argument rather than reaching back into the
    /// live `msdo_substream_max` (which a later same-CMVS MSDO could retarget).
    doh_constraint_flag: bool,
    /// The per-declaration `sub_xlayer_id[i]` / `sub_stream_max_*[i]` entries in
    /// declaration order (`0..=num_streams_minus_2 + 1`), for § 6.8.2 constraints 2 and 4
    /// (lines 1651-1671).
    sub_streams: Vec<MsdoSubStream>,
}

/// One accumulated OBU_MSDO snapshot for the § 6.8.2 MSDO↔global-LCR *agreement*
/// ([`ValidatorContext::msdo_agreement_snapshots`]). Distinct from the live last-wins
/// [`MsdoSubstreamMax`]: the agreement must hold for every MSDO present in the CMVS, so each
/// observed MSDO is retained (deduped by `offset`) and evaluated against the resolved
/// activated global LCR at deferred resolution.
#[derive(Debug, Clone)]
struct MsdoWindowSnapshot {
    /// The § 6.8.2 aggregate / per-substream fields this MSDO declared.
    aggregate: MsdoAggregate,
    /// Byte offset of the OBU_MSDO, for the diagnostic anchor and the dedup key.
    offset: ByteOffset,
    /// The [`CvsTracker::tu_index`] at which this MSDO was observed, for the § 6.8.2 "present
    /// in the same CMVS" window filter (`>= cmvs_start_tu_index`).
    observed_tu_index: u64,
}

/// One § 5.6 per-substream declaration (`sub_xlayer_id[i]` and the `sub_stream_max_*[i]`
/// PTL ceiling), kept in declaration order for the § 6.8.2 per-substream equality checks.
#[derive(Debug, Clone, Copy)]
struct MsdoSubStream {
    /// `sub_xlayer_id[i]`.
    sub_xlayer_id: u8,
    /// `sub_stream_max_profile[i]`.
    max_profile: u8,
    /// `sub_stream_max_level[i]`.
    max_level: u8,
    /// `sub_stream_max_tier[i]`.
    max_tier: u8,
}

/// One sub-stream's § 6.6 PTL ceiling (`sub_stream_max_profile` / `sub_stream_max_level`
/// / `sub_stream_max_tier`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SubStreamCeiling {
    max_profile: u8,
    max_level: u8,
    max_tier: u8,
}

/// Dedup key for the § 6.6 sub-stream PTL-ceiling findings: the activated header, its
/// coded-video-sequence epoch, the MSDO ceiling in force for the layer, and a
/// fingerprint of the activated header's checked value-space fields. A redefinition that
/// changes a checked field, or a new MSDO with a different ceiling, yields a distinct
/// key and re-emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SubstreamMaxFindingKey {
    xlayer: ExtendedLayerId,
    seq_header_id: SequenceHeaderId,
    cvs_epoch: u64,
    ceiling: SubStreamCeiling,
    value_space: AnnexAValueSpaceFingerprint,
}

/// Comparison key for the § 6.10.5 operating-point buffer-delay sum-constancy check:
/// the `(obu_xlayer_id, opsID, op)` triple Annex E binds the delays to
/// (`annex-e-decoder-model.md` lines 100–112).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct OpsBufferDelayKey {
    xlayer: ExtendedLayerId,
    ops_id: u8,
    op_index: u8,
}

/// The boundary scope of one § 6.10.5 buffer-delay observation: the per-extended-layer
/// CVS epoch, the per-extended-layer § 6.10.1 effective reset generation (global resets
/// plus that layer's local resets), and the per-OPS targeted-reset generation. Two
/// observations share the same scope — and so are subject to the error tier rather than
/// the cross-boundary advisory — only when all three match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BufferDelayScope {
    /// [`CvsTracker::cvs_generation_epoch`] of the OPS's extended layer (or the
    /// multistream-wide global epoch) at the observation.
    cvs_epoch: u64,
    /// [`OpsAvailabilityStore::effective_reset_generation`] of the observation's
    /// extended layer (global resets plus that layer's local resets), including the
    /// defining OPS's own `ops_reset_flag`. Per-layer (round-2): a reset of an unrelated
    /// extended layer no longer changes this scope.
    reset_generation: u64,
    /// [`OpsAvailabilityStore::targeted_reset_generation`] for the observation's
    /// `(obu_xlayer_id, opsID)`. A § 6.10.1 case-3 targeted reset of this OPS bumps it,
    /// re-baselining the comparison for exactly this OPS without disturbing any other.
    targeted_reset_generation: u64,
}

/// One stored operating-point buffer-delay baseline (§ 6.10.5): the last explicitly
/// signalled sum together with the boundary scope and temporal unit that scope the
/// error-tier comparison.
#[derive(Debug, Clone, Copy)]
struct BufferDelayBaseline {
    /// `ops_decoder_buffer_delay + ops_encoder_buffer_delay`, summed as `u64` so the
    /// two `u32` `uvlc()` values cannot overflow the comparison.
    sum: u64,
    /// The CVS / reset / targeted-reset scope at the baseline observation.
    scope: BufferDelayScope,
    /// [`CvsTracker::tu_index`] of the baseline observation. A CVS boundary is
    /// temporal-unit-granular (§ 7.3.6), so a baseline observed in an earlier temporal
    /// unit may be split into a different coded video sequence by a CLK later in the
    /// current temporal unit; the error-tier comparison is therefore routed through
    /// [`CvsTracker::defer_or_emit`] on this index.
    tu_index: u64,
}

/// Builds the § 6.10.5 operating-point cross-boundary advisory
/// (`decoder-model/buffer-delay-sum-changed-across-cvs`, severity `warning`) for a
/// change of the explicitly signalled buffer-delay sum from `previous_sum` to `sum` for
/// the `(obu_xlayer_id, opsID, op)` triple `key`. Shared by the eager cross-boundary
/// check (`check_ops_buffer_delay_cross_cvs`) and the deferred-error replacement path
/// (`check_ops_buffer_delay_sums`), where it is the `on_drop` diagnostic emitted when a
/// late CLK reveals the deferred intra-CVS error to be a genuine cross-CVS change.
fn ops_buffer_delay_cross_cvs_warning(
    key: &OpsBufferDelayKey,
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
fn ops_buffer_delay_intra_cvs_error(
    key: &OpsBufferDelayKey,
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
struct SeqBufferDelayBaseline {
    /// `decoder_buffer_delay + encoder_buffer_delay`, summed as `u64`.
    sum: u64,
    /// [`CvsTracker::cvs_generation_epoch`] of the extended layer at the baseline
    /// observation.
    cvs_epoch: u64,
    /// `seq_header_id` of the header that established the baseline, cited in the
    /// advisory message.
    seq_header_id: SequenceHeaderId,
}

/// The § 6.4.1 LCR association of one observed sequence header; see
/// [`ValidatorContext::lcr_associations`].
#[derive(Debug, Clone)]
struct LcrAssociation {
    /// `true` when the association resolved to a global LCR (no local record
    /// with the id existed in the header's extended layer at observation).
    lcr_is_global: bool,
    /// The associated record's id (the header's `seq_lcr_id`).
    lcr_id: u8,
    /// The record's § 5.8.8 embedded-layer maps at observation time; `None`
    /// when it carried no embedded-layer info (§ 6.8.9 binds "if present").
    maps: Option<LcrEmbeddedMaps>,
    /// The full § 6.8.2 agreement fields of the associated *global* LCR as observed at
    /// association time (a clone of the [`GlobalLcrRecord`] present prior to this header),
    /// or `None` when the association is local. The § 6.8.2 MSDO↔global-LCR agreement and
    /// DOH requirement read this snapshot rather than the live `global_lcr_records` map, so
    /// a same-id global-LCR redefinition *after* this header associated does not retarget
    /// the agreement at the later revision (codex finding 3393129741) — mirroring the
    /// existing § 6.8.9 dependency path, which also snapshots its associated maps.
    global_record: Option<GlobalLcrRecord>,
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
    ptl: Option<LcrPtlSnapshot>,
    /// The § 5.8.7 rep info of the associated LCR for this extended layer, snapshotted
    /// at association time for the § 6.8.8 equality checks (local record's
    /// `lcr_rep_info(0, xId)` for a local association, the global payload's
    /// `lcr_rep_info(1, xId)` for a global one). `None` when the associated record
    /// carried no rep info for the layer (absent rep-info compares nothing).
    rep_info: Option<LcrRepInfoSnapshot>,
}

/// Availability of in-band HLS objects, for the § 7.3.8 reference checks.
///
/// Sequence-header availability (§ 7.3.8.6), multi-frame-header availability
/// (§ 7.3.8.7), layer-configuration-record availability (§ 7.3.8.3), and local
/// atlas-segment availability (§ 7.3.8.4) are modeled. MSDO / OPS availability records
/// and the global atlas reference (§ 7.3.8.4 uses "can be available", not a hard
/// requirement) remain deferred.
///
/// The store is kept **monotonic** (entries are never removed): an object included
/// earlier in the bitstream stays available, so the validator never falsely reports
/// it unavailable. AV2 § 7.3.8.1's "HLS OBUs must be resent at each random access
/// point" requirement needs random-access state to model and is intentionally not
/// enforced (a sound-over-complete false negative).
#[derive(Debug, Default)]
struct HlsAvailabilityStore {
    /// `seq_header_id` values of sequence headers seen in-band so far (§ 7.3.8.6).
    sequence_header_ids: BTreeSet<u32>,
    /// Multi-frame-header records keyed by `mfhId`, seen in-band so far (§ 7.3.8.7).
    multi_frame_headers: BTreeMap<u32, MultiFrameHeaderRecord>,
    /// `lcr_xlayer_map` of each global LCR, keyed by `lcr_global_config_record_id`,
    /// seen in-band so far (§ 7.3.8.3).
    global_lcr_xlayer_maps: BTreeMap<u8, u32>,
    /// `lcr_local_id` values of local LCRs, keyed by their `obu_xlayer_id`, seen
    /// in-band so far (§ 7.3.8.3).
    local_lcr_ids: BTreeMap<ExtendedLayerId, BTreeSet<u8>>,
    /// § 5.8.8 embedded-layer maps of global LCR payloads, keyed by
    /// `(lcr_global_config_record_id, xId)`, for the § 6.8.9 dependency-map
    /// agreement checks. A redefinition overwrites the maps, mirroring
    /// [`Self::record_global_lcr`].
    global_lcr_embedded: BTreeMap<(u8, ExtendedLayerId), LcrEmbeddedMaps>,
    /// § 5.8.8 embedded-layer maps of local LCRs, keyed by
    /// `(obu_xlayer_id, lcr_local_id)`, for the § 6.8.9 dependency-map agreement
    /// checks.
    local_lcr_embedded: BTreeMap<(ExtendedLayerId, u8), LcrEmbeddedMaps>,
    /// § 5.8.4 `lcr_seq_profile_tier_level_info(xlayerId)` declared maxima of local
    /// LCRs, keyed by `(obu_xlayer_id, lcr_local_id)`, for the § 6.8.5 PTL-ceiling
    /// agreement checks. The § 6.8.5 sentences key the ceiling on the *local* LCR
    /// ("associated with the local LCR ... indicated in an extended layer with
    /// obu_xlayer_id equal to i"). Present only when the local record carried
    /// `lcr_profile_tier_level_info_present_flag == 1`; a redefinition replaces the
    /// entry wholesale (see [`Self::clear_local_lcr_extras`]).
    local_lcr_ptl: BTreeMap<(ExtendedLayerId, u8), LcrPtlSnapshot>,
    /// § 5.8.4 `lcr_seq_profile_tier_level_info(i)` declared maxima of global LCRs,
    /// keyed by `(lcr_global_config_record_id, obu_xlayer_id)`, for the § 6.8.5
    /// PTL-ceiling agreement checks when the activated record is a global LCR. Present
    /// only when the global record carried `lcr_seq_profile_tier_level_info_present_flag
    /// == 1` for that xlayer; a redefinition clears and re-records this id's entries.
    global_lcr_ptl: BTreeMap<(u8, ExtendedLayerId), LcrPtlSnapshot>,
    /// § 5.8.7 `lcr_rep_info(0, xId)` of local LCRs, keyed by
    /// `(obu_xlayer_id, lcr_local_id)`, for the § 6.8.8 rep-info equality agreement
    /// checks. Present only when the local record's `lcr_xlayer_info` carried rep info;
    /// a redefinition replaces the entry wholesale.
    local_lcr_rep_info: BTreeMap<(ExtendedLayerId, u8), LcrRepInfoSnapshot>,
    /// § 5.8.7 `lcr_rep_info(1, xId)` of global LCR payloads, keyed by
    /// `(lcr_global_config_record_id, obu_xlayer_id)`, for the § 6.8.8 rep-info
    /// equality agreement checks when the activated record is a global LCR. Present only
    /// for an xlayer whose global payload carried rep info; a redefinition clears and
    /// re-records this id's entries.
    global_lcr_rep_info: BTreeMap<(u8, ExtendedLayerId), LcrRepInfoSnapshot>,
    /// `(obu_xlayer_id, atlas_segment_id)` of local atlas segment OBUs seen in-band so
    /// far (§ 7.3.8.4).
    local_atlases: BTreeSet<(ExtendedLayerId, u8)>,
}

/// The § 5.8.8 embedded-layer maps of one LCR `lcr_xlayer_info` entry, retained for
/// the § 6.8.9 dependency-map agreement checks, plus the defining LCR OBU's byte
/// offset — the § 6.8.9 diagnostic points at the LCR OBU, not at the activating
/// sequence header or frame.
#[derive(Debug, Clone)]
struct LcrEmbeddedMaps {
    /// `lcr_mlayer_map[isGlobal][xId]`.
    mlayer_map: u8,
    /// `(embedded layer index, lcr_tlayer_map[isGlobal][xId][j])` pairs, in
    /// ascending set-bit order of `lcr_mlayer_map`.
    tlayer_maps: Vec<(u8, u8)>,
    /// Byte offset of the defining LCR OBU.
    offset: ByteOffset,
}

/// One LCR's `lcr_seq_profile_tier_level_info(i)` declared maxima (AV2 § 5.8.4 /
/// § 6.8.5), snapshotted for the § 6.8.5 PTL-ceiling agreement plus the defining LCR
/// OBU's byte offset (the diagnostic anchors at the LCR OBU when more informative than
/// the activating header). All four maxima are the LCR-declared ceilings the activated
/// sequence header's PTL must not exceed (`<=`, equality passes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LcrPtlSnapshot {
    /// `lcr_seq_profile_idc[i]`.
    seq_profile_idc: u8,
    /// `lcr_max_level_idx[i]`.
    max_level_idx: u8,
    /// `lcr_tier_flag[i]` as `0`/`1`.
    tier_flag: u8,
    /// `lcr_max_mlayer_count[i]`.
    max_mlayer_count: u8,
    /// Byte offset of the defining LCR OBU.
    offset: ByteOffset,
}

/// One LCR `lcr_rep_info(isGlobal, xId)` entry's representation info (AV2 § 5.8.7 /
/// § 6.8.8), snapshotted for the § 6.8.8 rep-info equality agreement plus the defining
/// LCR OBU's byte offset. `format` / `cropping` mirror the parsed `Option`s: a missing
/// `lcr_format_info_present_flag` / `lcr_cropping_window_present_flag` leaves the
/// corresponding field `None`, and the § 6.8.8 comparisons that gate on those flags
/// compare nothing when absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LcrRepInfoSnapshot {
    /// `lcr_max_pic_width[isGlobal][xId]` (always present).
    max_pic_width: u32,
    /// `lcr_max_pic_height[isGlobal][xId]` (always present).
    max_pic_height: u32,
    /// `(lcr_bit_depth_idc, lcr_chroma_format_idc)`, present when
    /// `lcr_format_info_present_flag == 1`.
    format: Option<(u32, u32)>,
    /// The four `lcr_cropping_win_*_offset` values, present when
    /// `lcr_cropping_window_present_flag == 1` (the present flag itself is the
    /// `Option::is_some`). Order: `(left, right, top, bottom)`.
    cropping: Option<(u32, u32, u32, u32)>,
    /// Byte offset of the defining LCR OBU.
    offset: ByteOffset,
}

/// How a referenced HLS object resolves against available objects (AV2 § 7.3.8).
enum HlsResolution {
    /// Available in the bitstream.
    InBand,
    /// Available only through caller-provided external HLS.
    External,
    /// Not available by any modeled means.
    Unavailable,
}

impl HlsAvailabilityStore {
    /// Records a sequence header (by `seq_header_id`) as available in-band
    /// (AV2 § 7.3.8.6).
    fn record_sequence_header(&mut self, seq_header_id: u32) {
        self.sequence_header_ids.insert(seq_header_id);
    }

    /// Resolves a `seq_header_id` reference against in-band then caller-provided
    /// external availability (AV2 § 7.3.8.6).
    fn resolve_sequence_header(&self, id: u32, options: &ValidationOptions) -> HlsResolution {
        if self.sequence_header_ids.contains(&id) {
            return HlsResolution::InBand;
        }
        if let ExternalHlsMode::Provided(set) = &options.external_hls
            && set.has_sequence_header(id)
        {
            return HlsResolution::External;
        }
        HlsResolution::Unavailable
    }

    /// Records a multi-frame header as available in-band, keyed by `mfhId`
    /// (AV2 § 7.3.8.7). A later redefinition of the same id overwrites the record but
    /// keeps the id available, preserving monotonic availability.
    fn record_multi_frame_header(&mut self, record: MultiFrameHeaderRecord) {
        self.multi_frame_headers.insert(record.mfh_id.get(), record);
    }

    /// Returns the in-band multi-frame-header record for `mfhId`, if available.
    ///
    /// External multi-frame-header availability is not modeled (`ValidationOptions`
    /// declares only external sequence headers); it remains future work.
    fn multi_frame_header(&self, id: MfhId) -> Option<&MultiFrameHeaderRecord> {
        self.multi_frame_headers.get(&id.get())
    }

    /// Records a global LCR (by `lcr_global_config_record_id`) and its `lcr_xlayer_map`
    /// as available in-band (AV2 § 7.3.8.3). A redefinition overwrites the map but
    /// keeps the id available, preserving monotonic availability.
    fn record_global_lcr(&mut self, global_id: u8, xlayer_map: u32) {
        self.global_lcr_xlayer_maps.insert(global_id, xlayer_map);
    }

    /// Returns the `lcr_xlayer_map` of the available global LCR with
    /// `lcr_global_config_record_id == global_id`, if any (AV2 § 7.3.8.3).
    fn global_lcr_xlayer_map(&self, global_id: u8) -> Option<u32> {
        self.global_lcr_xlayer_maps.get(&global_id).copied()
    }

    /// Records a local LCR (by `obu_xlayer_id` and `lcr_local_id`) as available in-band
    /// (AV2 § 7.3.8.3).
    fn record_local_lcr(&mut self, xlayer: ExtendedLayerId, local_id: u8) {
        self.local_lcr_ids
            .entry(xlayer)
            .or_default()
            .insert(local_id);
    }

    /// Returns `true` if a local LCR with `lcr_local_id == local_id` is available in
    /// the extended layer `xlayer` (AV2 § 7.3.8.3).
    fn has_local_lcr(&self, xlayer: ExtendedLayerId, local_id: u8) -> bool {
        self.local_lcr_ids
            .get(&xlayer)
            .is_some_and(|ids| ids.contains(&local_id))
    }

    /// Drops every stored embedded-layer map of the global LCR `global_id`. Called
    /// before re-recording a redefined global LCR so a payload set that drops an
    /// xlayer (or its embedded-layer info) cannot leave stale maps behind — the
    /// § 6.8.9 checks must only ever see the latest definition.
    fn clear_global_lcr_embedded(&mut self, global_id: u8) {
        self.global_lcr_embedded
            .retain(|(id, _), _| *id != global_id);
    }

    /// Drops the stored embedded-layer maps of the local LCR `(xlayer, local_id)`;
    /// see [`Self::clear_global_lcr_embedded`].
    fn clear_local_lcr_embedded(&mut self, xlayer: ExtendedLayerId, local_id: u8) {
        self.local_lcr_embedded.remove(&(xlayer, local_id));
    }

    /// Records a global LCR payload's § 5.8.8 embedded-layer maps for extended layer
    /// `xlayer` (§ 6.8.9 agreement checks).
    fn record_global_lcr_embedded(
        &mut self,
        global_id: u8,
        xlayer: ExtendedLayerId,
        maps: LcrEmbeddedMaps,
    ) {
        self.global_lcr_embedded.insert((global_id, xlayer), maps);
    }

    /// Returns the available global LCR's § 5.8.8 embedded-layer maps for
    /// `(global_id, xlayer)`, if signalled.
    fn global_lcr_embedded(
        &self,
        global_id: u8,
        xlayer: ExtendedLayerId,
    ) -> Option<&LcrEmbeddedMaps> {
        self.global_lcr_embedded.get(&(global_id, xlayer))
    }

    /// Records a local LCR's § 5.8.8 embedded-layer maps (§ 6.8.9 agreement checks).
    fn record_local_lcr_embedded(
        &mut self,
        xlayer: ExtendedLayerId,
        local_id: u8,
        maps: LcrEmbeddedMaps,
    ) {
        self.local_lcr_embedded.insert((xlayer, local_id), maps);
    }

    /// Returns the available local LCR's § 5.8.8 embedded-layer maps for
    /// `(xlayer, local_id)`, if signalled.
    fn local_lcr_embedded(
        &self,
        xlayer: ExtendedLayerId,
        local_id: u8,
    ) -> Option<&LcrEmbeddedMaps> {
        self.local_lcr_embedded.get(&(xlayer, local_id))
    }

    /// Drops the stored § 6.8.5 PTL and § 6.8.8 rep-info snapshots of the local LCR
    /// `(xlayer, local_id)` before re-recording a redefinition, mirroring
    /// [`Self::clear_local_lcr_embedded`] — a re-sent record that drops the PTL or
    /// rep-info must not leave stale entries for the § 6.8.5/§ 6.8.8 checks.
    fn clear_local_lcr_extras(&mut self, xlayer: ExtendedLayerId, local_id: u8) {
        self.local_lcr_ptl.remove(&(xlayer, local_id));
        self.local_lcr_rep_info.remove(&(xlayer, local_id));
    }

    /// Drops every stored § 6.8.5 PTL and § 6.8.8 rep-info snapshot of the global LCR
    /// `global_id` before re-recording a redefinition, mirroring
    /// [`Self::clear_global_lcr_embedded`].
    fn clear_global_lcr_extras(&mut self, global_id: u8) {
        self.global_lcr_ptl.retain(|(id, _), _| *id != global_id);
        self.global_lcr_rep_info
            .retain(|(id, _), _| *id != global_id);
    }

    /// Records a local LCR's § 5.8.4 PTL declared maxima (§ 6.8.5 ceiling checks).
    fn record_local_lcr_ptl(&mut self, xlayer: ExtendedLayerId, local_id: u8, ptl: LcrPtlSnapshot) {
        self.local_lcr_ptl.insert((xlayer, local_id), ptl);
    }

    /// Returns the available local LCR's § 5.8.4 PTL declared maxima for
    /// `(xlayer, local_id)`, if signalled.
    fn local_lcr_ptl(&self, xlayer: ExtendedLayerId, local_id: u8) -> Option<&LcrPtlSnapshot> {
        self.local_lcr_ptl.get(&(xlayer, local_id))
    }

    /// Records a global LCR's § 5.8.4 PTL declared maxima for extended layer `xlayer`
    /// (§ 6.8.5 ceiling checks).
    fn record_global_lcr_ptl(
        &mut self,
        global_id: u8,
        xlayer: ExtendedLayerId,
        ptl: LcrPtlSnapshot,
    ) {
        self.global_lcr_ptl.insert((global_id, xlayer), ptl);
    }

    /// Returns the available global LCR's § 5.8.4 PTL declared maxima for
    /// `(global_id, xlayer)`, if signalled.
    fn global_lcr_ptl(&self, global_id: u8, xlayer: ExtendedLayerId) -> Option<&LcrPtlSnapshot> {
        self.global_lcr_ptl.get(&(global_id, xlayer))
    }

    /// Records a local LCR's § 5.8.7 rep info (§ 6.8.8 equality checks).
    fn record_local_lcr_rep_info(
        &mut self,
        xlayer: ExtendedLayerId,
        local_id: u8,
        rep: LcrRepInfoSnapshot,
    ) {
        self.local_lcr_rep_info.insert((xlayer, local_id), rep);
    }

    /// Returns the available local LCR's § 5.8.7 rep info for `(xlayer, local_id)`, if
    /// signalled.
    fn local_lcr_rep_info(
        &self,
        xlayer: ExtendedLayerId,
        local_id: u8,
    ) -> Option<&LcrRepInfoSnapshot> {
        self.local_lcr_rep_info.get(&(xlayer, local_id))
    }

    /// Records a global LCR payload's § 5.8.7 rep info for extended layer `xlayer`
    /// (§ 6.8.8 equality checks).
    fn record_global_lcr_rep_info(
        &mut self,
        global_id: u8,
        xlayer: ExtendedLayerId,
        rep: LcrRepInfoSnapshot,
    ) {
        self.global_lcr_rep_info.insert((global_id, xlayer), rep);
    }

    /// Returns the available global LCR's § 5.8.7 rep info for `(global_id, xlayer)`, if
    /// signalled.
    fn global_lcr_rep_info(
        &self,
        global_id: u8,
        xlayer: ExtendedLayerId,
    ) -> Option<&LcrRepInfoSnapshot> {
        self.global_lcr_rep_info.get(&(global_id, xlayer))
    }

    /// Records a local atlas segment OBU (by `obu_xlayer_id` and `atlas_segment_id`)
    /// as available in-band (AV2 § 7.3.8.4).
    fn record_local_atlas(&mut self, xlayer: ExtendedLayerId, atlas_id: u8) {
        self.local_atlases.insert((xlayer, atlas_id));
    }

    /// Returns `true` if a local atlas segment OBU with `atlas_segment_id == atlas_id`
    /// is available in the extended layer `xlayer` (AV2 § 7.3.8.4).
    fn has_local_atlas(&self, xlayer: ExtendedLayerId, atlas_id: u8) -> bool {
        self.local_atlases.contains(&(xlayer, atlas_id))
    }
}

/// One active in-band operating point set, keyed by `(obu_xlayer_id, ops_id)`
/// (AV2 § 6.10, § 7.3.8.5).
///
/// The parser produces exactly `ops_cnt` operating point payloads (the § 5.10 loop
/// runs `ops_cnt` times or the parse fails), so a separate payload count is redundant
/// and is not stored.
#[derive(Debug, Clone)]
struct OperatingPointSetRecord {
    /// `obu_xlayer_id` of the OBU that defined this OPS (`GLOBAL_XLAYER_ID` for a
    /// global OPS).
    xlayer_id: ExtendedLayerId,
    /// `ops_id`.
    ops_id: u8,
    /// `ops_cnt`, compared against a referencing BRT's `br_ops_cnt` (§ 6.11).
    ops_cnt: u8,
    /// Source byte offset of the defining OBU, surfaced in referencing diagnostics.
    offset: ByteOffset,
    /// Explicitly signalled `ops_mlayer_info()` entries, retained for the § 6.10.7
    /// dependency-map agreement checks. Inherited and absent entries are not
    /// retained — § 6.10.7 binds the maps "if present", and an inherited entry's
    /// maps are checked when the referenced OPS is itself observed.
    explicit_entries: Vec<OpsExplicitEntry>,
}

/// One explicitly signalled `ops_mlayer_info()` entry of an active OPS (§ 5.11.5),
/// retained for the § 6.10.7 dependency-map agreement checks.
#[derive(Debug, Clone)]
struct OpsExplicitEntry {
    /// Operating-point payload index (`opIndex`).
    payload_index: u8,
    /// The included extended layer (`xLId`) whose configuration the maps describe.
    xlayer_id: ExtendedLayerId,
    /// `ops_mlayer_map` plus the per-set-bit `ops_tlayer_map`s.
    info: OpsMlayerInfo,
}

/// Collects the explicitly signalled `ops_mlayer_info()` entries of a parsed OPS
/// (§ 5.11.5) for the § 6.10.7 agreement checks.
fn ops_explicit_entries(ops: &OperatingPointSet) -> Vec<OpsExplicitEntry> {
    let mut entries = Vec::new();
    for payload in &ops.payloads {
        for entry in &payload.xlayer_entries {
            if let OpsMlayerSource::Explicit(info) = &entry.mlayer {
                entries.push(OpsExplicitEntry {
                    payload_index: payload.index,
                    xlayer_id: entry.xlayer_id,
                    info: info.clone(),
                });
            }
        }
    }
    entries
}

/// Which dependency map a § 6.10.7 / § 6.8.9 agreement finding concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DependencyMapKind {
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
enum DependencyFindingKey {
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
    fn seq_header_id(self) -> SequenceHeaderId {
        match self {
            Self::Ops { seq_header_id, .. } | Self::Lcr { seq_header_id, .. } => seq_header_id,
        }
    }
}

/// Which § 6.8.5 PTL ceiling a finding constrains; part of [`LcrPtlFindingKey`] so the
/// four sub-rules are deduped independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LcrPtlField {
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
struct LcrPtlFindingKey {
    xlayer: ExtendedLayerId,
    seq_header_id: SequenceHeaderId,
    lcr_is_global: bool,
    lcr_id: u8,
    lcr_offset: ByteOffset,
    field: LcrPtlField,
    /// The LCR-declared maximum in force (a redefinition with a new offset already
    /// yields a distinct key; this is kept for content symmetry with the header value).
    lcr_max: u32,
    /// The activated header's compared value.
    header_value: u32,
}

/// Which § 6.8.8 rep-info field a finding constrains; part of [`LcrRepInfoFindingKey`]
/// so the sub-fields are deduped independently and named in the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LcrRepInfoField {
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
struct LcrRepInfoFindingKey {
    xlayer: ExtendedLayerId,
    seq_header_id: SequenceHeaderId,
    lcr_is_global: bool,
    lcr_id: u8,
    lcr_offset: ByteOffset,
    field: LcrRepInfoField,
    lcr_value: u64,
    header_value: u64,
}

/// Scans an 8-bit embedded-layer bitmask for the first § 6.10.7 / § 6.8.9 closure
/// violation under `MLayerDependencyMap`: a set bit `cMId` for which the map
/// requires a dependency `rMId < cMId` (`MLayerDependencyMap[cMId][rMId] == 1`)
/// whose bit is not set. Returns the violating `(cMId, rMId)` pair.
fn mlayer_closure_violation(mask: u8, m_map: &MLayerDependencyMap) -> Option<(u8, u8)> {
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
fn tlayer_closure_violation(mlayer: u8, mask: u8, t_map: &TLayerDependencyMap) -> Option<(u8, u8)> {
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
fn rep_info_snapshot(rep_info: &LcrRepInfo, offset: ByteOffset) -> LcrRepInfoSnapshot {
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

/// `true` when caller-provided external HLS declares at least one sequence header.
/// An externally activated sequence header has unmodeled dependency maps, so every
/// "activated sequence header" agreement check is unreliable and suppressed
/// (precedent: [`ValidatorContext::validate_active_sequence_limits`]).
fn external_declares_sequence_header(options: &ValidationOptions) -> bool {
    if let ExternalHlsMode::Provided(set) = &options.external_hls {
        set.declares_any_sequence_header()
    } else {
        false
    }
}

/// Builds an Annex A Table A.4 interoperability-point presence diagnostic (error, spec
/// section `A.2`, anchored at `offset`).
fn annex_a_iop_error(rule_id: &'static str, offset: ByteOffset, message: String) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("A.2")
        .with_byte_offset(offset)
}

/// Active in-band operating point sets (AV2 § 6.10.1, § 7.3.8.5).
///
/// Unlike [`HlsAvailabilityStore`], this store is **not** monotonic: § 6.10.1 defines
/// explicit reset/update behavior, so records are removed on reset rather than kept
/// forever. State is modeled per extended layer; a global (`GLOBAL_XLAYER_ID`) reset
/// clears every modeled layer.
#[derive(Debug, Default)]
struct OpsAvailabilityStore {
    by_xlayer: BTreeMap<ExtendedLayerId, BTreeMap<u8, OperatingPointSetRecord>>,
    /// Monotonic count of § 6.10.1 *global* OPS resets (`ops_reset_flag == 1` on a
    /// `GLOBAL_XLAYER_ID` OBU): per § 6.10.1 case 1/2 a global reset resets "all layers
    /// if global", so this generation contributes to the effective reset generation of
    /// *every* extended layer (see [`Self::effective_reset_generation`]).
    global_reset_generation: u64,
    /// Per-extended-layer count of § 6.10.1 *local* OPS resets (`ops_reset_flag == 1` on
    /// an OBU with `obu_xlayer_id < GLOBAL_XLAYER_ID`): per § 6.10.1 case 1/2 a local
    /// reset resets only "all OPS for the associated extended layer", so it bumps only
    /// its own layer's generation. The § 6.10.5 buffer-delay sum-constancy error tier
    /// scopes its per-triple baseline by the *effective* reset generation
    /// (`global_reset_generation + local_reset_generation[xlayer]`): a redefinition is
    /// compared against the baseline only when no reset *of that layer* (local or global)
    /// intervened (the constraint says "with no intervening OPS reset"). A reset of an
    /// unrelated extended layer no longer re-baselines this layer (the round-2 fix —
    /// previously a single bitstream-wide counter over-reset every layer and suppressed
    /// a required error). Scoping by the effective generation only ever suppresses
    /// comparisons, never invents one.
    local_reset_generation: BTreeMap<ExtendedLayerId, u64>,
    /// Per-`(obu_xlayer_id, opsID)` count of § 6.10.1 *targeted* resets
    /// (`ops_reset_flag == 0` and `ops_cnt == 0`: case 3, "Only OPS x is reset"). A
    /// targeted reset re-baselines exactly that OPS without disturbing any other, so it
    /// must not bump the per-layer effective reset generation (see
    /// [`Self::effective_reset_generation`]) — that would over-suppress unrelated triples
    /// of the same layer. The § 6.10.5 buffer-delay error tier includes
    /// this per-key generation in its scope identity, so a redefinition of the same
    /// triple after a targeted reset of its OPS is treated like any other reset-spanning
    /// change: out of the error tier, into the cross-CVS advisory.
    targeted_reset_generation: BTreeMap<(ExtendedLayerId, u8), u64>,
}

impl OpsAvailabilityStore {
    /// Applies one OPS OBU's reset/update semantics (AV2 § 6.10.1):
    ///
    /// | `reset_flag` | `ops_cnt` | behavior |
    /// |---|---|---|
    /// | 1 | 0 | reset all OPS for the layer (all layers if global) |
    /// | 1 | >0 | reset, then define this `(xlayer, ops_id)` |
    /// | 0 | 0 | reset only this `(xlayer, ops_id)` |
    /// | 0 | >0 | define/update only this `(xlayer, ops_id)` |
    fn apply(&mut self, record: OperatingPointSetRecord, reset_flag: bool) {
        let xlayer = record.xlayer_id;
        let ops_id = record.ops_id;
        let defines = record.ops_cnt > 0;

        if reset_flag {
            // § 6.10.1 case 1/2: a global reset (GLOBAL_XLAYER_ID) resets "all layers",
            // so it bumps the global generation that every layer's effective generation
            // incorporates; a local reset resets only its own layer's OPS, so it bumps
            // only that layer's generation. Per-layer scoping keeps a reset of one
            // extended layer from re-baselining the § 6.10.5 comparison of another.
            if xlayer.is_global() {
                self.global_reset_generation += 1;
                self.by_xlayer.clear();
            } else {
                *self.local_reset_generation.entry(xlayer).or_default() += 1;
                self.by_xlayer.remove(&xlayer);
            }
            if defines {
                self.by_xlayer
                    .entry(xlayer)
                    .or_default()
                    .insert(ops_id, record);
            }
        } else if defines {
            self.by_xlayer
                .entry(xlayer)
                .or_default()
                .insert(ops_id, record);
        } else {
            // Case 3 (§ 6.10.1): a targeted reset of only this (xlayer, ops_id). Bump the
            // per-key targeted-reset generation so the § 6.10.5 error tier re-baselines
            // this OPS (and only this OPS) like a reset boundary, without touching the
            // per-layer effective reset generation.
            *self
                .targeted_reset_generation
                .entry((xlayer, ops_id))
                .or_default() += 1;
            // Remove only this (xlayer, ops_id), then prune the layer's map if it is
            // now empty so the store does not accumulate empty inner maps.
            let now_empty = match self.by_xlayer.get_mut(&xlayer) {
                Some(map) => {
                    map.remove(&ops_id);
                    map.is_empty()
                }
                None => false,
            };
            if now_empty {
                self.by_xlayer.remove(&xlayer);
            }
        }
    }

    /// Returns the active OPS record for `(xlayer, ops_id)`, if any.
    fn get(&self, xlayer: ExtendedLayerId, ops_id: u8) -> Option<&OperatingPointSetRecord> {
        self.by_xlayer.get(&xlayer).and_then(|map| map.get(&ops_id))
    }

    /// The effective § 6.10.1 reset generation for `xlayer`: the global reset count
    /// (a global reset resets all layers) plus this layer's own local reset count (a
    /// local reset resets only its layer). The § 6.10.5 buffer-delay error tier scopes
    /// a triple's baseline by this value, so only a reset *of this layer* — local or
    /// global — re-baselines its comparison (see [`Self::local_reset_generation`]).
    fn effective_reset_generation(&self, xlayer: ExtendedLayerId) -> u64 {
        self.global_reset_generation
            + self
                .local_reset_generation
                .get(&xlayer)
                .copied()
                .unwrap_or(0)
    }

    /// The current § 6.10.1 *targeted*-reset generation for `(xlayer, ops_id)` (see
    /// [`Self::targeted_reset_generation`]), or 0 before any targeted reset of that OPS.
    fn targeted_reset_generation(&self, xlayer: ExtendedLayerId, ops_id: u8) -> u64 {
        self.targeted_reset_generation
            .get(&(xlayer, ops_id))
            .copied()
            .unwrap_or(0)
    }

    /// Iterates the active OPS records in the `xlayer` bucket (§ 6.10.7
    /// activation-time re-checks).
    fn records_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> impl Iterator<Item = &OperatingPointSetRecord> {
        self.by_xlayer
            .get(&xlayer)
            .into_iter()
            .flat_map(BTreeMap::values)
    }
}

/// Per-level quantizer-matrix availability, recorded when a QM OBU specifies a level
/// (AV2 § 6.12 / § 7.3.8 foundation). Kept for future frame-reference checks; this
/// phase reads it only to cite the conflicting definition in a duplicate-level
/// diagnostic.
#[derive(Debug, Clone, Copy)]
struct QmLevelRecord {
    /// `QmMLayerId[level]` (`None` models the spec's `-1` for a reset).
    mlayer_id: Option<u8>,
    /// `QmTLayerId[level]` (`None` models the spec's `-1` for a reset).
    tlayer_id: Option<u8>,
    /// `QmDataPresent[level]`: `true` for user-defined data, `false` for a default.
    data_present: bool,
    /// `QmNumPlanes[level]`.
    num_planes: u8,
}

/// Quantizer-matrix validator state (AV2 § 6.12).
///
/// The window fields (`seen_levels_since_coded_frame`, `qm_obu_seen_since_coded_frame`)
/// reset at each coded-frame boundary (see [`ValidatorContext::reset_coded_frame_window`])
/// and drive the § 6.12 duplicate-reset / duplicate-level checks. The `available`
/// array is monotonic per-level HLS state, foundation for the deferred frame
/// quantization-reference checks (`using_qmatrix` / `qm_*`, § 7.3.8 / § 6.17.6).
#[derive(Debug, Default)]
struct QuantizerMatrixState {
    /// Levels (`qm_bit_map` bits) specified by a QM OBU since the last coded frame.
    seen_levels_since_coded_frame: u16,
    /// `true` once any QM OBU has been observed since the last coded frame. A
    /// `qm_bit_map == 0` reset is only conformant as the first QM OBU in the window.
    qm_obu_seen_since_coded_frame: bool,
    /// Monotonic per-level availability for future frame-reference validation.
    available: [Option<QmLevelRecord>; NUM_CUSTOM_QMS],
}

impl QuantizerMatrixState {
    /// Clears the §6.12 "between coded frames" window at a coded-frame boundary.
    fn reset_coded_frame_window(&mut self) {
        self.seen_levels_since_coded_frame = 0;
        self.qm_obu_seen_since_coded_frame = false;
    }
}

/// Per-slot film-grain availability, recorded when a film-grain OBU updates a slot
/// (AV2 § 6.13 / § 7.3.8 foundation). Kept for future frame-reference checks; this
/// phase reads it only to cite the conflicting update in a duplicate-slot diagnostic.
#[derive(Debug, Clone, Copy)]
struct FgmSlotRecord {
    /// `FgmChromaIdc[slot]`.
    chroma_idc: u32,
    /// `FgmMLayerId[slot]`.
    mlayer_id: u8,
    /// `FgmTLayerId[slot]`.
    tlayer_id: u8,
}

/// Film-grain validator state (AV2 § 6.13).
///
/// `updated_slots_since_coded_frame` resets at each coded-frame-unit boundary (see
/// [`ValidatorContext::reset_coded_frame_window`]) and drives the § 6.13 duplicate-slot
/// check. The `available` array is monotonic per-slot HLS state, foundation for the
/// deferred frame film-grain-reference checks (`apply_grain` / `fgm_id`, § 5.18.10.1 /
/// § 7.3.8).
#[derive(Debug, Default)]
struct FilmGrainState {
    /// Slots (`fgm_update_flags` bits) updated by a film-grain OBU since the last
    /// coded frame unit.
    updated_slots_since_coded_frame: u8,
    /// Monotonic per-slot availability for future frame-reference validation.
    available: [Option<FgmSlotRecord>; MAX_FILM_GRAIN],
}

impl FilmGrainState {
    /// Clears the §6.13 coded-frame-unit window at a coded-frame boundary.
    fn reset_coded_frame_window(&mut self) {
        self.updated_slots_since_coded_frame = 0;
    }
}

/// Key identifying a content-interpretation record: `(obu_xlayer_id, obu_mlayer_id)`.
type ContentInterpretationKey = (ExtendedLayerId, EmbeddedLayerId);

/// One observed content-interpretation OBU within its coded-video-sequence scope.
#[derive(Debug)]
struct ContentInterpretationRecord {
    /// Parsed § 5.15 syntax, used for cross-embedded-layer timing consistency
    /// (AV2 § 6.4.12) and the repeated-CI "same information" check (AV2 § 6.14).
    content: ContentInterpretation,
    /// Source byte offset of the OBU that produced this record.
    offset: ByteOffset,
    /// Temporal unit ([`CvsTracker::tu_index`]) of this record's latest appearance,
    /// used by the exact § 7.3.6 CVS scoping (CLK pruning and deferral decisions).
    tu_index: u64,
    /// Whether this record's latest appearance had its § 6.16.10 Table 6.18
    /// scan-type CI-time recheck SUPPRESSED by the epoch-aware identical-CI dedup
    /// guard (finding 1). A re-send whose scan-type-decisive content equalled the
    /// pre-RAP record's is suppressed at CI-time (the lagging epoch cannot tell it
    /// apart from an ordinary identical repeat); only such suppressed re-sends are
    /// re-paired by [`ValidatorContext::repair_post_rap_ci_pairings`] at the CLK/OLK.
    /// A re-send that CHANGED the decisive content already rechecked eagerly at
    /// CI-time, so re-pairing it would duplicate the diagnostic.
    scan_type_recheck_suppressed: bool,
    /// The § 6.16.7 n_frames analogue of [`Self::scan_type_recheck_suppressed`]:
    /// whether this record's latest appearance had its timecode n_frames CI-time
    /// recheck suppressed by the epoch-aware identical-CI dedup guard (finding 1).
    timecode_recheck_suppressed: bool,
}

/// Per extended layer, the § 7.3.6 first-CELU CI PRESENCE state (mirror lines 560-562,
/// `07-decoding-process.md#s-7-3-6`): "If an OBU_CONTENT_INTERPRETATION is present in any
/// coded extended layer unit, this OBU shall also be present in the first coded extended
/// layer unit of the sequence ... for a given embedded layer."
///
/// The "first coded extended layer unit of the sequence" for an extended layer is its CELU
/// in the coded video sequence's FIRST temporal unit (a CVS starts at the temporal unit
/// containing a CLK, § 7.3.6). This state records — scoped to the layer's CVS — the embedded
/// layers whose first CELU carried a CI, so a later CELU that adds a CI for an embedded layer
/// the first CELU lacked can be flagged. Reset per coded video sequence in
/// [`ValidatorContext::start_cvs_for_xlayer`].
///
/// **Unknown-first-CELU drop.** If the first CELU of the CVS was not observed — the stream
/// starts mid-CVS (no CLK seen for the layer, so the implicit CVS began before the first
/// observed OBU) — `first_celu_tu` is `None` and the presence judgment drops: the first
/// CELU's CI set is unknowable. An external-HLS `Provided` mode likewise drops the judgment
/// at the call site (an external CI in the first CELU cannot be enumerated by
/// [`crate::options::ExternalHlsSet`], which expresses only sequence headers and operating
/// point sets), consistent with the partial-declaration suppression policy.
#[derive(Debug, Default)]
struct CiFirstCeluState {
    /// The temporal-unit index of the CVS's first temporal unit — the temporal unit whose
    /// CELU is the "first coded extended layer unit of the sequence" for this layer. `None`
    /// until a CLK establishes the CVS start for the layer (so a mid-CVS join, where no CLK
    /// has been observed, leaves it `None` and drops the judgment).
    first_celu_tu: Option<u64>,
    /// The embedded layers (`obu_mlayer_id`) whose CI was observed in the first CELU of the
    /// CVS. A CI in a later CELU for an embedded layer absent from this set fires
    /// `celu/content-interpretation-not-in-first-celu`.
    first_celu_ci_mlayers: BTreeSet<EmbeddedLayerId>,
    /// The embedded layers already reported, so the diagnostic dedups per
    /// `(xlayer, mlayer, CVS epoch)` — a repeated later CI for the same missing embedded
    /// layer fires once per coded video sequence.
    reported: BTreeSet<EmbeddedLayerId>,
}

/// The bitstream-derived embedded-layer association of one HDR CLL / MDCV
/// metadata unit (AV2 § 6.16.5 / § 6.16.6 bind "metadata units **associated with
/// an embedded layer** in a coded video sequence"; the association is derivable
/// per § 6.16.3: "muh_layer_idc is used to signal a mode that specifies the
/// layers to which the signaled metadata applies").
///
/// Two units fall under the § 6.16.5 / § 6.16.6 same-content rule exactly when
/// their association sets intersect — share at least one embedded layer —
/// regardless of how the targeting was encoded (a global `LAYER_GLOBAL` unit
/// "applies to all layers" and a `LAYER_CURRENT` unit for a concrete
/// `(obu_xlayer_id, obu_mlayer_id)` are both associated with that embedded
/// layer). Units whose association is not derivable from the bitstream enter no
/// comparison and no baseline (see [`derive_hdr_association`]):
///
/// - `LAYER_UNSPECIFIED` (0) — § 6.16.3: "The current signaling does not specify
///   to what layers the metadata applies to. This information can potentially be
///   indicated or determined through external means." Comparing two such units
///   could manufacture a false positive (their real associations may differ);
///   skipping them is a documented false negative in the conservative direction.
/// - `LAYER_CURRENT` on a `GLOBAL_XLAYER_ID` OBU — § 6.2.2 forces
///   `obu_mlayer_id` to 0 there, but `GLOBAL_XLAYER_ID` is not an extended
///   layer, so the "current layer" names no concrete embedded layer
///   (conservative skip).
/// - Reserved `muh_layer_idc` 4..=7 — no defined association.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HdrAssociation {
    /// `LAYER_GLOBAL` on a `GLOBAL_XLAYER_ID` OBU: "The metadata applies to all
    /// layers" (§ 6.16.3) — every embedded layer of every extended layer.
    Universal,
    /// `LAYER_GLOBAL` on a concrete OBU: "layers with matching obu_xlayer_id
    /// only" (§ 6.16.3) — every embedded layer of that extended layer.
    XLayerWide(ExtendedLayerId),
    /// An explicit `(obu_xlayer_id, obu_mlayer_id)` pair set: `LAYER_CURRENT`
    /// (the carrying OBU's own pair) or `LAYER_VALUES` ("The metadata unit is
    /// intended for an extended layer x if bit x of muh_xlayer_map is equal to
    /// 1" and "... for an embedded layer m if bit m of muh_mlayer_map is equal
    /// to 1", § 6.16.3; map layout per § 5.17.3). Never empty.
    Pairs(Vec<(ExtendedLayerId, EmbeddedLayerId)>),
}

impl HdrAssociation {
    /// Returns `true` when the two association sets share at least one embedded
    /// layer — the condition under which § 6.16.5 / § 6.16.6 require the two
    /// units to "have the same content".
    fn intersects(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Universal, _) | (_, Self::Universal) => true,
            (Self::XLayerWide(a), Self::XLayerWide(b)) => a == b,
            (Self::XLayerWide(x), Self::Pairs(pairs))
            | (Self::Pairs(pairs), Self::XLayerWide(x)) => {
                pairs.iter().any(|(pair_x, _)| pair_x == x)
            }
            (Self::Pairs(a), Self::Pairs(b)) => a.iter().any(|pair| b.contains(pair)),
        }
    }

    /// Returns `true` when the association includes any embedded layer of
    /// `xlayer`, i.e. the record belongs to that extended layer's
    /// coded-video-sequence scope (drives the § 7.3.6 CLK pruning; a `Universal`
    /// record touches every layer, mirroring the global-record pruning of the
    /// other CVS-scoped stores).
    fn touches_xlayer(&self, xlayer: ExtendedLayerId) -> bool {
        match self {
            Self::Universal => true,
            Self::XLayerWide(x) => *x == xlayer,
            Self::Pairs(pairs) => pairs.iter().any(|(pair_x, _)| *pair_x == xlayer),
        }
    }

    /// The concrete extended layers this association enumerates (`Universal`
    /// applies to all layers and enumerates none).
    fn concrete_xlayers(&self) -> Vec<ExtendedLayerId> {
        match self {
            Self::Universal => Vec::new(),
            Self::XLayerWide(x) => vec![*x],
            Self::Pairs(pairs) => pairs.iter().map(|(pair_x, _)| *pair_x).collect(),
        }
    }

    /// The concrete `(obu_xlayer_id, obu_mlayer_id)` embedded-layer pairs this
    /// association names exactly, or `None` when the association covers every
    /// embedded layer of an extended layer (`XLayerWide`) or all layers
    /// (`Universal`) and so names no single concrete first coded picture. Used by
    /// the § 6.16.5 / § 6.16.6 first-coded-picture check, which fires only when
    /// every named embedded layer's first coded picture has already passed —
    /// requiring an exact set of named pairs to stay zero-false-positive.
    fn explicit_embedded_pairs(&self) -> Option<&[(ExtendedLayerId, EmbeddedLayerId)]> {
        match self {
            Self::Universal | Self::XLayerWide(_) => None,
            Self::Pairs(pairs) => Some(pairs),
        }
    }

    /// Returns `true` when a content interpretation OBU for embedded layer
    /// `(ci_xlayer, ci_mlayer)` is associated with the layers this metadata unit
    /// describes — the § 6.16.7 / Annex E.4.2 "content interpretation OBU
    /// associated with this extended layer" relation, refined by the unit's
    /// § 6.16.3 targeting (finding 4). A `Universal` unit describes every layer;
    /// an `XLayerWide` unit every embedded layer of its extended layer; an
    /// explicit `Pairs` unit only the `(obu_xlayer_id, obu_mlayer_id)` pairs it
    /// names, so a CI at an untargeted embedded layer cannot pair with it.
    fn associated_with_ci(&self, ci_xlayer: ExtendedLayerId, ci_mlayer: EmbeddedLayerId) -> bool {
        match self {
            Self::Universal => true,
            Self::XLayerWide(x) => *x == ci_xlayer,
            Self::Pairs(pairs) => pairs.contains(&(ci_xlayer, ci_mlayer)),
        }
    }

    /// Returns `true` when this association includes the concrete embedded-layer
    /// pair `(xlayer, mlayer)` — `Universal` includes every layer, `XLayerWide`
    /// every embedded layer of its extended layer, and `Pairs` only the pairs it
    /// names. Used by the § 6.16.5 / § 6.16.6 first-coded-picture check to decide
    /// **per pair** whether a prior baseline already established that layer's
    /// content (finding 4), so a unit targeting an established layer alongside a new
    /// one is still checked for the new layer.
    fn includes_embedded_pair(&self, xlayer: ExtendedLayerId, mlayer: EmbeddedLayerId) -> bool {
        match self {
            Self::Universal => true,
            Self::XLayerWide(x) => *x == xlayer,
            Self::Pairs(pairs) => pairs.contains(&(xlayer, mlayer)),
        }
    }
}

/// The single concrete extended layer scoping a comparison between two
/// intersecting [`HdrAssociation`]s, for [`CvsTracker::defer_or_emit`] tagging.
/// When the intersection spans several extended layers — or only the all-layers
/// `Universal` pair, which enumerates none — `GLOBAL_XLAYER_ID` tags it instead,
/// reusing the documented any-CLK-drops approximation of
/// [`CvsTracker::flush_completed_tu`] (sound: it only drops comparisons).
fn hdr_intersection_scope(a: &HdrAssociation, b: &HdrAssociation) -> ExtendedLayerId {
    let xlayers: Vec<ExtendedLayerId> = match (a, b) {
        (HdrAssociation::Universal, HdrAssociation::Universal) => Vec::new(),
        (HdrAssociation::Universal, other) | (other, HdrAssociation::Universal) => {
            other.concrete_xlayers()
        }
        (HdrAssociation::XLayerWide(x), _) | (_, HdrAssociation::XLayerWide(x)) => vec![*x],
        (HdrAssociation::Pairs(a_pairs), HdrAssociation::Pairs(b_pairs)) => a_pairs
            .iter()
            .filter(|pair| b_pairs.contains(pair))
            .map(|(pair_x, _)| *pair_x)
            .collect(),
    };
    match xlayers.as_slice() {
        [first, rest @ ..] if rest.iter().all(|x| x == first) => *first,
        _ => GLOBAL_XLAYER_ID,
    }
}

/// Describes an embedded-layer association the two intersecting
/// [`HdrAssociation`]s share, naming a concrete `(obu_xlayer_id, obu_mlayer_id)`
/// pair whenever one is enumerable so a cross-mode § 6.16.5 / § 6.16.6 finding
/// is intelligible.
fn describe_hdr_intersection(a: &HdrAssociation, b: &HdrAssociation) -> String {
    let common_pair = match (a, b) {
        (HdrAssociation::Pairs(a_pairs), HdrAssociation::Pairs(b_pairs)) => {
            a_pairs.iter().find(|pair| b_pairs.contains(pair)).copied()
        }
        (HdrAssociation::Pairs(pairs), HdrAssociation::XLayerWide(x))
        | (HdrAssociation::XLayerWide(x), HdrAssociation::Pairs(pairs)) => {
            pairs.iter().find(|(pair_x, _)| pair_x == x).copied()
        }
        (HdrAssociation::Pairs(pairs), HdrAssociation::Universal)
        | (HdrAssociation::Universal, HdrAssociation::Pairs(pairs)) => pairs.first().copied(),
        _ => None,
    };
    if let Some((xlayer, mlayer)) = common_pair {
        return format!(
            "embedded layer obu_xlayer_id {} / obu_mlayer_id {}",
            xlayer.get(),
            mlayer.get()
        );
    }
    match (a, b) {
        (HdrAssociation::XLayerWide(x), _) | (_, HdrAssociation::XLayerWide(x)) => {
            format!("every embedded layer of obu_xlayer_id {}", x.get())
        }
        _ => "all layers".to_owned(),
    }
}

/// Names a set of concrete `(obu_xlayer_id, obu_mlayer_id)` embedded-layer pairs for
/// a § 6.16.5 / § 6.16.6 first-coded-picture finding, so a unit late for a *subset*
/// of its targeted layers reports exactly which layers.
fn describe_embedded_pairs(pairs: &[(ExtendedLayerId, EmbeddedLayerId)]) -> String {
    let names: Vec<String> = pairs
        .iter()
        .map(|(xlayer, mlayer)| {
            format!(
                "obu_xlayer_id {} / obu_mlayer_id {}",
                xlayer.get(),
                mlayer.get()
            )
        })
        .collect();
    format!("embedded layer(s) {}", names.join(", "))
}

/// One observed HDR CLL / MDCV unit's content within its coded-video-sequence
/// scope (AV2 § 6.16.5 / § 6.16.6), compared against every later unit of the
/// same metadata type whose [`HdrAssociation`] intersects this one.
#[derive(Debug)]
struct HdrBaselineRecord {
    /// The unit's bitstream-derived embedded-layer association.
    association: HdrAssociation,
    /// `true` for `metadata_hdr_mdcv()`, `false` for `metadata_hdr_cll()` —
    /// § 6.16.5 and § 6.16.6 state the rule identically but each binds only its
    /// own metadata type.
    is_mdcv: bool,
    /// The parsed `metadata_hdr_cll()` / `metadata_hdr_mdcv()` payload, compared
    /// field-for-field against every later intersecting unit.
    payload: MetadataPayload,
    /// Source byte offset of the OBU that produced this record.
    offset: ByteOffset,
    /// Temporal unit ([`CvsTracker::tu_index`]) of this record's latest appearance,
    /// used by the exact § 7.3.6 CVS scoping (CLK pruning and deferral decisions).
    tu_index: u64,
}

/// The parsed `muh_*` unit-header fields the stateful metadata observers consume
/// for one non-cancel metadata unit (AV2 § 5.17.2 / § 5.17.3 / § 6.16.3).
struct MetadataUnitHeader<'a> {
    /// `muh_layer_idc` (short form: parsed from the 1-byte header; group form:
    /// per-unit).
    layer_idc: u8,
    /// `muh_persistence_idc`.
    persistence_idc: u8,
    /// The single-byte `muh_mlayer_map` collapse consumed by the § 6.16.3
    /// lifetime store ([`ActiveMetadataUnit::mlayer_map`]): the local group
    /// form's one byte, or a global group unit's byte when its `muh_xlayer_map`
    /// selected exactly one extended layer; `None` otherwise (including the
    /// short form, which carries no maps).
    collapsed_mlayer_map: Option<u8>,
    /// `muh_xlayer_map` (global group-form `LAYER_VALUES` only, § 5.17.3).
    xlayer_map: Option<u32>,
    /// Every parsed `muh_mlayer_map` byte: one per set `muh_xlayer_map` bit when
    /// global, a single byte when local, empty for the short form (§ 5.17.3).
    mlayer_maps: &'a [u8],
}

/// Derives the [`HdrAssociation`] of one non-cancel metadata unit from its
/// carrying OBU's layer ids and its `muh_*` layer targeting (AV2 § 6.16.3
/// per-`muh_layer_idc` modes; § 5.17.3 map layout). Returns `None` when the
/// association is not derivable from the bitstream (see [`HdrAssociation`]) or
/// when explicit `LAYER_VALUES` maps select no layer.
fn derive_hdr_association(
    obu: &ObuEnvelope<'_>,
    header: &MetadataUnitHeader<'_>,
) -> Option<HdrAssociation> {
    let xlayer = obu.header.extended_layer_id;
    match header.layer_idc {
        LAYER_GLOBAL if xlayer.is_global() => Some(HdrAssociation::Universal),
        LAYER_GLOBAL => Some(HdrAssociation::XLayerWide(xlayer)),
        LAYER_CURRENT if !xlayer.is_global() => Some(HdrAssociation::Pairs(vec![(
            xlayer,
            obu.header.embedded_layer_id,
        )])),
        LAYER_VALUES => {
            let mut pairs = Vec::new();
            if xlayer.is_global() {
                // Global form: one muh_mlayer_map per set muh_xlayer_map bit, in
                // ascending bit order (§ 5.17.3); bit 31 must be 0 (§ 6.16.3).
                let xlayer_map = header.xlayer_map?;
                let mut maps = header.mlayer_maps.iter();
                for x in 0..31u8 {
                    if xlayer_map & (1 << x) == 0 {
                        continue;
                    }
                    // The § 5.17.3 parser emits exactly one map per set bit;
                    // bail on a mismatch rather than misattribute maps to layers.
                    let &mlayer_map = maps.next()?;
                    push_mlayer_pairs(&mut pairs, ExtendedLayerId::from_bits(x), mlayer_map);
                }
            } else if let [single] = header.mlayer_maps {
                push_mlayer_pairs(&mut pairs, xlayer, *single);
            }
            (!pairs.is_empty()).then_some(HdrAssociation::Pairs(pairs))
        }
        // LAYER_UNSPECIFIED (0), LAYER_CURRENT on a global OBU, and the reserved
        // values 4..=7: no bitstream-derivable association.
        _ => None,
    }
}

/// Appends `(xlayer, m)` for each set bit `m` of one 8-bit `muh_mlayer_map`
/// (AV2 § 6.16.3: "The metadata unit is intended for an embedded layer m if bit
/// m of muh_mlayer_map is equal to 1").
fn push_mlayer_pairs(
    pairs: &mut Vec<(ExtendedLayerId, EmbeddedLayerId)>,
    xlayer: ExtendedLayerId,
    mlayer_map: u8,
) {
    for m in 0..8u8 {
        if mlayer_map >> m & 1 == 1 {
            pairs.push((xlayer, EmbeddedLayerId::from_bits(m)));
        }
    }
}

/// Table 6.18 picture-output group of a defined `mps_pic_struct_type` value
/// (AV2 § 6.16.10, `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-10`).
///
/// The three groups mirror the § 6.16.10 bitstream-conformance requirement: "It is
/// a requirement of bitstream conformance that when mps_pic_struct_type is present
/// that only one of the following conditions, for all pictures in the current CVS,
/// is true: – The value of mps_pic_struct_type is equal to 0, 7 or 8. – The value
/// of mps_pic_struct_type is equal to 1, 2, 9, 10, 11 or 12. – The value of
/// mps_pic_struct_type is equal to 3, 4, 5 or 6."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PicStructGroup {
    /// `mps_pic_struct_type` 0, 7 or 8 — frame output. Table 6.18 requires
    /// "ci_scan_type_idc shall be equal to 1" (and, for values 7 and 8,
    /// "equal_picture_interval shall be equal to 1").
    Frame,
    /// `mps_pic_struct_type` 1, 2, 9, 10, 11 or 12 — single-field output.
    /// Table 6.18 requires "ci_scan_type_idc shall be equal to 2".
    SingleField,
    /// `mps_pic_struct_type` 3, 4, 5 or 6 — field-pair output. Table 6.18 requires
    /// "ci_scan_type_idc shall be equal to 3".
    FieldPair,
}

impl PicStructGroup {
    /// Classifies a `mps_pic_struct_type` value into its Table 6.18 group; `None`
    /// for the reserved values above 12, which are excluded from the group state
    /// ("Decoders shall ignore reserved values of mps_pic_struct_type",
    /// AV2 § 6.16.10; the stateless `metadata/scan-type-pic-struct-reserved` check
    /// reports the reserved value itself).
    fn from_pic_struct(value: u8) -> Option<Self> {
        match value {
            0 | 7 | 8 => Some(Self::Frame),
            1 | 2 | 9..=12 => Some(Self::SingleField),
            3..=6 => Some(Self::FieldPair),
            _ => None,
        }
    }

    /// The `ci_scan_type_idc` value the Table 6.18 "Restrictions" column requires
    /// for this group (AV2 § 6.16.10): "ci_scan_type_idc shall be equal to" 1, 2,
    /// or 3 respectively.
    fn required_ci_scan_type_idc(self) -> u8 {
        match self {
            Self::Frame => 1,
            Self::SingleField => 2,
            Self::FieldPair => 3,
        }
    }

    /// The group's `mps_pic_struct_type` values, worded as in the § 6.16.10
    /// conformance requirement, for diagnostic messages.
    fn describe(self) -> &'static str {
        match self {
            Self::Frame => "0, 7 or 8",
            Self::SingleField => "1, 2, 9, 10, 11 or 12",
            Self::FieldPair => "3, 4, 5 or 6",
        }
    }
}

/// One defined-`mps_pic_struct_type` scan-type metadata observation within its
/// coded-video-sequence scope (AV2 § 6.16.10).
///
/// The Table 6.18 CI cross-checks pair each observation with each in-scope
/// content-interpretation record exactly once per distinct decisive CI content:
/// the metadata-time pass ([`ValidatorContext::check_scan_type_consistency`])
/// pairs a new observation against every record already in scope, and the
/// CI-time pass ([`ValidatorContext::recheck_scan_type_after_ci`]) runs only
/// when the new CI's Table 6.18-decisive content differs from the record it
/// replaces — so a repeated identical CI (the only legal repeat, § 6.14) never
/// re-reports, while a CI for a new embedded layer or with changed content is
/// evaluated against every stored observation.
#[derive(Debug)]
struct ScanTypeObservation {
    /// The observed `mps_pic_struct_type` (defined values 0..=12 only; reserved
    /// values never enter the state).
    mps_pic_struct_type: u8,
    /// Source byte offset of the carrying metadata OBU.
    offset: ByteOffset,
    /// Temporal unit ([`CvsTracker::tu_index`]) of the observation, used by the
    /// exact § 7.3.6 CVS scoping (CLK pruning and deferral decisions) and by the
    /// § 7.3.8.11 CI-parameter epoch checks.
    tu_index: u64,
    /// The content-interpretation identities `(obu_xlayer_id, obu_mlayer_id)` whose
    /// Table 6.18 restriction this observation already paired-and-emitted *eagerly*
    /// against, in its OWN temporal unit, at observation time (the scan-type analogue of
    /// the round-7 timecode finding 2). A CI key lands here when, at
    /// [`ValidatorContext::check_scan_type_consistency`], that already-recorded in-scope
    /// CI in this temporal unit decided a Table 6.18 restriction and the diagnostic was
    /// emitted (not deferred) — i.e. an identical CI was re-sent BEFORE the scan-type
    /// metadata in the same § 7.3.8.11 RAP temporal unit. The § 7.3.8.11 RAP re-pair
    /// ([`ValidatorContext::repair_post_rap_ci_pairings`]) skips only the
    /// `(observation, CI)` *pairs* recorded here, not the whole observation: a multi-layer
    /// stream can pair one observation with several CIs in opposite orderings relative to
    /// the metadata, so an eager emission against one CI must not suppress the re-pair of
    /// a different CI whose eager pairing was DEFERRED against a stale pre-RAP record (and
    /// dropped at the RAP). The set is empty for an observation that emitted nothing
    /// eagerly, and re-pairing covers every not-yet-emitted post-epoch pairing.
    eagerly_emitted: BTreeSet<ContentInterpretationKey>,
}

/// Per-scope scan-type observations (AV2 § 6.16.10). Append-only within the coded
/// video sequence: the group requirement binds the values *present* "for all
/// pictures in the current CVS", so neither § 6.16.3 cancellation nor persistence
/// expiry removes an observation — only the § 7.3.6 CVS boundary does (see
/// [`ValidatorContext::flush_scan_type_scope`]).
#[derive(Debug, Default)]
struct ScanTypeScope {
    observations: Vec<ScanTypeObservation>,
}

impl ScanTypeScope {
    /// The scope's group baseline: its first (oldest surviving) observation and
    /// that observation's Table 6.18 group. Stored observations carry only defined
    /// values, so the classification always succeeds; it is still expressed as a
    /// filter to keep the path panic-free.
    fn group_baseline(&self) -> Option<(&ScanTypeObservation, PicStructGroup)> {
        self.observations.first().and_then(|observation| {
            PicStructGroup::from_pic_struct(observation.mps_pic_struct_type)
                .map(|group| (observation, group))
        })
    }
}

/// Scan-type metadata CVS-consistency state (AV2 § 6.16.10), keyed by the carrying
/// OBU's `obu_xlayer_id`; [`GLOBAL_XLAYER_ID`] keys the global bucket. Global
/// scan-type metadata describes every layer's pictures, so the group rule compares
/// the global bucket against each concrete extended-layer scope and vice versa,
/// and the global bucket's Table 6.18 CI cross-checks consider every layer's
/// content-interpretation records.
#[derive(Debug, Default)]
struct ScanTypeCvsState {
    scopes: BTreeMap<ExtendedLayerId, ScanTypeScope>,
}

/// The pairing context of a § 6.16.10 Table 6.18 scan-type / content-interpretation
/// diagnostic: the content-interpretation side's layer ids, both OBUs' byte
/// offsets, and the byte offset to attach the diagnostic at (`at` — whichever OBU
/// completed the violating pair, the scan-type metadata OBU or the
/// content-interpretation OBU, whichever came second).
struct ScanTypeCiPair {
    ci_xlayer: ExtendedLayerId,
    ci_mlayer: EmbeddedLayerId,
    metadata_offset: ByteOffset,
    ci_offset: ByteOffset,
    at: ByteOffset,
}

/// The Table 6.18-decisive content of a content interpretation (AV2 § 6.16.10):
/// the established `ci_scan_type_idc` ("ci_scan_type_idc shall be equal to"
/// 1 / 2 / 3 per group) and whether a present `timing_info()` signals
/// `equal_picture_interval` 0 (the "equal_picture_interval shall be equal to 1"
/// half binding `mps_pic_struct_type` 7 / 8). Two content interpretations with
/// equal decisive content decide every Table 6.18 restriction identically.
fn scan_type_decisive_content(content: &ContentInterpretation) -> (u8, bool) {
    (
        content.scan_type_idc.get(),
        content
            .timing_info
            .is_some_and(|timing| !timing.equal_picture_interval),
    )
}

/// Builds the § 6.16.10 Table 6.18 `ci_scan_type_idc` mismatch diagnostic
/// (`metadata/scan-type-ci-scan-type-mismatch`).
fn scan_type_ci_mismatch_error(
    pic_struct: u8,
    required: u8,
    established: u8,
    pair: &ScanTypeCiPair,
) -> Diagnostic {
    Diagnostic::error(
        "metadata/scan-type-ci-scan-type-mismatch",
        format!(
            "mps_pic_struct_type {pic_struct} (scan-type metadata at byte {}) requires \
             ci_scan_type_idc equal to {required} per Table 6.18, but the content \
             interpretation for obu_xlayer_id {} / obu_mlayer_id {} (at byte {}) establishes \
             ci_scan_type_idc {established} within the coded video sequence",
            pair.metadata_offset,
            pair.ci_xlayer.get(),
            pair.ci_mlayer.get(),
            pair.ci_offset,
        ),
    )
    .with_spec_section("6.16.10")
    .with_byte_offset(pair.at)
}

/// Builds the § 6.16.10 Table 6.18 equal-picture-interval diagnostic
/// (`metadata/scan-type-equal-picture-interval-required`) for `mps_pic_struct_type`
/// 7 / 8 ("equal_picture_interval shall be equal to 1").
fn scan_type_equal_picture_interval_error(pic_struct: u8, pair: &ScanTypeCiPair) -> Diagnostic {
    Diagnostic::error(
        "metadata/scan-type-equal-picture-interval-required",
        format!(
            "mps_pic_struct_type {pic_struct} (scan-type metadata at byte {}) requires \
             equal_picture_interval equal to 1 per Table 6.18, but the content interpretation \
             timing_info() for obu_xlayer_id {} / obu_mlayer_id {} (at byte {}) signals \
             equal_picture_interval 0",
            pair.metadata_offset,
            pair.ci_xlayer.get(),
            pair.ci_mlayer.get(),
            pair.ci_offset,
        ),
    )
    .with_spec_section("6.16.10")
    .with_byte_offset(pair.at)
}

/// One observed `metadata_timecode()` unit's n_frames within its
/// coded-video-sequence scope (AV2 § 6.16.7), kept so a content interpretation that
/// arrives *after* the timecode (and establishes its `ci_timing_info_present_flag` /
/// timing) can re-evaluate the n_frames bound — the same arrival-order ambiguity the
/// § 6.16.10 scan-type / CI pairing handles (see [`ScanTypeObservation`]).
#[derive(Debug)]
struct TimecodeObservation {
    /// The observed `n_frames` value (AV2 § 6.16.7, `f(9)`).
    n_frames: u16,
    /// Source byte offset of the carrying metadata OBU (the diagnostic anchor — the
    /// offending timecode metadata OBU).
    offset: ByteOffset,
    /// Temporal unit ([`CvsTracker::tu_index`]) of the observation, for the exact
    /// § 7.3.6 CVS scoping and the § 7.3.8.11 CI-parameter epoch filter.
    tu_index: u64,
    /// The carrying OBU's `obu_xlayer_id` ([`GLOBAL_XLAYER_ID`] for a global OBU),
    /// used by the § 7.3.6 pruning when the unit's targeting is not derivable (finding
    /// 2 / finding 4).
    scope_xlayer: ExtendedLayerId,
    /// The unit's § 6.16.3 layer targeting, when derivable from the bitstream
    /// (finding 4): the n_frames bound pairs this timecode only with a content
    /// interpretation OBU for a layer it targets (see
    /// [`HdrAssociation::associated_with_ci`]). `None` when the targeting is not
    /// bitstream-derivable (LAYER_UNSPECIFIED, etc., see [`derive_hdr_association`]),
    /// in which case the n_frames bound compares NOTHING (the spec leaves the layer
    /// association unspecified, so no CI's rate binds this timecode — see
    /// [`timecode_ci_in_scope`]).
    targeting: Option<HdrAssociation>,
    /// The content-interpretation identities `(obu_xlayer_id, obu_mlayer_id)` whose
    /// n_frames bound this observation already paired-and-emitted *eagerly* against, in
    /// its OWN temporal unit, at observation time (round-7 finding 2). A CI key lands
    /// here when, at [`ValidatorContext::record_metadata_timecode_state`], that
    /// already-recorded in-scope CI in this temporal unit decided the bound and the
    /// diagnostic was emitted (not deferred) — i.e. an identical CI was re-sent BEFORE
    /// the timecode in the same § 7.3.8.11 RAP temporal unit. The § 7.3.8.11 RAP re-pair
    /// ([`ValidatorContext::repair_post_rap_ci_pairings`]) skips only the
    /// `(observation, CI)` *pairs* recorded here, not the whole observation: a multi-layer
    /// stream can pair one observation with several CIs in opposite orderings relative to
    /// the metadata, so an eager emission against one CI must not suppress the re-pair of
    /// a different CI whose eager pairing was DEFERRED against a stale pre-RAP record (and
    /// dropped at the RAP). The set is empty for an observation that emitted nothing
    /// eagerly, and re-pairing covers every not-yet-emitted post-epoch pairing.
    eagerly_emitted: BTreeSet<ContentInterpretationKey>,
}

impl TimecodeObservation {
    /// Whether this observation belongs to the coded video sequence of extended layer
    /// `xlayer` — i.e. a § 7.3.6 CVS restart for `xlayer` should drop it (finding 2).
    /// A derivable targeting decides it exactly (the layers the timecode describes); an
    /// underivable targeting (which compares nothing for the bound) falls back to the
    /// carrying `obu_xlayer_id` scope, with a global carrying scope touching every
    /// layer (the documented harmless any-CLK approximation for an inert observation).
    fn belongs_to_cvs_of(&self, xlayer: ExtendedLayerId) -> bool {
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
#[derive(Debug, Clone)]
struct TimecodeInferenceEntry {
    /// The previous set's literally-coded field presence (no OR with any inferred
    /// predecessor state — see the chain population in
    /// [`ValidatorContext::record_metadata_timecode_state`]).
    presence: TimecodeFieldPresence,
    /// The temporal unit the previous set was carried in, so the § 7.3.6 CVS
    /// boundary can tell an intra-CVS predecessor (same/later temporal unit) from
    /// one that belongs to the ending coded video sequence (earlier temporal unit).
    prev_tu: u64,
    /// The carrying OBU's `obu_xlayer_id` ([`GLOBAL_XLAYER_ID`] for a global OBU)
    /// of the set that wrote this entry — the fallback CVS scope when its targeting
    /// is not bitstream-derivable.
    scope_xlayer: ExtendedLayerId,
    /// The previous set's § 6.16.3 layer targeting, when derivable from the
    /// bitstream (round-7 finding 1). The chain entry is reset on a § 7.3.6 CLK only
    /// when that CLK restarts the coded video sequence of a layer the previous set
    /// actually targets, mirroring [`TimecodeObservation::belongs_to_cvs_of`] and
    /// [`PendingTimecodeInference::belongs_to_cvs_of`] — so a global `LAYER_VALUES`
    /// chain aimed at one extended layer survives a CLK for an unrelated layer rather
    /// than dropping on every CLK. `None` falls back to the carrying `obu_xlayer_id`
    /// scope (a global carrying scope touching every layer, the documented any-CLK
    /// approximation).
    targeting: Option<HdrAssociation>,
}

impl TimecodeInferenceEntry {
    /// Whether a § 7.3.6 CVS restart for extended layer `xlayer` detaches this chain
    /// entry's previous set — the same target-aware test as
    /// [`TimecodeObservation::belongs_to_cvs_of`] and
    /// [`PendingTimecodeInference::belongs_to_cvs_of`] (round-7 finding 1). A
    /// derivable targeting decides it exactly (the layers the previous set
    /// describes); an underivable targeting falls back to the carrying
    /// `obu_xlayer_id` scope, with a global carrying scope touching every layer (the
    /// documented harmless any-CLK approximation).
    fn belongs_to_cvs_of(&self, xlayer: ExtendedLayerId) -> bool {
        match &self.targeting {
            Some(association) => association.touches_xlayer(xlayer),
            None => self.scope_xlayer.is_global() || self.scope_xlayer == xlayer,
        }
    }
}

/// Per coded-video-sequence-scope timecode state (AV2 § 6.16.7).
///
/// Two § 6.16.7 facts are decidable from metadata alone and tracked here, each with
/// the keying the per-layer § 6.16.3 semantics demand:
///
/// - **Inference-presence** ([`inference`], the mirror's "When seconds_value
///   \[minutes_value, hours_value\] is not present, its value is inferred to be equal
///   to the value of \[that element\] for the previous set of clock timestamp syntax
///   elements **in decoding order**, and it is required that such a previous
///   \[element\] shall have been present",
///   `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-7`, lines
///   3873-3893). The chain is keyed by the carrying OBU's concrete
///   `(obu_xlayer_id, obu_mlayer_id)` (finding 3): § 6.16.3 marks
///   METADATA_TYPE_TIMECODE as layer-specific (Table 6.17, "Y"), so a timecode on
///   embedded layer `(x, m0)` is NOT the "previous set" of one on `(x, m1)` and must
///   not seed its inference. For a timecode whose targeting is unspecified
///   (`LAYER_UNSPECIFIED`, § 6.16.3 lines 3520-3521: "does not specify to what layers
///   the metadata applies to"), the chain still keys by the carrying OBU's own
///   `(obu_xlayer_id, obu_mlayer_id)` — the only concrete scope the bitstream pins
///   down (finding 4, documented sound choice: the "previous set in decoding order"
///   is read as the previous set carried at the same physical stream scope, which is
///   always derivable and never compares across unrelated targets).
/// - **n_frames bound re-check** ([`observations`]): observed timecodes' n_frames,
///   kept so a later content interpretation can re-evaluate the bound (the eager
///   metadata-time direction reads the already-stored CI timing). Each observation
///   carries its carrying-OBU `obu_xlayer_id` scope and its § 6.16.3 `targeting`, so
///   the § 7.3.6 per-extended-layer CVS pruning drops an observation only when a CLK
///   restarts the coded video sequence of a layer the observation actually targets
///   (finding 2 — a CLK for one extended layer no longer prunes a global-bucket
///   observation aimed at another).
///
/// Both facts reset at the § 7.3.6 per-extended-layer CVS boundary (a CLK starts a
/// new coded video sequence, breaking the decoding-order inference chain) via the
/// [`ValidatorContext::prune_timecode_scope`] call sites in
/// [`ValidatorContext::start_cvs_for_xlayer`].
#[derive(Debug, Default)]
struct TimecodeCvsState {
    /// Inference-presence state per carrying-OBU `(obu_xlayer_id, obu_mlayer_id)`:
    /// the previous-set field presence, the temporal unit of that previous set, and
    /// the § 6.16.3 targeting of the set that wrote it (finding 3). `None`-keyed
    /// entries do not exist — every timecode has a concrete carrying scope. The
    /// temporal unit lets the § 7.3.6 CVS boundary reset the chain (a previous set
    /// from an earlier temporal unit belongs to the ending coded video sequence and
    /// no longer seeds the new one); the targeting makes that reset target-aware so a
    /// CLK for an unrelated extended layer no longer drops a global `LAYER_VALUES`
    /// chain aimed at a different layer (round-7 finding 1, mirroring
    /// [`TimecodeObservation::belongs_to_cvs_of`]).
    inference: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), TimecodeInferenceEntry>,
    /// n_frames observations, flat and self-describing (each carries its
    /// carrying-`obu_xlayer_id` scope, § 7.3.8.11 epoch tu, and § 6.16.3 targeting),
    /// for the CI-after re-check of the n_frames bound and the target-aware § 7.3.6
    /// pruning (finding 2).
    observations: Vec<TimecodeObservation>,
    /// Inference-presence diagnostics whose firing depends on whether a § 7.3.6
    /// CVS boundary is crossed later in the current temporal unit (AV2 § 6.16.7).
    ///
    /// A timecode that omits a field, seeded only by a *present* value from a
    /// previous set in an **earlier** temporal unit, sits in the same coded video
    /// sequence as that seed *unless* a CLK later in this temporal unit starts a
    /// new coded video sequence (§ 7.3.6: the whole temporal unit containing a CLK
    /// joins the new sequence). If that happens, the seed belongs to the ending
    /// sequence, no source remains for the inference, and the diagnostic fires; if
    /// the temporal unit completes with no such boundary, the seed is intra-CVS and
    /// the field infers cleanly. The decision is therefore deferred to the temporal
    /// unit's resolution: [`ValidatorContext::emit_pending_timecode_inference`]
    /// emits matching entries on a CVS start, and
    /// [`ValidatorContext::drop_pending_timecode_inference`] drops the survivors
    /// silently at the temporal-unit flush. This mirrors the
    /// [`PendingPolarity::PreCvs`] machinery, but is kept dedicated to the timecode
    /// state because it keys the carrying OBU's exact `(obu_xlayer_id,
    /// obu_mlayer_id)`, which the per-layer [`CvsTracker::defer_pre_cvs`] path does
    /// not model.
    pending_inference: Vec<PendingTimecodeInference>,
}

/// A § 6.16.7 inference-presence diagnostic deferred until the current temporal
/// unit's § 7.3.6 CVS scope is resolved (see [`TimecodeCvsState::pending_inference`]).
#[derive(Debug)]
struct PendingTimecodeInference {
    /// The carrying OBU's `obu_xlayer_id` of the omitting timecode ([`GLOBAL_XLAYER_ID`]
    /// for a global OBU), the fallback CVS scope when the targeting is not derivable.
    xlayer: ExtendedLayerId,
    /// The omitting timecode's § 6.16.3 layer targeting, when derivable from the
    /// bitstream (finding 2). The deferred diagnostic fires only when a CLK restarts the
    /// coded video sequence of a layer this timecode actually targets — mirroring
    /// [`TimecodeObservation::belongs_to_cvs_of`] — so a global `LAYER_VALUES` timecode
    /// aimed at one extended layer is left pending by an unrelated layer's CLK rather
    /// than firing on every CLK. `None` falls back to the carrying `obu_xlayer_id`
    /// scope (a global carrying scope touching every layer, the documented any-CLK
    /// approximation).
    targeting: Option<HdrAssociation>,
    /// The inference-without-previous diagnostic to emit if the seed turns out to
    /// belong to the ending coded video sequence.
    diagnostic: Diagnostic,
}

impl PendingTimecodeInference {
    /// Whether a § 7.3.6 CVS restart for extended layer `xlayer` detaches this
    /// deferred timecode's earlier-temporal-unit inference seed — the same
    /// target-aware test as [`TimecodeObservation::belongs_to_cvs_of`] (finding 2). A
    /// derivable targeting decides it exactly (the layers the timecode describes); an
    /// underivable targeting falls back to the carrying `obu_xlayer_id` scope, with a
    /// global carrying scope touching every layer (the documented harmless any-CLK
    /// approximation, matching the eager-fire path of [`Self`] for a missing seed).
    fn belongs_to_cvs_of(&self, xlayer: ExtendedLayerId) -> bool {
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
struct TimecodeFieldPresence {
    seconds: bool,
    minutes: bool,
    hours: bool,
}

impl TimecodeFieldPresence {
    /// Records the present fields of a parsed timecode (each `Option` is `Some` when
    /// the field was coded, per the § 5.17.7 presence flags).
    fn of(timecode: &MetadataTimecode) -> Self {
        Self {
            seconds: timecode.seconds_value.is_some(),
            minutes: timecode.minutes_value.is_some(),
            hours: timecode.hours_value.is_some(),
        }
    }

    /// Whether the named clock-timestamp field (`"seconds_value"`,
    /// `"minutes_value"`, or `"hours_value"`) carried a present value.
    fn field(&self, name: &str) -> bool {
        match name {
            "seconds_value" => self.seconds,
            "minutes_value" => self.minutes,
            "hours_value" => self.hours,
            _ => false,
        }
    }
}

/// Whether a content interpretation OBU for embedded layer `(ci_xlayer,
/// ci_mlayer)` is in scope for a § 6.16.7 timecode's n_frames bound (finding 4).
///
/// When the timecode's § 6.16.3 `targeting` is bitstream-derivable, it decides the
/// pairing exactly (see [`HdrAssociation::associated_with_ci`]): a global
/// `LAYER_VALUES` timecode that names only some layers does not pair with a CI for
/// an untargeted layer; a `LAYER_GLOBAL` global unit ([`HdrAssociation::Universal`])
/// pairs with every CI ("The metadata applies to all layers", § 6.16.3).
///
/// **Underivable targeting compares NOTHING (zero-false-positive rule, finding 4).**
/// When the targeting is not bitstream-derivable (`None` — `LAYER_UNSPECIFIED` and
/// the other [`derive_hdr_association`] gaps), the pairing compares nothing. § 6.16.3
/// is explicit that `LAYER_UNSPECIFIED` "does not specify to what layers the metadata
/// applies to. This information can potentially be indicated or determined through
/// external means" (mirror
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-3`, lines
/// 3520-3521). The earlier coarse fallback — pair with every CI in the carrying OBU's
/// `obu_xlayer_id` scope, and with EVERY CI for the global bucket — could fire a hard
/// `metadata/timecode-n-frames-exceeds-rate` error against a CI for a layer the
/// timecode never claims to describe. Since the spec leaves the association
/// unspecified, the validator cannot soundly bind the n_frames rate of any particular
/// layer's CI to this timecode, so it compares nothing: an honest false-negative, never
/// a false positive against an unrelated layer.
///
/// **§ 7.3.8.11 inheritance residual (NOT modeled — honest narrowing).** A
/// `LAYER_VALUES` timecode targeting embedded layer `m` is governed by the content
/// interpretation parameters of layer `m`; but § 7.3.8.11 step 3 (mirror
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3-8-11`, lines 925-929) says that
/// when no content interpretation OBU is present for `m`, its parameters are
/// *inherited* from "the highest such embedded layer `k` less than `m`" with
/// `MLayerPresenceMap[m][k] == 1`. Resolving that inheritance requires
/// `MLayerPresenceMap` — the transitive closure of `MLayerDependencyMap` derived in
/// § 5.4.1 (mirror `05-syntax-structures.md` lines 583-607) — which this validator
/// does NOT model (`splot-core` carries `MLayerDependencyMap` but never derives the
/// presence closure, and no `mlayer_presence` / `MLayerPresenceMap` state exists).
/// Rather than guess the closure, the gate stays exact: when the timecode targets
/// `m` and the only in-scope CI is at a lower layer `k != m`,
/// [`HdrAssociation::associated_with_ci`]'s `Pairs` arm returns `false` and this
/// pairing compares NOTHING — an honest false-negative, not a silent in-scope-clean
/// pass. The unresolved inheritance is treated as not-pairable so the bound is never
/// evaluated against a CI whose governance over `m` cannot be confirmed from modeled
/// state. Resolving it (deriving `MLayerPresenceMap` and pairing a layer-`m` timecode
/// with its inherited layer-`k` CI) is deferred.
// TODO(spec: AV2-5.17.7-METADATA-TIMECODE): resolve § 7.3.8.11 step-3 content
// interpretation inheritance through MLayerPresenceMap so a LAYER_VALUES timecode
// targeting embedded layer m pairs with the n_frames bound of the inherited CI from
// the highest lower layer k with MLayerPresenceMap[m][k] == 1 when no CI exists for
// m. Blocked on modeling MLayerPresenceMap (§ 5.4.1 transitive closure of the
// already-modeled MLayerDependencyMap); until then an exact-pair miss with possible
// inheritance compares nothing.
fn timecode_ci_in_scope(
    targeting: &Option<HdrAssociation>,
    ci_xlayer: ExtendedLayerId,
    ci_mlayer: EmbeddedLayerId,
) -> bool {
    match targeting {
        Some(association) => association.associated_with_ci(ci_xlayer, ci_mlayer),
        // Underivable targeting (LAYER_UNSPECIFIED, ...): the spec does not say which
        // layers the metadata applies to, so no CI's rate can be soundly bound to it.
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
fn max_pic_per_second(timing: &TimingInfo) -> u64 {
    let ticks_per_picture = if timing.equal_picture_interval {
        // num_ticks_per_picture_minus_1 is Some when equal_picture_interval; treat an
        // unexpected None as 0 (TicksPerPicture == num_units_in_display_tick) — a
        // conservative fallback that never panics and never under-counts the bound.
        let ticks_minus_1 = u64::from(timing.num_ticks_per_picture_minus_1.unwrap_or(0));
        (ticks_minus_1 + 1) * u64::from(timing.num_units_in_display_tick)
    } else {
        u64::from(timing.num_units_in_display_tick)
    };
    // ceil(time_scale / ticks_per_picture) for positive integers.
    let time_scale = u64::from(timing.time_scale);
    time_scale.div_ceil(ticks_per_picture)
}

/// Builds the § 6.16.7 n_frames-exceeds-rate diagnostic
/// (`metadata/timecode-n-frames-exceeds-rate`), anchored at the offending timecode
/// metadata OBU.
fn timecode_n_frames_error(
    n_frames: u16,
    max_pic_per_second: u64,
    ci_xlayer: ExtendedLayerId,
    ci_mlayer: EmbeddedLayerId,
    ci_offset: ByteOffset,
    metadata_offset: ByteOffset,
    at: ByteOffset,
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
    .with_byte_offset(at)
}

/// Exact coded-video-sequence boundary tracker (AV2 § 7.3.6,
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3-6`).
///
/// "A new coded video sequence for an extended layer is defined to start at each
/// temporal unit that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY in
/// the coded extended layer unit corresponding to the extended layer" (§ 7.3.6).
/// Two consequences drive this design:
///
/// - The boundary event is per extended layer and per temporal unit: the first
///   `OBU_CLOSED_LOOP_KEY` observed for an `obu_xlayer_id` within a temporal unit
///   starts the new coded video sequence (later CLKs in the same temporal unit are
///   idempotent). The raw OBU header suffices — § 7.3.6 is stated at the OBU level,
///   so an unparsable CLK payload still starts the sequence. OLK / RAS OBUs do NOT
///   start one during sequential decoding (§ 2 "Coded video sequence"; § 7.4.4:
///   "During sequential decoding, the process does not start a new coded video
///   sequence for the extended layer").
/// - The new sequence starts *at the temporal unit*, so OBUs of the same extended
///   layer earlier in that temporal unit (e.g. the sequence header preceding the
///   activating CLK) already belong to the NEW coded video sequence — and the
///   validator cannot know a CLK is still coming when it observes them. CVS-scoped
///   comparisons whose baseline record came from an *earlier* temporal unit are
///   therefore deferred and flushed when the temporal unit completes: dropped when
///   the record's extended layer started a coded video sequence in that temporal
///   unit, emitted otherwise. Same-temporal-unit comparisons are always within one
///   coded video sequence and are emitted eagerly.
///
/// Records keyed under `GLOBAL_XLAYER_ID` have no single owning extended layer; as a
/// documented approximation their deferred diagnostics are dropped when ANY extended
/// layer started a coded video sequence in the completed temporal unit (sound — it
/// only drops comparisons, never inventing one).
#[derive(Debug, Default)]
struct CvsTracker {
    /// Index of the current temporal unit; incremented at each global
    /// `OBU_TEMPORAL_DELIMITER` (AV2 § 7.3.7).
    tu_index: u64,
    /// For each extended layer, the temporal unit in which its most recent coded
    /// video sequence started (§ 7.3.6 CLK boundary events).
    cvs_started_in_tu: BTreeMap<ExtendedLayerId, u64>,
    /// Monotonic count of coded-video-sequence starts across the whole bitstream,
    /// incremented once per § 7.3.6 CLK boundary event (idempotent within a temporal
    /// unit). A global (`GLOBAL_XLAYER_ID`) scope, which spans the whole multistream
    /// and has no single owning extended layer, uses this counter as its CVS epoch:
    /// any CLK in any extended layer bumps it, so two observations sharing this value
    /// are guaranteed to lie within one coded video sequence of every layer (sound —
    /// it only adds boundaries, never removes one). See [`CvsTracker::cvs_generation_epoch`].
    cvs_generation: u64,
    /// For each extended layer, the [`CvsTracker::cvs_generation`] value at which its
    /// most recent coded video sequence started — the per-layer CVS epoch used to
    /// scope the § 6.10.5 buffer-delay sum-constancy comparison.
    cvs_generation_for: BTreeMap<ExtendedLayerId, u64>,
    /// Deferred cross-temporal-unit CVS-scoped diagnostics, tagged with the extended
    /// layer that scopes the comparison; flushed when the temporal unit completes. Each
    /// entry may carry an optional `on_drop` replacement diagnostic emitted in place of
    /// the primary when the primary is dropped because a § 7.3.6 coded-video-sequence
    /// boundary was crossed (the comparison was genuinely cross-CVS after all). The
    /// mechanism is rule-id-agnostic: a caller that wants no replacement passes `None`.
    pending_cross_tu: Vec<PendingCrossTu>,
}

/// The flush polarity of a deferred [`PendingCrossTu`] comparison. The two § 7.3.6
/// boundary events (`CvsTracker::start_cvs` and `CvsTracker::flush_completed_tu`) handle
/// a pending entry oppositely depending on which side of the CLK boundary the comparison
/// is sound on; the polarity selects which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingPolarity {
    /// The comparison's baseline is in an EARLIER temporal unit, so the comparison is an
    /// intra-CVS assertion that a same-temporal-unit CLK could falsify. Emit the primary
    /// when the temporal unit completes without a CVS start for the layer; on a CVS start
    /// drop the primary and emit any `on_drop` replacement (the comparison spanned the
    /// § 7.3.6 boundary after all). This is the deferred-error polarity.
    CvsScoped,
    /// Both observations are in the CURRENT temporal unit but no coded video sequence has
    /// started for the layer yet (the pre-first-CLK silence path). Per § 7.3.6 the whole
    /// temporal unit containing a CLK belongs to the NEW coded video sequence, so a CLK
    /// later in this temporal unit pulls BOTH observations into one coded video sequence
    /// and the change is intra-CVS — emit the primary on the CVS start. If the temporal
    /// unit instead completes with no CLK for the layer, the observations remain in no
    /// coded video sequence (the § 6.10.5 random-access-point precondition is
    /// unsatisfied), so drop the primary silently. This is the inverse of `CvsScoped`.
    PreCvs,
}

/// One deferred cross-temporal-unit CVS-scoped diagnostic (see
/// [`CvsTracker::pending_cross_tu`]).
#[derive(Debug)]
struct PendingCrossTu {
    /// The extended layer scoping the comparison; `GLOBAL_XLAYER_ID` for a record with
    /// no single owning extended layer.
    xlayer: ExtendedLayerId,
    /// Which § 7.3.6 boundary event emits this entry's primary (see [`PendingPolarity`]).
    polarity: PendingPolarity,
    /// The primary diagnostic. For [`PendingPolarity::CvsScoped`] it is emitted when the
    /// temporal unit completes without a CVS start for the layer; for
    /// [`PendingPolarity::PreCvs`] it is emitted on a CVS start for the layer.
    primary: Diagnostic,
    /// The replacement emitted instead of `primary` when `primary` is dropped because a
    /// coded-video-sequence boundary was crossed (the comparison spanned the boundary).
    /// Used only by [`PendingPolarity::CvsScoped`]; a [`PendingPolarity::PreCvs`] entry
    /// that is dropped (its temporal unit closed with no CLK) leaves the observations in
    /// no coded video sequence, so its dropped comparison must simply vanish (`None`).
    on_drop: Option<Diagnostic>,
}

impl CvsTracker {
    /// Records a § 7.3.6 boundary event: a CLK OBU for `xlayer` starts a new coded
    /// video sequence at the current temporal unit. Pending deferred diagnostics for
    /// `xlayer` are resolved by their [`PendingPolarity`]:
    ///
    /// - [`PendingPolarity::CvsScoped`]: the comparison spanned this CVS boundary, so the
    ///   primary is dropped and any `on_drop` replacement is emitted in its place.
    /// - [`PendingPolarity::PreCvs`]: per § 7.3.6 the whole temporal unit containing this
    ///   CLK belongs to the new coded video sequence, so both observations are now
    ///   intra-CVS — the primary is emitted.
    ///
    /// Both pending kinds are recorded only in the current temporal unit (a `CvsScoped`
    /// entry whose baseline came from an earlier temporal unit, a `PreCvs` entry whose
    /// observations are both in this temporal unit) and are flushed at the temporal-unit
    /// boundary, so every entry present here is necessarily tagged to this temporal unit.
    /// Idempotent within a temporal unit.
    fn start_cvs(&mut self, xlayer: ExtendedLayerId, report: &mut ValidationReport) {
        // Bump the CVS generation only once per (xlayer, temporal unit): a redundant
        // CLK in the same temporal unit is the same § 7.3.6 boundary event, so it must
        // not advance the epoch (matches the idempotent `cvs_started_in_tu` insert).
        if self.cvs_started_in_tu.get(&xlayer) != Some(&self.tu_index) {
            self.cvs_generation += 1;
            self.cvs_generation_for.insert(xlayer, self.cvs_generation);
        }
        self.cvs_started_in_tu.insert(xlayer, self.tu_index);
        let mut retained = Vec::with_capacity(self.pending_cross_tu.len());
        for entry in std::mem::take(&mut self.pending_cross_tu) {
            if entry.xlayer == xlayer {
                match entry.polarity {
                    PendingPolarity::CvsScoped => {
                        if let Some(replacement) = entry.on_drop {
                            report.push(replacement);
                        }
                    }
                    PendingPolarity::PreCvs => report.push(entry.primary),
                }
            } else {
                retained.push(entry);
            }
        }
        self.pending_cross_tu = retained;
    }

    /// The temporal unit in which `xlayer`'s current coded video sequence started, or
    /// `None` when no CLK boundary event has been observed for it yet (the implicit
    /// coded video sequence that began at the start of the bitstream, § 7.3.6). Two
    /// events sharing a CVS epoch are in the same coded video sequence; `None` (no CLK)
    /// is distinct from `Some(0)` (a CLK in the first temporal unit), so a re-activation
    /// across a first-temporal-unit CLK is correctly treated as a new coded video
    /// sequence.
    fn cvs_epoch(&self, xlayer: ExtendedLayerId) -> Option<u64> {
        self.cvs_started_in_tu.get(&xlayer).copied()
    }

    /// The generation-counter CVS epoch scoping the § 6.10.5 / § 6.4.13 buffer-delay
    /// comparisons for `xlayer`: the [`CvsTracker::cvs_generation`] at which the layer's
    /// current coded video sequence started, or — for the multistream-wide
    /// `GLOBAL_XLAYER_ID` scope — the running generation counter so any CLK in any layer
    /// changes the epoch. Returns 0 before any CLK has been observed. Distinct from
    /// [`CvsTracker::cvs_epoch`], which returns the temporal-unit index used by the
    /// § 7.3.6 single-active-sequence-header check.
    fn cvs_generation_epoch(&self, xlayer: ExtendedLayerId) -> u64 {
        if xlayer.is_global() {
            self.cvs_generation
        } else {
            self.cvs_generation_for.get(&xlayer).copied().unwrap_or(0)
        }
    }

    /// Whether a coded video sequence has started for `xlayer` — i.e. its
    /// [`CvsTracker::cvs_generation_epoch`] is non-zero (§ 7.3.6: a CVS "is defined to
    /// start at each temporal unit that contains an OBU with obu_type equal to
    /// OBU_CLOSED_LOOP_KEY"). For the multistream-wide `GLOBAL_XLAYER_ID` scope this is
    /// true once any extended layer has started a coded video sequence. Before the first
    /// CLK the layer's OBUs lie in no coded video sequence at all, so the intra-CVS
    /// error tier (whose constraint binds only "within one coded video sequence") must
    /// not compare them.
    fn cvs_started(&self, xlayer: ExtendedLayerId) -> bool {
        self.cvs_generation_epoch(xlayer) > 0
    }

    /// Routes a CVS-scoped comparison diagnostic. `record_tu` is the temporal unit
    /// of the baseline record being compared against: a same-temporal-unit baseline
    /// is always in the same coded video sequence (§ 7.3.6: a coded video sequence
    /// starts at a temporal unit, never inside one), so the diagnostic is emitted
    /// eagerly; a baseline from an earlier temporal unit is deferred, because a CLK
    /// later in the current temporal unit would put the baseline and the new
    /// observation in different coded video sequences.
    fn defer_or_emit(
        &mut self,
        xlayer: ExtendedLayerId,
        record_tu: u64,
        diagnostic: Diagnostic,
        report: &mut ValidationReport,
    ) {
        self.defer_or_emit_with_replacement(xlayer, record_tu, diagnostic, None, report);
    }

    /// Like [`CvsTracker::defer_or_emit`], but a deferred primary that is later dropped
    /// because a coded-video-sequence boundary was crossed is replaced by `on_drop`
    /// (when `Some`). When the primary is emitted eagerly (same temporal unit) or
    /// flushed unchanged at the completed temporal unit, `on_drop` is discarded — the
    /// comparison stayed within one coded video sequence. The mechanism stays
    /// rule-id-agnostic; the caller decides what the cross-boundary replacement says.
    fn defer_or_emit_with_replacement(
        &mut self,
        xlayer: ExtendedLayerId,
        record_tu: u64,
        diagnostic: Diagnostic,
        on_drop: Option<Diagnostic>,
        report: &mut ValidationReport,
    ) {
        if record_tu == self.tu_index {
            report.push(diagnostic);
        } else {
            self.pending_cross_tu.push(PendingCrossTu {
                xlayer,
                polarity: PendingPolarity::CvsScoped,
                primary: diagnostic,
                on_drop,
            });
        }
    }

    /// Records a [`PendingPolarity::PreCvs`] comparison: both observations are in the
    /// current temporal unit, but no coded video sequence has started for `xlayer` yet
    /// (the pre-first-CLK silence path). Per § 7.3.6 a CLK later in this temporal unit
    /// pulls both observations into one coded video sequence, so the captured `diagnostic`
    /// is emitted on the next [`CvsTracker::start_cvs`] for `xlayer`; if the temporal unit
    /// closes first with no CLK for the layer, [`CvsTracker::flush_completed_tu`] drops it
    /// silently (the observations are in no coded video sequence). The caller must have
    /// already established `cvs_started(xlayer) == false`; `xlayer` must be a concrete
    /// extended layer (global keys keep the documented cross-CMVS under-report and are not
    /// deferred here).
    fn defer_pre_cvs(
        &mut self,
        xlayer: ExtendedLayerId,
        diagnostic: Diagnostic,
        report: &mut ValidationReport,
    ) {
        debug_assert!(
            !xlayer.is_global(),
            "PreCvs deferral is for concrete extended layers only",
        );
        // Guard against a logic error rather than emit a stray diagnostic in release: a
        // global key must never reach the per-layer pending machinery. Dropping it here
        // matches the documented global under-report and cannot fire on the only caller
        // (which screens out global keys before calling).
        if xlayer.is_global() {
            let _ = report;
            return;
        }
        self.pending_cross_tu.push(PendingCrossTu {
            xlayer,
            polarity: PendingPolarity::PreCvs,
            primary: diagnostic,
            on_drop: None,
        });
    }

    /// Flushes the deferred diagnostics of the just-completed temporal unit, resolving
    /// each entry by its [`PendingPolarity`]:
    ///
    /// - [`PendingPolarity::CvsScoped`]: the primary is dropped when its extended layer
    ///   started a new coded video sequence in this temporal unit (the compared records
    ///   then sit in different coded video sequences, § 7.3.6) and any `on_drop`
    ///   replacement is emitted in its place; otherwise the primary is emitted.
    /// - [`PendingPolarity::PreCvs`]: a CVS start would already have emitted and removed
    ///   the entry in [`CvsTracker::start_cvs`], so any `PreCvs` entry surviving to this
    ///   flush is one whose temporal unit closed with no CLK for the layer — its two
    ///   observations remain in no coded video sequence (the § 6.10.5 random-access-point
    ///   precondition is unsatisfied), so it is dropped silently (pre-first-CLK silence).
    ///
    /// An entry tagged with `GLOBAL_XLAYER_ID` scopes records with no single owning
    /// extended layer and treats "started a coded video sequence in this temporal unit"
    /// as ANY extended layer having done so (documented approximation, sound: it only
    /// drops comparisons).
    fn flush_completed_tu(&mut self, report: &mut ValidationReport) {
        let tu_index = self.tu_index;
        let any_started_this_tu = self.cvs_started_in_tu.values().any(|&tu| tu == tu_index);
        for entry in std::mem::take(&mut self.pending_cross_tu) {
            match entry.polarity {
                PendingPolarity::CvsScoped => {
                    let started_this_tu = if entry.xlayer.is_global() {
                        any_started_this_tu
                    } else {
                        self.cvs_started_in_tu.get(&entry.xlayer) == Some(&tu_index)
                    };
                    if started_this_tu {
                        if let Some(replacement) = entry.on_drop {
                            report.push(replacement);
                        }
                    } else {
                        report.push(entry.primary);
                    }
                }
                // A surviving PreCvs entry means no CLK arrived for the layer this
                // temporal unit: the observations are in no coded video sequence, so the
                // comparison is dropped silently (it carries no `on_drop` replacement).
                PendingPolarity::PreCvs => {}
            }
        }
    }

    /// Completes the current temporal unit at a global `OBU_TEMPORAL_DELIMITER`
    /// (AV2 § 7.3.7): flushes the deferred diagnostics, then advances `tu_index`.
    fn advance_temporal_unit(&mut self, report: &mut ValidationReport) {
        self.flush_completed_tu(report);
        self.tu_index += 1;
    }

    /// Drops pending deferred diagnostics carrying one of exactly `rule_ids`
    /// that are tagged with `xlayer` — or with `GLOBAL_XLAYER_ID`: a
    /// global-bucket comparison has no single owning extended layer, so dropping
    /// it at any layer's epoch event is the same documented sound approximation
    /// as [`CvsTracker::flush_completed_tu`] (it only drops comparisons). Used
    /// for the § 6.16.10 Table 6.18 pairing rules, whose deferred diagnostics a
    /// § 7.3.8.11 random access point invalidates without ending the coded video
    /// sequence (AV2 § 7.4.4); every other pending diagnostic is CVS-scoped and
    /// must survive.
    fn drop_pending_for_rules(&mut self, xlayer: ExtendedLayerId, rule_ids: &[&str]) {
        // The match invalidates the comparison outright — no `on_drop` replacement is
        // emitted, since the random access point did not cross a coded video sequence
        // boundary (the pairing diagnostics never carry one anyway).
        self.pending_cross_tu.retain(|entry| {
            !((entry.xlayer == xlayer || entry.xlayer.is_global())
                && rule_ids.contains(&entry.primary.rule_id.as_str()))
        });
    }
}

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
struct DistinctMlayerTracker {
    /// For each extended layer, the distinct `obu_mlayer_id` values counted in its
    /// current coded video sequence, and whether the § 6.4.1 exceedance has already
    /// been reported for that coded video sequence (emit once per CVS).
    per_xlayer: BTreeMap<ExtendedLayerId, DistinctMlayerState>,
    /// For each extended layer, the distinct `obu_mlayer_id` values observed so far in
    /// the *current temporal unit* (cleared at each temporal-unit advance, analogous to
    /// [`ValidatorContext::frames_seen_in_tu`]). § 7.3.6 (mirror
    /// `07-decoding-process.md` lines 604-606) starts a new coded video sequence AT the
    /// temporal unit containing the CLK, so every id of that temporal unit — including a
    /// § 7.3.8.1 resent-at-RAP sequence header observed before the CLK — belongs to the
    /// NEW coded video sequence. [`Self::reset_cvs`] re-seeds the new coded video
    /// sequence from this per-temporal-unit set so those pre-CLK ids are attributed to
    /// the new CVS (exact re-attribution, not the former whole-state drop).
    per_xlayer_tu: BTreeMap<ExtendedLayerId, BTreeSet<EmbeddedLayerId>>,
}

/// One extended layer's distinct-`obu_mlayer_id` count state within its current coded
/// video sequence; see [`DistinctMlayerTracker`].
#[derive(Debug, Default)]
struct DistinctMlayerState {
    /// The distinct `obu_mlayer_id` values seen so far in this coded video sequence.
    seen: BTreeSet<EmbeddedLayerId>,
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
    first_tu: Option<u64>,
    /// `true` once `sequence-state/distinct-mlayer-count-exceeds-seq-max` has been
    /// emitted for this coded video sequence (the check emits once per CVS).
    reported: bool,
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
    fn reset_cvs(&mut self, xlayer: ExtendedLayerId, tu_index: u64) {
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
    fn advance_temporal_unit(&mut self) {
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
    fn observe(
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
    fn mark_reported(&mut self, xlayer: ExtendedLayerId) {
        self.per_xlayer.entry(xlayer).or_default().reported = true;
    }

    /// Returns the distinct `obu_mlayer_id` count accumulated so far in `xlayer`'s
    /// current coded video sequence and the set's first-counted temporal unit, or `None`
    /// when nothing has been counted yet or the exceedance was already reported for this
    /// coded video sequence (emit once per CVS). Read-only: it does not record a new id,
    /// so it surfaces a count that [`Self::observe`] cannot re-yield (its already-seen ids
    /// return `None`). Used by the activation-path retroactive check, which compares a
    /// count accumulated *before* a sequence header became active for the extended layer.
    fn current_count(&self, xlayer: ExtendedLayerId) -> Option<(usize, u64)> {
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
struct MsdoKeyFields {
    /// `multistream_profile_idc`.
    profile_idc: u8,
    /// `multistream_level_idx`.
    level_idx: u8,
    /// `multistream_tier`.
    tier: u8,
    /// `num_streams_minus_2`.
    num_streams_minus_2: u8,
    /// `multistream_even_allocation_flag`.
    even_allocation_flag: bool,
    /// `multistream_large_picture_idc` (`None` under even allocation).
    large_picture_idc: Option<u8>,
}

impl MsdoKeyFields {
    /// Projects the § 7.3.2 condition-2 key fields out of a parsed MSDO.
    fn from_msdo(msdo: &MultistreamDecoderOperation) -> Self {
        Self {
            profile_idc: msdo.multistream_profile_idc,
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
enum MsdoObservation {
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
struct MsdoIdentityTracker {
    /// Payload fingerprint of the most recent OBU_MSDO resolved into the reference, or
    /// `None` until the first MSDO completes a temporal unit. The "previous OBU_MSDO"
    /// anchor for the next comparison.
    previous: Option<u64>,
    /// MSDOs seen in the temporal unit currently being observed, in decode order:
    /// `(payload_fingerprint, offset)`.
    current_tu: Vec<(u64, ByteOffset)>,
    /// Whether the temporal unit currently being observed is a § 7.4.1 random access
    /// point (contains a CLK / OLK / RAS OBU). Resolved at temporal-unit completion.
    current_tu_is_rap: bool,
}

impl MsdoIdentityTracker {
    /// Buffers one parsed OBU_MSDO's payload fingerprint and offset for the temporal
    /// unit currently being observed (resolved at [`Self::complete_temporal_unit`]).
    fn note_msdo(&mut self, fingerprint: u64, offset: ByteOffset) {
        self.current_tu.push((fingerprint, offset));
    }

    /// Marks the temporal unit currently being observed as a § 7.4.1 random access
    /// point (a CLK / OLK / RAS OBU was seen in it).
    fn note_random_access_point(&mut self) {
        self.current_tu_is_rap = true;
    }

    /// Resolves the § 7.3.8.2 identity rule for the just-completed temporal unit and
    /// resets the per-temporal-unit working state. Called at each global temporal
    /// delimiter and once at end of stream for the final temporal unit (see
    /// [`ValidatorContext::finish`]).
    fn complete_temporal_unit(&mut self, report: &mut ValidationReport) {
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
enum RapHlsKey {
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
    fn family(self) -> &'static str {
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
    fn family_section(self) -> &'static str {
        match self {
            Self::SequenceHeader(_) => "7.3.8.6",
            Self::MultiFrameHeader(_) => "7.3.8.7",
            Self::OperatingPointSet { .. } => "7.3.8.5",
            Self::LayerConfigurationRecord { .. } => "7.3.8.3",
            Self::Atlas { .. } => "7.3.8.4",
        }
    }

    /// A short identifier of the referenced object for the diagnostic message.
    fn describe(self) -> String {
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
struct RapResendEvent {
    /// The temporal-unit index in which the object was (re)sent.
    tu: u64,
    /// The extended layer whose coded extended layer unit carried the (re)send. A
    /// [`GLOBAL_XLAYER_ID`] send has no single owning layer (it is decoded by whichever
    /// layer first random-accesses there). § 7.4.6 sender-decodability uses this to decide
    /// whether the (re)send's layer is decoded under a given random-access start.
    sending_xlayer: ExtendedLayerId,
    /// Whether the sending temporal unit carried a LEADING_* frame OBU in *any* layer
    /// (resolved at temporal-unit completion). § 7.3.8.1: a decode starting at an earlier
    /// random access point "drops any temporal units containing leading frames", so a
    /// strictly-later (re)send in a leading temporal unit is not visible under that start.
    tu_has_any_leading: bool,
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
struct RapPendingReference {
    /// The referenced object.
    key: RapHlsKey,
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
    governing_xlayer: ExtendedLayerId,
    /// The object's (re)send events recorded in the *completed* prior temporal units
    /// (object-keyed, cross-extended-layer — § 7.3.8.6 models the sequence-header memory as
    /// a global `seq_header_id` namespace), snapshotted at reference time so a later resend
    /// cannot retroactively satisfy the reference. Their per-anchor visibility is resolved
    /// at completion.
    promoted_events: Vec<RapResendEvent>,
    /// The extended layers that (re)sent this object *earlier in this temporal unit*
    /// (before this reference, in-band order); empty if it was not resent before the
    /// reference. The before-reference resend counts when *any* of these senders is visible
    /// under the governing random access point. Their leading-ness / random-access-point-
    /// ness / § 7.4.6 sender-decodability is deferred to temporal-unit completion (see
    /// [`RapReplayTracker::complete_temporal_unit`]).
    this_tu_resend_xlayers: BTreeSet<ExtendedLayerId>,
    /// Byte offset of the referencing OBU, where the diagnostic is anchored.
    offset: ByteOffset,
}

/// § 7.3.8.1 random-access-point HLS availability replay tracker (AV2 v1.0.0
/// § 7.3.8.1, mirror `07-decoding-process.md` lines 685-693).
///
/// § 7.3.8.1 requires every referenced HLS OBU to remain available "if decoding process
/// starts at any random access point and drops any temporal units containing leading
/// frames" — the NOTE: HLS used at a random access point "need to be resent in the same
/// temporal unit (or be provided through external means)". The validator's linear
/// availability stores are monotonic, so a stream that sends an HLS OBU once before a
/// random access point and never resends it passes the linear `hls/unavailable-*` checks
/// while failing real random access. This tracker adds the replay dimension.
///
/// **Why temporal-unit-end resolution.** A temporal unit's § 7.4.1 random-access-point-
/// ness (a coded extended layer unit contains a CLK / OLK / RAS OBU) and its leading-
/// frame-ness (it contains a LEADING_* OBU) are only fully known once the temporal unit
/// ends. § 7.3.7 places global HLS before the frame OBUs of a temporal unit, so an object
/// (re)sent in a random access point's temporal unit is recorded before the frame
/// references in that unit; the reference resolution is nonetheless deferred to temporal-
/// unit completion (mirroring [`MsdoIdentityTracker`]) so the unit's random-access-point-
/// ness and leading-ness drive the verdict.
///
/// **Per-extended-layer random access (§ 7.4.6).** § 7.4 random access initiates *per
/// extended layer*: a temporal unit "may be a random access point for some extended
/// layers but not for others", and "the decoder shall not decode coded extended layer
/// units for an extended layer until a random access point for that extended layer is
/// encountered" (mirror `07-decoding-process.md` lines 1314-1318). So a CLK in extended
/// layer 0 makes the temporal unit a random access point for layer 0 *only* — a frame in
/// layer 1 still answers to layer 1's own most recent random access point. Random-access-
/// point-ness, leading-ness, and the governing anchor are therefore tracked per extended
/// layer (keyed by `obu_xlayer_id`); a [`GLOBAL_XLAYER_ID`] reference is governed by the
/// global anchor (the most recent random access point across *any* layer), since a
/// global-layer HLS OBU is decoded by whichever layer first random-accesses there.
///
/// **Anchor-relative visibility (the model).** A reference must remain available "if decoding
/// process starts at **any** random access point" (§ 7.3.8.1). So it is governed not by a
/// single anchor but by *every* random access point `R <= refTU` a decoder might start from:
/// every random access point of the reference's governing layer for a layer reference, or
/// every random access point across any layer for a [`GLOBAL_XLAYER_ID`] reference (a
/// global-layer HLS OBU is decoded by whichever layer first random-accesses there). The
/// reference is satisfied iff, for **every** such governing anchor `R`, some (re)send `S` of
/// the object is *visible under a decode that starts at R* (finding 2). The most recent anchor
/// alone is insufficient: a (re)send visible to a newer anchor can be invisible to an older one
/// (a clause-(a) resend in a temporal unit that also carries leading frames is its own start
/// for the newer anchor, but drops under the older anchor's start). A single global last-good
/// scalar cannot answer this either, because whether a (re)send is visible depends on *which*
/// random access point a decoder started from. The per-anchor predicate, evaluated against the
/// completed-temporal-unit facts (each temporal unit's leading-ness and per-layer random-
/// access-point-ness resolve at its end):
///
/// > `visible(S, R)` holds iff `S`'s sending layer is decoded under start-at-R (§ 7.4.6
/// > sender-decodability: the sender is [`GLOBAL_XLAYER_ID`] — decoded by whichever layer
/// > first random-accesses there — or the sending layer had a random access point at some
/// > temporal unit `T` with `R <= T <= S.tu`, so its coded extended layer units begin decoding
/// > by `S.tu`) AND either:
/// > - **(a)** `S.tu == R` (the random access point's own temporal unit is always
/// >   decoded — § 7.4.1 "Decoding can be correctly initiated at such a temporal unit"); OR
/// > - `S.tu > R` AND **(b)** `S`'s temporal unit carries no leading frame in any layer
/// >   (§ 7.3.8.1: a decode starting at `R` "drops any temporal units containing leading
/// >   frames", so a strictly-later leading temporal unit's sends are not decoded).
/// >
/// > Sender-decodability gates clause (a) too (finding 1): a (re)send in `R`'s own temporal
/// > unit carried by a *non-global* layer that has no random access point in that temporal unit
/// > is not decoded under start-at-R (§ 7.4.6). For `S.tu == R` the `[R, S.tu]` interval test
/// > reduces to "the sending layer random-accesses at `R`", which also subsumes the design
/// > sketch's "sender == the layer whose random access point `R` is" case.
///
/// **Soundness (never a false positive).** Only references that resolved *linearly* are
/// buffered, so the replay predicate is disjoint from the linear unavailability checks: a
/// reference with no availability at all is the linear check's job. A temporal unit whose
/// leading-ness is undecidable from OBU types alone never disqualifies a (re)send (the type-
/// detectable LEADING_* subset is a sound under-approximation — at worst a missed report,
/// never a false positive). Availability is tracked object-keyed (cross-extended-layer):
/// § 7.3.8.6 models the sequence-header memory as a global `seq_header_id` namespace, so a
/// *visible* (re)send in any decodable layer makes the object available — clause (c) keeps
/// this from over-counting a send in a layer the start-at-R decode never reaches.
///
/// **A reference whose own temporal unit drops is moot.** § 7.3.8.1 drops *whole* temporal
/// units containing leading frames. So a reference in a strictly-post-`R` temporal unit that
/// carries any leading frame is not decoded at all under start-at-R — its availability
/// requirement is moot and no diagnostic is emitted (the random access point's own temporal
/// unit, `reference_tu == R`, is never dropped — § 7.4.1).
///
/// **Leading-temporal-unit redefinition is an availability *non*-event (finding 4).** When an
/// object is available at the random access point and then *redefined* only in a later leading
/// temporal unit, the availability question this tracker answers is unchanged: the random-
/// access-point version is visible (clause (a)), so a later regular reference is *correctly*
/// satisfied — "invalidating" availability on a leading redefinition would be a false positive
/// (the object IS available at the random access point). § 7.4.4 ("Regular frames that follow
/// leading frames after the OLK temporal unit shall also not reference ... HLS OBUs that are
/// indicated in temporal units containing leading frames", mirror `07-decoding-process.md`
/// lines 1184-1185) is a *separate* content-identity divergence — sequential decoding would
/// use the leading-temporal-unit version while a random-access decode keeps the random-access-
/// point version — not an availability defect. Detecting it would require modelling each
/// (re)send's *content* (to tell a genuine redefinition from an identical leading re-send) plus
/// post-leading-regular-frame reference tracking, which is not yet modelled and is left as a
/// residual to avoid a false-positive-prone diagnostic.
// TODO(spec: AV2-7.3.8-HLS-AVAILABILITY): § 7.4.4 leading-temporal-unit content-identity
// divergence — a post-leading regular frame that references an HLS object redefined in a
// leading temporal unit is non-conformant even though the object is *available* at the random
// access point; modelling it needs per-resend content identity + post-leading reference
// tracking (currently a documented residual, not a diagnostic).
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
struct RapReplayTracker {
    /// Per object, every visible-candidate in-band (re)send event recorded in *completed*
    /// temporal units (object-keyed, cross-extended-layer — § 7.3.8.6 models the sequence-
    /// header memory as a global `seq_header_id` namespace). Anchor-relative visibility (see
    /// the type docs) replays these per reference against its governing random access point;
    /// a single scalar cannot, because a (re)send visible when starting at one random access
    /// point may be invisible when starting at an earlier one. Pruned of events older than
    /// every current anchor's floor.
    resend_events: BTreeMap<RapHlsKey, Vec<RapResendEvent>>,
    /// Objects (re)sent in the temporal unit currently being observed, mapped to the *set*
    /// of extended layers that sent each (eager, in-band order). Used both to snapshot the
    /// before-reference resends for the current unit and to append this unit's resend events
    /// into [`Self::resend_events`] at completion (whose visibility needs each sending
    /// layer's leading / random-access state). Cleared per unit. When an object is resent by
    /// several layers in one unit, *all* senders are retained: the object becomes available
    /// for a random access point if *any* of them is visible under that start (§ 7.3.8.1 is
    /// a per-object availability question, so one visible send suffices).
    resent_this_tu: BTreeMap<RapHlsKey, BTreeSet<ExtendedLayerId>>,
    /// References buffered in the temporal unit currently being observed, resolved at
    /// completion (see [`Self::complete_temporal_unit`]).
    pending_this_tu: Vec<RapPendingReference>,
    /// Extended layers for which the temporal unit currently being observed is a § 7.4.1
    /// random access point (a CLK / OLK / RAS OBU in that layer's coded extended layer
    /// unit). Resolved at completion.
    current_tu_rap_xlayers: BTreeSet<ExtendedLayerId>,
    /// Extended layers whose coded extended layer unit in the temporal unit currently
    /// being observed contains a LEADING_* frame OBU (§ 7.3.8.1: such units drop under
    /// random access, so their resends do not qualify — unless the unit is itself that
    /// layer's random access point).
    current_tu_leading_xlayers: BTreeSet<ExtendedLayerId>,
    /// Per extended layer, the temporal-unit index of its most recent random access point
    /// completed so far. Tracked for diagnostics/pruning context; § 7.3.8.1 satisfaction is
    /// resolved against *every* governing anchor (see [`Self::governing_rap_tus`] and
    /// [`Self::complete_temporal_unit`]), not just the most recent one, so this scalar is no
    /// longer the satisfaction anchor.
    most_recent_rap_tu: BTreeMap<ExtendedLayerId, u64>,
    /// The temporal-unit index of the most recent random access point across *any*
    /// extended layer, or `None` before any random access point. Retained as the most-recent
    /// global anchor for context; like [`Self::most_recent_rap_tu`] it is not the sole
    /// satisfaction anchor — [`GLOBAL_XLAYER_ID`] references must be satisfied at *every*
    /// random access point a decoder might start from (every entry of [`Self::rap_history_any`]
    /// at or before the reference), per § 7.3.8.1's "any random access point".
    most_recent_rap_tu_any: Option<u64>,
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
    rap_history: BTreeMap<ExtendedLayerId, BTreeMap<u64, bool>>,
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
    rap_history_any: BTreeMap<u64, bool>,
    /// Already-emitted `(object, random-access-point temporal unit)` findings, so one
    /// dangling object reports once per random access point even across several
    /// referencing frames in or after it (proposal dedup requirement).
    emitted: BTreeSet<(RapHlsKey, u64)>,
    /// A permanently-empty random-access-point history, returned by [`Self::governing_rap_tus`]
    /// for a layer with no recorded random access point. Held as a field (rather than a
    /// per-call temporary) so the returned `range(..)` iterator can borrow it.
    empty_rap_history: BTreeMap<u64, bool>,
}

impl RapReplayTracker {
    /// Records an in-band (re)send of `key` by extended layer `xlayer` in the temporal
    /// unit currently being observed (§ 7.3.8.1 / § 7.3.7: global HLS precedes the unit's
    /// frame OBUs, so this runs before any reference in the same unit). The sending layer
    /// is retained so its leading / random-access qualification can be resolved at
    /// completion.
    fn note_resend(&mut self, key: RapHlsKey, xlayer: ExtendedLayerId) {
        // Accumulate every sender in this unit (not last-writer-wins): a qualifying resend
        // must not be lost when a later non-qualifying (leading, non-random-access) resend
        // of the same object follows it in the same unit — § 7.3.8.1 availability holds if
        // *any* same-unit send qualifies.
        self.resent_this_tu.entry(key).or_default().insert(xlayer);
    }

    /// Marks the temporal unit currently being observed as a § 7.4.1 random access point
    /// for extended layer `xlayer` (a CLK / OLK / RAS OBU in that layer's coded extended
    /// layer unit).
    fn note_random_access_point(&mut self, xlayer: ExtendedLayerId) {
        self.current_tu_rap_xlayers.insert(xlayer);
    }

    /// Marks extended layer `xlayer`'s coded extended layer unit in the temporal unit
    /// currently being observed as containing a LEADING_* frame OBU (§ 7.3.8.1).
    fn note_leading_frame(&mut self, xlayer: ExtendedLayerId) {
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
    fn note_reference(
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
    fn governing_rap_tus(
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
    fn sender_decodable_at(
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
    fn event_visible_at(&self, event: RapResendEvent, rap_tu: u64) -> bool {
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
    fn anchor_floor(&self) -> Option<u64> {
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
    fn complete_temporal_unit(&mut self, tu_index: u64) -> Vec<(RapHlsKey, Diagnostic)> {
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
fn rap_replay_unavailable(key: RapHlsKey, rap_tu: u64, offset: ByteOffset) -> Diagnostic {
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
fn rap_replay_suppressed_by_external_hls(key: RapHlsKey, external_hls: &ExternalHlsMode) -> bool {
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

/// Stateful § 5.6 MSDO observer (AV2 v1.0.0 § 5.6 / § 7.3.2).
///
/// The validator otherwise touches `OBU_MSDO` only for temporal-unit ordering
/// (`is_global_hls_prefix_obu`); this observer parses the payload and remembers the
/// last-seen MSDO's § 7.3.2 condition-2 key fields so the [`CmvsTracker`] can detect
/// the "differs from the corresponding value in the previous OBU_MSDO" condition. It
/// holds no diagnostics of its own.
#[derive(Debug, Default)]
struct MsdoObserver {
    /// The § 7.3.2 condition-2 key fields of the most recently observed MSDO, or
    /// `None` until the first MSDO is seen.
    last: Option<MsdoKeyFields>,
}

impl MsdoObserver {
    /// Records one parsed MSDO and reports how it relates to the previous one
    /// (AV2 v1.0.0 § 7.3.2 condition 2).
    fn observe(&mut self, msdo: &MultistreamDecoderOperation) -> MsdoObservation {
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

/// Three-state § 7.3.2 coded-multistream-video-sequence (CMVS) membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CmvsState {
    /// Definitively not inside a CMVS.
    #[default]
    Outside,
    /// Definitively inside a CMVS.
    Inside,
    /// Membership cannot be derived soundly from the modeled state; checks gated on
    /// the CMVS do not fire here (conservative under-approximation).
    Unknown,
}

/// Per-temporal-unit facts the [`CmvsTracker`] accumulates while observing a temporal
/// unit, then evaluates against the § 7.3.2 begin/end conditions when the temporal
/// unit completes.
#[derive(Debug, Default)]
struct CmvsTuFacts {
    /// The temporal unit contains an `OBU_CLOSED_LOOP_KEY` for at least one extended
    /// layer (AV2 § 7.3.2 begin: "begins at a temporal unit that contains an OBU with
    /// obu_type equal to OBU_CLOSED_LOOP_KEY for at least one extended layer"; § 7.3.6:
    /// such a temporal unit begins a new coded video sequence for that extended layer).
    has_clk: bool,
    /// The MSDO observation for this temporal unit, or `None` when no MSDO is present
    /// (AV2 § 7.3.2 conditions 1 and 2 turn on MSDO presence and key-field change).
    msdo: Option<MsdoObservation>,
    /// A global layer configuration record OBU is present in this temporal unit. Used
    /// only to drive the conservative [`CmvsState::Unknown`] routing of the § 7.3.2
    /// condition-3 / end-condition-2 paths whose precise truth needs § 7.3.8
    /// activation state that is not yet modeled.
    global_lcr_present: bool,
}

/// Minimal three-state § 7.3.2 CMVS begin/end tracker (AV2 v1.0.0 § 7.3.2).
///
/// The tracker accumulates per-temporal-unit facts ([`CmvsTuFacts`]) as OBUs are
/// observed and applies the § 7.3.2 begin/end conditions when a temporal unit
/// completes (at the global `OBU_TEMPORAL_DELIMITER` that ends it, or at the end of
/// the bitstream). It exposes three states ([`CmvsState`]); checks gated on the CMVS
/// (e.g. § 6.4.1 `monotonic_output_order_flag` agreement) fire only in
/// [`CmvsState::Inside`]. [`Self::state`] returns the membership *effective for OBUs
/// observed so far in the current temporal unit*: end condition 2 (an MSDO-less CLK
/// temporal unit that begins a new coded video sequence) is applied as soon as it is
/// decidable — at the CLK, since § 7.3.7 places the at-most-one MSDO before every coded
/// extended layer unit — so the stale `Inside` of the previous temporal unit does not
/// leak into activation-time checks for OBUs that already sit outside the CMVS.
/// The tracker is a sound under-approximation: every transition whose truth cannot be
/// derived from the modeled state (notably anything depending on exact § 7.3.8
/// global-LCR activation) routes to [`CmvsState::Unknown`], never to a spurious
/// `Inside`/`Outside`.
///
/// Each transition below carries the exact § 7.3.2 sentence it implements, because no
/// real multistream conformance vectors exist yet and the spec text is the only
/// oracle.
#[derive(Debug, Default)]
struct CmvsTracker {
    /// Current CMVS membership.
    state: CmvsState,
    /// Facts accumulated for the temporal unit currently being observed.
    current_tu: CmvsTuFacts,
    /// § 6.4.1 monotonic-output-order disagreements emitted at sequence-header
    /// observation time while this temporal unit's CMVS membership is only
    /// *provisionally* [`CmvsState::Inside`] (committed `Inside`, but no CLK observed
    /// yet, so a later MSDO-less CLK could end the CMVS for this temporal unit —
    /// AV2 § 7.3.2 end condition 2, mirror `07-decoding-process.md` lines 335-341).
    /// They are flushed when the temporal unit completes ([`Self::complete_temporal_unit`]):
    /// emitted when the completed temporal unit is definitively `Inside`, dropped when a
    /// CLK turned it `Outside`/`Unknown`. Deferring avoids a false positive on the
    /// § 7.3.6-permitted same-CVS redefinition that immediately precedes the CLK that
    /// begins the new coded video sequence (mirror `07-decoding-process.md` lines
    /// 608-611).
    pending_monotonic: Vec<Diagnostic>,
    /// Facts of the just-completed temporal unit, for the § 7.3.2 boundary-set-identity
    /// check resolved at the same temporal-unit-completion point (see
    /// [`ValidatorContext::resolve_deferred_cmvs_boundary`]). Set by
    /// [`Self::complete_temporal_unit`]; `None` before the first completion.
    last_completed: Option<CmvsCompletedFacts>,
    /// The [`CvsTracker::tu_index`] of the temporal unit at which the *current* coded
    /// multistream video sequence began (§ 7.3.2), or `None` when no CMVS is active. A CMVS
    /// spans a contiguous run of temporal units; this records the index of its first one,
    /// captured by [`Self::complete_temporal_unit`] when a begin condition fires, carried
    /// across continuation temporal units, and cleared when the CMVS ends. Everything
    /// observed at `tu_index >= cmvs_start_tu_index` lies within the current CMVS — the
    /// window the § 6.8.2 agreement / DOH requirement and the § 6.6 MSDO DOH requirement
    /// scope their per-layer evaluation to (codex findings 3393129738 / 3393129745). A
    /// temporal unit is the atomic § 7.3.6 attribution unit (a CLK-bearing TU and all its
    /// pre-CLK HLS belong to the same new coded video sequence), so a TU-index lower bound
    /// avoids the pre-CLK / post-CLK generation ambiguity and is a sound
    /// under-approximation of CMVS membership.
    cmvs_start_tu_index: Option<u64>,
    /// The [`CvsTracker::tu_index`] of the just-completed temporal unit itself, captured by
    /// [`Self::complete_temporal_unit`]. The § 7.3.2 boundary-set-identity check
    /// (`cmvs/boundary-set-mismatch`) needs the BOUNDARY temporal unit's own index — not the
    /// CMVS-window start — because end condition 2's divergence turns on whether THAT temporal
    /// unit "has an activated global layer configuration record", a property of the boundary
    /// temporal unit alone (codex finding 3393274375). `None` before the first completion.
    last_completed_tu_index: Option<u64>,
}

/// The § 7.3.2 facts of a just-completed temporal unit, captured by
/// [`CmvsTracker::complete_temporal_unit`] for the boundary-set-identity check.
#[derive(Debug, Clone, Copy)]
struct CmvsCompletedFacts {
    /// The committed CMVS membership *before* this temporal unit's begin/end conditions
    /// were applied — i.e. whether a CMVS was active when the temporal unit started.
    was_inside_before: bool,
    /// The temporal unit contained an `OBU_CLOSED_LOOP_KEY` (§ 7.3.2 / § 7.3.6: begins a
    /// new coded video sequence for at least one extended layer).
    has_clk: bool,
    /// An `OBU_MSDO` was present in the temporal unit.
    msdo_present: bool,
    /// A global layer configuration record OBU was present in the temporal unit.
    global_lcr_present: bool,
}

/// The disposition of the § 6.4.1 cross-layer monotonic-output-order agreement check at
/// a sequence-header observation, given the § 7.3.2 CMVS tracker state; see
/// [`CmvsTracker::monotonic_verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonotonicVerdict {
    /// The tracker is not definitively inside a CMVS for the OBUs observed so far; the
    /// check does not fire.
    Skip,
    /// The tracker is definitively inside a CMVS; a disagreement is emitted eagerly.
    EmitNow,
    /// The tracker is only *provisionally* inside a CMVS (committed `Inside`, no CLK
    /// observed yet in this temporal unit, so a later MSDO-less CLK could end the
    /// CMVS); a disagreement is deferred to the temporal-unit flush.
    Defer,
}

impl CmvsTracker {
    /// Returns the § 7.3.2 CMVS membership *effective for OBUs observed so far in the
    /// current temporal unit*, applying the begin and end conditions as soon as they are
    /// decidable rather than only at temporal-unit completion.
    ///
    /// Consumed by the § 6.4.1 cross-xlayer `monotonic_output_order_flag` agreement
    /// check, which fires only in [`CmvsState::Inside`].
    ///
    /// The committed `self.state` reflects the previous completed temporal unit, so on its
    /// own it is stale for the temporal unit currently being observed in both directions.
    /// § 7.3.7 constrains the at-most-one MSDO of a temporal unit to precede every coded
    /// extended layer unit, so by the time any frame activation in the temporal unit runs,
    /// MSDO presence for the temporal unit is already final; the temporal unit's begin/end
    /// membership is therefore decidable at activation time once a CLK has been observed (a
    /// CLK lives inside a coded extended layer unit, so the MSDO, if any, was already seen).
    /// These two adjustments are mutually exclusive on MSDO presence, so they never both
    /// apply:
    ///
    /// - **Begin (CLK + MSDO present).** This temporal unit "contains an OBU with obu_type
    ///   equal to OBU_CLOSED_LOOP_KEY for at least one extended layer" and "an OBU with
    ///   obu_type equal to OBU_MSDO is present", so it begins a CMVS (§ 7.3.2 begin
    ///   condition 1 from a committed `Outside`, begin condition 2 / continuation from a
    ///   committed `Inside`). The result is [`CmvsState::Inside`] under every committed
    ///   state: from `Outside` (begin condition 1); from `Inside` (a changed MSDO begins a
    ///   new CMVS, an unchanged MSDO continues the active one — Inside either way); and from
    ///   `Unknown`, where both resolutions of the unknown (real `Outside` → begin condition
    ///   1, real `Inside` → continuation/begin condition 2) yield Inside. Without this
    ///   adjustment the committed `Outside`/`Unknown` would leak into activation-time checks
    ///   for OBUs that already sit inside the CMVS opened by this temporal unit.
    /// - **End (committed `Inside` + CLK + no MSDO).** This temporal unit "begins a new
    ///   coded video sequence for at least one extended layer but does not contain an OBU
    ///   with obu_type equal to OBU_MSDO" (§ 7.3.2 end condition 2), so it ends the active
    ///   CMVS — to [`CmvsState::Outside`], or to [`CmvsState::Unknown`] when a global LCR is
    ///   present (whose activation, and thus whether this is really an end, is not modeled).
    ///   Without this adjustment the stale `Inside` would leak into activation-time checks
    ///   for OBUs that already sit outside the CMVS.
    ///
    /// When no CLK has been observed yet in the temporal unit, neither adjustment applies
    /// and the committed state is returned unchanged.
    fn state(&self) -> CmvsState {
        if self.current_tu.has_clk {
            // § 7.3.7: an MSDO, if any, precedes the CLK's coded extended layer unit, so
            // MSDO presence for this temporal unit is already final at activation time.
            if self.current_tu.msdo.is_some() {
                // § 7.3.2 begin: a CLK temporal unit with an MSDO present is inside the
                // CMVS it opens (or continues), regardless of the committed state.
                return CmvsState::Inside;
            }
            if matches!(self.state, CmvsState::Inside) {
                // § 7.3.2 end condition 2 is already decidable: an MSDO-less CLK temporal
                // unit begins a new coded video sequence but carries no OBU_MSDO.
                if self.current_tu.global_lcr_present {
                    return CmvsState::Unknown;
                }
                return CmvsState::Outside;
            }
        }
        self.state
    }

    /// The *committed* § 7.3.2 CMVS membership — the membership of the most recently
    /// completed temporal unit, ignoring any partial facts of the temporal unit currently
    /// being observed.
    ///
    /// After [`Self::complete_temporal_unit`] runs at a temporal-unit boundary, this is
    /// the just-completed temporal unit's final membership (the per-temporal-unit facts
    /// have been reset, so [`Self::state`] also returns the committed value — but this
    /// accessor names the intent). The deferred § 6.6 `msdo/doh-constraint-required`
    /// evaluation queries it at boundary resolution to decide whether the just-completed
    /// temporal unit's frame-confirmed activations sit inside a CMVS (see
    /// [`ValidatorContext::resolve_deferred_doh_constraint`]).
    fn committed_inside(&self) -> bool {
        matches!(self.state, CmvsState::Inside)
    }

    /// The disposition of the § 6.4.1 cross-layer monotonic-output-order agreement check
    /// at a sequence-header observation, given the OBUs observed so far in the current
    /// temporal unit.
    ///
    /// The check fires only in [`CmvsState::Inside`]. When [`Self::state`] is `Inside`
    /// *and a CLK has already been observed* this temporal unit, the membership is final
    /// (§ 7.3.7 places the at-most-one MSDO before every coded extended layer unit, so a
    /// CLK temporal unit's begin/end membership is decided at the CLK), so a disagreement
    /// is emitted eagerly ([`MonotonicVerdict::EmitNow`]). When `state()` is `Inside` but
    /// *no CLK has been observed yet*, the verdict is only provisional: a later MSDO-less
    /// CLK in this temporal unit would end the CMVS (§ 7.3.2 end condition 2), placing a
    /// header observed before it — canonically a § 7.3.6-permitted same-CVS redefinition
    /// immediately preceding the CLK that begins the new coded video sequence (mirror
    /// `07-decoding-process.md` lines 608-611) — outside the CMVS. A disagreement is then
    /// deferred ([`MonotonicVerdict::Defer`]) and resolved at temporal-unit completion.
    /// Any non-`Inside` state skips the check ([`MonotonicVerdict::Skip`]).
    fn monotonic_verdict(&self) -> MonotonicVerdict {
        match self.state() {
            CmvsState::Inside if self.current_tu.has_clk => MonotonicVerdict::EmitNow,
            CmvsState::Inside => MonotonicVerdict::Defer,
            CmvsState::Outside | CmvsState::Unknown => MonotonicVerdict::Skip,
        }
    }

    /// Queues a provisional-`Inside` § 6.4.1 monotonic-output-order disagreement for
    /// resolution at temporal-unit completion (see [`Self::monotonic_verdict`] /
    /// [`Self::complete_temporal_unit`]).
    fn queue_provisional_monotonic(&mut self, diagnostic: Diagnostic) {
        self.pending_monotonic.push(diagnostic);
    }

    /// Records that the temporal unit being observed contains an
    /// `OBU_CLOSED_LOOP_KEY` for some extended layer (AV2 § 7.3.2 / § 7.3.6).
    fn note_clk(&mut self) {
        self.current_tu.has_clk = true;
    }

    /// Records the MSDO observation for the temporal unit being observed
    /// (AV2 § 7.3.2 conditions 1 and 2). A temporal unit carries at most one MSDO
    /// (§ 7.3.7), so this is set at most once per temporal unit.
    fn note_msdo(&mut self, observation: MsdoObservation) {
        self.current_tu.msdo = Some(observation);
    }

    /// Records that a global layer configuration record OBU is present in the temporal
    /// unit being observed (AV2 § 7.3.2 condition 3 / end condition 2). Whether it is
    /// *activated* needs § 7.3.8 activation state that is not modeled; the tracker
    /// therefore treats presence as an "activation cannot be ruled out" signal and
    /// routes the affected transitions to [`CmvsState::Unknown`].
    fn note_global_lcr_present(&mut self) {
        self.current_tu.global_lcr_present = true;
    }

    /// The [`CvsTracker::tu_index`] of the temporal unit at which the current coded
    /// multistream video sequence began, or `None` when no CMVS is active. Observations
    /// (frame-confirmed activations, global-LCR OBUs) tagged with a `tu_index` at or after
    /// this value lie within the current CMVS; earlier ones belong to a prior CMVS and are
    /// excluded from the § 6.8.2 / § 6.6 DOH evaluations (codex findings 3393129738 /
    /// 3393129745).
    fn current_cmvs_start_tu_index(&self) -> Option<u64> {
        self.cmvs_start_tu_index
    }

    /// The [`CvsTracker::tu_index`] of the just-completed (boundary) temporal unit, for the
    /// § 7.3.2 boundary-set check's "the BOUNDARY temporal unit has an activated global LCR"
    /// scoping (end condition 2 divergence). `None` before the first completion.
    fn last_completed_tu_index(&self) -> Option<u64> {
        self.last_completed_tu_index
    }

    /// Completes the temporal unit being observed, applying the § 7.3.2 begin/end
    /// conditions, then resets the per-temporal-unit facts for the next one. Called at
    /// each temporal-unit boundary and at the end of the bitstream. `completed_tu_index` is
    /// the [`CvsTracker::tu_index`] of the just-completed temporal unit (captured before
    /// `advance_temporal_unit` bumps it), used to stamp the CMVS-window start when a begin
    /// condition fires.
    ///
    /// Provisional-`Inside` § 6.4.1 monotonic disagreements deferred during this temporal
    /// unit ([`Self::queue_provisional_monotonic`]) are resolved here against the
    /// temporal unit's final membership: emitted when the completed temporal unit is
    /// definitively [`CmvsState::Inside`], dropped when a CLK ended the CMVS
    /// ([`CmvsState::Outside`]/[`CmvsState::Unknown`], § 7.3.2 end condition 2).
    fn complete_temporal_unit(&mut self, completed_tu_index: u64, report: &mut ValidationReport) {
        let facts = std::mem::take(&mut self.current_tu);
        let was_inside_before = matches!(self.state, CmvsState::Inside);
        self.last_completed = Some(CmvsCompletedFacts {
            was_inside_before,
            has_clk: facts.has_clk,
            msdo_present: facts.msdo.is_some(),
            global_lcr_present: facts.global_lcr_present,
        });
        let (next, window_action) = self.next_state(&facts);
        self.state = next;
        // The boundary temporal unit's own index, for the § 7.3.2 boundary-set check's
        // boundary-TU-scoped activated-global-LCR resolution (codex finding 3393274375).
        self.last_completed_tu_index = Some(completed_tu_index);
        // § 7.3.2 window bookkeeping: the live window for the *next* temporal unit. An Open
        // starts a fresh window at this temporal unit's index; a Keep carries the existing
        // start; a Close clears
        // it. Capturing this lower bound at the authoritative temporal-unit-completion
        // resolution lets the deferred § 6.8.2 / § 6.6 evaluations scope their per-layer
        // loops to observations made at or after this temporal unit (the current CMVS).
        match window_action {
            CmvsWindowAction::Open => self.cmvs_start_tu_index = Some(completed_tu_index),
            // Seed the start on the first-ever continuation if it was somehow never set.
            CmvsWindowAction::Keep => {
                self.cmvs_start_tu_index.get_or_insert(completed_tu_index);
            }
            CmvsWindowAction::Close => self.cmvs_start_tu_index = None,
        }
        let pending = std::mem::take(&mut self.pending_monotonic);
        if matches!(self.state, CmvsState::Inside) {
            for diagnostic in pending {
                report.push(diagnostic);
            }
        }
    }

    /// Whether the just-completed temporal unit is the § 7.3.2 boundary-set-identity
    /// divergence *candidate*: a temporal unit that, while a CMVS was active, begins a new
    /// coded video sequence (has a CLK) with no OBU_MSDO present but a global layer
    /// configuration record present. Under the MSDO-alone boundary rules such a temporal
    /// unit ENDS the CMVS (§ 7.3.2 end condition 2 fires — "does not contain an OBU_MSDO
    /// and does not have an activated global LCR", and there is no MSDO); under the
    /// MSDO+activated-global-LCR rules it does NOT end (the activated global LCR makes end
    /// condition 2 false), so the two boundary sets diverge here. Whether the global LCR is
    /// genuinely *activated* (making the divergence real and decidable) is confirmed by the
    /// caller against the association chain; this only reports the structural candidate.
    fn last_completed_is_boundary_divergence_candidate(&self) -> bool {
        self.last_completed.is_some_and(|f| {
            f.was_inside_before && f.has_clk && !f.msdo_present && f.global_lcr_present
        })
    }

    /// Computes the § 7.3.2 CMVS state after a completed temporal unit with `facts`,
    /// given the current `self.state`, plus the [`CmvsWindowAction`] for the CMVS-window
    /// scoping. Begin conditions are evaluated before end conditions because a temporal
    /// unit that begins a new CMVS is the *earliest* end of the current one (§ 7.3.2 end
    /// condition 1). The window action drives the start-of-window bookkeeping in
    /// [`Self::complete_temporal_unit`].
    ///
    /// The window opens (a fresh lower bound) on *any* begin condition — including begin
    /// condition 3 (a CLK temporal unit activating a global LCR with no MSDO), which the
    /// membership state routes to [`CmvsState::Unknown`] because § 7.3.8 activation is not
    /// modeled. Opening the window there is sound: the § 6.8.2 DOH requirement and agreement
    /// resolve "an activated global LCR" from the association chain (`activated_global_lcr`),
    /// which IS decidable, so an LCR-only CMVS still needs a window for those checks to scope
    /// to; if the chain finds no activated global LCR, nothing fires regardless of the window.
    fn next_state(&self, facts: &CmvsTuFacts) -> (CmvsState, CmvsWindowAction) {
        // AV2 § 7.3.2: "A coded multistream video sequence begins at a temporal unit
        // that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY for at least
        // one extended layer and satisfies one of the following conditions". Without a
        // CLK in the temporal unit, no begin condition can fire.
        if facts.has_clk {
            let currently_active = matches!(self.state, CmvsState::Inside);
            match facts.msdo {
                // AV2 § 7.3.2 begin condition 1: "No coded multistream video sequence is
                // currently active and an OBU with obu_type equal to OBU_MSDO is present."
                Some(_) if !currently_active => {
                    return (CmvsState::Inside, CmvsWindowAction::Open);
                }
                // AV2 § 7.3.2 begin condition 2: "A coded multistream video sequence is
                // currently active, an OBU with obu_type equal to OBU_MSDO is present,
                // and the value of multistream_profile_idc, multistream_level_idx,
                // multistream_tier, num_streams_minus_2, multistream_even_allocation_flag,
                // or multistream_large_picture_idc differs from the corresponding value
                // in the previous OBU_MSDO." A changed MSDO begins a new CMVS (which is
                // still Inside); an unchanged MSDO leaves the active CMVS intact.
                Some(MsdoObservation::Changed) => {
                    return (CmvsState::Inside, CmvsWindowAction::Open);
                }
                Some(MsdoObservation::First | MsdoObservation::Unchanged) => {
                    // Active CMVS (the `!currently_active` arm above already handled the
                    // inactive case for any MSDO), MSDO present but unchanged: this temporal
                    // unit neither begins a new CMVS (condition 2 needs a change) nor ends
                    // the current one (end condition 2 excludes an MSDO-accompanied CVS
                    // start), so the CMVS continues.
                    return (CmvsState::Inside, CmvsWindowAction::Keep);
                }
                None => {
                    // AV2 § 7.3.2 begin condition 3: "No coded multistream video sequence
                    // is currently active and a global layer configuration record is
                    // activated." Exact § 7.3.8 global-LCR activation is not modeled, so the
                    // membership cannot be soundly classified Inside — route to Unknown — but
                    // the window opens at this temporal unit so the chain-decidable LCR-only
                    // § 6.8.2 DOH/agreement checks can scope to it.
                    if facts.global_lcr_present && !currently_active {
                        return (CmvsState::Unknown, CmvsWindowAction::Open);
                    }
                }
            }
        }

        // AV2 § 7.3.2: "A coded multistream video sequence ends at the earliest of:"
        // (begin conditions above already handled end condition 1, "A temporal unit
        // that begins a new coded multistream video sequence as defined above").
        if matches!(self.state, CmvsState::Inside) {
            // AV2 § 7.3.2 end condition 2: "A temporal unit that begins a new coded
            // video sequence for at least one extended layer but does not contain an OBU
            // with obu_type equal to OBU_MSDO and does not have an activated global layer
            // configuration record." A CLK temporal unit (§ 7.3.6: begins a new coded
            // video sequence for its extended layer) with no MSDO ends the CMVS — unless
            // a global LCR is present, whose activation (and thus whether this is really
            // an end) is not modeled, so route that ambiguous case to Unknown.
            if facts.has_clk && facts.msdo.is_none() {
                if facts.global_lcr_present {
                    return (CmvsState::Unknown, CmvsWindowAction::Close);
                }
                return (CmvsState::Outside, CmvsWindowAction::Close);
            }
            // Otherwise the active CMVS continues across this temporal unit.
            return (CmvsState::Inside, CmvsWindowAction::Keep);
        }

        // No begin condition fired and the state is not Inside (so this temporal unit
        // contains no CLK — a CLK with an Inside committed state is handled by the
        // end-condition block above, and a CLK from Outside/Unknown would have matched a
        // begin arm). § 7.3.2 end conditions 1 and 2 both require a temporal unit that
        // "begins a new coded video sequence" (a CLK, § 7.3.6); with no CLK, NO end
        // condition can fire here, so an active window must be carried, not closed.
        //
        // - `Unknown` with an open window is an LCR-only CMVS (opened via begin condition 3,
        //   which the membership router cannot soundly classify Inside) whose end is still
        //   undecided: a non-CLK temporal unit cannot end it, so Keep preserves its window
        //   for the chain-decidable § 6.8.2 LCR-DOH / agreement checks to scope to later
        //   frame-confirmed activations (codex finding 3393274378). Without this the window
        //   closed prematurely and those later activations were skipped.
        // - `Outside`, or an `Unknown` whose window was already cleared (e.g. a divergence
        //   candidate that ended the CMVS at line ~3024 with Close), has no live window;
        //   Close keeps it cleared and avoids `complete_temporal_unit`'s Keep `get_or_insert`
        //   seeding a bogus window at this non-CLK temporal unit.
        if matches!(self.state, CmvsState::Unknown) && self.cmvs_start_tu_index.is_some() {
            return (self.state, CmvsWindowAction::Keep);
        }
        (self.state, CmvsWindowAction::Close)
    }
}

/// The § 7.3.2 CMVS-window action for a completed temporal unit, computed alongside the
/// membership state by [`CmvsTracker::next_state`] and applied by
/// [`CmvsTracker::complete_temporal_unit`]. The window is the [`CvsTracker::tu_index`]
/// lower bound the § 6.8.2 / § 6.6 deferred checks scope their per-layer loops to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmvsWindowAction {
    /// A begin condition (1, 2, or 3) fired: open a fresh window at this temporal unit.
    Open,
    /// A continuation: keep the existing window start.
    Keep,
    /// An end condition / undecidable carry: close the window for the next temporal unit.
    Close,
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

    /// Maintains the § 7.23 reference-frame buffer state for a frame-bearing `obu`.
    ///
    /// § 7.23 runs at `decode_frame_wrapup`, the final step of decoding a frame, AFTER the
    /// frame is decoded. To keep later frames' reference checks consistent with that
    /// ordering, each frame's § 7.23 update is *deferred* in [`Self::pending_ref_update`]
    /// and committed at the next frame's coded-frame boundary (or the end-of-bitstream
    /// flush). The segmenter's `boundary` is the coded-frame-unit authority:
    ///
    /// - [`FrameBoundary::OpensNewUnit`]: this OBU opens a NEW coded frame. The previous
    ///   frame is complete, so commit its pending update FIRST (its decode finished),
    ///   then run this frame's reference checks against the post-update buffer, then
    ///   record this frame's own pending update.
    /// - [`FrameBoundary::ContinuesUnit`]: a non-first tile group of the SAME coded
    ///   frame. The frame's update was already derived from its first tile group; nothing
    ///   to do (no double-update, no premature commit).
    /// - [`FrameBoundary::Ambiguous`]: an unreadable frame delimiter — the OBU may open a
    ///   new coded frame or continue one. Commit any pending update (the prior frame is
    ///   done either way) and poison ALL slots: an ambiguous boundary makes this frame's
    ///   refresh effect on the buffer unknowable (the Unknown invariant).
    ///
    /// A `None` boundary is a global frame-bearing OBU (the segmenter ignores globals);
    /// such an OBU is invalid (diagnosed elsewhere) and not part of any coded frame unit,
    /// so it neither commits nor produces a reference-state update.
    fn observe_reference_state(
        &mut self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        boundary: Option<FrameBoundary>,
        report: &mut ValidationReport,
    ) {
        let Some(boundary) = boundary else {
            return;
        };
        match boundary {
            FrameBoundary::ContinuesUnit => {
                // Same coded frame as its first tile group; its pending update already
                // captured the §7.23 effect. Nothing to commit or re-derive.
            }
            FrameBoundary::OpensNewUnit => {
                // The previous coded frame completed: commit its deferred §7.23 update so
                // this frame's reference checks see the post-decode buffer.
                self.commit_pending_ref_update();
                // Run this frame's reference-state checks against the committed buffer
                // (the §6.17.2 show-existing-frame slot-validity diagnostic).
                self.reference_state_checks(obu, first_picture_in_tu, report);
                // Derive and stage this frame's own §7.23 update (committed at the NEXT
                // frame boundary or the end-of-bitstream flush).
                let update = self.derive_ref_update(obu, first_picture_in_tu);
                self.pending_ref_update = Some((obu.header.extended_layer_id, update));
            }
            FrameBoundary::Ambiguous => {
                // The prior frame is done; commit its update. This frame's own refresh
                // effect is unknowable, so stage a poison-all (no reference checks fire —
                // a poisoned buffer proves nothing).
                self.commit_pending_ref_update();
                self.pending_ref_update =
                    Some((obu.header.extended_layer_id, FrameRefUpdate::PoisonAll));
            }
        }
    }

    /// Commits the deferred § 7.23 update (if any) into the reference-state tracker. Used
    /// at each frame boundary that closes the previous coded frame and at the
    /// end-of-bitstream flush (the final frame has no following delimiter).
    fn commit_pending_ref_update(&mut self) {
        if let Some((xlayer, update)) = self.pending_ref_update.take() {
            self.reference_state.apply(xlayer, update);
        }
    }

    /// Derives the grounded § 7.23 [`FrameRefUpdate`] for a frame-bearing `obu` from its
    /// parsed core, honestly poisoning when the refresh mask / frame type / dims / order
    /// hint cannot be grounded.
    ///
    /// - A show-existing-frame sets `refresh_frame_flags = 0` (§ 5.18.2 :4180), so it
    ///   updates no slot ([`FrameRefUpdate::SefNoUpdate`]).
    /// - A CLK that starts a new CVS (`OBU_CLOSED_LOOP_KEY && FirstPictureInTU`) resets
    ///   `RefValid[i] = 0` over `0..NumRefFrames` (§ 5.18.2 :4449-4455) then applies its
    ///   own refresh ([`FrameRefUpdate::ClkReset`]).
    /// - Any other frame whose `refresh_frame_flags`, `frame_type`, dims, and order hint
    ///   all parsed applies the § 7.23 update with the key/switch `first` rule.
    /// - Otherwise (the core did not resolve, an inter/TIP/bridge path, a truncation, or
    ///   any missing fact) the mask could touch any slot, so poison all
    ///   ([`FrameRefUpdate::PoisonAll`]).
    fn derive_ref_update(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> FrameRefUpdate {
        // The core must resolve to the active (== referenced) sequence header, exactly as
        // the output-class / order-hint derivation requires — otherwise the parsed fields
        // were read against a stale header and cannot be trusted (the same guard
        // `frame_celu_facts` uses).
        let Some(core) = self.frame_core_against_referenced_header(obu, first_picture_in_tu) else {
            return FrameRefUpdate::PoisonAll;
        };

        // A show-existing-frame updates no slot (§ 5.18.2 :4180).
        if core.show_existing_frame == Some(true) {
            return FrameRefUpdate::SefNoUpdate;
        }

        // Every grounded update needs the refresh mask, the frame type (for the §7.23
        // RefValid `first` rule), and the stored facts (OrderHint + dims). Any missing
        // fact poisons (the mask could refresh any slot, and a partial store is a guess).
        let (Some(refresh_frame_flags), Some(frame_type)) =
            (core.refresh_frame_flags, core.frame_type)
        else {
            return FrameRefUpdate::PoisonAll;
        };
        let Some(facts) = slot_facts(
            core.order_hint_lsb,
            core.frame_size.map(|size| size.width),
            core.frame_size.map(|size| size.height),
        ) else {
            return FrameRefUpdate::PoisonAll;
        };

        // The §5.18.2 CLK reset (`OBU_CLOSED_LOOP_KEY && FirstPictureInTU`) clears
        // RefValid[i] over 0..NumRefFrames before the refresh (mirror :4449-4455). The
        // core records `starts_cvs` for exactly this condition.
        if core.starts_cvs && obu.header.obu_type == ObuType::ClosedLoopKey {
            let num_ref_frames = self
                .active_sequence_by_xlayer
                .get(&obu.header.extended_layer_id)
                .and_then(|seq_id| self.sequence_headers.get(seq_id))
                .and_then(|seq| seq.inter.as_ref())
                .map_or(NUM_REF_FRAMES, |inter| usize::from(inter.num_ref_frames));
            return FrameRefUpdate::ClkReset {
                num_ref_frames,
                refresh_frame_flags,
                facts,
            };
        }

        FrameRefUpdate::Refresh {
            refresh_frame_flags,
            is_key_or_switch: is_key_or_switch(frame_type),
            facts,
        }
    }

    /// Emits the reference-state-gated frame-header diagnostics that the modeled § 7.23
    /// buffer makes locally decidable (AV2 § 6.17.2).
    ///
    /// Currently: a show-existing-frame whose `frame_to_show_map_idx` names a slot the
    /// modeled buffer **proves** invalid (`RefValid == 0`). A *poisoned* (Unknown) slot
    /// drops to silence — the buffer cannot prove a violation there (the Unknown
    /// invariant). The check runs only when the frame's core resolved against its active
    /// sequence header (the parsed `frame_to_show_map_idx` is trustworthy).
    fn reference_state_checks(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        report: &mut ValidationReport,
    ) {
        let Some(core) = self.frame_core_against_referenced_header(obu, first_picture_in_tu) else {
            return;
        };
        if core.show_existing_frame != Some(true) {
            return;
        }
        let Some(idx) = core.frame_to_show_map_idx else {
            return;
        };
        // AV2 § 6.17.2 (mirror :4178-4179) / § 7.23: a show-existing-frame outputs the
        // frame stored at `frame_to_show_map_idx`; that reference frame must be valid
        // (`RefValid[ frame_to_show_map_idx ] == 1`). The buffer fires ONLY when it
        // PROVES the slot invalid (a CLK reset with no re-validating refresh since); a
        // poisoned (Unknown) slot stays silent.
        if self
            .reference_state
            .slot(obu.header.extended_layer_id, idx as usize)
            == SlotState::ProvenInvalid
        {
            report.push(frame_header_error(
                "frame-header/show-existing-frame-invalid-slot",
                "6.17.2",
                obu,
                format!(
                    "show-existing-frame references reference slot frame_to_show_map_idx {idx}, \
                     but the §7.23 reference-frame buffer state proves RefValid[{idx}] == 0 \
                     (the slot was invalidated by a CLK reset and not refreshed since)"
                ),
            ));
        }
    }

    /// Classifies `obu` into its coded-frame-unit [`SegRole`] (AV2 § 7.3.3 /
    /// § 7.3.4). The frame-header-derived facts come from the same best-effort
    /// parse the activation path uses; any field the parse cannot reach is left
    /// `None`, which the segmenter treats as undecidable (routing the unit to
    /// Unknown for the output class, or skipping a first-tile-group check).
    fn seg_role_for(&self, obu: &ObuEnvelope<'_>, first_picture_in_tu: bool) -> SegRole {
        let obu_type = obu.header.obu_type;
        if obu_type == ObuType::Padding {
            return SegRole::Padding;
        }
        match obu_type {
            ObuType::ContentInterpretation => SegRole::ContentInterpretation,
            ObuType::MultiFrameHeader => SegRole::MultiFrameHeader,
            ObuType::BufferRemovalTiming => SegRole::BufferRemovalTiming,
            ObuType::QuantizationMatrix => SegRole::QuantizationMatrix,
            ObuType::FilmGrain => SegRole::FilmGrain,
            ObuType::MetadataShort | ObuType::MetadataGroup => SegRole::Metadata {
                is_suffix: metadata_is_suffix(obu),
            },
            ObuType::LeadingSef | ObuType::RegularSef => SegRole::SefFrame,
            ObuType::BridgeFrame => SegRole::BridgeFrame,
            ObuType::LeadingTip | ObuType::RegularTip => SegRole::TipFrame {
                output: self.frame_output_class(obu, first_picture_in_tu),
            },
            _ if obu_type.is_tile_group() => SegRole::TileFrame {
                is_first_tile_group: self.frame_is_first_tile_group(obu),
                output: self.frame_output_class(obu, first_picture_in_tu),
            },
            // Sequence headers, LCR/OPS/atlas/MSDO, temporal delimiters, reserved:
            // not part of a coded frame unit's grammar (§ 7.3.3 / § 7.3.4 list none
            // of them). They live at the temporal-unit / coded-extended-layer level
            // and are ordered by the § 7.3.7 / § 7.3.6 machinery. Map to Padding so
            // the segmenter treats them as position-free separators (they neither
            // start nor advance a coded frame unit).
            _ => SegRole::Padding,
        }
    }

    /// Reads `is_first_tile_group` from a tile-group OBU's prefix (AV2 § 5.19),
    /// `None` if the first bit cannot be read.
    fn frame_is_first_tile_group(&self, obu: &ObuEnvelope<'_>) -> Option<bool> {
        if !obu.header.obu_type.is_tile_group() {
            return None;
        }
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        reader.read_bit().ok().map(|bit| bit != 0)
    }

    /// Records a completed first tile group's frame-header bits and, for a non-first tile
    /// group of the same coded frame, checks its `frame_header_copy()` region bit-for-bit
    /// (AV2 § 5.18.1 mirror :3960-3981; § 6.17.1 mirror :4296-4300).
    ///
    /// `frame_header(isFirst=1)` records `NumFrameHeaderBits` over `frame_header_info()`;
    /// `frame_header(isFirst=0)` is `frame_header_copy()`, exactly that many raw
    /// `header_bit` `f(1)` reads (§ 5.18.1). § 6.17.1 states it is "a requirement of
    /// bitstream conformance that `header_bit[ i ]` is equal to the value of the bit at
    /// offset `i` from the start of the frame_header structure sent with the first tile
    /// group", so a differing bit is a defect (`frame-header/copy-bits-mismatch`) and a
    /// payload shorter than `NumFrameHeaderBits` is a § 6.2.1 truncation
    /// (`frame-header/copy-bits-truncated`).
    ///
    /// The segmenter's `boundary` is the coded-frame authority, and its record lifecycle is
    /// driven for **any** frame-bearing OBU — a SEF / TIP / bridge frame is its own
    /// single-OBU coded frame (§ 7.3.3) that ENDS a preceding tile coded frame in the same
    /// triple, so its boundary must clear / poison the stale record even though it carries
    /// no copy region of its own:
    ///
    /// - [`FrameBoundary::OpensNewUnit`] resets the triple's record (a new coded frame
    ///   opened). When the OBU is a *tile-group* first whose header parsed to completion
    ///   ([`FrameHeaderParseStatus::IntraHeaderComplete`]), its bits are re-recorded; a
    ///   SEF / TIP / bridge first re-records nothing (no copy region).
    /// - [`FrameBoundary::ContinuesUnit`] on a non-first *tile group*
    ///   (`is_first_tile_group == 0`, `frame_header_present_flag == 1`) pairs against the
    ///   triple's record and checks the copy region; a non-tile continuation has no copy.
    /// - [`FrameBoundary::Ambiguous`] drops the pairing (the Unknown invariant) AND poisons
    ///   the triple's record: the undecidable OBU (an unreadable `is_first_tile_group`
    ///   delimiter, or a same-type no-delimiter TIP / bridge) may have started a new coded
    ///   frame, so the recorded first header can no longer be trusted to pair with a later
    ///   tile group. The record is removed so subsequent continuations stay silent until the
    ///   next decided [`FrameBoundary::OpensNewUnit`] re-records.
    ///
    /// A SEF / TIP / bridge OBU opening a new coded frame in the same triple as a recorded
    /// tile frame must therefore clear that record (codex round-9 F2): otherwise a later
    /// flag-0 tile group the segmenter routes as continuing that SEF coded frame (the
    /// `frame-unit/sef-single-obu` case) would pair against the stale predecessor and
    /// false-positive a `frame-header/copy-bits-*` mismatch.
    ///
    /// An incomplete / coverage-stopped / unresolvable first header records nothing, so a
    /// later non-first tile group finds no record and the copy region stays unparsed (as
    /// today). A non-first tile group whose frame had no completed first header (e.g. the
    /// first tile group itself was truncated, or a flag-0 tile group with no preceding
    /// first — already diagnosed by the segmenter) likewise finds no record and is silent.
    fn observe_frame_header_copy(
        &mut self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        boundary: Option<FrameBoundary>,
        report: &mut ValidationReport,
    ) {
        // The segmenter ignores globals (a frame-bearing OBU may not use the global xlayer,
        // already diagnosed elsewhere), returning no boundary — nothing to pair.
        let Some(boundary) = boundary else {
            return;
        };
        let key = (
            obu.header.extended_layer_id,
            obu.header.embedded_layer_id,
            obu.header.temporal_layer_id,
        );

        // The record lifecycle is driven by the segmenter boundary for ANY frame-bearing OBU,
        // NOT only tile groups. A SEF / TIP / bridge OBU is its own single-OBU coded frame
        // (§ 7.3.3): when it opens a new coded frame in the same triple it ENDS the previous
        // tile coded frame whose first header may be recorded here, so its OpensNewUnit /
        // Ambiguous boundary must clear / poison that record. Were the non-tile early return
        // kept before this, the stale record would survive and a later flag-0 tile group the
        // segmenter routes as continuing that SEF coded frame (the sef-single-obu case) would
        // pair against it and false-positive a copy-bits-* mismatch (codex round-9 F2). So:
        // OpensNewUnit clears (and re-records only for a completed tile-group first), Ambiguous
        // poisons, for every frame-bearing OBU; the tile-group-only record/check is gated below.
        let is_tile_group = obu.header.obu_type.is_tile_group();
        match boundary {
            FrameBoundary::OpensNewUnit => {
                // The first OBU of a (possibly freshly opened) coded frame. Any prior record
                // for this triple belonged to an earlier coded frame; drop it. Record header
                // bits only for a tile-group first that parsed to completion through
                // frame_header_info() — a SEF / TIP / bridge frame carries no copy region to
                // pair, so it records nothing (it only clears the stale predecessor).
                self.frame_header_copy_record.remove(&key);
                if is_tile_group
                    && let Some(recorded) = self.record_first_frame_header(obu, first_picture_in_tu)
                {
                    self.frame_header_copy_record.insert(key, recorded);
                }
            }
            FrameBoundary::ContinuesUnit => {
                // Only a tile-group OBU carries tile_group_obu() with the frame_header_copy()
                // region (§ 5.19); it carries the copy only when frame_header_present_flag == 1.
                // A non-tile continuation has no copy region to check. Pair a tile-group
                // continuation against this triple's recorded first header when present.
                if is_tile_group && let Some(recorded) = self.frame_header_copy_record.get(&key) {
                    check_frame_header_copy(obu, recorded, report);
                }
            }
            FrameBoundary::Ambiguous => {
                // The OBU's role in the coded frame is undecidable (an unreadable
                // is_first_tile_group delimiter, or a same-type no-delimiter TIP / bridge).
                // Make no copy judgment for this OBU (the Unknown invariant) — but the OBU MAY
                // have started a new coded frame, so the triple's recorded first header can no
                // longer be trusted to belong to whatever later tile group pairs against it. In
                // the other valid interpretation the OBU opened an ambiguous new frame whose
                // first header is unknown, so a later readable flag-0 tile group belongs to that
                // frame, not the recorded one. Poison the record (drop it) so subsequent
                // ContinuesUnit pairings stay silent until the next decided OpensNewUnit
                // re-records — the established poison-scope rule, matching the CELU layer's
                // unit-count poison.
                self.frame_header_copy_record.remove(&key);
            }
        }
    }

    /// Records the bits of a first tile group's frame header when it parses to completion
    /// (AV2 § 5.18.1 `NumFrameHeaderBits`). Returns `None` (record nothing → Unknown
    /// routing) when the active sequence header is unavailable, the referenced header is
    /// not the active one, the core parse did not reach
    /// [`FrameHeaderParseStatus::IntraHeaderComplete`], or the bits cannot be re-read.
    fn record_first_frame_header(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> Option<RecordedFrameHeaderBits> {
        let core = self.frame_core_against_referenced_header(obu, first_picture_in_tu)?;
        // Only a fully-consumed first header has a known NumFrameHeaderBits boundary. The
        // SEF/show-existing-frame completion is not reachable here: a tile-group first header
        // that completes does so through the intra tail (IntraHeaderComplete); a
        // show-existing-frame CLK runs decode_frame_wrapup() with no following tile group, so
        // it never pairs with a copy. Gate on IntraHeaderComplete to record exactly the
        // bit-accountable intra path.
        if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
            return None;
        }
        // The recorded bits are the frame_header() syntax — starting AFTER the
        // tile_group_obu() is_first_tile_group flag (§ 6.17.1 mirror :4303-4305: the copy
        // excludes the bits sent before frame_header). Re-read from the payload at that
        // position and capture exactly NumFrameHeaderBits == core.consumed_bits bits.
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // is_first_tile_group == 1 (the first bit) — skip it so the reader sits at the start
        // of frame_header(), where frame_header_info() (and thus the copy) begins.
        if reader.read_bit().ok()? == 0 {
            return None;
        }
        RecordedFrameHeaderBits::record(&mut reader, core.consumed_bits).ok()
    }

    /// Parses a frame-bearing OBU's [`FrameHeaderCore`] against the layer's active sequence
    /// header, but **only** returns it when the frame's referenced sequence header resolved
    /// to the very header parsed against (AV2 § 5.18.2). `None` (undecidable → Unknown
    /// routing) when:
    ///
    /// - no sequence header is active for the layer, or its stored header is missing;
    /// - the core parse failed (a payload the skeleton cannot reach); or
    /// - the frame's referenced sequence header is **not** the active header parsed against.
    ///
    /// A frame's referenced sequence header is the active header when **either**:
    ///
    /// - **`cur_mfh_id == 0`** (direct reference) and the §5.18.2 prefix's
    ///   `referenced_sequence_header_id` (set only when `seq_header_id_in_frame_header` is in
    ///   range) equals the parsed-against id; or
    /// - **`cur_mfh_id > 0`** (multi-frame-header reference) and an *in-band* multi-frame
    ///   header record resolves that `cur_mfh_id` (in range, present in
    ///   [`HlsAvailabilityStore::multi_frame_header`]) whose `mfh_seq_header_id` equals the
    ///   parsed-against id (§ 7.3.8.7). The §5.18.2 control region through the output flags is
    ///   determined by the active (== resolved) sequence header alone, so the output class /
    ///   `order_hint` are decidable on this path even though `referenced_sequence_header_id` is
    ///   `None` (the prefix leaves it unset for `cur_mfh_id > 0`).
    ///
    /// External-HLS caveat: an MFH only *externally* declared (`ExternalHlsMode::Provided`, not
    /// in-band) is **not** a verifiable association — `multi_frame_header` returns `None` for
    /// it — so the frame stays Unknown (the PR #49 partial-declaration policy). An out-of-range
    /// `cur_mfh_id`, an absent record, or an MFH whose `mfh_seq_header_id` names a different
    /// header all keep Unknown.
    ///
    /// This is the stale-activation safety: the sequence-header-dependent field widths
    /// (`order_hint` is `f(OrderHintBits)`, etc.) make any post-prefix field a misparse when
    /// read against the wrong header, so the output class and `order_hint` would be garbage. The
    /// activation/reference prefix (`cur_mfh_id`, `seq_header_id_in_frame_header`,
    /// `referenced_sequence_header_id`) is parsed *before* any sequence-dependent field, so it
    /// stays reliable even when the parse ran against a stale header — making the resolution
    /// check sound. The same guard is applied by the frame-unit segmenter's output-class
    /// derivation ([`Self::frame_output_class`]) so the two layers route to Unknown together.
    /// Resolves a frame's `cur_mfh_id` (`> 0`) reference to the in-band multi-frame
    /// header record the `cur_mfh_id > 0` core parse must consume, with the §7.3.8.7
    /// resolution discipline (AV2 § 5.18.2): the lightweight prefix parse reads only the
    /// activation fields (`cur_mfh_id` is before any sequence-dependent field, so it is
    /// reliable even against a stale active header), the `cur_mfh_id` must be nonzero
    /// and in range, an in-band record must resolve it, and that record's
    /// `mfh_seq_header_id` must equal `seq_id` — the sequence header the frame is parsed
    /// against. `None` for a `cur_mfh_id == 0` direct reference, an out-of-range
    /// `cur_mfh_id`, an absent record, or a record naming a different sequence header;
    /// the core parser then keeps its `cur_mfh_id > 0`-unresolvable early-stop rather
    /// than guessing a multi-frame-header-derived size.
    ///
    /// Shared by [`Self::frame_core_against_referenced_header`] (output-class /
    /// reference-header derivation) and [`frame_header_core_checks`] (frame-header
    /// diagnostics) so the resolution predicate has a single definition.
    fn resolve_frame_mfh_record(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        seq_id: SequenceHeaderId,
    ) -> Option<&MultiFrameHeaderRecord> {
        let prefix = parse_frame_prefix(obu, first_picture_in_tu)?;
        if prefix.cur_mfh_id.is_zero() || !prefix.cur_mfh_id.in_range() {
            return None;
        }
        let record = self.hls.multi_frame_header(prefix.cur_mfh_id)?;
        // §7.3.8.7: the resolved record must name the sequence header parsed against,
        // otherwise the multi-frame-header state would be applied against the wrong
        // maxima; a mismatch keeps the unresolvable early-stop.
        (record.mfh_seq_header_id == seq_id).then_some(record)
    }

    fn frame_core_against_referenced_header(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> Option<FrameHeaderCore> {
        let seq_id = *self
            .active_sequence_by_xlayer
            .get(&obu.header.extended_layer_id)?;
        let active_sequence = self.sequence_headers.get(&seq_id)?;

        // Resolve the frame's `cur_mfh_id` (> 0) reference to its in-band multi-frame header,
        // so the parser can be invoked with the resolving record (shared §7.3.8.7 discipline).
        let mfh_record = self.resolve_frame_mfh_record(obu, first_picture_in_tu, seq_id);

        // AV2 § 7.23: thread the modeled per-extended-layer reference-frame buffer view
        // into the core parse (forward plumbing for the §5.18 inter reference paths; no
        // intra branch reads it today). The scratch arrays must outlive the parse, so
        // they are stack-local here and borrowed by the view; an extended layer with no
        // modeled buffer yet threads `unknown()`.
        let mut ref_valid = [false; NUM_REF_FRAMES];
        let mut ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        let reference_state = if self
            .reference_state
            .view_into(
                obu.header.extended_layer_id,
                &mut ref_valid,
                &mut ref_oh,
                &mut ref_w,
                &mut ref_h,
            )
            .is_some()
        {
            FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h)
        } else {
            FrameReferenceStateView::unknown()
        };

        let core = parse_frame_core(
            obu,
            first_picture_in_tu,
            active_sequence,
            mfh_record,
            reference_state,
        )?;

        // The referenced sequence header must be the one parsed against. For a `cur_mfh_id == 0`
        // direct reference, `referenced_sequence_header_id` carries the §5.18.2 prefix's
        // resolved id (set only when `seq_header_id_in_frame_header` is in range). For a
        // `cur_mfh_id > 0` reference, the resolved in-band MFH record's `mfh_seq_header_id`
        // is the referenced id (§ 7.3.8.7); a record only externally declared, out of range,
        // absent, or naming a different sequence header leaves `mfh_record == None` (the shared
        // resolver already enforced the seq-id match) and routes to Unknown.
        let referenced = if core.cur_mfh_id.is_zero() {
            core.referenced_sequence_header_id
        } else {
            mfh_record.map(|record| record.mfh_seq_header_id)
        };
        if referenced != Some(seq_id) {
            return None;
        }
        Some(core)
    }

    /// Derives a frame-bearing OBU's output class (`immediate_output_frame == 1 ||
    /// implicit_output_frame == 1`, AV2 § 7.3.3 / § 6.17.2) from a best-effort core
    /// parse against its active sequence header. `None` (undecidable) when the
    /// active sequence is unavailable, the frame's referenced sequence header is not the
    /// active header parsed against ([`Self::frame_core_against_referenced_header`]), or the
    /// core parse stops before the output flags — which routes the unit to Unknown rather
    /// than guessing.
    fn frame_output_class(&self, obu: &ObuEnvelope<'_>, first_picture_in_tu: bool) -> Option<bool> {
        let core = self.frame_core_against_referenced_header(obu, first_picture_in_tu)?;
        match (core.immediate_output_frame, core.implicit_output_frame) {
            (Some(immediate), Some(implicit)) => Some(immediate || implicit),
            // One flag known and already true settles the output class; the other
            // being unreached cannot flip an output frame to non-output.
            (Some(true), _) | (_, Some(true)) => Some(true),
            _ => None,
        }
    }

    /// Classifies a **non-frame-bearing** `obu` into its coded-extended-layer-unit
    /// [`CeluRole`] (AV2 § 7.3.6), parallel to [`Self::seg_role_for`]. The HLS headers (LCR /
    /// OPS / atlas / sequence header) and content-interpretation map directly; all other
    /// coded-extended-layer-interior OBUs (BRT / QM / FGM / metadata / MFH) are `FrameInterior`;
    /// padding is position-free. Frame-bearing OBUs are dispatched by the caller (see
    /// [`Self::observe_frame_bearing_obu`]) so their facts and OrderHintBits come from a single
    /// shared parse + resolution; if one reaches here it is treated as transparent padding.
    fn celu_role_for(&self, obu: &ObuEnvelope<'_>) -> CeluRole {
        match obu.header.obu_type {
            ObuType::Padding => CeluRole::Padding,
            ObuType::LayerConfigurationRecord => CeluRole::LayerConfigurationRecord,
            ObuType::OperatingPointSet => CeluRole::OperatingPointSet,
            ObuType::AtlasSegment => CeluRole::AtlasSegment,
            ObuType::SequenceHeader => CeluRole::SequenceHeader,
            ObuType::ContentInterpretation => CeluRole::ContentInterpretation,
            ObuType::BufferRemovalTiming
            | ObuType::QuantizationMatrix
            | ObuType::FilmGrain
            | ObuType::MetadataShort
            | ObuType::MetadataGroup
            | ObuType::MultiFrameHeader => CeluRole::FrameInterior,
            // Reserved types (and the global-only temporal delimiter / MSDO, which the
            // tracker filters as global) are ignored by the § 7.3.6 grammar ("OBU types that
            // are not defined in this specification can be ignored", mirror line 618). Map to
            // Padding so they are transparent — neither opening a frame nor advancing an HLS
            // phase.
            _ => CeluRole::Padding,
        }
    }

    /// Derives the [`FrameFacts`] for a frame-bearing OBU from a best-effort core parse
    /// against its active sequence header (AV2 § 5.18.2). Leading-ness is type-decided from
    /// `obu_type` (see [`frame_leadingness`]), so it never routes to Unknown; the output
    /// class and `order_hint` are `None` when the parse stops before them or the active
    /// sequence header is unavailable (the Unknown invariant).
    ///
    /// The coded-frame-unit `boundary` is the [`FrameUnitSegmenter`]'s authoritative signal
    /// for this OBU (the segmenter is the single source of truth for coded-frame-unit
    /// boundaries, § 7.3.6); the CELU tracker consumes it rather than re-deriving boundaries.
    /// `boundary` is `None` only for a *global* frame-bearing OBU (the segmenter ignores
    /// globals), which the CELU tracker also filters before it reads this field — so the
    /// `OpensNewUnit` fallback is never observed.
    fn frame_celu_facts(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        boundary: Option<FrameBoundary>,
    ) -> (FrameFacts, Option<u32>) {
        let obu_type = obu.header.obu_type;
        let leadingness = frame_leadingness(obu_type);

        // F3: the output class is TYPE-DECIDED for a SEF (§ 7.3.3 "Or" branch -> output) and a
        // BRIDGE (§ 7.3.4 list only -> non-output) by `obu_type` alone, BEFORE consulting any
        // parsed flag — `type_decided_output` is the single source of truth shared with the
        // frame-unit segmenter. A bridge parser stops early and would otherwise route to Unknown,
        // suppressing the § 7.3.6 presence checks; the type decision keeps it decided.
        let type_decided = type_decided_output(obu_type);

        // F4: one core parse + resolution drives BOTH the flag-derived facts AND the OrderHintBits
        // contribution. `frame_core_against_referenced_header` returns `Some` only when the
        // frame's referenced sequence header resolved to the active header it parsed against
        // (the stale-activation guard). When it resolves, the active header IS the referenced
        // one, so its `OrderHintBits` is this frame's bits; when it does not resolve, the bits
        // contribution is `None` (not the stale active header's bits) so the § 7.3.7
        // same-OrderHintBits-in-TU check is never fed a wrong-bits value.
        let core = self.frame_core_against_referenced_header(obu, first_picture_in_tu);
        let (flag_output, order_hint, bits) = match &core {
            Some(core) => {
                let flag_output = match (core.immediate_output_frame, core.implicit_output_frame) {
                    (Some(immediate), Some(implicit)) => Some(immediate || implicit),
                    // One flag known and already true settles output; the other being unreached
                    // cannot flip an output frame to non-output (mirror § 6.17.2).
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    _ => None,
                };
                // The resolved frame's OrderHintBits is the active (== referenced) header's,
                // when its inter config was parsed (the Unknown invariant otherwise).
                let bits = self
                    .active_sequence_by_xlayer
                    .get(&obu.header.extended_layer_id)
                    .and_then(|seq_id| self.sequence_headers.get(seq_id))
                    .and_then(|seq| seq.inter.as_ref())
                    .map(|inter| u32::from(inter.order_hint_bits));
                (flag_output, core.order_hint_lsb, bits)
            }
            None => (None, None, None),
        };

        // The type decision wins when present; otherwise the flag-derived class (Unknown when
        // the parse did not resolve / reach the flags). `order_hint` is the parsed `order_hint_lsb`
        // (the LSB proxy, see [`crate::celu`]); a SEF/bridge with an absent reference keeps its
        // type-decided output but contributes no order_hint / bits.
        let output = type_decided.or(flag_output);

        (
            FrameFacts {
                obu_type,
                boundary: boundary.unwrap_or(FrameBoundary::OpensNewUnit),
                output,
                order_hint,
                // Round-6 F2: carry the per-frame OrderHintBits into the facts so the CELU
                // tracker can gate the cross-CELU §7.3.7 OrderHint comparison on only the two
                // COMPARED output units' bits being known and equal — the SAME resolved bits
                // value also threaded TU-wide to `note_order_hint_bits` for the same-bits
                // judgment (constraint 1). A frame whose referenced header did not resolve
                // contributes `None` here too (the stale-activation guard).
                order_hint_bits: bits,
                leadingness,
            },
            bits,
        )
    }

    /// Whether the § 7.3.7 / § 7.4.6 DOH constraints are active for the *just-completed*
    /// temporal unit at a global-temporal-delimiter boundary or at the end of the bitstream
    /// (round-5 F1 / round-6 F1). Resolves the LCR side against the GOVERNING CMVS window of
    /// the completed unit rather than the live window the [`CmvsTracker`](CmvsTracker) has
    /// just mutated.
    ///
    /// The base disjunction (see [`Self::doh_constraint_flag_active_in_window`]) is:
    /// `multistream_doh_constraint_flag` in the preceding MSDO equals 1, or
    /// `lcr_doh_constraint_flag` in the activated global LCR equals 1. Either source being
    /// absent contributes `false`, so when neither source declares the constraint the
    /// flag-gated checks stay silent.
    ///
    /// Per § 7.3.2 the completed unit is *contained* in whichever CMVS it belongs to:
    ///
    /// - When the unit BEGINS a new CMVS (begin condition 1, 2, or 3), it is the FIRST unit of
    ///   the NEW CMVS, whose window the tracker has just opened at this unit's index — the live
    ///   (post-completion) window `Some(start)`, used here.
    /// - When the unit only CONTINUES the CMVS, the window is unchanged either way.
    /// - When the unit ENDS the CMVS without beginning a new one (end condition 2 — a CLK with
    ///   no MSDO, no activated global LCR — Closes the live window to `None`), it is the LAST
    ///   unit of the ENDING CMVS, whose window was the pre-completion start. The live window is
    ///   `None`, so this falls back to `cmvs_window_before_completion`.
    ///
    /// So the governing window is the post-completion window when it is `Some`, else the
    /// pre-completion window — which is exactly the window of the CMVS that contains the
    /// completed unit. The MSDO side is window-independent: `msdo_substream_max` is last-wins
    /// live state that `complete_temporal_unit` does not clear, so it already reflects the MSDO
    /// that governed the completed unit (it must remain the preceding MSDO regardless of the
    /// LCR window).
    fn doh_constraint_flag_active_for_completed_tu(
        &self,
        cmvs_window_before_completion: Option<u64>,
    ) -> bool {
        let governing_window = self
            .cmvs
            .current_cmvs_start_tu_index()
            .or(cmvs_window_before_completion);
        self.doh_constraint_flag_active_in_window(governing_window)
    }

    /// The base § 7.3.7 / § 7.4.6 DOH constraint-active disjunction (mirror lines 650-657 /
    /// 1316-1320), resolving the activated-global-LCR side against an explicit CMVS-window
    /// start (`None` → no window → no activated global LCR, via
    /// [`Self::activated_global_lcr_in_window`]). The MSDO side is window-independent (the
    /// live last-wins preceding MSDO, [`Self::msdo_substream_max`]; § 7.3.8.2 keeps a non-RAP
    /// MSDO identical to its predecessor). The DOH flag is active iff either source declares
    /// the constraint.
    fn doh_constraint_flag_active_in_window(&self, cmvs_window_start: Option<u64>) -> bool {
        let msdo_flag = self
            .msdo_substream_max
            .as_ref()
            .is_some_and(|m| m.doh_constraint_flag);
        let lcr_flag = cmvs_window_start.is_some_and(|cmvs_start| {
            self.activated_global_lcr_in_window(cmvs_start)
                .is_some_and(|(_, record)| record.doh_constraint_flag)
        });
        msdo_flag || lcr_flag
    }

    /// Resets the §6.12/§6.13 coded-frame windows for quantizer-matrix and film-grain
    /// state at any coded frame, including a SEF (the § 7.3.3 grammar makes a SEF its
    /// own coded frame unit and calls it a frame, so it is a coded-frame boundary for
    /// both the QM between-coded-frames window and the film-grain coded-frame-unit
    /// window — see [`is_frame_bearing`]).
    fn reset_coded_frame_window(&mut self) {
        self.qm.reset_coded_frame_window();
        self.film_grain.reset_coded_frame_window();
    }

    /// Tracks the boundary events that scope the coded-video-sequence comparison
    /// state (sequence-header fingerprints, AV2 § 7.3.6, and content-interpretation
    /// records, § 6.4.12 / § 6.14).
    ///
    /// A global `OBU_TEMPORAL_DELIMITER` completes the current temporal unit: the
    /// deferred cross-temporal-unit diagnostics are flushed (see [`CvsTracker`]) and
    /// the per-temporal-unit frame set resets. An `OBU_CLOSED_LOOP_KEY` starts a new
    /// coded video sequence for its extended layer at the *current* temporal unit
    /// (AV2 § 7.3.6: "A new coded video sequence for an extended layer is defined to
    /// start at each temporal unit that contains an OBU with obu_type equal to
    /// OBU_CLOSED_LOOP_KEY in the coded extended layer unit corresponding to the
    /// extended layer"); see `start_cvs_for_xlayer`. This models the exact § 7.3.6
    /// boundary for sequential decoding from the raw OBU headers alone — no
    /// random-access state is needed. The § 7.4.4 treat-as-new-CVS behavior when a
    /// decoder *initiates* decoding at an OLK applies only to random-access
    /// decoding, not to the sequential decoding a bitstream validator models
    /// ("During sequential decoding, the process does not start a new coded video
    /// sequence for the extended layer", § 7.4.4), so OLK / RAS OBUs are
    /// deliberately not CVS boundary events here. An OLK (like a CLK) is,
    /// however, a § 7.3.8.11 random access point that unconditionally
    /// re-initializes its extended layer's content interpretation parameters, so
    /// both record the CI-parameter epoch (see `observe_ci_rap`). Frame-header
    /// activation drives the *active* sequence header (see
    /// `observe_frame_bearing_obu`); these events drive the fingerprint /
    /// content-interpretation scope.
    ///
    /// NB: the §6.12/§6.13 quantizer-matrix / film-grain duplicate windows are
    /// deliberately NOT reset here. Those windows close at a *coded frame*, not at a
    /// temporal-unit or CVS boundary, so a QM level / film-grain slot reused across
    /// a temporal delimiter with no intervening frame is still a duplicate (see
    /// reset_coded_frame_window, called from the frame-bearing branch).
    fn observe_cvs_boundary_events(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if obu.header.obu_type == ObuType::TemporalDelimiter
            && obu.header.extended_layer_id.is_global()
        {
            // The just-completed temporal unit's index, captured before
            // `advance_temporal_unit` bumps `tu_index` to the next temporal unit. The Annex
            // A Table A.4 IOP commit (below) needs it to decide whether this temporal unit
            // begins a new coded video sequence relative to the open window.
            let completed_tu_index = self.cvs.tu_index;
            self.cvs.advance_temporal_unit(report);
            // AV2 § 6.16.7 / § 7.3.6: any deferred inference-presence diagnostic that
            // survived this temporal unit saw no CLK detach its earlier-temporal-unit
            // seed, so the seed stayed intra-CVS and the field inferred cleanly — drop
            // it silently (see TimecodeCvsState::pending_inference).
            self.drop_pending_timecode_inference();
            // AV2 § 7.3.2: the global temporal delimiter ends the just-observed
            // temporal unit, so the accumulated § 7.3.2 begin/end facts are evaluated
            // now, before the per-temporal-unit facts reset for the next unit. Any
            // deferred provisional-Inside § 6.4.1 monotonic disagreements are resolved
            // here against the completed temporal unit's final CMVS membership.
            // `completed_tu_index` (captured before advance_temporal_unit) is the
            // just-completed temporal unit's index, which stamps the CMVS-window start
            // (§ 7.3.2 scoping).
            //
            // The CMVS-window start of the *just-completed* temporal unit, captured BEFORE
            // `complete_temporal_unit` applies this unit's § 7.3.2 begin/end conditions and
            // mutates the live window (round-5 F1). The § 7.3.7 DOH flag for the completed unit
            // must be sampled against the CMVS that CONTAINS it: when this unit ENDS the CMVS
            // (end condition 2 — a CLK with no MSDO, no activated global LCR), it is the LAST
            // temporal unit of the ENDING CMVS, so its governing window is this pre-completion
            // start — not the live window the tracker is about to clear (see
            // [`Self::doh_constraint_flag_active_in_window`]).
            let cmvs_window_before_completion = self.cmvs.current_cmvs_start_tu_index();
            self.cmvs.complete_temporal_unit(completed_tu_index, report);
            // AV2 § 6.6: now that the just-completed temporal unit's CMVS membership is
            // resolved, evaluate the deferred `msdo/doh-constraint-required` check for its
            // frame-confirmed activations (see resolve_deferred_doh_constraint). Must run
            // after cmvs.complete_temporal_unit so the membership is final.
            self.resolve_deferred_doh_constraint(options, report);
            // AV2 § 6.8.2: with the just-completed temporal unit's CMVS membership
            // resolved, evaluate the deferred MSDO↔global-LCR agreement and the LCR
            // DOH-constraint requirement for its frame-confirmed activations (see
            // resolve_deferred_lcr_msdo_agreement).
            self.resolve_deferred_lcr_msdo_agreement(options, report);
            // AV2 § 7.3.2: evaluate the boundary-set-identity check for the just-completed
            // temporal unit (a CLK-without-MSDO with an activated global LCR diverges the
            // MSDO-alone and MSDO+LCR boundary sets; see resolve_deferred_cmvs_boundary).
            self.resolve_deferred_cmvs_boundary(options, report);
            // Annex A Table A.4: commit the just-completed temporal unit's IOP pending facts
            // to the right coded-video-sequence window — flushing and evaluating the prior
            // window first when this temporal unit begins a new coded video sequence (a CLK
            // in a temporal unit later than the open window's start, § 7.3.6).
            self.commit_annex_a_iop_pending(completed_tu_index, options, report);
            // AV2 § 7.3.8.2: the just-completed temporal unit's buffered OBU_MSDO(s) are
            // resolved against the previous OBU_MSDO now that the temporal unit's
            // § 7.4.1 random-access-point-ness is fully known.
            self.msdo_identity.complete_temporal_unit(report);
            // AV2 § 7.3.8.1: with the just-completed temporal unit's § 7.4.1 random-
            // access-point-ness and leading-frame-ness now known, resolve the buffered
            // HLS-availability replay references for it (suppressed under any external-HLS
            // Provided mode per the partial-declaration policy).
            self.complete_rap_replay_tu(completed_tu_index, options, report);
            self.frames_seen_in_tu.clear();
            // AV2 § 7.3.3 / § 7.3.4 / § 7.3.7: a coded frame unit does not span
            // temporal units, so the segmenter resolves its just-completed temporal
            // unit's still-open units' deferred (output-class-dependent) checks and
            // clears its per-temporal-unit state. The § 7.3.8.10 first-coded-frame-
            // unit CI counters likewise reset per temporal unit.
            self.frame_unit.reset_temporal_unit(report);
            // AV2 § 5.18.1 / § 7.3.7: a coded frame does not span temporal units, so the
            // per-coded-frame recorded first-header bits (for the § 6.17.1
            // frame_header_copy() bit-identity check) cannot pair across this boundary.
            // Clear them with the segmenter's per-temporal-unit state.
            self.frame_header_copy_record.clear();
            // AV2 § 7.3.6 / § 7.3.7 / § 7.4.6: resolve the just-completed temporal unit's
            // coded-extended-layer-unit constraints (output-frame presence, OrderHint
            // agreement, CLK/OLK first-unit and lowest-layer rules, all-leading-or-none) and
            // the flag-gated DOH OrderHint / OrderHintBits checks, then clear the per-TU
            // CELU state. The DOH flag must be recorded from the *just-completed* temporal
            // unit's activated global LCR / preceding MSDO before resolution. Runs after the
            // CMVS / activation resolution above so the activation chain is final. The LCR side
            // is sampled against the GOVERNING window of the completed unit (captured before
            // `complete_temporal_unit` cleared the live window, round-5 F1), so a CLK boundary
            // unit that ends the CMVS is still governed by the activated global LCR of the CMVS
            // that contained it.
            self.celu.set_doh_flag_active(
                self.doh_constraint_flag_active_for_completed_tu(cmvs_window_before_completion),
            );
            self.celu.reset_temporal_unit(report);
            // AV2 § 7.3.6 (round-6 F3): resolve the just-completed temporal unit's CIs against
            // the first-coded-extended-layer-unit-of-the-sequence presence rule (mirror lines
            // 560-562). Runs after the CLK boundary events of the temporal unit have been
            // applied (they are processed at the CLK OBU, earlier in the same temporal unit, so
            // `start_cvs_for_xlayer` already re-seeded the first-CELU state), so each CI's CVS
            // membership is final.
            self.resolve_ci_first_celu_for_tu(completed_tu_index, options, report);
            // AV2 § 7.3.7: clear the per-temporal-unit distinct-`obu_mlayer_id` sets so a
            // CLK in the next temporal unit re-attributes only that temporal unit's ids
            // to the new coded video sequence (see DistinctMlayerTracker::reset_cvs).
            self.distinct_mlayer.advance_temporal_unit();
        } else if obu.header.obu_type == ObuType::ClosedLoopKey {
            // AV2 § 7.3.6: this temporal unit begins a new coded video sequence for the
            // CLK's extended layer.
            self.start_cvs_for_xlayer(obu.header.extended_layer_id, report);
            self.observe_ci_rap(obu.header.extended_layer_id);
            // AV2 § 6.16.7 / § 6.16.10 / § 7.3.8.11 (finding 1, CLK re-pair). The
            // epoch-aware CI dedup deduplicates a CI re-sent in this RAP temporal unit
            // BEFORE the CLK (the lagging epoch could not tell it apart from an ordinary
            // identical repeat at CI-time, so the recheck was skipped). observe_ci_rap
            // has now advanced the epoch and dropped the stale pre-RAP timecode /
            // scan-type pairings; the CI re-sent in this temporal unit is the
            // § 7.3.8.11 authority for the new coded video sequence's pictures, so
            // re-pair the new epoch's observations against it now — once, with no
            // duplicate (the pre-RAP pairing was dropped, not reported).
            self.repair_post_rap_ci_pairings(obu.header.extended_layer_id, report);
            // AV2 § 7.3.2 / § 7.3.6: a CLK makes this temporal unit one that "contains
            // an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY for at least one
            // extended layer" (and begins a new coded video sequence for that layer).
            self.cmvs.note_clk();
            // AV2 § 7.3.6: a CLK also begins a new coded video sequence for the Annex A
            // Table A.4 IOP window. Recording it on the per-temporal-unit pending facts
            // means a same-temporal-unit pre-CLK OBU_MSDO/LCR is attributed to the NEW
            // coded video sequence when the temporal unit commits (lesson 8).
            self.annex_a_iop.note_clk();
            // AV2 § 7.4.1: a CLK makes the temporal unit a random access point.
            self.msdo_identity.note_random_access_point();
            // AV2 § 7.3.8.1 / § 7.4.6: the same random access point drives the HLS
            // availability replay (see RapReplayTracker), scoped to the CLK's own extended
            // layer — random access initiates per extended layer.
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        } else if obu.header.obu_type == ObuType::OpenLoopKey {
            // An OLK is NOT a § 7.3.6 CVS boundary during sequential decoding
            // (§ 7.4.4), but it IS a § 7.3.8.11 random access point that
            // re-initializes the extended layer's content interpretation
            // parameters to defaults.
            self.observe_ci_rap(obu.header.extended_layer_id);
            // AV2 § 6.16.7 / § 6.16.10 / § 7.3.8.11 (finding 1, OLK re-pair). Like
            // the CLK branch above, an OLK is a § 7.3.8.11 random access point, so a
            // CI re-sent identically in this RAP temporal unit BEFORE the OLK was
            // deduplicated by the epoch-aware CI guard (the lagging epoch could not
            // tell it apart from an ordinary identical repeat at CI-time). observe_ci_rap
            // has now advanced the epoch and dropped the stale pre-RAP timecode /
            // scan-type pairings; the CI re-sent in this temporal unit is the
            // § 7.3.8.11 authority for the new epoch's pictures, so re-pair the new
            // epoch's observations against it now — once, with no duplicate.
            self.repair_post_rap_ci_pairings(obu.header.extended_layer_id, report);
            // AV2 § 7.4.1: an OLK makes the temporal unit a random access point.
            self.msdo_identity.note_random_access_point();
            // AV2 § 7.3.8.1 / § 7.4.6: the same random access point drives the HLS
            // availability replay (see RapReplayTracker), scoped to the OLK's own extended
            // layer — random access initiates per extended layer.
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        } else if obu.header.obu_type == ObuType::RasFrame {
            // AV2 § 7.4.1: a RAS frame (OBU_RAS_FRAME) makes the temporal unit a random
            // access point. It is not a § 7.3.6 sequential-decoding CVS boundary, so it
            // touches only the § 7.3.8.2 identity tracker and § 7.3.8.1 replay tracker
            // here. The replay anchor is scoped to the RAS frame's own extended layer
            // (§ 7.4.6: random access initiates per extended layer).
            self.msdo_identity.note_random_access_point();
            self.rap_replay
                .note_random_access_point(obu.header.extended_layer_id);
        }
    }

    /// Records a § 7.3.8.11 random access point (CLK or OLK) for `xlayer` at the
    /// current temporal unit: "The content interpretation parameters for each
    /// embedded layer in an extended layer are initialized to default values ...
    /// at each random access point of the extended layer (i.e., at each temporal
    /// unit containing an OBU in the extended layer with obu_type equal to
    /// OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY)". The § 6.16.10 Table 6.18
    /// scan-type / CI pairing epoch starts here. The epoch starts AT the temporal
    /// unit, so same-temporal-unit records and observations belong to the new
    /// epoch (a CI OBU in the random access point's own temporal unit
    /// re-establishes the parameters, § 7.3.8.11 step 2) — matching the
    /// `tu_index >= epoch` retention convention of the CVS stores. Pending
    /// deferred § 6.16.10 Table 6.18 pairing diagnostics and the § 6.16.7
    /// n_frames-bound pairing for the extended layer pair pre-epoch CI content
    /// (`ci_scan_type_idc` / `equal_picture_interval` / `ci_timing_info_present_flag`)
    /// with post-epoch pictures (or vice versa), so exactly those three rules are
    /// dropped; every other pending diagnostic (§ 6.14 repeated-CI identity,
    /// § 6.4.12 timing, group consistency) is CVS-scoped and survives an OLK.
    fn observe_ci_rap(&mut self, xlayer: ExtendedLayerId) {
        self.ci_rap_started_in_tu.insert(xlayer, self.cvs.tu_index);
        self.cvs.drop_pending_for_rules(
            xlayer,
            &[
                "metadata/scan-type-ci-scan-type-mismatch",
                "metadata/scan-type-equal-picture-interval-required",
                // § 6.16.7 / § 7.3.8.11: the n_frames bound is established by a CI's
                // ci_timing_info_present_flag, which a random access point reinitializes
                // to 0 (finding 5). A deferred n_frames pairing against a prior-TU CI
                // pairs that pre-epoch timing with post-epoch pictures, so this reinit
                // invalidates it exactly as it does the scan-type pairings above.
                "metadata/timecode-n-frames-exceeds-rate",
            ],
        );
    }

    /// The temporal unit at which `xlayer`'s current § 7.3.8.11
    /// content-interpretation-parameter epoch started (its most recent CLK / OLK
    /// random access point), or 0 when none has been observed.
    fn ci_rap_epoch(&self, xlayer: ExtendedLayerId) -> u64 {
        self.ci_rap_started_in_tu.get(&xlayer).copied().unwrap_or(0)
    }

    /// Buffers a linearly-resolved § 7.3.8.1 HLS reference for the random-access-point
    /// availability replay, governed by the referencing OBU's extended layer `xlayer`
    /// (resolved at temporal-unit completion; see [`RapReplayTracker`]). § 7.4 random
    /// access initiates per extended layer (§ 7.4.6), so a reference answers to its own
    /// layer's most recent random access point (a [`GLOBAL_XLAYER_ID`] reference answers
    /// to the global anchor). The caller buffers only references whose object was available
    /// in-band at reference time and not suppressed by external HLS, keeping the replay
    /// predicate disjoint from the linear `hls/unavailable-*` checks.
    fn note_rap_reference(&mut self, key: RapHlsKey, xlayer: ExtendedLayerId, offset: ByteOffset) {
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
    fn note_frame_rap_references(
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
    fn complete_rap_replay_tu(
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

    /// Re-pairs the § 6.16.7 n_frames bound and the § 6.16.10 Table 6.18 scan-type
    /// restrictions of the new coded video sequence's observations against the content
    /// interpretation OBUs re-sent IDENTICALLY in this CLK's temporal unit (finding 1,
    /// the CLK side of the epoch-aware dedup).
    ///
    /// A content interpretation re-sent in a § 7.3.8.11 random-access-point temporal
    /// unit re-establishes the parameters for the new coded video sequence (§ 7.3.8.11
    /// step 2). When it repeats the pre-RAP content **identically** the epoch-aware dedup
    /// ([`Self::observe_content_interpretation`]) skipped its CI-time recheck — at
    /// CI-time the epoch had not advanced past the still-present pre-RAP record, so the
    /// re-sent CI could not be told apart from an ordinary identical repeat. By the time
    /// the CLK runs, [`Self::observe_ci_rap`] has advanced the epoch to this temporal
    /// unit and dropped the stale pre-RAP pairings. Re-running the suppressed rechecks
    /// now pairs the new epoch's observations (`tu_index >= epoch`, i.e. this temporal
    /// unit's metadata, since the epoch filter inside the rechecks excludes the dropped
    /// previous-epoch observations) against the re-sent CI exactly once — the
    /// authoritative pairing, with no duplicate because the pre-RAP pairing was dropped
    /// rather than reported.
    ///
    /// The re-pair is filtered to the CIs whose CI-time recheck the dedup guard actually
    /// SUPPRESSED — i.e. an identical re-send of the pre-RAP record (finding 1). A CI
    /// re-sent in this RAP temporal unit with a CHANGED (different) decisive content
    /// defeats the dedup guard and rechecked EAGERLY at CI-time, already reporting any
    /// violation; re-pairing it here too would duplicate the diagnostic, so the
    /// per-recheck `*_recheck_suppressed` flags exclude it. The scan-type and timecode
    /// suppressions are filtered independently, since a re-send can change one decisive
    /// content while leaving the other identical.
    ///
    /// Only the content interpretations re-sent IN this temporal unit (at/after the
    /// epoch) for the CLK's extended layer (or a global-keyed CI, which describes every
    /// layer) drive the re-pair; a CI from an earlier temporal unit belongs to the
    /// ending coded video sequence and is excluded by the epoch.
    fn repair_post_rap_ci_pairings(
        &mut self,
        clk_xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        let epoch = self.ci_rap_epoch(clk_xlayer);
        // Idempotent within a temporal unit: a malformed temporal unit with two CLK/OLK
        // random access points for the SAME extended layer calls this twice. The second
        // observe_ci_rap leaves the epoch at this temporal unit and drops nothing new, so
        // a second re-pair would replay the same post-epoch CI snapshot and duplicate
        // every repaired diagnostic. Run once per (extended layer, temporal unit).
        let tu_index = self.cvs.tu_index;
        if self.repaired_post_rap_in_tu.get(&clk_xlayer) == Some(&tu_index) {
            return;
        }
        self.repaired_post_rap_in_tu.insert(clk_xlayer, tu_index);
        // Snapshot the re-sent (post-epoch) CI records for this extended layer to avoid
        // holding the content_interpretations borrow across the rechecks (which mutate
        // the deferral state). ContentInterpretation is Copy, so this is cheap. The
        // two suppression flags select which recheck to replay (finding 1): only a
        // recheck the dedup guard skipped at CI-time is re-paired here, so a re-send
        // that changed the content (already rechecked eagerly) is not re-reported.
        let resent: Vec<(
            ExtendedLayerId,
            EmbeddedLayerId,
            ContentInterpretation,
            ByteOffset,
            bool,
            bool,
        )> = self
            .content_interpretations
            .iter()
            .filter(|((ci_xlayer, _), record)| {
                (*ci_xlayer == clk_xlayer || ci_xlayer.is_global()) && record.tu_index >= epoch
            })
            .map(|((ci_xlayer, ci_mlayer), record)| {
                (
                    *ci_xlayer,
                    *ci_mlayer,
                    record.content,
                    record.offset,
                    record.scan_type_recheck_suppressed,
                    record.timecode_recheck_suppressed,
                )
            })
            .collect();
        for (ci_xlayer, ci_mlayer, content, ci_offset, scan_suppressed, timecode_suppressed) in
            resent
        {
            if scan_suppressed {
                // Re-pair (`repair = true`): a `(observation, this CI)` pair already
                // paired-and-emitted eagerly against an in-scope same-RAP-TU CI (one
                // re-sent BEFORE the observation) is skipped to avoid duplicating the
                // diagnostic (the scan-type analogue of the round-7 timecode finding 2).
                // The skip is per-CI, so a pair whose eager pairing was deferred against
                // the stale pre-RAP CI — dropped by observe_ci_rap at the RAP — is
                // re-paired, even when the SAME observation already emitted eagerly
                // against a DIFFERENT CI.
                self.recheck_scan_type_after_ci(
                    ci_xlayer, ci_mlayer, &content, ci_offset, true, report,
                );
            }
            if timecode_suppressed {
                // Re-pair (`repair = true`): a `(observation, this CI)` pair already
                // paired-and-emitted eagerly against an in-scope same-RAP-TU CI (one
                // re-sent BEFORE the observation) is skipped to avoid duplicating the
                // diagnostic (round-7 finding 2). The skip is per-CI, so a pair whose
                // eager pairing was deferred against the stale pre-RAP CI — dropped by
                // observe_ci_rap at the RAP — is re-paired, even when the SAME observation
                // already emitted eagerly against a DIFFERENT CI.
                self.recheck_timecode_n_frames_after_ci(
                    ci_xlayer, ci_mlayer, &content, ci_offset, true, report,
                );
            }
        }
    }

    /// Starts a new coded video sequence for `xlayer` at the current temporal unit
    /// (AV2 § 7.3.6): drops the extended layer's CVS-scoped records (sequence
    /// fingerprints, content interpretations, active metadata, HDR baselines,
    /// scan-type observations) from earlier temporal units — same-temporal-unit
    /// records, e.g. the sequence header preceding the activating CLK, joined the
    /// new coded video sequence and stay — and its pending deferred diagnostics.
    /// Records keyed under `GLOBAL_XLAYER_ID` belong to no single extended layer,
    /// so they are pruned at every boundary event as a documented approximation
    /// (§ 6.16.10-style "current CVS" scoping for layer-nonspecific global records
    /// has no single owner; their deferred diagnostics are filtered at the
    /// temporal-unit flush instead, see [`CvsTracker::flush_completed_tu`]).
    /// Idempotent within a temporal unit.
    fn start_cvs_for_xlayer(&mut self, xlayer: ExtendedLayerId, report: &mut ValidationReport) {
        self.cvs.start_cvs(xlayer, report);
        let tu_index = self.cvs.tu_index;
        // AV2 § 7.3.6 (mirror lines 560-562, round-6 F3): this CLK starts a new coded video
        // sequence for `xlayer` at this temporal unit, whose CELU is the "first coded extended
        // layer unit of the sequence". Reset the first-CELU CI presence state so the new
        // sequence judges its own first CELU. Idempotent within a temporal unit (a redundant
        // CLK in the same temporal unit is the same boundary event, so it must not drop CI
        // presence already recorded for this first CELU): only re-seed when the recorded first
        // CELU temporal unit differs from this one.
        let ci_state = self.ci_first_celu.entry(xlayer).or_default();
        if ci_state.first_celu_tu != Some(tu_index) {
            *ci_state = CiFirstCeluState {
                first_celu_tu: Some(tu_index),
                ..CiFirstCeluState::default()
            };
        }
        // AV2 § 7.3.6: "A new coded video sequence for an extended layer is defined to
        // start at each temporal unit that contains an OBU with obu_type equal to
        // OBU_CLOSED_LOOP_KEY ..." (mirror `07-decoding-process.md` lines 604–606). The
        // whole temporal unit containing this CLK lies in the NEW coded video sequence,
        // so an OPS buffer-delay baseline observed EARLIER in this same temporal unit
        // (before the CLK) belongs to the new coded video sequence, not the old one: its
        // stored CVS epoch is migrated to the layer's new epoch. A later OPS in this same
        // temporal unit then shares the migrated baseline's epoch and the § 6.10.5 error
        // tier compares them within one coded video sequence (the complementary case to
        // the deferred-error `on_drop` path: there the comparison's deferred error is
        // dropped/replaced; here the baseline was stored with no comparison pending).
        // Baselines from EARLIER temporal units genuinely belong to the old coded video
        // sequence and are left untouched. Only baselines keyed under this exact extended
        // layer are migrated; global-keyed (`GLOBAL_XLAYER_ID`) baselines keep the
        // documented `cvs_generation` approximation (re-stamping them could promote an
        // intentionally under-reported cross-CMVS advisory to an error). The migration
        // never compares; it only re-scopes, so it cannot itself emit a diagnostic.
        let migrated_epoch = self.cvs.cvs_generation_epoch(xlayer);
        for (key, baseline) in self.ops_buffer_delay_sums.iter_mut() {
            if key.xlayer == xlayer && baseline.tu_index == tu_index {
                baseline.scope.cvs_epoch = migrated_epoch;
            }
        }
        // The scan-type scopes flush before the content-interpretation pruning
        // below: the § 6.16.10 unestablished-CI warning for the ENDING coded video
        // sequence is evaluated against the records still present (mostly the
        // ending sequence's; a same-temporal-unit record that already joined the
        // new sequence may suppress the warning — an acceptable lenient
        // approximation for a warning-severity derived diagnostic).
        self.flush_scan_type_scope(xlayer, tu_index, report);
        if !xlayer.is_global() {
            self.flush_scan_type_scope(GLOBAL_XLAYER_ID, tu_index, report);
        }
        // § 6.16.7 timecode state: a CLK starts a new coded video sequence for THIS
        // extended layer at this temporal unit (§ 7.3.6). The prune is target-aware
        // (finding 2): it drops only the earlier-temporal-unit n_frames observations
        // and inference chains whose coded video sequence actually restarted — a CLK
        // for one extended layer leaves a global-bucket observation aimed at another
        // extended layer untouched. Same-temporal-unit observations joined the new
        // sequence and stay. (No flush: the timecode checks are eager, never deferred
        // to a flush.) Run BEFORE the content-interpretation migration below so the
        // re-pair afterwards sees the post-RAP observations only.
        self.prune_timecode_scope(xlayer, tu_index);
        // A deferred inference-presence diagnostic whose seed came from an earlier
        // temporal unit now fires: this CLK put the omitting timecode in a new coded
        // video sequence, detaching the seed (§ 7.3.6 / finding 2/3).
        self.emit_pending_timecode_inference(xlayer, report);
        self.sequence_fingerprints
            .retain(|(x, _), &mut (_, record_tu)| {
                !(*x == xlayer || x.is_global()) || record_tu >= tu_index
            });
        self.content_interpretations.retain(|(x, _), record| {
            !(*x == xlayer || x.is_global()) || record.tu_index >= tu_index
        });
        self.metadata.reset_cvs(xlayer, tu_index);
        // An HDR baseline joins the coded video sequence of every extended layer
        // its association touches; the CLK drops earlier-temporal-unit baselines
        // that touch its extended layer (a Universal record touches every layer,
        // mirroring the global-record pruning of the other stores). Pruning a
        // multi-xlayer record at any of its layers' boundaries only drops
        // comparisons, never inventing one.
        self.hdr_baselines.retain(|record| {
            !record.association.touches_xlayer(xlayer) || record.tu_index >= tu_index
        });
        // AV2 § 6.16.5 / § 6.16.6: the "first coded picture of that embedded layer
        // in the coded video sequence" state is per coded video sequence, so a CLK
        // that starts a new coded video sequence for `xlayer` clears the
        // first-picture-seen flags for all of its embedded layers — the next coded
        // picture in the new sequence is again a first coded picture. (A CLK is a
        // coded frame, so its own observe_obu re-sets the flag afterwards.) Records
        // keyed under GLOBAL_XLAYER_ID never enter this set (frame-bearing OBUs are
        // non-global).
        self.embedded_layer_first_picture_seen
            .retain(|(record_xlayer, _), _| *record_xlayer != xlayer);
        // AV2 § 6.4.1 / § 7.3.6: the distinct-`obu_mlayer_id` count is scoped to "the coded
        // video sequence associated with this sequence header", which starts AT this
        // temporal unit (mirror `07-decoding-process.md` lines 604-606). The
        // same-temporal-unit OBUs observed before this CLK — canonically the § 7.3.8.1
        // resent-at-RAP sequence header (forced to obu_mlayer_id 0) — belong to the NEW
        // coded video sequence, so reset_cvs *re-attributes* the boundary temporal unit's
        // seen ids to it (exact re-attribution, not the former whole-state drop). A
        // pending exceedance counted into the ENDING coded video sequence's set whose
        // members spanned an earlier temporal unit is still dropped at the temporal-unit
        // flush via the first_tu deferral (see count_distinct_mlayer /
        // CvsTracker::flush_completed_tu).
        //
        // The § 6.4.1 exceedance comparison on the re-seeded set is NOT run here: this
        // boundary event fires from observe_cvs_boundary_events BEFORE
        // observe_frame_bearing_obu parses the CLK's frame header and activates the
        // header the CLK *references* (mirror `06-syntax-structures-semantics.md` lines
        // 445-447 scope the count to "the coded video sequence associated with this
        // sequence header" — for the NEW coded video sequence that is the CLK-activated
        // header, not the still-active outgoing one). Comparing against the outgoing
        // header here is a wrong-header comparison (PR #41 false positive: outgoing max 1,
        // CLK-activated max 2, re-seeded set count 2). The re-seeded set is therefore
        // compared against the CLK-activated header in observe_frame_bearing_obu's
        // activation path via retroactive_distinct_mlayer_check (anchored at the CLK's
        // extension byte, the same anchor this removed check used). Conservative miss: if
        // the CLK's frame header is unparsable or its referenced header cannot be resolved
        // in-band, no activation happens and the re-seeded-set check is skipped — a sound
        // false negative, since the correct SeqMaxMlayerCnt is then unknown.
        self.distinct_mlayer.reset_cvs(xlayer, tu_index);
    }

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
    fn observe_msdo(
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
        self.annex_a_iop
            .note_msdo(msdo.num_streams(), msdo.multistream_profile_idc, obu.offset);

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
            profile_idc: msdo.multistream_profile_idc,
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

    /// Observes a frame-bearing OBU — a tile-group OBU (tile group, switch, RAS
    /// frame) or a SEF / TIP / bridge frame — by parsing its frame-header prefix
    /// (best-effort) and running the HLS reference and sequence-activation checks
    /// (AV2 § 5.18.2 / § 7.3.8).
    fn observe_frame_bearing_obu(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !is_frame_bearing(obu.header.obu_type) {
            return;
        }

        // AV2 § 6.17.2: FirstPictureInTU is per extended layer ("the first frame
        // unit in a coded extended layer unit in a temporal unit"), so a frame in
        // another extended layer earlier in this temporal unit does not clear it.
        let first_picture_in_tu = self.first_picture_in_tu(obu.header.extended_layer_id);
        self.frames_seen_in_tu.insert(obu.header.extended_layer_id);

        // A parse failure is silent: a frame/tile-group payload the skeleton cannot
        // reach is not-yet-validated coverage, not a conformance error in this phase.
        let Some(prefix) = parse_frame_prefix(obu, first_picture_in_tu) else {
            return;
        };

        let resolved = self.resolve_frame_header_reference(&prefix, obu, options, report);

        // AV2 § 7.3.8.1: buffer this frame's in-band-resolved HLS references for the
        // random-access-point availability replay. Only in-band-resolved references are
        // buffered (so the replay predicate stays disjoint from the linear
        // `hls/unavailable-*` checks, and an externally-supplied reference is not
        // double-judged): `resolved` is the in-band sequence-header id, and a
        // `cur_mfh_id > 0` that resolves to an in-band multi-frame header is the frame's
        // § 7.3.8.7 MFH reference. The resolution captures each object's qualifying-resend
        // snapshot as of this reference (intra-temporal-unit order).
        self.note_frame_rap_references(&prefix, resolved, obu.header.extended_layer_id, obu.offset);

        // AV2 § 5.18.2: frame_header_info() calls load_sequence_header() for EVERY
        // frame (both cur_mfh_id == 0 and cur_mfh_id > 0), before the `if (keyFrame)`
        // block — so any parsed frame header, not only a CLK/OLK key frame, activates
        // the referenced sequence header for its extended layer, overriding the
        // OBU-order fallback. Only an in-band reference is activated (its layer limits
        // are modeled); an external reference already suppresses the layer-limit
        // checks.
        if let Some(seq_id) = resolved {
            let xlayer = obu.header.extended_layer_id;
            // Snapshot the prior activation state *before* it is overwritten below, so
            // the § 7.3.6 single-active-sequence-header check can compare against the
            // previous frame-confirmed activation.
            let prior_seq = self.active_sequence_by_xlayer.get(&xlayer).copied();
            let prior_frame_confirmed = self.frame_confirmed_xlayers.contains(&xlayer);
            let prior_activation_cvs = self.frame_confirmed_activation_cvs.get(&xlayer).copied();
            self.check_single_active_sequence_header(
                obu,
                seq_id,
                prior_seq,
                prior_frame_confirmed,
                prior_activation_cvs,
                options,
                report,
            );

            let previous = self.active_sequence_by_xlayer.insert(xlayer, seq_id);
            // A frame-header reference is the § 5.18.2 load_sequence_header path:
            // it *confirms* the layer's activation (the OBU-order fallback was a
            // guess), so the deferred § 6.10.7 / § 6.8.9 agreement checks become
            // decidable on the first confirmation even when the id is unchanged,
            // and again whenever the id changes.
            let newly_confirmed = self.frame_confirmed_xlayers.insert(xlayer);
            // Record the coded video sequence epoch of this frame-confirmed activation,
            // so a later activation can tell whether a CLK intervened (AV2 § 7.3.6).
            self.frame_confirmed_activation_cvs
                .insert(xlayer, self.cvs.cvs_epoch(xlayer));
            // Record the temporal unit of this frame-confirmed activation, so the § 6.8.2 /
            // § 6.6 DOH loops can scope to the current CMVS window (codex finding 3393129745).
            self.frame_confirmed_activation_tu
                .insert(xlayer, self.cvs.tu_index);
            if previous != Some(seq_id) || newly_confirmed {
                self.on_sequence_activation(xlayer, options, report);
            } else if obu.header.obu_type == ObuType::ClosedLoopKey {
                // AV2 § 7.3.6 / Annex A Table A.4: a CLK that re-references the
                // already-active header opens a new coded video sequence (§ 7.3.6) without
                // changing the activated id, so `on_sequence_activation` is skipped. Re-seed
                // the IOP window's pending facts from the active confirmed header so the new
                // coded video sequence's window is decidable from the header carried across
                // the boundary (lesson 9), matching the `is_clk` re-run of the distinct-mlayer
                // check below.
                self.note_annex_a_iop_activation(xlayer, options);
            }
            // AV2 § 6.4.1: compare this extended layer's accumulated distinct-obu_mlayer_id
            // count against the header that just activated, the moment it activates — the
            // § 5.18.2 load_sequence_header confirmation path. Two cases reach here:
            //   (1) a count accumulated before any header was active (the eager
            //       count_distinct_mlayer had no SeqMaxMlayerCnt to compare against), or
            //   (2) the re-seeded boundary-temporal-unit set this OBU's own CLK
            //       re-attributed to the new coded video sequence in observe_cvs_boundary_events
            //       (start_cvs_for_xlayer). Case (2) must run even when the CLK re-references
            //       the SAME already-frame-confirmed header (so the id is unchanged and
            //       `newly_confirmed` is false), because DistinctMlayerTracker::observe never
            //       re-yields an already-seen id and so the eager check cannot re-surface the
            //       re-seeded set — hence the `is_clk` term. Running here, after activation,
            //       compares against the CLK-activated header (the header "associated with"
            //       the new coded video sequence, mirror `06-syntax-structures-semantics.md`
            //       lines 445-447), not the outgoing header still active when the boundary
            //       event fired (PR #41 false positive). The activating frame's own
            //       obu_mlayer_id is counted afterward by observe_obu's count_distinct_mlayer,
            //       so an id already in the set yields nothing new and never triggers the eager
            //       comparison here. Suppressed under caller-provided external HLS for the same
            //       reason as the eager check: an out-of-band header may carry a SeqMaxMlayerCnt
            //       this validator does not model.
            let is_clk = obu.header.obu_type == ObuType::ClosedLoopKey;
            if previous != Some(seq_id) || newly_confirmed || is_clk {
                let external_hls_suppresses = matches!(
                    &options.external_hls,
                    ExternalHlsMode::Provided(set) if set.declares_any_sequence_header()
                );
                if !external_hls_suppresses {
                    // Anchor to the activating OBU's extension byte (obu.offset + 1,
                    // bit 0), the same idiom as the eager count_distinct_mlayer. For a CLK
                    // this is the same anchor the removed reset-time check used.
                    let byte_offset = obu.offset.saturating_add(1);
                    self.retroactive_distinct_mlayer_check(xlayer, byte_offset, report);
                }
            }
            // AV2 § 6.4.1: cross-extended-layer monotonic_output_order_flag agreement,
            // gated on the § 7.3.2 CMVS tracker being definitively inside a CMVS.
            self.check_monotonic_output_order_agreement(xlayer, obu.offset, options, report);

            // AV2 § 6.4.13 cross-CVS advisory: evaluate on EVERY frame-confirmed
            // activation, not only an id change or first confirmation. A same-id
            // reconfiguration across a coded-video-sequence boundary (legal at the
            // boundary, § 7.3.6) re-confirms the unchanged id, so the short-circuit above
            // would skip it; this check must still re-compare. A CLK starts the new coded
            // video sequence before its own frame header activates (boundary events run
            // first in `observe_obu`), so by here the CVS epoch is already the new one.
            // The comparison is idempotent within a coded video sequence (it overwrites
            // its baseline with the same sum at the same epoch).
            self.check_seq_buffer_delay_sum(
                obu.header.extended_layer_id,
                obu.offset,
                options,
                report,
            );

            // With the in-band active sequence header available, run the frame-header
            // core parser and emit the locally decidable § 6.17 diagnostics. Parsing
            // and the checks are silent on failure or on paths that need reference
            // state (AV2 § 6.17.2 / § 6.17.4 / § 6.4.6).
            //
            // A `cur_mfh_id > 0` frame derives FrameWidth/FrameHeight (and the
            // §5.18.7.1 segmentation arm) from its resolved multi-frame header on the
            // non-override path, so resolve that record with the shared §7.3.8.7
            // discipline and thread it in; without it the core parse stops before
            // frame_size() and the §6.17.2 MFH-dims / §6.17.7 tile / quant diagnostics
            // would be skipped for MFH-backed frames. An unresolvable MFH stays `None`,
            // preserving the early-stop (no guessing).
            let mfh_record = self.resolve_frame_mfh_record(obu, first_picture_in_tu, seq_id);
            if let Some(active_sequence) = self.sequence_headers.get(&seq_id) {
                frame_header_core_checks(
                    obu,
                    first_picture_in_tu,
                    active_sequence,
                    mfh_record,
                    FrameReferenceAvailability {
                        qm: &self.qm,
                        film_grain: &self.film_grain,
                    },
                    options,
                    report,
                );
            }
        }
    }

    /// Emits `hls/multiple-active-sequence-headers` (AV2 § 7.3.6) when a frame-confirmed
    /// activation of `new_seq` for `obu`'s extended layer follows an earlier
    /// frame-confirmed activation of a *different* sequence header within the *same*
    /// coded video sequence.
    ///
    /// AV2 § 7.3.6 (mirror `07-decoding-process.md` lines 613-616): "Within each
    /// extended layer, only one sequence header shall remain active for the duration of
    /// a coded video sequence, i.e., until a CLK is encountered for that extended layer.
    /// Additional sequence header OBUs with a different seq_header_id can be present in
    /// the bitstream but are not activated and have no effect on the decoding process
    /// until referenced by a subsequent CLK frame header."
    ///
    /// The four gates (design decision 5):
    /// 1. the prior activation for this extended layer was *frame-confirmed*
    ///    (`prior_frame_confirmed`) — an OBU-order fallback guess never fires the check,
    ///    because a guess a later frame can contradict could not be retracted;
    /// 2. no § 7.3.6 coded-video-sequence start intervened — the prior frame-confirmed
    ///    activation shares this activation's coded video sequence epoch
    ///    ([`CvsTracker::cvs_epoch`]); a CLK advances the epoch, so a re-activation
    ///    across a CLK (a legal new coded video sequence) does not match;
    /// 3. the newly activated `seq_header_id` differs from the prior one; and
    /// 4. caller-provided external HLS does not declare any sequence header — only a
    ///    *declared* external sequence header can be the out-of-band active header that
    ///    makes the in-band activation history unreliable. An external channel that
    ///    declares no sequence header (`Provided(ExternalHlsSet::new())`, or one
    ///    declaring only operating point sets) cannot supply an active header, so it does
    ///    not suppress (precedent: [`ValidatorContext::validate_active_sequence_limits`]).
    #[allow(clippy::too_many_arguments)]
    fn check_single_active_sequence_header(
        &self,
        obu: &ObuEnvelope<'_>,
        new_seq: SequenceHeaderId,
        prior_seq: Option<SequenceHeaderId>,
        prior_frame_confirmed: bool,
        prior_activation_cvs: Option<Option<u64>>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // Gate 4: caller-provided external HLS that declares a sequence header may supply
        // the active one out of band, making the in-band activation history unreliable.
        // An external channel that declares no sequence header cannot, so it does not
        // suppress (mirrors validate_active_sequence_limits' narrow gate).
        if external_declares_sequence_header(options) {
            return;
        }
        let xlayer = obu.header.extended_layer_id;
        // Gates 1 + 3: a prior *frame-confirmed* activation of a different sequence
        // header. `prior_activation_cvs` is `Some(epoch)` exactly when a prior
        // frame-confirmed activation was recorded; pair it with the frame-confirmed flag
        // and a recorded prior id.
        let (Some(prior_seq), true, Some(prior_epoch)) =
            (prior_seq, prior_frame_confirmed, prior_activation_cvs)
        else {
            return;
        };
        if prior_seq == new_seq {
            return;
        }
        // Gate 2: both activations are in the same coded video sequence (no CLK between
        // them advanced the epoch). The prior activation's recorded epoch — `None` for
        // the implicit pre-first-CLK coded video sequence — must equal the epoch now in
        // effect for this extended layer. A first-temporal-unit CLK gives `Some(0)`,
        // distinct from the pre-CLK `None`, so a re-activation across it does not match.
        if prior_epoch != self.cvs.cvs_epoch(xlayer) {
            return;
        }
        report.push(
            Diagnostic::error(
                "hls/multiple-active-sequence-headers",
                format!(
                    "obu_xlayer_id {} activates sequence header {} while sequence header {} is \
                     still active for the same coded video sequence; only one sequence header \
                     may remain active until a CLK starts a new coded video sequence",
                    xlayer.get(),
                    new_seq.get(),
                    prior_seq.get()
                ),
            )
            .with_spec_section("7.3.6")
            .with_byte_offset(obu.offset),
        );
    }

    /// Resolves a parsed frame header's sequence-header reference, emitting range and
    /// availability diagnostics (AV2 § 5.18.2 / § 7.3.8.6 / § 7.3.8.7). Returns the
    /// in-band-resolved `seq_header_id` for activation, or `None` when it is out of
    /// range, resolved only externally, or unavailable.
    fn resolve_frame_header_reference(
        &self,
        prefix: &FrameHeaderPrefix,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) -> Option<SequenceHeaderId> {
        if prefix.cur_mfh_id.is_zero() {
            // cur_mfh_id == 0: the frame references a sequence header directly.
            let raw = prefix.seq_header_id_in_frame_header?;
            if raw >= MAX_SEQ_NUM {
                report.push(frame_header_error(
                    "frame-header/seq-header-id-out-of-range",
                    "6.17",
                    obu,
                    format!(
                        "seq_header_id_in_frame_header {raw} must be less than MAX_SEQ_NUM \
                         ({MAX_SEQ_NUM})"
                    ),
                ));
                return None;
            }
            self.resolve_referenced_sequence_header(raw, obu, options, report)
        } else {
            // cur_mfh_id > 0: resolve the multi-frame header, then its sequence header.
            let cur = prefix.cur_mfh_id;
            if !cur.in_range() {
                report.push(frame_header_error(
                    "frame-header/cur-mfh-id-out-of-range",
                    "6.17",
                    obu,
                    format!(
                        "cur_mfh_id {} must be less than MAX_MFH_NUM ({MAX_MFH_NUM})",
                        cur.get()
                    ),
                ));
                return None;
            }
            let Some(record) = self.hls.multi_frame_header(cur) else {
                // AV2 § 7.3.8.7: a multi-frame header may be provided "by inclusion in
                // the bitstream or by provision through external means". The validator
                // models external sequence headers but does not yet model external
                // multi-frame headers, so under ExternalHlsMode::Provided an
                // out-of-band MFH could satisfy this reference — suppress the hard
                // error to avoid rejecting a conformant external-HLS stream. Under the
                // default (Disabled) there is no external means, so it is unavailable.
                // TODO(spec: AV2-7.3.8-HLS-AVAILABILITY): declare external multi-frame
                // headers in ValidationOptions instead of suppressing under Provided.
                if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    report.push(frame_header_unavailable_mfh(cur, obu));
                }
                return None;
            };
            let mfh_mlayer_id = record.mfh_mlayer_id;
            let mfh_tlayer_id = record.mfh_tlayer_id;
            let seq_raw = u32::from(record.mfh_seq_header_id.get());
            let resolved = self.resolve_referenced_sequence_header(seq_raw, obu, options, report);

            // AV2 § 7.3.8.7: "the layer dependency constraints TLayerDependencyMap
            // and MLayerDependencyMap are satisfied for the referenced multi-frame
            // header OBU", with the concrete predicate from § 6.17.2, evaluated
            // after the sequence header is loaded:
            // MLayerDependencyMap[obu_mlayer_id][MfhMLayerId[cur_mfh_id]] == 1 and
            // TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId[cur_mfh_id]]
            // == 1, where obu_{m,t}layer_id are the frame header's. Only an
            // in-band-resolved sequence header has modeled § 5.4.1 maps; an external
            // or unavailable resolution is skipped (the availability diagnostics own
            // those cases, and unmodeled maps must not produce false positives).
            if let Some(seq_id) = resolved
                && let Some(header) = self.sequence_headers.get(&seq_id)
            {
                let general = header.general;
                let frame_mlayer = obu.header.embedded_layer_id;
                let frame_tlayer = obu.header.temporal_layer_id;
                if !general
                    .mlayer_dependency_map
                    .depends_on(frame_mlayer, mfh_mlayer_id)
                {
                    report.push(frame_header_error(
                        "frame-header/mfh-mlayer-dependency-missing",
                        "7.3.8.7",
                        obu,
                        format!(
                            "frame header at obu_mlayer_id {} references multi-frame header {} \
                             recorded at obu_mlayer_id {}, but the loaded sequence header {}'s \
                             MLayerDependencyMap[{}][{}] is 0 (§ 6.17.2)",
                            frame_mlayer.get(),
                            cur.get(),
                            mfh_mlayer_id.get(),
                            seq_id.get(),
                            frame_mlayer.get(),
                            mfh_mlayer_id.get(),
                        ),
                    ));
                }
                if !general.tlayer_dependency_map.depends_on(
                    frame_mlayer,
                    frame_tlayer,
                    mfh_tlayer_id,
                ) {
                    report.push(frame_header_error(
                        "frame-header/mfh-tlayer-dependency-missing",
                        "7.3.8.7",
                        obu,
                        format!(
                            "frame header at obu_tlayer_id {} references multi-frame header {} \
                             recorded at obu_tlayer_id {}, but the loaded sequence header {}'s \
                             TLayerDependencyMap[{}][{}][{}] is 0 (§ 6.17.2)",
                            frame_tlayer.get(),
                            cur.get(),
                            mfh_tlayer_id.get(),
                            seq_id.get(),
                            frame_mlayer.get(),
                            frame_tlayer.get(),
                            mfh_tlayer_id.get(),
                        ),
                    ));
                }
            }
            resolved
        }
    }

    /// Resolves an in-range `seq_header_id` referenced by a frame header against the
    /// HLS store, emitting `hls/unavailable-sequence-header` (§ 7.3.8.6) — and the
    /// external-HLS advisory under the default — when unavailable. Returns the id only
    /// for in-band availability (so it can be activated; an external reference has no
    /// modeled layer limits).
    fn resolve_referenced_sequence_header(
        &self,
        seq_header_id: u32,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) -> Option<SequenceHeaderId> {
        match self.hls.resolve_sequence_header(seq_header_id, options) {
            HlsResolution::InBand => SequenceHeaderId::try_new(seq_header_id),
            HlsResolution::External => None,
            HlsResolution::Unavailable => {
                let external_note = if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    " (external HLS is disabled)"
                } else {
                    " in-band or through the supplied external HLS"
                };
                report.push(
                    Diagnostic::error(
                        "hls/unavailable-sequence-header",
                        format!(
                            "frame header references sequence header {seq_header_id}, but no \
                             sequence header with that id is available{external_note}"
                        ),
                    )
                    .with_spec_section("7.3.8.6")
                    .with_byte_offset(obu.offset),
                );
                if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    report.push(external_hls_disabled_advisory(seq_header_id, obu));
                }
                None
            }
        }
    }

    /// Observes a content-interpretation OBU: checks cross-embedded-layer timing
    /// consistency (AV2 § 6.4.12) and repeated-CI identity (AV2 § 6.14) within the
    /// coded video sequence of the OBU's extended layer (exact § 7.3.6 boundaries:
    /// a CLK boundary event drops earlier-temporal-unit records, and a comparison
    /// against an earlier temporal unit's record is deferred to the temporal-unit
    /// flush; see [`CvsTracker`]).
    ///
    /// Timing values are compared only between two present `timing_info()` values of
    /// different embedded layers within the same extended layer — exactly the
    /// § 6.4.12 "within a coded video sequence ... across all embedded layers"
    /// scope, since a coded video sequence belongs to one extended layer (AV2 § 2).
    ///
    /// Also re-evaluates the stored § 6.16.10 scan-type observations against the
    /// new record (see [`ValidatorContext::recheck_scan_type_after_ci`]).
    fn observe_content_interpretation(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures are reported by the stateless ContentInterpretationSyntax
        // check; here we only act on a successful parse.
        let Ok(content_interpretation) = parse_content_interpretation(&mut reader) else {
            return;
        };

        let xlayer = obu.header.extended_layer_id;
        let mlayer = obu.header.embedded_layer_id;
        let tu_index = self.cvs.tu_index;

        // AV2 § 7.3.6 (mirror lines 560-562): record this CI's `(xlayer, mlayer)` presence in
        // the current temporal unit for the first-CELU-of-the-sequence presence judgment,
        // resolved at the temporal-unit boundary (round-6 F3). A CI belongs to a coded
        // extended layer unit, which is per non-global extended layer (§ 7.3.6); a global CI
        // is not part of any CELU, so it is excluded. The first appearance's offset anchors
        // the diagnostic.
        if !xlayer.is_global() {
            self.ci_observed_in_tu
                .entry((xlayer, mlayer))
                .or_insert(obu.offset);
        }

        // Cross-embedded-layer timing consistency: compare this layer's timing
        // against the first other embedded layer (same extended layer) that already
        // carries present timing within this CVS scope.
        if let Some(new_timing) = content_interpretation.timing_info
            && let Some((existing_mlayer, existing_timing, existing_tu)) = self
                .content_interpretations
                .iter()
                .find(|((x, m), record)| {
                    *x == xlayer && *m != mlayer && record.content.timing_info.is_some()
                })
                .and_then(|((_, m), record)| {
                    record.content.timing_info.map(|t| (*m, t, record.tu_index))
                })
        {
            for diagnostic in compare_timing_across_embedded_layers(
                existing_mlayer,
                &existing_timing,
                &new_timing,
                obu,
            ) {
                self.cvs
                    .defer_or_emit(xlayer, existing_tu, diagnostic, report);
            }
        }

        // A content interpretation can arrive after the scan-type metadata whose
        // Table 6.18 restrictions it decides (AV2 § 6.16.10); re-evaluate the stored
        // observations of this scope and the global bucket — unless an existing record
        // at the same (xlayer, mlayer) key already carries identical Table 6.18-decisive
        // content AND is still the post-epoch authority for it. In that case every
        // stored observation has already been paired against that content (at
        // metadata-observation time by check_scan_type_consistency, or by the recheck
        // that ran when the record's decisive content last changed), so re-evaluating
        // would only duplicate reports for the identical repeats § 6.14 explicitly
        // allows. A content interpretation for a NEW key, or one whose decisive content
        // changed (itself flagged by content-interpretation/repeated-ci-not-identical
        // below), forms genuinely new (observation, CI-content) pairs and is
        // re-evaluated.
        //
        // EPOCH-AWARE dedup (finding 1, unified model). The predicate is
        // content-identical AND no § 7.3.8.11 random-access-point reinit occurred
        // *after* the existing record's temporal unit (`existing.tu_index >=
        // ci_rap_epoch`). Two cases the prior temporal-unit-identity predicate got
        // wrong:
        //
        //   - An ORDINARY identical repeat in a LATER temporal unit with no intervening
        //     RAP: the existing record is still the post-epoch authority that already
        //     paired (and reported) the observations, so re-running the recheck would
        //     re-report them. `existing.tu_index >= ci_rap_epoch` holds (no RAP since),
        //     so this dedups — the over-correction the temporal-unit-only predicate
        //     caused is gone.
        //   - A RAP temporal unit re-sends an identical CI BEFORE its CLK: the pre-RAP
        //     record is stale (its observations belong to the ending epoch). At CI-time
        //     the epoch has not advanced yet (the CLK follows), so this predicate ALSO
        //     dedups here — but the re-pair of the new epoch's observations is then done
        //     at the CLK, once observe_ci_rap has advanced the epoch and dropped the
        //     stale deferred pairings (see repair_post_rap_ci_pairings). That keeps the
        //     RAP-resent re-pair correct without an eager re-pair that the lagging epoch
        //     cannot soundly authorize.
        let decisive_content_unchanged = self
            .content_interpretations
            .get(&(xlayer, mlayer))
            .is_some_and(|existing| {
                existing.tu_index >= self.ci_rap_epoch(xlayer)
                    && scan_type_decisive_content(&existing.content)
                        == scan_type_decisive_content(&content_interpretation)
            });
        if !decisive_content_unchanged {
            // Eager CI-after-metadata re-pair (`repair = false`): the eager-emission skip
            // (the scan-type analogue of the round-7 timecode finding 2) applies only to
            // the RAP re-pair.
            self.recheck_scan_type_after_ci(
                xlayer,
                mlayer,
                &content_interpretation,
                obu.offset,
                false,
                report,
            );
        }
        // Finding 1 (CLK re-pair filter): record whether the scan-type recheck was
        // SUPPRESSED here. Only a suppressed re-send (a pre-RAP-identical copy whose
        // recheck the lagging epoch skipped) is re-paired at the CLK/OLK by
        // repair_post_rap_ci_pairings; a re-send that CHANGED the decisive content
        // rechecked eagerly just above, so re-pairing it would duplicate the diagnostic.
        let scan_type_recheck_suppressed = decisive_content_unchanged;

        // AV2 § 6.16.7: a content interpretation establishing ci_timing_info_present_flag
        // / timing may arrive after the timecode metadata whose n_frames bound it
        // decides; re-evaluate the stored timecode observations — but only when the
        // n_frames-decisive content (the timing_info) differs from the record this CI
        // replaces OR the existing record is no longer the post-epoch authority,
        // mirroring the scan-type dedup so a repeated identical CI (the only legal
        // repeat, § 6.14) never re-reports.
        //
        // EPOCH-AWARE dedup (finding 1, unified model — identical to the scan-type guard
        // above). `existing.tu_index >= ci_rap_epoch` means no § 7.3.8.11 random access
        // point reinitialized the parameters after the existing record's temporal unit,
        // so the existing record is still the authority that already paired (and
        // reported) every observation against this timing: re-running the recheck would
        // duplicate those reports for an ordinary identical repeat in a later temporal
        // unit (the over-correction the temporal-unit-only predicate caused). When a RAP
        // re-sends an identical CI BEFORE its CLK the epoch has not advanced yet, so this
        // dedups at CI-time too; the new epoch's observations are re-paired at the CLK
        // (see repair_post_rap_ci_pairings) after observe_ci_rap advances the epoch and
        // drops the stale deferred pairings.
        let timing_unchanged = self
            .content_interpretations
            .get(&(xlayer, mlayer))
            .is_some_and(|existing| {
                existing.tu_index >= self.ci_rap_epoch(xlayer)
                    && existing.content.timing_info == content_interpretation.timing_info
            });
        if !timing_unchanged {
            // Eager CI-after-timecode re-pair (`repair = false`): the round-7 finding 2
            // eager-emission skip applies only to the RAP re-pair below.
            self.recheck_timecode_n_frames_after_ci(
                xlayer,
                mlayer,
                &content_interpretation,
                obu.offset,
                false,
                report,
            );
        }
        // Finding 1 (CLK re-pair filter, the n_frames analogue of
        // `scan_type_recheck_suppressed`): a re-send whose timing CHANGED rechecked
        // eagerly just above, so repair_post_rap_ci_pairings must NOT re-pair it (that
        // would duplicate the diagnostic); only a suppressed identical re-send is
        // re-paired at the CLK/OLK.
        let timecode_recheck_suppressed = timing_unchanged;

        match self.content_interpretations.entry((xlayer, mlayer)) {
            Entry::Vacant(slot) => {
                slot.insert(ContentInterpretationRecord {
                    content: content_interpretation,
                    offset: obu.offset,
                    tu_index,
                    scan_type_recheck_suppressed,
                    timecode_recheck_suppressed,
                });
            }
            Entry::Occupied(mut slot) => {
                let existing = slot.get();
                // AV2 § 6.14: a repeated CI OBU for the same embedded layer within a
                // CVS must carry the same *information* (a weaker requirement than the
                // sequence header's bit-identity in § 7.3.6). Each compared field
                // resolves to a canonical value (incl. unspecified defaults for absent
                // color/aspect), so any observation is a complete baseline; the
                // decoder-ignored ci_reserved_2bit is excluded.
                if content_interpretation_information_differs(
                    &existing.content,
                    &content_interpretation,
                ) {
                    let diagnostic = Diagnostic::error(
                        "content-interpretation/repeated-ci-not-identical",
                        format!(
                            "content interpretation OBU for obu_xlayer_id {} / obu_mlayer_id {} \
                             is repeated within the coded video sequence with different \
                             information (previous copy at byte {})",
                            xlayer.get(),
                            mlayer.get(),
                            existing.offset
                        ),
                    )
                    .with_spec_section("6.14")
                    .with_byte_offset(obu.offset);
                    self.cvs
                        .defer_or_emit(xlayer, existing.tu_index, diagnostic, report);
                }
                // Refresh the record to this latest appearance: § 7.3.6 starts a new
                // coded video sequence *at the temporal unit*, so a copy re-sent in a
                // CLK temporal unit must survive the CLK pruning as the new coded
                // video sequence's baseline (a differing repeat also becomes the new
                // baseline after being routed above). The suppression flags carry
                // whether THIS appearance's rechecks were skipped by the dedup guard
                // (finding 1), so the CLK/OLK re-pair touches only an identical re-send.
                slot.insert(ContentInterpretationRecord {
                    content: content_interpretation,
                    offset: obu.offset,
                    tu_index,
                    scan_type_recheck_suppressed,
                    timecode_recheck_suppressed,
                });
            }
        }
    }

    /// Resolves the § 7.3.6 first-CELU-of-the-sequence CI PRESENCE judgment (mirror lines
    /// 560-562, round-6 F3) for the just-completed temporal unit `completed_tu_index`. Called
    /// at each global-temporal-delimiter boundary and at the end of the bitstream, after the
    /// CLK boundary events of the temporal unit have been applied (so the CVS the temporal
    /// unit belongs to is final — the whole temporal unit containing a CLK belongs to the new
    /// coded video sequence, § 7.3.6). Drains [`Self::ci_observed_in_tu`].
    ///
    /// For each `(xlayer, mlayer)` CI observed in the temporal unit:
    ///
    /// - Under an external-HLS `Provided` mode the judgment DROPS: an external CI in the first
    ///   CELU cannot be enumerated by [`crate::options::ExternalHlsSet`] (it expresses only
    ///   sequence headers and operating point sets), so the validator cannot prove the first
    ///   CELU lacked the CI — consistent with the partial-declaration suppression policy.
    /// - If the layer's coded video sequence start was not observed (`first_celu_tu` is `None`
    ///   — a mid-CVS join, no CLK seen) the judgment DROPS: the first CELU's CI set is
    ///   unknowable (documented Unknown-first-CELU drop, see [`CiFirstCeluState`]).
    /// - If this temporal unit IS the CVS's first temporal unit (`completed_tu_index ==
    ///   first_celu_tu`), the CI is in the first CELU — record `mlayer` as present there.
    /// - Otherwise the CI is in a LATER CELU: if `mlayer` was absent from the first CELU's CI
    ///   set (and not already reported this CVS), fire `celu/content-interpretation-not-in-
    ///   first-celu`, anchored at the offending CI, and dedup per `(xlayer, mlayer, CVS epoch)`.
    fn resolve_ci_first_celu_for_tu(
        &mut self,
        completed_tu_index: u64,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let observed = std::mem::take(&mut self.ci_observed_in_tu);
        // External HLS (any Provided mode): an external CI the set cannot enumerate may be the
        // first CELU's CI, so the presence judgment is not decidable — drop wholesale. The
        // per-TU buffer is still drained above so it does not leak into the next temporal unit.
        if matches!(options.external_hls, ExternalHlsMode::Provided(_)) {
            return;
        }
        for ((xlayer, mlayer), offset) in observed {
            let state = self.ci_first_celu.entry(xlayer).or_default();
            let Some(first_celu_tu) = state.first_celu_tu else {
                // No CLK established the CVS start for this layer (mid-CVS join): the first
                // coded extended layer unit of the sequence was not observed, so the presence
                // judgment is undecidable — drop (documented Unknown-first-CELU drop).
                continue;
            };
            if completed_tu_index == first_celu_tu {
                // This temporal unit is the CVS's first temporal unit, so this CI is in the
                // first coded extended layer unit of the sequence — record the embedded layer.
                state.first_celu_ci_mlayers.insert(mlayer);
            } else if !state.first_celu_ci_mlayers.contains(&mlayer)
                && state.reported.insert(mlayer)
            {
                // A later coded extended layer unit carries a CI for an embedded layer the
                // sequence's first CELU lacked (§ 7.3.6 lines 560-562). Dedup per
                // (xlayer, mlayer, CVS epoch) via `reported`.
                report.push(
                    Diagnostic::error(
                        "celu/content-interpretation-not-in-first-celu",
                        format!(
                            "OBU_CONTENT_INTERPRETATION is present for obu_xlayer_id {} / \
                             obu_mlayer_id {} in a coded extended layer unit that is not the \
                             first coded extended layer unit of the coded video sequence, but \
                             the first coded extended layer unit of the sequence carried no \
                             content interpretation for that embedded layer; § 7.3.6 requires a \
                             CI present in any coded extended layer unit to also be present in \
                             the first coded extended layer unit of the sequence",
                            xlayer.get(),
                            mlayer.get()
                        ),
                    )
                    .with_spec_section("7.3.6")
                    .with_byte_offset(offset),
                );
            }
        }
    }

    /// Observes a metadata OBU (short or group form), feeding the § 6.16.3
    /// persistence / cancellation lifetime store and the § 6.16.5 / § 6.16.6 HDR
    /// repeat-content baselines. A parse failure is silent, matching
    /// `observe_content_interpretation` — the stateless `MetadataSyntax` check owns
    /// failure reporting.
    fn observe_metadata(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        match obu.header.obu_type {
            ObuType::MetadataShort => {
                let Ok(short) = parse_metadata_short(&mut reader, obu.payload.len()) else {
                    return;
                };
                if short.muh_cancel_flag {
                    // A short-form cancel unit DOES carry muh_layer_idc and
                    // muh_persistence_idc — § 5.17.2 reads them before the
                    // muh_cancel_flag early return — unlike the group form, whose
                    // cancel units skip every muh_* field but metadata_type
                    // (§ 5.17.3). The asymmetry is syntactic only: § 6.16.3 keys
                    // cancellation on the metadata type and the OBU's extended
                    // layer alone, so the extra short-form fields carry no cancel
                    // semantics.
                    self.metadata
                        .cancel(obu.header.extended_layer_id, short.metadata_type.value());
                    return;
                }
                let Some(unit) = short.unit else {
                    return;
                };
                self.observe_metadata_unit(
                    obu,
                    MetadataUnitHeader {
                        layer_idc: short.muh_layer_idc,
                        persistence_idc: short.muh_persistence_idc,
                        collapsed_mlayer_map: None,
                        xlayer_map: None,
                        mlayer_maps: &[],
                    },
                    unit,
                    report,
                );
            }
            ObuType::MetadataGroup => {
                let Ok(group) = parse_metadata_group(&mut reader, obu.header.extended_layer_id)
                else {
                    return;
                };
                for group_unit in group.units {
                    if group_unit.muh_cancel_flag {
                        // Group-form cancel units carry only metadata_type
                        // (§ 5.17.3; see the short-form arm for the asymmetry).
                        self.metadata.cancel(
                            obu.header.extended_layer_id,
                            group_unit.metadata_type.value(),
                        );
                        continue;
                    }
                    let (Some(layer_idc), Some(persistence_idc), Some(unit)) = (
                        group_unit.muh_layer_idc,
                        group_unit.muh_persistence_idc,
                        group_unit.unit,
                    ) else {
                        continue;
                    };
                    // Collapsed LAYER_VALUES targeting for the § 6.16.3 lifetime
                    // store: the single muh_mlayer_map byte of the local form, or
                    // of a global form whose muh_xlayer_map selected exactly one
                    // extended layer (§ 5.17.3). A global unit with several
                    // per-xlayer maps has no single-byte representation, so the
                    // lifetime store does not model its explicit targeting in
                    // this phase; the § 6.16.5 / § 6.16.6 HDR association uses
                    // the full maps instead (see derive_hdr_association).
                    let collapsed_mlayer_map = match group_unit.muh_mlayer_maps.as_slice() {
                        [single] => Some(*single),
                        _ => None,
                    };
                    self.observe_metadata_unit(
                        obu,
                        MetadataUnitHeader {
                            layer_idc,
                            persistence_idc,
                            collapsed_mlayer_map,
                            xlayer_map: group_unit.muh_xlayer_map,
                            mlayer_maps: &group_unit.muh_mlayer_maps,
                        },
                        unit,
                        report,
                    );
                }
            }
            _ => {}
        }
    }

    /// Feeds one parsed non-cancel metadata unit into the § 6.16.3 lifetime store,
    /// after running the § 6.16.5 / § 6.16.6 HDR repeat-content check.
    fn observe_metadata_unit(
        &mut self,
        obu: &ObuEnvelope<'_>,
        header: MetadataUnitHeader<'_>,
        unit: MetadataUnit,
        report: &mut ValidationReport,
    ) {
        self.check_hdr_repeat_content(obu, &header, &unit, report);
        if let MetadataPayload::ScanType(scan) = &unit.payload {
            self.check_scan_type_consistency(obu, scan, report);
        }
        if let MetadataPayload::Timecode(timecode) = &unit.payload {
            // The § 6.16.3 layer targeting scopes the n_frames bound's pairing to the
            // content interpretation OBUs of the layers this timecode describes
            // (finding 4); `None` when targeting is not bitstream-derivable.
            let targeting = derive_hdr_association(obu, &header);
            self.check_timecode_consistency(obu, timecode, targeting, report);
        }

        self.metadata.observe_unit(
            obu.header.extended_layer_id,
            ActiveMetadataUnit {
                metadata_type: unit.metadata_type,
                persistence: PersistenceMode::from_idc(header.persistence_idc),
                layer_idc: header.layer_idc,
                source_mlayer: obu.header.embedded_layer_id,
                source_tlayer: obu.header.temporal_layer_id,
                mlayer_map: header.collapsed_mlayer_map,
                payload: unit.payload,
                offset: obu.offset,
                tu_index: self.cvs.tu_index,
            },
        );
    }

    /// Checks the § 6.16.5 / § 6.16.6 repeated-content rule for an HDR CLL / MDCV
    /// metadata unit: "Any additional metadata_hdr_cll \[metadata_hdr_mdcv\]
    /// metadata units associated with an embedded layer in a coded video sequence
    /// shall have the same content." The unit's embedded-layer association set is
    /// derived from its § 6.16.3 layer targeting (see [`derive_hdr_association`])
    /// and the unit is compared against every stored baseline of the same
    /// metadata type whose association set intersects it — sharing an embedded
    /// layer is exactly when the rule binds the two units, independent of how
    /// each encoded its targeting (e.g. a global `LAYER_GLOBAL` unit against a
    /// later `LAYER_CURRENT` unit for one concrete embedded layer). Disjoint
    /// associations are never compared — the § 6.16.5 / § 6.16.6
    /// inheritance/override sentence shows different embedded layers may
    /// legitimately carry different content — and units with no
    /// bitstream-derivable association (see [`HdrAssociation`]) enter no
    /// comparison and no baseline. The comparison is independent of § 6.16.3
    /// cancellation (the rule has no cancel exception); the CVS scope is exact
    /// per § 7.3.6 (a CLK boundary event prunes earlier-temporal-unit baselines
    /// touching its extended layer, and a comparison against an earlier temporal
    /// unit's baseline is deferred to the temporal-unit flush; see
    /// [`CvsTracker`]).
    ///
    /// The other half of § 6.16.5 / § 6.16.6 — "metadata associated with an
    /// embedded layer, when present, shall be indicated at the first coded picture
    /// of that embedded layer in the coded video sequence" — is now enforced for
    /// the sound subset (see [`Self::check_hdr_first_coded_picture`]): an
    /// explicit-pair-targeted HDR CLL / MDCV unit that *first establishes* its
    /// content after every named embedded layer's first coded picture of the coded
    /// video sequence has already passed. `XLayerWide` / `Universal` targeting and
    /// the color-inheritance refinement stay deferred to avoid false positives.
    fn check_hdr_repeat_content(
        &mut self,
        obu: &ObuEnvelope<'_>,
        header: &MetadataUnitHeader<'_>,
        unit: &MetadataUnit,
        report: &mut ValidationReport,
    ) {
        let is_mdcv = match unit.payload {
            MetadataPayload::HdrCll(_) => false,
            MetadataPayload::HdrMdcv(_) => true,
            _ => return,
        };
        let Some(association) = derive_hdr_association(obu, header) else {
            return;
        };
        let tu_index = self.cvs.tu_index;
        // § 6.16.5 / § 6.16.6 first-coded-picture half: a baseline of the same type
        // whose association includes a given embedded layer means the content was
        // already established for *that* layer earlier in the coded video sequence, so
        // this unit is an allowed (later) repeat for it — not a fresh first
        // establishment. The rule binds PER associated embedded layer (finding 4), so
        // the "already established" gate is applied per (obu_xlayer_id, obu_mlayer_id)
        // pair: a unit targeting {an established layer + a NEW layer} is still checked
        // for the new layer. (`check_hdr_first_coded_picture` only inspects explicit
        // pairs; for `XLayerWide` / `Universal` targeting it returns early regardless,
        // so the per-pair filter is a no-op there.)
        self.check_hdr_first_coded_picture(obu, &association, is_mdcv, report);
        for record in &self.hdr_baselines {
            if record.is_mdcv != is_mdcv
                || record.payload == unit.payload
                || !record.association.intersects(&association)
            {
                continue;
            }
            let (rule_id, spec_section, unit_name) = if is_mdcv {
                (
                    "metadata/hdr-mdcv-repeat-content-differs",
                    "6.16.6",
                    "metadata_hdr_mdcv",
                )
            } else {
                (
                    "metadata/hdr-cll-repeat-content-differs",
                    "6.16.5",
                    "metadata_hdr_cll",
                )
            };
            let diagnostic = Diagnostic::error(
                rule_id,
                format!(
                    "{unit_name} metadata associated with {} is repeated within the coded \
                     video sequence with different content (previous copy at byte {})",
                    describe_hdr_intersection(&record.association, &association),
                    record.offset
                ),
            )
            .with_spec_section(spec_section)
            .with_byte_offset(obu.offset);
            self.cvs.defer_or_emit(
                hdr_intersection_scope(&record.association, &association),
                record.tu_index,
                diagnostic,
                report,
            );
        }
        // Refresh or append the baseline: § 7.3.6 starts a new coded video
        // sequence *at the temporal unit*, so a unit re-sent in a CLK temporal
        // unit must survive the CLK pruning as the new coded video sequence's
        // baseline (a differing repeat also becomes the new baseline after being
        // routed above), matching the sequence-fingerprint and
        // content-interpretation stores. A same-association record is overwritten
        // in place; a new association appends its own baseline.
        if let Some(record) = self
            .hdr_baselines
            .iter_mut()
            .find(|record| record.is_mdcv == is_mdcv && record.association == association)
        {
            record.payload = unit.payload.clone();
            record.offset = obu.offset;
            record.tu_index = tu_index;
        } else {
            self.hdr_baselines.push(HdrBaselineRecord {
                association,
                is_mdcv,
                payload: unit.payload.clone(),
                offset: obu.offset,
                tu_index,
            });
        }
    }

    /// Enforces the § 6.16.5 / § 6.16.6 "shall be indicated at the first coded
    /// picture of that embedded layer in the coded video sequence" rule for the
    /// sound subset (mirror `06-syntax-structures-semantics.md` lines 3687-3688 /
    /// 3736-3737).
    ///
    /// The § 6.16.5 / § 6.16.6 requirement binds **independently per associated
    /// embedded layer**, so the check is applied **per named pair** and fires when
    /// **any** `(obu_xlayer_id, obu_mlayer_id)` pair *first establishes* its content
    /// (no prior same-type baseline already includes that pair) after that pair has
    /// already passed its first coded picture of the coded video sequence — not only
    /// when every pair is late (finding 6), and not gated away when a *different*
    /// targeted pair was already established by an earlier unit (finding 4: a unit
    /// targeting {an established layer + a NEW layer} must still fire for the new
    /// layer). The check requires **explicit-pair** targeting (`LAYER_CURRENT` /
    /// `LAYER_VALUES`); `XLayerWide` / `Universal` targeting names no single concrete
    /// first coded picture, so it is skipped (zero-false-positive). A pair observed in
    /// the pre-frame region of its layer's first coded frame unit has no first-picture
    /// entry yet, so it is not late; a pair a prior baseline already established is an
    /// allowed later repeat, not a fresh establishment, so it is filtered out. A
    /// **suffix** metadata (`metadata_is_suffix == 1`) is placed by § 7.3.3 in the tail
    /// *after* the coded frame but still inside the same coded frame unit, so when it
    /// falls within the layer's first coded frame unit of this temporal unit (the
    /// segmenter reports no completed unit for the pair yet) it is "indicated at the
    /// first coded picture" and is not late — the predicate keys on coded-frame-UNIT
    /// boundaries, not first-frame-OBU order.
    ///
    /// Each late pair's first picture carries the temporal-unit index at which it was
    /// observed. A same-temporal-unit first picture is unambiguously in the current
    /// coded video sequence (§ 7.3.6: a CVS starts at a temporal unit, never inside
    /// one), so the finding is emitted eagerly. A first picture from an *earlier*
    /// temporal unit may belong to a previous CVS — a CLK later in this temporal unit
    /// would start a new CVS for its extended layer and re-establish its first-picture
    /// state — so the finding is **deferred** to the temporal-unit flush via
    /// [`CvsTracker::defer_or_emit`] (finding 2: stale previous-CVS first-picture state
    /// must not fire on a new CVS's first unit).
    fn check_hdr_first_coded_picture(
        &mut self,
        obu: &ObuEnvelope<'_>,
        association: &HdrAssociation,
        is_mdcv: bool,
        report: &mut ValidationReport,
    ) {
        let Some(pairs) = association.explicit_embedded_pairs() else {
            return;
        };
        let current_tu = self.cvs.tu_index;
        // Partition the named pairs that are *late* (their first coded picture has
        // already passed) into same-TU (eager, definitely the current CVS) and
        // earlier-TU (deferred, possibly a previous CVS) groups. A pair a prior
        // same-type baseline already includes is filtered first (finding 4): it was
        // established earlier in the coded video sequence, so this unit is an allowed
        // later repeat for that pair, not a fresh first establishment — the per-pair
        // gate, replacing the former whole-unit `any(intersects)` suppression.
        // § 7.3.3 places the suffix-metadata tail *after* the coded frame but still
        // inside the same coded frame unit. So a suffix metadata (`metadata_is_suffix
        // == 1`) appearing after the first coded picture's OBUs, yet within that
        // picture's own coded frame unit, is "indicated at the first coded picture" —
        // it is NOT late. The lateness predicate therefore keys on coded-frame-UNIT
        // boundaries, not first-frame-OBU order: a suffix metadata is timely when the
        // segmenter reports the embedded layer is still within its first coded frame
        // unit of this temporal unit (no unit completed yet). A prefix metadata
        // (`Some(false)`) heads a *new* unit, so the same-unit grace does not apply;
        // and a coded frame unit never spans temporal units (§ 7.3.7), so this grace is
        // scoped to the same temporal unit as the first picture (`seen_tu ==
        // current_tu`), where the completed-unit count is reliable.
        let is_suffix_metadata = metadata_is_suffix(obu) == Some(true);
        let mut eager_late: Vec<(ExtendedLayerId, EmbeddedLayerId)> = Vec::new();
        let mut deferred_late: Vec<((ExtendedLayerId, EmbeddedLayerId), u64)> = Vec::new();
        for &pair in pairs {
            let already_established = self.hdr_baselines.iter().any(|record| {
                record.is_mdcv == is_mdcv
                    && record.association.includes_embedded_pair(pair.0, pair.1)
            });
            if already_established {
                continue;
            }
            let Some(&seen_tu) = self.embedded_layer_first_picture_seen.get(&pair) else {
                continue;
            };
            if seen_tu == current_tu {
                // Same-temporal-unit first picture. A suffix metadata still inside the
                // layer's first coded frame unit of this temporal unit (no completed
                // unit yet) is in the same unit as the first picture, so it is timely.
                if is_suffix_metadata
                    && self
                        .frame_unit
                        .completed_units_for_embedded_layer(pair.0, pair.1)
                        == 0
                {
                    continue;
                }
                eager_late.push(pair);
            } else {
                deferred_late.push((pair, seen_tu));
            }
        }
        if eager_late.is_empty() && deferred_late.is_empty() {
            return;
        }
        let (rule_id, spec_section, unit_name) = if is_mdcv {
            (
                "metadata/hdr-mdcv-first-coded-picture",
                "6.16.6",
                "metadata_hdr_mdcv",
            )
        } else {
            (
                "metadata/hdr-cll-first-coded-picture",
                "6.16.5",
                "metadata_hdr_cll",
            )
        };
        let build = |late: &[(ExtendedLayerId, EmbeddedLayerId)]| {
            Diagnostic::error(
                rule_id,
                format!(
                    "{unit_name} metadata first establishes content for {} after that embedded \
                     layer's first coded picture of the coded video sequence; it shall be \
                     indicated at the first coded picture",
                    describe_embedded_pairs(late)
                ),
            )
            .with_spec_section(spec_section)
            .with_byte_offset(obu.offset)
        };
        // Same-TU late pairs: emit one eager finding naming them all.
        if !eager_late.is_empty() {
            report.push(build(&eager_late));
        }
        // Earlier-TU late pairs: defer each on its own extended layer, so a CLK that
        // starts a new CVS for that layer in the current temporal unit drops the stale
        // finding at the flush (the pair's first picture was in the previous CVS).
        for (pair, seen_tu) in deferred_late {
            self.cvs
                .defer_or_emit(pair.0, seen_tu, build(std::slice::from_ref(&pair)), report);
        }
    }

    /// Folds one non-cancel `metadata_scan_type()` unit into the § 6.16.10 CVS
    /// consistency state and runs the Table 6.18 checks (AV2 § 6.16.10,
    /// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-10`):
    ///
    /// - **Group consistency**: "It is a requirement of bitstream conformance that
    ///   when mps_pic_struct_type is present that only one of the following
    ///   conditions, for all pictures in the current CVS, is true" (the three
    ///   [`PicStructGroup`] value sets). The new value's group is compared against
    ///   each in-scope group baseline (the scope's first observation in the coded
    ///   video sequence); a global-bucket unit is also compared against every
    ///   concrete extended-layer scope and vice versa, since global metadata
    ///   describes every layer's pictures.
    /// - **CI cross-check**: the Table 6.18 "Restrictions" column
    ///   ("ci_scan_type_idc shall be equal to" 1 / 2 / 3 per group) against every
    ///   in-scope content-interpretation record with an established non-zero
    ///   `ci_scan_type_idc`; an established value of 0 is Unspecified and decides
    ///   nothing (the scope-level absence is the
    ///   `metadata/scan-type-ci-scan-type-unestablished` warning at the CVS flush
    ///   instead). For values 7 and 8 additionally "equal_picture_interval shall
    ///   be equal to 1", checked against records carrying `timing_info()`; a
    ///   record without `timing_info()` is silently skipped for this half — the
    ///   mirror attaches the restriction to the signaled element and states no
    ///   absent-timing rule. A record from before its extended layer's most
    ///   recent random access point is skipped: § 7.3.8.11 re-initializes the
    ///   content interpretation parameters to defaults at each CLK / OLK temporal
    ///   unit, so a pre-epoch record no longer establishes the parameters this
    ///   picture sees (a record re-sent at or after the random access point
    ///   refreshes its temporal unit and re-enters pairing).
    ///
    /// Reserved values above 12 never enter the state ("Decoders shall ignore
    /// reserved values of mps_pic_struct_type", § 6.16.10). Comparisons against a
    /// baseline from an earlier temporal unit are routed through
    /// [`CvsTracker::defer_or_emit`], tagged with the baseline's owning scope, so
    /// the exact § 7.3.6 CVS boundary applies.
    ///
    /// `mps_source_scan_type_idc` is deliberately NOT cross-checked against
    /// `ci_scan_type_idc`: the mirror's complete semantics are
    /// "mps_source_scan_type_idc specifies the scan type with the same semantics
    /// as for ci_scan_type_idc" (§ 6.16.10) — no consistency requirement exists.
    fn check_scan_type_consistency(
        &mut self,
        obu: &ObuEnvelope<'_>,
        scan: &MetadataScanType,
        report: &mut ValidationReport,
    ) {
        let value = scan.mps_pic_struct_type;
        let Some(group) = PicStructGroup::from_pic_struct(value) else {
            return;
        };
        let scope_key = obu.header.extended_layer_id;
        let tu_index = self.cvs.tu_index;

        // Group consistency against the unit's own scope plus the paired
        // global / concrete scopes.
        for (key, scope) in &self.scan_type.scopes {
            if !(*key == scope_key || key.is_global() || scope_key.is_global()) {
                continue;
            }
            let Some((baseline, baseline_group)) = scope.group_baseline() else {
                continue;
            };
            if baseline_group != group {
                let diagnostic = Diagnostic::error(
                    "metadata/scan-type-pic-struct-group-inconsistent",
                    format!(
                        "mps_pic_struct_type {value} falls into Table 6.18 group {{{}}} but \
                         mps_pic_struct_type {} (at byte {}) established group {{{}}}; only one \
                         group is allowed for all pictures in the coded video sequence",
                        group.describe(),
                        baseline.mps_pic_struct_type,
                        baseline.offset,
                        baseline_group.describe(),
                    ),
                )
                .with_spec_section("6.16.10")
                .with_byte_offset(obu.offset);
                self.cvs
                    .defer_or_emit(*key, baseline.tu_index, diagnostic, report);
            }
        }

        // Table 6.18 CI cross-check against the in-scope content-interpretation
        // records already observed (a CI arriving later re-evaluates instead; see
        // recheck_scan_type_after_ci). Each record applies its own extended
        // layer's § 7.3.8.11 epoch (for the global bucket too — an epoch only
        // resets the CI parameters of its own extended layer).
        //
        // `eagerly_emitted` collects the CI identities whose same-temporal-unit in-scope
        // Table 6.18 restriction was decided HERE and emitted (not deferred) — i.e. an
        // identical CI was re-sent BEFORE this scan-type metadata in the same § 7.3.8.11
        // RAP temporal unit. The RAP re-pair (repair_post_rap_ci_pairings) skips exactly
        // those `(observation, CI)` pairs so the diagnostic is not emitted twice (the
        // scan-type analogue of the round-7 timecode finding 2), while still re-pairing
        // any OTHER CI for this observation. A pairing DEFERRED against an
        // earlier-temporal-unit (stale pre-RAP) CI does NOT enter the set: that deferred
        // diagnostic is dropped at the RAP, so the re-pair must still cover it.
        let required = group.required_ci_scan_type_idc();
        let mut eagerly_emitted = BTreeSet::new();
        for ((ci_xlayer, ci_mlayer), record) in &self.content_interpretations {
            if !(scope_key.is_global() || *ci_xlayer == scope_key) {
                continue;
            }
            if record.tu_index < self.ci_rap_epoch(*ci_xlayer) {
                continue;
            }
            let pair = ScanTypeCiPair {
                ci_xlayer: *ci_xlayer,
                ci_mlayer: *ci_mlayer,
                metadata_offset: obu.offset,
                ci_offset: record.offset,
                at: obu.offset,
            };
            // defer_or_emit emits eagerly iff the CI is in this temporal unit; a
            // same-temporal-unit emission is the case to skip in the RAP re-pair, keyed
            // by the CI's identity so only this exact pairing is skipped.
            let same_tu = record.tu_index == tu_index;
            let established = record.content.scan_type_idc.get();
            if established != 0 && established != required {
                let diagnostic = scan_type_ci_mismatch_error(value, required, established, &pair);
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
                if same_tu {
                    eagerly_emitted.insert((*ci_xlayer, *ci_mlayer));
                }
            }
            if matches!(value, 7 | 8)
                && let Some(timing) = record.content.timing_info
                && !timing.equal_picture_interval
            {
                let diagnostic = scan_type_equal_picture_interval_error(value, &pair);
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
                if same_tu {
                    eagerly_emitted.insert((*ci_xlayer, *ci_mlayer));
                }
            }
        }

        // Push the observation after the loop so `eagerly_emitted` is final, tagged with
        // whether its Table 6.18 restriction was already emitted eagerly above (the
        // scan-type analogue of the round-7 timecode finding 2).
        self.scan_type
            .scopes
            .entry(scope_key)
            .or_default()
            .observations
            .push(ScanTypeObservation {
                mps_pic_struct_type: value,
                offset: obu.offset,
                tu_index,
                eagerly_emitted,
            });
    }

    /// Re-evaluates the § 6.16.10 Table 6.18 restrictions of the stored scan-type
    /// observations against a newly observed content-interpretation record — the
    /// CI may arrive after the scan-type metadata it constrains. The caller
    /// ([`ValidatorContext::observe_content_interpretation`]) invokes this only
    /// when the CI's Table 6.18-decisive content differs from the record it
    /// replaces, so a repeated identical CI never re-reports while every
    /// genuinely new (observation, CI-content) pair is evaluated exactly once
    /// (see [`ScanTypeObservation`]). Observations from a temporal unit before
    /// the CI extended layer's most recent random access point are skipped —
    /// their pictures' content interpretation parameters belong to the previous
    /// § 7.3.8.11 epoch, the same epoch mismatch in the other direction as the
    /// pre-epoch-record skip in
    /// [`ValidatorContext::check_scan_type_consistency`]. The CI's own
    /// extended-layer scope and the global bucket are re-evaluated (global
    /// scan-type metadata describes every layer's pictures); the baseline of
    /// each comparison is the metadata observation, so
    /// [`CvsTracker::defer_or_emit`] routes on its temporal unit.
    ///
    /// `repair` flags the call as the § 7.3.8.11 RAP re-pair from
    /// [`Self::repair_post_rap_ci_pairings`] (the scan-type analogue of the round-7
    /// timecode finding 2). The eager CI-after-metadata caller passes `false`; the RAP
    /// re-pair passes `true`, which skips an `(observation, CI)` pair that already
    /// paired-and-emitted eagerly against this in-scope same-temporal-unit CI at
    /// observation time (the [`ScanTypeObservation::eagerly_emitted`] set contains the
    /// CI's identity — populated when an identical CI was already recorded BEFORE the
    /// observation in the same RAP temporal unit, so the eager observation-time pairing
    /// emitted directly). Re-pairing such a pair would duplicate the diagnostic; the skip
    /// is per-CI, so a DIFFERENT CI for the same observation — whose eager pairing was
    /// instead DEFERRED against a stale pre-RAP CI (and dropped by `observe_ci_rap` at the
    /// RAP) — still gets re-paired.
    fn recheck_scan_type_after_ci(
        &mut self,
        ci_xlayer: ExtendedLayerId,
        ci_mlayer: EmbeddedLayerId,
        content: &ContentInterpretation,
        ci_offset: ByteOffset,
        repair: bool,
        report: &mut ValidationReport,
    ) {
        let (established, bad_interval) = scan_type_decisive_content(content);
        if established == 0 && !bad_interval {
            return;
        }
        let epoch = self.ci_rap_epoch(ci_xlayer);
        let scope_keys: &[ExtendedLayerId] = if ci_xlayer.is_global() {
            &[GLOBAL_XLAYER_ID]
        } else {
            &[ci_xlayer, GLOBAL_XLAYER_ID]
        };
        for &scope_key in scope_keys {
            let Some(scope) = self.scan_type.scopes.get(&scope_key) else {
                continue;
            };
            for observation in &scope.observations {
                if observation.tu_index < epoch {
                    continue;
                }
                // The RAP re-pair additionally skips a `(observation, CI)` pair already
                // paired-and-emitted eagerly at observation time (the scan-type analogue
                // of the round-7 timecode finding 2). The skip is keyed by THIS CI's
                // identity, so an eager emission against a different CI does not suppress
                // re-pairing this one (the multi-layer opposite-ordering case).
                if repair
                    && observation
                        .eagerly_emitted
                        .contains(&(ci_xlayer, ci_mlayer))
                {
                    continue;
                }
                let value = observation.mps_pic_struct_type;
                let Some(group) = PicStructGroup::from_pic_struct(value) else {
                    continue;
                };
                let pair = ScanTypeCiPair {
                    ci_xlayer,
                    ci_mlayer,
                    metadata_offset: observation.offset,
                    ci_offset,
                    at: ci_offset,
                };
                if established != 0 {
                    let required = group.required_ci_scan_type_idc();
                    if established != required {
                        let diagnostic =
                            scan_type_ci_mismatch_error(value, required, established, &pair);
                        self.cvs
                            .defer_or_emit(scope_key, observation.tu_index, diagnostic, report);
                    }
                }
                if matches!(value, 7 | 8) && bad_interval {
                    let diagnostic = scan_type_equal_picture_interval_error(value, &pair);
                    self.cvs
                        .defer_or_emit(scope_key, observation.tu_index, diagnostic, report);
                }
            }
        }
    }

    /// Returns whether any in-scope content-interpretation record established a
    /// non-zero `ci_scan_type_idc` for `scope_key`: a concrete extended layer
    /// matches its own records, the global bucket matches every record (global
    /// scan-type metadata describes every layer's pictures).
    ///
    /// The § 7.3.8.11 random-access epoch is deliberately NOT applied here: a
    /// pre-OLK record keeps suppressing the
    /// `metadata/scan-type-ci-scan-type-unestablished` warning after an OLK
    /// re-initializes the parameters to `ci_scan_type_idc` 0 — a documented
    /// lenient false-negative approximation in the conservative direction for a
    /// warning-severity diagnostic derived from a literal Table 6.18 reading
    /// (tightening it would make the derived warning fire more often).
    fn scan_type_ci_established(&self, scope_key: ExtendedLayerId) -> bool {
        self.content_interpretations
            .iter()
            .any(|((ci_xlayer, _), record)| {
                (scope_key.is_global() || *ci_xlayer == scope_key)
                    && record.content.scan_type_idc.get() != 0
            })
    }

    /// Ends the coded video sequence of `scope_key`'s scan-type scope: emits the
    /// `metadata/scan-type-ci-scan-type-unestablished` warning when observations
    /// are being retired and no in-scope content-interpretation record established
    /// a non-zero `ci_scan_type_idc`, then drops observations with
    /// `tu_index < keep_from_tu` (pass `u64::MAX` to retire the whole scope at the
    /// end of the bitstream). One warning per scope, citing the first retiring
    /// observation.
    ///
    /// The warning is a **derived** diagnostic from a literal reading of
    /// Table 6.18 (AV2 § 6.16.10): every defined `mps_pic_struct_type` row
    /// restricts `ci_scan_type_idc` to 1, 2 or 3, while the default content
    /// interpretation parameter — in effect when no content interpretation OBU
    /// establishes one — is "ci_scan_type_idc = 0 (unspecified)" (AV2 § 7.3.8.11),
    /// which satisfies no row. The mirror states no explicit
    /// presence requirement for the content interpretation OBU, so this is a
    /// warning, never an error.
    fn flush_scan_type_scope(
        &mut self,
        scope_key: ExtendedLayerId,
        keep_from_tu: u64,
        report: &mut ValidationReport,
    ) {
        let established = self.scan_type_ci_established(scope_key);
        let Some(scope) = self.scan_type.scopes.get_mut(&scope_key) else {
            return;
        };
        if !established
            && let Some(first) = scope
                .observations
                .iter()
                .find(|observation| observation.tu_index < keep_from_tu)
        {
            report.push(
                Diagnostic::warning(
                    "metadata/scan-type-ci-scan-type-unestablished",
                    format!(
                        "scan-type metadata with mps_pic_struct_type {} (first at byte {}) was \
                         signaled, but no content interpretation in scope established a non-zero \
                         ci_scan_type_idc within the coded video sequence; the default is \
                         ci_scan_type_idc = 0 (unspecified) per AV2 § 7.3.8.11, which satisfies \
                         no Table 6.18 restriction — a diagnostic derived from a literal reading \
                         of Table 6.18",
                        first.mps_pic_struct_type, first.offset,
                    ),
                )
                .with_spec_section("6.16.10")
                .with_byte_offset(first.offset),
            );
        }
        scope
            .observations
            .retain(|observation| observation.tu_index >= keep_from_tu);
        if scope.observations.is_empty() {
            self.scan_type.scopes.remove(&scope_key);
        }
    }

    /// Prunes the § 6.16.7 timecode state at a § 7.3.6 CVS boundary: a CLK for
    /// `clk_xlayer` starts a new coded video sequence for THAT extended layer at
    /// `keep_from_tu` (mirror `07-decoding-process.md` lines 604-606, "A new coded
    /// video sequence for an extended layer is defined to start ... in the coded
    /// extended layer unit corresponding to the extended layer").
    ///
    /// § 7.3.6 CVS boundaries are per extended layer (finding 2), so this prunes only
    /// the state whose coded video sequence actually restarted:
    ///
    /// - **n_frames observations**: an observation belongs to the coded video sequences
    ///   of the extended layers it targets (its § 6.16.3 `targeting`), so it is dropped
    ///   only when `clk_xlayer` is one of them and it predates `keep_from_tu`. An
    ///   observation whose targeting is not bitstream-derivable (`None`) never fires the
    ///   bound (see [`timecode_ci_in_scope`]); it is keyed by its carrying
    ///   `obu_xlayer_id` scope and dropped when that scope's CVS restarts (a global
    ///   carrying scope keeps the documented any-CLK approximation, harmless because it
    ///   compares nothing). A global LAYER_VALUES observation aimed at extended layer 1
    ///   therefore survives a CLK for extended layer 0 and is still in scope for layer
    ///   1's later n_frames re-checks.
    /// - **inference chain**: each `(obu_xlayer_id, obu_mlayer_id)` entry whose previous
    ///   set both belongs to a coded video sequence `clk_xlayer` restarts — the
    ///   target-aware [`TimecodeInferenceEntry::belongs_to_cvs_of`] test, matching the
    ///   n_frames-observation pruning above (round-7 finding 1) — and predates
    ///   `keep_from_tu` is reset (the seed belongs to the ending coded video sequence; a
    ///   same-temporal-unit predecessor joined the new sequence and still seeds it). Pre-
    ///   fix the entry was dropped whenever its carrying `obu_xlayer_id` matched
    ///   `clk_xlayer` or was global, so a global `LAYER_VALUES` chain aimed at one
    ///   extended layer was reset by an unrelated layer's CLK; the targeting now spares
    ///   it, just as it does the matching observation and pending-inference entries.
    fn prune_timecode_scope(&mut self, clk_xlayer: ExtendedLayerId, keep_from_tu: u64) {
        self.timecode.observations.retain(|observation| {
            // Keep observations at/after the boundary, and observations whose coded
            // video sequence did NOT restart at this CLK (the CLK is for a different
            // extended layer than any the observation belongs to).
            observation.tu_index >= keep_from_tu || !observation.belongs_to_cvs_of(clk_xlayer)
        });
        self.timecode.inference.retain(|_, entry| {
            // Keep entries whose previous set is at/after the boundary, and entries
            // whose coded video sequence did NOT restart at this CLK (the CLK is for a
            // different extended layer than any the previous set targets) — the same
            // target-aware test as the observation pruning above (round-7 finding 1),
            // replacing the pre-fix carrying-scope-only `xlayer == clk_xlayer ||
            // is_global` predicate that dropped a global LAYER_VALUES chain on any CLK.
            entry.prev_tu >= keep_from_tu || !entry.belongs_to_cvs_of(clk_xlayer)
        });
    }

    /// Emits the deferred § 6.16.7 inference-presence diagnostics whose seed now
    /// belongs to an ending coded video sequence because a CLK started a new coded
    /// video sequence for `xlayer` at this temporal unit (§ 7.3.6). A pending entry
    /// fires when the CLK restarts the coded video sequence of a layer the omitting
    /// timecode actually targets — the target-aware
    /// [`PendingTimecodeInference::belongs_to_cvs_of`] test, mirroring the
    /// observation pruning in [`Self::prune_timecode_scope`] (finding 2). § 7.3.6 CVS
    /// boundaries are per extended layer, so a CLK for one extended layer detaches the
    /// seed of a timecode carried on (or, for a global `LAYER_VALUES` timecode,
    /// targeting) that extended layer only; a CLK for an UNRELATED extended layer
    /// leaves a global timecode aimed at a different layer pending (pre-fix any global
    /// carrying scope fired on every CLK, a false positive). A global timecode with no
    /// derivable targeting keeps the documented any-CLK approximation (its
    /// `obu_xlayer_id` is global). Survivors are left for
    /// [`Self::drop_pending_timecode_inference`] at the temporal-unit flush. See
    /// [`TimecodeCvsState::pending_inference`].
    fn emit_pending_timecode_inference(
        &mut self,
        xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        let mut retained = Vec::with_capacity(self.timecode.pending_inference.len());
        for entry in std::mem::take(&mut self.timecode.pending_inference) {
            if entry.belongs_to_cvs_of(xlayer) {
                report.push(entry.diagnostic);
            } else {
                retained.push(entry);
            }
        }
        self.timecode.pending_inference = retained;
    }

    /// Drops the deferred § 6.16.7 inference-presence diagnostics that survived the
    /// just-completed temporal unit with no CVS boundary: their earlier-temporal-unit
    /// seed stayed in the same coded video sequence (§ 7.3.6), so the field infers
    /// cleanly and the diagnostic is silently discarded. See
    /// [`TimecodeCvsState::pending_inference`].
    fn drop_pending_timecode_inference(&mut self) {
        self.timecode.pending_inference.clear();
    }

    /// Checks the locally-decidable § 6.16.7 timecode rules for one
    /// `metadata_timecode()` unit
    /// (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-7`):
    ///
    /// 1. **Inference-presence** (lines 3873-3893): for each of `seconds_value`,
    ///    `minutes_value`, `hours_value` that is *not present* in this set, the mirror
    ///    infers its value from "the previous set of clock timestamp syntax elements in
    ///    decoding order, and it is required that such a previous \[element\] shall have
    ///    been present". When no previous set in this CVS scope carried that field, the
    ///    inference has no source, so `metadata/timecode-inferred-without-previous`
    ///    (error) is emitted naming the field.
    ///
    ///    **Interpretation choice — literal "present" reading (documented):** the
    ///    mirror requires, of an omitted field, that "such a previous seconds_value
    ///    \[minutes_value, hours_value\] shall have been present" (lines 3873-3893,
    ///    `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-7`). "Present"
    ///    is read literally as the syntax element having been *coded in the immediate
    ///    predecessor set in decoding order* — i.e. the previous set's own presence
    ///    flags. An *inferred* value in the previous set therefore does NOT make the
    ///    element "present" for the next set: a set that omits a field, followed by
    ///    another set that also omits it, fires the diagnostic on the second omission
    ///    too (the chain never seeds itself from an inference). The lenient chained-
    ///    inference reading — where the first omitted-but-inferred value would then
    ///    count as "present" and satisfy the next omitting set — was rejected for
    ///    lacking textual support: the sentence speaks of the element having "been
    ///    present", not of a value having been *established* (whether by presence or by
    ///    inference). AVM differential testing may revisit this if the reference
    ///    decoder treats a propagated inferred value as satisfying the requirement.
    ///    [`TimecodeFieldPresence`] therefore records each set's own literal field
    ///    presence only, never an OR with the predecessor's inferred state.
    /// 2. **n_frames bound** (lines 3865-3867): "When ci_timing_info_present_flag is
    ///    equal to 1, n_frames shall be less than maxPicPerSecond". The
    ///    `ci_timing_info_present_flag` is the content interpretation OBU's flag
    ///    associated with the timecode's extended layer (annex-e-decoder-model.md line
    ///    293: "ci_timing_info_present_flag equal to 1 in the content interpretation OBU
    ///    associated with this extended layer"); a present `timing_info()` in an
    ///    in-scope content interpretation is exactly that flag set. The bound is checked
    ///    against every in-scope content-interpretation record at/after the timecode
    ///    layer's § 7.3.8.11 CI-parameter epoch (the same epoch filter the § 6.16.10
    ///    scan-type / CI pairing applies); a content interpretation arriving *after* the
    ///    timecode re-evaluates instead (see
    ///    [`ValidatorContext::recheck_timecode_n_frames_after_ci`]). "In scope" is the
    ///    unit's § 6.16.3 layer targeting (`targeting`): when the targeting is
    ///    bitstream-derivable, only the CIs of the layers the timecode describes pair
    ///    with it, so a global `LAYER_VALUES` timecode naming only some layers does not
    ///    pair with an untargeted layer's CI (finding 4, see [`timecode_ci_in_scope`]);
    ///    an underivable targeting falls back to the `obu_xlayer_id` scope.
    ///
    /// Both diagnostics anchor at the offending timecode metadata OBU. These are
    /// metadata-local facts, so they are not gated by [`ValidationOptions`]'
    /// external-HLS mode.
    fn check_timecode_consistency(
        &mut self,
        obu: &ObuEnvelope<'_>,
        timecode: &MetadataTimecode,
        targeting: Option<HdrAssociation>,
        report: &mut ValidationReport,
    ) {
        let scope_xlayer = obu.header.extended_layer_id;
        // The inference chain is keyed by the carrying OBU's concrete
        // `(obu_xlayer_id, obu_mlayer_id)` (finding 3): METADATA_TYPE_TIMECODE is
        // layer-specific (§ 6.16.3 Table 6.17), so a timecode on embedded layer
        // `(x, m0)` is not the "previous set in decoding order" of one on `(x, m1)` and
        // must not seed its inference. For unspecified targeting the carrying OBU's own
        // pair is still the soundest concrete scope (finding 4, documented).
        // TODO(spec: AV2-5.17.7-METADATA-TIMECODE): a group-form LAYER_VALUES timecode
        // carries GLOBAL_XLAYER_ID, so two groups targeting disjoint layer sets share
        // this carrying-pair key and a present value from one set can seed an omitted
        // field aimed at another -- a known false-negative (never a false positive);
        // keying chains per derived target set would close it.
        let inference_key = (scope_xlayer, obu.header.embedded_layer_id);
        let tu_index = self.cvs.tu_index;

        // 1. Inference-presence (decoding-order, per carrying-layer scope). The
        // "previous set in decoding order" is the immediate predecessor for THIS
        // carrying layer; its presence is read literally (round-1 finding): an inferred
        // value in the predecessor does NOT make the element present, so the chain never
        // seeds itself from an inference.
        let prev = self.timecode.inference.get(&inference_key).cloned();
        // Record this set's own literal field presence as the new previous set (no OR
        // with the predecessor's inferred state), carrying its § 6.16.3 targeting and
        // carrying scope so the § 7.3.6 chain reset is target-aware (round-7 finding 1).
        // Also append the n_frames observation (likewise carrying the targeting and the
        // carrying scope) for the CI-after re-check and the target-aware § 7.3.6 pruning.
        let this = TimecodeFieldPresence::of(timecode);
        self.timecode.inference.insert(
            inference_key,
            TimecodeInferenceEntry {
                presence: this,
                prev_tu: tu_index,
                scope_xlayer,
                targeting: targeting.clone(),
            },
        );
        // For each absent field, a previous *present* value (literally coded in the
        // immediate predecessor set in decoding order) is required.
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
                // No previous present value in scope — the inference has no source
                // regardless of any later § 7.3.6 boundary, so fire eagerly.
                None => report.push(diagnostic),
                Some(entry) if !entry.presence.field(field) => report.push(diagnostic),
                // A present predecessor in THIS temporal unit always shares the coded
                // video sequence (§ 7.3.6 sequences start at temporal units, never
                // inside one), so it seeds the inference cleanly — silent.
                Some(entry) if entry.prev_tu == tu_index => {}
                // A present predecessor in an EARLIER temporal unit seeds only if no CLK
                // later in this temporal unit starts a new coded video sequence
                // (finding 2 / § 7.3.6). Defer the decision to the temporal unit's
                // resolution: emit on a matching CVS start, drop silently otherwise.
                Some(_) => self
                    .timecode
                    .pending_inference
                    .push(PendingTimecodeInference {
                        xlayer: scope_xlayer,
                        // Carry the § 6.16.3 targeting so emit_pending_timecode_inference
                        // fires only on a CLK for a layer this timecode targets (finding
                        // 2), mirroring the n_frames observation's target-aware pruning.
                        targeting: targeting.clone(),
                        diagnostic,
                    }),
            }
        }

        // 2. n_frames bound against the already-observed in-scope content
        // interpretations (a later CI re-evaluates via
        // recheck_timecode_n_frames_after_ci). The § 6.16.3 targeting scopes the
        // pairing to the CIs of the layers this timecode describes; an underivable
        // targeting compares nothing (finding 4, see timecode_ci_in_scope).
        //
        // `eagerly_emitted` collects the CI identities whose same-temporal-unit in-scope
        // bound was decided HERE and emitted (not deferred) — i.e. an identical CI was
        // re-sent BEFORE this timecode in the same § 7.3.8.11 RAP temporal unit. The RAP
        // re-pair (repair_post_rap_ci_pairings) skips exactly those `(observation, CI)`
        // pairs so the diagnostic is not emitted twice (round-7 finding 2), while still
        // re-pairing any OTHER CI for this observation. A pairing DEFERRED against an
        // earlier-temporal-unit (stale pre-RAP) CI does NOT enter the set: that deferred
        // diagnostic is dropped at the RAP, so the re-pair must still cover it.
        let mut eagerly_emitted = BTreeSet::new();
        for ((ci_xlayer, ci_mlayer), record) in &self.content_interpretations {
            if !timecode_ci_in_scope(&targeting, *ci_xlayer, *ci_mlayer) {
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
                    obu.offset,
                );
                // defer_or_emit emits eagerly iff the CI is in this temporal unit; a
                // same-temporal-unit emission is the round-7 finding 2 case to skip in
                // the RAP re-pair, keyed by the CI's identity so only this exact pairing
                // is skipped.
                if record.tu_index == tu_index {
                    eagerly_emitted.insert((*ci_xlayer, *ci_mlayer));
                }
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
            }
        }

        // Append the n_frames observation (carrying the § 6.16.3 targeting and the
        // carrying scope) for the CI-after re-check and the target-aware § 7.3.6 pruning,
        // tagged with whether its bound was already emitted eagerly above (round-7
        // finding 2). Pushed after the loop so `eagerly_emitted` is final.
        self.timecode.observations.push(TimecodeObservation {
            n_frames: timecode.n_frames,
            offset: obu.offset,
            tu_index,
            scope_xlayer,
            targeting,
            eagerly_emitted,
        });
    }

    /// Re-evaluates the § 6.16.7 n_frames bound of the stored timecode observations
    /// against a newly observed content-interpretation record — the content
    /// interpretation may arrive after the timecode metadata it constrains (the same
    /// arrival-order handling as
    /// [`ValidatorContext::recheck_scan_type_after_ci`]). Only a content
    /// interpretation with a present `timing_info()` (i.e.
    /// `ci_timing_info_present_flag == 1`) establishes the bound; observations from a
    /// temporal unit before the CI layer's § 7.3.8.11 random access point are skipped
    /// (their pictures' content interpretation parameters belong to the previous
    /// epoch). The diagnostic anchors at the offending timecode metadata OBU.
    ///
    /// `repair` flags the call as the § 7.3.8.11 RAP re-pair from
    /// [`Self::repair_post_rap_ci_pairings`] (round-7 finding 2). The eager
    /// CI-after-timecode caller passes `false`; the RAP re-pair passes `true`, which
    /// skips an `(observation, CI)` pair that already paired-and-emitted eagerly against
    /// this in-scope same-temporal-unit CI at observation time (the
    /// [`TimecodeObservation::eagerly_emitted`] set contains the CI's identity —
    /// populated when an identical CI was already recorded BEFORE the observation in the
    /// same RAP temporal unit, so the eager observation-time pairing emitted directly).
    /// Re-pairing such a pair would duplicate the diagnostic; the skip is per-CI, so a
    /// DIFFERENT CI for the same observation — whose eager pairing was instead DEFERRED
    /// against a stale pre-RAP CI (and dropped by `observe_ci_rap` at the RAP) — still
    /// gets re-paired.
    fn recheck_timecode_n_frames_after_ci(
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
        // The observations are a single flat list now; the § 6.16.3 targeting decides
        // which of them this CI's layer can bind, so an untargeted layer's CI cannot
        // pair with an observation aimed elsewhere, and an underivable-targeting
        // observation binds to nothing (finding 4, see timecode_ci_in_scope). The
        // § 7.3.8.11 epoch filter (tu_index >= epoch) drops observations whose pictures
        // belong to a previous content-interpretation-parameter epoch. The RAP re-pair
        // additionally skips an observation already paired-and-emitted eagerly against
        // THIS CI at observation time (round-7 finding 2), keyed by the CI's identity so
        // an eager emission against a different CI does not suppress this one. Snapshot
        // first to avoid borrowing self twice.
        let violations: Vec<(u16, ByteOffset, u64)> = self
            .timecode
            .observations
            .iter()
            .filter(|observation| {
                observation.tu_index >= epoch
                    && !(repair
                        && observation
                            .eagerly_emitted
                            .contains(&(ci_xlayer, ci_mlayer)))
                    && u64::from(observation.n_frames) >= max_pic
                    && timecode_ci_in_scope(&observation.targeting, ci_xlayer, ci_mlayer)
            })
            .map(|observation| {
                (
                    observation.n_frames,
                    observation.offset,
                    observation.tu_index,
                )
            })
            .collect();
        for (n_frames, metadata_offset, observation_tu) in violations {
            let diagnostic = timecode_n_frames_error(
                n_frames,
                max_pic,
                ci_xlayer,
                ci_mlayer,
                ci_offset,
                metadata_offset,
                // Anchor at the offending timecode metadata OBU (the message also
                // names it), not the CI OBU.
                metadata_offset,
            );
            self.cvs
                .defer_or_emit(ci_xlayer, observation_tu, diagnostic, report);
        }
    }

    /// Returns the active metadata units for `(xlayer, metadata_type_raw)` from the
    /// § 6.16.3 lifetime store.
    ///
    /// Query surface for the validator test suite and the deferred per-frame
    /// metadata checks (tasks 8-9 of the `metadata-semantic-validation` change).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn active_metadata_units(
        &self,
        xlayer: ExtendedLayerId,
        metadata_type_raw: u32,
    ) -> &[ActiveMetadataUnit] {
        self.metadata.active_units(xlayer, metadata_type_raw)
    }

    /// Returns whether `record` applies to `(target_mlayer, target_tlayer)` per the
    /// § 6.16.3 propagation rules (see [`MetadataLifetimeStore::applies_to`]),
    /// resolving the dependency maps from `xlayer`'s active sequence header
    /// (`SequenceHeaderGeneral::{m,t}layer_dependency_map`, AV2 § 5.4.1). When no
    /// sequence header is active for the extended layer (including the
    /// `GLOBAL_XLAYER_ID` bucket, which never activates one), falls back to the
    /// § 5.4.1 default fills built from the record's own source layer ids — the
    /// most conservative read: with `max_mlayer_id = K` and `max_tlayer_id = T` the
    /// default fills make the metadata apply only at its own `(K, T)` source point,
    /// never inventing propagation a real sequence header might not permit.
    ///
    /// Pure query — never emits diagnostics.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn metadata_applies_to(
        &self,
        xlayer: ExtendedLayerId,
        record: &ActiveMetadataUnit,
        target_mlayer: EmbeddedLayerId,
        target_tlayer: TemporalLayerId,
    ) -> bool {
        let general = self
            .active_sequence_by_xlayer
            .get(&xlayer)
            .and_then(|id| self.sequence_headers.get(id))
            .map(|header| header.general);
        let (m_map, t_map) = match general {
            Some(general) => (general.mlayer_dependency_map, general.tlayer_dependency_map),
            None => (
                MLayerDependencyMap::default_for(record.source_mlayer),
                TLayerDependencyMap::default_for(record.source_tlayer, record.source_mlayer),
            ),
        };
        MetadataLifetimeStore::applies_to(record, target_mlayer, target_tlayer, &t_map, &m_map)
    }

    /// Observes a multi-frame header OBU and checks that the sequence header it
    /// references via `mfh_seq_header_id` is available in-band or through
    /// caller-provided external HLS (AV2 § 7.3.8.6 / § 7.3.8.7).
    fn observe_multi_frame_header(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures and the mfh_seq_header_id range check are handled by the
        // stateless MultiFrameHeaderSyntax check; here we only resolve the reference.
        let Ok(mfh) = parse_multi_frame_header(&mut reader) else {
            return;
        };
        // An out-of-range id (>= MAX_SEQ_NUM) cannot name a valid sequence header and
        // is already flagged as mfh/seq-header-id-out-of-range; do not double-report.
        if !mfh.seq_header_id_in_range() {
            return;
        }

        let id = mfh.mfh_seq_header_id;
        match self.hls.resolve_sequence_header(id, options) {
            HlsResolution::InBand | HlsResolution::External => {}
            HlsResolution::Unavailable => {
                let external_note = if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    " (external HLS is disabled)"
                } else {
                    " in-band or through the supplied external HLS"
                };
                report.push(
                    Diagnostic::error(
                        "mfh/sequence-header-unavailable",
                        format!(
                            "multi-frame header references mfh_seq_header_id {id}, but no sequence \
                             header with that id is available{external_note}"
                        ),
                    )
                    .with_spec_section("7.3.8.6")
                    .with_byte_offset(obu.offset),
                );
                if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    report.push(external_hls_disabled_advisory(id, obu));
                }
            }
        }

        // Gate availability on the same validation the SequenceHeaderSyntax /
        // observe_sequence_header path uses: a fully parsed MFH (now including
        // seg_info()) must have a valid §5.2.1 payload tail (obu_extension_flag +
        // trailing_bits). A malformed tail makes the MFH not a valid available HLS
        // object, so a later cur_mfh_id reference must treat it as unavailable rather
        // than resolve through it.
        if finish_obu_payload(
            &mut reader,
            obu.payload,
            obu.header.obu_type.is_extensible_obu(),
        )
        .is_err()
        {
            return;
        }

        // Record this multi-frame header's in-band availability (AV2 § 7.3.8.7) so a
        // later frame header's cur_mfh_id reference can resolve it. Both ids are in
        // range here (mfh_seq_header_id checked above; mfhId checked now). The MFH is
        // recorded even when its own sequence-header reference is unavailable — that
        // is a separate finding above, not a reason to treat the MFH as absent.
        if mfh.mfh_id_in_range()
            && let Some(seq_id) = SequenceHeaderId::try_new(mfh.mfh_seq_header_id)
            && let Ok(mfh_id_value) = u32::try_from(mfh.mfh_id())
        {
            self.hls.record_multi_frame_header(MultiFrameHeaderRecord {
                mfh_id: MfhId::from_raw(mfh_id_value),
                mfh_seq_header_id: seq_id,
                mfh_tlayer_id: obu.header.temporal_layer_id,
                mfh_mlayer_id: obu.header.embedded_layer_id,
                // Carry the parsed §5.7 state a `cur_mfh_id > 0` frame header consumes at
                // §5.18.2 (default frame dimensions and the §5.18.7.1 segmentation arm)
                // and the deblocking-update groundwork bit.
                mfh_frame_size: mfh.mfh_frame_size,
                mfh_seg_info_present_flag: mfh.mfh_seg_info_present_flag,
                mfh_ext_seg_flag: mfh.mfh_ext_seg_flag,
                mfh_allow_seg_info_change: mfh.mfh_allow_seg_info_change,
                mfh_segment_info: mfh.segment_info,
                mfh_deblocking_filter_update: mfh.mfh_deblocking_filter_update,
                mfh_apply_deblocking_filter: mfh.mfh_apply_deblocking_filter,
                offset: obu.offset,
            });
            // AV2 § 7.3.8.1: note this MFH's in-band (re)send for the replay, and — when
            // its mfh_seq_header_id resolved in-band (so the linear check did not fire and
            // external HLS did not suppress) — buffer the § 7.3.8.6 sequence-header
            // reference this MFH makes (a MFH at a random access point references a
            // sequence header that must itself be available at that point).
            self.rap_replay.note_resend(
                RapHlsKey::MultiFrameHeader(mfh_id_value),
                obu.header.extended_layer_id,
            );
            if matches!(
                self.hls.resolve_sequence_header(id, options),
                HlsResolution::InBand
            ) {
                self.note_rap_reference(
                    RapHlsKey::SequenceHeader(id),
                    obu.header.extended_layer_id,
                    obu.offset,
                );
            }
        }
    }

    /// Observes a layer configuration record OBU: records its in-band availability and
    /// checks a local record's references to a global LCR (AV2 § 7.3.8.3) and a local
    /// atlas segment OBU (AV2 § 7.3.8.4). Availability is recorded only after a
    /// successful parse and a valid § 5.2.1 payload tail, mirroring the sequence-header
    /// and multi-frame-header observers.
    fn observe_layer_config_record(
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

    /// Observes an atlas segment info OBU and records a local atlas segment's in-band
    /// availability (AV2 § 7.3.8.4). A global atlas (§ 7.3.8.4 uses "can be available",
    /// not a hard requirement) is not recorded. Recording is gated on a successful
    /// parse and a valid § 5.2.1 payload tail.
    fn observe_atlas_segment(&mut self, obu: &ObuEnvelope<'_>) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures and syntax diagnostics are handled by the stateless
        // AtlasSegmentSyntax check; here we only record availability.
        let Ok(atlas) = parse_atlas_segment(&mut reader) else {
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
        if !xlayer.is_global() {
            self.hls.record_local_atlas(xlayer, atlas.atlas_segment_id);
            // AV2 § 7.3.8.1: note this *local* atlas segment's in-band (re)send (its own
            // extended layer) for the random-access-point availability replay, so a local
            // LCR's lcr_local_atlas_id referencing it at a random access point must find it
            // resent there. A global atlas is excluded (§ 7.3.8.4 "can be available").
            self.rap_replay.note_resend(
                RapHlsKey::Atlas {
                    xlayer: xlayer.get(),
                    id: atlas.atlas_segment_id,
                },
                xlayer,
            );
        }
    }

    /// Observes an operating point set OBU: emits the locally-checkable § 6.10
    /// conformance diagnostics and then applies the § 6.10.1 reset/update semantics to
    /// the active OPS state. The local checks run against the *prior* OPS state (before
    /// this OBU is applied) so cross-OPS inheritance references resolve correctly.
    /// Acting is gated on a successful parse and a valid § 5.2.1 extensible tail.
    fn observe_operating_point_set(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(ops) = parse_operating_point_set(&mut reader, obu.header.extended_layer_id) else {
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

        self.check_operating_point_set_semantics(obu, &ops, report);

        // Annex A.4: OPS-signaled value-space checks (Annex A applies its constraints
        // per sub-bitstream using OPS-derived values, mirror lines 443-451) — a reserved
        // ops_level_idx in 22-30 (Table A.7), and a High ops_tier_flag below level 4.0
        // (Table A.9 NOTE). The OPS PTL carries ops_tier_flag unconditionally, so the
        // high-tier-below-4.0 case is reachable here (unlike the seq-header arm).
        check_ops_level_tier_value_space(obu, &ops, report);

        // AV2 § 6.10.5: the per-(obu_xlayer_id, opsID, op) buffer-delay sum-constancy
        // checks, run before the § 6.10.1 reset/update is applied so the defining OPS's
        // own reset_flag re-baselines its values (the constraint excludes intervening
        // resets) — see check_ops_buffer_delay_sums.
        self.check_ops_buffer_delay_sums(obu, &ops, options, report);

        // AV2 § 6.10.7: explicitly signalled maps are checked against the currently
        // activated sequence headers now, and retained on the record so a later
        // activation can complete the pairing (see on_sequence_activation).
        let explicit_entries = ops_explicit_entries(&ops);
        self.check_ops_entries_against_active(
            obu.offset,
            ops.ops_id,
            &explicit_entries,
            options,
            report,
        );

        // AV2 § 6.10.1: apply reset/update to the active OPS state after the checks.
        let defines = ops.ops_cnt > 0;
        self.ops.apply(
            OperatingPointSetRecord {
                xlayer_id: ops.xlayer_id,
                ops_id: ops.ops_id,
                ops_cnt: ops.ops_cnt,
                offset: obu.offset,
                explicit_entries,
            },
            ops.reset_flag,
        );
        // AV2 § 7.3.8.1: note this OPS (re)send for the random-access-point availability
        // replay, but only when the OBU actually *defines* `(obu_xlayer_id, ops_id)`
        // (`ops_cnt > 0`); a pure reset (`ops_cnt == 0`) makes no OPS available, so it is
        // not a qualifying resend.
        if defines {
            self.rap_replay.note_resend(
                RapHlsKey::OperatingPointSet {
                    xlayer: ops.xlayer_id.get(),
                    ops_id: ops.ops_id,
                },
                obu.header.extended_layer_id,
            );
        }
    }

    /// Checks the § 6.10.5 operating-point buffer-delay sum-constancy constraint:
    ///
    /// > "For a video sequence that includes one or more random access points the sum
    /// > of ops_decoder_buffer_delay and ops_encoder_buffer_delay shall be kept
    /// > constant." (AV2 v1.0.0 § 6.10.5,
    /// > `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md` lines 2810–2825.)
    ///
    /// The error tier (`decoder-model/buffer-delay-sum-changed`) fires only for the
    /// sub-case that is non-conforming under *every* plausible reading of the
    /// undefined term "video sequence":
    ///
    /// 1. Per coded video sequence (CVS). Every CVS for an extended layer starts at a
    ///    closed random access point (§ 7.3.6: a CVS "is defined to start at each
    ///    temporal unit that contains an OBU with obu_type equal to
    ///    OBU_CLOSED_LOOP_KEY"), so "includes one or more random access points" always
    ///    holds and the constraint binds within the same CVS epoch.
    /// 2. Per coded multistream video sequence (CMVS) — a superset of each per-layer
    ///    CVS; two values in one CVS are also in one CMVS.
    /// 3. Per whole per-`obu_xlayer_id` sub-bitstream — the broadest reading; two
    ///    values in one CVS are trivially in one sub-bitstream.
    ///
    /// So a change of the explicitly signalled sum for the *same*
    /// `(obu_xlayer_id, opsID, op)` triple, within one CVS epoch and with no
    /// intervening § 6.10.1 OPS reset, is non-conforming under all three readings and
    /// is reported as an error. Changes that only span a CVS or reset boundary are
    /// conforming under the per-CVS reading; those are the advisory warning tier
    /// (`decoder-model/buffer-delay-sum-changed-across-cvs`, see
    /// `check_ops_buffer_delay_cross_cvs`).
    ///
    /// Two preconditions keep the error tier sound at the temporal-unit granularity of
    /// § 7.3.6:
    ///
    /// - **A coded video sequence must have started** ([`CvsTracker::cvs_started`]).
    ///   Before the first CLK for the layer (or, for the `GLOBAL_XLAYER_ID` scope,
    ///   anywhere) the OBUs lie in *no* coded video sequence, so the per-CVS reading's
    ///   "video sequence that includes one or more random access points" precondition is
    ///   unsatisfied and the constraint does not bind. The error tier is silent there;
    ///   the change is out of scope for both tiers (it spans no CVS boundary either).
    ///   This silence also covers a narrow under-report: two pre-CLK observations in
    ///   *different* temporal units share the same epoch-0 [`BufferDelayScope`] (the
    ///   scope carries no `tu_index`), so a late CLK in the second observation's
    ///   temporal unit — which retroactively places that observation in a fresh coded
    ///   video sequence — produces no deferred error (`cvs_started` was false when the
    ///   pair was compared, so no `on_drop` advisory was armed) and no eager advisory
    ///   (the scopes were equal, so [`Self::check_ops_buffer_delay_cross_cvs`] returns
    ///   early). The change only surfaces if a later post-CLK observation repeats it.
    ///   Reporting it would require retroactively reclassifying an already-emitted
    ///   comparison at CLK time; leaving it silent is the sound-over-complete choice
    ///   consistent with the reset-spanning under-report noted below.
    /// - **The comparison is routed through [`CvsTracker::defer_or_emit`].** A baseline
    ///   from the same temporal unit is always in the same coded video sequence (a CVS
    ///   starts *at* a temporal unit, never inside one), so it is emitted eagerly. A
    ///   baseline from an earlier temporal unit is deferred, because a CLK later in the
    ///   current temporal unit would split the baseline and the new observation into
    ///   different coded video sequences — exactly the case the warning tier covers.
    ///
    /// Only explicitly signalled values participate: a payload without
    /// `ops_decoder_model_info_for_this_op_present_flag` contributes no new signalled
    /// value, and the Annex E resource-availability defaults
    /// (`DecoderBufferDelay = 70000` / `EncoderBufferDelay = 20000`,
    /// `annex-e-decoder-model.md` lines 261–272) are fallbacks, not signalled values,
    /// so they never synthesize a comparison. Per Annex E.1 (mirror lines 25–27) a
    /// *redefinition* of a `(obu_xlayer_id, opsID)` that omits the decoder-model info
    /// for an operating point does not let the previous parameters persist: a defining
    /// OPS (`ops_cnt > 0`) clears the stored baseline for every op triple of the key it
    /// no longer signals explicitly (and for op indices it no longer covers), so a later
    /// explicit value is not compared against vanished parameters. Clearing — never a
    /// default-value comparison — keeps the Annex E mode defaults out of comparisons.
    ///
    /// NB: whether an OPS reset re-baselines a reused `opsID` is itself ambiguous; the
    /// reset-spanning case stays in the warning tier (sound choice, may under-report),
    /// so the error tier deliberately requires the reset generation to match.
    fn check_ops_buffer_delay_sums(
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
                    ops_buffer_delay_intra_cvs_error(&key, previous.sum, sum, obu.offset);
                if cvs_started {
                    // When the error is deferred (the baseline came from an earlier
                    // temporal unit) and then dropped because a late CLK starts a new
                    // coded video sequence in this temporal unit, the comparison was
                    // genuinely cross-CVS: emit the cross-boundary advisory in the
                    // error's place so the change is not silently lost (§ 7.3.6
                    // temporal-unit-granular CVS boundary).
                    let on_drop =
                        ops_buffer_delay_cross_cvs_warning(&key, previous.sum, sum, obu.offset);
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
            self.check_ops_buffer_delay_cross_cvs(obu, &key, sum, scope, report);

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
    fn check_ops_buffer_delay_cross_cvs(
        &self,
        obu: &ObuEnvelope<'_>,
        key: &OpsBufferDelayKey,
        sum: u64,
        scope: BufferDelayScope,
        report: &mut ValidationReport,
    ) {
        let Some(previous) = self.ops_buffer_delay_sums.get(key) else {
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

    /// Emits the locally-checkable § 6.10 OPS conformance diagnostics: local reserved
    /// bits (§ 6.10.2), reserved `ops_mlayer_info_idc` (§ 6.10.2), PTL reserved bits
    /// (§ 6.10.4), `opsBytes` vs `ops_data_size` mismatch (§ 6.10.2), and inherited
    /// operating-point-index bounds (§ 6.10.2).
    fn check_operating_point_set_semantics(
        &self,
        obu: &ObuEnvelope<'_>,
        ops: &OperatingPointSet,
        report: &mut ValidationReport,
    ) {
        if ops.has_nonzero_local_reserved_bits() {
            report.push(
                Diagnostic::error(
                    "ops/local-reserved-bits-nonzero",
                    format!(
                        "local operating point set for obu_xlayer_id {} has ops_reserved_2bits {}, \
                         which must be 0",
                        ops.xlayer_id.get(),
                        ops.local_reserved_2bits.unwrap_or(0)
                    ),
                )
                .with_spec_section("6.10.2")
                .with_byte_offset(obu.offset),
            );
        }

        if ops.has_reserved_mlayer_info_idc() {
            report.push(
                Diagnostic::error(
                    "ops/mlayer-info-idc-reserved",
                    format!(
                        "global operating point set {} has ops_mlayer_info_idc == 3, which is \
                         reserved",
                        ops.ops_id
                    ),
                )
                .with_spec_section("6.10.2")
                .with_byte_offset(obu.offset),
            );
        }

        for payload in &ops.payloads {
            if payload.has_size_mismatch() {
                report.push(
                    Diagnostic::error(
                        "ops/payload-size-mismatch",
                        format!(
                            "ops_data_size declares {} byte(s) for OPS {} payload index {}, but \
                             {} byte(s) were parsed",
                            payload.declared_size_bytes,
                            ops.ops_id,
                            payload.index,
                            payload.computed_size_bytes
                        ),
                    )
                    .with_spec_section("6.10.2")
                    .with_byte_offset(obu.offset),
                );
            }

            for entry in &payload.xlayer_entries {
                if let Some(ptl) = &entry.ptl_info
                    && ptl.reserved_2bits != 0
                {
                    report.push(
                        Diagnostic::error(
                            "ops/ptl-reserved-bits-nonzero",
                            format!(
                                "ops_ptl_reserved_2bits is {} for OPS {} payload index {} extended \
                                 layer {}, which must be 0",
                                ptl.reserved_2bits,
                                ops.ops_id,
                                payload.index,
                                entry.xlayer_id.get()
                            ),
                        )
                        .with_spec_section("6.10.4")
                        .with_byte_offset(obu.offset),
                    );
                }

                if let OpsMlayerSource::Inherited {
                    embedded_ops_id,
                    embedded_op_index,
                } = entry.mlayer
                {
                    self.check_inherited_op_index(
                        obu,
                        ops,
                        entry.xlayer_id.get(),
                        embedded_ops_id,
                        embedded_op_index,
                        report,
                    );
                }
            }
        }
    }

    /// Checks an inherited operating-point reference against the § 6.10.2 bounds:
    /// `ops_embedded_op_index < ops_cnt[obu_xlayer_id][refID]`, and — when the
    /// reference is to the current OPS — additionally `ops_embedded_op_index < j` (the
    /// included extended layer). A cross-OPS reference is resolved against the prior
    /// active OPS state; an unresolved cross-OPS reference is not flagged here (it may
    /// be available through external HLS, and the optional
    /// `ops/inherited-ops-unavailable` check is not emitted).
    fn check_inherited_op_index(
        &self,
        obu: &ObuEnvelope<'_>,
        ops: &OperatingPointSet,
        xlayer_index: u8,
        ref_ops_id: u8,
        op_index: u8,
        report: &mut ValidationReport,
    ) {
        let out_of_range = if ref_ops_id == ops.ops_id {
            op_index >= ops.ops_cnt || op_index >= xlayer_index
        } else if let Some(referenced) = self.ops.get(ops.xlayer_id, ref_ops_id) {
            op_index >= referenced.ops_cnt
        } else {
            return;
        };

        if out_of_range {
            report.push(
                Diagnostic::error(
                    "ops/inherited-op-index-out-of-range",
                    format!(
                        "OPS {} payload extended layer {} inherits from ops_embedded_ops_id {} \
                         ops_embedded_op_index {}, which is out of range for the referenced \
                         operating point set",
                        ops.ops_id, xlayer_index, ref_ops_id, op_index
                    ),
                )
                .with_spec_section("6.10.2")
                .with_byte_offset(obu.offset),
            );
        }
    }

    /// Returns the activated in-band sequence header's general fields for `xlayer`,
    /// if any. The fields are copied out so callers can keep mutating `self`.
    fn active_general_for(
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
    fn frame_confirmed_xlayers_in_current_cmvs(&self) -> Vec<ExtendedLayerId> {
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
    fn agreement_activation_for(
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
    fn frame_confirmed_activation_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> Option<(SequenceHeaderId, SequenceHeaderGeneral)> {
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return None;
        }
        self.active_general_for(xlayer)
    }

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
    /// Two correctness properties of this resolution (codex findings 3393129738 /
    /// 3393129741):
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
    fn activated_global_lcr(&self) -> Option<(u8, &GlobalLcrRecord)> {
        let cmvs_start = self.cmvs.current_cmvs_start_tu_index()?;
        self.activated_global_lcr_in_window(cmvs_start)
    }

    /// As [`Self::activated_global_lcr`], but resolves against an explicit CMVS-window start.
    /// [`Self::activated_global_lcr`] passes the live window; this seam keeps the window start
    /// an explicit parameter for callers that resolve against a non-live window.
    fn activated_global_lcr_in_window(&self, cmvs_start: u64) -> Option<(u8, &GlobalLcrRecord)> {
        // § 6.8.2 "present in the same CMVS": the associated record must have been observed at
        // or after the CMVS-window start. (The activation xlayer is not consulted here — only
        // the record's observation temporal unit bounds the window.)
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
    /// no activation of its own — both rule sets end the CMVS there, so there is no mismatch
    /// (codex finding 3393274375). The scope is the *activation* temporal unit, not the global
    /// record's observation temporal unit, because a same-id CLK re-references an already-active
    /// header (re-activating in the boundary TU) without re-sending its sequence header — so the
    /// association snapshot keeps the global record's earlier observation timestamp while the
    /// activation is genuinely in the boundary TU.
    fn activated_global_lcr_in_tu(&self, boundary_tu_index: u64) -> Option<(u8, &GlobalLcrRecord)> {
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
    fn activated_global_lcr_where(
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

    /// Checks explicitly signalled OPS maps against the sequence header activated
    /// for each entry's extended layer (AV2 § 6.10.7): for any included embedded
    /// layer `cMId` with `MLayerDependencyMap[cMId][rMId] == 1`, embedded layer
    /// `rMId` must also be included, and likewise per temporal-layer map under
    /// `TLayerDependencyMap`. An entry whose extended layer has no decidable
    /// activated in-band sequence header is skipped (the maps are never
    /// fabricated or guessed; see `agreement_activation_for`), the whole check
    /// is suppressed when external HLS declares any sequence header, and the
    /// [`DependencyFindingKey`] dedup makes activation-time re-checks
    /// idempotent.
    fn check_ops_entries_against_active(
        &mut self,
        ops_offset: ByteOffset,
        ops_id: u8,
        entries: &[OpsExplicitEntry],
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        for entry in entries {
            let Some((seq_header_id, general)) = self.agreement_activation_for(entry.xlayer_id)
            else {
                continue;
            };

            if let Some((curr, reference)) =
                mlayer_closure_violation(entry.info.mlayer_map, &general.mlayer_dependency_map)
            {
                let key = DependencyFindingKey::Ops {
                    ops_offset,
                    payload_index: entry.payload_index,
                    entry_xlayer: entry.xlayer_id,
                    seq_header_id,
                    map: DependencyMapKind::Mlayer,
                };
                if self.emitted_dependency_findings.insert(key) {
                    report.push(
                        Diagnostic::error(
                            "ops/mlayer-dependency-missing",
                            format!(
                                "OPS {ops_id} operating point {} for extended layer {} includes \
                                 embedded layer {curr} but not embedded layer {reference}, which \
                                 the activated sequence header {}'s \
                                 MLayerDependencyMap[{curr}][{reference}] requires",
                                entry.payload_index,
                                entry.xlayer_id.get(),
                                seq_header_id.get(),
                            ),
                        )
                        .with_spec_section("6.10.7")
                        .with_byte_offset(ops_offset),
                    );
                }
            }

            for &(mlayer, tlayer_mask) in &entry.info.tlayer_maps {
                let Some((curr, reference)) =
                    tlayer_closure_violation(mlayer, tlayer_mask, &general.tlayer_dependency_map)
                else {
                    continue;
                };
                let key = DependencyFindingKey::Ops {
                    ops_offset,
                    payload_index: entry.payload_index,
                    entry_xlayer: entry.xlayer_id,
                    seq_header_id,
                    map: DependencyMapKind::Tlayer { mlayer },
                };
                if self.emitted_dependency_findings.insert(key) {
                    report.push(
                        Diagnostic::error(
                            "ops/tlayer-dependency-missing",
                            format!(
                                "OPS {ops_id} operating point {} for extended layer {} includes \
                                 temporal layer {curr} of embedded layer {mlayer} but not \
                                 temporal layer {reference}, which the activated sequence header \
                                 {}'s TLayerDependencyMap[{mlayer}][{curr}][{reference}] requires",
                                entry.payload_index,
                                entry.xlayer_id.get(),
                                seq_header_id.get(),
                            ),
                        )
                        .with_spec_section("6.10.7")
                        .with_byte_offset(ops_offset),
                    );
                }
            }
        }
    }

    /// Runs the § 6.10.7 / § 6.8.9 agreement checks that become decidable when a
    /// sequence header is newly activated (or re-activated to a different id) for
    /// `xlayer`: the stored explicit maps of active OPS records describing the
    /// layer (its local bucket plus global-OPS entries), and the § 6.8.9 pairing
    /// through the activated header's `seq_lcr_id`. The dedup keys make repeated
    /// activation idempotent.
    fn on_sequence_activation(
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

    /// Records the frame-confirmed sequence-header activation for `xlayer` into the Annex A
    /// Table A.4 IOP pending facts: the header's profile (for the interoperability point),
    /// its embedded-layer count (`seq_max_mlayer_cnt_minus_1 + 1`), and the
    /// `LcrMaxNumXLayerCount` of the *activated* global LCR its `seq_lcr_id` resolves to
    /// (only an activated global LCR is recorded — the Table A.4 global-LCR arms require an
    /// activated record, lesson 10). Only a frame-confirmed activation is recorded (a staged
    /// fallback guess could be contradicted by a later frame, § 7.3.6). Suppressed under any
    /// Provided external HLS (`matches!(.., Provided(_))`): an external header may shadow the
    /// in-band one, so in-band presence counting is unsound — the same gate the window
    /// evaluation uses (`evaluate_annex_a_iop_window`).
    fn note_annex_a_iop_activation(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
    ) {
        if matches!(options.external_hls, ExternalHlsMode::Provided(_)) {
            return;
        }
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return;
        }
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        let offset = self
            .sequence_header_offsets
            .get(&seq_header_id)
            .copied()
            .unwrap_or(ByteOffset::new(0));
        // The activated global LCR span for this layer, if its association resolved a
        // global record. Only an *activated* (associated) global LCR contributes the
        // Table A.3 declared count / satisfies the Table A.4 global-LCR arms.
        //
        // Read `LcrMaxNumXLayerCount` from the association-time snapshot
        // (`association.global_record`), NOT a live `global_lcr_records` lookup, exactly like
        // the § 6.8.2 agreement path (`activated_global_lcr_in_window`). A same-id global-LCR
        // redefinition *after* this header associated otherwise retargets the count to the
        // later revision's `lcr_xlayer_map`; the snapshot keeps the Table A.4 layer accounting
        // pinned to the revision this header actually associated to.
        let activated_global_count = self
            .lcr_associations
            .get(&(xlayer, seq_header_id))
            .filter(|a| a.lcr_is_global)
            .and_then(|a| a.global_record.as_ref())
            .map(|record| record.max_num_xlayer_count);
        self.annex_a_iop.note_activation(
            general.seq_profile_idc.get(),
            u32::from(general.seq_max_mlayer_count.get()),
            activated_global_count,
            offset,
        );
    }

    /// Emits the Annex A.2 / Annex A.4 profile and level/tier *value-space* diagnostics
    /// for the sequence header activated for `xlayer`, once per activated header per
    /// coded video sequence:
    ///
    /// - `annex-a/profile-reserved` (error, Annex A.2 Table A.1, mirror line 85):
    ///   `seq_profile_idc` in the reserved range `5..=30`.
    /// - `annex-a/profile-chroma-format-mismatch` (error, Annex A.2 Table A.1, mirror
    ///   lines 61-90): `chroma_format_idc` outside the activated profile's allowed set.
    ///   Skipped for the Configurable profile (31), whose Table A.1 chroma column is a
    ///   dash, and for a reserved profile (the reserved-profile error already fires and
    ///   no allowed set is defined).
    /// - `annex-a/profile-bit-depth-mismatch` (error, Annex A.2 Table A.1, mirror lines
    ///   61-90): `bit_depth_idc` not `0` or `1` for profiles `0..=4`. Skipped for the
    ///   Configurable profile. The parsed [`BitDepthIdc`] only models `0`/`1`, so a
    ///   sequence header that reaches activation always has an in-range bit depth; this
    ///   check is defensive and currently never fires (documented below).
    /// - `annex-a/level-reserved` (error, Annex A.4 Table A.7, mirror line 321):
    ///   `seq_level_idx` in the reserved range `22..=30`. The Maximum-parameters value
    ///   31 is valid and not flagged.
    /// - `annex-a/high-tier-below-4-0` (warning, Annex A.4 Table A.9 NOTE, mirror lines
    ///   436-437): `seq_tier == High` with `seq_level_idx < 4` (level 4.0). Warning, not
    ///   error: the only spec statement is the informative Table A.9 NOTE ("seq_tier
    ///   equal to 1 can only be signaled for level 4.0 and above") plus the undefined
    ///   HighMbps/HighCR cells, so error severity would overclaim a non-normative source.
    ///   This sequence-header arm is syntax-*unreachable*: the § 5.4.1 parser only reads
    ///   `seq_tier` when `seq_level_idx > 3`, so a parseable header below level 4.0
    ///   always infers `Tier::Main`. The *reachable* arm is the OPS path — a
    ///   sub-bitstream's `seq_tier`/`seq_level_idx` may be derived from the OPS-signaled
    ///   `ops_tier_flag`/`ops_level_idx` (mirror lines 443-451), and the OPS PTL syntax
    ///   carries `ops_tier_flag` unconditionally (§ 5.11.2) — so it is also checked, in
    ///   [`check_ops_level_tier_value_space`].
    ///
    /// Anchored at the defining sequence-header OBU ([`Self::sequence_header_offsets`]),
    /// not the activating frame OBU.
    ///
    /// Emitted only for a *frame-confirmed* in-band activation (`frame_confirmed_xlayers`,
    /// the § 5.18.2 `load_sequence_header` path): a staged-but-unactivated header that no
    /// frame has loaded does not fire (§ 7.3.6 permits staging several headers, so the
    /// OBU-order fallback — even when momentarily the sole candidate — is a guess a later
    /// frame can contradict). Unlike the agreement checks, this runs even when the caller
    /// declares external HLS, because the active header recorded for `xlayer` is always
    /// the in-band one and its value-space facts are locally decidable regardless of any
    /// external sequence header (see [`Self::on_sequence_activation`]).
    fn check_annex_a_value_space(
        &mut self,
        xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        // Emit only for a *frame-confirmed* activation — one a parsed frame-header
        // reference loaded (§ 5.18.2 load_sequence_header). The OBU-order first-seen
        // fallback is a guess (§ 7.3.6 permits staging headers before any frame
        // activates one): even while a staged header is momentarily the sole in-band
        // candidate it can be superseded by a later staged header that a frame then
        // references instead, and a value-space error already emitted against the guess
        // could not be retracted. So, unlike the § 6.10.7 / § 6.8.9 agreement checks
        // (whose `agreement_activation_for` also admits the sole-header shortcut because
        // they emit nothing without an OPS/LCR present), the Annex A value-space check —
        // which fires unconditionally on a reserved/mismatched field — defers entirely to
        // frame-driven activation and re-enters here the moment the frame confirms it.
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return;
        }
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        let offset = self
            .sequence_header_offsets
            .get(&seq_header_id)
            .copied()
            .unwrap_or(ByteOffset::new(0));
        // Emit once per activated header per coded video sequence: the same activation
        // can be re-confirmed by multiple frames in one coded video sequence (and a CLK
        // re-activation across a coded-video-sequence boundary legitimately re-emits). The
        // key carries a fingerprint of the checked value-space fields (§ 7.3.6 permits a
        // same-`seq_header_id` redefinition with different content): a redefinition that
        // changes any field this check inspects re-runs the checks rather than being
        // suppressed by the original activation's key.
        let epoch = self.cvs.cvs_generation_epoch(xlayer);
        let fingerprint = annex_a_value_space_fingerprint(&general);
        if !self
            .emitted_annex_a_value_space
            .insert((xlayer, seq_header_id, epoch, fingerprint))
        {
            return;
        }

        let profile_idc = general.seq_profile_idc.get();
        let level_idx = general.seq_level_idx.get();
        let is_configurable = profile_idc == crate::annex_a::CONFIGURABLE_PROFILE_IDC;

        // Annex A.2 Table A.1: reserved seq_profile_idc (5-30).
        if is_reserved_profile(profile_idc) {
            report.push(
                Diagnostic::error(
                    "annex-a/profile-reserved",
                    format!(
                        "seq_profile_idc {profile_idc} is reserved (5..=30); it conforms to no \
                         AV2 profile defined in this version of the specification"
                    ),
                )
                .with_spec_section("A.2")
                .with_byte_offset(offset),
            );
        } else if !is_configurable {
            // Annex A.2 Table A.1: chroma_format_idc must be in the profile's allowed
            // set. Configurable (31) and reserved profiles have no defined set, so the
            // mismatch check applies only to profiles 0-4.
            if !profile_allows_chroma(profile_idc, general.chroma_format_idc) {
                report.push(
                    Diagnostic::error(
                        "annex-a/profile-chroma-format-mismatch",
                        format!(
                            "chroma_format_idc {} is not in the allowed set of seq_profile_idc {}",
                            general.chroma_format_idc.get(),
                            profile_idc
                        ),
                    )
                    .with_spec_section("A.2")
                    .with_byte_offset(offset),
                );
            }
            // Annex A.2 Table A.1: bit_depth_idc must be 0 or 1 for profiles 0-4. The
            // parsed BitDepthIdc enum only represents 0 (10-bit) and 1 (8-bit) — any
            // other value is rejected at parse time as BitDepthOutOfRange before a
            // header can be activated — so this branch is defensively never reachable
            // today; it is kept to make the Table A.1 column explicit and to remain
            // correct if a future profile widens the bit-depth value space.
            let bit_depth_value = general.bit_depth_idc.get();
            if bit_depth_value > 1 {
                report.push(
                    Diagnostic::error(
                        "annex-a/profile-bit-depth-mismatch",
                        format!(
                            "bit_depth_idc {bit_depth_value} is not 0 or 1, the only values \
                             allowed for seq_profile_idc {profile_idc}"
                        ),
                    )
                    .with_spec_section("A.2")
                    .with_byte_offset(offset),
                );
            }
        }

        // Annex A.4 Table A.7: reserved seq_level_idx (22-30).
        if is_reserved_level(level_idx) {
            report.push(
                Diagnostic::error(
                    "annex-a/level-reserved",
                    format!(
                        "seq_level_idx {level_idx} is reserved (22..=30); it maps to no AV2 level \
                         defined in this version of the specification"
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(offset),
            );
        }

        // Annex A.4 Table A.9 NOTE (informative): seq_tier == 1 (High) can only be
        // signaled for level 4.0 (LevelIdx 4) and above. Warning, not error: the source
        // is a non-normative NOTE plus the undefined HighMbps/HighCR cells below 4.0.
        if matches!(general.seq_tier, Tier::High) && level_idx < HIGH_TIER_MIN_LEVEL_IDX {
            report.push(
                Diagnostic::warning(
                    "annex-a/high-tier-below-4-0",
                    format!(
                        "seq_tier is High (1) with seq_level_idx {level_idx} below level 4.0 \
                         (LevelIdx 4); the Table A.9 NOTE states High tier can only be signaled \
                         for level 4.0 and above (advisory: the source is an informative NOTE)"
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(offset),
            );
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
    fn check_substream_max_ceilings(
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
    fn check_doh_constraint_required(
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

    /// Resolves the deferred § 6.6 `msdo/doh-constraint-required` evaluation for a
    /// just-completed temporal unit, at temporal-unit-completion time (a temporal
    /// delimiter boundary or the end-of-bitstream flush), *after* the [`CmvsTracker`] has
    /// applied the temporal unit's § 7.3.2 begin/end conditions.
    ///
    /// The requirement is scoped to a coded multistream *video* sequence, so the
    /// evaluation only fires when the completed temporal unit resolved to a definitive
    /// CMVS [`CmvsState::Inside`] ([`CmvsTracker::committed_inside`]). Deferring to this
    /// point — rather than evaluating eagerly at sequence-header activation as the
    /// original landing did — handles both arrival-order corner cases the eager check
    /// missed (codex findings 3392940061 and 3392940072):
    ///
    /// - A same-id header redefinition with `monotonic_output_order_flag == 0` at the top
    ///   of a temporal unit that a later MSDO-less CLK ENDS (§ 7.3.2 end condition 2) sits
    ///   *outside* the CMVS. The eager check, gated on the still-`Inside` committed state
    ///   at activation, fired a false positive; this deferred path sees the resolved
    ///   `Outside`/`Unknown` and does not.
    /// - A same-id CLK that re-references an already-frame-confirmed active header opens a
    ///   CMVS at the CLK without re-entering `on_sequence_activation` (the seq id is
    ///   unchanged and the layer was already confirmed), so the eager activation-time path
    ///   never ran; this deferred path re-examines the in-CMVS frame-confirmed activations
    ///   against the resolved membership, so the transition to `Inside` is caught.
    ///
    /// It re-runs [`Self::check_doh_constraint_required`] for the frame-confirmed extended
    /// layers activated *within the current CMVS window*
    /// ([`Self::frame_confirmed_xlayers_in_current_cmvs`]), NOT the whole-history
    /// `frame_confirmed_xlayers` accumulator — so a non-monotonic header left active from an
    /// earlier, already-ended coded video sequence outside this CMVS does not trip the § 6.6
    /// MSDO DOH requirement against this CMVS's MSDO (codex finding 3393129745, the same
    /// whole-history scope bug the § 6.8.2 LCR DOH check had). The
    /// `(xlayer, seq_header_id, cvs_epoch)` dedup inside that method keeps a resolved
    /// evaluation from re-spamming a diagnostic across successive temporal units of the same
    /// CMVS.
    fn resolve_deferred_doh_constraint(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !self.cmvs.committed_inside() {
            return;
        }
        let xlayers = self.frame_confirmed_xlayers_in_current_cmvs();
        for xlayer in xlayers {
            self.check_doh_constraint_required(xlayer, options, report);
        }
    }

    /// Resolves the deferred § 6.8.2 MSDO↔global-LCR agreement (mirror
    /// `06-syntax-structures-semantics.md` lines 1646-1678) and the § 6.8.2 LCR
    /// DOH-constraint requirement (lines 1619-1621), at temporal-unit-completion time
    /// (a temporal-delimiter boundary or the end-of-bitstream flush), *after* the
    /// [`CmvsTracker`] has applied the temporal unit's § 7.3.2 begin/end conditions.
    ///
    /// The two checks have *different* presence preconditions (codex finding 3393129743):
    ///
    /// - The § 6.8.2 **agreement** constraints hold "when both an OBU with obu_type equal to
    ///   OBU_MSDO and an activated global layer configuration record OBU are present in the
    ///   same coded multistream video sequence" (mirror lines 1646-1648), so they fire only
    ///   when both an MSDO and an activated global LCR are present in the *current* CMVS.
    /// - The § 6.8.2 **LCR DOH** requirement (lines 1619-1621) is LCR-only — "when
    ///   monotonic_output_order_flag is equal to 0 in any activated sequence header of the
    ///   coded multistream video sequence, lcr_doh_constraint_flag shall be equal to 1". It
    ///   requires only an activated global LCR, *not* an MSDO: a global-LCR-only CMVS (legal
    ///   per the Annex A IOP2 Table A.4 rows, opened via § 7.3.2 begin condition 3) must
    ///   still satisfy it.
    ///
    /// Both gate on the association chain resolving an *activated* global LCR present in the
    /// current CMVS ([`Self::activated_global_lcr`], window-scoped) — an
    /// observed-but-never-activated global LCR, or one present only in an earlier CMVS,
    /// resolves nothing and triggers no diagnostic. The agreement additionally requires a
    /// recorded MSDO whose observation temporal unit lies within the current CMVS window
    /// (so a stale earlier-CMVS MSDO is not compared against this CMVS's global LCR). Gating
    /// on the chain-decidable `activated_global_lcr` rather than the conservative
    /// [`CmvsTracker::committed_inside`] is what lets the LCR-only requirement fire in the
    /// § 7.3.2 begin-condition-3 case the membership tracker routes to Unknown: the
    /// activation evidence is decidable from the association chain even when the tracker
    /// cannot soundly classify membership.
    ///
    /// In-band-only: when external HLS declares any sequence header the activation chain
    /// (and thus which global LCR, if any, is activated) is not reliably in-band, so the
    /// agreement is suppressed — the same `external_declares_sequence_header` gate the
    /// sibling agreement checks use. (The locally-decidable in-band §6.8.2 value-space
    /// checks are unaffected; they live on the stateless syntax path.)
    fn resolve_deferred_lcr_msdo_agreement(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        // The activated global LCR present in the current CMVS gates BOTH checks. This
        // resolution is window-scoped (§ 6.8.2 "present in the same CMVS") and decided from
        // the frame-confirmed association chain, so it fires even for an LCR-only CMVS the
        // membership tracker routes to Unknown.
        let Some(cmvs_start) = self.cmvs.current_cmvs_start_tu_index() else {
            return;
        };
        // CMVS-window starts only advance (TU indices are monotonic and a new CMVS opens at a
        // later temporal unit), so an MSDO observed before the current window can never be
        // in-window again — drop it to keep the accumulator bounded. Correctness rests on the
        // in-window filter below, not this prune.
        self.msdo_agreement_snapshots
            .retain(|snapshot| snapshot.observed_tu_index >= cmvs_start);
        let Some((global_id, global)) = self.activated_global_lcr() else {
            return;
        };
        // Snapshot the global record so the borrow on `self` is released before pushing
        // diagnostics through `&mut self` dedup state.
        let global = global.clone();

        // § 6.8.2 agreement: EVERY MSDO present in this CMVS must agree with the activated
        // global LCR (mirror lines 1646-1648). Evaluate each accumulated MSDO whose
        // observation temporal unit lies within the current CMVS window — a stale MSDO
        // recorded only in an earlier CMVS (its observation temporal unit precedes this CMVS's
        // window) is excluded. A per-MSDO last-wins overwrite of the live `msdo_substream_max`
        // would let an earlier non-conforming MSDO escape when a later conforming one replaced
        // it before this resolution; iterating the accumulator closes that hole (codex finding
        // 3393274380). The `emitted_lcr_agreement` key carries each MSDO's offset, so distinct
        // MSDOs fire distinctly and a re-resolution across the CMVS's temporal units does not
        // re-spam. Snapshot the in-window MSDOs before the `&mut self` push loop so the borrow
        // on `self.msdo_agreement_snapshots` is released.
        let in_window_msdos: Vec<(MsdoAggregate, ByteOffset)> = self
            .msdo_agreement_snapshots
            .iter()
            .filter(|snapshot| snapshot.observed_tu_index >= cmvs_start)
            .map(|snapshot| (snapshot.aggregate.clone(), snapshot.offset))
            .collect();
        for (msdo, msdo_offset) in in_window_msdos {
            self.check_lcr_msdo_agreement(global_id, &global, &msdo, msdo_offset, report);
        }

        // § 6.8.2 LCR DOH requirement: LCR-only, runs regardless of MSDO presence.
        self.check_lcr_doh_constraint_required(global_id, &global, report);
    }

    /// Resolves the deferred § 7.3.2 boundary-set-identity check
    /// (`cmvs/boundary-set-mismatch`, mirror `07-decoding-process.md` line 351) at
    /// temporal-unit-completion time, *after* the [`CmvsTracker`] has applied the temporal
    /// unit's begin/end conditions.
    ///
    /// > It is a requirement of bitstream conformance that, in a coded multistream video
    /// > sequence in which both an OBU_MSDO and an activated global layer configuration
    /// > record are present, the set of coded multistream video sequence boundaries
    /// > obtained by applying the rules of this section using both the MSDO and the
    /// > activated global layer configuration record shall be identical to the set of
    /// > boundaries obtained by applying those rules using the MSDO alone.
    ///
    /// **Decidable-disagreement-only (lesson 12 — Unknown never fires).** The only place
    /// the two boundary sets can diverge is § 7.3.2 end condition 2: a temporal unit that
    /// begins a new coded video sequence (a CLK) with no OBU_MSDO ENDS the CMVS under the
    /// MSDO-alone rules, but does NOT end it when it "has an activated global layer
    /// configuration record". The [`CmvsTracker`] flags this structural candidate
    /// ([`CmvsTracker::last_completed_is_boundary_divergence_candidate`]); the divergence is
    /// real, and decidable, only when the chain confirms the global LCR present *in the
    /// boundary temporal unit itself* is genuinely *activated*
    /// ([`Self::activated_global_lcr_in_tu`], scoped to the boundary TU index — not the whole
    /// CMVS window, codex finding 3393274375) and the CMVS it ended contained an MSDO
    /// (`msdo_substream_max`). End condition 2's "does not have an activated global layer
    /// configuration record" is a property of the BOUNDARY temporal unit, so an activated
    /// global LCR present only earlier in the CMVS keeps end condition 2 true at this boundary
    /// (both rule sets end the CMVS here — no mismatch). When both hold, the MSDO-alone set
    /// has a boundary the MSDO+LCR set lacks, so the requirement is violated. When the
    /// boundary TU's global LCR is only *present* but not activated (the tracker routes that
    /// to Unknown), nothing fires. The diagnostic anchors at the activated global LCR (the
    /// disagreeing record) and is deduped by the CMVS's MSDO offset.
    ///
    /// Suppressed when external HLS declares any sequence header (the activation chain that
    /// decides whether the global LCR is activated is then not reliably in-band) — the same
    /// gate the § 6.8.2 agreement uses.
    fn resolve_deferred_cmvs_boundary(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        if !self.cmvs.last_completed_is_boundary_divergence_candidate() {
            return;
        }
        // The boundary-identity requirement applies only when both an OBU_MSDO and an
        // activated global LCR are present in the CMVS (mirror line 351). The CMVS being
        // divergently-ended was definitively Inside, so it was opened by an MSDO; require a
        // recorded MSDO and a chain-confirmed activated global LCR.
        let Some(substream_max) = self.msdo_substream_max.as_ref() else {
            return;
        };
        let msdo_offset = substream_max.offset;
        // § 7.3.2 end condition 2 verbatim: a temporal unit "that begins a new coded video
        // sequence for at least one extended layer but does not contain an OBU with obu_type
        // equal to OBU_MSDO and does not have an activated global layer configuration record"
        // ends the CMVS. The MSDO-alone rules always end the CMVS at this CLK-with-no-MSDO
        // boundary TU; the MSDO+LCR rules end it too UNLESS the boundary TU "has an activated
        // global layer configuration record". The divergence — and the mismatch — therefore
        // exists ONLY when the BOUNDARY temporal unit itself has an activated global LCR. An
        // activated global LCR present only EARLIER in the CMVS does not make end condition 2
        // false at this boundary TU, so the resolution is scoped to the boundary temporal unit
        // (`activated_global_lcr_in_tu`), NOT the whole CMVS window (codex finding 3393274375):
        // when the boundary TU carries no activated global LCR, both rule sets end the CMVS
        // here and there is no mismatch.
        let Some(boundary_tu_index) = self.cmvs.last_completed_tu_index() else {
            return;
        };
        let Some((global_id, global)) = self.activated_global_lcr_in_tu(boundary_tu_index) else {
            return;
        };
        let global_offset = global.offset;
        if !self.emitted_cmvs_boundary.insert(msdo_offset) {
            return;
        }
        report.push(
            Diagnostic::error(
                "cmvs/boundary-set-mismatch",
                format!(
                    "§ 7.3.2: a temporal unit begins a new coded video sequence with no OBU_MSDO \
                     but with the activated global layer configuration record {global_id}, so it \
                     ends the coded multistream video sequence under the MSDO-alone boundary rules \
                     yet continues it under the MSDO-plus-global-LCR rules; § 7.3.2 requires the \
                     two boundary sets to be identical in a CMVS containing both an OBU_MSDO and an \
                     activated global LCR",
                ),
            )
            .with_spec_section("7.3.2")
            .with_byte_offset(global_offset),
        );
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
    fn check_lcr_msdo_agreement(
        &mut self,
        global_id: u8,
        global: &GlobalLcrRecord,
        msdo: &MsdoAggregate,
        msdo_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        // Constraint 1: num_streams_minus_2 + 2 == LcrMaxNumXLayerCount (line 1650).
        if msdo.num_streams != global.max_num_xlayer_count {
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
                    msdo.num_streams, global.max_num_xlayer_count,
                ),
                report,
            );
        }

        // Constraint 2: every sub_xlayer_id[i] is in LcrXLayerID[] (lines 1651-1652).
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

        // Constraint 3: aggregate-info agreement, gated on lcr_aggregate_info_present_flag
        // (lines 1657-1664).
        if let Some(agg) = global.aggregate_info {
            self.check_lcr_aggregate_agreement(
                global_id,
                &agg,
                msdo,
                msdo_offset,
                global.offset,
                report,
            );
        }

        // Constraint 4: per-substream PTL equality, gated on
        // lcr_seq_profile_tier_level_info_present_flag (lines 1666-1671).
        if global.seq_ptl_present {
            self.check_lcr_substream_ptl_agreement(global_id, global, msdo, msdo_offset, report);
        }

        // Constraint 5: multistream_doh_constraint_flag == lcr_doh_constraint_flag (line
        // 1673). The MSDO's flag travels in the snapshot argument, so the whole check
        // operates on `msdo`/`global` and never reaches back into the live
        // `msdo_substream_max` (which a later same-CMVS MSDO could have retargeted).
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
    fn check_lcr_aggregate_agreement(
        &mut self,
        global_id: u8,
        agg: &LcrAggregateInfo,
        msdo: &MsdoAggregate,
        msdo_offset: ByteOffset,
        global_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        // multistream_profile_idc consistent with lcr_config_idc per Annex A.3 Table A.6
        // (lines 1659-1660). Only a *defined* configuration (0..=2) has a value space; a
        // reserved lcr_config_idc is the § 6.8.4 Annex-A range residual, not this check.
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

        // The interoperability point of multistream_profile_idc (Annex A.2 Table A.1) equals
        // lcr_max_interop (lines 1661-1662). A reserved / Configurable profile has no
        // table-determined IOP, so the equality is undecidable there and is skipped (the
        // reserved-profile case is owned by annex-a/profile-reserved).
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

        // multistream_level_idx == lcr_aggregate_level_idx (line 1663).
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

        // multistream_tier == lcr_max_tier_flag (line 1664).
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
    fn check_lcr_substream_ptl_agreement(
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
    /// this CMVS's global LCR (codex finding 3393129745).
    fn check_lcr_doh_constraint_required(
        &mut self,
        global_id: u8,
        global: &GlobalLcrRecord,
        report: &mut ValidationReport,
    ) {
        // lcr_doh_constraint_flag == 1 already satisfies the requirement.
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
                // The dedup key uses the activating header offset so each disagreeing
                // header fires once; the global-LCR offset keeps redefinitions distinct.
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
    fn push_lcr_agreement(
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

    /// Commits the just-completed temporal unit's Annex A Table A.4 IOP pending facts to
    /// the right coded-(multistream-)video-sequence window (AV2 § 7.3.6 per-temporal-unit
    /// attribution, lesson 8). `completed_tu_index` is the temporal unit's index.
    ///
    /// When this temporal unit begins a NEW coded video sequence (it has a CLK and the open
    /// window's coded video sequence began in an *earlier* temporal unit), the prior window
    /// is first flushed and evaluated, then a fresh window is seeded from this temporal
    /// unit's pending facts — so a same-temporal-unit pre-CLK OBU_MSDO/LCR belongs to the
    /// NEW coded video sequence (§ 7.3.6: the new coded video sequence starts at the
    /// temporal unit containing the CLK). Otherwise the pending facts merge into the open
    /// window (the same coded video sequence continues across this temporal unit). The
    /// pending facts reset for the next temporal unit.
    fn commit_annex_a_iop_pending(
        &mut self,
        completed_tu_index: u64,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if self.annex_a_iop.pending_starts_new_cvs(completed_tu_index) {
            // This temporal unit begins the next coded video sequence: flush+evaluate the
            // ending window, then seed a fresh one from this temporal unit's pending facts.
            self.flush_annex_a_iop_window(options, report);
            let pending = std::mem::take(&mut self.annex_a_iop.pending);
            self.annex_a_iop.window = Some(AnnexAIopTracker::window_from_pending(
                &pending,
                completed_tu_index,
            ));
        } else {
            // The same coded video sequence continues (or leading evidence before the first
            // CLK): merge this temporal unit's pending facts into the open window.
            let pending = std::mem::take(&mut self.annex_a_iop.pending);
            let window = self
                .annex_a_iop
                .window
                .get_or_insert_with(AnnexAIopWindow::default);
            AnnexAIopTracker::merge_pending_into(window, &pending, completed_tu_index);
        }
        // Both branches above `std::mem::take` `self.annex_a_iop.pending`, leaving it at
        // `TuIopFacts::default()`, so an explicit `reset_pending()` here would be a no-op.
    }

    /// Takes the current Annex A Table A.4 IOP window and evaluates its MSDO/LCR
    /// interoperability-point presence requirements, resetting the window for the next coded
    /// video sequence. Suppressed (the window is taken but no diagnostic is emitted) under
    /// any Provided external HLS, which makes in-band presence counting unsound.
    fn flush_annex_a_iop_window(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let Some(window) = self.annex_a_iop.window.take() else {
            return;
        };
        if matches!(options.external_hls, ExternalHlsMode::Provided(_)) {
            return;
        }
        self.evaluate_annex_a_iop_window(&window, report);
    }

    /// Emits the Annex A Table A.4 MSDO/LCR interoperability-point presence diagnostics for
    /// one coded-(multistream-)video-sequence `window` (AV2 v1.0.0 Annex A.2 Table A.4,
    /// mirror `annex-a-profiles-levels-and-tiers.md` lines 178-201).
    ///
    /// The interoperability point is taken from the window's OBU_MSDO `multistream_profile_idc`
    /// when an MSDO is present (mirror lines 1659-1662), else from the window's
    /// frame-confirmed activated headers (lesson; see [`AnnexAIopWindow::iop`]). A window with
    /// no decidable single interoperability point (no in-band profile, a reserved /
    /// Configurable profile whose IOP is not table-determined, or mixed IOPs across layers) is
    /// a no-op — the Table A.4 row is not determinable.
    ///
    /// `E = "Number of Extended Layers > 1"` and `M = "Number of Embedded Layers > 1"` are the
    /// Table A.3 counts ([`annex_a_extended_layers`] in declared precedence, mirror lines
    /// 146-151; the embedded-layer maximum, lines 152-153). The Table A.4 rows, by IOP:
    ///
    /// - IOP0 (lines 183-185): MSDO prohibited when `!E`, required when `E`.
    /// - IOP1 (lines 187-191): `!E && !M` -> MSDO prohibited; `E && !M` -> MSDO required;
    ///   `!E && M` -> MSDO prohibited and a local LCR required. (`E && M` exceeds IOP1's Table
    ///   A.3 layer budget and has no Table A.4 row.)
    /// - IOP2 (lines 193-201): `!E && !M` -> MSDO prohibited; `E && !M` -> MSDO **or** an
    ///   activated global LCR required (either satisfies); `!E && M` -> MSDO prohibited and an
    ///   LCR (local or activated global) required; `E && M` -> (MSDO **and** local LCR) **or**
    ///   an activated global LCR required.
    ///
    /// Only an *activated* global LCR ([`AnnexAIopWindow::activated_global_count`], resolved
    /// via the association chain) satisfies the global-LCR arms (lesson 10); an
    /// observed-but-unactivated global LCR does not.
    fn evaluate_annex_a_iop_window(&self, window: &AnnexAIopWindow, report: &mut ValidationReport) {
        // The MSDO's multistream_profile_idc determines the IOP when an MSDO is present
        // (mirror lines 1659-1662); otherwise the activated headers' agreed IOP is used.
        let iop = match window.msdo_profile_idc {
            Some(profile) => match interoperability_point(profile) {
                Some(iop) => iop,
                // Reserved / Configurable multistream_profile_idc: IOP not table-determined.
                None => return,
            },
            None => match window.iop {
                Some(AnnexAIopState::Single(iop)) => iop,
                // No in-band profile, or activated profiles disagree: row not determinable.
                _ => return,
            },
        };
        let extended_layers = annex_a_extended_layers(window);
        let e = extended_layers > 1;
        let m = window.max_embedded_layers.max(1) > 1;
        let offset = window.anchor_offset;
        let global_lcr = window.activated_global_count.is_some();
        // TODO(spec: AV2-A-LEVELS-TIERS): the Table A.3 layer-budget bound (the combination
        // flag must be 0 for IOP 0/1, mirror lines 154-158) is not enforced here; an IOP1
        // window with both E and M exceeds that budget but has no Table A.4 row, so Table A.4
        // alone makes no presence requirement for it.
        match iop {
            InteroperabilityPoint::Iop0 => {
                // Rows 1-2 (lines 183-185): embedded layers are N/A.
                self.emit_iop_msdo_presence(e, window, extended_layers, offset, report);
            }
            InteroperabilityPoint::Iop1 => {
                if !m {
                    // Rows 3-4 (lines 187-189): MSDO prohibited (!E) / required (E).
                    self.emit_iop_msdo_presence(e, window, extended_layers, offset, report);
                } else if !e {
                    // Row 5 (line 191): !E && M -> MSDO prohibited; local LCR required.
                    self.emit_msdo_prohibited(window, offset, report);
                    self.emit_iop1_local_lcr_required(window, offset, report);
                }
                // E && M: no Table A.4 row (outside IOP1's layer budget); see the TODO.
            }
            InteroperabilityPoint::Iop2 => {
                self.evaluate_iop2(e, m, window, global_lcr, extended_layers, offset, report);
            }
        }
    }

    /// Table A.4 IOP0 rows and IOP1 `!M` rows: MSDO required when `E`, prohibited when `!E`.
    fn emit_iop_msdo_presence(
        &self,
        e: bool,
        window: &AnnexAIopWindow,
        extended_layers: u32,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if e {
            if !window.msdo_present {
                report.push(annex_a_iop_error(
                    "annex-a/msdo-required-for-iop",
                    offset,
                    format!(
                        "Annex A Table A.4: the coded video sequence has more than one extended \
                         layer ({extended_layers}) but contains no OBU_MSDO, which the activated \
                         profile's interoperability point requires"
                    ),
                ));
            }
        } else {
            self.emit_msdo_prohibited(window, offset, report);
        }
    }

    /// Table A.4 IOP2 rows (mirror lines 193-201). `global_lcr` is whether an *activated*
    /// global LCR is present in the window (only an activated one satisfies the global-LCR
    /// arms).
    #[allow(clippy::too_many_arguments)]
    fn evaluate_iop2(
        &self,
        e: bool,
        m: bool,
        window: &AnnexAIopWindow,
        global_lcr: bool,
        extended_layers: u32,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        match (e, m) {
            // Row "2 N N" (line 193): MSDO prohibited.
            (false, false) => self.emit_msdo_prohibited(window, offset, report),
            // Row "2 Y N" (line 195): MSDO or an activated global LCR required (either
            // satisfies); MSDO is not prohibited here.
            (true, false) => {
                if !window.msdo_present && !global_lcr {
                    report.push(annex_a_iop_error(
                        "annex-a/msdo-required-for-iop",
                        offset,
                        format!(
                            "Annex A Table A.4: interoperability point 2 with more than one \
                             extended layer ({extended_layers}) requires an OBU_MSDO or an \
                             activated global OBU_LAYER_CONFIGURATION_RECORD, but neither is \
                             present in the coded video sequence"
                        ),
                    ));
                }
            }
            // Row "2 N Y" (line 197): MSDO prohibited; LCR (local or activated global)
            // required.
            (false, true) => {
                self.emit_msdo_prohibited(window, offset, report);
                if !global_lcr && !window.local_lcr_present {
                    report.push(annex_a_iop_error(
                        "annex-a/lcr-required-for-iop",
                        offset,
                        "Annex A Table A.4: interoperability point 2 with more than one embedded \
                         layer requires a local or activated global \
                         OBU_LAYER_CONFIGURATION_RECORD, but none is present in the coded video \
                         sequence"
                            .to_owned(),
                    ));
                }
            }
            // Row "2 Y Y" (lines 199-200): (MSDO and local LCR) or an activated global LCR
            // required.
            (true, true) => {
                let satisfied = (window.msdo_present && window.local_lcr_present) || global_lcr;
                if !satisfied {
                    report.push(annex_a_iop_error(
                        "annex-a/lcr-required-for-iop",
                        offset,
                        "Annex A Table A.4: interoperability point 2 with more than one extended \
                         layer and more than one embedded layer requires either an OBU_MSDO plus a \
                         local OBU_LAYER_CONFIGURATION_RECORD, or an activated global \
                         OBU_LAYER_CONFIGURATION_RECORD, but neither combination is present in the \
                         coded video sequence"
                            .to_owned(),
                    ));
                }
            }
        }
    }

    /// Emits `annex-a/msdo-prohibited-for-iop` when an MSDO is present in a window whose
    /// Table A.4 row prohibits one.
    ///
    /// This is the documented *defensive* arm. Under the Table A.3 "Number of Extended
    /// Layers" definition ([`annex_a_extended_layers`], declared precedence), a present
    /// OBU_MSDO declares `num_streams_minus_2 + 2 >= 2`, so `E = extended_layers > 1` is
    /// always true when `msdo_present` is true. Every Table A.4 "MSDO Prohibited" row
    /// requires `E` to be false (`E == 1`), so a caller reaching this method with `!E`
    /// already has `!msdo_present`, and this body never fires in-band today. The genuine
    /// violation the prohibition rows would catch — an MSDO declaring substreams that never
    /// materialize as distinct extended layers — is the declared-vs-observed reconciliation
    /// owned by the § 6.6 sub-stream change, not this presence window. The id stays emitted
    /// (and registered) so a future declared-vs-observed model can reach it.
    fn emit_msdo_prohibited(
        &self,
        window: &AnnexAIopWindow,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if window.msdo_present {
            report.push(annex_a_iop_error(
                "annex-a/msdo-prohibited-for-iop",
                offset,
                "Annex A Table A.4: the coded video sequence does not have more than one extended \
                 layer, so an OBU_MSDO is prohibited for the activated profile's interoperability \
                 point"
                    .to_owned(),
            ));
        }
    }

    /// Emits `annex-a/lcr-required-for-iop` for the IOP1 `!E && M` "Required (Local)" row
    /// (mirror line 191) when no local LCR is present in the window.
    fn emit_iop1_local_lcr_required(
        &self,
        window: &AnnexAIopWindow,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if !window.local_lcr_present {
            report.push(annex_a_iop_error(
                "annex-a/lcr-required-for-iop",
                offset,
                "Annex A Table A.4: interoperability point 1 with more than one embedded layer \
                 requires a local OBU_LAYER_CONFIGURATION_RECORD, but none is present in the coded \
                 video sequence"
                    .to_owned(),
            ));
        }
    }

    /// Emits `sequence-state/monotonic-output-order-mismatch` (AV2 § 6.4.1) when, with
    /// the § 7.3.2 CMVS tracker definitively *inside* a coded multistream video
    /// sequence, the sequence header just activated for `xlayer` disagrees on
    /// `monotonic_output_order_flag` with the active sequence header of any other
    /// extended layer.
    ///
    /// AV2 § 6.4.1 (mirror `06-syntax-structures-semantics.md` lines 324-325): "It is a
    /// requirement of bitstream conformance that in a coded multistream video sequence,
    /// all extended layers shall be associated with the same value of
    /// monotonic_output_order_flag." The requirement is scoped to a CMVS, so the check
    /// fires only in [`CmvsState::Inside`] — never in `Outside` or `Unknown`
    /// (conservative under-approximation; the CMVS tracker is the only oracle, as no
    /// real multistream conformance vectors exist). `byte_offset` locates the activating
    /// OBU.
    ///
    /// Both sides of the comparison use only *decidable* activations
    /// ([`Self::agreement_activation_for`]): a frame-confirmed activation, or the
    /// OBU-order fallback while it is the sole in-band candidate. § 7.3.6 permits
    /// "additional sequence header OBUs with a different seq_header_id ... not activated
    /// ... until referenced by a subsequent CLK frame header", so an extended layer
    /// whose only in-band header is an as-yet-unreferenced first-seen guess that a
    /// later frame can contradict is not yet associated with a flag — comparing against
    /// that guess would emit an error a retraction could not undo. The activating layer
    /// `xlayer` is likewise skipped until its own activation is decidable.
    ///
    /// The verdict is routed through [`CmvsTracker::monotonic_verdict`]: when the
    /// activation is observed at a sequence-header OBU *before* any CLK in the temporal
    /// unit, the committed `Inside` is only provisional (a later MSDO-less CLK could end
    /// the CMVS, § 7.3.2 end condition 2), so a disagreement is deferred and resolved at
    /// temporal-unit completion. This defers-and-drops the § 7.3.6-permitted same-CVS
    /// redefinition that immediately precedes the CLK beginning the new coded video
    /// sequence (mirror `07-decoding-process.md` lines 608-611), which the eager
    /// header-time emission would otherwise flag as a false positive.
    fn check_monotonic_output_order_agreement(
        &mut self,
        xlayer: ExtendedLayerId,
        byte_offset: ByteOffset,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // An externally activated sequence header has an unmodeled
        // monotonic_output_order_flag, so the cross-layer comparison is unreliable. Only a
        // *declared* external sequence header suppresses it: `ExternalHlsSet` cannot
        // declare external MSDO/LCR objects, but the CmvsTracker enters Inside only on
        // definitive in-band evidence and missing external MSDO/LCR evidence errs toward
        // Outside/Unknown (false-negative-only), so undeclared external objects cannot
        // make Inside spurious — narrowing this gate to declares_any_sequence_header() (as
        // the sibling gates and validate_active_sequence_limits do) is sound.
        if external_declares_sequence_header(options) {
            return;
        }
        // § 6.4.1 scopes the agreement to a coded multistream video sequence. Decide
        // whether the check fires now, is deferred (provisional Inside), or is skipped.
        let verdict = self.cmvs.monotonic_verdict();
        if matches!(verdict, MonotonicVerdict::Skip) {
            return;
        }
        // The activating layer's flag, only when its activation is decidable (a
        // frame-confirmed reference, or the sole in-band candidate). A first-seen
        // OBU-order fallback that several headers could contradict is not yet an
        // association (§ 7.3.6).
        let Some((_, general)) = self.agreement_activation_for(xlayer) else {
            return;
        };
        let flag = general.monotonic_output_order_flag;
        let mut disagreements = Vec::new();
        for &other_xlayer in self.active_sequence_by_xlayer.keys() {
            if other_xlayer == xlayer {
                continue;
            }
            // Compare only against another extended layer whose activation is equally
            // decidable; an unconfirmed first-seen guess for the other layer is not yet
            // associated with a flag, so a disagreement against it could be retracted by
            // a later frame and must not be emitted.
            let Some((_, other_general)) = self.agreement_activation_for(other_xlayer) else {
                continue;
            };
            if other_general.monotonic_output_order_flag != flag {
                disagreements.push(
                    Diagnostic::error(
                        "sequence-state/monotonic-output-order-mismatch",
                        format!(
                            "obu_xlayer_id {} activates a sequence header with \
                             monotonic_output_order_flag {} but obu_xlayer_id {} is associated \
                             with monotonic_output_order_flag {} in the same coded multistream \
                             video sequence; all extended layers must agree",
                            xlayer.get(),
                            u8::from(flag),
                            other_xlayer.get(),
                            u8::from(other_general.monotonic_output_order_flag)
                        ),
                    )
                    .with_spec_section("6.4.1")
                    .with_byte_offset(byte_offset),
                );
            }
        }
        for diagnostic in disagreements {
            match verdict {
                MonotonicVerdict::EmitNow => report.push(diagnostic),
                MonotonicVerdict::Defer => self.cmvs.queue_provisional_monotonic(diagnostic),
                MonotonicVerdict::Skip => {}
            }
        }
    }

    /// Emits the § 6.4.13 cross-CVS buffer-delay advisory
    /// (`decoder-model/buffer-delay-sum-changed-across-cvs`, severity `warning`) for the
    /// activated sequence header of `xlayer`:
    ///
    /// > "For a video sequence that includes one or more random access points the sum
    /// > of decoder_buffer_delay and encoder_buffer_delay shall be kept constant."
    /// > (AV2 v1.0.0 § 6.4.13,
    /// > `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md` lines 1301–1302.)
    ///
    /// Only the *advisory* tier exists for the sequence-header variant: within one
    /// coded video sequence the activated header is bit-identical (§ 7.3.6,
    /// `07-decoding-process.md` lines 604–610, already enforced by
    /// `hls/repeated-sequence-header-not-identical`), so a non-vacuous comparison must
    /// span a CLK boundary — which is conforming under the per-CVS reading of the
    /// unspecified "video sequence" scope. The advisory therefore fires only across a
    /// CLK boundary (a CVS-epoch change) and asserts only the broad reading.
    ///
    /// This check is driven only from the frame path
    /// (`observe_frame_bearing_obu`), so the activation is always frame-confirmed
    /// (the § 5.18.2 `load_sequence_header` reference): a fallback-guess activation
    /// resolved by OBU order never establishes or triggers the baseline, matching the
    /// design's "frame-confirmed activations only". Only an explicit
    /// `seq_decoder_model_info()` sum is compared; the Annex E defaults
    /// (`annex-e-decoder-model.md` lines 261–272) are not signalled values. Per
    /// Annex E.1 (mirror lines 24–25) a frame-confirmed activation of a header WITHOUT
    /// explicit decoder-model info clears this layer's stored baseline (the previous
    /// parameters do not persist), so a later explicit header is not compared against
    /// vanished parameters.
    ///
    /// `ExternalHlsMode::Provided` suppresses the advisory unconditionally (externally
    /// supplied HLS may legitimately differ), matching the OPS tier
    /// (`check_ops_buffer_delay_sums`).
    fn check_seq_buffer_delay_sum(
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

    /// Takes the § 6.4.1 LCR-association snapshot for one observed sequence
    /// header: `seq_lcr_id == 0` or an unresolved reference clears any previous
    /// snapshot (the latest observation of an id defines its association);
    /// otherwise the local-first-then-global resolution against the LCRs present
    /// prior to this header is stored, with the resolved record's embedded-layer
    /// maps as of this observation.
    fn snapshot_lcr_association(
        &mut self,
        xlayer: ExtendedLayerId,
        seq_header_id: SequenceHeaderId,
        seq_lcr_id: u8,
    ) {
        let key = (xlayer, seq_header_id);
        if seq_lcr_id == 0 {
            // AV2 § 6.4.1: seq_lcr_id == 0 means no LCR is associated.
            self.lcr_associations.remove(&key);
            return;
        }
        let association = if self.hls.has_local_lcr(xlayer, seq_lcr_id) {
            Some(LcrAssociation {
                lcr_is_global: false,
                lcr_id: seq_lcr_id,
                maps: self.hls.local_lcr_embedded(xlayer, seq_lcr_id).cloned(),
                // A local association carries no § 6.8.2 global-agreement record.
                global_record: None,
                // § 6.8.5/§ 6.8.8 snapshot the local record's PTL / rep-info present
                // *prior to this header* (the same discipline as `maps`), so a later
                // same-id local redefinition cannot retarget the ceiling/equality
                // comparison. The § 6.8.5 sentences key the ceiling on the *local* LCR.
                ptl: self.hls.local_lcr_ptl(xlayer, seq_lcr_id).copied(),
                rep_info: self.hls.local_lcr_rep_info(xlayer, seq_lcr_id).copied(),
            })
        } else if self.hls.global_lcr_xlayer_map(seq_lcr_id).is_some() {
            Some(LcrAssociation {
                lcr_is_global: true,
                lcr_id: seq_lcr_id,
                maps: self.hls.global_lcr_embedded(seq_lcr_id, xlayer).cloned(),
                // § 6.8.2: snapshot the full global record present *prior to this header*
                // so a later same-id redefinition cannot retarget the agreement (codex
                // finding 3393129741). `has_local_lcr` failing and the xlayer map being
                // present means the chain resolved to this in-band global record.
                global_record: self.global_lcr_records.get(&seq_lcr_id).cloned(),
                // § 6.8.5/§ 6.8.8: a global association reads the global record's PTL /
                // rep-info for this xlayer, snapshotted alongside `global_record`.
                ptl: self.hls.global_lcr_ptl(seq_lcr_id, xlayer).copied(),
                rep_info: self.hls.global_lcr_rep_info(seq_lcr_id, xlayer).copied(),
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

    /// AV2 § 6.8.9: the activated LCR's `lcr_mlayer_map[isGlobal][xId]` /
    /// `lcr_tlayer_map[isGlobal][xId][cMId]`, if present, must be
    /// dependency-closed under the activated sequence header's maps. The pairing
    /// is the sequence header activated for `xlayer` and that header's § 6.4.1
    /// LCR association — the snapshot taken at the header's latest observation
    /// (see [`ValidatorContext::lcr_associations`]), NOT a live resolution: a
    /// record redefined after the header is not the associated one. Only the
    /// `xId == xlayer` entry is constrained by this activation. Unresolved
    /// references are owned by the existing § 7.3.8.3 availability diagnostics,
    /// and an association without embedded-layer info has nothing to check. The
    /// diagnostics carry the associated LCR OBU's byte offset.
    fn check_lcr_dependency_agreement(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // Suppressed under any Provided external-HLS mode. This check pairs the in-band
        // header's dependency maps against the LCR its `seq_lcr_id` resolves to under
        // § 6.4.1, which is *association-dependent*: `ExternalHlsSet` cannot enumerate an
        // external LCR (it models only sequence-header ids and operating-point sets), but
        // a Provided declaration is a *partial* one — other external HLS OBUs, including
        // local LCRs, MAY exist unenumerated (see `ExternalHlsMode::Provided`). An external
        // *local* LCR with this `seq_lcr_id` would win the local-first § 6.4.1 resolution
        // ahead of the in-band record, so the association the validator paired may not be
        // the one a real decoder uses, and an in-band "violation" against it would be a
        // false positive (zero-false-positive principle). This is the identical
        // local-first-shadowing reasoning `check_seq_lcr_reference` uses to suppress
        // lcr/global-xlayer-map-missing-xlayer. The gate is "any Provided mode", not
        // "declares a sequence header": an external LCR can be shadowing even when the set
        // enumerates only OPS or nothing at all, so the suppression is not about sequence
        // headers.
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        // Strict frame-confirmed activation (no sole-in-band-header fallback): a check that
        // fires unconditionally on a violation must not emit against a guessed activation.
        let Some((seq_header_id, general)) = self.frame_confirmed_activation_for(xlayer) else {
            return;
        };
        let Some(association) = self.lcr_associations.get(&(xlayer, seq_header_id)) else {
            // seq_lcr_id == 0 or unresolved at the header's observation (§ 6.4.1:
            // no OBU is associated).
            return;
        };
        let lcr_is_global = association.lcr_is_global;
        let seq_lcr_id = association.lcr_id;
        let Some(maps) = association.maps.clone() else {
            return;
        };

        if let Some((curr, reference)) =
            mlayer_closure_violation(maps.mlayer_map, &general.mlayer_dependency_map)
        {
            let key = DependencyFindingKey::Lcr {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id: seq_lcr_id,
                lcr_offset: maps.offset,
                map: DependencyMapKind::Mlayer,
            };
            if self.emitted_dependency_findings.insert(key) {
                report.push(
                    Diagnostic::error(
                        "lcr/mlayer-dependency-missing",
                        format!(
                            "activated {} layer configuration record {seq_lcr_id} includes \
                             embedded layer {curr} but not embedded layer {reference} for \
                             extended layer {}, which the activated sequence header {}'s \
                             MLayerDependencyMap[{curr}][{reference}] requires",
                            if lcr_is_global { "global" } else { "local" },
                            xlayer.get(),
                            seq_header_id.get(),
                        ),
                    )
                    .with_spec_section("6.8.9")
                    .with_byte_offset(maps.offset),
                );
            }
        }

        for &(mlayer, tlayer_mask) in &maps.tlayer_maps {
            let Some((curr, reference)) =
                tlayer_closure_violation(mlayer, tlayer_mask, &general.tlayer_dependency_map)
            else {
                continue;
            };
            let key = DependencyFindingKey::Lcr {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id: seq_lcr_id,
                lcr_offset: maps.offset,
                map: DependencyMapKind::Tlayer { mlayer },
            };
            if self.emitted_dependency_findings.insert(key) {
                report.push(
                    Diagnostic::error(
                        "lcr/tlayer-dependency-missing",
                        format!(
                            "activated {} layer configuration record {seq_lcr_id} includes \
                             temporal layer {curr} of embedded layer {mlayer} but not temporal \
                             layer {reference} for extended layer {}, which the activated \
                             sequence header {}'s \
                             TLayerDependencyMap[{mlayer}][{curr}][{reference}] requires",
                            if lcr_is_global { "global" } else { "local" },
                            xlayer.get(),
                            seq_header_id.get(),
                        ),
                    )
                    .with_spec_section("6.8.9")
                    .with_byte_offset(maps.offset),
                );
            }
        }
    }

    /// AV2 § 6.8.5: when `lcr_seq_profile_tier_level_info(i)` is present in the LCR
    /// activated by extended layer `i`'s frame-confirmed sequence header, the header's
    /// `seq_profile_idc`, `seq_level_idx`, `seq_tier`, and `seq_max_mlayer_cnt_minus_1 +
    /// 1` must each be less than or equal to the corresponding LCR-declared maximum
    /// (`lcr_seq_profile_idc[i]` / `lcr_max_level_idx[i]` / `lcr_tier_flag[i]` /
    /// `lcr_max_mlayer_count[i]`), with equality passing
    /// (mirror `06-syntax-structures-semantics.md#s-6-8-5`, lines 1774-1810).
    ///
    /// The pairing is the sequence header activated for `xlayer` and that header's
    /// § 6.4.1 LCR association (the [`LcrAssociation::ptl`] snapshot taken at the
    /// header's latest observation, NOT a live resolution — a record redefined after the
    /// header is not the associated one). The § 6.8.5 sentence keys the ceiling on the
    /// *local* LCR; the snapshot reads the local record's PTL for a local association and
    /// the global record's PTL for that xlayer for a global one. An association without
    /// PTL info has nothing to check (absent PTL compares nothing), and unresolved
    /// references are owned by the existing § 7.3.8.3 availability diagnostics. The
    /// diagnostics anchor at the associated LCR OBU (its declared maxima are the
    /// informative source). Suppressed under any Provided external-HLS mode (the
    /// association is § 6.4.1-resolved and an unmodeled external local LCR could shadow
    /// the in-band record) and gated on a strict frame-confirmed activation — see
    /// [`Self::check_lcr_dependency_agreement`] for the full rationale.
    fn check_lcr_ptl_ceilings(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // Suppress under any Provided mode and gate on a strict frame-confirmed
        // activation; see check_lcr_dependency_agreement for the full rationale (the
        // § 6.4.1 association an unmodeled external local LCR could shadow, plus the
        // no-emit-against-a-guess requirement).
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        let Some((seq_header_id, general)) = self.frame_confirmed_activation_for(xlayer) else {
            return;
        };
        let Some(association) = self.lcr_associations.get(&(xlayer, seq_header_id)) else {
            return;
        };
        let Some(ptl) = association.ptl else {
            // § 6.8.5 "when lcr_seq_profile_tier_level_info(i) is present": absent PTL
            // info compares nothing.
            return;
        };
        let lcr_is_global = association.lcr_is_global;
        let lcr_id = association.lcr_id;
        let lcr_offset = ptl.offset;
        let scope = if lcr_is_global { "global" } else { "local" };

        // The activated header's compared PTL values.
        let seq_profile = u32::from(general.seq_profile_idc.get());
        let seq_level = u32::from(general.seq_level_idx.get());
        let seq_tier = u32::from(u8::from(matches!(general.seq_tier, Tier::High)));
        let seq_mlayer_count = u32::from(general.seq_max_mlayer_count.get());

        // Each ceiling: header value <= LCR-declared maximum (equality passes).
        let checks = [
            (
                LcrPtlField::Profile,
                "lcr/ptl-profile-exceeds-max",
                seq_profile,
                u32::from(ptl.seq_profile_idc),
                "seq_profile_idc",
                "lcr_seq_profile_idc",
            ),
            (
                LcrPtlField::Level,
                "lcr/ptl-level-exceeds-max",
                seq_level,
                u32::from(ptl.max_level_idx),
                "seq_level_idx",
                "lcr_max_level_idx",
            ),
            (
                LcrPtlField::Tier,
                "lcr/ptl-tier-exceeds-max",
                seq_tier,
                u32::from(ptl.tier_flag),
                "seq_tier",
                "lcr_tier_flag",
            ),
            (
                LcrPtlField::MlayerCount,
                "lcr/ptl-mlayer-count-exceeds-max",
                seq_mlayer_count,
                u32::from(ptl.max_mlayer_count),
                "seq_max_mlayer_cnt_minus_1 + 1",
                "lcr_max_mlayer_count",
            ),
        ];

        for (field, rule_id, header_value, lcr_max, header_name, lcr_name) in checks {
            if header_value <= lcr_max {
                continue;
            }
            let key = LcrPtlFindingKey {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id,
                lcr_offset,
                field,
                lcr_max,
                header_value,
            };
            if !self.emitted_lcr_ptl_findings.insert(key) {
                continue;
            }
            report.push(
                Diagnostic::error(
                    rule_id,
                    format!(
                        "sequence header {} activated for extended layer {} has {header_name} \
                         {header_value}, exceeding the activated {scope} layer configuration \
                         record {lcr_id}'s {lcr_name}[{}] = {lcr_max} (§ 6.8.5)",
                        seq_header_id.get(),
                        xlayer.get(),
                        xlayer.get(),
                    ),
                )
                .with_spec_section("6.8.5")
                .with_byte_offset(lcr_offset),
            );
        }
    }

    /// AV2 § 6.8.8: the activated LCR's `lcr_rep_info(isGlobal, j)`, when present, must
    /// agree with each sequence header activated by extended layer `j` — `lcr_max_pic_width`
    /// / `lcr_max_pic_height` equal `max_frame_width/height_minus_1 + 1`,
    /// `lcr_bit_depth_idc` / `lcr_chroma_format_idc` (when
    /// `lcr_format_info_present_flag == 1`) equal `bit_depth_idc` / `chroma_format_idc`,
    /// `lcr_cropping_window_present_flag` equals `seq_cropping_window_present_flag`, and
    /// (when the LCR cropping window is present) the four `lcr_cropping_win_*_offset`
    /// equal the `seq_cropping_win_*_offset` (mirror
    /// `06-syntax-structures-semantics.md#s-6-8-8`, lines 1925-1968). Each disagreement
    /// emits `lcr/rep-info-mismatch` (error) naming the field.
    ///
    /// Same pairing discipline as [`Self::check_lcr_ptl_ceilings`]: the [`LcrAssociation::rep_info`]
    /// snapshot, a strict frame-confirmed activation, absent rep-info (or absent
    /// format-info / cropping window) comparing nothing, and the LCR OBU as the diagnostic
    /// anchor. Likewise suppressed under any Provided external-HLS mode — see
    /// [`Self::check_lcr_dependency_agreement`].
    fn check_lcr_rep_info_agreement(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // Suppress under any Provided mode and gate on a strict frame-confirmed
        // activation; see check_lcr_dependency_agreement for the full rationale.
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        let Some((seq_header_id, general)) = self.frame_confirmed_activation_for(xlayer) else {
            return;
        };
        let Some(association) = self.lcr_associations.get(&(xlayer, seq_header_id)) else {
            return;
        };
        let Some(rep) = association.rep_info else {
            // Absent rep-info compares nothing.
            return;
        };
        let lcr_is_global = association.lcr_is_global;
        let lcr_id = association.lcr_id;
        let lcr_offset = rep.offset;
        let scope = if lcr_is_global { "global" } else { "local" };

        // Collect (field, lcr_value, header_value, message-fragment) for each
        // disagreeing comparison. lcr_value / header_value also feed the dedup key.
        let mut mismatches: Vec<(LcrRepInfoField, u64, u64, String)> = Vec::new();

        // § 6.8.8 lines 1925-1933: lcr_max_pic_width/height shall equal
        // max_frame_width/height_minus_1 + 1 (always present in the rep info).
        let header_width = general.max_frame_width.get();
        if rep.max_pic_width != header_width {
            mismatches.push((
                LcrRepInfoField::Width,
                u64::from(rep.max_pic_width),
                u64::from(header_width),
                format!(
                    "lcr_max_pic_width {} != max_frame_width_minus_1 + 1 = {header_width}",
                    rep.max_pic_width
                ),
            ));
        }
        let header_height = general.max_frame_height.get();
        if rep.max_pic_height != header_height {
            mismatches.push((
                LcrRepInfoField::Height,
                u64::from(rep.max_pic_height),
                u64::from(header_height),
                format!(
                    "lcr_max_pic_height {} != max_frame_height_minus_1 + 1 = {header_height}",
                    rep.max_pic_height
                ),
            ));
        }

        // § 6.8.8 lines 1950-1958: lcr_bit_depth_idc / lcr_chroma_format_idc shall equal
        // bit_depth_idc / chroma_format_idc — present only when
        // lcr_format_info_present_flag == 1 (absent compares nothing).
        if let Some((lcr_bit_depth, lcr_chroma)) = rep.format {
            let header_bit_depth = u32::from(general.bit_depth_idc.get());
            if lcr_bit_depth != header_bit_depth {
                mismatches.push((
                    LcrRepInfoField::BitDepth,
                    u64::from(lcr_bit_depth),
                    u64::from(header_bit_depth),
                    format!(
                        "lcr_bit_depth_idc {lcr_bit_depth} != bit_depth_idc {header_bit_depth}"
                    ),
                ));
            }
            let header_chroma = u32::from(general.chroma_format_idc.get());
            if lcr_chroma != header_chroma {
                mismatches.push((
                    LcrRepInfoField::ChromaFormat,
                    u64::from(lcr_chroma),
                    u64::from(header_chroma),
                    format!(
                        "lcr_chroma_format_idc {lcr_chroma} != chroma_format_idc {header_chroma}"
                    ),
                ));
            }
        }

        // § 6.8.8 lines 1943-1968: lcr_cropping_window_present_flag shall equal
        // seq_cropping_window_present_flag; the offsets shall match the seq_cropping_*
        // offsets (the LCR offsets are present only when the LCR cropping window is).
        let lcr_cropping_present = rep.cropping.is_some();
        let header_cropping_present = general.seq_cropping_window_present_flag;
        if lcr_cropping_present != header_cropping_present {
            mismatches.push((
                LcrRepInfoField::CroppingPresent,
                u64::from(lcr_cropping_present),
                u64::from(header_cropping_present),
                format!(
                    "lcr_cropping_window_present_flag {} != seq_cropping_window_present_flag {}",
                    u8::from(lcr_cropping_present),
                    u8::from(header_cropping_present),
                ),
            ));
        }
        if let Some((lcr_left, lcr_right, lcr_top, lcr_bottom)) = rep.cropping {
            // The header's seq_cropping_win_* offsets are 0 when the window is absent
            // (§ 6.4.1 inference). The present-flag mismatch above already fires in that
            // case; the offset comparisons still run against the header's effective
            // (possibly inferred-0) values per the § 6.8.8 "shall match" sentence.
            let crop = general.cropping_window;
            for (field, lcr_value, header_value, name) in [
                (LcrRepInfoField::CropLeft, lcr_left, crop.left, "left"),
                (LcrRepInfoField::CropRight, lcr_right, crop.right, "right"),
                (LcrRepInfoField::CropTop, lcr_top, crop.top, "top"),
                (
                    LcrRepInfoField::CropBottom,
                    lcr_bottom,
                    crop.bottom,
                    "bottom",
                ),
            ] {
                if lcr_value != header_value {
                    mismatches.push((
                        field,
                        u64::from(lcr_value),
                        u64::from(header_value),
                        format!(
                            "lcr_cropping_win_{name}_offset {lcr_value} != \
                             seq_cropping_win_{name}_offset {header_value}"
                        ),
                    ));
                }
            }
        }

        for (field, lcr_value, header_value, fragment) in mismatches {
            let key = LcrRepInfoFindingKey {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id,
                lcr_offset,
                field,
                lcr_value,
                header_value,
            };
            if !self.emitted_lcr_rep_info_findings.insert(key) {
                continue;
            }
            report.push(
                Diagnostic::error(
                    "lcr/rep-info-mismatch",
                    format!(
                        "activated {scope} layer configuration record {lcr_id}'s rep info for \
                         extended layer {} disagrees with sequence header {} activated for that \
                         layer: {fragment} (§ 6.8.8)",
                        xlayer.get(),
                        seq_header_id.get(),
                    ),
                )
                .with_spec_section("6.8.8")
                .with_byte_offset(lcr_offset),
            );
        }
    }

    /// Observes a quantizer matrix OBU (§ 5.13), running the locally-checkable § 6.12
    /// duplicate-reset / duplicate-level diagnostics and recording per-level
    /// availability. A parse failure or malformed payload tail is silent (the OBU is
    /// not-yet-validated coverage in that case), consistent with the OPS observer.
    fn observe_quantizer_matrix(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(qm) = parse_quantizer_matrix(&mut reader) else {
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
        // Emit diagnostics against the window/availability captured before this OBU,
        // then fold this OBU into the state.
        self.emit_quantizer_matrix_diagnostics(obu, &qm, report);
        self.check_quantizer_matrix(obu, &qm);
    }

    /// Emits the § 6.12 quantizer-matrix diagnostics for `qm`, reading the window and
    /// per-level availability captured before this OBU.
    fn emit_quantizer_matrix_diagnostics(
        &self,
        obu: &ObuEnvelope<'_>,
        qm: &QuantizerMatrixObu,
        report: &mut ValidationReport,
    ) {
        if qm.qm_bit_map == 0 {
            // AV2 § 6.12: only the first quantizer matrix OBU between coded frames may
            // have qm_bit_map == 0.
            if self.qm.qm_obu_seen_since_coded_frame {
                report.push(
                    Diagnostic::error(
                        "qm/duplicate-reset-between-frames",
                        "a quantizer matrix OBU with qm_bit_map == 0 is only permitted as the \
                         first quantizer matrix OBU between coded frames",
                    )
                    .with_spec_section("6.12")
                    .with_byte_offset(obu.offset),
                );
            }
            return;
        }

        // AV2 § 6.12: the same quantizer matrix level must not be specified twice
        // between coded frames.
        let overlap = self.qm.seen_levels_since_coded_frame & qm.qm_bit_map;
        for level in 0..NUM_CUSTOM_QMS {
            if overlap & (1 << level) == 0 {
                continue;
            }
            let prior = match self.qm.available[level] {
                Some(record) => format!(
                    " (previously specified by a quantizer matrix OBU at embedded layer {}, \
                     temporal layer {}, data_present={}, num_planes={})",
                    // QmMLayerId / QmTLayerId are -1 in the spec for a reset.
                    record
                        .mlayer_id
                        .map_or_else(|| "-1".to_owned(), |m| m.to_string()),
                    record
                        .tlayer_id
                        .map_or_else(|| "-1".to_owned(), |t| t.to_string()),
                    record.data_present,
                    record.num_planes,
                ),
                None => String::new(),
            };
            report.push(
                Diagnostic::error(
                    "qm/duplicate-level-between-frames",
                    format!(
                        "quantizer matrix level {level} is specified twice between coded \
                         frames{prior}"
                    ),
                )
                .with_spec_section("6.12")
                .with_byte_offset(obu.offset),
            );
        }
    }

    /// Updates the §6.12 window and per-level availability after the diagnostics.
    fn check_quantizer_matrix(&mut self, obu: &ObuEnvelope<'_>, qm: &QuantizerMatrixObu) {
        self.qm.qm_obu_seen_since_coded_frame = true;
        if qm.qm_bit_map == 0 {
            // AV2 § 5.13 reset path: every custom level returns to its defaults
            // (QmDataPresent = 0, QmMLayerId = QmTLayerId = -1, QmNumPlanes = numPlanes),
            // so a frame-reference check after a reset must not see stale layer/data
            // state from a previously defined matrix.
            for record in &mut self.qm.available {
                *record = Some(QmLevelRecord {
                    mlayer_id: None,
                    tlayer_id: None,
                    data_present: false,
                    num_planes: qm.num_planes,
                });
            }
            return;
        }
        self.qm.seen_levels_since_coded_frame |= qm.qm_bit_map;
        for level in &qm.levels {
            let index = level.level as usize;
            if index < NUM_CUSTOM_QMS {
                self.qm.available[index] = Some(QmLevelRecord {
                    mlayer_id: Some(obu.header.embedded_layer_id.get()),
                    tlayer_id: Some(obu.header.temporal_layer_id.get()),
                    data_present: !level.is_default,
                    num_planes: qm.num_planes,
                });
            }
        }
    }

    /// Observes a film grain OBU (§ 5.14), running the locally-checkable § 6.13
    /// diagnostics (zero update flags, out-of-range chroma idc, duplicate slot in the
    /// coded frame unit) and recording per-slot availability. A parse failure or
    /// malformed payload tail is silent, consistent with the OPS observer.
    fn observe_film_grain(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(fg) = parse_film_grain(&mut reader) else {
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
        self.emit_film_grain_diagnostics(obu, &fg, report);
        self.emit_film_grain_model_diagnostics(obu, &fg, report);
        self.record_film_grain(obu, &fg);
    }

    /// Emits the locally-decidable § 6.17.10.2 film-grain *model* conformance
    /// diagnostics for each updated slot: scaling-point counts (`num_*_points <= 14`),
    /// strictly-increasing-and-`< 256` scaling-point values, and the 4:2:0 chroma
    /// pairing rule (when `subX == 1 && subY == 1`, `num_cb_points` and `num_cr_points`
    /// must be both zero or both non-zero).
    fn emit_film_grain_model_diagnostics(
        &self,
        obu: &ObuEnvelope<'_>,
        fg: &FilmGrainObu,
        report: &mut ValidationReport,
    ) {
        for update in &fg.models {
            let model = &update.model;
            let slot = update.slot;
            for (channel, count) in [
                ("y", model.num_y_points),
                ("cb", model.num_cb_points),
                ("cr", model.num_cr_points),
            ] {
                if count > MAX_FILM_GRAIN_SCALING_POINTS {
                    report.push(
                        Diagnostic::error(
                            "film-grain/scaling-points-out-of-range",
                            format!(
                                "film grain slot {slot} num_{channel}_points {count} must be less \
                                 than or equal to {MAX_FILM_GRAIN_SCALING_POINTS}"
                            ),
                        )
                        .with_spec_section("6.17.10.2")
                        .with_byte_offset(obu.offset),
                    );
                }
            }

            for (channel, points) in [
                ("y", &model.point_y),
                ("cb", &model.point_cb),
                ("cr", &model.point_cr),
            ] {
                emit_scaling_point_order_diagnostics(channel, points, slot, obu, report);
            }

            // AV2 § 6.17.10.2: in 4:2:0 (subX == 1 && subY == 1), film grain applies to
            // both chroma components or neither.
            if fg.sub_x && fg.sub_y && (model.num_cb_points == 0) != (model.num_cr_points == 0) {
                report.push(
                    Diagnostic::error(
                        "film-grain/chroma-points-not-paired",
                        format!(
                            "film grain slot {slot}: in 4:2:0, num_cb_points ({}) and \
                             num_cr_points ({}) must both be zero or both non-zero",
                            model.num_cb_points, model.num_cr_points
                        ),
                    )
                    .with_spec_section("6.17.10.2")
                    .with_byte_offset(obu.offset),
                );
            }
        }
    }

    /// Emits the § 6.13 film-grain diagnostics for `fg`, reading the coded-frame-unit
    /// window and per-slot availability captured before this OBU.
    fn emit_film_grain_diagnostics(
        &self,
        obu: &ObuEnvelope<'_>,
        fg: &FilmGrainObu,
        report: &mut ValidationReport,
    ) {
        // AV2 § 6.13: fgm_update_flags is not equal to 0.
        if fg.update_flags == 0 {
            report.push(
                Diagnostic::error(
                    "film-grain/update-flags-zero",
                    "fgm_update_flags must not be 0",
                )
                .with_spec_section("6.13")
                .with_byte_offset(obu.offset),
            );
        }

        // AV2 § 6.13: fgm_chroma_idc is less than or equal to 3.
        if fg.chroma_idc > 3 {
            report.push(
                Diagnostic::error(
                    "film-grain/chroma-idc-out-of-range",
                    format!(
                        "fgm_chroma_idc {} must be less than or equal to 3",
                        fg.chroma_idc
                    ),
                )
                .with_spec_section("6.13")
                .with_byte_offset(obu.offset),
            );
        }

        // AV2 § 6.13: bit i of fgm_update_flags is set in at most one film grain OBU
        // per coded frame unit.
        let overlap = self.film_grain.updated_slots_since_coded_frame & fg.update_flags;
        for slot in 0..MAX_FILM_GRAIN {
            if overlap & (1 << slot) == 0 {
                continue;
            }
            let prior = match self.film_grain.available[slot] {
                Some(record) => format!(
                    " (previously updated by a film grain OBU at embedded layer {}, temporal \
                     layer {}, fgm_chroma_idc {})",
                    record.mlayer_id, record.tlayer_id, record.chroma_idc,
                ),
                None => String::new(),
            };
            report.push(
                Diagnostic::error(
                    "film-grain/duplicate-slot-in-coded-frame-unit",
                    format!(
                        "film grain slot {slot} is updated more than once in the same coded \
                         frame unit{prior}"
                    ),
                )
                .with_spec_section("6.13")
                .with_byte_offset(obu.offset),
            );
        }
    }

    /// Updates the §6.13 coded-frame-unit window and per-slot availability.
    fn record_film_grain(&mut self, obu: &ObuEnvelope<'_>, fg: &FilmGrainObu) {
        self.film_grain.updated_slots_since_coded_frame |= fg.update_flags;
        for update in &fg.models {
            let index = update.slot as usize;
            if index < MAX_FILM_GRAIN {
                self.film_grain.available[index] = Some(FgmSlotRecord {
                    chroma_idc: fg.chroma_idc,
                    mlayer_id: obu.header.embedded_layer_id.get(),
                    tlayer_id: obu.header.temporal_layer_id.get(),
                });
            }
        }
    }

    /// Observes a buffer removal timing OBU. For the OPS-dependent form (§ 5.12,
    /// § 6.11), resolves `(obu_xlayer_id, br_ops_id)` against the active OPS state: an
    /// unavailable OPS under external-HLS-disabled mode is `brt/unavailable-operating-
    /// point-set`, and a `br_ops_cnt` differing from the active `ops_cnt` is
    /// `brt/ops-count-mismatch`. The extended-layer form has nothing to resolve here.
    fn observe_buffer_removal_timing(
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

        match self.ops.get(xlayer, br_ops_id) {
            Some(record) => {
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
            }
            None => {
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

    /// Resolves a sequence header's `seq_lcr_id` reference (AV2 § 6.4.1 / § 7.3.8.3 /
    /// § 7.3.8.6): when non-zero it must resolve to an available local LCR (same
    /// xlayer, `lcr_local_id == seq_lcr_id`) or, failing that, an available global LCR
    /// (`lcr_global_config_record_id == seq_lcr_id`) whose `lcr_xlayer_map` includes
    /// this header's xlayer. Availability diagnostics are gated on external HLS being
    /// disabled (an externally-provided LCR is not modeled).
    fn check_seq_lcr_reference(
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

    /// Buffers a sequence header's `seq_lcr_id` § 7.3.8.3 reference for the random-access-
    /// point availability replay, but only when it resolved to an in-band LCR (so the
    /// linear § 7.3.8.3 availability check did not fire — keeping the replay predicate
    /// disjoint). Mirrors [`Self::check_seq_lcr_reference`]'s § 6.4.1 resolution order
    /// (local LCR in this extended layer first, then global LCR). The reference is governed
    /// by the sequence header's own extended layer.
    fn note_seq_lcr_rap_reference(&mut self, obu: &ObuEnvelope<'_>, seq_lcr_id: u8) {
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

    fn observe_sequence_header(
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

    fn validate_active_sequence_limits(
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

        if obu.header.temporal_layer_id > sequence_header.general.max_tlayer_id {
            report.push(sequence_state_error(
                "sequence-state/tlayer-exceeds-max",
                "6.2.2",
                obu,
                Some(BitOffset::from_bits(6)),
                format!(
                    "obu_tlayer_id {} exceeds active sequence max_tlayer_id {}",
                    obu.header.temporal_layer_id.get(),
                    sequence_header.general.max_tlayer_id.get()
                ),
            ));
        }

        if obu.header.embedded_layer_id > sequence_header.general.max_mlayer_id {
            let byte_offset = obu.offset.saturating_add(1);
            report.push(
                Diagnostic::error(
                    "sequence-state/mlayer-exceeds-max",
                    format!(
                        "obu_mlayer_id {} exceeds active sequence max_mlayer_id {}",
                        obu.header.embedded_layer_id.get(),
                        sequence_header.general.max_mlayer_id.get()
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
    fn count_distinct_mlayer(
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
    fn retroactive_distinct_mlayer_check(
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
    fn emit_distinct_mlayer_exceedance(
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

/// Compares two present `timing_info()` values from different embedded layers of
/// the same coded video sequence, returning a diagnostic per differing field
/// (AV2 § 6.4.12: these values, when present, shall be the same across all embedded
/// layers). `new` is located at `obu` (embedded layer `obu.header.embedded_layer_id`);
/// `existing` is the value previously seen for `existing_mlayer`. Both embedded-layer
/// ids are named in each message so the finding is self-contained. The caller routes
/// each diagnostic through [`CvsTracker::defer_or_emit`], since the comparison is
/// scoped to the coded video sequence (AV2 § 7.3.6).
fn compare_timing_across_embedded_layers(
    existing_mlayer: EmbeddedLayerId,
    existing: &TimingInfo,
    new: &TimingInfo,
    obu: &ObuEnvelope<'_>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let new_mlayer = obu.header.embedded_layer_id.get();
    let existing_mlayer = existing_mlayer.get();
    if existing.num_units_in_display_tick != new.num_units_in_display_tick {
        diagnostics.push(timing_mismatch_error(
            "sequence-header/timing-display-tick-mismatch",
            obu,
            format!(
                "num_units_in_display_tick {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) \
                 in the same coded video sequence",
                new.num_units_in_display_tick,
                new_mlayer,
                existing.num_units_in_display_tick,
                existing_mlayer
            ),
        ));
    }
    if existing.time_scale != new.time_scale {
        diagnostics.push(timing_mismatch_error(
            "sequence-header/timing-time-scale-mismatch",
            obu,
            format!(
                "time_scale {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) in the same \
                 coded video sequence",
                new.time_scale, new_mlayer, existing.time_scale, existing_mlayer
            ),
        ));
    }
    if existing.equal_picture_interval != new.equal_picture_interval {
        diagnostics.push(timing_mismatch_error(
            "sequence-header/timing-equal-picture-interval-mismatch",
            obu,
            format!(
                "equal_picture_interval {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) in \
                 the same coded video sequence",
                new.equal_picture_interval, new_mlayer, existing.equal_picture_interval, existing_mlayer
            ),
        ));
    }
    // num_ticks_per_picture_minus_1 is only present when equal_picture_interval is
    // set; compare it only when both layers carry it (AV2 § 6.4.12).
    if let (Some(existing_ticks), Some(new_ticks)) = (
        existing.num_ticks_per_picture_minus_1,
        new.num_ticks_per_picture_minus_1,
    ) && existing_ticks != new_ticks
    {
        diagnostics.push(timing_mismatch_error(
            "sequence-header/timing-num-ticks-mismatch",
            obu,
            format!(
                "num_ticks_per_picture_minus_1 {new_ticks} (obu_mlayer_id {new_mlayer}) differs \
                 from {existing_ticks} (obu_mlayer_id {existing_mlayer}) in the same coded video \
                 sequence"
            ),
        ));
    }
    diagnostics
}

/// Returns `true` if two content-interpretation OBUs carry different *information*
/// (AV2 § 6.14: a repeated CI OBU must "contain the same information").
///
/// `ci_reserved_2bit` is excluded — it is decoder-ignored (§ 6.14) and surfaced
/// separately as a warning. The color description and aspect ratio are compared by
/// their *derived* values (§ 6.14 Table 6.13 / the § 5.15 aspect tables), resolving
/// presets, reserved ids, and absence to their canonical (incl. unspecified)
/// values: an alias-equivalent re-encoding (a preset vs. the equivalent explicit
/// triple / SAR, or a reserved id vs. an explicit unspecified one) is not flagged,
/// while genuinely different color/aspect information is — including a present value
/// vs. an absent (unspecified) one. The aspect ratio is compared only when both
/// derived SARs are decidable; a reserved `ci_aspect_ratio_idc` (already an
/// out-of-range error) yields no derived SAR and is not double-reported here.
fn content_interpretation_information_differs(
    a: &ContentInterpretation,
    b: &ContentInterpretation,
) -> bool {
    a.scan_type_idc != b.scan_type_idc
        || a.chroma_sample_position != b.chroma_sample_position
        || a.timing_info != b.timing_info
        || a.derived_color() != b.derived_color()
        || aspect_ratio_information_differs(a, b)
}

/// Compares the derived sample aspect ratios (§ 5.15), resolving absence to the
/// unspecified `(0, 0)`. Only flags when both SARs are decidable; a reserved
/// `ci_aspect_ratio_idc` yields no derived SAR (it is already an out-of-range error).
fn aspect_ratio_information_differs(a: &ContentInterpretation, b: &ContentInterpretation) -> bool {
    match (
        a.derived_sample_aspect_ratio(),
        b.derived_sample_aspect_ratio(),
    ) {
        (Some(sar_a), Some(sar_b)) => sar_a != sar_b,
        _ => false,
    }
}

/// Builds a § 6.4.12 cross-embedded-layer timing-mismatch diagnostic located at `obu`.
fn timing_mismatch_error(
    rule_id: &'static str,
    obu: &ObuEnvelope<'_>,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("6.4.12")
        .with_byte_offset(obu.offset)
}

/// Computes a stable 64-bit FNV-1a fingerprint over an OBU payload's bytes.
///
/// Used to compare OBU payloads for bit identity without pulling in a hashing
/// dependency: repeated activated sequence headers (AV2 § 7.3.6) and the non-RAP
/// MSDO identity rule (§ 7.3.8.2).
fn payload_fingerprint(payload: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in payload {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[derive(Debug, Default)]
struct TemporalUnitState {
    phase: TemporalUnitPhase,
    current_coded_xlayer: Option<ExtendedLayerId>,
    reported_missing_delimiter: bool,
    /// `true` once any non-reserved, non-delimiter OBU has appeared since the most
    /// recent global temporal delimiter. Used to detect back-to-back delimiters.
    saw_obu_since_delimiter: bool,
    /// `true` once a *global suffix* metadata OBU (`metadata_is_suffix == 1`) has
    /// appeared in this temporal unit. A global suffix metadata is part of a coded
    /// frame unit's suffix tail (§ 7.3.3 / § 7.3.4), which lies inside / after the
    /// coded extended layer units, so a later global HLS prefix OBU is out of
    /// order (§ 7.3.7): `obu-order/global-hls-after-metadata-suffix`.
    saw_global_suffix_metadata: bool,
    /// The set of extended layers whose coded *frame* OBUs (frame-bearing or
    /// pre-frame content) have begun in the current coded extended layer unit. § 7.3.6
    /// orders the coded extended layer unit as LCR → OPS → atlas → sequence header
    /// → frame units, so a non-global HLS *header* OBU (LCR / OPS / atlas /
    /// sequence header) for an extended layer whose frame region has already begun
    /// is out of order: `obu-order/non-global-hls-before-coded-layer`. Tracking a
    /// *set* (not the last layer alone) catches a reordered header for an earlier
    /// extended layer after a later layer's frame region has begun.
    coded_frame_started_xlayer: BTreeSet<ExtendedLayerId>,
}

impl TemporalUnitState {
    fn observe_obu(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type.is_reserved() {
            return;
        }

        if obu.header.obu_type == ObuType::TemporalDelimiter {
            if obu.header.extended_layer_id.is_global() {
                if !matches!(self.phase, TemporalUnitPhase::AwaitingDelimiter)
                    && !self.saw_obu_since_delimiter
                {
                    report.push(ordering_error(
                        "obu-order/duplicate-temporal-delimiter",
                        obu,
                        "a temporal unit must start with exactly one global \
                         OBU_TEMPORAL_DELIMITER; found a second delimiter with no \
                         intervening OBU"
                            .to_owned(),
                    ));
                }
                self.start_temporal_unit();
            } else if matches!(self.phase, TemporalUnitPhase::AwaitingDelimiter) {
                self.report_missing_delimiter_once(obu, report);
            }
            return;
        }

        if matches!(self.phase, TemporalUnitPhase::AwaitingDelimiter) {
            self.report_missing_delimiter_once(obu, report);
        }
        self.saw_obu_since_delimiter = true;

        if is_padding_obu(obu) {
            self.observe_padding(obu, report);
        } else if is_metadata_obu(obu) {
            self.observe_metadata(obu, report);
        } else if is_global_hls_prefix_obu(obu) {
            self.observe_global_hls_prefix(obu, report);
        } else if is_coded_extended_layer_obu(obu) {
            self.observe_coded_extended_layer_obu(obu, report);
        }
    }

    /// Classifies a metadata OBU for temporal-unit ordering from its `metadata_is_suffix`
    /// bit (AV2 § 6.16.3 / § 7.3.7).
    ///
    /// A global *prefix* metadata OBU (`metadata_is_suffix == 0`) is a global temporal-
    /// unit prefix OBU, so it is flagged if it follows a coded extended layer unit. A
    /// global *suffix* metadata OBU (`metadata_is_suffix == 1`) is not a prefix and is
    /// left unclassified (its precise § 7.3.3 / § 7.3.4 placement inside coded frame
    /// units needs frame/tile parsing, which is deferred). Non-global metadata is a coded
    /// extended layer OBU. A metadata OBU whose first payload bit cannot be read is left
    /// unclassified; the structural parse error is reported by the metadata syntax check.
    fn observe_metadata(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.extended_layer_id.is_global() {
            // A global *prefix* (metadata_is_suffix == 0) is a § 7.3.7 global prefix OBU.
            // A global *suffix* (metadata_is_suffix == 1) is part of a coded frame unit's
            // suffix tail; record it so a later global HLS prefix OBU is flagged. An
            // unreadable first bit is left unclassified (the metadata syntax check reports
            // the structural error).
            match metadata_is_suffix(obu) {
                Some(false) => self.observe_global_hls_prefix(obu, report),
                Some(true) => self.saw_global_suffix_metadata = true,
                None => {}
            }
        } else {
            // Non-global metadata sits inside a coded frame unit (§ 7.3.3 / § 7.3.4) of
            // its extended layer's coded extended layer unit, i.e. in the frame region.
            self.observe_coded_extended_layer_obu(obu, report);
        }
    }

    fn start_temporal_unit(&mut self) {
        self.phase = TemporalUnitPhase::GlobalPrefix;
        self.current_coded_xlayer = None;
        self.reported_missing_delimiter = false;
        self.saw_obu_since_delimiter = false;
        self.saw_global_suffix_metadata = false;
        self.coded_frame_started_xlayer.clear();
    }

    fn report_missing_delimiter_once(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        if self.reported_missing_delimiter {
            return;
        }
        self.reported_missing_delimiter = true;
        report.push(ordering_error(
            "obu-order/temporal-unit-missing-delimiter",
            obu,
            format!(
                "{} appears before a global OBU_TEMPORAL_DELIMITER starts the temporal unit",
                obu.header.obu_type.spec_name()
            ),
        ));
    }

    fn observe_padding(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.extended_layer_id.is_global() {
            return;
        }

        let inside_current_coded_layer = matches!(self.phase, TemporalUnitPhase::CodedLayers)
            && self.current_coded_xlayer == Some(obu.header.extended_layer_id);
        if !inside_current_coded_layer {
            report.push(ordering_error(
                "obu-order/padding-non-global-outside-coded-layer",
                obu,
                format!(
                    "OBU_PADDING outside a coded extended layer unit must use \
                     obu_xlayer_id == GLOBAL_XLAYER_ID, found {}",
                    obu.header.extended_layer_id.get()
                ),
            ));
        }
    }

    fn observe_global_hls_prefix(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        // A global suffix metadata is more specific evidence that the prefix region
        // is over than merely being in the coded-layers phase, so prefer the
        // metadata-suffix rule when it applies (§ 7.3.7).
        if self.saw_global_suffix_metadata {
            report.push(ordering_error(
                "obu-order/global-hls-after-metadata-suffix",
                obu,
                format!(
                    "{} with GLOBAL_XLAYER_ID appears after a global suffix metadata OBU \
                     (metadata_is_suffix == 1); the global HLS prefix region must precede the \
                     coded extended layer units and their suffix metadata",
                    obu.header.obu_type.spec_name()
                ),
            ));
            return;
        }
        if matches!(self.phase, TemporalUnitPhase::CodedLayers) {
            report.push(ordering_error(
                "obu-order/global-hls-after-coded-layer",
                obu,
                format!(
                    "{} with GLOBAL_XLAYER_ID appears after a coded extended layer unit",
                    obu.header.obu_type.spec_name()
                ),
            ));
        }
    }

    fn observe_coded_extended_layer_obu(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let xlayer = obu.header.extended_layer_id;
        match self.current_coded_xlayer {
            Some(current) if xlayer < current => {
                report.push(ordering_error(
                    "obu-order/xlayer-order-not-ascending",
                    obu,
                    format!(
                        "coded extended layer units must appear in ascending obu_xlayer_id order \
                         within a temporal unit (found {} after {})",
                        xlayer.get(),
                        current.get()
                    ),
                ));
            }
            Some(current) if xlayer == current => {}
            _ => {
                self.current_coded_xlayer = Some(xlayer);
            }
        }
        self.phase = TemporalUnitPhase::CodedLayers;

        // AV2 § 7.3.6: a coded extended layer unit is ordered LCR → OPS → atlas →
        // sequence header → frame units. A non-global HLS *header* OBU (LCR / OPS /
        // atlas / sequence header) appearing after the frame region of this same
        // extended layer has begun is out of order. The frame region begins at the
        // first non-HLS-header coded-extended-layer OBU (a content-interpretation,
        // multi-frame-header, pre-frame BRT/QM/FGM/metadata, or frame-bearing OBU).
        let is_hls_header = matches!(
            obu.header.obu_type,
            ObuType::LayerConfigurationRecord
                | ObuType::OperatingPointSet
                | ObuType::AtlasSegment
                | ObuType::SequenceHeader
        );
        if is_hls_header {
            if self.coded_frame_started_xlayer.contains(&xlayer) {
                // This rule belongs to § 7.3.6 (coded extended layer unit ordering: LCR
                // → OPS → atlas → sequence header → frame units), not the § 7.3.7
                // temporal-unit ordering that the shared `ordering_error` helper assumes;
                // override the section so the emitted spec_section matches the registry
                // entry (VALIDATOR-DIAGNOSTICS.md documents this diagnostic as § 7.3.6).
                report.push(
                    ordering_error(
                        "obu-order/non-global-hls-before-coded-layer",
                        obu,
                        format!(
                            "{} for obu_xlayer_id {} appears after the coded frame region of its \
                             coded extended layer unit has begun; the HLS header OBUs (LCR / OPS / \
                             atlas / sequence header) must precede the coded frame units",
                            obu.header.obu_type.spec_name(),
                            xlayer.get()
                        ),
                    )
                    .with_spec_section("7.3.6"),
                );
            }
        } else {
            // A frame-region OBU for this extended layer: record that the frame
            // region has begun, so a later HLS header for *that* layer fires — tracked
            // per extended layer so a reordered header for an earlier layer after a
            // later layer's frame region is not masked by the last-layer-only scalar.
            self.coded_frame_started_xlayer.insert(xlayer);
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum TemporalUnitPhase {
    #[default]
    AwaitingDelimiter,
    GlobalPrefix,
    CodedLayers,
}

fn sequence_header_can_activate(obu: &ObuEnvelope<'_>) -> bool {
    !obu.header.extended_layer_id.is_global()
        && obu.header.temporal_layer_id.get() == 0
        && obu.header.embedded_layer_id.get() == 0
}

fn requires_active_sequence(obu: &ObuEnvelope<'_>) -> bool {
    !obu.header.extended_layer_id.is_global()
        && !matches!(
            obu.header.obu_type,
            ObuType::Reserved0
                | ObuType::Reserved(_)
                | ObuType::SequenceHeader
                | ObuType::TemporalDelimiter
                | ObuType::LayerConfigurationRecord
                | ObuType::OperatingPointSet
                | ObuType::AtlasSegment
        )
}

fn is_padding_obu(obu: &ObuEnvelope<'_>) -> bool {
    obu.header.obu_type == ObuType::Padding
}

fn is_metadata_obu(obu: &ObuEnvelope<'_>) -> bool {
    matches!(
        obu.header.obu_type,
        ObuType::MetadataShort | ObuType::MetadataGroup
    )
}

/// Reads `metadata_is_suffix` (the first payload bit of both metadata OBU types,
/// AV2 § 5.17.2 / § 5.17.3), returning `None` if the payload is empty.
fn metadata_is_suffix(obu: &ObuEnvelope<'_>) -> Option<bool> {
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    reader.read_bit().ok().map(|bit| bit != 0)
}

fn is_global_hls_prefix_obu(obu: &ObuEnvelope<'_>) -> bool {
    // AV2 § 7.3.7 lists the global temporal-unit prefix OBUs exhaustively: MSDO,
    // global LCR, global OPS, global atlas segment, and global *prefix* metadata. The two
    // metadata OBU types are NOT matched here: global metadata is classified by its
    // parsed metadata_is_suffix bit in observe_metadata (a suffix is not a prefix), so it
    // must not be matched as an unconditional global prefix.
    //
    // OBU_BUFFER_REMOVAL_TIMING is deliberately NOT in this set: § 7.3.7 does not list
    // it as a global prefix OBU, and § 7.3.3 / § 7.3.4 place a BRT inside a coded
    // frame unit at the frame's own xlayer (see is_coded_extended_layer_obu). A global
    // BRT therefore has no § 7.3.7 prefix position to enforce here, so it is left
    // unclassified rather than flagged — a sound-over-complete choice that avoids
    // false positives.
    //
    // TODO(spec: AV2-7.3-OBU-ORDERING): a hard `brt/global-ordering-position`
    // diagnostic for a global BRT would need the § 7.3.8 decoder-model / random-access
    // state that is not yet modeled.
    obu.header.extended_layer_id.is_global()
        && matches!(
            obu.header.obu_type,
            ObuType::Msdo
                | ObuType::LayerConfigurationRecord
                | ObuType::OperatingPointSet
                | ObuType::AtlasSegment
        )
}

fn is_coded_extended_layer_obu(obu: &ObuEnvelope<'_>) -> bool {
    // A local (non-global) OBU_BUFFER_REMOVAL_TIMING falls here: AV2 § 7.3.3 / § 7.3.4
    // place it inside a coded output / non-output frame unit at the frame's xlayer, so
    // it follows the same coded-extended-layer classification as the frame OBUs.
    !obu.header.extended_layer_id.is_global()
        && !matches!(
            obu.header.obu_type,
            ObuType::TemporalDelimiter
                | ObuType::Padding
                | ObuType::Reserved0
                | ObuType::Reserved(_)
        )
}

fn sequence_state_error(
    rule_id: &'static str,
    spec_section: &'static str,
    obu: &ObuEnvelope<'_>,
    bit_offset: Option<BitOffset>,
    message: String,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(rule_id, message)
        .with_spec_section(spec_section)
        .with_byte_offset(obu.offset);
    if let Some(bit_offset) = bit_offset {
        diagnostic = diagnostic.with_bit_offset(bit_offset);
    }
    diagnostic
}

fn ordering_error(rule_id: &'static str, obu: &ObuEnvelope<'_>, message: String) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("7.3.7")
        .with_byte_offset(obu.offset)
}

/// A fingerprint of the sequence-header fields the Annex A value-space checks inspect
/// (`seq_profile_idc`, `chroma_format_idc`, `bit_depth_idc`, `seq_tier`, `seq_level_idx`).
/// Part of the [`ValidatorContext::emitted_annex_a_value_space`] dedup key so a § 7.3.6
/// same-`seq_header_id` redefinition with different checked content re-runs the checks
/// instead of being suppressed by the original activation's entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AnnexAValueSpaceFingerprint {
    profile_idc: u8,
    chroma_format_idc: u8,
    bit_depth_idc: u8,
    tier: u8,
    level_idx: u8,
}

/// Projects the Annex A value-space dedup fingerprint out of an activated sequence
/// header's general fields (see [`AnnexAValueSpaceFingerprint`]).
fn annex_a_value_space_fingerprint(general: &SequenceHeaderGeneral) -> AnnexAValueSpaceFingerprint {
    AnnexAValueSpaceFingerprint {
        profile_idc: general.seq_profile_idc.get(),
        chroma_format_idc: general.chroma_format_idc.get(),
        bit_depth_idc: general.bit_depth_idc.get(),
        tier: u8::from(matches!(general.seq_tier, Tier::High)),
        level_idx: general.seq_level_idx.get(),
    }
}

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
struct LcrAgreementValueFingerprint {
    /// `SeqMaxMlayerCnt` (`seq_max_mlayer_cnt_minus_1 + 1`), the § 6.8.5 mlayer-count
    /// ceiling operand.
    max_mlayer_count: u8,
    /// `max_frame_width_minus_1 + 1`, the § 6.8.8 `lcr_max_pic_width` operand.
    max_frame_width: u32,
    /// `max_frame_height_minus_1 + 1`, the § 6.8.8 `lcr_max_pic_height` operand.
    max_frame_height: u32,
    /// `seq_cropping_window_present_flag`, the § 6.8.8 cropping-present operand.
    cropping_present: bool,
    /// `seq_cropping_win_{left,right,top,bottom}_offset`, the § 6.8.8 cropping offsets
    /// (inferred to 0 when the window is absent).
    cropping_offsets: (u32, u32, u32, u32),
}

/// Projects the LCR-agreement dedup fingerprint out of an activated sequence header's
/// general fields (see [`LcrAgreementValueFingerprint`]).
fn lcr_agreement_value_fingerprint(
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

/// Returns `true` if `obu_type`'s payload begins with a `frame_header()` or
/// `tile_group_obu()` (AV2 v1.0.0 § 5.2.1): the tile-group types, plus the SEF / TIP
/// / bridge frames that call `frame_header( 1 )` directly.
/// Emits `film-grain/scaling-point-not-increasing` for any scaling point whose
/// (cumulative) value is not strictly greater than its predecessor or is not less than
/// 256 (AV2 v1.0.0 § 6.17.10.2: for `i > 0`, `point_*_value[i] > point_*_value[i - 1]`
/// and `< 256`).
fn emit_scaling_point_order_diagnostics(
    channel: &str,
    points: &[FilmGrainScalingPoint],
    slot: u8,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    for i in 1..points.len() {
        let value = points[i].value;
        if value <= points[i - 1].value || value >= 256 {
            report.push(
                Diagnostic::error(
                    "film-grain/scaling-point-not-increasing",
                    format!(
                        "film grain slot {slot} {channel} scaling point {i} value {value} must be \
                         strictly greater than the previous point and less than 256"
                    ),
                )
                .with_spec_section("6.17.10.2")
                .with_byte_offset(obu.offset),
            );
        }
    }
}

fn is_frame_bearing(obu_type: ObuType) -> bool {
    obu_type.is_tile_group()
        || obu_type.is_sef()
        || obu_type.is_tip_frame()
        || obu_type == ObuType::BridgeFrame
}

/// The § 7.3.6 all-leading-or-none [`Leadingness`] derived from `obu_type`, mirroring AVM's
/// tri-state `is_leading_picture` (`av2/decoder/obu.c:2544-2549`) rather than the § 6.4.1-area
/// gloss (`06-syntax-structures-semantics.md:4546`) that reads `IsRegular == 0` as exactly
/// "leading":
///
/// - the `av2_is_leading_vcl_obu` set (`av2/decoder/obu.c:1666` — `OBU_LEADING_TILE_GROUP`,
///   `OBU_LEADING_SEF`, `OBU_LEADING_TIP`) is [`Leadingness::Leading`];
/// - the `av2_is_regular_vcl_obu` set (`av2/decoder/decodeframe.c:7015` — `OLK` plus
///   `REGULAR_TILE_GROUP` / `REGULAR_SEF` / `REGULAR_TIP` / `SWITCH` / `RAS` / `BRIDGE`,
///   i.e. the § 5.18.2 `IsRegular == 1` set) is [`Leadingness::Regular`];
/// - a CLK lands in neither AVM set, so the oracle leaves `is_leading_picture == -1`; the
///   validator follows it and classes a CLK [`Leadingness::Indeterminate`], excluding it
///   from the all-leading-or-none judgment (the documented ambiguous-spec under-report).
///
/// Type-decided, so the § 7.3.6 all-leading-or-none rule never routes to Unknown.
fn frame_leadingness(obu_type: ObuType) -> Leadingness {
    match obu_type {
        ObuType::LeadingTileGroup | ObuType::LeadingSef | ObuType::LeadingTip => {
            Leadingness::Leading
        }
        ObuType::OpenLoopKey
        | ObuType::RegularTileGroup
        | ObuType::RegularTip
        | ObuType::RegularSef
        | ObuType::Switch
        | ObuType::RasFrame
        | ObuType::BridgeFrame => Leadingness::Regular,
        // A CLK is neither leading nor regular under the AVM tri-state (the § 6.4.1 gloss
        // would call it leading; the oracle and this validator do not).
        _ => Leadingness::Indeterminate,
    }
}

/// Parses a frame-bearing OBU's frame-header prefix (best-effort), returning `None`
/// on any parse failure or when no parseable frame header is present (an absent
/// header or a non-first tile group's `frame_header_copy()`).
fn parse_frame_prefix(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
) -> Option<FrameHeaderPrefix> {
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    if obu.header.obu_type.is_tile_group() {
        // Tile-group OBUs carry tile_group_obu(); a frame header is parseable only for
        // the first tile group (a non-first tile group carries frame_header_copy()).
        parse_tile_group_prefix(&mut reader, obu.header.obu_type, first_picture_in_tu)
            .ok()
            .and_then(|tile_group| tile_group.frame_header)
    } else {
        // SEF / TIP / bridge frames call frame_header( 1 ) directly (AV2 § 5.2.1).
        parse_frame_header_prefix(&mut reader, obu.header.obu_type, first_picture_in_tu).ok()
    }
}

/// Parses the frame-header core of a frame-bearing OBU against its active sequence
/// header (AV2 § 5.18.2), positioning the reader past the `tile_group_obu()` prefix
/// for tile-group OBUs. Returns `None` when there is no parseable first-tile-group
/// frame header or the core parse fails (best-effort, never an error).
///
/// `mfh_record` is the in-band multi-frame header resolving this frame's `cur_mfh_id`
/// (`> 0`) reference, or `None` for a `cur_mfh_id == 0` direct reference (or when the
/// MFH is unavailable). It is threaded into the core parser so the `cur_mfh_id > 0`
/// paths can resolve their multi-frame-header-derived state as that coverage lands; the
/// currently-reachable fields (the §5.18.2 control region through the output flags) are
/// determined by the active sequence header alone.
fn parse_frame_core(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
    active_sequence: &SequenceHeader,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    reference_state: FrameReferenceStateView<'_>,
) -> Option<FrameHeaderCore> {
    let obu_type = obu.header.obu_type;
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    if obu_type.is_tile_group() {
        // tile_group_obu(): only the first tile group carries a parseable frame_header(1);
        // its frame_header_present_flag is inferred 1 (AV2 § 5.19).
        if reader.read_bit().ok()? == 0 {
            return None;
        }
    } else if !is_frame_bearing(obu_type) {
        return None;
    }
    let input = FrameHeaderParseInput {
        obu_type,
        first_picture_in_tu,
        active_sequence: Some(active_sequence),
        mfh_record,
        // AV2 § 7.23: the modeled per-extended-layer reference-frame buffer view. No
        // §5.18 INTRA parse branch consumes it today (the intra paths derive their state
        // without RefValid/RefOrderHint/dims); it is forward plumbing so the §5.18 INTER
        // reference paths (explicit reference map, frame_size_with_refs, primary-ref) can
        // read the modeled state once they land (AV2-5.18.2-FRAME-HEADER-INFO inter path)
        // without changing the parser's call signature. The validator already consumes
        // the modeled state directly for the §6.17.2 show-existing-frame slot check.
        reference_state,
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).ok()
}

/// The cross-OBU HLS-availability state a frame's reference checks consult (AV2 § 7.3.8):
/// per-level quantizer-matrix availability (§ 7.3.8.9 / § 6.17.6.2) and per-slot
/// film-grain availability (§ 7.3.8.8 / § 6.17.10.1). Bundled so the frame-header check
/// keeps one availability parameter as more reference families land.
#[derive(Clone, Copy)]
struct FrameReferenceAvailability<'a> {
    /// Per-level custom quantizer-matrix availability (AV2 § 6.17.6.2).
    qm: &'a QuantizerMatrixState,
    /// Per-slot film-grain model availability (AV2 § 6.17.10.1 / § 7.3.8.8).
    film_grain: &'a FilmGrainState,
}

/// Emits the locally decidable frame-header-info / frame-size diagnostics for a frame
/// whose active sequence header is available (AV2 § 6.17.2 / § 6.17.4 / § 6.4.6).
///
/// Only state-supported checks are emitted; paths that need reference-frame buffer
/// state (show-existing-frame slot validity, explicit reference maps,
/// `primary_ref_frame` range) are left to a future phase rather than guessed.
fn frame_header_core_checks(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
    active_sequence: &SequenceHeader,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    reference_state: FrameReferenceAvailability<'_>,
    options: &ValidationOptions,
    report: &mut ValidationReport,
) {
    let FrameReferenceAvailability {
        qm: qm_state,
        film_grain: film_grain_state,
    } = reference_state;
    // AV2 § 6.4.6: if long_term_frame_id_bits == 0, no OBU_RAS_FRAME shall be present
    // in the coded video sequence. Decidable from obu_type + the active sequence alone.
    if obu.header.obu_type == ObuType::RasFrame
        && let Some(inter) = active_sequence.inter.as_ref()
        && inter.long_term_frame_id_bits == 0
    {
        report.push(frame_header_error(
            "frame-header/ras-requires-long-term-frame-id-bits",
            "6.4.6",
            obu,
            "OBU_RAS_FRAME is present, but the active sequence header has \
             long_term_frame_id_bits == 0"
                .to_owned(),
        ));
    }

    // AV2 § 6.4.1: "If obu_type is equal to either OBU_SWITCH or OBU_RAS_FRAME, it is a
    // requirement of bitstream conformance that, for any embedded layer ID m not equal
    // to obu_mlayer_id, MLayerDependencyMap[obu_mlayer_id][m] shall be equal to 0."
    // (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1, lines 615-617). A
    // SWITCH / RAS frame must be self-contained: its embedded layer may not depend on
    // any other embedded layer. Decidable from obu_type + obu_mlayer_id + the active
    // sequence's MLayerDependencyMap alone, like the § 6.4.6 RAS check above.
    if matches!(obu.header.obu_type, ObuType::Switch | ObuType::RasFrame) {
        let curr = obu.header.embedded_layer_id;
        let map = &active_sequence.general.mlayer_dependency_map;
        // Scanning the full 0..MAX_NUM_MLAYERS range never reports a layer undeclared by
        // this sequence header: the § 5.4.1 parser only ever sets MLayerDependencyMap
        // entries where refLayer <= currLayer <= max_mlayer_id (default fill and signaled
        // override alike), so depends_on(curr, m) is unconditionally false for any
        // m > max_mlayer_id and the dependency-scope constraint cannot yield a false
        // positive here.
        for raw_m in 0..MAX_NUM_MLAYERS {
            // raw_m fits in the 3-bit obu_mlayer_id range (MAX_NUM_MLAYERS == 8).
            let m = EmbeddedLayerId::from_bits(raw_m as u8);
            if m == curr {
                continue;
            }
            if map.depends_on(curr, m) {
                report.push(frame_header_error(
                    "frame-header/switch-or-ras-mlayer-dependency-not-self-contained",
                    "6.4.1",
                    obu,
                    format!(
                        "{} with obu_mlayer_id {} has MLayerDependencyMap[{}][{}] != 0 in the \
                         active sequence header, but a SWITCH / RAS frame must not depend on any \
                         other embedded layer",
                        obu.header.obu_type.spec_name(),
                        curr.get(),
                        curr.get(),
                        m.get()
                    ),
                ));
            }
        }
    }

    let max_width = active_sequence.general.max_frame_width.get();
    let max_height = active_sequence.general.max_frame_height.get();

    // AV2 § 6.17.2: after load_sequence_header(), for every `cur_mfh_id > 0` frame it is a
    // requirement of bitstream conformance that the *referenced multi-frame header's stored*
    // dimensions satisfy mfh_frame_width_minus_1[ cur_mfh_id ] <= max_frame_width_minus_1 and
    // mfh_frame_height_minus_1[ cur_mfh_id ] <= max_frame_height_minus_1
    // (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2, mirror :4348-4349).
    // This bounds the MFH's *stored* dims and is INDEPENDENT of frame_size_override_flag and
    // of how far the referencing frame header parses: it is decidable from the resolved MFH
    // record and the active sequence maxima alone, at the load_sequence_header point. So it
    // runs here, BEFORE (and independent of) the `parse_frame_core` outcome below — a
    // truncated / malformed frame-header remainder (`core == None`) must not silence this
    // decidable diagnostic. A frame overriding to in-range dims (so `core.frame_size` is
    // conformant) still must not reference an out-of-range MFH. The predicate (stored MFH
    // dims) differs from the §6.17.4.1 derived-FrameWidth check below, so it has its own
    // rule id. An MFH with no `mfh_frame_size` payload infers its default dims to the
    // sequence maxima (§5.18.2, mirror :4101) and is trivially in range, so the omitted-size
    // case is silent here. Anchored at `obu` (the referencing frame's OBU) and emitted once
    // per referencing frame header. On this resolution path `record.mfh_id == cur_mfh_id`
    // (`resolve_frame_mfh_record` looks the record up by the prefix's `cur_mfh_id`), so the
    // message's id matches the referencing frame's `cur_mfh_id`. An unresolvable MFH leaves
    // `mfh_record == None` (the shared guard) and stays silent.
    let mfh_stored_dims = if let Some(record) = mfh_record
        && let Some(mfh_size) = record.mfh_frame_size
    {
        let mfh_width = mfh_size.width_minus_1 + 1;
        let mfh_height = mfh_size.height_minus_1 + 1;
        if mfh_width > max_width || mfh_height > max_height {
            report.push(frame_header_error(
                "frame-header/mfh-frame-size-exceeds-sequence-max",
                "6.17.2",
                obu,
                format!(
                    "the referenced multi-frame header (cur_mfh_id {}) stores \
                     FrameWidth={}, FrameHeight={}, which exceeds the active sequence \
                     maximum {}x{} (§6.17.2 mfh_frame_width/height_minus_1 <= \
                     max_frame_width/height_minus_1)",
                    record.mfh_id.get(),
                    mfh_width,
                    mfh_height,
                    max_width,
                    max_height
                ),
            ));
        }
        Some((mfh_width, mfh_height))
    } else {
        None
    };

    // This call site emits the §6.17 bridge-ref / frame-size / tile / quant diagnostics.
    // A `cur_mfh_id > 0` frame's FrameWidth/FrameHeight come from `mfh_record` on the
    // non-override path (§5.18.4.1, mirror :5767), so the resolved record is threaded in
    // with the §7.3.8.7 discipline (the caller passes `resolve_frame_mfh_record`'s result).
    // For a `cur_mfh_id == 0` frame, or a `cur_mfh_id > 0` frame whose in-band MFH is
    // unresolvable, this is `None` and the core parser keeps its existing early-stop. The
    // §6.17.2 stored-MFH bound above already ran, so it is not lost when the core parse stops.
    //
    // The §7.23 reference-frame buffer view is `unknown()` here: this free function holds
    // no reference-state tracker, and none of the §6.17 diagnostics it emits consult
    // reference state (they are decidable from the active sequence header alone). The
    // modeled buffer is threaded into the method core parse
    // (`frame_core_against_referenced_header`) instead, where the validator owns it.
    let Some(core) = parse_frame_core(
        obu,
        first_picture_in_tu,
        active_sequence,
        mfh_record,
        FrameReferenceStateView::unknown(),
    ) else {
        return;
    };

    // AV2 § 6.2.1 / § 5.18.2: the frame_header_info() syntax elements (§ 5.18.2) are
    // mandatory — `frame_header( )` reads them sequentially from the OBU payload inside
    // open_bitstream_unit() (§ 5.2.1). The payload is bounded by obuPayloadSize and "lies
    // between the first bit of the given bytes and the last bit before the first trailing
    // bit" (§ 6.2.1, docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-1, lines
    // 47-60); trailing bits are always present (unless header-only), so a payload that ends
    // BEFORE a mandatory syntax element is malformed — the §6.2.1 NOTE makes "the parsing
    // of the OBU header and payload leads to the consumption of bits within the trailing
    // bits" a detectable error condition. The core parser preserves the already-parsed
    // facts and reports the truncation through one of the EOF-in-a-fully-modeled-region
    // statuses (StoppedInsideFilterParams / StoppedInsideIntraTail /
    // StoppedInsideShowExistingFrame). Those — and ONLY those — are a decidable defect:
    // an unsupported-coverage stop (StoppedBeforeWienerNsFilter, UnsupportedUntilFeature,
    // the MFH-unresolvable stops, CoreFieldsOnly) stops where this parser does not fully
    // model the following syntax, so its early end is not evidence of truncation and must
    // stay silent. `is_truncated_in_modeled_region()` is the exact partition (documented on
    // FrameHeaderParseStatus). Anchored at the frame's OBU. The facts path is untouched:
    // the preserved core fields still feed every diagnostic below, so a truncated frame
    // keeps contributing its decided facts (celu / frame-unit judgments unchanged).
    if core.status.is_truncated_in_modeled_region() {
        report.push(frame_header_error(
            "frame-header/truncated-frame-header",
            "6.2.1",
            obu,
            format!(
                "the OBU payload ends inside the frame header before mandatory \
                 frame_header_info() syntax (§5.18.2) could be read (parse stopped: {}); \
                 the §6.2.1 OBU payload must contain every mandatory frame-header syntax \
                 element",
                core.status.label()
            ),
        ));
    }

    // AV2 § 5.2.1 (:124-152) / § 5.2.3 / § 6.2.1: a show-existing-frame OBU's payload is
    // exactly the SEF frame_header() plus trailing_bits( remainingPayloadBits ). The SEF
    // arm of § 5.18.2 (mirror :4145) return()s right after film_grain_config() (:4186), and
    // a SEF OBU is not an is_tile_group() type, so usedArith == 0 and there is no tile data
    // — the boundary is decidable from the payload alone. A non-conformant tail (no
    // trailing_one_bit, or a stray set bit after it, including the grain_seed-eats-the-marker
    // case) is a § 6.2.1 / § 6.2.3 conformance defect. The core parser classifies the tail
    // without failing (the parsed SEF facts survive), so surface a non-Valid outcome here.
    if let Some(violation) = core.sef_trailing_bits
        && let Some(message) = violation.violation_message()
    {
        report.push(frame_header_error(
            "frame-header/sef-trailing-bits-invalid",
            "6.2.3",
            obu,
            format!(
                "the show-existing-frame OBU payload's §5.2.3 trailing_bits() is malformed: \
                 {message}"
            ),
        ));
    }

    // AV2 § 6.17.2: bridge_frame_ref_idx must name a valid reference slot, so it must
    // be less than NumRefFrames.
    if let Some(idx) = core.bridge_frame_ref_idx
        && let Some(inter) = active_sequence.inter.as_ref()
        && idx >= u32::from(inter.num_ref_frames)
    {
        report.push(frame_header_error(
            "frame-header/bridge-ref-index-out-of-range",
            "6.17.2",
            obu,
            format!(
                "bridge_frame_ref_idx {idx} must be less than NumRefFrames {}",
                inter.num_ref_frames
            ),
        ));
    }

    // FrameWidth/FrameHeight do not exceed the active sequence maximum
    // (FrameWidth <= MaxFrameWidth, FrameHeight <= MaxFrameHeight). On the explicit
    // override path this is AV2 § 6.17.4.1 (frame_width_minus_1 <= max_frame_width_minus_1,
    // frame_height_minus_1 <= max_frame_height_minus_1,
    // docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1, mirror :5200-5205).
    // On the `cur_mfh_id > 0` non-override path FrameWidth = mfh_frame_width_minus_1 + 1
    // (mirror :5767), so `core.frame_size` carries the MFH's stored dims verbatim — that
    // exact case is already the §6.17.2 stored-MFH check above, the single home for
    // stored-MFH dims. To avoid double-reporting the identical numbers, the derived check
    // defers ONLY on that parsed PATH — `frame_size_override_flag == 0` on a resolved
    // `cur_mfh_id > 0` frame (§5.18.4 / §5.18.2, mirror :5767), where FrameWidth/Height are
    // the MFH default dimensions and carry no explicit fields of their own. The suppression
    // keys on provenance (the override flag), NOT on dimension equality: an override==1 frame
    // that explicitly codes the same out-of-range dims the MFH stores commits a genuine,
    // separate §6.17.4.1 violation through its own frame_width/height_minus_1 fields, so both
    // checks legitimately fire even when the numbers coincide. (`mfh_stored_dims.is_some()`
    // bounds the deferral to the case the §6.17.2 home actually examined those dims.)
    let derived_is_mfh_default = core.frame_size_override_flag == Some(false)
        && !core.cur_mfh_id.is_zero()
        && mfh_stored_dims.is_some();
    if !derived_is_mfh_default
        && let Some(size) = core.frame_size
        && (size.width > max_width || size.height > max_height)
    {
        report.push(frame_header_error(
            "frame-header/frame-size-exceeds-sequence-max",
            "6.17.4.1",
            obu,
            format!(
                "frame_header_info() derives FrameWidth={}, FrameHeight={}, which \
                 exceeds the active sequence maximum {}x{}",
                size.width, size.height, max_width, max_height
            ),
        ));
    }

    // AV2 § 6.17.2: ref_long_term_id[i] != (1 << long_term_frame_id_bits) - 1.
    if core.forbidden_ref_long_term_id {
        report.push(frame_header_error(
            "frame-header/ref-long-term-id-reserved",
            "6.17.2",
            obu,
            "a ref_long_term_id[i] equals the reserved (1 << long_term_frame_id_bits) - 1"
                .to_owned(),
        ));
    }

    // AV2 § 6.17.2: if immediate_output_frame == 0, refresh_frame_flags must be nonzero
    // (a deferred-output frame must update at least one reference slot).
    if core.immediate_output_frame == Some(false) && core.refresh_frame_flags == Some(0) {
        report.push(frame_header_error(
            "frame-header/refresh-frame-flags-zero-on-deferred-output",
            "6.17.2",
            obu,
            "immediate_output_frame == 0 requires refresh_frame_flags to be nonzero".to_owned(),
        ));
    }

    // AV2 § 6.17.2: still_picture == 1 requires FrameType == KEY_FRAME and
    // immediate_output_frame == 1.
    if active_sequence.general.still_picture
        && (matches!(core.frame_type, Some(frame_type) if frame_type != FrameType::Key)
            || core.immediate_output_frame == Some(false))
    {
        report.push(frame_header_error(
            "frame-header/still-picture-requires-key-frame",
            "6.17.2",
            obu,
            "a still_picture sequence requires a KEY_FRAME with immediate_output_frame == 1"
                .to_owned(),
        ));
    }

    // AV2 § 6.17.7.2: tile-info bounds for a parsed `tile_info()`.
    if let Some(tile_info) = core.tile_info.as_ref() {
        frame_tile_info_checks(tile_info, obu, report);
    }

    // AV2 § 5.19 / § 6.18: the post-frame-header tile_group_obu() structure — the
    // tile-group range (tg_start/tg_end) and the headerBytes/payload boundary — is
    // decidable on the intra-complete first-tile-group path (use_bru/bru_inactive derive
    // to 0). Emits the locally-decidable §6.18 tg-range diagnostics; a non-intra-complete
    // or non-first-tile-group frame is the BRU-undecidable honest stop and stays silent.
    tile_group_range_checks(
        obu,
        first_picture_in_tu,
        active_sequence,
        mfh_record,
        report,
    );

    // Annex A.4 static level limits for the parsed frame size / tile count against the
    // active sequence header's seq_level_idx / seq_tier.
    frame_annex_a_level_checks(&core, active_sequence, obu, report);

    // AV2 § 6.17.6.2: custom-QM plane-count references for a parsed
    // `setup_qm_params()`, gated on recorded quantizer-matrix availability state.
    if let Some(setup_qm) = core.setup_qm_params.as_ref() {
        frame_qm_reference_checks(setup_qm, active_sequence, qm_state, obu, report);
    }

    // AV2 § 6.17.7.8: per-plane CCSO field bounds for a parsed `ccso_params()`.
    if let Some(ccso) = core.ccso_params.as_ref() {
        frame_ccso_params_checks(ccso, obu, report);
    }

    // AV2 § 6.17.10.1 / § 7.3.8.8: when `apply_grain == 1`, a film grain OBU that has set
    // FilmGrainPresent[ fgm_id ] == 1 for the referenced fgm_id must be available. The
    // parsed film_grain_config() lives on the SEF path (`sef_film_grain`) or the intra tail
    // (`intra_tail.film_grain`).
    if let Some(film_grain) = core
        .sef_film_grain
        .as_ref()
        .or_else(|| core.intra_tail.as_ref().map(|tail| &tail.film_grain))
    {
        frame_film_grain_reference_checks(film_grain, film_grain_state, options, obu, report);
    }

    // The remaining checks compare refresh_frame_flags against NumRefFrames.
    let Some(num_ref_frames) = active_sequence
        .inter
        .as_ref()
        .map(|inter| u32::from(inter.num_ref_frames))
    else {
        return;
    };
    let Some(refresh) = core.refresh_frame_flags else {
        return;
    };
    // 1 << NumRefFrames as the exclusive upper bound of a valid refresh mask.
    let Some(all_slots_plus_1) = 1u32.checked_shl(num_ref_frames) else {
        return;
    };

    // AV2 § 6.17.2: frame_to_refresh < NumRefFrames. In the compact refresh mode
    // refresh_frame_flags == 1 << frame_to_refresh, so an out-of-range slot is exactly a
    // mask with a bit at or beyond NumRefFrames; the full and all-frames forms are always
    // below 1 << NumRefFrames.
    if refresh >= all_slots_plus_1 {
        report.push(frame_header_error(
            "frame-header/frame-to-refresh-out-of-range",
            "6.17.2",
            obu,
            format!(
                "refresh_frame_flags {refresh:#x} sets a reference slot at or beyond \
                 NumRefFrames {num_ref_frames} (frame_to_refresh must be less than NumRefFrames)"
            ),
        ));
    }

    // AV2 § 6.17.2: an INTRA_ONLY_FRAME with NumRefFrames > 1 must not refresh every slot
    // (refresh_frame_flags != (1 << NumRefFrames) - 1).
    if core.frame_type == Some(FrameType::IntraOnly)
        && num_ref_frames > 1
        && refresh == all_slots_plus_1 - 1
    {
        report.push(frame_header_error(
            "frame-header/intra-only-refresh-all-slots",
            "6.17.2",
            obu,
            format!(
                "an INTRA_ONLY_FRAME with NumRefFrames {num_ref_frames} must not set \
                 refresh_frame_flags to all slots"
            ),
        ));
    }
}

/// Emits the locally decidable § 6.18 tile-group-range diagnostics for the FIRST tile
/// group of an intra-complete coded frame (AV2 v1.0.0 § 5.19 / § 6.18).
///
/// The § 5.19 structure after `frame_header()` is decidable only when the first tile
/// group's frame header parsed to completion on the intra path
/// ([`FrameHeaderParseStatus::IntraHeaderComplete`] with `frame_is_intra == Some(true)`
/// and a parsed `tile_info()`): then `use_bru == 0` and `bru_inactive == 0` are the
/// § 5.18.2 intra-derived constants (mirror :4127-4129 / :4653), so the `bru_inactive`
/// early-return and the `use_bru` `bru_tile_active` loop are both dead, and
/// [`parse_tile_group_structure`] consumes the structure exactly. `NumTiles` /
/// `TileColsLog2` / `TileRowsLog2` come from the parsed `tile_info()`.
///
/// The locally-decidable § 6.18 clauses for the FIRST tile group are:
///
/// - **tg_start of the first tile group is 0** (mirror :6215-6216: `tg_start` equals
///   `TileNum` at `tile_group_payload`, and `TileNum = 0` for the first tile group of a
///   regular intra frame, mirror :3956);
/// - **tg_end >= tg_start** (mirror :6220);
/// - **tg_end <= NumTiles - 1** (mirror :6218-6223 — `tg_end` is a zero-based tile index,
///   and the last tile group's `tg_end` is `NumTiles - 1`, so no `tg_end` may exceed it).
///
/// Under-reported (needs prior-tile-group state the segmenter would thread): the
/// cross-tile-group continuity (`tg_start == previous tg_end + 1`) and the requirement
/// that the LAST tile group's `tg_end == NumTiles - 1` when the range is split across
/// multiple groups (residual: tile-group-continuity-across-groups). Only the first tile
/// group is checked here, so only the `TileNum == 0` instance of the continuity clause is
/// decided.
fn tile_group_range_checks(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
    active_sequence: &SequenceHeader,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    report: &mut ValidationReport,
) {
    // Only a tile-group OBU carries the §5.19 tile_group_obu() structure; SEF / TIP /
    // bridge frames route through decode_frame_wrapup() (mirror :3942-3958) with no
    // tile_group_obu() control region.
    if !obu.header.obu_type.is_tile_group() {
        return;
    }

    // Re-parse from the OBU payload start so the reader is positioned exactly past
    // frame_header() (the same span parse_frame_core consumes), then derive the structure.
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    // tile_group_obu(): is_first_tile_group must be 1 for a parseable first frame_header(1)
    // (a non-first tile group carries frame_header_copy(), not checked here — its tg range
    // needs the prior-group continuity state). A read failure or a 0 flag leaves the
    // structure undecidable.
    let Ok(is_first) = reader.read_bit() else {
        return;
    };
    if is_first == 0 {
        return;
    }
    let input = FrameHeaderParseInput {
        obu_type: obu.header.obu_type,
        first_picture_in_tu,
        active_sequence: Some(active_sequence),
        mfh_record,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    let Ok(core) = parse_frame_header_core(&mut reader, &input) else {
        return;
    };

    // The §5.19 structure is decidable only on the intra-complete path: IntraHeaderComplete
    // guarantees the whole frame_header_info() parsed (so the reader sits exactly at the end
    // of frame_header()), frame_is_intra makes use_bru/bru_inactive the derived 0 constants,
    // and tile_info supplies NumTiles / TileColsLog2 / TileRowsLog2. Any other stop (a
    // coverage stop, an inter/TIP/bridge path, or a truncation) is the BRU-undecidable
    // honest stop (BruUndecidable::NotIntraComplete) — leave the range unjudged.
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete
        || core.frame_is_intra != Some(true)
    {
        return;
    }
    let Some(tile_info) = core.tile_info.as_ref() else {
        return;
    };

    let layout = TileGroupLayout::new(
        tile_info.tile_cols,
        tile_info.tile_rows,
        tile_info.tile_cols_log2,
        tile_info.tile_rows_log2,
    );
    let num_tiles = layout.num_tiles;
    // sz is the OBU payload size in bytes (§5.2.1); obu.payload is exactly that slice.
    let sz = obu.payload.len() as u64;
    let Ok(structure) = parse_tile_group_structure(&mut reader, layout, sz) else {
        // The only non-EOF error is a §6.2.4 byte_alignment() zero-bit defect. The
        // byte_alignment() reachability is owned by AV2-5.2.4-BYTE-ALIGNMENT and the
        // tile-group dispatch does not yet route that diagnostic to this OBU; surface it
        // through the dedicated tile-group rule so the defect is not silently dropped.
        report.push(frame_header_error(
            "tile-group/byte-alignment-zero-bit",
            "6.2.4",
            obu,
            "the §5.19 tile_group_obu() byte_alignment() padding contains a non-zero \
             zero_bit (§6.2.4 requires every alignment bit to be 0)"
                .to_owned(),
        ));
        return;
    };

    // A truncation inside the §5.19 structure means the OBU payload ended before the
    // tile-group range / byte_alignment() could be read — a §6.2.1 mandatory-syntax
    // truncation, parallel to frame-header/truncated-frame-header. The already-parsed
    // facts are preserved on `structure`; surface the truncation rather than judging an
    // incomplete range.
    if structure.outcome == TileGroupStructureOutcome::Truncated {
        report.push(frame_header_error(
            "tile-group/truncated-structure",
            "6.2.1",
            obu,
            "the OBU payload ends inside the §5.19 tile_group_obu() structure \
             (tile_start_and_end_present_flag / tg_start / tg_end / byte_alignment) before \
             it could be read; the §6.2.1 OBU payload must contain every mandatory \
             tile-group syntax element"
                .to_owned(),
        ));
        return;
    }

    // §6.18 (mirror :6215-6216): tg_start of the FIRST tile group equals TileNum == 0.
    // Only the explicit-range path (tile_start_and_end_present_flag == 1) can violate it;
    // the inferred path sets tg_start = 0 by construction.
    if structure.tile_start_and_end_present_flag && structure.tg_start != 0 {
        report.push(frame_header_error(
            "tile-group/first-tg-start-not-zero",
            "6.18",
            obu,
            format!(
                "the first tile group codes tg_start={}, but §6.18 requires tg_start to \
                 equal TileNum at tile_group_payload, which is 0 for the first tile group \
                 of the coded frame (§5.19 mirror :3956)",
                structure.tg_start
            ),
        ));
    }

    // §6.18 (mirror :6220): tg_end >= tg_start.
    if structure.tg_end < structure.tg_start {
        report.push(frame_header_error(
            "tile-group/tg-end-before-tg-start",
            "6.18",
            obu,
            format!(
                "the tile group codes tg_end={} < tg_start={}, but §6.18 requires tg_end to \
                 be greater than or equal to tg_start",
                structure.tg_end, structure.tg_start
            ),
        ));
    }

    // §6.18 (mirror :6218-6223): tg_end is a zero-based tile index and the last tile
    // group's tg_end is NumTiles - 1, so no tg_end may exceed NumTiles - 1. Decidable from
    // the explicit range and NumTiles; the inferred path sets tg_end = NumTiles - 1.
    if structure.tile_start_and_end_present_flag
        && num_tiles > 0
        && structure.tg_end > num_tiles - 1
    {
        report.push(frame_header_error(
            "tile-group/tg-end-out-of-range",
            "6.18",
            obu,
            format!(
                "the tile group codes tg_end={}, which exceeds NumTiles-1={} (§6.18: tg_end \
                 is a zero-based tile index and the last tile group's tg_end is NumTiles-1)",
                structure.tg_end,
                num_tiles - 1
            ),
        ));
    }

    // Likewise tg_start must be a valid tile index (< NumTiles). For the first tile group
    // the stricter tg_start == 0 check above subsumes this, but a coded tg_start beyond
    // NumTiles is still independently a §6.18 bounds defect worth its own anchor only when
    // the first-tg-start-not-zero rule did not already fire — which it always does for any
    // nonzero tg_start. So no separate tg_start-bounds rule is emitted for the first group.
}

/// Emits the locally decidable § 6.17.7.2 tile-info diagnostics for a parsed frame
/// `tile_info()` (AV2 v1.0.0 § 6.17.7.2,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-2`):
/// `TileCols <= MAX_TILE_COLS`, `TileRows <= MAX_TILE_ROWS`, and
/// `context_update_tile_id < TileCols * TileRows`. `MAX_TILE_COLS` /
/// `MAX_TILE_ROWS` are 64 (AV2 § 3, `docs/spec/av2/1.0.0/03-symbols.md`).
fn frame_tile_info_checks(
    tile_info: &TileInfo,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    // AV2 § 6.17.7.2: "It is a requirement of bitstream conformance that TileCols is
    // less than or equal to MAX_TILE_COLS." Reachable for a non-uniform layout that
    // codes more than MAX_TILE_COLS one-superblock tiles.
    if tile_info.tile_cols > MAX_TILE_COLS {
        report.push(frame_header_error(
            "frame-header/tile-cols-out-of-range",
            "6.17.7.2",
            obu,
            format!(
                "tile_info() derives TileCols {}, which must be less than or equal to \
                 MAX_TILE_COLS ({MAX_TILE_COLS})",
                tile_info.tile_cols
            ),
        ));
    }
    // AV2 § 6.17.7.2: "It is a requirement of bitstream conformance that TileRows is
    // less than or equal to MAX_TILE_ROWS."
    if tile_info.tile_rows > MAX_TILE_ROWS {
        report.push(frame_header_error(
            "frame-header/tile-rows-out-of-range",
            "6.17.7.2",
            obu,
            format!(
                "tile_info() derives TileRows {}, which must be less than or equal to \
                 MAX_TILE_ROWS ({MAX_TILE_ROWS})",
                tile_info.tile_rows
            ),
        ));
    }
    // AV2 § 6.17.7.2: "It is a requirement of bitstream conformance that
    // context_update_tile_id is less than TileCols * TileRows." Reachable because the
    // f(TileRowsLog2 + TileColsLog2) read can encode values at or beyond the actual
    // tile count when the count is not a power of two. The skipped-read paths
    // (single tile, avg-CDF gating) leave the value 0, which never trips the bound
    // for the >= 1 tile counts every parsed layout produces.
    let tile_count = u64::from(tile_info.tile_cols) * u64::from(tile_info.tile_rows);
    if u64::from(tile_info.context_update_tile_id) >= tile_count {
        report.push(frame_header_error(
            "frame-header/context-update-tile-id-out-of-range",
            "6.17.7.2",
            obu,
            format!(
                "context_update_tile_id {} must be less than TileCols * TileRows ({} * {})",
                tile_info.context_update_tile_id, tile_info.tile_cols, tile_info.tile_rows
            ),
        ));
    }
}

/// Emits the locally decidable § 6.17.7.8 CCSO-params diagnostics for a parsed frame
/// `ccso_params()` (AV2 v1.0.0 § 6.17.7.8,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-8`, mirror
/// :5819 / :5824):
///
/// - `frame-header/ccso-ext-filter-reserved` (error): `ccso_ext_filter != 7` (mirror
///   :5819). `ccso_ext_filter` is `f(3)` (0..=7), so the reserved value 7 is reachable.
/// - `frame-header/ccso-max-band-out-of-range` (error): `1 << ccso_max_band_log2 <=
///   CCSO_BAND_NUM` (mirror :5824). `ccso_max_band_log2` is `f(2 + ccso_bo_only)`
///   (0..=7), so a value > 6 (`1 << 7 == 128 > CCSO_BAND_NUM == 64`) violates the bound;
///   it is only reachable in the `ccso_bo_only` arm (`f(3)`).
///
/// Both bounds are fully determined by the parsed per-plane fields, so they hold on the
/// intra path independent of reference-frame state. The reference-state CCSO requirements
/// (`ccso_ref_idx < NumTotalRefs`, the `SavedCcso*` / `RefMi*` reuse equalities) are dead
/// on the intra path (`NumTotalRefs == 0`), so they are not modeled here.
fn frame_ccso_params_checks(
    ccso: &CcsoParams,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    for (plane, params) in ccso.planes.iter().enumerate() {
        // AV2 § 6.17.7.8 (:5819): ccso_ext_filter is not equal to 7. Present only on the
        // non-ccso_bo_only arm (otherwise inferred 0); `None` when ccso_planes[plane] == 0.
        if params.ccso_ext_filter == Some(7) {
            report.push(frame_header_error(
                "frame-header/ccso-ext-filter-reserved",
                "6.17.7.8",
                obu,
                format!(
                    "ccso_ext_filter for plane {plane} is 7, which is the reserved value \
                     §6.17.7.8 forbids"
                ),
            ));
        }
        // AV2 § 6.17.7.8 (:5824): 1 << ccso_max_band_log2 <= CCSO_BAND_NUM. Use a widened
        // shift so a non-conformant value cannot overflow: ccso_max_band_log2 is f(2..=3)
        // (0..=7), and `1u32 << 7` is in range.
        if let Some(max_band_log2) = params.ccso_max_band_log2 {
            let max_band = 1u32 << u32::from(max_band_log2);
            if max_band > CCSO_BAND_NUM {
                report.push(frame_header_error(
                    "frame-header/ccso-max-band-out-of-range",
                    "6.17.7.8",
                    obu,
                    format!(
                        "ccso_max_band_log2 for plane {plane} is {max_band_log2}, so \
                         1 << ccso_max_band_log2 == {max_band} exceeds CCSO_BAND_NUM \
                         ({CCSO_BAND_NUM})"
                    ),
                ));
            }
        }
    }
}

/// Emits the Annex A.4 static level-limit diagnostics for a parsed frame against the
/// active sequence header's level (AV2 v1.0.0 Annex A.4 static conformance block,
/// mirror lines 615-629):
///
/// - `annex-a/frame-size-exceeds-level` (error): `FrameWidth * FrameHeight >
///   MaxPicSize` (line 618), `FrameWidth > MaxHSize` (line 619), or
///   `FrameHeight > MaxVSize` (line 620).
/// - `annex-a/frame-size-below-minimum` (error): `FrameWidth < 16` (line 628) or
///   `FrameHeight < 16` (line 629).
/// - `annex-a/tile-count-exceeds-level` (error): `NumTiles > MaxTiles` (line 621) or
///   `TileCols > MaxTileCols` (line 622). `NumTiles = TileCols * TileRows` and
///   `TileCols` come from the parsed `tile_info()`.
///
/// All of these are inside the "When the mapped level ID, LevelIdx is contained in the
/// tables above" block (mirror lines 615-616), so they apply only when `seq_level_idx`
/// maps to a defined level (`0..=21`). [`level_limits`] returns `None` for the
/// Maximum-parameters level 31 ("there are no level-based constraints", mirror lines
/// 659-660) and for the reserved indices `22..=30`, which disables every check here —
/// the minimum-dimension `>= 16` rule included (it lives in the same gated block).
fn frame_annex_a_level_checks(
    core: &FrameHeaderCore,
    active_sequence: &SequenceHeader,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    let level_idx = active_sequence.general.seq_level_idx.get();
    // Annex A.4: level 31 and reserved levels are not in Tables A.8/A.9, so no
    // level-limit constraint binds. A bounds-checked lookup (no indexing panic).
    let Some(limits) = level_limits(level_idx) else {
        return;
    };

    if let Some(frame_size) = core.frame_size {
        let width = frame_size.width;
        let height = frame_size.height;
        let pic_size = u64::from(width) * u64::from(height);

        // Annex A.4 line 618: FrameWidth * FrameHeight <= MaxPicSize.
        if pic_size > limits.max_pic_size {
            report.push(
                Diagnostic::error(
                    "annex-a/frame-size-exceeds-level",
                    format!(
                        "FrameWidth * FrameHeight ({width} * {height} = {pic_size}) exceeds \
                         MaxPicSize {} for seq_level_idx {level_idx}",
                        limits.max_pic_size
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
        // Annex A.4 line 619: FrameWidth <= MaxHSize.
        if width > limits.max_h_v_size {
            report.push(
                Diagnostic::error(
                    "annex-a/frame-size-exceeds-level",
                    format!(
                        "FrameWidth {width} exceeds MaxHSize {} for seq_level_idx {level_idx}",
                        limits.max_h_v_size
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
        // Annex A.4 line 620: FrameHeight <= MaxVSize (same shared column value).
        if height > limits.max_h_v_size {
            report.push(
                Diagnostic::error(
                    "annex-a/frame-size-exceeds-level",
                    format!(
                        "FrameHeight {height} exceeds MaxVSize {} for seq_level_idx {level_idx}",
                        limits.max_h_v_size
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
        // Annex A.4 lines 628-629: FrameWidth >= 16 and FrameHeight >= 16.
        if width < MIN_FRAME_DIMENSION || height < MIN_FRAME_DIMENSION {
            report.push(
                Diagnostic::error(
                    "annex-a/frame-size-below-minimum",
                    format!(
                        "FrameWidth {width} / FrameHeight {height} must both be at least \
                         {MIN_FRAME_DIMENSION} for seq_level_idx {level_idx}"
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
    }

    if let Some(tile_info) = core.tile_info.as_ref() {
        let tile_cols = tile_info.tile_cols;
        let num_tiles = u64::from(tile_cols) * u64::from(tile_info.tile_rows);
        // Annex A.4 line 621: NumTiles <= MaxTiles.
        if num_tiles > u64::from(limits.max_tiles) {
            report.push(
                Diagnostic::error(
                    "annex-a/tile-count-exceeds-level",
                    format!(
                        "NumTiles {num_tiles} (TileCols {tile_cols} * TileRows {}) exceeds \
                         MaxTiles {} for seq_level_idx {level_idx}",
                        tile_info.tile_rows, limits.max_tiles
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
        // Annex A.4 line 622: TileCols <= MaxTileCols.
        if tile_cols > limits.max_tile_cols {
            report.push(
                Diagnostic::error(
                    "annex-a/tile-count-exceeds-level",
                    format!(
                        "TileCols {tile_cols} exceeds MaxTileCols {} for seq_level_idx \
                         {level_idx}",
                        limits.max_tile_cols
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }

        // TODO(spec: AV2-A-LEVELS-TIERS): the per-tile constraints in the same Annex A.4
        // gated block (mirror lines 623-627) are not checked yet: TileWidth <=
        // Tile_Width_Scaling_Factor[seq_tier][LevelIdx] * MAX_TILE_WIDTH / 4, TileWidth
        // >= 64 for non-rightmost tiles, and TileWidth * TileHeight <=
        // Tile_Area_Scaling_Factor[seq_tier][LevelIdx] * 4096 * 2304 / 4. They need the
        // per-tile layout geometry and the tier-dependent scaling-factor tables
        // (currently private to splot-core's tile parser, which already bounds tile
        // sizing at parse via the same tables).
    }
}

/// Emits the OPS-carried Annex A profile/tier/level value-space diagnostics for each
/// included extended layer's `ops_seq_profile_tier_level_info()` (§ 5.11.2):
///
/// - `annex-a/profile-reserved` (error, Annex A.2 Table A.1, mirror line 85) when
///   `ops_seq_profile_idc` is in the reserved range `5..=30`; it conforms to no defined
///   profile, so it is as non-conformant as a reserved `seq_profile_idc`. Annex A maps
///   the OPS-derived profile id onto Table A.1 per sub-bitstream (§ 6.10.4, mirror lines
///   443-451), and the OPS PTL carries `ops_seq_profile_idc` per included extended layer
///   (§ 5.11.2).
/// - `annex-a/level-reserved` (error, Annex A.4 Table A.7, mirror line 321) when
///   `ops_level_idx` is in the reserved range `22..=30`; it maps to no defined level,
///   so it is as non-conformant as a reserved `seq_level_idx`.
/// - `annex-a/high-tier-below-4-0` (warning, Annex A.4 Table A.9 NOTE, mirror lines
///   436-437) when `ops_tier_flag == 1` (High) with `ops_level_idx < 4` (level 4.0).
///   Warning, not error: the only spec statement is the informative Table A.9 NOTE
///   ("seq_tier equal to 1 can only be signaled for level 4.0 and above") plus the
///   undefined HighMbps/HighCR cells below 4.0, so error severity would overclaim a
///   non-normative source. This is the *reachable* high-tier-below-4.0 arm: a
///   sub-bitstream's `seq_tier`/`seq_level_idx` "may be derived from the corresponding
///   ops_tier_flag and ops_level_idx values signaled in the operating_point_set_obu()"
///   (mirror lines 443-451), and the OPS PTL syntax signals both `ops_level_idx` and
///   `ops_tier_flag` unconditionally (§ 5.11.2) — unlike the sequence-header arm, where
///   `seq_tier` is only signaled for `seq_level_idx > 3` and the warning is
///   syntax-unreachable.
///
/// Annex A applies its level/tier constraints per sub-bitstream using the OPS-derived
/// `ops_tier_flag` / `ops_level_idx` values (mirror lines 443-451). Both values live in
/// each included extended layer's `ops_seq_profile_tier_level_info()`
/// ([`OpsSeqProfileTierLevelInfo::level_idx`] / [`OpsSeqProfileTierLevelInfo::tier_flag`],
/// § 5.11.2). The aggregate `ops_aggregate_level_idx` (§ 5.11.1) is a separate value
/// space tracked by `AV2-5.11.2-OPS-SEQ-PTL-INFO` and is not flagged here. Anchored at
/// the OPS OBU.
fn check_ops_level_tier_value_space(
    obu: &ObuEnvelope<'_>,
    ops: &OperatingPointSet,
    report: &mut ValidationReport,
) {
    // TODO(spec: AV2-A-LEVELS-TIERS): this checks only the OPS-carried *value space*
    // (reserved ops_level_idx / high-tier-below-4.0). § 6.10.4
    // (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-4) additionally
    // requires the operating point's bitstream to satisfy the Annex A.4 level limits
    // (frame size, tile geometry) with seq_level_idx set to ops_level_idx — i.e. the
    // static level-limit checks now run only against the activated seq_level_idx must
    // *also* run against each OPS-advertised ops_level_idx. That needs an
    // operating-point-to-frame mapping (which frames belong to which operating point)
    // the validator does not model yet, so the planned `annex-a/frame-exceeds-ops-level`
    // diagnostic is backlogged (see the Planned diagnostics backlog in
    // docs/VALIDATOR-ROADMAP.md, blocked on operating-point frame mapping).
    for payload in &ops.payloads {
        for entry in &payload.xlayer_entries {
            let Some(ptl) = entry.ptl_info.as_ref() else {
                continue;
            };
            // Annex A.2 Table A.1: a reserved ops_seq_profile_idc (5-30) conforms to no
            // defined profile. Annex A applies its profile constraints per sub-bitstream
            // using the OPS-derived profile id (§ 6.10.4, mirror lines 443-451).
            if is_reserved_profile(ptl.seq_profile_idc) {
                report.push(
                    Diagnostic::error(
                        "annex-a/profile-reserved",
                        format!(
                            "ops_seq_profile_idc {} for extended layer {} in OPS {} operating \
                             point {} is reserved (5..=30); it conforms to no AV2 profile defined \
                             in this version of the specification",
                            ptl.seq_profile_idc,
                            entry.xlayer_id.get(),
                            ops.ops_id,
                            payload.index
                        ),
                    )
                    .with_spec_section("A.2")
                    .with_byte_offset(obu.offset),
                );
            }
            if is_reserved_level(ptl.level_idx) {
                report.push(
                    Diagnostic::error(
                        "annex-a/level-reserved",
                        format!(
                            "ops_level_idx {} for extended layer {} in OPS {} operating point {} \
                             is reserved (22..=30); it maps to no AV2 level defined in this \
                             version of the specification",
                            ptl.level_idx,
                            entry.xlayer_id.get(),
                            ops.ops_id,
                            payload.index
                        ),
                    )
                    .with_spec_section("A.4")
                    .with_byte_offset(obu.offset),
                );
            }
            // Annex A.4 Table A.9 NOTE (informative): a High tier (ops_tier_flag == 1)
            // can only be signaled for level 4.0 (LevelIdx 4) and above. Unlike the
            // sequence header (where seq_tier is gated on seq_level_idx > 3), the OPS PTL
            // syntax carries ops_tier_flag unconditionally, so this is a reachable case.
            if ptl.tier_flag && ptl.level_idx < HIGH_TIER_MIN_LEVEL_IDX {
                report.push(
                    Diagnostic::warning(
                        "annex-a/high-tier-below-4-0",
                        format!(
                            "ops_tier_flag is High (1) with ops_level_idx {} below level 4.0 \
                             (LevelIdx 4) for extended layer {} in OPS {} operating point {}; the \
                             Table A.9 NOTE states High tier can only be signaled for level 4.0 \
                             and above (advisory: the source is an informative NOTE)",
                            ptl.level_idx,
                            entry.xlayer_id.get(),
                            ops.ops_id,
                            payload.index
                        ),
                    )
                    .with_spec_section("A.4")
                    .with_byte_offset(obu.offset),
                );
            }
        }
    }
}

/// Emits the locally decidable § 6.17.6.2 custom-QM plane-count diagnostics for a
/// parsed frame `setup_qm_params()` (AV2 v1.0.0 § 6.17.6.2,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-6-2`): each
/// `qm_y[i]` / `qm_u[i]` / `qm_v[i]` less than `NUM_CUSTOM_QMS` references a custom
/// quantizer-matrix slot whose `QmNumPlanes` must equal the active sequence's
/// `NumPlanes`.
///
/// Only slots with recorded quantizer-matrix OBU state are checked: a referenced
/// slot with no available record is silent here (HLS availability is owned by the
/// deferred § 7.3.8 reference checks), never a guessed false positive.
fn frame_qm_reference_checks(
    setup_qm: &SetupQmParams,
    active_sequence: &SequenceHeader,
    qm_state: &QuantizerMatrixState,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    // The qm_y/qm_u/qm_v syntax (and its conformance bullets) exists only when
    // using_qmatrix == 1 (AV2 § 5.18.6.2).
    if !setup_qm.using_qmatrix {
        return;
    }
    // AV2 § 6.4.1: NumPlanes = Monochrome ? 1 : 3.
    let num_planes: u8 = if active_sequence.general.chroma_format_idc.is_monochrome() {
        1
    } else {
        3
    };
    // TODO(spec: AV2-5.18.6-QUANTIZATION): the § 6.17.6.2 layer-dependency
    // constraints (MLayerDependencyMap[obu_mlayer_id][QmMLayerId[...]] == 1 and the
    // TLayerDependencyMap analogue) are deferred rather than fabricated: the
    // sequence-header model now exposes the § 5.4.1 dependency maps (consumed by
    // the § 6.10.7 / § 6.8.9 / § 7.3.8.7 agreement checks), but the QM-side check
    // also needs the defining QM OBU's layer identity threaded through the
    // availability state and is not implemented yet.
    let qm_num = usize::from(setup_qm.pic_qm_num_minus_1) + 1;
    // Distinct referenced custom slots: qm_uv_same_as_y / shared-UV copies and
    // repeated levels across the qmNum sets reference the same slot, which violates
    // (or satisfies) § 6.17.6.2 once, not once per syntax element.
    let mut referenced = [false; NUM_CUSTOM_QMS];
    for set in setup_qm.levels.iter().take(qm_num) {
        // qm_y[i] is always present; qm_u[i] / qm_v[i] exist only when NumPlanes > 1
        // (AV2 § 5.18.6.2) — the parsed zeroed placeholders for a monochrome
        // sequence are not bitstream references.
        if let Some(slot) = referenced.get_mut(usize::from(set.qm_y)) {
            *slot = true;
        }
        if num_planes > 1 {
            if let Some(slot) = referenced.get_mut(usize::from(set.qm_u)) {
                *slot = true;
            }
            if let Some(slot) = referenced.get_mut(usize::from(set.qm_v)) {
                *slot = true;
            }
        }
    }
    for (level, _) in referenced
        .iter()
        .enumerate()
        .filter(|(_, referenced)| **referenced)
    {
        // Missing per-slot state stays silent (no false positive when no quantizer
        // matrix OBU has defined or reset this slot).
        let Some(record) = qm_state.available[level] else {
            continue;
        };
        if record.num_planes != num_planes {
            report.push(frame_header_error(
                "frame-header/qm-plane-count-mismatch",
                "6.17.6.2",
                obu,
                format!(
                    "setup_qm_params() references custom quantizer matrix level {level}, whose \
                     recorded QmNumPlanes {} differs from the active sequence's NumPlanes \
                     {num_planes}",
                    record.num_planes
                ),
            ));
        }
    }
}

/// Emits the locally-decidable § 6.17.10.1 / § 7.3.8.8 film-grain *availability*
/// diagnostic for a parsed `film_grain_config()`: when `apply_grain == 1`, the referenced
/// `fgm_id` slot must have a received film-grain model (`FilmGrainPresent[ fgm_id ] == 1`).
///
/// **Scope and under-reporting (zero-false-positive discipline, AGENTS.md § 7).** This
/// covers ONLY the `FilmGrainPresent[ fgm_id ] == 1` requirement of § 6.17.10.1, and only
/// the in-band-availability half:
///
/// - **External means.** § 7.3.8.8 allows the model to be available "by provision through
///   external means". [`ExternalHlsSet`](crate::options::ExternalHlsSet) cannot express
///   film-grain OBUs (only sequence headers and operating point sets), so under any
///   `ExternalHlsMode::Provided` the model MAY be external without being listed — exactly
///   the inexpressible-kind case the blanket "any Provided suppresses" policy covers. The
///   check therefore fires only under `ExternalHlsMode::Disabled`.
/// - **Random-access-point visibility (§ 7.3.8.1).** A model available only from an earlier
///   position is unavailable at a later random access point that drops it. `available[]` is
///   monotonic (never reset at a random access point), so this check OVER-approximates
///   presence and silently UNDER-reports that random-access-point-unavailability direction.
///   That is a named residual on AV2-7.3.8-HLS-AVAILABILITY (no random-access-point replay
///   for film-grain references yet), not a false positive: the linear absence test can only
///   miss findings, never invent them. The companion § 6.17.10.1 layer-dependency
///   constraints (FgmTLayerId / FgmMLayerId / FgmChromaIdc) also remain a residual.
///
/// A `None`-for-the-slot under `Disabled` is therefore decidable and sound: no in-band film
/// grain OBU ever set the slot before this frame and no external provision is possible.
fn frame_film_grain_reference_checks(
    film_grain: &splot_core::headers::frame::FilmGrainConfig,
    film_grain_state: &FilmGrainState,
    options: &ValidationOptions,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    // The fgm_id reference (and its § 6.17.10.1 requirement) exists only when apply_grain.
    if !film_grain.apply_grain {
        return;
    }
    // Film grain OBUs cannot be expressed by ExternalHlsSet, so under any Provided mode the
    // referenced model MAY be supplied externally without being listed — suppress to avoid a
    // false positive. Only the external-disabled case is decidable from the bitstream alone.
    if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
        return;
    }
    let Some(fgm_id) = film_grain.fgm_id else {
        return;
    };
    let slot = usize::from(fgm_id);
    // A slot outside the modeled range cannot be matched against availability state; the
    // fgm_id field is f(3) (0..=7), so this never trips for a parsed config, but guard it.
    let Some(record) = film_grain_state.available.get(slot) else {
        return;
    };
    if record.is_none() {
        report.push(frame_header_error(
            "frame-header/film-grain-model-unavailable",
            "6.17.10.1",
            obu,
            format!(
                "film_grain_config() has apply_grain == 1 and references fgm_id {fgm_id}, but no \
                 film grain OBU has set FilmGrainPresent[{fgm_id}] == 1 (no received model for \
                 that slot)"
            ),
        ));
    }
}

/// Parses a non-first tile group's `frame_header_copy()` region and compares it
/// bit-for-bit against the recorded first header (AV2 § 5.18.1 / § 6.17.1).
///
/// `obu` is the non-first tile-group OBU; `recorded` is its coded frame's first tile
/// group's recorded header bits. The function re-reads the `tile_group_obu()` prefix
/// (`is_first_tile_group`, `frame_header_present_flag`), positions at the copy region, and:
///
/// - emits `frame-header/copy-bits-mismatch` (§ 6.17.1) when a copied `header_bit[i]`
///   differs from the first header's bit at offset `i` — the conformance requirement that
///   the copy be bit-identical — anchored at the precise byte+bit of the offending
///   `header_bit` (the copy-region start translated through `mismatch_bit`), not the OBU
///   header;
/// - emits `frame-header/copy-bits-truncated` (§ 5.18.1 / § 6.2.1) when the payload ends
///   before all `NumFrameHeaderBits` copy bits could be read.
///
/// It is silent when the copy matches, and a no-op when the prefix is not the expected
/// non-first-with-header shape (a flag/EOF the caller's segmenter has already judged).
fn check_frame_header_copy(
    obu: &ObuEnvelope<'_>,
    recorded: &RecordedFrameHeaderBits,
    report: &mut ValidationReport,
) {
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    // tile_group_obu(): is_first_tile_group must be 0 here (the segmenter reported a
    // continuation), and frame_header_present_flag is then read. A non-first tile group with
    // frame_header_present_flag == 0 carries no copy (nothing to check). Any read failure
    // leaves the copy unparsed (the payload is too short even for the prefix flags).
    let Ok(is_first) = reader.read_bit() else {
        return;
    };
    if is_first != 0 {
        // The bit disagrees with the segmenter's continuation classification (it was read
        // there too); make no copy judgment rather than guess.
        return;
    }
    let Ok(frame_header_present) = reader.read_bit() else {
        return;
    };
    if frame_header_present == 0 {
        // frame_header_present_flag == 0: no frame_header_copy() in this tile group.
        return;
    }

    // The copy region begins HERE, after the two tile_group_obu() prefix bits
    // (is_first_tile_group + frame_header_present_flag). Capture its start position so a
    // mismatch can be anchored at the exact byte+bit of the offending header_bit, rather than
    // at the OBU header (§ 6.17.1 reports a per-bit conformance requirement). `mismatch_bit`
    // is zero-based from this copy-region start (the two prefix bits are excluded), so the
    // offending bit sits `bit_offset_in_byte + mismatch_bit` bits past `start_byte`.
    let start_byte = reader.byte_offset();
    let start_bit = u64::from(reader.bit_offset().get());

    match parse_frame_header_copy(&mut reader, recorded) {
        FrameHeaderCopyOutcome::Matches => {}
        FrameHeaderCopyOutcome::Mismatch { mismatch_bit } => {
            // Translate the copy-region start (2 prefix bits already consumed) + mismatch_bit
            // into the OBU-payload byte offset and MSB-first bit-within-byte of the offending
            // header_bit. `start_bit` is 0..=7 and `mismatch_bit < NumFrameHeaderBits`, so the
            // sum is bounded; the byte advance is its whole-byte part.
            let absolute_bit = start_bit.saturating_add(mismatch_bit);
            let mismatch_byte = start_byte.saturating_add(absolute_bit / 8);
            // `absolute_bit % 8` is 0..=7 by construction, so `try_new` always succeeds; fall
            // back to the byte-aligned position rather than panic if it ever did not.
            let mismatch_bit_in_byte =
                BitOffset::try_new((absolute_bit % 8) as u8).unwrap_or(BitOffset::from_bits(0));
            report.push(
                Diagnostic::error(
                    "frame-header/copy-bits-mismatch",
                    format!(
                        "frame_header_copy() differs from the first tile group's frame header: \
                         header_bit[{mismatch_bit}] is not equal to the bit at offset \
                         {mismatch_bit} of the first frame header (NumFrameHeaderBits == {}); \
                         the differing bit is at byte {mismatch_byte}, bit {mismatch_bit_in_byte} \
                         (MSB-first) of the OBU payload",
                        recorded.num_frame_header_bits()
                    ),
                )
                .with_spec_section("6.17.1")
                .with_byte_offset(mismatch_byte)
                .with_bit_offset(mismatch_bit_in_byte),
            );
        }
        FrameHeaderCopyOutcome::Truncated { available_bits } => {
            report.push(frame_header_error(
                "frame-header/copy-bits-truncated",
                "6.2.1",
                obu,
                format!(
                    "the OBU payload ends inside frame_header_copy() after {available_bits} of \
                     {} header_bit f(1) reads; frame_header( isFirst == 0 ) must contain exactly \
                     NumFrameHeaderBits copied bits (§ 5.18.1), read from the § 6.2.1 OBU payload",
                    recorded.num_frame_header_bits()
                ),
            ));
        }
        // `FrameHeaderCopyOutcome` is `#[non_exhaustive]`; a future outcome variant with no
        // established conformance meaning is silent rather than guessed (zero false positives).
        _ => {}
    }
}

/// Builds a frame-header reference diagnostic located at `obu`.
fn frame_header_error(
    rule_id: &'static str,
    spec_section: &'static str,
    obu: &ObuEnvelope<'_>,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section(spec_section)
        .with_byte_offset(obu.offset)
}

/// Builds the `hls/unavailable-multi-frame-header` diagnostic (AV2 § 7.3.8.7). Only
/// emitted under the default (external-disabled) options; external multi-frame
/// headers are not modeled, so under `ExternalHlsMode::Provided` the reference is left
/// unresolved without a hard error (see `resolve_frame_header_reference`).
fn frame_header_unavailable_mfh(cur_mfh_id: MfhId, obu: &ObuEnvelope<'_>) -> Diagnostic {
    Diagnostic::error(
        "hls/unavailable-multi-frame-header",
        format!(
            "frame header references cur_mfh_id {}, but no multi-frame header with that id is \
             available in-band (external HLS is disabled)",
            cur_mfh_id.get()
        ),
    )
    .with_spec_section("7.3.8.7")
    .with_byte_offset(obu.offset)
}

/// Builds the advisory `hls/external-hls-disabled` warning (AV2 § 7.3.8.1) for a
/// sequence-header reference that is unavailable in-band under the default
/// (external-disabled) options.
fn external_hls_disabled_advisory(seq_header_id: u32, obu: &ObuEnvelope<'_>) -> Diagnostic {
    // The finding assumes no external HLS. If the referenced sequence header is
    // supplied out-of-band, the caller can declare it via ValidationOptions to refine
    // the check (AV2 § 7.3.8.1).
    Diagnostic::warning(
        "hls/external-hls-disabled",
        format!(
            "sequence header {seq_header_id} is not available in-band and external HLS is \
             disabled; supply it via ValidationOptions if it is provided through external means"
        ),
    )
    .with_spec_section("7.3.8.1")
    .with_byte_offset(obu.offset)
}

#[cfg(test)]
mod tests {
    use splot_core::bitio::BitReader;
    use splot_core::headers::sequence::{MLayerDependencyMap, TLayerDependencyMap};
    use splot_core::hls::parse_msdo;
    use splot_core::span::ByteOffset;
    use splot_core::types::{EmbeddedLayerId, TemporalLayerId};

    use super::{
        CmvsState, CmvsTracker, MsdoObservation, MsdoObserver, ValidationReport,
        mlayer_closure_violation, tlayer_closure_violation,
    };

    /// Builds the synthetic `multistream_decoder_operation_obu()` payload bytes with the
    /// given § 7.3.2 condition-2 key fields. `num_streams_minus_2` drives the
    /// per-substream loop; the substream entries are filled with zeros (they are not
    /// § 7.3.2 key fields). The bytes are parsed by [`parse_test_msdo`].
    fn msdo_bytes(profile_idc: u8, level_idx: u8, tier: u8, num_streams_minus_2: u8) -> Vec<u8> {
        msdo_bytes_uneven(profile_idc, level_idx, tier, num_streams_minus_2, None)
    }

    /// Like [`msdo_bytes`] but lets the caller set `multistream_even_allocation_flag`
    /// and the `multistream_large_picture_idc` carried when allocation is not even.
    fn msdo_bytes_uneven(
        profile_idc: u8,
        level_idx: u8,
        tier: u8,
        num_streams_minus_2: u8,
        large_picture_idc: Option<u8>,
    ) -> Vec<u8> {
        let mut bits = MsdoBits::default();
        bits.f(u32::from(num_streams_minus_2), 3);
        bits.f(u32::from(profile_idc), 5);
        bits.f(u32::from(level_idx), 5);
        bits.f(u32::from(tier), 1);
        match large_picture_idc {
            None => bits.f(1, 1), // multistream_even_allocation_flag = 1
            Some(idc) => {
                bits.f(0, 1); // multistream_even_allocation_flag = 0
                bits.f(u32::from(idc), 3); // multistream_large_picture_idc
            }
        }
        for _ in 0..(u32::from(num_streams_minus_2) + 2) {
            bits.f(0, 5); // sub_xlayer_id
            bits.f(0, 5); // sub_stream_max_profile
            bits.f(0, 5); // sub_stream_max_level
            bits.f(0, 1); // sub_stream_max_tier
        }
        bits.f(0, 1); // multistream_doh_constraint_flag
        bits.into_bytes()
    }

    /// Feeds synthetic MSDO payload `bytes` to a [`MsdoObserver`] and returns the
    /// observation. Parsing is asserted to succeed (`unwrap`/`expect`/`panic` are
    /// denied workspace-wide); the `None` arm returns a deterministic sentinel that the
    /// observer treats as an ordinary observation, so a builder bug fails the test via
    /// the assertion rather than panicking.
    fn observe_test_msdo(observer: &mut MsdoObserver, bytes: &[u8]) -> MsdoObservation {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        let parsed = parse_msdo(&mut reader).ok();
        assert!(parsed.is_some(), "synthetic MSDO must parse");
        match parsed {
            Some(msdo) => observer.observe(&msdo),
            None => MsdoObservation::Unchanged,
        }
    }

    /// Minimal MSB-first bit writer for the MSDO test payloads.
    #[derive(Default)]
    struct MsdoBits {
        bits: Vec<u8>,
    }

    impl MsdoBits {
        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bits.push(((value >> shift) & 1) as u8);
            }
        }

        fn into_bytes(mut self) -> Vec<u8> {
            // `parse_msdo` reads exactly the signalled fields; pad to a byte boundary so
            // the backing slice is well-formed.
            while !self.bits.len().is_multiple_of(8) {
                self.bits.push(0);
            }
            self.bits
                .chunks(8)
                .map(|chunk| {
                    chunk
                        .iter()
                        .enumerate()
                        .fold(0u8, |byte, (i, bit)| byte | (*bit << (7 - i)))
                })
                .collect()
        }
    }

    #[test]
    fn msdo_observer_reports_first_then_unchanged() {
        let mut observer = MsdoObserver::default();
        assert_eq!(
            observe_test_msdo(&mut observer, &msdo_bytes(1, 2, 0, 1)),
            MsdoObservation::First
        );
        // An identical MSDO is not a § 7.3.2 condition-2 change.
        assert_eq!(
            observe_test_msdo(&mut observer, &msdo_bytes(1, 2, 0, 1)),
            MsdoObservation::Unchanged
        );
    }

    #[test]
    fn msdo_observer_detects_each_condition_two_key_field_change() {
        // Each of the six § 7.3.2 condition-2 key fields, changed in isolation against a
        // fixed baseline, must be reported as a Changed observation.
        let baseline = msdo_bytes(1, 2, 0, 1);
        let changes = [
            msdo_bytes(2, 2, 0, 1),                 // multistream_profile_idc
            msdo_bytes(1, 3, 0, 1),                 // multistream_level_idx
            msdo_bytes(1, 2, 1, 1),                 // multistream_tier
            msdo_bytes(1, 2, 0, 2),                 // num_streams_minus_2
            msdo_bytes_uneven(1, 2, 0, 1, Some(0)), // multistream_even_allocation_flag
        ];
        for changed in &changes {
            let mut observer = MsdoObserver::default();
            assert_eq!(
                observe_test_msdo(&mut observer, &baseline),
                MsdoObservation::First
            );
            assert_eq!(
                observe_test_msdo(&mut observer, changed),
                MsdoObservation::Changed,
                "expected a key-field change to be detected"
            );
        }
        // multistream_large_picture_idc (only present under uneven allocation).
        let mut observer = MsdoObserver::default();
        assert_eq!(
            observe_test_msdo(&mut observer, &msdo_bytes_uneven(1, 2, 0, 1, Some(1))),
            MsdoObservation::First
        );
        assert_eq!(
            observe_test_msdo(&mut observer, &msdo_bytes_uneven(1, 2, 0, 1, Some(2))),
            MsdoObservation::Changed
        );
    }

    #[test]
    fn msdo_observer_ignores_non_key_field_changes() {
        // The doh-constraint flag and the substream entries are not § 7.3.2 key fields;
        // two MSDOs with the same key fields stay Unchanged.
        let mut observer = MsdoObserver::default();
        assert_eq!(
            observe_test_msdo(&mut observer, &msdo_bytes(0, 0, 0, 0)),
            MsdoObservation::First
        );
        assert_eq!(
            observe_test_msdo(&mut observer, &msdo_bytes(0, 0, 0, 0)),
            MsdoObservation::Unchanged
        );
    }

    /// Drives a [`CmvsTracker`] through one temporal unit with the given facts and
    /// returns the resulting state. `clk` toggles the CLK-present fact; `msdo` records
    /// an MSDO observation; `global_lcr` toggles a global-LCR-present fact.
    fn cmvs_after_tu(
        tracker: &mut CmvsTracker,
        clk: bool,
        msdo_obs: Option<MsdoObservation>,
        global_lcr: bool,
    ) -> CmvsState {
        if clk {
            tracker.note_clk();
        }
        if let Some(observation) = msdo_obs {
            tracker.note_msdo(observation);
        }
        if global_lcr {
            tracker.note_global_lcr_present();
        }
        let mut report = ValidationReport::default();
        // These CMVS-state unit tests do not exercise the CMVS-window-start bookkeeping, so
        // a fixed `cvs_generation` of 0 suffices.
        tracker.complete_temporal_unit(0, &mut report);
        tracker.state()
    }

    #[test]
    fn cmvs_starts_outside() {
        let tracker = CmvsTracker::default();
        assert_eq!(tracker.state(), CmvsState::Outside);
    }

    #[test]
    fn cmvs_begin_condition_1_clk_plus_msdo_enters_inside() {
        // § 7.3.2 begin condition 1: no CMVS active + CLK temporal unit + MSDO present.
        let mut tracker = CmvsTracker::default();
        let state = cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false);
        assert_eq!(state, CmvsState::Inside);
    }

    #[test]
    fn cmvs_begin_condition_2_changed_msdo_keeps_inside() {
        // § 7.3.2 begin condition 2: active CMVS + CLK + MSDO with changed key fields
        // begins a new CMVS (still Inside). An unchanged MSDO leaves the CMVS active.
        let mut tracker = CmvsTracker::default();
        assert_eq!(
            cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
            CmvsState::Inside
        );
        assert_eq!(
            cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::Changed), false),
            CmvsState::Inside
        );
        assert_eq!(
            cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::Unchanged), false),
            CmvsState::Inside
        );
    }

    #[test]
    fn cmvs_begin_condition_3_global_lcr_only_is_unknown() {
        // § 7.3.2 begin condition 3 needs an *activated* global LCR, which is not
        // modeled: a CLK temporal unit with a global LCR present but no MSDO is routed
        // to Unknown rather than guessed Inside/Outside.
        let mut tracker = CmvsTracker::default();
        let state = cmvs_after_tu(&mut tracker, true, None, true);
        assert_eq!(state, CmvsState::Unknown);
    }

    #[test]
    fn cmvs_end_condition_2_clk_without_msdo_exits_inside() {
        // § 7.3.2 end condition 2: a CLK temporal unit (begins a new CVS, § 7.3.6) with
        // no MSDO and no global LCR ends the CMVS.
        let mut tracker = CmvsTracker::default();
        assert_eq!(
            cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
            CmvsState::Inside
        );
        let state = cmvs_after_tu(&mut tracker, true, None, false);
        assert_eq!(state, CmvsState::Outside);
    }

    #[test]
    fn cmvs_end_condition_2_with_global_lcr_is_unknown() {
        // Inside + a CLK temporal unit without an MSDO but *with* a global LCR present:
        // whether the global LCR is activated (and so whether the CMVS really ends) is
        // not modeled, so the ambiguous transition routes to Unknown.
        let mut tracker = CmvsTracker::default();
        assert_eq!(
            cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
            CmvsState::Inside
        );
        let state = cmvs_after_tu(&mut tracker, true, None, true);
        assert_eq!(state, CmvsState::Unknown);
    }

    #[test]
    fn cmvs_inside_continues_across_non_boundary_tu() {
        // Inside, then a temporal unit with no CLK: no begin condition (no CLK) and no
        // end condition (end condition 2 needs a CVS start, i.e. a CLK) — the CMVS
        // continues.
        let mut tracker = CmvsTracker::default();
        assert_eq!(
            cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
            CmvsState::Inside
        );
        let state = cmvs_after_tu(&mut tracker, false, None, false);
        assert_eq!(state, CmvsState::Inside);
    }

    #[test]
    fn cmvs_no_clk_temporal_unit_does_not_begin() {
        // § 7.3.2 begin: every begin condition requires a CLK temporal unit. An MSDO
        // with no CLK does not begin a CMVS.
        let mut tracker = CmvsTracker::default();
        let state = cmvs_after_tu(&mut tracker, false, Some(MsdoObservation::First), false);
        assert_eq!(state, CmvsState::Outside);
    }

    #[test]
    fn cmvs_unknown_is_conservative_and_persists() {
        // Once Unknown, a temporal unit that is not itself an unambiguous begin keeps
        // the tracker out of a spurious Inside/Outside. A clean begin-condition-1
        // temporal unit (CLK + MSDO) still resolves it to Inside.
        let mut tracker = CmvsTracker::default();
        assert_eq!(
            cmvs_after_tu(&mut tracker, true, None, true),
            CmvsState::Unknown
        );
        // A non-CLK temporal unit cannot begin a CMVS, so Unknown persists.
        assert_eq!(
            cmvs_after_tu(&mut tracker, false, None, false),
            CmvsState::Unknown
        );
        // A CLK + MSDO temporal unit is an unambiguous begin condition 1 (no CMVS
        // definitively active), resolving the ambiguity to Inside.
        assert_eq!(
            cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
            CmvsState::Inside
        );
    }

    /// § 5.4.1 default fill for `max_mlayer_id == 1`: `MLayerDependencyMap[1][0]`
    /// is 1, so a mask including layer 1 without layer 0 violates the closure.
    #[test]
    fn mlayer_closure_violation_reports_missing_required_dependency() {
        let m_map = MLayerDependencyMap::default_for(EmbeddedLayerId::from_bits(1));
        assert_eq!(mlayer_closure_violation(0b10, &m_map), Some((1, 0)));
    }

    #[test]
    fn mlayer_closure_violation_accepts_closed_and_independent_masks() {
        let m_map = MLayerDependencyMap::default_for(EmbeddedLayerId::from_bits(1));
        // Closed mask: every required lower layer is included.
        assert_eq!(mlayer_closure_violation(0b11, &m_map), None);
        // Layer 0 has no lower layers to require.
        assert_eq!(mlayer_closure_violation(0b01, &m_map), None);
        // Layers above max_mlayer_id have no map dependencies (out of range reads
        // false), so a high stray bit alone is not a closure violation.
        assert_eq!(mlayer_closure_violation(0b1000_0000, &m_map), None);
    }

    /// § 5.4.1 default fill for `max_tlayer_id == 1`: within embedded layer 0,
    /// `TLayerDependencyMap[0][1][0]` is 1.
    #[test]
    fn tlayer_closure_violation_reports_missing_required_dependency() {
        let t_map = TLayerDependencyMap::default_for(
            TemporalLayerId::from_bits(1),
            EmbeddedLayerId::from_bits(1),
        );
        assert_eq!(tlayer_closure_violation(0, 0b10, &t_map), Some((1, 0)));
    }

    #[test]
    fn tlayer_closure_violation_accepts_closed_masks_and_out_of_range_layers() {
        let t_map = TLayerDependencyMap::default_for(
            TemporalLayerId::from_bits(1),
            EmbeddedLayerId::from_bits(1),
        );
        assert_eq!(tlayer_closure_violation(0, 0b11, &t_map), None);
        assert_eq!(tlayer_closure_violation(0, 0b01, &t_map), None);
        // An embedded layer above max_mlayer_id has an all-false map row.
        assert_eq!(tlayer_closure_violation(5, 0b10, &t_map), None);
    }
}
