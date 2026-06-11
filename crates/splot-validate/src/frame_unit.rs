// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coded-frame-unit segmentation (AV2 v1.0.0 § 7.3.3 / § 7.3.4 / § 7.3.5).
//!
//! The validator partitions each `(obu_xlayer_id, obu_mlayer_id, obu_tlayer_id)`
//! triple's consecutive OBUs in a temporal unit into *coded frame units* and
//! enforces the § 7.3.3 (output) / § 7.3.4 (non-output) presence order. Both
//! grammars share the region order
//!
//! 1. content interpretation — zero or one (`§ 7.3.3`/`§ 7.3.4` first bullet),
//! 2. multi-frame headers — zero or more,
//! 3. the **pre-frame region** — buffer-removal-timing, quantization-matrix,
//!    film-grain, and prefix metadata (`metadata_is_suffix == 0`), present in any
//!    order. The asymmetry the two sections encode: a coded **non-output** frame
//!    unit allows **zero or one** buffer-removal-timing OBU (mirror
//!    `07-decoding-process.md` line 452), while a coded **output** frame unit
//!    allows **zero or more** (mirror line 384),
//! 4. exactly one **coded frame** — either one-or-more same-`obu_type` tile OBUs
//!    with the `is_first_tile_group` 1-then-0 rule (mirror lines 413-414 /
//!    486-487), or exactly one SEF (`§ 7.3.3` "Or: one OBU of either type
//!    OBU_LEADING_SEF or OBU_REGULAR_SEF", mirror line 417), and
//! 5. the **suffix-metadata tail** — suffix metadata (`metadata_is_suffix == 1`),
//!    in any order (mirror lines 420-431 / 488-494).
//!
//! `OBU_PADDING` is position-free within a coded frame unit (mirror lines 433-434 /
//! 496-497), so it never advances a region or starts a unit.
//!
//! ## Output classification and the Unknown invariant
//!
//! Output vs non-output classification selects the § 7.3.3 / § 7.3.4 grammar and
//! the buffer-removal-timing multiplicity bound. A SEF is always an output coded
//! frame (it sits only in the § 7.3.3 grammar), `OBU_BRIDGE_FRAME` is always a
//! non-output coded frame (it appears only in the § 7.3.4 list, mirror line 470),
//! and the remaining frame types carry `immediate_output_frame` /
//! `implicit_output_frame` from the core frame-header parser. When that
//! classification is undecidable (an unsupported frame-header parse path), the
//! coded frame's output class is [`OutputClass::Unknown`] and the **output-class-
//! derived judgment** — the § 7.3.4 non-output BRT bound and the grammar branch —
//! is silently dropped (PRs #46-#51: undecidable output classification never
//! fires).
//!
//! The **structural** presence-order facts — region order, duplicate CI, the
//! first-tile-group flag, mixed coded-frame types, the SEF single-OBU rule,
//! suffix-before-coded-frame, and the § 7.3.8.10 first-coded-frame-unit CI rule —
//! are decidable from OBU types and the `is_first_tile_group` / `metadata_is_suffix`
//! bits alone, independent of the output classification, so they fire eagerly even
//! when the output class is Unknown (the prompt's "presence-order checks that ARE
//! decidable mid-unit may fire eagerly if sound").
//!
//! One distinct undecidability is the **region pointer** itself: a metadata OBU
//! whose `metadata_is_suffix` bit cannot be read has no determinable region, so the
//! unit's region progression is no longer reliable. That sets [`UnitState::region_blind`],
//! which suppresses the remaining *region-order* structural checks for the unit
//! (their region comparisons would be against an untrustworthy pointer) while still
//! tracking the coded frame for the BRT resolution.
//!
//! A second is the **coded-frame boundary between same-type no-delimiter frames**.
//! `OBU_LEADING_TIP` / `OBU_REGULAR_TIP` / `OBU_BRIDGE_FRAME` carry no
//! `is_first_tile_group` flag (they are absent from the mirror lines 404-411 /
//! 473-484 first-tile-group lists), so when one follows a *completed* coded frame
//! ([`Region::CodedFrame`]) the segmenter splits only on a **decidable** cue:
//!
//! - a **different** `obu_type` cannot share the open coded frame ("the OBUs of the
//!   coded frame have the same obu_type", mirror lines 392-393 / 459-461), so it
//!   begins a new coded frame unit (`starts_new_unit`) — silently, not a
//!   `mixed-coded-frame-types` finding,
//! - a **same** `obu_type` is genuinely undecidable (a later OBU of the one coded
//!   frame, or the first of a new same-type one), so the OBU stays in the open
//!   coded frame and its output class is set to [`OutputClass::Unknown`] — never a
//!   split guess, never a structural diagnostic, dropping only the output-class-
//!   dependent BRT bound (the Unknown-soundness invariant). Tile-group types are
//!   unaffected: their `is_first_tile_group` flag is the in-band delimiter. A
//!   **bridge** frame is the exception within this same-type case: its output class
//!   is NonOutput *by type* (mirror line 470), so the frame-vs-unit ambiguity cannot
//!   change it — both interpretations are non-output. The type-decided class is kept
//!   (it is not dropped to Unknown), so the § 7.3.4 non-output BRT bound stays
//!   evaluable across back-to-back same-type bridge units. Only `OBU_LEADING_TIP` /
//!   `OBU_REGULAR_TIP`, whose output comes from a per-frame header parse, route to
//!   Unknown.
//!
//! ## Resolution timing
//!
//! The structural facts fire eagerly. The one output-class-dependent fact — the
//! § 7.3.4 non-output BRT multiplicity bound — is resolved at the unit boundary
//! (the established TU-end timing for whole-unit attribution), once the coded
//! frame's output class is known.
//!
//! ## Boundary signal for the CELU layer
//!
//! The segmenter is the single source of truth for coded-frame-unit boundaries
//! (§ 7.3.6). [`Self::observe`](FrameUnitSegmenter::observe) returns each frame-bearing
//! OBU's [`FrameBoundary`] — [`FrameBoundary::OpensNewUnit`] for the first OBU of a new
//! coded frame unit, [`FrameBoundary::ContinuesUnit`] for a decided continuation, and
//! [`FrameBoundary::Ambiguous`] for the undecidable same-type-no-delimiter /
//! unreadable-delimiter cases. The
//! [`CodedExtendedLayerTracker`](crate::celu::CodedExtendedLayerTracker) consumes this
//! signal rather than re-deriving boundaries, so the two layers agree by construction
//! (e.g. on `TIP, BRT, TIP` the BRT head splits the two TIPs into two units).

use std::collections::{BTreeMap, BTreeSet};

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
    /// `OBU_BRIDGE_FRAME`: always a non-output coded frame (mirror line 470); it
    /// appears only in the § 7.3.4 list. Carries no `is_first_tile_group`.
    BridgeFrame,
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
    /// The `(xlayer, mlayer)` embedded layers whose first coded frame of the
    /// temporal unit has been observed — i.e. whose first coded frame unit has
    /// begun (§ 7.3.8.10 first-coded-frame-unit CI rule).
    first_coded_unit_started: BTreeSet<(ExtendedLayerId, EmbeddedLayerId)>,
    /// Number of distinct coded frames *opened* for each `(xlayer, mlayer)` embedded
    /// layer in the current temporal unit, counted when each coded frame's first
    /// frame-bearing OBU is seen. Each coded frame belongs to its own coded frame unit
    /// (§ 7.3.3 / § 7.3.4: one coded frame per unit), so the count of *completed* units
    /// is this minus the currently-open one (see
    /// [`Self::completed_units_for_embedded_layer`]). Counting at the coded-frame open —
    /// in `observe_frame`, keyed by `(xlayer, mlayer)` — makes it **tlayer-agnostic**:
    /// the § 7.3.8.10 "first coded frame unit of each embedded layer within a temporal
    /// unit" scope is *not* keyed by `obu_tlayer_id` (mirror line 880), so a second
    /// coded frame at a *different* `obu_tlayer_id` of the same embedded layer counts as
    /// a later unit even though it opens in a fresh per-triple state that
    /// `starts_new_unit` never observes.
    coded_frames_opened_for_embedded_layer: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), u32>,
}

impl FrameUnitSegmenter {
    /// Resets all per-temporal-unit state. Called at each global temporal
    /// delimiter (AV2 § 7.3.7): a coded frame unit does not span temporal units.
    /// Any still-open unit's deferred (output-class-dependent) checks are resolved
    /// before the reset; the eager structural checks have already fired.
    pub(crate) fn reset_temporal_unit(&mut self, report: &mut ValidationReport) {
        // A temporal-unit boundary definitively seals every open unit: the temporal
        // unit has ended (a coded frame unit cannot span it, § 7.3.7), so a head-only
        // unit is a hard § 7.3.3 / § 7.3.4 violation (it can no longer receive a coded
        // frame).
        self.flush_open_units(FlushKind::TemporalUnitBoundary, report);
        self.layers.clear();
        self.first_coded_unit_started.clear();
        self.coded_frames_opened_for_embedded_layer.clear();
    }

    /// Number of coded frame units this `(xlayer, mlayer)` embedded layer has
    /// **completed** in the current temporal unit (AV2 § 7.3.8.10 scope, across
    /// temporal layers). Derived as the number of distinct coded frames opened for the
    /// embedded layer minus the currently-open one: each coded frame is its own unit
    /// (§ 7.3.3 / § 7.3.4), so a unit is *completed* once a later coded frame has begun.
    /// While a unit's coded frame and its suffix-metadata tail (§ 7.3.3) are still open
    /// — with no later coded frame yet — this count does not include it. Consumers (the
    /// § 6.16.5 / § 6.16.6 first-coded-picture lateness predicate) use a count of `0` to
    /// mean "still within the layer's first coded frame unit of this temporal unit": a
    /// suffix metadata after the frame's OBUs but inside the same unit is not yet in a
    /// later unit. Because the open is counted per `(xlayer, mlayer)` (not per
    /// `(xlayer, mlayer, tlayer)` triple), a second coded frame at a different
    /// `obu_tlayer_id` correctly counts as a later unit (mirror line 880).
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
        // At the end of the bitstream a head-only unit could equally be a truncated
        // capture (the coded frame's OBUs were cut off) as a malformed unit, so it is
        // reported at a lower (warning) severity than the same run at an internal
        // temporal-unit boundary.
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
        // A global OBU is never part of a coded frame unit (those live at a concrete
        // xlayer/mlayer/tlayer, § 7.3.3 / § 7.3.4). Global OBUs are ordered by the
        // § 7.3.7 temporal-unit machine, not here.
        if obu.header.extended_layer_id.is_global() {
            return None;
        }
        // Padding is position-free (mirror lines 433-434 / 496-497).
        if matches!(role, SegRole::Padding) {
            return None;
        }

        let key = (
            obu.header.extended_layer_id,
            obu.header.embedded_layer_id,
            obu.header.temporal_layer_id,
        );
        let embedded_key = (obu.header.extended_layer_id, obu.header.embedded_layer_id);
        let first_coded_unit_started = &mut self.first_coded_unit_started;
        let coded_frames_opened_for_embedded_layer =
            &mut self.coded_frames_opened_for_embedded_layer;
        let state = self.layers.entry(key).or_insert_with(|| LayerState {
            unit: UnitState::new(),
        });

        Self::observe_in_layer(
            state,
            embedded_key,
            first_coded_unit_started,
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
        first_coded_unit_started: &mut BTreeSet<(ExtendedLayerId, EmbeddedLayerId)>,
        coded_frames_opened_for_embedded_layer: &mut BTreeMap<
            (ExtendedLayerId, EmbeddedLayerId),
            u32,
        >,
        obu: &ObuEnvelope<'_>,
        role: SegRole,
        report: &mut ValidationReport,
    ) -> Option<FrameBoundary> {
        // A head OBU (CI / MFH / pre-frame) or a new coded frame after a *completed*
        // coded frame (or its suffix tail) starts a new coded frame unit (the prior
        // unit's coded frame and tail are done). § 7.3.3 / § 7.3.4 require every
        // coded frame unit to contain a coded frame, so a head / frame OBU after one
        // is unambiguously a new unit — the grammar permits back-to-back units, so a
        // pre-frame OBU after a coded frame is NOT a same-unit misplacement. (The
        // per-embedded-layer unit count is incremented at the coded-frame open in
        // `observe_frame`, not here, so it stays tlayer-agnostic — see
        // `coded_frames_opened_for_embedded_layer`.)
        if Self::starts_new_unit(&state.unit, role, obu.header.obu_type) {
            Self::resolve_unit(&state.unit, report);
            state.unit = UnitState::new();
        }

        // A head OBU (CI / MFH / pre-frame BRT / QM / FGM / prefix metadata) obligates
        // the unit to receive a coded frame (§ 7.3.3 / § 7.3.4); record its offset so a
        // head-only unit (the head run ends at a temporal-unit / bitstream boundary
        // with no coded frame) can be reported. An *unreadable*-suffix metadata
        // (`Some(false) | None` minus the readable prefix) is NOT recorded as a head:
        // its region (prefix head vs suffix tail) is undecidable, so obligating a coded
        // frame would risk a head-only-unit false positive; it stays region-blind in
        // whatever unit it lands in (see `observe_metadata`).
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
                    first_coded_unit_started,
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
                first_coded_unit_started,
                coded_frames_opened_for_embedded_layer,
                is_first_tile_group,
                output_class(output),
                obu,
                report,
            )),
            SegRole::SefFrame => Some(Self::observe_frame(
                state,
                embedded_key,
                first_coded_unit_started,
                coded_frames_opened_for_embedded_layer,
                None,
                OutputClass::Output,
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
                first_coded_unit_started,
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
            SegRole::BridgeFrame => Some(Self::observe_frame(
                state,
                embedded_key,
                first_coded_unit_started,
                coded_frames_opened_for_embedded_layer,
                None,
                OutputClass::NonOutput,
                false,
                true,  // OBU_BRIDGE_FRAME carries no in-band coded-frame delimiter
                true,  // a bridge frame is non-output by type (mirror line 470), not a header parse
                false, // a bridge frame carries no is_first_tile_group bit to be unreadable
                obu,
                report,
            )),
            SegRole::Padding => None,
        }
    }

    /// Whether `role` begins a new coded frame unit given the current unit's
    /// region.
    ///
    /// A coded frame consists of *one or more* OBUs (mirror lines 391-393 /
    /// 459-461), so a frame OBU while still in [`Region::CodedFrame`] is normally a
    /// *continuation* of the current coded frame — its same-`obu_type` / SEF /
    /// first-tile-group judgment happens in `observe_frame`, not a new unit. A new
    /// unit begins when:
    ///
    /// - a **head** OBU (CI / MFH / pre-frame) follows a completed coded frame
    ///   ([`Region::CodedFrame`]) or its suffix tail ([`Region::SuffixTail`]); the
    ///   grammar permits back-to-back units, so this is unambiguous,
    /// - any **frame** OBU follows the suffix tail ([`Region::SuffixTail`]) — the
    ///   prior unit is fully complete (coded frame + tail), or
    /// - a **tile** OBU with `is_first_tile_group == 1` arrives while a coded frame
    ///   is open ([`Region::CodedFrame`]). § 7.3.6 permits back-to-back coded frame
    ///   units in one coded extended layer unit, and the first OBU of a coded frame
    ///   shall have `is_first_tile_group == 1` (mirror lines 413-414 / 486-487), so
    ///   a tile OBU re-asserting that flag while a frame is open *starts the next
    ///   unit* (closing the current one) rather than being an out-of-place
    ///   non-first tile. A tile OBU with `is_first_tile_group == 0` (or an
    ///   undecidable flag) continues the open coded frame, where the same-type /
    ///   first-tile-group continuation rules apply.
    /// - a **SEF** OBU (`OBU_LEADING_SEF` / `OBU_REGULAR_SEF`) arrives while a coded
    ///   frame is open ([`Region::CodedFrame`]). A SEF is the complete coded-frame
    ///   alternative of its unit — "Or: one OBU of either type OBU_LEADING_SEF or
    ///   OBU_REGULAR_SEF" (mirror line 417), exactly one OBU — so it can never be a
    ///   continuation of an already-open coded frame. Like a flag-1 tile OBU it
    ///   *starts the next coded frame unit* (§ 7.3.6 back-to-back units), whether the
    ///   open frame is a SEF (SEF after SEF) or tile OBUs (SEF after a completed tile
    ///   coded frame). The genuine `sef-single-obu` violation is the inverse — a
    ///   *non-SEF* frame OBU claiming to continue a SEF coded frame — which does not
    ///   split and is judged in `observe_frame`.
    /// - a **no-delimiter frame** OBU (`OBU_LEADING_TIP` / `OBU_REGULAR_TIP` /
    ///   `OBU_BRIDGE_FRAME`) of a **different `obu_type`** than the open coded frame
    ///   arrives while a coded frame is open ([`Region::CodedFrame`]). The OBUs of a
    ///   coded frame all share one `obu_type` (mirror lines 392-393 / 460-461), and
    ///   these types carry no `is_first_tile_group` delimiter (they are absent from
    ///   the mirror lines 404-411 / 473-484 first-tile-group lists), so a type change
    ///   cannot be a same-frame continuation — it can only begin a new coded frame
    ///   unit (§ 7.3.6 back-to-back units). That boundary is *decidable* from the type
    ///   change alone, so it splits silently rather than misreporting
    ///   `mixed-coded-frame-types`. A *same*-`obu_type` no-delimiter OBU is instead
    ///   left to `observe_frame`: with no in-band delimiter the validator cannot
    ///   decide whether it continues the one coded frame or begins a new same-type
    ///   one, so it stays in the open coded frame (the undecidable case routes to
    ///   Unknown — no split guess, no diagnostic; see `observe_frame`).
    ///
    /// Suffix metadata never starts a unit (its placement is judged in
    /// `observe_metadata`).
    fn starts_new_unit(unit: &UnitState, role: SegRole, obu_type: ObuType) -> bool {
        // An *unreadable*-suffix metadata (`is_suffix == None`) is excluded: its region
        // (prefix head vs suffix tail) is undecidable, so it must not close a valid open
        // unit (which could cascade false positives downstream). It instead stays in the
        // current unit and sets region-blind (see `observe_metadata`).
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
        // A SEF is the complete coded-frame alternative of its unit ("Or: one OBU of
        // either type OBU_LEADING_SEF or OBU_REGULAR_SEF", mirror line 417): it is
        // exactly one OBU and is never a continuation of an already-open coded frame.
        // So a SEF arriving while a coded frame is open ([`Region::CodedFrame`]) starts
        // the next coded frame unit (§ 7.3.6 back-to-back units), exactly like a flag-1
        // tile OBU — whether the open frame is a SEF (SEF after SEF) or tile OBUs (SEF
        // after a completed tile coded frame). The genuine sef-single-obu violation —
        // a *non-SEF* frame OBU claiming to continue a SEF coded frame — does not split
        // and is judged in `observe_frame`.
        let starts_next_sef = matches!(role, SegRole::SefFrame);
        // A no-in-band-delimiter frame OBU (TIP / bridge) whose obu_type differs from
        // the open coded frame begins a new unit: the type change is a decidable
        // coded-frame boundary (mirror lines 392-393 / 460-461; these types carry no
        // is_first_tile_group flag, so the spec offers no continuation marker).
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
        first_coded_unit_started: &BTreeSet<(ExtendedLayerId, EmbeddedLayerId)>,
        coded_frames_opened_for_embedded_layer: &BTreeMap<(ExtendedLayerId, EmbeddedLayerId), u32>,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let unit = &mut state.unit;
        // § 7.3.8.10: a CI may appear only in the *first coded frame unit of each
        // embedded layer within the temporal unit* (mirror line 880) — a scope not
        // keyed by `obu_tlayer_id`. This CI is outside that first unit when either:
        //
        // - the embedded layer already *completed* a coded frame unit in this temporal
        //   unit (a later coded frame has begun, so `coded_frames_opened > 1`, tracked
        //   across temporal layers), or
        // - the embedded layer's first coded frame has already begun
        //   (`first_coded_unit_started`) while this CI's *current* unit holds no coded
        //   frame yet — i.e. the first coded frame lives in a different (earlier or
        //   other-tlayer) unit, so this CI heads a later unit. (A legitimate first-unit
        //   CI precedes that layer's first coded frame, so the flag is still unset.)
        //
        // A head OBU after a coded frame always splits the unit first (the current
        // unit's `coded_frame` is therefore `None` by the time `observe_ci` runs), so
        // the second clause catches a CI that started a fresh later unit even within
        // the same temporal layer. Independent of region order, so reported even when
        // region-blind.
        let completed = coded_frames_opened_for_embedded_layer
            .get(&embedded_key)
            .copied()
            .unwrap_or(0)
            .saturating_sub(1);
        let in_later_unit = completed > 0
            || (unit.coded_frame.is_none() && first_coded_unit_started.contains(&embedded_key));
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
            // Unreadable suffix bit: the region is undecidable, so the region pointer
            // is no longer reliable — set region-blind rather than guess (the metadata
            // syntax check reports the structural error).
            None => unit.region_blind = true,
            Some(false) => {
                // Prefix metadata: pre-frame region.
                if !unit.region_blind {
                    unit.region = Region::PreFrame;
                }
            }
            Some(true) => {
                // Suffix metadata: the tail, after the coded frame. Suffix before the
                // coded frame is out of order (structural, region-pointer-based).
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
        first_coded_unit_started: &mut BTreeSet<(ExtendedLayerId, EmbeddedLayerId)>,
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
        // A tile-group OBU whose is_first_tile_group bit could not be read, arriving
        // while a coded frame is already open, has an undecidable continue-vs-next-unit
        // boundary (that bit is the delimiter, mirror lines 413-414 / 486-487). The
        // structural continuation judgments in `observe_frame` are suppressed and the
        // open frame routes to Unknown rather than guessing. When this is the *first*
        // tile of the unit (`first_in_frame`) there is no open frame to continue, so
        // the flag is just opening a new coded frame and the ambiguity does not arise.
        let delimiter_unreadable = !first_in_frame && is_first_tile_group.is_none();
        let boundary = Self::observe_frame(
            state,
            embedded_key,
            first_coded_unit_started,
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
        // The first-tile-group flag is decidable from the parsed bit alone,
        // independent of the output class (mirror line 413-414 / 486-487): the first
        // tile OBU of a coded frame shall have is_first_tile_group == 1.
        //
        // Under § 7.3.6 back-to-back-unit splitting, a tile OBU with
        // `is_first_tile_group == 1` arriving while a coded frame is open *starts the
        // next coded frame unit* (`starts_new_unit`), so this OBU is always the first
        // OBU of its (possibly freshly opened) coded frame when the flag is 1. The only
        // remaining `is_first_tile_group` violation is therefore a *first* tile OBU
        // carrying flag 0. A non-first tile OBU continuing an open coded frame must
        // have flag 0 (or an undecidable flag) — a flag-1 continuation cannot reach
        // here, having split into a new unit — so the former "non-first tile with flag
        // 1" branch is unreachable and removed.
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

    /// Observes a frame-bearing OBU joining (or extending) the unit's coded frame:
    /// records the coded frame's identity / output class on the first OBU, and
    /// enforces the SEF-single-OBU and same-`obu_type` rules on later OBUs. These
    /// are structural (type / SEF identity), so they fire independent of the output
    /// class. `obu_type_for_match` is the type used for the same-type rule (`None`
    /// for SEF/TIP/bridge, which use the OBU's own type). `is_no_delimiter_frame`
    /// marks a TIP / bridge OBU (no in-band coded-frame delimiter), so a same-type
    /// continuation normally routes the open coded frame's output class to Unknown
    /// — unless `output_is_type_decided` is set (a bridge frame, always non-output by
    /// type, mirror line 470), in which case the frame-vs-unit ambiguity cannot change
    /// the output class, so the type-decided class (and the § 7.3.4 BRT bound it
    /// gates) is preserved.
    ///
    /// `delimiter_unreadable` marks a tile-group OBU whose `is_first_tile_group` bit
    /// could not be read. While a coded frame is open that bit is *the* cue that would
    /// decide whether this OBU continues the open coded frame or begins the next coded
    /// frame unit (mirror lines 413-414 / 486-487); without it the boundary is
    /// undecidable, so the structural continuation judgments (`sef-single-obu`,
    /// `mixed-coded-frame-types`) must be suppressed — they would rest on a guess that
    /// the OBU continues the open frame — and the open frame's output class routes to
    /// Unknown rather than guessing which frame's output applies.
    ///
    /// Returns this OBU's [`FrameBoundary`]: [`FrameBoundary::OpensNewUnit`] when it is
    /// the first OBU of a coded frame (the `None` arm), [`FrameBoundary::Ambiguous`]
    /// when the continue-vs-next-unit boundary is undecidable (the same-type
    /// no-delimiter TIP case, or an unreadable tile-group delimiter), and
    /// [`FrameBoundary::ContinuesUnit`] for every decided continuation of the open
    /// coded frame (including the out-of-order `sef-single-obu` / `mixed-coded-frame-types`
    /// OBUs the segmenter keeps in the open frame, and a same-type bridge whose output is
    /// type-decided).
    #[allow(clippy::too_many_arguments)]
    fn observe_frame(
        state: &mut LayerState,
        embedded_key: (ExtendedLayerId, EmbeddedLayerId),
        first_coded_unit_started: &mut BTreeSet<(ExtendedLayerId, EmbeddedLayerId)>,
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
                first_coded_unit_started.insert(embedded_key);
                // A coded frame is opening for this embedded layer. Count it (per
                // `(xlayer, mlayer)`, tlayer-agnostic — mirror line 880) so the
                // completed-unit count and the § 7.3.8.10 / § 6.16.5 / § 6.16.6 consumers
                // see a later coded frame at *any* temporal layer of the embedded layer
                // as a new unit. The `None` arm is reached once per coded frame (the unit
                // resets `coded_frame` to `None` between frames), so this counts distinct
                // coded frames, not their continuation OBUs.
                let opened = coded_frames_opened_for_embedded_layer
                    .entry(embedded_key)
                    .or_insert(0);
                *opened = opened.saturating_add(1);
                FrameBoundary::OpensNewUnit
            }
            Some(mut frame) => {
                if delimiter_unreadable {
                    // A tile-group OBU whose `is_first_tile_group` bit could not be read,
                    // arriving while a coded frame is open. That bit is exactly the
                    // delimiter that would decide whether this OBU continues the open
                    // coded frame or begins the next coded frame unit (mirror lines
                    // 413-414 / 486-487). With it unreadable the boundary is undecidable,
                    // so the structural continuation judgments below (`sef-single-obu`,
                    // `mixed-coded-frame-types`) — which all assume the OBU *continues*
                    // the open frame — would rest on a guess and must be suppressed. The
                    // OBU stays in the open coded frame (splitting would equally be a
                    // guess), and the open frame's output class routes to Unknown: the
                    // § 7.3.4 BRT bound would otherwise rest on a guess of which frame's
                    // output applies. The metadata/tile-group syntax check reports the
                    // unreadable bit itself.
                    if frame.output != OutputClass::Unknown {
                        frame.output = OutputClass::Unknown;
                        state.unit.coded_frame = Some(frame);
                    }
                    // The continue-vs-next-unit boundary is undecidable (the unreadable
                    // delimiter bit was the only cue), so the CELU layer must poison its
                    // unit-count-dependent judgments for this embedded layer.
                    FrameBoundary::Ambiguous
                } else if frame.is_sef {
                    // A SEF coded frame is exactly one OBU (mirror line 417), and a SEF
                    // is the complete coded-frame alternative of its unit, so a SEF can
                    // never *continue* an open coded frame: a SEF arriving while a frame
                    // is open already started a new unit (`starts_new_unit`), so the
                    // incoming OBU here is never a SEF. The remaining violation is the
                    // inverse — a *non-SEF* frame OBU (a tile-group continuation claim,
                    // is_first_tile_group == 0) following the SEF in the same unit: the
                    // SEF is already the unit's complete coded frame, so the extra OBU
                    // cannot belong to it. (An *unreadable*-delimiter tile OBU is excluded
                    // above — it is undecidable, not a continuation claim.)
                    report.push(frame_unit_error(
                        "frame-unit/sef-single-obu",
                        obu,
                        "7.3.3",
                        "a SEF coded frame must consist of exactly one OBU; a non-SEF frame OBU \
                         follows the SEF in the same coded frame unit"
                            .to_owned(),
                    ));
                    // The offending OBU is kept in the open (SEF) coded frame, so for the
                    // CELU layer it does not open a new frame unit.
                    FrameBoundary::ContinuesUnit
                } else if frame.obu_type != obu_type {
                    // Mixed frame OBU types in one coded frame (mirror lines 392-393 /
                    // 459-461: "the OBUs of the coded frame have the same obu_type").
                    // A no-delimiter (TIP / bridge) different-type OBU never reaches
                    // here — `starts_new_unit` already split it into its own unit (the
                    // type change is a decidable boundary). So this branch fires only
                    // for a tile-group OBU making an explicit in-band continuation claim
                    // (is_first_tile_group == 0, a *readable* flag) against a mismatched
                    // open type; an unreadable flag is undecidable and handled above.
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
                    // The mismatched OBU is kept in the open coded frame (the explicit
                    // in-band flag-0 claim), so for the CELU layer it continues the unit.
                    FrameBoundary::ContinuesUnit
                } else if is_no_delimiter_frame && !output_is_type_decided {
                    // Same-`obu_type` no-delimiter (TIP) OBU continuing an open coded
                    // frame of that type: the spec gives these types no
                    // is_first_tile_group flag (mirror lines 404-411 / 473-484 omit
                    // them), so the validator cannot decide whether this OBU is a later
                    // OBU of the one coded frame ("one or more OBUs", mirror lines
                    // 391-393 / 459-461) or the first OBU of a new same-type coded frame.
                    // It stays in the open coded frame (no false split), but the output
                    // class becomes Unknown: an output-class-dependent judgment (the
                    // § 7.3.4 non-output BRT bound) would otherwise rest on a guess of
                    // which frame's output applies. (No structural diagnostic fires —
                    // same type, not a SEF.)
                    //
                    // A bridge frame (`output_is_type_decided`) is excluded: its output
                    // class is NonOutput *by type* (mirror line 470), not from an
                    // ambiguous per-frame header parse, so neither interpretation (one
                    // bridge coded frame or two back-to-back same-type bridge units)
                    // changes the class. The § 7.3.4-derived non-output BRT bound stays
                    // evaluable, so the type-decided class must NOT be dropped to Unknown.
                    if frame.output != OutputClass::Unknown {
                        frame.output = OutputClass::Unknown;
                        state.unit.coded_frame = Some(frame);
                    }
                    // The same-type no-delimiter boundary is undecidable (one coded frame
                    // vs two back-to-back same-type units), so the CELU layer poisons its
                    // unit-count-dependent judgments for this embedded layer.
                    FrameBoundary::Ambiguous
                } else {
                    // A decided continuation of the open coded frame: a readable
                    // `is_first_tile_group == 0` same-type tile OBU, or a same-type bridge
                    // whose output class is type-decided (so the frame-vs-unit ambiguity is
                    // moot). It does not open a new CELU frame unit.
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
            // No coded frame: the unit's grammar / output class is undetermined, so
            // the BRT bound is not judged.
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
            | SegRole::BridgeFrame
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
    matches!(role, SegRole::TipFrame { .. } | SegRole::BridgeFrame)
}

/// Maps a parsed output flag to a classification (`None` → Unknown).
fn output_class(output: Option<bool>) -> OutputClass {
    match output {
        Some(true) => OutputClass::Output,
        Some(false) => OutputClass::NonOutput,
        None => OutputClass::Unknown,
    }
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
