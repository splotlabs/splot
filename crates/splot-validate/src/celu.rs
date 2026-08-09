// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coded-extended-layer-unit (CELU) constraints (AV2 v1.0.0 § 7.3.6) and the
//! § 7.3.7 / § 7.4.6 display-order-hint (DOH) constraints.
//!
//! A CELU is the OBUs sharing one `obu_xlayer_id` within a temporal unit, ordered HLS
//! headers (LCR → OPS → atlas → sequence header) then, per embedded layer in ascending
//! `obu_mlayer_id`, zero or more coded non-output frame units then zero or one coded
//! output frame unit (`OBU_PADDING` position-free). This tracker sits above the
//! [`FrameUnitSegmenter`](crate::frame_unit::FrameUnitSegmenter) and consumes the
//! [`FrameBoundary`] it reports per frame-bearing OBU as the single source of truth for
//! coded-frame-unit boundaries, so the two layers never diverge.
//!
//! The segmenter keys per `(xlayer, mlayer, tlayer)` triple; this tracker aggregates per
//! `(xlayer, mlayer)`. An [`FrameBoundary::Ambiguous`] OBU's existence as a new unit is
//! undecided: it poisons only the embedded layer's unit-count/index-dependent judgments
//! (per-unit accounting, OrderHint accumulators, and output-presence — the latter only
//! when the OBU's output class is not type-decided non-output, so an ambiguous bridge does
//! not over-poison). Decided-pair-order judgments (`celu/in-unit-order` output-slot
//! grammar, `celu/key-not-in-first-unit`) survive, since an intervening ambiguous OBU
//! cannot reorder two decided units; CLK/OLK identity and leading-ness are type-decided
//! and recorded before any poison.
//!
//! Disjointness: `obu-order/non-global-hls-before-coded-layer` owns "HLS header after the
//! frame region began"; `celu/in-unit-order` owns the inter-HLS-header and
//! ascending-`obu_mlayer_id` ordering. `frame-unit/ci-not-in-first-frame-unit` (§ 7.3.8.10,
//! temporal-unit-scoped) and `celu/content-interpretation-not-in-first-unit` (§ 7.3.6,
//! CELU-scoped) are distinct ids. The coded-video-sequence-scoped CI-presence half (mirror
//! lines 560-562) lives in [`crate::context`]
//! (`celu/content-interpretation-not-in-first-celu`).
//!
//! The Unknown invariant: every output-classification- or OrderHint-derived judgment is
//! dropped, never guessed, when the underlying fact is undecidable. Leading-ness is a
//! tri-state ([`Leadingness`]) mirroring AVM's `is_leading_picture`; a CLK is
//! [`Leadingness::Indeterminate`] and excluded from the all-leading-or-none judgment (the
//! spec gloss and AVM conflict, so the validator under-reports).
//!
//! § 7.3.7 imposes two flag-gated DOH constraints (mirror lines 650-657): (1) all frame
//! units in the temporal unit share one `OrderHintBits`; (2) coded output frame units in
//! multiple CELUs share one `OrderHint`. The `order_hint` LSB is a proxy for the decoded
//! OrderHint, sound only when the two compared output units share one known `OrderHintBits`,
//! so output-CELU samples are grouped by their known bits and each compared to its group's
//! representative. A known-but-unequal-bits pair is covered by constraint (1) instead.

use std::collections::BTreeMap;

use splot_core::annexb::ObuEnvelope;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, ObuType};

use crate::diagnostic::{Diagnostic, ValidationReport};
use crate::frame_unit::FrameBoundary;

/// The CELU-relevant classification of one OBU, computed by the validator from the
/// already-parsed state and handed to [`CodedExtendedLayerTracker::observe`].
///
/// Mirrors the [`crate::frame_unit::SegRole`] split: the validator derives the
/// frame-header facts (output class, `order_hint`, frame kind) once and passes the
/// result; the tracker holds no parser state.
#[derive(Debug, Clone, Copy)]
pub(crate) enum CeluRole {
    /// `OBU_LAYER_CONFIGURATION_RECORD` (in-unit phase 1).
    LayerConfigurationRecord,
    /// `OBU_OPERATING_POINT_SET` (in-unit phase 2).
    OperatingPointSet,
    /// `OBU_ATLAS_SEGMENT` (in-unit phase 3).
    AtlasSegment,
    /// `OBU_SEQUENCE_HEADER` (in-unit phase 4).
    SequenceHeader,
    /// `OBU_CONTENT_INTERPRETATION` — judged by the § 7.3.6 CELU-scoped first-frame-unit
    /// rule. It is part of a frame unit's head (§ 7.3.3) but does not open the coded
    /// frame, so it does not by itself advance the embedded-layer frame-unit count.
    ContentInterpretation,
    /// A frame-bearing OBU (tile group / SEF / TIP / bridge / CLK / OLK / switch / RAS).
    /// The first OBU of each coded frame opens a new frame unit in its embedded layer.
    Frame(FrameFacts),
    /// `OBU_PADDING` — position-free, never advances or starts a CELU phase
    /// (mirror lines 531-532).
    Padding,
    /// Any other OBU type (BRT / QM / FGM / metadata / MFH): part of a frame unit's
    /// interior, owned by the [`FrameUnitSegmenter`](crate::frame_unit). It neither opens a
    /// coded frame nor is an HLS header, so it is transparent to the CELU phase/ordering
    /// machine but still marks that the frame region of the CELU has begun.
    FrameInterior,
}

/// The CELU-relevant facts of one frame-bearing OBU, derived by the validator.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameFacts {
    /// The OBU type (for the CLK/OLK identity and same-`obu_type` coded-frame grouping).
    pub obu_type: ObuType,
    /// The coded-frame-unit boundary this OBU sits at, as reported by the
    /// [`FrameUnitSegmenter`](crate::frame_unit) (the single source of truth for § 7.3.6
    /// boundaries). An [`FrameBoundary::Ambiguous`] boundary poisons the embedded layer's
    /// unit-count-dependent judgments (the Unknown invariant).
    pub boundary: FrameBoundary,
    /// The output classification: `Some(true)` output, `Some(false)` non-output, `None`
    /// undecidable (routes the output-class-derived judgments to silence).
    pub output: Option<bool>,
    /// The `order_hint` LSB syntax of this frame when the core parse read it; `None`
    /// otherwise. A proxy for the §7.3.6/§7.3.7 decoded OrderHint: the LSB comparison is a
    /// sound under-approximation within one CELU (one OrderHintBits), but the cross-CELU
    /// comparison must additionally gate on equal known OrderHintBits (see
    /// [`DohTuAccumulator`]). Decoded-OrderHint comparison is a residual blocked on
    /// reference-state modelling (AV2-5.18.2-FRAME-HEADER-INFO).
    pub order_hint: Option<u32>,
    /// The `OrderHintBits` of this frame (from its active sequence header) when the core parse
    /// resolved it; `None` otherwise. Carried per output unit so the cross-CELU OrderHint
    /// comparison (constraint 2, mirror lines 656-657) can be gated on only the two compared
    /// output units' bits being known and equal, independent of an unrelated frame unit
    /// elsewhere in the temporal unit.
    pub order_hint_bits: Option<u32>,
    /// The leading-ness of the frame for the § 7.3.6 all-leading-or-none rule. Always
    /// type-decided from `obu_type`; an [`Leadingness::Indeterminate`] unit (a CLK) is
    /// excluded from the judgment entirely (see [`Leadingness`]).
    pub leadingness: Leadingness,
}

/// The leading-ness of a frame-bearing OBU for the § 7.3.6 all-leading-or-none rule
/// (mirror `07-decoding-process.md` lines 555-556), a tri-state mirroring AVM's
/// `is_leading_picture` (`av2/decoder/obu.c:2544-2549`).
///
/// The § 6.4.1-area gloss (`06-syntax-structures-semantics.md:4546`) would class a CLK as
/// leading (`IsRegular == 0`), but AVM tri-states a CLK to indeterminate. The spec text and
/// AVM conflict, so the validator under-reports per the ambiguous-spec policy: a CLK is
/// [`Self::Indeterminate`] and excluded from the judgment entirely, which fires only when a
/// [`Self::Leading`] and a [`Self::Regular`] unit coexist in one CELU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leadingness {
    /// A `LEADING_*` frame (`av2_is_leading_vcl_obu`): `OBU_LEADING_TILE_GROUP` /
    /// `OBU_LEADING_SEF` / `OBU_LEADING_TIP`. A trigger for the mix rule.
    Leading,
    /// A regular frame (`av2_is_regular_vcl_obu`): `OLK` / `REGULAR_TILE_GROUP` /
    /// `REGULAR_SEF` / `REGULAR_TIP` / `SWITCH` / `RAS` / `BRIDGE`. The counterpart that,
    /// coexisting with a [`Self::Leading`] unit, fires the mix rule.
    Regular,
    /// A CLK (`OBU_CLOSED_LOOP_KEY`): neither leading nor regular under the AVM tri-state
    /// (`is_leading_picture == -1`). Excluded from the all-leading-or-none judgment.
    Indeterminate,
}

/// The in-unit ordering phase a CELU has reached (AV2 § 7.3.6 lines 521-529). The
/// HLS-header phases are strictly ascending; the frame region follows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CeluPhase {
    /// Before any OBU of the CELU.
    Start,
    /// In or after the layer-configuration-record run.
    LayerConfigurationRecord,
    /// In or after the operating-point-set run.
    OperatingPointSet,
    /// In or after the atlas-segment run.
    AtlasSegment,
    /// In or after the sequence-header run.
    SequenceHeader,
    /// In the per-embedded-layer frame-unit region.
    Frames,
}

/// The accumulating per-embedded-layer state within one CELU.
#[derive(Debug, Default, Clone, Copy)]
struct EmbeddedLayerState {
    /// Number of coded frame units opened for this embedded layer in the CELU.
    units_opened: u32,
    /// Whether a coded **output** frame unit has been seen for this embedded layer.
    saw_output_unit: bool,
    /// Whether a coded **non-output** frame unit has been seen for this embedded layer.
    saw_nonoutput_unit: bool,
    /// `true` once any unit of this embedded layer had an Unknown output class — the
    /// non-output-implies-output judgment for the layer is then dropped.
    output_class_unknown: bool,
    /// `true` once a *decided*-output unit has opened, consuming the layer's single output
    /// slot (mirror lines 528-529). Any later *decided* (`OpensNewUnit`) unit then violates
    /// the presence grammar (the output unit must be last), regardless of its own output
    /// class; an `Ambiguous` OBU neither consumes the slot nor suppresses the grammar for a
    /// later decided unit.
    output_slot_consumed: bool,
    /// `true` once the output-slot grammar (`celu/in-unit-order`) has fired for this
    /// embedded layer, so a run of later units after the output slot reports once.
    output_slot_grammar_reported: bool,
    /// `true` once an [`FrameBoundary::Ambiguous`] OBU here could itself be a coded output
    /// frame unit (`facts.output != Some(false)`), dropping the per-layer
    /// non-output-implies-output judgment (mirror lines 537-538). An ambiguous bridge
    /// (type-decided non-output) does not set this.
    output_presence_poisoned: bool,
}

/// One coded extended layer unit's accumulating state, keyed by `obu_xlayer_id` within
/// the temporal unit.
#[derive(Debug)]
struct CeluState {
    /// The in-unit ordering phase reached so far.
    phase: CeluPhase,
    /// The byte offset of the first OBU of the CELU, the fallback diagnostic anchor for
    /// the whole-CELU constraint findings.
    first_offset: ByteOffset,
    /// Per-embedded-layer frame-unit accounting (output presence, ascending order).
    embedded: BTreeMap<EmbeddedLayerId, EmbeddedLayerState>,
    /// The highest `obu_mlayer_id` whose frame unit has opened, for the ascending-mlayer
    /// frame-unit ordering rule. `None` until the first frame unit opens.
    max_embedded_seen: Option<EmbeddedLayerId>,
    /// `true` once at least one frame-bearing OBU has been observed. A CELU is only created
    /// from a non-padding constituent OBU, so a CELU that never sets this is a header-only
    /// CELU and fires `celu/missing-output-frame-unit` (§ 7.3.6 line 536, anchored at its
    /// first constituent OBU).
    saw_frame_bearing_obu: bool,
    /// `true` once at least one coded output frame unit has been seen anywhere in the CELU
    /// (the § 7.3.6 "at least one coded output frame unit" presence rule).
    saw_any_output_unit: bool,
    /// `true` once a unit with an Unknown output class was seen — the
    /// output-unit-presence rule is then dropped (the CELU might contain an output unit
    /// the validator could not classify).
    any_output_class_unknown: bool,
    /// `true` once an [`FrameBoundary::Ambiguous`] OBU here could itself be a coded output
    /// frame unit (`facts.output != Some(false)`), dropping the CELU-scoped
    /// `celu/missing-output-frame-unit` rule (mirror line 536). An ambiguous bridge does not
    /// set this.
    missing_output_poisoned: bool,
    /// The shared `OrderHint` (an `order_hint` LSB proxy — see [`FrameFacts::order_hint`]) of
    /// the output units seen so far, the first output unit's `OrderHintBits` (for the
    /// cross-CELU §7.3.7 gate), and the first output unit's offset; `None` until the first
    /// output unit with a readable `order_hint`. Within one CELU all frame units share one
    /// OrderHintBits, so the LSB comparison is sound here without a cross-width gate.
    output_order_hint: Option<(u32, Option<u32>, ByteOffset)>,
    /// `true` once an output unit's `order_hint` could not be read, so the CELU is not
    /// contributed to the cross-CELU DOH accumulator (the Unknown invariant). Does not
    /// suppress this CELU's own [`Self::order_hint_mismatch`], proven between two known units.
    order_hint_undecidable: bool,
    /// The (first, found, anchor) of the first output unit whose `OrderHint` disagreed with
    /// [`Self::output_order_hint`], if any. Proven between two known units and emitted at
    /// [`CodedExtendedLayerTracker::resolve_celu`] regardless of any undecidable member.
    order_hint_mismatch: Option<(u32, u32, ByteOffset)>,
    /// The leading-ness shared by the *decidable* frame units seen so far (`Some(true)` all
    /// [`Leadingness::Leading`], `Some(false)` all [`Leadingness::Regular`]); `None` until
    /// the first non-[`Leadingness::Indeterminate`] frame unit. Indeterminate units (CLK)
    /// are excluded from the all-leading-or-none judgment entirely.
    leading: Option<bool>,
    /// Whether the all-leading-or-none rule has already fired for this CELU.
    leading_mismatch_reported: bool,
    /// `true` if a CLK OBU has been seen anywhere in the CELU.
    saw_clk: bool,
    /// `true` if an OLK OBU has been seen anywhere in the CELU.
    saw_olk: bool,
    /// Whether the no-CLK+OLK-mix rule has already fired for this CELU.
    clk_olk_mix_reported: bool,
    /// The lowest embedded layer that has opened a frame unit, and whether that layer's
    /// first frame unit was a CLK / an OLK, for the lowest-layer-first rules.
    lowest_embedded_first_unit: Option<LowestEmbeddedFirstUnit>,
}

/// The kind of the lowest embedded layer's first coded frame unit, for the CLK/OLK
/// lowest-layer-first rules (AV2 § 7.3.6 lines 543-545 / 551-553).
#[derive(Debug, Clone, Copy)]
struct LowestEmbeddedFirstUnit {
    embedded: EmbeddedLayerId,
    is_clk: bool,
    is_olk: bool,
    offset: ByteOffset,
}

impl CeluState {
    fn new(first_offset: ByteOffset) -> Self {
        Self {
            phase: CeluPhase::Start,
            first_offset,
            embedded: BTreeMap::new(),
            max_embedded_seen: None,
            saw_frame_bearing_obu: false,
            saw_any_output_unit: false,
            any_output_class_unknown: false,
            missing_output_poisoned: false,
            output_order_hint: None,
            order_hint_undecidable: false,
            order_hint_mismatch: None,
            leading: None,
            leading_mismatch_reported: false,
            saw_clk: false,
            saw_olk: false,
            clk_olk_mix_reported: false,
            lowest_embedded_first_unit: None,
        }
    }
}

/// The per-temporal-unit cross-CELU accumulator for the § 7.3.7 / § 7.4.6 DOH "same
/// OrderHint across the output units of multiple CELUs" and "same OrderHintBits for all
/// frame units in the temporal unit" checks. Mismatches are detected as values arrive and
/// emitted at [`Self::resolve`], which the caller gates on the DOH constraint flag.
///
/// The `order_hint` LSB is a proxy for the decoded OrderHint, sound for constraint (2) only
/// when the two compared output units share one known OrderHintBits, so the cross-CELU
/// comparison ([`Self::note_celu_output_order_hint`]) is gated per compared pair rather than
/// on the temporal-unit-wide same-bits judgment. Constraint (1) ([`Self::bits_mismatch`])
/// covers every frame unit's OrderHintBits. Both mismatches are recorded only between known
/// samples, so an undecidable participant cannot suppress a proven mismatch.
#[derive(Debug, Default)]
struct DohTuAccumulator {
    /// Per known `OrderHintBits` value, each group's first decidable output-CELU sample (its
    /// OrderHint LSB proxy and anchor). A later output CELU is compared to its own group's
    /// representative, since the LSB proxy is sound only within one bits width; unknown-bits
    /// output CELUs stay out of all groups.
    output_order_hint_by_bits: BTreeMap<u32, (u32, ByteOffset)>,
    /// The (representative, found, anchor) of the first within-group output-CELU OrderHint
    /// that disagreed with its bits-group's representative, if any — emitted at
    /// [`Self::resolve`], deduplicated to one emission per temporal unit. A known-but-unequal
    /// bits pair is covered by `celu/doh-order-hint-bits-mismatch` (constraint 1) instead.
    order_hint_mismatch: Option<(u32, u32, ByteOffset)>,
    /// The first frame's OrderHintBits and its anchor offset; `None` until the first frame
    /// with a readable OrderHintBits.
    first_order_hint_bits: Option<(u32, ByteOffset)>,
    /// The (value, anchor) of the first frame OrderHintBits that disagreed with
    /// [`Self::first_order_hint_bits`], if any — emitted at [`Self::resolve`].
    bits_mismatch: Option<(u32, u32, ByteOffset)>,
}

/// Coded-extended-layer-unit constraint tracker (AV2 § 7.3.6 / § 7.3.7 / § 7.4.6).
///
/// One instance lives in the validator context. It is fed every OBU in stream order via
/// [`Self::observe`], reset per temporal unit via [`Self::reset_temporal_unit`], and
/// flushed at the end of the bitstream via [`Self::finish`]. CELU state is keyed by
/// `obu_xlayer_id` (a CELU is the per-extended-layer slice of a temporal unit, § 7.3.6);
/// global OBUs are not part of any CELU.
#[derive(Debug, Default)]
pub(crate) struct CodedExtendedLayerTracker {
    /// The open CELUs of the current temporal unit, keyed by `obu_xlayer_id`.
    celus: BTreeMap<ExtendedLayerId, CeluState>,
    /// The order CELUs were first opened in, so the per-CELU constraint family resolves
    /// deterministically (ascending `obu_xlayer_id` via the `BTreeMap` key order suffices,
    /// but resolution is over the map directly).
    doh: DohTuAccumulator,
    /// Whether the temporal unit's recorded DOH constraint flag is active (`lcr_doh_…` in
    /// the activated global LCR, or `multistream_doh_…` in the preceding MSDO, == 1). Set
    /// once per temporal unit by the validator before the boundary resolution; `None`
    /// before it is known, treated as "not active".
    doh_flag_active: bool,
}

impl CodedExtendedLayerTracker {
    /// Feeds one OBU to the tracker in stream order. Global OBUs (temporal delimiter,
    /// MSDO, global HLS / metadata / padding) are not part of any CELU and are ignored
    /// here — they are ordered by the § 7.3.7 temporal-unit machine in [`crate::context`].
    pub(crate) fn observe(
        &mut self,
        obu: &ObuEnvelope<'_>,
        role: CeluRole,
        report: &mut ValidationReport,
    ) {
        if obu.header.extended_layer_id.is_global() {
            return;
        }
        if matches!(role, CeluRole::Padding) {
            return;
        }

        let xlayer = obu.header.extended_layer_id;
        let embedded = obu.header.embedded_layer_id;
        let celu = self
            .celus
            .entry(xlayer)
            .or_insert_with(|| CeluState::new(obu.offset));

        match role {
            CeluRole::LayerConfigurationRecord => {
                Self::advance_hls_phase(celu, CeluPhase::LayerConfigurationRecord, obu, report);
            }
            CeluRole::OperatingPointSet => {
                Self::advance_hls_phase(celu, CeluPhase::OperatingPointSet, obu, report);
            }
            CeluRole::AtlasSegment => {
                Self::advance_hls_phase(celu, CeluPhase::AtlasSegment, obu, report);
            }
            CeluRole::SequenceHeader => {
                Self::advance_hls_phase(celu, CeluPhase::SequenceHeader, obu, report);
            }
            CeluRole::ContentInterpretation => {
                celu.phase = CeluPhase::Frames;
                Self::note_embedded_layer_ordering(celu, embedded, obu, report);
                Self::observe_ci(celu, embedded, obu, report);
            }
            CeluRole::FrameInterior => {
                celu.phase = CeluPhase::Frames;
                Self::note_embedded_layer_ordering(celu, embedded, obu, report);
            }
            CeluRole::Frame(facts) => {
                celu.phase = CeluPhase::Frames;
                celu.saw_frame_bearing_obu = true;
                Self::observe_frame(celu, embedded, facts, obu, report);
            }
            CeluRole::Padding => {}
        }
    }

    /// Advances the CELU's HLS-header phase, reporting a `celu/in-unit-order` violation
    /// when an HLS header appears *before* an earlier HLS-header phase (e.g. an LCR after
    /// an OPS). The disjoint "HLS header after the frame region began" case is owned by
    /// `obu-order/non-global-hls-before-coded-layer` ([`crate::context`]), so it is **not**
    /// reported here — only the inter-HLS-header ordering this tracker uniquely covers.
    fn advance_hls_phase(
        celu: &mut CeluState,
        phase: CeluPhase,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let earlier_hls_phase = matches!(
            celu.phase,
            CeluPhase::LayerConfigurationRecord
                | CeluPhase::OperatingPointSet
                | CeluPhase::AtlasSegment
                | CeluPhase::SequenceHeader
        );
        if earlier_hls_phase && celu.phase > phase {
            report.push(celu_error(
                "celu/in-unit-order",
                obu,
                format!(
                    "{} appears after a later HLS-header phase in its coded extended layer \
                     unit; § 7.3.6 orders the HLS headers layer-configuration-record → \
                     operating-point-set → atlas-segment → sequence-header",
                    obu.header.obu_type.spec_name()
                ),
            ));
        }
        // Advance monotonically: never move the phase backward, so a later in-order header
        // of an already-passed phase (zero-or-more of each) does not reset progress.
        if phase > celu.phase {
            celu.phase = phase;
        }
    }

    /// Threads one frame-unit-constituent OBU's `obu_mlayer_id` into the ascending-mlayer
    /// frame-unit ordering accounting (mirror line 525), reporting `celu/in-unit-order` when
    /// the embedded layer is below the highest seen so far. Every § 7.3.3 constituent (head,
    /// frame, suffix tail) participates with its own mlayer; padding is excluded.
    fn note_embedded_layer_ordering(
        celu: &mut CeluState,
        embedded: EmbeddedLayerId,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        if let Some(prev) = celu.max_embedded_seen
            && embedded < prev
        {
            report.push(celu_error(
                "celu/in-unit-order",
                obu,
                format!(
                    "a frame unit at obu_mlayer_id {} opens after a frame unit at \
                     obu_mlayer_id {}; § 7.3.6 orders the embedded-layer frame units in \
                     ascending obu_mlayer_id",
                    embedded.get(),
                    prev.get()
                ),
            ));
        }
        if celu.max_embedded_seen.is_none_or(|prev| embedded > prev) {
            celu.max_embedded_seen = Some(embedded);
        }
    }

    /// Records a frame-bearing OBU's CLK / OLK identity and fires the no-CLK+OLK-mix rule
    /// (mirror line 554). The identity is type-decided from `obu_type`, so it is recorded for
    /// every frame-bearing OBU regardless of its (even [`FrameBoundary::Ambiguous`]) boundary.
    fn record_clk_olk_identity(
        celu: &mut CeluState,
        obu_type: ObuType,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        if obu_type == ObuType::ClosedLoopKey {
            celu.saw_clk = true;
        }
        if obu_type == ObuType::OpenLoopKey {
            celu.saw_olk = true;
        }
        if celu.saw_clk && celu.saw_olk && !celu.clk_olk_mix_reported {
            celu.clk_olk_mix_reported = true;
            report.push(celu_error(
                "celu/clk-olk-mixed",
                obu,
                "a coded extended layer unit contains both an OBU_CLOSED_LOOP_KEY and an \
                 OBU_OPEN_LOOP_KEY; § 7.3.6 forbids mixing the two in one coded extended \
                 layer unit"
                    .to_owned(),
            ));
        }
    }

    /// Records a frame-bearing OBU's leading-ness for the all-leading-or-none rule (mirror
    /// lines 555-556), firing only when a decidable [`Leadingness::Leading`] and
    /// [`Leadingness::Regular`] unit coexist; [`Leadingness::Indeterminate`] (a CLK) is
    /// excluded (see [`Leadingness`]). Type-decided from `obu_type`, so recorded for every
    /// frame-bearing OBU regardless of its (even [`FrameBoundary::Ambiguous`]) boundary.
    fn record_leadingness(
        celu: &mut CeluState,
        leadingness: Leadingness,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let leading = match leadingness {
            Leadingness::Leading => Some(true),
            Leadingness::Regular => Some(false),
            Leadingness::Indeterminate => None,
        };
        if let Some(is_leading) = leading {
            match celu.leading {
                None => celu.leading = Some(is_leading),
                Some(prev) if prev != is_leading && !celu.leading_mismatch_reported => {
                    celu.leading_mismatch_reported = true;
                    report.push(celu_error(
                        "celu/leading-frame-mix",
                        obu,
                        "a coded extended layer unit mixes leading and non-leading frame units; \
                         § 7.3.6 requires all frame units in a coded extended layer unit to be \
                         leading frames if any is a leading frame"
                            .to_owned(),
                    ));
                }
                Some(_) => {}
            }
        }
    }

    /// Observes a content-interpretation OBU for the § 7.3.6 CELU-scoped first-frame-unit
    /// CI rule (mirror lines 557-559): a CI may appear only in the first frame unit of each
    /// embedded layer within the CELU. The temporal-unit-scoped form is the disjoint
    /// `frame-unit/ci-not-in-first-frame-unit` rule (§ 7.3.8.10).
    fn observe_ci(
        celu: &CeluState,
        embedded: EmbeddedLayerId,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let units_opened = celu
            .embedded
            .get(&embedded)
            .map_or(0, |state| state.units_opened);
        if units_opened >= 1 {
            report.push(celu_error(
                "celu/content-interpretation-not-in-first-unit",
                obu,
                "OBU_CONTENT_INTERPRETATION appears outside the first frame unit of its \
                 embedded layer in the coded extended layer unit (§ 7.3.6)"
                    .to_owned(),
            ));
        }
    }

    /// Observes a frame-bearing OBU, updating the CELU's per-embedded-layer unit accounting
    /// and the constraint-family accumulators. Only an [`FrameBoundary::OpensNewUnit`] OBU
    /// opens a new frame unit; a [`FrameBoundary::ContinuesUnit`] OBU is transparent for the
    /// unit-count judgments; an [`FrameBoundary::Ambiguous`] OBU only poisons them (the Unknown
    /// invariant). The ascending-`obu_mlayer_id` ordering is boundary-independent, so it runs
    /// before the unit-count branch.
    fn observe_frame(
        celu: &mut CeluState,
        embedded: EmbeddedLayerId,
        facts: FrameFacts,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        match facts.boundary {
            FrameBoundary::ContinuesUnit => {
                Self::note_embedded_layer_ordering(celu, embedded, obu, report);
                Self::record_clk_olk_identity(celu, facts.obu_type, obu, report);
                Self::record_leadingness(celu, facts.leadingness, obu, report);
                return;
            }
            FrameBoundary::Ambiguous => {
                Self::note_embedded_layer_ordering(celu, embedded, obu, report);
                Self::record_clk_olk_identity(celu, facts.obu_type, obu, report);
                Self::record_leadingness(celu, facts.leadingness, obu, report);
                if facts.output != Some(false) {
                    let layer = celu.embedded.entry(embedded).or_default();
                    layer.output_presence_poisoned = true;
                    celu.missing_output_poisoned = true;
                }
                return;
            }
            FrameBoundary::OpensNewUnit => {}
        }

        {
            let layer = celu.embedded.entry(embedded).or_default();
            if layer.output_slot_consumed && !layer.output_slot_grammar_reported {
                layer.output_slot_grammar_reported = true;
                report.push(celu_error(
                    "celu/in-unit-order",
                    obu,
                    format!(
                        "a coded frame unit opens in obu_mlayer_id {} after that embedded \
                         layer's coded output frame unit; § 7.3.6 requires the coded output \
                         frame unit to be the last frame unit in each embedded layer of a \
                         coded extended layer unit",
                        embedded.get()
                    ),
                ));
            }
        }

        Self::note_embedded_layer_ordering(celu, embedded, obu, report);

        let is_clk = facts.obu_type == ObuType::ClosedLoopKey;
        let is_olk = facts.obu_type == ObuType::OpenLoopKey;
        Self::record_clk_olk_identity(celu, facts.obu_type, obu, report);

        let layer = celu.embedded.entry(embedded).or_default();
        let unit_index = layer.units_opened;
        layer.units_opened = layer.units_opened.saturating_add(1);
        match facts.output {
            Some(true) => {
                layer.saw_output_unit = true;
                celu.saw_any_output_unit = true;
                layer.output_slot_consumed = true;
            }
            Some(false) => layer.saw_nonoutput_unit = true,
            None => {
                layer.output_class_unknown = true;
                celu.any_output_class_unknown = true;
            }
        }

        let is_first_unit_of_layer = unit_index == 0;
        if is_first_unit_of_layer {
            let replace = celu
                .lowest_embedded_first_unit
                .is_none_or(|cur| embedded < cur.embedded);
            if replace {
                celu.lowest_embedded_first_unit = Some(LowestEmbeddedFirstUnit {
                    embedded,
                    is_clk,
                    is_olk,
                    offset: obu.offset,
                });
            }
        }

        if (is_clk || is_olk) && !is_first_unit_of_layer {
            let key = if is_clk {
                "OBU_CLOSED_LOOP_KEY"
            } else {
                "OBU_OPEN_LOOP_KEY"
            };
            report.push(celu_error(
                "celu/key-not-in-first-unit",
                obu,
                format!(
                    "{key} opens a frame unit that is not the first coded frame unit of its \
                     embedded layer in the coded extended layer unit; § 7.3.6 permits CLK / \
                     OLK OBUs only in each embedded layer's first frame unit"
                ),
            ));
        }

        Self::record_leadingness(celu, facts.leadingness, obu, report);

        if facts.output == Some(true) {
            match facts.order_hint {
                Some(order_hint) => match celu.output_order_hint {
                    None => {
                        celu.output_order_hint =
                            Some((order_hint, facts.order_hint_bits, obu.offset));
                    }
                    Some((first, _, _)) => {
                        if first != order_hint && celu.order_hint_mismatch.is_none() {
                            celu.order_hint_mismatch = Some((first, order_hint, obu.offset));
                        }
                    }
                },
                None => celu.order_hint_undecidable = true,
            }
        }
    }

    /// Resolves one CELU's whole-unit constraints at temporal-unit boundary, after all its
    /// OBUs have been observed: the output-unit-presence and non-output-implies-output
    /// rules, the CLK/OLK lowest-layer-first rules, and the CELU's contribution to the
    /// cross-CELU DOH OrderHint accumulator.
    fn resolve_celu(celu: &CeluState, doh: &mut DohTuAccumulator, report: &mut ValidationReport) {
        let has_decided_unit = celu
            .embedded
            .values()
            .any(|l| l.saw_output_unit || l.saw_nonoutput_unit);
        let header_only = !celu.saw_frame_bearing_obu;
        let frame_bearing_without_output = has_decided_unit
            && !celu.saw_any_output_unit
            && !celu.any_output_class_unknown
            && !celu.missing_output_poisoned;
        if header_only || frame_bearing_without_output {
            report.push(
                Diagnostic::error(
                    "celu/missing-output-frame-unit",
                    "a coded extended layer unit contains no coded output frame unit; § 7.3.6 \
                     requires at least one coded output frame unit per coded extended layer unit"
                        .to_owned(),
                )
                .with_spec_section("7.3.6")
                .with_byte_offset(celu.first_offset),
            );
        }

        for (embedded, layer) in &celu.embedded {
            if layer.saw_nonoutput_unit
                && !layer.saw_output_unit
                && !layer.output_class_unknown
                && !layer.output_presence_poisoned
            {
                report.push(
                    Diagnostic::error(
                        "celu/non-output-without-output",
                        format!(
                            "embedded layer {} of a coded extended layer unit has a coded \
                             non-output frame unit but no coded output frame unit; § 7.3.6 \
                             requires a coded output frame unit in each embedded layer that \
                             has a coded non-output frame unit",
                            embedded.get()
                        ),
                    )
                    .with_spec_section("7.3.6")
                    .with_byte_offset(celu.first_offset),
                );
            }
        }

        if let Some(lowest) = celu.lowest_embedded_first_unit
            && !(celu.saw_clk && celu.saw_olk)
        {
            if celu.saw_clk && !lowest.is_clk {
                report.push(
                    Diagnostic::error(
                        "celu/lowest-layer-not-key",
                        "a coded extended layer unit contains an OBU_CLOSED_LOOP_KEY, but the \
                         lowest embedded layer's first coded frame unit is not a CLK; § 7.3.6 \
                         requires it to be a CLK"
                            .to_owned(),
                    )
                    .with_spec_section("7.3.6")
                    .with_byte_offset(lowest.offset),
                );
            }
            if celu.saw_olk && !lowest.is_olk {
                report.push(
                    Diagnostic::error(
                        "celu/lowest-layer-not-key",
                        "a coded extended layer unit contains an OBU_OPEN_LOOP_KEY, but the \
                         lowest embedded layer's first coded frame unit is not an OLK; § 7.3.6 \
                         requires it to be an OLK"
                            .to_owned(),
                    )
                    .with_spec_section("7.3.6")
                    .with_byte_offset(lowest.offset),
                );
            }
        }

        if let Some((first, found, offset)) = celu.order_hint_mismatch {
            report.push(
                Diagnostic::error(
                    "celu/output-order-hint-mismatch",
                    format!(
                        "coded output frame units in one coded extended layer unit carry \
                         different OrderHint values ({first} and {found}); § 7.3.6 requires \
                         all coded output frame units in a coded extended layer unit to share \
                         one OrderHint"
                    ),
                )
                .with_spec_section("7.3.6")
                .with_byte_offset(offset),
            );
        }

        if !celu.order_hint_undecidable
            && let Some((order_hint, order_hint_bits, offset)) = celu.output_order_hint
        {
            doh.note_celu_output_order_hint(order_hint, order_hint_bits, offset);
        }
    }

    /// Threads one frame's `OrderHintBits` into the temporal-unit DOH accumulator (mirror
    /// line 655). Called for every frame-bearing OBU; a `None` (undecidable) frame is not
    /// recorded (the Unknown invariant).
    pub(crate) fn note_order_hint_bits(
        &mut self,
        order_hint_bits: Option<u32>,
        offset: ByteOffset,
    ) {
        if let Some(bits) = order_hint_bits {
            self.doh.note_order_hint_bits(bits, offset);
        }
    }

    /// Records whether the current temporal unit's DOH constraint flag is active — the
    /// `lcr_doh_constraint_flag` of the activated global LCR, or the
    /// `multistream_doh_constraint_flag` of the preceding MSDO, equal to 1 (mirror lines
    /// 650-657 / 1316-1320). Set by the validator before the boundary resolution; the
    /// § 7.3.7 / § 7.4.6 DOH OrderHint checks fire only when this is `true`.
    pub(crate) fn set_doh_flag_active(&mut self, active: bool) {
        self.doh_flag_active = active;
    }

    /// Resolves the per-temporal-unit § 7.3.7 / § 7.4.6 DOH OrderHint checks, gated on the
    /// recorded DOH constraint flag, and the per-CELU constraint family. Called at each
    /// temporal-unit boundary (a global temporal delimiter) and at the end of the
    /// bitstream.
    pub(crate) fn reset_temporal_unit(&mut self, report: &mut ValidationReport) {
        for celu in self.celus.values() {
            Self::resolve_celu(celu, &mut self.doh, report);
        }
        if self.doh_flag_active {
            self.doh.resolve(report);
        }
        self.celus.clear();
        self.doh = DohTuAccumulator::default();
        self.doh_flag_active = false;
    }

    /// Resolves the final temporal unit's CELUs at the end of the bitstream (AV2 § 7.3.2
    /// end condition: the final temporal unit has no trailing global temporal delimiter),
    /// exactly as an internal boundary would.
    pub(crate) fn finish(&mut self, report: &mut ValidationReport) {
        self.reset_temporal_unit(report);
    }
}

impl DohTuAccumulator {
    /// Notes one CELU's resolved output OrderHint (and OrderHintBits) for the cross-CELU
    /// §7.3.7 agreement check (constraint 2, mirror lines 656-657), recording the first
    /// disagreement; emission is deferred to [`Self::resolve`].
    ///
    /// Output CELUs are grouped by their known OrderHintBits and each compared to its own
    /// group's representative, since the LSB proxy is sound only within one bits width; an
    /// unknown-bits output CELU stays out of all groups. A known-but-different-bits pair is
    /// covered by `celu/doh-order-hint-bits-mismatch` (constraint 1) instead. At most one
    /// mismatch per temporal unit, anchored at the offending later sample.
    fn note_celu_output_order_hint(
        &mut self,
        order_hint: u32,
        order_hint_bits: Option<u32>,
        offset: ByteOffset,
    ) {
        let Some(bits) = order_hint_bits else {
            return;
        };
        match self.output_order_hint_by_bits.get(&bits) {
            None => {
                self.output_order_hint_by_bits
                    .insert(bits, (order_hint, offset));
            }
            Some(&(representative, _)) => {
                if representative != order_hint && self.order_hint_mismatch.is_none() {
                    self.order_hint_mismatch = Some((representative, order_hint, offset));
                }
            }
        }
    }

    /// Notes one frame's OrderHintBits for the same-OrderHintBits-in-TU check. Records the
    /// first disagreement; emission is deferred to [`Self::resolve`].
    fn note_order_hint_bits(&mut self, bits: u32, offset: ByteOffset) {
        match self.first_order_hint_bits {
            None => self.first_order_hint_bits = Some((bits, offset)),
            Some((first, _)) => {
                if first != bits && self.bits_mismatch.is_none() {
                    self.bits_mismatch = Some((first, bits, offset));
                }
            }
        }
    }

    /// Resolves the per-temporal-unit § 7.3.7 / § 7.4.6 DOH OrderHint / OrderHintBits checks.
    /// The caller gates this on the active DOH constraint flag (mirror lines 650-657); each
    /// recorded mismatch is proven between two known samples, so it is emitted regardless of
    /// any undecidable participant.
    fn resolve(&self, report: &mut ValidationReport) {
        if let Some((first, found, offset)) = self.bits_mismatch {
            report.push(
                Diagnostic::error(
                    "celu/doh-order-hint-bits-mismatch",
                    format!(
                        "frame units in one temporal unit carry different OrderHintBits \
                         ({first} and {found}) while a DOH constraint flag is set; § 7.3.7 \
                         requires all frame units in a temporal unit to share one OrderHintBits"
                    ),
                )
                .with_spec_section("7.3.7")
                .with_byte_offset(offset),
            );
        }
        if let Some((first, found, offset)) = self.order_hint_mismatch {
            report.push(
                Diagnostic::error(
                    "celu/doh-order-hint-mismatch",
                    format!(
                        "coded output frame units in different coded extended layer units of \
                         one temporal unit carry different OrderHint values ({first} and \
                         {found}) while a DOH constraint flag is set; § 7.3.7 / § 7.4.6 \
                         require them to share one OrderHint"
                    ),
                )
                .with_spec_section("7.3.7")
                .with_byte_offset(offset),
            );
        }
    }
}

/// Builds a `celu/` § 7.3.6 error anchored at the offending OBU.
fn celu_error(rule_id: &'static str, obu: &ObuEnvelope<'_>, message: String) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("7.3.6")
        .with_byte_offset(obu.offset)
}

#[cfg(test)]
mod tests;
