// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Eight-bit unscaled § 7.13.3.18 interpolation in byte lanes.
//!
//! Every `Subpel_Filters` tap is even, so halved taps fit `i8` and one
//! `pmaddubsw` weighs a pair of source bytes. Exactness of the halving:
//! `Round2(2s, 3) == Round2(s, 2)`, `Round2(2s, 11) == Round2(s, 10)` and
//! `Round2(2s, 7) == Round2(s, 6)`; a zero phase skips its pass, where
//! `Round2(Round2(x, 3) << 7, 11) == Round2(Round2(x, 3), 4)` and
//! `Round2(16x, 11) == Round2(x, 7)` keep the two-pass value. Halved sums of
//! one row lie in `-7140..=23460`, so `i16` lanes are exact, and each byte
//! pair stays below the `pmaddubsw` saturation bound.
//!
//! Eight-bit compound predictors fit `i16` ([`Sink::PrepNarrow`]): with
//! the § 7.13.3.16 `InterRound0 = 3` and compound `InterRound1 = 7` and
//! the § 7.13.3.18 filter rows (taps sum to 128, positive taps at most 184,
//! negative at least -56), the horizontal pass spans `-1785..=5865` and
//! the vertical pass `Round2(-656880, 7)..=Round2(1179120, 7)`, that is
//! `-5132..=9212`; a zero phase stores `sample << 4 <= 4080`. The sum of two
//! predictors, the equal-weight average, stays in `i16` too.
//!
//! A vector covers 16 outputs: one strip row of a block at least 16 wide, two
//! rows of an 8-wide block or four rows of a 4-wide block. On AArch64 the
//! [`neon`] kernels take the same blocks. Other widths, 10-bit sinks and
//! other targets decline to the portable kernels.

use super::block::{self, Block, Sink};
use crate::format::ReconSample;

#[cfg(target_arch = "aarch64")]
pub(super) mod neon;

/// Predicts an eight-bit block in byte lanes and returns `true`, or returns
/// `false` without writing when the block is outside the byte-lane subset.
pub(super) fn predict<T: ReconSample>(
    block: &Block<'_, T>,
    sink: &mut Sink<'_>,
    stage: &mut [i16],
) -> bool {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2()
        && let Some(source) = T::u8_slice(block.source)
        && fits(block)
        && stage.len() >= block::stage_needed(block)
        && match sink {
            Sink::PutWide { max, .. } => *max == u16::from(u8::MAX),
            Sink::AverageWide { .. } => false,
            _ => true,
        }
    {
        avx2::predict(token, &block.with_source(source), sink, stage);
        return true;
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(token) = fearless_simd::Level::baseline().as_neon()
        && let Some(source) = T::u8_slice(block.source)
    {
        return neon::predict(token, &block.with_source(source), sink);
    }
    let _ = (block, sink, stage);
    false
}

/// Stores the `Clip1` single-reference prediction of `block` into `output`
/// rows `stride` apart.
#[allow(clippy::inline_always, reason = "measured put hot path")]
#[inline(always)]
pub(super) fn put(block: &Block<'_, u8>, output: &mut [u8], stride: usize) {
    if block.horizontal.is_none() && block.vertical.is_none() && copy(block, output, stride) {
        return;
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(token) = fearless_simd::Level::baseline().as_neon()
        && neon::put(token, block, output, stride)
    {
        return;
    }
    #[cfg(target_arch = "x86_64")]
    if super::cell::put_x86(block, output, stride) {
        return;
    }
    block::kernels(block, &mut Sink::Put { output, stride }, None);
}

/// [`put`] of two blocks of one window, the U and V planes of a block, each
/// into its `(output, stride)`: a filtered pair in one call of the pair
/// kernel where the target has one.
#[inline]
pub(super) fn put_pair(blocks: [&Block<'_, u8>; 2], outputs: [(&mut [u8], usize); 2]) {
    let [(first, first_stride), (second, second_stride)] = outputs;
    let filtered = blocks[0].horizontal.is_some() || blocks[0].vertical.is_some();
    if filtered
        && put_pair_kernel(
            blocks,
            [(&mut *first, first_stride), (&mut *second, second_stride)],
        )
    {
        return;
    }
    put(blocks[0], first, first_stride);
    put(blocks[1], second, second_stride);
}

/// The NEON pair kernel of [`put_pair`], `false` without writing when it
/// does not cover the pair.
#[cfg(target_arch = "aarch64")]
#[inline]
fn put_pair_kernel(blocks: [&Block<'_, u8>; 2], outputs: [(&mut [u8], usize); 2]) -> bool {
    let [(first, first_stride), (second, second_stride)] = outputs;
    fearless_simd::Level::baseline()
        .as_neon()
        .is_some_and(|token| {
            let first = &mut neon::Put {
                output: first,
                stride: first_stride,
            };
            let second = &mut neon::Put {
                output: second,
                stride: second_stride,
            };
            neon::put_pair(token, blocks, [first, second])
        })
}

/// No pair kernel: [`put_pair`] stores each block through [`put`].
#[cfg(not(target_arch = "aarch64"))]
#[inline]
fn put_pair_kernel(_: [&Block<'_, u8>; 2], _: [(&mut [u8], usize); 2]) -> bool {
    false
}

/// Stores a zero-phase block (no filter pass): its prediction is its
/// samples. `false` without writing when its rows leave the storage of
/// `block` or `output`.
#[inline(never)]
fn copy(block: &Block<'_, u8>, output: &mut [u8], stride: usize) -> bool {
    let (w, h) = (block.w, block.h);
    let fits = |len: usize, stride: usize| {
        (h - 1)
            .checked_mul(stride)
            .and_then(|end| end.checked_add(w))
            .is_some_and(|end| end <= len)
    };
    let Some(source) = block.source.get(block.origin..).filter(|source| {
        stride >= w && fits(source.len(), block.stride) && fits(output.len(), stride)
    }) else {
        return false;
    };
    macro_rules! copy_lines {
        ($n:literal) => {
            for row in 0..h {
                let (from, to) = (row * block.stride, row * stride);
                if let (Some(source), Some(target)) = (
                    source.get(from..).and_then(<[u8]>::first_chunk::<$n>),
                    output.get_mut(to..).and_then(<[u8]>::first_chunk_mut::<$n>),
                ) {
                    *target = *source;
                }
            }
        };
    }
    match w {
        4 => copy_lines!(4),
        8 => copy_lines!(8),
        16 => copy_lines!(16),
        32 => copy_lines!(32),
        64 => copy_lines!(64),
        128 => copy_lines!(128),
        _ => {
            let lines = source
                .chunks(block.stride)
                .zip(output.chunks_mut(stride))
                .take(h);
            for (source, target) in lines {
                if let (Some(source), Some(target)) = (source.get(..w), target.get_mut(..w)) {
                    // splot-copy-ok: a zero-phase prediction is its reference samples
                    target.copy_from_slice(source);
                }
            }
        }
    }
    true
}

/// Predicts both blocks of a compound pair of one shape and stores their
/// COMPOUND_AVERAGE into `output`.
pub(super) fn compound(
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    #[cfg(target_arch = "aarch64")]
    if let Some(token) = fearless_simd::Level::baseline().as_neon()
        && neon::compound(token, blocks, forward, output, stride)
    {
        return;
    }
    #[cfg(target_arch = "x86_64")]
    if super::cell::compound_x86(blocks, forward, output, stride) {
        return;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2()
        && blocks
            .iter()
            .all(|block| fits(block) && block.w * block.h <= 64)
    {
        let mut stage = [0; 24 * 16 + 64];
        avx2::compound(token, blocks, forward, &mut stage, output, stride);
        return;
    }
    let (w, h) = (blocks[0].w, blocks[0].h);
    super::with_compound_pred0_narrow(w * h, |pred0| {
        let sink = &mut Sink::PrepNarrow {
            output: pred0,
            stride: w,
        };
        block::kernels(blocks[0], sink, None);
        let sink = &mut Sink::AverageNarrow {
            pred0,
            forward,
            output,
            stride,
        };
        block::kernels(blocks[1], sink, None);
    });
}

/// Whether the byte-lane kernels cover the shape of `block`.
#[cfg(target_arch = "x86_64")]
fn fits<T>(block: &Block<'_, T>) -> bool {
    u32::try_from(block.source.len()).is_ok()
        && block.w.is_multiple_of(4)
        && block.h.is_multiple_of(16 / width(block.w))
        && !(width(block.w) == 4 && block.horizontal.is_some_and(|taps| taps.count > 2))
}

/// The strip width of a block: 16 columns, else strips of 8 columns in
/// pairs of rows, else strips of 4 in quads of rows.
#[cfg(target_arch = "x86_64")]
const fn width(w: usize) -> usize {
    if w.is_multiple_of(16) {
        16
    } else if w.is_multiple_of(8) {
        8
    } else {
        4
    }
}

#[cfg(target_arch = "x86_64")]
pub(super) mod avx2 {
    use super::super::block::{Block, Sink, Taps};
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, i16x8, i16x16, i32x4, i32x8, u8x16, u8x32};

    /// `pshufb` patterns gathering source byte pairs `(2j + k, 2j + k + 1)`
    /// for output `k` of each lane, or of each 8-byte half for 4-wide rows.
    const fn pair_shuffles(half_rows: bool) -> [[u8; 32]; 4] {
        let mut out = [[0; 32]; 4];
        let mut pair = 0;
        while pair < 4 {
            let mut lane = 0;
            while lane < 32 {
                let byte = lane % 16;
                let (base, output) = if half_rows {
                    ((byte / 8) * 8, (byte % 8) / 2)
                } else {
                    (0, byte / 2)
                };
                out[pair][lane] = (base + 2 * pair + output + byte % 2) as u8;
                lane += 1;
            }
            pair += 1;
        }
        out
    }

    const LANE_SHUFFLES: [[u8; 32]; 4] = pair_shuffles(false);
    const HALF_SHUFFLES: [[u8; 32]; 4] = pair_shuffles(true);

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn chunk<const N: usize, T>(samples: &[T], at: usize) -> Option<&[T; N]> {
        samples.get(at..)?.first_chunk::<N>()
    }

    /// The `N` bytes of `source` from `at`: a `u32` start ends one compare
    /// from the storage end.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    pub(in crate::subpel_mc) fn row_bytes<const N: usize>(
        source: &[u8],
        at: u32,
    ) -> Option<&[u8; N]> {
        let at = at as usize;
        if at + N > source.len() {
            return None;
        }
        chunk(source, at)
    }

    /// An output row start of [`store_rows`].
    pub(in crate::subpel_mc) trait RowStart:
        Copy + core::ops::Add<Output = Self> + core::ops::Mul<Output = Self> + From<u8>
    {
        fn row_mut<const N: usize>(self, output: &mut [u8]) -> Option<&mut [u8; N]>;
    }

    impl RowStart for usize {
        #[allow(clippy::inline_always, reason = "measured subpel hot path")]
        #[inline(always)]
        fn row_mut<const N: usize>(self, output: &mut [u8]) -> Option<&mut [u8; N]> {
            output.get_mut(self..)?.first_chunk_mut::<N>()
        }
    }

    impl RowStart for u32 {
        #[allow(clippy::inline_always, reason = "measured subpel hot path")]
        #[inline(always)]
        fn row_mut<const N: usize>(self, output: &mut [u8]) -> Option<&mut [u8; N]> {
            let at = self as usize;
            if at + N > output.len() {
                return None;
            }
            output.get_mut(at..)?.first_chunk_mut::<N>()
        }
    }

    /// Calls `$f::<W, P>` for the run-time strip width and pair count.
    macro_rules! specialize {
        ($width:expr, $pairs:expr, $f:ident($($arg:expr),*)) => {
            match ($width, $pairs) {
                (4, 1) => $f::<4, 1>($($arg),*),
                (4, 2) => $f::<4, 2>($($arg),*),
                (4, 3) => $f::<4, 3>($($arg),*),
                (4, _) => $f::<4, 4>($($arg),*),
                (8, 1) => $f::<8, 1>($($arg),*),
                (8, 2) => $f::<8, 2>($($arg),*),
                (8, 3) => $f::<8, 3>($($arg),*),
                (8, _) => $f::<8, 4>($($arg),*),
                (_, 1) => $f::<16, 1>($($arg),*),
                (_, 2) => $f::<16, 2>($($arg),*),
                (_, 3) => $f::<16, 3>($($arg),*),
                (_, _) => $f::<16, 4>($($arg),*),
            }
        };
    }

    /// Calls `$f::<W>` for the strip width of a group.
    macro_rules! by_width {
        ($width:expr, $f:ident($($arg:expr),*)) => {
            match $width {
                4 => $f::<4>($($arg),*),
                8 => $f::<8>($($arg),*),
                _ => $f::<16>($($arg),*),
            }
        };
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn predict(
            t: Avx2,
            block: &Block<'_, u8>,
            sink: &mut Sink<'_>,
            stage: &mut [i16],
        ) {
            let w = block.w;
            match sink {
                Sink::Put { output, stride } => put_rows(t, block, stage, output, *stride),
                Sink::PutWide { output, stride, .. } => {
                    let stride = *stride;
                    passes(t, block, true, stage, &mut |width, row, strip, values| {
                        let at = row * stride + strip * width;
                        by_width!(width, store_wide(t, output, at, stride, values));
                    });
                }
                Sink::Prep { output, stride } => {
                    let stride = *stride;
                    passes(t, block, false, stage, &mut |width, row, strip, values| {
                        let at = row * stride + strip * width;
                        by_width!(width, store_prep(t, output, at, stride, values));
                    });
                }
                Sink::PrepNarrow { output, stride } => {
                    let stride = *stride;
                    passes(t, block, false, stage, &mut |width, row, strip, values| {
                        let at = row * stride + strip * width;
                        by_width!(width, store_prep16(t, output, at, stride, values));
                    });
                }
                Sink::AverageNarrow {
                    pred0,
                    forward,
                    output,
                    stride,
                } => {
                    let (pred0, forward, stride) = (&**pred0, *forward, *stride);
                    passes(t, block, false, stage, &mut |width, row, strip, values| {
                        let first =
                            by_width!(width, first_group16(t, pred0, row * w + strip * width, w));
                        let at = row * stride + strip * width;
                        let blended = average(first, values, forward);
                        by_width!(width, store_bytes(t, output, at, stride, blended));
                    });
                }
                Sink::Average {
                    pred0,
                    forward,
                    output,
                    stride,
                } => {
                    let (pred0, forward, stride) = (&**pred0, *forward, *stride);
                    passes(t, block, false, stage, &mut |width, row, strip, values| {
                        let first =
                            by_width!(width, first_group(t, pred0, row * w + strip * width, w));
                        let at = row * stride + strip * width;
                        let blended = average(first, values, forward);
                        by_width!(width, store_bytes(t, output, at, stride, blended));
                    });
                }
                Sink::AverageWide { .. } => {}
            }
        }
    );

    /// The `Clip1` single-reference prediction of `block` into `output`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn put_rows(
        t: Avx2,
        block: &Block<'_, u8>,
        stage: &mut [i16],
        output: &mut [u8],
        stride: usize,
    ) {
        passes(t, block, true, stage, &mut |width, row, strip, values| {
            let at = row * stride + strip * width;
            by_width!(width, store_bytes(t, output, at, stride, values));
        });
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn compound(
            t: Avx2,
            blocks: [&Block<'_, u8>; 2],
            forward: i32,
            stage: &mut [i16],
            output: &mut [u8],
            stride: usize,
        ) {
            let h = blocks[0].h;
            let mut first = [_mm256_setzero_si256(); 4];
            passes(
                t,
                blocks[0],
                false,
                stage,
                &mut |width, row, strip, values| {
                    if let Some(slot) = first.get_mut((strip * h + row) * width / 16) {
                        *slot = values;
                    }
                },
            );
            passes(
                t,
                blocks[1],
                false,
                stage,
                &mut |width, row, strip, values| {
                    let Some(&first) = first.get((strip * h + row) * width / 16) else {
                        return;
                    };
                    let at = row * stride + strip * width;
                    let blended = average(first, values, forward);
                    by_width!(width, store_bytes(t, output, at, stride, blended));
                },
            );
        }
    );

    /// Runs the passes of `block`, handing `out` the 16 final-pass values of
    /// each group as `(W, row, strip, values)`: compound predictors, or with
    /// `put` single-reference predictions before `Clip1`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn passes(
        t: Avx2,
        block: &Block<'_, u8>,
        put: bool,
        stage: &mut [i16],
        out: &mut impl FnMut(usize, usize, usize, __m256i),
    ) {
        let width = super::width(block.w);
        match (block.horizontal, block.vertical) {
            (None, None) => match width {
                4 => copy::<4>(t, block, put, out),
                8 => copy::<8>(t, block, put, out),
                _ => copy::<16>(t, block, put, out),
            },
            (Some(taps), None) => {
                specialize!(width, taps.count, horizontal(t, block, taps, put, out));
            }
            (None, Some(taps)) => {
                specialize!(width, taps.count, vertical(t, block, taps, put, stage, out));
            }
            (Some(h_taps), Some(v_taps)) => {
                let rows = block.h + v_taps.span() - 1;
                for strip in 0..block.w / width {
                    specialize!(
                        width,
                        h_taps.count,
                        stage_rows(t, block, h_taps, rows, strip, stage)
                    );
                    specialize!(
                        width,
                        v_taps.count,
                        filter_stage(t, block, v_taps, put, strip, stage, out)
                    );
                }
            }
        }
    }

    /// The COMPOUND_AVERAGE of two groups of compound predictors, before
    /// `Clip1`.
    #[inline]
    #[target_feature(enable = "avx2")]
    pub(in crate::subpel_mc) fn average(first: __m256i, second: __m256i, forward: i32) -> __m256i {
        if forward == 8 {
            return _mm256_mulhrs_epi16(
                _mm256_add_epi16(first, second),
                _mm256_set1_epi16(1 << 10),
            );
        }
        let weights = _mm256_set1_epi32((forward & 0xffff) | ((16 - forward) << 16));
        let bias = _mm256_set1_epi32(1 << 7);
        let low = _mm256_madd_epi16(_mm256_unpacklo_epi16(first, second), weights);
        let high = _mm256_madd_epi16(_mm256_unpackhi_epi16(first, second), weights);
        _mm256_packs_epi32(
            _mm256_srai_epi32::<8>(_mm256_add_epi32(low, bias)),
            _mm256_srai_epi32::<8>(_mm256_add_epi32(high, bias)),
        )
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn load16(t: Avx2, bytes: &[u8; 16]) -> __m128i {
        u8x16::load_array_ref(t, bytes).into()
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn load8(bytes: [u8; 8]) -> i64 {
        i64::from_le_bytes(bytes)
    }

    /// The tap windows of the 16 outputs of rows starting at window index
    /// `start`, one group: each lane, or each 8-byte half of 4-wide rows,
    /// holds the bytes its outputs read.
    #[inline]
    #[target_feature(enable = "avx2")]
    pub(in crate::subpel_mc) fn tap_bytes<const W: usize>(
        t: Avx2,
        source: &[u8],
        starts: [u32; 4],
    ) -> Option<__m256i> {
        let start = starts[0];
        Some(match W {
            4 => _mm256_set_epi64x(
                load8(*row_bytes(source, starts[3])?),
                load8(*row_bytes(source, starts[2])?),
                load8(*row_bytes(source, starts[1])?),
                load8(*row_bytes(source, starts[0])?),
            ),
            8 => _mm256_set_m128i(
                load16(t, row_bytes(source, starts[1])?),
                load16(t, row_bytes(source, starts[0])?),
            ),
            _ => {
                let bytes = row_bytes::<24>(source, start)?;
                let (low, high) = (bytes.first_chunk::<16>()?, bytes.last_chunk::<16>()?);
                _mm256_set_m128i(load16(t, high), load16(t, low))
            }
        })
    }

    /// The source starts of the group rows at window row `row` of strip
    /// `strip`, rows past `last` repeating it.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn starts<const W: usize>(
        block: &Block<'_, u8>,
        row: usize,
        strip: usize,
        last: usize,
    ) -> [u32; 4] {
        let start = |q: usize| block.start32((row + q).min(last)) + (strip * W) as u32;
        [start(0), start(1), start(2), start(3)]
    }

    pub(in crate::subpel_mc) struct Horizontal {
        shuffles: [__m256i; 4],
        weights: [__m256i; 4],
    }

    impl Horizontal {
        #[inline]
        #[target_feature(enable = "avx2")]
        pub(in crate::subpel_mc) fn new<const W: usize, const P: usize>(
            t: Avx2,
            taps: &Taps,
        ) -> Self {
            let patterns = if W == 4 {
                &HALF_SHUFFLES
            } else {
                &LANE_SHUFFLES
            };
            Self {
                shuffles: core::array::from_fn(|pair| {
                    u8x32::load_array_ref(t, &patterns[pair.min(P.max(1) - 1)]).into()
                }),
                weights: byte_weights::<P>(taps),
            }
        }

        /// The halved-tap sums of the 16 outputs of `bytes`.
        #[inline]
        #[target_feature(enable = "avx2")]
        pub(in crate::subpel_mc) fn sums<const P: usize>(&self, bytes: __m256i) -> __m256i {
            let mut sums = _mm256_maddubs_epi16(
                _mm256_shuffle_epi8(bytes, self.shuffles[0]),
                self.weights[0],
            );
            for pair in 1..P {
                sums = _mm256_add_epi16(
                    sums,
                    _mm256_maddubs_epi16(
                        _mm256_shuffle_epi8(bytes, self.shuffles[pair]),
                        self.weights[pair],
                    ),
                );
            }
            sums
        }
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn byte_weights<const P: usize>(taps: &Taps) -> [__m256i; 4] {
        core::array::from_fn(|pair| _mm256_set1_epi16(taps.bytes[pair.min(P.max(1) - 1)]))
    }

    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn copy<const W: usize>(
        t: Avx2,
        block: &Block<'_, u8>,
        put: bool,
        out: &mut impl FnMut(usize, usize, usize, __m256i),
    ) {
        let source = block.source;
        for strip in 0..block.w / W {
            for row in (0..block.h).step_by(16 / W) {
                let starts = starts::<W>(block, row, strip, usize::MAX);
                let bytes = match W {
                    4 => {
                        let row = |q: usize| {
                            row_bytes::<4>(source, starts[q])
                                .map(|bytes| i32::from_le_bytes(*bytes))
                        };
                        let (Some(first), Some(second), Some(third), Some(fourth)) =
                            (row(0), row(1), row(2), row(3))
                        else {
                            return;
                        };
                        _mm_set_epi32(fourth, third, second, first)
                    }
                    8 => {
                        let (Some(first), Some(second)) =
                            (row_bytes(source, starts[0]), row_bytes(source, starts[1]))
                        else {
                            return;
                        };
                        _mm_set_epi64x(load8(*second), load8(*first))
                    }
                    _ => {
                        let Some(bytes) = row_bytes(source, starts[0]) else {
                            return;
                        };
                        load16(t, bytes)
                    }
                };
                let samples = _mm256_cvtepu8_epi16(bytes);
                let values = if put {
                    samples
                } else {
                    _mm256_slli_epi16::<4>(samples)
                };
                out(W, row, strip, values);
            }
        }
    }

    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn horizontal<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_, u8>,
        taps: &Taps,
        put: bool,
        out: &mut impl FnMut(usize, usize, usize, __m256i),
    ) {
        let filter = Horizontal::new::<W, P>(t, taps);
        let (bias, round) = (_mm256_set1_epi16(34), _mm256_set1_epi16(1 << 13));
        for strip in 0..block.w / W {
            for row in (0..block.h).step_by(16 / W) {
                let starts = starts::<W>(block, row, strip, usize::MAX);
                let Some(bytes) = tap_bytes::<W>(t, block.source, starts) else {
                    return;
                };
                let sums = filter.sums::<P>(bytes);
                let values = if put {
                    _mm256_srai_epi16::<6>(_mm256_add_epi16(sums, bias))
                } else {
                    _mm256_mulhrs_epi16(sums, round)
                };
                out(W, row, strip, values);
            }
        }
    }

    /// The vertical-only pass: byte pairs of consecutive window rows. Rows
    /// of narrow blocks are first packed contiguously into `stage`.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn vertical<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_, u8>,
        taps: &Taps,
        put: bool,
        stage: &mut [i16],
        out: &mut impl FnMut(usize, usize, usize, __m256i),
    ) {
        let weights = byte_weights::<P>(taps);
        let round = _mm256_set1_epi16(if put { 1 << 9 } else { 1 << 13 });
        let source = block.source;
        let rows = block.h + 2 * P - 1;
        let zero = [0i16; 8];
        for strip in 0..block.w / W {
            if W < 16 {
                for (row, slot) in stage.chunks_exact_mut(W / 2).take(rows).enumerate() {
                    let Some(bytes) =
                        row_bytes::<W>(source, block.start32(row) + (strip * W) as u32)
                    else {
                        return;
                    };
                    for (slot, pair) in slot.iter_mut().zip(bytes.chunks_exact(2)) {
                        *slot = i16::from_le_bytes([pair[0], pair[1]]);
                    }
                }
            }
            let line = |row: usize| -> __m256i {
                let lanes = if W < 16 {
                    i16x8::load_array_ref(t, chunk::<8, i16>(stage, row * W / 2).unwrap_or(&zero))
                        .into()
                } else {
                    row_bytes::<16>(source, block.start32(row) + (strip * 16) as u32)
                        .map_or(_mm_setzero_si128(), |bytes| load16(t, bytes))
                };
                _mm256_permute4x64_epi64::<0b01_01_00_00>(_mm256_castsi128_si256(lanes))
            };
            for row in (0..block.h).step_by(16 / W) {
                let mut sums = _mm256_setzero_si256();
                for (pair, &weights) in weights.iter().enumerate().take(P) {
                    let pairs =
                        _mm256_unpacklo_epi8(line(row + 2 * pair), line(row + 2 * pair + 1));
                    sums = _mm256_add_epi16(sums, _mm256_maddubs_epi16(pairs, weights));
                }
                out(W, row, strip, _mm256_mulhrs_epi16(sums, round));
            }
        }
    }

    /// The horizontal pass of strip `strip`: `rows` intermediate rows of `W`
    /// samples, `Round2(sum, InterRound0)`, into `stage`; a ragged last group
    /// repeats the last row.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn stage_rows<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_, u8>,
        taps: &Taps,
        rows: usize,
        strip: usize,
        stage: &mut [i16],
    ) {
        let filter = Horizontal::new::<W, P>(t, taps);
        let round = _mm256_set1_epi16(1 << 13);
        for (group, slot) in stage
            .chunks_exact_mut(16)
            .take(rows.div_ceil(16 / W))
            .enumerate()
        {
            let starts = starts::<W>(block, group * (16 / W), strip, rows - 1);
            let Some(bytes) = tap_bytes::<W>(t, block.source, starts) else {
                return;
            };
            let values = _mm256_mulhrs_epi16(filter.sums::<P>(bytes), round);
            if let Some(slot) = slot.first_chunk_mut::<16>() {
                i16x16::<Avx2>::simd_from(t, values).store_array(slot);
            }
        }
    }

    /// The vertical pass of strip `strip` over the intermediate rows of
    /// [`stage_rows`].
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn filter_stage<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_, u8>,
        taps: &Taps,
        put: bool,
        strip: usize,
        stage: &[i16],
        out: &mut impl FnMut(usize, usize, usize, __m256i),
    ) {
        let weights: [__m256i; 4] =
            core::array::from_fn(|pair| _mm256_set1_epi32(taps.words[pair.min(P.max(1) - 1)]));
        let (bias, shift) = if put { (1 << 9, 10) } else { (1 << 5, 6) };
        let (bias, shift) = (_mm256_set1_epi32(bias), _mm_cvtsi32_si128(shift));
        let span = (2 * P - 1) * W + 16;
        for row in (0..block.h).step_by(16 / W) {
            let Some(window) = stage.get(row * W..row * W + span) else {
                return;
            };
            let line = |row: usize| -> __m256i {
                let lanes = window
                    .get(row * W..row * W + 16)
                    .and_then(<[i16]>::first_chunk::<16>);
                lanes.map_or(_mm256_setzero_si256(), |lanes| {
                    i16x16::load_array_ref(t, lanes).into()
                })
            };
            let (mut low, mut high) = (bias, bias);
            for (pair, &weights) in weights.iter().enumerate().take(P) {
                let (a, b) = (line(2 * pair), line(2 * pair + 1));
                low =
                    _mm256_add_epi32(low, _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weights));
                high = _mm256_add_epi32(
                    high,
                    _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weights),
                );
            }
            let values =
                _mm256_packs_epi32(_mm256_sra_epi32(low, shift), _mm256_sra_epi32(high, shift));
            out(W, row, strip, values);
        }
    }

    /// `Clip1` of the 16 values of a group into `u16` storage from `at`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn store_wide<const W: usize>(
        t: Avx2,
        output: &mut [u16],
        at: usize,
        stride: usize,
        values: __m256i,
    ) {
        let values = _mm256_min_epi16(
            _mm256_max_epi16(values, _mm256_setzero_si256()),
            _mm256_set1_epi16(255),
        );
        let lanes = |half: __m128i| -> [u16; 8] {
            let mut lanes = [0; 8];
            fearless_simd::u16x8::<Avx2>::simd_from(t, half).store_array(&mut lanes);
            lanes
        };
        let (low, high) = (
            lanes(_mm256_castsi256_si128(values)),
            lanes(_mm256_extracti128_si256::<1>(values)),
        );
        let mut put = |at: usize, lanes: &[u16]| {
            if let Some(slot) = output.get_mut(at..at + lanes.len()) {
                for (slot, &lane) in slot.iter_mut().zip(lanes) {
                    *slot = lane;
                }
            }
        };
        match W {
            4 => {
                put(at, &low[..4]);
                put(at + stride, &low[4..]);
                put(at + 2 * stride, &high[..4]);
                put(at + 3 * stride, &high[4..]);
            }
            8 => {
                put(at, &low);
                put(at + stride, &high);
            }
            _ => {
                put(at, &low);
                put(at + 8, &high);
            }
        }
    }

    /// The 16 compound predictors of a group as `i32` into `output` from
    /// `at`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn store_prep<const W: usize>(
        t: Avx2,
        output: &mut [i32],
        at: usize,
        stride: usize,
        values: __m256i,
    ) {
        let low = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(values));
        let high = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(values));
        let mut put8 = |at: usize, lanes: __m256i| {
            if let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<8>) {
                i32x8::<Avx2>::simd_from(t, lanes).store_array(slot);
            }
        };
        match W {
            4 => {
                let mut put4 = |at: usize, lanes: __m128i| {
                    if let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>)
                    {
                        i32x4::<Avx2>::simd_from(t, lanes).store_array(slot);
                    }
                };
                put4(at, _mm256_castsi256_si128(low));
                put4(at + stride, _mm256_extracti128_si256::<1>(low));
                put4(at + 2 * stride, _mm256_castsi256_si128(high));
                put4(at + 3 * stride, _mm256_extracti128_si256::<1>(high));
            }
            8 => {
                put8(at, low);
                put8(at + stride, high);
            }
            _ => {
                put8(at, low);
                put8(at + 8, high);
            }
        }
    }

    /// [`store_prep`] of `i16` predictors.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn store_prep16<const W: usize>(
        t: Avx2,
        output: &mut [i16],
        at: usize,
        stride: usize,
        values: __m256i,
    ) {
        if W == 16 {
            if let Some(slot) = output
                .get_mut(at..)
                .and_then(<[i16]>::first_chunk_mut::<16>)
            {
                i16x16::<Avx2>::simd_from(t, values).store_array(slot);
            }
            return;
        }
        let lanes = i16x16::<Avx2>::simd_from(t, values).to_array();
        for (row, lanes) in lanes.as_chunks::<W>().0.iter().enumerate() {
            if let Some(slot) = output
                .get_mut(at + row * stride..)
                .and_then(<[i16]>::first_chunk_mut::<W>)
            {
                *slot = *lanes;
            }
        }
    }

    /// [`first_group`] of `i16` predictors.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn first_group16<const W: usize>(t: Avx2, pred0: &[i16], at: usize, w: usize) -> __m256i {
        let mut lanes = [0i16; 16];
        for (row, slot) in lanes.as_chunks_mut::<W>().0.iter_mut().enumerate() {
            let from = if W == 16 { at } else { at + row * w };
            if let Some(first) = pred0.get(from..).and_then(<[i16]>::first_chunk::<W>) {
                *slot = *first;
            }
        }
        i16x16::load_array(t, lanes).into()
    }

    /// The first predictors of the group at `at` of the contiguous `w`-wide
    /// `pred0`, in the layout of a group's values.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn first_group<const W: usize>(t: Avx2, pred0: &[i32], at: usize, w: usize) -> __m256i {
        let first = if W == 4 {
            let load4 = |at: usize| -> __m128i {
                chunk::<4, i32>(pred0, at).map_or(_mm_setzero_si128(), |lanes| {
                    i32x4::load_array_ref(t, lanes).into()
                })
            };
            _mm256_packs_epi32(
                _mm256_set_m128i(load4(at + w), load4(at)),
                _mm256_set_m128i(load4(at + 3 * w), load4(at + 2 * w)),
            )
        } else {
            let load8 = |at: usize| -> __m256i {
                chunk::<8, i32>(pred0, at).map_or(_mm256_setzero_si256(), |lanes| {
                    i32x8::load_array_ref(t, lanes).into()
                })
            };
            let next = if W == 8 { at + w } else { at + 8 };
            _mm256_packs_epi32(load8(at), load8(next))
        };
        _mm256_permute4x64_epi64::<0b11_01_10_00>(first)
    }

    /// `Clip1` of the 16 values of a group into `output` from `at`.
    #[inline]
    #[target_feature(enable = "avx2")]
    pub(in crate::subpel_mc) fn store_bytes<const W: usize>(
        t: Avx2,
        output: &mut [u8],
        at: usize,
        stride: usize,
        values: __m256i,
    ) {
        store_rows::<W, usize>(t, output, at, stride, values);
    }

    /// [`store_bytes`] from rows `at + r * stride`.
    #[inline]
    #[target_feature(enable = "avx2")]
    pub(in crate::subpel_mc) fn store_rows<const W: usize, S: RowStart>(
        t: Avx2,
        output: &mut [u8],
        at: S,
        stride: S,
        values: __m256i,
    ) {
        let row = |row: u8| at + S::from(row) * stride;
        let packed = _mm256_packus_epi16(values, values);
        let lanes = _mm256_castsi256_si128(_mm256_permute4x64_epi64::<0b1000>(packed));
        match W {
            4 => {
                let words = [
                    _mm_cvtsi128_si32(lanes),
                    _mm_extract_epi32::<1>(lanes),
                    _mm_extract_epi32::<2>(lanes),
                    _mm_extract_epi32::<3>(lanes),
                ];
                for (index, word) in (0..).zip(words) {
                    if let Some(slot) = row(index).row_mut::<4>(output) {
                        *slot = word.to_le_bytes();
                    }
                }
            }
            8 => {
                let words = [_mm_cvtsi128_si64(lanes), _mm_extract_epi64::<1>(lanes)];
                for (index, word) in (0..).zip(words) {
                    if let Some(slot) = row(index).row_mut::<8>(output) {
                        *slot = word.to_le_bytes();
                    }
                }
            }
            _ => {
                if let Some(slot) = at.row_mut::<16>(output) {
                    u8x16::<Avx2>::simd_from(t, lanes).store_array(slot);
                }
            }
        }
    }
}
