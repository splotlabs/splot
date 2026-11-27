// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! State-aware non-intra frame-header control parsing (AV2 v1.0.0 § 5.18.2,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`).
//!
//! Parses reference selection, frame size, BRU/TIP, motion controls, and CDF flags.
//! [`InterStop`] identifies the selected terminal/shared tail or missing state.
//! [`parse_inter_control_into`] updates caller-owned state so modeled fields survive EOF.
//! A missing reference fact stops parsing before any read whose presence or width
//! depends on that fact.

#[cfg(test)]
use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::frame::config::{
    parse_intrabc_params_with_reader, parse_screen_content_params_with_reader,
};
use crate::headers::frame::filtering::{
    InterpolationFilter, read_interpolation_filter_with_reader,
};
use crate::headers::frame::get_ref_frames::{
    GetRefFrames, GetRefFramesInput, RESTRICTED_OH, RESTRICTED_ORDER_HINT, RefIdxBuf, RefSlot,
    get_ref_frames, get_relative_dist,
};
use crate::headers::frame::size::{FrameSize, ceil_log2};
use crate::headers::sequence::{MLayerDependencyMap, SuperblockSize, TLayerDependencyMap};
use crate::syntax::{SyntaxObserver, SyntaxReader};
use crate::types::{EmbeddedLayerId, ObuType, TemporalLayerId};

use super::FrameReferenceStateView;
use super::info::FrameType;

/// `NUM_REF_FRAMES` (AV2 v1.0.0 § 3): the number of reference-frame buffer slots.
pub(crate) const NUM_REF_FRAMES: usize = 16;

/// `MAX_REF_MV_STACK_SIZE` (AV2 v1.0.0 § 3): bounds the `max_drl_bits_minus_1` `ns(n)`
/// range (`n = MAX_REF_MV_STACK_SIZE - 2`, § 5.18.2 mirror :4871).
const MAX_REF_MV_STACK_SIZE: u32 = 6;

/// `MOTION_MODES` (AV2 v1.0.0 § 3): the number of motion modes, bounding the
/// `frame_enabled_motion_modes[mode]` loop (§ 5.18.2 mirror :4921).
pub const MOTION_MODES: usize = 5;

/// `INTERINTRA` (AV2 v1.0.0 § 3): the first motion-mode index read in the
/// `frame_enabled_motion_modes` loop (§ 5.18.2 mirror :4921).
pub const INTERINTRA: usize = 1;

/// `LOCALWARP` (AV2 v1.0.0 § 3).
pub const LOCALWARP: usize = 2;

/// `DELTAWARP` (AV2 v1.0.0 § 3).
pub const DELTAWARP: usize = 3;

/// `EXTENDWARP` (AV2 v1.0.0 § 3).
pub const EXTENDWARP: usize = 4;

/// `PRIMARY_REF_NONE` (AV2 v1.0.0 § 3).
pub(super) const PRIMARY_REF_NONE: u8 = 7;

/// `PRIMARY_REF_CHOOSE` (AV2 v1.0.0 § 3): the value `primary_ref_frame` takes when
/// `signal_primary_ref_frame == 0` (§ 5.18.2 mirror :4397).
const PRIMARY_REF_CHOOSE: u8 = 8;

/// `REFINE_NONE` (AV2 v1.0.0 § 3).
const REFINE_NONE: u32 = 0;

/// `REFINE_SWITCHABLE` (AV2 v1.0.0 Table 6.5, § 6.17.2 mirror :947): the `opfl_refine_type`
/// value at which `frame_opfl_refine_type()` does not read `opfl_refine_all` (§ 5.18.3.2
/// mirror :5601, `if ( opfl_refine_type != REFINE_SWITCHABLE )`). `opfl_refine_type` is read
/// as `f(1)`, so its value space is {0, 1}: a value of 2 here would make the inequality
/// always true and consume an extra `opfl_refine_all` bit, mis-positioning every following
/// field.
const REFINE_SWITCHABLE: u32 = 1;

/// `REFINE_ALL` (AV2 v1.0.0 § 3).
const REFINE_ALL: u32 = 2;

/// `REFINE_AUTO` (AV2 v1.0.0 § 3): the `enable_opfl_refine` value that makes
/// `frame_opfl_refine_type()` signal `opfl_refine_type` (§ 5.18.3.2 mirror :5597).
const REFINE_AUTO: u8 = 3;

/// `MV_PRECISION_*` (AV2 v1.0.0 § 3): the frame MV precision selected by the
/// `use_qtr_precision_mv` / `allow_high_precision_mv` reads (§ 5.18.2 mirror :4885-4917).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MvPrecision {
    /// `MV_PRECISION_ONE_PEL` (the inferred precision when `force_integer_mv`).
    OnePel,
    /// `MV_PRECISION_HALF_PEL`.
    HalfPel,
    /// `MV_PRECISION_QUARTER_PEL` (`use_qtr_precision_mv == 1`).
    QuarterPel,
    /// `MV_PRECISION_EIGHTH_PEL` (`allow_high_precision_mv == 1`).
    EighthPel,
}

impl MvPrecision {
    /// Returns the AV2 v1.0.0 § 5.18.2 / § 6.17.2 motion-vector precision code from Table 6.19
    /// (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2`).
    #[must_use]
    pub const fn av2_code(self) -> u8 {
        match self {
            Self::OnePel => 3,
            Self::HalfPel => 4,
            Self::QuarterPel => 5,
            Self::EighthPel => 6,
        }
    }

    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OnePel => "one_pel",
            Self::HalfPel => "half_pel",
            Self::QuarterPel => "quarter_pel",
            Self::EighthPel => "eighth_pel",
        }
    }
}

/// `TIP_FRAME_*` (AV2 v1.0.0 § 3): the temporal-interpolated-prediction frame mode
/// (§ 5.18.2 mirror :4743-4757).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TipFrameMode {
    /// `TIP_FRAME_DISABLED`.
    Disabled,
    /// `TIP_FRAME_AS_REF` (`tip_frame_mode == 1` on non-TIP OBUs).
    AsRef,
    /// `TIP_FRAME_AS_OUTPUT` (`EnableTipOutput && is_tip_frame()`).
    AsOutput,
    /// The signalled `tip_frame_mode` value that is neither disabled nor as-output.
    Other(u8),
}

impl TipFrameMode {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::AsRef => "as_ref",
            Self::AsOutput => "as_output",
            Self::Other(_) => "other",
        }
    }
}

/// `TipGlobalMv` from the TIP-as-output frame-header arm (§ 5.18.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TipGlobalMv {
    /// Signed vertical component derived from `tip_mv_row` and `tip_mv_row_sign`.
    pub row: i32,
    /// Signed horizontal component derived from `tip_mv_col` and `tip_mv_col_sign`.
    pub col: i32,
}

impl TipGlobalMv {
    /// The global MV of `row` and `col`.
    #[must_use]
    pub const fn new(row: i32, col: i32) -> Self {
        Self { row, col }
    }
}

/// Why the inter control-region parse stopped (a **coverage** stop, never a truncation).
///
/// The variants split into the two classes the module documents: a derivation
/// that needs unmodeled syntax, and a derivation that needs reference-state facts the
/// model has not proven for the consumed slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterStop {
    /// A § 5.18.2 derivation this phase does not model is required to continue
    /// (`get_ref_frames()` for the implicit reference map). The facts parsed before it are
    /// preserved. The TIP block's `get_past_future_cur_ref_lists()` past/future ref counts
    /// are a reference-state derivation and stop with [`Self::PoisonedReferenceState`].
    UnmodeledDerivation,
    /// A § 5.18.2 derivation that affects a bit position needs a reference slot the
    /// model has not proven valid (`frame_size_with_refs()` / `frame_size_with_bridge()`
    /// dims, or the TIP block's `usesEqualWeight` past/future ref counts). The facts
    /// parsed before it are preserved.
    PoisonedReferenceState,
    /// The inter control region parsed through every modeled field up to (and not
    /// including) the shared tail (`tile_info()` onward, mirror :5183), and `bru_inactive`
    /// / `IsBridge` were not set (so the early-return arm at mirror :4971/:5045 is not
    /// taken). The caller continues into the shared tail.
    ReachedSharedTail,
    /// BRU inactivity or a bridge selects the early-return tail (§ 5.18.2).
    /// The caller completes that tail when its reference-derived state is available.
    BruInactiveOrBridgeReturn,
    /// `TipFrameMode == TIP_FRAME_AS_OUTPUT`, so the control region selects the TIP-output
    /// terminal tail (mirror :4945-5031). The caller continues with its quantization,
    /// deblocking, conditional tile-info, and film-grain reads before the return at :5177.
    TipAsOutputReturn,
}

impl InterStop {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::UnmodeledDerivation => "unmodeled_derivation",
            Self::PoisonedReferenceState => "poisoned_reference_state",
            Self::ReachedSharedTail => "reached_shared_tail",
            Self::BruInactiveOrBridgeReturn => "bru_inactive_or_bridge_return",
            Self::TipAsOutputReturn => "tip_as_output_return",
        }
    }
}

/// The parsed non-intra control region (AV2 v1.0.0 § 5.18.2). Every field is `Option`,
/// present only when the corresponding syntax was reached and exactly determined.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InterControl {
    /// Where the inter control-region parse stopped.
    pub stop: Option<InterStop>,
    /// `signal_primary_ref_frame` (mirror :4383), when read.
    pub signal_primary_ref_frame: Option<bool>,
    /// `disable_cross_frame_cdf_init` (mirror :4387), when read.
    pub disable_cross_frame_cdf_init: Option<bool>,
    /// `disable_cdf_update` (mirror :5041), read on the ordinary inter / switch path —
    /// the `else` arm of `if ( TipFrameMode == TIP_FRAME_AS_OUTPUT || bru_inactive ||
    /// IsBridge )` — immediately before the shared-tail `tile_info()` (mirror :5183).
    /// `None` on the early-return arms (TIP-as-output / bru-inactive / bridge), which take
    /// the `if` branch and never read this bit.
    pub disable_cdf_update: Option<bool>,
    /// `primary_ref_frame` (mirror :4393), when read or inferred.
    pub primary_ref_frame: Option<u8>,
    /// `bridge_frame_overwrite_flag` (mirror :4425), when read (bridge frames).
    pub bridge_frame_overwrite_flag: Option<bool>,
    /// `refresh_frame_flags` (mirror :4429-4537), when read or derived.
    pub refresh_frame_flags: Option<u32>,
    /// `explicitRefFrameMap` (mirror :4581-4593), when derived.
    pub explicit_ref_frame_map: Option<bool>,
    /// `NumTotalRefs` (mirror :4597-4609), when read or derived.
    pub num_total_refs: Option<u32>,
    /// `ref_frame_idx[0..NumTotalRefs]` (mirror :4611-4625), when read or derived.
    pub ref_frame_idx: RefIdxBuf,
    /// `OrderHints[0..NumTotalRefs] = RefOrderHint[ref_frame_idx[i]]` (mirror :4711).
    pub order_hints: RefIdxBuf,
    /// `FrameWidth`/`FrameHeight` from the reference-grounded frame size (mirror
    /// :4627-4643), when exactly known.
    pub frame_size: Option<FrameSize>,
    /// `use_bru` (mirror :4657), when read.
    pub use_bru: Option<bool>,
    /// `bru_ref` (mirror :4663), when read.
    pub bru_ref: Option<u32>,
    /// `bru_inactive` (mirror :4665), when read or inferred.
    pub bru_inactive: Option<bool>,
    /// `use_ref_frame_mvs` (mirror :4685-4695), when read or inferred.
    pub use_ref_frame_mvs: Option<bool>,
    /// `tmvp_sample_step_minus_1` (mirror :4699), when read.
    pub tmvp_sample_step_minus_1: Option<bool>,
    /// `TipFrameMode` (mirror :4743-4845), when derived.
    pub tip_frame_mode: Option<TipFrameMode>,
    /// `allow_tip_hole_fill` (mirror :4773), when read or inferred.
    pub allow_tip_hole_fill: Option<bool>,
    /// `tip_global_wtd_index` (mirror :4791), when read or inferred.
    pub tip_global_wtd_index: Option<u8>,
    /// `TipGlobalMv[0..1]` (mirror :4795-4825), when exactly derived.
    pub tip_global_mv: Option<TipGlobalMv>,
    /// `TipInterpFilter` (mirror :4829-4839), when exactly derived.
    pub tip_interpolation_filter: Option<InterpolationFilter>,
    /// `allow_df_sub_pu` from the TIP-as-output tail (mirror :4953), read when
    /// `enable_df_sub_pu` or inferred `false` otherwise.
    pub allow_df_sub_pu: Option<bool>,
    /// `apply_deblocking_filter_tip` from the TIP-as-output tail (mirror :4957), read
    /// when `allow_df_sub_pu` or inferred `false` otherwise.
    pub apply_deblocking_filter_tip: Option<bool>,
    /// The terminal TIP-as-output `film_grain_config()` (§ 5.18.10.1), when parsed.
    pub tip_film_grain: Option<super::tail::FilmGrainConfig>,
    /// `opfl_refine_type` from `frame_opfl_refine_type()` (§ 5.18.3.2), when derived.
    pub opfl_refine_type: Option<u32>,
    /// `max_drl_bits_minus_1` (mirror :4863-4881), when read or inferred.
    pub max_drl_bits_minus_1: Option<u32>,
    /// `FrameMvPrecision` (mirror :4885-4917), when derived.
    pub mv_precision: Option<MvPrecision>,
    /// `force_integer_mv` from `screen_content_params()` (§ 5.18.3.3), when read on the inter path.
    pub force_integer_mv: Option<bool>,
    /// `interpolation_filter` from `read_interpolation_filter()` (§ 5.18.5.1), when read.
    pub interpolation_filter: Option<InterpolationFilter>,
    /// `frame_enabled_motion_modes[INTERINTRA..MOTION_MODES]` (mirror :4921-4939), when read.
    pub frame_enabled_motion_modes: Option<[bool; MOTION_MODES]>,
    /// `allow_screen_content_tools` (mirror :4859), when read on the inter path.
    pub allow_screen_content_tools: Option<bool>,
    /// `allow_intrabc` (mirror :4861), when read on the inter path.
    pub allow_intrabc: Option<bool>,
    /// The coded `max_bvp_drl_bits_minus_1` `ns(2)` of § 5.18.3.4, when
    /// `change_bvp_drl` was set on the inter path.
    pub max_bvp_drl_bits_minus_1: Option<u32>,
    /// `true` when a parsed `ref_frame_idx[i]` names an index at or beyond the **active**
    /// reference-slot count (`idx >= NumRefFrames`). AV2 § 6.17.2 (mirror :4605-4606)
    /// requires `RefValid[ ref_frame_idx[i] ] == 1`, and RefValid is defined only over the
    /// active slots `0..NumRefFrames-1`; an index >= NumRefFrames names a slot outside that
    /// buffer, so it is an unconditional violation decidable from the parsed value alone (it
    /// arises because the read width `CeilLog2(NumRefFrames)` over-covers a non-power-of-two
    /// NumRefFrames — e.g. 6 -> 3 bits -> values up to 7). This active bound subsumes the
    /// `NUM_REF_FRAMES` (16) buffer bound, since NumRefFrames <= NUM_REF_FRAMES always.
    /// Recorded for the validator. The in-range proven-invalid case (`RefValid[idx] == false`
    /// for `idx < NumRefFrames`) is decided separately by the validator's §7.23
    /// reference-state check (`ValidatorContext::reference_state_checks`), not by this flag —
    /// the parser cannot distinguish an Unknown slot from a proven-invalid one here.
    pub has_invalid_ref_frame_idx: bool,
}

/// Sequence-derived scalars the inter control region consumes (AV2 v1.0.0 § 5.4.x),
/// gathered alongside the intra `CoreSeqView` by the caller. Named per the spec fields.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterSeqView {
    /// `NumRefFrames` (§ 5.4.6): the active reference-slot count.
    pub num_ref_frames: u32,
    /// `enable_short_refresh_frame_flags` (§ 5.4.6).
    pub enable_short_refresh_frame_flags: bool,
    /// `explicit_ref_frame_map` (§ 5.4.6).
    pub explicit_ref_frame_map: bool,
    /// `enable_ref_frame_mvs` (§ 5.4.6).
    pub enable_ref_frame_mvs: bool,
    /// `enable_bru` (§ 5.4.6).
    pub enable_bru: bool,
    /// `enable_tip` (§ 5.4.6).
    pub enable_tip: bool,
    /// `enable_tip_output` (§ 5.4.6): `EnableTipOutput` for the § 5.18.2
    /// `tip_frame_mode` derived-versus-coded arm.
    pub enable_tip_output: bool,
    /// `enable_tip_hole_fill` (§ 5.4.6).
    pub enable_tip_hole_fill: bool,
    /// `enable_refinemv` (§ 5.4.6).
    pub enable_refinemv: bool,
    /// `enable_tip_refinemv` (§ 5.4.6).
    pub enable_tip_refinemv: bool,
    /// `seq_max_drl_bits_minus_1` (§ 5.4.6).
    pub seq_max_drl_bits_minus_1: u32,
    /// `allow_frame_max_drl_bits` (§ 5.4.6).
    pub allow_frame_max_drl_bits: bool,
    /// `seq_frame_motion_modes_present_flag` (§ 5.4.6).
    pub seq_frame_motion_modes_present_flag: bool,
    /// `seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]` (§ 5.4.6).
    pub seq_enabled_motion_modes: [bool; MOTION_MODES],
    /// `enable_opfl_refine` (§ 5.4.6): selects the `frame_opfl_refine_type()` arm
    /// (§ 5.18.3.2). `REFINE_AUTO` (`3`) reads bits; otherwise it reads none.
    pub enable_opfl_refine: u8,
    /// `max_mlayer_id` (§ 5.4.1): selects the RAS `refresh_frame_flags` derivation arm.
    pub max_mlayer_id: u8,
    /// `MLayerDependencyMap` (§ 5.4.1).
    pub mlayer_dependency_map: MLayerDependencyMap,
    /// `TLayerDependencyMap` (§ 5.4.1).
    pub tlayer_dependency_map: TLayerDependencyMap,
    /// `seq_force_screen_content_tools` (§ 5.4.7).
    pub seq_force_screen_content_tools: u8,
    /// `seq_force_integer_mv` (§ 5.4.7).
    pub seq_force_integer_mv: u8,
    /// `allow_frame_max_bvp_drl_bits` (§ 5.4.6): gates `intrabc_params()`'s DRL change.
    pub allow_frame_max_bvp_drl_bits: bool,
    /// `frame_width_bits` (`frame_width_bits_minus_1 + 1`, § 5.4.1).
    pub frame_width_bits: u32,
    /// `frame_height_bits` (`frame_height_bits_minus_1 + 1`, § 5.4.1).
    pub frame_height_bits: u32,
    /// The frame `SbSize` for the inter path (`frame_sb_size(false)`, mirror :4317-4329).
    pub sb_size: SuperblockSize,
}

/// Per-frame inputs the inter control region needs that the caller derives from the
/// already-parsed prefix / output-control region (AV2 v1.0.0 § 5.18.2).
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterFrameContext {
    /// The OBU type (selects TIP / switch / RAS arms via `is_tip_frame()` etc.).
    pub obu_type: ObuType,
    /// `obu_mlayer_id` of the current frame OBU.
    pub obu_mlayer_id: EmbeddedLayerId,
    /// `obu_tlayer_id` of the current frame OBU.
    pub obu_tlayer_id: TemporalLayerId,
    /// The derived `FrameType`.
    pub frame_type: FrameType,
    /// `IsBridge`.
    pub is_bridge: bool,
    /// `bridge_frame_ref_idx`, when this is a bridge frame.
    pub bridge_frame_ref_idx: Option<u32>,
    /// The § 5.18.4.1 non-override `frame_size()` dims, `mfh_frame_width/height_minus_1[
    /// cur_mfh_id ] + 1`; `None` when a `cur_mfh_id > 0` record is unresolved.
    pub default_frame_size: Option<FrameSize>,
    /// `OrderHint` (§ 5.18.2), including any extension beyond the coded `OrderHintLsbs`,
    /// threaded into the implicit reference-map ranking (`get_ref_frames()` § 7.7).
    pub order_hint: Option<u32>,
    /// Bitmask of slots holding long-term references listed by the current RAS frame.
    /// `None` means the required § 7.23 long-term-id state was unavailable.
    pub ras_long_term_ref_mask: Option<u32>,
}

/// `RefValid[idx]` from the modeled reference state: `true` only when the slot is in
/// range and the model proved it valid; `Unknown` / out-of-range / no-buffer all yield
/// `false` (the parser must not treat an unproven slot as available).
fn ref_valid(reference_state: &FrameReferenceStateView<'_>, idx: u32) -> bool {
    reference_state
        .ref_valid
        .and_then(|slots| usize::try_from(idx).ok().and_then(|i| slots.get(i)))
        .copied()
        .unwrap_or(false)
}

/// `RefFrameWidth[idx]` / `RefFrameHeight[idx]` from the modeled reference state, `Some`
/// only when the slot is proven `RefValid` and its dims are modeled.
fn ref_dims(reference_state: &FrameReferenceStateView<'_>, idx: u32) -> Option<(u32, u32)> {
    if !ref_valid(reference_state, idx) {
        return None;
    }
    let i = usize::try_from(idx).ok()?;
    let w = reference_state.ref_frame_width?.get(i).copied()?;
    let h = reference_state.ref_frame_height?.get(i).copied()?;
    Some((w, h))
}

fn ref_order_hint_to_spec(raw: u32) -> Option<i32> {
    if raw == RESTRICTED_ORDER_HINT {
        Some(RESTRICTED_OH)
    } else {
        i32::try_from(raw).ok()
    }
}

fn tip_ref_counts(
    reference_state: &FrameReferenceStateView<'_>,
    ref_frame_idx: &[u32],
    current_order_hint: u32,
) -> Option<(usize, usize)> {
    let hints = reference_state.ref_order_hint?;
    let current = ref_order_hint_to_spec(current_order_hint)?;
    let mut num_past_refs = 0usize;
    let mut num_future_refs = 0usize;
    for &idx in ref_frame_idx {
        if !ref_valid(reference_state, idx) {
            return None;
        }
        let hint = ref_order_hint_to_spec(*hints.get(usize::try_from(idx).ok()?)?)?;
        if hint == RESTRICTED_OH {
            continue;
        }
        let dist = get_relative_dist(current, hint);
        if dist > 0 {
            num_past_refs += 1;
        } else if dist < 0 {
            num_future_refs += 1;
        }
    }
    Some((num_past_refs, num_future_refs))
}

/// Whether a TIP frame over `ref_frame_idx` at `order_hint` codes
/// `tip_global_wtd_index` (§ 5.18.2): a `TIP_FRAME_AS_OUTPUT` frame with
/// `as_output`, else a `TIP_FRAME_AS_REF` frame whose coded
/// `opfl_refine_type` is `frame_opfl`; `None` when `refs` cannot count its
/// past and future references.
pub(crate) fn tip_weight_coded(
    seq: &InterSeqView,
    refs: &FrameReferenceStateView<'_>,
    (ref_frame_idx, order_hint): (&[u32], u32),
    as_output: bool,
    frame_opfl: u32,
) -> Option<bool> {
    let (past, future) = tip_ref_counts(refs, ref_frame_idx, order_hint)?;
    let opfl_refine_type = if as_output {
        tip_output_opfl_refine_type(seq)
    } else if seq.enable_opfl_refine == REFINE_AUTO {
        frame_opfl
    } else {
        u32::from(seq.enable_opfl_refine)
    };
    Some(tip_unequal_weighted_allowed(
        seq,
        past,
        future,
        opfl_refine_type,
    ))
}

fn tip_unequal_weighted_allowed(
    seq: &InterSeqView,
    num_past_refs: usize,
    num_future_refs: usize,
    opfl_refine_type: u32,
) -> bool {
    let has_both_sides_refs = num_past_refs > 0 && num_future_refs > 0;
    !has_both_sides_refs
        || !seq.enable_tip_refinemv
        || (opfl_refine_type == REFINE_NONE && !seq.enable_refinemv)
}

/// Parses the non-intra control region (AV2 § 5.18.2) into a caller-owned `control`, so the
/// fields parsed before any [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof)
/// survive on the caller's `control` rather than being dropped with a local value. This is
/// the facts-preserving entry the core parser uses to convert an EOF inside the modeled
/// inter / bridge control region into a truncation status: on `Err(UnexpectedEof)`
/// the caller keeps `control`'s partial facts and records the truncation; on `Ok` the region
/// reached one of its modeled coverage stops.
///
/// # Errors
/// Returns a typed error if the payload ends or is malformed before a modeled field can be
/// read; the partial facts up to that point remain on `control`.
#[cfg(test)]
pub(crate) fn parse_inter_control_into(
    reader: &mut BitReader<'_>,
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    reference_state: &FrameReferenceStateView<'_>,
    frame_size_override_flag: bool,
    control: &mut InterControl,
) -> Result<()> {
    SyntaxReader::with_noop(reader, |sr| {
        parse_inter_control_into_with_reader(
            sr,
            seq,
            ctx,
            reference_state,
            frame_size_override_flag,
            control,
        )
    })
}

pub(crate) fn parse_inter_control_into_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    reference_state: &FrameReferenceStateView<'_>,
    frame_size_override_flag: bool,
    control: &mut InterControl,
) -> Result<()> {
    if ctx.is_bridge || ctx.frame_type == FrameType::Switch {
        sr.inferred_u64(
            "primary_ref_frame",
            "IsBridge || FrameType == SWITCH_FRAME",
            u64::from(PRIMARY_REF_NONE),
        );
        sr.inferred_bool(
            "disable_cross_frame_cdf_init",
            "IsBridge || FrameType == SWITCH_FRAME",
            false,
        );
        control.primary_ref_frame = Some(PRIMARY_REF_NONE);
        control.disable_cross_frame_cdf_init = Some(false);
    } else {
        let signal = sr.read_flag("signal_primary_ref_frame")?;
        control.signal_primary_ref_frame = Some(signal);
        if !ctx.obu_type.is_tip_frame() {
            control.disable_cross_frame_cdf_init =
                Some(sr.read_flag("disable_cross_frame_cdf_init")?);
        } else {
            sr.inferred_bool("disable_cross_frame_cdf_init", "is_tip_frame", false);
            control.disable_cross_frame_cdf_init = Some(false);
        }
        if signal {
            control.primary_ref_frame = Some(sr.read_bits_u8("primary_ref_frame", 3)?);
        } else {
            sr.inferred_u64(
                "primary_ref_frame",
                "!signal_primary_ref_frame",
                u64::from(PRIMARY_REF_CHOOSE),
            );
            control.primary_ref_frame = Some(PRIMARY_REF_CHOOSE);
        }
    }

    if ctx.is_bridge {
        control.bridge_frame_overwrite_flag = Some(sr.read_flag("bridge_frame_overwrite_flag")?);
    }

    control.refresh_frame_flags = read_inter_refresh_frame_flags(sr, seq, ctx, control)?;

    if control.refresh_frame_flags.is_none() {
        control.stop = Some(InterStop::UnmodeledDerivation);
        return Ok(());
    }

    parse_inter_reference_region(
        sr,
        seq,
        ctx,
        reference_state,
        frame_size_override_flag,
        control,
    )?;

    Ok(())
}

fn read_inter_refresh_frame_flags<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    control: &InterControl,
) -> Result<Option<u32>> {
    if ctx.is_bridge {
        let overwrite = control.bridge_frame_overwrite_flag.unwrap_or(false);
        if !overwrite {
            let idx = ctx.bridge_frame_ref_idx.unwrap_or(0);
            let mask = 1u32.wrapping_shl(idx);
            sr.inferred_u64(
                "refresh_frame_flags",
                "IsBridge && !bridge_frame_overwrite_flag",
                u64::from(mask),
            );
            return Ok(Some(mask));
        }
        if seq.enable_short_refresh_frame_flags {
            return if sr.read_flag("has_refresh_frame_flags")? {
                let idx = sr.read_bits("frame_to_refresh", ceil_log2(seq.num_ref_frames))?;
                Ok(Some(1u32.wrapping_shl(idx)))
            } else {
                sr.inferred_u64("refresh_frame_flags", "!has_refresh_frame_flags", 0);
                Ok(Some(0))
            };
        }
        return Ok(Some(
            sr.read_bits("refresh_frame_flags", seq.num_ref_frames)?,
        ));
    }

    if ctx.obu_type == ObuType::RasFrame && seq.max_mlayer_id == 0 {
        let Some(long_term_refs) = ctx.ras_long_term_ref_mask else {
            return Ok(None);
        };
        let all_frames = 1u32
            .checked_shl(seq.num_ref_frames)
            .unwrap_or(0)
            .wrapping_sub(1);
        let mask = all_frames & !long_term_refs;
        sr.inferred_u64(
            "refresh_frame_flags",
            "obu_type == OBU_RAS_FRAME && max_mlayer_id == 0",
            u64::from(mask),
        );
        return Ok(Some(mask));
    }

    if ctx.frame_type == FrameType::Switch {
        return Ok(Some(
            sr.read_bits("refresh_frame_flags", seq.num_ref_frames)?,
        ));
    }

    if seq.enable_short_refresh_frame_flags {
        let has = sr.read_flag("has_refresh_frame_flags")?;
        if has {
            let frame_to_refresh =
                sr.read_bits("frame_to_refresh", ceil_log2(seq.num_ref_frames))?;
            return Ok(Some(1u32.wrapping_shl(frame_to_refresh)));
        }
        sr.inferred_u64("refresh_frame_flags", "!has_refresh_frame_flags", 0);
        return Ok(Some(0));
    }

    Ok(Some(
        sr.read_bits("refresh_frame_flags", seq.num_ref_frames)?,
    ))
}

fn parse_inter_reference_region<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    reference_state: &FrameReferenceStateView<'_>,
    frame_size_override_flag: bool,
    control: &mut InterControl,
) -> Result<()> {
    let is_tip = ctx.obu_type.is_tip_frame();

    let explicit_ref_frame_map = if ctx.frame_type == FrameType::Switch || ctx.is_bridge {
        sr.inferred_bool(
            "frame_explicit_ref_frame_map",
            "FrameType == SWITCH_FRAME || IsBridge",
            true,
        );
        true
    } else if seq.explicit_ref_frame_map {
        sr.read_flag("frame_explicit_ref_frame_map")?
    } else {
        sr.inferred_bool(
            "frame_explicit_ref_frame_map",
            "!explicit_ref_frame_map",
            false,
        );
        false
    };
    control.explicit_ref_frame_map = Some(explicit_ref_frame_map);

    let mut num_total_refs = if ctx.is_bridge {
        sr.inferred_u64("num_total_refs", "IsBridge", 1);
        1
    } else if explicit_ref_frame_map {
        sr.read_bits("num_total_refs", 3)?
    } else if let Some(map) = derive_implicit_ref_map(seq, ctx, reference_state, false, None) {
        control.ref_frame_idx = map.ref_frame_idx;
        map.num_total_refs
    } else {
        control.stop = Some(InterStop::UnmodeledDerivation);
        return Ok(());
    };
    control.num_total_refs = Some(num_total_refs);

    let ref_idx_bits = ceil_log2(seq.num_ref_frames);
    let mut ref_frame_idx = RefIdxBuf::default();
    for i in 0..num_total_refs {
        let idx = if ctx.is_bridge {
            ctx.bridge_frame_ref_idx.unwrap_or(0)
        } else if explicit_ref_frame_map {
            sr.in_loop_item("ref", i as usize, |sr| {
                sr.read_bits("ref_frame_idx", ref_idx_bits)
            })?
        } else {
            break;
        };
        let active_slot_bound = seq.num_ref_frames.min(NUM_REF_FRAMES as u32);
        if idx >= active_slot_bound {
            control.has_invalid_ref_frame_idx = true;
        }
        ref_frame_idx.push(idx);
    }
    if explicit_ref_frame_map || ctx.is_bridge {
        control.ref_frame_idx = ref_frame_idx;
    }

    if ctx.is_bridge {
        let n_w = seq.frame_width_bits;
        let n_h = seq.frame_height_bits;
        let bridge_w = sr
            .read_bits("bridge_frame_width_minus_1", n_w)?
            .saturating_add(1);
        let bridge_h = sr
            .read_bits("bridge_frame_height_minus_1", n_h)?
            .saturating_add(1);
        if let Some((ref_w, ref_h)) = ctx
            .bridge_frame_ref_idx
            .and_then(|idx| ref_dims(reference_state, idx))
        {
            control.frame_size = Some(FrameSize::new(bridge_w.min(ref_w), bridge_h.min(ref_h)));
        } else {
            control.stop = Some(InterStop::PoisonedReferenceState);
            return Ok(());
        }
    } else if frame_size_override_flag && ctx.frame_type != FrameType::Switch {
        control.frame_size =
            parse_frame_size_with_refs(sr, seq, reference_state, &control.ref_frame_idx)?;
    } else if frame_size_override_flag {
        let w = sr
            .read_bits("frame_width_minus_1", seq.frame_width_bits)?
            .saturating_add(1);
        let h = sr
            .read_bits("frame_height_minus_1", seq.frame_height_bits)?
            .saturating_add(1);
        control.frame_size = Some(FrameSize::new(w, h));
    } else if let Some(size) = ctx.default_frame_size {
        control.frame_size = Some(size);
    } else {
        control.stop = Some(InterStop::UnmodeledDerivation);
        return Ok(());
    }

    if !explicit_ref_frame_map && !ctx.is_bridge {
        if let Some(map) =
            derive_implicit_ref_map(seq, ctx, reference_state, true, control.frame_size)
        {
            num_total_refs = map.num_total_refs;
            control.num_total_refs = Some(num_total_refs);
            control.ref_frame_idx = map.ref_frame_idx;
        } else {
            control.stop = Some(InterStop::UnmodeledDerivation);
            return Ok(());
        }
    }

    control.order_hints = control
        .ref_frame_idx
        .iter()
        .take(num_total_refs as usize)
        .map(|&slot| {
            reference_state
                .ref_order_hint
                .and_then(|hints| hints.get(slot as usize).copied())
                .unwrap_or(RESTRICTED_ORDER_HINT)
        })
        .collect();

    if seq.enable_bru && ctx.frame_type == FrameType::Inter && !is_tip && !ctx.is_bridge {
        let use_bru = sr.read_flag("use_bru")?;
        control.use_bru = Some(use_bru);
        if use_bru {
            let n = ceil_log2(num_total_refs);
            control.bru_ref = Some(sr.read_bits("bru_ref", n)?);
            control.bru_inactive = Some(sr.read_flag("bru_inactive")?);
        } else {
            sr.inferred_bool("bru_inactive", "!use_bru", false);
            control.bru_inactive = Some(false);
        }
    } else {
        sr.inferred_bool("use_bru", "!enable_bru || !regular_inter", false);
        sr.inferred_bool("bru_inactive", "!use_bru", false);
        control.use_bru = Some(false);
        control.bru_inactive = Some(false);
    }
    let bru_inactive = control.bru_inactive.unwrap_or(false);

    let use_ref_frame_mvs = if ctx.frame_type == FrameType::Switch
        || !seq.enable_ref_frame_mvs
        || ctx.is_bridge
        || bru_inactive
    {
        sr.inferred_bool("use_ref_frame_mvs", "!allow_ref_frame_mvs", false);
        false
    } else {
        sr.read_flag("use_ref_frame_mvs")?
    };
    control.use_ref_frame_mvs = Some(use_ref_frame_mvs);

    if use_ref_frame_mvs && num_total_refs > 1 && seq.sb_size != SuperblockSize::Block64x64 {
        control.tmvp_sample_step_minus_1 = Some(sr.read_flag("tmvp_sample_step_minus_1")?);
    }

    let tip_gate = seq.enable_tip && use_ref_frame_mvs && num_total_refs >= 2 && !bru_inactive;
    if tip_gate {
        let tip_frame_mode = if is_tip && seq.enable_tip_output {
            TipFrameMode::AsOutput
        } else if sr.read_flag("use_tip_frame_as_ref")? {
            TipFrameMode::AsRef
        } else {
            TipFrameMode::Disabled
        };
        control.tip_frame_mode = Some(tip_frame_mode);
        if tip_frame_mode != TipFrameMode::AsOutput {
            control.tip_global_mv = Some(TipGlobalMv::default());
            control.tip_interpolation_filter = Some(InterpolationFilter::EighttapSharp);
        }

        let opfl_refine_type = if tip_frame_mode == TipFrameMode::AsOutput {
            tip_output_opfl_refine_type(seq)
        } else {
            read_frame_opfl_refine_type(sr, seq.enable_opfl_refine)?
        };
        control.opfl_refine_type = Some(opfl_refine_type);
        let allow_tip_hole_fill =
            if tip_frame_mode != TipFrameMode::Disabled && seq.enable_tip_hole_fill {
                sr.read_flag("allow_tip_hole_fill")?
            } else {
                sr.inferred_bool(
                    "allow_tip_hole_fill",
                    "TipFrameMode == TIP_FRAME_DISABLED || !enable_tip_hole_fill",
                    false,
                );
                false
            };
        control.allow_tip_hole_fill = Some(allow_tip_hole_fill);
        if tip_frame_mode != TipFrameMode::Disabled {
            let Some((num_past_refs, num_future_refs)) = ctx.order_hint.and_then(|order_hint| {
                tip_ref_counts(reference_state, &control.ref_frame_idx, order_hint)
            }) else {
                control.stop = Some(InterStop::PoisonedReferenceState);
                return Ok(());
            };
            control.tip_global_wtd_index = Some(
                if tip_unequal_weighted_allowed(
                    seq,
                    num_past_refs,
                    num_future_refs,
                    opfl_refine_type,
                ) {
                    sr.read_bits_u8("tip_global_wtd_index", 3)?
                } else {
                    sr.inferred_u64("tip_global_wtd_index", "!tip_unequal_weighted_allowed", 0);
                    0
                },
            );
        } else {
            control.tip_global_wtd_index = Some(0);
        }
        if tip_frame_mode == TipFrameMode::AsOutput {
            let tip_global_mv = if sr.read_flag("tip_global_mv_zero")? {
                TipGlobalMv::default()
            } else {
                let row = i32::from(sr.read_bits_u8("tip_global_mv_row_abs", 4)?);
                let col = i32::from(sr.read_bits_u8("tip_global_mv_col_abs", 4)?);
                TipGlobalMv {
                    row: read_tip_mv_sign(sr, "tip_global_mv_row_sign", row)?,
                    col: read_tip_mv_sign(sr, "tip_global_mv_col_sign", col)?,
                }
            };
            let tip_sharp = sr.read_flag("tip_interpolation_filter_sharp")?;
            let tip_interpolation_filter = if tip_sharp {
                InterpolationFilter::EighttapSharp
            } else {
                let tip_regular = sr.read_flag("tip_interpolation_filter_regular")?;
                if tip_regular {
                    InterpolationFilter::Eighttap
                } else {
                    InterpolationFilter::EighttapSmooth
                }
            };
            control.tip_global_mv = Some(tip_global_mv);
            control.tip_interpolation_filter = Some(tip_interpolation_filter);
            control.stop = Some(InterStop::TipAsOutputReturn);
            return Ok(());
        }
    } else {
        control.tip_frame_mode = Some(TipFrameMode::Disabled);
        control.allow_tip_hole_fill = Some(false);
        control.tip_global_wtd_index = Some(0);
        if !bru_inactive && !ctx.is_bridge {
            control.opfl_refine_type =
                Some(read_frame_opfl_refine_type(sr, seq.enable_opfl_refine)?);
        }
    }

    if bru_inactive || ctx.is_bridge {
        control.stop = Some(InterStop::BruInactiveOrBridgeReturn);
        return Ok(());
    }

    let scc = parse_screen_content_params_with_reader(
        sr,
        seq.seq_force_screen_content_tools,
        seq.seq_force_integer_mv,
    )?;
    control.allow_screen_content_tools = Some(scc.allow_screen_content_tools);
    let intrabc = parse_intrabc_params_with_reader(sr, false, seq.allow_frame_max_bvp_drl_bits)?;
    control.allow_intrabc = Some(intrabc.allow_intrabc);
    control.max_bvp_drl_bits_minus_1 = intrabc.max_bvp_drl_bits_minus_1;

    let mut max_drl_bits_minus_1 = seq.seq_max_drl_bits_minus_1;
    if seq.allow_frame_max_drl_bits {
        let change_drl = sr.read_flag("change_drl")?;
        if change_drl {
            let n = MAX_REF_MV_STACK_SIZE - 2;
            let mut value = sr.read_ns("max_drl_bits_minus_1", n)?;
            if value >= seq.seq_max_drl_bits_minus_1 {
                value += 1;
            }
            max_drl_bits_minus_1 = value;
        }
    }
    control.max_drl_bits_minus_1 = Some(max_drl_bits_minus_1);

    let mv_precision = if scc.force_integer_mv {
        sr.absent("use_qtr_precision_mv", "force_integer_mv");
        MvPrecision::OnePel
    } else {
        let use_qtr_precision_mv = sr.read_flag("use_qtr_precision_mv")?;
        if use_qtr_precision_mv {
            MvPrecision::QuarterPel
        } else {
            let allow_high_precision_mv = sr.read_flag("allow_high_precision_mv")?;
            if allow_high_precision_mv {
                MvPrecision::EighthPel
            } else {
                MvPrecision::HalfPel
            }
        }
    };
    control.mv_precision = Some(mv_precision);
    control.force_integer_mv = Some(scc.force_integer_mv);

    control.interpolation_filter = Some(read_interpolation_filter_with_reader(sr)?);

    let mut motion_modes = [false; MOTION_MODES];
    for (mode, enabled) in motion_modes.iter_mut().enumerate().skip(INTERINTRA) {
        if !seq.seq_frame_motion_modes_present_flag {
            *enabled = seq.seq_enabled_motion_modes[mode];
        } else if seq.seq_enabled_motion_modes[mode] {
            *enabled = sr.in_loop_item("mode", mode, |sr| {
                sr.read_flag("frame_enabled_motion_modes")
            })?;
        } else {
            *enabled = false;
        }
    }
    control.frame_enabled_motion_modes = Some(motion_modes);

    control.disable_cdf_update = Some(sr.read_flag("disable_cdf_update")?);

    control.stop = Some(InterStop::ReachedSharedTail);
    Ok(())
}

fn read_tip_mv_sign<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    name: &'static str,
    magnitude: i32,
) -> Result<i32> {
    if magnitude != 0 && sr.read_flag(name)? {
        Ok(-magnitude)
    } else {
        Ok(magnitude)
    }
}

fn read_frame_opfl_refine_type<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    enable_opfl_refine: u8,
) -> Result<u32> {
    sr.in_struct("frame_opfl_refine_type", Some("5.18.3.2"), |sr| {
        if enable_opfl_refine == REFINE_AUTO {
            let opfl_refine_type = sr.read_bits("opfl_refine_type", 1)?;
            if opfl_refine_type != REFINE_SWITCHABLE {
                return Ok(if sr.read_bit("opfl_refine_all")? != 0 {
                    REFINE_ALL
                } else {
                    REFINE_NONE
                });
            }
            return Ok(REFINE_SWITCHABLE);
        }
        sr.inferred_u64(
            "opfl_refine_type",
            "enable_opfl_refine != REFINE_AUTO",
            u64::from(enable_opfl_refine),
        );
        Ok(u32::from(enable_opfl_refine))
    })
}

fn tip_output_opfl_refine_type(seq: &InterSeqView) -> u32 {
    if !seq.enable_tip_refinemv || u32::from(seq.enable_opfl_refine) == REFINE_NONE {
        REFINE_NONE
    } else {
        REFINE_ALL
    }
}

fn parse_frame_size_with_refs<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    seq: &InterSeqView,
    reference_state: &FrameReferenceStateView<'_>,
    ref_frame_idx: &[u32],
) -> Result<Option<FrameSize>> {
    sr.in_struct("frame_size_with_refs", Some("5.18.4.3"), |sr| {
        for (i, &idx) in ref_frame_idx.iter().enumerate() {
            let found_ref = sr.in_loop_item("ref", i, |sr| sr.read_flag("found_ref"))?;
            if found_ref {
                return Ok(ref_dims(reference_state, idx).map(|(w, h)| FrameSize::new(w, h)));
            }
        }
        let w = sr
            .read_bits("frame_width_minus_1", seq.frame_width_bits)?
            .saturating_add(1);
        let h = sr
            .read_bits("frame_height_minus_1", seq.frame_height_bits)?
            .saturating_add(1);
        Ok(Some(FrameSize::new(w, h)))
    })
}

/// Derives the implicit map via AV2 § 7.7 with single-layer dependency defaults.
/// Multiple valid slots require complete quantizer, order-hint, and dimension slices
/// (and frame dimensions for `check_res`). At most one valid slot needs no ranking score.
/// Returns `None` when required state is unavailable.
fn derive_implicit_ref_map(
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    reference_state: &FrameReferenceStateView<'_>,
    check_res: bool,
    frame_size: Option<FrameSize>,
) -> Option<GetRefFrames> {
    let ref_valid = reference_state.ref_valid?;
    let num_ref_frames = (seq.num_ref_frames as usize).min(NUM_REF_FRAMES);

    let allowed_frames = ctx.ras_long_term_ref_mask.unwrap_or(u32::MAX)
        & reference_state.allowed_frames.unwrap_or(u32::MAX);

    let ref_mlayers = reference_state.ref_mlayer_id;
    let ref_tlayers = reference_state.ref_tlayer_id;
    let valid_count = ref_valid
        .iter()
        .enumerate()
        .take(num_ref_frames)
        .filter(|&(i, v)| {
            *v && ((allowed_frames >> i) & 1) != 0
                && ref_mlayers.is_none_or(|layers| {
                    layers.get(i).copied().is_some_and(|ref_mlayer| {
                        seq.mlayer_dependency_map
                            .depends_on(ctx.obu_mlayer_id, EmbeddedLayerId::from_bits(ref_mlayer))
                    })
                })
                && ref_tlayers.is_none_or(|layers| {
                    layers.get(i).copied().is_some_and(|ref_tlayer| {
                        seq.tlayer_dependency_map.depends_on(
                            ctx.obu_mlayer_id,
                            ctx.obu_tlayer_id,
                            TemporalLayerId::from_bits(ref_tlayer),
                        )
                    })
                })
        })
        .count();
    let current_order_hint = if valid_count <= 1 {
        ctx.order_hint.unwrap_or(0)
    } else {
        ctx.order_hint?
    };
    if valid_count > 1 {
        let covers_active =
            |slice: Option<&[u32]>| slice.is_some_and(|s| s.len() >= num_ref_frames);
        let ranking_inputs_complete = ref_valid.len() >= num_ref_frames
            && covers_active(reference_state.ref_base_q_idx)
            && covers_active(reference_state.ref_order_hint)
            && covers_active(reference_state.ref_frame_width)
            && covers_active(reference_state.ref_frame_height)
            && (!check_res || frame_size.is_some());
        if !ranking_inputs_complete {
            return None;
        }
    }

    let ref_order_hint = reference_state.ref_order_hint;
    let ref_w = reference_state.ref_frame_width;
    let ref_h = reference_state.ref_frame_height;
    let ref_base_q_idx = reference_state.ref_base_q_idx;

    let default_slot = RefSlot {
        valid: false,
        order_hint: 0,
        base_q_idx: 0,
        counter: 0,
        mlayer_id: 0,
        tlayer_id: 0,
        width: 0,
        height: 0,
    };
    let mut slots = [default_slot; NUM_REF_FRAMES];
    for (i, slot) in slots.iter_mut().enumerate().take(num_ref_frames) {
        let valid = ref_valid.get(i).copied().unwrap_or(false);
        let order_hint = ref_order_hint
            .and_then(|s| s.get(i).copied())
            .map_or(0, |oh| ref_order_hint_to_spec(oh).unwrap_or(i32::MAX));
        slot.valid = valid;
        slot.order_hint = order_hint;
        slot.counter = reference_state
            .ref_counter
            .and_then(|counters| counters.get(i).copied())
            .unwrap_or(i as u32);
        slot.mlayer_id = ref_mlayers.and_then(|s| s.get(i).copied()).unwrap_or(0);
        slot.tlayer_id = ref_tlayers.and_then(|s| s.get(i).copied()).unwrap_or(0);
        slot.width = ref_w.and_then(|s| s.get(i).copied()).unwrap_or(0);
        slot.height = ref_h.and_then(|s| s.get(i).copied()).unwrap_or(0);
        slot.base_q_idx = ref_base_q_idx.and_then(|s| s.get(i).copied()).unwrap_or(0);
    }

    let (frame_width, frame_height) = frame_size.map_or((0, 0), |fs| (fs.width, fs.height));

    let input = GetRefFramesInput {
        num_ref_frames: seq.num_ref_frames,
        slots,
        order_hint: i32::try_from(current_order_hint).unwrap_or(i32::MAX),
        obu_mlayer_id: ctx.obu_mlayer_id.get(),
        obu_tlayer_id: ctx.obu_tlayer_id.get(),
        allowed_frames: allowed_frames as i32,
        is_bridge: false,
        bridge_frame_ref_idx: 0,
        frame_width,
        frame_height,
        mlayer_dependency_map: Some(&seq.mlayer_dependency_map),
        tlayer_dependency_map: Some(&seq.tlayer_dependency_map),
    };
    Some(get_ref_frames(&input, check_res))
}

#[cfg(test)]
#[path = "inter_tests.rs"]
mod tests;
