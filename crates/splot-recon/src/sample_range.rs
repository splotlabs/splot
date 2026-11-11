// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Active-bit-depth range scans and edge sums shared by the sample kernels.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`.

use fearless_simd::{Bytes as _, Simd, SimdBase, u16x16};

use crate::ReconSample;

/// Reports whether any sample of any storage type exceeds the active bit depth.
///
/// `u16` storage takes the lane-group scan; every other storage type keeps the
/// scalar walk its own `to_u16` conversion needs, unless no value of the type
/// can exceed `max_sample`.
#[inline]
pub(crate) fn samples_exceed<T: ReconSample>(samples: &[T], max_sample: u16) -> bool {
    if max_sample >= T::MAX_VALUE {
        return false;
    }
    T::u16_slice(samples).map_or_else(
        || samples.iter().any(|sample| sample.to_u16() > max_sample),
        |samples| u16_samples_exceed(samples, max_sample),
    )
}

/// Reports whether any `u16` sample exceeds the active bit depth.
///
/// The scan accumulates a running lane-wise peak and compares once at the end
/// rather than comparing and reducing every lane group: a clean buffer — every
/// buffer on a conforming stream — costs one load and one `umax` per group
/// instead of a compare, a cross-lane reduction and a branch. The comparison is
/// against the same peak either way, so the answer is unchanged; only the error
/// path loses an early exit it does not need, because it already rescans sample
/// by sample to name the offender. A ragged tail rereads the last full lane
/// group instead of walking sample by sample.
pub(crate) fn u16_samples_exceed(samples: &[u16], max_sample: u16) -> bool {
    baseline_simd!(simd => u16_peak_exceeds(simd, samples, max_sample))
}

#[inline]
fn u16_peak_exceeds<S: Simd>(simd: S, samples: &[u16], max_sample: u16) -> bool {
    let (chunks, _) = samples.as_chunks::<16>();
    let tail = samples
        .last_chunk::<16>()
        .copied()
        .unwrap_or_else(|| overlapped(samples));
    let peak = chunks
        .iter()
        .fold(u16x16::from_slice(simd, &tail), |peak, chunk| {
            peak.max(u16x16::from_slice(simd, chunk))
        });
    peak.reduce_max() > max_sample
}

/// A slice of fewer than 16 samples as one group of two overlapping copies of
/// the largest power of two that fits, the rest zero; a peak over it is the
/// slice's peak.
fn overlapped<T: Copy + Default>(samples: &[T]) -> [T; 16] {
    fn pair<T: Copy + Default, const W: usize>(samples: &[T]) -> [T; 16] {
        let mut group = [T::default(); 16];
        if let (Some(head), Some(tail)) = (samples.first_chunk::<W>(), samples.last_chunk::<W>()) {
            group[..W].copy_from_slice(head); // splot-copy-ok: gather a short sample run
            group[W..2 * W].copy_from_slice(tail); // splot-copy-ok: gather a short sample run
        }
        group
    }
    match samples.len() {
        0 => [T::default(); 16],
        1 => pair::<T, 1>(samples),
        2..=3 => pair::<T, 2>(samples),
        4..=7 => pair::<T, 4>(samples),
        _ => pair::<T, 8>(samples),
    }
}

/// Samples [`sum_samples`] adds: `u8`, `u16` and non-negative `i16`, each
/// widened to `u16` lanes.
pub trait SummedSample: Copy + Default {
    #[doc(hidden)]
    fn lanes<S: Simd>(simd: S, group: &[Self; 16]) -> u16x16<S>;
}

impl SummedSample for u8 {
    #[inline]
    fn lanes<S: Simd>(simd: S, group: &[u8; 16]) -> u16x16<S> {
        let (low, high) = simd.widen_u8x16(fearless_simd::u8x16::from_slice(simd, group));
        simd.combine_u16x8(low, high)
    }
}

impl SummedSample for u16 {
    #[inline]
    fn lanes<S: Simd>(simd: S, group: &[u16; 16]) -> u16x16<S> {
        u16x16::from_slice(simd, group)
    }
}

impl SummedSample for i16 {
    #[inline]
    fn lanes<S: Simd>(simd: S, group: &[i16; 16]) -> u16x16<S> {
        fearless_simd::i16x16::from_slice(simd, group).bitcast()
    }
}

/// The sum of `samples`, or of every second one from the first when
/// `every_second`, in 16-lane groups; the last group is zero-padded from
/// fixed-size copies, so no sample is added twice.
pub fn sum_samples<T: SummedSample>(samples: &[T], every_second: bool) -> u32 {
    baseline_simd!(simd => sum_lanes(simd, samples, every_second))
}

#[inline]
fn sum_lanes<S: Simd, T: SummedSample>(simd: S, samples: &[T], every_second: bool) -> u32 {
    let (chunks, rest) = samples.as_chunks::<16>();
    let mask = u16x16::from_slice(
        simd,
        &core::array::from_fn::<u16, 16, _>(|lane| {
            if every_second && lane % 2 == 1 {
                0
            } else {
                u16::MAX
            }
        }),
    );
    let mut padded = [T::default(); 16];
    let mut at = 0;
    for width in [8, 4, 2, 1] {
        if rest.len() - at >= width {
            padded[at..at + width].copy_from_slice(&rest[at..at + width]); // splot-copy-ok: zero-pad the last sample group
            at += width;
        }
    }
    let sum = chunks.iter().chain(core::iter::once(&padded)).fold(
        fearless_simd::u32x8::splat(simd, 0),
        |sum, group| {
            let (low, high) = simd.widen_u16x16(T::lanes(simd, group) & mask);
            sum + low + high
        },
    );
    simd.reduce_sum_u32x8(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_and_peaks_match_the_scalar_walk_at_every_length() {
        for len in 0..=80usize {
            let wide: Vec<u16> = (0..len).map(|i| ((i * 7919 + 13) % 1024) as u16).collect();
            let narrow: Vec<u8> = wide.iter().map(|&v| (v >> 2) as u8).collect();
            let signed: Vec<i16> = wide.iter().map(|&v| (v * 8) as i16).collect();
            let as_u32 = |v: &[u16]| v.iter().map(|&x| u32::from(x)).collect::<Vec<_>>();
            for every_second in [false, true] {
                let step = if every_second { 2 } else { 1 };
                let want = |values: &[u32]| values.iter().step_by(step).sum::<u32>();
                assert_eq!(sum_samples(&wide, every_second), want(&as_u32(&wide)));
                let n: Vec<u16> = narrow.iter().map(|&v| u16::from(v)).collect();
                assert_eq!(sum_samples(&narrow, every_second), want(&as_u32(&n)));
                let q: Vec<u16> = signed.iter().map(|&v| v as u16).collect();
                assert_eq!(sum_samples(&signed, every_second), want(&as_u32(&q)));
            }
            for hot in 0..len {
                let mut samples = wide.iter().map(|&v| v.min(1000)).collect::<Vec<_>>();
                assert!(!u16_samples_exceed(&samples, 1000));
                samples[hot] = 1001;
                assert!(u16_samples_exceed(&samples, 1000), "len {len} hot {hot}");
            }
        }
    }
}
