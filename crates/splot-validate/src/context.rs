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
    FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameHeaderPrefix,
    FrameReferenceStateView, FrameType, SetupQmParams, TileInfo, parse_frame_header_core,
    parse_frame_header_prefix,
};
use splot_core::headers::layer_config_record::{
    LayerConfigurationRecord, parse_layer_config_record,
};
use splot_core::headers::metadata::{
    MetadataPayload, MetadataScanType, MetadataUnit, parse_metadata_group, parse_metadata_short,
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
use splot_core::headers::tile_group::parse_tile_group_prefix;
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
    InteroperabilityPoint, MIN_FRAME_DIMENSION, interoperability_point, is_reserved_level,
    is_reserved_profile, level_limits, profile_allows_chroma,
};
use crate::diagnostic::{Diagnostic, ValidationReport};
use crate::metadata_lifetime::{
    ActiveMetadataUnit, LAYER_CURRENT, LAYER_GLOBAL, LAYER_VALUES, MetadataLifetimeStore,
    PersistenceMode,
};
use crate::options::{ExternalHlsMode, ValidationOptions};

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
    /// Scan-type metadata observations per coded-video-sequence scope, for the
    /// § 6.16.10 Table 6.18 consistency checks; see [`ScanTypeCvsState`]. Scoped to
    /// the coded video sequence via the [`CvsTracker`] CLK hook and flushed at the
    /// end of the bitstream (see [`ValidatorContext::finish`]).
    scan_type: ScanTypeCvsState,
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
    /// `(xlayer, seq_header_id, cvs_generation_epoch)`, so the activation-driven
    /// re-checks emit each finding once per activated header per coded video sequence
    /// rather than per OBU (Annex A.2 Table A.1 / Annex A.4 Table A.7/A.9).
    emitted_annex_a_value_space: BTreeSet<(ExtendedLayerId, SequenceHeaderId, u64)>,
    /// Byte offset of the OBU carrying each stored sequence header, keyed by
    /// `seq_header_id`, so the Annex A value-space diagnostics — emitted at activation
    /// time, which may be a frame OBU — anchor at the defining sequence-header OBU.
    sequence_header_offsets: BTreeMap<SequenceHeaderId, ByteOffset>,
    /// Per-extended-layer Annex A interoperability-point evidence accumulated within
    /// the current coded video sequence, for the Table A.4 MSDO/LCR presence checks
    /// evaluated at coded-video-sequence end; see [`AnnexAIopTracker`].
    annex_a_iop: AnnexAIopTracker,
    /// Whether external HLS is provided (`ExternalHlsMode::Provided`), captured during
    /// observation so the end-of-stream Annex A Table A.4 flush — which runs from
    /// [`Self::finish`], without access to [`ValidationOptions`] — can suppress the
    /// in-band MSDO/LCR presence checks when externally-supplied HLS makes in-band
    /// presence counting unsound (design).
    external_hls_provided: bool,
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

/// Accumulates the Annex A interoperability-point (Table A.4) evidence for the current
/// coded-(multistream-)video-sequence window, evaluated at the window's end (the start
/// of the next coded video sequence, or the end of the bitstream).
///
/// Table A.4 (mirror lines 178-201) states MSDO/LCR presence requirements for
/// interoperability points 0-2 as a function of E = "Number of Extended Layers > 1"
/// and M = "Number of Embedded Layers > 1" (Table A.3 definitions, mirror lines
/// 144-161). The window scope is the whole coded (multistream-)video-sequence: it
/// accumulates from the start of the bitstream (or the start of the previous coded video
/// sequence) and stays open across *every* temporal unit until a CLK in a *later*
/// temporal unit begins the next coded video sequence (§ 7.3.6), or the end of the
/// bitstream. Closing only at the next coded-video-sequence boundary — not at the
/// temporal unit that merely *contained* the random-access CLK — is what makes the
/// Table A.3 counts span the whole coded video sequence: a second distinct
/// `obu_xlayer_id` that first appears in a *later* temporal unit of the same coded video
/// sequence is still attributed to that sequence. Every extended layer's CLK within one
/// random-access temporal unit accumulates into the same window (a multistream coded
/// video sequence has one CLK per extended layer within the temporal unit).
/// "Presence" means an OBU of that type occurred in the window.
///
/// What is **not** modeled here (recorded as `TODO(spec: AV2-A-LEVELS-TIERS)` at the
/// call sites): the `MultiStreamDecoderMode == 1` substream level scaling (mirror lines
/// 456-523), and the Table A.3 layer-budget bound that the combination flag must be 0
/// for IOP 0/1 (mirror lines 154-158) — exceeding the IOP layer budget is a separate
/// interoperability conformance check that needs the full Table A.3 bounds.
#[derive(Debug, Default)]
struct AnnexAIopTracker {
    /// The window's accumulated evidence. `None` before the first observation of the
    /// current window (so a freshly reset window with no evidence yet flushes nothing).
    window: Option<AnnexAIopWindow>,
}

/// One coded-(multistream-)video-sequence window's accumulated Table A.4 evidence; see
/// [`AnnexAIopTracker`].
#[derive(Debug)]
struct AnnexAIopWindow {
    /// Distinct non-global `obu_xlayer_id` values observed in the window (Table A.3
    /// "Number of Extended Layers" base case, mirror lines 146-151: "the number of
    /// distinct values of obu_xlayer_id (excluding GLOBAL_XLAYER_ID)").
    distinct_xlayers: BTreeSet<ExtendedLayerId>,
    /// The largest `num_streams_minus_2 + 2` of any MSDO observed in the window, when
    /// `MultiStreamDecoderMode == 1` (Table A.3, mirror lines 148-149). An MSDO sets
    /// `MultiStreamDecoderMode == 1` (§ 5.6), so its presence selects this count.
    msdo_num_streams: Option<u32>,
    /// The largest activated global LCR `LcrMaxNumXLayerCount` (set-bit count of
    /// `lcr_xlayer_map`) observed in the window (Table A.3, mirror lines 149-150: "When
    /// a global layer configuration record is activated, this value is equal to
    /// LcrMaxNumXLayerCount").
    global_lcr_xlayer_count: Option<u32>,
    /// The maximum embedded-layer count (`seq_max_mlayer_cnt_minus_1 + 1`) across the
    /// sequence headers activated in the window (Table A.3 "Number of Embedded Layers",
    /// mirror lines 152-153).
    max_embedded_layers: u32,
    /// `true` if an `OBU_MSDO` occurred in the window.
    msdo_present: bool,
    /// `true` if a global `OBU_LAYER_CONFIGURATION_RECORD` was activated in the window.
    global_lcr_present: bool,
    /// `true` if a local `OBU_LAYER_CONFIGURATION_RECORD` was activated in the window.
    local_lcr_present: bool,
    /// The interoperability point of the profile activated in the window, when known.
    /// Derived from the first non-reserved, non-Configurable activated profile; if a
    /// later activation disagrees, the window is left with mixed IOP and the check is
    /// skipped (the multistream profile-agreement rules are out of scope here).
    iop: Option<AnnexAIopState>,
    /// Byte offset to anchor the Table A.4 diagnostic at — the latest activating OBU's
    /// offset, the location where the (now-final) layer configuration is known.
    anchor_offset: ByteOffset,
    /// The [`CvsTracker::tu_index`] of the temporal unit in which this window's coded
    /// video sequence began (the temporal unit carrying its `OBU_CLOSED_LOOP_KEY`s), or
    /// `None` for leading evidence accumulated before the first CLK. The window spans the
    /// whole coded (multistream-)video-sequence: it stays open across every temporal unit
    /// until a CLK in a *later* temporal unit begins the next coded video sequence
    /// (§ 7.3.6), which is when it is flushed (mirror Table A.3 counts span the whole
    /// coded video sequence, not just its random-access temporal unit).
    cvs_start_tu: Option<u64>,
}

impl Default for AnnexAIopWindow {
    fn default() -> Self {
        Self {
            distinct_xlayers: BTreeSet::new(),
            msdo_num_streams: None,
            global_lcr_xlayer_count: None,
            max_embedded_layers: 0,
            msdo_present: false,
            global_lcr_present: false,
            local_lcr_present: false,
            iop: None,
            anchor_offset: ByteOffset::new(0),
            cvs_start_tu: None,
        }
    }
}

/// The interoperability-point state of an Annex A IOP window: a single agreed IOP, or
/// `Mixed` when activated profiles disagree (the check is then skipped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnnexAIopState {
    /// A single agreed interoperability point.
    Single(InteroperabilityPoint),
    /// Activated profiles disagree on the interoperability point; skip the check.
    Mixed,
}

impl AnnexAIopTracker {
    fn window_mut(&mut self) -> &mut AnnexAIopWindow {
        self.window.get_or_insert_with(AnnexAIopWindow::default)
    }

    /// Records a non-global `obu_xlayer_id` seen in the window (Table A.3 extended-layer
    /// base count).
    fn note_xlayer(&mut self, xlayer: ExtendedLayerId) {
        if !xlayer.is_global() {
            self.window_mut().distinct_xlayers.insert(xlayer);
        }
    }

    /// Records that an `OBU_MSDO` with `num_streams` substreams occurred in the window
    /// (`MultiStreamDecoderMode == 1`).
    fn note_msdo(&mut self, num_streams: u32, offset: ByteOffset) {
        let window = self.window_mut();
        window.msdo_present = true;
        window.msdo_num_streams = Some(window.msdo_num_streams.unwrap_or(0).max(num_streams));
        window.anchor_offset = offset;
    }

    /// Records that a global LCR with `xlayer_count == LcrMaxNumXLayerCount` was
    /// activated in the window.
    fn note_global_lcr(&mut self, xlayer_count: u32, offset: ByteOffset) {
        let window = self.window_mut();
        window.global_lcr_present = true;
        window.global_lcr_xlayer_count = Some(
            window
                .global_lcr_xlayer_count
                .unwrap_or(0)
                .max(xlayer_count),
        );
        window.anchor_offset = offset;
    }

    /// Records that a local LCR was activated in the window.
    fn note_local_lcr(&mut self, offset: ByteOffset) {
        let window = self.window_mut();
        window.local_lcr_present = true;
        window.anchor_offset = offset;
    }

    /// Records an activated sequence header's profile (for the IOP) and embedded-layer
    /// count, anchoring the window's diagnostic at the activating OBU.
    fn note_activation(&mut self, profile_idc: u8, embedded_layers: u32, offset: ByteOffset) {
        let window = self.window_mut();
        window.max_embedded_layers = window.max_embedded_layers.max(embedded_layers);
        window.anchor_offset = offset;
        if let Some(iop) = interoperability_point(profile_idc) {
            window.iop = Some(match window.iop {
                None => AnnexAIopState::Single(iop),
                Some(AnnexAIopState::Single(existing)) if existing == iop => {
                    AnnexAIopState::Single(existing)
                }
                Some(_) => AnnexAIopState::Mixed,
            });
        }
    }

    /// Whether a CLK observed in temporal unit `tu_index` begins the *next* coded video
    /// sequence relative to the currently-open window, so the prior window must be flushed
    /// before this CLK's evidence joins a fresh window. True only when a window is open and
    /// its coded video sequence began in an *earlier* temporal unit: a CLK in the same
    /// temporal unit (a second extended layer's CLK in one multistream random-access
    /// temporal unit) joins the same window, and leading evidence with no recorded CVS
    /// start (`cvs_start_tu == None`) is absorbed by the first coded video sequence.
    fn clk_starts_new_cvs(&self, tu_index: u64) -> bool {
        matches!(
            self.window.as_ref().and_then(|w| w.cvs_start_tu),
            Some(start) if start != tu_index
        )
    }

    /// Records that the current window's coded video sequence began in temporal unit
    /// `tu_index` (the temporal unit carrying its CLK), opening the window if it does not
    /// yet exist. Idempotent within a temporal unit, and a no-op once the window's coded
    /// video sequence start is recorded (every later CLK in the same coded video sequence
    /// keeps the original start).
    fn note_cvs_start(&mut self, tu_index: u64) {
        self.window_mut().cvs_start_tu.get_or_insert(tu_index);
    }

    /// Seeds the (possibly freshly opened) window's interoperability point and
    /// embedded-layer count from the currently-active frame-confirmed sequence header when
    /// a CLK opens a new coded video sequence. The frame path only re-runs
    /// `on_sequence_activation` (and so `note_activation`) when the activated
    /// `seq_header_id` changes or is newly confirmed; a coded video sequence that reuses
    /// the same confirmed header would otherwise open a window with no interoperability
    /// point and be skipped at evaluation. Seeding here makes the window decidable from the
    /// active header carried across the coded-video-sequence boundary. A `None` profile
    /// (reserved / Configurable) leaves the IOP unset, matching `note_activation`.
    fn seed_from_active(&mut self, profile_idc: u8, embedded_layers: u32, offset: ByteOffset) {
        self.note_activation(profile_idc, embedded_layers, offset);
    }

    /// Ends the current window and returns its accumulated evidence for Table A.4
    /// evaluation, resetting the tracker for the next window. Returns `None` when no
    /// evidence was accumulated (an empty window flushes nothing).
    fn take_window(&mut self) -> Option<AnnexAIopWindow> {
        self.window.take()
    }
}

impl AnnexAIopWindow {
    /// The Table A.3 "Number of Extended Layers" for this window (mirror lines 146-151).
    ///
    /// Evaluated in the exact priority order the mirror's definition gives: a *declared*
    /// count takes precedence over the observed coded structure.
    ///
    /// 1. When `MultiStreamDecoderMode == 1` (an `OBU_MSDO` is present in the window), the
    ///    value is `num_streams_minus_2 + 2` (mirror lines 148-149) — the MSDO's declared
    ///    substream count, regardless of how many distinct `obu_xlayer_id` materialize.
    /// 2. Otherwise, when a global layer configuration record is activated, the value is
    ///    `LcrMaxNumXLayerCount` (mirror lines 149-150) — the global LCR's declared
    ///    extended-layer span (set-bit count of `lcr_xlayer_map`).
    /// 3. Otherwise, the value is the number of distinct non-global `obu_xlayer_id` values
    ///    actually present (mirror lines 150-151) — the observed coded structure, at least
    ///    1 (Table A.3: "For a coded video sequence, this value is equal to 1").
    ///
    /// Consequence for the Table A.4 *Prohibited* rows: because a present MSDO declares
    /// `num_streams >= 2`, this method returns `>= 2` whenever an MSDO is in the window, so
    /// E > 1 holds and the "MSDO Prohibited" rows (which require E == 1) are structurally
    /// unreachable in-band when an MSDO is actually present. The genuine real-world
    /// violation those rows would catch — an MSDO declaring substreams that never
    /// materialize as distinct `obu_xlayer_id` — is a declared-vs-observed reconciliation
    /// that is out of scope here.
    ///
    /// `TODO(spec: AV2-A-LEVELS-TIERS)`: the declared-vs-observed reconciliation (an MSDO
    /// `num_streams` or global-LCR `LcrMaxNumXLayerCount` larger than the distinct
    /// `obu_xlayer_id` count actually coded) is owned by the upcoming
    /// `msdo-substream-constraint-checks` change, not modeled here.
    fn extended_layers(&self) -> u32 {
        // Declared precedence, in the mirror's definition order: MSDO num_streams (when
        // MultiStreamDecoderMode == 1), then global-LCR LcrMaxNumXLayerCount, then the
        // observed distinct non-global obu_xlayer_id count (at least 1).
        if let Some(num_streams) = self.msdo_num_streams {
            return num_streams;
        }
        if let Some(global_count) = self.global_lcr_xlayer_count {
            return global_count;
        }
        (self.distinct_xlayers.len() as u32).max(1)
    }

    /// The Table A.3 "Number of Embedded Layers" for this window (mirror lines 152-153):
    /// the maximum `seq_max_mlayer_cnt_minus_1 + 1` across activated headers, at least 1.
    fn embedded_layers(&self) -> u32 {
        self.max_embedded_layers.max(1)
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

    /// Completes the temporal unit being observed, applying the § 7.3.2 begin/end
    /// conditions, then resets the per-temporal-unit facts for the next one. Called at
    /// each temporal-unit boundary and at the end of the bitstream.
    ///
    /// Provisional-`Inside` § 6.4.1 monotonic disagreements deferred during this temporal
    /// unit ([`Self::queue_provisional_monotonic`]) are resolved here against the
    /// temporal unit's final membership: emitted when the completed temporal unit is
    /// definitively [`CmvsState::Inside`], dropped when a CLK ended the CMVS
    /// ([`CmvsState::Outside`]/[`CmvsState::Unknown`], § 7.3.2 end condition 2).
    fn complete_temporal_unit(&mut self, report: &mut ValidationReport) {
        let facts = std::mem::take(&mut self.current_tu);
        self.state = self.next_state(&facts);
        let pending = std::mem::take(&mut self.pending_monotonic);
        if matches!(self.state, CmvsState::Inside) {
            for diagnostic in pending {
                report.push(diagnostic);
            }
        }
    }

    /// Computes the § 7.3.2 CMVS state after a completed temporal unit with `facts`,
    /// given the current `self.state`. Begin conditions are evaluated before end
    /// conditions because a temporal unit that begins a new CMVS is the *earliest* end
    /// of the current one (§ 7.3.2 end condition 1).
    fn next_state(&self, facts: &CmvsTuFacts) -> CmvsState {
        // AV2 § 7.3.2: "A coded multistream video sequence begins at a temporal unit
        // that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY for at least
        // one extended layer and satisfies one of the following conditions". Without a
        // CLK in the temporal unit, no begin condition can fire.
        if facts.has_clk {
            let currently_active = matches!(self.state, CmvsState::Inside);
            match facts.msdo {
                // AV2 § 7.3.2 begin condition 1: "No coded multistream video sequence is
                // currently active and an OBU with obu_type equal to OBU_MSDO is present."
                Some(_) if !currently_active => return CmvsState::Inside,
                // AV2 § 7.3.2 begin condition 2: "A coded multistream video sequence is
                // currently active, an OBU with obu_type equal to OBU_MSDO is present,
                // and the value of multistream_profile_idc, multistream_level_idx,
                // multistream_tier, num_streams_minus_2, multistream_even_allocation_flag,
                // or multistream_large_picture_idc differs from the corresponding value
                // in the previous OBU_MSDO." A changed MSDO begins a new CMVS (which is
                // still Inside); an unchanged MSDO leaves the active CMVS intact.
                Some(MsdoObservation::Changed) => return CmvsState::Inside,
                Some(MsdoObservation::First | MsdoObservation::Unchanged) => {
                    // Active CMVS (the `!currently_active` arm above already handled the
                    // inactive case for any MSDO), MSDO present but unchanged: this temporal
                    // unit neither begins a new CMVS (condition 2 needs a change) nor ends
                    // the current one (end condition 2 excludes an MSDO-accompanied CVS
                    // start), so the CMVS continues.
                    return CmvsState::Inside;
                }
                None => {
                    // AV2 § 7.3.2 begin condition 3: "No coded multistream video sequence
                    // is currently active and a global layer configuration record is
                    // activated." Exact § 7.3.8 global-LCR activation is not modeled, so a
                    // CLK temporal unit with a global LCR present but no MSDO cannot be
                    // soundly classified: it may begin a CMVS (condition 3) or not. Route
                    // to Unknown rather than guess.
                    if facts.global_lcr_present && !currently_active {
                        return CmvsState::Unknown;
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
                    return CmvsState::Unknown;
                }
                return CmvsState::Outside;
            }
            // Otherwise the active CMVS continues across this temporal unit.
            return CmvsState::Inside;
        }

        // No begin condition fired and the CMVS was not active: preserve the current
        // state (Outside or a prior Unknown that nothing resolved).
        self.state
    }
}

impl ValidatorContext {
    /// Observes one parsed OBU, updating context and emitting stateful diagnostics.
    pub(crate) fn observe_obu(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // Temporal-unit and coded-video-sequence boundary events run first: a global
        // temporal delimiter completes the previous temporal unit (flushing deferred
        // CVS-scoped diagnostics) and a CLK starts a new coded video sequence for its
        // extended layer (AV2 § 7.3.6); see observe_cvs_boundary_events.
        // Capture external-HLS state for the end-of-stream Annex A Table A.4 flush,
        // which runs from `finish` without access to the options.
        self.external_hls_provided = matches!(options.external_hls, ExternalHlsMode::Provided(_));

        // Annex A Table A.4: the IOP window spans the WHOLE coded (multistream-)video
        // sequence, so it closes at the START of the next coded video sequence — a CLK in
        // a temporal unit *later* than the one in which the open window's coded video
        // sequence began (§ 7.3.6) — not at the temporal unit that merely contained the
        // random-access CLK. (`self.cvs.tu_index` is the current temporal unit's index; a
        // global temporal delimiter has already advanced it via the prior call's
        // `observe_cvs_boundary_events`.) Flushing here, before this CLK's evidence opens a
        // fresh window, evaluates the prior coded video sequence's MSDO/LCR presence
        // requirements with every temporal unit of that sequence accounted for. Every
        // extended layer's CLK within one random-access temporal unit joins the same
        // window (a multistream coded video sequence has one CLK per extended layer in the
        // temporal unit), so a same-temporal-unit CLK does not flush.
        if obu.header.obu_type == ObuType::ClosedLoopKey
            && self.annex_a_iop.clk_starts_new_cvs(self.cvs.tu_index)
        {
            self.flush_annex_a_iop_window(options, report);
        }

        self.observe_cvs_boundary_events(obu, report);

        self.temporal_unit.observe_obu(obu, report);

        // Annex A Table A.3: count the distinct non-global obu_xlayer_id values present
        // in the current Annex A IOP window (mirror lines 146-151). Recorded for every
        // OBU after the boundary events (so a CLK's own xlayer joins the new window).
        self.annex_a_iop.note_xlayer(obu.header.extended_layer_id);

        // Annex A Table A.4: a CLK begins (or continues, for a multistream same-temporal
        // unit CLK) the current coded video sequence for its extended layer (§ 7.3.6).
        // Record the window's coded-video-sequence start temporal unit (so the next
        // sequence's CLK can detect the boundary) and seed the window's interoperability
        // point / embedded-layer count from the active frame-confirmed sequence header for
        // this extended layer. The frame path re-runs `on_sequence_activation` only when
        // the activated `seq_header_id` changes or is newly confirmed, so a coded video
        // sequence that reuses the same confirmed header would otherwise leave the window
        // with no interoperability point; seeding here keeps it decidable.
        if obu.header.obu_type == ObuType::ClosedLoopKey {
            self.annex_a_iop.note_cvs_start(self.cvs.tu_index);
            self.seed_annex_a_iop_from_active(obu.header.extended_layer_id);
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
            ObuType::Msdo => self.observe_msdo(obu),
            _ => {}
        }

        // AV2 § 6.12 / § 6.13: the quantizer-matrix "between coded frames" and
        // film-grain "coded frame unit" windows close at a coded frame, so the window
        // resets after observing each frame-bearing OBU — NOT at a temporal-unit
        // boundary, since a QM level / film-grain slot reused across a temporal
        // delimiter with no intervening frame is still a duplicate. is_frame_bearing()
        // (tile groups plus SEF / TIP / bridge) is a superset of AVM's
        // reset-before-tile-group point, so it always resets at every coded frame (no
        // false positive on a conformant stream) and can only ever drop a duplicate
        // detection across a SEF-only unit (a documented false negative).
        if is_frame_bearing(obu.header.obu_type) {
            self.reset_coded_frame_window();
        }
    }

    /// Resets the §6.12/§6.13 coded-frame windows for quantizer-matrix and film-grain
    /// state at a coded-frame boundary, and expires `NO_PERSISTENCE` metadata
    /// (AV2 § 6.16.3: "Used only for the current frame" — a unit observed before a
    /// coded frame lapses once that frame has been observed).
    fn reset_coded_frame_window(&mut self) {
        self.qm.reset_coded_frame_window();
        self.film_grain.reset_coded_frame_window();
        self.metadata.expire_no_persistence();
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
        report: &mut ValidationReport,
    ) {
        if obu.header.obu_type == ObuType::TemporalDelimiter
            && obu.header.extended_layer_id.is_global()
        {
            self.cvs.advance_temporal_unit(report);
            // AV2 § 7.3.2: the global temporal delimiter ends the just-observed
            // temporal unit, so the accumulated § 7.3.2 begin/end facts are evaluated
            // now, before the per-temporal-unit facts reset for the next unit. Any
            // deferred provisional-Inside § 6.4.1 monotonic disagreements are resolved
            // here against the completed temporal unit's final CMVS membership.
            self.cmvs.complete_temporal_unit(report);
            self.frames_seen_in_tu.clear();
            // AV2 § 7.3.7: clear the per-temporal-unit distinct-`obu_mlayer_id` sets so a
            // CLK in the next temporal unit re-attributes only that temporal unit's ids
            // to the new coded video sequence (see DistinctMlayerTracker::reset_cvs).
            self.distinct_mlayer.advance_temporal_unit();
        } else if obu.header.obu_type == ObuType::ClosedLoopKey {
            // Annex A Table A.4: this temporal unit begins a new coded video sequence. The
            // prior IOP window was already flushed in `observe_obu` when this CLK was found
            // to start a coded video sequence later than the open window's (so the window
            // spans the whole prior coded video sequence, not just its random-access
            // temporal unit); the window's coded-video-sequence start is recorded and
            // seeded there too.
            self.start_cvs_for_xlayer(obu.header.extended_layer_id, report);
            self.observe_ci_rap(obu.header.extended_layer_id);
            // AV2 § 7.3.2 / § 7.3.6: a CLK makes this temporal unit one that "contains
            // an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY for at least one
            // extended layer" (and begins a new coded video sequence for that layer).
            self.cmvs.note_clk();
        } else if obu.header.obu_type == ObuType::OpenLoopKey {
            // An OLK is NOT a § 7.3.6 CVS boundary during sequential decoding
            // (§ 7.4.4), but it IS a § 7.3.8.11 random access point that
            // re-initializes the extended layer's content interpretation
            // parameters to defaults.
            self.observe_ci_rap(obu.header.extended_layer_id);
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
    /// deferred Table 6.18 pairing diagnostics for the extended layer pair
    /// pre-epoch CI content with post-epoch pictures (or vice versa), so exactly
    /// those two rules are dropped; every other pending diagnostic (§ 6.14
    /// repeated-CI identity, § 6.4.12 timing, group consistency) is CVS-scoped
    /// and survives an OLK.
    fn observe_ci_rap(&mut self, xlayer: ExtendedLayerId) {
        self.ci_rap_started_in_tu.insert(xlayer, self.cvs.tu_index);
        self.cvs.drop_pending_for_rules(
            xlayer,
            &[
                "metadata/scan-type-ci-scan-type-mismatch",
                "metadata/scan-type-equal-picture-interval-required",
            ],
        );
    }

    /// The temporal unit at which `xlayer`'s current § 7.3.8.11
    /// content-interpretation-parameter epoch started (its most recent CLK / OLK
    /// random access point), or 0 when none has been observed.
    fn ci_rap_epoch(&self, xlayer: ExtendedLayerId) -> u64 {
        self.ci_rap_started_in_tu.get(&xlayer).copied().unwrap_or(0)
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
    /// condition-2 key fields in the stateful [`MsdoObserver`], and forwards the
    /// observation to the [`CmvsTracker`] for the temporal unit currently being
    /// observed. A parse failure is silent — the structural MSDO syntax diagnostics
    /// are owned by the stateless check (AV2-5.6-MSDO), and the CMVS tracker treats an
    /// unparsable MSDO conservatively (no MSDO observation is recorded for the temporal
    /// unit, so no MSDO-driven begin condition fires).
    fn observe_msdo(&mut self, obu: &ObuEnvelope<'_>) {
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
        // Annex A Table A.3 / Table A.4: an MSDO is present in this IOP window and sets
        // MultiStreamDecoderMode == 1, so num_streams_minus_2 + 2 is the Table A.3
        // extended-layer count for the window (mirror lines 148-149).
        // TODO(spec: AV2-A-LEVELS-TIERS): the MultiStreamDecoderMode == 1 substream
        // level scaling (mirror lines 456-523) is not modeled; only the Table A.4
        // presence requirements and the Table A.3 extended-layer count are used here.
        self.annex_a_iop.note_msdo(msdo.num_streams(), obu.offset);
        let observation = self.msdo.observe(&msdo);
        self.cmvs.note_msdo(observation);
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
    pub(crate) fn finish(&mut self, report: &mut ValidationReport) {
        self.cvs.flush_completed_tu(report);
        // AV2 § 7.3.2 end condition 3: "The end of the bitstream." The final temporal
        // unit (which has no trailing global temporal delimiter) is completed here so
        // its § 7.3.2 begin/end facts are applied exactly as at an internal boundary.
        // Any deferred provisional-Inside § 6.4.1 monotonic disagreements that never saw
        // a CLK (the temporal unit stayed inside the CMVS until the end of the bitstream)
        // are emitted here.
        self.cmvs.complete_temporal_unit(report);
        let scope_keys: Vec<ExtendedLayerId> = self.scan_type.scopes.keys().copied().collect();
        for scope_key in scope_keys {
            self.flush_scan_type_scope(scope_key, u64::MAX, report);
        }
        // Annex A Table A.4: the end of the bitstream ends the final coded
        // (multistream-)video-sequence window (AV2 § 2: a coded video sequence continues
        // "until ... the end of the bitstream"), so evaluate its MSDO/LCR presence
        // requirements. The external-HLS suppression decision was captured during
        // observation (see `external_hls_provided`).
        self.flush_annex_a_iop_window_for_finish(report);
    }

    /// Evaluates and resets the current Annex A Table A.4 IOP window using the
    /// already-captured external-HLS state, for the end-of-stream flush in
    /// [`Self::finish`] (which has no [`ValidationOptions`]). See
    /// [`Self::evaluate_annex_a_iop_window`].
    fn flush_annex_a_iop_window_for_finish(&mut self, report: &mut ValidationReport) {
        let suppressed = self.external_hls_provided;
        self.evaluate_annex_a_iop_window(suppressed, report);
    }

    /// Evaluates and resets the current Annex A Table A.4 IOP window at a coded video
    /// sequence boundary (a CLK in any extended layer), suppressing the presence checks
    /// when external HLS is provided (design: externally-supplied HLS makes in-band
    /// presence counting unsound).
    fn flush_annex_a_iop_window(
        &mut self,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let suppressed = matches!(options.external_hls, ExternalHlsMode::Provided(_));
        self.evaluate_annex_a_iop_window(suppressed, report);
    }

    /// Takes the current Annex A IOP window's accumulated evidence and emits the Table
    /// A.4 MSDO/LCR presence diagnostics for it (AV2 v1.0.0 Annex A.2 Table A.4, mirror
    /// lines 178-201). A `None` window (no evidence) or a window with no decidable single
    /// interoperability point is a no-op. When `suppressed`, the window is still taken
    /// (so the next window starts clean) but no diagnostic is emitted.
    ///
    /// The Table A.4 semantics, by IOP and the booleans `e = extended layers > 1`,
    /// `m = embedded layers > 1`:
    ///
    /// - IOP0 (mirror lines 183-185): MSDO prohibited when `!e`, required when `e`.
    /// - IOP1 (mirror lines 187-191): `!e && !m` -> MSDO prohibited; `e && !m` -> MSDO
    ///   required; `!e && m` -> MSDO prohibited and a local LCR required. (`e && m` has
    ///   no Table A.4 row — outside IOP1's Table A.3 layer budget; see the TODO below.)
    /// - IOP2 (mirror lines 193-201): `!e && !m` -> MSDO prohibited; `e && !m` -> MSDO
    ///   **or** a global LCR required (either satisfies); `!e && m` -> MSDO prohibited
    ///   and an LCR (global or local) required; `e && m` -> (MSDO **and** local LCR)
    ///   **or** a global LCR required.
    fn evaluate_annex_a_iop_window(&mut self, suppressed: bool, report: &mut ValidationReport) {
        let Some(window) = self.annex_a_iop.take_window() else {
            return;
        };
        if suppressed {
            return;
        }
        let Some(AnnexAIopState::Single(iop)) = window.iop else {
            // No decidable single interoperability point (no profile activated in-band,
            // a reserved/Configurable profile, or mixed profiles across layers): the
            // Table A.4 row is not determinable, so no diagnostic.
            return;
        };
        let e = window.extended_layers() > 1;
        let m = window.embedded_layers() > 1;
        let offset = window.anchor_offset;
        // TODO(spec: AV2-A-LEVELS-TIERS): the Table A.3 layer-budget bound (the
        // combination flag must be 0 for IOP 0/1, mirror lines 154-158) is not enforced
        // here; an IOP1 window with both e and m exceeds that budget but has no Table A.4
        // row, so Table A.4 alone makes no presence requirement for it.
        match iop {
            InteroperabilityPoint::Iop0 => {
                // Table A.4 rows 1-2 (mirror lines 183-185): embedded layers are N/A.
                self.emit_iop_msdo_requirement(e, &window, offset, report);
            }
            InteroperabilityPoint::Iop1 => {
                if !m {
                    // Rows 3-4 (mirror lines 187-189): MSDO prohibited (!e) / required (e).
                    self.emit_iop_msdo_requirement(e, &window, offset, report);
                } else if !e {
                    // Row 5 (mirror line 191): !e && m -> MSDO prohibited; local LCR
                    // required.
                    self.emit_msdo_prohibited(&window, offset, report);
                    self.emit_local_lcr_required(&window, offset, report);
                }
                // e && m: no Table A.4 row (outside IOP1's layer budget); see the TODO.
            }
            InteroperabilityPoint::Iop2 => {
                self.evaluate_iop2(e, m, &window, offset, report);
            }
        }
    }

    /// Table A.4 IOP0 rows and IOP1 `!m` rows: MSDO required when `e`, prohibited when
    /// `!e`.
    fn emit_iop_msdo_requirement(
        &self,
        e: bool,
        window: &AnnexAIopWindow,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if e {
            self.emit_msdo_required(window, offset, report);
        } else {
            self.emit_msdo_prohibited(window, offset, report);
        }
    }

    /// Table A.4 IOP2 rows (mirror lines 193-201).
    fn evaluate_iop2(
        &self,
        e: bool,
        m: bool,
        window: &AnnexAIopWindow,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        match (e, m) {
            // Row "2 N N" (mirror line 193): MSDO prohibited.
            (false, false) => self.emit_msdo_prohibited(window, offset, report),
            // Row "2 Y N" (mirror line 195): MSDO or global LCR required (either
            // satisfies); MSDO is not prohibited here.
            (true, false) => {
                if !window.msdo_present && !window.global_lcr_present {
                    report.push(annex_a_iop_error(
                        "annex-a/msdo-required-for-iop",
                        offset,
                        format!(
                            "Annex A Table A.4: interoperability point 2 with more than one \
                             extended layer ({}) requires an OBU_MSDO or a global \
                             OBU_LAYER_CONFIGURATION_RECORD, but neither is present in the coded \
                             video sequence",
                            window.extended_layers()
                        ),
                    ));
                }
            }
            // Row "2 N Y" (mirror line 197): MSDO prohibited; LCR (global or local)
            // required.
            (false, true) => {
                self.emit_msdo_prohibited(window, offset, report);
                if !window.global_lcr_present && !window.local_lcr_present {
                    report.push(annex_a_iop_error(
                        "annex-a/lcr-required-for-iop",
                        offset,
                        format!(
                            "Annex A Table A.4: interoperability point 2 with more than one \
                             embedded layer ({}) requires a global or local \
                             OBU_LAYER_CONFIGURATION_RECORD, but none is present in the coded video \
                             sequence",
                            window.embedded_layers()
                        ),
                    ));
                }
            }
            // Row "2 Y Y" (mirror lines 199-200): (MSDO and local LCR) or global LCR
            // required.
            (true, true) => {
                let satisfied =
                    (window.msdo_present && window.local_lcr_present) || window.global_lcr_present;
                if !satisfied {
                    report.push(annex_a_iop_error(
                        "annex-a/lcr-required-for-iop",
                        offset,
                        "Annex A Table A.4: interoperability point 2 with more than one extended \
                         layer and more than one embedded layer requires either an OBU_MSDO plus a \
                         local OBU_LAYER_CONFIGURATION_RECORD, or a global \
                         OBU_LAYER_CONFIGURATION_RECORD, but neither combination is present in the \
                         coded video sequence"
                            .to_owned(),
                    ));
                }
            }
        }
    }

    /// Emits `annex-a/msdo-required-for-iop` when no MSDO is present in the window.
    fn emit_msdo_required(
        &self,
        window: &AnnexAIopWindow,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if !window.msdo_present {
            report.push(annex_a_iop_error(
                "annex-a/msdo-required-for-iop",
                offset,
                format!(
                    "Annex A Table A.4: the coded video sequence has more than one extended layer \
                     ({}) but contains no OBU_MSDO, which the activated profile's interoperability \
                     point requires",
                    window.extended_layers()
                ),
            ));
        }
    }

    /// Emits `annex-a/msdo-prohibited-for-iop` when an MSDO is present in the window.
    ///
    /// Under the strict Table A.3 "Number of Extended Layers" definition (mirror lines
    /// 146-151, see [`AnnexAIopWindow::extended_layers`]), a present OBU_MSDO sets
    /// `MultiStreamDecoderMode == 1` and declares `num_streams_minus_2 + 2 >= 2` extended
    /// layers, so `e = extended_layers() > 1` is always true when `msdo_present` is true.
    /// The Table A.4 "MSDO Prohibited" rows require `e` to be false (E == 1), so every
    /// caller reaching this method with `!e` already has `!msdo_present`, and this body's
    /// guard never fires in-band: it is the *defensive* arm. This is deliberate — the
    /// prohibition does NOT fire on an observed single distinct `obu_xlayer_id` overriding
    /// the MSDO's declared count. The genuine real-world violation that the prohibition
    /// rows would catch (an MSDO declaring substreams that never materialize as distinct
    /// extended layers) is the declared-vs-observed reconciliation owned by the upcoming
    /// `msdo-substream-constraint-checks` change, not this skeleton. The id stays
    /// registered so a future declared-vs-observed model can reach it.
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

    /// Emits `annex-a/lcr-required-for-iop` when no local LCR is present in the window
    /// (the IOP1 `!e && m` "Required (Local)" row).
    fn emit_local_lcr_required(
        &self,
        window: &AnnexAIopWindow,
        offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        if !window.local_lcr_present {
            report.push(annex_a_iop_error(
                "annex-a/lcr-required-for-iop",
                offset,
                format!(
                    "Annex A Table A.4: interoperability point 1 with more than one embedded layer \
                     ({}) requires a local OBU_LAYER_CONFIGURATION_RECORD, but none is present in \
                     the coded video sequence",
                    window.embedded_layers()
                ),
            ));
        }
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
            if previous != Some(seq_id) || newly_confirmed {
                self.on_sequence_activation(xlayer, options, report);
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
            if let Some(active_sequence) = self.sequence_headers.get(&seq_id) {
                frame_header_core_checks(
                    obu,
                    first_picture_in_tu,
                    active_sequence,
                    &self.qm,
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
        // Table 6.18 restrictions it decides (AV2 § 6.16.10); re-evaluate the
        // stored observations of this scope and the global bucket — unless an
        // existing record at the same (xlayer, mlayer) key already carries
        // identical Table 6.18-decisive content. In that case every stored
        // observation has already been paired against that content (at
        // metadata-observation time by check_scan_type_consistency, or by the
        // recheck that ran when the record's decisive content last changed), so
        // re-evaluating would only duplicate reports for the identical repeats
        // § 6.14 explicitly allows. A content interpretation for a NEW key, or
        // one whose decisive content changed (itself flagged by
        // content-interpretation/repeated-ci-not-identical below), forms
        // genuinely new (observation, CI-content) pairs and is re-evaluated.
        let decisive_content_unchanged = self
            .content_interpretations
            .get(&(xlayer, mlayer))
            .is_some_and(|existing| {
                scan_type_decisive_content(&existing.content)
                    == scan_type_decisive_content(&content_interpretation)
            });
        if !decisive_content_unchanged {
            self.recheck_scan_type_after_ci(
                xlayer,
                mlayer,
                &content_interpretation,
                obu.offset,
                report,
            );
        }

        match self.content_interpretations.entry((xlayer, mlayer)) {
            Entry::Vacant(slot) => {
                slot.insert(ContentInterpretationRecord {
                    content: content_interpretation,
                    offset: obu.offset,
                    tu_index,
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
                // baseline after being routed above).
                slot.insert(ContentInterpretationRecord {
                    content: content_interpretation,
                    offset: obu.offset,
                    tu_index,
                });
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
    /// of that embedded layer in the coded video sequence" — is deferred:
    // TODO(spec: AV2-5.17.5-METADATA-HDR-CLL): first-coded-picture placement needs
    // metadata<->picture association (prefix/suffix within the coded frame unit)
    // and the color-inheritance rule; deferred to avoid false positives.
    // TODO(spec: AV2-5.17.6-METADATA-HDR-MDCV): same first-coded-picture deferral
    // for metadata_hdr_mdcv (§ 6.16.6 states the rule identically).
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
        let required = group.required_ci_scan_type_idc();
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
            let established = record.content.scan_type_idc.get();
            if established != 0 && established != required {
                let diagnostic = scan_type_ci_mismatch_error(value, required, established, &pair);
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
            }
            if matches!(value, 7 | 8)
                && let Some(timing) = record.content.timing_info
                && !timing.equal_picture_interval
            {
                let diagnostic = scan_type_equal_picture_interval_error(value, &pair);
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
            }
        }

        self.scan_type
            .scopes
            .entry(scope_key)
            .or_default()
            .observations
            .push(ScanTypeObservation {
                mps_pic_struct_type: value,
                offset: obu.offset,
                tu_index,
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
    fn recheck_scan_type_after_ci(
        &mut self,
        ci_xlayer: ExtendedLayerId,
        ci_mlayer: EmbeddedLayerId,
        content: &ContentInterpretation,
        ci_offset: ByteOffset,
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
                offset: obu.offset,
            });
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
                // Annex A Table A.3 / Table A.4: a global LCR is present in this Annex A
                // IOP window, and its LcrMaxNumXLayerCount is the set-bit count of
                // lcr_xlayer_map (§ 5.8.1, mirror lines 382-384). "Presence" here is the
                // Table A.4 sense — an OBU of that type occurred in the window (design).
                self.annex_a_iop
                    .note_global_lcr(info.xlayer_map.count_ones(), obu.offset);
                // AV2 § 7.3.8.3: record the global LCR's id and xlayer map for later
                // local-LCR and sequence-header references.
                self.hls
                    .record_global_lcr(info.global_config_record_id, info.xlayer_map);
                // AV2 § 6.8.9: retain each payload's embedded-layer maps for the
                // dependency-map agreement checks. A redefinition replaces the maps
                // wholesale so a dropped payload cannot leave stale entries.
                self.hls
                    .clear_global_lcr_embedded(info.global_config_record_id);
                for payload in &info.payloads {
                    if let Some(embedded) = &payload.xlayer_info.embedded_layer_info {
                        self.hls.record_global_lcr_embedded(
                            info.global_config_record_id,
                            ExtendedLayerId::from_bits(payload.xlayer_id),
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
                }
            }
            LayerConfigurationRecord::Local(info) => {
                // AV2 § 7.3.8.3: a local LCR's lcr_global_id (when non-zero) must
                // resolve to an available global LCR.
                if info.global_id != 0
                    && self.hls.global_lcr_xlayer_map(info.global_id).is_none()
                    && external_disabled
                {
                    report.push(
                        Diagnostic::error(
                            "lcr/global-lcr-unavailable",
                            format!(
                                "local layer configuration record for obu_xlayer_id {} references \
                                 lcr_global_id {}, but no global layer configuration record with \
                                 that id is available in-band (external HLS is disabled)",
                                xlayer.get(),
                                info.global_id
                            ),
                        )
                        .with_spec_section("7.3.8.3")
                        .with_byte_offset(obu.offset),
                    );
                }
                // AV2 § 7.3.8.4: a local LCR's lcr_local_atlas_id must resolve to an
                // available local atlas segment OBU in the same extended layer.
                if let Some(atlas_id) = info.local_atlas_id
                    && !self.hls.has_local_atlas(xlayer, atlas_id)
                    && external_disabled
                {
                    report.push(
                        Diagnostic::error(
                            "atlas/local-atlas-unavailable",
                            format!(
                                "local layer configuration record for obu_xlayer_id {} references \
                                 lcr_local_atlas_id {}, but no local atlas segment OBU with that id \
                                 is available in-band for that extended layer (external HLS is \
                                 disabled)",
                                xlayer.get(),
                                atlas_id
                            ),
                        )
                        .with_spec_section("7.3.8.4")
                        .with_byte_offset(obu.offset),
                    );
                }
                // Annex A Table A.4: a local LCR is present in this IOP window.
                self.annex_a_iop.note_local_lcr(obu.offset);
                self.hls.record_local_lcr(xlayer, info.local_id);
                // AV2 § 6.8.9: retain the embedded-layer maps for the dependency-map
                // agreement checks. A redefinition replaces the maps wholesale so a
                // re-sent record without embedded info cannot leave stale entries.
                self.hls.clear_local_lcr_embedded(xlayer, info.local_id);
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
        self.check_lcr_dependency_agreement(xlayer, options, report);
        // Annex A.2 / Annex A.4 profile and level/tier value-space checks on the
        // header just activated for this extended layer. Intrinsic to the header (no
        // external-HLS shadowing concern beyond the already-applied
        // external_declares_sequence_header gate above), emitted once per activated
        // header per coded video sequence.
        self.check_annex_a_value_space(xlayer, report);
    }

    /// Seeds the current Annex A Table A.4 IOP window's interoperability point and
    /// embedded-layer count from the frame-confirmed sequence header active for `xlayer`,
    /// when a CLK opens a new coded video sequence (see the call site in `observe_obu`).
    ///
    /// The frame path drives the window via `note_activation`, but only re-runs
    /// `on_sequence_activation` when the activated `seq_header_id` changes or is newly
    /// confirmed (`observe_frame_bearing_obu`). A second coded video sequence that reuses
    /// the same already-confirmed header therefore never re-fires `note_activation`, so its
    /// window would open with no interoperability point and be skipped at evaluation —
    /// missing, e.g., an IOP0 two-extended-layer second coded video sequence without an
    /// MSDO. Seeding from the active header at the coded-video-sequence boundary keeps the
    /// window decidable. Only a frame-confirmed activation is used (the OBU-order fallback
    /// is a guess a later frame can contradict, matching `agreement_activation_for`); a
    /// reserved / Configurable profile leaves the IOP unset, like `note_activation`.
    fn seed_annex_a_iop_from_active(&mut self, xlayer: ExtendedLayerId) {
        if self.external_hls_provided {
            // External HLS makes in-band presence counting unsound and the Table A.4
            // evaluation is suppressed under it (see `evaluate_annex_a_iop_window`); do not
            // seed from an in-band header that may be shadowed by an external one. The
            // broad `Provided` gate matches that suppression decision.
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
        self.annex_a_iop.seed_from_active(
            general.seq_profile_idc.get(),
            u32::from(general.seq_max_mlayer_count.get()),
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
    fn check_annex_a_value_space(
        &mut self,
        xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        let offset = self
            .sequence_header_offsets
            .get(&seq_header_id)
            .copied()
            .unwrap_or(ByteOffset::new(0));
        // Annex A Table A.3 / Table A.4: record this activation's interoperability point
        // (from the activated profile) and embedded-layer count
        // (seq_max_mlayer_cnt_minus_1 + 1) for the IOP window, regardless of the
        // once-per-CVS value-space dedup below — the Table A.4 evaluation accumulates
        // every activation in the window. Only a *frame-confirmed* activation (the
        // § 5.18.2 load_sequence_header path) determines the window's interoperability
        // point: a first-seen OBU-order fallback is a guess a later frame can contradict
        // (§ 7.3.6), and a Table A.4 presence error emitted against the guess could not
        // be retracted (matches the decidability gate of `agreement_activation_for`).
        if self.frame_confirmed_xlayers.contains(&xlayer) {
            self.annex_a_iop.note_activation(
                general.seq_profile_idc.get(),
                u32::from(general.seq_max_mlayer_count.get()),
                offset,
            );
        }
        // Emit once per activated header per coded video sequence: the same activation
        // can be re-confirmed by multiple frames in one coded video sequence (and a CLK
        // re-activation across a coded-video-sequence boundary legitimately re-emits).
        let epoch = self.cvs.cvs_generation_epoch(xlayer);
        if !self
            .emitted_annex_a_value_space
            .insert((xlayer, seq_header_id, epoch))
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
            })
        } else if self.hls.global_lcr_xlayer_map(seq_lcr_id).is_some() {
            Some(LcrAssociation {
                lcr_is_global: true,
                lcr_id: seq_lcr_id,
                maps: self.hls.global_lcr_embedded(seq_lcr_id, xlayer).cloned(),
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
        // Stricter gate than the OPS checks: external HLS cannot declare LCRs, and
        // per § 6.4.1 an externally-provided *local* LCR would resolve seq_lcr_id
        // ahead of an in-band global record, so under any Provided mode the
        // resolved record may not be the activated one — the same rationale as
        // `check_seq_lcr_reference`'s lcr/global-xlayer-map-missing-xlayer gate.
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        let Some((seq_header_id, general)) = self.agreement_activation_for(xlayer) else {
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
                if br_ops_cnt != record.ops_cnt {
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
                                record.offset,
                                record.ops_cnt
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
            // sequence header's obu_xlayer_id. This is suppressed under external HLS:
            // an externally-provided local LCR (not modeled) could resolve seq_lcr_id
            // ahead of this in-band global, making the global's map irrelevant, so
            // flagging it would be a false positive — consistent with the unavailable
            // branch below and the multi-frame-header precedent.
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

        // AV2 § 6.4.1 / § 7.3.8.3 / § 7.3.8.6: when seq_lcr_id != 0, the referenced
        // layer configuration record must be available (local-then-global resolution),
        // and a referenced global LCR must include this header's xlayer in its map.
        self.check_seq_lcr_reference(obu, general.seq_lcr_id.get(), options, report);

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
        // TODO(spec: AV2-7.3.8-HLS-AVAILABILITY): validate cross-xlayer seq_header_id
        // identity once the full HLS availability store exists.
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
        let agreement_inputs_changed = previous_header.is_some_and(|previous| {
            let old = previous.general;
            old.mlayer_dependency_map != new_general.mlayer_dependency_map
                || old.tlayer_dependency_map != new_general.tlayer_dependency_map
                || old.seq_lcr_id != new_general.seq_lcr_id
        });
        let mut layers_to_check = BTreeSet::new();
        if self.active_sequence_by_xlayer.get(&xlayer) == Some(&seq_header_id) {
            layers_to_check.insert(xlayer);
        }
        if agreement_inputs_changed {
            self.emitted_dependency_findings
                .retain(|key| key.seq_header_id() != seq_header_id);
            layers_to_check.extend(
                self.active_sequence_by_xlayer
                    .iter()
                    .filter(|(_, id)| **id == seq_header_id)
                    .map(|(layer, _)| *layer),
            );
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
/// Used to compare repeated activated sequence headers for bit identity without
/// pulling in a hashing dependency (AV2 § 7.3.6).
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
            // A global suffix, or an unreadable first bit, is left unclassified.
            if metadata_is_suffix(obu) == Some(false) {
                self.observe_global_hls_prefix(obu, report);
            }
        } else {
            self.observe_coded_extended_layer_obu(obu, report);
        }
    }

    fn start_temporal_unit(&mut self) {
        self.phase = TemporalUnitPhase::GlobalPrefix;
        self.current_coded_xlayer = None;
        self.reported_missing_delimiter = false;
        self.saw_obu_since_delimiter = false;
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

/// Builds an Annex A Table A.4 interoperability-point presence diagnostic (error,
/// spec section `A.2`, anchored at `offset`).
fn annex_a_iop_error(rule_id: &'static str, offset: ByteOffset, message: String) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("A.2")
        .with_byte_offset(offset)
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
fn parse_frame_core(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
    active_sequence: &SequenceHeader,
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
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).ok()
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
    qm_state: &QuantizerMatrixState,
    report: &mut ValidationReport,
) {
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

    let Some(core) = parse_frame_core(obu, first_picture_in_tu, active_sequence) else {
        return;
    };

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

    // AV2 § 6.17.4.1: frame_width_minus_1 <= max_frame_width_minus_1 and
    // frame_height_minus_1 <= max_frame_height_minus_1, i.e. FrameWidth/FrameHeight do
    // not exceed the active sequence maximum.
    if let Some(size) = core.frame_size {
        let max_width = active_sequence.general.max_frame_width.get();
        let max_height = active_sequence.general.max_frame_height.get();
        if size.width > max_width || size.height > max_height {
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

    // Annex A.4 static level limits for the parsed frame size / tile count against the
    // active sequence header's seq_level_idx / seq_tier.
    frame_annex_a_level_checks(&core, active_sequence, obu, report);

    // AV2 § 6.17.6.2: custom-QM plane-count references for a parsed
    // `setup_qm_params()`, gated on recorded quantizer-matrix availability state.
    if let Some(setup_qm) = core.setup_qm_params.as_ref() {
        frame_qm_reference_checks(setup_qm, active_sequence, qm_state, obu, report);
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
    for payload in &ops.payloads {
        for entry in &payload.xlayer_entries {
            let Some(ptl) = entry.ptl_info.as_ref() else {
                continue;
            };
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
        tracker.complete_temporal_unit(&mut report);
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
