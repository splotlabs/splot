// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coded-extended-layer-unit (CELU) constraints (AV2 v1.0.0 § 7.3.6) and the
//! § 7.3.7 / § 7.4.6 display-order-hint (DOH) constraints.
//!
//! A coded extended layer unit (mirror `07-decoding-process.md` lines 517-617) is the
//! collection of OBUs sharing one `obu_xlayer_id` within a temporal unit, ordered
//!
//! 1. zero or more `OBU_LAYER_CONFIGURATION_RECORD`,
//! 2. zero or more `OBU_OPERATING_POINT_SET`,
//! 3. zero or more `OBU_ATLAS_SEGMENT`,
//! 4. zero or more `OBU_SEQUENCE_HEADER`,
//! 5. for each embedded layer present, in ascending `obu_mlayer_id`, zero or more coded
//!    non-output frame units then zero or one coded output frame unit,
//!
//! with `OBU_PADDING` position-free (mirror lines 521-532). This tracker sits **above**
//! the [`FrameUnitSegmenter`](crate::frame_unit::FrameUnitSegmenter): the segmenter owns
//! the § 7.3.3 / § 7.3.4 within-frame-unit grammar (one coded frame per unit, region
//! order, the § 7.3.8.10 temporal-unit-scoped CI rule); the CELU tracker owns the
//! § 7.3.6 facts scoped to one `obu_xlayer_id` across a temporal unit (in-unit OBU
//! ordering between the HLS-header phases and across embedded layers, the output-unit /
//! OrderHint / CLK-OLK / leading-frame constraint family, and the CELU-scoped CI rule).
//!
//! ## Boundary authority: the segmenter is the single source of truth
//!
//! The CELU tracker does **not** re-derive coded-frame-unit boundaries from frame-delimiter
//! bits. It consumes the [`FrameBoundary`] the
//! [`FrameUnitSegmenter`](crate::frame_unit::FrameUnitSegmenter) reports for each
//! frame-bearing OBU, so the two layers can never diverge on where one coded frame unit ends
//! and the next begins. The segmenter's richer split semantics (a flag-1 tile group / SEF
//! always splits; a different-`obu_type` no-delimiter frame splits; a unit-head / suffix-tail
//! region boundary — e.g. a BRT / QM / FGM / prefix-metadata head after a frame's tail —
//! starts a new unit) are therefore honoured here too. The decisive example is `TIP, BRT,
//! TIP`: the segmenter splits at the BRT (a new unit head), so the second TIP is a *new*
//! coded frame unit — a last-`obu_type` comparison (the former CELU logic) would have wrongly
//! merged the two TIPs.
//!
//! The segmenter keys per `(xlayer, mlayer, tlayer)` triple; this tracker aggregates per
//! `(xlayer, mlayer)`. The boundary signal is per-OBU and computed from that OBU's own
//! triple's open-unit state, so it maps directly to the embedded-layer aggregation: an OBU
//! that opens a unit in *any* triple of an `(xlayer, mlayer)` reports
//! [`FrameBoundary::OpensNewUnit`] and so opens a coded frame unit for that embedded layer
//! (a second coded frame at a different `obu_tlayer_id` opens a fresh triple state, hence a
//! new unit — mirror line 880).
//!
//! An [`FrameBoundary::Ambiguous`] boundary (a same-`obu_type` no-delimiter TIP, or an
//! unreadable tile-group delimiter, while a coded frame is open) means the segmenter could
//! not decide whether the OBU opened a new unit or continued the open one. Its existence as a
//! new unit is undecided, so it **poisons** the embedded layer's unit-count-dependent
//! judgments — the within-layer output-slot grammar, the CLK/OLK-first-unit rule, and (it
//! already rests only on decided units) the missing-output judgment — which are dropped for
//! that layer rather than guessed (the Unknown invariant). The former
//! "same-type-no-delimiter run is one unit" behaviour is therefore replaced by this
//! poison-on-ambiguity semantics, keeping zero false positives when in doubt.
//!
//! ## Within-layer output-slot presence grammar (mirror lines 528-529)
//!
//! Each embedded layer is "zero or more coded non-output frame units then zero or one coded
//! output frame unit": the single coded output frame unit must be **last**. The tracker
//! records when a *decided*-output unit consumes the layer's output slot; any later *decided*
//! (`OpensNewUnit`) unit in the same layer — regardless of its own output class, even an
//! Unknown-class one, since its mere existence after the slot is the violation — fires
//! `celu/in-unit-order`. An Unknown-class *earlier* unit does not consume the slot (the
//! validator cannot confirm it is the coded output frame unit), so a later unit does not fire.
//!
//! ## Header-only CELU presence (mirror line 536)
//!
//! "At least one coded output frame unit shall be present in the coded extended layer unit"
//! applies to *every* CELU. A CELU is opened only from a **non-padding** constituent OBU
//! (padding and reserved types — which [`crate::context`]'s `celu_role_for` maps to
//! [`CeluRole::Padding`] — never open one), so a **header-only CELU** (≥ 1 non-padding
//! constituent OBU — an HLS header / CI / frame-interior — and *zero* frame-bearing OBUs)
//! fires `celu/missing-output-frame-unit`, anchored at the CELU's first constituent OBU. A
//! padding-only (or reserved-type-only) `obu_xlayer_id` group never constitutes a CELU and is
//! always silent. A frame-bearing CELU whose output classes are all Unknown still drops the
//! rule (the Unknown invariant).
//!
//! ## Disjointness with the existing § 7.3.6 / § 7.3.7 checks
//!
//! Two existing checks already cover parts of § 7.3.6 / § 7.3.7; the `celu/` predicates
//! are kept disjoint from both (the PR #51 linear/replay disjointness precedent):
//!
//! - `obu-order/non-global-hls-before-coded-layer` (§ 7.3.6, [`crate::context`]) fires
//!   when an HLS *header* OBU (LCR / OPS / atlas / sequence header) appears after the
//!   coded *frame* region of its CELU has begun. The `celu/in-unit-order` rule therefore
//!   does **not** re-report that case; it covers the disjoint remainder — the ordering
//!   *between* the four HLS-header phases (an OPS before an LCR, an atlas before an OPS,
//!   a sequence header before an atlas) and the ascending-`obu_mlayer_id` ordering of the
//!   frame units. A header after the frame region is left wholly to the existing rule.
//! - `frame-unit/ci-not-in-first-frame-unit` (§ 7.3.8.10, [`crate::frame_unit`]) is the
//!   *temporal-unit*-scoped CI rule ("the first coded frame unit of each embedded layer
//!   within the temporal unit"). The `celu/content-interpretation-not-in-first-unit` rule
//!   is the distinct § 7.3.6 *CELU*-scoped form ("the first frame unit of each embedded
//!   layer within this coded extended layer unit"). Distinct ids, distinct sections; the
//!   CELU is the per-`obu_xlayer_id` slice of a temporal unit, so a second CELU for the
//!   same embedded layer cannot arise within one temporal unit (one CELU per xlayer per
//!   TU) — the two rules coincide on a single-CELU embedded layer and the § 7.3.6 form is
//!   the citation that scopes the constraint to the CELU.
//!
//! ## The Unknown invariant (PRs #46-#52)
//!
//! Every output-classification-derived or OrderHint-derived judgment is *dropped* — never
//! guessed — when the underlying fact is undecidable (the frame-header parse stopped
//! before the field, or the active sequence header was unavailable). A unit whose output
//! class is Unknown is excluded from the output-unit-presence, non-output-implies-output,
//! CLK/OLK-first-unit, and OrderHint checks; an output unit whose `order_hint` could not
//! be read drops only the OrderHint-agreement judgment for its CELU, leaving the other
//! type-decided facts (leading-ness, CLK/OLK identity) intact. Leading-ness is type-decided
//! from `obu_type` for every frame-bearing OBU, so the all-leading-or-none rule never routes
//! to the Unknown path. It is a *tri-state* ([`Leadingness`]) rather than a bool, mirroring
//! AVM's `is_leading_picture` (`av2/decoder/obu.c:2544-2549`): `LEADING_*` is
//! [`Leadingness::Leading`], the `IsRegular == 1` set (OLK / `REGULAR_*` / `SWITCH` / `RAS`
//! / `BRIDGE`) is [`Leadingness::Regular`], and a CLK is [`Leadingness::Indeterminate`]. The
//! § 6.4.1-area gloss (`06-syntax-structures-semantics.md:4546`) would class a CLK as leading
//! (`IsRegular == 0`), but the oracle disagrees, so the validator excludes the indeterminate
//! CLK from the all-leading-or-none judgment entirely — a documented sound under-approximation
//! that keeps the rule silent on the CLK-plus-regular structure the § 7.3.6 CLK rule
//! explicitly contemplates (mirror lines 541-549).

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
    /// The coded-frame-unit boundary this OBU sits at, **as reported by the
    /// [`FrameUnitSegmenter`](crate::frame_unit)** — the single source of truth for
    /// coded-frame-unit boundaries (§ 7.3.6). The tracker no longer re-derives boundaries
    /// from frame-delimiter bits; it consumes this signal, so the two layers never diverge
    /// (e.g. on the `TIP, BRT, TIP` case the segmenter splits at the new-unit head between
    /// the TIPs while a last-type comparison would merge them). An
    /// [`FrameBoundary::Ambiguous`] boundary poisons the embedded layer's
    /// unit-count-dependent judgments (the Unknown invariant).
    pub boundary: FrameBoundary,
    /// The output classification: `Some(true)` output, `Some(false)` non-output, `None`
    /// undecidable (routes the output-class-derived judgments to silence).
    pub output: Option<bool>,
    /// The `order_hint` LSB syntax (`OrderHintLsbs`) of this frame, when the core parse read
    /// it; `None` when the parse stopped before it or the active sequence header was
    /// unavailable. This is a **proxy** for the §7.3.6/§7.3.7 DECODED OrderHint
    /// (`get_disp_order_hint`, the MSB extension for non-CLK frames): within one CELU (one
    /// xlayer, one active header → one OrderHintBits) the LSB comparison is a sound
    /// under-approximation (differing LSBs imply differing OrderHints; same LSBs with
    /// diverging MSBs under-report), but the cross-CELU comparison must additionally gate on
    /// equal known OrderHintBits (see [`DohTuAccumulator`]). Decoded-OrderHint comparison is a
    /// named residual blocked on reference-state modelling (AV2-5.18.2-FRAME-HEADER-INFO).
    pub order_hint: Option<u32>,
    /// The leading-ness of the frame for the § 7.3.6 all-leading-or-none rule. Always
    /// type-decided from `obu_type` (never routed from a parse failure), so the rule never
    /// reaches the Unknown path; an [`Leadingness::Indeterminate`] unit (a CLK) is excluded
    /// from the judgment entirely (see [`Leadingness`]).
    pub leadingness: Leadingness,
}

/// The leading-ness of a frame-bearing OBU for the § 7.3.6 all-leading-or-none rule
/// (mirror `07-decoding-process.md` lines 555-556), mirroring AVM's tri-state classification.
///
/// The § 6.4.1-area gloss at `06-syntax-structures-semantics.md:4546` ("If IsRegular is
/// equal to 0 (i.e., this is a leading frame)") reads as if `IsRegular == 0` were exactly
/// "leading", which would class a CLK (§ 5.18.2 excludes CLK from `IsRegular`) as leading.
/// The AVM oracle disagrees and is treated as authoritative here: it tri-states the
/// decoder's `is_leading_picture` (`av2/decoder/obu.c:2544-2549`) to `1` for the
/// `av2_is_leading_vcl_obu` set (`obu.c:1666` — exactly `{OBU_LEADING_TILE_GROUP,
/// OBU_LEADING_SEF, OBU_LEADING_TIP}`), `0` for the `av2_is_regular_vcl_obu` set
/// (`av2/decoder/decodeframe.c:7015` — `OLK` plus the `REGULAR_*` / `SWITCH` / `RAS` /
/// `BRIDGE` set, excluding CLK), and `-1` (indeterminate) otherwise — into which a CLK
/// lands. Because the spec text and the oracle conflict, the validator under-reports and
/// documents (the established ambiguous-spec policy): a CLK is [`Self::Indeterminate`] and
/// is excluded from the all-leading-or-none judgment entirely (neither a trigger nor an
/// offender). The mix rule fires only when a [`Self::Leading`] unit and a [`Self::Regular`]
/// unit coexist in one CELU — a documented sound under-approximation that is silent on the
/// CLK case the § 7.3.6 CLK rule explicitly contemplates (mirror lines 541-549: a higher
/// embedded layer's first coded frame unit may be a non-CLK regular frame).
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
    /// `true` once a coded **output** frame unit (a *decided*-output unit) has opened for
    /// this embedded layer — the embedded layer's single output slot (mirror lines 528-529:
    /// "zero or more coded non-output frame units then zero or one coded output frame unit")
    /// is then consumed. Any later unit whose existence is *decided* (an `OpensNewUnit`
    /// boundary, not `Ambiguous`) violates the per-layer presence grammar (the coded output
    /// frame unit must be last), regardless of its own output class. An `Ambiguous` unit
    /// neither consumes the slot nor triggers the grammar (its existence is undecided).
    output_slot_consumed: bool,
    /// `true` once the output-slot grammar (`celu/in-unit-order`) has fired for this
    /// embedded layer, so a run of later units after the output slot reports once.
    output_slot_grammar_reported: bool,
    /// `true` once a frame-bearing OBU of this embedded layer reported an
    /// [`FrameBoundary::Ambiguous`] boundary — the segmenter could not decide whether it
    /// opened a new coded frame unit or continued the open one. The unit-count-dependent
    /// per-layer judgments (the output-slot grammar above, the CLK/OLK first-unit rule, and
    /// the missing-output judgment, which already rests only on decided units) are then
    /// dropped for this embedded layer (never guessed — the Unknown invariant).
    ambiguous_poisoned: bool,
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
    /// `true` once at least one frame-bearing OBU (tile group / SEF / TIP / bridge / CLK /
    /// OLK / switch / RAS) has been observed in this CELU — i.e. the CELU has at least one
    /// coded frame unit. A CELU is only ever created from a *non-padding* constituent OBU
    /// (padding and reserved types return before the CELU is opened), so a CELU that exists
    /// but never sets this flag is a **header-only CELU**: ≥ 1 non-padding constituent OBU
    /// (an HLS header / CI / frame-interior OBU) and zero frame-bearing OBUs. § 7.3.6 line
    /// 536 ("at least one coded output frame unit shall be present") applies to *every*
    /// CELU, so a header-only CELU fires `celu/missing-output-frame-unit` (anchored at the
    /// CELU's first constituent OBU). A padding-only (or reserved-type-only) xlayer group
    /// never opens a CELU, so it cannot fire.
    saw_frame_bearing_obu: bool,
    /// `true` once at least one coded output frame unit has been seen anywhere in the CELU
    /// (the § 7.3.6 "at least one coded output frame unit" presence rule).
    saw_any_output_unit: bool,
    /// `true` once a unit with an Unknown output class was seen — the
    /// output-unit-presence rule is then dropped (the CELU might contain an output unit
    /// the validator could not classify).
    any_output_class_unknown: bool,
    /// The shared `OrderHint` (an `order_hint` LSB proxy — see [`FrameFacts::order_hint`]) of
    /// the output units seen so far, plus the offset of the first output unit; `None` until
    /// the first output unit with a readable `order_hint`. Within one CELU all frame units
    /// share one active header, hence one OrderHintBits, so the LSB comparison is a sound
    /// under-approximation here (no cross-width gate needed — unlike the cross-CELU check).
    output_order_hint: Option<(u32, ByteOffset)>,
    /// `true` once an output unit's `order_hint` could not be read — the
    /// same-OrderHint-across-output-units judgment is then dropped (it would rest on a
    /// partial set), so the deferred mismatch is suppressed.
    order_hint_undecidable: bool,
    /// The (first, found, anchor) of the first output unit whose `OrderHint` disagreed with
    /// [`Self::output_order_hint`], if any. Detected eagerly but **emitted** at
    /// [`CodedExtendedLayerTracker::resolve_celu`], so a later undecidable output unit can
    /// still drop the whole judgment (the Unknown invariant): the same-OrderHint rule rests
    /// on the *full* set of the CELU's output units, so an undecidable member retroactively
    /// invalidates a mismatch detected among the decidable ones.
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

/// The per-temporal-unit cross-CELU OrderHint accumulator for the § 7.3.7 / § 7.4.6 DOH
/// "same OrderHint across the coded output frame units of multiple CELUs" check and the
/// "same OrderHintBits for all frame units in the temporal unit" check. The mismatches
/// are *detected* as values arrive but *emitted* only at [`Self::resolve`], which the
/// caller gates on the DOH constraint flag — a mismatch present with the flag off is
/// conforming (the DOH constraints are flag-gated).
///
/// **OrderHint LSB proxy.** §7.3.6/§7.3.7 compare the DECODED OrderHint
/// (`get_disp_order_hint`'s output, the MSB extension for non-CLK frames); the validator
/// compares the raw `order_hint` LSB syntax as a proxy (decoded-OrderHint comparison is a
/// named residual blocked on reference-state modelling — AV2-5.18.2-FRAME-HEADER-INFO).
/// The cross-CELU comparison is gated in [`Self::resolve`] on the participating frame units
/// sharing one KNOWN OrderHintBits: across different bit widths equal decoded OrderHints can
/// carry different-width LSB encodings, so the proxy would false-positive.
#[derive(Debug, Default)]
struct DohTuAccumulator {
    /// The first CELU's resolved output OrderHint (an `order_hint` LSB proxy, see the type
    /// doc) and its anchor offset; `None` until a CELU resolves a decidable output OrderHint.
    first_output_order_hint: Option<(u32, ByteOffset)>,
    /// `true` once a CELU's output OrderHint was undecidable — the cross-CELU agreement
    /// judgment is then dropped (it would rest on a partial set).
    undecidable: bool,
    /// The (value, anchor) of the first CELU output OrderHint (LSB proxy) that disagreed with
    /// [`Self::first_output_order_hint`], if any — emitted at [`Self::resolve`] only when the
    /// temporal unit's OrderHintBits are known and uniform (else the LSB proxy is unsound and
    /// the comparison is dropped).
    order_hint_mismatch: Option<(u32, u32, ByteOffset)>,
    /// The first frame's OrderHintBits and its anchor offset; `None` until the first frame
    /// with a readable OrderHintBits.
    first_order_hint_bits: Option<(u32, ByteOffset)>,
    /// `true` once a frame's OrderHintBits could not be read — the same-OrderHintBits
    /// judgment is then dropped.
    bits_undecidable: bool,
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
            // Padding is position-free within a CELU (mirror lines 531-532).
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
                // A content-interpretation OBU is the head of a frame unit (§ 7.3.3), so it
                // is in the frame region; it does not open the coded frame, so it does not
                // advance the embedded-layer unit count. The CELU-scoped first-frame-unit CI
                // rule is judged against the embedded layer's already-opened unit count.
                celu.phase = CeluPhase::Frames;
                Self::observe_ci(celu, embedded, obu, report);
            }
            CeluRole::FrameInterior => {
                // BRT / QM / FGM / metadata / MFH: the frame region has begun, but this OBU
                // neither opens a coded frame nor is an HLS header. The within-frame-unit
                // grammar is owned by the FrameUnitSegmenter.
                celu.phase = CeluPhase::Frames;
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
        // Only fire when the violation is between HLS-header phases (the current phase is an
        // HLS-header phase strictly later than this OBU's). Once the frame region has begun
        // (`CeluPhase::Frames`), an HLS header is out of order, but that is the existing
        // `obu-order/non-global-hls-before-coded-layer` rule's territory — keep disjoint.
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
        // A CI heads a frame unit (§ 7.3.3). It is in the first frame unit of its embedded
        // layer iff no coded frame for that layer has opened yet (`units_opened == 0`); a
        // CI after the layer's first coded frame has opened is heading a later unit.
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

    /// Observes a frame-bearing OBU, updating the CELU's per-embedded-layer unit
    /// accounting and the constraint-family accumulators. The
    /// [`FrameUnitSegmenter`](crate::frame_unit) is the single source of truth for
    /// coded-frame-unit boundaries (§ 7.3.6): each OBU carries its segmenter-reported
    /// [`FrameBoundary`]. Only an [`FrameBoundary::OpensNewUnit`] OBU opens a new frame
    /// unit; an [`FrameBoundary::ContinuesUnit`] OBU is transparent here (a later OBU of an
    /// already-counted coded frame); an [`FrameBoundary::Ambiguous`] OBU's existence as a
    /// new unit is undecided, so it neither opens a unit nor feeds the accumulators — it
    /// only *poisons* the embedded layer's unit-count-dependent judgments (never guessed —
    /// the Unknown invariant).
    fn observe_frame(
        celu: &mut CeluState,
        embedded: EmbeddedLayerId,
        facts: FrameFacts,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        match facts.boundary {
            // A decided continuation of the open coded frame: a later OBU of an
            // already-counted coded frame unit. Transparent here.
            FrameBoundary::ContinuesUnit => return,
            // The segmenter could not decide whether this OBU opened a new coded frame unit
            // or continued the open one (a same-type no-delimiter TIP, or an unreadable
            // tile-group delimiter). Its existence as a new unit is undecided, so the
            // unit-count-dependent per-layer judgments must be dropped — poison the embedded
            // layer and stop (do not feed the accumulators on a guess).
            FrameBoundary::Ambiguous => {
                celu.embedded
                    .entry(embedded)
                    .or_default()
                    .ambiguous_poisoned = true;
                return;
            }
            // A decided new coded frame unit opens for this embedded layer.
            FrameBoundary::OpensNewUnit => {}
        }

        // --- per-embedded-layer output-slot presence grammar (mirror lines 528-529) ---
        // Each embedded layer is "zero or more coded non-output frame units then zero or one
        // coded output frame unit": the single coded output frame unit must be LAST. Once a
        // decided-output unit has consumed the slot (`output_slot_consumed`), any later
        // *decided* (`OpensNewUnit`) unit in the same layer violates the grammar — regardless
        // of its own output class (even an Unknown-class later unit: its mere existence after
        // the output slot is the violation). Dropped for a layer poisoned by an Ambiguous
        // boundary (its unit count is undecided). The slot is consumed only by a decided
        // *output* unit below (an Unknown-class earlier unit does not consume it, so a later
        // unit does not fire).
        {
            let layer = celu.embedded.entry(embedded).or_default();
            if layer.output_slot_consumed
                && !layer.ambiguous_poisoned
                && !layer.output_slot_grammar_reported
            {
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

        // --- ascending-`obu_mlayer_id` frame-unit ordering (mirror line 525) ---
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

        // --- CLK / OLK identity (type-decided) ---
        let is_clk = facts.obu_type == ObuType::ClosedLoopKey;
        let is_olk = facts.obu_type == ObuType::OpenLoopKey;
        if is_clk {
            celu.saw_clk = true;
        }
        if is_olk {
            celu.saw_olk = true;
        }
        // No CLK+OLK mix in one CELU (mirror line 554).
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

        // --- per-embedded-layer unit accounting ---
        let layer = celu.embedded.entry(embedded).or_default();
        let unit_index = layer.units_opened;
        let layer_poisoned = layer.ambiguous_poisoned;
        layer.units_opened = layer.units_opened.saturating_add(1);
        match facts.output {
            Some(true) => {
                layer.saw_output_unit = true;
                celu.saw_any_output_unit = true;
                // A *decided* output unit consumes the embedded layer's single output slot
                // (mirror lines 528-529): a later decided unit then violates the per-layer
                // presence grammar above. An Unknown-class unit does NOT consume the slot.
                layer.output_slot_consumed = true;
            }
            Some(false) => layer.saw_nonoutput_unit = true,
            None => {
                layer.output_class_unknown = true;
                celu.any_output_class_unknown = true;
            }
        }

        // --- lowest-embedded-layer first-unit kind (for the CLK/OLK lowest-layer rules) ---
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

        // --- CLK/OLK only-first-frame-unit per embedded layer (mirror lines 543-545 /
        // 551-553) --- a CLK/OLK OBU may only be in the FIRST coded frame unit of each
        // embedded layer of the CELU. Type-decided, fires eagerly — but dropped for a layer
        // poisoned by an Ambiguous boundary, whose unit index (`is_first_unit_of_layer`) may
        // be wrong (an undecided earlier unit was not counted), so "not the first unit" would
        // rest on a guess.
        if (is_clk || is_olk) && !is_first_unit_of_layer && !layer_poisoned {
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

        // --- leading / non-leading (all-leading-or-none, mirror lines 555-556) --- the rule
        // fires only when a decidable Leading unit and a decidable Regular unit coexist.
        // Indeterminate units (CLK) are excluded from the judgment entirely (neither a
        // trigger nor an offender — see Leadingness): the spec text and the AVM oracle
        // conflict on whether a CLK is "leading", so the validator under-reports (the
        // ambiguous-spec policy). Type-decided, so it never routes to Unknown.
        let leading = match facts.leadingness {
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

        // --- same-OrderHint across the CELU's output units (mirror lines 539-540) --- the
        // mismatch is detected here but emitted at resolve_celu, so a later undecidable
        // output unit can still drop the whole judgment (the Unknown invariant).
        if facts.output == Some(true) {
            match facts.order_hint {
                Some(order_hint) => match celu.output_order_hint {
                    None => celu.output_order_hint = Some((order_hint, obu.offset)),
                    Some((first, _)) => {
                        if first != order_hint && celu.order_hint_mismatch.is_none() {
                            celu.order_hint_mismatch = Some((first, order_hint, obu.offset));
                        }
                    }
                },
                // An output unit whose order_hint could not be read: drop the
                // same-OrderHint judgment for this CELU (it would rest on a partial set).
                None => celu.order_hint_undecidable = true,
            }
        }
    }

    /// Resolves one CELU's whole-unit constraints at temporal-unit boundary, after all its
    /// OBUs have been observed: the output-unit-presence and non-output-implies-output
    /// rules, the CLK/OLK lowest-layer-first rules, and the CELU's contribution to the
    /// cross-CELU DOH OrderHint accumulator.
    fn resolve_celu(celu: &CeluState, doh: &mut DohTuAccumulator, report: &mut ValidationReport) {
        // --- at least one coded output frame unit (mirror line 536) --- this applies to
        // *every* CELU. A CELU is only opened from a non-padding constituent OBU (padding and
        // reserved types never open one), so two non-conforming shapes fire:
        //
        // - a **header-only CELU**: ≥ 1 non-padding constituent OBU (HLS header / CI /
        //   frame-interior) and *zero* frame-bearing OBUs (`!saw_frame_bearing_obu`, a
        //   type-decided fact). It has no coded output frame unit at all, so it fires
        //   unconditionally — § 7.3.6 line 536 admits no exception for a frame-less CELU.
        // - a **frame-bearing CELU** with at least one output-class-decided frame unit but no
        //   coded output frame unit. Dropped when any unit's output class was Unknown (the
        //   CELU might contain an unclassified output unit — the Unknown invariant) or when
        //   the CELU has only Ambiguous (undecided) units and no decided one.
        //
        // A padding-only (or reserved-type-only) xlayer group never opens a CELU, so it is
        // never reported here.
        let has_decided_unit = celu
            .embedded
            .values()
            .any(|l| l.saw_output_unit || l.saw_nonoutput_unit);
        // A header-only CELU has no frame-bearing OBU at all (so no output unit and no
        // Unknown class to consider); a frame-bearing CELU fires only with a decided unit,
        // no output unit, and no Unknown class.
        let header_only = !celu.saw_frame_bearing_obu;
        let frame_bearing_without_output =
            has_decided_unit && !celu.saw_any_output_unit && !celu.any_output_class_unknown;
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

        // --- non-output-implies-output per embedded layer (mirror lines 537-538) --- if a
        // layer has a coded non-output frame unit, it must also have a coded output one.
        // Dropped for a layer whose output class was ever Unknown.
        for (embedded, layer) in &celu.embedded {
            if layer.saw_nonoutput_unit && !layer.saw_output_unit && !layer.output_class_unknown {
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

        // --- CLK / OLK lowest-embedded-layer-first (mirror lines 543-545 / 551-553) --- if
        // the CELU contains a CLK, the lowest embedded layer's first frame unit shall be a
        // CLK; likewise for OLK. Dropped when no CLK/OLK is present, or the two are mixed
        // (already reported), since the lowest-layer rule's premise is the per-kind one.
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

        // --- same-OrderHint across the CELU's output units (mirror lines 539-540) --- the
        // mismatch was detected eagerly; emit it now only if no output unit's order_hint was
        // undecidable (an undecidable member drops the whole judgment — the Unknown
        // invariant). The comparison is over the `order_hint` LSB proxy
        // ([`FrameFacts::order_hint`]); within one CELU all output units share one active
        // header (one OrderHintBits), so it is a sound under-approximation with no cross-width
        // gate needed (unlike the cross-CELU §7.3.7 check, which gates on equal known bits).
        if !celu.order_hint_undecidable
            && let Some((first, found, offset)) = celu.order_hint_mismatch
        {
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

        // --- contribute this CELU's output OrderHint to the cross-CELU DOH accumulator ---
        // The CELU's "associated order hint" is the OrderHint shared by its coded output
        // frame units (mirror lines 571-576). It resolves only when the CELU had at least
        // one output unit with a decidable order_hint and the same-OrderHint judgment was
        // not dropped (no undecidable output unit). Otherwise the CELU contributes an
        // undecidable signal, dropping the cross-CELU agreement judgment.
        if celu.order_hint_undecidable {
            doh.undecidable = true;
        } else if let Some((order_hint, offset)) = celu.output_order_hint {
            doh.note_celu_output_order_hint(order_hint, offset);
        }
    }

    /// Threads one frame's `OrderHintBits` into the temporal-unit DOH accumulator (mirror
    /// line 655: all frame units in the temporal unit share one `OrderHintBits`). Called by
    /// the validator for every frame-bearing OBU, since the bits come from the active
    /// sequence header rather than the per-frame parse and span CELUs. `None` drops the
    /// same-OrderHintBits judgment for the temporal unit.
    pub(crate) fn note_order_hint_bits(
        &mut self,
        order_hint_bits: Option<u32>,
        offset: ByteOffset,
    ) {
        match order_hint_bits {
            Some(bits) => self.doh.note_order_hint_bits(bits, offset),
            None => self.doh.bits_undecidable = true,
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
        // Resolve each CELU's whole-unit constraints (output presence, non-output-implies-
        // output, lowest-layer key) and accumulate its output OrderHint for the cross-CELU
        // DOH check.
        for celu in self.celus.values() {
            Self::resolve_celu(celu, &mut self.doh, report);
        }
        // The § 7.3.7 / § 7.4.6 DOH OrderHint checks fire only when the temporal unit's DOH
        // constraint flag is active (mirror lines 650-657). With the flag off they stay
        // silent.
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
    /// Notes one CELU's resolved output OrderHint for the cross-CELU agreement check.
    /// Records the first disagreement; emission is deferred to [`Self::resolve`] so the
    /// check stays flag-gated.
    fn note_celu_output_order_hint(&mut self, order_hint: u32, offset: ByteOffset) {
        match self.first_output_order_hint {
            None => self.first_output_order_hint = Some((order_hint, offset)),
            Some((first, _)) => {
                if first != order_hint && self.order_hint_mismatch.is_none() {
                    self.order_hint_mismatch = Some((first, order_hint, offset));
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

    /// Resolves the per-temporal-unit § 7.3.7 / § 7.4.6 DOH OrderHint / OrderHintBits
    /// checks. The caller gates this on the active DOH constraint flag, so a mismatch under
    /// a flag-off temporal unit is never reported (the DOH constraints are flag-gated,
    /// mirror lines 650-657). The undecidable signals drop their respective judgments
    /// (Unknown invariant).
    fn resolve(&mut self, report: &mut ValidationReport) {
        // § 7.3.7: all frame units in the temporal unit shall use the same OrderHintBits.
        if !self.bits_undecidable
            && let Some((first, found, offset)) = self.bits_mismatch
        {
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
        // § 7.3.7 / § 7.4.6: coded output frame units present in multiple CELUs within the
        // temporal unit shall share the same OrderHint.
        //
        // The validator compares the raw `order_hint` LSB syntax as a proxy for the DECODED
        // OrderHint (the §7.3.6/§7.3.7 quantity is `get_disp_order_hint`'s output, the MSB
        // extension for non-CLK frames, which needs reference-state modelling — see
        // AV2-5.18.2-FRAME-HEADER-INFO). The cross-CELU proxy is only sound when every
        // participating frame unit shares ONE KNOWN OrderHintBits: differing decoded
        // OrderHints with equal-width LSBs still differ in the low bits, but EQUAL decoded
        // OrderHints can be encoded with different-width LSBs when the bit widths differ, so a
        // cross-width comparison can FALSE-POSITIVE. When the widths differ the
        // `celu/doh-order-hint-bits-mismatch` rule above already fires; this OrderHint
        // comparison must then DROP. Gate it on the bits being known (at least one recorded)
        // and uniform (no undecidable, no mismatch).
        let bits_known_and_uniform = self.first_order_hint_bits.is_some()
            && !self.bits_undecidable
            && self.bits_mismatch.is_none();
        if !self.undecidable
            && bits_known_and_uniform
            && let Some((first, found, offset)) = self.order_hint_mismatch
        {
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
mod tests {
    use super::*;
    use splot_core::obu::ObuHeader;
    use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};

    /// Synthetic OBU envelope for feeding the tracker directly. The payload is unused
    /// (the tracker holds no parser state — facts arrive via [`CeluRole`]); the offset is
    /// the running OBU index, so anchors are distinguishable.
    fn obu(obu_type: ObuType, xlayer: u8, mlayer: u8, offset: u64) -> ObuEnvelope<'static> {
        ObuEnvelope {
            offset: ByteOffset::new(offset),
            size: 1,
            header: ObuHeader {
                has_header_extension: true,
                obu_type,
                temporal_layer_id: TemporalLayerId::from_bits(0),
                embedded_layer_id: EmbeddedLayerId::from_bits(mlayer),
                extended_layer_id: ExtendedLayerId::from_bits(xlayer),
                header_size_bytes: 2,
            },
            payload: &[],
        }
    }

    /// A frame `CeluRole` with the given facts and an explicit segmenter
    /// [`FrameBoundary`]. The `opens` flag is mapped to
    /// [`FrameBoundary::OpensNewUnit`] / [`FrameBoundary::ContinuesUnit`]; the
    /// [`FrameBoundary::Ambiguous`] case is exercised by [`ambiguous_role`].
    fn frame_role(
        obu_type: ObuType,
        opens: bool,
        output: Option<bool>,
        order_hint: Option<u32>,
        leadingness: Leadingness,
    ) -> CeluRole {
        frame_role_with_boundary(
            obu_type,
            if opens {
                FrameBoundary::OpensNewUnit
            } else {
                FrameBoundary::ContinuesUnit
            },
            output,
            order_hint,
            leadingness,
        )
    }

    /// A frame `CeluRole` carrying an explicit segmenter [`FrameBoundary`] — the tracker
    /// consumes the segmenter's boundary verbatim (§ 7.3.6), so the tests drive it directly.
    fn frame_role_with_boundary(
        obu_type: ObuType,
        boundary: FrameBoundary,
        output: Option<bool>,
        order_hint: Option<u32>,
        leadingness: Leadingness,
    ) -> CeluRole {
        CeluRole::Frame(FrameFacts {
            obu_type,
            boundary,
            output,
            order_hint,
            leadingness,
        })
    }

    /// A frame `CeluRole` whose segmenter boundary is [`FrameBoundary::Ambiguous`] (a
    /// same-type no-delimiter TIP, or an unreadable tile-group delimiter).
    fn ambiguous_role(
        obu_type: ObuType,
        output: Option<bool>,
        order_hint: Option<u32>,
        leadingness: Leadingness,
    ) -> CeluRole {
        frame_role_with_boundary(
            obu_type,
            FrameBoundary::Ambiguous,
            output,
            order_hint,
            leadingness,
        )
    }

    /// A simple output tile-group frame at (xlayer, mlayer) with a given OrderHint.
    fn output_frame(order_hint: u32) -> CeluRole {
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(true),
            Some(order_hint),
            Leadingness::Regular,
        )
    }

    fn has(report: &ValidationReport, rule: &str) -> bool {
        report.errors().any(|d| d.rule_id == rule)
    }

    fn fresh() -> CodedExtendedLayerTracker {
        CodedExtendedLayerTracker::default()
    }

    // --- in-unit ordering (HLS-header phases) ---

    #[test]
    fn hls_headers_out_of_order_is_flagged() {
        // § 7.3.6: LCR -> OPS -> atlas -> sequence header. An OPS then an LCR (LCR after a
        // later HLS phase) violates the in-unit order.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::OperatingPointSet, 0, 0, 0),
            CeluRole::OperatingPointSet,
            &mut r,
        );
        t.observe(
            &obu(ObuType::LayerConfigurationRecord, 0, 0, 1),
            CeluRole::LayerConfigurationRecord,
            &mut r,
        );
        assert!(has(&r, "celu/in-unit-order"), "report: {r}");
    }

    #[test]
    fn hls_headers_in_order_is_silent() {
        // The conforming order LCR -> OPS -> atlas -> sequence header is silent, including
        // repeats of an already-passed phase (zero-or-more of each).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        for (i, role) in [
            CeluRole::LayerConfigurationRecord,
            CeluRole::LayerConfigurationRecord,
            CeluRole::OperatingPointSet,
            CeluRole::AtlasSegment,
            CeluRole::SequenceHeader,
        ]
        .into_iter()
        .enumerate()
        {
            let ty = match role {
                CeluRole::LayerConfigurationRecord => ObuType::LayerConfigurationRecord,
                CeluRole::OperatingPointSet => ObuType::OperatingPointSet,
                CeluRole::AtlasSegment => ObuType::AtlasSegment,
                CeluRole::SequenceHeader => ObuType::SequenceHeader,
                _ => unreachable!(),
            };
            t.observe(&obu(ty, 0, 0, i as u64), role, &mut r);
        }
        assert!(!has(&r, "celu/in-unit-order"), "report: {r}");
    }

    #[test]
    fn hls_header_after_frame_region_is_not_celu_in_unit_order() {
        // Disjointness: an HLS header after the frame region began is the existing
        // obu-order/non-global-hls-before-coded-layer rule's territory, NOT celu/in-unit-order.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::LayerConfigurationRecord, 0, 0, 1),
            CeluRole::LayerConfigurationRecord,
            &mut r,
        );
        assert!(
            !has(&r, "celu/in-unit-order"),
            "the celu tracker must defer the after-frame-region case to obu-order/*; report: {r}"
        );
    }

    #[test]
    fn descending_mlayer_frame_units_is_flagged() {
        // § 7.3.6: frame units in ascending obu_mlayer_id. A frame unit at mlayer 0 after one
        // at mlayer 1 violates the order.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            output_frame(0),
            &mut r,
        );
        assert!(has(&r, "celu/in-unit-order"), "report: {r}");
    }

    #[test]
    fn ascending_mlayer_frame_units_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 1),
            output_frame(0),
            &mut r,
        );
        assert!(!has(&r, "celu/in-unit-order"), "report: {r}");
    }

    #[test]
    fn padding_is_transparent_to_in_unit_order() {
        // PADDING is position-free: an OPS, a PADDING, then an LCR still flags the LCR (the
        // padding does not advance or reset the phase).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::OperatingPointSet, 0, 0, 0),
            CeluRole::OperatingPointSet,
            &mut r,
        );
        t.observe(&obu(ObuType::Padding, 0, 0, 1), CeluRole::Padding, &mut r);
        t.observe(
            &obu(ObuType::LayerConfigurationRecord, 0, 0, 2),
            CeluRole::LayerConfigurationRecord,
            &mut r,
        );
        assert!(
            has(&r, "celu/in-unit-order"),
            "padding must be transparent; report: {r}"
        );
    }

    // --- output-unit presence / non-output-implies-output ---

    #[test]
    fn celu_with_no_output_unit_is_flagged() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // A single coded non-output frame unit, no output unit.
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(has(&r, "celu/missing-output-frame-unit"), "report: {r}");
    }

    #[test]
    fn celu_with_output_unit_is_silent_on_presence() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(!has(&r, "celu/missing-output-frame-unit"), "report: {r}");
    }

    #[test]
    fn missing_output_unit_drops_when_a_unit_output_class_is_unknown() {
        // Unknown invariant: a unit with an undecidable output class means the CELU might
        // contain an unclassified output unit; the presence rule is dropped.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                None,
                None,
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/missing-output-frame-unit"),
            "an Unknown output class must drop the presence rule; report: {r}"
        );
    }

    #[test]
    fn non_output_without_output_in_layer_is_flagged() {
        // One embedded layer with a non-output unit but no output unit; another layer with an
        // output unit (so the whole-CELU presence rule is satisfied, isolating the per-layer one).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(has(&r, "celu/non-output-without-output"), "report: {r}");
    }

    #[test]
    fn non_output_with_output_in_same_layer_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // mlayer 0: a non-output then an output unit (ascending order is fine; both open).
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            output_frame(0),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(!has(&r, "celu/non-output-without-output"), "report: {r}");
    }

    #[test]
    fn non_output_without_output_drops_when_layer_output_unknown() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // Whole-CELU presence satisfied by a decided output unit in layer 1.
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 0),
            output_frame(0),
            &mut r,
        );
        // Layer 0: a non-output unit and an Unknown-output unit -> drop the per-layer rule.
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 2),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                None,
                None,
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/non-output-without-output"),
            "an Unknown output class in the layer must drop the per-layer rule; report: {r}"
        );
    }

    // --- within-layer output-slot presence grammar (Finding 1, mirror lines 528-529) ---

    #[test]
    fn second_output_unit_in_layer_fires_output_slot_grammar() {
        // A layer's single coded output frame unit must be LAST (zero or more non-output then
        // zero or one output). A SECOND decided-output unit in the same embedded layer opens
        // after the first consumed the output slot, so the output-slot grammar fires.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            output_frame(0),
            &mut r,
        );
        assert!(
            has(&r, "celu/in-unit-order"),
            "a second output unit after the output slot must fire the grammar; report: {r}"
        );
    }

    #[test]
    fn non_output_after_output_in_layer_fires_output_slot_grammar() {
        // A decided NON-OUTPUT unit opening after the layer's coded output frame unit also
        // violates the grammar (the output unit was not last), regardless of the later unit's
        // own output class.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        assert!(
            has(&r, "celu/in-unit-order"),
            "a non-output unit after the output slot must fire the grammar; report: {r}"
        );
    }

    #[test]
    fn unknown_class_after_output_in_layer_fires_output_slot_grammar() {
        // Even an Unknown-output-class later unit fires: its mere EXISTENCE (a decided
        // OpensNewUnit boundary) after the output slot is the violation, independent of its
        // own (undecidable) output class.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                None,
                None,
                Leadingness::Regular,
            ),
            &mut r,
        );
        assert!(
            has(&r, "celu/in-unit-order"),
            "an Unknown-class unit after the output slot must still fire the grammar; report: {r}"
        );
    }

    #[test]
    fn unknown_class_earlier_unit_does_not_consume_output_slot() {
        // An Unknown-output-class earlier unit does NOT consume the output slot (the validator
        // cannot confirm it is the layer's coded output frame unit), so a later unit in the
        // same layer does not fire the output-slot grammar (drop, never guess).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                None,
                None,
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        assert!(
            !has(&r, "celu/in-unit-order"),
            "an Unknown earlier unit must not consume the output slot; report: {r}"
        );
    }

    #[test]
    fn non_outputs_then_output_in_layer_is_silent_on_output_slot_grammar() {
        // The conformant shape: zero or more non-output units THEN the output unit. The output
        // unit is last, so the grammar stays silent.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 2),
            output_frame(0),
            &mut r,
        );
        assert!(
            !has(&r, "celu/in-unit-order"),
            "non-output units then a final output unit is conformant; report: {r}"
        );
    }

    // --- header-only CELU presence (Finding 3, mirror line 536) ---

    #[test]
    fn header_only_celu_fires_missing_output() {
        // A CELU consisting only of HLS-header / CI / interior OBUs (≥ 1 non-padding OBU at a
        // non-global xlayer, zero frame-bearing OBUs) has no coded output frame unit, so § 7.3.6
        // line 536 fires. Here an LCR-only CELU at a non-global xlayer.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::LayerConfigurationRecord, 0, 0, 0),
            CeluRole::LayerConfigurationRecord,
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(
            has(&r, "celu/missing-output-frame-unit"),
            "a header-only CELU must fire missing-output; report: {r}"
        );
    }

    #[test]
    fn padding_only_xlayer_group_is_silent() {
        // A padding-only (or reserved-type-only, which celu_role_for maps to Padding) xlayer
        // group is NOT a CELU and must stay silent — padding never opens a CELU.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(&obu(ObuType::Padding, 0, 0, 0), CeluRole::Padding, &mut r);
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/missing-output-frame-unit"),
            "a padding-only group must not constitute a CELU; report: {r}"
        );
    }

    #[test]
    fn frame_bearing_with_unknown_classes_is_silent_on_missing_output() {
        // A CELU with a frame-bearing OBU whose output class is Unknown must still drop the
        // missing-output rule (the existing Unknown-invariant behavior must keep passing): the
        // header-only branch does not apply (a frame-bearing OBU exists).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                None,
                None,
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/missing-output-frame-unit"),
            "a frame-bearing CELU with an Unknown output class must drop missing-output; \
             report: {r}"
        );
    }

    // --- same-OrderHint across output units ---

    #[test]
    fn output_units_with_different_order_hint_is_flagged() {
        // The same-OrderHint judgment resolves at the CELU (temporal-unit) boundary, so a
        // later undecidable output unit can still drop it (the Unknown invariant).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(3),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 1),
            output_frame(7),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(has(&r, "celu/output-order-hint-mismatch"), "report: {r}");
    }

    #[test]
    fn output_units_with_same_order_hint_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(5),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 1),
            output_frame(5),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(!has(&r, "celu/output-order-hint-mismatch"), "report: {r}");
    }

    #[test]
    fn output_order_hint_mismatch_drops_when_an_output_unit_hint_is_unknown() {
        // Unknown invariant: an output unit whose order_hint could not be read drops the
        // same-OrderHint judgment for the CELU — even when the decidable output units already
        // disagreed (the rule rests on the full set of output units).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(3),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(true),
                None,
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 2, 2),
            output_frame(7),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/output-order-hint-mismatch"),
            "an undecidable output order_hint must drop the rule; report: {r}"
        );
    }

    // --- CLK / OLK rules ---

    #[test]
    fn clk_and_olk_mixed_is_flagged() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 0, 0),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::OpenLoopKey, 0, 1, 1),
            frame_role(
                ObuType::OpenLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        assert!(has(&r, "celu/clk-olk-mixed"), "report: {r}");
    }

    #[test]
    fn clk_not_in_first_unit_of_layer_is_flagged() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // mlayer 0: first unit a regular output frame, then a CLK opening a SECOND unit.
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 0, 1),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        assert!(has(&r, "celu/key-not-in-first-unit"), "report: {r}");
    }

    #[test]
    fn clk_in_first_unit_of_layer_is_silent_on_first_unit_rule() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 0, 0),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        assert!(!has(&r, "celu/key-not-in-first-unit"), "report: {r}");
    }

    #[test]
    fn clk_lowest_layer_not_key_is_flagged() {
        // A CELU contains a CLK, but the lowest embedded layer's first unit is a regular
        // frame (CLK is at a higher layer).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 1, 1),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(has(&r, "celu/lowest-layer-not-key"), "report: {r}");
    }

    #[test]
    fn clk_lowest_layer_is_key_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // Lowest layer (mlayer 0) first unit is a CLK; higher layer is a CLK too (no OLK mix).
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 0, 0),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 1, 1),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(!has(&r, "celu/lowest-layer-not-key"), "report: {r}");
    }

    // --- all-leading-or-none ---

    #[test]
    fn mixed_leading_and_non_leading_is_flagged() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::LeadingTileGroup, 0, 0, 0),
            frame_role(
                ObuType::LeadingTileGroup,
                true,
                Some(true),
                Some(0),
                Leadingness::Leading,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(true),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        assert!(has(&r, "celu/leading-frame-mix"), "report: {r}");
    }

    #[test]
    fn clk_base_layer_with_regular_higher_layer_is_silent_on_leading_mix() {
        // Regression (celu/leading-frame-mix false positive): a CLK in the base embedded
        // layer (embedded layer 0) plus a Regular frame unit in a higher embedded layer is
        // the structure the § 7.3.6 CLK rule explicitly contemplates (mirror lines 541-549:
        // higher embedded layers' first units may be non-CLK regular frames). § 5.18.2
        // IsRegular classes a CLK as non-regular, but the AVM oracle tri-states a CLK as
        // INDETERMINATE (obu.c:2544-2549), neither leading nor regular. The mix rule must
        // therefore stay SILENT — the CLK is excluded from the all-leading-or-none judgment.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 0, 0),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 1, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(true),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        assert!(
            !has(&r, "celu/leading-frame-mix"),
            "a CLK (indeterminate leading-ness) with a regular frame must not fire the mix \
             rule; report: {r}"
        );
    }

    #[test]
    fn clk_with_leading_is_silent_on_leading_mix() {
        // A CLK (indeterminate) coexisting with a LEADING_* frame must also stay silent: the
        // indeterminate unit is excluded entirely, leaving an all-leading CELU.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 0, 0),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(true),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::LeadingTileGroup, 0, 1, 1),
            frame_role(
                ObuType::LeadingTileGroup,
                true,
                Some(true),
                Some(0),
                Leadingness::Leading,
            ),
            &mut r,
        );
        assert!(
            !has(&r, "celu/leading-frame-mix"),
            "a CLK (indeterminate) with a leading frame must not fire the mix rule; report: {r}"
        );
    }

    #[test]
    fn all_leading_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::LeadingTileGroup, 0, 0, 0),
            frame_role(
                ObuType::LeadingTileGroup,
                true,
                Some(true),
                Some(0),
                Leadingness::Leading,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::LeadingSef, 0, 1, 1),
            frame_role_with_boundary(
                ObuType::LeadingSef,
                FrameBoundary::OpensNewUnit,
                Some(true),
                Some(0),
                Leadingness::Leading,
            ),
            &mut r,
        );
        assert!(!has(&r, "celu/leading-frame-mix"), "report: {r}");
    }

    // --- CELU-scoped CI rule ---

    #[test]
    fn ci_outside_first_frame_unit_is_flagged() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // First frame unit for mlayer 0, then a CI -> the CI heads a later unit.
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.observe(
            &obu(ObuType::ContentInterpretation, 0, 0, 1),
            CeluRole::ContentInterpretation,
            &mut r,
        );
        assert!(
            has(&r, "celu/content-interpretation-not-in-first-unit"),
            "report: {r}"
        );
    }

    #[test]
    fn ci_in_first_frame_unit_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // CI before the layer's first coded frame -> in the first unit.
        t.observe(
            &obu(ObuType::ContentInterpretation, 0, 0, 0),
            CeluRole::ContentInterpretation,
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 1),
            output_frame(0),
            &mut r,
        );
        assert!(
            !has(&r, "celu/content-interpretation-not-in-first-unit"),
            "report: {r}"
        );
    }

    // --- DOH OrderHint (cross-CELU) ---

    #[test]
    fn doh_cross_celu_order_hint_mismatch_under_flag_is_flagged() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // xlayer 0 output OrderHint 1; xlayer 1 output OrderHint 2; DOH flag active. Both
        // frames carry the SAME, KNOWN OrderHintBits, so the cross-CELU OrderHint comparison
        // (an LSB proxy) is sound: same-width LSBs that differ imply different OrderHints.
        t.note_order_hint_bits(Some(4), ByteOffset::new(0));
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(1),
            &mut r,
        );
        t.note_order_hint_bits(Some(4), ByteOffset::new(1));
        t.observe(
            &obu(ObuType::RegularTileGroup, 1, 0, 1),
            output_frame(2),
            &mut r,
        );
        t.set_doh_flag_active(true);
        t.reset_temporal_unit(&mut r);
        assert!(has(&r, "celu/doh-order-hint-mismatch"), "report: {r}");
    }

    #[test]
    fn doh_cross_celu_order_hint_mismatch_drops_when_bits_differ() {
        // Finding B gate: the cross-CELU §7.3.7 DOH OrderHint comparison is an LSB proxy; it
        // is UNSOUND when the two layers' OrderHintBits differ (equal decoded OrderHints can
        // carry different-width LSB encodings -> a false positive). When the bits differ, the
        // celu/doh-order-hint-bits-mismatch rule already fires; the OrderHint comparison must
        // then DROP. Two CELUs, DOH flag on, KNOWN but different OrderHintBits, LSBs differ:
        // only doh-order-hint-bits-mismatch fires, doh-order-hint-mismatch is SILENT.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.note_order_hint_bits(Some(4), ByteOffset::new(0));
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(1),
            &mut r,
        );
        t.note_order_hint_bits(Some(5), ByteOffset::new(1)); // different OrderHintBits
        t.observe(
            &obu(ObuType::RegularTileGroup, 1, 0, 1),
            output_frame(2),
            &mut r,
        );
        t.set_doh_flag_active(true);
        t.reset_temporal_unit(&mut r);
        assert!(
            has(&r, "celu/doh-order-hint-bits-mismatch"),
            "the differing OrderHintBits must fire the bits-mismatch rule; report: {r}"
        );
        assert!(
            !has(&r, "celu/doh-order-hint-mismatch"),
            "the cross-CELU OrderHint comparison must DROP when OrderHintBits differ (the LSB \
             proxy is unsound across different bit widths); report: {r}"
        );
    }

    #[test]
    fn doh_cross_celu_order_hint_mismatch_drops_when_bits_unknown() {
        // Finding B gate: when one participating frame's OrderHintBits is unknown, the
        // cross-CELU OrderHint comparison cannot be confirmed sound (the LSB widths may
        // differ), so it DROPS — even though the LSBs differ and the flag is set.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.note_order_hint_bits(Some(4), ByteOffset::new(0));
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(1),
            &mut r,
        );
        t.note_order_hint_bits(None, ByteOffset::new(1)); // unknown OrderHintBits
        t.observe(
            &obu(ObuType::RegularTileGroup, 1, 0, 1),
            output_frame(2),
            &mut r,
        );
        t.set_doh_flag_active(true);
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/doh-order-hint-mismatch"),
            "an unknown OrderHintBits must drop the cross-CELU OrderHint comparison; report: {r}"
        );
    }

    #[test]
    fn doh_cross_celu_order_hint_mismatch_without_flag_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(1),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 1, 0, 1),
            output_frame(2),
            &mut r,
        );
        // DOH flag NOT set -> the cross-CELU agreement is not required.
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/doh-order-hint-mismatch"),
            "the DOH check must stay silent with the flag off; report: {r}"
        );
    }

    #[test]
    fn doh_cross_celu_order_hint_agreement_under_flag_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(4),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTileGroup, 1, 0, 1),
            output_frame(4),
            &mut r,
        );
        t.set_doh_flag_active(true);
        t.reset_temporal_unit(&mut r);
        assert!(!has(&r, "celu/doh-order-hint-mismatch"), "report: {r}");
    }

    #[test]
    fn doh_cross_celu_order_hint_drops_when_a_celu_hint_is_unknown() {
        // Unknown invariant: a CELU whose output OrderHint is undecidable drops the
        // cross-CELU agreement judgment even under the flag.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(1),
            &mut r,
        );
        // xlayer 1's only output unit has an unreadable order_hint -> undecidable CELU hint.
        t.observe(
            &obu(ObuType::RegularTileGroup, 1, 0, 1),
            frame_role(
                ObuType::RegularTileGroup,
                true,
                Some(true),
                None,
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.set_doh_flag_active(true);
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/doh-order-hint-mismatch"),
            "an undecidable CELU OrderHint must drop the cross-CELU rule; report: {r}"
        );
    }

    // --- DOH OrderHintBits ---

    #[test]
    fn doh_order_hint_bits_mismatch_under_flag_is_flagged() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.note_order_hint_bits(Some(4), ByteOffset::new(0));
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            output_frame(0),
            &mut r,
        );
        t.note_order_hint_bits(Some(5), ByteOffset::new(1));
        t.observe(
            &obu(ObuType::RegularTileGroup, 1, 0, 1),
            output_frame(0),
            &mut r,
        );
        t.set_doh_flag_active(true);
        t.reset_temporal_unit(&mut r);
        assert!(has(&r, "celu/doh-order-hint-bits-mismatch"), "report: {r}");
    }

    #[test]
    fn doh_order_hint_bits_mismatch_without_flag_is_silent() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.note_order_hint_bits(Some(4), ByteOffset::new(0));
        t.note_order_hint_bits(Some(5), ByteOffset::new(1));
        t.reset_temporal_unit(&mut r);
        assert!(!has(&r, "celu/doh-order-hint-bits-mismatch"), "report: {r}");
    }

    #[test]
    fn doh_order_hint_bits_drops_when_undecidable() {
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.note_order_hint_bits(Some(4), ByteOffset::new(0));
        t.note_order_hint_bits(None, ByteOffset::new(1)); // undecidable -> drop
        t.note_order_hint_bits(Some(5), ByteOffset::new(2));
        t.set_doh_flag_active(true);
        t.reset_temporal_unit(&mut r);
        assert!(
            !has(&r, "celu/doh-order-hint-bits-mismatch"),
            "an undecidable OrderHintBits must drop the rule; report: {r}"
        );
    }

    // --- no-delimiter boundary (TIP/bridge): segmenter-authoritative (Finding 2) ---

    #[test]
    fn ambiguous_run_poisons_key_not_in_first_unit() {
        // The same-type no-delimiter case is now AMBIGUOUS, not a silent merge: the segmenter
        // cannot decide whether the second same-type TIP continues the open coded frame or
        // begins a new same-type one, so it reports FrameBoundary::Ambiguous. That poisons the
        // embedded layer's unit-count-dependent judgments. A CLK opening immediately after the
        // run (a decided new unit, different type) would otherwise look like the layer's second
        // unit and fire key-not-in-first-unit — but the poison drops that judgment (its unit
        // index rests on the undecided run). Zero false positives in doubt.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // First TIP opens unit 0 (OpensNewUnit from the segmenter).
        t.observe(
            &obu(ObuType::RegularTip, 0, 0, 0),
            frame_role(
                ObuType::RegularTip,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        // Second same-type TIP: Ambiguous -> poisons the layer.
        t.observe(
            &obu(ObuType::RegularTip, 0, 0, 1),
            ambiguous_role(
                ObuType::RegularTip,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        // A CLK now opens a decided new unit at the same mlayer; the poison drops
        // key-not-in-first-unit (the layer's unit count is undecided).
        t.observe(
            &obu(ObuType::ClosedLoopKey, 0, 0, 2),
            frame_role(
                ObuType::ClosedLoopKey,
                true,
                Some(false),
                Some(0),
                Leadingness::Indeterminate,
            ),
            &mut r,
        );
        assert!(
            !has(&r, "celu/key-not-in-first-unit"),
            "an Ambiguous boundary in the layer must drop key-not-in-first-unit; report: {r}"
        );
    }

    #[test]
    fn tip_brt_tip_yields_two_units() {
        // Finding 2 regression: TIP, BRT, TIP. The FrameUnitSegmenter splits at the BRT (a new
        // unit head after the first TIP's coded frame), so the second TIP reports
        // FrameBoundary::OpensNewUnit (a brand-new unit), not a same-type continuation that an
        // obu_type-only comparison would have merged. The second unit's facts are therefore
        // enforced: it is a decided NON-OUTPUT unit opening after the first (decided OUTPUT)
        // unit consumed the layer's output slot, so the output-slot grammar (celu/in-unit-order)
        // fires — proving the two TIPs counted as two units.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        // First TIP: a decided OUTPUT unit (consumes the output slot). The intervening BRT is a
        // FrameInterior OBU between them in the real stream; the segmenter resolves the split,
        // and here we feed the segmenter-decided OpensNewUnit boundary for the second TIP.
        t.observe(
            &obu(ObuType::RegularTip, 0, 0, 0),
            frame_role(
                ObuType::RegularTip,
                true,
                Some(true),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        // Second TIP after the BRT split: OpensNewUnit, a decided NON-OUTPUT unit after the
        // output slot.
        t.observe(
            &obu(ObuType::RegularTip, 0, 0, 2),
            frame_role(
                ObuType::RegularTip,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        assert!(
            has(&r, "celu/in-unit-order"),
            "the second TIP is a new unit after the output slot; report: {r}"
        );
    }

    #[test]
    fn ambiguous_run_with_no_output_fires_missing_output_on_decided_unit() {
        // A same-type no-delimiter run with no output unit: the first TIP opens a decided
        // non-output unit; the second is Ambiguous (poison). The decided non-output unit gives
        // the CELU a decided unit with no output, so missing-output-frame-unit fires on it
        // (the decided unit, not the ambiguous one), and no spurious in-unit-order fires (a
        // single mlayer, no output slot consumed).
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTip, 0, 1, 0),
            frame_role(
                ObuType::RegularTip,
                true,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.observe(
            &obu(ObuType::RegularTip, 0, 1, 1),
            ambiguous_role(
                ObuType::RegularTip,
                Some(false),
                Some(0),
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.reset_temporal_unit(&mut r);
        assert!(
            has(&r, "celu/missing-output-frame-unit") && !has(&r, "celu/in-unit-order"),
            "the decided non-output unit fires missing-output; the ambiguous unit is silent; \
             report: {r}"
        );
    }

    #[test]
    fn fully_unknown_celu_is_silent() {
        // A CELU whose only frame has every derived fact undecidable fires nothing.
        let mut t = fresh();
        let mut r = ValidationReport::new();
        t.observe(
            &obu(ObuType::RegularTileGroup, 0, 0, 0),
            frame_role_with_boundary(
                ObuType::RegularTileGroup,
                FrameBoundary::OpensNewUnit,
                None,
                None,
                Leadingness::Regular,
            ),
            &mut r,
        );
        t.set_doh_flag_active(true);
        t.reset_temporal_unit(&mut r);
        assert!(
            r.diagnostics.is_empty(),
            "a fully-Unknown CELU must be silent; report: {r}"
        );
    }
}
