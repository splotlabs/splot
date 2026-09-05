// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.18.9 global-motion parameters and subexponential decoding
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-9`).
//!
//! [`GlobalMotionReferenceState`] supplies the retained warp predictors and order hints.
//! When omitted, [`GlobalMotionStop`] records the first state-dependent boundary.
//! With reference state present, restricted order hints produce typed conformance errors.

use crate::bitio::BitReader;
use crate::error::{GlobalMotionErrorKind, Result};

use super::get_ref_frames::{RESTRICTED_OH, get_relative_dist};
use super::info::FrameType;

/// `WARPEDMODEL_PREC_BITS` (AV2 v1.0.0 § 3): internal precision of warped motion models.
/// The fractional precision every `gm_params` value is stored with (mirror :8002 derives
/// `precDiff` from it).
pub(crate) const WARPEDMODEL_PREC_BITS: u32 = 16;

/// `GM_ALPHA_PREC_BITS` (AV2 v1.0.0 § 3): fractional bits for the non-translational
/// (`idx >= 2`) warp coefficients (mirror :7990).
const GM_ALPHA_PREC_BITS: u32 = 10;

/// `GM_TRANS_PREC_BITS` (AV2 v1.0.0 § 3): fractional bits for the translational
/// (`idx < 2`) warp coefficients (mirror :7996).
const GM_TRANS_PREC_BITS: u32 = 3;

/// `GM_ABS_ALPHA_BITS` (AV2 v1.0.0 § 3): bits encoded for non-translational components.
const GM_ABS_ALPHA_BITS: u32 = 9;

/// `GM_ABS_TRANS_BITS` (AV2 v1.0.0 § 3): bits encoded for translational components.
const GM_ABS_TRANS_BITS: u32 = 14;

/// `GM_ALPHA_MAX = (1 << GM_ABS_ALPHA_BITS) - 1` (AV2 v1.0.0 § 3): the `mx` bound for
/// `idx >= 2` (mirror :7992).
const GM_ALPHA_MAX: i32 = (1i32 << GM_ABS_ALPHA_BITS) - 1;

/// `GM_TRANS_MAX = (1 << GM_ABS_TRANS_BITS) - 1` (AV2 v1.0.0 § 3): the `mx` bound for
/// `idx < 2` (mirror :7998).
const GM_TRANS_MAX: i32 = (1i32 << GM_ABS_TRANS_BITS) - 1;

/// The fixed subexp parameter `k = 3` used by every `read_global_param()` call (mirror
/// :8012, `decode_signed_subexp_with_ref( -mx, mx + 1, r, 3 )`).
const GLOBAL_PARAM_K: u32 = 3;

/// `IDENTITY` (AV2 v1.0.0 § 3): warp model is the identity transform (`GmType` value `0`).
const IDENTITY: u8 = 0;

/// `ROTZOOM` (AV2 v1.0.0 § 3): rotation + symmetric zoom warp model (`GmType` value `1`).
const ROTZOOM: u8 = 1;

/// `AFFINE` (AV2 v1.0.0 § 3): general affine warp model (`GmType` value `2`).
const AFFINE: u8 = 2;

/// `REFS_PER_FRAME` (AV2 v1.0.0 § 3): the per-frame `GmType` / `gm_params` table size
/// (mirror :7778). Every reference is initialised to the identity warp before the inter
/// arm overwrites the active `0..NumTotalRefs` entries.
pub(crate) const REFS_PER_FRAME: usize = 7;

/// One frame's § 7.23 `SavedGmParams[slot]` table.
pub type SavedGlobalMotionParams = [[i32; 6]; REFS_PER_FRAME];

/// One frame's § 7.23 `SavedOrderHints[slot]` table. `u32::MAX` represents
/// `RESTRICTED_OH`, matching the rest of the frame-header reference-state API.
pub type SavedGlobalMotionOrderHints = [u32; REFS_PER_FRAME];

/// Cross-frame § 7.23 state consumed by the inter `global_motion_params()` arm.
#[derive(Debug, Clone, Copy)]
pub struct GlobalMotionReferenceState<'a> {
    /// Current decoded `OrderHint`.
    pub order_hint: u32,
    /// `RefOrderHint[slot]`; `u32::MAX` represents `RESTRICTED_OH`.
    pub ref_order_hint: &'a [u32],
    /// `RefNumTotalRefs[slot]`.
    pub ref_num_total_refs: &'a [u32],
    /// `SavedOrderHints[slot]`.
    pub saved_order_hints: &'a [SavedGlobalMotionOrderHints],
    /// `SavedGmParams[slot]`.
    pub saved_gm_params: &'a [SavedGlobalMotionParams],
}

/// `Default_Warp_Params[6]` (AV2 v1.0.0 § 7, mirror :4702): the identity warp model. Index
/// `i % 3 == 2` (the diagonal scale terms 2 and 5) is `1 << WARPEDMODEL_PREC_BITS`; every
/// other term is `0` (mirror :7784-7786 derives the same identity initialiser).
const DEFAULT_WARP_PARAMS: [i32; 6] = [
    0,
    0,
    1 << WARPEDMODEL_PREC_BITS,
    0,
    0,
    1 << WARPEDMODEL_PREC_BITS,
];

/// The warp model kind selected for one reference by the § 5.18.9.1 per-reference loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GmType {
    /// `IDENTITY` — the reference has no signalled warp (loop gate skipped it, or
    /// `is_global == 0`). The `gm_params` are the identity warp.
    Identity,
    /// `ROTZOOM` — rotation + symmetric zoom (`is_global == 1 && is_rot_zoom == 1`). Only
    /// `gm_params[2]` / `gm_params[3]` are read; `[4]`/`[5]` are derived, `[0]`/`[1]` read.
    RotZoom,
    /// `AFFINE` — general affine (`is_global == 1 && is_rot_zoom == 0`). All six
    /// `gm_params` are read (`[2..6]` then `[0]`/`[1]`).
    Affine,
}

impl GmType {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::RotZoom => "rotzoom",
            Self::Affine => "affine",
        }
    }

    /// The numeric `GmType` value (AV2 § 3): `IDENTITY = 0`, `ROTZOOM = 1`, `AFFINE = 2`.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::Identity => IDENTITY,
            Self::RotZoom => ROTZOOM,
            Self::Affine => AFFINE,
        }
    }
}

/// Why the § 5.18.9.1 inter global-motion parse stopped before reading every reference's
/// warp model — a **coverage** stop, never a truncation. Both variants name a cross-frame
/// fact the model does not carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GlobalMotionStop {
    /// The base-warp `our_ref != NumTotalRefs` arm (mirror :7826-7846) needs
    /// `RefNumTotalRefs[ ref_frame_idx[ our_ref ] ]` (the referenced frame's reference
    /// count) to test `> 0` and to size the `their_ref ns(...)` read, plus the referenced
    /// frame's `SavedGmParams` / `SavedOrderHints`. None of that per-slot saved state is
    /// modeled, and the next read's width depends on it, so the parse stops here. The
    /// facts parsed before it (`use_global_motion`, `our_ref`) are preserved.
    RefNumTotalRefsUnmodeled,
    /// The per-reference warp loop (mirror :7853-7857) reads warp bits for a reference only
    /// when `dist != 0 && OrderHints[ ref ] != RESTRICTED_OH`, and `OrderHint` /
    /// `OrderHints[ ref ]` are cross-frame order-hint state. The *presence* of the first
    /// per-reference bit therefore depends on unmodeled state, so the parse stops at the
    /// loop boundary with `use_global_motion` and the base selection preserved.
    OrderHintsUnmodeled,
}

impl GlobalMotionStop {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RefNumTotalRefsUnmodeled => "ref_num_total_refs_unmodeled",
            Self::OrderHintsUnmodeled => "order_hints_unmodeled",
        }
    }
}

/// The parsed § 5.18.9.1 inter global-motion state. Every field exactly determined by the
/// reached bits is `Some`; a [`GlobalMotionStop`] records where the cross-frame boundary
/// halted the parse.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GlobalMotionParams {
    /// `use_global_motion` (mirror :7798). `false` is recorded both for an explicit
    /// `use_global_motion == 0` (whole structure returns) and is never inferred otherwise.
    pub use_global_motion: bool,
    /// `our_ref` (mirror :7816-7822): the base-warp reference selector. `NumTotalRefs` for
    /// a `SWITCH_FRAME` (inferred, no bits), otherwise the `ns(NumTotalRefs + 1)` value.
    /// `None` when `use_global_motion == 0` (the structure returned before this).
    pub our_ref: Option<u32>,
    /// Where the parse stopped at the cross-frame boundary, when it did. `None` means the
    /// structure returned cleanly (intra/disabled return or `use_global_motion == 0`).
    pub stop: Option<GlobalMotionStop>,
    /// The per-reference warp models, indexed `0..REFS_PER_FRAME`. Populated only when the
    /// per-reference loop is reached (which requires the cross-frame `OrderHints` state);
    /// at the honest stop every entry is the identity initialiser (mirror :7780-7788).
    pub references: [GlobalMotionRef; REFS_PER_FRAME],
}

/// One reference's parsed warp model (AV2 § 5.18.9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GlobalMotionRef {
    /// `GmType[ ref ]`.
    pub gm_type: GmType,
    /// `gm_params[ ref ][ 0..6 ]` (the six warp coefficients at `WARPEDMODEL_PREC_BITS`
    /// fractional precision). The identity initialiser until the reference signals a warp.
    pub gm_params: [i32; 6],
}

impl GlobalMotionRef {
    /// The identity warp model (mirror :7780-7788): `GmType = IDENTITY` and the
    /// `Default_Warp_Params` identity coefficients (mirror :4702).
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            gm_type: GmType::Identity,
            gm_params: DEFAULT_WARP_PARAMS,
        }
    }
}

/// Inputs the § 5.18.9.1 inter arm consumes from the already-parsed frame / sequence state
/// (AV2 v1.0.0 § 5.18.9.1). All are known to the core parser when the shared tail is
/// reached: the derived `FrameType`, `FrameIsIntra`, the sequence `enable_global_motion`
/// (§ 5.4.6), and the inter control region's `NumTotalRefs` / `ref_frame_idx`.
#[derive(Debug, Clone, Copy)]
pub struct GlobalMotionInput<'a> {
    /// `FrameIsIntra` — the structure returns immediately when set (mirror :7792).
    pub frame_is_intra: bool,
    /// The derived `FrameType` (selects the `SWITCH_FRAME` `our_ref` inference, mirror
    /// :7814).
    pub frame_type: FrameType,
    /// `enable_global_motion` (§ 5.4.6) — the structure returns immediately when clear
    /// (mirror :7792).
    pub enable_global_motion: bool,
    /// `NumTotalRefs` (the inter control region's reference count): the `our_ref`
    /// `ns(NumTotalRefs + 1)` range and the per-reference loop bound (mirror :7820/:7853).
    pub num_total_refs: u32,
    /// `ref_frame_idx[ 0..NumTotalRefs ]` (the inter control region's reference map): the
    /// base-warp arm reads `ref_frame_idx[ our_ref ]` (mirror :7828).
    pub ref_frame_idx: &'a [u32],
    /// Cross-frame state needed once `use_global_motion == 1`. `None` preserves an honest
    /// coverage stop for inspectors that do not model the § 7.23 reference buffer.
    pub reference_state: Option<GlobalMotionReferenceState<'a>>,
}

/// Parses the § 5.18.9.1 `global_motion_params()` inter arm
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-9`, mirror :7776-7931).
///
/// Returns a [`GlobalMotionParams`] carrying every exactly-determined field plus the
/// terminal [`GlobalMotionStop`] (if the cross-frame boundary was reached). On the
/// intra / disabled return (`FrameIsIntra || !enable_global_motion`) or an explicit
/// `use_global_motion == 0`, the result is the identity warp with no stop.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload ends
/// before a mandated field (`use_global_motion`, `our_ref`) can be read. A cross-frame
/// boundary stops with `Ok` and a [`GlobalMotionStop`], never an error and never a guess.
pub fn parse_global_motion_params(
    reader: &mut BitReader<'_>,
    input: &GlobalMotionInput<'_>,
) -> Result<GlobalMotionParams> {
    let mut references = [GlobalMotionRef::identity(); REFS_PER_FRAME];

    if input.frame_is_intra || !input.enable_global_motion || !reader.read_flag()? {
        return Ok(GlobalMotionParams {
            use_global_motion: false,
            our_ref: None,
            stop: None,
            references,
        });
    }

    let our_ref = if input.frame_type == FrameType::Switch {
        input.num_total_refs
    } else {
        let n = input.num_total_refs.saturating_add(1);
        reader.read_ns(n)?
    };

    let count = usize::try_from(input.num_total_refs).unwrap_or(usize::MAX);
    if count > REFS_PER_FRAME || input.ref_frame_idx.len() < count {
        return Err(invalid_global_motion(
            reader,
            GlobalMotionErrorKind::ReferenceCountOutOfRange,
        ));
    }

    if count == 0 {
        return Ok(GlobalMotionParams {
            use_global_motion: true,
            our_ref: Some(our_ref),
            stop: None,
            references,
        });
    }

    let Some(state) = input.reference_state else {
        return Ok(GlobalMotionParams {
            use_global_motion: true,
            our_ref: Some(our_ref),
            stop: Some(if our_ref == input.num_total_refs {
                GlobalMotionStop::OrderHintsUnmodeled
            } else {
                GlobalMotionStop::RefNumTotalRefsUnmodeled
            }),
            references,
        });
    };

    let mut base_params = DEFAULT_WARP_PARAMS;
    let mut base_distance = 1;

    if our_ref != input.num_total_refs {
        let our_ref = usize::try_from(our_ref).map_err(|_| {
            invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceCountOutOfRange)
        })?;
        let ref_idx = mapped_reference_slot(reader, input.ref_frame_idx, our_ref, state)?;
        let our_hint = reference_order_hint(reader, ref_idx, state)?;
        if our_hint == RESTRICTED_OH {
            return Err(invalid_global_motion(
                reader,
                GlobalMotionErrorKind::OurReferenceRestricted,
            ));
        }
        let saved_count = *state.ref_num_total_refs.get(ref_idx).ok_or_else(|| {
            invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceSlotOutOfRange)
        })?;
        if usize::try_from(saved_count).unwrap_or(usize::MAX) > REFS_PER_FRAME {
            return Err(invalid_global_motion(
                reader,
                GlobalMotionErrorKind::ReferenceCountOutOfRange,
            ));
        }
        if saved_count > 0 {
            let their_ref = reader.read_ns(saved_count)? as usize;
            let saved_params = state.saved_gm_params.get(ref_idx).ok_or_else(|| {
                invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceSlotOutOfRange)
            })?;
            let saved_hints = state.saved_order_hints.get(ref_idx).ok_or_else(|| {
                invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceSlotOutOfRange)
            })?;
            base_params = *saved_params.get(their_ref).ok_or_else(|| {
                invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceCountOutOfRange)
            })?;
            let saved_hint = order_hint_to_spec(
                reader,
                *saved_hints.get(their_ref).ok_or_else(|| {
                    invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceCountOutOfRange)
                })?,
            )?;
            if saved_hint == RESTRICTED_OH {
                return Err(invalid_global_motion(
                    reader,
                    GlobalMotionErrorKind::SavedReferenceRestricted,
                ));
            }
            base_distance = get_relative_dist(our_hint, saved_hint);
        }
    }

    let current_order_hint = order_hint_to_spec(reader, state.order_hint)?;
    for (reference, &slot) in references
        .iter_mut()
        .zip(input.ref_frame_idx.iter())
        .take(count)
    {
        let slot = usize::try_from(slot).map_err(|_| {
            invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceSlotOutOfRange)
        })?;
        let order_hint = reference_order_hint(reader, slot, state)?;
        let dist = get_relative_dist(current_order_hint, order_hint);
        if dist == 0 || order_hint == RESTRICTED_OH {
            continue;
        }

        let previous = scale_warp_model(base_params, base_distance, dist);
        let gm_type = if !reader.read_flag()? {
            GmType::Identity
        } else if reader.read_flag()? {
            GmType::RotZoom
        } else {
            GmType::Affine
        };
        reference.gm_type = gm_type;
        if gm_type == GmType::Identity {
            continue;
        }

        reference.gm_params[2] = read_global_param(reader, 2, previous[2])?;
        reference.gm_params[3] = read_global_param(reader, 3, previous[3])?;
        if gm_type == GmType::Affine {
            reference.gm_params[4] = read_global_param(reader, 4, previous[4])?;
            reference.gm_params[5] = read_global_param(reader, 5, previous[5])?;
        } else {
            reference.gm_params[4] = -reference.gm_params[3];
            reference.gm_params[5] = reference.gm_params[2];
        }
        reference.gm_params[0] = read_global_param(reader, 0, previous[0])?;
        reference.gm_params[1] = read_global_param(reader, 1, previous[1])?;
    }

    Ok(GlobalMotionParams {
        use_global_motion: true,
        our_ref: Some(our_ref),
        stop: None,
        references,
    })
}

fn mapped_reference_slot(
    reader: &BitReader<'_>,
    ref_frame_idx: &[u32],
    logical_ref: usize,
    state: GlobalMotionReferenceState<'_>,
) -> Result<usize> {
    let slot = *ref_frame_idx.get(logical_ref).ok_or_else(|| {
        invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceCountOutOfRange)
    })?;
    let slot = usize::try_from(slot).map_err(|_| {
        invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceSlotOutOfRange)
    })?;
    if slot >= state.ref_order_hint.len() {
        return Err(invalid_global_motion(
            reader,
            GlobalMotionErrorKind::ReferenceSlotOutOfRange,
        ));
    }
    Ok(slot)
}

fn reference_order_hint(
    reader: &BitReader<'_>,
    slot: usize,
    state: GlobalMotionReferenceState<'_>,
) -> Result<i32> {
    let raw = *state.ref_order_hint.get(slot).ok_or_else(|| {
        invalid_global_motion(reader, GlobalMotionErrorKind::ReferenceSlotOutOfRange)
    })?;
    order_hint_to_spec(reader, raw)
}

fn order_hint_to_spec(reader: &BitReader<'_>, raw: u32) -> Result<i32> {
    if raw == u32::MAX {
        Ok(RESTRICTED_OH)
    } else {
        i32::try_from(raw)
            .map_err(|_| invalid_global_motion(reader, GlobalMotionErrorKind::OrderHintOutOfRange))
    }
}

/// `scale_warp_model()` from AV2 § 5.18.9.1.
#[must_use]
pub fn scale_warp_model(
    base_params: [i32; 6],
    mut base_distance: i32,
    mut distance: i32,
) -> [i32; 6] {
    if base_distance == 0 {
        return DEFAULT_WARP_PARAMS;
    }
    if base_distance < 0 {
        base_distance = -base_distance;
        distance = -distance;
    }
    let (div_shift, div_factor) = resolve_positive_divisor(base_distance as u32);
    let mut params = DEFAULT_WARP_PARAMS;
    for index in 0..6 {
        let center = DEFAULT_WARP_PARAMS[index];
        let input = (i64::from(base_params[index]) - i64::from(center))
            .clamp(-(1 << 22) + 1, (1 << 22) - 1);
        let scaled = round2_signed(input * div_factor, div_shift);
        let shift = if index < 2 {
            WARPEDMODEL_PREC_BITS - GM_TRANS_PREC_BITS
        } else {
            WARPEDMODEL_PREC_BITS - GM_ALPHA_PREC_BITS
        };
        let limit = if index < 2 {
            GM_TRANS_MAX
        } else {
            GM_ALPHA_MAX
        };
        let output = round2_signed(scaled * i64::from(distance), shift)
            .clamp(-i64::from(limit), i64::from(limit))
            << shift;
        params[index] = center + output as i32;
    }
    params
}

fn resolve_positive_divisor(denominator: u32) -> (u32, i64) {
    let n = denominator.ilog2();
    let excess = u64::from(denominator) - (1u64 << n);
    let index = if n > 7 {
        round2_unsigned(excess, n - 7)
    } else {
        excess << (7 - n)
    };
    let divisor = 128 + index;
    let factor = (65_536 + divisor / 2) / divisor;
    (n + 9, factor as i64)
}

const fn round2_unsigned(value: u64, shift: u32) -> u64 {
    if shift == 0 {
        value
    } else {
        (value + (1u64 << (shift - 1))) >> shift
    }
}

fn round2_signed(value: i64, shift: u32) -> i64 {
    let value = i128::from(value);
    let magnitude = if value < 0 { -value } else { value };
    let rounded = if shift == 0 {
        magnitude
    } else {
        (magnitude + (1i128 << (shift - 1))) >> shift
    };
    (if value < 0 { -rounded } else { rounded }) as i64
}

fn invalid_global_motion(
    reader: &BitReader<'_>,
    kind: GlobalMotionErrorKind,
) -> crate::error::Error {
    crate::error::Error::InvalidGlobalMotion {
        offset: reader.byte_offset(),
        bit_offset: reader.bit_offset(),
        kind,
    }
}

/// Reads `read_global_param( ref, idx )` (AV2 v1.0.0 § 5.18.9.2, mirror :7988-8014) for one
/// `gm_params[ ref ][ idx ]`, given the reference value `prev_gm_param = PrevGmParams[ ref
/// ][ idx ]` (`scale_warp_model()` output — cross-frame, supplied by the caller).
///
/// The bit position is **independent** of `prev_gm_param`: the subexp chain reads its bits
/// from `decode_subexp( 2*mx + 1, 3 )` (mirror :8076), which depends only on `mx` and `k`,
/// and `prev_gm_param` only recenters the already-decoded value (mirror :8054-8062). The
/// returned value is the signed warp coefficient at `WARPEDMODEL_PREC_BITS` precision.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the subexp code
/// is truncated, or [`Error::InvalidNs`](crate::error::Error::InvalidNs) for a degenerate
/// `ns(0)` (unreachable for the spec `mx`).
pub fn read_global_param(
    reader: &mut BitReader<'_>,
    idx: usize,
    prev_gm_param: i32,
) -> Result<i32> {
    let (prec_bits, mx) = if idx < 2 {
        (GM_TRANS_PREC_BITS, GM_TRANS_MAX)
    } else {
        (GM_ALPHA_PREC_BITS, GM_ALPHA_MAX)
    };
    let prec_diff = WARPEDMODEL_PREC_BITS - prec_bits;
    let is_scale_term = (idx % 3) == 2;
    let round = if is_scale_term {
        1i32 << WARPEDMODEL_PREC_BITS
    } else {
        0
    };
    let sub = if is_scale_term { 1i32 << prec_bits } else { 0 };
    let r = (prev_gm_param >> prec_diff) - sub;
    let decoded = decode_signed_subexp_with_ref(reader, -mx, mx + 1, r, GLOBAL_PARAM_K)?;
    Ok((decoded << prec_diff) + round)
}

/// `decode_signed_subexp_with_ref( low, high, r, k )` (AV2 v1.0.0 § 5.18.9.3, mirror
/// :8032-8038). Returns a value in `low ..= high - 1` (§ 6.17.9.3).
///
/// # Errors
/// Propagates a truncated subexp read or a degenerate `ns(0)`.
pub fn decode_signed_subexp_with_ref(
    reader: &mut BitReader<'_>,
    low: i32,
    high: i32,
    r: i32,
    k: u32,
) -> Result<i32> {
    let x = decode_unsigned_subexp_with_ref(
        reader,
        high.saturating_sub(low),
        r.saturating_sub(low),
        k,
    )?;
    Ok(x.saturating_add(low))
}

/// `decode_unsigned_subexp_with_ref( mx, r, k )` (AV2 v1.0.0 § 5.18.9.4, mirror
/// :8050-8064). Returns a value in `0 ..= mx - 1` (§ 6.17.9.4).
///
/// # Errors
/// Propagates a truncated subexp read or a degenerate `ns(0)`.
pub fn decode_unsigned_subexp_with_ref(
    reader: &mut BitReader<'_>,
    mx: i32,
    r: i32,
    k: u32,
) -> Result<i32> {
    let v = decode_subexp(reader, mx, k)?;
    let r = r.clamp(0, mx.saturating_sub(1));
    if r <= mx / 2 {
        Ok(inverse_recenter(r, v))
    } else {
        let mirrored = inverse_recenter(mx.wrapping_sub(1).wrapping_sub(r), v);
        Ok(mx.wrapping_sub(1).wrapping_sub(mirrored))
    }
}

/// `decode_subexp( numSyms, k )` (AV2 v1.0.0 § 5.18.9.5, mirror :8076-8122).
///
/// The growth variables (`mk`, `a`, the `f(b2)` width) are computed in `u32` so a
/// constructed input that forces `subexp_more_bits == 1` repeatedly cannot overflow the
/// `1 << b2` shift before the `numSyms <= mk + 3*a` guard terminates the loop. For the spec
/// callers `numSyms = 2*mx + 1 <= 32767`, so `b2` stays small; the widening only hardens the
/// standalone helper against arbitrary inputs (the proptest panic-freedom contract).
///
/// `numSyms <= 0` yields a degenerate `ns(0)` on the first iteration and surfaces as
/// [`Error::InvalidNs`](crate::error::Error::InvalidNs) rather than a panic.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the code is
/// truncated, or [`Error::InvalidNs`](crate::error::Error::InvalidNs) for a degenerate
/// `ns(0)` range.
pub fn decode_subexp(reader: &mut BitReader<'_>, num_syms: i32, k: u32) -> Result<i32> {
    let num_syms_u = u32::try_from(num_syms).unwrap_or(0);
    let mut i: u32 = 0;
    let mut mk = 0u32;
    loop {
        let b2: u32 = if i == 0 {
            k
        } else {
            k.saturating_add(i).saturating_sub(1)
        };
        let a = 1u32 << b2.min(31);
        let three_a = a.saturating_mul(3);
        let bound = mk.saturating_add(three_a);
        if num_syms_u <= bound {
            let n = num_syms_u - mk;
            let final_bits = reader.read_ns(n)?;
            return i32::try_from(final_bits.saturating_add(mk))
                .map_err(|_| invalid_subexp_value(reader));
        }
        let more_bits = reader.read_flag()?;
        if more_bits {
            i = i.saturating_add(1);
            mk = mk.saturating_add(a);
        } else {
            let bits = reader.read_bits(b2)?;
            return i32::try_from(bits.saturating_add(mk))
                .map_err(|_| invalid_subexp_value(reader));
        }
    }
}

/// `inverse_recenter( r, v )` (AV2 v1.0.0 § 5.18.9.6, mirror :8134-8142).
///
/// All branches use wrapping `i32` operations so the bounded warp arithmetic is exact and
/// stays panic-free for arbitrary `i32`
/// inputs from the proptest, using `wrapping_*` only where the spec's unbounded integers
/// would otherwise be modeled — here the warp ranges keep every value well inside i64).
#[must_use]
pub fn inverse_recenter(r: i32, v: i32) -> i32 {
    if v > r.wrapping_mul(2) {
        v
    } else if v & 1 != 0 {
        r.wrapping_sub((v.wrapping_add(1)) >> 1)
    } else {
        r.wrapping_add(v >> 1)
    }
}

/// Builds the structured error for a subexp value that does not fit in `i32` (unreachable
/// for spec inputs; defends the standalone helper against constructed inputs).
fn invalid_subexp_value(reader: &BitReader<'_>) -> crate::error::Error {
    crate::error::Error::InvalidNs {
        offset: reader.byte_offset(),
        bit_offset: reader.bit_offset(),
        message: "decoded subexp value does not fit in i64".to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    #[test]
    fn inverse_recenter_hand_vectors() {
        assert_eq!(inverse_recenter(0, 0), 0);
        assert_eq!(inverse_recenter(5, 12), 12);
        assert_eq!(inverse_recenter(5, 3), 3);
        assert_eq!(inverse_recenter(5, 4), 7);
        assert_eq!(inverse_recenter(10, 7), 6);
        assert_eq!(inverse_recenter(16383, 1), 16382);
    }

    #[test]
    fn decode_subexp_first_iteration_ns_branch() {
        let mut bits = Bits::default();
        bits.raw("101");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_subexp(&mut r, 10, 3).unwrap(), 5);
        assert_eq!(r.consumed_bits(), 3);
    }

    #[test]
    fn decode_subexp_immediate_subexp_bits_branch() {
        let mut bits = Bits::default();
        bits.raw("0110");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_subexp(&mut r, 100, 3).unwrap(), 6);
        assert_eq!(r.consumed_bits(), 4);
    }

    #[test]
    fn decode_subexp_more_bits_then_subexp_bits_branch() {
        let mut bits = Bits::default();
        bits.raw("10010");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_subexp(&mut r, 100, 3).unwrap(), 10);
        assert_eq!(r.consumed_bits(), 5);
    }

    #[test]
    fn decode_subexp_zero_numsyms_is_invalid_ns_not_panic() {
        let mut r = reader(&[0xFF]);
        assert!(matches!(
            decode_subexp(&mut r, 0, 3),
            Err(Error::InvalidNs { .. })
        ));
    }

    #[test]
    fn decode_subexp_eof_is_error() {
        let mut r = reader(&[]);
        assert!(matches!(
            decode_subexp(&mut r, 100, 3),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn decode_unsigned_subexp_with_ref_recenter_branch() {
        let mut bits = Bits::default();
        bits.raw("01");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_unsigned_subexp_with_ref(&mut r, 9, 4, 1).unwrap(), 3);
    }

    #[test]
    fn decode_unsigned_subexp_with_ref_mirror_branch() {
        let mut bits = Bits::default();
        bits.raw("100");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_unsigned_subexp_with_ref(&mut r, 9, 8, 1).unwrap(), 6);
    }

    #[test]
    fn decode_signed_subexp_with_ref_hand_vector() {
        let mut bits = Bits::default();
        bits.raw("01");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(
            decode_signed_subexp_with_ref(&mut r, -4, 5, 0, 1).unwrap(),
            -1
        );
    }

    #[test]
    fn decode_signed_subexp_with_ref_returns_in_low_high_range() {
        let mut bits = Bits::default();
        bits.raw("01");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let value = decode_signed_subexp_with_ref(&mut r, 10, 15, 10, 1).unwrap();
        assert_eq!(value, 11);
        assert!((10..15).contains(&value));
    }

    #[test]
    fn read_global_param_translational_idx0_prev_zero() {
        let mut bits = Bits::default();
        bits.raw("0001");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(read_global_param(&mut r, 0, 0).unwrap(), -8192);
        assert_eq!(r.consumed_bits(), 4);
    }

    #[test]
    fn read_global_param_scale_term_idx2_identity_prev_recovers_identity_round() {
        let mut bits = Bits::default();
        bits.raw("0000");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(
            read_global_param(&mut r, 2, 1i32 << WARPEDMODEL_PREC_BITS).unwrap(),
            65536
        );
        assert_eq!(r.consumed_bits(), 4);
    }

    #[test]
    fn read_global_param_eof_is_error() {
        let mut r = reader(&[]);
        assert!(matches!(
            read_global_param(&mut r, 0, 0),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    fn base_input(ref_frame_idx: &[u32]) -> GlobalMotionInput<'_> {
        GlobalMotionInput {
            frame_is_intra: false,
            frame_type: FrameType::Inter,
            enable_global_motion: true,
            num_total_refs: u32::try_from(ref_frame_idx.len()).unwrap(),
            ref_frame_idx,
            reference_state: None,
        }
    }

    fn parse_with_single_reference(bits: &str) -> Result<GlobalMotionParams> {
        let ref_order_hint = [1];
        let ref_num_total_refs = [0];
        let saved_order_hints = [[0; REFS_PER_FRAME]];
        let saved_gm_params = [[GlobalMotionRef::identity().gm_params; REFS_PER_FRAME]];
        let mut coded = Bits::default();
        coded.raw(bits);
        let data = coded.into_bytes();
        let mut r = reader(&data);
        parse_global_motion_params(
            &mut r,
            &GlobalMotionInput {
                frame_is_intra: false,
                frame_type: FrameType::Inter,
                enable_global_motion: true,
                num_total_refs: 1,
                ref_frame_idx: &[0],
                reference_state: Some(GlobalMotionReferenceState {
                    order_hint: 2,
                    ref_order_hint: &ref_order_hint,
                    ref_num_total_refs: &ref_num_total_refs,
                    saved_order_hints: &saved_order_hints,
                    saved_gm_params: &saved_gm_params,
                }),
            },
        )
    }

    #[test]
    fn complete_state_parses_identity_rotzoom_and_affine_models() {
        let identity = parse_with_single_reference("110").unwrap();
        assert_eq!(identity.stop, None);
        assert_eq!(identity.references[0].gm_type, GmType::Identity);

        let rotzoom = parse_with_single_reference("11110000000000000000").unwrap();
        assert_eq!(rotzoom.references[0].gm_type, GmType::RotZoom);
        assert_eq!(rotzoom.references[0].gm_params, DEFAULT_WARP_PARAMS);

        let affine = parse_with_single_reference("1110000000000000000000000000").unwrap();
        assert_eq!(affine.references[0].gm_type, GmType::Affine);
        assert_eq!(affine.references[0].gm_params, DEFAULT_WARP_PARAMS);
    }

    #[test]
    fn complete_state_reports_eof_inside_global_model() {
        assert!(matches!(
            parse_with_single_reference("1111"),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn complete_state_rejects_restricted_base_reference() {
        let ref_order_hint = [u32::MAX];
        let ref_num_total_refs = [0];
        let saved_order_hints = [[0; REFS_PER_FRAME]];
        let saved_gm_params = [[GlobalMotionRef::identity().gm_params; REFS_PER_FRAME]];
        let mut r = reader(&[0b1000_0000]);
        let error = parse_global_motion_params(
            &mut r,
            &GlobalMotionInput {
                frame_is_intra: false,
                frame_type: FrameType::Inter,
                enable_global_motion: true,
                num_total_refs: 1,
                ref_frame_idx: &[0],
                reference_state: Some(GlobalMotionReferenceState {
                    order_hint: 2,
                    ref_order_hint: &ref_order_hint,
                    ref_num_total_refs: &ref_num_total_refs,
                    saved_order_hints: &saved_order_hints,
                    saved_gm_params: &saved_gm_params,
                }),
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidGlobalMotion {
                kind: GlobalMotionErrorKind::OurReferenceRestricted,
                ..
            }
        ));
    }

    #[test]
    fn complete_state_uses_saved_model_as_cross_frame_predictor() {
        let saved = [131_072, 65_536, 65_600, 256, -256, 65_600];
        let ref_order_hint = [1];
        let ref_num_total_refs = [1];
        let saved_order_hints = [[0; REFS_PER_FRAME]];
        let mut saved_gm_params = [[GlobalMotionRef::identity().gm_params; REFS_PER_FRAME]];
        saved_gm_params[0][0] = saved;
        let mut coded = Bits::default();
        coded.raw("10110000000000000000");
        let data = coded.into_bytes();
        let mut r = reader(&data);
        let parsed = parse_global_motion_params(
            &mut r,
            &GlobalMotionInput {
                frame_is_intra: false,
                frame_type: FrameType::Inter,
                enable_global_motion: true,
                num_total_refs: 1,
                ref_frame_idx: &[0],
                reference_state: Some(GlobalMotionReferenceState {
                    order_hint: 2,
                    ref_order_hint: &ref_order_hint,
                    ref_num_total_refs: &ref_num_total_refs,
                    saved_order_hints: &saved_order_hints,
                    saved_gm_params: &saved_gm_params,
                }),
            },
        )
        .unwrap();
        assert_eq!(parsed.our_ref, Some(0));
        assert_eq!(parsed.references[0].gm_type, GmType::RotZoom);
        assert_eq!(parsed.references[0].gm_params, saved);
    }

    #[test]
    fn complete_state_rejects_restricted_saved_reference() {
        let ref_order_hint = [1];
        let ref_num_total_refs = [1];
        let mut saved_order_hints = [[0; REFS_PER_FRAME]];
        saved_order_hints[0][0] = u32::MAX;
        let saved_gm_params = [[GlobalMotionRef::identity().gm_params; REFS_PER_FRAME]];
        let mut r = reader(&[0b1000_0000]);
        let error = parse_global_motion_params(
            &mut r,
            &GlobalMotionInput {
                frame_is_intra: false,
                frame_type: FrameType::Inter,
                enable_global_motion: true,
                num_total_refs: 1,
                ref_frame_idx: &[0],
                reference_state: Some(GlobalMotionReferenceState {
                    order_hint: 2,
                    ref_order_hint: &ref_order_hint,
                    ref_num_total_refs: &ref_num_total_refs,
                    saved_order_hints: &saved_order_hints,
                    saved_gm_params: &saved_gm_params,
                }),
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidGlobalMotion {
                kind: GlobalMotionErrorKind::SavedReferenceRestricted,
                ..
            }
        ));
    }

    #[test]
    fn scale_warp_model_preserves_identity() {
        assert_eq!(
            scale_warp_model(DEFAULT_WARP_PARAMS, 3, -7),
            DEFAULT_WARP_PARAMS
        );
    }

    #[test]
    fn zero_total_refs_completes_without_stop() {
        let input = base_input(&[]);
        let mut r = reader(&[0b1000_0000]);
        let gm = parse_global_motion_params(&mut r, &input).unwrap();
        assert!(gm.use_global_motion);
        assert_eq!(gm.our_ref, Some(0));
        assert_eq!(
            gm.stop, None,
            "a zero-reference loop is complete, not stopped"
        );
    }

    #[test]
    fn signed_subexp_extreme_bounds_do_not_panic() {
        let mut r = reader(&[0x00, 0x00, 0x00, 0x00]);
        let _ = decode_signed_subexp_with_ref(&mut r, i32::MIN, i32::MAX, 0, 1);
    }

    #[test]
    fn unsigned_subexp_extreme_recenter_does_not_panic() {
        let mut r = reader(&[0x00, 0x00, 0x00, 0x00]);
        let _ = decode_unsigned_subexp_with_ref(&mut r, i32::MAX, i32::MAX - 1, 1);
        let mut r2 = reader(&[0x00, 0x00, 0x00, 0x00]);
        let _ = decode_unsigned_subexp_with_ref(&mut r2, 8, i32::MIN, 1);
    }

    #[test]
    fn intra_returns_identity_no_bits() {
        let mut input = base_input(&[0, 1]);
        input.frame_is_intra = true;
        let mut r = reader(&[0xFF]);
        let gm = parse_global_motion_params(&mut r, &input).unwrap();
        assert!(!gm.use_global_motion);
        assert_eq!(gm.our_ref, None);
        assert_eq!(gm.stop, None);
        assert_eq!(gm.references[0], GlobalMotionRef::identity());
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn disabled_returns_identity_no_bits() {
        let mut input = base_input(&[0, 1]);
        input.enable_global_motion = false;
        let mut r = reader(&[0xFF]);
        let gm = parse_global_motion_params(&mut r, &input).unwrap();
        assert!(!gm.use_global_motion);
        assert_eq!(gm.stop, None);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn use_global_motion_zero_returns_after_one_bit() {
        let mut bits = Bits::default();
        bits.bit(0); // use_global_motion = 0
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let gm = parse_global_motion_params(&mut r, &base_input(&[0, 1])).unwrap();
        assert!(!gm.use_global_motion);
        assert_eq!(gm.our_ref, None);
        assert_eq!(gm.stop, None);
        assert_eq!(r.consumed_bits(), 1);
    }

    #[test]
    fn our_ref_equal_num_total_refs_stops_at_order_hints_boundary() {
        let mut bits = Bits::default();
        bits.bit(1); // use_global_motion = 1
        bits.raw("11"); // our_ref ns(3) = 2 == NumTotalRefs
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let gm = parse_global_motion_params(&mut r, &base_input(&[0, 1])).unwrap();
        assert!(gm.use_global_motion);
        assert_eq!(gm.our_ref, Some(2));
        assert_eq!(gm.stop, Some(GlobalMotionStop::OrderHintsUnmodeled));
        assert_eq!(r.consumed_bits(), 3);
    }

    #[test]
    fn our_ref_not_num_total_refs_stops_at_ref_num_total_refs() {
        let mut bits = Bits::default();
        bits.bit(1); // use_global_motion = 1
        bits.raw("0"); // our_ref ns(3) = 0 != NumTotalRefs
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let gm = parse_global_motion_params(&mut r, &base_input(&[3, 5])).unwrap();
        assert!(gm.use_global_motion);
        assert_eq!(gm.our_ref, Some(0));
        assert_eq!(gm.stop, Some(GlobalMotionStop::RefNumTotalRefsUnmodeled));
        assert_eq!(r.consumed_bits(), 2);
    }

    #[test]
    fn switch_frame_infers_our_ref_num_total_refs_no_ns_bits() {
        let mut input = base_input(&[0, 1, 2]);
        input.frame_type = FrameType::Switch;
        let mut bits = Bits::default();
        bits.bit(1); // use_global_motion = 1 (only bit read)
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let gm = parse_global_motion_params(&mut r, &input).unwrap();
        assert!(gm.use_global_motion);
        assert_eq!(gm.our_ref, Some(3)); // == NumTotalRefs, no ns read
        assert_eq!(gm.stop, Some(GlobalMotionStop::OrderHintsUnmodeled));
        assert_eq!(r.consumed_bits(), 1);
    }

    #[test]
    fn ns_boundary_max_our_ref_value() {
        let mut bits = Bits::default();
        bits.bit(1); // use_global_motion
        bits.bit(1); // our_ref ns(2) = 1 == NumTotalRefs
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let gm = parse_global_motion_params(&mut r, &base_input(&[4])).unwrap();
        assert_eq!(gm.our_ref, Some(1));
        assert_eq!(gm.stop, Some(GlobalMotionStop::OrderHintsUnmodeled));

        let mut bits = Bits::default();
        bits.bit(1); // use_global_motion
        bits.bit(0); // our_ref ns(2) = 0 != NumTotalRefs
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let gm = parse_global_motion_params(&mut r, &base_input(&[4])).unwrap();
        assert_eq!(gm.our_ref, Some(0));
        assert_eq!(gm.stop, Some(GlobalMotionStop::RefNumTotalRefsUnmodeled));
    }

    #[test]
    fn eof_before_use_global_motion_is_error() {
        let mut r = reader(&[]);
        assert!(matches!(
            parse_global_motion_params(&mut r, &base_input(&[0, 1])),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn eof_inside_our_ref_ns_is_error() {
        let mut bits = Bits::default();
        bits.f(0, 7); // 7 leading pad bits (the test consumes these)
        bits.bit(1); // use_global_motion at bit index 7 (last bit of the byte)
        let data = bits.into_bytes(); // exactly 1 byte
        let mut r = reader(&data);
        r.read_bits(7).unwrap(); // consume the pad; reader now at bit 7
        let input = base_input(&[0, 1, 2, 3, 4, 5, 6]); // NumTotalRefs=7 -> our_ref ns(8)
        assert!(matches!(
            parse_global_motion_params(&mut r, &input),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn references_table_is_refs_per_frame_identity_at_stop() {
        let mut bits = Bits::default();
        bits.bit(1); // use_global_motion
        bits.raw("11"); // our_ref = NumTotalRefs (OrderHints stop)
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let gm = parse_global_motion_params(&mut r, &base_input(&[0, 1])).unwrap();
        assert_eq!(gm.references.len(), REFS_PER_FRAME);
        for reference in &gm.references {
            assert_eq!(*reference, GlobalMotionRef::identity());
            assert_eq!(reference.gm_type, GmType::Identity);
            assert_eq!(reference.gm_params[2], 1 << WARPEDMODEL_PREC_BITS);
            assert_eq!(reference.gm_params[0], 0);
        }
    }

    #[test]
    fn gm_type_values_match_spec() {
        assert_eq!(GmType::Identity.value(), 0);
        assert_eq!(GmType::RotZoom.value(), 1);
        assert_eq!(GmType::Affine.value(), 2);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// The subexp leaves must never panic over arbitrary warp bit patterns / params —
        /// the constructed-input overflow audit (the `1 << b2` growth in decode_subexp, the
        /// `2 * r` in inverse_recenter) is enforced by exercising the chain with adversarial
        /// inputs.
        #[test]
        fn decode_subexp_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            num_syms in any::<i32>(),
            k in 0u32..=40,
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = decode_subexp(&mut reader, num_syms, k);
        }

        /// decode_signed_subexp_with_ref must never panic over arbitrary bounds / ref / bits
        /// (the `high - low` / `r - low` / `2 * r` arithmetic is bounded for warp inputs but
        /// must stay panic-free for the constructed range here).
        #[test]
        fn decode_signed_subexp_with_ref_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            low in -100_000i32..100_000,
            span in 1i32..200_000,
            r in -200_000i32..200_000,
            k in 0u32..=20,
        ) {
            let high = low + span;
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = decode_signed_subexp_with_ref(&mut reader, low, high, r, k);
        }

        /// read_global_param must never panic over arbitrary idx / prev_gm_param / bits.
        #[test]
        fn read_global_param_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            idx in 0usize..6,
            prev in any::<i32>(),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = read_global_param(&mut reader, idx, prev);
        }

        /// inverse_recenter must never panic over arbitrary i64 r / v.
        #[test]
        fn inverse_recenter_never_panics(r in any::<i32>(), v in any::<i32>()) {
            let _ = inverse_recenter(r, v);
        }

        /// The inter arm parser must never panic on arbitrary input / state.
        #[test]
        fn parse_global_motion_params_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            frame_is_intra in any::<bool>(),
            switch in any::<bool>(),
            enable_global_motion in any::<bool>(),
            ref_count in 0usize..=7,
            ref_seed in any::<u32>(),
        ) {
            let ref_frame_idx: Vec<u32> = (0..ref_count)
                .map(|i| ref_seed.wrapping_add(i as u32) % 16)
                .collect();
            let input = GlobalMotionInput {
                frame_is_intra,
                frame_type: if switch { FrameType::Switch } else { FrameType::Inter },
                enable_global_motion,
                num_total_refs: ref_count as u32,
                ref_frame_idx: &ref_frame_idx,
                reference_state: None,
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_global_motion_params(&mut reader, &input);
        }
    }
}
