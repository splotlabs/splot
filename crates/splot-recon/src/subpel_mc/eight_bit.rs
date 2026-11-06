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
//! A vector covers 16 outputs: one strip row of a block at least 16 wide, two
//! rows of an 8-wide block or four rows of a 4-wide block. Blocks whose
//! filter windows need § 7.13.3.18 clipping, other widths, and targets
//! without AVX2 decline to the general core.

use super::{
    NUM_FILTER_TYPES, NUM_PHASES, NUM_TAPS, ReferencePlaneView, SCALE_SUBPEL_BITS, SUBPEL_FILTERS,
    SUBPEL_MASK, SubpelPredictParams, even_tap_span,
};

/// Where the final pass of a block lands.
pub(super) enum Sink<'a> {
    /// `Clip1` of a single-reference prediction.
    Put { output: &'a mut [u8], stride: usize },
    /// [`Self::Put`] into `u16` storage.
    PutWide {
        output: &'a mut [u16],
        stride: usize,
    },
    /// The `InterRound1 == 7` compound predictor.
    Prep {
        output: &'a mut [i32],
        stride: usize,
    },
    /// The § 7.13.3.15 COMPOUND_AVERAGE of the contiguous `w * h` first
    /// predictor `pred0` and this block's compound predictor.
    Average {
        pred0: &'a [i32],
        forward: i32,
        output: &'a mut [u8],
        stride: usize,
    },
}

/// One pass's halved active taps from tap `start`: `count` pairs, as byte
/// pairs for `pmaddubsw` and as `i16` pairs for `pmaddwd`.
#[derive(Clone, Copy)]
struct Taps {
    start: usize,
    count: usize,
    bytes: [i16; NUM_TAPS / 2],
    words: [i32; NUM_TAPS / 2],
}

/// The [`Taps`] of every nonzero `Subpel_Filters` phase.
const TAPS: [[Option<Taps>; NUM_PHASES]; NUM_FILTER_TYPES] = {
    let mut table = [[None; NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 1;
        while phase < NUM_PHASES {
            let (start, taps) = even_tap_span(filter, phase);
            let row = &SUBPEL_FILTERS[filter][phase];
            let (mut bytes, mut words) = ([0; NUM_TAPS / 2], [0; NUM_TAPS / 2]);
            let mut pair = 0;
            while pair < taps / 2 {
                let (a, b) = (row[start + 2 * pair] / 2, row[start + 2 * pair + 1] / 2);
                bytes[pair] = i16::from_le_bytes([a as u8, b as u8]);
                words[pair] = (a & 0xffff) | (b << 16);
                pair += 1;
            }
            table[filter][phase] = Some(Taps {
                start,
                count: taps / 2,
                bytes,
                words,
            });
            phase += 1;
        }
        filter += 1;
    }
    table
};

impl Taps {
    const fn span(&self) -> usize {
        2 * self.count
    }
}

/// Rows and columns of one 16-output vector.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Rows4,
    Rows2,
    Strip,
}

impl Shape {
    const fn width(self) -> usize {
        match self {
            Self::Rows4 => 4,
            Self::Rows2 => 8,
            Self::Strip => 16,
        }
    }

    const fn rows(self) -> usize {
        16 / self.width()
    }
}

/// An unclipped block: row `r`, column `c` of its read window is
/// `source[origin + r * stride + c]`.
struct Block<'a> {
    source: &'a [u8],
    stride: usize,
    origin: usize,
    w: usize,
    h: usize,
    shape: Shape,
    horizontal: Option<Taps>,
    vertical: Option<Taps>,
}

/// Predicts the block into `sink` and returns `true`, or returns `false`
/// without writing when the block is outside the byte-lane subset.
pub(super) fn predict(
    reference: &ReferencePlaneView<'_, u8>,
    params: &SubpelPredictParams,
    sink: Sink<'_>,
    scratch: Option<&mut [i16]>,
) -> bool {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2()
        && let Some(block) = Block::new(reference, params)
        && sink_fits(&sink, params)
    {
        let (width, group) = (block.shape.width(), block.shape.rows());
        let len = match (block.horizontal, block.vertical) {
            (_, None) => 0,
            (Some(_), Some(taps)) => (block.h + taps.span() - 1).next_multiple_of(group) * width,
            (None, Some(_)) if width == 16 => 0,
            (None, Some(taps)) => (block.h + taps.span() - 1) * width / 2 + 8,
        };
        let mut sink = sink;
        match scratch.and_then(|scratch| scratch.get_mut(..len)) {
            Some(stage) => avx2::predict(token, &block, &mut sink, stage),
            None if len == 0 => avx2::predict(token, &block, &mut sink, &mut []),
            None => super::with_subpel_intermediate(len, |stage| {
                avx2::predict(token, &block, &mut sink, stage);
            }),
        }
        return true;
    }
    let _ = (reference, params, sink, scratch);
    false
}

fn sink_fits(sink: &Sink<'_>, params: &SubpelPredictParams) -> bool {
    let end = |stride: usize| (params.h - 1) * stride + params.w;
    match sink {
        Sink::Put { output, stride } => output.len() >= end(*stride),
        Sink::PutWide { output, stride } => output.len() >= end(*stride),
        Sink::Prep { output, stride } => output.len() >= end(*stride),
        Sink::Average {
            pred0,
            output,
            stride,
            ..
        } => output.len() >= end(*stride) && pred0.len() >= params.w * params.h,
    }
}

impl<'a> Block<'a> {
    fn new(reference: &ReferencePlaneView<'a, u8>, params: &SubpelPredictParams) -> Option<Self> {
        if params.step_x != 1 << SCALE_SUBPEL_BITS || params.step_y != 1 << SCALE_SUBPEL_BITS {
            return None;
        }
        let (w, h) = (params.w, params.h);
        let shape = match w {
            4 => Shape::Rows4,
            8 => Shape::Rows2,
            w if w % 16 == 0 => Shape::Strip,
            _ => return None,
        };
        if h == 0 || !h.is_multiple_of(shape.rows()) || h > super::MAX_BLOCK_DIM {
            return None;
        }
        let taps = |size: usize, start: i32| {
            TAPS[params.interp.pass_index(size as u32) as usize]
                [((start >> 6) & SUBPEL_MASK) as usize]
        };
        let horizontal = taps(w, params.start_x);
        let vertical = taps(h, params.start_y);
        if shape == Shape::Rows4 && horizontal.is_some_and(|taps| taps.count > 2) {
            return None;
        }
        let window = |start: i32, size: usize, taps: Option<Taps>| {
            let base = start >> SCALE_SUBPEL_BITS;
            match taps {
                Some(taps) => (base - 3 + taps.start as i32, size + taps.span() - 1),
                None => (base, size),
            }
        };
        let (column, columns) = window(params.start_x, w, horizontal);
        let (row, rows) = window(params.start_y, h, vertical);
        let last = |first: i32, count: usize| first + count as i32 - 1;
        if column < params.first_x.max(0)
            || row < params.first_y.max(0)
            || last(column, columns) > params.last_x.min(reference.width as i32 - 1)
            || last(row, rows) > params.last_y.min(reference.readable_rows as i32 - 1)
        {
            return None;
        }
        let reach = if horizontal.is_some() {
            (w + 8).max(16)
        } else {
            w
        };
        let origin = row as usize * reference.stride + column as usize;
        let end = origin + (rows - 1) * reference.stride + reach;
        (end <= reference.samples.len()).then_some(Self {
            source: reference.samples,
            stride: reference.stride,
            origin,
            w,
            h,
            shape,
            horizontal,
            vertical,
        })
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::{Block, Sink, Taps};
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

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn predict(t: Avx2, block: &Block<'_>, sink: &mut Sink<'_>, stage: &mut [i16]) {
            let width = block.shape.width();
            match (block.horizontal, block.vertical) {
                (None, None) => match width {
                    4 => copy::<4>(t, block, sink),
                    8 => copy::<8>(t, block, sink),
                    _ => copy::<16>(t, block, sink),
                },
                (Some(taps), None) => {
                    specialize!(width, taps.count, horizontal(t, block, &taps, sink));
                }
                (None, Some(taps)) => {
                    specialize!(width, taps.count, vertical(t, block, &taps, stage, sink));
                }
                (Some(h_taps), Some(v_taps)) => {
                    let rows = block.h + v_taps.span() - 1;
                    for strip in 0..block.w / width {
                        specialize!(
                            width,
                            h_taps.count,
                            stage_rows(t, block, &h_taps, rows, strip, stage)
                        );
                        specialize!(
                            width,
                            v_taps.count,
                            filter_stage(t, block, &v_taps, strip, stage, sink)
                        );
                    }
                }
            }
        }
    );

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
    fn tap_bytes<const W: usize>(
        t: Avx2,
        source: &[u8],
        stride: usize,
        start: usize,
        last: usize,
    ) -> Option<__m256i> {
        let row = |q: usize| (start + q * stride).min(last);
        Some(match W {
            4 => _mm256_set_epi64x(
                load8(*chunk(source, row(3))?),
                load8(*chunk(source, row(2))?),
                load8(*chunk(source, row(1))?),
                load8(*chunk(source, row(0))?),
            ),
            8 => _mm256_set_m128i(
                load16(t, chunk(source, row(1))?),
                load16(t, chunk(source, row(0))?),
            ),
            _ => {
                let bytes = chunk::<24, u8>(source, start)?;
                let (low, high) = (bytes.first_chunk::<16>()?, bytes.last_chunk::<16>()?);
                _mm256_set_m128i(load16(t, high), load16(t, low))
            }
        })
    }

    struct Horizontal {
        shuffles: [__m256i; 4],
        weights: [__m256i; 4],
    }

    impl Horizontal {
        #[inline]
        #[target_feature(enable = "avx2")]
        fn new<const W: usize, const P: usize>(t: Avx2, taps: &Taps) -> Self {
            let patterns = if W == 4 {
                &HALF_SHUFFLES
            } else {
                &LANE_SHUFFLES
            };
            Self {
                shuffles: core::array::from_fn(|pair| {
                    u8x32::load_array_ref(t, &patterns[pair.min(P - 1)]).into()
                }),
                weights: byte_weights::<P>(taps),
            }
        }

        /// The halved-tap sums of the 16 outputs of `bytes`.
        #[inline]
        #[target_feature(enable = "avx2")]
        fn sums<const P: usize>(&self, bytes: __m256i) -> __m256i {
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
        core::array::from_fn(|pair| _mm256_set1_epi16(taps.bytes[pair.min(P - 1)]))
    }

    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn copy<const W: usize>(t: Avx2, block: &Block<'_>, sink: &mut Sink<'_>) {
        let put = matches!(sink, Sink::Put { .. } | Sink::PutWide { .. });
        let (source, stride) = (block.source, block.stride);
        for strip in 0..block.w / W {
            let mut start = block.origin + strip * W;
            for row in (0..block.h).step_by(16 / W) {
                let bytes = match W {
                    4 => {
                        let row = |q: usize| {
                            chunk::<4, u8>(source, start + q * stride)
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
                            (chunk(source, start), chunk(source, start + stride))
                        else {
                            return;
                        };
                        _mm_set_epi64x(load8(*second), load8(*first))
                    }
                    _ => {
                        let Some(bytes) = chunk(source, start) else {
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
                emit::<W>(t, sink, block.w, row, strip, values);
                start += (16 / W) * stride;
            }
        }
    }

    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn horizontal<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_>,
        taps: &Taps,
        sink: &mut Sink<'_>,
    ) {
        let filter = Horizontal::new::<W, P>(t, taps);
        let put = matches!(sink, Sink::Put { .. } | Sink::PutWide { .. });
        let (bias, round) = (_mm256_set1_epi16(34), _mm256_set1_epi16(1 << 13));
        let stride = block.stride;
        for strip in 0..block.w / W {
            let mut start = block.origin + strip * W;
            for row in (0..block.h).step_by(16 / W) {
                let Some(bytes) = tap_bytes::<W>(t, block.source, stride, start, usize::MAX) else {
                    return;
                };
                let sums = filter.sums::<P>(bytes);
                let values = if put {
                    _mm256_srai_epi16::<6>(_mm256_add_epi16(sums, bias))
                } else {
                    _mm256_mulhrs_epi16(sums, round)
                };
                emit::<W>(t, sink, block.w, row, strip, values);
                start += (16 / W) * stride;
            }
        }
    }

    /// The vertical-only pass: byte pairs of consecutive window rows. Rows
    /// of narrow blocks are first packed contiguously into `stage`.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn vertical<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_>,
        taps: &Taps,
        stage: &mut [i16],
        sink: &mut Sink<'_>,
    ) {
        let weights = byte_weights::<P>(taps);
        let round = _mm256_set1_epi16(if matches!(sink, Sink::Put { .. } | Sink::PutWide { .. }) {
            1 << 9
        } else {
            1 << 13
        });
        let (source, stride) = (block.source, block.stride);
        let rows = block.h + 2 * P - 1;
        if W < 16 {
            for (row, slot) in stage.chunks_exact_mut(W / 2).take(rows).enumerate() {
                let Some(bytes) = source
                    .get(block.origin + row * stride..)
                    .and_then(|row| row.get(..W))
                else {
                    return;
                };
                for (slot, pair) in slot.iter_mut().zip(bytes.chunks_exact(2)) {
                    *slot = i16::from_le_bytes([pair[0], pair[1]]);
                }
            }
        }
        let zero = [0i16; 8];
        for strip in 0..block.w / W {
            let line = |row: usize| -> __m256i {
                let lanes = if W < 16 {
                    i16x8::load_array_ref(t, chunk::<8, i16>(stage, row * W / 2).unwrap_or(&zero))
                        .into()
                } else {
                    chunk(source, block.origin + row * stride + strip * 16)
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
                emit::<W>(
                    t,
                    sink,
                    block.w,
                    row,
                    strip,
                    _mm256_mulhrs_epi16(sums, round),
                );
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
        block: &Block<'_>,
        taps: &Taps,
        rows: usize,
        strip: usize,
        stage: &mut [i16],
    ) {
        let filter = Horizontal::new::<W, P>(t, taps);
        let round = _mm256_set1_epi16(1 << 13);
        let stride = block.stride;
        let first = block.origin + strip * W;
        let last = first + (rows - 1) * stride;
        let mut start = first;
        for slot in stage.chunks_exact_mut(16).take(rows.div_ceil(16 / W)) {
            let Some(bytes) = tap_bytes::<W>(t, block.source, stride, start, last) else {
                return;
            };
            let values = _mm256_mulhrs_epi16(filter.sums::<P>(bytes), round);
            if let Some(slot) = slot.first_chunk_mut::<16>() {
                i16x16::<Avx2>::simd_from(t, values).store_array(slot);
            }
            start += (16 / W) * stride;
        }
    }

    /// The vertical pass of strip `strip` over the intermediate rows of
    /// [`stage_rows`].
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn filter_stage<const W: usize, const P: usize>(
        t: Avx2,
        block: &Block<'_>,
        taps: &Taps,
        strip: usize,
        stage: &[i16],
        sink: &mut Sink<'_>,
    ) {
        let weights: [__m256i; 4] =
            core::array::from_fn(|pair| _mm256_set1_epi32(taps.words[pair.min(P - 1)]));
        let (bias, shift) = if matches!(sink, Sink::Put { .. } | Sink::PutWide { .. }) {
            (1 << 9, 10)
        } else {
            (1 << 5, 6)
        };
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
            emit::<W>(t, sink, block.w, row, strip, values);
        }
    }

    /// Stores the 16 final-pass values of the group at `row`, `strip`: one
    /// strip row of `W == 16`, else `16 / W` whole rows.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn emit<const W: usize>(
        t: Avx2,
        sink: &mut Sink<'_>,
        w: usize,
        row: usize,
        strip: usize,
        values: __m256i,
    ) {
        let column = strip * W;
        match sink {
            Sink::Put { output, stride } => {
                store_bytes::<W>(t, output, row * *stride + column, *stride, values);
            }
            Sink::PutWide { output, stride } => {
                let stride = *stride;
                let at = row * stride + column;
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
            Sink::Prep { output, stride } => {
                let stride = *stride;
                let at = row * stride + column;
                let low = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(values));
                let high = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(values));
                let mut put8 = |at: usize, lanes: __m256i| {
                    if let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<8>)
                    {
                        i32x8::<Avx2>::simd_from(t, lanes).store_array(slot);
                    }
                };
                match W {
                    4 => {
                        let mut put4 = |at: usize, lanes: __m128i| {
                            if let Some(slot) =
                                output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>)
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
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let at = row * w + column;
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
                let first = _mm256_permute4x64_epi64::<0b11_01_10_00>(first);
                let blended = if *forward == 8 {
                    _mm256_mulhrs_epi16(_mm256_add_epi16(first, values), _mm256_set1_epi16(1 << 10))
                } else {
                    let weights = _mm256_set1_epi32((*forward & 0xffff) | ((16 - *forward) << 16));
                    let bias = _mm256_set1_epi32(1 << 7);
                    let low = _mm256_madd_epi16(_mm256_unpacklo_epi16(first, values), weights);
                    let high = _mm256_madd_epi16(_mm256_unpackhi_epi16(first, values), weights);
                    _mm256_packs_epi32(
                        _mm256_srai_epi32::<8>(_mm256_add_epi32(low, bias)),
                        _mm256_srai_epi32::<8>(_mm256_add_epi32(high, bias)),
                    )
                };
                store_bytes::<W>(t, output, row * *stride + column, *stride, blended);
            }
        }
    }

    /// `Clip1` of the 16 values of a group into `output` from `at`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn store_bytes<const W: usize>(
        t: Avx2,
        output: &mut [u8],
        at: usize,
        stride: usize,
        values: __m256i,
    ) {
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
                for (row, word) in words.into_iter().enumerate() {
                    if let Some(slot) = output
                        .get_mut(at + row * stride..)
                        .and_then(<[u8]>::first_chunk_mut::<4>)
                    {
                        *slot = word.to_le_bytes();
                    }
                }
            }
            8 => {
                let words = [_mm_cvtsi128_si64(lanes), _mm_extract_epi64::<1>(lanes)];
                for (row, word) in words.into_iter().enumerate() {
                    if let Some(slot) = output
                        .get_mut(at + row * stride..)
                        .and_then(<[u8]>::first_chunk_mut::<8>)
                    {
                        *slot = word.to_le_bytes();
                    }
                }
            }
            _ => {
                if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<16>) {
                    u8x16::<Avx2>::simd_from(t, lanes).store_array(slot);
                }
            }
        }
    }
}
