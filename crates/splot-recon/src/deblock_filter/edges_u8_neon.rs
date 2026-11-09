// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! NEON § 7.17.7 deblocking of one 8-bit chunk of four segments, 16 lines
//! (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-7`): the 128-bit
//! form of the AVX2 kernel in `edges_u8.rs`.
//!
//! Vector `8 + j` of a chunk holds byte `s[j]` of all 16 lines: horizontal
//! edges load sample rows as they are, vertical edges transpose 8 or 16
//! bytes of each line. The § 7.17.7.2 choice reads only the first and last
//! line of a segment, so it runs on one vector of 16-bit lanes of those two
//! lines of the four segments; the § 7.17.7.1 ramp widens a row only to
//! apply its taps. A chunk takes the cheapest kernel that covers its widest
//! `maxWidthPos`.

use core::arch::aarch64::*;

use fearless_simd::{SimdBase, SimdFrom, i16x8, kernel, u8x16};

use super::{CLAMP, DeblockEdge, Q_FIRST, RAMP, REACH, SEGMENTS};

/// Lines of one chunk, one per byte lane.
const LINES: usize = 4 * SEGMENTS;
/// A `tbl` index that reads zero.
const ZERO: u8 = 0xff;

/// A `tbl` that sets word `w` to bytes `low[w]` and `high[w]`.
const fn words(low: [u8; 8], high: [u8; 8]) -> [u8; 16] {
    let mut bytes = [0; 16];
    let mut w = 0;
    while w < 8 {
        bytes[2 * w] = low[w];
        bytes[2 * w + 1] = high[w];
        w += 1;
    }
    bytes
}

/// The first lines of the four segments, then their last lines.
const SIDES: [u8; 16] = words([0, 4, 8, 12, 3, 7, 11, 15], [ZERO; 8]);
/// A segment word over the lines of the low and high halves of a widened
/// row: lines `0..8` and `8..16`.
const LOW_LINES: [u8; 16] = words([0, 0, 0, 0, 2, 2, 2, 2], [1, 1, 1, 1, 3, 3, 3, 3]);
const HIGH_LINES: [u8; 16] = words([4, 4, 4, 4, 6, 6, 6, 6], [5, 5, 5, 5, 7, 7, 7, 7]);

/// The `tbl` over the four packed edges that sets decision word `w` to
/// bytes `low` and `high` (or zero) of edge `w % 4`.
const fn field(low: u8, high: u8) -> [u8; 16] {
    let mut bytes = [0; 16];
    let mut w = 0;
    while w < 8 {
        let edge = 8 * (w % 4) as u8;
        bytes[2 * w] = edge + low;
        bytes[2 * w + 1] = if high == ZERO { ZERO } else { edge + high };
        w += 1;
    }
    bytes
}

const Q_THR: [u8; 16] = field(0, 1);
const SIDE: [u8; 16] = field(2, 3);
const MAX_POS: [u8; 16] = field(4, ZERO);
const MAX_NEG: [u8; 16] = field(5, ZERO);
const FLAGS: [u8; 16] = field(6, ZERO);

/// [`RAMP`] scaled for a rounding doubling high multiply: `RAMP` is a
/// multiple of 64, and `vqrdmulh(d, r / 4) == Round2(d * r, 17)` for the
/// non-negative `r`.
const RAMP_QUARTER: [[i16; 8]; REACH] = {
    let mut table = [[0; 8]; REACH];
    let mut j = 0;
    while j < REACH {
        let mut index = 0;
        while index < 8 {
            table[j][index] = RAMP[j][index] / 4;
            index += 1;
        }
        j += 1;
    }
    table
};

/// The § 7.17.7 fields of the four edges over the decision words.
#[derive(Clone, Copy)]
struct Fields {
    q: int16x8_t,
    side: int16x8_t,
    max_pos: int16x8_t,
    max_neg: int16x8_t,
    flags: int16x8_t,
}

kernel!(
    #[inline(always)]
    fn bytes(token: Neon, values: &[u8; 16]) -> uint8x16_t {
        u8x16::load_array_ref(token, values).into()
    }
);

kernel!(
    #[inline(always)]
    fn halfwords(token: Neon, values: &[i16; 8]) -> uint8x16_t {
        vreinterpretq_u8_s16(i16x8::load_array_ref(token, values).into())
    }
);

kernel!(
    /// The [`Fields`] of `edges`.
    #[inline(always)]
    fn fields(token: Neon, edges: &[DeblockEdge; SEGMENTS]) -> Fields {
        let mut packed = [0u8; 8 * SEGMENTS];
        for (bytes, edge) in packed.as_chunks_mut::<8>().0.iter_mut().zip(edges) {
            *bytes = edge.0.to_le_bytes();
        }
        let (low, high) = packed.split_at(16);
        let table = uint8x16x2_t(
            u8x16::from_slice(token, low).into(),
            u8x16::from_slice(token, high).into(),
        );
        let pick = |pattern| vreinterpretq_s16_u8(vqtbl2q_u8(table, bytes(token, pattern)));
        Fields {
            q: pick(&Q_THR),
            side: pick(&SIDE),
            max_pos: pick(&MAX_POS),
            max_neg: pick(&MAX_NEG),
            flags: pick(&FLAGS),
        }
    }
);

kernel!(
    /// One round of the byte network that transposes 8 bytes of 16 lines,
    /// vector `i` holding lines `2i` and `2i + 1`: interleaving vector `i`
    /// with `i + 4` rotates the 7-bit vector-and-byte address left by one,
    /// so four rounds transpose and three more restore.
    #[inline(always)]
    fn narrow_round(token: Neon, v: &mut [uint8x16_t; 8]) {
        let source = *v;
        for i in 0..4 {
            v[2 * i] = vzip1q_u8(source[i], source[i + 4]);
            v[2 * i + 1] = vzip2q_u8(source[i], source[i + 4]);
        }
    }
);

kernel!(
    /// [`narrow_round`] for 16 bytes of 16 lines, vector `i` holding line
    /// `i`: four rounds transpose, four more restore.
    #[inline(always)]
    fn wide_round(token: Neon, v: &mut [uint8x16_t; 16]) {
        let source = *v;
        for i in 0..8 {
            v[2 * i] = vzip1q_u8(source[i], source[i + 8]);
            v[2 * i + 1] = vzip2q_u8(source[i], source[i + 8]);
        }
    }
);

/// Filters the chunk whose sample rows across the edge are `$rows`
/// (`$rows[REACH + j]` holds `s[j]` of every line) for edges whose widest
/// `maxWidthPos` is at most `$class`: reads the § 7.17.7.2 `s[-n..n]`, `n =
/// Clip3(3, 8, $class + 1)`, and changes `s[-$class..$class]`. Evaluates to
/// whether any sample may have changed. A macro rather than a kernel so that
/// the rows stay in registers in each kernel that loads them.
macro_rules! filter_chunk {
    ($token:ident, $rows:ident, $fields:expr, $class:literal) => {
        'filter: {
            let token = $token;
            let Fields {
                q,
                side,
                max_pos,
                max_neg,
                flags,
            } = $fields;
            let read = ($class.min(7) + 1).max(3);
            let mut sides = [vdupq_n_s16(0); 2 * REACH];
            let span = REACH - read..REACH + read;
            for (side, &row) in sides[span.clone()].iter_mut().zip(&$rows[span]) {
                *side = vreinterpretq_s16_u8(vqtbl1q_u8(row, bytes(token, &SIDES)));
            }
            let e = |j: isize| sides[(REACH as isize + j) as usize];
            let zero = vdupq_n_s16(0);
            let splat = |value: i16| vdupq_n_s16(value);
            let gt = |a, b| vreinterpretq_s16_u16(vcgtq_s16(a, b));
            let mean = |x: int16x8_t| {
                let x = vreinterpretq_u16_s16(x);
                vreinterpretq_s16_u16(vrhaddq_u16(x, vextq_u16::<4>(x, x)))
            };
            let second = |a, b, c| mean(vabsq_s16(vsubq_s16(vaddq_s16(a, c), vaddq_s16(b, b))));
            let (em2, em1, e0, e1) = (e(-2), e(-1), e(0), e(1));
            let high_deriv = vmaxq_s16(second(e(-3), em2, em1), second(e0, e1, e(2)));
            let valid = gt(vminq_s16(q, side), zero);
            let one = vbicq_s16(valid, gt(high_deriv, side));
            let mut index = vsubq_s16(zero, one);
            if $class > 1 {
                let transition = vaddq_s16(second(em2, em1, e0), second(em1, e0, e1));
                let fails = |side_shift, q_limit| {
                    vorrq_s16(gt(high_deriv, side_shift), gt(transition, q_limit))
                };
                let two = vbicq_s16(
                    vandq_s16(one, gt(max_pos, splat(1))),
                    fails(vshrq_n_s16::<2>(side), vshlq_n_s16::<2>(q)),
                );
                let three = vbicq_s16(two, fails(vshrq_n_s16::<3>(side), vmulq_s16(q, splat(3))));
                let toward = |edge, inner, outer, n: i16| {
                    let step = vmulq_s16(vsubq_s16(edge, inner), splat(n));
                    mean(vabsq_s16(vsubq_s16(vsubq_s16(edge, outer), step)))
                };
                let ends = |dist: i16, end| {
                    let reach = dist as isize;
                    let neg = vandq_s16(
                        gt(max_neg, splat(dist - 1)),
                        gt(toward(em1, em2, e(-reach - 1), dist), end),
                    );
                    vorrq_s16(neg, gt(toward(e0, e1, e(reach), dist), end))
                };
                let end = vshrq_n_s16::<4>(vmulq_s16(side, splat(3)));
                let three = vbicq_s16(three, ends(3, end));
                index = vsubq_s16(vsubq_s16(index, two), three);
                if $class > 3 {
                    let wide = |previous, dist: i16, q_first: i32, end| {
                        let q_limit = vshrq_n_s16::<4>(vmulq_s16(q, splat(q_first as i16)));
                        let fail = vorrq_s16(gt(transition, q_limit), ends(dist.min(7), end));
                        vbicq_s16(vandq_s16(previous, gt(max_pos, splat(dist - 1))), fail)
                    };
                    let four = wide(three, 4, Q_FIRST[0], vshrq_n_s16::<2>(side));
                    index = vsubq_s16(index, four);
                    if $class > 4 {
                        let six_end = vshrq_n_s16::<4>(vmulq_s16(side, splat(6)));
                        let six = wide(four, 6, Q_FIRST[2], six_end);
                        index = vsubq_s16(index, six);
                        if $class > 6 {
                            let eight = wide(six, 8, Q_FIRST[4], vshrq_n_s16::<1>(side));
                            index = vsubq_s16(index, eight);
                        }
                    }
                }
            }
            if vmaxvq_u16(vreinterpretq_u16_s16(index)) == 0 {
                break 'filter false;
            }
            let lookup = |x: uint8x16_t, pattern| vqtbl1q_u8(x, bytes(token, pattern));
            let keep =
                |index, bit| vbicq_s16(index, vreinterpretq_s16_u16(vtstq_s16(flags, splat(bit))));
            let neg_width = vaddq_s16(
                max_neg,
                vaddq_s16(gt(max_neg, splat(4)), gt(max_neg, splat(6))),
            );
            let table_bytes = |index| {
                vreinterpretq_u8_s16(vaddq_s16(vmulq_s16(index, splat(0x0202)), splat(0x0100)))
            };
            let pos = table_bytes(keep(index, 2));
            let neg = table_bytes(keep(vminq_s16(index, neg_width), 1));
            let clamp = vmulq_s16(
                q,
                vreinterpretq_s16_u8(vqtbl1q_u8(halfwords(token, &CLAMP), table_bytes(index))),
            );
            let clamp = vreinterpretq_u8_s16(clamp);
            let (p1, p0, q0, q1) = (
                $rows[REACH - 2],
                $rows[REACH - 1],
                $rows[REACH],
                $rows[REACH + 1],
            );
            let (twelve, four) = (vdupq_n_u8(12), vdupq_n_u8(4));
            let taps_low = |a, b| {
                vmlal_u8(
                    vmull_u8(vget_low_u8(a), vget_low_u8(twelve)),
                    vget_low_u8(b),
                    vget_low_u8(four),
                )
            };
            let taps_high = |a, b| vmlal_high_u8(vmull_high_u8(a, twelve), b, four);
            let delta = |clamp: uint8x16_t, outer, inner| {
                let clamp = vreinterpretq_s16_u8(clamp);
                let delta = vreinterpretq_s16_u16(vsubq_u16(outer, inner));
                vminq_s16(vmaxq_s16(delta, vnegq_s16(clamp)), clamp)
            };
            let low_delta = delta(
                lookup(clamp, &LOW_LINES),
                taps_low(q0, p1),
                taps_low(p0, q1),
            );
            let high_delta = delta(
                lookup(clamp, &HIGH_LINES),
                taps_high(q0, p1),
                taps_high(p0, q1),
            );
            let (pos_low, pos_high) = (lookup(pos, &LOW_LINES), lookup(pos, &HIGH_LINES));
            let (neg_low, neg_high) = (lookup(neg, &LOW_LINES), lookup(neg, &HIGH_LINES));
            for j in 0..$class {
                let ramp = halfwords(token, &RAMP_QUARTER[j]);
                let diff = |delta, index| {
                    vqrdmulhq_s16(delta, vreinterpretq_s16_u8(vqtbl1q_u8(ramp, index)))
                };
                let (curr_low, curr_high) = (diff(low_delta, pos_low), diff(high_delta, pos_high));
                let (prev_low, prev_high) = (diff(low_delta, neg_low), diff(high_delta, neg_high));
                if j == 0 {
                    let any = vorrq_s16(
                        vorrq_s16(curr_low, curr_high),
                        vorrq_s16(prev_low, prev_high),
                    );
                    if vmaxvq_u16(vreinterpretq_u16_s16(any)) == 0 {
                        break 'filter false;
                    }
                }
                let sub = |curr: uint16x8_t, diff| vsubq_s16(vreinterpretq_s16_u16(curr), diff);
                let curr = $rows[REACH + j];
                $rows[REACH + j] = vqmovun_high_s16(
                    vqmovun_s16(sub(vmovl_u8(vget_low_u8(curr)), curr_low)),
                    sub(vmovl_high_u8(curr), curr_high),
                );
                let add =
                    |diff, prev| vreinterpretq_s16_u16(vaddw_u8(vreinterpretq_u16_s16(diff), prev));
                let prev = $rows[REACH - 1 - j];
                $rows[REACH - 1 - j] = vqmovun_high_s16(
                    vqmovun_s16(add(prev_low, vget_low_u8(prev))),
                    vreinterpretq_s16_u16(vaddw_high_u8(vreinterpretq_u16_s16(prev_high), prev)),
                );
            }
            true
        }
    };
}

/// Filters the horizontal edges of the chunk at `first` for `$class`,
/// reading `$read` rows either side.
macro_rules! horizontal {
    ($token:ident, $samples:ident, $stride:ident, $origin:ident, $fields:ident, $class:literal, $read:literal) => {{
        let mut rows = [vdupq_n_u8(0); 2 * REACH];
        let mut loaded = true;
        for (j, row) in (REACH - $read..).zip(&mut rows[REACH - $read..REACH + $read]) {
            match $samples
                .get($origin + j * $stride..)
                .and_then(<[u8]>::first_chunk::<LINES>)
            {
                Some(line) => *row = u8x16::load_array_ref($token, line).into(),
                None => loaded = false,
            }
        }
        if loaded && filter_chunk!($token, rows, $fields, $class) {
            for (j, &row) in (REACH - $class..).zip(&rows[REACH - $class..REACH + $class]) {
                if let Some(line) = $samples
                    .get_mut($origin + j * $stride..)
                    .and_then(<[u8]>::first_chunk_mut::<LINES>)
                {
                    *line = u8x16::simd_from($token, row).to_array();
                }
            }
        }
    }};
}

/// Filters the vertical edges of the chunk at `first` for `$class` over
/// `s[-8..8]` of each line.
macro_rules! vertical_wide {
    ($token:ident, $samples:ident, $stride:ident, $origin:ident, $fields:ident, $class:literal) => {{
        let mut v = [vdupq_n_u8(0); 2 * REACH];
        let mut loaded = true;
        for (line, v) in v.iter_mut().enumerate() {
            match $samples
                .get($origin + line * $stride..)
                .and_then(<[u8]>::first_chunk::<{ 2 * REACH }>)
            {
                Some(bytes) => *v = u8x16::load_array_ref($token, bytes).into(),
                None => loaded = false,
            }
        }
        if loaded {
            for _ in 0..4 {
                wide_round($token, &mut v);
            }
            if filter_chunk!($token, v, $fields, $class) {
                for _ in 0..4 {
                    wide_round($token, &mut v);
                }
                for (line, &v) in v.iter().enumerate() {
                    if let Some(bytes) = $samples
                        .get_mut($origin + line * $stride..)
                        .and_then(<[u8]>::first_chunk_mut::<{ 2 * REACH }>)
                    {
                        *bytes = u8x16::simd_from($token, v).to_array();
                    }
                }
            }
        }
    }};
}

/// Filters the vertical edges of the chunk at `first` for `$class` over
/// `s[-4..4]` of each line.
macro_rules! vertical_narrow {
    ($token:ident, $samples:ident, $stride:ident, $origin:ident, $fields:ident, $class:literal) => {{
        let at = |line: usize| $origin + line * $stride + REACH / 2;
        let mut v = [vdupq_n_u8(0); 8];
        let mut loaded = true;
        for (i, v) in v.iter_mut().enumerate() {
            let word = |line| {
                $samples
                    .get(at(line)..)
                    .and_then(<[u8]>::first_chunk::<8>)
                    .map(|bytes| vcreate_u8(u64::from_le_bytes(*bytes)))
            };
            match (word(2 * i), word(2 * i + 1)) {
                (Some(low), Some(high)) => *v = vcombine_u8(low, high),
                _ => loaded = false,
            }
        }
        if loaded {
            for _ in 0..4 {
                narrow_round($token, &mut v);
            }
            let mut rows = [vdupq_n_u8(0); 2 * REACH];
            rows[REACH / 2..REACH / 2 + 8].copy_from_slice(&v); // splot-copy-ok: vectors stay in registers
            if filter_chunk!($token, rows, $fields, $class) {
                v.copy_from_slice(&rows[REACH / 2..REACH / 2 + 8]); // splot-copy-ok: vectors stay in registers
                for _ in 0..3 {
                    narrow_round($token, &mut v);
                }
                for (i, &v) in v.iter().enumerate() {
                    let v = vreinterpretq_u64_u8(v);
                    let lines = [(2 * i, vgetq_lane_u64::<0>(v)), (2 * i + 1, vgetq_lane_u64::<1>(v))];
                    for (line, word) in lines {
                        if let Some(bytes) = $samples
                            .get_mut(at(line)..)
                            .and_then(<[u8]>::first_chunk_mut::<8>)
                        {
                            *bytes = word.to_le_bytes();
                        }
                    }
                }
            }
        }
    }};
}

kernel!(
    /// Filters the [`SEGMENTS`] edges at `first` as [`super::Chunk::chunk`]
    /// does, across vertical edges when `vertical`: the window is
    /// `first ± 8` across the edge and 16 lines along it.
    #[inline]
    pub(super) fn chunk(
        token: Neon,
        samples: &mut [u8],
        stride: usize,
        first: usize,
        edges: &[DeblockEdge; SEGMENTS],
        vertical: bool,
    ) {
        let widest = edges
            .iter()
            .fold(0, |widest, edge| widest.max(edge.max_width_pos()));
        let origin = if vertical {
            first.checked_sub(REACH)
        } else {
            first.checked_sub(REACH * stride)
        };
        let Some(origin) = origin.filter(|_| widest != 0) else {
            return;
        };
        let fields = fields(token, edges);
        match (vertical, widest) {
            (true, 1) => vertical_narrow!(token, samples, stride, origin, fields, 1),
            (true, 3) => vertical_narrow!(token, samples, stride, origin, fields, 3),
            (true, 4) => vertical_wide!(token, samples, stride, origin, fields, 4),
            (true, 6) => vertical_wide!(token, samples, stride, origin, fields, 6),
            (true, _) => vertical_wide!(token, samples, stride, origin, fields, 8),
            (false, 1) => horizontal!(token, samples, stride, origin, fields, 1, 3),
            (false, 3) => horizontal!(token, samples, stride, origin, fields, 3, 4),
            (false, 4) => horizontal!(token, samples, stride, origin, fields, 4, 5),
            (false, 6) => horizontal!(token, samples, stride, origin, fields, 6, 7),
            (false, _) => horizontal!(token, samples, stride, origin, fields, 8, 8),
        }
    }
);
