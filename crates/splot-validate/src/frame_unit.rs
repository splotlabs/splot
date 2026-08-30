// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coded-frame-unit segmentation (AV2 v1.0.0 § 7.3.3 / § 7.3.4 / § 7.3.5).
//!
//! The validator partitions each `(obu_xlayer_id, obu_mlayer_id, obu_tlayer_id)`
//! triple's consecutive OBUs in a temporal unit into coded frame units and enforces
//! the § 7.3.3 (output) / § 7.3.4 (non-output) presence order. Both grammars share
//! the region order: content interpretation (zero or one), multi-frame headers, the
//! pre-frame region (BRT / QM / FGM / prefix metadata in any order), exactly one
//! coded frame, then the suffix-metadata tail. The asymmetry: a non-output frame
//! unit allows zero or one BRT OBU (mirror line 452), an output frame unit zero or
//! more (mirror line 384). `OBU_PADDING` is position-free (mirror lines 433-434 /
//! 496-497).
//!
//! ## Output classification and the Unknown invariant
//!
//! Output vs non-output selects the § 7.3.3 / § 7.3.4 grammar and the BRT
//! multiplicity bound. A SEF is always output, `OBU_BRIDGE_FRAME` always non-output
//! (mirror line 470); the rest carry `immediate_output_frame` /
//! `implicit_output_frame` from the core parser. When that classification is
//! undecidable the output class is [`OutputClass::Unknown`] and the
//! output-class-derived judgment (the § 7.3.4 BRT bound and the grammar branch) is
//! dropped, never guessed. The structural presence-order facts are decidable from
//! OBU types and the `is_first_tile_group` / `metadata_is_suffix` bits alone, so they
//! fire eagerly even when the output class is Unknown.
//!
//! Two distinct undecidabilities. (1) The region pointer: an unreadable
//! `metadata_is_suffix` bit sets [`UnitState::region_blind`], suppressing the
//! remaining region-order checks while still tracking the coded frame for the BRT
//! resolution. (2) The coded-frame boundary between same-type no-delimiter frames:
//! `OBU_LEADING_TIP` / `OBU_REGULAR_TIP` / `OBU_BRIDGE_FRAME` carry no
//! `is_first_tile_group` flag, so a different-`obu_type` neighbour splits decidably
//! (silently, not `mixed-coded-frame-types`) while a same-`obu_type` neighbour is
//! unit-count-undecidable: it stays in the open coded frame and reports
//! [`FrameBoundary::Ambiguous`]. A TIP routes the open frame's class to Unknown; a
//! bridge keeps its type-decided non-output class, so its § 7.3.4 BRT bound stays
//! evaluable.
//!
//! The structural facts fire eagerly; the one output-class-dependent fact (the
//! § 7.3.4 non-output BRT multiplicity bound) is resolved at the unit boundary.
//!
//! ## Boundary signal for the CELU layer
//!
//! The segmenter is the single source of truth for coded-frame-unit boundaries
//! (§ 7.3.6). [`Self::observe`](FrameUnitSegmenter::observe) returns each
//! frame-bearing OBU's [`FrameBoundary`], which the
//! [`CodedExtendedLayerTracker`](crate::celu::CodedExtendedLayerTracker) consumes
//! rather than re-deriving, so the two layers agree by construction (e.g. on
//! `TIP, BRT, TIP` the BRT head splits the two TIPs into two units).

use std::collections::BTreeMap;

use splot_core::annexb::ObuEnvelope;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, ObuType, TemporalLayerId};

use crate::diagnostic::{Diagnostic, ValidationReport};

/// Per-OBU classification handed to the segmenter by the validator.
///
/// The validator computes the frame-header-derived facts (output class,
/// `is_first_tile_group`, `metadata_is_suffix`) once, on the parse paths it
/// supports, and passes the result here; the segmenter holds no parser state.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SegRole {
    /// `OBU_CONTENT_INTERPRETATION`.
    ContentInterpretation,
    /// `OBU_MULTI_FRAME_HEADER`.
    MultiFrameHeader,
    /// `OBU_BUFFER_REMOVAL_TIMING`.
    BufferRemovalTiming,
    /// `OBU_QUANTIZATION_MATRIX`.
    QuantizationMatrix,
    /// `OBU_FILM_GRAIN`.
    FilmGrain,
    /// `OBU_METADATA_SHORT` / `OBU_METADATA_GROUP`. `Some(false)` is a prefix
    /// (`metadata_is_suffix == 0`), `Some(true)` a suffix, `None` an unreadable
    /// suffix bit (sets the unit's region-blind state rather than guessing).
    Metadata { is_suffix: Option<bool> },
    /// A tile-group frame OBU (`OBU_*_TILE_GROUP`, `OBU_SWITCH`, `OBU_RAS_FRAME`,
    /// `OBU_CLOSED_LOOP_KEY`, `OBU_OPEN_LOOP_KEY`). `is_first_tile_group` and the
    /// output class are `None` when the prefix/core parse could not derive them.
    TileFrame {
        is_first_tile_group: Option<bool>,
        output: Option<bool>,
    },
    /// A SEF frame OBU (`OBU_LEADING_SEF` / `OBU_REGULAR_SEF`): always an output
    /// coded frame, exactly one OBU per coded frame (mirror line 417).
    SefFrame,
    /// A TIP frame OBU (`OBU_LEADING_TIP` / `OBU_REGULAR_TIP`). The output class is
    /// `None` when undecidable. TIP frames carry no `is_first_tile_group` (they are
    /// not in the first-tile-group list, mirror lines 404-411).
    TipFrame { output: Option<bool> },
    /// `OBU_BRIDGE_FRAME`. Carries no `is_first_tile_group`; single-picture
    /// headers infer output while ordinary bridge frames infer non-output.
    BridgeFrame { output: Option<bool> },
    /// `OBU_PADDING`: position-free within a coded frame unit.
    Padding,
}

/// How a frame-bearing OBU relates to the coded frame unit boundary in its
/// `(xlayer, mlayer, tlayer)` triple, reported by [`FrameUnitSegmenter::observe`].
///
/// The segmenter is the single source of truth for coded-frame-unit boundaries
/// (§ 7.3.3 / § 7.3.4 / § 7.3.5): the [`CodedExtendedLayerTracker`](crate::celu)
/// consumes this signal rather than re-deriving boundaries from frame-delimiter
/// bits, so the two layers never diverge on where one coded frame unit ends and the
/// next begins.
///
/// The signal is computed from this OBU's own triple's open-unit state, so an OBU at
/// a *different* `obu_tlayer_id` of the same `(xlayer, mlayer)` opens a fresh triple
/// state and therefore reports [`Self::OpensNewUnit`] for its coded frame's first
/// OBU. The CELU layer aggregates per `(xlayer, mlayer)`, so this per-triple signal
/// maps directly: an OBU that opens a unit in *any* triple of an `(xlayer, mlayer)`
/// opens a coded frame unit for that embedded layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameBoundary {
    /// This OBU is the first OBU of a *new* coded frame unit in its triple — the
    /// prior unit (if any) is complete. The CELU layer counts a new frame unit for
    /// the OBU's `(xlayer, mlayer)`.
    OpensNewUnit,
    /// This OBU is a *decided* continuation of the open coded frame unit (a
    /// later OBU of the same coded frame: a `is_first_tile_group == 0` tile OBU, a
    /// same-type readable continuation, the out-of-order `sef-single-obu` /
    /// `mixed-coded-frame-types` OBUs the segmenter keeps in the open frame, or a
    /// same-type bridge whose output is type-decided). It does *not* open a CELU
    /// frame unit.
    ContinuesUnit,
    /// The boundary is *undecidable* (a same-type no-delimiter TIP OBU, or a
    /// tile-group OBU whose `is_first_tile_group` bit could not be read, while a
    /// coded frame is open). The OBU stays in the open coded frame (no false split),
    /// but every unit-count-dependent CELU judgment for the OBU's `(xlayer, mlayer)`
    /// is *poisoned* — dropped, never guessed (the Unknown invariant).
    Ambiguous,
}

/// Resolved output classification of a coded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputClass {
    Output,
    NonOutput,
    /// Undecidable (unsupported parse path) — the output-class-derived judgment
    /// (the § 7.3.4 BRT bound and the grammar branch) is dropped for the unit.
    Unknown,
}

/// One frame-bearing OBU's identity within the coded frame, for the
/// same-`obu_type`, SEF-single-OBU, and BRT-multiplicity rules.
#[derive(Debug, Clone, Copy)]
struct CodedFrameState {
    /// The `obu_type` of the coded frame's OBUs (the tile/SEF/TIP/bridge type).
    obu_type: ObuType,
    /// `true` once a SEF has been counted (a SEF coded frame is exactly one OBU).
    is_sef: bool,
    /// The resolved output classification.
    output: OutputClass,
}

/// The region a coded frame unit is currently in (the § 7.3.3 / § 7.3.4 order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    /// Before any OBU of the unit has been seen.
    Start,
    /// After the (single) content-interpretation OBU.
    AfterCi,
    /// In or after the multi-frame-header run.
    Mfh,
    /// In the pre-frame region (BRT / QM / FGM / prefix metadata).
    PreFrame,
    /// In the coded frame (tile OBUs / SEF / TIP / bridge).
    CodedFrame,
    /// In the suffix-metadata tail.
    SuffixTail,
}

/// The accumulating state of one coded frame unit for a single layer triple.
#[derive(Debug)]
struct UnitState {
    region: Region,
    /// `true` once any CI has been seen in this unit (for the duplicate-CI rule).
    saw_ci: bool,
    /// Number of buffer-removal-timing OBUs seen in this unit (for the § 7.3.4
    /// non-output zero-or-one bound, resolved at unit end).
    brt_count: u32,
    /// The byte offset of the unit's *second* buffer-removal-timing OBU, the anchor
    /// for the deferred non-output multiplicity diagnostic.
    second_brt_offset: Option<ByteOffset>,
    /// The coded frame's identity, once its first frame-bearing OBU is seen.
    coded_frame: Option<CodedFrameState>,
    /// The byte offset of the unit's first **head** OBU (CI / MFH / pre-frame), the
    /// anchor for the head-only-unit diagnostic. A unit that accumulates head OBUs
    /// but never a coded frame (the head run ends at a temporal-unit / bitstream
    /// boundary) violates the § 7.3.3 / § 7.3.4 requirement that every coded frame
    /// unit contain a coded frame; this offset anchors the report at the offending
    /// head run's start.
    head_offset: Option<ByteOffset>,
    /// `true` once a metadata OBU's `metadata_is_suffix` bit could not be read: the
    /// region pointer is no longer reliable, so region-order structural checks are
    /// suppressed for the rest of this unit (the structural facts that do not depend
    /// on the region — the coded frame's identity/output for the BRT resolution —
    /// still update).
    region_blind: bool,
}

impl UnitState {
    fn new() -> Self {
        Self {
            region: Region::Start,
            saw_ci: false,
            brt_count: 0,
            second_brt_offset: None,
            coded_frame: None,
            head_offset: None,
            region_blind: false,
        }
    }

    /// Records the offset of the unit's first head OBU (CI / MFH / pre-frame), used
    /// as the head-only-unit anchor if the unit never receives a coded frame.
    fn note_head(&mut self, offset: ByteOffset) {
        self.head_offset.get_or_insert(offset);
    }
}

/// Why the segmenter is flushing its open units, selecting the head-only-unit
/// diagnostic severity (a temporal-unit boundary is a hard error; the end of the
/// bitstream is a warning, since a trailing head run may be a truncated stream).
#[derive(Debug, Clone, Copy)]
enum FlushKind {
    TemporalUnitBoundary,
    EndOfStream,
}

/// Per-layer-triple segmentation state within the current temporal unit.
#[derive(Debug)]
struct LayerState {
    unit: UnitState,
}

/// Coded-frame-unit segmenter (AV2 § 7.3.3 / § 7.3.4 / § 7.3.5 / § 7.3.8.10).
///
/// One instance lives in the validator context. It is fed every OBU in stream
/// order via [`Self::observe`], reset per temporal unit via
/// [`Self::reset_temporal_unit`], and flushed at the end of the bitstream via
/// [`Self::finish`]. All state is keyed by the full `(xlayer, mlayer, tlayer)`
/// triple, since § 7.3.3 / § 7.3.4 define a coded frame unit over that triple.
#[derive(Debug, Default)]
pub(crate) struct FrameUnitSegmenter {
    layers: BTreeMap<(ExtendedLayerId, EmbeddedLayerId, TemporalLayerId), LayerState>,
    /// Number of distinct coded frames opened for each `(xlayer, mlayer)` embedded layer in
    /// the current temporal unit, counted at each coded frame's first OBU. Each coded frame is
    /// its own unit (§ 7.3.3 / § 7.3.4), so completed units is this minus the open one (see
    /// [`Self::completed_units_for_embedded_layer`]). Keying by `(xlayer, mlayer)` makes it
    /// tlayer-agnostic, matching the § 7.3.8.10 scope (mirror line 880): a second coded frame
    /// at a different `obu_tlayer_id` counts as a later unit.
    coded_frames_opened_for_embedded_layer: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), u32>,
}

impl FrameUnitSegmenter {
    /// Resets all per-temporal-unit state. Called at each global temporal
    /// delimiter (AV2 § 7.3.7): a coded frame unit does not span temporal units.
    /// Any still-open unit's deferred (output-class-dependent) checks are resolved
    /// before the reset; the eager structural checks have already fired.
    pub(crate) fn reset_temporal_unit(&mut self, report: &mut ValidationReport) {
        self.flush_open_units(FlushKind::TemporalUnitBoundary, report);
        self.layers.clear();
        self.coded_frames_opened_for_embedded_layer.clear();
    }

    /// Number of coded frame units this `(xlayer, mlayer)` embedded layer has completed in
    /// the current temporal unit (AV2 § 7.3.8.10 scope, across temporal layers): the distinct
    /// coded frames opened minus the currently-open one. Consumers (the § 6.16.5 / § 6.16.6
    /// first-coded-picture lateness predicate) read `0` as "still within the layer's first
    /// coded frame unit of this temporal unit".
    pub(crate) fn completed_units_for_embedded_layer(
        &self,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
    ) -> u32 {
        self.coded_frames_opened_for_embedded_layer
            .get(&(xlayer, mlayer))
            .copied()
            .unwrap_or(0)
            .saturating_sub(1)
    }

    /// Flushes the final temporal unit's open units (AV2 § 7.3.2 end condition:
    /// end of bitstream), resolving their deferred checks.
    pub(crate) fn finish(&mut self, report: &mut ValidationReport) {
        self.flush_open_units(FlushKind::EndOfStream, report);
    }

    /// Resolves every open unit's deferred (unit-end) checks and reports any
    /// head-only unit (head OBUs with no coded frame) at the `kind`-appropriate
    /// severity.
    fn flush_open_units(&mut self, kind: FlushKind, report: &mut ValidationReport) {
        for state in self.layers.values_mut() {
            Self::resolve_unit(&state.unit, report);
            Self::report_head_only_unit(&state.unit, kind, report);
        }
    }

    /// Reports a head-only unit — one that accumulated head OBUs (CI / MFH /
    /// pre-frame) but never a coded frame — at a flush boundary. § 7.3.3 / § 7.3.4
    /// require every coded frame unit to contain exactly one coded frame, so a head
    /// run that ends at a temporal-unit / bitstream boundary with no coded frame is
    /// non-conforming. At a temporal-unit boundary this is a hard error; at the end
    /// of the bitstream it is reported as a warning, since a trailing head run may be
    /// a truncated stream rather than a malformed unit.
    fn report_head_only_unit(unit: &UnitState, kind: FlushKind, report: &mut ValidationReport) {
        let (Some(offset), None) = (unit.head_offset, unit.coded_frame) else {
            return;
        };
        let diagnostic = match kind {
            FlushKind::TemporalUnitBoundary => Diagnostic::error(
                "frame-unit/missing-coded-frame",
                "a coded frame unit's head OBUs (content-interpretation / multi-frame-header / \
                 pre-frame) are not followed by a coded frame before the temporal unit ends; \
                 every coded frame unit must contain exactly one coded frame"
                    .to_owned(),
            ),
            FlushKind::EndOfStream => Diagnostic::warning(
                "frame-unit/missing-coded-frame",
                "a coded frame unit's head OBUs (content-interpretation / multi-frame-header / \
                 pre-frame) are not followed by a coded frame before the end of the bitstream; \
                 every coded frame unit must contain exactly one coded frame (the stream may be \
                 truncated)"
                    .to_owned(),
            ),
        };
        report.push(
            diagnostic
                .with_spec_section("7.3.3")
                .with_byte_offset(offset),
        );
    }

    /// Non-mutating peek: would this frame-bearing `obu` (with the precomputed `role`)
    /// cause [`Self::observe`] to commit the previous coded frame's deferred § 7.23 update
    /// for its layer triple — i.e. would the boundary be [`FrameBoundary::OpensNewUnit`] or
    /// [`FrameBoundary::Ambiguous`], the set for which
    /// [`ValidatorContext::observe_reference_state`](crate::context) commits the pending
    /// update?
    ///
    /// Must mirror the authoritative boundary [`Self::observe`] returns so the early commit
    /// before the reference-buffer snapshot agrees with the later one. A new-unit reset or no
    /// open coded frame is `OpensNewUnit` (commit); an unreadable tile-group delimiter or a
    /// same-`obu_type` no-delimiter TIP / bridge is `Ambiguous` (commit); a decided
    /// continuation is `ContinuesUnit` (no commit — its pending update is its own frame's).
    /// The state is not advanced; [`Self::observe`] re-commits idempotently later in stream
    /// order.
    pub(crate) fn commits_pending_ref_update(&self, obu: &ObuEnvelope<'_>, role: SegRole) -> bool {
        if obu.header.extended_layer_id.is_global() || matches!(role, SegRole::Padding) {
            return false;
        }
        let key = (
            obu.header.extended_layer_id,
            obu.header.embedded_layer_id,
            obu.header.temporal_layer_id,
        );
        let Some(state) = self.layers.get(&key) else {
            return true;
        };
        if Self::starts_new_unit(&state.unit, role, obu.header.obu_type) {
            return true;
        }
        if state.unit.coded_frame.is_none() {
            return true;
        }
        match role {
            SegRole::TileFrame {
                is_first_tile_group,
                ..
            } => is_first_tile_group.is_none(), // unreadable delimiter -> Ambiguous
            SegRole::TipFrame { .. } | SegRole::BridgeFrame { .. } => {
                debug_assert!(is_no_delimiter_frame_role(role));
                true
            }
            _ => false,
        }
    }

    /// Feeds one OBU to the segmenter in stream order.
    ///
    /// Returns the OBU's coded-frame-unit boundary signal ([`FrameBoundary`]) when it
    /// is frame-bearing (a tile group / SEF / TIP / bridge OBU), so the
    /// [`CodedExtendedLayerTracker`](crate::celu) can use the segmenter as the single
    /// source of truth for coded-frame-unit boundaries. Returns `None` for every
    /// non-frame-bearing OBU (heads, interior, padding, globals) — those neither open
    /// nor continue a coded frame unit.
    pub(crate) fn observe(
        &mut self,
        obu: &ObuEnvelope<'_>,
        role: SegRole,
        report: &mut ValidationReport,
    ) -> Option<FrameBoundary> {
        if obu.header.extended_layer_id.is_global() {
            return None;
        }
        if matches!(role, SegRole::Padding) {
            return None;
        }

        let key = (
            obu.header.extended_layer_id,
            obu.header.embedded_layer_id,
            obu.header.temporal_layer_id,
        );
        let embedded_key = (obu.header.extended_layer_id, obu.header.embedded_layer_id);
        let coded_frames_opened_for_embedded_layer =
            &mut self.coded_frames_opened_for_embedded_layer;
        let state = self.layers.entry(key).or_insert_with(|| LayerState {
            unit: UnitState::new(),
        });

        Self::observe_in_layer(
            state,
            embedded_key,
            coded_frames_opened_for_embedded_layer,
            obu,
            role,
            report,
        )
    }

    /// Drives one layer-triple's state machine for a single OBU. Returns the
    /// frame-bearing OBU's [`FrameBoundary`] (`None` for a non-frame OBU).
    fn observe_in_layer(
        state: &mut LayerState,
        embedded_key: (ExtendedLayerId, EmbeddedLayerId),
        coded_frames_opened_for_embedded_layer: &mut BTreeMap<
            (ExtendedLayerId, EmbeddedLayerId),
            u32,
        >,
        obu: &ObuEnvelope<'_>,
        role: SegRole,
        report: &mut ValidationReport,
    ) -> Option<FrameBoundary> {
        if Self::starts_new_unit(&state.unit, role, obu.header.obu_type) {
            Self::resolve_unit(&state.unit, report);
            state.unit = UnitState::new();
        }

        if matches!(
            role,
            SegRole::ContentInterpretation
                | SegRole::MultiFrameHeader
                | SegRole::BufferRemovalTiming
                | SegRole::QuantizationMatrix
                | SegRole::FilmGrain
                | SegRole::Metadata {
                    is_suffix: Some(false),
                }
        ) {
            state.unit.note_head(obu.offset);
        }

        match role {
            SegRole::ContentInterpretation => {
                Self::observe_ci(
                    state,
                    embedded_key,
                    coded_frames_opened_for_embedded_layer,
                    obu,
                    report,
                );
                None
            }
            SegRole::MultiFrameHeader => {
                Self::observe_mfh(&mut state.unit, obu, report);
                None
            }
            SegRole::BufferRemovalTiming => {
                Self::observe_brt(&mut state.unit, obu);
                None
            }
            SegRole::QuantizationMatrix | SegRole::FilmGrain => {
                if !state.unit.region_blind {
                    state.unit.region = Region::PreFrame;
                }
                None
            }
            SegRole::Metadata { is_suffix } => {
                Self::observe_metadata(&mut state.unit, is_suffix, obu, report);
                None
            }
            SegRole::TileFrame {
                is_first_tile_group,
                output,
            } => Some(Self::observe_tile_frame(
                state,
                embedded_key,
                coded_frames_opened_for_embedded_layer,
                is_first_tile_group,
                output_class(output),
                obu,
                report,
            )),
            SegRole::SefFrame => Some(Self::observe_frame(
                state,
                embedded_key,
                coded_frames_opened_for_embedded_layer,
                None,
                output_class(type_decided_output(obu.header.obu_type)),
                true,
                false, // a SEF is a single-OBU coded frame, judged by the SEF rule
                true,  // SEF output is decided by type (always output), not a header parse
                false, // a SEF carries no is_first_tile_group bit to be unreadable
                obu,
                report,
            )),
            SegRole::TipFrame { output } => Some(Self::observe_frame(
                state,
                embedded_key,
                coded_frames_opened_for_embedded_layer,
                None,
                output_class(output),
                false,
                true,  // TIP frames carry no in-band coded-frame delimiter
                false, // TIP output is derived from the per-frame header, not the type
                false, // a TIP frame carries no is_first_tile_group bit to be unreadable
                obu,
                report,
            )),
            SegRole::BridgeFrame { output } => Some(Self::observe_frame(
                state,
                embedded_key,
                coded_frames_opened_for_embedded_layer,
                None,
                output_class(output),
                false,
                true,             // OBU_BRIDGE_FRAME carries no in-band coded-frame delimiter
                output.is_some(), // the active sequence header decides the bridge class
                false, // a bridge frame carries no is_first_tile_group bit to be unreadable
                obu,
                report,
            )),
            SegRole::Padding => None,
        }
    }

    /// Whether `role` begins a new coded frame unit given the current unit's region.
    ///
    /// A coded frame is one or more OBUs (mirror lines 391-393 / 459-461), so a frame OBU in
    /// [`Region::CodedFrame`] is normally a continuation judged in `observe_frame`. A new unit
    /// begins when:
    ///
    /// - a head OBU (CI / MFH / pre-frame) follows a completed coded frame or its suffix tail
    ///   (§ 7.3.6 back-to-back units),
    /// - any frame OBU follows the suffix tail (the prior unit is complete),
    /// - a tile OBU with `is_first_tile_group == 1` arrives while a coded frame is open
    ///   (mirror lines 413-414 / 486-487); a flag-0 or undecidable tile OBU continues it,
    /// - a SEF arrives while a coded frame is open (a SEF is a single-OBU coded frame, mirror
    ///   line 417, so it cannot continue another), or
    /// - a no-delimiter frame OBU (TIP / bridge) of a *different* `obu_type` than the open
    ///   coded frame arrives (a type change cannot be a same-frame continuation; this splits
    ///   decidably and silently). A *same*-`obu_type` no-delimiter OBU is left to
    ///   `observe_frame` (the undecidable case).
    ///
    /// Suffix metadata never starts a unit (judged in `observe_metadata`).
    fn starts_new_unit(unit: &UnitState, role: SegRole, obu_type: ObuType) -> bool {
        let is_unit_head = matches!(
            role,
            SegRole::ContentInterpretation
                | SegRole::MultiFrameHeader
                | SegRole::BufferRemovalTiming
                | SegRole::QuantizationMatrix
                | SegRole::FilmGrain
                | SegRole::Metadata {
                    is_suffix: Some(false),
                }
        );
        let starts_next_coded_frame = matches!(
            role,
            SegRole::TileFrame {
                is_first_tile_group: Some(true),
                ..
            }
        );
        let starts_next_sef = matches!(role, SegRole::SefFrame);
        let starts_next_no_delimiter_frame = is_no_delimiter_frame_role(role)
            && unit
                .coded_frame
                .is_some_and(|frame| frame.obu_type != obu_type);
        match unit.region {
            Region::CodedFrame => {
                is_unit_head
                    || starts_next_coded_frame
                    || starts_next_sef
                    || starts_next_no_delimiter_frame
            }
            Region::SuffixTail => is_unit_head || is_frame_role(role),
            _ => false,
        }
    }

    /// Observes a content-interpretation OBU (region 1; § 7.3.8.10 placement).
    fn observe_ci(
        state: &mut LayerState,
        embedded_key: (ExtendedLayerId, EmbeddedLayerId),
        coded_frames_opened_for_embedded_layer: &BTreeMap<(ExtendedLayerId, EmbeddedLayerId), u32>,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let unit = &mut state.unit;
        let completed = coded_frames_opened_for_embedded_layer
            .get(&embedded_key)
            .copied()
            .unwrap_or(0)
            .saturating_sub(1);
        let in_later_unit = completed > 0
            || (unit.coded_frame.is_none()
                && coded_frames_opened_for_embedded_layer.contains_key(&embedded_key));
        if in_later_unit {
            report.push(frame_unit_error(
                "frame-unit/ci-not-in-first-frame-unit",
                obu,
                "7.3.8.10",
                "OBU_CONTENT_INTERPRETATION appears outside the first coded frame unit of its \
                 embedded layer in the temporal unit"
                    .to_owned(),
            ));
        }

        if !unit.region_blind {
            if unit.saw_ci {
                report.push(frame_unit_error(
                    "frame-unit/duplicate-content-interpretation",
                    obu,
                    "7.3.3",
                    "a coded frame unit carries more than one OBU_CONTENT_INTERPRETATION (zero \
                     or one permitted)"
                        .to_owned(),
                ));
            } else if !matches!(unit.region, Region::Start) {
                report.push(frame_unit_error(
                    "frame-unit/region-order",
                    obu,
                    "7.3.3",
                    "OBU_CONTENT_INTERPRETATION must precede the multi-frame-header, pre-frame, \
                     and coded-frame regions of its coded frame unit"
                        .to_owned(),
                ));
            }
        }
        unit.saw_ci = true;
        if matches!(unit.region, Region::Start) {
            unit.region = Region::AfterCi;
        }
    }

    /// Observes a multi-frame-header OBU (region 2).
    fn observe_mfh(unit: &mut UnitState, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if !unit.region_blind
            && matches!(
                unit.region,
                Region::PreFrame | Region::CodedFrame | Region::SuffixTail
            )
        {
            report.push(frame_unit_error(
                "frame-unit/region-order",
                obu,
                "7.3.3",
                "OBU_MULTI_FRAME_HEADER must precede the pre-frame and coded-frame regions of \
                 its coded frame unit"
                    .to_owned(),
            ));
        }
        if !unit.region_blind {
            unit.region = Region::Mfh;
        }
    }

    /// Observes a buffer-removal-timing OBU (pre-frame region). The non-output
    /// multiplicity bound (§ 7.3.4) is resolved at unit end once the output class is
    /// known; placement (pre-frame) is structural and needs no diagnostic here (a
    /// BRT after the coded frame starts a new unit, § 7.3.3 back-to-back units).
    fn observe_brt(unit: &mut UnitState, obu: &ObuEnvelope<'_>) {
        unit.brt_count = unit.brt_count.saturating_add(1);
        if unit.brt_count == 2 {
            unit.second_brt_offset = Some(obu.offset);
        }
        if !unit.region_blind {
            unit.region = Region::PreFrame;
        }
    }

    /// Observes a metadata OBU (prefix → pre-frame region, suffix → tail).
    fn observe_metadata(
        unit: &mut UnitState,
        is_suffix: Option<bool>,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        match is_suffix {
            None => unit.region_blind = true,
            Some(false) => {
                if !unit.region_blind {
                    unit.region = Region::PreFrame;
                }
            }
            Some(true) => {
                if !unit.region_blind {
                    if matches!(
                        unit.region,
                        Region::Start | Region::AfterCi | Region::Mfh | Region::PreFrame
                    ) {
                        report.push(frame_unit_error(
                            "frame-unit/suffix-metadata-before-coded-frame",
                            obu,
                            "7.3.3",
                            "suffix metadata (metadata_is_suffix == 1) appears before the coded \
                             frame of its coded frame unit"
                                .to_owned(),
                        ));
                    } else {
                        unit.region = Region::SuffixTail;
                    }
                }
            }
        }
    }

    /// Observes a tile-group frame OBU (the coded frame; first-tile-group rule).
    #[allow(clippy::too_many_arguments)]
    fn observe_tile_frame(
        state: &mut LayerState,
        embedded_key: (ExtendedLayerId, EmbeddedLayerId),
        coded_frames_opened_for_embedded_layer: &mut BTreeMap<
            (ExtendedLayerId, EmbeddedLayerId),
            u32,
        >,
        is_first_tile_group: Option<bool>,
        class: OutputClass,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) -> FrameBoundary {
        let obu_type = obu.header.obu_type;
        let first_in_frame = state.unit.coded_frame.is_none();
        let delimiter_unreadable = !first_in_frame && is_first_tile_group.is_none();
        let boundary = Self::observe_frame(
            state,
            embedded_key,
            coded_frames_opened_for_embedded_layer,
            Some(obu_type),
            class,
            false,
            false, // tile-group OBUs carry the is_first_tile_group delimiter
            false, // tile-group output is derived from the per-frame header, not the type
            delimiter_unreadable,
            obu,
            report,
        );
        if first_in_frame && is_first_tile_group == Some(false) {
            report.push(frame_unit_error(
                "frame-unit/first-tile-group-flag",
                obu,
                "7.3.3",
                "the first tile OBU of a coded frame must have is_first_tile_group == 1".to_owned(),
            ));
        }
        boundary
    }

    /// Observes a frame-bearing OBU joining (or extending) the unit's coded frame: records
    /// the coded frame's identity / output class on the first OBU, and enforces the
    /// structural SEF-single-OBU and same-`obu_type` rules (independent of output class) on
    /// later OBUs. `obu_type_for_match` is the same-type rule's type (`None` for
    /// SEF/TIP/bridge, which use the OBU's own type). `is_no_delimiter_frame` marks a TIP /
    /// bridge OBU, whose same-type adjacency is unit-count-ambiguous
    /// ([`FrameBoundary::Ambiguous`]); `output_is_type_decided` distinguishes a TIP (routes
    /// the open frame's class to Unknown) from a bridge (keeps its type-decided non-output
    /// class, mirror line 470, so its § 7.3.4 BRT bound stays evaluable).
    ///
    /// `delimiter_unreadable` marks a tile-group OBU whose `is_first_tile_group` bit could
    /// not be read while a coded frame is open: the boundary is undecidable, so the structural
    /// continuation judgments are suppressed and the open frame's output class routes to
    /// Unknown. For bridge frames, the active sequence header decides whether the frame is
    /// the single-picture output form or the video-sequence non-output form.
    ///
    /// Returns this OBU's [`FrameBoundary`]: `OpensNewUnit` for the first OBU of a coded frame,
    /// `Ambiguous` for the undecidable boundary, `ContinuesUnit` for every decided continuation.
    #[allow(clippy::too_many_arguments)]
    fn observe_frame(
        state: &mut LayerState,
        embedded_key: (ExtendedLayerId, EmbeddedLayerId),
        coded_frames_opened_for_embedded_layer: &mut BTreeMap<
            (ExtendedLayerId, EmbeddedLayerId),
            u32,
        >,
        obu_type_for_match: Option<ObuType>,
        class: OutputClass,
        is_sef: bool,
        is_no_delimiter_frame: bool,
        output_is_type_decided: bool,
        delimiter_unreadable: bool,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) -> FrameBoundary {
        let obu_type = obu_type_for_match.unwrap_or(obu.header.obu_type);
        match state.unit.coded_frame {
            None => {
                state.unit.coded_frame = Some(CodedFrameState {
                    obu_type,
                    is_sef,
                    output: class,
                });
                if !state.unit.region_blind {
                    state.unit.region = Region::CodedFrame;
                }
                let opened = coded_frames_opened_for_embedded_layer
                    .entry(embedded_key)
                    .or_insert(0);
                *opened = opened.saturating_add(1);
                FrameBoundary::OpensNewUnit
            }
            Some(mut frame) => {
                if delimiter_unreadable {
                    if frame.output != OutputClass::Unknown {
                        frame.output = OutputClass::Unknown;
                        state.unit.coded_frame = Some(frame);
                    }
                    FrameBoundary::Ambiguous
                } else if frame.is_sef {
                    report.push(frame_unit_error(
                        "frame-unit/sef-single-obu",
                        obu,
                        "7.3.3",
                        "a SEF coded frame must consist of exactly one OBU; a non-SEF frame OBU \
                         follows the SEF in the same coded frame unit"
                            .to_owned(),
                    ));
                    FrameBoundary::ContinuesUnit
                } else if frame.obu_type != obu_type {
                    report.push(frame_unit_error(
                        "frame-unit/mixed-coded-frame-types",
                        obu,
                        "7.3.3",
                        format!(
                            "the OBUs of a coded frame must all share one obu_type; {} follows {}",
                            obu_type.spec_name(),
                            frame.obu_type.spec_name()
                        ),
                    ));
                    FrameBoundary::ContinuesUnit
                } else if is_no_delimiter_frame {
                    if !output_is_type_decided && frame.output != OutputClass::Unknown {
                        frame.output = OutputClass::Unknown;
                        state.unit.coded_frame = Some(frame);
                    }
                    FrameBoundary::Ambiguous
                } else {
                    FrameBoundary::ContinuesUnit
                }
            }
        }
    }

    /// Resolves a unit's deferred (output-class-dependent) check at the unit
    /// boundary: the § 7.3.4 non-output buffer-removal-timing zero-or-one bound. A
    /// coded *non-output* frame unit with two-or-more BRT OBUs is non-conforming; an
    /// output unit with two is conforming (mirror line 384 vs 452). An Unknown
    /// output class is not `NonOutput`, so the bound is silently dropped.
    fn resolve_unit(unit: &UnitState, report: &mut ValidationReport) {
        let Some(frame) = &unit.coded_frame else {
            return;
        };
        if frame.output == OutputClass::NonOutput
            && unit.brt_count >= 2
            && let Some(offset) = unit.second_brt_offset
        {
            report.push(
                Diagnostic::error(
                    "frame-unit/buffer-removal-timing-multiplicity",
                    "a coded non-output frame unit carries more than one \
                     OBU_BUFFER_REMOVAL_TIMING (zero or one permitted; an output frame unit \
                     permits more)"
                        .to_owned(),
                )
                .with_spec_section("7.3.4")
                .with_byte_offset(offset),
            );
        }
    }
}

/// Whether `role` is a frame-bearing OBU (the coded frame of its unit).
fn is_frame_role(role: SegRole) -> bool {
    matches!(
        role,
        SegRole::TileFrame { .. }
            | SegRole::SefFrame
            | SegRole::TipFrame { .. }
            | SegRole::BridgeFrame { .. }
    )
}

/// Whether `role` is a frame OBU of a type that carries **no in-band coded-frame
/// delimiter**: `OBU_LEADING_TIP` / `OBU_REGULAR_TIP` (mirror lines 400-401 /
/// 468-469) and `OBU_BRIDGE_FRAME` (mirror line 470). Unlike the tile-group types,
/// these are absent from the `is_first_tile_group` lists (mirror lines 404-411 /
/// 473-484), so the bitstream has no flag marking where one coded frame of such a
/// type ends and the next begins. SEF is excluded — § 7.3.3 makes a SEF its own
/// single-OBU coded frame, judged by the SEF-single-OBU rule in `observe_frame`.
fn is_no_delimiter_frame_role(role: SegRole) -> bool {
    matches!(role, SegRole::TipFrame { .. } | SegRole::BridgeFrame { .. })
}

/// Maps a parsed output flag to a classification (`None` → Unknown).
fn output_class(output: Option<bool>) -> OutputClass {
    match output {
        Some(true) => OutputClass::Output,
        Some(false) => OutputClass::NonOutput,
        None => OutputClass::Unknown,
    }
}

/// The **type-decided** output classification of a frame-bearing OBU, when its `obu_type` alone
/// settles it (AV2 § 7.3.3 / § 7.3.4): `Some(true)` for a SEF (`OBU_LEADING_SEF` /
/// `OBU_REGULAR_SEF` — the § 7.3.3 "Or" branch makes a SEF a coded *output* frame unit, mirror
/// line 417), `Some(false)` for `OBU_BRIDGE_FRAME` (it appears only in the § 7.3.4
/// coded-*non-output*-frame-unit list, mirror line 470). `None` for every other frame type
/// (CLK / OLK / `*_TILE_GROUP` / `SWITCH` / `RAS` / `*_TIP` / `BRIDGE`), whose class is
/// carried by the
/// `immediate_output_frame` / `implicit_output_frame` flags — they appear in *both* § 7.3.3 and
/// § 7.3.4.
///
/// The bridge class is instead derived from the active sequence header by
/// `ValidatorContext::bridge_output_class`, because a single-picture bridge is an output frame.
pub(crate) fn type_decided_output(obu_type: ObuType) -> Option<bool> {
    if obu_type.is_sef() { Some(true) } else { None }
}

/// Builds a `frame-unit/` presence-order error anchored at the offending OBU.
fn frame_unit_error(
    rule_id: &'static str,
    obu: &ObuEnvelope<'_>,
    spec_section: &'static str,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section(spec_section)
        .with_byte_offset(obu.offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use splot_core::obu::ObuHeader;

    /// A synthetic OBU envelope at (xlayer 0, mlayer 0, tlayer 0) for driving the
    /// segmenter directly. The payload is unused (the segmenter consumes the
    /// pre-parsed [`SegRole`]); the offset is the running OBU index so boundaries are
    /// distinguishable.
    fn obu(obu_type: ObuType, offset: u64) -> ObuEnvelope<'static> {
        ObuEnvelope {
            offset: ByteOffset::new(offset),
            size: 1,
            header: ObuHeader {
                has_header_extension: true,
                obu_type,
                temporal_layer_id: TemporalLayerId::from_bits(0),
                embedded_layer_id: EmbeddedLayerId::from_bits(0),
                extended_layer_id: ExtendedLayerId::from_bits(0),
                header_size_bytes: 2,
            },
            payload: &[],
        }
    }

    #[test]
    fn same_type_adjacent_bridge_reports_ambiguous_boundary() {
        let mut seg = FrameUnitSegmenter::default();
        let mut r = ValidationReport::new();
        let first = seg.observe(
            &obu(ObuType::BridgeFrame, 0),
            SegRole::BridgeFrame {
                output: Some(false),
            },
            &mut r,
        );
        let second = seg.observe(
            &obu(ObuType::BridgeFrame, 1),
            SegRole::BridgeFrame {
                output: Some(false),
            },
            &mut r,
        );
        assert_eq!(
            first,
            Some(FrameBoundary::OpensNewUnit),
            "the first bridge opens a coded frame unit"
        );
        assert_eq!(
            second,
            Some(FrameBoundary::Ambiguous),
            "a same-type no-delimiter bridge adjacency is unit-count-ambiguous, so the \
             second bridge must report Ambiguous (not ContinuesUnit)"
        );
        assert!(
            r.errors().next().is_none(),
            "the ambiguous bridge adjacency emits no structural diagnostic (same type, no \
             SEF); report: {r}"
        );
    }

    #[test]
    fn same_type_adjacent_bridge_keeps_type_decided_non_output_class() {
        let mut seg = FrameUnitSegmenter::default();
        let mut r = ValidationReport::new();
        seg.observe(
            &obu(ObuType::BufferRemovalTiming, 0),
            SegRole::BufferRemovalTiming,
            &mut r,
        );
        seg.observe(
            &obu(ObuType::BufferRemovalTiming, 1),
            SegRole::BufferRemovalTiming,
            &mut r,
        );
        seg.observe(
            &obu(ObuType::BridgeFrame, 2),
            SegRole::BridgeFrame {
                output: Some(false),
            },
            &mut r,
        );
        let second = seg.observe(
            &obu(ObuType::BridgeFrame, 3),
            SegRole::BridgeFrame {
                output: Some(false),
            },
            &mut r,
        );
        assert_eq!(second, Some(FrameBoundary::Ambiguous));
        seg.finish(&mut r);
        assert!(
            r.errors()
                .any(|d| d.rule_id == "frame-unit/buffer-removal-timing-multiplicity"),
            "the bridge unit keeps its type-decided non-output class across the ambiguous \
             boundary, so the § 7.3.4 BRT multiplicity bound must still fire; report: {r}"
        );
    }

    #[test]
    fn different_type_adjacent_no_delimiter_frames_split_decidedly() {
        let mut seg = FrameUnitSegmenter::default();
        let mut r = ValidationReport::new();
        seg.observe(
            &obu(ObuType::BridgeFrame, 0),
            SegRole::BridgeFrame {
                output: Some(false),
            },
            &mut r,
        );
        let second = seg.observe(
            &obu(ObuType::RegularTip, 1),
            SegRole::TipFrame { output: Some(true) },
            &mut r,
        );
        assert_eq!(
            second,
            Some(FrameBoundary::OpensNewUnit),
            "a different-type no-delimiter frame is a decidable boundary, so it opens a \
             new unit (not Ambiguous)"
        );
    }

    #[test]
    fn commits_pending_ref_update_matches_observe_commit_decision() {
        fn commit_for(boundary: Option<FrameBoundary>) -> bool {
            matches!(
                boundary,
                Some(FrameBoundary::OpensNewUnit | FrameBoundary::Ambiguous)
            )
        }

        let mut peek_seg = FrameUnitSegmenter::default();
        let mut authority_seg = FrameUnitSegmenter::default();
        let mut r = ValidationReport::new();
        let bridge = SegRole::BridgeFrame {
            output: Some(false),
        };
        peek_seg.observe(&obu(ObuType::BridgeFrame, 0), bridge, &mut r);
        authority_seg.observe(&obu(ObuType::BridgeFrame, 0), bridge, &mut r);
        let second = obu(ObuType::BridgeFrame, 1);
        let peek = peek_seg.commits_pending_ref_update(&second, bridge);
        let boundary = authority_seg.observe(&second, bridge, &mut r);
        assert_eq!(boundary, Some(FrameBoundary::Ambiguous));
        assert!(
            peek,
            "a same-type no-delimiter bridge opener is an Ambiguous boundary that commits \
             the prior frame's §7.23 update; the peek must agree (codex F1)"
        );
        assert_eq!(peek, commit_for(boundary));

        let peek_seg = FrameUnitSegmenter::default();
        let mut authority_seg = FrameUnitSegmenter::default();
        let first = obu(ObuType::BridgeFrame, 0);
        let bridge = SegRole::BridgeFrame {
            output: Some(false),
        };
        let peek = peek_seg.commits_pending_ref_update(&first, bridge);
        let boundary = authority_seg.observe(&first, bridge, &mut r);
        assert_eq!(boundary, Some(FrameBoundary::OpensNewUnit));
        assert_eq!(peek, commit_for(boundary));
        assert!(peek);

        let mut peek_seg = FrameUnitSegmenter::default();
        let mut authority_seg = FrameUnitSegmenter::default();
        let open = SegRole::TileFrame {
            is_first_tile_group: Some(true),
            output: Some(true),
        };
        let cont = SegRole::TileFrame {
            is_first_tile_group: Some(false),
            output: Some(true),
        };
        peek_seg.observe(&obu(ObuType::RegularTileGroup, 0), open, &mut r);
        authority_seg.observe(&obu(ObuType::RegularTileGroup, 0), open, &mut r);
        let next = obu(ObuType::RegularTileGroup, 1);
        let peek = peek_seg.commits_pending_ref_update(&next, cont);
        let boundary = authority_seg.observe(&next, cont, &mut r);
        assert_eq!(boundary, Some(FrameBoundary::ContinuesUnit));
        assert!(
            !peek,
            "a decided same-type tile continuation does not commit the pending update"
        );
        assert_eq!(peek, commit_for(boundary));

        let mut peek_seg = FrameUnitSegmenter::default();
        let mut authority_seg = FrameUnitSegmenter::default();
        let unreadable = SegRole::TileFrame {
            is_first_tile_group: None,
            output: Some(true),
        };
        peek_seg.observe(&obu(ObuType::RegularTileGroup, 0), open, &mut r);
        authority_seg.observe(&obu(ObuType::RegularTileGroup, 0), open, &mut r);
        let next = obu(ObuType::RegularTileGroup, 1);
        let peek = peek_seg.commits_pending_ref_update(&next, unreadable);
        let boundary = authority_seg.observe(&next, unreadable, &mut r);
        assert_eq!(boundary, Some(FrameBoundary::Ambiguous));
        assert!(peek);
        assert_eq!(peek, commit_for(boundary));
    }
}
