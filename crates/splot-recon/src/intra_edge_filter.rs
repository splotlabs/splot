// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.13.2.18 intra edge filter and § 7.13.2.14 corner filter.
//!
//! Feature tracking: `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`.

use fearless_simd::{Level, Simd, SimdBase, dispatch, u16x16};

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
/// a filtered sample falls outside the representable range (the `(s + 8) >> 4`
/// average of in-range samples is always representable, so this cannot fire for
/// valid edges, but the conversion is checked rather than truncating).
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
    if filter_intra_edge_lanes(edge, sz, kernel) {
        return Ok(());
    }
    let last = sz - 1;
    let [tap_m2, tap_m1, tap_0, tap_p1, tap_p2] = *kernel;
    let mut two_back = edge[0].to_u16();
    let mut one_back = two_back;
    for i in 1..sz {
        let current = edge[i].to_u16();
        let one_ahead = edge[i.saturating_add(1).min(last)].to_u16();
        let two_ahead = edge[i.saturating_add(2).min(last)].to_u16();
        let s = tap_m2 * i32::from(two_back)
            + tap_m1 * i32::from(one_back)
            + tap_0 * i32::from(current)
            + tap_p1 * i32::from(one_ahead)
            + tap_p2 * i32::from(two_ahead);
        let value = (s + 8) >> 4;
        let value = u16::try_from(value).map_err(|_| ReconError::ArithmeticOverflow {
            context: "intra edge filter output out of range",
        })?;
        edge[i] = T::try_from_u16(value)?;
        two_back = one_back;
        one_back = current;
    }
    Ok(())
}

/// Widest edge [`filter_intra_edge_lanes`] filters.
const MAX_LANE_EDGE: usize = 256;
/// Largest 12-bit sample, whose weighted sums fit the `u16` lanes.
const MAX_LANE_SAMPLE: u16 = 4095;
/// Clamped samples before the edge in the lane copy.
const PAD: usize = 2;

/// [`apply_intra_edge_filter`] in 16 `u16` lanes over a clamped copy of the
/// edge, for `u8`/`u16` storage of samples up to 12 bits. The taps are
/// non-negative and sum to 16, so each output is a weighted average of stored
/// samples: it fits `u16` lanes (`16 * 4095 + 8`) and needs no range check.
fn filter_intra_edge_lanes<T: ReconSample>(
    edge: &mut [T],
    sz: usize,
    kernel: &[i32; INTRA_EDGE_TAPS],
) -> bool {
    if sz < 2 || sz + 2 * PAD + 16 > MAX_LANE_EDGE || sz > edge.len() {
        return false;
    }
    if let Some(samples) = T::u8_slice_mut(&mut edge[..sz]) {
        let source = clamped_copy(samples);
        dispatch!(Level::baseline(), simd => filter_lanes(simd, &source, kernel, &mut samples[1..]));
        return true;
    }
    let Some(samples) = T::u16_slice_mut(&mut edge[..sz]) else {
        return false;
    };
    if samples.iter().any(|&sample| sample > MAX_LANE_SAMPLE) {
        return false;
    }
    let source = clamped_copy(samples);
    dispatch!(Level::baseline(), simd => filter_lanes(simd, &source, kernel, &mut samples[1..]));
    true
}

/// `samples` after [`PAD`] copies of its first sample, then its last sample to
/// the end.
fn clamped_copy<P: Copy + Default>(samples: &[P]) -> [P; MAX_LANE_EDGE] {
    let mut source = [P::default(); MAX_LANE_EDGE];
    source[PAD..PAD + samples.len()].copy_from_slice(samples); // splot-copy-ok: clamped edge copy the filter reads
    source[..PAD].fill(samples[0]);
    source[PAD + samples.len()..].fill(samples[samples.len() - 1]);
    source
}

/// Filters `out[j] = edge[j + 1]` from the clamped copy `source`.
#[inline]
fn filter_lanes<S: Simd, P: IntraLaneSample>(
    simd: S,
    source: &[P; MAX_LANE_EDGE],
    kernel: &[i32; INTRA_EDGE_TAPS],
    out: &mut [P],
) {
    for at in (0..out.len()).step_by(16) {
        let mut sum = u16x16::splat(simd, 8);
        for (k, &tap) in kernel.iter().enumerate() {
            if let Some(window) = source[at + 1 + k..].first_chunk::<16>() {
                sum += P::load16(simd, window) * tap as u16;
            }
        }
        let mut values = [P::default(); 16];
        P::store16(simd, sum >> 4, &mut values);
        let count = (out.len() - at).min(16);
        out[at..at + count].copy_from_slice(&values[..count]); // splot-copy-ok: publish the filtered edge
    }
}

/// AV2 §7.13.2.14 Filter corner process. Returns the three-tap top-left corner
/// value `Round2(LeftCol[0] * 5 + AboveRow[-1] * 6 + AboveRow[0] * 5, 4)`,
/// written by the caller into BOTH `AboveRow[-1]` and `LeftCol[-1]`. Transcribed
/// VERBATIM from the committed spec mirror
/// (`filter_intra_edge_corner_high`, `~/Devel/avm/av2/common/reconintra.c:1020-1028`):
/// `s = left0 * 5 + above_neg1 * 6 + above0 * 5; out = (s + 8) >> 4`.
#[must_use]
pub fn filter_intra_edge_corner(left0: u16, above_neg1: u16, above0: u16) -> u16 {
    let s = i32::from(left0) * 5 + i32::from(above_neg1) * 6 + i32::from(above0) * 5;
    ((s + 8) >> 4) as u16
}
