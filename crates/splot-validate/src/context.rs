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
use splot_core::headers::frame::{FrameHeaderPrefix, parse_frame_header_prefix};
use splot_core::headers::layer_config_record::{
    LayerConfigurationRecord, parse_layer_config_record,
};
use splot_core::headers::operating_point_set::{
    OperatingPointSet, OpsMlayerSource, parse_operating_point_set,
};
use splot_core::headers::quantizer_matrix::{
    NUM_CUSTOM_QMS, QuantizerMatrixObu, parse_quantizer_matrix,
};
use splot_core::headers::sequence::{
    MAX_SEQ_NUM, SequenceHeaderGeneral, SequenceHeaderId, TimingInfo, parse_sequence_header,
};
use splot_core::headers::tile_group::parse_tile_group_prefix;
use splot_core::hls::{MAX_MFH_NUM, MfhId, MultiFrameHeaderRecord, parse_multi_frame_header};
use splot_core::obu::finish_obu_payload;
use splot_core::span::{BitOffset, ByteOffset};
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, ObuType};

use crate::diagnostic::{Diagnostic, ValidationReport};
use crate::options::{ExternalHlsMode, ValidationOptions};

/// Maximum conformant `num_*_points` for a film-grain scaling function
/// (AV2 v1.0.0 § 6.17.10.2).
const MAX_FILM_GRAIN_SCALING_POINTS: u8 = 14;

/// Stateful validator data derived from parseable high-level syntax OBUs.
#[derive(Debug, Default)]
pub(crate) struct ValidatorContext {
    sequence_headers: BTreeMap<SequenceHeaderId, SequenceHeaderGeneral>,
    active_sequence_by_xlayer: BTreeMap<ExtendedLayerId, SequenceHeaderId>,
    /// Payload fingerprints for activated sequence headers, keyed by
    /// `(obu_xlayer_id, seq_header_id)`, used to detect non-bit-identical repeats
    /// of an activated sequence header (AV2 § 7.3.8).
    sequence_fingerprints: BTreeMap<(ExtendedLayerId, SequenceHeaderId), u64>,
    /// Content-interpretation records keyed by `(obu_xlayer_id, obu_mlayer_id)`
    /// within the modeled coded-video-sequence scope, used for cross-embedded-layer
    /// timing consistency (AV2 § 6.4.12) and repeated-CI identity (AV2 § 6.14).
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
    temporal_unit: TemporalUnitState,
    /// `true` once a frame-bearing OBU has been observed since the most recent global
    /// temporal delimiter. Used to derive `FirstPictureInTU` for parsed frame headers
    /// (AV2 § 5.18.2 `startCVS`).
    seen_frame_in_tu: bool,
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

/// One observed content-interpretation OBU within the modeled CVS scope.
#[derive(Debug)]
struct ContentInterpretationRecord {
    /// Parsed § 5.15 syntax, used for cross-embedded-layer timing consistency
    /// (AV2 § 6.4.12) and the repeated-CI "same information" check (AV2 § 6.14).
    content: ContentInterpretation,
    /// Source byte offset of the OBU that produced this record.
    offset: ByteOffset,
}

impl ValidatorContext {
    /// Observes one parsed OBU, updating context and emitting stateful diagnostics.
    pub(crate) fn observe_obu(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        // A new temporal unit resets CVS-scoped comparison state; see
        // reset_at_temporal_unit_boundary.
        self.reset_at_temporal_unit_boundary(obu);

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
            _ => {}
        }

        // AV2 § 6.12 / § 6.13: the quantizer-matrix "between coded frames" and
        // film-grain "coded frame unit" windows close at a coded frame. The validator
        // does not model exact coded-frame-unit boundaries, so it resets after
        // observing each frame-bearing OBU (and at each temporal-unit boundary; see
        // reset_at_temporal_unit_boundary), giving the next window a clean slate. This
        // over-resets relative to the AVM reset-before-tile-group point, so it can only
        // drop a duplicate detection (a documented false negative), never raise a false
        // positive on a conformant stream.
        if is_frame_bearing(obu.header.obu_type) {
            self.reset_coded_frame_window();
        }
    }

    /// Resets the §6.12/§6.13 coded-frame windows for quantizer-matrix and film-grain
    /// state at a coded-frame boundary.
    fn reset_coded_frame_window(&mut self) {
        self.qm.reset_coded_frame_window();
        self.film_grain.reset_coded_frame_window();
    }

    /// Resets the CVS-scoped comparison state at each temporal-unit boundary
    /// (a global `OBU_TEMPORAL_DELIMITER`).
    ///
    /// Sequence-header fingerprints (AV2 § 7.3.8) and content-interpretation records
    /// (§ 6.4.12 / § 6.14) are compared within a coded video sequence. Modeling the
    /// exact CVS boundary needs the `OBU_CLOSED_LOOP_KEY` frame header that starts a
    /// CVS together with random-access state, which is out of scope. Resetting at the
    /// temporal-unit boundary is sound-over-complete: it keeps a CVS-opening sequence
    /// header's fingerprint across the activating CLK (which follows it in the same
    /// temporal unit, AV2 § 7.3.6), so a non-identical repeat later in the temporal
    /// unit is caught; a coded video sequence that spans temporal units yields a
    /// documented false negative for a repeat crossing a temporal-unit boundary —
    /// never a false positive (it only drops comparisons). Frame-header activation
    /// drives the *active* sequence header (see `observe_frame_bearing_obu`); this
    /// reset drives the fingerprint / content-interpretation scope.
    // TODO(spec: AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY): scope per-CVS state
    // exactly once random-access / long-term-reference detection is modeled.
    fn reset_at_temporal_unit_boundary(&mut self, obu: &ObuEnvelope<'_>) {
        if obu.header.obu_type == ObuType::TemporalDelimiter
            && obu.header.extended_layer_id.is_global()
        {
            self.sequence_fingerprints.clear();
            self.content_interpretations.clear();
            self.seen_frame_in_tu = false;
            // A new temporal unit also opens a fresh §6.12/§6.13 coded-frame window.
            self.reset_coded_frame_window();
        }
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

        let first_picture_in_tu = !self.seen_frame_in_tu;
        self.seen_frame_in_tu = true;

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
            // == 1. The sequence-header model does not expose MLayerDependencyMap /
            // TLayerDependencyMap (parse_dependency_map_bits discards the bits), so this
            // check is deferred rather than fabricated from max layer ids.
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
    /// modeled coded-video-sequence scope.
    ///
    /// Timing values are compared only between two present `timing_info()` values
    /// that are both within the same extended layer's modeled CVS scope (a sound
    /// subset of the spec's "across all embedded layers" requirement; exact
    /// cross-extended-layer scoping needs CLK frame-header activation).
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

        // Cross-embedded-layer timing consistency: compare this layer's timing
        // against the first other embedded layer (same extended layer) that already
        // carries present timing within this CVS scope.
        if let Some(new_timing) = content_interpretation.timing_info
            && let Some((existing_mlayer, existing_timing)) = self
                .content_interpretations
                .iter()
                .find(|((x, m), record)| {
                    *x == xlayer && *m != mlayer && record.content.timing_info.is_some()
                })
                .and_then(|((_, m), record)| record.content.timing_info.map(|t| (*m, t)))
        {
            compare_timing_across_embedded_layers(
                existing_mlayer,
                &existing_timing,
                &new_timing,
                obu,
                report,
            );
        }

        match self.content_interpretations.entry((xlayer, mlayer)) {
            Entry::Vacant(slot) => {
                slot.insert(ContentInterpretationRecord {
                    content: content_interpretation,
                    offset: obu.offset,
                });
            }
            Entry::Occupied(slot) => {
                let existing = slot.get();
                // AV2 § 6.14: a repeated CI OBU for the same embedded layer within a
                // CVS must carry the same *information* (a weaker requirement than the
                // sequence header's bit-identity in § 7.3.8). Each compared field
                // resolves to a canonical value (incl. unspecified defaults for absent
                // color/aspect), so the first record is a complete baseline and is
                // kept as-is (matching the sequence-header first-wins approximation);
                // the decoder-ignored ci_reserved_2bit is excluded.
                if content_interpretation_information_differs(
                    &existing.content,
                    &content_interpretation,
                ) {
                    report.push(
                        Diagnostic::error(
                            "content-interpretation/repeated-ci-not-identical",
                            format!(
                                "content interpretation OBU for obu_xlayer_id {} / obu_mlayer_id {} \
                                 is repeated within the coded video sequence with different \
                                 information (first seen at byte {})",
                                xlayer.get(),
                                mlayer.get(),
                                existing.offset
                            ),
                        )
                        .with_spec_section("6.14")
                        .with_byte_offset(obu.offset),
                    );
                }
            }
        }
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

        // AV2 § 7.3.8: within a coded video sequence, a repeated activated sequence
        // header is allowed only if its payload bytes are bit-identical. Compare a
        // payload fingerprint, not parsed fields, since inferred values can hide
        // syntax differences. Fingerprints are cleared per extended layer at CVS
        // boundaries (see maybe_reset_coded_video_sequence).
        //
        // NOTE: the fingerprint key is (xlayer, seq_header_id); cross-xlayer identity
        // for the same seq_header_id is not yet enforced.
        // TODO(spec: AV2-7.3.8-HLS-AVAILABILITY): validate cross-xlayer seq_header_id
        // identity once the full HLS availability store exists.
        match self.sequence_fingerprints.entry((xlayer, seq_header_id)) {
            Entry::Vacant(slot) => {
                slot.insert(fingerprint);
            }
            Entry::Occupied(slot) => {
                if *slot.get() != fingerprint {
                    report.push(
                        Diagnostic::error(
                            "hls/repeated-sequence-header-not-identical",
                            format!(
                                "activated sequence header seq_header_id {} for obu_xlayer_id {} \
                                 is repeated with different payload bytes",
                                seq_header_id.get(),
                                xlayer.get()
                            ),
                        )
                        .with_spec_section("7.3.8")
                        .with_byte_offset(obu.offset),
                    );
                }
            }
        }

        // Store the latest well-formed header per seq_header_id, so a reconfiguration
        // (a later sequence header reusing the id with different layer limits) is the
        // one used for max_tlayer_id / max_mlayer_id checks once a frame header
        // activates it. A non-identical repeat within a CVS is still flagged above.
        self.sequence_headers.insert(seq_header_id, general);
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

        if obu.header.temporal_layer_id > sequence_header.max_tlayer_id {
            report.push(sequence_state_error(
                "sequence-state/tlayer-exceeds-max",
                "6.2.2",
                obu,
                Some(BitOffset::from_bits(6)),
                format!(
                    "obu_tlayer_id {} exceeds active sequence max_tlayer_id {}",
                    obu.header.temporal_layer_id.get(),
                    sequence_header.max_tlayer_id.get()
                ),
            ));
        }

        if obu.header.embedded_layer_id > sequence_header.max_mlayer_id {
            let byte_offset = obu.offset.saturating_add(1);
            report.push(
                Diagnostic::error(
                    "sequence-state/mlayer-exceeds-max",
                    format!(
                        "obu_mlayer_id {} exceeds active sequence max_mlayer_id {}",
                        obu.header.embedded_layer_id.get(),
                        sequence_header.max_mlayer_id.get()
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
/// the same coded video sequence and emits a diagnostic per differing field
/// (AV2 § 6.4.12: these values, when present, shall be the same across all embedded
/// layers). `new` is located at `obu` (embedded layer `obu.header.embedded_layer_id`);
/// `existing` is the value previously seen for `existing_mlayer`. Both embedded-layer
/// ids are named in each message so the finding is self-contained.
fn compare_timing_across_embedded_layers(
    existing_mlayer: EmbeddedLayerId,
    existing: &TimingInfo,
    new: &TimingInfo,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    let new_mlayer = obu.header.embedded_layer_id.get();
    let existing_mlayer = existing_mlayer.get();
    if existing.num_units_in_display_tick != new.num_units_in_display_tick {
        report.push(timing_mismatch_error(
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
        report.push(timing_mismatch_error(
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
        report.push(timing_mismatch_error(
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
        report.push(timing_mismatch_error(
            "sequence-header/timing-num-ticks-mismatch",
            obu,
            format!(
                "num_ticks_per_picture_minus_1 {new_ticks} (obu_mlayer_id {new_mlayer}) differs \
                 from {existing_ticks} (obu_mlayer_id {existing_mlayer}) in the same coded video \
                 sequence"
            ),
        ));
    }
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
/// pulling in a hashing dependency (AV2 § 7.3.8).
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
        } else if is_global_hls_prefix_obu(obu) {
            self.observe_global_hls_prefix(obu, report);
        } else if is_coded_extended_layer_obu(obu) {
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

fn is_global_hls_prefix_obu(obu: &ObuEnvelope<'_>) -> bool {
    // AV2 § 7.3.7 lists the global temporal-unit prefix OBUs exhaustively: MSDO,
    // global LCR, global OPS, global atlas segment, and global metadata (is_suffix=0).
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
                | ObuType::MetadataShort
                | ObuType::MetadataGroup
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
