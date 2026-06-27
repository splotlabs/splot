// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 5.18.9 `global_motion_params()` **inter arm**
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-9`, mirror :7776-7931) and its
//! subexp decode chain (§ 5.18.9.2-.6, mirror :7988-8142).
//!
//! On the intra path `global_motion_params()` returns before any bit read (mirror :7792,
//! `FrameIsIntra || !enable_global_motion`) — that no-bit arm is recorded by
//! [`super::tail`]. This module models the **inter arm**, which sits in the § 5.18.2
//! shared tail after `reduced_tx_set` (mirror :5337-5339), before `film_grain_config()`:
//!
//! ```text
//! global_motion_params( ) {
//!     for ref ... { GmType[ref] = IDENTITY; gm_params[ref] = identity warp }   // no bits
//!     if ( FrameIsIntra || !enable_global_motion ) return                       // intra/disabled
//!     use_global_motion                                                  f(1)
//!     if ( !use_global_motion ) return
//!     baseParams = Default_Warp_Params; baseDistance = 1                         // no bits
//!     if ( FrameType == SWITCH_FRAME ) our_ref = NumTotalRefs                    // no bits
//!     else                            our_ref                            ns(NumTotalRefs + 1)
//!     if ( our_ref != NumTotalRefs ) {
//!         refIdx = ref_frame_idx[ our_ref ]
//!         if ( RefNumTotalRefs[ refIdx ] > 0 ) {                                 // CROSS-FRAME
//!             their_ref                                                  ns(RefNumTotalRefs[refIdx])
//!             baseParams = SavedGmParams[ refIdx ][ their_ref ]                  // CROSS-FRAME
//!             baseDistance = get_relative_dist(...)                              // CROSS-FRAME
//!         }
//!     }
//!     for ( ref = 0; ref < NumTotalRefs; ref++ ) {
//!         dist = get_relative_dist(OrderHint, OrderHints[ ref ])                 // CROSS-FRAME
//!         if ( dist == 0 || OrderHints[ ref ] == RESTRICTED_OH ) { identity }    // no bits
//!         else {
//!             PrevGmParams[ ref ] = scale_warp_model(baseParams, baseDistance, dist)
//!             is_global                                                  f(1)
//!             if ( is_global ) { is_rot_zoom f(1); type = is_rot_zoom ? ROTZOOM : AFFINE }
//!             else type = IDENTITY
//!             if ( type >= ROTZOOM ) { read_global_param(ref, 2/3 [/4/5] /0/1) }
//!         }
//!     }
//! }
//! ```
//!
//! ## Honest stops (the cross-frame boundary)
//!
//! Two § 5.18.9.1 derivations consume per-slot facts of the *referenced* frame that this
//! phase does not model — they are recorded as [`GlobalMotionStop`] coverage stops, never
//! truncations, with every fact parsed before the stop preserved:
//!
//! - **`our_ref != NumTotalRefs` base-warp load** (mirror :7826-7846): the
//!   `RefNumTotalRefs[ refIdx ] > 0` test, the `their_ref ns(RefNumTotalRefs[refIdx])`
//!   read, and the `SavedGmParams` / `SavedOrderHints` loads need `RefNumTotalRefs` and the
//!   per-slot saved warp state of `ref_frame_idx[ our_ref ]`, which are facts of a
//!   previously decoded frame the model does not carry. Because the very next read
//!   (`their_ref`) has a *width* (`ns(RefNumTotalRefs[refIdx])`) set by that unmodeled
//!   count, the parse stops at [`GlobalMotionStop::RefNumTotalRefsUnmodeled`] rather than
//!   guess. When `our_ref == NumTotalRefs` this whole arm is skipped (baseParams stay the
//!   identity default), so the parse continues.
//! - **per-reference loop gate** (mirror :7853-7857): each reference reads warp bits only
//!   when `dist != 0 && OrderHints[ ref ] != RESTRICTED_OH`, and both `OrderHint` and
//!   `OrderHints[ ref ]` are cross-frame order-hint state. The *presence* of the first
//!   per-reference bit therefore depends on unmodeled state, so the parse stops at
//!   [`GlobalMotionStop::OrderHintsUnmodeled`] at the loop boundary.
//!
//! The reachable production depth is thus `use_global_motion` and the `our_ref` base
//! selection. The per-reference warp decode and its subexp chain are fully implemented and
//! unit-tested here against hand-computed vectors; they become production-reachable once the
//! § 7.23 cross-frame `OrderHints` / `RefNumTotalRefs` / `SavedGmParams` state is threaded
//! (named residuals on `AV2-5.18.9-GLOBAL-MOTION`).
//!
//! ## § 6.17.9 conformance
//!
//! Both § 6.17.9.1 conformance clauses — `OrderHints[ our_ref ] != RESTRICTED_OH` when
//! `our_ref != NumTotalRefs`, and `SavedOrderHints[ refIdx ][ their_ref ] != RESTRICTED_OH`
//! — read the same cross-frame order-hint state the honest stops gate on, so neither is
//! locally decidable; both are named residuals, not diagnostics. The arithmetic-only
//! § 6.17.9.3-.5 notes (the decode ranges) are encoded as the chain's typed behavior and
//! its tests, not as diagnostics.

use crate::bitio::BitReader;
use crate::error::Result;

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
const GM_ALPHA_MAX: i64 = (1i64 << GM_ABS_ALPHA_BITS) - 1;

/// `GM_TRANS_MAX = (1 << GM_ABS_TRANS_BITS) - 1` (AV2 v1.0.0 § 3): the `mx` bound for
/// `idx < 2` (mirror :7998).
const GM_TRANS_MAX: i64 = (1i64 << GM_ABS_TRANS_BITS) - 1;

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

/// `Default_Warp_Params[6]` (AV2 v1.0.0 § 7, mirror :4702): the identity warp model. Index
/// `i % 3 == 2` (the diagonal scale terms 2 and 5) is `1 << WARPEDMODEL_PREC_BITS`; every
/// other term is `0` (mirror :7784-7786 derives the same identity initialiser).
const DEFAULT_WARP_PARAMS: [i64; 6] = [
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
    pub gm_params: [i64; 6],
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
    // mirror :7778-7790: every reference starts at the identity warp (no bits).
    let references = [GlobalMotionRef::identity(); REFS_PER_FRAME];

    // mirror :7792-7796: intra / global-motion-disabled return — no bits.
    if input.frame_is_intra || !input.enable_global_motion {
        return Ok(GlobalMotionParams {
            use_global_motion: false,
            our_ref: None,
            stop: None,
            references,
        });
    }

    // mirror :7798: use_global_motion f(1).
    let use_global_motion = reader.read_flag()?;
    if !use_global_motion {
        // mirror :7800-7804: return — no warp parameters present.
        return Ok(GlobalMotionParams {
            use_global_motion: false,
            our_ref: None,
            stop: None,
            references,
        });
    }

    // mirror :7806-7812: baseParams = Default_Warp_Params; baseDistance = 1 (no bits).
    // These feed scale_warp_model() in the per-reference loop, which is gated behind the
    // OrderHints honest stop, so the values are not threaded further here.

    // mirror :7814-7824: our_ref selection.
    let our_ref = if input.frame_type == FrameType::Switch {
        // mirror :7816: SWITCH_FRAME -> our_ref = NumTotalRefs (no bits).
        input.num_total_refs
    } else {
        // mirror :7820-7822: n = NumTotalRefs + 1; our_ref ns(n).
        // NumTotalRefs <= 7 (REFS_PER_FRAME), so n <= 8 and n+1 never overflows the read.
        let n = input.num_total_refs.saturating_add(1);
        reader.read_ns(n)?
    };

    // mirror :7826-7846: the our_ref != NumTotalRefs base-warp load. The very next read
    // (their_ref ns(RefNumTotalRefs[refIdx])) has a width set by RefNumTotalRefs — a
    // per-slot fact of the REFERENCED frame, not modeled — and the SavedGmParams /
    // SavedOrderHints loads it gates are likewise cross-frame. Stop honestly at the boundary
    // with use_global_motion / our_ref preserved. When our_ref == NumTotalRefs the whole arm
    // is skipped (baseParams stay the identity default), so the parse continues.
    if our_ref != input.num_total_refs {
        return Ok(GlobalMotionParams {
            use_global_motion: true,
            our_ref: Some(our_ref),
            stop: Some(GlobalMotionStop::RefNumTotalRefsUnmodeled),
            references,
        });
    }

    // mirror :7853-7857: the per-reference loop reads warp bits for a reference only when
    // dist != 0 && OrderHints[ ref ] != RESTRICTED_OH. With zero references the loop has no
    // iterations, consults no cross-frame state, and the structure completes (stop: None).
    if input.num_total_refs == 0 {
        return Ok(GlobalMotionParams {
            use_global_motion: true,
            our_ref: Some(our_ref),
            stop: None,
            references,
        });
    }
    // Otherwise both OrderHint and OrderHints[ ref ] are cross-frame order-hint state, so
    // the presence of the first per-reference bit is undeterminable here — stop honestly at
    // the loop boundary.
    Ok(GlobalMotionParams {
        use_global_motion: true,
        our_ref: Some(our_ref),
        stop: Some(GlobalMotionStop::OrderHintsUnmodeled),
        references,
    })
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
    prev_gm_param: i64,
) -> Result<i64> {
    // mirror :7990-8000: precBits / mx select on idx < 2 (translational) vs >= 2 (alpha).
    let (prec_bits, mx) = if idx < 2 {
        (GM_TRANS_PREC_BITS, GM_TRANS_MAX)
    } else {
        (GM_ALPHA_PREC_BITS, GM_ALPHA_MAX)
    };
    // mirror :8002: precDiff = WARPEDMODEL_PREC_BITS - precBits (>= 0 for both branches:
    // 16 - 3 = 13, 16 - 10 = 6).
    let prec_diff = WARPEDMODEL_PREC_BITS - prec_bits;
    // mirror :8004-8006: round / sub when (idx % 3) == 2 (the diagonal scale terms).
    let is_scale_term = (idx % 3) == 2;
    let round: i64 = if is_scale_term {
        1i64 << WARPEDMODEL_PREC_BITS
    } else {
        0
    };
    let sub: i64 = if is_scale_term { 1i64 << prec_bits } else { 0 };
    // mirror :8008: r = (PrevGmParams[ ref ][ idx ] >> precDiff) - sub. An arithmetic
    // (sign-preserving) shift in i64 matches the spec's signed integer arithmetic; the warp
    // coefficient magnitude is bounded well within i64.
    let r = (prev_gm_param >> prec_diff) - sub;
    // mirror :8010-8012: decode_signed_subexp_with_ref( -mx, mx + 1, r, 3 ) << precDiff + round.
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
    low: i64,
    high: i64,
    r: i64,
    k: u32,
) -> Result<i64> {
    // mirror :8034: x = decode_unsigned_subexp_with_ref( high - low, r - low, k ).
    // Saturating spans: in-spec warp bounds never saturate, but this helper is public and
    // a constructed (low, high, r) such as (i64::MIN, i64::MAX, _) must not overflow-panic;
    // a saturated span keeps the ordering semantics and stays panic-free.
    let x = decode_unsigned_subexp_with_ref(
        reader,
        high.saturating_sub(low),
        r.saturating_sub(low),
        k,
    )?;
    // mirror :8036: return x + low.
    Ok(x.saturating_add(low))
}

/// `decode_unsigned_subexp_with_ref( mx, r, k )` (AV2 v1.0.0 § 5.18.9.4, mirror
/// :8050-8064). Returns a value in `0 ..= mx - 1` (§ 6.17.9.4).
///
/// # Errors
/// Propagates a truncated subexp read or a degenerate `ns(0)`.
pub fn decode_unsigned_subexp_with_ref(
    reader: &mut BitReader<'_>,
    mx: i64,
    r: i64,
    k: u32,
) -> Result<i64> {
    // mirror :8052: v = decode_subexp( mx, k ). decode_subexp's bit reads depend only on mx
    // and k; r recenters the decoded value below (so r never affects the bit position).
    let v = decode_subexp(reader, mx, k)?;
    // mirror :8054-8062: recenter v around r. The doubling comparison runs in i128 so an
    // arbitrary public-API r never overflow-panics (in-spec warp values are far inside
    // range); the else arm uses the same wrapping arithmetic contract as inverse_recenter
    // (panic-free; out-of-contract inputs yield wrapped values, never UB or a panic).
    if i128::from(r) * 2 <= i128::from(mx) {
        Ok(inverse_recenter(r, v))
    } else {
        let mirrored = inverse_recenter(mx.wrapping_sub(1).wrapping_sub(r), v);
        Ok(mx.wrapping_sub(1).wrapping_sub(mirrored))
    }
}

/// `decode_subexp( numSyms, k )` (AV2 v1.0.0 § 5.18.9.5, mirror :8076-8122).
///
/// The growth variables (`mk`, `a`, the `f(b2)` width) are computed in `u64` so a
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
pub fn decode_subexp(reader: &mut BitReader<'_>, num_syms: i64, k: u32) -> Result<i64> {
    // numSyms is non-negative for every spec call (2*mx + 1); a non-positive input drives the
    // first ns(n) toward n <= 0, handled by read_ns's InvalidNs (no panic). Clamp the working
    // value to u64 for the unsigned growth arithmetic; a negative numSyms maps to 0 so the
    // first-iteration guard takes the ns branch with n == 0 -> InvalidNs.
    let num_syms_u: u64 = u64::try_from(num_syms).unwrap_or(0);
    let mut i: u32 = 0;
    let mut mk: u64 = 0;
    loop {
        // mirror :8084: b2 = i ? k + i - 1 : k.
        let b2: u32 = if i == 0 {
            k
        } else {
            k.saturating_add(i).saturating_sub(1)
        };
        // mirror :8086: a = 1 << b2. Capped at 1 << 63 so the shift never overflows u64; the
        // cap is only reached far past the loop's numSyms-bounded termination for real inputs.
        let a: u64 = 1u64 << b2.min(63);
        // mirror :8088: if ( numSyms <= mk + 3 * a ) — saturating so the comparison is exact
        // even when 3 * a would overflow u64 (then mk + 3*a saturates to a value >= numSyms).
        let three_a = a.saturating_mul(3);
        let bound = mk.saturating_add(three_a);
        if num_syms_u <= bound {
            // mirror :8090-8094: n = numSyms - mk; subexp_final_bits ns(n); return + mk.
            // num_syms_u >= mk here (mk only advances while num_syms_u > mk + 3*a > mk, so the
            // previous mk was < num_syms_u and mk += a kept it < num_syms_u — see the module
            // overflow note). For num_syms_u >= 1 the first iteration has mk == 0 <
            // num_syms_u, so n >= 1; the degenerate num_syms_u == 0 input yields n == 0 and
            // read_ns(0) correctly rejects it with InvalidNs. The i64 cast below is exact
            // for the bounded n.
            let n = num_syms_u - mk;
            let n_u32 = u32::try_from(n).unwrap_or(u32::MAX);
            let final_bits = u64::from(reader.read_ns(n_u32)?);
            return i64::try_from(final_bits + mk).map_err(|_| invalid_subexp_value(reader));
        }
        // mirror :8098: subexp_more_bits f(1).
        let more_bits = reader.read_flag()?;
        if more_bits {
            // mirror :8102-8104: i++; mk += a.
            i = i.saturating_add(1);
            mk = mk.saturating_add(a);
        } else {
            // mirror :8108-8110: subexp_bits f(b2); return subexp_bits + mk.
            let bits = u64::from(reader.read_f(b2)?);
            return i64::try_from(bits + mk).map_err(|_| invalid_subexp_value(reader));
        }
    }
}

/// `inverse_recenter( r, v )` (AV2 v1.0.0 § 5.18.9.6, mirror :8134-8142).
///
/// All branches are computed in `i64` so the `2 * r` / `r - ((v + 1) >> 1)` arithmetic never
/// overflows or wraps for the bounded warp `r` (and stays panic-free for arbitrary `i64`
/// inputs from the proptest, using `wrapping_*` only where the spec's unbounded integers
/// would otherwise be modeled — here the warp ranges keep every value well inside i64).
#[must_use]
pub fn inverse_recenter(r: i64, v: i64) -> i64 {
    // mirror :8135-8136: if ( v > 2 * r ) return v. Every branch uses wrapping i64 ops so
    // an adversarial (out-of-warp-range) r / v cannot overflow-panic; for the bounded warp
    // inputs the wrapping never triggers, so the result is the exact spec value.
    if v > r.wrapping_mul(2) {
        v
    } else if v & 1 != 0 {
        // mirror :8137-8138: else if ( v & 1 ) return r - ((v + 1) >> 1).
        r.wrapping_sub((v.wrapping_add(1)) >> 1)
    } else {
        // mirror :8139-8140: else return r + (v >> 1).
        r.wrapping_add(v >> 1)
    }
}

/// Builds the structured error for a subexp value that does not fit in `i64` (unreachable
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

    // ---- § 5.18.9.6 inverse_recenter (pure arithmetic, no reads) ----

    #[test]
    fn inverse_recenter_hand_vectors() {
        // mirror :8134-8142.
        // r=0, v=0: v > 2*r (0>0)? no. v&1? no. else r + (v>>1) = 0 + 0 = 0.
        assert_eq!(inverse_recenter(0, 0), 0);
        // r=5, v=12: v > 2*r (12>10)? yes -> v = 12.
        assert_eq!(inverse_recenter(5, 12), 12);
        // r=5, v=3: 3>10? no. 3&1=1 -> r - ((v+1)>>1) = 5 - (4>>1) = 5 - 2 = 3.
        assert_eq!(inverse_recenter(5, 3), 3);
        // r=5, v=4: 4>10? no. 4&1=0 -> r + (v>>1) = 5 + 2 = 7.
        assert_eq!(inverse_recenter(5, 4), 7);
        // r=10, v=7: 7>20? no. 7&1=1 -> 10 - ((7+1)>>1) = 10 - 4 = 6.
        assert_eq!(inverse_recenter(10, 7), 6);
        // r=16383, v=1 (the read_global_param idx=0/prev=0 chain step): 1>32766? no.
        // 1&1=1 -> 16383 - ((1+1)>>1) = 16383 - 1 = 16382.
        assert_eq!(inverse_recenter(16383, 1), 16382);
    }

    // ---- § 5.18.9.5 decode_subexp ----

    #[test]
    fn decode_subexp_first_iteration_ns_branch() {
        // numSyms=10, k=3: i=0, mk=0, b2=3, a=8. 10 <= 0 + 24 -> take ns(n=10) branch.
        // ns(10): w = 4 (10 < 16), m = 16 - 10 = 6. read w-1 = 3 bits = 101b = 5; 5 < 6 so
        // value = 5. decode_subexp returns 5 + mk(0) = 5. Bits = "101".
        let mut bits = Bits::default();
        bits.raw("101");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_subexp(&mut r, 10, 3).unwrap(), 5);
        assert_eq!(r.consumed_bits(), 3);
    }

    #[test]
    fn decode_subexp_immediate_subexp_bits_branch() {
        // numSyms=100, k=3: i=0, mk=0, b2=3, a=8. 100 <= 24? no. subexp_more_bits f(1) = 0
        // -> subexp_bits f(3) = 110b = 6. return 6 + mk(0) = 6. Bits = "0110".
        let mut bits = Bits::default();
        bits.raw("0110");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_subexp(&mut r, 100, 3).unwrap(), 6);
        assert_eq!(r.consumed_bits(), 4);
    }

    #[test]
    fn decode_subexp_more_bits_then_subexp_bits_branch() {
        // numSyms=100, k=3:
        //   i=0, mk=0, b2=3, a=8. 100<=24? no. more_bits = 1 -> i=1, mk=8.
        //   i=1, mk=8, b2=k+i-1=3, a=8. 100<=8+24=32? no. more_bits = 0 -> subexp_bits f(3)
        //     = 010b = 2. return 2 + mk(8) = 10. Bits = "1 0 010" = "10010".
        let mut bits = Bits::default();
        bits.raw("10010");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_subexp(&mut r, 100, 3).unwrap(), 10);
        assert_eq!(r.consumed_bits(), 5);
    }

    #[test]
    fn decode_subexp_zero_numsyms_is_invalid_ns_not_panic() {
        // numSyms=0, k=3: 0 <= mk(0) + 3*a -> ns(n=0) -> InvalidNs (degenerate range).
        let mut r = reader(&[0xFF]);
        assert!(matches!(
            decode_subexp(&mut r, 0, 3),
            Err(Error::InvalidNs { .. })
        ));
    }

    #[test]
    fn decode_subexp_eof_is_error() {
        // numSyms=100, k=3 needs at least the more_bits f(1); empty payload overruns.
        let mut r = reader(&[]);
        assert!(matches!(
            decode_subexp(&mut r, 100, 3),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    // ---- § 5.18.9.4 decode_unsigned_subexp_with_ref ----

    #[test]
    fn decode_unsigned_subexp_with_ref_recenter_branch() {
        // mx=9, r=4, k=1: (r<<1)=8 <= mx=9 -> inverse_recenter(4, v).
        //   v = decode_subexp(9, 1): i=0, mk=0, b2=1, a=2. 9<=0+6? no. more_bits=0 ->
        //     subexp_bits f(1) = 1. v = 1. Bits = "01".
        //   inverse_recenter(4, 1): 1>8? no. 1&1=1 -> 4 - ((1+1)>>1) = 4 - 1 = 3.
        let mut bits = Bits::default();
        bits.raw("01");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_unsigned_subexp_with_ref(&mut r, 9, 4, 1).unwrap(), 3);
    }

    #[test]
    fn decode_unsigned_subexp_with_ref_mirror_branch() {
        // mx=9, r=8, k=1: (r<<1)=16 <= mx=9? no -> mx-1 - inverse_recenter(mx-1-r, v)
        //   = 8 - inverse_recenter(8-8=0, v).
        //   v = decode_subexp(9, 1): i=0, mk=0, b2=1, a=2. 9<=6? no. more_bits=1 -> i=1, mk=2.
        //     i=1, mk=2, b2=k+i-1=1, a=2. 9<=2+6=8? no. more_bits=0 -> subexp_bits f(1)=0.
        //     v = 0 + mk(2) = 2. Bits = "1 0 0" = "100".
        //   inverse_recenter(0, 2): 2>0? yes -> 2. result = 8 - 2 = 6.
        let mut bits = Bits::default();
        bits.raw("100");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(decode_unsigned_subexp_with_ref(&mut r, 9, 8, 1).unwrap(), 6);
    }

    // ---- § 5.18.9.3 decode_signed_subexp_with_ref ----

    #[test]
    fn decode_signed_subexp_with_ref_hand_vector() {
        // low=-4, high=5, r=0, k=1: x = decode_unsigned_subexp_with_ref(high-low=9, r-low=4, 1)
        //   (the recenter-branch vector above) = 3. return x + low = 3 + (-4) = -1.
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
        // § 6.17.9.3: the result is in low..=high-1. low=10, high=15, r=10, k=1:
        //   x = decode_unsigned_subexp_with_ref(high-low=5, r-low=0, 1):
        //     v = decode_subexp(5, 1): i=0, mk=0, b2=1, a=2. 5 <= 0+6 -> ns(n=5).
        //       ns(5): w=3, m=8-5=3. read w-1=2 bits "01"=1; 1<3 -> value=1. v=1. Bits="01".
        //     (r<<1)=0 <= mx=5 -> inverse_recenter(0, 1): 1>0 -> 1. x=1.
        //   return x + low = 1 + 10 = 11 (in 10..=14).
        let mut bits = Bits::default();
        bits.raw("01");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let value = decode_signed_subexp_with_ref(&mut r, 10, 15, 10, 1).unwrap();
        assert_eq!(value, 11);
        assert!((10..15).contains(&value));
    }

    // ---- § 5.18.9.2 read_global_param ----

    #[test]
    fn read_global_param_translational_idx0_prev_zero() {
        // idx=0 (translational): precBits=3, mx=16383, precDiff=13, round=0, sub=0.
        // prev=0 -> r = (0 >> 13) - 0 = 0.
        // decode_signed_subexp_with_ref(-16383, 16384, 0, 3):
        //   decode_unsigned_subexp_with_ref(32767, 16383, 3):
        //     v = decode_subexp(32767, 3): i=0, mk=0, b2=3, a=8. 32767<=24? no. more_bits=0
        //       -> subexp_bits f(3) = 001b = 1. v = 1. Bits = "0 001" = "0001".
        //     (16383<<1)=32766 <= 32767 -> inverse_recenter(16383, 1) = 16382.
        //   x = 16382 + (-16383) = -1.
        // result = (-1 << 13) + 0 = -8192.
        let mut bits = Bits::default();
        bits.raw("0001");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(read_global_param(&mut r, 0, 0).unwrap(), -8192);
        assert_eq!(r.consumed_bits(), 4);
    }

    #[test]
    fn read_global_param_scale_term_idx2_identity_prev_recovers_identity_round() {
        // idx=2 (scale term, idx%3==2): precBits=10, mx=511, precDiff=6, round=1<<16=65536,
        // sub=1<<10=1024. prev = Default_Warp_Params[2] = 1<<16 = 65536 ->
        //   r = (65536 >> 6) - 1024 = 1024 - 1024 = 0.
        // decode_signed_subexp_with_ref(-511, 512, 0, 3):
        //   decode_unsigned_subexp_with_ref(1023, 511, 3):
        //     v = decode_subexp(1023, 3): i=0, mk=0, b2=3, a=8. 1023<=24? no. more_bits=0 ->
        //       subexp_bits f(3) = 000b = 0. v = 0. Bits = "0 000" = "0000".
        //     (511<<1)=1022 <= 1023 -> inverse_recenter(511, 0): 0>1022? no. 0&1=0 ->
        //       511 + (0>>1) = 511.
        //   x = 511 + (-511) = 0.
        // result = (0 << 6) + 65536 = 65536 -> the identity scale term round-trips.
        let mut bits = Bits::default();
        bits.raw("0000");
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert_eq!(
            read_global_param(&mut r, 2, 1i64 << WARPEDMODEL_PREC_BITS).unwrap(),
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

    // ---- § 5.18.9.1 parse_global_motion_params (the inter arm) ----

    fn base_input(ref_frame_idx: &[u32]) -> GlobalMotionInput<'_> {
        GlobalMotionInput {
            frame_is_intra: false,
            frame_type: FrameType::Inter,
            enable_global_motion: true,
            num_total_refs: u32::try_from(ref_frame_idx.len()).unwrap(),
            ref_frame_idx,
        }
    }

    #[test]
    fn zero_total_refs_completes_without_stop() {
        // mirror :7853-7857: with NumTotalRefs == 0 the per-reference loop has zero
        // iterations and consults no cross-frame state, so the structure COMPLETES
        // (stop: None) instead of reporting an honest stop (codex PR #64 review).
        // Bits: use_global_motion == 1, then our_ref ns(1) reads no bits (n == 1 ->
        // single symbol 0 == NumTotalRefs -> the base-load arm is skipped).
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
        // Public-API hardening (codex PR #64 review): (low, high, r) spans like
        // (i64::MIN, i64::MAX, 0) must not overflow-panic; the saturated span keeps the
        // call panic-free and returns a structured result.
        let mut r = reader(&[0x00, 0x00, 0x00, 0x00]);
        let _ = decode_signed_subexp_with_ref(&mut r, i64::MIN, i64::MAX, 0, 1);
    }

    #[test]
    fn unsigned_subexp_extreme_recenter_does_not_panic() {
        // Public-API hardening (codex PR #64 review): an arbitrary r outside
        // i64::MIN/2..=i64::MAX/2 must not overflow the doubling comparison (run in
        // i128) or the mirrored recenter arm (wrapping, like inverse_recenter).
        let mut r = reader(&[0x00, 0x00, 0x00, 0x00]);
        let _ = decode_unsigned_subexp_with_ref(&mut r, i64::MAX, i64::MAX - 1, 1);
        let mut r2 = reader(&[0x00, 0x00, 0x00, 0x00]);
        let _ = decode_unsigned_subexp_with_ref(&mut r2, 8, i64::MIN, 1);
    }

    #[test]
    fn intra_returns_identity_no_bits() {
        // mirror :7792: FrameIsIntra -> return before use_global_motion (no bits).
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
        // mirror :7792: !enable_global_motion -> return (no bits).
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
        // mirror :7798-7804: use_global_motion f(1) == 0 -> return.
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
        // use_global_motion=1, our_ref ns(NumTotalRefs+1) selecting our_ref == NumTotalRefs:
        // the base-warp arm is skipped (baseParams stay default) and the parse stops at the
        // per-reference loop's OrderHints gate. NumTotalRefs=2 -> our_ref ns(3): w=2, m=4-3=1.
        // read w-1=1 bit; to get value 2 (== NumTotalRefs): v(1 bit) >= m(1) -> read extra bit;
        // value = (v<<1) - m + extra. v=1,extra=1 -> (2) - 1 + 1 = 2. Bits "1" then "1" = "11".
        let mut bits = Bits::default();
        bits.bit(1); // use_global_motion = 1
        bits.raw("11"); // our_ref ns(3) = 2 == NumTotalRefs
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let gm = parse_global_motion_params(&mut r, &base_input(&[0, 1])).unwrap();
        assert!(gm.use_global_motion);
        assert_eq!(gm.our_ref, Some(2));
        assert_eq!(gm.stop, Some(GlobalMotionStop::OrderHintsUnmodeled));
        // 1 (use_global_motion) + 2 (our_ref ns(3) = 2) = 3.
        assert_eq!(r.consumed_bits(), 3);
    }

    #[test]
    fn our_ref_not_num_total_refs_stops_at_ref_num_total_refs() {
        // our_ref ns(3) selecting our_ref=0 (!= NumTotalRefs=2): the RefNumTotalRefs arm is
        // entered but RefNumTotalRefs[refIdx] is cross-frame -> honest stop. ns(3): w=2,
        // m=1. read 1 bit = 0; 0 < m(1) -> value = 0. Bits "0".
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
        // mirror :7814-7816: SWITCH_FRAME -> our_ref = NumTotalRefs (no ns bits). Then the
        // OrderHints loop boundary stop. Only the use_global_motion f(1) is read.
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
        // ns(NumTotalRefs+1) boundary: NumTotalRefs=1 -> ns(2): w=1, m=2-2=0. read w-1=0
        // bits, then since 0 >= m(0) read the extra bit; value = (0<<1) - 0 + extra = extra.
        // extra=1 -> our_ref=1 == NumTotalRefs -> OrderHints stop. extra=0 -> our_ref=0 !=
        // NumTotalRefs -> RefNumTotalRefs stop.
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
        // The payload ends before use_global_motion f(1) — a truncation the caller routes.
        let mut r = reader(&[]);
        assert!(matches!(
            parse_global_motion_params(&mut r, &base_input(&[0, 1])),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn eof_inside_our_ref_ns_is_error() {
        // Position the reader so use_global_motion f(1) reads the LAST bit of a 1-byte
        // payload; the our_ref ns(8) (NumTotalRefs=7, w=3) read then starts at EOF and
        // overruns. The 7 leading pad bits are consumed by the test before the parse so the
        // reader sits exactly before use_global_motion.
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
        // At any honest stop the per-reference table is the identity initialiser (mirror
        // :7780-7788) — the warp loop is never entered without OrderHints.
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
            num_syms in any::<i64>(),
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
            low in -100_000i64..100_000,
            span in 1i64..200_000,
            r in -200_000i64..200_000,
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
            let _ = read_global_param(&mut reader, idx, i64::from(prev));
        }

        /// inverse_recenter must never panic over arbitrary i64 r / v.
        #[test]
        fn inverse_recenter_never_panics(r in any::<i64>(), v in any::<i64>()) {
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
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_global_motion_params(&mut reader, &input);
        }
    }
}
