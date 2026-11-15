// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AVX2 § 7.17.7 deblocking of 8-bit runs, eight segments (32 lines) per
//! chunk (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-7`).
//!
//! Vector `8 + j` of a chunk holds byte `s[j]` of all 32 lines: horizontal
//! edges load sample rows as they are, vertical edges transpose 8 or 16
//! bytes of each line. The § 7.17.7.2 choice reads only the first and last
//! line of a segment, so it runs on 16-bit lanes of those two lines of all
//! eight segments; the § 7.17.7.1 ramp widens a row only to apply its taps.
//! A chunk takes the cheapest kernel that covers its widest `maxWidthPos`.

use core::arch::x86_64::*;

use fearless_simd::{Avx2, SimdBase, SimdFrom, i16x16, kernel, u8x16, u8x32};

use super::{CLAMP, DeblockEdge, Q_FIRST, RAMP, REACH};

/// Segments, and lines, one chunk filters.
const SEGMENTS: usize = 8;
const LINES: usize = 4 * SEGMENTS;

/// A shuffle that sets word `w` of each 128-bit lane to bytes `low[w]` and
/// `high[w]` of that lane, [`ZERO`] clearing a byte.
const fn words(low: [u8; 8], high: [u8; 8]) -> [u8; 32] {
    let mut bytes = [0; 32];
    let mut i = 0;
    while i < 16 {
        bytes[i] = if i % 2 == 0 { low[i / 2] } else { high[i / 2] };
        bytes[i + 16] = bytes[i];
        i += 1;
    }
    bytes
}

const ZERO: u8 = 0x80;
/// The first lines of a lane's four segments, then their last lines.
const SIDES: [u8; 32] = words([0, 4, 8, 12, 3, 7, 11, 15], [ZERO; 8]);
/// The edge fields over both lines of each segment, from the packed words.
const Q_THR: [u8; 32] = words([0, 4, 8, 12, 0, 4, 8, 12], [1, 5, 9, 13, 1, 5, 9, 13]);
const SIDE: [u8; 32] = words([2, 6, 10, 14, 2, 6, 10, 14], [3, 7, 11, 15, 3, 7, 11, 15]);
const MAX_POS: [u8; 32] = words([0, 4, 8, 12, 0, 4, 8, 12], [ZERO; 8]);
const MAX_NEG: [u8; 32] = words([1, 5, 9, 13, 1, 5, 9, 13], [ZERO; 8]);
const FLAGS: [u8; 32] = words([2, 6, 10, 14, 2, 6, 10, 14], [ZERO; 8]);
/// A segment word over the lines of the low and high halves of a widened
/// row: lines `0..8` and `8..16` of each lane's 16.
const LOW_LINES: [u8; 32] = words([0, 0, 0, 0, 2, 2, 2, 2], [1, 1, 1, 1, 3, 3, 3, 3]);
const HIGH_LINES: [u8; 32] = words([4, 4, 4, 4, 6, 6, 6, 6], [5, 5, 5, 5, 7, 7, 7, 7]);

/// [`RAMP`] scaled for a rounding high multiply, then [`CLAMP`], over both
/// lanes.
const RAMP_LANES: [[i16; 16]; REACH] = {
    let mut table = [[0; 16]; REACH];
    let mut j = 0;
    while j < REACH {
        let mut index = 0;
        while index < 16 {
            table[j][index] = RAMP[j][index % 8] / 4;
            index += 1;
        }
        j += 1;
    }
    table
};
const CLAMP_LANES: [i16; 16] = {
    let mut table = [0; 16];
    let mut index = 0;
    while index < 16 {
        table[index] = CLAMP[index % 8];
        index += 1;
    }
    table
};

/// Filters the validated horizontal-edge segments `edges` of one run. Each
/// chunk covers eight segments; a run's last chunk ends at its last segment
/// and skips those an earlier chunk filtered, and a run of fewer than eight
/// filters a copy of its window.
pub(super) fn horizontal_run(
    token: Avx2,
    samples: &mut [u8],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge],
) {
    if edges.len() < SEGMENTS {
        window::<false>(token, samples, stride, first, edges);
    } else {
        horizontal(token, samples, stride, first, edges);
    }
}

/// Filters the validated vertical-edge band of
/// [`super::deblock_vertical_edge_band`]; a band of fewer than eight rows of
/// segments filters each live column through a copy of its window.
pub(super) fn band(
    token: Avx2,
    samples: &mut [u8],
    stride: usize,
    first: usize,
    (edges, segments, edge_stride): (&[DeblockEdge], usize, usize),
    live: &[bool],
) {
    if segments >= SEGMENTS {
        return vertical(
            token,
            samples,
            stride,
            first,
            (edges, segments, edge_stride),
            live,
        );
    }
    for (column, _) in live.iter().enumerate().filter(|(_, live)| **live) {
        let mut run = [DeblockEdge::NONE; SEGMENTS];
        for (k, slot) in run.iter_mut().take(segments).enumerate() {
            *slot = edges
                .get(column + k * edge_stride)
                .copied()
                .unwrap_or_default();
        }
        let first = first + 4 * column;
        window::<true>(token, samples, stride, first, &run[..segments]);
    }
}

/// Filters the `edges.len() < 8` segments at `first` through a copy of
/// their window, so that no load or store leaves their lines.
fn window<const VERTICAL: bool>(
    token: Avx2,
    samples: &mut [u8],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge],
) {
    let count = edges.len();
    let mut padded = [DeblockEdge::NONE; SEGMENTS];
    for (lane, edge) in padded.iter_mut().zip(edges) {
        *lane = *edge;
    }
    if padded.iter().all(|edge| edge.is_none()) {
        return;
    }
    let lines = count * 4;
    let mut window = [0; 2 * REACH * LINES];
    let (copies, origin, len) = if VERTICAL {
        (lines, first - REACH, 2 * REACH)
    } else {
        (2 * REACH, first - REACH * stride, lines)
    };
    let window_stride = if VERTICAL { 2 * REACH } else { LINES };
    for (index, copy) in window
        .chunks_exact_mut(window_stride)
        .take(copies)
        .enumerate()
    {
        let start = origin + index * stride;
        copy[..len].copy_from_slice(&samples[start..start + len]); // splot-copy-ok: stage a short run's window
    }
    if VERTICAL {
        let runs = (&padded[..], SEGMENTS, 1);
        vertical(token, &mut window, window_stride, REACH, runs, &[true]);
    } else {
        horizontal(token, &mut window, window_stride, REACH * LINES, &padded);
    }
    for (index, copy) in window.chunks_exact(window_stride).take(copies).enumerate() {
        let start = origin + index * stride;
        samples[start..start + len].copy_from_slice(&copy[..len]); // splot-copy-ok: publish a short run's window
    }
}

/// Largest row stride the kernels take: bounding it lets the line loops
/// prove their trip counts and row offsets.
pub(super) const MAX_STRIDE: usize = 1 << 24;

/// Runs `$body` for each chunk of a run of `edges.len() >= 8` segments that
/// filters something, with `$start` its first segment, `$low` and `$high`
/// the packed words of its edges (`qThr | sideThr << 16` and `maxWidthPos |
/// maxWidthNeg << 8 | flags << 16`, four edges per 128-bit lane) and
/// `$widest` their largest `maxWidthPos`.
macro_rules! for_chunks {
    ($token:ident, $edges:ident, |$start:ident, $low:ident, $high:ident, $widest:ident| $body:block) => {
        let count = $edges.len();
        for chunk in 0..count.div_ceil(SEGMENTS) {
            let $start = (chunk * SEGMENTS).min(count - SEGMENTS);
            let done = chunk * SEGMENTS - $start;
            let Some(run) = $edges.get($start..).and_then(<[DeblockEdge]>::first_chunk::<SEGMENTS>)
            else {
                return;
            };
            let lanes = run.map(|edge| edge.0 as i64);
            if lanes.iter().fold(0, |any, lane| any | lane) == 0 {
                continue;
            }
            let ($low, $high, $widest) = pack($token, lanes, done as i32);
            if $widest != 0 $body
        }
    };
}

kernel!(
    /// The packed words of eight edges, the first `done` cleared, and their
    /// largest `maxWidthPos`.
    #[inline(always)]
    fn pack(token: Avx2, lanes: [i64; SEGMENTS], done: i32) -> (__m256i, __m256i, usize) {
        let split = _mm256_setr_epi32(0, 2, 4, 6, 1, 3, 5, 7);
        let half = |e: &[i64]| {
            _mm256_permutevar8x32_epi32(_mm256_setr_epi64x(e[0], e[1], e[2], e[3]), split)
        };
        let (a, b) = (half(&lanes[..4]), half(&lanes[4..]));
        let keep = _mm256_cmpgt_epi32(
            _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7),
            _mm256_set1_epi32(done - 1),
        );
        let low = _mm256_and_si256(_mm256_permute2x128_si256::<0x20>(a, b), keep);
        let high = _mm256_and_si256(_mm256_permute2x128_si256::<0x31>(a, b), keep);
        let pos = _mm256_and_si256(high, _mm256_set1_epi32(0xff));
        let pos = _mm256_max_epu32(pos, _mm256_shuffle_epi32::<0x4e>(pos));
        let pos = _mm256_max_epu32(pos, _mm256_shuffle_epi32::<0xb1>(pos));
        let pos = _mm_max_epu32(
            _mm256_castsi256_si128(pos),
            _mm256_extracti128_si256::<1>(pos),
        );
        (low, high, _mm_cvtsi128_si32(pos) as usize)
    }
);

kernel!(
    #[inline(always)]
    fn constant(token: Avx2, bytes: &[u8; 32]) -> __m256i {
        u8x32::from_slice(token, bytes).into()
    }
);

kernel!(
    #[inline(always)]
    fn words16(token: Avx2, values: &[i16; 16]) -> __m256i {
        i16x16::from_slice(token, values).into()
    }
);

/// Filters the chunk whose sample rows across the edge are `$rows`
/// (`$rows[REACH + j]` holds `s[j]` of every line) for edges whose widest
/// `maxWidthPos` is at most `$class`: reads the § 7.17.7.2 `s[-n..n]`, `n =
/// Clip3(3, 8, $class + 1)`, and changes `s[-$class..$class]`. Evaluates to
/// whether any sample may have changed.
macro_rules! filter_chunk {
    ($token:ident, $rows:ident, $low:expr, $high:expr, $class:literal) => {{
        let token = $token;
        let (low, high): (__m256i, __m256i) = ($low, $high);
        let pick =
            |x: __m256i, pattern: &[u8; 32]| _mm256_shuffle_epi8(x, constant(token, pattern));
        let (q, side) = (pick(low, &Q_THR), pick(low, &SIDE));
        let (max_pos, max_neg) = (pick(high, &MAX_POS), pick(high, &MAX_NEG));
        let zero = _mm256_setzero_si256();
        let splat = |value: i16| _mm256_set1_epi16(value);
        let gt = |a, b| _mm256_cmpgt_epi16(a, b);
        let e = |j: isize| pick($rows[(REACH as isize + j) as usize], &SIDES);
        let mean = |x| _mm256_avg_epu16(x, _mm256_shuffle_epi32::<0x4e>(x));
        let second = |a, b, c| {
            let sum = _mm256_sub_epi16(_mm256_add_epi16(a, c), _mm256_add_epi16(b, b));
            mean(_mm256_abs_epi16(sum))
        };
        let (em2, em1, e0, e1) = (e(-2), e(-1), e(0), e(1));
        let high_deriv = _mm256_max_epi16(second(e(-3), em2, em1), second(e0, e1, e(2)));
        let valid = gt(_mm256_min_epi16(q, side), zero);
        let one = _mm256_andnot_si256(gt(high_deriv, side), valid);
        let mut index = _mm256_sub_epi16(zero, one);
        if $class > 1 {
            let transition = _mm256_add_epi16(second(em2, em1, e0), second(em1, e0, e1));
            let fails = |side_shift: __m256i, q_limit| {
                _mm256_or_si256(gt(high_deriv, side_shift), gt(transition, q_limit))
            };
            let two = _mm256_andnot_si256(
                fails(_mm256_srai_epi16::<2>(side), _mm256_slli_epi16::<2>(q)),
                _mm256_and_si256(one, gt(max_pos, splat(1))),
            );
            let three = _mm256_andnot_si256(
                fails(
                    _mm256_srai_epi16::<3>(side),
                    _mm256_mullo_epi16(q, splat(3)),
                ),
                two,
            );
            let toward = |edge, inner, outer, n: i16| {
                let step = _mm256_mullo_epi16(_mm256_sub_epi16(edge, inner), splat(n));
                mean(_mm256_abs_epi16(_mm256_sub_epi16(
                    _mm256_sub_epi16(edge, outer),
                    step,
                )))
            };
            let ends = |dist: i16, end| {
                let reach = dist as isize;
                let neg = _mm256_and_si256(
                    gt(max_neg, splat(dist - 1)),
                    gt(toward(em1, em2, e(-reach - 1), dist), end),
                );
                _mm256_or_si256(neg, gt(toward(e0, e1, e(reach), dist), end))
            };
            let end = _mm256_srai_epi16::<4>(_mm256_mullo_epi16(side, splat(3)));
            let three = _mm256_andnot_si256(ends(3, end), three);
            index = _mm256_sub_epi16(_mm256_sub_epi16(index, two), three);
            if $class > 3 {
                let wide = |previous, dist: i16, q_first: i32, end| {
                    let q_limit =
                        _mm256_srai_epi16::<4>(_mm256_mullo_epi16(q, splat(q_first as i16)));
                    let fail = _mm256_or_si256(gt(transition, q_limit), ends(dist.min(7), end));
                    _mm256_andnot_si256(
                        fail,
                        _mm256_and_si256(previous, gt(max_pos, splat(dist - 1))),
                    )
                };
                let four = wide(three, 4, Q_FIRST[0], _mm256_srai_epi16::<2>(side));
                index = _mm256_sub_epi16(index, four);
                if $class > 4 {
                    let six_end = _mm256_srai_epi16::<4>(_mm256_mullo_epi16(side, splat(6)));
                    let six = wide(four, 6, Q_FIRST[2], six_end);
                    index = _mm256_sub_epi16(index, six);
                    if $class > 6 {
                        let eight = wide(six, 8, Q_FIRST[4], _mm256_srai_epi16::<1>(side));
                        index = _mm256_sub_epi16(index, eight);
                    }
                }
            }
        }
        if _mm256_testz_si256(index, index) != 0 {
            false
        } else {
            let flags = pick(high, &FLAGS);
            let keep = |bit| _mm256_cmpeq_epi16(_mm256_and_si256(flags, splat(bit)), zero);
            let neg_width = _mm256_add_epi16(
                max_neg,
                _mm256_add_epi16(gt(max_neg, splat(4)), gt(max_neg, splat(6))),
            );
            let bytes =
                |index| _mm256_add_epi16(_mm256_mullo_epi16(index, splat(0x0202)), splat(0x0100));
            let pos = bytes(_mm256_and_si256(index, keep(2)));
            let neg = bytes(_mm256_and_si256(
                _mm256_min_epi16(index, neg_width),
                keep(1),
            ));
            let clamp = _mm256_mullo_epi16(
                q,
                _mm256_shuffle_epi8(words16(token, &CLAMP_LANES), bytes(index)),
            );
            let (p1, p0, q0, q1) = (
                $rows[REACH - 2],
                $rows[REACH - 1],
                $rows[REACH],
                $rows[REACH + 1],
            );
            let taps = splat(0x040c);
            let delta = |clamp, outer: __m256i, inner: __m256i| {
                let delta = _mm256_sub_epi16(
                    _mm256_maddubs_epi16(outer, taps),
                    _mm256_maddubs_epi16(inner, taps),
                );
                _mm256_min_epi16(
                    _mm256_max_epi16(delta, _mm256_sub_epi16(zero, clamp)),
                    clamp,
                )
            };
            let low_delta = delta(
                pick(clamp, &LOW_LINES),
                _mm256_unpacklo_epi8(q0, p1),
                _mm256_unpacklo_epi8(p0, q1),
            );
            let high_delta = delta(
                pick(clamp, &HIGH_LINES),
                _mm256_unpackhi_epi8(q0, p1),
                _mm256_unpackhi_epi8(p0, q1),
            );
            let (pos_low, pos_high) = (pick(pos, &LOW_LINES), pick(pos, &HIGH_LINES));
            let (neg_low, neg_high) = (pick(neg, &LOW_LINES), pick(neg, &HIGH_LINES));
            let mut moved = true;
            for j in 0..$class {
                let ramp = words16(token, &RAMP_LANES[j]);
                let diff =
                    |delta, index| _mm256_mulhrs_epi16(delta, _mm256_shuffle_epi8(ramp, index));
                let (curr_low, curr_high) = (diff(low_delta, pos_low), diff(high_delta, pos_high));
                let (prev_low, prev_high) = (diff(low_delta, neg_low), diff(high_delta, neg_high));
                if j == 0 {
                    let any = _mm256_or_si256(
                        _mm256_or_si256(curr_low, curr_high),
                        _mm256_or_si256(prev_low, prev_high),
                    );
                    if _mm256_testz_si256(any, any) != 0 {
                        moved = false;
                        break;
                    }
                }
                let curr = $rows[REACH + j];
                $rows[REACH + j] = _mm256_packus_epi16(
                    _mm256_sub_epi16(_mm256_unpacklo_epi8(curr, zero), curr_low),
                    _mm256_sub_epi16(_mm256_unpackhi_epi8(curr, zero), curr_high),
                );
                let prev = $rows[REACH - 1 - j];
                $rows[REACH - 1 - j] = _mm256_packus_epi16(
                    _mm256_add_epi16(_mm256_unpacklo_epi8(prev, zero), prev_low),
                    _mm256_add_epi16(_mm256_unpackhi_epi8(prev, zero), prev_high),
                );
            }
            moved
        }
    }};
}

kernel!(
    /// Filters the horizontal edges of the `edges.len() >= 8` segments at
    /// `first`.
    #[inline]
    fn horizontal(
        token: Avx2,
        samples: &mut [u8],
        stride: usize,
        first: usize,
        edges: &[DeblockEdge],
    ) {
        if !(LINES..=MAX_STRIDE).contains(&stride) {
            return;
        }
        for_chunks!(token, edges, |start, low, high, widest| {
            let first = first + 4 * start;
            let mut rows = [_mm256_setzero_si256(); 2 * REACH];
            macro_rules! band {
                ($reach:literal, $row:ident, $vector:ident, $body:block) => {{
                    let origin = first - $reach * stride;
                    let span = (2 * $reach - 1) * stride;
                    if let Some(band) = samples.get_mut(origin..origin + span + LINES) {
                        let vectors = &mut rows[REACH - $reach..REACH + $reach];
                        for (line, $vector) in vectors.iter_mut().enumerate() {
                            let $row = &mut band[line * stride..line * stride + LINES];
                            $body
                        }
                    }
                }};
            }
            macro_rules! rows {
                ($class:literal, $reach:literal) => {{
                    band!($reach, row, vector, {
                        *vector = u8x32::from_slice(token, row).into();
                    });
                    if filter_chunk!(token, rows, low, high, $class) {
                        band!($class, row, vector, {
                            u8x32::simd_from(token, *vector).store_slice(row);
                        });
                    }
                }};
            }
            match widest {
                1 => rows!(1, 3),
                3 => rows!(3, 4),
                4 => rows!(4, 5),
                6 => rows!(6, 7),
                _ => rows!(8, 8),
            }
        });
    }
);

kernel!(
    /// One round of the byte network that transposes 8 bytes of 32 lines:
    /// vector `i` of `v` holds lines `2i, 2i + 1` and `16 + 2i, 17 + 2i`.
    /// Interleaving vector `i` with `i + 4` rotates the 7-bit vector-and-byte
    /// address left by one, so four rounds leave byte `c` of every line in
    /// vector `c` and three more restore the lines.
    #[inline(always)]
    fn narrow_round(token: Avx2, v: &mut [__m256i; 8]) {
        let source = *v;
        for i in 0..4 {
            v[2 * i] = _mm256_unpacklo_epi8(source[i], source[i + 4]);
            v[2 * i + 1] = _mm256_unpackhi_epi8(source[i], source[i + 4]);
        }
    }
);

kernel!(
    /// [`narrow_round`] for 16 bytes of 32 lines, vector `i` holding lines
    /// `i` and `16 + i`: four rounds transpose, four more restore.
    #[inline(always)]
    fn wide_round(token: Avx2, v: &mut [__m256i; 16]) {
        let source = *v;
        for i in 0..8 {
            v[2 * i] = _mm256_unpacklo_epi8(source[i], source[i + 8]);
            v[2 * i + 1] = _mm256_unpackhi_epi8(source[i], source[i + 8]);
        }
    }
);

/// Runs `$body` once for each literal in the list, with `$i` bound to it as
/// a constant, so that every array index folds.
macro_rules! each {
    ($i:ident in [$($n:literal),*] $body:block) => {
        $({
            const $i: usize = $n;
            $body
        })*
    };
}

kernel!(
    /// Filters the vertical edges of a band of `segments >= 8` rows of
    /// segments, `edges[c + k * edge_stride]` for column `c` and row `k`,
    /// chunk row by chunk row and the `live` columns of each left to right:
    /// bytes `s[-4..4]` of each line for chunks no wider than 3, `s[-8..8]`
    /// otherwise. Lines load and store in place: the bounded stride lets one
    /// check of the chunk's block cover every line.
    #[inline]
    fn vertical(
        token: Avx2,
        samples: &mut [u8],
        stride: usize,
        first: usize,
        runs: (&[DeblockEdge], usize, usize),
        live: &[bool],
    ) {
        let (edges, segments, edge_stride) = runs;
        if !(2 * REACH..=MAX_STRIDE).contains(&stride) || segments < SEGMENTS {
            return;
        }
        let columns = live.len();
        let span = (LINES - 1) * stride;
        let block_len = span + 2 * REACH;
        for chunk in 0..segments.div_ceil(SEGMENTS) {
            let start = (chunk * SEGMENTS).min(segments - SEGMENTS);
            let done = chunk * SEGMENTS - start;
            let rows: [&[DeblockEdge]; SEGMENTS] = core::array::from_fn(|lane| {
                let row = (start + lane) * edge_stride;
                edges.get(row..row + columns).unwrap_or_default()
            });
            if rows.iter().any(|row| row.len() != columns) {
                return;
            }
            let origin = first + 4 * start * stride - REACH;
            let Some(band) =
                samples.get_mut(origin..origin + 4 * columns.saturating_sub(1) + block_len)
            else {
                return;
            };
            for (column, _) in live.iter().enumerate().filter(|(_, live)| **live) {
                let lanes = rows.map(|row| row.get(column).map_or(0, |edge| edge.0 as i64));
                let (low, high, widest) = pack(token, lanes, done as i32);
                if widest == 0 {
                    continue;
                }
                let Some(block) = band.get_mut(4 * column..4 * column + block_len) else {
                    return;
                };
                macro_rules! line {
                    ($i:expr) => {
                        &mut block[$i * stride..$i * stride + 2 * REACH]
                    };
                }
                macro_rules! narrow {
                    ($class:literal) => {{
                        let mut word = |i: usize| {
                            let mut bytes = [0; 8];
                            bytes.copy_from_slice(&line!(i)[REACH / 2..REACH / 2 + 8]); // splot-copy-ok: one 8-byte load
                            f64::from_bits(u64::from_le_bytes(bytes))
                        };
                        let mut pair = |low, high| _mm_castpd_si128(_mm_setr_pd(word(low), word(high)));
                        let mut v = [_mm256_setzero_si256(); 8];
                        each!(I in [0, 1, 2, 3, 4, 5, 6, 7] {
                            v[I] = _mm256_inserti128_si256::<1>(
                                _mm256_castsi128_si256(pair(2 * I, 2 * I + 1)),
                                pair(16 + 2 * I, 17 + 2 * I),
                            );
                        });
                        for _ in 0..4 {
                            narrow_round(token, &mut v);
                        }
                        let mut rows = [_mm256_setzero_si256(); 2 * REACH];
                        each!(I in [0, 1, 2, 3, 4, 5, 6, 7] {
                            rows[REACH / 2 + I] = v[I];
                        });
                        if filter_chunk!(token, rows, low, high, $class) {
                            each!(I in [0, 1, 2, 3, 4, 5, 6, 7] {
                                v[I] = rows[REACH / 2 + I];
                            });
                            for _ in 0..3 {
                                narrow_round(token, &mut v);
                            }
                            let mut put = |i: usize, word: i64| {
                                line!(i)[REACH / 2..REACH / 2 + 8].copy_from_slice(&word.to_le_bytes()); // splot-copy-ok: one 8-byte store
                            };
                            each!(I in [0, 1, 2, 3, 4, 5, 6, 7] {
                                put(2 * I, _mm256_extract_epi64::<0>(v[I]));
                                put(2 * I + 1, _mm256_extract_epi64::<1>(v[I]));
                                put(16 + 2 * I, _mm256_extract_epi64::<2>(v[I]));
                                put(17 + 2 * I, _mm256_extract_epi64::<3>(v[I]));
                            });
                        }
                    }};
                }
                macro_rules! wide {
                    ($class:literal) => {{
                        let mut load = |i: usize| -> __m128i { u8x16::from_slice(token, line!(i)).into() };
                        let mut v = [_mm256_setzero_si256(); 16];
                        each!(I in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
                            v[I] = _mm256_inserti128_si256::<1>(_mm256_castsi128_si256(load(I)), load(I + 16));
                        });
                        for _ in 0..4 {
                            wide_round(token, &mut v);
                        }
                        if filter_chunk!(token, v, low, high, $class) {
                            for _ in 0..4 {
                                wide_round(token, &mut v);
                            }
                            let mut put = |i: usize, value: __m128i| {
                                u8x16::simd_from(token, value).store_slice(line!(i));
                            };
                            each!(I in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
                                put(I, _mm256_castsi256_si128(v[I]));
                                put(I + 16, _mm256_extracti128_si256::<1>(v[I]));
                            });
                        }
                    }};
                }
                match widest {
                    1 => narrow!(1),
                    3 => narrow!(3),
                    4 => wide!(4),
                    6 => wide!(6),
                    _ => wide!(8),
                }
            }
        }
    }
);
