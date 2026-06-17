// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.14.2 dequantization quantizer-value lookup.
//!
//! This module implements the scheduler-free AV2 § 7.14.2 quantizer lookup core
//! used by the dequantization process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-14-2`): the `Ac_Qlookup` base table, the `qlookup` shift-extension
//! function, the `MaxQ` derivation from the active decoded bit depth
//! (§ 6.4.1 Table 6.3), and `get_q`, which maps a resolved quantizer index and a
//! signed per-plane delta to a quantizer value.
//!
//! Feature tracking: `RECON-DEQUANT-QUANTIZER-LOOKUP`.
//!
//! Scope: this is the § 7.14.2 lookup core only. The § 7.14.2 `get_qindex`
//! segment / `delta_q` index resolution, the per-plane `get_dc_quant` /
//! `get_ac_quant` composition, and the § 7.14.4 dequantization process that
//! applies these quantizers (with quantizer-matrix weighting) to coded
//! coefficients are out of scope and tracked by their own future rows. The
//! functions take caller-resolved inputs and never read frame, segment, or
//! tile state.

use crate::BitDepth;

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
}
