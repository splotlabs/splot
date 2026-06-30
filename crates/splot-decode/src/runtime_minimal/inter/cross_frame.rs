// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

const PRIMARY_REF_NONE: u8 = 7;
const PRIMARY_REF_CHOOSE: u8 = 8;
const INITIAL_QP_DIFF: i64 = 512;

/// Resolved saved-CDF source for AV2 § 5 `set_primary_ref_frame_and_ctx`.
pub(super) enum ResolvedCdfLoad {
    /// The frame initializes CDFs from defaults.
    Default,
    /// The frame loads saved CDFs from a primary reference slot and may blend a secondary.
    LoadSlot {
        /// Resolved reference-buffer slot for the primary CDF load.
        primary: u32,
        /// Resolved reference-buffer slot for the secondary CDF blend.
        blend: Option<u32>,
    },
    /// A signalled real primary reference is outside `ref_frame_idx`.
    OutOfRangePrimary,
}

/// Resolves the saved-CDF slot AV2 § 5 would load, including `PRIMARY_REF_CHOOSE`.
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

/// Returns AV2 § 5 primary and secondary `ref_frame_idx` indices for CDF loading.
#[allow(clippy::too_many_arguments)]
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
    let mut primary = RankedRef::NONE;
    let mut secondary = RankedRef::NONE;
    for (i, &slot) in ref_frame_idx.iter().enumerate() {
        let slot = slot as usize;
        if ref_is_inter.get(slot).copied() != Some(true) {
            continue;
        }
        let candidate = RankedRef::from_reference(
            i,
            slot,
            ref_base_q_idx,
            ref_order_hint,
            ref_frame_width,
            ref_frame_height,
            current_base_q_idx,
        );
        if candidate.beats(primary, current_order_hint) {
            secondary = primary;
            primary = candidate;
        } else if candidate.beats(secondary, current_order_hint) {
            secondary = candidate;
        }
    }
    if signal_primary_ref_frame == Some(true) {
        let signalled = primary_ref_frame.unwrap_or(PRIMARY_REF_NONE);
        if signalled == PRIMARY_REF_NONE {
            primary = RankedRef::NONE;
            secondary = RankedRef::NONE;
        } else if signalled != primary.index {
            if secondary.index == PRIMARY_REF_NONE || secondary.index == signalled {
                secondary = primary;
            }
            primary.index = signalled;
        }
    }
    (primary.index, secondary.index)
}

#[derive(Clone, Copy)]
struct RankedRef {
    index: u8,
    qp_diff: i64,
    order_hint: i32,
    ratio: i32,
}

impl RankedRef {
    const NONE: Self = Self {
        index: PRIMARY_REF_NONE,
        qp_diff: INITIAL_QP_DIFF,
        order_hint: 0,
        ratio: 0,
    };

    fn from_reference(
        index: usize,
        slot: usize,
        ref_base_q_idx: &[u32],
        ref_order_hint: &[u32],
        ref_frame_width: &[u32],
        ref_frame_height: &[u32],
        current_base_q_idx: u32,
    ) -> Self {
        let base_q_idx = ref_base_q_idx.get(slot).copied().unwrap_or(0);
        let order_hint =
            i32::try_from(ref_order_hint.get(slot).copied().unwrap_or(0)).unwrap_or(i32::MAX);
        let width = ref_frame_width.get(slot).copied().unwrap_or(0);
        let height = ref_frame_height.get(slot).copied().unwrap_or(0);
        Self {
            index: u8::try_from(index).unwrap_or(PRIMARY_REF_NONE),
            qp_diff: i64::from(base_q_idx.abs_diff(current_base_q_idx)),
            order_hint,
            ratio: floor_log2(u64::from(width) * u64::from(height)),
        }
    }

    fn beats(self, other: Self, current_order_hint: i32) -> bool {
        self.qp_diff < other.qp_diff
            || (self.qp_diff == other.qp_diff
                && is_ref_better(
                    current_order_hint,
                    self.order_hint,
                    other.order_hint,
                    self.ratio,
                    other.ratio,
                ))
    }
}

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

fn get_relative_dist(a: i32, b: i32) -> i32 {
    (a - b).clamp(-127, 127)
}

/// `FloorLog2(x)` (AV2 § 4): the index of the most-significant set bit, 0 for `x == 0`.
pub(super) fn floor_log2(x: u64) -> i32 {
    if x == 0 { 0 } else { x.ilog2() as i32 }
}

/// Returns whether stored reference order hints can be used without AV2 § 5.18.2 wrap fixup.
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
