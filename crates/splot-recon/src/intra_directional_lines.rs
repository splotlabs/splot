// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.13.2.8 directional prediction over `u8` and `u16` edges.
//!
//! Every directional zone is a set of lines whose `shift` is fixed and whose
//! `base` steps by one sample: zone-1 rows, zone-3 columns, and the zone-2 above
//! row runs and left column runs. A line is one 4-tap filter over a contiguous
//! edge window. The IDIF taps are `Dr_Interp_Filter[shift]`, and the bilinear
//! `Round2(a * (32 - shift) + b * shift, 5)` is the same filter with taps `[0,
//! 4 * (32 - shift), 4 * shift, 0]` rounded by 7 bits, so both take one line
//! kernel per sample type. Column lines are filtered into a transposed scratch
//! first.
//!
//! Feature tracking: `RECON-INTRA-DIRECTIONAL-ANGLE`.

use fearless_simd::{
    Bytes as _, Simd, SimdBase, i16x16, i32x8, u8x16, u16x8, u16x16, u32x4, u32x8,
};
#[cfg(target_arch = "x86_64")]
use fearless_simd::{SimdFrom, u8x32};

use super::{
    DR_INTERP_FILTER, DirectionalAngleBranch, IntraMiddleDirectionalAngle, directional_shift,
    for_each_middle_left_run,
};
use crate::intra_dc_math::IntraLaneSample;
use crate::{IntraRectBlockSize, ReconError, Result};

/// Samples a padded edge holds: the longest edge (the one-sided IDIF `64 + 64
/// + 4 + 2 * 3`), one leading sample and the kernel over-read.
const PADDED_LEN: usize = 184;
/// Widest block side.
const MAX_SIDE: usize = 64;
/// Samples past a line's samples its kernels read: three taps and the rest of a
/// 32-sample step.
const OVER_READ: usize = 40;

/// The two § 7.13.2.8 line filters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LineFilter {
    Idif,
    Bilinear,
}

impl LineFilter {
    /// The 4-tap kernel at `shift`, or `None` for the `shift == 0` copy of the
    /// second tap. Every IDIF tap but the unused `128` of shift 0 fits `i8`, as
    /// do the scaled bilinear taps.
    fn taps(self, shift: u16) -> Option<[i8; 4]> {
        if shift == 0 {
            return None;
        }
        Some(match self {
            Self::Idif => IDIF_TAPS[usize::from(shift & 31)],
            Self::Bilinear => [0, (4 * (32 - shift)) as i8, (4 * shift) as i8, 0],
        })
    }
}

/// `DR_INTERP_FILTER` as `i8` taps.
static IDIF_TAPS: [[i8; 4]; 32] = {
    let mut taps = [[0i8; 4]; 32];
    let mut shift = 0;
    while shift < 32 {
        let mut tap = 0;
        while tap < 4 {
            taps[shift][tap] = DR_INTERP_FILTER[shift][tap] as i8;
            tap += 1;
        }
        shift += 1;
    }
    taps
};

/// A sample type with a directional line kernel and a column transpose.
pub(super) trait LineSample: IntraLaneSample + Default {
    /// Filters `out.len()` samples: `out[j] = Clip1(Round2(Σ taps[t] *
    /// window[j + t], 7))` with `Clip1` to `max`, `taps` those of `filter`.
    /// `window` holds [`OVER_READ`] samples past the line.
    fn filter(window: &[Self], filter: LineFilter, taps: [i8; 4], max: u16, out: &mut [Self]);

    /// Copies `out.len()` samples of `window`, which holds [`OVER_READ`]
    /// samples past them.
    fn copy(window: &[Self], out: &mut [Self]);

    /// Sixteen small-block lanes: `Clip1(Round2(sum of taps[t] * weights[t], 7))`
    /// with `Clip1` to `max`, `last` where `along >= active`.
    fn small_lanes(
        lanes: &SmallLanes<Self>,
        filter: LineFilter,
        max: u16,
        last: Self,
    ) -> [Self; 16];

    /// Sixteen lanes of `width`-sample rows: `left` where the lane's column is
    /// below its row's left run `runs`, else `above`.
    fn select_left(
        left: &[Self; 16],
        above: &[Self; 16],
        runs: &[u16; 16],
        along: &[u16; 16],
    ) -> [Self; 16] {
        let mut values = [Self::default(); 16];
        baseline_simd!(simd => {
            let pick = simd.simd_lt_u16x16(
                u16x16::from_slice(simd, along),
                u16x16::from_slice(simd, runs),
            );
            let chosen = simd.select_u16x16(
                pick,
                Self::load16(simd, left),
                Self::load16(simd, above),
            );
            Self::store16(simd, chosen, &mut values);
        });
        values
    }

    /// Writes the first `width` columns of the column-major `columns` (column
    /// `c` at `c * height`) as the rows of a `height`-row block. `width` and
    /// `height` are multiples of 4; `columns` holds [`COLUMN_SLACK`] samples
    /// past its last column.
    fn transpose(width: usize, height: usize, columns: &[Self], output: &mut [Self], stride: usize);
}

impl LineSample for u8 {
    #[inline]
    fn small_lanes(lanes: &SmallLanes<u8>, _filter: LineFilter, _max: u16, last: u8) -> [u8; 16] {
        baseline_simd!(simd => {
            let tap = |t: usize| {
                let (low, high) = simd.widen_u8x16(u8x16::from_slice(simd, &lanes.taps[t][..16]));
                simd.combine_u16x8(low, high) * u16x16::from_slice(simd, &lanes.weights[t][..16])
            };
            let sum = tap(0) + tap(1) + tap(2) + tap(3) + (LINE_BIAS * 128 + 64);
            let values =
                simd.saturating_sub_u16x16(sum >> 7, u16x16::splat(simd, LINE_BIAS)).min(255);
            let values = lanes.clamp_tail(simd, values, u16::from(last));
            let (low, high) = simd.split_u16x16(values);
            simd.relaxed_narrow_u16x8(low, high).into()
        })
    }

    #[inline]
    fn filter(window: &[u8], _filter: LineFilter, taps: [i8; 4], _max: u16, out: &mut [u8]) {
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
            let pair = |low: i8, high: i8| i16::from_le_bytes([low as u8, high as u8]);
            line_avx2(
                token,
                window,
                pair(taps[0], taps[1]),
                pair(taps[2], taps[3]),
                out,
            );
            return;
        }
        baseline_simd!(simd => line_lanes(simd, window, taps, out));
    }

    #[inline]
    fn copy(window: &[u8], out: &mut [u8]) {
        store_chunks::<u8, 16>(out, |at| window_chunk(window, at));
    }

    fn transpose(width: usize, height: usize, columns: &[u8], output: &mut [u8], stride: usize) {
        baseline_simd!(simd => transpose_tiles(simd, width, height, columns, output, stride));
    }
}

impl LineSample for u16 {
    #[inline]
    fn small_lanes(lanes: &SmallLanes<u16>, filter: LineFilter, max: u16, last: u16) -> [u16; 16] {
        baseline_simd!(simd => small_lanes_u16(simd, lanes, filter, max, last))
    }

    #[allow(
        clippy::inline_always,
        reason = "measured: one call per line was the line cost"
    )]
    #[inline(always)]
    fn filter(window: &[u16], filter: LineFilter, taps: [i8; 4], max: u16, out: &mut [u16]) {
        if filter == LineFilter::Bilinear {
            let weights = [taps[1], taps[2]].map(|tap| tap as u8 as u16 >> 2);
            baseline_simd!(simd => bilinear_lanes_u16(simd, window, weights, out));
            return;
        }
        baseline_simd!(simd => line_lanes_u16(simd, window, taps, max, out));
    }

    #[inline]
    fn copy(window: &[u16], out: &mut [u16]) {
        store_chunks::<u16, 8>(out, |at| window_chunk(window, at));
    }

    fn transpose(width: usize, height: usize, columns: &[u16], output: &mut [u16], stride: usize) {
        baseline_simd!(simd => transpose_tiles_u16(simd, width, height, columns, output, stride));
    }
}

/// An edge copied after `front` zero samples into a buffer every line kernel
/// may over-read, with the `Clip1` maximum of its bit depth.
pub(super) struct PaddedEdge<P: LineSample> {
    samples: [P; PADDED_LEN],
    max: u16,
}

impl<P: LineSample> PaddedEdge<P> {
    pub(super) fn new(edge: &[P], front: usize, max: u16) -> Result<Self> {
        let mut samples = [P::default(); PADDED_LEN];
        samples
            .get_mut(front..front + edge.len())
            .filter(|_| front + edge.len() + OVER_READ <= PADDED_LEN)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "directional padded edge length",
            })?
            .copy_from_slice(edge); // splot-copy-ok: pad the directional edge for the line kernels
        Ok(Self { samples, max })
    }

    /// Filters `out.len()` samples from the window at `first`: `out[j] =
    /// Clip1(Round2(Σ taps[t] * edge[first + j + t], 7))`.
    #[allow(
        clippy::inline_always,
        reason = "measured: one call per line was the line cost"
    )]
    #[inline(always)]
    pub(super) fn line(
        &self,
        filter: LineFilter,
        first: usize,
        shift: u16,
        out: &mut [P],
    ) -> Result<()> {
        if out.is_empty() {
            return Ok(());
        }
        let window = self
            .samples
            .get(first..first + out.len() + OVER_READ)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "directional line window",
            })?;
        let Some(taps) = filter.taps(shift) else {
            P::copy(&window[1..], out);
            return Ok(());
        };
        P::filter(window, filter, taps, self.max, out);
        Ok(())
    }
}

/// Bias that lifts every 8-bit 4-tap sum above zero in [`line_lanes`]: the
/// negative taps total at least `-24`, so `-24 * 255 + 48 * 128 > 0`, and the
/// positive at most `152`, so `152 * 255 + 48 * 128 + 64` fits `u16`.
const LINE_BIAS: u16 = 48;

/// The portable 8-bit line kernel in wrapping `u16` lanes: the biased sum is
/// exact, so `(sum >> 7) - 48` is the signed `Round2`, clipped by the
/// saturating subtract and the minimum.
#[allow(
    clippy::inline_always,
    reason = "measured: a call per line was the line cost"
)]
#[inline(always)]
fn line_lanes<S: Simd>(simd: S, window: &[u8], taps: [i8; 4], out: &mut [u8]) {
    let taps = taps.map(|tap| i16::from(tap) as u16);
    let bias = u16x16::splat(simd, LINE_BIAS);
    store_chunks::<u8, 16>(out, |at| {
        let load = |tap: usize| {
            let (low, high) =
                simd.widen_u8x16(u8x16::from_slice(simd, &window[at + tap..at + tap + 16]));
            simd.combine_u16x8(low, high)
        };
        let sum = load(0) * taps[0]
            + load(1) * taps[1]
            + load(2) * taps[2]
            + load(3) * taps[3]
            + (LINE_BIAS * 128 + 64);
        let values = simd.saturating_sub_u16x16(sum >> 7, bias).min(255);
        let (low, high) = simd.split_u16x16(values);
        simd.relaxed_narrow_u16x8(low, high).into()
    });
}

/// Writes `out` from the `N`-sample chunks `lanes(at)` of `out[at..at + N]`:
/// the whole chunks, then the last `N` samples once more (overlapping, so the
/// same values), or for a line shorter than `N` one chunk by two overlapping
/// power-of-two stores, so no store is ever partial.
#[allow(clippy::inline_always, reason = "keeps every store a fixed-size one")]
#[inline(always)]
fn store_chunks<P: Copy, const N: usize>(out: &mut [P], mut lanes: impl FnMut(usize) -> [P; N]) {
    let len = out.len();
    if len < N {
        if len > 0 {
            store_short(out, &lanes(0));
        }
        return;
    }
    let mut at = 0;
    while let Some(chunk) = out.get_mut(at..).and_then(<[P]>::first_chunk_mut::<N>) {
        *chunk = lanes(at);
        at += N;
    }
    if at < len
        && let Some(chunk) = out.last_chunk_mut::<N>()
    {
        *chunk = lanes(len - N);
    }
}

/// The `N` samples of `window` at `at`, zero past its end.
#[inline]
fn window_chunk<P: Copy + Default, const N: usize>(window: &[P], at: usize) -> [P; N] {
    window
        .get(at..)
        .and_then(<[P]>::first_chunk::<N>)
        .copied()
        .unwrap_or([P::default(); N])
}

/// Stores the first `out.len()` (`1..N`) of `lanes` as two overlapping copies
/// of the largest power of two that fits.
#[allow(clippy::inline_always, reason = "keeps every store a fixed-size one")]
#[inline(always)]
fn store_short<P: Copy, const N: usize>(out: &mut [P], lanes: &[P; N]) {
    fn pair<P: Copy, const W: usize>(out: &mut [P], lanes: &[P]) {
        let len = out.len();
        if let (Some(head), Some(from)) = (out.first_chunk_mut::<W>(), lanes.first_chunk::<W>()) {
            *head = *from;
        }
        if let (Some(tail), Some(from)) = (
            out.last_chunk_mut::<W>(),
            lanes.get(len - W..).and_then(<[P]>::first_chunk::<W>),
        ) {
            *tail = *from;
        }
    }
    match out.len() {
        0 => {}
        1 => out[0] = lanes[0],
        2..=3 => pair::<P, 2>(out, lanes),
        4..=7 => pair::<P, 4>(out, lanes),
        8..=15 => pair::<P, 8>(out, lanes),
        16..=31 => pair::<P, 16>(out, lanes),
        _ => pair::<P, 32>(out, lanes),
    }
}

/// The `u16` IDIF line kernel, eight samples per step: two pairwise
/// multiply-adds of the interleaved windows `(edge[j], edge[j + 1])` and
/// `(edge[j + 2], edge[j + 3])` with the tap pairs, in `i32` sums. Callers
/// validate the edge samples to the bit depth, so each fits an `i16` lane.
#[allow(
    clippy::inline_always,
    reason = "measured: a call per line was the line cost"
)]
#[inline(always)]
fn line_lanes_u16<S: Simd>(simd: S, window: &[u16], taps: [i8; 4], max: u16, out: &mut [u16]) {
    let pair = |low: i8, high: i8| {
        i32x8::splat(simd, i32::from(low as i16 as u16) | i32::from(high) << 16)
            .bitcast::<i16x16<S>>()
    };
    let (taps01, taps23) = (pair(taps[0], taps[1]), pair(taps[2], taps[3]));
    let max = i32::from(max);
    store_chunks::<u16, 8>(out, |at| {
        let load = |tap: usize| u16x8::from_slice(simd, &window[at + tap..at + tap + 8]);
        let madd = |near: u16x8<S>, far: u16x8<S>, weights: i16x16<S>| {
            let pairs = simd.combine_u16x8(
                simd.zip_low_u16x8(near, far),
                simd.zip_high_u16x8(near, far),
            );
            simd.pairwise_dot_product_i16x16(pairs.bitcast::<i16x16<S>>(), weights)
        };
        let sum = madd(load(0), load(1), taps01) + madd(load(2), load(3), taps23);
        let values = ((sum + 64) >> 7).max(0).min(max);
        let (low, high) = simd.split_u32x8(values.bitcast());
        simd.relaxed_narrow_u32x4(low, high).into()
    });
}

/// [`LineSample::small_lanes`] over `u16` samples: the bilinear form in `u16`
/// lanes, the IDIF form as two pairwise multiply-adds in `i32` lanes.
#[inline]
fn small_lanes_u16<S: Simd>(
    simd: S,
    lanes: &SmallLanes<u16>,
    filter: LineFilter,
    max: u16,
    last: u16,
) -> [u16; 16] {
    let load = |t: usize| u16x16::from_slice(simd, &lanes.taps[t][..16]);
    let weight = |t: usize| u16x16::from_slice(simd, &lanes.weights[t][..16]);
    let values = if filter == LineFilter::Bilinear {
        (load(1) * (weight(1) >> 2) + load(2) * (weight(2) >> 2) + 16) >> 5
    } else {
        let zip = |x: u16x16<S>, y: u16x16<S>| {
            let ((x0, x1), (y0, y1)) = (simd.split_u16x16(x), simd.split_u16x16(y));
            [
                simd.combine_u16x8(simd.zip_low_u16x8(x0, y0), simd.zip_high_u16x8(x0, y0)),
                simd.combine_u16x8(simd.zip_low_u16x8(x1, y1), simd.zip_high_u16x8(x1, y1)),
            ]
        };
        let madd = |a: usize, b: usize| {
            let (samples, weights) = (zip(load(a), load(b)), zip(weight(a), weight(b)));
            [0, 1].map(|half| {
                simd.pairwise_dot_product_i16x16(
                    samples[half].bitcast::<i16x16<S>>(),
                    weights[half].bitcast::<i16x16<S>>(),
                )
            })
        };
        let (near, far) = (madd(0, 1), madd(2, 3));
        let max = i32::from(max);
        let [low, high] = [0, 1].map(|half| {
            let values: u32x8<S> = ((near[half] + far[half] + 64) >> 7)
                .max(0)
                .min(max)
                .bitcast();
            let (a, b) = simd.split_u32x8(values);
            simd.relaxed_narrow_u32x4(a, b)
        });
        simd.combine_u16x8(low, high)
    };
    lanes.clamp_tail(simd, values, last).into()
}

/// The `u16` bilinear line kernel, sixteen samples per step: `Round2(a * (32 -
/// shift) + b * shift, 5)` of the windows at taps 1 and 2. Callers validate the
/// edge samples to the bit depth (at most 10 bits), so each weighted sum fits
/// `u16` lanes.
#[allow(
    clippy::inline_always,
    reason = "measured: a call per line was the line cost"
)]
#[inline(always)]
fn bilinear_lanes_u16<S: Simd>(simd: S, window: &[u16], [near, far]: [u16; 2], out: &mut [u16]) {
    store_chunks::<u16, 16>(out, |at| {
        let load = |tap: usize| u16x16::from_slice(simd, &window[at + tap..at + tap + 16]);
        ((load(1) * near + load(2) * far + 16) >> 5).into()
    });
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// The AVX2 line kernel, 32 samples per step: in-lane byte interleaves of
    /// the windows at taps 0/1 and 2/3, two `pmaddubsw`, a saturating add, a
    /// `pmulhrsw` by 256 for `Round2(sum, 7)` and an unsigned pack. A sum past
    /// `i16::MAX` saturates to a value that still packs to 255.
    #[inline]
    fn line_avx2(token: Avx2, window: &[u8], pair01: i16, pair23: i16, out: &mut [u8]) {
        use core::arch::x86_64::{
            __m256i, _mm256_adds_epi16, _mm256_maddubs_epi16, _mm256_mulhrs_epi16,
            _mm256_packus_epi16, _mm256_set1_epi16, _mm256_unpackhi_epi8, _mm256_unpacklo_epi8,
        };
        if out.len() <= 16 {
            return short_line_avx2(token, window, pair01, pair23, out);
        }
        let taps01 = _mm256_set1_epi16(pair01);
        let taps23 = _mm256_set1_epi16(pair23);
        let round = _mm256_set1_epi16(256);
        let chunk = |at: usize| -> [u8; 32] {
            macro_rules! load {
                ($tap:expr) => {{
                    let lanes: __m256i =
                        u8x32::from_slice(token, &window[at + $tap..at + $tap + 32]).into();
                    lanes
                }};
            }
            let (x0, x1, x2, x3) = (load!(0), load!(1), load!(2), load!(3));
            let low = _mm256_adds_epi16(
                _mm256_maddubs_epi16(_mm256_unpacklo_epi8(x0, x1), taps01),
                _mm256_maddubs_epi16(_mm256_unpacklo_epi8(x2, x3), taps23),
            );
            let high = _mm256_adds_epi16(
                _mm256_maddubs_epi16(_mm256_unpackhi_epi8(x0, x1), taps01),
                _mm256_maddubs_epi16(_mm256_unpackhi_epi8(x2, x3), taps23),
            );
            u8x32::simd_from(
                token,
                _mm256_packus_epi16(
                    _mm256_mulhrs_epi16(low, round),
                    _mm256_mulhrs_epi16(high, round),
                ),
            )
            .into()
        };
        store_chunks::<u8, 32>(out, chunk);
    }
);

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// [`line_avx2`] in 128-bit lanes for lines of at most 16 samples.
    #[inline]
    fn short_line_avx2(token: Avx2, window: &[u8], pair01: i16, pair23: i16, out: &mut [u8]) {
        use core::arch::x86_64::{
            __m128i, _mm_adds_epi16, _mm_maddubs_epi16, _mm_mulhrs_epi16, _mm_packus_epi16,
            _mm_set1_epi16, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
        };
        macro_rules! load {
            ($tap:expr) => {{
                let lanes: __m128i = u8x16::from_slice(token, &window[$tap..$tap + 16]).into();
                lanes
            }};
        }
        let (taps01, taps23) = (_mm_set1_epi16(pair01), _mm_set1_epi16(pair23));
        let (x0, x1, x2, x3) = (load!(0), load!(1), load!(2), load!(3));
        let low = _mm_adds_epi16(
            _mm_maddubs_epi16(_mm_unpacklo_epi8(x0, x1), taps01),
            _mm_maddubs_epi16(_mm_unpacklo_epi8(x2, x3), taps23),
        );
        let high = _mm_adds_epi16(
            _mm_maddubs_epi16(_mm_unpackhi_epi8(x0, x1), taps01),
            _mm_maddubs_epi16(_mm_unpackhi_epi8(x2, x3), taps23),
        );
        let round = _mm_set1_epi16(256);
        let packed = u8x16::simd_from(
            token,
            _mm_packus_epi16(_mm_mulhrs_epi16(low, round), _mm_mulhrs_epi16(high, round)),
        );
        let bytes: [u8; 16] = packed.into();
        store_chunks::<u8, 16>(out, |_| bytes);
    }
);

/// One line of a one-sided zone: its window, shift, and how many of its
/// samples precede the clamped `maxBase` tail.
#[derive(Clone, Copy, Debug)]
pub(super) struct Line {
    pub(super) first: usize,
    pub(super) shift: u16,
    pub(super) active: usize,
}

/// Samples a small block's padded edge holds: an 8x8 block's longest edge (`8 +
/// 8 + 4 + 2 * 3`), one leading sample and an 8-sample line plus three taps
/// past its last base.
const SMALL_PADDED_LEN: usize = 48;

/// The sixteen lanes of up to four small-block lines: their four tap windows,
/// tap weights, positions along the line and active lengths.
pub(super) struct SmallLanes<P> {
    taps: [[P; 32]; 4],
    weights: [[u16; 32]; 4],
    along: [u16; 16],
    active: [u16; 16],
}

impl<P> SmallLanes<P> {
    /// `values` with `last` in every lane at or past its line's active length.
    #[inline]
    fn clamp_tail<S: Simd>(&self, simd: S, values: u16x16<S>, last: u16) -> u16x16<S> {
        let tail = simd.simd_ge_u16x16(
            u16x16::from_slice(simd, &self.along),
            u16x16::from_slice(simd, &self.active),
        );
        simd.select_u16x16(tail, u16x16::splat(simd, last), values)
    }
}

/// Whether [`write_one_sided_small`] takes a block of `size`: at most 64
/// samples in lines of 4 or 8.
pub(super) fn is_small_one_sided(size: IntraRectBlockSize, branch: DirectionalAngleBranch) -> bool {
    let along = match branch {
        DirectionalAngleBranch::Above { .. } => size.width(),
        DirectionalAngleBranch::Left { .. } => size.height(),
    };
    size.width() * size.height() <= 64 && matches!(along, 4 | 8)
}

/// [`write_one_sided`] for a block that [`is_small_one_sided`]: four 4-sample
/// or two 8-sample lines per sixteen lanes, read from a small padded copy of
/// `edge` after `front` zero samples, with no per-line kernel call.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_one_sided_small<P: LineSample>(
    size: IntraRectBlockSize,
    branch: DirectionalAngleBranch,
    filter: LineFilter,
    (edge, front): (&[P], usize),
    max: u16,
    last: P,
    line: impl FnMut(usize) -> Result<Line>,
    output: &mut [P],
    stride: usize,
) -> Result<()> {
    let along = match branch {
        DirectionalAngleBranch::Above { .. } => size.width(),
        DirectionalAngleBranch::Left { .. } => size.height(),
    };
    let request = SmallRequest {
        size,
        branch,
        filter,
        max,
        last,
    };
    if along == 4 {
        small_lines::<P, 4>(request, (edge, front), line, output, stride)
    } else {
        small_lines::<P, 8>(request, (edge, front), line, output, stride)
    }
}

/// The block facts of one [`write_one_sided_small`] call.
#[derive(Clone, Copy)]
struct SmallRequest<P> {
    size: IntraRectBlockSize,
    branch: DirectionalAngleBranch,
    filter: LineFilter,
    max: u16,
    last: P,
}

#[inline]
fn small_lines<P: LineSample, const L: usize>(
    request: SmallRequest<P>,
    (edge, front): (&[P], usize),
    mut line: impl FnMut(usize) -> Result<Line>,
    output: &mut [P],
    stride: usize,
) -> Result<()> {
    let SmallRequest {
        size,
        branch,
        filter,
        max,
        last,
    } = request;
    let overflow = || ReconError::ArithmeticOverflow {
        context: "small directional block edge",
    };
    if front + edge.len() + L + 3 > SMALL_PADDED_LEN || edge.len() < 8 {
        return Err(overflow());
    }
    let mut padded = [last; SMALL_PADDED_LEN];
    padded[0] = P::default();
    for at in (0..edge.len() - 8).step_by(8).chain([edge.len() - 8]) {
        if let (Some(to), Some(from)) = (
            padded
                .get_mut(front + at..)
                .and_then(<[P]>::first_chunk_mut::<8>),
            edge.get(at..).and_then(<[P]>::first_chunk::<8>),
        ) {
            *to = *from;
        }
    }
    let rows = matches!(branch, DirectionalAngleBranch::Above { .. });
    let lines = if rows { size.height() } else { size.width() };
    let mut columns = [P::default(); 64 + COLUMN_SLACK];
    let mut lanes = SmallLanes {
        taps: [[P::default(); 32]; 4],
        weights: [[0u16; 32]; 4],
        along: core::array::from_fn(|lane| (lane % L) as u16),
        active: [0; 16],
    };
    for group in (0..lines).step_by(16 / L) {
        for j in 0..16 / L {
            let spec = line(group + j)?;
            let first = if spec.active == 0 { 0 } else { spec.first };
            let window = padded
                .get(first..)
                .and_then(<[P]>::first_chunk::<19>)
                .ok_or_else(overflow)?;
            let taps = filter
                .taps(spec.shift)
                .map_or([0, 128, 0, 0], |taps| taps.map(|tap| i16::from(tap) as u16));
            put_line::<P, L>(&mut lanes, window, j * L, taps);
            if let Some(to) = lanes
                .active
                .get_mut(j * L..)
                .and_then(<[u16]>::first_chunk_mut::<L>)
            {
                *to = [spec.active.min(L) as u16; L];
            }
        }
        let values = P::small_lanes(&lanes, filter, max, last);
        if rows {
            for j in 0..16 / L {
                if let (Some(row), Some(from)) = (
                    output
                        .get_mut((group + j) * stride..)
                        .and_then(<[P]>::first_chunk_mut::<L>),
                    values.get(j * L..).and_then(<[P]>::first_chunk::<L>),
                ) {
                    *row = *from;
                }
            }
        } else if let Some(to) = columns
            .get_mut(group * L..)
            .and_then(<[P]>::first_chunk_mut::<16>)
        {
            *to = values;
        }
    }
    if !rows {
        P::transpose(size.width(), size.height(), &columns, output, stride);
    }
    Ok(())
}

/// `edge` after `front` zero samples in a [`SMALL_PADDED_LEN`] buffer whose
/// other samples repeat `fill`, by overlapping fixed-size copies.
fn small_padded<P: Copy + Default>(
    edge: &[P],
    front: usize,
    fill: P,
) -> Result<[P; SMALL_PADDED_LEN]> {
    fn copy<P: Copy, const W: usize>(to: &mut [P], from: &[P]) {
        let len = from.len();
        for at in (0..len - W).step_by(W).chain([len - W]) {
            if let (Some(to), Some(from)) = (
                to.get_mut(at..).and_then(<[P]>::first_chunk_mut::<W>),
                from.get(at..).and_then(<[P]>::first_chunk::<W>),
            ) {
                *to = *from;
            }
        }
    }
    let mut padded = [fill; SMALL_PADDED_LEN];
    padded[0] = P::default();
    let to = padded
        .get_mut(front..front + edge.len())
        .filter(|_| front + edge.len() + 19 <= SMALL_PADDED_LEN)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "small directional block edge",
        })?;
    match edge.len() {
        0 => {}
        1 => to[0] = edge[0],
        2..=3 => copy::<P, 2>(to, edge),
        4..=7 => copy::<P, 4>(to, edge),
        _ => copy::<P, 8>(to, edge),
    }
    Ok(padded)
}

/// Whether [`write_middle_small`] takes a block of `size`: at most 64 samples.
pub(super) fn is_small_middle(size: IntraRectBlockSize) -> bool {
    size.width() * size.height() <= 64
}

/// [`write_middle`] for a block that [`is_small_middle`]: every sample's above
/// candidate from its row's line and left candidate from its column's line,
/// each line copied whole into sixteen-lane groups from the first lane it
/// owns, then one select per lane by the row's left run.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_middle_small<P: LineSample>(
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    filter: LineFilter,
    mrl_index: usize,
    [left, above]: [&[P]; 2],
    (edge_front, front): (usize, i32),
    max: u16,
    output: &mut [P],
    stride: usize,
) -> Result<()> {
    let (Some(&left_last), Some(&above_last)) = (left.last(), above.last()) else {
        return Err(ReconError::ArithmeticOverflow {
            context: "middle directional angle empty edge",
        });
    };
    let edges = [
        small_padded(left, edge_front, left_last)?,
        small_padded(above, edge_front, above_last)?,
    ];
    let request = MiddleRequest {
        angle,
        filter,
        mrl: mrl_index as i32,
        front,
        max,
    };
    match (size.width(), size.height()) {
        (4, 4) => middle_small::<P, 4, 4>(request, &edges, output, stride),
        (4, 8) => middle_small::<P, 4, 8>(request, &edges, output, stride),
        (8, 4) => middle_small::<P, 8, 4>(request, &edges, output, stride),
        (8, 8) => middle_small::<P, 8, 8>(request, &edges, output, stride),
        (4, 16) => middle_small::<P, 4, 16>(request, &edges, output, stride),
        (16, 4) => middle_small::<P, 16, 4>(request, &edges, output, stride),
        _ => Err(ReconError::ArithmeticOverflow {
            context: "small middle directional block size",
        }),
    }
}

/// The angle facts of one [`write_middle_small`] call.
#[derive(Clone, Copy)]
struct MiddleRequest {
    angle: IntraMiddleDirectionalAngle,
    filter: LineFilter,
    mrl: i32,
    front: i32,
    max: u16,
}

/// Copies line `window` from its lane `from` into lane group `lanes` at
/// `at`, with the line's `taps` weights; `at + N` may pass the group into
/// its slack.
#[inline]
fn put_line<P: Copy, const N: usize>(
    lanes: &mut SmallLanes<P>,
    window: &[P],
    at: usize,
    taps: [u16; 4],
) {
    for (t, &tap) in taps.iter().enumerate() {
        if let (Some(to), Some(from)) = (
            lanes.taps[t]
                .get_mut(at..)
                .and_then(<[P]>::first_chunk_mut::<N>),
            window.get(t..).and_then(<[P]>::first_chunk::<N>),
        ) {
            *to = *from;
        }
        if let Some(to) = lanes.weights[t]
            .get_mut(at..)
            .and_then(<[u16]>::first_chunk_mut::<N>)
        {
            *to = [tap; N];
        }
    }
}

/// The 19-sample tap window of a small block's line from padded position
/// `first`.
fn small_window<P>(edge: &[P; SMALL_PADDED_LEN], first: i32) -> Option<&[P; 19]> {
    usize::try_from(first)
        .ok()
        .and_then(|first| edge.get(first..))
        .and_then(<[P]>::first_chunk::<19>)
}

#[inline]
fn middle_small<P: LineSample, const W: usize, const H: usize>(
    request: MiddleRequest,
    [left, above]: &[[P; SMALL_PADDED_LEN]; 2],
    output: &mut [P],
    stride: usize,
) -> Result<()> {
    let MiddleRequest {
        angle,
        filter,
        mrl,
        front,
        max,
    } = request;
    let overflow = || ReconError::ArithmeticOverflow {
        context: "middle directional angle logical edge coverage",
    };
    let branch = angle.branch();
    let (dx, dy) = (i32::from(branch.dx), i32::from(branch.dy));
    let taps = |idx: i32| {
        filter
            .taps(directional_shift(idx))
            .map_or([0, 128, 0, 0], |taps| taps.map(|tap| i16::from(tap) as u16))
    };
    let mut left_runs = [0usize; H];
    for (row, run) in left_runs.iter_mut().enumerate() {
        let above_idx = -(row as i32 + 1 + mrl) * dx;
        *run = usize::try_from(-1 - mrl - (above_idx >> 6))
            .unwrap_or(0)
            .min(W);
    }
    let mut lanes = SmallLanes {
        taps: [[P::default(); 32]; 4],
        weights: [[0u16; 32]; 4],
        along: [0; 16],
        active: [u16::MAX; 16],
    };
    let mut above_values = [P::default(); 64];
    for group in (0..H).step_by(16 / W.min(16)) {
        for (row, &run) in left_runs.iter().enumerate().skip(group).take(16 / W) {
            if run == W {
                continue;
            }
            let above_idx = 64 * run as i32 - (row as i32 + 1 + mrl) * dx;
            let from = small_window(above, (above_idx >> 6) + front).ok_or_else(overflow)?;
            put_line::<P, W>(&mut lanes, from, (row - group) * W + run, taps(above_idx));
        }
        let values = P::small_lanes(&lanes, filter, max, P::default());
        if let Some(to) = above_values
            .get_mut(group * W..)
            .and_then(<[P]>::first_chunk_mut::<16>)
        {
            *to = values;
        }
    }
    let left_columns = left_runs.iter().copied().max().unwrap_or(0);
    let mut columns = [P::default(); 64 + COLUMN_SLACK];
    for group in (0..left_columns).step_by(16 / H) {
        for column in group..(group + 16 / H).min(left_columns) {
            let first_row = left_runs.iter().position(|&run| run > column).unwrap_or(H);
            let left_idx = 64 * first_row as i32 - (column as i32 + 1 + mrl) * dy;
            let from = small_window(left, (left_idx >> 6) + front).ok_or_else(overflow)?;
            put_line::<P, H>(
                &mut lanes,
                from,
                (column - group) * H + first_row,
                taps(left_idx),
            );
        }
        let values = P::small_lanes(&lanes, filter, max, P::default());
        if let Some(to) = columns
            .get_mut(group * H..)
            .and_then(<[P]>::first_chunk_mut::<16>)
        {
            *to = values;
        }
    }
    let mut left_values = [P::default(); 64];
    if left_columns > 0 {
        P::transpose(W, H, &columns, &mut left_values, W);
    }
    let along: [u16; 16] = core::array::from_fn(|lane| (lane % W) as u16);
    for group in (0..W * H).step_by(16) {
        let runs: [u16; 16] =
            core::array::from_fn(|lane| left_runs[((group + lane) / W).min(H - 1)] as u16);
        let (Some(left), Some(above)) = (
            left_values.get(group..).and_then(<[P]>::first_chunk::<16>),
            above_values.get(group..).and_then(<[P]>::first_chunk::<16>),
        ) else {
            continue;
        };
        let values = if left_columns == 0 {
            *above
        } else {
            P::select_left(left, above, &runs, &along)
        };
        for row in 0..16 / W {
            if let (Some(out), Some(from)) = (
                output
                    .get_mut((group / W + row) * stride..)
                    .and_then(<[P]>::first_chunk_mut::<W>),
                values.get(row * W..).and_then(<[P]>::first_chunk::<W>),
            ) {
                *out = *from;
            }
        }
    }
    Ok(())
}

/// Writes a zone-1 (`rows`) or zone-3 prediction whose `line(i)` gives row or
/// column `i`, filling each line past its active samples with `last`.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_one_sided<P: LineSample>(
    size: IntraRectBlockSize,
    branch: DirectionalAngleBranch,
    filter: LineFilter,
    edge: &PaddedEdge<P>,
    last: P,
    mut line: impl FnMut(usize) -> Result<Line>,
    output: &mut [P],
    stride: usize,
) -> Result<()> {
    let (width, height) = (size.width(), size.height());
    if let DirectionalAngleBranch::Above { .. } = branch {
        for row in 0..height {
            let spec = line(row)?;
            let out = &mut output[row * stride..row * stride + width];
            let active = spec.active.min(width);
            edge.line(filter, spec.first, spec.shift, &mut out[..active])?;
            if active < width {
                out[active..].fill(last);
            }
        }
        return Ok(());
    }
    with_columns(width * height, |columns| {
        for (column, samples) in columns.chunks_exact_mut(height).enumerate().take(width) {
            let spec = line(column)?;
            let active = spec.active.min(height);
            edge.line(filter, spec.first, spec.shift, &mut samples[..active])?;
            if active < height {
                samples[active..].fill(last);
            }
        }
        P::transpose(width, height, columns, output, stride);
        Ok(())
    })
}

#[inline]
fn transpose_tiles<S: Simd>(
    simd: S,
    width: usize,
    height: usize,
    columns: &[u8],
    output: &mut [u8],
    stride: usize,
) {
    let tile_width = if width >= 8 { 8 } else { 4 };
    let tile_rows = if height >= 8 { 8 } else { 4 };
    for c0 in (0..width).step_by(tile_width) {
        for r0 in (0..height).step_by(tile_rows) {
            let column = |c: usize| {
                let at = (c0 + c) * height + r0;
                columns
                    .get(at..at + 16)
                    .map_or(u8x16::splat(simd, 0), |lanes| {
                        u8x16::from_slice(simd, lanes)
                    })
            };
            let pairs = |a: usize| {
                simd.zip_low_u8x16(column(a), column(a + 1))
                    .bitcast::<u16x8<S>>()
            };
            let (t0, t1) = (pairs(0), pairs(2));
            let low: [u8x16<S>; 2] = [
                simd.zip_low_u16x8(t0, t1).bitcast(),
                simd.zip_high_u16x8(t0, t1).bitcast(),
            ];
            let mut store = |row: usize, bytes: &[u8]| {
                let start = (r0 + row) * stride + c0;
                if let Some(out) = output.get_mut(start..start + bytes.len()) {
                    out.copy_from_slice(bytes); // splot-copy-ok: publish one transposed tile row
                }
            };
            if tile_width == 4 {
                for (half, rows) in low.iter().enumerate().take(tile_rows / 4) {
                    let bytes: [u8; 16] = (*rows).into();
                    for (row, samples) in bytes.as_chunks::<4>().0.iter().enumerate() {
                        store(4 * half + row, samples);
                    }
                }
                continue;
            }
            let (t2, t3) = (pairs(4), pairs(6));
            let high: [u8x16<S>; 2] = [
                simd.zip_low_u16x8(t2, t3).bitcast(),
                simd.zip_high_u16x8(t2, t3).bitcast(),
            ];
            for (half, (left, right)) in low.iter().zip(&high).enumerate().take(tile_rows / 4) {
                let (left, right) = (left.bitcast::<u32x4<S>>(), right.bitcast::<u32x4<S>>());
                for (pair, rows) in [
                    simd.zip_low_u32x4(left, right),
                    simd.zip_high_u32x4(left, right),
                ]
                .into_iter()
                .enumerate()
                {
                    let bytes: [u8; 16] = rows.bitcast::<u8x16<S>>().into();
                    store(4 * half + 2 * pair, &bytes[..8]);
                    store(4 * half + 2 * pair + 1, &bytes[8..]);
                }
            }
        }
    }
}

/// The `u16` [`LineSample::transpose`]: each 4- or 8-wide tile of 4 or 8 rows
/// loads its columns as eight `u16x8` (the missing ones zero), transposes them
/// by three rounds of interleaving lane `i` with lane `i + 4`, and stores the
/// tile's rows.
#[inline]
fn transpose_tiles_u16<S: Simd>(
    simd: S,
    width: usize,
    height: usize,
    columns: &[u16],
    output: &mut [u16],
    stride: usize,
) {
    let tile_width = if width >= 8 { 8 } else { 4 };
    let tile_rows = if height >= 8 { 8 } else { 4 };
    for c0 in (0..width).step_by(tile_width) {
        for r0 in (0..height).step_by(tile_rows) {
            let mut rows: [u16x8<S>; 8] = core::array::from_fn(|c| {
                let at = (c0 + c) * height + r0;
                columns
                    .get(at..at + 8)
                    .filter(|_| c < tile_width)
                    .map_or(u16x8::splat(simd, 0), |lanes| {
                        u16x8::from_slice(simd, lanes)
                    })
            });
            for _ in 0..3 {
                let mut next = rows;
                for i in 0..4 {
                    next[2 * i] = simd.zip_low_u16x8(rows[i], rows[i + 4]);
                    next[2 * i + 1] = simd.zip_high_u16x8(rows[i], rows[i + 4]);
                }
                rows = next;
            }
            for (row, values) in rows.iter().enumerate().take(tile_rows) {
                let lanes: [u16; 8] = (*values).into();
                let out = output.get_mut((r0 + row) * stride + c0..);
                if tile_width == 8 {
                    if let Some(out) = out.and_then(<[u16]>::first_chunk_mut::<8>) {
                        *out = lanes;
                    }
                } else if let (Some(out), Some(lanes)) = (
                    out.and_then(<[u16]>::first_chunk_mut::<4>),
                    lanes.first_chunk::<4>(),
                ) {
                    *out = *lanes;
                }
            }
        }
    }
}

/// Samples past a block's columns that [`LineSample::transpose`] may read.
const COLUMN_SLACK: usize = 16;

/// Runs `write` over a zeroed column-major scratch of at least `len` samples
/// plus [`COLUMN_SLACK`], sized to the block so small blocks clear little.
fn with_columns<P: LineSample, R>(len: usize, write: impl FnOnce(&mut [P]) -> R) -> R {
    if len <= 256 {
        write(&mut [P::default(); 256 + COLUMN_SLACK])
    } else if len <= 1024 {
        write(&mut [P::default(); 1024 + COLUMN_SLACK])
    } else {
        write(&mut [P::default(); MAX_SIDE * MAX_SIDE + COLUMN_SLACK])
    }
}

/// The window start and shift of a zone-2 line from its § 7.13.2.8 index,
/// with `front` the padded position of the line's first tap less its base.
fn middle_line(idx: i32, front: i32) -> Result<(usize, u16)> {
    let first =
        usize::try_from((idx >> 6) + front).map_err(|_| ReconError::ArithmeticOverflow {
            context: "middle directional angle logical edge coverage",
        })?;
    Ok((first, directional_shift(idx)))
}

/// Writes a zone-2 prediction: each row's above run, then each column's left
/// runs into a transposed scratch that the rows read back. `left_front` and
/// `above_front` map a tap-0 base to its padded window start, and
/// `above_last`/`left_last` bound each run's active samples as in the lane
/// writers (`None` when the run has no clamped tail).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_middle<P: LineSample>(
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    filter: LineFilter,
    mrl_index: usize,
    [left, above]: [&PaddedEdge<P>; 2],
    [left_front, above_front]: [i32; 2],
    [left_last, above_last]: [Option<(usize, P)>; 2],
    output: &mut [P],
    stride: usize,
) -> Result<()> {
    let branch = angle.branch();
    let (width, height) = (size.width(), size.height());
    let run = |edge: &PaddedEdge<P>,
               (first, shift): (usize, u16),
               last: Option<(usize, P)>,
               out: &mut [P]| {
        let active = last.map_or(out.len(), |(end, _)| {
            end.saturating_sub(first).min(out.len())
        });
        edge.line(filter, first, shift, &mut out[..active])?;
        if let Some((_, value)) = last
            && active < out.len()
        {
            out[active..].fill(value);
        }
        Ok::<(), ReconError>(())
    };
    let above_idx = |row: usize| -((row + 1 + mrl_index) as i32) * i32::from(branch.dx);
    let mut left_runs = [0u8; MAX_SIDE];
    for (row, slot) in left_runs.iter_mut().enumerate().take(height) {
        *slot = usize::try_from(-1 - mrl_index as i32 - (above_idx(row) >> 6))
            .unwrap_or(0)
            .min(width) as u8;
    }
    let left_runs = &left_runs[..height];
    let left_columns = usize::from(left_runs.iter().copied().max().unwrap_or(0));
    let dy = i32::from(branch.dy);
    with_columns(left_columns * height, |columns| {
        let mut write = |column: usize, first: usize, rows: usize, left_idx: i32| {
            let line = middle_line(left_idx, left_front)?;
            let start = column * height + first;
            run(left, line, left_last, &mut columns[start..start + rows])
        };
        if left_runs.is_sorted() {
            let mut first = 0;
            for column in 0..left_columns {
                while usize::from(left_runs[first]) <= column {
                    first += 1;
                }
                let left_idx = first as i32 * 64 - (column + 1 + mrl_index) as i32 * dy;
                write(column, first, height - first, left_idx)?;
            }
        } else {
            for_each_middle_left_run(left_runs, dy, mrl_index, write)?;
        }
        let tile_width = if width >= 8 { 8 } else { 4 };
        P::transpose(
            left_columns.next_multiple_of(tile_width).min(width),
            height,
            columns,
            output,
            stride,
        );
        Ok::<(), ReconError>(())
    })?;
    for (row, &left_run) in left_runs.iter().enumerate() {
        let left_run = usize::from(left_run);
        if left_run < width {
            let line = middle_line(above_idx(row) + 64 * left_run as i32, above_front)?;
            let start = row * stride;
            run(
                above,
                line,
                above_last,
                &mut output[start + left_run..start + width],
            )?;
        }
    }
    Ok(())
}
