// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The 8x8 block of a bytes-only luma segment in one 64-lane vector.

use super::*;
use fearless_simd::{i8x64, i16x32, u8x64, u16x32, u64x8};

/// A backend's 64-lane kernel for the 8x8 block of a bytes-only luma
/// segment, `None` where the backend has no vector that wide.
pub(super) trait WideLuma: Copy {
    fn filter<T: crate::ReconSample>(
        self,
        inputs: (&[u8; LUMA_PADDED_BYTES], &CdefFilterPlan),
        mode: u8,
        out: (&mut [T], usize),
    ) -> Option<bool>;
}

impl WideLuma for BaselineSegment {
    #[inline]
    fn filter<T: crate::ReconSample>(
        self,
        _inputs: (&[u8; LUMA_PADDED_BYTES], &CdefFilterPlan),
        _mode: u8,
        _out: (&mut [T], usize),
    ) -> Option<bool> {
        None
    }
}

impl<B: SimdBackend> WideLuma for TypedSegment<B> {
    #[expect(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn filter<T: crate::ReconSample>(
        self,
        inputs: (&[u8; LUMA_PADDED_BYTES], &CdefFilterPlan),
        mode: u8,
        out: (&mut [T], usize),
    ) -> Option<bool> {
        (B::KIND == splot_core::simd::SelectedBackend::Avx512)
            .then(|| luma_wide64_with(self.0.token(), inputs, mode, out))
    }
}

/// [`luma_wide8_with`] on the 64-lane kernel, for a filter that
/// [`cdef_wide8_fits`] admits and its window's [`CdefFilterPlan::mode`].
#[expect(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn luma_wide64_with<S: SimdToken, T: crate::ReconSample>(
    simd: S,
    inputs: (&[u8; LUMA_PADDED_BYTES], &CdefFilterPlan),
    mode: u8,
    out: (&mut [T], usize),
) -> bool {
    let filter = &inputs.1.filter;
    macro_rules! run {
        ($mode:ident) => {
            cdef_by_families!(
                filter,
                cdef_filter_block_wide64::<S, _, $mode>(simd, inputs, out),
                Some(())
            )
        };
    }
    match mode {
        CDEF_LINEAR => run!(CDEF_LINEAR),
        CDEF_CLAMPED => run!(CDEF_CLAMPED),
        _ => run!(CDEF_GENERAL),
    }
    .is_some()
}

/// [`cdef_filter_rows_wide8`] for the 8x8 block of a bytes-only luma segment,
/// its eight rows in one 64-lane vector: the same sums and rounding, with the
/// rows in order and every tap view loaded once.
#[inline(never)]
fn cdef_filter_block_wide64<
    S: SimdToken,
    T: crate::ReconSample,
    const MODE: u8,
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    (bytes, plan): (&[u8; LUMA_PADDED_BYTES], &CdefFilterPlan),
    (out, out_stride): (&mut [T], usize),
) -> Option<()> {
    simd.vectorize(
        #[inline(always)]
        || {
            let filter = &plan.filter;
            let (center_bytes, pri_bytes, sec_bytes) =
                cdef_tap_views::<
                    _,
                    LUMA_BYTE_STRIDE,
                    2,
                    LUMA_BYTE_SPAN,
                    LUMA_PADDED_BYTES,
                    PRI,
                    SEC,
                >(bytes, filter.dir, &LUMA_BYTE_STARTS)?;
            let out = out.get_mut(..out_stride.checked_mul(7)?.checked_add(8)?)?;
            let heavy_first = (filter.pri_str >> filter.coeff_shift) & 1 == 0;
            let pri_double = i8x64::splat(simd, if heavy_first { -1 } else { 0 });
            let pri_weight: i16 = if heavy_first { 2 } else { 3 };
            let constrain =
                |strength: i32, adj: i32| (u8x64::splat(simd, strength as u8), adj as u32);
            let pri = constrain(filter.pri_str, plan.adj[0]);
            let sec = constrain(filter.sec_str, plan.adj[1]);
            let block = |view: &[u8; LUMA_BYTE_SPAN]| -> Option<i8x64<S>> {
                let rows =
                    |half: &[u8; 64]| u8x64::load_array_ref(simd, half).bitcast::<u64x8<S>>();
                let (low, high) = (rows(view.first_chunk()?), rows(view.last_chunk()?));
                Some(simd.unzip_low_u64x8(low, high).bitcast())
            };
            let center_signed = block(center_bytes)?;
            let (mut min, mut max) = (i8x64::splat(simd, 0), i8x64::splat(simd, 0));
            macro_rules! tap {
                ($view:expr, $constrain:expr) => {{
                    let diff = block($view)?.saturating_sub(center_signed);
                    if PRI && SEC {
                        min = min.min(diff);
                        max = max.max(diff);
                    }
                    let (threshold, shift): (u8x64<S>, u32) = $constrain;
                    match MODE {
                        CDEF_LINEAR => diff,
                        CDEF_CLAMPED => {
                            let strength = threshold.bitcast::<i8x64<S>>();
                            diff.min(strength).max(-strength)
                        }
                        _ => {
                            let magnitude = diff.abs().bitcast::<u8x64<S>>();
                            let clip = threshold
                                .saturating_sub(magnitude >> shift)
                                .bitcast::<i8x64<S>>();
                            diff.min(clip).max(-clip)
                        }
                    }
                }};
            }
            let primary = if PRI {
                let pair0 = tap!(pri_bytes[0], pri) + tap!(pri_bytes[1], pri);
                let pair1 = tap!(pri_bytes[2], pri) + tap!(pri_bytes[3], pri);
                pair0 + pair1 + (pair0 & pri_double)
            } else {
                i8x64::splat(simd, 0)
            };
            let secondary = if SEC {
                let view = |k: usize| sec_bytes[k];
                let family0 = tap!(view(0), sec)
                    + tap!(view(1), sec)
                    + tap!(view(2), sec)
                    + tap!(view(3), sec);
                let family1 = tap!(view(4), sec)
                    + tap!(view(5), sec)
                    + tap!(view(6), sec)
                    + tap!(view(7), sec);
                family0 + family0 + family1
            } else {
                i8x64::splat(simd, 0)
            };
            let widen = |lanes: i8x64<S>| -> [i16x32<S>; 2] {
                let (low, high) = simd.widen_i8x64(lanes);
                [low, high]
            };
            let round = |sum: i16x32<S>| (sum + 8 + (sum >> 15)) >> 4;
            let narrow = |[low, high]: [i16x32<S>; 2]| simd.saturating_narrow_i16x32(low, high);
            let offset = if PRI && SEC {
                let (p, s) = (widen(primary), widen(secondary));
                narrow([0, 1].map(|half| round(p[half] * pri_weight + s[half])))
                    .max(min)
                    .min(max)
            } else if PRI {
                narrow(widen(primary).map(|sum| round(sum * pri_weight)))
            } else {
                narrow(widen(secondary).map(round))
            };
            if let Some(out) = T::u8_slice_mut(&mut *out) {
                let filtered: [u8; 64] = ((center_signed + offset).bitcast::<u8x64<S>>()
                    ^ u8x64::splat(simd, 0x80))
                .into();
                for (lanes, row) in filtered.as_chunks::<8>().0.iter().zip(0..) {
                    *cdef_output_row::<8, u8>(out, out_stride, row)? = *lanes;
                }
            } else {
                let offset = widen(offset);
                let center = widen(center_signed).map(|center| center + 128);
                let out = T::u16_slice_mut(&mut *out)?;
                for half in 0..2 {
                    let filtered: [u16; 32] =
                        (center[half] + offset[half]).bitcast::<u16x32<S>>().into();
                    for (lanes, row) in filtered.as_chunks::<8>().0.iter().zip(4 * half..) {
                        *cdef_output_row::<8, u16>(out, out_stride, row)? = *lanes;
                    }
                }
            }
            Some(())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block's biased bytes, the samples of its window within `spread` of
    /// each other, and the range of that window.
    fn block(state: &mut u32, spread: u32) -> ([u8; LUMA_PADDED_BYTES], i32) {
        let mut next = || {
            *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *state >> 8
        };
        let base = next() % (256 - spread);
        let mut bytes = [0u8; LUMA_PADDED_BYTES];
        let (mut min, mut max) = (u8::MAX, 0);
        for (index, byte) in bytes[..12 * LUMA_BYTE_STRIDE].iter_mut().enumerate() {
            let sample = (base + next() % (spread + 1)) as u8;
            *byte = sample ^ 0x80;
            if index % LUMA_BYTE_STRIDE < 12 {
                (min, max) = (min.min(sample), max.max(sample));
            }
        }
        (bytes, i32::from(max) - i32::from(min))
    }

    fn plans() -> impl Iterator<Item = CdefFilterPlan> {
        let strengths = [0, 1, 2, 3, 4, 6, 8, 12, 16, 21]
            .into_iter()
            .flat_map(|pri| [0, 1, 2, 4, 8, 10].map(|sec| (pri, sec)));
        strengths
            .flat_map(|(pri_str, sec_str)| {
                (3..=6).flat_map(move |damping| {
                    (0..8).map(move |dir| {
                        CdefFilterPlan::new(CdefBlockFilter {
                            pri_str,
                            sec_str,
                            damping,
                            dir,
                            coeff_shift: 0,
                        })
                    })
                })
            })
            .filter(|plan| plan.wide8 && (plan.filter.pri_str != 0 || plan.filter.sec_str != 0))
    }

    /// The 64-lane kernel writes what the 32-lane kernel writes, into bytes
    /// and into words, for every direction, tap family and window mode; on a
    /// host with AVX-512 also through its native vectors.
    #[test]
    fn wide_luma_blocks_match_the_32_lane_kernel() {
        let native = Level::new().as_avx512();
        assert!(native.is_some() || std::env::var_os("SPLOT_REQUIRE_AVX512").is_none());
        let mut state = 0x1357_9bdf;
        let mut modes = [0usize; 3];
        for plan in plans() {
            for spread in [1, 6, 40, 255] {
                let (bytes, range) = block(&mut state, spread);
                let mode = plan.mode(range);
                modes[usize::from(mode)] += 1;
                let label = format!("{:?} range {range}", plan.filter);
                let (mut narrow8, mut narrow16) = ([0u8; 64], [0u16; 64]);
                baseline_simd!(simd => {
                    let lanes = (Ordered, BaselineSegment);
                    assert!(luma_wide8_with(simd, lanes, Some(&bytes), range, &plan, (&mut narrow8, 8)));
                    assert!(luma_wide8_with(simd, lanes, Some(&bytes), range, &plan, (&mut narrow16, 8)));
                    let (mut wide8, mut wide16) = ([0u8; 64], [0u16; 64]);
                    assert!(luma_wide64_with(simd, (&bytes, &plan), mode, (&mut wide8, 8)));
                    assert!(luma_wide64_with(simd, (&bytes, &plan), mode, (&mut wide16, 8)));
                    assert_eq!(wide8, narrow8, "{label}");
                    assert_eq!(wide16, narrow16, "{label}");
                });
                if let Some(simd) = native {
                    let (mut wide8, mut wide16) = ([0u8; 64], [0u16; 64]);
                    assert!(luma_wide64_with(
                        simd,
                        (&bytes, &plan),
                        mode,
                        (&mut wide8, 8)
                    ));
                    assert!(luma_wide64_with(
                        simd,
                        (&bytes, &plan),
                        mode,
                        (&mut wide16, 8)
                    ));
                    assert_eq!(wide8, narrow8, "native {label}");
                    assert_eq!(wide16, narrow16, "native {label}");
                }
            }
        }
        assert!(modes.iter().all(|&count| count > 100), "{modes:?}");
    }

    /// Run on one pinned core of an AVX-512 host. Each sample is one 8x8
    /// block through the kernel dispatch, without the gather or the direction
    /// search.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "native AVX-512 timing"]
    fn measure_wide_luma_blocks() {
        use std::{hint::black_box, time::Instant};

        const ITERATIONS: u32 = 1 << 18;
        fn measure(run: impl Fn(&mut [u8; 64])) -> f64 {
            let mut out = [0u8; 64];
            let start = Instant::now();
            for _ in 0..ITERATIONS {
                run(black_box(&mut out));
            }
            black_box(out);
            start.elapsed().as_secs_f64() * 1.0e9 / f64::from(ITERATIONS)
        }

        let level = Level::new();
        let avx2 = level.as_avx2().expect("AVX2 required");
        let avx512 = level.as_avx512().expect("AVX-512 required");
        let mut state = 0x0bad_5eed;
        println!("pri,sec,mode,round,avx2_ns_per_block,avx512_ns_per_block");
        for (pri_str, sec_str) in [(4, 2), (4, 0), (0, 2)] {
            for spread in [1, 6, 255] {
                let plan = CdefFilterPlan::new(CdefBlockFilter {
                    pri_str,
                    sec_str,
                    damping: 5,
                    dir: 3,
                    coeff_shift: 0,
                });
                let (bytes, range) = block(&mut state, spread);
                let mode = plan.mode(range);
                let narrow = || {
                    measure(|out| {
                        let lanes = (InLane, BaselineSegment);
                        let bytes = black_box(Some(&bytes));
                        assert!(luma_wide8_with(avx2, lanes, bytes, range, &plan, (out, 8)));
                    })
                };
                let wide = || {
                    measure(|out| {
                        let inputs = (black_box(&bytes), &plan);
                        assert!(luma_wide64_with(avx512, inputs, mode, (out, 8)));
                    })
                };
                let _ = (narrow(), wide());
                for round in 0..12 {
                    let (avx2, avx512) = if round % 2 == 0 {
                        (narrow(), wide())
                    } else {
                        let avx512 = wide();
                        (narrow(), avx512)
                    };
                    println!(
                        "{pri_str},{sec_str},{mode},{},{avx2:.2},{avx512:.2}",
                        round + 1
                    );
                }
            }
        }
    }
}
