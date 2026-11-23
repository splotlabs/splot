// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.13.2.18 intra edge filter and § 7.13.2.14 corner filter.
//!
//! Feature tracking: `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`.

use fearless_simd::{Bytes, Simd, SimdBase, SimdMask, u16x16, u32x8};

use crate::intra_dc_math::IntraLaneSample;
use crate::{ReconError, ReconSample, Result};

/// Number of §7.13.2.18 intra edge filter kernel rows (`INTRA_EDGE_KERNELS`).
const INTRA_EDGE_KERNELS: usize = 3;
/// Number of taps per §7.13.2.18 intra edge filter kernel (`INTRA_EDGE_TAPS`).
const INTRA_EDGE_TAPS: usize = 5;

/// AV2 §7.13.2.18 `Intra_Edge_Kernel[INTRA_EDGE_KERNELS][INTRA_EDGE_TAPS]`,
/// transcribed VERBATIM from the committed spec mirror
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-18`. Indexed by
/// `strength - 1` (strength `1..=3`).
pub(crate) const INTRA_EDGE_KERNEL: [[i32; INTRA_EDGE_TAPS]; INTRA_EDGE_KERNELS] =
    [[0, 4, 8, 4, 0], [0, 5, 6, 5, 0], [2, 4, 4, 4, 2]];

/// AV2 §7.13.2.18 Intra edge filter process. Filters the first `sz` samples of
/// `edge` in place with the given `strength` (`0..=3`). `strength == 0` is a
/// no-op (the process returns without modifying the edge). Transcribed VERBATIM
/// from the committed spec mirror (`av2_filter_intra_edge_high_c`,
/// `~/Devel/avm/av2/common/reconintra.c:997-1018`):
///
/// `edge` holds the §7.13.2.1 reference samples with `edge[i]` the §7.13.2.18
/// array element (the caller positions the slice so `edge[0]` is the logical
/// `AboveRow[-1]` / `LeftCol[-1]` corner, `edge[1]` the logical `[0]`, etc). For
/// `i = 1..sz-1`: `s = Σ(j=0..4) Intra_Edge_Kernel[strength-1][j] *
/// edge[Clip3(0, sz-1, i-2+j)]`, then `edge[i] = (s + 8) >> 4`. The `i == 0`
/// corner sample is read but never overwritten (matching the §7.13.2.18
/// `AboveRow[-1]` / `LeftCol[-1]` invariance), so a preceding §7.13.2.14 corner
/// rewrite survives into the §7.13.2.8 prediction.
///
/// # Errors
/// Returns [`ReconError::ArithmeticOverflow`] when `sz` exceeds `edge.len()`, or
/// `strength` is outside `0..=3` for a nonempty edge.
pub fn apply_intra_edge_filter<T: ReconSample>(
    edge: &mut [T],
    sz: usize,
    strength: u8,
) -> Result<()> {
    if strength == 0 || sz == 0 {
        return Ok(());
    }
    if sz > edge.len() {
        return Err(ReconError::ArithmeticOverflow {
            context: "intra edge filter size exceeds edge length",
        });
    }
    let Some(kernel) = INTRA_EDGE_KERNEL.get(usize::from(strength - 1)) else {
        return Err(ReconError::ArithmeticOverflow {
            context: "intra edge filter strength out of range",
        });
    };
    if sz < 2 {
        return Ok(());
    }
    let edge = &mut edge[..sz];
    if let Some(samples) = T::u8_slice_mut(edge) {
        baseline_simd!(simd => filter_rolling::<_, _, false>(simd, samples, kernel));
    } else if let Some(samples) = T::u16_slice_mut(edge) {
        baseline_simd!(simd => {
            if fits_narrow_sum(simd, samples) {
                filter_rolling::<_, _, false>(simd, samples, kernel);
            } else {
                filter_rolling::<_, _, true>(simd, samples, kernel);
            }
        });
    }
    Ok(())
}

#[inline]
fn load_clamped<S: Simd, P: IntraLaneSample>(simd: S, samples: &[P], last: P) -> u16x16<S> {
    if let Some(full) = samples.first_chunk::<16>() {
        P::load16(simd, full)
    } else if samples.is_empty() {
        u16x16::splat(simd, last.to_u16())
    } else {
        let mut padded = [last; 16];
        padded[..samples.len()].copy_from_slice(samples); // splot-copy-ok: clamp the final SIMD input lanes
        P::load16(simd, &padded)
    }
}

#[inline]
fn fits_narrow_sum<S: Simd>(simd: S, samples: &[u16]) -> bool {
    const MAX_NARROW_SAMPLE: u16 = (u16::MAX - 8) / 16;
    let (full, tail) = samples.as_chunks::<16>();
    let high = full.iter().fold(u16x16::splat(simd, 0), |high, input| {
        high.max(u16x16::from_slice(simd, input))
    });
    let high = high.max(load_clamped(simd, tail, 0));
    high.simd_le(u16x16::splat(simd, MAX_NARROW_SAMPLE))
        .all_true()
}

/// Keeps the previous and current source vectors until their last use.
#[inline]
fn filter_rolling<S: Simd, P: IntraLaneSample, const WIDE: bool>(
    simd: S,
    samples: &mut [P],
    kernel: &[i32; INTRA_EDGE_TAPS],
) {
    let (Some(&first), Some(&last)) = (samples.first(), samples.last()) else {
        return;
    };
    let mut remaining = &mut samples[1..];
    let mut previous = u16x16::splat(simd, first.to_u16());
    let mut current = load_clamped(simd, remaining, last);
    let [outer, inner, center, _, _] = *kernel;
    loop {
        let next = load_clamped(simd, remaining.get(16..).unwrap_or_default(), last);
        let left1 = simd
            .slide_i16x16::<15>(previous.bitcast(), current.bitcast())
            .bitcast();
        let right1 = simd
            .slide_i16x16::<1>(current.bitcast(), next.bitcast())
            .bitcast();
        let left2 = simd
            .slide_i16x16::<14>(previous.bitcast(), current.bitcast())
            .bitcast();
        let right2 = simd
            .slide_i16x16::<2>(current.bitcast(), next.bitcast())
            .bitcast();
        let value = if WIDE {
            let [(lb2, hb2), (lb1, hb1), (lc, hc), (la1, ha1), (la2, ha2)] =
                [left2, left1, current, right1, right2].map(|v| simd.widen_u16x16(v));
            let combine = |left2: u32x8<S>, left1, middle, right1, right2| {
                ((left2 + right2) * outer as u32
                    + (left1 + right1) * inner as u32
                    + middle * center as u32
                    + 8)
                    >> 4
            };
            simd.relaxed_narrow_u32x8(
                combine(lb2, lb1, lc, la1, la2),
                combine(hb2, hb1, hc, ha1, ha2),
            )
        } else {
            ((left2 + right2) * outer as u16
                + (left1 + right1) * inner as u16
                + current * center as u16
                + 8)
                >> 4
        };
        if remaining.len() < 16 {
            let mut tail = [last; 16];
            P::store16(simd, value, &mut tail);
            remaining.copy_from_slice(&tail[..remaining.len()]); // splot-copy-ok: store only the valid SIMD output lanes
            break;
        }
        let Some((out, rest)) = remaining.split_first_chunk_mut::<16>() else {
            break;
        };
        P::store16(simd, value, out);
        if rest.is_empty() {
            break;
        }
        remaining = rest;
        previous = current;
        current = next;
    }
}

/// AV2 §7.13.2.14 Filter corner process. Returns the three-tap top-left corner
/// value `Round2(LeftCol[0] * 5 + AboveRow[-1] * 6 + AboveRow[0] * 5, 4)`,
/// written by the caller into both `AboveRow[-1]` and `LeftCol[-1]`. Transcribed
/// VERBATIM from the committed spec mirror
/// (`filter_intra_edge_corner_high`, `~/Devel/avm/av2/common/reconintra.c:1020-1028`):
/// `s = left0 * 5 + above_neg1 * 6 + above0 * 5; out = (s + 8) >> 4`.
#[must_use]
pub fn filter_intra_edge_corner(left0: u16, above_neg1: u16, above0: u16) -> u16 {
    let s = i32::from(left0) * 5 + i32::from(above_neg1) * 6 + i32::from(above0) * 5;
    ((s + 8) >> 4) as u16
}
