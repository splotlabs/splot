// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.14.2 dequantization quantizer functions.
//!
//! This module implements the scheduler-free AV2 § 7.14.2 quantizer functions
//! used by the dequantization process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-14-2`): the `Ac_Qlookup` base table, the `qlookup` shift-extension
//! function, the `MaxQ` derivation from the active decoded bit depth
//! (§ 6.4.1 Table 6.3), `get_q` ([`quantizer_value`]) mapping a resolved
//! quantizer index and a signed delta to a quantizer value, `get_qindex`
//! ([`quantizer_index`]) resolving the per-block quantizer index from
//! caller-supplied frame and segment facts, and the per-plane `get_dc_quant` /
//! `get_ac_quant` composition ([`dc_quantizer`] / [`ac_quantizer`]).
//!
//! Feature tracking: `RECON-DEQUANT-QUANTIZER-LOOKUP`,
//! `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION`.
//!
//! Scope: every function takes caller-resolved inputs and never reads frame,
//! segment, or tile state. `get_qindex`'s segmentation and `delta_q` evaluation
//! (`seg_feature_active_idx`, the `FeatureData` array, the `CurrentQIndex`
//! update) stays with the caller, which passes the already-resolved facts. The
//! § 7.14.4 dequantization process that applies these quantizers (with
//! quantizer-matrix weighting) to coded coefficients, the § 7.14.3 reconstruct
//! process, inverse transforms, and residual addition are out of scope and
//! tracked by their own future rows.

use crate::{BitDepth, PlaneId};

/// AV2 § 7.14.2 `Ac_Qlookup` base quantizer table (25 entries).
///
/// Spec:
/// [`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
/// `#s-7-14-2`.
#[rustfmt::skip]
const AC_QLOOKUP: [u16; 25] = [
    64, 40, 41, 43, 44, 45, 47, 48, 49, 51, 52,
    54, 55, 57, 59, 60, 62, 64, 66, 68, 70, 72,
    74, 76, 78,
];

/// AV2 § 3 `MAXQ_8_BITS`: maximum quantizer index when bit depth is 8.
const MAXQ_8_BITS: u32 = 255;
/// AV2 § 3 `MAXQ_OFFSET`: quantizer-index increase per bit-depth step.
const MAXQ_OFFSET: u32 = 24;
/// AV2 § 3 `MAXQ_10_BITS`: maximum quantizer index when bit depth is 10.
const MAXQ_10_BITS: u32 = MAXQ_8_BITS + 2 * MAXQ_OFFSET;

/// Returns the AV2 § 6.4.1 Table 6.3 `MaxQ` for the active decoded bit depth.
///
/// 8-bit decoded output uses `MAXQ_8_BITS` (255) and 10-bit decoded output uses
/// `MAXQ_10_BITS` (303); AV2 v1.0.0 Table 6.3 defines no other decoded bit
/// depth (`bit_depth_idc` greater than 1 is reserved), which is why
/// [`BitDepth`] models only those two cases.
///
/// This is the spec `MaxQ` used to clamp quantizer indices in
/// [`quantizer_value`]; it is distinct from `MAXQ_BITS`, the bit-depth-agnostic
/// segmentation feature ceiling.
#[must_use]
pub const fn max_quantizer_index(bit_depth: BitDepth) -> u32 {
    match bit_depth {
        BitDepth::Eight => MAXQ_8_BITS,
        BitDepth::Ten => MAXQ_10_BITS,
    }
}

/// AV2 § 7.14.2 `qlookup( q )`: the shift-extended base quantizer value.
///
/// For `q < 25` the value is `Ac_Qlookup[q]`; otherwise it is
/// `Ac_Qlookup[((q - 1) % 24) + 1] << ((q - 1) / 24)`.
///
/// Callers pass `q` in `0..=MaxQ` for the active bit depth ([`quantizer_value`]
/// guarantees this by clamping before calling), so for the largest defined
/// `MaxQ` (303) the shift is at most 12 and the result is exact. For any
/// out-of-contract `q` whose shift would reach or exceed the `u32` width the
/// result saturates to `u32::MAX` via `checked_shl`, so the function is total
/// and never panics regardless of caller input.
fn qlookup(q: u32) -> u32 {
    if q < 25 {
        u32::from(AC_QLOOKUP[q as usize])
    } else {
        let index = ((q - 1) % 24 + 1) as usize;
        let shift = (q - 1) / 24;
        u32::from(AC_QLOOKUP[index])
            .checked_shl(shift)
            .unwrap_or(u32::MAX)
    }
}

/// AV2 § 7.14.2 `get_q( qindex, delta )`: a quantizer value for the active bit
/// depth.
///
/// `qindex` is the resolved quantizer index from the § 7.14.2 `get_qindex`
/// process (in `0..=MaxQ`), and `delta` is the signed per-plane DC or AC delta
/// that `get_dc_quant` / `get_ac_quant` would add. The result is `Ac_Qlookup[0]`
/// when `qindex` is 0 and `delta` is non-positive; otherwise it is `qlookup`
/// of `qindex + delta` clamped to `1..=MaxQ`.
///
/// The clamp arithmetic uses `i64` intermediates so any caller value (including
/// out-of-contract `qindex` or `delta` extremes) produces a clamped quantizer
/// value rather than overflowing or panicking.
#[must_use]
pub fn quantizer_value(qindex: u32, delta: i32, bit_depth: BitDepth) -> u32 {
    if qindex == 0 && delta <= 0 {
        return u32::from(AC_QLOOKUP[0]);
    }
    let max = i64::from(max_quantizer_index(bit_depth));
    let clamped = (i64::from(qindex) + i64::from(delta)).clamp(1, max);
    // `clamped` is within `1..=MaxQ` (at most 303), so the cast cannot truncate.
    qlookup(clamped as u32)
}

/// AV2 § 7.14.2 `get_qindex( ignoreDeltaQ, segmentId )`: the resolved quantizer
/// index for the current block.
///
/// The segmentation and `delta_q` facts are resolved by the caller (this crate
/// holds no frame, segment, or tile state):
/// - `base_q_idx` is the frame base quantizer index.
/// - `current_q_index` is the running `CurrentQIndex` (already clamped to
///   `1..=MaxQ` where the caller maintains it).
/// - `segment_alt_q_active` is `seg_feature_active_idx( segmentId, SEG_LVL_ALT_Q )`.
/// - `segment_alt_q_data` is `FeatureData[ segmentId ][ SEG_LVL_ALT_Q ]`.
/// - `delta_q_present` is the frame `delta_q_present` flag.
/// - `ignore_delta_q` is the spec `ignoreDeltaQ` argument.
///
/// When the alternative-quantizer segment feature is active, the index is
/// `Clip3(0, MaxQ, base + segment_alt_q_data)` where `base` is `current_q_index`
/// if `delta_q` applies (`!ignore_delta_q && delta_q_present`) and `base_q_idx`
/// otherwise. When the feature is inactive, the result is `current_q_index` if
/// `delta_q` applies and `base_q_idx` otherwise (returned unclamped, matching the
/// spec, since those inputs are already in range where the caller maintains
/// them). The clamp arithmetic uses `i64` intermediates so all inputs are total
/// and panic-free.
#[must_use]
pub fn quantizer_index(
    base_q_idx: u32,
    current_q_index: u32,
    segment_alt_q_active: bool,
    segment_alt_q_data: i32,
    delta_q_present: bool,
    ignore_delta_q: bool,
    bit_depth: BitDepth,
) -> u32 {
    let delta_q_applies = !ignore_delta_q && delta_q_present;
    if segment_alt_q_active {
        let base = if delta_q_applies {
            i64::from(current_q_index)
        } else {
            i64::from(base_q_idx)
        };
        let max = i64::from(max_quantizer_index(bit_depth));
        let qindex = (base + i64::from(segment_alt_q_data)).clamp(0, max);
        // `qindex` is within `0..=MaxQ` (at most 303), so the cast cannot truncate.
        qindex as u32
    } else if delta_q_applies {
        current_q_index
    } else {
        base_q_idx
    }
}

/// AV2 § 7.14.2 resolved per-plane DC and AC quantizer delta offsets.
///
/// Each field is the caller-resolved sum that the spec's `get_dc_quant` /
/// `get_ac_quant` add to the quantizer index. The luma AC delta is always 0 per
/// § 7.14.2 and is therefore not stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuantizerDeltas {
    /// Y plane DC delta: `DeltaQYDc + BaseYDcDeltaQ`.
    pub y_dc: i32,
    /// U plane DC delta: `DeltaQUDc + BaseUVDcDeltaQ`.
    pub u_dc: i32,
    /// V plane DC delta: `DeltaQVDc + BaseUVDcDeltaQ`.
    pub v_dc: i32,
    /// U plane AC delta: `DeltaQUAc + BaseUVAcDeltaQ`.
    pub u_ac: i32,
    /// V plane AC delta: `DeltaQVAc + BaseUVAcDeltaQ`.
    pub v_ac: i32,
}

/// AV2 § 7.14.2 `get_dc_quant( plane )`: the DC quantizer value for a plane.
///
/// Selects the plane's DC delta from `deltas` and applies [`quantizer_value`] to
/// the resolved `qindex` (the [`quantizer_index`] output).
#[must_use]
pub fn dc_quantizer(
    plane: PlaneId,
    qindex: u32,
    deltas: QuantizerDeltas,
    bit_depth: BitDepth,
) -> u32 {
    let delta = match plane {
        PlaneId::Y => deltas.y_dc,
        PlaneId::U => deltas.u_dc,
        PlaneId::V => deltas.v_dc,
    };
    quantizer_value(qindex, delta, bit_depth)
}

/// AV2 § 7.14.2 `get_ac_quant( plane )`: the AC quantizer value for a plane.
///
/// The luma AC delta is 0 per § 7.14.2; chroma selects its AC delta from
/// `deltas`. The result applies [`quantizer_value`] to the resolved `qindex`
/// (the [`quantizer_index`] output).
#[must_use]
pub fn ac_quantizer(
    plane: PlaneId,
    qindex: u32,
    deltas: QuantizerDeltas,
    bit_depth: BitDepth,
) -> u32 {
    let delta = match plane {
        PlaneId::Y => 0,
        PlaneId::U => deltas.u_ac,
        PlaneId::V => deltas.v_ac,
    };
    quantizer_value(qindex, delta, bit_depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_quantizer_index_matches_table_6_3() {
        assert_eq!(max_quantizer_index(BitDepth::Eight), 255);
        assert_eq!(max_quantizer_index(BitDepth::Ten), 303);
    }

    #[test]
    fn qlookup_returns_table_for_small_q() {
        assert_eq!(qlookup(0), 64);
        assert_eq!(qlookup(1), 40);
        assert_eq!(qlookup(10), 52);
        assert_eq!(qlookup(24), 78);
    }

    #[test]
    fn qlookup_applies_shift_extension_for_large_q() {
        // q = 25: index ((24 % 24) + 1) = 1 -> 40, shift 24 / 24 = 1 -> 40 << 1.
        assert_eq!(qlookup(25), 80);
        // q = 49: index 1 -> 40, shift 2 -> 40 << 2.
        assert_eq!(qlookup(49), 160);
        // q = 255 (8-bit MaxQ): index 15 -> 60, shift 10 -> 60 << 10.
        assert_eq!(qlookup(255), 61_440);
        // q = 303 (10-bit MaxQ): index 15 -> 60, shift 12 -> 60 << 12.
        assert_eq!(qlookup(303), 245_760);
    }

    #[test]
    fn qlookup_is_total_beyond_contract() {
        // Out-of-contract q whose shift reaches the u32 width must saturate,
        // not panic. q = 769 -> shift (768 / 24) = 32 -> checked_shl is None.
        assert_eq!(qlookup(769), u32::MAX);
        assert_eq!(qlookup(u32::MAX), u32::MAX);
    }

    #[test]
    fn quantizer_value_uses_zero_index_special_case() {
        // qindex == 0 and delta <= 0 returns Ac_Qlookup[0] regardless of delta.
        assert_eq!(quantizer_value(0, 0, BitDepth::Eight), 64);
        assert_eq!(quantizer_value(0, -255, BitDepth::Eight), 64);
        // qindex == 0 with a positive delta is not the special case.
        assert_eq!(quantizer_value(0, 3, BitDepth::Eight), qlookup(3));
    }

    #[test]
    fn quantizer_value_adds_delta_then_looks_up() {
        assert_eq!(quantizer_value(10, 0, BitDepth::Eight), qlookup(10));
        assert_eq!(quantizer_value(20, 5, BitDepth::Eight), qlookup(25));
    }

    #[test]
    fn quantizer_value_clamps_into_one_through_max_q() {
        // Low clamp: a large negative delta clamps the index up to 1.
        assert_eq!(quantizer_value(5, -100, BitDepth::Eight), qlookup(1));
        // High clamp depends on the bit-depth-specific MaxQ.
        assert_eq!(quantizer_value(255, 48, BitDepth::Eight), qlookup(255));
        assert_eq!(quantizer_value(255, 48, BitDepth::Ten), qlookup(303));
    }

    #[test]
    fn quantizer_value_is_panic_free_at_input_extremes() {
        // Out-of-contract extremes must clamp, not overflow or panic. A huge
        // index clamps up to MaxQ; a small index plus the most negative delta
        // clamps down to 1.
        assert_eq!(
            quantizer_value(u32::MAX, i32::MAX, BitDepth::Ten),
            qlookup(303)
        );
        assert_eq!(quantizer_value(1, i32::MIN, BitDepth::Eight), qlookup(1));
    }

    // Convenience: the common ignore_delta_q=false / delta_q_present=false form.
    fn qindex(
        base_q_idx: u32,
        current_q_index: u32,
        seg_active: bool,
        seg_data: i32,
        delta_q_present: bool,
        ignore_delta_q: bool,
        bit_depth: BitDepth,
    ) -> u32 {
        quantizer_index(
            base_q_idx,
            current_q_index,
            seg_active,
            seg_data,
            delta_q_present,
            ignore_delta_q,
            bit_depth,
        )
    }

    #[test]
    fn quantizer_index_baseline_returns_base_q_idx() {
        // Segment feature off and delta_q not applied -> base_q_idx, unclamped.
        assert_eq!(qindex(40, 99, false, 0, false, false, BitDepth::Eight), 40);
        assert_eq!(qindex(40, 99, false, 0, true, true, BitDepth::Eight), 40);
    }

    #[test]
    fn quantizer_index_delta_q_returns_current_q_index() {
        // Segment feature off, delta_q applied -> CurrentQIndex, unclamped.
        assert_eq!(qindex(40, 99, false, 0, true, false, BitDepth::Eight), 99);
    }

    #[test]
    fn quantizer_index_segment_feature_uses_base_then_clips() {
        // Segment active, delta_q not applied -> Clip3(0, MaxQ, base_q_idx + data).
        assert_eq!(qindex(40, 99, true, 5, false, false, BitDepth::Eight), 45);
        // Segment active, delta_q applied -> Clip3(0, MaxQ, CurrentQIndex + data).
        assert_eq!(qindex(40, 99, true, 5, true, false, BitDepth::Eight), 104);
        // ignore_delta_q forces the base_q_idx form even when delta_q_present.
        assert_eq!(qindex(40, 99, true, 5, true, true, BitDepth::Eight), 45);
    }

    #[test]
    fn quantizer_index_segment_feature_clips_both_bounds() {
        // Low clamp to 0 (the lossless-segment approach with data = -255).
        assert_eq!(qindex(10, 0, true, -255, false, false, BitDepth::Eight), 0);
        // High clamp to the bit-depth-specific MaxQ.
        assert_eq!(
            qindex(255, 0, true, 255, false, false, BitDepth::Eight),
            255
        );
        assert_eq!(qindex(255, 0, true, 255, false, false, BitDepth::Ten), 303);
    }

    #[test]
    fn quantizer_index_is_panic_free_at_input_extremes() {
        // Only the segment-feature branch clamps; extremes must not overflow.
        assert_eq!(
            qindex(u32::MAX, 0, true, i32::MAX, false, false, BitDepth::Ten),
            303
        );
        assert_eq!(
            qindex(0, 0, true, i32::MIN, false, false, BitDepth::Eight),
            0
        );
    }

    #[test]
    fn dc_quantizer_selects_plane_delta() {
        let deltas = QuantizerDeltas {
            y_dc: 1,
            u_dc: 2,
            v_dc: 3,
            u_ac: 4,
            v_ac: 5,
        };
        assert_eq!(
            dc_quantizer(PlaneId::Y, 50, deltas, BitDepth::Eight),
            quantizer_value(50, 1, BitDepth::Eight)
        );
        assert_eq!(
            dc_quantizer(PlaneId::U, 50, deltas, BitDepth::Eight),
            quantizer_value(50, 2, BitDepth::Eight)
        );
        assert_eq!(
            dc_quantizer(PlaneId::V, 50, deltas, BitDepth::Eight),
            quantizer_value(50, 3, BitDepth::Eight)
        );
    }

    #[test]
    fn ac_quantizer_uses_zero_for_luma_and_delta_for_chroma() {
        let deltas = QuantizerDeltas {
            y_dc: 1,
            u_dc: 2,
            v_dc: 3,
            u_ac: 4,
            v_ac: 5,
        };
        // Luma AC delta is always 0 per §7.14.2.
        assert_eq!(
            ac_quantizer(PlaneId::Y, 50, deltas, BitDepth::Eight),
            quantizer_value(50, 0, BitDepth::Eight)
        );
        assert_eq!(
            ac_quantizer(PlaneId::U, 50, deltas, BitDepth::Eight),
            quantizer_value(50, 4, BitDepth::Eight)
        );
        assert_eq!(
            ac_quantizer(PlaneId::V, 50, deltas, BitDepth::Eight),
            quantizer_value(50, 5, BitDepth::Eight)
        );
    }

    #[test]
    fn quantizer_composition_reaches_zero_index_special_case() {
        // qindex resolving to 0 plus a non-positive plane delta yields
        // Ac_Qlookup[0] (64) end-to-end through quantizer_value's special case.
        let deltas = QuantizerDeltas {
            y_dc: 0,
            u_dc: -10,
            v_dc: 0,
            u_ac: 0,
            v_ac: 0,
        };
        let q = qindex(0, 0, false, 0, false, false, BitDepth::Eight);
        assert_eq!(q, 0);
        assert_eq!(dc_quantizer(PlaneId::U, q, deltas, BitDepth::Eight), 64);
        assert_eq!(ac_quantizer(PlaneId::Y, q, deltas, BitDepth::Eight), 64);
    }
}
