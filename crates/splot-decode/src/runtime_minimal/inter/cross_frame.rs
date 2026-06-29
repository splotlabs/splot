// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5 / § 7.7 cross-frame reference-state resolution for the minimal inter decode.
//!
//! These helpers model the cross-frame state a later inter frame inherits from prior
//! decoded frames — the § 5 `set_primary_ref_frame_and_ctx` CDF-load decision (including
//! the `PRIMARY_REF_CHOOSE` resolution via `choose_primary_secondary_ref_frame`) and the
//! § 5.18.2 order-hint wrap check. The minimal decoder does NOT model § 7.23 cross-frame
//! CDF save/load, § 7.23 `SavedMvs`, or an order-hint-wrapped history, so the inter decode
//! uses these to REJECT (before any output) a stream that would inherit unmodeled
//! cross-frame state, rather than confidently mis-decoding it.
//!
//! Feature tracking: `DECODE-INTER-MULTIREF-RUNTIME`.

/// `PRIMARY_REF_NONE` (AV2 v1.0.0 § 3): values `0..PRIMARY_REF_NONE` (0..=6) are
/// resolved real references; `PRIMARY_REF_NONE` (7) and `PRIMARY_REF_CHOOSE` (8) are not.
const PRIMARY_REF_NONE: u8 = 7;
/// `PRIMARY_REF_CHOOSE` (AV2 v1.0.0 § 3): the value `primary_ref_frame` takes when
/// `signal_primary_ref_frame == 0` (§ 5.18.2 mirror :4397); resolved by
/// `set_primary_ref_frame_and_ctx` (mirror :5414-5415).
const PRIMARY_REF_CHOOSE: u8 = 8;

/// The reference slot whose saved CDFs a frame's `load_cdfs(ref_frame_idx[primary_ref_frame])`
/// would load, after AV2 § 5 `set_primary_ref_frame_and_ctx` resolution
/// (mirror `docs/spec/av2/1.0.0/05-syntax-structures.md` :5411-5430). [`Self::Default`] when
/// the frame loads NO cross-frame CDFs (the `init_non_coeff_cdfs()` arm, mirror :5426-5428).
pub(super) enum ResolvedCdfLoad {
    /// The frame initialises CDFs from defaults — `primary_ref_frame == PRIMARY_REF_NONE`
    /// (or resolved to it via `DerivedPrimaryRefFrame == PRIMARY_REF_NONE`), OR
    /// `disable_cross_frame_cdf_init == 1`. No saved-CDF inheritance.
    Default,
    /// The frame loads `ref_frame_idx[primary_ref_frame]`'s saved CDFs (`primary`), and —
    /// when the § 5 :5431-5439 `blend_cdfs` arm applies (`enable_avg_cdf && !avg_cdf_type`
    /// and `blendFrame != PRIMARY_REF_NONE`) — ALSO blends the secondary slot (`blend`).
    LoadSlot {
        /// The resolved § 7.23 buffer slot whose saved CDFs `load_cdfs` loads.
        primary: u32,
        /// The § 5 :5431-5439 `blend_cdfs` secondary slot, when a blend occurs; else `None`.
        blend: Option<u32>,
    },
    /// A signalled `primary_ref_frame` named a real reference (`< PRIMARY_REF_NONE`) that is
    /// `>= NumTotalRefs` (out of `ref_frame_idx` bounds) — a NON-conformant frame
    /// (§ 6.17.2 requires `primary_ref_frame < NumTotalRefs`). The caller rejects it rather
    /// than decoding from default CDFs, since the minimal path runs no later range rule.
    OutOfRangePrimary,
}

/// Models AV2 § 5 `set_primary_ref_frame_and_ctx`'s CDF-load decision (mirror :5411-5430),
/// including the `PRIMARY_REF_CHOOSE` resolution (mirror :5414-5415) via
/// `choose_primary_secondary_ref_frame` (mirror :5451-5510).
///
/// The minimal decoder does NOT model `load_cdfs` (every frame decodes from the default
/// `init_*_cdfs` state). To stay honest the caller resolves the actual loaded slot here and
/// rejects (rather than confidently mis-decoding from defaults) when that slot's saved CDFs
/// were ADAPTED. Resolving `PRIMARY_REF_CHOOSE` is required because a CHOOSE frame can
/// resolve to a real ADAPTED inter reference, which the prior `CHOOSE -> no load` shortcut
/// silently let bypass the guard.
///
/// `DerivedPrimaryRefFrame` is the §5 :5497-5508 signal-overridden value: when
/// `signal_primary_ref_frame == Some(true)` it is the explicitly-signalled
/// `primary_ref_frame` itself (the inter-only ranking is overridden — so a signalled
/// primary CAN change the resolved load slot, including to an adapted non-inter slot the
/// ranking would not pick); when unsignalled it is the
/// `choose_primary_secondary_ref_frame` ranking (mirror :5468-5495), which scores ONLY
/// `RefFrameType == INTER_FRAME` slots (`ref_is_inter`), excludes `RESTRICTED_OH`
/// (not produced here), and `first_slot_with_ref` is a no-op (distinct slots), so a key /
/// intra-only reference history resolves to `PRIMARY_REF_NONE` (the [`ResolvedCdfLoad::Default`]
/// arm) — exactly the committed 2-frame fixtures (unsignalled CHOOSE, KEY-only valid slot).
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_cdf_load(
    signal_primary_ref_frame: Option<bool>,
    primary_ref_frame: Option<u8>,
    disable_cross_frame_cdf_init: Option<bool>,
    ref_frame_idx: &[u32],
    ref_is_inter: &[bool],
    ref_base_q_idx: &[u32],
    ref_order_hint: &[u32],
    ref_frame_width: &[u32],
    ref_frame_height: &[u32],
    current_base_q_idx: u32,
    current_order_hint: i32,
    enable_avg_cdf: bool,
    avg_cdf_type: u8,
) -> ResolvedCdfLoad {
    let (derived, derived_secondary) = choose_primary_secondary_ref_frame(
        signal_primary_ref_frame,
        primary_ref_frame,
        ref_frame_idx,
        ref_is_inter,
        ref_base_q_idx,
        ref_order_hint,
        ref_frame_width,
        ref_frame_height,
        current_base_q_idx,
        current_order_hint,
    );
    let mut primary = match primary_ref_frame {
        Some(PRIMARY_REF_CHOOSE) => derived,
        Some(p) => p,
        None => PRIMARY_REF_NONE,
    };
    let mut cross_frame_init_disabled = disable_cross_frame_cdf_init == Some(true);
    if derived == PRIMARY_REF_NONE || primary == PRIMARY_REF_NONE {
        primary = PRIMARY_REF_NONE;
        cross_frame_init_disabled = true;
    }
    if primary == PRIMARY_REF_NONE || cross_frame_init_disabled {
        return ResolvedCdfLoad::Default;
    }
    let Some(&primary_slot) = ref_frame_idx.get(primary as usize) else {
        return ResolvedCdfLoad::OutOfRangePrimary;
    };
    let blend = if enable_avg_cdf && avg_cdf_type == 0 {
        let blend_frame = if primary == derived {
            derived_secondary
        } else {
            derived
        };
        if blend_frame == PRIMARY_REF_NONE {
            None
        } else {
            ref_frame_idx.get(blend_frame as usize).copied()
        }
    } else {
        None
    };
    ResolvedCdfLoad::LoadSlot {
        primary: primary_slot,
        blend,
    }
}

/// `choose_primary_secondary_ref_frame()` (AV2 § 5 mirror :5451-5510), returning
/// `(DerivedPrimaryRefFrame, derivedSecondaryRefFrame)` as `primary` / `secondary` slot
/// indices into `ref_frame_idx` (or [`PRIMARY_REF_NONE`]).
///
/// This is the inter-only, single-spatial-layer reduction the minimal subset needs: the
/// loop scores each `i` in `0..NumTotalRefs` whose `ref_frame_idx[i]` slot is
/// `RefFrameType == INTER_FRAME` (mirror :5470 — a key / intra-only reference is skipped),
/// by `qpDiff = Abs(RefBaseQIdx - base_q_idx)` then the `is_ref_better` order-hint
/// tie-break (mirror :5476-5495), tracking BOTH the best (primary) and second-best
/// (secondary) candidate. `first_slot_with_ref` (distinct slots) and the `RESTRICTED_OH`
/// exclusion are no-ops for the verified subset. The `signal_primary_ref_frame` tail
/// (mirror :5497-5508) UNCONDITIONALLY overrides the derived primary to the signalled
/// `primary_ref_frame` (demoting the ranking primary to secondary), so a signalled frame's
/// derived primary is its signalled value even with no inter ranking candidate.
#[allow(clippy::too_many_arguments, clippy::many_single_char_names)]
pub(super) fn choose_primary_secondary_ref_frame(
    signal_primary_ref_frame: Option<bool>,
    primary_ref_frame: Option<u8>,
    ref_frame_idx: &[u32],
    ref_is_inter: &[bool],
    ref_base_q_idx: &[u32],
    ref_order_hint: &[u32],
    ref_frame_width: &[u32],
    ref_frame_height: &[u32],
    current_base_q_idx: u32,
    current_order_hint: i32,
) -> (u8, u8) {
    let mut primary: u8 = PRIMARY_REF_NONE;
    let mut primary_qp_diff: i64 = 512;
    let mut primary_d: i32 = 0;
    let mut primary_ratio: i32 = 0;
    let mut secondary: u8 = PRIMARY_REF_NONE;
    let mut secondary_qp_diff: i64 = 512;
    let mut secondary_d: i32 = 0;
    let mut secondary_ratio: i32 = 0;
    for (i, &slot) in ref_frame_idx.iter().enumerate() {
        let slot = slot as usize;
        if ref_is_inter.get(slot).copied() != Some(true) {
            continue;
        }
        let q = ref_base_q_idx.get(slot).copied().unwrap_or(0);
        let d = i32::try_from(ref_order_hint.get(slot).copied().unwrap_or(0)).unwrap_or(i32::MAX);
        let (w, h) = (
            ref_frame_width.get(slot).copied().unwrap_or(0),
            ref_frame_height.get(slot).copied().unwrap_or(0),
        );
        let d_ratio = floor_log2(u64::from(w) * u64::from(h));
        let qp_diff = i64::from(q.abs_diff(current_base_q_idx));
        let i = u8::try_from(i).unwrap_or(PRIMARY_REF_NONE);
        if qp_diff < primary_qp_diff
            || (qp_diff == primary_qp_diff
                && is_ref_better(current_order_hint, d, primary_d, d_ratio, primary_ratio))
        {
            secondary = primary;
            secondary_qp_diff = primary_qp_diff;
            secondary_d = primary_d;
            secondary_ratio = primary_ratio;
            primary = i;
            primary_qp_diff = qp_diff;
            primary_d = d;
            primary_ratio = d_ratio;
        } else if qp_diff < secondary_qp_diff
            || (qp_diff == secondary_qp_diff
                && is_ref_better(current_order_hint, d, secondary_d, d_ratio, secondary_ratio))
        {
            secondary = i;
            secondary_qp_diff = qp_diff;
            secondary_d = d;
            secondary_ratio = d_ratio;
        }
    }
    if signal_primary_ref_frame == Some(true) {
        let signalled = primary_ref_frame.unwrap_or(PRIMARY_REF_NONE);
        if signalled == PRIMARY_REF_NONE {
            primary = PRIMARY_REF_NONE;
            secondary = PRIMARY_REF_NONE;
        } else if signalled != primary {
            if secondary == PRIMARY_REF_NONE || secondary == signalled {
                secondary = primary;
            }
            primary = signalled;
        }
    }
    (primary, secondary)
}

/// `is_ref_better(refDisp, bestDisp, refRatio, bestRatio)` (AV2 § 5 mirror :5512-5522).
fn is_ref_better(
    order_hint: i32,
    ref_disp: i32,
    best_disp: i32,
    ref_ratio: i32,
    best_ratio: i32,
) -> bool {
    let d0 = get_relative_dist(order_hint, ref_disp).abs() - (ref_ratio << 1);
    let d1 = get_relative_dist(order_hint, best_disp).abs() - (best_ratio << 1);
    if d0 < d1 {
        return true;
    }
    d0 == d1 && get_relative_dist(ref_disp, best_disp) > 0
}

/// `get_relative_dist(a, b)` (AV2 § 5.18.3.1 mirror :5565-5575) without the `RESTRICTED_OH`
/// sentinel arms (the verified subset never stores a restricted reference).
fn get_relative_dist(a: i32, b: i32) -> i32 {
    (a - b).clamp(-127, 127)
}

/// `FloorLog2(x)` (AV2 § 4): the index of the most-significant set bit, 0 for `x == 0`.
pub(super) fn floor_log2(x: u64) -> i32 {
    if x == 0 { 0 } else { x.ilog2() as i32 }
}

/// Whether the order-hint history (the currently-valid slots' stored `RefOrderHint` plus
/// this frame's `next_order_hint_lsb`) is provably NON-wrapping for `order_hint_bits`, so
/// the stored `RefOrderHint` (= `OrderHintLsbs`) equals the unwrapped `OrderHint`
/// (`get_disp_order_hint()`, AV2 § 5.18.2 mirror :5368-5381) for every frame.
///
/// `get_disp_order_hint()` (AV2 § 5.18.2 mirror :5368-5381) applies its wrap correction to
/// THIS frame's LSB whenever `maxDisp - (window >> 1) - OrderHintLsbs >= 0`, i.e. once the
/// max prior valid reference's order hint exceeds this frame's LSB by at least HALF a window
/// (`window == 1 << OrderHintBits`) — a DIRECTIONAL (wrap-back) condition: a small LSB after
/// larger prior hints. A forward frame (`currentLSB >= maxDisp`) is never corrected, so a
/// large FORWARD span is exact and admitted; only a `monotonic_output_order_flag == 0`
/// wrap-back is corrected and would mis-order the § 7.7 ranking. Each prior reference was
/// itself checked non-wrapping at its own decode (so its stored hint is exact); the § 7.7
/// `get_relative_dist` then merely clamps, so only this frame vs the max prior hint matters.
/// Returns `true` iff `maxDisp - next_order_hint_lsb < (1 << order_hint_bits) / 2`.
/// `order_hint_bits == 0` (no order-hint signaling) or no prior reference is non-wrapping.
pub(super) fn order_hint_history_unwrapped(
    ref_valid: &[bool],
    ref_order_hint: &[u32],
    order_hint_bits: u32,
    next_order_hint_lsb: u32,
) -> bool {
    if order_hint_bits == 0 {
        return true;
    }
    let half_window = 1u32 << (order_hint_bits.min(31).saturating_sub(1));
    let max_prior = ref_valid
        .iter()
        .enumerate()
        .filter(|(_, valid)| **valid)
        .map(|(i, _)| ref_order_hint.get(i).copied().unwrap_or(0))
        .max();
    match max_prior {
        Some(max_disp) => max_disp.saturating_sub(next_order_hint_lsb) < half_window,
        None => true,
    }
}
