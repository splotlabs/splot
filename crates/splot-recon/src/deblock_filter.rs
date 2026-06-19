// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.17.7.1 deblocking sample-filter process.
//!
//! This module implements the scheduler-free per-edge sample filter
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-17-7-1`): given the line of reconstructed samples perpendicular to a
//! block boundary, it modifies up to `maxWidthNeg` samples on the previous
//! (`p`) side and `Max(maxWidthNeg, maxWidthPos)` samples on the current (`q`)
//! side using the `deltaM2` ramp, the `Q_Thresh_Mults` / `W_Mult` § 9.2 weights,
//! `Round2`, and the § 4.8 `Clip1` clamp.
//!
//! Feature tracking: `RECON-DEBLOCK-SAMPLE-FILTER`.
//!
//! Scope: this is the § 7.17.7.1 sample filter alone, over a caller-supplied
//! perpendicular sample line. The § 7.17.1 / § 7.17.2 edge traversal, the
//! § 7.17.3-§ 7.17.7.2 filter-size / strength / choice derivation (which need the
//! `DeblockingTxSizes`, filter levels, and block state), and the
//! `Q_Thresh_Mults[width - 1]` / `W_Mult[maxWidth - 1]` table lookups stay with
//! the caller — the caller passes the resolved `q_thr`, the per-side widths, and
//! the three pre-indexed table weights as scalars, exactly as the other
//! `splot-recon` primitives take caller-resolved spec-derived values. It does not
//! read frame, segment, or tile state or wire into the runtime decode path.

use crate::intra_dc_math::validate_sample_type;
use crate::{BitDepth, ReconError, ReconSample, Result};

/// AV2 § 3 `DF_SHIFT`: the deblocking-filter ramp shift
/// (`docs/spec/av2/1.0.0/03-symbols.md`, `DF_SHIFT = 8`).
const DF_SHIFT: u32 = 8;

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
}
