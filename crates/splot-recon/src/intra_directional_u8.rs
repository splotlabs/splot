// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! 8-bit § 7.13.2.8 directional prediction over `u8` edges.
//!
//! Every directional zone is a set of lines whose `shift` is fixed and whose
//! `base` steps by one sample: zone-1 rows, zone-3 columns, and the zone-2 above
//! row runs and left column runs. A line is one 4-tap filter over a contiguous
//! edge window. The IDIF taps are `Dr_Interp_Filter[shift]`, and the bilinear
//! `Round2(a * (32 - shift) + b * shift, 5)` is the same filter with taps `[0,
//! 4 * (32 - shift), 4 * shift, 0]` rounded by 7 bits, so both take one `u8`
//! kernel. Column lines are filtered into a transposed scratch first.
//!
//! Feature tracking: `RECON-INTRA-DIRECTIONAL-ANGLE`.

use fearless_simd::{Simd, SimdBase, u8x16, u16x16};
#[cfg(target_arch = "x86_64")]
use fearless_simd::{SimdFrom, u8x32};

use super::{
    DR_INTERP_FILTER, DirectionalAngleBranch, IntraMiddleDirectionalAngle, directional_shift,
    for_each_middle_left_run, logical_idif_edge_offset_mrl,
};
use crate::{IntraRectBlockSize, ReconError, Result};

/// Bytes a padded edge holds: the longest edge, one leading sample, a
/// 64-sample line from its last sample and the kernel over-read.
const PADDED_LEN: usize = 256;
/// Widest block side.
const MAX_SIDE: usize = 64;
/// Bytes past a line's samples its kernels read: three taps and the rest of a
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

/// An edge copied after `front` zero samples into a buffer every line kernel
/// may over-read.
pub(super) struct PaddedEdge {
    samples: [u8; PADDED_LEN],
}

impl PaddedEdge {
    pub(super) fn new(edge: &[u8], front: usize) -> Option<Self> {
        let mut samples = [0u8; PADDED_LEN];
        samples
            .get_mut(front..front + edge.len())
            .filter(|_| front + edge.len() + OVER_READ <= PADDED_LEN)?
            .copy_from_slice(edge); // splot-copy-ok: pad the directional edge for the line kernels
        Some(Self { samples })
    }

    /// Filters `out.len()` samples from the window at `first`: `out[j] =
    /// Clip1(Round2(Σ taps[t] * edge[first + j + t], 7))`.
    #[inline]
    pub(super) fn line(
        &self,
        filter: LineFilter,
        first: usize,
        shift: u16,
        out: &mut [u8],
    ) -> Result<()> {
        if out.is_empty() {
            return Ok(());
        }
        let window = self
            .samples
            .get(first..first + out.len() + OVER_READ)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "8-bit directional line window",
            })?;
        let Some(taps) = filter.taps(shift) else {
            out.copy_from_slice(&window[1..=out.len()]); // splot-copy-ok: unfiltered directional line
            return Ok(());
        };
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
            return Ok(());
        }
        baseline_simd!(simd => line_lanes(simd, window, taps, out));
        Ok(())
    }
}

/// Bias that lifts every 8-bit 4-tap sum above zero in [`line_lanes`]: the
/// negative taps total at least `-24`, so `-24 * 255 + 48 * 128 > 0`, and the
/// positive at most `152`, so `152 * 255 + 48 * 128 + 64` fits `u16`.
const LINE_BIAS: u16 = 48;

/// The portable line kernel in wrapping `u16` lanes: the biased sum is exact,
/// so `(sum >> 7) - 48` is the signed `Round2`, clipped by the saturating
/// subtract and the minimum.
#[inline]
fn line_lanes<S: Simd>(simd: S, window: &[u8], taps: [i8; 4], out: &mut [u8]) {
    let taps = taps.map(|tap| i16::from(tap) as u16);
    let bias = u16x16::splat(simd, LINE_BIAS);
    for at in (0..out.len()).step_by(16) {
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
        let bytes: [u8; 16] = simd.relaxed_narrow_u16x8(low, high).into();
        let count = (out.len() - at).min(16);
        out[at..at + count].copy_from_slice(&bytes[..count]); // splot-copy-ok: publish one line chunk
    }
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
        for at in (0..out.len()).step_by(32) {
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
            let packed = u8x32::simd_from(
                token,
                _mm256_packus_epi16(
                    _mm256_mulhrs_epi16(low, round),
                    _mm256_mulhrs_epi16(high, round),
                ),
            );
            let count = out.len() - at;
            if count >= 32 {
                packed.store_slice(&mut out[at..at + 32]);
            } else {
                let bytes: [u8; 32] = packed.into();
                out[at..].copy_from_slice(&bytes[..count]); // splot-copy-ok: publish one line tail
            }
        }
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
        if let Some(out) = out.first_chunk_mut::<16>() {
            packed.store_slice(out);
        } else {
            let bytes: [u8; 16] = packed.into();
            out.copy_from_slice(&bytes[..out.len()]); // splot-copy-ok: publish one short line
        }
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

/// Writes a zone-1 (`rows`) or zone-3 prediction whose `line(i)` gives row or
/// column `i`, filling each line past its active samples with `last`.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_one_sided(
    size: IntraRectBlockSize,
    branch: DirectionalAngleBranch,
    filter: LineFilter,
    edge: &PaddedEdge,
    last: u8,
    mut line: impl FnMut(usize) -> Result<Line>,
    output: &mut [u8],
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
        for row in 0..height {
            let out = &mut output[row * stride..row * stride + width];
            for (slot, samples) in out.iter_mut().zip(columns.chunks_exact(height)) {
                *slot = samples[row];
            }
        }
        Ok(())
    })
}

/// Runs `write` over a zeroed column-major scratch of at least `len` samples,
/// sized to the block so small blocks clear little.
fn with_columns<R>(len: usize, write: impl FnOnce(&mut [u8]) -> R) -> R {
    if len <= 256 {
        write(&mut [0u8; 256])
    } else if len <= 1024 {
        write(&mut [0u8; 1024])
    } else {
        write(&mut [0u8; MAX_SIDE * MAX_SIDE])
    }
}

/// The window start and shift of a zone-2 line from its § 7.13.2.8 index,
/// with `offset(base)` the padded position of the line's first tap.
fn middle_line(idx: i32, offset: impl Fn(i32) -> Result<usize>) -> Result<(usize, u16)> {
    Ok((offset(idx >> 6)?, directional_shift(idx)))
}

/// Writes a zone-2 prediction: each row's above run, then each column's left
/// runs into a transposed scratch that the rows read back. `above_offset` and
/// `left_offset` map a tap-0 base to its padded window start, and
/// `above_last`/`left_last` bound each run's active samples as in the lane
/// writers (`None` when the run has no clamped tail).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_middle(
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    filter: LineFilter,
    mrl_index: usize,
    [left, above]: [&PaddedEdge; 2],
    [left_offset, above_offset]: [&dyn Fn(i32) -> Result<usize>; 2],
    [left_last, above_last]: [Option<(usize, u8)>; 2],
    output: &mut [u8],
    stride: usize,
) -> Result<()> {
    let branch = angle.branch();
    let (width, height) = (size.width(), size.height());
    let run = |edge: &PaddedEdge,
               (first, shift): (usize, u16),
               last: Option<(usize, u8)>,
               out: &mut [u8]| {
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
    let mut left_runs = [0u8; MAX_SIDE];
    for (row, slot) in left_runs.iter_mut().enumerate().take(height) {
        let above_idx = -((row + 1 + mrl_index) as i32) * i32::from(branch.dx);
        let left_run = usize::try_from(-1 - mrl_index as i32 - (above_idx >> 6))
            .unwrap_or(0)
            .min(width);
        *slot = left_run as u8;
        if left_run < width {
            let line = middle_line(above_idx + 64 * left_run as i32, above_offset)?;
            let start = row * stride;
            run(
                above,
                line,
                above_last,
                &mut output[start + left_run..start + width],
            )?;
        }
    }
    let left_runs = &left_runs[..height];
    let left_columns = usize::from(left_runs.iter().copied().max().unwrap_or(0));
    let dy = i32::from(branch.dy);
    with_columns(left_columns * height, |columns| {
        let mut write = |column: usize, first: usize, rows: usize, left_idx: i32| {
            let line = middle_line(left_idx, left_offset)?;
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
        for (row, &left_run) in left_runs.iter().enumerate().take(height) {
            let out = &mut output[row * stride..row * stride + usize::from(left_run)];
            for (slot, samples) in out.iter_mut().zip(columns.chunks_exact(height)) {
                *slot = samples[row];
            }
        }
        Ok(())
    })
}

/// The padded window start of an IDIF tap-0 base on an edge whose `slice[0]`
/// is logical `-2 - mrlIndex`.
pub(super) fn idif_offset(len: usize, mrl_index: usize) -> impl Fn(i32) -> Result<usize> {
    move |base| logical_idif_edge_offset_mrl(base - 1, len, mrl_index)
}
