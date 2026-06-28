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
//! An [`FrameBoundary::Ambiguous`] boundary (a same-`obu_type` no-delimiter TIP **or
//! bridge**, or an unreadable tile-group delimiter, while a coded frame is open) means the
//! segmenter could not decide whether the OBU opened a new unit or continued the open one. Its
//! existence as a new unit is undecided, so it changes the embedded layer's **unit
//! count / index** — but *only* that. The former "same-type-no-delimiter run is one unit"
//! behaviour is replaced by this poison-on-ambiguity semantics, keeping zero false positives
//! when in doubt. A same-type bridge adjacency is included here (round-7 F2): the unit count is
//! ambiguous even though the bridge's output class is type-decided non-output (the boundary
//! ambiguity is about unit count, not class).
//!
//! ### The poison-scope rule: index-quantified judgments drop, decided-pair-order survive
//!
//! An ambiguity changes the unit COUNT/INDEX, **not** the relative order of two *decided* units
//! (an [`FrameBoundary::OpensNewUnit`] is the segmenter's decided split, which no resolution of
//! the ambiguity can un-split or merge). So the poison is scoped to exactly what it can change
//! (round-8 F1/F2):
//!
//! - **Index-quantified judgments drop.** A judgment whose truth depends on an *exact* unit
//!   index or count is dropped for the poisoned layer: the per-unit accounting / OrderHint
//!   accumulators (the ambiguous OBU opens no counted unit), and the **output-presence**
//!   judgments — "is there NO coded output frame unit in this layer / CELU" — which the ambiguous
//!   OBU could itself satisfy. See below for the output-presence precision.
//! - **Decided-pair-order judgments survive.** A judgment that needs only "a *decided* unit X
//!   precedes a *decided* unit Y in this layer" no longer consults the poison, because the
//!   ambiguous OBU lies *between* (or before) two decided units and cannot reorder them. Two such
//!   judgments survive: the within-layer **output-slot grammar** (a slot consumed by a decided
//!   output unit → any *later decided* `OpensNewUnit` unit fires `celu/in-unit-order`, F1) and
//!   **`celu/key-not-in-first-unit`** (a CLK/OLK opening a decided unit when a *decided earlier*
//!   unit already exists — `units_opened >= 1`, counting only decided units — is provably not the
//!   layer's first coded frame unit, F2).
//!
//! The dropped case for `celu/key-not-in-first-unit` is the **asymmetric** one: when the KEY
//! ITSELF is the first DECIDED unit (`units_opened == 0`) but an ambiguous OBU preceded it, the
//! ambiguous OBU might or might not have been an earlier unit, so "not first" would rest on a
//! guess — that case stays dropped (it is silent by the same `units_opened >= 1` guard). The
//! lowest-layer rule **`celu/lowest-layer-not-key`** shares this asymmetry by construction: its
//! predicate is about the identity of the **first** unit of the lowest layer, and an ambiguous
//! OBU before the candidate first unit leaves the identity of "first" unknown — there is no
//! decided-pair-order reformulation for "the first unit", so it stays dropped.
//!
//! ### Output-presence precision: only poison what the ambiguity can change
//!
//! An ambiguous OBU might open a new coded frame unit whose class *could be output*, so it
//! poisons the **output-presence** judgments — the CELU-scoped `celu/missing-output-frame-unit`
//! rule and the per-layer `celu/non-output-without-output` rule — but **only when its output
//! class is not type-decided non-output**. The precision boundary: a same-type undecided-class
//! ambiguous tile-group / TIP (`output == None`) might be the missing output unit, so it
//! poisons both (CELU-level for missing-output, layer-level for non-output-without-output); an
//! ambiguous **BRIDGE** is type-decided non-output (`output == Some(false)`) whichever way the
//! boundary resolves, so it can never satisfy output presence and must **not** poison those
//! rules — over-poisoning a bridge would hide a genuine missing-output. The CLK/OLK identity
//! and leading-ness facts are type-decided and therefore recorded *before* the ambiguity drops
//! the unit-count facts (see [`CodedExtendedLayerTracker::observe_frame`]'s
//! [`FrameBoundary::Ambiguous`] arm): an OLK plus an ambiguous CLK still fires
//! `celu/clk-olk-mixed`, and a LEADING frame plus an ambiguous Regular-typed OBU still fires
//! `celu/leading-frame-mix`.
//!
//! ## Ascending-`obu_mlayer_id` ordering counts frame-unit heads
//!
//! A coded frame unit's constituents (§ 7.3.3) include not just the coded frame but its head /
//! pre-frame OBUs (CI, BRT, QM, FGM, prefix metadata, MFH) and its suffix-metadata tail. The
//! ascending-`obu_mlayer_id` frame-unit ordering rule (`celu/in-unit-order`, mirror line 525)
//! therefore advances `max_embedded_seen` on **every** frame-unit-constituent OBU at its own
//! `obu_mlayer_id` — the [`CeluRole::ContentInterpretation`] head and the
//! [`CeluRole::FrameInterior`] constituents as well as the coded-`Frame` OBU
//! ([`CodedExtendedLayerTracker::note_embedded_layer_ordering`]) — so a CI heading a higher
//! embedded layer's unit makes a later lower-mlayer coded frame out of order. A suffix metadata
//! belongs to the just-closed unit of its **own** mlayer (the same mlayer as the frame it
//! follows), so it never lowers `max_embedded_seen` and monotonicity is unaffected (no
//! special-casing needed). `OBU_PADDING` is position-free and stays excluded.
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
//! This is a **decided-pair-order** judgment, so it survives the ambiguity poison (round-8 F1):
//! both the slot-consuming output unit and the later unit are decided, and an intervening
//! ambiguous OBU cannot reorder them.
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
//! always silent. A frame-bearing CELU whose output classes are all Unknown — or which carries
//! an ambiguous OBU that could itself be an output unit (the ambiguity-poison precision rule
//! above) — still drops the rule (the Unknown invariant).
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
//!
//! ## The two § 7.3.7 DOH constraints are independently gated (round-6 F2)
//!
//! When a DOH constraint flag is set (mirror lines 650-657), § 7.3.7 imposes **two distinct**
//! requirements, each gated separately in [`DohTuAccumulator`]:
//!
//! 1. **All frame units in the temporal unit share one `OrderHintBits`** (line 655). Judged
//!    over **every** frame unit of the temporal unit; an unknown-bits frame unit (output or
//!    not) drops this judgment (the Unknown invariant) — `celu/doh-order-hint-bits-mismatch`.
//! 2. **Coded output frame units in multiple CELUs share one `OrderHint`** (lines 656-657).
//!    The validator compares the `order_hint` LSB **proxy** for the decoded OrderHint, which is
//!    sound only when the two **compared** output units share one known `OrderHintBits` (equal
//!    decoded OrderHints can carry different-width LSB encodings). The soundness gate is applied
//!    **per compared pair** on the two output CELUs' own bits (carried alongside each output
//!    sample in [`FrameFacts::order_hint_bits`]) — **not** via constraint (1)'s
//!    temporal-unit-wide same-bits judgment. So an unknown-bits non-output (or unrelated) frame
//!    unit elsewhere in the temporal unit drops constraint (1) but does **not** suppress a
//!    decidable constraint (2) mismatch between two output CELUs whose own bits are known and
//!    equal — `celu/doh-order-hint-mismatch`. To realise the per-pair gate over *all* CELUs (not
//!    just against the first sample), the output-CELU samples are **grouped by their known
//!    `OrderHintBits` value** and each is compared to its own group's representative (round-8 F3):
//!    samples (bits, hint) = (4, 0), (5, 1), (5, 2) then prove the (5, 1)/(5, 2) within-group
//!    mismatch that a compare-only-to-the-first-sample scheme would miss (both fail the equal-bits
//!    gate against the (4, 0) representative). An unknown-bits output sample stays out of all
//!    groups. When two output units have known but **unequal** bits they land in different groups,
//!    so constraint (2) never compares them; constraint (1) fires the bits-mismatch instead.
//!
//! ## The § 7.3.6 first-CELU-of-the-sequence CI presence rule lives in `context` (round-6 F3)
//!
//! Mirror lines 560-562: "If an OBU_CONTENT_INTERPRETATION is present in any coded extended
//! layer unit, this OBU shall also be present in the first coded extended layer unit of the
//! sequence ... for a given embedded layer." The **contents-identity** half ("the same contents
//! in all its repetitions") is owned by `content-interpretation/repeated-ci-not-identical`
//! (§ 6.14). The **presence** half — a later CELU carries a CI for an embedded layer the coded
//! video sequence's first CELU lacked — is **coded-video-sequence-scoped**, so it cannot live in
//! this per-temporal-unit tracker; it is implemented in [`crate::context`]
//! (`ValidatorContext::resolve_ci_first_celu_for_tu`, `celu/content-interpretation-not-in-first-celu`),
//! which holds the per-extended-layer CVS epoch and the external-HLS surface. It drops when the
//! first CELU of the sequence was not observed (a mid-CVS join, or an external-HLS `Provided`
//! mode whose unenumerable external CI could be the first CELU's). The CELU-scoped
//! first-*frame-unit* CI rule (mirror lines 557-559, `celu/content-interpretation-not-in-first-unit`)
//! and the § 7.3.8.10 temporal-unit form (`frame-unit/ci-not-in-first-frame-unit`) remain
//! distinct.

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
    /// The `OrderHintBits` of this frame (from its active sequence header), when the core
    /// parse resolved it against the referenced header (the stale-activation guard); `None`
    /// otherwise. This is the SAME value the validator threads to the temporal-unit-wide
    /// [`CodedExtendedLayerTracker::note_order_hint_bits`] for the § 7.3.7 same-bits judgment
    /// (constraint 1), but here it is carried PER OUTPUT UNIT so the cross-CELU OrderHint
    /// comparison (constraint 2, mirror lines 656-657) can be gated on only the two COMPARED
    /// output units' bits being known and equal — independent of an unrelated non-output /
    /// unknown-bits frame unit elsewhere in the temporal unit (round-6 F2). The cross-CELU
    /// `order_hint` LSB proxy is sound exactly when the compared units share one known
    /// OrderHintBits (equal-width LSBs that differ imply different decoded OrderHints).
    pub order_hint_bits: Option<u32>,
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
    /// frame unit must be last), regardless of its own output class. An `Ambiguous` OBU
    /// neither consumes the slot nor itself triggers the grammar (its existence as a separate
    /// unit is undecided), but it does NOT suppress the grammar for a *later decided* unit
    /// (round-8 F1): the slot-consuming output unit and the later decided unit are both decided,
    /// so their relative order is a decided-pair-order fact the intervening ambiguity cannot
    /// change.
    output_slot_consumed: bool,
    /// `true` once the output-slot grammar (`celu/in-unit-order`) has fired for this
    /// embedded layer, so a run of later units after the output slot reports once.
    output_slot_grammar_reported: bool,
    /// `true` once an [`FrameBoundary::Ambiguous`] OBU in this embedded layer could itself be a
    /// coded *output* frame unit — its output class is not type-decided non-output
    /// (`facts.output != Some(false)`), so the ambiguous OBU might open the layer's output unit
    /// whichever way the boundary resolves (F1). The per-layer non-output-implies-output
    /// judgment (mirror lines 537-538) is then dropped for this layer: the validator cannot
    /// confirm the layer lacks a coded output frame unit. An ambiguous BRIDGE (type-decided
    /// non-output) does NOT set this — it can never satisfy output presence, so it must not
    /// suppress the rule.
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
    /// `true` once an [`FrameBoundary::Ambiguous`] OBU in this CELU could itself be a coded
    /// *output* frame unit — its output class is not type-decided non-output
    /// (`facts.output != Some(false)`), so the ambiguous OBU might open a coded output frame
    /// unit whichever way the boundary resolves (F1). The CELU-scoped output-presence rule
    /// (`celu/missing-output-frame-unit`, mirror line 536) is then dropped: the validator
    /// cannot confirm the CELU lacks a coded output frame unit. An ambiguous BRIDGE (type-
    /// decided non-output) does NOT set this — it can never satisfy output presence.
    missing_output_poisoned: bool,
    /// The shared `OrderHint` (an `order_hint` LSB proxy — see [`FrameFacts::order_hint`]) of
    /// the output units seen so far, the `OrderHintBits` of the first output unit (for the
    /// cross-CELU §7.3.7 comparison gate, round-6 F2), and the offset of the first output
    /// unit; `None` until the first output unit with a readable `order_hint`. Within one CELU
    /// all frame units share one active header, hence one OrderHintBits, so the LSB comparison
    /// is a sound under-approximation here (no cross-width gate needed — unlike the cross-CELU
    /// check, which gates on the COMPARED CELUs' output-unit bits being known and equal).
    output_order_hint: Option<(u32, Option<u32>, ByteOffset)>,
    /// `true` once an output unit's `order_hint` could not be read. The CELU's single
    /// "associated order hint" cannot then be confirmed, so the CELU is **not contributed** to
    /// the cross-CELU DOH accumulator (feeding the known units' value would be a guess —
    /// round-7 F5). It does **not** suppress this CELU's own in-CELU
    /// [`Self::order_hint_mismatch`], which is already proven between two known output units.
    order_hint_undecidable: bool,
    /// The (first, found, anchor) of the first output unit whose `OrderHint` disagreed with
    /// [`Self::output_order_hint`], if any. Detected eagerly between two KNOWN output units and
    /// **emitted** at [`CodedExtendedLayerTracker::resolve_celu`] regardless of whether another
    /// output unit's `order_hint` was undecidable (round-7 F3): an undecidable member can only
    /// prevent proving agreement, never excuse a pair already proven to differ.
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
/// The cross-CELU comparison is gated in [`Self::note_celu_output_order_hint`] on the two
/// COMPARED output units sharing one KNOWN OrderHintBits: across different bit widths equal
/// decoded OrderHints can carry different-width LSB encodings, so the proxy would
/// false-positive. The gate is over the *compared* output units' own bits (carried alongside
/// each output sample, round-6 F2) — NOT the temporal-unit-wide same-bits judgment, which
/// also covers non-output and unrelated frame units. §7.3.7 has two distinct constraints:
/// (1) all frame units in the temporal unit share one OrderHintBits (the
/// [`Self::bits_mismatch`] judgment over EVERY frame unit), and
/// (2) coded OUTPUT frame units in multiple CELUs share one OrderHint (this cross-CELU
/// comparison, sound when only the compared output units' bits agree). An unknown-bits
/// non-output frame unit drops constraint (1) but must NOT suppress a decidable constraint
/// (2) mismatch between two output CELUs whose own bits are known and equal.
///
/// **Proven mismatches ignore undecidable participants (round-7 F3/F4/F5).** Both
/// [`Self::bits_mismatch`] and [`Self::order_hint_mismatch`] are recorded ONLY between two
/// KNOWN samples (constraint (2)'s additionally between a known-and-equal-bits pair). An
/// undecidable frame / CELU is never recorded as a mismatch and no longer suppresses one
/// already proven: an unknown can prevent proving AGREEMENT (never reported) but cannot make
/// a proven differing pair conforming. So [`Self::resolve`] emits a recorded mismatch
/// unconditionally (still flag-gated by the caller); the dropped per-temporal-unit
/// "undecidable" flags are gone.
#[derive(Debug, Default)]
struct DohTuAccumulator {
    /// The per-`OrderHintBits` representative output OrderHint of the CELUs seen so far, keyed
    /// by the KNOWN bits value: each group's first decidable output-CELU sample (its OrderHint
    /// LSB proxy and anchor offset). Grouping is the round-8 F3 fix — the LSB proxy is sound
    /// only WITHIN one known OrderHintBits width, so a later output CELU is compared to its own
    /// group's representative rather than only to the very first sample of any width. An
    /// unknown-bits output CELU stays out of all groups (never compared, never recorded — the
    /// Unknown invariant for that sample); a known-but-different-bits pair is covered by
    /// constraint (1) `celu/doh-order-hint-bits-mismatch`, not by this cross-width comparison.
    output_order_hint_by_bits: BTreeMap<u32, (u32, ByteOffset)>,
    /// The (representative-value, found-value, anchor) of the first within-group output-CELU
    /// OrderHint (LSB proxy) that disagreed with its bits-group's representative, if any —
    /// emitted at [`Self::resolve`]. Recorded only WITHIN one known OrderHintBits group (so the
    /// LSB proxy is sound for that pair); deduplicated to one emission per temporal unit (the
    /// first within-group disagreement found across all groups), anchored at the offending later
    /// sample. A pair whose compared bits are unknown or unequal does NOT record a mismatch here
    /// (the proxy is unsound across widths; a known-but-unequal pair is instead covered by
    /// `celu/doh-order-hint-bits-mismatch` from constraint (1)).
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
                // rule is judged against the embedded layer's already-opened unit count. As a
                // frame-unit constituent it still participates in the ascending-`obu_mlayer_id`
                // ordering accounting (mirror line 525): a CI heading a higher embedded layer's
                // unit evidences that unit has begun, so a later lower-mlayer frame unit is out
                // of order (F2).
                celu.phase = CeluPhase::Frames;
                Self::note_embedded_layer_ordering(celu, embedded, obu, report);
                Self::observe_ci(celu, embedded, obu, report);
            }
            CeluRole::FrameInterior => {
                // BRT / QM / FGM / metadata / MFH: the frame region has begun, but this OBU
                // neither opens a coded frame nor is an HLS header. The within-frame-unit
                // grammar is owned by the FrameUnitSegmenter. As a frame-unit constituent
                // (§ 7.3.3 head / pre-frame OBU or suffix-metadata tail) it participates in the
                // ascending-`obu_mlayer_id` ordering accounting with its own `obu_mlayer_id`
                // (F2). A suffix metadata belongs to the just-closed unit of its own mlayer —
                // the same mlayer as the frame it follows — so it never lowers
                // `max_embedded_seen` and leaves monotonicity unaffected (no special-casing).
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

    /// Threads one frame-unit-constituent OBU's `obu_mlayer_id` into the ascending-mlayer
    /// frame-unit ordering accounting (mirror line 525), reporting `celu/in-unit-order` when
    /// the embedded layer is *below* the highest seen so far. A coded frame unit's constituents
    /// (§ 7.3.3) include its head / pre-frame OBUs (CI, BRT, QM, FGM, prefix metadata, MFH) and
    /// the coded frame itself, plus the suffix-metadata tail — each evidences its embedded
    /// layer's frame unit has begun, so every one of them participates with its own mlayer (F2).
    /// A suffix metadata shares the just-closed unit's mlayer, so it never lowers
    /// `max_embedded_seen` and monotonicity is unaffected. Padding is excluded (it never reaches
    /// here — [`Self::observe`] returns early on padding).
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
    /// (mirror line 554). The identity is **type-decided** from `obu_type`, so it is recorded
    /// for every frame-bearing OBU regardless of its coded-frame-unit boundary — including an
    /// [`FrameBoundary::Ambiguous`] one (F5): a CELU contains both a CLK and an OLK whichever
    /// way the ambiguous boundary resolves, so the mix is a boundary-independent fact.
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
    /// lines 555-556). The rule fires only when a decidable [`Leadingness::Leading`] unit and a
    /// decidable [`Leadingness::Regular`] unit coexist; [`Leadingness::Indeterminate`] (a CLK)
    /// is excluded entirely (neither a trigger nor an offender — see [`Leadingness`]: the spec
    /// text and the AVM oracle conflict on whether a CLK is "leading", so the validator
    /// under-reports per the ambiguous-spec policy). Leading-ness is **type-decided** from
    /// `obu_type`, so it is recorded for every frame-bearing OBU regardless of its
    /// coded-frame-unit boundary — including an [`FrameBoundary::Ambiguous`] one (F5): a
    /// LEADING-/Regular-typed OBU evidences a leading/regular frame unit whichever unit it
    /// belongs to (frames of one unit share one `obu_type`, so the unit's character is
    /// evidenced by any constituent).
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
    /// unit; an [`FrameBoundary::ContinuesUnit`] OBU is transparent for the unit-count-
    /// dependent judgments (a later OBU of an already-counted coded frame); an
    /// [`FrameBoundary::Ambiguous`] OBU's existence as a new unit is undecided, so it neither
    /// opens a unit nor feeds the accumulators — it only *poisons* the embedded layer's
    /// unit-count-dependent judgments (never guessed — the Unknown invariant).
    ///
    /// The ascending-`obu_mlayer_id` ordering ([`Self::note_embedded_layer_ordering`]) is
    /// **boundary-independent** (F3): every frame OBU belongs to *some* frame unit of its
    /// embedded layer whichever way its boundary resolves — a `ContinuesUnit` OBU belongs to
    /// its opener's (already-begun) unit, and an `Ambiguous` OBU belongs to *some* layer-m unit
    /// either way — so each evidences that its embedded layer's frame unit has begun and
    /// participates in the ordering accounting before the unit-count branch.
    fn observe_frame(
        celu: &mut CeluState,
        embedded: EmbeddedLayerId,
        facts: FrameFacts,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        match facts.boundary {
            // A decided continuation of the open coded frame: a later OBU of an
            // already-counted coded frame unit. Transparent for the unit-count-dependent
            // judgments, but it still belongs to its (already-begun) embedded-layer frame unit,
            // so it participates in the ascending-mlayer ordering (F3): a continuation at a
            // lower mlayer after a higher embedded layer's unit began is out of order. Like the
            // Ambiguous arm below, the boundary-INDEPENDENT type-decided facts are still recorded
            // before returning (round-5 F2): CLK/OLK identity and leading-ness are decided by
            // `obu_type` alone, so a CELU containing a CLK and an OLK-typed continuation still
            // mixes CLK+OLK, and a LEADING-/Regular-typed continuation still evidences a leading /
            // regular frame unit — § 7.3.6 forbids both mixes regardless of unit structure. The
            // segmenter reports ContinuesUnit for a non-first tile group of an already-open coded
            // frame even when its `obu_type` differs (it also flags
            // frame-unit/mixed-coded-frame-types), so these type-decided CELU mixes would
            // otherwise be silently skipped. The unit-count-dependent accounting stays skipped (a
            // continuation opens no new unit).
            FrameBoundary::ContinuesUnit => {
                Self::note_embedded_layer_ordering(celu, embedded, obu, report);
                Self::record_clk_olk_identity(celu, facts.obu_type, obu, report);
                Self::record_leadingness(celu, facts.leadingness, obu, report);
                return;
            }
            // The segmenter could not decide whether this OBU opened a new coded frame unit
            // or continued the open one (a same-type no-delimiter TIP, or an unreadable
            // tile-group delimiter). Its existence as a new unit is undecided. The
            // POISON-SCOPE RULE (round-8): the ambiguity changes only the unit COUNT/INDEX, so
            // only INDEX-QUANTIFIED judgments drop here — the OUTPUT-PRESENCE judgments (which
            // ask "is there NO output unit", i.e. whether this could be the missing/only output
            // unit) — while DECIDED-PAIR-ORDER judgments survive (they ask "does a decided unit X
            // precede a decided unit Y", which the ambiguous OBU cannot change: an `OpensNewUnit`
            // is the segmenter's decided split). So the output-slot grammar (F1) and
            // key-not-in-first-unit (F2) are NOT poisoned — they are decided-pair-order judgments
            // evaluated on the surrounding decided units and no longer consult any per-layer
            // poison. Boundary-INDEPENDENT type-decided facts are likewise recorded before
            // returning (F5): the CLK/OLK identity and the leading-ness are decided by `obu_type`
            // alone, so a CELU containing an OLK and an ambiguous CLK still mixes CLK+OLK, and a
            // LEADING-/Regular-typed ambiguous OBU still evidences a leading / regular frame unit,
            // whichever unit each belongs to. The ascending-mlayer ordering is likewise
            // boundary-independent (F3): an ambiguous OBU belongs to some layer-m frame unit
            // either way, so a lower mlayer after a higher one began is out of order. The
            // remaining poisoned facts are the per-unit accounting and accumulators (this OBU
            // opens no counted unit) and the output-presence judgments below.
            FrameBoundary::Ambiguous => {
                Self::note_embedded_layer_ordering(celu, embedded, obu, report);
                Self::record_clk_olk_identity(celu, facts.obu_type, obu, report);
                Self::record_leadingness(celu, facts.leadingness, obu, report);
                // An ambiguous OBU might open a new coded frame unit whose class could be OUTPUT,
                // so it poisons the output-presence judgments — UNLESS its output class is
                // type-decided non-output (`Some(false)`, e.g. a BRIDGE), which can never satisfy
                // output presence whichever way the boundary resolves. Poison the CELU-scoped
                // missing-output rule and the per-layer non-output-implies-output rule only when
                // the ambiguous OBU could itself be an output unit.
                if facts.output != Some(false) {
                    let layer = celu.embedded.entry(embedded).or_default();
                    layer.output_presence_poisoned = true;
                    celu.missing_output_poisoned = true;
                }
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
        // the output slot is the violation). The slot is consumed only by a decided *output*
        // unit below (an Unknown-class earlier unit does not consume it, so a later unit does
        // not fire).
        //
        // This judgment does NOT check the per-layer ambiguous poison (round-8 F1): the poison
        // changes only the unit COUNT/INDEX, never the relative order of two DECIDED units. Here
        // both the slot-consuming output unit and this later unit are DECIDED (an `OpensNewUnit`
        // is the segmenter's decided split), so this unit is decidedly separate from and after
        // the decided output unit whatever the intervening ambiguous OBU was — no resolution of
        // the ambiguity can merge them into one unit. The decided-pair-order judgment therefore
        // survives the poison (an index-quantified judgment would drop; this one needs no index).
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

        // --- ascending-`obu_mlayer_id` frame-unit ordering (mirror line 525) ---
        Self::note_embedded_layer_ordering(celu, embedded, obu, report);

        // --- CLK / OLK identity (type-decided) ---
        let is_clk = facts.obu_type == ObuType::ClosedLoopKey;
        let is_olk = facts.obu_type == ObuType::OpenLoopKey;
        Self::record_clk_olk_identity(celu, facts.obu_type, obu, report);

        // --- per-embedded-layer unit accounting ---
        let layer = celu.embedded.entry(embedded).or_default();
        let unit_index = layer.units_opened;
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
        // embedded layer of the CELU. Type-decided, fires eagerly. The guard is "a DECIDED
        // earlier unit exists in this layer" (`!is_first_unit_of_layer`, i.e. `units_opened >=
        // 1`, where `units_opened` counts ONLY decided `OpensNewUnit` units) — NOT the blanket
        // ambiguous poison (round-8 F2). When a decided earlier unit exists, this CLK/OLK opens a
        // decidedly-later unit, so it is provably not the layer's first coded frame unit WHATEVER
        // the intervening ambiguity was: a decided-pair-order judgment that needs no exact index,
        // so it survives the poison. The poison-dropped case is instead `is_first_unit_of_layer`
        // (the KEY ITSELF is the first DECIDED unit while an ambiguous OBU preceded it): the
        // ambiguous OBU might or might not have been an earlier unit, so "not first" rests on a
        // guess — that case is already silent here (the `!is_first_unit_of_layer` guard). The
        // lowest-layer-not-key rule (resolve_celu) keeps the same asymmetry by construction: it
        // is about the FIRST unit's identity, so an ambiguous OBU before the candidate first unit
        // leaves the identity of "first" unknown and it stays dropped (no decided-pair-order
        // reformulation exists for "first unit").
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

        // --- leading / non-leading (all-leading-or-none, mirror lines 555-556) ---
        Self::record_leadingness(celu, facts.leadingness, obu, report);

        // --- same-OrderHint across the CELU's output units (mirror lines 539-540) --- the
        // mismatch is detected here but emitted at resolve_celu, so a later undecidable
        // output unit can still drop the whole judgment (the Unknown invariant).
        if facts.output == Some(true) {
            match facts.order_hint {
                Some(order_hint) => match celu.output_order_hint {
                    // The CELU's output OrderHint and the OrderHintBits of its first output
                    // unit are captured together (round-6 F2): all output units of one CELU
                    // share one active header, so the first output unit's bits represent the
                    // CELU for the cross-CELU §7.3.7 comparison gate.
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
        // F1: an ambiguous OBU that could itself be a coded output frame unit (its output class
        // is not type-decided non-output) poisons the CELU-scoped presence judgment — the
        // ambiguous OBU might be the missing output unit. A header-only CELU has no
        // frame-bearing OBU at all, so it can carry no ambiguous boundary and still fires.
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

        // --- non-output-implies-output per embedded layer (mirror lines 537-538) --- if a
        // layer has a coded non-output frame unit, it must also have a coded output one.
        // Dropped for a layer whose output class was ever Unknown, or (F1) poisoned by an
        // ambiguous OBU that could itself be the layer's coded output frame unit
        // (`output_presence_poisoned`): the validator cannot then confirm the layer lacks one.
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
        // mismatch was detected eagerly between two KNOWN output units; emit it now regardless
        // of whether some other output unit's order_hint was undecidable (round-7 F3). An
        // undecidable member can only prevent proving AGREEMENT (which the validator never
        // reports); it cannot make a pair already proven to differ conforming, so it no longer
        // gates emission. The comparison is over the `order_hint` LSB proxy
        // ([`FrameFacts::order_hint`]); within one CELU all output units share one active
        // header (one OrderHintBits), so it is a sound under-approximation with no cross-width
        // gate needed (unlike the cross-CELU §7.3.7 check, which gates on equal known bits).
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

        // --- contribute this CELU's output OrderHint to the cross-CELU DOH accumulator ---
        // The CELU's "associated order hint" is the OrderHint shared by its coded output
        // frame units (mirror lines 571-576). It is well-defined for the cross-CELU comparison
        // only when the CELU had at least one output unit with a decidable order_hint AND no
        // output unit's order_hint was undecidable: with an undecidable output unit the CELU's
        // single associated hint cannot be confirmed, so feeding the known units' value to the
        // cross-CELU comparison would be a GUESS that could false-positive. Such a CELU is
        // therefore simply NOT contributed (round-7 F5): cross-CELU sees only fully-decidable
        // CELUs, so any recorded cross-CELU mismatch is proven between two such CELUs and is
        // emitted regardless of other, undecidable CELUs (see [`DohTuAccumulator::resolve`]).
        if !celu.order_hint_undecidable
            && let Some((order_hint, order_hint_bits, offset)) = celu.output_order_hint
        {
            doh.note_celu_output_order_hint(order_hint, order_hint_bits, offset);
        }
    }

    /// Threads one frame's `OrderHintBits` into the temporal-unit DOH accumulator (mirror
    /// line 655: all frame units in the temporal unit share one `OrderHintBits`). Called by
    /// the validator for every frame-bearing OBU, since the bits come from the active
    /// sequence header rather than the per-frame parse and span CELUs. A `None` (undecidable)
    /// frame is simply not recorded: it cannot establish a mismatch (those are proven only
    /// between two KNOWN values) and, round-7 F4, no longer suppresses a mismatch already
    /// proven between two known frames.
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
    /// Notes one CELU's resolved output OrderHint (and the OrderHintBits of its output units)
    /// for the cross-CELU §7.3.7 agreement check (constraint 2, mirror lines 656-657). Records
    /// the first disagreement; emission is deferred to [`Self::resolve`] so the check stays
    /// flag-gated.
    ///
    /// The `order_hint` LSB comparison is a proxy for the decoded OrderHint and is sound only
    /// when the two COMPARED output units share one KNOWN OrderHintBits (round-6 F2). So output
    /// CELUs are GROUPED BY their known OrderHintBits value (round-8 F3) and each is compared to
    /// its OWN group's representative (the group's first decidable sample), not only to the very
    /// first sample of any width: across different (or unknown) bit widths equal decoded
    /// OrderHints can carry different-width LSB encodings, so a cross-width comparison would
    /// false-positive. A known-but-different-bits pair is instead covered by
    /// `celu/doh-order-hint-bits-mismatch` (constraint 1); an UNKNOWN-bits output CELU stays out
    /// of all groups and records nothing (the Unknown invariant for that sample — nothing is
    /// proven). Crucially, the grouping is over each compared output unit's own bits — not the
    /// temporal-unit-wide same-bits judgment — so an unrelated non-output / unknown-bits frame
    /// unit elsewhere in the temporal unit no longer suppresses a decidable within-group mismatch
    /// between two output CELUs. A CELU whose own output OrderHint is undecidable is never
    /// contributed here (see [`CodedExtendedLayerTracker::resolve_celu`]), so a recorded mismatch
    /// is always proven between two fully-decidable CELUs (round-7 F5). At most one mismatch is
    /// recorded per temporal unit (the first within-group disagreement found), anchored at the
    /// offending later sample.
    fn note_celu_output_order_hint(
        &mut self,
        order_hint: u32,
        order_hint_bits: Option<u32>,
        offset: ByteOffset,
    ) {
        // An unknown-bits output CELU is not in any group: the LSB proxy is unsound against a
        // sample of an unknown (possibly different) width, so it is neither compared nor recorded.
        let Some(bits) = order_hint_bits else {
            return;
        };
        match self.output_order_hint_by_bits.get(&bits) {
            // First sample of this OrderHintBits group becomes its representative.
            None => {
                self.output_order_hint_by_bits
                    .insert(bits, (order_hint, offset));
            }
            // A later same-bits output CELU: compare within the group (equal known bits -> sound
            // proxy). Record the first within-group disagreement across all groups (dedup: one
            // emission per temporal unit), anchored at this offending later sample.
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

    /// Resolves the per-temporal-unit § 7.3.7 / § 7.4.6 DOH OrderHint / OrderHintBits
    /// checks. The caller gates this on the active DOH constraint flag, so a mismatch under
    /// a flag-off temporal unit is never reported (the DOH constraints are flag-gated,
    /// mirror lines 650-657). Each recorded mismatch is a disagreement PROVEN between two
    /// known samples, so it is emitted regardless of any undecidable frame / CELU (round-7
    /// F3/F4/F5): an undecidable participant only prevents proving agreement, never excuses a
    /// proven mismatch.
    fn resolve(&mut self, report: &mut ValidationReport) {
        // § 7.3.7: all frame units in the temporal unit shall use the same OrderHintBits. The
        // mismatch was recorded between two KNOWN OrderHintBits; emit it regardless of whether
        // another frame unit's bits were undecidable (round-7 F4). An undecidable participant
        // only prevents proving agreement, never excuses a pair already proven to differ.
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
        // § 7.3.7 / § 7.4.6: coded output frame units present in multiple CELUs within the
        // temporal unit shall share the same OrderHint (constraint 2).
        //
        // The validator compares the raw `order_hint` LSB syntax as a proxy for the DECODED
        // OrderHint (the §7.3.6/§7.3.7 quantity is `get_disp_order_hint`'s output, the MSB
        // extension for non-CLK frames, which needs reference-state modelling — see
        // AV2-5.18.2-FRAME-HEADER-INFO). The cross-CELU proxy is only sound when the two
        // COMPARED output units share ONE KNOWN OrderHintBits: differing decoded OrderHints
        // with equal-width LSBs still differ in the low bits, but EQUAL decoded OrderHints can
        // be encoded with different-width LSBs when the bit widths differ, so a cross-width
        // comparison can FALSE-POSITIVE.
        //
        // Round-6 F2: the soundness gate is applied PER COMPARED PAIR in
        // [`Self::note_celu_output_order_hint`] (on the two output CELUs' own bits), NOT via
        // the temporal-unit-wide `bits_mismatch` judgment — which also
        // covers non-output and unrelated frame units (constraint 1). An unknown-bits non-
        // output frame unit elsewhere in the temporal unit drops constraint (1) but must not
        // suppress a decidable constraint (2) mismatch between two output CELUs whose own bits
        // are known and equal. So `order_hint_mismatch` is recorded ONLY when its pair's bits
        // were known and equal — a mismatch PROVEN between two known CELUs.
        //
        // Round-7 F5: that proven mismatch is emitted regardless of the cross-CELU
        // `undecidable` signal. An undecidable CELU output OrderHint can only prevent proving
        // AGREEMENT among the CELUs (which the validator never reports); it cannot make a pair
        // already proven to differ conforming. The per-pair bits gate above still keeps an
        // unknown-or-unequal-bits PAIR from recording a mismatch in the first place.
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
