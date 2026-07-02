// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! State-aware parsing of the non-intra `frame_header_info()` control region
//! (AV2 v1.0.0 § 5.18.2, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`).
//!
//! This is the inter / switch / TIP / bridge counterpart of the intra control region in
//! [`super::info`]. On the non-bridge path it begins after the output-control flags
//! (`immediate_output_frame` / `implicit_output_frame`); on the bridge path the caller
//! enters with `is_bridge == true` just after `bridge_frame_ref_idx`. It parses the
//! reference-control region — primary-reference signaling (`signal_primary_ref_frame`,
//! `disable_cross_frame_cdf_init`, `primary_ref_frame`), the bridge
//! `bridge_frame_overwrite_flag`, the per-arm `refresh_frame_flags`, the explicit
//! reference map (`frame_explicit_ref_frame_map`, `num_total_refs`, `ref_frame_idx[i]`),
//! the reference-state-grounded frame sizes (`frame_size_with_refs()` § 5.18.4.3,
//! `frame_size_with_bridge()` § 5.18.4.2), the BRU triple (`use_bru` / `bru_ref` /
//! `bru_inactive`), `use_ref_frame_mvs` / `tmvp_sample_step_minus_1`, the TIP block,
//! `frame_opfl_refine_type()` (§ 5.18.3.2), `screen_content_params()` / `intrabc_params()`,
//! the `max_drl_bits_minus_1` override, the MV precision, `read_interpolation_filter()`
//! (§ 5.18.5.1), the `frame_enabled_motion_modes` loop, and — on the ordinary inter /
//! switch path — `disable_cdf_update` (mirror :5041) immediately before the shared tail —
//! exactly per the mirror, gated on the parsed sequence configuration and the modeled
//! reference state.
//!
//! Per-block bit alignment is anchored on a few § 3 / Table 6.5 constants that select read
//! widths and presence: [`PRIMARY_REF_NONE`] / [`PRIMARY_REF_CHOOSE`] (the inferred
//! primary-reference values), [`REFINE_AUTO`] / [`REFINE_SWITCHABLE`] (the
//! `frame_opfl_refine_type()` arm and its `opfl_refine_all` gate), and
//! [`MAX_REF_MV_STACK_SIZE`] / [`MOTION_MODES`] / [`INTERINTRA`] (the `ns(n)` / loop
//! bounds). A wrong constant silently mis-positions every following field.
//!
//! ## Stop taxonomy
//!
//! [`parse_inter_control_into`] fills a caller-owned [`InterControl`] carrying every
//! exactly-determined field plus the terminal [`InterStop`]. The variants are all
//! **coverage** stops, never
//! truncations:
//! - [`InterStop::ReachedSharedTail`] — the control region parsed through every modeled
//!   field, including `disable_cdf_update`, up to the shared `tile_info()` (mirror :5183);
//!   the caller continues into the shared structure cluster.
//! - [`InterStop::BruInactiveOrBridgeReturn`] / [`InterStop::TipAsOutputReturn`] — the
//!   `bru_inactive` / `IsBridge` (mirror :4971/:5045) or TIP-as-output (mirror :4945) arm
//!   `return`s after `film_grain_config()` / `tile_info()` reads needing reference-frame
//!   dims this phase does not thread.
//! - [`InterStop::UnmodeledDerivation`] — a derivation needing unmodeled syntax
//!   (`get_ref_frames()` for the implicit reference map).
//! - [`InterStop::PoisonedReferenceState`] — see *Honest poisoning* below.
//!
//! ## Honest poisoning
//!
//! Many § 5.18.2 inter derivations consume per-slot reference facts the validator models
//! in [`super::FrameReferenceStateView`] (`RefValid[]`, `RefOrderHint[]`, dims). Where a
//! derivation that affects **bit positions** needs a slot the model has not proven valid
//! (an `Unknown` / `ProvenInvalid` slot, or no modeled buffer at all), the parser stops
//! honestly with [`InterStop::PoisonedReferenceState`] — the facts parsed up to that
//! branch are preserved on the returned [`InterControl`]. Derivations the model cannot
//! perform without unmodeled syntax (`get_ref_frames()` for the implicit reference map,
//! `get_past_future_cur_ref_lists()`) stop with [`InterStop::UnmodeledDerivation`].
//!
//! The parser never guesses a bit position: a stop is taken *before* the first read whose
//! width or presence depends on an unavailable derivation.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::frame::config::{parse_intrabc_params, parse_screen_content_params_full};
use crate::headers::frame::filtering::{InterpolationFilter, read_interpolation_filter};
use crate::headers::frame::get_ref_frames::{
    GetRefFrames, GetRefFramesInput, RefSlot, get_ref_frames,
};
use crate::headers::frame::size::{FrameSize, ceil_log2};
use crate::headers::sequence::SuperblockSize;
use crate::types::ObuType;

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

/// `EXTENDWARP` (AV2 v1.0.0 § 3).
pub const EXTENDWARP: usize = 4;

/// `PRIMARY_REF_NONE` (AV2 v1.0.0 § 3).
const PRIMARY_REF_NONE: u8 = 7;

/// `PRIMARY_REF_CHOOSE` (AV2 v1.0.0 § 3): the value `primary_ref_frame` takes when
/// `signal_primary_ref_frame == 0` (§ 5.18.2 mirror :4397).
const PRIMARY_REF_CHOOSE: u8 = 8;

/// `REFINE_SWITCHABLE` (AV2 v1.0.0 Table 6.5, § 6.17.2 mirror :947): the `opfl_refine_type`
/// value at which `frame_opfl_refine_type()` does NOT read `opfl_refine_all` (§ 5.18.3.2
/// mirror :5601, `if ( opfl_refine_type != REFINE_SWITCHABLE )`). `opfl_refine_type` is read
/// as `f(1)`, so its value space is {0, 1}: a value of 2 here would make the inequality
/// always true and consume an extra `opfl_refine_all` bit, mis-positioning every following
/// field.
const REFINE_SWITCHABLE: u32 = 1;

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
    /// `TIP_FRAME_AS_OUTPUT` (`EnableTipOutput && is_tip_frame()`, or `tip_frame_mode == 1`).
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
            Self::AsOutput => "as_output",
            Self::Other(_) => "other",
        }
    }
}

/// Why the inter control-region parse stopped (a **coverage** stop, never a truncation).
///
/// The variants split into the two honest classes the module documents: a derivation
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
    /// `bru_inactive` or `IsBridge` was set, so the control region took the early-return arm
    /// (mirror :4971/:5045). On this non-intra path the arm's `base_q_idx = RefBaseQIdx[refIdx]` /
    /// `DeltaQ` are reference-derived (no-bit) values this phase does not thread, so the parse
    /// stops at the start of that arm with the facts preserved. (`tile_info()` reads zero bits for
    /// a bridge, and `film_grain_config()` reads zero bits here because `immediate_output_frame ==
    /// 0` on this path forces `apply_grain == 0`; they do NOT need reference-frame dims. The
    /// single-picture-bridge `FrameIsIntra` path, where `immediate_output_frame == 1` makes the
    /// grain tail non-trivial, is handled separately by `parse_single_picture_bridge_tail`.)
    BruInactiveOrBridgeReturn,
    /// `TipFrameMode == TIP_FRAME_AS_OUTPUT`, so the control region takes the TIP-output
    /// arm (mirror :4945) and `return`s (mirror :5177) after its quant / deblocking /
    /// `film_grain_config()` reads — which need reference-frame dims this phase does not
    /// yet thread, so the parse stops at the start of that arm with the facts preserved.
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

    /// `true` when the stop converges into the shared tail (`tile_info()` onward), so the
    /// caller parses the shared structure cluster after the inter control region.
    #[must_use]
    pub const fn reaches_shared_tail(self) -> bool {
        matches!(self, Self::ReachedSharedTail)
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
    pub ref_frame_idx: Vec<u32>,
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
    /// `seq_max_drl_bits_minus_1` (§ 5.4.6).
    pub seq_max_drl_bits_minus_1: u32,
    /// `allow_frame_max_drl_bits` (§ 5.4.6).
    pub allow_frame_max_drl_bits: bool,
    /// `enable_flex_mvres` (§ 5.4.6) — derives `UsePerBlockMvPrecision` (no bits).
    pub enable_flex_mvres: bool,
    /// `seq_frame_motion_modes_present_flag` (§ 5.4.6).
    pub seq_frame_motion_modes_present_flag: bool,
    /// `seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]` (§ 5.4.6).
    pub seq_enabled_motion_modes: [bool; MOTION_MODES],
    /// `enable_opfl_refine` (§ 5.4.6): selects the `frame_opfl_refine_type()` arm
    /// (§ 5.18.3.2). `REFINE_AUTO` (`3`) reads bits; otherwise it reads none.
    pub enable_opfl_refine: u8,
    /// `max_mlayer_id` (§ 5.4.1): selects the RAS `refresh_frame_flags` derivation arm.
    pub max_mlayer_id: u8,
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
    /// `max_frame_width` (§ 5.4.1): the non-override `frame_size()` default dims.
    pub max_frame_width: u32,
    /// `max_frame_height` (§ 5.4.1): the non-override `frame_size()` default dims.
    pub max_frame_height: u32,
    /// The frame `SbSize` for the inter path (`frame_sb_size(false)`, mirror :4317-4329).
    pub sb_size: SuperblockSize,
}

/// Per-frame inputs the inter control region needs that the caller derives from the
/// already-parsed prefix / output-control region (AV2 v1.0.0 § 5.18.2).
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterFrameContext {
    /// The OBU type (selects TIP / switch / RAS arms via `is_tip_frame()` etc.).
    pub obu_type: ObuType,
    /// The derived `FrameType`.
    pub frame_type: FrameType,
    /// `IsBridge`.
    pub is_bridge: bool,
    /// `bridge_frame_ref_idx`, when this is a bridge frame.
    pub bridge_frame_ref_idx: Option<u32>,
    /// `true` when `cur_mfh_id == 0` (the non-override `frame_size()` default dims come
    /// from the sequence maxima; a `cur_mfh_id > 0` non-override inter size needs the
    /// resolved MFH defaults this phase does not thread on the inter path).
    pub cur_mfh_id_is_zero: bool,
    /// `OrderHint` (§ 5.18.2): the parsed `order_hint` LSB value (`OrderHintLsbs`), threaded
    /// for the implicit reference-map ranking (`get_ref_frames()` § 7.7). The minimal
    /// at-most-one-valid-reference case does not depend on its value, but it is supplied for
    /// the modeled `get_ref_frames()` input completeness.
    pub order_hint: u32,
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

/// Parses the non-intra `frame_header_info()` control region (AV2 v1.0.0 § 5.18.2) into a
/// freshly default [`InterControl`], starting after the output-control flags. The reader is
/// positioned just before `disable_cross_frame_cdf_init` (mirror :4341); on a
/// [`InterStop::ReachedSharedTail`] stop it is positioned at the shared `tile_info()` (mirror
/// :5183).
///
/// `frame_size_override_flag` was read (or inferred) by the caller along with the
/// output-control flags; it selects the reference-grounded frame-size arm.
///
/// This is the value-returning convenience used by the unit tests; production callers use
/// [`parse_inter_control_into`] with a caller-owned `control` so a mid-field
/// [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) preserves the partial facts
/// across EOF.
///
/// # Errors
/// Returns a typed error if the payload ends or is malformed before a modeled field can
/// be read. A branch needing unmodeled state or poisoned reference facts returns `Ok`
/// with the corresponding [`InterStop`], never an error and never a guessed value.
#[cfg(test)]
pub(crate) fn parse_inter_control(
    reader: &mut BitReader<'_>,
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    reference_state: &FrameReferenceStateView<'_>,
    frame_size_override_flag: bool,
) -> Result<InterControl> {
    let mut control = InterControl::default();
    parse_inter_control_into(
        reader,
        seq,
        ctx,
        reference_state,
        frame_size_override_flag,
        &mut control,
    )?;
    Ok(control)
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
pub(crate) fn parse_inter_control_into(
    reader: &mut BitReader<'_>,
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    reference_state: &FrameReferenceStateView<'_>,
    frame_size_override_flag: bool,
    control: &mut InterControl,
) -> Result<()> {
    if ctx.is_bridge || ctx.frame_type == FrameType::Switch {
        control.primary_ref_frame = Some(PRIMARY_REF_NONE);
        control.disable_cross_frame_cdf_init = Some(false);
    } else {
        let signal = reader.read_flag()?;
        control.signal_primary_ref_frame = Some(signal);
        if !ctx.obu_type.is_tip_frame() {
            control.disable_cross_frame_cdf_init = Some(reader.read_flag()?);
        } else {
            control.disable_cross_frame_cdf_init = Some(false);
        }
        if signal {
            control.primary_ref_frame = Some(reader.read_bits_u8(3)?);
        } else {
            control.primary_ref_frame = Some(PRIMARY_REF_CHOOSE);
        }
    }

    if ctx.is_bridge {
        control.bridge_frame_overwrite_flag = Some(reader.read_flag()?);
    }

    control.refresh_frame_flags = read_inter_refresh_frame_flags(reader, seq, ctx, control)?;

    if control.refresh_frame_flags.is_none() {
        control.stop = Some(InterStop::UnmodeledDerivation);
        return Ok(());
    }

    parse_inter_reference_region(
        reader,
        seq,
        ctx,
        reference_state,
        frame_size_override_flag,
        control,
    )?;

    Ok(())
}

/// Reads / derives `refresh_frame_flags` for the non-intra arms (AV2 § 5.18.2 mirror
/// :4429-4537). Returns `Ok(None)` only for the RAS arm whose derivation needs
/// reference state the model has not grounded (an honest stop the caller surfaces).
fn read_inter_refresh_frame_flags(
    reader: &mut BitReader<'_>,
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    control: &InterControl,
) -> Result<Option<u32>> {
    if ctx.is_bridge {
        let overwrite = control.bridge_frame_overwrite_flag.unwrap_or(false);
        if !overwrite {
            let idx = ctx.bridge_frame_ref_idx.unwrap_or(0);
            return Ok(Some(1u32.wrapping_shl(idx)));
        }
        return Ok(Some(reader.read_f(seq.num_ref_frames)?));
    }

    if ctx.obu_type == ObuType::RasFrame && seq.max_mlayer_id == 0 {
        return Ok(None);
    }

    if ctx.frame_type == FrameType::Switch {
        return Ok(Some(reader.read_f(seq.num_ref_frames)?));
    }

    if seq.enable_short_refresh_frame_flags {
        let has = reader.read_flag()?;
        if has {
            let frame_to_refresh = reader.read_f(ceil_log2(seq.num_ref_frames))?;
            return Ok(Some(1u32.wrapping_shl(frame_to_refresh)));
        }
        return Ok(Some(0));
    }

    Ok(Some(reader.read_f(seq.num_ref_frames)?))
}

/// Parses the `!FrameIsIntra` reference-control region (AV2 § 5.18.2 mirror :4577-5181),
/// setting `control` and the terminal [`InterStop`]. Stops honestly before any read whose
/// width / presence depends on a derivation this phase cannot perform.
#[allow(clippy::too_many_lines)]
fn parse_inter_reference_region(
    reader: &mut BitReader<'_>,
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    reference_state: &FrameReferenceStateView<'_>,
    frame_size_override_flag: bool,
    control: &mut InterControl,
) -> Result<()> {
    let is_tip = ctx.obu_type.is_tip_frame();

    let explicit_ref_frame_map = if ctx.frame_type == FrameType::Switch || ctx.is_bridge {
        true
    } else if seq.explicit_ref_frame_map {
        reader.read_flag()? // frame_explicit_ref_frame_map f(1)
    } else {
        false
    };
    control.explicit_ref_frame_map = Some(explicit_ref_frame_map);

    let mut num_total_refs = if ctx.is_bridge {
        1
    } else if explicit_ref_frame_map {
        reader.read_bits(3)? // num_total_refs f(3)
    } else if let Some(map) = derive_implicit_ref_map(seq, ctx, reference_state, false, None) {
        control.ref_frame_idx = map.ref_frame_idx;
        map.num_total_refs
    } else {
        control.stop = Some(InterStop::UnmodeledDerivation);
        return Ok(());
    };
    control.num_total_refs = Some(num_total_refs);

    let ref_idx_bits = ceil_log2(seq.num_ref_frames);
    let mut ref_frame_idx = Vec::with_capacity(num_total_refs as usize);
    for _ in 0..num_total_refs {
        let idx = if ctx.is_bridge {
            ctx.bridge_frame_ref_idx.unwrap_or(0)
        } else if explicit_ref_frame_map {
            reader.read_f(ref_idx_bits)?
        } else {
            break;
        };
        let active_slot_bound = u64::from(seq.num_ref_frames).min(NUM_REF_FRAMES as u64);
        if u64::from(idx) >= active_slot_bound {
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
        let bridge_w = reader.read_bits(n_w)?.saturating_add(1);
        let bridge_h = reader.read_bits(n_h)?.saturating_add(1);
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
            parse_frame_size_with_refs(reader, seq, reference_state, &control.ref_frame_idx)?;
    } else if frame_size_override_flag {
        let w = reader.read_bits(seq.frame_width_bits)?.saturating_add(1);
        let h = reader.read_bits(seq.frame_height_bits)?.saturating_add(1);
        control.frame_size = Some(FrameSize::new(w, h));
    } else if ctx.cur_mfh_id_is_zero {
        control.frame_size = Some(FrameSize::new(seq.max_frame_width, seq.max_frame_height));
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

    if seq.enable_bru && ctx.frame_type == FrameType::Inter && !is_tip && !ctx.is_bridge {
        let use_bru = reader.read_flag()?; // use_bru f(1)
        control.use_bru = Some(use_bru);
        if use_bru {
            let n = ceil_log2(num_total_refs);
            control.bru_ref = Some(reader.read_f(n)?); // bru_ref f(n)
            control.bru_inactive = Some(reader.read_flag()?); // bru_inactive f(1)
        } else {
            control.bru_inactive = Some(false);
        }
    } else {
        control.use_bru = Some(false);
        control.bru_inactive = Some(false);
    }
    let bru_inactive = control.bru_inactive.unwrap_or(false);

    let use_ref_frame_mvs = if ctx.frame_type == FrameType::Switch
        || !seq.enable_ref_frame_mvs
        || ctx.is_bridge
        || bru_inactive
    {
        false
    } else {
        reader.read_flag()? // use_ref_frame_mvs f(1)
    };
    control.use_ref_frame_mvs = Some(use_ref_frame_mvs);

    if use_ref_frame_mvs && num_total_refs > 1 && seq.sb_size != SuperblockSize::Block64x64 {
        control.tmvp_sample_step_minus_1 = Some(reader.read_flag()?);
    }

    let tip_gate = seq.enable_tip && use_ref_frame_mvs && num_total_refs >= 2 && !bru_inactive;
    if tip_gate {
        // 5.18.2: TIP-output OBUs derive TIP_FRAME_AS_OUTPUT with no coded bit; both TIP arms defer
        if is_tip || reader.read_flag()? {
            control.stop = Some(InterStop::PoisonedReferenceState);
            return Ok(());
        }
    }
    control.tip_frame_mode = Some(TipFrameMode::Disabled);
    if !bru_inactive && !ctx.is_bridge {
        read_frame_opfl_refine_type(reader, seq.enable_opfl_refine)?;
    }

    if bru_inactive || ctx.is_bridge {
        control.stop = Some(InterStop::BruInactiveOrBridgeReturn);
        return Ok(());
    }

    let scc = parse_screen_content_params_full(
        reader,
        seq.seq_force_screen_content_tools,
        seq.seq_force_integer_mv,
    )?;
    control.allow_screen_content_tools = Some(scc.allow_screen_content_tools);
    control.allow_intrabc = Some(parse_intrabc_params(
        reader,
        false, // FrameIsIntra == false on the inter path
        seq.allow_frame_max_bvp_drl_bits,
    )?);

    let mut max_drl_bits_minus_1 = seq.seq_max_drl_bits_minus_1;
    if seq.allow_frame_max_drl_bits {
        let change_drl = reader.read_flag()?; // change_drl f(1)
        if change_drl {
            let n = MAX_REF_MV_STACK_SIZE - 2;
            let mut value = reader.read_ns(n)?;
            if value >= seq.seq_max_drl_bits_minus_1 {
                value += 1;
            }
            max_drl_bits_minus_1 = value;
        }
    }
    control.max_drl_bits_minus_1 = Some(max_drl_bits_minus_1);

    let mv_precision = if scc.force_integer_mv {
        MvPrecision::OnePel
    } else {
        let use_qtr_precision_mv = reader.read_flag()?;
        if use_qtr_precision_mv {
            MvPrecision::QuarterPel
        } else {
            let allow_high_precision_mv = reader.read_flag()?;
            if allow_high_precision_mv {
                MvPrecision::EighthPel
            } else {
                MvPrecision::HalfPel
            }
        }
    };
    control.mv_precision = Some(mv_precision);
    control.force_integer_mv = Some(scc.force_integer_mv);
    let _ = seq.enable_flex_mvres;

    control.interpolation_filter = Some(read_interpolation_filter(reader)?);

    let mut motion_modes = [false; MOTION_MODES];
    for (mode, enabled) in motion_modes.iter_mut().enumerate().skip(INTERINTRA) {
        if !seq.seq_frame_motion_modes_present_flag {
            *enabled = seq.seq_enabled_motion_modes[mode];
        } else if seq.seq_enabled_motion_modes[mode] {
            *enabled = reader.read_flag()?;
        } else {
            *enabled = false;
        }
    }
    control.frame_enabled_motion_modes = Some(motion_modes);

    control.disable_cdf_update = Some(reader.read_flag()?);

    control.stop = Some(InterStop::ReachedSharedTail);
    Ok(())
}

/// Parses `frame_opfl_refine_type()` (AV2 v1.0.0 § 5.18.3.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3-2`) on the non-TIP-output path.
///
/// With `TipFrameMode != TIP_FRAME_AS_OUTPUT`, the structure reads `opfl_refine_type`
/// f(1) (and a conditional `opfl_refine_all` f(1)) only when
/// `enable_opfl_refine == REFINE_AUTO`; otherwise it reads no bits (the type is the
/// sequence value). The derived `opfl_refine_type` feeds reconstruction this phase does
/// not model, so it is consumed for bit alignment only.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if a signaled
/// field is truncated.
fn read_frame_opfl_refine_type(reader: &mut BitReader<'_>, enable_opfl_refine: u8) -> Result<()> {
    if enable_opfl_refine == REFINE_AUTO {
        let opfl_refine_type = reader.read_bits(1)?;
        if opfl_refine_type != REFINE_SWITCHABLE {
            reader.read_bit()?;
        }
    }
    Ok(())
}

/// Parses `frame_size_with_refs()` (AV2 § 5.18.4.3): reads `found_ref` f(1) per ref until
/// a hit, then copies `RefFrameWidth/Height[ ref_frame_idx[i] ]`. Returns `Ok(None)` when
/// a hit lands on a slot whose dims the model has not proven — `None` here means the size is
/// **unknown but every consumed bit is exact** (the `found_ref` bits up to and including the
/// hit were read; per § 5.18.4.3 a hit then calls `compute_image_size()`, which reads no
/// bits, mirror :5847), so the caller continues parsing the rest of the modeled inter
/// control region rather than stopping. When no ref is found the fallback
/// `frame_size()` is the non-override default, which the caller does not reach here (override
/// is set), so an all-miss reads `NumTotalRefs` `found_ref` bits then falls to the override
/// `frame_size()` explicit dims.
fn parse_frame_size_with_refs(
    reader: &mut BitReader<'_>,
    seq: &InterSeqView,
    reference_state: &FrameReferenceStateView<'_>,
    ref_frame_idx: &[u32],
) -> Result<Option<FrameSize>> {
    for &idx in ref_frame_idx {
        let found_ref = reader.read_flag()?; // found_ref f(1)
        if found_ref {
            return Ok(ref_dims(reference_state, idx).map(|(w, h)| FrameSize::new(w, h)));
        }
    }
    let w = reader.read_bits(seq.frame_width_bits)?.saturating_add(1);
    let h = reader.read_bits(seq.frame_height_bits)?.saturating_add(1);
    Ok(Some(FrameSize::new(w, h)))
}

/// Derives the implicit reference map via `get_ref_frames()` (AV2 § 7.7), gated to the
/// **at-most-one-valid-reference** case the modeled reference state can resolve EXACTLY.
///
/// § 7.7 ranks distinct references by a score built from `RefBaseQIdx`, `RefMLayerId`,
/// `RefTLayerId`, `RefCounter`, the per-frame `AllowedFrames`, and the layer-dependency maps —
/// state [`FrameReferenceStateView`] does not (yet) model beyond `RefValid` / `RefOrderHint` /
/// dims. When the modeled view proves **at most one** `RefValid` slot, the score / sort / drop
/// / restricted-append machinery is irrelevant: the result is `NumTotalRefs = Min(NRanked,
/// ActiveNumRefFrames)` over a single distinct reference, i.e. `ref_frame_idx = [theSlot]`
/// (one valid slot) or the empty map (none). That outcome is independent of every unmodeled
/// score input, so building a [`GetRefFramesInput`] with deterministic single-layer defaults
/// (each valid slot a distinct `RefCounter`, `AllowedFrames = -1`, layers depend) yields the
/// EXACT § 7.7 answer — this is the real ranking, not a hardcoded `[0]`.
///
/// Returns `Some(GetRefFrames)` when the gate holds (`<= 1` proven-valid slot AND the view
/// models `RefValid`), else `None` (the caller stops with [`InterStop::UnmodeledDerivation`] —
/// a richer reference state needs the unmodeled scoring inputs). `check_res` is the § 7.7
/// `checkRes` input (`false` for the first call mirror :4607, `true` for the second :4647).
fn derive_implicit_ref_map(
    seq: &InterSeqView,
    ctx: &InterFrameContext,
    reference_state: &FrameReferenceStateView<'_>,
    check_res: bool,
    frame_size: Option<FrameSize>,
) -> Option<GetRefFrames> {
    let ref_valid = reference_state.ref_valid?;
    let num_ref_frames = (seq.num_ref_frames as usize).min(NUM_REF_FRAMES);

    let valid_count = ref_valid
        .iter()
        .take(num_ref_frames)
        .filter(|v| **v)
        .count();
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
            .map_or(0, |oh| i32::try_from(oh).unwrap_or(i32::MAX));
        slot.valid = valid;
        slot.order_hint = order_hint;
        slot.counter = i as u32; // distinct per slot (dedup is a no-op for <= 1 valid)
        slot.width = ref_w.and_then(|s| s.get(i).copied()).unwrap_or(0);
        slot.height = ref_h.and_then(|s| s.get(i).copied()).unwrap_or(0);
        slot.base_q_idx = ref_base_q_idx.and_then(|s| s.get(i).copied()).unwrap_or(0);
    }

    let (frame_width, frame_height) = frame_size.map_or((0, 0), |fs| (fs.width, fs.height));

    let input = GetRefFramesInput {
        num_ref_frames: seq.num_ref_frames,
        slots,
        order_hint: i32::try_from(ctx.order_hint).unwrap_or(i32::MAX),
        obu_mlayer_id: 0,
        obu_tlayer_id: 0,
        allowed_frames: -1,
        is_bridge: false,
        bridge_frame_ref_idx: 0,
        frame_width,
        frame_height,
        layer_dependency: |_frame_mlayer, _frame_tlayer, _ref_mlayer, _ref_tlayer| true,
    };
    Some(get_ref_frames(&input, check_res))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn inter_seq() -> InterSeqView {
        InterSeqView {
            num_ref_frames: 8,
            enable_short_refresh_frame_flags: false,
            explicit_ref_frame_map: true,
            enable_ref_frame_mvs: true,
            enable_bru: false,
            enable_tip: false,
            seq_max_drl_bits_minus_1: 0,
            allow_frame_max_drl_bits: false,
            enable_flex_mvres: false,
            seq_frame_motion_modes_present_flag: false,
            seq_enabled_motion_modes: [false; MOTION_MODES],
            enable_opfl_refine: 0,
            max_mlayer_id: 0,
            seq_force_screen_content_tools: 0,
            seq_force_integer_mv: 0,
            allow_frame_max_bvp_drl_bits: false,
            frame_width_bits: 12,
            frame_height_bits: 12,
            max_frame_width: 4096,
            max_frame_height: 2304,
            sb_size: SuperblockSize::Block128x128,
        }
    }

    fn inter_ctx() -> InterFrameContext {
        InterFrameContext {
            obu_type: ObuType::RegularTileGroup,
            frame_type: FrameType::Inter,
            is_bridge: false,
            bridge_frame_ref_idx: None,
            cur_mfh_id_is_zero: true,
            order_hint: 0,
        }
    }

    #[test]
    fn inter_explicit_map_parses_through_to_shared_tail() {
        let mut bits = Bits::default();
        bits.bit(1); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
        bits.f(2, 3); // primary_ref_frame
        bits.f(0b1010_1010, 8); // refresh_frame_flags f(NumRefFrames)
        bits.bit(1); // frame_explicit_ref_frame_map (seq explicit_ref_frame_map)
        bits.f(2, 3); // num_total_refs
        bits.f(3, 3); // ref_frame_idx[0]
        bits.f(5, 3); // ref_frame_idx[1]
        bits.bit(0); // use_ref_frame_mvs = 0
        bits.bit(0); // intrabc_params(): allow_intrabc = 0 (one bit)
        bits.bit(0); // use_qtr_precision_mv = 0
        bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
        bits.bit(0); // is_filter_switchable = 0
        bits.f(2, 2); // interpolation_filter = 2 (EIGHTTAP_SHARP)
        bits.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let seq = inter_seq();
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();

        assert_eq!(control.signal_primary_ref_frame, Some(true));
        assert_eq!(control.primary_ref_frame, Some(2));
        assert_eq!(control.refresh_frame_flags, Some(0b1010_1010));
        assert_eq!(control.explicit_ref_frame_map, Some(true));
        assert_eq!(control.num_total_refs, Some(2));
        assert_eq!(control.ref_frame_idx, vec![3, 5]);
        assert_eq!(control.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(control.use_ref_frame_mvs, Some(false));
        assert_eq!(control.tip_frame_mode, Some(TipFrameMode::Disabled));
        assert_eq!(control.allow_screen_content_tools, Some(false));
        assert_eq!(control.allow_intrabc, Some(false));
        assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
        assert_eq!(
            control.interpolation_filter,
            Some(InterpolationFilter::EighttapSharp)
        );
        assert_eq!(
            control.frame_enabled_motion_modes,
            Some([false; MOTION_MODES])
        );
        assert_eq!(control.disable_cdf_update, Some(false));
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    /// AV2 § 5.18.2 (mirror :5039-5043): on the ordinary inter / switch path (TipFrameMode
    /// disabled, `!bru_inactive`, `!IsBridge`), the control region reads `disable_cdf_update`
    /// f(1) — the `else` arm of `if ( TipFrameMode == TIP_FRAME_AS_OUTPUT || bru_inactive ||
    /// IsBridge )` — immediately before the shared-tail `tile_info()` (mirror :5183). On the
    /// `ReachedSharedTail` stop the parser must consume that bit and record it, so the shared
    /// tail starts at the exact bit position. Asserts the consumed-bit count and the recorded
    /// flag for both `disable_cdf_update` values. (Pre-fix the bit was not consumed: the
    /// consumed count was one short and `disable_cdf_update` stayed `None`, so the shared tail
    /// would have started one bit early.)
    fn reached_shared_tail_consumes_disable_cdf_update(disable_cdf_update_bit: u8) {
        const BITS_BEFORE_DISABLE_CDF_UPDATE: u64 = 30;
        let mut bits = Bits::default();
        bits.bit(1); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
        bits.f(2, 3); // primary_ref_frame
        bits.f(0b1010_1010, 8); // refresh_frame_flags f(NumRefFrames)
        bits.bit(1); // frame_explicit_ref_frame_map (seq explicit_ref_frame_map)
        bits.f(2, 3); // num_total_refs
        bits.f(3, 3); // ref_frame_idx[0]
        bits.f(5, 3); // ref_frame_idx[1]
        bits.bit(0); // use_ref_frame_mvs = 0
        bits.bit(0); // intrabc_params(): allow_intrabc = 0
        bits.bit(0); // use_qtr_precision_mv = 0
        bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
        bits.bit(0); // is_filter_switchable = 0
        bits.f(2, 2); // interpolation_filter = 2
        bits.bit(disable_cdf_update_bit); // disable_cdf_update f(1) (mirror :5041)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let seq = inter_seq();
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();

        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
        assert_eq!(
            control.disable_cdf_update,
            Some(disable_cdf_update_bit != 0)
        );
        assert_eq!(reader.consumed_bits(), BITS_BEFORE_DISABLE_CDF_UPDATE + 1);
    }

    #[test]
    fn reached_shared_tail_consumes_disable_cdf_update_zero() {
        reached_shared_tail_consumes_disable_cdf_update(0);
    }

    #[test]
    fn reached_shared_tail_consumes_disable_cdf_update_one() {
        reached_shared_tail_consumes_disable_cdf_update(1);
    }

    /// Builds the explicit-map inter prefix up to (and not including) the
    /// `frame_opfl_refine_type()` reads, with `num_total_refs == 1` so the TIP gate / tmvp
    /// reads stay absent. The caller appends the `opfl_refine_type` / `opfl_refine_all` bits.
    fn opfl_refine_prefix(bits: &mut Bits) {
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
        bits.f(0, 8); // refresh_frame_flags f(NumRefFrames)
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(1, 3); // num_total_refs = 1
        bits.f(0, 3); // ref_frame_idx[0]
        bits.bit(0); // use_ref_frame_mvs = 0 (num_total_refs == 1 -> no tmvp)
    }

    /// Appends the inter tail after `frame_opfl_refine_type()`: screen_content / intrabc /
    /// MV-precision / interpolation-filter, converging into the shared tail.
    fn opfl_refine_tail(bits: &mut Bits) {
        bits.bit(0); // intrabc_params(): allow_intrabc = 0
        bits.bit(0); // use_qtr_precision_mv = 0
        bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
        bits.bit(1); // is_filter_switchable = 1 (no interpolation_filter f(2))
        bits.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    }

    #[test]
    fn opfl_refine_auto_switchable_skips_opfl_refine_all() {
        let mut bits = Bits::default();
        opfl_refine_prefix(&mut bits);
        bits.bit(1); // opfl_refine_type = 1 (REFINE_SWITCHABLE) -> NO opfl_refine_all
        opfl_refine_tail(&mut bits);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.enable_opfl_refine = REFINE_AUTO;
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.tip_frame_mode, Some(TipFrameMode::Disabled));
        assert_eq!(control.allow_intrabc, Some(false));
        assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
        assert_eq!(
            control.interpolation_filter,
            Some(InterpolationFilter::Switchable)
        );
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    #[test]
    fn opfl_refine_auto_non_switchable_reads_opfl_refine_all() {
        let mut bits = Bits::default();
        opfl_refine_prefix(&mut bits);
        bits.bit(0); // opfl_refine_type = 0 (REFINE_NONE) -> opfl_refine_all IS read
        bits.bit(0); // opfl_refine_all f(1)
        opfl_refine_tail(&mut bits);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.enable_opfl_refine = REFINE_AUTO;
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.tip_frame_mode, Some(TipFrameMode::Disabled));
        assert_eq!(control.allow_intrabc, Some(false));
        assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
        assert_eq!(
            control.interpolation_filter,
            Some(InterpolationFilter::Switchable)
        );
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    #[test]
    fn inter_implicit_map_stops_unmodeled() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags f(8)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.explicit_ref_frame_map = false;
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.explicit_ref_frame_map, Some(false));
        assert_eq!(control.num_total_refs, None);
        assert_eq!(control.stop, Some(InterStop::UnmodeledDerivation));
    }

    /// AV2 § 7.7 — the implicit reference map (`!explicitRefFrameMap`) is now MODELED when the
    /// reference state proves exactly ONE valid slot: `get_ref_frames()` derives
    /// `NumTotalRefs == 1`, `ref_frame_idx == [theSlot]` (no bits), and the control region
    /// parses through to the shared tail. (The fixture-bytes end-to-end proof lives in
    /// info.rs::frame_header_core_inter_implicit_map_reaches_shared_tail_on_fixture.)
    #[test]
    fn inter_implicit_map_one_valid_slot_reaches_shared_tail() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags f(8)
        bits.bit(0); // use_ref_frame_mvs = 0
        bits.bit(0); // intrabc_params(): allow_intrabc = 0
        bits.bit(0); // use_qtr_precision_mv = 0
        bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
        bits.bit(1); // is_filter_switchable = 1
        bits.bit(0); // disable_cdf_update f(1), just before the shared tail
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.explicit_ref_frame_map = false;
        seq.max_frame_width = 64;
        seq.max_frame_height = 64;
        let ctx = inter_ctx();
        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[0] = true;
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        ref_w[0] = 64;
        ref_h[0] = 64;
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.explicit_ref_frame_map, Some(false));
        assert_eq!(control.num_total_refs, Some(1));
        assert_eq!(control.ref_frame_idx, vec![0]);
        assert_eq!(control.use_ref_frame_mvs, Some(false));
        assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
        assert_eq!(control.disable_cdf_update, Some(false));
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    /// AV2 § 7.7 — the implicit map with NO valid slot derives `NumTotalRefs == 0` and reaches
    /// the shared tail (`ref_frame_idx` empty); the gate admits the zero-reference case too.
    #[test]
    fn inter_implicit_map_no_valid_slot_reaches_shared_tail_with_zero_refs() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags f(8)
        bits.bit(0); // use_ref_frame_mvs = 0
        bits.bit(0); // allow_intrabc = 0
        bits.bit(0); // use_qtr_precision_mv = 0
        bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
        bits.bit(1); // is_filter_switchable = 1
        bits.bit(0); // disable_cdf_update f(1)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.explicit_ref_frame_map = false;
        let ctx = inter_ctx();
        let ref_valid = [false; NUM_REF_FRAMES];
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let ref_w = [0u32; NUM_REF_FRAMES];
        let ref_h = [0u32; NUM_REF_FRAMES];
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.num_total_refs, Some(0));
        assert!(control.ref_frame_idx.is_empty());
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    /// AV2 § 7.7 — the implicit map with TWO valid slots STAYS an honest UnmodeledDerivation
    /// stop (the § 7.7 ranking needs the unmodeled RefBaseQIdx / RefCounter / layer scoring).
    #[test]
    fn inter_implicit_map_two_valid_slots_stops_unmodeled() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags f(8)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.explicit_ref_frame_map = false;
        let ctx = inter_ctx();
        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[0] = true;
        ref_valid[2] = true;
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let ref_w = [64u32; NUM_REF_FRAMES];
        let ref_h = [64u32; NUM_REF_FRAMES];
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.num_total_refs, None);
        assert_eq!(control.stop, Some(InterStop::UnmodeledDerivation));
    }

    /// AV2 § 7.7 — with `RefBaseQIdx` modeled, the implicit map over TWO valid slots is
    /// derived EXACTLY (the multi-reference brick `DECODE-INTER-MULTIREF-RUNTIME`). This
    /// is the parser-side proof that `from_slots_with_base_q_idx` lifts the at-most-one
    /// gate: with order_hint 10, slot 0 (oh 8, q 40) and slot 1 (oh 5, q 40), both past
    /// references at equal dims, § 7.7 ranks slot 0 (score -248) before slot 1 (-220), so
    /// `NumTotalRefs == 2` and `ref_frame_idx == [0, 1]`, mirroring the
    /// `get_ref_frames::two_distinct_refs_rank_by_score` table-level worked example.
    #[test]
    fn inter_implicit_map_two_valid_slots_with_base_q_idx_ranks_exact() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
        bits.f(0, 8); // refresh_frame_flags f(NumRefFrames)
        bits.bit(0); // use_ref_frame_mvs = 0 (NumTotalRefs > 1, but mvs off -> no tmvp bits)
        bits.bit(0); // intrabc_params(): allow_intrabc = 0
        bits.bit(0); // use_qtr_precision_mv = 0
        bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
        bits.bit(0); // is_filter_switchable = 0
        bits.f(2, 2); // interpolation_filter = 2
        bits.bit(0); // disable_cdf_update f(1)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.explicit_ref_frame_map = false;
        let mut ctx = inter_ctx();
        ctx.order_hint = 10;
        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[0] = true;
        ref_valid[1] = true;
        let mut ref_oh = [0u32; NUM_REF_FRAMES];
        ref_oh[0] = 8;
        ref_oh[1] = 5;
        let ref_w = [4096u32; NUM_REF_FRAMES];
        let ref_h = [2304u32; NUM_REF_FRAMES];
        let mut ref_q = [0u32; NUM_REF_FRAMES];
        ref_q[0] = 40;
        ref_q[1] = 40;
        let rs = FrameReferenceStateView::from_slots_with_base_q_idx(
            &ref_valid, &ref_oh, &ref_w, &ref_h, &ref_q,
        );
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.num_total_refs, Some(2));
        assert_eq!(control.ref_frame_idx, vec![0, 1]);
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    /// AV2 § 7.7 — even with TWO valid slots, a `from_slots` view (no `RefBaseQIdx`) STAYS
    /// an honest `UnmodeledDerivation` stop: the `from_slots_with_base_q_idx` constructor is
    /// the ONLY way to lift the gate, so the historical callers are unaffected.
    #[test]
    fn inter_implicit_map_two_valid_slots_without_base_q_idx_stops_unmodeled() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags f(8)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.explicit_ref_frame_map = false;
        let mut ctx = inter_ctx();
        ctx.order_hint = 10;
        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[0] = true;
        ref_valid[1] = true;
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let ref_w = [64u32; NUM_REF_FRAMES];
        let ref_h = [64u32; NUM_REF_FRAMES];
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.num_total_refs, None);
        assert_eq!(control.stop, Some(InterStop::UnmodeledDerivation));
    }

    /// AV2 § 7.7 — INCOMPLETE ranking inputs over TWO valid slots STAY an honest
    /// `UnmodeledDerivation` stop even though `RefBaseQIdx` is present: a `RefOrderHint`
    /// slice SHORTER than the active reference-slot count cannot cover both valid slots, so
    /// `derive_implicit_ref_map` must NOT silently default the missing slot's order hint to
    /// zero and derive `ref_frame_idx` from fabricated state. This is the parser-side proof
    /// of the `DECODE-INTER-MULTIREF-RUNTIME` completeness gate.
    #[test]
    fn inter_implicit_map_two_valid_slots_incomplete_ranking_inputs_stop_unmodeled() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags f(8)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.explicit_ref_frame_map = false;
        let mut ctx = inter_ctx();
        ctx.order_hint = 10;
        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[0] = true;
        ref_valid[1] = true;
        let ref_oh_short = [8u32];
        let ref_w = [4096u32; NUM_REF_FRAMES];
        let ref_h = [2304u32; NUM_REF_FRAMES];
        let mut ref_q = [0u32; NUM_REF_FRAMES];
        ref_q[0] = 40;
        ref_q[1] = 40;
        let rs = FrameReferenceStateView::from_slots_with_base_q_idx(
            &ref_valid,
            &ref_oh_short,
            &ref_w,
            &ref_h,
            &ref_q,
        );
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.num_total_refs, None);
        assert_eq!(control.stop, Some(InterStop::UnmodeledDerivation));
    }

    #[test]
    fn inter_ref_idx_out_of_buffer_range_is_flagged() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 20); // refresh_frame_flags f(NumRefFrames == 20)
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(1, 3); // num_total_refs = 1
        bits.f(17, 5); // ref_frame_idx[0] = 17 (>= NUM_REF_FRAMES 16)
        bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // use_qtr_precision_mv
        bits.bit(0); // allow_high_precision_mv
        bits.bit(1); // is_filter_switchable
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.num_ref_frames = 20;
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert!(control.has_invalid_ref_frame_idx);
        assert_eq!(control.ref_frame_idx, vec![17]);
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    /// AV2 § 6.17.2 (mirror :4605-4606). With a non-power-of-two NumRefFrames the
    /// read width `CeilLog2(NumRefFrames)` over-covers the active slot range, so an encoded
    /// `ref_frame_idx[i]` can exceed `NumRefFrames - 1` while still fitting the buffer bound
    /// of 16. RefValid is defined only over `0..NumRefFrames-1`, so such an index is an
    /// unconditional violation. NumRefFrames == 6 -> 3-bit ref_frame_idx (values 0..7); an
    /// idx of 7 is >= 6 and must be flagged even though it is < NUM_REF_FRAMES (16).
    fn parse_ref_idx_with_num_ref_frames_6(idx: u32) -> InterControl {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 6); // refresh_frame_flags f(NumRefFrames == 6)
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(1, 3); // num_total_refs = 1
        bits.f(idx, 3); // ref_frame_idx[0] f(CeilLog2(6) == 3)
        bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // use_qtr_precision_mv
        bits.bit(0); // allow_high_precision_mv
        bits.bit(1); // is_filter_switchable
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.num_ref_frames = 6;
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap()
    }

    #[test]
    fn inter_ref_idx_beyond_active_num_ref_frames_is_flagged() {
        let control = parse_ref_idx_with_num_ref_frames_6(7);
        assert!(
            control.has_invalid_ref_frame_idx,
            "ref_frame_idx == 7 with NumRefFrames == 6 names a slot outside the active \
             buffer (RefValid undefined there); §6.17.2 requires it be flagged"
        );
        assert_eq!(control.ref_frame_idx, vec![7]);
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    #[test]
    fn inter_ref_idx_within_active_num_ref_frames_is_silent() {
        let control = parse_ref_idx_with_num_ref_frames_6(5);
        assert!(
            !control.has_invalid_ref_frame_idx,
            "ref_frame_idx == 5 with NumRefFrames == 6 is in the active range, so the \
             active-bound flag must not fire"
        );
        assert_eq!(control.ref_frame_idx, vec![5]);
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    #[test]
    fn switch_frame_infers_primary_ref_none_and_reads_explicit_map() {
        let mut bits = Bits::default();
        bits.f(0xFF, 8);
        bits.f(1, 3); // num_total_refs = 1
        bits.f(4, 3); // ref_frame_idx[0]
        bits.f(1920 - 1, 12);
        bits.f(1080 - 1, 12);
        bits.bit(0); // allow_intrabc
        bits.bit(0); // use_qtr_precision_mv
        bits.bit(0); // allow_high_precision_mv
        bits.bit(1); // is_filter_switchable
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let seq = inter_seq();
        let mut ctx = inter_ctx();
        ctx.frame_type = FrameType::Switch;
        ctx.obu_type = ObuType::Switch;
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, true).unwrap();
        assert_eq!(control.primary_ref_frame, Some(PRIMARY_REF_NONE));
        assert_eq!(control.explicit_ref_frame_map, Some(true));
        assert_eq!(control.num_total_refs, Some(1));
        assert_eq!(control.frame_size, Some(FrameSize::new(1920, 1080)));
        assert_eq!(control.use_ref_frame_mvs, Some(false));
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }

    #[test]
    fn frame_size_with_refs_copies_valid_ref_dims() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(1, 3); // num_total_refs = 1
        bits.f(2, 3); // ref_frame_idx[0] = 2
        bits.bit(1); // found_ref = 1 (frame_size_with_refs)
        bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // use_qtr_precision_mv
        bits.bit(0); // allow_high_precision_mv
        bits.bit(1); // is_filter_switchable
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let seq = inter_seq();
        let ctx = inter_ctx();
        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[2] = true;
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        ref_w[2] = 1280;
        ref_h[2] = 720;
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, true).unwrap();
        assert_eq!(control.frame_size, Some(FrameSize::new(1280, 720)));
    }

    #[test]
    fn frame_size_with_refs_continues_when_hit_slot_unknown() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(1, 3); // num_total_refs = 1
        bits.f(2, 3); // ref_frame_idx[0] = 2 (Unknown slot under the unknown() view)
        bits.bit(1); // found_ref = 1 -> hits the unmodeled slot 2 (size unknown)
        bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // use_qtr_precision_mv
        bits.bit(0); // allow_high_precision_mv
        bits.bit(1); // is_filter_switchable
        bits.bit(0); // disable_cdf_update f(1), just before the shared tail
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let seq = inter_seq();
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, true).unwrap();
        assert_eq!(
            control.frame_size, None,
            "the hit slot's dims are unmodeled, so the size is genuinely unknown"
        );
        assert_eq!(
            control.use_ref_frame_mvs,
            Some(false),
            "the presence-known field after the size hit is reached, not dropped (F4)"
        );
        assert_eq!(control.allow_intrabc, Some(false));
        assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
        assert_eq!(control.disable_cdf_update, Some(false));
        assert_eq!(
            control.stop,
            Some(InterStop::ReachedSharedTail),
            "with the bit position exact, the control region parses through to the shared tail"
        );
    }

    #[test]
    fn bru_triple_reads_and_inactive_returns() {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(2, 3); // num_total_refs = 2
        bits.f(0, 3); // ref_frame_idx[0]
        bits.f(1, 3); // ref_frame_idx[1]
        bits.bit(1); // use_bru
        bits.f(1, 1); // bru_ref f(CeilLog2(2)=1)
        bits.bit(1); // bru_inactive = 1
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.enable_bru = true;
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
        assert_eq!(control.use_bru, Some(true));
        assert_eq!(control.bru_ref, Some(1));
        assert_eq!(control.bru_inactive, Some(true));
        assert_eq!(control.use_ref_frame_mvs, Some(false));
        assert_eq!(control.stop, Some(InterStop::BruInactiveOrBridgeReturn));
    }

    #[test]
    fn tip_gate_stops_poisoned_on_tip_frame_mode() {
        for tip_output_obu in [false, true] {
            let mut bits = Bits::default();
            let mut fields = vec![(0, 1)]; // signal_primary_ref_frame
            if !tip_output_obu {
                fields.push((0, 1)); // disable_cross_frame_cdf_init (uncoded for TIP OBUs)
            }
            fields.extend([
                (0, 8), // refresh_frame_flags
                (1, 1), // frame_explicit_ref_frame_map
                (2, 3), // num_total_refs = 2
                (0, 3), // ref_frame_idx[0]
                (1, 3), // ref_frame_idx[1]
                (1, 1), // use_ref_frame_mvs = 1
                (0, 1), // tmvp_sample_step_minus_1 (num_total_refs>1, sb 128 != 64x64)
            ]);
            if !tip_output_obu {
                fields.push((1, 1)); // tip_frame_mode = 1 (uncoded for TIP OBUs: AS_OUTPUT)
            }
            for (value, width) in fields {
                bits.f(value, width);
            }
            let data = bits.into_bytes();
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let mut seq = inter_seq();
            seq.enable_tip = true;
            let mut ctx = inter_ctx();
            if tip_output_obu {
                ctx.obu_type = ObuType::RegularTip;
            }
            let rs = FrameReferenceStateView::unknown();
            let control = parse_inter_control(&mut reader, &seq, &ctx, &rs, false).unwrap();
            assert_eq!(
                (
                    control.use_ref_frame_mvs,
                    control.tmvp_sample_step_minus_1,
                    control.tip_frame_mode,
                    control.stop,
                ),
                (
                    Some(true),
                    Some(false),
                    None,
                    Some(InterStop::PoisonedReferenceState),
                ),
                "tip_output_obu={tip_output_obu}"
            );
        }
    }

    #[test]
    fn inter_eof_before_primary_ref_is_error() {
        let data: [u8; 0] = [];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let seq = inter_seq();
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        assert!(parse_inter_control(&mut reader, &seq, &ctx, &rs, false).is_err());
    }

    /// AV2 § 7.7 + § 5.18.2 — drive the REAL `syn-key-inter-64x64` fixture's inter frame
    /// through `parse_frame_header_core` with the post-key reference state, and prove the
    /// implicit reference map (`get_ref_frames()`) now advances the parser PAST
    /// `InterStop::UnmodeledDerivation` to the shared tail.
    ///
    /// The fixture is a `OBU_CLOSED_LOOP_KEY` key (`refresh_frame_flags == 255`,
    /// `base_q_idx == 70`, 64x64) + an `OBU_REGULAR_TILE_GROUP` inter frame
    /// (`order_hint_lsb == 1`, `explicitRefFrameMap == 0`, `primary_ref_frame ==
    /// PRIMARY_REF_CHOOSE`). After the key frame, the § 7.23 `first` rule (mirror :14132,
    /// `(KEY) ? first : 1`) leaves ONLY slot 0 `RefValid`, with `RefOrderHint[0] == 0` and
    /// `RefFrameWidth/Height[0] == 64` — exactly the at-most-one-valid-reference case the
    /// modeled `get_ref_frames()` resolves. The bit-level proof is that the parser consumes
    /// the SAME inter-header field sequence the explicit map would (frame size / BRU /
    /// MV-precision / interpolation filter / motion modes / disable_cdf_update) and reaches
    /// `InterStop::ReachedSharedTail` — possible only if the implicit map yielded
    /// `NumTotalRefs == 1`, `ref_frame_idx == [0]` and the parse continued from the right bit.
    #[test]
    fn frame_header_core_inter_implicit_map_reaches_shared_tail_on_fixture() {
        use crate::headers::frame::info::{
            FrameHeaderParseInput, FrameHeaderParseMode, parse_frame_header_core,
        };
        use crate::headers::sequence::SequenceHeader;
        use crate::obu::{ParsedObu, PayloadStatus};
        use crate::stream::{ParsedBitstream, parse_bitstream_partial};

        let data = include_bytes!(
            "../../../../../tests/conformance/vectors/valid/syn-key-inter-64x64.ivf"
        );
        let parsed = parse_bitstream_partial(data);
        let ParsedBitstream::Ivf(ivf) = parsed else {
            panic!("fixture is an IVF container");
        };

        // Collect every OBU across the IVF frame payloads, in stream order.
        let mut seq_header: Option<SequenceHeader> = None;
        let mut inter_obu: Option<crate::annexb::ObuEnvelope<'_>> = None;
        for frame in &ivf.frames {
            for obu in &frame.obus {
                match obu.header.obu_type {
                    ObuType::SequenceHeader => {
                        if let Ok(PayloadStatus::Parsed(ParsedObu::SequenceHeader(sh))) =
                            obu.payload_status()
                        {
                            seq_header = Some(*sh);
                        }
                    }
                    ObuType::RegularTileGroup if inter_obu.is_none() => {
                        inter_obu = Some(*obu);
                    }
                    _ => {}
                }
            }
        }
        let seq_header = seq_header.expect("fixture has a sequence header");
        let inter_obu = inter_obu.expect("fixture has a regular tile group (inter) frame");

        let mut ref_valid = [false; NUM_REF_FRAMES];
        let mut ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        ref_valid[0] = true;
        ref_oh[0] = 0;
        ref_w[0] = 64;
        ref_h[0] = 64;
        let reference_state =
            FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

        let mut reader = BitReader::new(inter_obu.payload, inter_obu.payload_offset());
        let input = FrameHeaderParseInput {
            obu_type: inter_obu.header.obu_type,
            first_picture_in_tu: false,
            active_sequence: Some(&seq_header),
            mfh_record: None,
            reference_state,
            mode: FrameHeaderParseMode::Core,
        };
        let core = parse_frame_header_core(&mut reader, &input).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::Inter));
        let inter = core
            .inter
            .as_ref()
            .expect("the inter control region was parsed");
        assert_eq!(
            inter.explicit_ref_frame_map,
            Some(false),
            "the fixture uses the IMPLICIT reference map"
        );
        assert_eq!(inter.num_total_refs, Some(1));
        assert_eq!(inter.ref_frame_idx, vec![0]);
        assert_eq!(
            inter.stop,
            Some(InterStop::ReachedSharedTail),
            "with get_ref_frames() modeled, the implicit-map inter frame reaches the shared tail"
        );
        assert!(!core.status.is_truncated_in_modeled_region());
    }

    /// AV2 § 7.7 — the implicit map STAYS an honest `UnmodeledDerivation` stop when the
    /// modeled reference state proves TWO valid slots (the § 7.7 ranking then needs the
    /// unmodeled `RefBaseQIdx` / `RefCounter` / layer scoring inputs). Same fixture inter
    /// bytes, but a reference state with two valid slots: the gate refuses to guess.
    #[test]
    fn frame_header_core_inter_implicit_map_two_valid_slots_stops_unmodeled() {
        use crate::headers::frame::info::{
            FrameHeaderParseInput, FrameHeaderParseMode, parse_frame_header_core,
        };
        use crate::headers::sequence::SequenceHeader;
        use crate::obu::{ParsedObu, PayloadStatus};
        use crate::stream::{ParsedBitstream, parse_bitstream_partial};

        let data = include_bytes!(
            "../../../../../tests/conformance/vectors/valid/syn-key-inter-64x64.ivf"
        );
        let ParsedBitstream::Ivf(ivf) = parse_bitstream_partial(data) else {
            panic!("fixture is an IVF container");
        };
        let mut seq_header: Option<SequenceHeader> = None;
        let mut inter_obu: Option<crate::annexb::ObuEnvelope<'_>> = None;
        for frame in &ivf.frames {
            for obu in &frame.obus {
                match obu.header.obu_type {
                    ObuType::SequenceHeader => {
                        if let Ok(PayloadStatus::Parsed(ParsedObu::SequenceHeader(sh))) =
                            obu.payload_status()
                        {
                            seq_header = Some(*sh);
                        }
                    }
                    ObuType::RegularTileGroup if inter_obu.is_none() => {
                        inter_obu = Some(*obu);
                    }
                    _ => {}
                }
            }
        }
        let seq_header = seq_header.unwrap();
        let inter_obu = inter_obu.unwrap();

        // Two valid slots: the § 7.7 ranking now depends on the unmodeled scoring inputs.
        let mut ref_valid = [false; NUM_REF_FRAMES];
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        ref_valid[0] = true;
        ref_valid[1] = true;
        ref_w[0] = 64;
        ref_h[0] = 64;
        ref_w[1] = 64;
        ref_h[1] = 64;
        let reference_state =
            FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

        let mut reader = BitReader::new(inter_obu.payload, inter_obu.payload_offset());
        let input = FrameHeaderParseInput {
            obu_type: inter_obu.header.obu_type,
            first_picture_in_tu: false,
            active_sequence: Some(&seq_header),
            mfh_record: None,
            reference_state,
            mode: FrameHeaderParseMode::Core,
        };
        let core = parse_frame_header_core(&mut reader, &input).unwrap();
        let inter = core.inter.as_ref().unwrap();
        assert_eq!(inter.explicit_ref_frame_map, Some(false));
        assert_eq!(
            inter.stop,
            Some(InterStop::UnmodeledDerivation),
            "two valid slots need the unmodeled § 7.7 scoring inputs — no guessing"
        );
    }

    /// AV2 § 5.18.2 — drive the REAL `syn-2frame-inter-64x64` fixture's inter frame through
    /// `parse_frame_header_core` with the post-key reference state, and prove the inter
    /// frame header now parses END-TO-END through its § 5.18.2 shared tail and inter-specific
    /// arms, reaching the terminal `FrameHeaderParseStatus::InterHeaderComplete`.
    ///
    /// The fixture (`--enable-global-motion=1 --qp=80 --sb-size=64`) is a `OBU_CLOSED_LOOP_KEY`
    /// key + an `OBU_REGULAR_TILE_GROUP` inter frame: a single 64x64 zero-MV skip block,
    /// `explicitRefFrameMap == 0`, `NumTotalRefs == 1`, `TipFrameMode == TIP_FRAME_DISABLED`,
    /// `!IsBridge`, `!bru_inactive`, broad tools off, exactly the minimal subset the shared
    /// tail models. The asserted values are derived from the § 5.18.2 spec + the fixture
    /// sequence config (the hand-decoded 56-bit inter payload: 27-bit control region + 21-bit
    /// shared tail + 8-bit § 5.19 tile-group tail) and confirmed against the bit-exact
    /// avmdec/dav2d decode the fixture records. The bit-level proof is that the parse consumes
    /// EXACTLY the shared-tail field sequence and reaches `InterHeaderComplete` — possible only
    /// if every field width / presence (tile_info / quant / segmentation / deblocking's inter
    /// allow_df_sub_pu arm / the inter coding-mode arms / global_motion / film_grain) was read
    /// at the right bit.
    #[test]
    fn frame_header_core_inter_fixture_reaches_inter_header_complete() {
        use crate::headers::frame::info::{
            FrameHeaderParseInput, FrameHeaderParseMode, FrameHeaderParseStatus,
            parse_frame_header_core,
        };
        use crate::headers::frame::tail::TxMode;
        use crate::headers::sequence::SequenceHeader;
        use crate::obu::{ParsedObu, PayloadStatus};
        use crate::stream::{ParsedBitstream, parse_bitstream_partial};

        let data = include_bytes!(
            "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf"
        );
        let ParsedBitstream::Ivf(ivf) = parse_bitstream_partial(data) else {
            panic!("fixture is an IVF container");
        };
        let mut seq_header: Option<SequenceHeader> = None;
        let mut inter_obu: Option<crate::annexb::ObuEnvelope<'_>> = None;
        for frame in &ivf.frames {
            for obu in &frame.obus {
                match obu.header.obu_type {
                    ObuType::SequenceHeader => {
                        if let Ok(PayloadStatus::Parsed(ParsedObu::SequenceHeader(sh))) =
                            obu.payload_status()
                        {
                            seq_header = Some(*sh);
                        }
                    }
                    ObuType::RegularTileGroup if inter_obu.is_none() => {
                        inter_obu = Some(*obu);
                    }
                    _ => {}
                }
            }
        }
        let seq_header = seq_header.expect("fixture has a sequence header");
        let inter_obu = inter_obu.expect("fixture has a regular tile group (inter) frame");

        let mut ref_valid = [false; NUM_REF_FRAMES];
        let mut ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        ref_valid[0] = true;
        ref_oh[0] = 0;
        ref_w[0] = 64;
        ref_h[0] = 64;
        let reference_state =
            FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

        let mut reader = BitReader::new(inter_obu.payload, inter_obu.payload_offset());
        let input = FrameHeaderParseInput {
            obu_type: inter_obu.header.obu_type,
            first_picture_in_tu: false,
            active_sequence: Some(&seq_header),
            mfh_record: None,
            reference_state,
            mode: FrameHeaderParseMode::Core,
        };
        let core = parse_frame_header_core(&mut reader, &input).unwrap();

        assert_eq!(
            core.status,
            FrameHeaderParseStatus::InterHeaderComplete,
            "the minimal-subset inter frame parses its whole § 5.18.2 shared tail"
        );
        assert!(
            !core.status.is_truncated_in_modeled_region(),
            "a complete header is not a truncation"
        );
        assert_eq!(core.frame_type, Some(FrameType::Inter));

        let inter = core.inter.as_ref().expect("inter control region parsed");
        assert_eq!(inter.stop, Some(InterStop::ReachedSharedTail));
        assert_eq!(inter.explicit_ref_frame_map, Some(false));
        assert_eq!(inter.num_total_refs, Some(1));
        assert_eq!(inter.ref_frame_idx, vec![0]);

        let tile_info = core.tile_info.as_ref().expect("tile_info parsed");
        assert_eq!(
            tile_info.tile_cols, 1,
            "single 64x64 superblock -> one tile"
        );
        assert_eq!(tile_info.tile_rows, 1);
        let quant = core
            .quantization_params
            .as_ref()
            .expect("quantization_params parsed");
        assert_eq!(
            quant.base_q_idx, 119,
            "base_q_idx f(8) for the qp80 inter frame"
        );
        let seg = core
            .segmentation_params
            .as_ref()
            .expect("segmentation parsed");
        assert!(
            !seg.segmentation_enabled,
            "the minimal fixture disables segmentation"
        );
        let deblocking = core
            .deblocking_filter_params
            .as_ref()
            .expect("deblocking parsed");
        assert_eq!(
            deblocking.apply_deblocking_filter, [false; 4],
            "deblocking is off for the flat copy frame"
        );

        let tail = core.inter_tail.as_ref().expect("inter tail parsed");
        assert_eq!(tail.tx_mode, TxMode::Largest, "tx_mode_select == 0");
        assert!(!tail.reference_select, "single-reference frame");
        assert!(!tail.skip_mode_present);
        assert!(!tail.allow_bawp, "enable_bawp off");
        assert!(!tail.allow_warpmv_mode, "no DELTAWARP motion mode enabled");
        assert_eq!(tail.reduced_tx_set, 0);
        assert!(
            !tail.use_global_motion,
            "global motion enabled in the sequence but unused by this frame"
        );
        assert!(!tail.apply_grain, "no film grain applied");
    }
}
