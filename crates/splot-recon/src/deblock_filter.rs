// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.17 deblocking-filter sample math.
//!
//! This module implements the scheduler-free per-edge AV2 deblocking primitives
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)):
//! the § 7.17.7.1 sample filter ([`deblock_sample_filter`], `#s-7-17-7-1`), which
//! modifies up to `maxWidthNeg` samples on the previous (`p`) side and
//! `Max(maxWidthNeg, maxWidthPos)` samples on the current (`q`) side using the
//! `deltaM2` ramp, the `Q_Thresh_Mults` / `W_Mult` § 9.2 weights, `Round2`, and
//! the § 4.8 `Clip1` clamp; the § 7.17.3 filter-maximum-width derivation
//! ([`deblock_filter_max_width`], `#s-7-17-3`), which produces the per-side
//! widths the sample filter consumes; and the § 7.17.5 adaptive filter strength
//! ([`deblock_adaptive_filter_strength`] / [`deblock_side_threshold_index`],
//! `#s-7-17-5`), which produces the `qThr` / `side` thresholds from the filter
//! level.
//!
//! Feature tracking: `RECON-DEBLOCK-SAMPLE-FILTER`,
//! `RECON-DEBLOCK-FILTER-MAX-WIDTH`, `RECON-DEBLOCK-ADAPTIVE-STRENGTH`.
//!
//! Scope: these are the per-edge sample math and the parameter derivations over
//! caller-resolved spec-derived values. The § 7.17.1 / § 7.17.2 edge traversal,
//! the § 7.17.6 filter-level selection and § 7.17.7.2 filter choice (which need
//! the `DeblockingTxSizes`, segment/qindex maps, and block state), and the
//! `Q_Thresh_Mults` / `W_Mult` / `Side_Thresholds` § 9.2 table lookups stay with
//! the caller — it passes the resolved widths, weights, level, and pre-indexed
//! threshold as scalars, exactly as the other `splot-recon` primitives take
//! caller-resolved spec-derived values. It does not read frame, segment, or tile
//! state or wire into the runtime decode path.

use crate::dequant::quantizer_value;
use crate::intra_dc_math::validate_sample_type;
use crate::{BitDepth, ReconError, ReconSample, Result};

/// AV2 § 3 `DF_SHIFT`: the deblocking-filter ramp shift
/// (`docs/spec/av2/1.0.0/03-symbols.md`, `DF_SHIFT = 8`).
const DF_SHIFT: u32 = 8;

/// AV2 § 3 `MAX_SIDE_TABLE`: the length of the § 9.2 `Side_Thresholds` array, the
/// upper bound (exclusive) on the § 7.17.5 `qInd`.
const MAX_SIDE_TABLE: usize = 296;

/// AV2 § 3 `QUANT_TABLE_BITS`: the § 7.14.4 / § 7.17.5 quantizer-table shift.
const QUANT_TABLE_BITS: u32 = 3;

/// AV2 § 3 `MAX_DBL_FLT_LEN`: the maximum deblocking-filter length, i.e. the
/// length of `Q_Thresh_Mults` / `W_Mult` and the maximum per-side width.
const MAX_DBL_FLT_LEN: usize = 8;

/// Caller-resolved parameters for the AV2 § 7.17.7.1 deblocking sample filter.
///
/// `boundary` is the index in `line` of the first current-side sample (`q0`); the
/// previous-side samples (`p0`, `p1`, …) are at `boundary - 1`, `boundary - 2`, …
/// `max_width_neg` / `max_width_pos` are the § 7.17 per-side maximum widths
/// (`1..=MAX_DBL_FLT_LEN`); `q_thr` is the filter threshold; `q_thresh_mult` is
/// `Q_Thresh_Mults[Max(max_width_neg, max_width_pos) - 1]` and `w_mult_neg` /
/// `w_mult_pos` are `W_Mult[max_width_neg - 1]` / `W_Mult[max_width_pos - 1]`,
/// resolved by the caller from the § 9.2 tables; `prev_lossless` / `curr_lossless`
/// gate the two sides; `bit_depth` bounds the `Clip1`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeblockSampleFilter {
    /// Index in `line` of the first current-side sample (`q0`).
    pub boundary: usize,
    /// § 7.17 filter threshold `qThr`.
    pub q_thr: i32,
    /// Maximum modified previous-side (`p`) samples (`maxWidthNeg`, `1..=8`).
    pub max_width_neg: usize,
    /// Maximum modified current-side (`q`) samples (`maxWidthPos`, `1..=8`).
    pub max_width_pos: usize,
    /// `Q_Thresh_Mults[Max(max_width_neg, max_width_pos) - 1]`.
    pub q_thresh_mult: i32,
    /// `W_Mult[max_width_neg - 1]`.
    pub w_mult_neg: i32,
    /// `W_Mult[max_width_pos - 1]`.
    pub w_mult_pos: i32,
    /// Whether the previous-side samples are in a lossless segment (skips them).
    pub prev_lossless: bool,
    /// Whether the current-side samples are in a lossless segment (skips them).
    pub curr_lossless: bool,
    /// Active decoded bit depth (bounds the `Clip1`).
    pub bit_depth: BitDepth,
}

/// Applies the AV2 § 7.17.7.1 deblocking sample filter to the perpendicular
/// sample `line`, modifying it in place.
///
/// `deltaM2 = Clip3(-qThrClamp, qThrClamp, (p1 - q1 + 3*(q0 - p0)) * 4)` (with
/// `qThrClamp = q_thr * q_thresh_mult`) drives a per-sample ramp: for
/// `i = 0..Max(maxWidthNeg, maxWidthPos)`, the current-side sample at
/// `boundary + i` is `Clip1(sample - Round2(deltaM2 * w_mult_pos * (maxWidthPos -
/// i), 3 + DF_SHIFT))` (unless `curr_lossless`), and, for `i < maxWidthNeg`, the
/// previous-side sample at `boundary - 1 - i` is `Clip1(sample + Round2(deltaM2 *
/// w_mult_neg * (maxWidthNeg - i), 3 + DF_SHIFT))` (unless `prev_lossless`).
/// `q0`/`q1`/`p0`/`p1` are read from the original `line` before any write.
///
/// The computation is total and panic-free for valid inputs: the ramp uses `i64`
/// (each term is far inside `i64`), the `qThrClamp` bound is clamped non-negative
/// so `Clip3` never inverts, and the line bounds are validated before any sample
/// is read or written.
///
/// # Errors
/// Returns [`ReconError::SampleTypeUnsupportedBitDepth`] if `T` cannot represent
/// `bit_depth`, [`ReconError::DeblockFilterInvalidWidth`] if `max_width_neg` /
/// `max_width_pos` are not in `1..=8`, and [`ReconError::DeblockFilterLineTooShort`]
/// if `line` does not contain the previous- and current-side samples the filter
/// reads and writes around `boundary`. All inputs are validated before any sample
/// is modified.
pub fn deblock_sample_filter<T: ReconSample>(
    line: &mut [T],
    params: &DeblockSampleFilter,
) -> Result<()> {
    validate_sample_type::<T>(params.bit_depth)?;
    let DeblockSampleFilter {
        boundary,
        q_thr,
        max_width_neg,
        max_width_pos,
        q_thresh_mult,
        w_mult_neg,
        w_mult_pos,
        prev_lossless,
        curr_lossless,
        bit_depth,
    } = *params;

    if !(1..=MAX_DBL_FLT_LEN).contains(&max_width_neg)
        || !(1..=MAX_DBL_FLT_LEN).contains(&max_width_pos)
    {
        return Err(ReconError::DeblockFilterInvalidWidth {
            max_width_neg,
            max_width_pos,
        });
    }
    let width = max_width_neg.max(max_width_pos);
    // Previous side reads `p1 = line[boundary - 2]` and writes down to
    // `line[boundary - max_width_neg]`; current side reads `q1 = line[boundary +
    // 1]` and writes up to `line[boundary + width - 1]`. `boundary` must leave
    // room for both (with `.max(2)` covering the `p1` / `q1` reads).
    let low_extent = max_width_neg.max(2);
    let high_extent = width.max(2);
    if boundary < low_extent || boundary + high_extent > line.len() {
        return Err(ReconError::DeblockFilterLineTooShort {
            boundary,
            max_width_neg,
            width,
            len: line.len(),
        });
    }

    let q0 = i64::from(line[boundary].to_u16());
    let q1 = i64::from(line[boundary + 1].to_u16());
    let p0 = i64::from(line[boundary - 1].to_u16());
    let p1 = i64::from(line[boundary - 2].to_u16());

    // § 7.17.7.1 `deltaM2`. `qThrClamp` is non-negative for conformant inputs;
    // `.max(0)` keeps `Clip3` well-formed (and the filter inert) otherwise.
    let q_thr_clamp = (i64::from(q_thr) * i64::from(q_thresh_mult)).max(0);
    let delta_m2 = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-q_thr_clamp, q_thr_clamp);
    let delta_m2_neg = delta_m2 * i64::from(w_mult_neg);
    let delta_m2_pos = delta_m2 * i64::from(w_mult_pos);

    let shift = 3 + DF_SHIFT;
    let max_sample = i64::from(bit_depth.max_sample());
    for i in 0..width {
        let signed_i = i as i64;
        let diff_pos = round2(delta_m2_pos * (max_width_pos as i64 - signed_i), shift);
        if !curr_lossless {
            let index = boundary + i;
            let value = (i64::from(line[index].to_u16()) - diff_pos).clamp(0, max_sample);
            line[index] = T::try_from_u16(value as u16)?;
        }
        if i < max_width_neg && !prev_lossless {
            let diff_neg = round2(delta_m2_neg * (max_width_neg as i64 - signed_i), shift);
            let index = boundary - 1 - i;
            let value = (i64::from(line[index].to_u16()) + diff_neg).clamp(0, max_sample);
            line[index] = T::try_from_u16(value as u16)?;
        }
    }
    Ok(())
}

/// AV2 § 4.8 `Round2(value, n) = (value + (1 << (n - 1))) >> n` for `n > 0`, over
/// `i64` (arithmetic shift, so a negative `value` rounds toward negative
/// infinity, matching the spec). `n` here is `3 + DF_SHIFT = 11`.
const fn round2(value: i64, n: u32) -> i64 {
    (value + (1i64 << (n - 1))) >> n
}

/// Derives the AV2 § 7.17.3 deblocking filter maximum per-side widths
/// `(maxWidthNeg, maxWidthPos)`
/// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-3`).
///
/// `filter_size` is the § 7.17.4 maximum filter size (a transform dimension);
/// `is_chroma` is the spec `plane != 0`; `sb_edge` is whether the edge is at a
/// super-block boundary. The result is the pair of caller-resolved widths the
/// § 7.17.7.1 sample filter ([`deblock_sample_filter`]) takes as `max_width_neg`
/// / `max_width_pos`.
///
/// `maxWidthPos` is `1` for `filter_size <= 4`, `3` for `8`, `is_chroma ? 4 : 6`
/// for `16`, and `is_chroma ? 4 : 8` otherwise; `maxWidthNeg` is
/// `Min(maxWidthPos, is_chroma ? 2 : 6)` at a super-block edge and `maxWidthPos`
/// otherwise. This is a total `const fn`: every input maps to a defined pair.
pub const fn deblock_filter_max_width(
    filter_size: usize,
    is_chroma: bool,
    sb_edge: bool,
) -> (usize, usize) {
    let max_width_pos = if filter_size <= 4 {
        1
    } else if filter_size == 8 {
        3
    } else if filter_size == 16 {
        if is_chroma { 4 } else { 6 }
    } else if is_chroma {
        4
    } else {
        8
    };
    let max_width_neg = if sb_edge {
        let cap = if is_chroma { 2 } else { 6 };
        if max_width_pos < cap {
            max_width_pos
        } else {
            cap
        }
    } else {
        max_width_pos
    };
    (max_width_neg, max_width_pos)
}

// `deblock_filter_max_width` is a `const fn`: a fixed configuration resolves at
// compile time. This pins luma `filter_size == 32` to `maxWidthPos == 8` and the
// non-super-block `maxWidthNeg == 8`, as a compile-time § 7.17.3 spec contract.
const _MAX_WIDTH_CONST_CHECK: () =
    assert!(matches!(deblock_filter_max_width(32, false, false), (8, 8)));

/// Derives the AV2 § 7.17.5 `qInd`, the index into the § 9.2 `Side_Thresholds`
/// array (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-5`):
/// `Clip3(0, MAX_SIDE_TABLE - 1, lvl - 24 * (BitDepth - 8))`.
///
/// `lvl` is the § 7.17.6 adaptive filter level. The caller uses the result to
/// look up `Side_Thresholds[qInd]` and pass it as the `side_threshold` of
/// [`deblock_adaptive_filter_strength`] (`Side_Thresholds` lives in `splot-core`'s
/// generated § 9.2 tables, which `splot-recon` cannot reach). This is a total
/// `const fn`.
pub const fn deblock_side_threshold_index(lvl: u32, bit_depth: BitDepth) -> usize {
    let q = lvl as i64 - 24 * (bit_depth.bits() as i64 - 8);
    if q < 0 {
        0
    } else if q > (MAX_SIDE_TABLE as i64 - 1) {
        MAX_SIDE_TABLE - 1
    } else {
        q as usize
    }
}

/// Derives the AV2 § 7.17.5 adaptive filter strength outputs `(qThr, side)` from
/// the filter level `lvl`, the caller-resolved `side_threshold =
/// Side_Thresholds[qInd]` (with `qInd` from [`deblock_side_threshold_index`]), and
/// the active `bit_depth`
/// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-5`).
///
/// `qThr = Round2(get_q(lvl, 0), QUANT_TABLE_BITS) >> 6` (the § 7.14.2
/// quantizer-value lookup; [`quantizer_value`](crate::quantizer_value)), and
/// `side = Max(side_threshold + (1 << (12 - BitDepth)), 0) >> (13 - BitDepth)`.
/// `qThr` is the threshold the § 7.17.7.1 sample filter
/// ([`deblock_sample_filter`]) takes as `q_thr`; `side` is the side threshold the
/// § 7.17.7.2 filter-choice process uses.
///
/// The computation is total and panic-free: the quantizer lookup is total, and
/// the `i64` arithmetic with `bit_depth` shifts (`12 - BitDepth` and
/// `13 - BitDepth` are positive for the 8- and 10-bit depths) cannot overflow.
pub fn deblock_adaptive_filter_strength(
    lvl: u32,
    side_threshold: i32,
    bit_depth: BitDepth,
) -> (i32, i32) {
    let bits = i64::from(bit_depth.bits());
    // qThr = Round2(get_q(lvl, 0), QUANT_TABLE_BITS) >> 6.
    let get_q = i64::from(quantizer_value(lvl, 0, bit_depth));
    let q_thr = ((get_q + (1 << (QUANT_TABLE_BITS - 1))) >> QUANT_TABLE_BITS) >> 6;
    // side = Max(Side_Thresholds[qInd] + (1 << (12 - BitDepth)), 0) >> (13 - BitDepth).
    let side = (i64::from(side_threshold) + (1i64 << (12 - bits))).max(0) >> (13 - bits);
    (q_thr as i32, side as i32)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn params(
        boundary: usize,
        q_thr: i32,
        max_width_neg: usize,
        max_width_pos: usize,
        q_thresh_mult: i32,
        w_mult_neg: i32,
        w_mult_pos: i32,
        prev_lossless: bool,
        curr_lossless: bool,
    ) -> DeblockSampleFilter {
        DeblockSampleFilter {
            boundary,
            q_thr,
            max_width_neg,
            max_width_pos,
            q_thresh_mult,
            w_mult_neg,
            w_mult_pos,
            prev_lossless,
            curr_lossless,
            bit_depth: BitDepth::Eight,
        }
    }

    // Independent in-place re-trace of § 7.17.7.1 (mirrors the spec directly, not
    // a call of the function under test).
    fn reference(line: &mut [u8], p: &DeblockSampleFilter) {
        let width = p.max_width_neg.max(p.max_width_pos);
        let q0 = i64::from(line[p.boundary]);
        let q1 = i64::from(line[p.boundary + 1]);
        let p0 = i64::from(line[p.boundary - 1]);
        let p1 = i64::from(line[p.boundary - 2]);
        let bound = (i64::from(p.q_thr) * i64::from(p.q_thresh_mult)).max(0);
        let delta = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-bound, bound);
        let dn = delta * i64::from(p.w_mult_neg);
        let dp = delta * i64::from(p.w_mult_pos);
        let max = i64::from(BitDepth::Eight.max_sample());
        for i in 0..width {
            let diff_pos = round2(dp * (p.max_width_pos as i64 - i as i64), 3 + DF_SHIFT);
            if !p.curr_lossless {
                let idx = p.boundary + i;
                line[idx] = (i64::from(line[idx]) - diff_pos).clamp(0, max) as u8;
            }
            if i < p.max_width_neg && !p.prev_lossless {
                let diff_neg = round2(dn * (p.max_width_neg as i64 - i as i64), 3 + DF_SHIFT);
                let idx = p.boundary - 1 - i;
                line[idx] = (i64::from(line[idx]) + diff_neg).clamp(0, max) as u8;
            }
        }
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
    fn matches_hand_computed_symmetric_width_2() {
        // line = [p1, p0, q0, q1] with boundary = 2, width 2, q_thr 100,
        // q_thresh_mult = Q_Thresh_Mults[1] = 25, w_mult = W_Mult[1] = 51.
        // deltaM2 = (10 - 50 + 3*(60-20)) * 4 = 320 (< qThrClamp = 2500).
        // i=0: diff = Round2(320*51*2, 11) = Round2(32640, 11) = 16
        //   -> q0' = Clip1(60-16)=44, p0' = Clip1(20+16)=36
        // i=1: diff = Round2(320*51*1, 11) = Round2(16320, 11) = 8
        //   -> q1' = Clip1(50-8)=42, p1' = Clip1(10+8)=18
        let mut line = [10u8, 20, 60, 50];
        deblock_sample_filter(&mut line, &params(2, 100, 2, 2, 25, 51, 51, false, false)).unwrap();
        assert_eq!(line, [18, 36, 44, 42]);
    }

    #[test]
    fn matches_reference_across_configs() {
        // Asymmetric widths, lossless gating, and clamped deltaM2 all match the
        // independent re-trace.
        let base = [40u8, 60, 50, 70, 55, 80, 45, 90, 35, 100];
        let configs = [
            params(4, 80, 3, 2, 19, 37, 51, false, false),
            params(4, 200, 2, 3, 19, 51, 37, false, false), // maxWidthPos > maxWidthNeg
            params(5, 20, 4, 4, 19, 28, 28, false, false),  // small q_thr clamps deltaM2
            params(4, 80, 2, 2, 25, 51, 51, true, false),   // prev lossless: p-side skipped
            params(4, 80, 2, 2, 25, 51, 51, false, true),   // curr lossless: q-side skipped
        ];
        for p in &configs {
            let mut produced = base;
            deblock_sample_filter(&mut produced, p).unwrap();
            let mut expected = base;
            reference(&mut expected, p);
            assert_eq!(produced, expected, "config {p:?}");
        }
    }

    #[test]
    fn lossless_sides_are_untouched() {
        let base = [40u8, 60, 50, 70, 55, 80];
        // Both sides lossless: the line is unchanged.
        let mut both = base;
        deblock_sample_filter(&mut both, &params(3, 100, 2, 2, 25, 51, 51, true, true)).unwrap();
        assert_eq!(both, base);
    }

    #[test]
    fn clip1_clamps_to_bit_depth() {
        // A large deltaM2 must clamp the current side to 0 and the previous side
        // to the 8-bit max (255) rather than overflow.
        let mut line = [10u8, 250, 5, 240, 0, 255];
        deblock_sample_filter(
            &mut line,
            &params(3, 10_000, 2, 2, 32, 85, 85, false, false),
        )
        .unwrap();
        // A huge positive deltaM2 drives the current side toward 0 and the
        // previous side toward 255 (every value stays a valid clamped u8).
        let mut reference_line = [10u8, 250, 5, 240, 0, 255];
        reference(
            &mut reference_line,
            &params(3, 10_000, 2, 2, 32, 85, 85, false, false),
        );
        assert_eq!(line, reference_line);
        // 10-bit path: a u16 line clamps to 1023.
        let mut wide = [10u16, 1000, 5, 1020, 0, 1023];
        deblock_sample_filter(
            &mut wide,
            &DeblockSampleFilter {
                bit_depth: BitDepth::Ten,
                ..params(3, 10_000, 2, 2, 32, 85, 85, false, false)
            },
        )
        .unwrap();
        assert!(wide.iter().all(|&v| v <= 1023));
    }

    #[test]
    fn rejects_invalid_width_and_short_line() {
        let mut line = [0u8; 8];
        assert!(matches!(
            deblock_sample_filter(&mut line, &params(4, 0, 0, 2, 19, 51, 51, false, false)),
            Err(ReconError::DeblockFilterInvalidWidth {
                max_width_neg: 0,
                max_width_pos: 2
            })
        ));
        assert!(matches!(
            deblock_sample_filter(&mut line, &params(4, 0, 2, 9, 19, 51, 51, false, false)),
            Err(ReconError::DeblockFilterInvalidWidth { .. })
        ));
        // boundary too close to the start (needs boundary >= max(max_width_neg, 2)).
        assert!(matches!(
            deblock_sample_filter(&mut line, &params(1, 0, 2, 2, 19, 51, 51, false, false)),
            Err(ReconError::DeblockFilterLineTooShort { .. })
        ));
        // boundary too close to the end (needs boundary + max(width, 2) <= len).
        let mut short = [0u8; 5];
        assert!(matches!(
            deblock_sample_filter(&mut short, &params(4, 0, 2, 2, 19, 51, 51, false, false)),
            Err(ReconError::DeblockFilterLineTooShort { .. })
        ));
    }

    #[test]
    fn is_total_for_extreme_inputs() {
        // Extreme samples, thresholds, and weights must not overflow the i64 ramp
        // or panic, across symmetric and asymmetric widths and lossless gating.
        for &(neg, pos) in &[(1usize, 8usize), (8, 1), (8, 8)] {
            let mut line = [0u8; 24];
            for (i, s) in line.iter_mut().enumerate() {
                *s = if i % 2 == 0 { 0 } else { 255 };
            }
            deblock_sample_filter(
                &mut line,
                &params(10, i32::MAX, neg, pos, 32, 85, 85, false, false),
            )
            .unwrap();
        }
    }

    #[test]
    fn max_width_covers_every_spec_branch() {
        // maxWidthPos by filter_size (no super-block edge -> maxWidthNeg == pos):
        assert_eq!(deblock_filter_max_width(4, false, false), (1, 1));
        assert_eq!(deblock_filter_max_width(2, true, false), (1, 1)); // <= 4
        assert_eq!(deblock_filter_max_width(8, false, false), (3, 3));
        assert_eq!(deblock_filter_max_width(8, true, false), (3, 3));
        assert_eq!(deblock_filter_max_width(16, false, false), (6, 6)); // luma
        assert_eq!(deblock_filter_max_width(16, true, false), (4, 4)); // chroma
        assert_eq!(deblock_filter_max_width(32, false, false), (8, 8)); // luma > 16
        assert_eq!(deblock_filter_max_width(64, true, false), (4, 4)); // chroma > 16

        // Super-block edge caps maxWidthNeg at Min(maxWidthPos, chroma ? 2 : 6):
        assert_eq!(deblock_filter_max_width(32, false, true), (6, 8)); // luma: cap 6
        assert_eq!(deblock_filter_max_width(64, true, true), (2, 4)); // chroma: cap 2
        assert_eq!(deblock_filter_max_width(8, true, true), (2, 3)); // chroma: cap 2 < 3
        assert_eq!(deblock_filter_max_width(4, false, true), (1, 1)); // cap 6 but pos 1
    }

    #[test]
    fn side_threshold_index_clips_to_table_range() {
        // 8-bit: qInd = lvl (no offset), clamped to 0..=295.
        assert_eq!(deblock_side_threshold_index(10, BitDepth::Eight), 10);
        assert_eq!(deblock_side_threshold_index(0, BitDepth::Eight), 0);
        assert_eq!(deblock_side_threshold_index(400, BitDepth::Eight), 295);
        // 10-bit: qInd = lvl - 48, clamped (negative -> 0).
        assert_eq!(deblock_side_threshold_index(10, BitDepth::Ten), 0); // 10 - 48 < 0
        assert_eq!(deblock_side_threshold_index(100, BitDepth::Ten), 52); // 100 - 48
    }

    #[test]
    fn adaptive_filter_strength_matches_spec() {
        // `side` is independent of get_q: Max(threshold + (1 << (12 - bits)), 0)
        // >> (13 - bits). 8-bit: +16 >> 5; 10-bit: +4 >> 3.
        assert_eq!(
            deblock_adaptive_filter_strength(40, 100, BitDepth::Eight).1,
            3
        ); // (100+16)>>5
        assert_eq!(
            deblock_adaptive_filter_strength(40, -16, BitDepth::Eight).1,
            0
        ); // (0)>>5
        assert_eq!(
            deblock_adaptive_filter_strength(40, 1678, BitDepth::Eight).1,
            52
        ); // (1694)>>5
        assert_eq!(
            deblock_adaptive_filter_strength(40, 100, BitDepth::Ten).1,
            13
        ); // (104)>>3

        // `qThr` composes get_q with Round2(_, 3) >> 6; verify against the
        // independently-tested quantizer_value for several levels and depths.
        for &(lvl, bd) in &[
            (40u32, BitDepth::Eight),
            (128, BitDepth::Eight),
            (200, BitDepth::Ten),
        ] {
            let expected = (((i64::from(quantizer_value(lvl, 0, bd)) + 4) >> 3) >> 6) as i32;
            assert_eq!(deblock_adaptive_filter_strength(lvl, 0, bd).0, expected);
        }
    }
}
