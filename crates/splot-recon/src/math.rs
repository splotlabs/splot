// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 4.8 mathematical functions shared across reconstruction primitives
//! (`docs/spec/av2/1.0.0/04-conventions.md#s-4-8`).
//!
//! These are the spec's integer-operation definitions of `Round2`,
//! `Round2Signed`, and `Clip3` over `i64`, given one citable home so the
//! reconstruction, deblocking, restoration, secondary-transform, dequant, and
//! motion-compensation paths do not each re-derive them. Per-file `FloorLog2`
//! helpers and the `isize`-typed, value-first `Clip3` specializations in loop
//! restoration and chroma WienerNS keep their own definitions: they have
//! incompatible signatures, not duplicated logic.

/// AV2 § 4.8 `Round2` evaluated in `i128` so the public `i64` wrappers stay
/// total and panic-free for any argument. The rounding add cannot overflow
/// `i128` for in-`i64` magnitudes, and a shift at or above the `i128` width
/// saturates to the arithmetic-shift limit rather than shifting past the type
/// width. The codec only ever passes a small `n`; the guards exist so untrusted
/// public callers cannot trigger an overflow-checked panic.
const fn round2_i128(value: i128, n: u32) -> i128 {
    if n == 0 {
        value
    } else if n >= i128::BITS {
        value >> (i128::BITS - 1)
    } else {
        (value + (1i128 << (n - 1))) >> n
    }
}

/// AV2 § 4.8 `Round2(value, n)` using the spec's integer-operation definition:
/// `value` when `n == 0`, otherwise `(value + (1 << (n - 1))) >> n`
/// (`docs/spec/av2/1.0.0/04-conventions.md#s-4-8`, eq. 6).
///
/// The shift is arithmetic, so a negative `value` rounds toward negative
/// infinity, matching the spec's floored division. Total and panic-free for
/// every `(value, n)`: the rounding add is widened to `i128` so it cannot
/// overflow near `i64::MAX`, and an out-of-range `n` saturates instead of
/// shifting past the type width.
pub const fn round2(value: i64, n: u32) -> i64 {
    round2_i128(value as i128, n) as i64
}

/// AV2 § 4.8 `Round2Signed(value, n)`: `Round2(value, n)` for `value >= 0` and
/// `-Round2(-value, n)` for `value < 0`
/// (`docs/spec/av2/1.0.0/04-conventions.md#s-4-8`, eq. 7).
///
/// Total and panic-free for every `(value, n)`, including `i64::MIN` (whose
/// `i64` negation would overflow): the magnitude and the final negation are
/// taken in `i128`.
pub const fn round2_signed(value: i64, n: u32) -> i64 {
    let widened = value as i128;
    let magnitude = if widened < 0 { -widened } else { widened };
    let rounded = round2_i128(magnitude, n);
    (if widened < 0 { -rounded } else { rounded }) as i64
}

/// AV2 § 4.8 `Clip3(low, high, value)` (spec `Clip3(x, y, z)`): `low` when
/// `value < low`, `high` when `value > high`, else `value`
/// (`docs/spec/av2/1.0.0/04-conventions.md#s-4-8`, eq. 3).
pub const fn clip3(low: i64, high: i64, value: i64) -> i64 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round2_identity_when_shift_is_zero() {
        assert_eq!(round2(5, 0), 5);
        assert_eq!(round2(-5, 0), -5);
    }

    #[test]
    fn round2_rounds_toward_negative_infinity() {
        assert_eq!(round2(0, 11), 0); // (0 + 1024) >> 11 = 0
        assert_eq!(round2(1024, 11), 1); // (1024 + 1024) >> 11 = 1
        assert_eq!(round2(1023, 11), 0); // (1023 + 1024) >> 11 = 0
        assert_eq!(round2(-1024, 11), 0); // (-1024 + 1024) >> 11 = 0
        assert_eq!(round2(-2048, 11), -1); // arithmetic: (-2048 + 1024) >> 11 = -1
    }

    #[test]
    fn round2_signed_matches_spec_for_both_signs() {
        // Round2Signed(x, 7) mirrors negatives via -Round2(-x, 7).
        assert_eq!(round2_signed(0, 7), 0);
        assert_eq!(round2_signed(64, 7), 1); // (64 + 64) >> 7 = 1
        assert_eq!(round2_signed(63, 7), 0);
        assert_eq!(round2_signed(-64, 7), -1);
        assert_eq!(round2_signed(-63, 7), 0);
        assert_eq!(round2_signed(192, 7), 2); // (192 + 64) >> 7 = 2
        assert_eq!(round2_signed(-192, 7), -2);
    }

    #[test]
    fn round2_is_total_for_public_callers() {
        // Public primitive: must not panic on extreme inputs even with
        // workspace `overflow-checks = true`.
        assert_eq!(round2(i64::MAX, 0), i64::MAX); // n == 0 is the identity
        assert_eq!(round2(i64::MAX, 1), 1 << 62); // (2^63 - 1 + 1) >> 1
        assert_eq!(round2(i64::MIN, 1), -(1 << 62)); // widened add, no overflow
        assert_eq!(round2(0, 64), 0); // shift >= width saturates, no panic
        assert_eq!(round2(i64::MAX, 200), 0); // shift >= i128 width saturates
    }

    #[test]
    fn round2_signed_is_total_at_i64_min() {
        // `-i64::MIN` overflows i64; round2_signed must stay panic-free.
        assert_eq!(round2_signed(i64::MIN, 0), i64::MIN); // n == 0 is the identity
        assert_eq!(round2_signed(i64::MIN, 1), -(1 << 62)); // -Round2(2^63, 1)
        let _ = round2_signed(i64::MIN, 7);
        let _ = round2_signed(i64::MAX, 7);
    }

    #[test]
    fn clip3_clamps_to_inclusive_range() {
        assert_eq!(clip3(0, 10, -1), 0); // below low
        assert_eq!(clip3(0, 10, 0), 0); // at low
        assert_eq!(clip3(0, 10, 5), 5); // within
        assert_eq!(clip3(0, 10, 10), 10); // at high
        assert_eq!(clip3(0, 10, 11), 10); // above high
        assert_eq!(clip3(-5, 5, -9), -5); // negative range
    }
}
