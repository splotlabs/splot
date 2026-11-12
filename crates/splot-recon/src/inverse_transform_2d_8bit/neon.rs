// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! NEON form of the 8-bit [`InverseTransformAdd`].
//!
//! At 8 bits the § 7.14.4 dequantization clamp and the § 7.15.2.1 row
//! clamps are all `±2^15`, the `i16` range, so the row inputs and outputs
//! are `i16` lanes and each of those clamps is one `sqxtn` saturation. Each
//! product is one `smlal` by lane into `i32`: a row's lanes are its outputs,
//! a column's lanes are the block's columns. A `DCT` row splits into even
//! and odd inputs at every length and a `DCT` column into even and odd rows,
//! by the § 9.6 table identities checked here and in the parent module;
//! every product is still an input times a table entry, so the sums are the
//! § 7.15.2.1 matrix sums. The add is `Clip1(p + Sat16(r))` as in the parent.
//! The row kernels are macros: as functions, LLVM keeps the larger ones out
//! of line and passes their sums through memory.

use core::arch::aarch64::*;

use fearless_simd::{Neon, SimdBase as _, SimdFrom as _, i16x8, i32x4, u8x16};
use splot_tables::tables::transform_1d::{
    ADST_KERNEL4, ADST_KERNEL8, ADST_KERNEL16, DCT_KERNEL4, DCT_KERNEL8, DCT_KERNEL16,
    DCT_KERNEL32, DDTX_KERNEL8, DDTX_KERNEL16, FDST_KERNEL4, FDST_KERNEL8, FDST_KERNEL16,
};

use super::{FlatDequant, InverseTransformAdd, identity_scale};
use crate::inverse_transform::InverseTransform1dType;
use crate::inverse_transform_2d::InverseTransform2dDim;

/// Eight `i16` table entries, one vector.
type Lanes = [i16; 8];

/// `kernel[j][i]` (input `j`, output `i`) as `i16` in groups of eight
/// outputs, zero padded, the output index reversed for `FDDT`.
const fn table<const N: usize, const G: usize>(
    kernel: &[[i32; N]; N],
    reversed: bool,
) -> [[Lanes; G]; N] {
    assert!(8 * G >= N);
    let mut out = [[[0; 8]; G]; N];
    let mut j = 0;
    while j < N {
        let mut i = 0;
        while i < N {
            let v = kernel[j][if reversed { N - 1 - i } else { i }];
            assert!(v == v as i16 as i32);
            out[j][i / 8][i % 8] = v as i16;
            i += 1;
        }
        j += 1;
    }
    out
}

/// The odd rows of the `DCT` `kernel` over its first `H = N / 2` outputs.
const fn odd<const N: usize, const H: usize, const G: usize>(
    kernel: &[[i32; N]; N],
) -> [[Lanes; G]; H] {
    assert!(2 * H == N && 8 * G >= H);
    let mut out = [[[0; 8]; G]; H];
    let mut m = 0;
    while m < H {
        let mut i = 0;
        while i < H {
            out[m][i / 8][i % 8] = kernel[2 * m + 1][i] as i16;
            i += 1;
        }
        m += 1;
    }
    out
}

/// The `DCT` `kernel` for the column pass: entry `[p][g]` holds
/// `kernel[2p][4g..4g + 4]`, then `kernel[2p + 1][4g..4g + 4]`.
const fn pairs<const N: usize, const H: usize, const G: usize>(
    kernel: &[[i32; N]; N],
) -> [[Lanes; G]; H] {
    assert!(2 * H == N && 8 * G == N);
    let mut out = [[[0; 8]; G]; H];
    let mut p = 0;
    while p < H {
        let mut i = 0;
        while i < H {
            out[p][i / 4][i % 4] = kernel[2 * p][i] as i16;
            out[p][i / 4][4 + i % 4] = kernel[2 * p + 1][i] as i16;
            i += 1;
        }
        p += 1;
    }
    out
}

/// Row quads of the `DCT` `kernel` for the two-level column split: with
/// `odd == 0`, entry `[q][g]` holds `kernel[4q][4g..4g + 4]` then
/// `kernel[4q + 2][4g..4g + 4]` over the first `N / 4` outputs; with
/// `odd == 1`, `kernel[4q + 1]` then `kernel[4q + 3]` over the first `N / 2`.
const fn quads<const N: usize, const Q: usize, const G: usize>(
    kernel: &[[i32; N]; N],
    odd: usize,
) -> [[Lanes; G]; Q] {
    assert!(4 * Q == N && 4 * G == if odd == 0 { N / 4 } else { N / 2 });
    let mut out = [[[0; 8]; G]; Q];
    let mut q = 0;
    while q < Q {
        let mut i = 0;
        while i < 4 * G {
            out[q][i / 4][i % 4] = kernel[4 * q + odd][i] as i16;
            out[q][i / 4][4 + i % 4] = kernel[4 * q + 2 + odd][i] as i16;
            i += 1;
        }
        q += 1;
    }
    out
}

/// `full[2m][i] == half[m][i]` for `i < H`: the even rows of a `DCT` are the
/// half-size `DCT`.
const fn even_rows_halve<const N: usize, const H: usize>(
    full: &[[i32; N]; N],
    half: &[[i32; H]; H],
) -> bool {
    let mut m = 0;
    while m < H {
        let mut i = 0;
        while i < H {
            if full[2 * m][i] != half[m][i] {
                return false;
            }
            i += 1;
        }
        m += 1;
    }
    true
}

const _: () = assert!(
    even_rows_halve(&DCT_KERNEL8, &DCT_KERNEL4)
        && even_rows_halve(&DCT_KERNEL16, &DCT_KERNEL8)
        && even_rows_halve(&DCT_KERNEL32, &DCT_KERNEL16)
);

static DCT4: [[Lanes; 1]; 4] = table(&DCT_KERNEL4, false);
static ADST4: [[Lanes; 1]; 4] = table(&ADST_KERNEL4, false);
static FDST4: [[Lanes; 1]; 4] = table(&FDST_KERNEL4, false);
static ADST8: [[Lanes; 1]; 8] = table(&ADST_KERNEL8, false);
static FDST8: [[Lanes; 1]; 8] = table(&FDST_KERNEL8, false);
static DDTX8: [[Lanes; 1]; 8] = table(&DDTX_KERNEL8, false);
static FDDT8: [[Lanes; 1]; 8] = table(&DDTX_KERNEL8, true);
static ADST16: [[Lanes; 2]; 16] = table(&ADST_KERNEL16, false);
static FDST16: [[Lanes; 2]; 16] = table(&FDST_KERNEL16, false);
static DDTX16: [[Lanes; 2]; 16] = table(&DDTX_KERNEL16, false);
static FDDT16: [[Lanes; 2]; 16] = table(&DDTX_KERNEL16, true);
static ODD8: [[Lanes; 1]; 4] = odd(&DCT_KERNEL8);
static ODD16: [[Lanes; 1]; 8] = odd(&DCT_KERNEL16);
static ODD32: [[Lanes; 2]; 16] = odd(&DCT_KERNEL32);
static COL8: [[Lanes; 1]; 4] = pairs(&DCT_KERNEL8);
static COL16: [[Lanes; 2]; 8] = pairs(&DCT_KERNEL16);
static COL32: [[Lanes; 4]; 16] = pairs(&DCT_KERNEL32);
static EVEN16: [[Lanes; 1]; 4] = quads(&DCT_KERNEL16, 0);
static ODD16C: [[Lanes; 2]; 4] = quads(&DCT_KERNEL16, 1);
static EVEN32: [[Lanes; 2]; 8] = quads(&DCT_KERNEL32, 0);
static ODD32C: [[Lanes; 4]; 8] = quads(&DCT_KERNEL32, 1);

/// The row `DCT` tables: the 4-point `DCT` and the odd rows at 8, 16 and 32.
type DctTables = (
    [[Lanes; 1]; 4],
    [[Lanes; 1]; 4],
    [[Lanes; 1]; 8],
    [[Lanes; 2]; 16],
);
static DCT_TABLES: DctTables = (DCT4, ODD8, ODD16, ODD32);

/// [`DCT_TABLES`] behind an opaque pointer: read once per row, it keeps the
/// table loads in the row loops instead of hoisting more vectors than there
/// are registers.
#[inline]
fn tables() -> &'static DctTables {
    core::hint::black_box(&DCT_TABLES)
}

/// One resolved pass.
#[derive(Clone, Copy)]
enum Pass {
    /// The § 7.15.2.3 identity with its scale.
    Identity(i16),
    /// A 4-point kernel.
    Four(&'static [[Lanes; 1]; 4]),
    /// An 8-point kernel other than `DCT`.
    Eight(&'static [[Lanes; 1]; 8]),
    /// A 16-point kernel other than `DCT`.
    Sixteen(&'static [[Lanes; 2]; 16]),
    /// The `DCT` of the given length, at least 8.
    Dct(usize),
}

impl Pass {
    /// The pass of `dim` over `len` samples, as
    /// [`crate::inverse_transform::select_kernel`] resolves it.
    fn of(dim: InverseTransform2dDim, len: usize) -> Self {
        use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
        let InverseTransform2dDim::Kernel(kind) = dim else {
            return Self::Identity(identity_scale(len) as i16);
        };
        match (len, kind) {
            (4, Dct) => Self::Four(&DCT4),
            (4, Adst) => Self::Four(&ADST4),
            (4, _) => Self::Four(&FDST4),
            (8, Adst) => Self::Eight(&ADST8),
            (8, Fdst) => Self::Eight(&FDST8),
            (8, Ddtx) => Self::Eight(&DDTX8),
            (8, Fddt) => Self::Eight(&FDDT8),
            (16, Adst) => Self::Sixteen(&ADST16),
            (16, Fdst) => Self::Sixteen(&FDST16),
            (16, Ddtx) => Self::Sixteen(&DDTX16),
            (16, Fddt) => Self::Sixteen(&FDDT16),
            _ => Self::Dct(len),
        }
    }
}

/// `lanes` as a vector.
#[inline]
#[target_feature(enable = "neon")]
fn lanes(t: Neon, lanes: &Lanes) -> int16x8_t {
    i16x8::load_array_ref(t, lanes).into()
}

/// `acc += k[j][0][0..4] * x[l]` for each `(j, l)`.
macro_rules! mac4 {
    ($t:ident, $acc:ident, $k:expr, $x:expr; $($j:literal $l:literal),*) => {
        $(
            $acc = vmlal_laneq_s16::<$l>($acc, vget_low_s16(lanes($t, &$k[$j][0])), $x);
        )*
    };
}

/// `(lo, hi) += k[j0 + l][g] * x[l]` for each lane `l`.
macro_rules! mac8 {
    ($t:ident, $lo:ident, $hi:ident, $k:expr, $j0:literal, $g:literal, $x:expr) => {
        mac8!(@ $t, $lo, $hi, $k, $j0, $g, $x; 0 1 2 3 4 5 6 7)
    };
    (@ $t:ident, $lo:ident, $hi:ident, $k:expr, $j0:literal, $g:literal, $x:expr; $($l:literal)*) => {
        $(
            let kv = lanes($t, &$k[$j0 + $l][$g]);
            $lo = vmlal_laneq_s16::<$l>($lo, vget_low_s16(kv), $x);
            $hi = vmlal_high_laneq_s16::<$l>($hi, kv, $x);
        )*
    };
}

/// The four lanes of `v` reversed.
#[inline]
#[target_feature(enable = "neon")]
fn reverse(v: int32x4_t) -> int32x4_t {
    let pairs = vrev64q_s32(v);
    vextq_s32::<2>(pairs, pairs)
}

/// Two 4-wide rows in the low and high lanes of `x` by the 4-point kernel
/// `k`.
macro_rules! four {
    ($t:ident, $k:expr, $x:expr) => {{
        let (mut a, mut b, k, x) = (vdupq_n_s32(0), vdupq_n_s32(0), $k, $x);
        mac4!($t, a, k, x; 0 0, 1 1, 2 2, 3 3);
        mac4!($t, b, k, x; 0 4, 1 5, 2 6, 3 7);
        [a, b]
    }};
}

/// One 8-wide row `x` by the 8-point kernel `k`.
macro_rules! eight {
    ($t:ident, $k:expr, $x:expr) => {{
        let (mut lo, mut hi, k, x) = (vdupq_n_s32(0), vdupq_n_s32(0), $k, $x);
        mac8!($t, lo, hi, k, 0, 0, x);
        [lo, hi]
    }};
}

/// One 16-wide row `x` by the 16-point kernel `k`.
macro_rules! sixteen {
    ($t:ident, $k:expr, $x:expr) => {{
        let ([mut a, mut b, mut c, mut d], k, [x0, x1]) = ([vdupq_n_s32(0); 4], $k, $x);
        mac8!($t, a, b, k, 0, 0, x0);
        mac8!($t, c, d, k, 0, 1, x0);
        mac8!($t, a, b, k, 8, 0, x1);
        mac8!($t, c, d, k, 8, 1, x1);
        [a, b, c, d]
    }};
}

/// The 8-point `DCT` of `x`: the even lanes by the 4-point `DCT`, the odd
/// lanes by the odd rows.
macro_rules! dct8 {
    ($t:ident, $k:expr, $x:expr) => {{
        let (mut even, mut odd, [x]) = (vdupq_n_s32(0), vdupq_n_s32(0), $x);
        let k: &DctTables = $k;
        mac4!($t, even, k.0, x; 0 0, 1 2, 2 4, 3 6);
        mac4!($t, odd, k.1, x; 0 1, 1 3, 2 5, 3 7);
        [vaddq_s32(even, odd), reverse(vsubq_s32(even, odd))]
    }};
}

/// The 16-point `DCT` of `x`.
macro_rules! dct16 {
    ($t:ident, $k:expr, $x:expr) => {{
        let ([x0, x1], k): ([int16x8_t; 2], &DctTables) = ($x, $k);
        let [e0, e1] = dct8!($t, k, [vuzp1q_s16(x0, x1)]);
        let [o0, o1] = eight!($t, k.2, vuzp2q_s16(x0, x1));
        [
            vaddq_s32(e0, o0),
            vaddq_s32(e1, o1),
            reverse(vsubq_s32(e1, o1)),
            reverse(vsubq_s32(e0, o0)),
        ]
    }};
}

/// The 32-point `DCT` of `x`.
macro_rules! dct32 {
    ($t:ident, $k:expr, $x:expr) => {{
        let ([x0, x1, x2, x3], k): ([int16x8_t; 4], &DctTables) = ($x, $k);
        let even = dct16!($t, k, [vuzp1q_s16(x0, x1), vuzp1q_s16(x2, x3)]);
        let odd = sixteen!($t, k.3, [vuzp2q_s16(x0, x1), vuzp2q_s16(x2, x3)]);
        let mut out = [vdupq_n_s32(0); 8];
        for i in 0..4 {
            out[i] = vaddq_s32(even[i], odd[i]);
            out[7 - i] = reverse(vsubq_s32(even[i], odd[i]));
        }
        out
    }};
}

/// The products of `x` and the identity scale `s`.
#[inline]
#[target_feature(enable = "neon")]
fn scaled<const N: usize, const M: usize>(x: [int16x8_t; N], s: i16) -> [int32x4_t; M] {
    let mut sums = [vdupq_n_s32(0); M];
    for (k, x) in x.into_iter().enumerate() {
        sums[2 * k] = vmull_n_s16(vget_low_s16(x), s);
        sums[2 * k + 1] = vmull_high_n_s16(x, s);
    }
    sums
}

/// `Sat16(Round2(v, shift))` of two vectors, for `shift` holding `-shift`.
#[inline]
#[target_feature(enable = "neon")]
fn narrow(lo: int32x4_t, hi: int32x4_t, shift: int32x4_t) -> int16x8_t {
    vqmovn_high_s32(vqmovn_s32(vrshlq_s32(lo, shift)), vrshlq_s32(hi, shift))
}

/// Half of the scratch: 1024 `i16` in groups of eight.
type Half = [[i32; 4]; 128];

/// The eight `i16` from `i16` index `at` (a multiple of 8) of `buf`.
#[inline]
#[target_feature(enable = "neon")]
fn get16(t: Neon, buf: &Half, at: usize) -> int16x8_t {
    vreinterpretq_s16_s32(i32x4::from_slice(t, &buf[(at / 8) & 127]).into())
}

/// Stores eight `i16` at `i16` index `at` (a multiple of 8) of `buf`.
#[inline]
#[target_feature(enable = "neon")]
fn put16(t: Neon, buf: &mut Half, at: usize, v: int16x8_t) {
    i32x4::<Neon>::simd_from(t, vreinterpretq_s32_s16(v)).store_array(&mut buf[(at / 8) & 127]);
}

/// `tbl` indices that move the last `n` of eight `i32` lanes to lanes
/// `0..n` and zero the rest, for `n` below 8.
static TAIL: [[[u8; 16]; 2]; 8] = {
    let mut out = [[[0xFF; 16]; 2]; 8];
    let mut n = 0;
    while n < 8 {
        let mut byte = 0;
        while byte < 4 * n {
            out[n][byte / 16][byte % 16] = (byte + 4 * (8 - n)) as u8;
            byte += 1;
        }
        n += 1;
    }
    out
};

/// Eight `i32` as two vectors.
#[inline]
#[target_feature(enable = "neon")]
fn split8(t: Neon, v: &[i32; 8]) -> (int32x4_t, int32x4_t) {
    let (a, b) = v.split_at(4);
    (
        i32x4::from_slice(t, a).into(),
        i32x4::from_slice(t, b).into(),
    )
}

/// The coefficients of `coeffs` from `start` (fewer than eight), zero
/// padded to eight lanes.
#[inline]
#[target_feature(enable = "neon")]
fn load_tail(t: Neon, coeffs: &[i32], start: usize) -> (int32x4_t, int32x4_t) {
    let n = coeffs.len().saturating_sub(start).min(7);
    let last = coeffs
        .len()
        .checked_sub(8)
        .and_then(|at| coeffs.get(at..)?.first_chunk::<8>());
    let Some(last) = last else {
        let lanes: [i32; 8] = core::array::from_fn(|i| coeffs.get(start + i).copied().unwrap_or(0));
        return split8(t, &lanes);
    };
    let (a, b) = split8(t, last);
    let table = uint8x16x2_t(vreinterpretq_u8_s32(a), vreinterpretq_u8_s32(b));
    let [lo, hi] = TAIL[n].map(|index| vqtbl2q_u8(table, u8x16::from_slice(t, &index).into()));
    (vreinterpretq_s32_u8(lo), vreinterpretq_s32_u8(hi))
}

/// § 7.14.4 `sign * (Round2(Abs(q) * q2 & 0xFFFFFF, 3) >> denom)`, unclamped,
/// for `q2_8` holding `q2 << 8` and `denom` holding `-denom`: the product
/// by `q2 << 8` wraps to `(Abs(q) * q2 & 0xFFFFFF) << 8`, so one rounding
/// shift by 11 is the masked `Round2(.., 3)`.
#[inline]
#[target_feature(enable = "neon")]
fn dequantize<const DENOM: bool>(v: int32x4_t, q2_8: uint32x4_t, denom: int32x4_t) -> int32x4_t {
    let sign = vshrq_n_s32::<31>(v);
    let product = vmulq_u32(vreinterpretq_u32_s32(vabsq_s32(v)), q2_8);
    let scaled = vrshrq_n_u32::<11>(product);
    let scaled = if DENOM {
        vshlq_u32(scaled, denom)
    } else {
        scaled
    };
    vsubq_s32(veorq_s32(vreinterpretq_s32_u32(scaled), sign), sign)
}

/// Hands chunk `i` of `$coeffs`, its eight inputs `(a, b)` through `$f`,
/// to `$sink(i, v)`, skipping all-zero chunks when `$mode` is
/// `(first, true)`; `$dc` marks chunk 0 when `first`. Returns the chunk
/// count.
macro_rules! load_chunks {
    ($t:ident, $coeffs:expr, $mode:expr, $sink:expr, |$a:ident, $b:ident, $dc:ident| $f:expr) => {{
        let (coeffs, (first, skip)): (&[i32], (bool, bool)) = ($coeffs, $mode);
        let full = coeffs.as_chunks::<8>().0;
        let quiet = |(a, b): (int32x4_t, int32x4_t)| {
            skip && vmaxvq_u32(vreinterpretq_u32_s32(vorrq_s32(a, b))) == 0
        };
        for (i, lanes) in full.iter().enumerate() {
            let (($a, $b), $dc) = (split8($t, lanes), first && i == 0);
            if !quiet(($a, $b)) {
                $sink(i, $f);
            }
        }
        if coeffs.len() % 8 == 0 {
            full.len()
        } else {
            let i = full.len();
            let (($a, $b), $dc) = (load_tail($t, coeffs, 8 * i), first && i == 0);
            if !quiet(($a, $b)) {
                $sink(i, $f);
            }
            i + 1
        }
    }};
}

/// Hands each chunk of eight row inputs of `coeffs` to `sink(i, v)`, with
/// `SKIP` only the chunks with a nonzero coefficient: the § 7.14.4
/// dequantization with its clamp when `dequant` is given, the optional
/// `Round2(x * 2896, 12)` rescale (`sqrdmulh` by `2896 * 8` of the clamped
/// value) and the § 7.15.2.1 row input clamp; an input without `dequant`,
/// the secondary transform output, is already in the clamp. `first` marks
/// chunk 0 as holding the DC coefficient. Returns the chunk count.
#[inline]
#[target_feature(enable = "neon")]
fn load_prefix<const SKIP: bool>(
    t: Neon,
    coeffs: &[i32],
    (dequant, rescale): (Option<FlatDequant>, bool),
    first: bool,
    mut sink: impl FnMut(usize, int16x8_t),
) -> usize {
    let first = (first, SKIP);
    let narrow = |a, b| vqmovn_high_s32(vqmovn_s32(a), b);
    let Some(dq) = dequant else {
        return if rescale {
            load_chunks!(t, coeffs, first, sink, |a, b, _dc| {
                vqrdmulhq_n_s16(narrow(a, b), 2896 * 8)
            })
        } else {
            load_chunks!(t, coeffs, first, sink, |a, b, _dc| narrow(a, b))
        };
    };
    let ac = vdupq_n_u32(dq.ac << 8);
    let dc = vsetq_lane_u32::<0>(dq.dc << 8, ac);
    let denom = vdupq_n_s32(-(dq.denom_shift as i32));
    macro_rules! dequant {
        ($denom:literal, $a:ident, $b:ident, $first:ident) => {{
            let a = dequantize::<$denom>($a, if $first { dc } else { ac }, denom);
            narrow(a, dequantize::<$denom>($b, ac, denom))
        }};
    }
    match (rescale, dq.denom_shift != 0) {
        (true, true) => load_chunks!(t, coeffs, first, sink, |a, b, dc| {
            vqrdmulhq_n_s16(dequant!(true, a, b, dc), 2896 * 8)
        }),
        (true, false) => load_chunks!(t, coeffs, first, sink, |a, b, dc| {
            vqrdmulhq_n_s16(dequant!(false, a, b, dc), 2896 * 8)
        }),
        (false, true) => load_chunks!(t, coeffs, first, sink, |a, b, dc| dequant!(true, a, b, dc)),
        (false, false) => {
            load_chunks!(t, coeffs, first, sink, |a, b, dc| dequant!(false, a, b, dc))
        }
    }
}

/// The row inputs of `coeffs` (rows `in_w` apart) as `i16` rows of `w` in
/// `out` for the first `coded` rows, as [`load_prefix`] gives them.
#[inline]
#[target_feature(enable = "neon")]
fn load_rows(
    t: Neon,
    (coeffs, in_w): (&[i32], usize),
    load: (Option<FlatDequant>, bool),
    (coded, w): (usize, usize),
    out: &mut Half,
) {
    if in_w == w {
        let chunks = load_prefix::<false>(t, coeffs, load, true, |i, v| put16(t, out, 8 * i, v));
        for i in chunks..(coded * w).div_ceil(8) {
            put16(t, out, 8 * i, vdupq_n_s16(0));
        }
        return;
    }
    for r in 0..coded {
        let row = coeffs.get(r * in_w..).unwrap_or_default();
        let row = row.get(..in_w).unwrap_or(row);
        let chunks =
            load_prefix::<false>(t, row, load, r == 0, |i, v| put16(t, out, r * w + 8 * i, v));
        for i in chunks..w / 8 {
            put16(t, out, r * w + 8 * i, vdupq_n_s16(0));
        }
    }
}

/// `Sat16(Round2(v * scale, shift))`.
#[inline]
#[target_feature(enable = "neon")]
fn scale(v: int16x8_t, scale: i16, shift: int32x4_t) -> int16x8_t {
    narrow(
        vmull_n_s16(vget_low_s16(v), scale),
        vmull_high_n_s16(v, scale),
        shift,
    )
}

/// Whether any lane of the vectors of `x` from `from` is nonzero.
#[inline]
#[target_feature(enable = "neon")]
fn nonzero_from<const N: usize>(x: &[int16x8_t; N], from: usize) -> bool {
    let any = x
        .iter()
        .skip(from)
        .fold(vdupq_n_s16(0), |a, &v| vorrq_s16(a, v));
    from < N && vmaxvq_u16(vreinterpretq_u16_s16(any)) != 0
}

/// The row pass of `$kernel` over the `$n` input vectors `$x` of each of
/// the first `coded` rows of `$rows`, each output vector `k` of row `r`
/// handed to `$put(r, k, v)`. Inputs past a row's last nonzero vector are
/// zero: each expansion of `$kernel` sees them as literal zero vectors, so
/// their products fold away.
macro_rules! row_loop {
    ($t:ident, $geometry:expr, $rows:expr, $put:expr, |$x:ident: $n:literal| $kernel:expr) => {{
        let (coded, w, shift): (usize, usize, int32x4_t) = $geometry;
        let rows: &Half = $rows;
        let step = if w == 4 { 2 } else { 1 };
        for r in (0..coded).step_by(step) {
            let x: [int16x8_t; $n] = core::array::from_fn(|k| get16($t, rows, r * w + 8 * k));
            let zero = vdupq_n_s16(0);
            let sums: [int32x4_t; 2 * $n] = if nonzero_from(&x, 2) {
                let $x = x;
                $kernel
            } else if nonzero_from(&x, 1) {
                let $x: [int16x8_t; $n] = core::array::from_fn(|k| if k < 2 { x[k] } else { zero });
                $kernel
            } else {
                let $x: [int16x8_t; $n] = core::array::from_fn(|k| if k < 1 { x[k] } else { zero });
                $kernel
            };
            for k in 0..$n {
                $put(r, k, narrow(sums[2 * k], sums[2 * k + 1], shift));
            }
        }
    }};
}

/// The row pass of `pass` over the first `coded` rows of `rows`, each output
/// vector `k` of row `r` handed to `put(r, k, v)`.
#[inline]
#[target_feature(enable = "neon")]
fn row_pass(
    t: Neon,
    pass: Pass,
    geometry: (usize, usize, int32x4_t),
    rows: &Half,
    mut put: impl FnMut(usize, usize, int16x8_t),
) {
    match (pass, geometry.1) {
        (Pass::Identity(s), 4 | 8) => row_loop!(t, geometry, rows, put, |x: 1| scaled(x, s)),
        (Pass::Identity(s), 16) => row_loop!(t, geometry, rows, put, |x: 2| scaled(x, s)),
        (Pass::Identity(s), _) => row_loop!(t, geometry, rows, put, |x: 4| scaled(x, s)),
        (Pass::Four(k), _) => row_loop!(t, geometry, rows, put, |x: 1| four!(t, k, x[0])),
        (Pass::Eight(k), _) => row_loop!(t, geometry, rows, put, |x: 1| eight!(t, k, x[0])),
        (Pass::Sixteen(k), _) => row_loop!(t, geometry, rows, put, |x: 2| sixteen!(t, k, x)),
        (Pass::Dct(8), _) => row_loop!(t, geometry, rows, put, |x: 1| dct8!(t, &DCT_TABLES, x)),
        (Pass::Dct(16), _) => {
            row_loop!(t, geometry, rows, put, |x: 2| dct16!(t, &DCT_TABLES, x));
        }
        (Pass::Dct(_), _) => row_loop!(t, geometry, rows, put, |x: 4| dct32!(t, tables(), x)),
    }
}

/// `Clip1(p + v)` of the eight bytes from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn add8(dst: &mut [u8], at: usize, v: int16x8_t) {
    if let Some(out) = dst.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<8>) {
        *out = add_bytes(u64::from_le_bytes(*out), v).to_le_bytes();
    }
}

/// `Clip1(p + v)` of the four bytes from `at` (low lanes) and from
/// `at + stride` (high lanes).
#[inline]
#[target_feature(enable = "neon")]
fn add4x2(dst: &mut [u8], at: usize, stride: usize, v: int16x8_t) {
    let word = |dst: &[u8], at: usize| {
        dst.get(at..)
            .and_then(<[u8]>::first_chunk::<4>)
            .map_or(0, |b| u32::from_le_bytes(*b))
    };
    let pred = u64::from(word(dst, at)) | u64::from(word(dst, at + stride)) << 32;
    let sum = add_bytes(pred, v);
    for (at, half) in [(at, sum as u32), (at + stride, (sum >> 32) as u32)] {
        if let Some(out) = dst.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<4>) {
            *out = half.to_le_bytes();
        }
    }
}

/// `Clip1(p + v)` of the eight bytes of `pred`.
#[inline]
#[target_feature(enable = "neon")]
fn add_bytes(pred: u64, v: int16x8_t) -> u64 {
    let pred = vreinterpretq_s16_u16(vmovl_u8(vcreate_u8(pred)));
    vget_lane_u64::<0>(vreinterpret_u64_u8(vqmovun_s16(vqaddq_s16(v, pred))))
}

/// The row outputs of a block and its destination.
struct Columns<'a> {
    /// The `i16` row outputs, rows `w` apart.
    rows: &'a Half,
    w: usize,
    /// Rows of `rows` to read, even.
    coded: usize,
    /// `-colShift`.
    shift: int32x4_t,
    dst: &'a mut [u8],
    stride: usize,
}

impl Columns<'_> {
    /// Row `r`, columns `8 * c..8 * c + 8` (rows `r` and `r + 1` of a
    /// 4-wide block).
    #[inline]
    #[target_feature(enable = "neon")]
    fn row(&self, t: Neon, r: usize, c: usize) -> int16x8_t {
        get16(t, self.rows, r * self.w + 8 * c)
    }

    /// Adds output row `o` of column chunk `c` from its sums.
    #[inline]
    #[target_feature(enable = "neon")]
    fn put(&mut self, o: usize, c: usize, [lo, hi]: [int32x4_t; 2]) {
        add8(
            self.dst,
            o * self.stride + 8 * c,
            narrow(lo, hi, self.shift),
        );
    }

    /// Adds output rows `o` and `o + 1` of a 4-wide block from their sums.
    #[inline]
    #[target_feature(enable = "neon")]
    fn put4(&mut self, o: usize, a: int32x4_t, b: int32x4_t) {
        add4x2(
            self.dst,
            o * self.stride,
            self.stride,
            narrow(a, b, self.shift),
        );
    }
}

/// `acc[o] += y * kv[l]` for each pair `o l`, both halves of `y`.
macro_rules! col_mac {
    ($acc:ident, $y:expr, $kv:expr; $($o:literal $l:literal)*) => {
        $(
            $acc[$o][0] = vmlal_laneq_s16::<$l>($acc[$o][0], vget_low_s16($y), $kv);
            $acc[$o][1] = vmlal_high_laneq_s16::<$l>($acc[$o][1], $y, $kv);
        )*
    };
}

/// `a[o] += low(y) * k0[l0]` and `b[o] += high(y) * k1[l1]` for each `o l0 l1`.
macro_rules! col_mac4 {
    ($a:ident, $b:ident, $y:expr, $k0:expr, $k1:expr; $($o:literal $l0:literal $l1:literal)*) => {
        $(
            $a[$o] = vmlal_laneq_s16::<$l0>($a[$o], vget_low_s16($y), $k0);
            $b[$o] = vmlal_high_laneq_s16::<$l1>($b[$o], $y, $k1);
        )*
    };
}

/// The § 7.15.2.3 identity column pass of `scale`.
#[inline]
#[target_feature(enable = "neon")]
fn col_identity(t: Neon, scale: i16, cols: &mut Columns<'_>) {
    if cols.w == 4 {
        for r in (0..cols.coded).step_by(2) {
            let y = cols.row(t, r, 0);
            cols.put4(
                r,
                vmull_n_s16(vget_low_s16(y), scale),
                vmull_high_n_s16(y, scale),
            );
        }
        return;
    }
    for r in 0..cols.coded {
        for c in 0..cols.w / 8 {
            let y = cols.row(t, r, c);
            cols.put(
                r,
                c,
                [
                    vmull_n_s16(vget_low_s16(y), scale),
                    vmull_high_n_s16(y, scale),
                ],
            );
        }
    }
}

/// The column pass of an `M`-point kernel `k` other than a split `DCT`.
#[inline]
#[target_feature(enable = "neon")]
fn col_matrix<const M: usize, const G: usize>(
    t: Neon,
    k: &[[Lanes; G]; M],
    cols: &mut Columns<'_>,
) {
    let zero = vdupq_n_s32(0);
    let kernel = |r: usize, g: usize| lanes(t, &k[r % M][g % G]);
    if cols.w == 4 {
        for g in 0..G {
            let (mut upper, mut lower) = ([zero; 8], [zero; 8]);
            for r in (0..cols.coded).step_by(2) {
                let (y, k0, k1) = (cols.row(t, r, 0), kernel(r, g), kernel(r + 1, g));
                if M == 4 {
                    col_mac4!(upper, lower, y, k0, k1; 0 0 0 1 1 1 2 2 2 3 3 3);
                } else {
                    col_mac4!(upper, lower, y, k0, k1; 0 0 0 1 1 1 2 2 2 3 3 3 4 4 4 5 5 5 6 6 6 7 7 7);
                }
            }
            for o in (0..M.min(8)).step_by(2) {
                let sum = |i: usize| vaddq_s32(upper[i], lower[i]);
                cols.put4(8 * g + o, sum(o), sum(o + 1));
            }
        }
        return;
    }
    for c in 0..cols.w / 8 {
        for g in 0..G {
            let mut acc = [[zero; 2]; 8];
            for r in 0..cols.coded {
                let (y, kv) = (cols.row(t, r, c), kernel(r, g));
                if M == 4 {
                    col_mac!(acc, y, kv; 0 0 1 1 2 2 3 3);
                } else {
                    col_mac!(acc, y, kv; 0 0 1 1 2 2 3 3 4 4 5 5 6 6 7 7);
                }
            }
            for (o, &acc) in acc.iter().enumerate().take(M.min(8)) {
                cols.put(8 * g + o, c, acc);
            }
        }
    }
}

/// The column pass of the `M`-point `DCT` by its row pairs `k`: the even
/// rows sum `E` and the odd rows `O` over the first `M / 2` outputs, and
/// output `M - 1 - o` is `E - O` where output `o` is `E + O`.
#[inline]
#[target_feature(enable = "neon")]
fn col_dct<const M: usize, const H: usize, const G: usize>(
    t: Neon,
    k: &[[Lanes; G]; H],
    cols: &mut Columns<'_>,
) {
    let zero = vdupq_n_s32(0);
    let kernel = |r: usize, g: usize| lanes(t, &k[(r / 2) % H][g % G]);
    if cols.w == 4 {
        for g in 0..G {
            let (mut even, mut odd) = ([zero; 4], [zero; 4]);
            for r in (0..cols.coded).step_by(2) {
                let (y, kv) = (cols.row(t, r, 0), kernel(r, g));
                col_mac4!(even, odd, y, kv, kv; 0 0 4 1 1 5 2 2 6 3 3 7);
            }
            let sum = |i: usize| vaddq_s32(even[i], odd[i]);
            let diff = |i: usize| vsubq_s32(even[i], odd[i]);
            for o in [0, 2] {
                cols.put4(4 * g + o, sum(o), sum(o + 1));
                cols.put4(M - 2 - 4 * g - o, diff(o + 1), diff(o));
            }
        }
        return;
    }
    for c in 0..cols.w / 8 {
        for g in 0..G {
            let (mut even, mut odd) = ([[zero; 2]; 4], [[zero; 2]; 4]);
            for r in (0..cols.coded).step_by(2) {
                let kv = kernel(r, g);
                col_mac!(even, cols.row(t, r, c), kv; 0 0 1 1 2 2 3 3);
                col_mac!(odd, cols.row(t, r + 1, c), kv; 0 4 1 5 2 6 3 7);
            }
            for (o, (e, d)) in even.iter().zip(&odd).enumerate() {
                cols.put(4 * g + o, c, [vaddq_s32(e[0], d[0]), vaddq_s32(e[1], d[1])]);
                cols.put(
                    M - 1 - 4 * g - o,
                    c,
                    [vsubq_s32(e[0], d[0]), vsubq_s32(e[1], d[1])],
                );
            }
        }
    }
}

/// The column pass of the `M`-point `DCT`, `M >= 16`, over a block at
/// least 8 wide, split twice: rows `0 mod 4` sum `EE` and rows `2 mod 4`
/// sum `EO` over the first `M / 4` outputs, giving the even-row sums `E`
/// over the first `M / 2` (`EE + EO`, then `EE - EO` reversed), which
/// combine with the odd-row sums `O` as in [`col_dct`]. `even` and `odd`
/// are the [`quads`] of the kernel.
#[allow(
    clippy::needless_range_loop,
    reason = "the group index also places the outputs"
)]
#[inline]
#[target_feature(enable = "neon")]
fn col_dct2<const M: usize, const Q: usize, const GE: usize, const GO: usize>(
    t: Neon,
    (even, odd): (&[[Lanes; GE]; Q], &[[Lanes; GO]; Q]),
    cols: &mut Columns<'_>,
) {
    let zero = vdupq_n_s32(0);
    let quads = cols.coded.div_ceil(4);
    for c in 0..cols.w / 8 {
        let mut e = [[zero; 2]; 16];
        for g in 0..GE {
            let (mut ee, mut eo) = ([[zero; 2]; 4], [[zero; 2]; 4]);
            for q in 0..quads {
                let kv = lanes(t, &even[q % Q][g]);
                col_mac!(ee, cols.row(t, 4 * q, c), kv; 0 0 1 1 2 2 3 3);
                col_mac!(eo, cols.row(t, 4 * q + 2, c), kv; 0 4 1 5 2 6 3 7);
            }
            for o in 0..4 {
                let (a, b) = (ee[o], eo[o]);
                e[(4 * g + o) % 16] = [vaddq_s32(a[0], b[0]), vaddq_s32(a[1], b[1])];
                e[(M / 2 - 1 - 4 * g - o) % 16] = [vsubq_s32(a[0], b[0]), vsubq_s32(a[1], b[1])];
            }
        }
        for g in 0..GO {
            let mut o4 = [[zero; 2]; 4];
            let mut o4b = [[zero; 2]; 4];
            for q in 0..quads {
                let kv = lanes(t, &odd[q % Q][g]);
                col_mac!(o4, cols.row(t, 4 * q + 1, c), kv; 0 0 1 1 2 2 3 3);
                col_mac!(o4b, cols.row(t, 4 * q + 3, c), kv; 0 4 1 5 2 6 3 7);
            }
            for o in 0..4 {
                let i = 4 * g + o;
                let ev = e[i % 16];
                let d = [0, 1].map(|half| vaddq_s32(o4[o][half], o4b[o][half]));
                cols.put(i, c, [vaddq_s32(ev[0], d[0]), vaddq_s32(ev[1], d[1])]);
                cols.put(
                    M - 1 - i,
                    c,
                    [vsubq_s32(ev[0], d[0]), vsubq_s32(ev[1], d[1])],
                );
            }
        }
    }
}

/// The column pass of `pass`.
#[inline]
#[target_feature(enable = "neon")]
fn col_pass(t: Neon, pass: Pass, cols: &mut Columns<'_>) {
    match pass {
        Pass::Identity(scale) => col_identity(t, scale, cols),
        Pass::Four(k) => col_matrix(t, k, cols),
        Pass::Eight(k) => col_matrix(t, k, cols),
        Pass::Sixteen(k) => col_matrix(t, k, cols),
        Pass::Dct(8) => col_dct::<8, 4, 1>(t, &COL8, cols),
        Pass::Dct(16) if cols.w == 4 => col_dct::<16, 8, 2>(t, &COL16, cols),
        Pass::Dct(16) => col_dct2::<16, 4, 1, 2>(t, (&EVEN16, &ODD16C), cols),
        Pass::Dct(_) if cols.w == 4 => col_dct::<32, 16, 4>(t, &COL32, cols),
        Pass::Dct(_) => col_dct2::<32, 8, 2, 4>(t, (&EVEN32, &ODD32C), cols),
    }
}

/// Adds the constant `residual` to the `w x h` block at the start of `dst`.
#[inline]
#[target_feature(enable = "neon")]
fn add_constant(t: Neon, dst: &mut [u8], (w, h, stride): (usize, usize, usize), residual: i32) {
    let (up, down) = (
        vdupq_n_u8(residual.clamp(0, 255) as u8),
        vdupq_n_u8((-residual).clamp(0, 255) as u8),
    );
    let apply = |v: uint8x16_t| vqsubq_u8(vqaddq_u8(v, up), down);
    let apply8 = |bytes: u64| {
        let sum = apply(vcombine_u8(vcreate_u8(bytes), vcreate_u8(0)));
        vgetq_lane_u64::<0>(vreinterpretq_u64_u8(sum))
    };
    for row in 0..h {
        let rest = dst.get_mut(row * stride..).unwrap_or_default();
        if w == 4 {
            if let Some(out) = rest.first_chunk_mut::<4>() {
                *out = (apply8(u64::from(u32::from_le_bytes(*out))) as u32).to_le_bytes();
            }
        } else if w == 8 {
            if let Some(out) = rest.first_chunk_mut::<8>() {
                *out = apply8(u64::from_le_bytes(*out)).to_le_bytes();
            }
        } else {
            for out in rest.as_chunks_mut::<16>().0.iter_mut().take(w / 16) {
                let sum = apply(u8x16::from_slice(t, out).into());
                u8x16::<Neon>::simd_from(t, sum).store_array(out);
            }
        }
    }
}

/// An 8x8 block with its rows in registers: no scratch and no loops over
/// a runtime size.
#[inline]
#[target_feature(enable = "neon")]
fn block8(
    t: Neon,
    (row, col): (Pass, Pass),
    (coeffs, load): (&[i32], (Option<FlatDequant>, bool)),
    (row_shift, col_shift): (int32x4_t, int32x4_t),
    (dst, stride): (&mut [u8], usize),
) {
    let zero = vdupq_n_s16(0);
    let mut inputs = [zero; 8];
    let coded = load_prefix::<false>(t, coeffs, load, true, |i, v| inputs[i % 8] = v);
    macro_rules! rows {
        (|$x:ident| $kernel:expr) => {
            core::array::from_fn(|r| {
                if r < coded {
                    let $x = inputs[r];
                    let [lo, hi]: [int32x4_t; 2] = $kernel;
                    narrow(lo, hi, row_shift)
                } else {
                    zero
                }
            })
        };
    }
    let y: [int16x8_t; 8] = match row {
        Pass::Identity(s) => rows!(|x| [vmull_n_s16(vget_low_s16(x), s), vmull_high_n_s16(x, s)]),
        Pass::Eight(k) => rows!(|x| eight!(t, k, x)),
        _ => rows!(|x| dct8!(t, &DCT_TABLES, [x])),
    };
    let put = |dst: &mut [u8], o: usize, [lo, hi]: [int32x4_t; 2]| {
        add8(dst, o * stride, narrow(lo, hi, col_shift));
    };
    let zero32 = vdupq_n_s32(0);
    match col {
        Pass::Identity(s) => {
            for (r, &y) in y.iter().enumerate().take(coded) {
                add8(dst, r * stride, scale(y, s, col_shift));
            }
        }
        Pass::Eight(k) => {
            let mut acc = [[zero32; 2]; 8];
            for (r, &y) in y.iter().enumerate() {
                let kv = lanes(t, &k[r][0]);
                col_mac!(acc, y, kv; 0 0 1 1 2 2 3 3 4 4 5 5 6 6 7 7);
            }
            for (o, &acc) in acc.iter().enumerate() {
                put(dst, o, acc);
            }
        }
        _ => {
            let (mut even, mut odd) = ([[zero32; 2]; 4], [[zero32; 2]; 4]);
            for (p, pair) in y.as_chunks::<2>().0.iter().enumerate() {
                let kv = lanes(t, &COL8[p][0]);
                col_mac!(even, pair[0], kv; 0 0 1 1 2 2 3 3);
                col_mac!(odd, pair[1], kv; 0 4 1 5 2 6 3 7);
            }
            for (o, (e, d)) in even.iter().zip(&odd).enumerate() {
                put(dst, o, [vaddq_s32(e[0], d[0]), vaddq_s32(e[1], d[1])]);
                put(dst, 7 - o, [vsubq_s32(e[0], d[0]), vsubq_s32(e[1], d[1])]);
            }
        }
    }
}

/// [`add`] of a block with more than a DC coefficient or another type than
/// `DCT_DCT`. With identity rows and columns each output depends on its own
/// coefficient only, so all-zero chunks add nothing.
#[inline(never)]
#[target_feature(enable = "neon")]
fn transform(
    t: Neon,
    shape: InverseTransformAdd,
    (coeffs, in_w, dequant): (&[i32], usize, Option<FlatDequant>),
    (dst, stride): (&mut [u8], usize),
    scratch: &mut [i32; 32 * 32],
) {
    let (w, h) = (shape.w(), shape.h());
    let Some((inputs, rest)) = scratch
        .as_chunks_mut::<4>()
        .0
        .split_first_chunk_mut::<128>()
    else {
        return;
    };
    let Some(rows) = rest.first_chunk_mut::<128>() else {
        return;
    };
    let row_shift = vdupq_n_s32(-i32::from(shape.row_shift));
    let col_shift = vdupq_n_s32(-i32::from(shape.col_shift));
    let (row, col) = (Pass::of(shape.row, w), Pass::of(shape.col, h));
    let add = |dst: &mut [u8], at: usize, v: int16x8_t| {
        if w == 4 {
            add4x2(dst, at / 4 * stride, stride, v);
        } else {
            add8(dst, at / w * stride + at % w, v);
        }
    };
    if let (Pass::Identity(row_scale), Pass::Identity(col_scale), true) = (row, col, in_w == w) {
        let load = (dequant, shape.rescale);
        load_prefix::<true>(t, coeffs, load, true, |i, v| {
            add(
                dst,
                8 * i,
                scale(scale(v, row_scale, row_shift), col_scale, col_shift),
            );
        });
        return;
    }
    if (w, h, in_w) == (8, 8, 8) {
        let shifts = (row_shift, col_shift);
        block8(
            t,
            (row, col),
            (coeffs, (dequant, shape.rescale)),
            shifts,
            (dst, stride),
        );
        return;
    }
    let coded = coeffs.len().div_ceil(in_w).min(h);
    let coded = if w == 4 {
        coded.next_multiple_of(2)
    } else {
        coded
    };
    load_rows(
        t,
        (coeffs, in_w),
        (dequant, shape.rescale),
        (coded, w),
        inputs,
    );
    let geometry = (coded, w, row_shift);
    if let Pass::Identity(col_scale) = col {
        row_pass(t, row, geometry, inputs, |r, k, y| {
            add(dst, r * w + 8 * k, scale(y, col_scale, col_shift));
        });
        return;
    }
    row_pass(t, row, geometry, inputs, |r, k, y| {
        put16(t, rows, r * w + 8 * k, y);
    });
    for r in coded..coded.next_multiple_of(4).min(h) {
        for k in 0..w / 8 {
            put16(t, rows, r * w + 8 * k, vdupq_n_s16(0));
        }
    }
    let cols = &mut Columns {
        rows,
        w,
        coded: coded.next_multiple_of(2).min(h),
        shift: col_shift,
        dst,
        stride,
    };
    col_pass(t, col, cols);
}

/// [`add`] of a `DCT_DCT` block whose only coefficient is `dc`: one
/// constant residual.
#[inline(never)]
#[target_feature(enable = "neon")]
fn flat(
    t: Neon,
    shape: InverseTransformAdd,
    (dc, dequant): (i32, Option<FlatDequant>),
    (dst, stride): (&mut [u8], usize),
) {
    let x = vdupq_n_s32(dc);
    let x = match dequant {
        Some(dq) => {
            let denom = vdupq_n_s32(-(dq.denom_shift as i32));
            dequantize::<true>(x, vdupq_n_u32(dq.dc << 8), denom)
        }
        None => x,
    };
    let x = vqmovn_s32(x);
    let x = if shape.rescale {
        vqrdmulh_n_s16(x, 2896 * 8)
    } else {
        x
    };
    let shift = |s: u8| vdupq_n_s32(-i32::from(s));
    let row = vqmovn_s32(vrshlq_s32(vmull_n_s16(x, 64), shift(shape.row_shift)));
    let col = vqmovn_s32(vrshlq_s32(vmull_n_s16(row, 64), shift(shape.col_shift)));
    let r = i32::from(vget_lane_s16::<0>(col)).clamp(-256, 255);
    add_constant(t, dst, (shape.w(), shape.h(), stride), r);
}

fearless_simd::kernel!(
    /// Adds the transform of the coefficients (rows `in_w` apart) to the
    /// 8-bit block at the start of `dst`, as [`InverseTransformAdd::add`]
    /// does; the caller checked the shape, the lengths and that the block
    /// fits.
    #[inline(never)]
    pub(super) fn add(
        t: Neon,
        shape: InverseTransformAdd,
        input: (&[i32], usize, Option<FlatDequant>),
        target: (&mut [u8], usize),
        scratch: &mut [i32; 32 * 32],
    ) {
        match input {
            ([], ..) => {}
            (&[dc], _, dequant) if shape.flat => flat(t, shape, (dc, dequant), target),
            _ => transform(t, shape, input, target, scratch),
        }
    }
);
