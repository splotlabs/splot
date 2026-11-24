// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Active-bit-depth range scans and edge sums shared by the sample kernels.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`.

use fearless_simd::{Bytes as _, Simd, SimdBase, u16x16};
use splot_core::simd::SimdBackend;

use crate::ReconSample;

/// Reports whether any sample exceeds the active bit depth: never for `u8`
/// storage (`max_sample >= 255`), the lane-group scan for `u16` storage.
#[inline]
pub(crate) fn samples_exceed<T: ReconSample>(samples: &[T], max_sample: u16) -> bool {
    max_sample < T::MAX_VALUE
        && T::u16_slice(samples).is_some_and(|samples| u16_samples_exceed(samples, max_sample))
}

/// Typed [`samples_exceed`] using caller-selected `backend` with no baseline re-probe.
#[inline]
pub(crate) fn samples_exceed_with<B: SimdBackend, T: ReconSample>(
    backend: B,
    samples: &[T],
    max_sample: u16,
) -> bool {
    max_sample < T::MAX_VALUE
        && T::u16_slice(samples)
            .is_some_and(|samples| u16_samples_exceed_with(backend, samples, max_sample))
}

/// Reports whether any `u16` sample exceeds the active bit depth.
///
/// The scan keeps a running lane-wise peak and compares it once at the end, so
/// a clean buffer costs one load and one `umax` per lane group; it has no early
/// exit, because the error path rescans to name the offender. A ragged tail
/// rereads the last full lane group.
pub(crate) fn u16_samples_exceed(samples: &[u16], max_sample: u16) -> bool {
    baseline_simd!(simd => u16_peak_exceeds(simd, samples, max_sample))
}

/// Typed [`u16_samples_exceed`] using caller-selected `backend` with no baseline re-probe.
pub(crate) fn u16_samples_exceed_with<B: SimdBackend>(
    backend: B,
    samples: &[u16],
    max_sample: u16,
) -> bool {
    let narrow = backend.narrow();
    splot_core::with_simd!(narrow, simd => u16_peak_exceeds(simd, samples, max_sample))
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

/// Typed [`sum_samples`] using caller-selected `backend` with no baseline re-probe.
pub fn sum_samples_with<B: SimdBackend, T: SummedSample>(
    backend: B,
    samples: &[T],
    every_second: bool,
) -> u32 {
    let narrow = backend.narrow();
    splot_core::with_simd!(narrow, simd => sum_lanes(simd, samples, every_second))
}

#[inline]
pub(crate) fn sum_lanes<S: Simd, T: SummedSample>(
    simd: S,
    samples: &[T],
    every_second: bool,
) -> u32 {
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
    let add = |sum: fearless_simd::u32x8<S>, group: &[T; 16]| {
        let (low, high) = simd.widen_u16x16(T::lanes(simd, group) & mask);
        sum + low + high
    };
    let mut sum = chunks
        .iter()
        .fold(fearless_simd::u32x8::splat(simd, 0), add);
    if !rest.is_empty() {
        let mut padded = [T::default(); 16];
        let mut at = 0;
        for width in [8, 4, 2, 1] {
            if rest.len() - at >= width {
                padded[at..at + width].copy_from_slice(&rest[at..at + width]); // splot-copy-ok: zero-pad the last sample group
                at += width;
            }
        }
        sum = add(sum, &padded);
    }
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

    #[test]
    fn typed_backend_entry_matches_compatibility_scans() {
        use splot_core::simd::{SimdPolicy, SimdScope, resolve_backend};

        struct ParityProbe<'a>(&'a [u16], &'a [u8], &'a [i16]);
        impl SimdScope<()> for ParityProbe<'_> {
            fn run<B: SimdBackend>(self, backend: B) {
                for every_second in [false, true] {
                    assert_eq!(
                        sum_samples_with(backend, self.0, every_second),
                        sum_samples(self.0, every_second)
                    );
                    assert_eq!(
                        sum_samples_with(backend, self.1, every_second),
                        sum_samples(self.1, every_second)
                    );
                    assert_eq!(
                        sum_samples_with(backend, self.2, every_second),
                        sum_samples(self.2, every_second)
                    );
                }
                assert!(!samples_exceed_with(backend, self.1, 255));
                assert_eq!(
                    samples_exceed_with(backend, self.0, 1000),
                    samples_exceed(self.0, 1000)
                );
                assert_eq!(
                    u16_samples_exceed_with(backend, self.0, 1000),
                    u16_samples_exceed(self.0, 1000)
                );
            }
        }

        let resolved = resolve_backend(SimdPolicy::Auto).expect("auto backend");
        for len in [0usize, 1, 7, 15, 16, 17, 63] {
            let mut wide: Vec<u16> = (0..len).map(|i| ((i * 7919 + 13) % 1024) as u16).collect();
            let narrow: Vec<u8> = wide.iter().map(|&v| (v >> 2) as u8).collect();
            let signed: Vec<i16> = wide.iter().map(|&v| (v * 8) as i16).collect();
            resolved.enter(ParityProbe(&wide, &narrow, &signed));
            if let Some(last) = wide.last_mut() {
                *last = 1001;
                resolved.enter(ParityProbe(&wide, &narrow, &signed));
            }
        }
    }
}
