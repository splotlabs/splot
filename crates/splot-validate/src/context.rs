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
    OperatingPointSet, OpsMlayerSource, parse_operating_point_set,
};
use splot_core::headers::quantizer_matrix::{
    NUM_CUSTOM_QMS, QuantizerMatrixObu, parse_quantizer_matrix,
};
use splot_core::headers::sequence::{
    MAX_SEQ_NUM, MLayerDependencyMap, SequenceHeader, SequenceHeaderId, TLayerDependencyMap,
    TimingInfo, parse_sequence_header,
};
use splot_core::headers::tile_group::parse_tile_group_prefix;
use splot_core::hls::{MAX_MFH_NUM, MfhId, MultiFrameHeaderRecord, parse_multi_frame_header};
use splot_core::obu::finish_obu_payload;
use splot_core::span::{BitOffset, ByteOffset};
use splot_core::tile::{MAX_TILE_COLS, MAX_TILE_ROWS};
use splot_core::types::{
    EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId,
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
    /// `(obu_xlayer_id, atlas_segment_id)` of local atlas segment OBUs seen in-band so
    /// far (§ 7.3.8.4).
    local_atlases: BTreeSet<(ExtendedLayerId, u8)>,
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
            if xlayer.is_global() {
                self.by_xlayer.clear();
            } else {
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
    /// Deferred cross-temporal-unit CVS-scoped diagnostics, tagged with the extended
    /// layer that scopes the comparison; flushed when the temporal unit completes.
    pending_cross_tu: Vec<(ExtendedLayerId, Diagnostic)>,
}

impl CvsTracker {
    /// Records a § 7.3.6 boundary event: a CLK OBU for `xlayer` starts a new coded
    /// video sequence at the current temporal unit. Pending deferred diagnostics for
    /// `xlayer` compare records of the previous coded video sequence against this
    /// one, so they are dropped. Idempotent within a temporal unit.
    fn start_cvs(&mut self, xlayer: ExtendedLayerId) {
        self.cvs_started_in_tu.insert(xlayer, self.tu_index);
        self.pending_cross_tu.retain(|(x, _)| *x != xlayer);
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
        if record_tu == self.tu_index {
            report.push(diagnostic);
        } else {
            self.pending_cross_tu.push((xlayer, diagnostic));
        }
    }

    /// Flushes the deferred diagnostics of the just-completed temporal unit: an
    /// entry is dropped when its extended layer started a new coded video sequence
    /// in that temporal unit (the compared records then sit in different coded video
    /// sequences, § 7.3.6) and emitted otherwise. An entry tagged with
    /// `GLOBAL_XLAYER_ID` scopes records with no single owning extended layer and is
    /// dropped when ANY extended layer started a coded video sequence in the
    /// temporal unit (documented approximation, sound: it only drops comparisons).
    fn flush_completed_tu(&mut self, report: &mut ValidationReport) {
        let tu_index = self.tu_index;
        let any_started_this_tu = self.cvs_started_in_tu.values().any(|&tu| tu == tu_index);
        for (xlayer, diagnostic) in std::mem::take(&mut self.pending_cross_tu) {
            let started_this_tu = if xlayer.is_global() {
                any_started_this_tu
            } else {
                self.cvs_started_in_tu.get(&xlayer) == Some(&tu_index)
            };
            if !started_this_tu {
                report.push(diagnostic);
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
        self.pending_cross_tu.retain(|(x, diagnostic)| {
            !((*x == xlayer || x.is_global()) && rule_ids.contains(&diagnostic.rule_id.as_str()))
        });
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
        self.observe_cvs_boundary_events(obu, report);

        self.temporal_unit.observe_obu(obu, report);

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

        match obu.header.obu_type {
            ObuType::ContentInterpretation => self.observe_content_interpretation(obu, report),
            ObuType::MultiFrameHeader => self.observe_multi_frame_header(obu, options, report),
            ObuType::LayerConfigurationRecord => {
                self.observe_layer_config_record(obu, options, report);
            }
            ObuType::AtlasSegment => self.observe_atlas_segment(obu),
            ObuType::OperatingPointSet => self.observe_operating_point_set(obu, report),
            ObuType::BufferRemovalTiming => {
                self.observe_buffer_removal_timing(obu, options, report);
            }
            ObuType::QuantizationMatrix => self.observe_quantizer_matrix(obu, report),
            ObuType::FilmGrain => self.observe_film_grain(obu, report),
            ObuType::MetadataShort | ObuType::MetadataGroup => {
                self.observe_metadata(obu, report);
            }
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
            self.frames_seen_in_tu.clear();
        } else if obu.header.obu_type == ObuType::ClosedLoopKey {
            self.start_cvs_for_xlayer(obu.header.extended_layer_id, report);
            self.observe_ci_rap(obu.header.extended_layer_id);
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
        self.cvs.start_cvs(xlayer);
        let tu_index = self.cvs.tu_index;
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
        let scope_keys: Vec<ExtendedLayerId> = self.scan_type.scopes.keys().copied().collect();
        for scope_key in scope_keys {
            self.flush_scan_type_scope(scope_key, u64::MAX, report);
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
            self.active_sequence_by_xlayer
                .insert(obu.header.extended_layer_id, seq_id);

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
            // TODO(spec: AV2-5.7-MULTI-FRAME-HEADER): when cur_mfh_id > 0, also enforce
            // the §7.3.8.7 layer-dependency constraints
            // MLayerDependencyMap[obu_mlayer_id][MfhMLayerId[cur_mfh_id]] == 1 and
            // TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId[cur_mfh_id]]
            // == 1. The sequence-header model now exposes the §5.4.1 dependency maps
            // (SequenceHeaderGeneral::{m,t}layer_dependency_map), but the §7.3.8.7
            // check itself is deferred to the multi-frame-header change.
            let seq_raw = u32::from(record.mfh_seq_header_id.get());
            self.resolve_referenced_sequence_header(seq_raw, obu, options, report)
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
                // AV2 § 7.3.8.3: record the global LCR's id and xlayer map for later
                // local-LCR and sequence-header references.
                self.hls
                    .record_global_lcr(info.global_config_record_id, info.xlayer_map);
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
                self.hls.record_local_lcr(xlayer, info.local_id);
            }
            // `LayerConfigurationRecord` is `#[non_exhaustive]`; only global and local
            // scopes exist in AV2 v1.0.0, so any future variant is ignored here.
            _ => {}
        }
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

        // AV2 § 6.10.1: apply reset/update to the active OPS state after the checks.
        self.ops.apply(
            OperatingPointSetRecord {
                xlayer_id: ops.xlayer_id,
                ops_id: ops.ops_id,
                ops_cnt: ops.ops_cnt,
                offset: obu.offset,
            },
            ops.reset_flag,
        );
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
        self.sequence_headers.insert(seq_header_id, sequence_header);
        // The active sequence header for an extended layer defaults to the first one
        // seen in OBU order; a parsed CLK/OLK frame header overrides this with the
        // sequence header it references (see observe_frame_bearing_obu), which is the
        // exact AV2 § 5.18.2 activation point for the paths the skeleton parses.
        self.active_sequence_by_xlayer
            .entry(xlayer)
            .or_insert(seq_header_id);
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
    // TLayerDependencyMap analogue) need the sequence dependency maps, which the
    // parsed sequence model does not expose (parse_dependency_map_bits discards the
    // bits); they are deferred rather than fabricated.
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
