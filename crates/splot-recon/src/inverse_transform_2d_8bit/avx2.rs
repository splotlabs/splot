// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AVX2 form of the 8-bit § 7.15.4.1 transform.
//!
//! The row pass computes the rows of one column pair together and stores them
//! as the column pass's `pmaddwd` operands: column `n` of both rows as one
//! `i16` pair. A row's lanes are its outputs, so a `DCT` row splits once into
//! even and odd inputs. A column's lanes are the block's columns, so a `DCT`
//! column factors fully: by `Dct_Kernel2N[2i][j] == Dct_KernelN[i][j]` its
//! even rows give the half-size `DCT` of its first `N` outputs, and by
//! `Dct_KernelN[j][N - 1 - i] == (-1)^j Dct_KernelN[j][i]` the last outputs
//! are the first ones with the odd half negated. Both identities of the § 9.6
//! tables are checked at compile time; every product is a raw input times a
//! table entry, so the sums equal the § 7.15.2.1 matrix sums exactly.
//!
//! The row `Clip3` is the `packssdw` saturation. Before the § 7.14.3 add the
//! column `Clip3` folds into the saturating `i16` add, since
//! `Clip1(p + Clip3(-256, 255, r)) == Clip1(p + Sat16(r))` for 8-bit `p`.

use core::arch::x86_64::*;

use fearless_simd::{Avx2, SimdBase as _, SimdInto as _, i32x4, i32x8};
use splot_tables::tables::transform_1d::{
    ADST_KERNEL4, ADST_KERNEL8, ADST_KERNEL16, DCT_KERNEL4, DCT_KERNEL8, DCT_KERNEL16,
    DCT_KERNEL32, DDTX_KERNEL8, DDTX_KERNEL16, FDST_KERNEL4, FDST_KERNEL8, FDST_KERNEL16,
};

use super::{FlatDequant, InverseTransformAdd};
use crate::inverse_transform::InverseTransform1dType;
use crate::inverse_transform_2d::InverseTransform2dDim;

/// Entry `[q][i]` packs `kernel[2q][i]` (low half) and `kernel[2q + 1][i]`
/// (high half), with the output index reversed for `FDDT`.
const fn pair_kernel<const N: usize, const H: usize>(
    kernel: &[[i32; N]; N],
    reversed: bool,
) -> [[u32; N]; H] {
    assert!(2 * H == N);
    let mut out = [[0u32; N]; H];
    let mut q = 0;
    while q < H {
        let mut i = 0;
        while i < N {
            let column = if reversed { N - 1 - i } else { i };
            let (low, high) = (kernel[2 * q][column], kernel[2 * q + 1][column]);
            assert!(low == low as i16 as i32 && high == high as i16 as i32);
            out[q][i] = (low as u16 as u32) | ((high as u16 as u32) << 16);
            i += 1;
        }
        q += 1;
    }
    out
}

static DCT4: [[u32; 4]; 2] = pair_kernel(&DCT_KERNEL4, false);
static ADST4: [[u32; 4]; 2] = pair_kernel(&ADST_KERNEL4, false);
static FDST4: [[u32; 4]; 2] = pair_kernel(&FDST_KERNEL4, false);
static DCT8: [[u32; 8]; 4] = pair_kernel(&DCT_KERNEL8, false);
static ADST8: [[u32; 8]; 4] = pair_kernel(&ADST_KERNEL8, false);
static FDST8: [[u32; 8]; 4] = pair_kernel(&FDST_KERNEL8, false);
static DDTX8: [[u32; 8]; 4] = pair_kernel(&DDTX_KERNEL8, false);
static FDDT8: [[u32; 8]; 4] = pair_kernel(&DDTX_KERNEL8, true);
static DCT16: [[u32; 16]; 8] = pair_kernel(&DCT_KERNEL16, false);
static ADST16: [[u32; 16]; 8] = pair_kernel(&ADST_KERNEL16, false);
static FDST16: [[u32; 16]; 8] = pair_kernel(&FDST_KERNEL16, false);
static DDTX16: [[u32; 16]; 8] = pair_kernel(&DDTX_KERNEL16, false);
static FDDT16: [[u32; 16]; 8] = pair_kernel(&DDTX_KERNEL16, true);
static DCT32: [[u32; 32]; 16] = pair_kernel(&DCT_KERNEL32, false);
static DCT8_SPLIT: [[u32; 8]; 4] = split_pair_kernel(&DCT_KERNEL8);
static DCT16_SPLIT: [[u32; 16]; 8] = split_pair_kernel(&DCT_KERNEL16);
static DCT32_SPLIT: [[u32; 32]; 16] = split_pair_kernel(&DCT_KERNEL32);

/// [`pair_kernel`] over the row pairs `4m + j, 4m + j + 2` (pair `2m + j`)
/// of a split `DCT` column pass, for the first `N / 2` outputs.
const fn split_pair_kernel<const N: usize, const H: usize>(
    kernel: &[[i32; N]; N],
) -> [[u32; N]; H] {
    assert!(2 * H == N);
    let mut out = [[0u32; N]; H];
    let mut q = 0;
    while q < H {
        let a = 4 * (q / 2) + q % 2;
        let b = a + 2;
        let mut i = 0;
        while i < N / 2 {
            let (low, high) = (kernel[a][i], kernel[b][i]);
            assert!(low == low as i16 as i32 && high == high as i16 as i32);
            assert!(kernel[a][N - 1 - i] == low * (1 - 2 * (a % 2) as i32));
            assert!(kernel[b][N - 1 - i] == high * (1 - 2 * (b % 2) as i32));
            out[q][i] = (low as u16 as u32) | ((high as u16 as u32) << 16);
            i += 1;
        }
        q += 1;
    }
    out
}

/// The `pshufb` that swaps the middle two samples of every four.
const SPLIT_ORDER: [u8; 32] = {
    let mut order = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        let sample = i % 16 / 2;
        let swapped = match sample % 4 {
            1 => sample + 1,
            2 => sample - 1,
            _ => sample,
        };
        order[i] = (2 * swapped + i % 2) as u8;
        i += 1;
    }
    order
};

/// One pass of a fused transform: the § 7.15.2.3 identity or a § 7.15.2.1
/// kernel at its length, with the paired kernel the scalar selection picks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Pass {
    Identity4,
    Identity8,
    Identity16,
    Identity32,
    Dct4,
    Adst4,
    Fdst4,
    Dct8,
    Adst8,
    Fdst8,
    Ddtx8,
    Fddt8,
    Dct16,
    Adst16,
    Fdst16,
    Ddtx16,
    Fddt16,
    Dct32,
}

impl Pass {
    /// The pass of `dim` at length `len`.
    pub(super) const fn of(dim: InverseTransform2dDim, len: usize) -> Option<Self> {
        use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
        use InverseTransform2dDim::{Identity, Kernel};
        Some(match (dim, len) {
            (Identity, 4) => Self::Identity4,
            (Identity, 8) => Self::Identity8,
            (Identity, 16) => Self::Identity16,
            (Identity, 32) => Self::Identity32,
            (Kernel(Dct), 4) => Self::Dct4,
            (Kernel(Adst), 4) => Self::Adst4,
            (Kernel(Fdst | Ddtx | Fddt), 4) => Self::Fdst4,
            (Kernel(Dct), 8) => Self::Dct8,
            (Kernel(Adst), 8) => Self::Adst8,
            (Kernel(Fdst), 8) => Self::Fdst8,
            (Kernel(Ddtx), 8) => Self::Ddtx8,
            (Kernel(Fddt), 8) => Self::Fddt8,
            (Kernel(Dct), 16) => Self::Dct16,
            (Kernel(Adst), 16) => Self::Adst16,
            (Kernel(Fdst), 16) => Self::Fdst16,
            (Kernel(Ddtx), 16) => Self::Ddtx16,
            (Kernel(Fddt), 16) => Self::Fddt16,
            (Kernel(_), 32) => Self::Dct32,
            _ => return None,
        })
    }

    const ALL: [Self; 18] = [
        Self::Identity4,
        Self::Identity8,
        Self::Identity16,
        Self::Identity32,
        Self::Dct4,
        Self::Adst4,
        Self::Fdst4,
        Self::Dct8,
        Self::Adst8,
        Self::Fdst8,
        Self::Ddtx8,
        Self::Fddt8,
        Self::Dct16,
        Self::Adst16,
        Self::Fdst16,
        Self::Ddtx16,
        Self::Fddt16,
        Self::Dct32,
    ];

    /// Whether this is the factored `DCT` of a column pass.
    const fn is_dct(self) -> bool {
        matches!(self, Self::Dct4 | Self::Dct8 | Self::Dct16 | Self::Dct32)
    }
}

/// Where the column pass goes.
pub(super) enum Sink<'a> {
    /// Added in place to the 8-bit block at the start of `dst`.
    Add { dst: &'a mut [u8], stride: usize },
    /// The `w * h` column `Clip3` outputs.
    Residual(&'a mut [i32]),
}

const fn pack(low: i32, high: i32) -> u32 {
    assert!(low == low as i16 as i32 && high == high as i16 as i32);
    (low as u16 as u32) | ((high as u16 as u32) << 16)
}

/// `Dct_KernelN` odd-row pairs `4p + 1, 4p + 3` at `[p]` over outputs
/// `0..N / 2`, checking each odd row's antisymmetry.
const fn odd_pairs<const N: usize, const H: usize, const P: usize>(
    kernel: &[[i32; N]; N],
) -> [[u32; H]; P] {
    assert!(2 * H == N && 4 * P == N);
    let mut out = [[0u32; H]; P];
    let mut p = 0;
    while p < P {
        let mut i = 0;
        while i < H {
            let (a, b) = (kernel[4 * p + 1], kernel[4 * p + 3]);
            assert!(a[N - 1 - i] == -a[i] && b[N - 1 - i] == -b[i]);
            out[p][i] = pack(a[i], b[i]);
            i += 1;
        }
        p += 1;
    }
    out
}

/// `Dct_Kernel2N[2i][j] == Dct_KernelN[i][j]`, and the even rows of
/// `Dct_KernelN` symmetric.
const fn halves<const N: usize, const M: usize>(half: &[[i32; N]; N], full: &[[i32; M]; M]) {
    assert!(2 * N == M);
    let mut i = 0;
    while i < N {
        let mut j = 0;
        while j < N {
            assert!(full[2 * i][j] == half[i][j]);
            if i % 2 == 0 {
                assert!(half[i][N - 1 - j] == half[i][j]);
            }
            j += 1;
        }
        i += 1;
    }
}

const _: () = {
    halves(&DCT_KERNEL4, &DCT_KERNEL8);
    halves(&DCT_KERNEL8, &DCT_KERNEL16);
    halves(&DCT_KERNEL16, &DCT_KERNEL32);
    assert!(DCT_KERNEL4[0][0] == 64 && DCT_KERNEL4[0][1] == 64);
    assert!(DCT_KERNEL4[2][0] == 64 && DCT_KERNEL4[2][1] == -64);
};

static ODD4: [[u32; 2]; 1] = odd_pairs(&DCT_KERNEL4);
static ODD8: [[u32; 4]; 2] = odd_pairs(&DCT_KERNEL8);
static ODD16: [[u32; 8]; 4] = odd_pairs(&DCT_KERNEL16);
static ODD32: [[u32; 16]; 8] = odd_pairs(&DCT_KERNEL32);

/// The row pairs of an `m`-point `DCT` column in the order its factorization
/// reads them: those of the half-size `DCT` on the even rows, then the odd
/// pairs `4p + 1, 4p + 3`.
const fn dct_pairs(m: usize) -> [(u8, u8); 16] {
    let mut out = [(0u8, 1u8); 16];
    let (mut len, mut size) = (1, 2);
    while size < m {
        let mut i = 0;
        while i < len {
            out[i] = (2 * out[i].0, 2 * out[i].1);
            i += 1;
        }
        size *= 2;
        let mut p = 0;
        while p < size / 4 {
            out[len + p] = ((4 * p + 1) as u8, (4 * p + 3) as u8);
            p += 1;
        }
        len += size / 4;
    }
    out
}

static DCT_PAIRS: [[(u8, u8); 16]; 4] = [dct_pairs(4), dct_pairs(8), dct_pairs(16), dct_pairs(32)];

/// Adjacent row pairs `2q, 2q + 1`.
static ADJACENT_PAIRS: [(u8, u8); 16] = {
    let mut out = [(0u8, 0u8); 16];
    let mut q = 0;
    while q < 16 {
        out[q] = (2 * q as u8, 2 * q as u8 + 1);
        q += 1;
    }
    out
};

/// `pshufb` interleaving the `i16` samples `0..4` and `4..8` of a lane, in
/// order or (`reversed`) from the last.
const fn interleave(low_reversed: bool, high_reversed: bool) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        let reversed = if i < 16 { low_reversed } else { high_reversed };
        let pair = (i % 16) / 4;
        let sample = if reversed { 3 - pair } else { pair } + 4 * ((i % 4) / 2);
        out[i] = (2 * sample + i % 2) as u8;
        i += 1;
    }
    out
}

static INTERLEAVE: [u8; 32] = interleave(false, false);
static INTERLEAVE_HIGH_REVERSED: [u8; 32] = interleave(false, true);
static INTERLEAVE_REVERSED: [u8; 32] = interleave(true, true);

#[inline]
#[target_feature(enable = "avx2")]
fn bytes(t: Avx2, order: &[u8; 32]) -> __m256i {
    fearless_simd::u8x32::load_array_ref(t, order).into()
}

#[inline]
#[target_feature(enable = "avx2")]
fn load8(t: Avx2, s: &[i32], at: usize) -> __m256i {
    match s.get(at..).and_then(<[i32]>::first_chunk::<8>) {
        Some(chunk) => i32x8::load_array_ref(t, chunk).into(),
        None => _mm256_setzero_si256(),
    }
}

#[inline]
#[target_feature(enable = "avx2")]
fn load4(t: Avx2, s: &[i32], at: usize) -> __m128i {
    match s.get(at..).and_then(<[i32]>::first_chunk::<4>) {
        Some(chunk) => i32x4::load_array_ref(t, chunk).into(),
        None => _mm_setzero_si128(),
    }
}

#[inline]
#[target_feature(enable = "avx2")]
fn store8(t: Avx2, s: &mut [i32], at: usize, v: __m256i) {
    if let Some(out) = s.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<8>) {
        let v: i32x8<Avx2> = v.simd_into(t);
        v.store_array(out);
    }
}

#[inline]
#[target_feature(enable = "avx2")]
fn store4(t: Avx2, s: &mut [i32], at: usize, v: __m128i) {
    if let Some(out) = s.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>) {
        let v: i32x4<Avx2> = v.simd_into(t);
        v.store_array(out);
    }
}

/// The 128-bit pair weights at `k` in both lanes.
#[inline]
#[target_feature(enable = "avx2")]
fn weights128(t: Avx2, k: &[u32]) -> __m256i {
    let k = k.first_chunk::<4>().map_or([0; 4], |k| k.map(|k| k as i32));
    _mm256_broadcastsi128_si256(i32x4::load_array(t, k).into())
}

#[inline]
#[target_feature(enable = "avx2")]
fn weights(t: Avx2, k: &[u32]) -> __m256i {
    let k = k.first_chunk::<8>().map_or([0; 8], |k| k.map(|k| k as i32));
    i32x8::load_array(t, k).into()
}

#[inline]
#[target_feature(enable = "avx2")]
fn madd_splat(acc: __m256i, v: __m256i, k: u32) -> __m256i {
    _mm256_add_epi32(acc, _mm256_madd_epi16(v, _mm256_set1_epi32(k as i32)))
}

/// `Round2(v, shift)` for the rounding offset `rnd` and the count `shift`.
#[derive(Clone, Copy)]
struct Round {
    rnd: __m256i,
    shift: __m128i,
}

impl Round {
    #[inline]
    #[target_feature(enable = "avx2")]
    fn new(shift: u32) -> Self {
        Self {
            rnd: _mm256_set1_epi32(1 << (shift - 1)),
            shift: _mm_cvtsi32_si128(shift as i32),
        }
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn apply(self, v: __m256i) -> __m256i {
        _mm256_sra_epi32(_mm256_add_epi32(v, self.rnd), self.shift)
    }
}

/// The coefficient prefix with the § 7.14.4 flat dequantization in lanes
/// with `DQ` and the `Round2(x * 2896, 12)` rescale with `RS`.
#[derive(Clone, Copy)]
struct Input<'a, const DQ: bool, const RS: bool> {
    coeffs: &'a [i32],
    dc: __m256i,
    ac: __m256i,
    shift: __m128i,
}

/// Stages the first `count` coefficients of `coeffs` (see [`Input::stage`]).
#[inline(never)]
#[target_feature(enable = "avx2")]
fn stage<const DQ: bool, const RS: bool>(
    t: Avx2,
    coeffs: &[i32],
    dq: &FlatDequant,
    count: usize,
    staged: &mut [i32],
) {
    Input::<DQ, RS>::new(coeffs, dq).stage(t, count, staged);
}

impl<'a, const DQ: bool, const RS: bool> Input<'a, DQ, RS> {
    #[inline]
    #[target_feature(enable = "avx2")]
    fn new(coeffs: &'a [i32], dq: &FlatDequant) -> Self {
        let (dc, ac) = (dq.dc as i32, dq.ac as i32);
        Self {
            coeffs,
            dc: _mm256_setr_epi32(dc, ac, ac, ac, ac, ac, ac, ac),
            ac: _mm256_set1_epi32(ac),
            shift: _mm_cvtsi32_si128(3 + dq.denom_shift as i32),
        }
    }

    /// Four coefficients from `start`, zero past the end of the prefix.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn coeffs4(self, t: Avx2, start: usize) -> __m128i {
        if start + 4 <= self.coeffs.len() {
            return load4(t, self.coeffs, start);
        }
        let mut padded = [0i32; 4];
        for (i, slot) in padded.iter_mut().enumerate().take(3) {
            if let Some(&coeff) = self.coeffs.get(start + i) {
                *slot = coeff;
            }
        }
        i32x4::load_array(t, padded).into()
    }

    /// The row inputs, before the clamp, of the eight coefficients from `at`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn at(self, t: Avx2, at: usize) -> __m256i {
        self.finish(self.coeffs8(t, at), at == 0)
    }

    /// Eight coefficients from `start`, zero past the end of the prefix.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn coeffs8(self, t: Avx2, start: usize) -> __m256i {
        let len = self.coeffs.len();
        if start + 8 <= len {
            return load8(t, self.coeffs, start);
        }
        if start >= len {
            return _mm256_setzero_si256();
        }
        let n = (len - start) as i32;
        let lane = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
        if len >= 8 {
            let tail = load8(t, self.coeffs, len - 8);
            let index = _mm256_add_epi32(lane, _mm256_set1_epi32(8 - n));
            let moved = _mm256_permutevar8x32_epi32(tail, index);
            return _mm256_and_si256(moved, _mm256_cmpgt_epi32(_mm256_set1_epi32(n), lane));
        }
        let mut padded = [0i32; 8];
        for (i, slot) in padded.iter_mut().enumerate().take(7) {
            if let Some(&coeff) = self.coeffs.get(start + i) {
                *slot = coeff;
            }
        }
        i32x8::load_array(t, padded).into()
    }

    /// The § 7.15.2.1 row inputs, before the clamp, of eight coefficients;
    /// `dc` marks lane 0 as the DC coefficient.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn finish(self, mut v: __m256i, dc: bool) -> __m256i {
        if DQ {
            let q = if dc { self.dc } else { self.ac };
            let product = _mm256_and_si256(
                _mm256_mullo_epi32(_mm256_abs_epi32(v), q),
                _mm256_set1_epi32(0x00FF_FFFF),
            );
            let level =
                _mm256_srl_epi32(_mm256_add_epi32(product, _mm256_set1_epi32(4)), self.shift);
            v = _mm256_sign_epi32(level, v);
            if RS {
                v = clamp16(v);
            }
        }
        if RS {
            v = rescale(v);
        }
        v
    }

    /// Writes the clamped § 7.15.2.1 row inputs of the first `count`
    /// coefficients to `staged` as `i16` in raster order, in whole vectors of
    /// 16.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn stage(self, t: Avx2, count: usize, staged: &mut [i32]) {
        for (j, out) in staged
            .chunks_exact_mut(8)
            .take(count.div_ceil(16))
            .enumerate()
        {
            let low = self.finish(self.coeffs8(t, 16 * j), j == 0);
            let high = self.finish(self.coeffs8(t, 16 * j + 8), false);
            let packed = _mm256_permute4x64_epi64::<0xD8>(_mm256_packs_epi32(low, high));
            store8(t, out, 0, packed);
        }
    }
}

/// The `Round2(x * 2896, 12)` row input rescale.
#[inline]
#[target_feature(enable = "avx2")]
fn rescale(v: __m256i) -> __m256i {
    let scaled = _mm256_mullo_epi32(v, _mm256_set1_epi32(2896));
    _mm256_srai_epi32::<12>(_mm256_add_epi32(scaled, _mm256_set1_epi32(1 << 11)))
}

#[inline]
#[target_feature(enable = "avx2")]
fn clamp16(v: __m256i) -> __m256i {
    _mm256_min_epi32(
        _mm256_max_epi32(v, _mm256_set1_epi32(i32::from(i16::MIN))),
        _mm256_set1_epi32(i32::from(i16::MAX)),
    )
}

#[inline]
#[target_feature(enable = "avx2")]
fn is_zero(v: __m256i) -> bool {
    _mm256_testz_si256(v, v) != 0
}

/// The staged `i16` row inputs of an `N`-wide block whose first `coded` rows
/// are staged; later rows are zero.
#[derive(Clone, Copy)]
struct Staged<'a, const N: usize> {
    rows: &'a [i32],
    coded: usize,
}

impl<const N: usize> Staged<'_, N> {
    /// Sixteen inputs of row `r` from column `at`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn x16(self, t: Avx2, r: usize, at: usize) -> __m256i {
        if r < self.coded {
            load8(t, self.rows, (r * N + at) / 2)
        } else {
            _mm256_setzero_si256()
        }
    }

    /// Eight inputs of row `r` from column `at`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn x8(self, t: Avx2, r: usize, at: usize) -> __m128i {
        if r < self.coded {
            load4(t, self.rows, (r * N + at) / 2)
        } else {
            _mm_setzero_si128()
        }
    }
}

/// Packs rows `a` and `b` (four outputs per lane) as eight column pairs in
/// the lane order of `order`.
#[inline]
#[target_feature(enable = "avx2")]
fn pair_columns(t: Avx2, a: __m256i, b: __m256i, order: &[u8; 32]) -> __m256i {
    _mm256_shuffle_epi8(_mm256_packs_epi32(a, b), bytes(t, order))
}

/// Rows `a` (low lane) and `b` (high lane) of four outputs as four column
/// pairs.
#[inline]
#[target_feature(enable = "avx2")]
fn unpack4(v: __m256i) -> __m128i {
    let packed = _mm256_packs_epi32(v, v);
    _mm_unpacklo_epi16(
        _mm256_castsi256_si128(packed),
        _mm256_extracti128_si256::<1>(packed),
    )
}

const IDENTITY: u8 = 0;
const SPLIT: u8 = 1;
const DENSE: u8 = 2;

/// The row pass of a 16- or 32-wide block (`H = N / 2`) over the column `pairs`
/// with the `KIND` transform on the pair `kernel` or the identity `scale`:
/// writes pair `q` at `scratch[q * N..][..N]` and returns the written pairs.
#[inline(never)]
#[target_feature(enable = "avx2")]
#[allow(
    clippy::many_single_char_names,
    reason = "rows a and b of a column pair, as in § 7.15.4.1"
)]
fn row_pass<const N: usize, const H: usize, const KIND: u8>(
    t: Avx2,
    rows: Staged<'_, N>,
    pairs: &[(u8, u8)],
    (shift, scale): (u32, i32),
    kernel: &[[u32; N]; H],
    scratch: &mut [i32],
) -> u32 {
    let round = Round::new(shift);
    let mut mask = 0;
    for (q, &(a, b)) in pairs.iter().enumerate() {
        let (a, b) = (usize::from(a), usize::from(b));
        if a >= rows.coded && b >= rows.coded {
            continue;
        }
        let Some(out) = scratch.get_mut(q * N..q * N + N) else {
            break;
        };
        let written = if KIND == IDENTITY {
            let mut any = _mm256_setzero_si256();
            let scale = _mm256_set1_epi32(scale);
            for c in 0..N / 8 {
                let (xa, xb) = (rows.x8(t, a, 8 * c), rows.x8(t, b, 8 * c));
                any = _mm256_or_si256(any, _mm256_set_m128i(xa, xb));
                let va = round.apply(_mm256_mullo_epi32(_mm256_cvtepi16_epi32(xa), scale));
                let vb = round.apply(_mm256_mullo_epi32(_mm256_cvtepi16_epi32(xb), scale));
                store8(t, out, 8 * c, pair_columns(t, va, vb, &INTERLEAVE));
            }
            !is_zero(any)
        } else {
            row_wide::<N, H>(t, rows, (a, b), kernel, KIND == SPLIT, round, out)
        };
        mask |= u32::from(written) << q;
    }
    mask
}

/// [`row_pass`] of a 4- or 8-wide block straight from the coefficient
/// prefix.
#[inline(never)]
#[target_feature(enable = "avx2")]
#[allow(
    clippy::many_single_char_names,
    reason = "rows a and b of a column pair, as in § 7.15.4.1"
)]
fn row_pass_direct<
    const N: usize,
    const H: usize,
    const KIND: u8,
    const DQ: bool,
    const RS: bool,
>(
    t: Avx2,
    input: Input<'_, DQ, RS>,
    pairs: &[(u8, u8)],
    (shift, scale): (u32, i32),
    kernel: &[[u32; N]; H],
    scratch: &mut [i32],
) -> u32 {
    let round = Round::new(shift);
    let coded = input.coeffs.len().div_ceil(N);
    let mut mask = 0;
    for (q, &(a, b)) in pairs.iter().enumerate() {
        let (a, b) = (usize::from(a), usize::from(b));
        if a >= coded && b >= coded {
            continue;
        }
        let Some(out) = scratch.get_mut(q * N..q * N + N) else {
            break;
        };
        let written = if N == 4 {
            let x = _mm256_set_m128i(input.coeffs4(t, 4 * b), input.coeffs4(t, 4 * a));
            let x = input.finish(x, a == 0);
            if KIND == IDENTITY {
                let v = _mm256_mullo_epi32(clamp16(x), _mm256_set1_epi32(scale));
                store4(t, out, 0, unpack4(round.apply(v)));
                !is_zero(x)
            } else {
                row4(
                    t,
                    _mm256_packs_epi32(x, x),
                    kernel.as_flattened(),
                    round,
                    out,
                )
            }
        } else {
            let (xa, xb) = (input.at(t, 8 * a), input.at(t, 8 * b));
            if KIND == IDENTITY {
                let scale = _mm256_set1_epi32(scale);
                let va = round.apply(_mm256_mullo_epi32(clamp16(xa), scale));
                let vb = round.apply(_mm256_mullo_epi32(clamp16(xb), scale));
                store8(t, out, 0, pair_columns(t, va, vb, &INTERLEAVE));
                !is_zero(_mm256_or_si256(xa, xb))
            } else {
                let x = _mm256_permute4x64_epi64::<0xD8>(_mm256_packs_epi32(xa, xb));
                row8::<KIND>(t, x, kernel.as_flattened(), round, out)
            }
        };
        mask |= u32::from(written) << q;
    }
    mask
}

/// One pair of 4-sample rows, `x` holding row `a` in its low and row `b` in
/// its high lane, on the 4-point pair kernel `k`.
#[inline]
#[target_feature(enable = "avx2")]
fn row4(t: Avx2, x: __m256i, k: &[u32], round: Round, out: &mut [i32]) -> bool {
    if is_zero(x) {
        return false;
    }
    store4(t, out, 0, row4_pairs(t, x, k, round));
    true
}

/// The column pairs of [`row4`].
#[inline]
#[target_feature(enable = "avx2")]
fn row4_pairs(t: Avx2, x: __m256i, k: &[u32], round: Round) -> __m128i {
    let sum = _mm256_add_epi32(
        _mm256_madd_epi16(_mm256_shuffle_epi32::<0x00>(x), weights128(t, k)),
        _mm256_madd_epi16(
            _mm256_shuffle_epi32::<0x55>(x),
            weights128(t, k.get(4..).unwrap_or_default()),
        ),
    );
    unpack4(round.apply(sum))
}

/// One pair of 8-sample rows, `x` holding row `a` in its low and row `b` in
/// its high lane: the split `DCT` on `DCT8_SPLIT` or the paired kernel `k`.
#[inline]
#[target_feature(enable = "avx2")]
fn row8<const KIND: u8>(t: Avx2, x: __m256i, k: &[u32], round: Round, out: &mut [i32]) -> bool {
    if is_zero(x) {
        return false;
    }
    store8(t, out, 0, row8_pairs::<KIND>(t, x, k, round));
    true
}

/// The column pairs of [`row8`].
#[inline]
#[target_feature(enable = "avx2")]
fn row8_pairs<const KIND: u8>(t: Avx2, x: __m256i, k: &[u32], round: Round) -> __m256i {
    let (low, high, order) = if KIND == SPLIT {
        let y = _mm256_shuffle_epi8(x, bytes(t, &SPLIT_ORDER));
        let at = |p: usize| weights128(t, k.get(8 * p..).unwrap_or_default());
        let even = _mm256_add_epi32(
            _mm256_madd_epi16(_mm256_shuffle_epi32::<0x00>(y), at(0)),
            _mm256_madd_epi16(_mm256_shuffle_epi32::<0xAA>(y), at(2)),
        );
        let even = _mm256_add_epi32(even, round.rnd);
        let odd = _mm256_add_epi32(
            _mm256_madd_epi16(_mm256_shuffle_epi32::<0x55>(y), at(1)),
            _mm256_madd_epi16(_mm256_shuffle_epi32::<0xFF>(y), at(3)),
        );
        (
            _mm256_add_epi32(even, odd),
            _mm256_sub_epi32(even, odd),
            &INTERLEAVE_HIGH_REVERSED,
        )
    } else {
        let inputs = [
            _mm256_shuffle_epi32::<0x00>(x),
            _mm256_shuffle_epi32::<0x55>(x),
            _mm256_shuffle_epi32::<0xAA>(x),
            _mm256_shuffle_epi32::<0xFF>(x),
        ];
        let mut low = round.rnd;
        let mut high = round.rnd;
        for (q, input) in inputs.into_iter().enumerate() {
            let k = k.get(8 * q..).unwrap_or_default();
            low = _mm256_add_epi32(low, _mm256_madd_epi16(input, weights128(t, k)));
            let k = k.get(4..).unwrap_or_default();
            high = _mm256_add_epi32(high, _mm256_madd_epi16(input, weights128(t, k)));
        }
        (low, high, &INTERLEAVE)
    };
    let shift = |v| _mm256_sra_epi32(v, round.shift);
    let packed = _mm256_packs_epi32(shift(low), shift(high));
    let packed = _mm256_permute4x64_epi64::<0xD8>(packed);
    _mm256_shuffle_epi8(packed, bytes(t, order))
}

/// One pair of `N`-sample rows (`H = N / 2`) on the pair `kernel`, split
/// into even and odd inputs with `split`. Both rows share one pass over the
/// kernel rows up to the last non-zero input pair of either.
#[inline]
#[target_feature(enable = "avx2")]
#[allow(
    clippy::many_single_char_names,
    reason = "rows a and b of a column pair, as in § 7.15.4.1"
)]
#[allow(
    clippy::needless_range_loop,
    reason = "indexed accumulators of both rows stay in registers"
)]
fn row_wide<const N: usize, const H: usize>(
    t: Avx2,
    rows: Staged<'_, N>,
    (a, b): (usize, usize),
    kernel: &[[u32; N]; H],
    split: bool,
    round: Round,
    out: &mut [i32],
) -> bool {
    let chunks = N / 8;
    let mut inputs = [[0u32; 16]; 2];
    let mut nonzero = 0u32;
    for (r, inputs) in [a, b].into_iter().zip(&mut inputs) {
        for (j, chunk) in inputs
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .take(N / 16)
            .enumerate()
        {
            let mut x = rows.x16(t, r, 16 * j);
            if split {
                x = _mm256_shuffle_epi8(x, bytes(t, &SPLIT_ORDER));
            }
            let zero = _mm256_cmpeq_epi32(x, _mm256_setzero_si256());
            nonzero |= (!_mm256_movemask_ps(_mm256_castsi256_ps(zero)) as u32 & 0xff) << (8 * j);
            let x: fearless_simd::u32x8<Avx2> = x.simd_into(t);
            x.store_array(chunk);
        }
    }
    if nonzero == 0 {
        return false;
    }
    let used = (u32::BITS - nonzero.leading_zeros()).div_ceil(2) as usize;
    let [xa, xb] = &inputs;
    let steps = xa
        .as_chunks::<2>()
        .0
        .iter()
        .zip(xb.as_chunks::<2>().0)
        .zip(kernel.as_chunks::<2>().0)
        .take(used);
    let splat = |p: u32| _mm256_set1_epi32(p as i32);
    let shift = |v| _mm256_sra_epi32(v, round.shift);
    let (mut ya, mut yb) = ([_mm256_setzero_si256(); 4], [_mm256_setzero_si256(); 4]);
    if split {
        let (mut even, mut odd) = ([[round.rnd; 2]; 2], [[_mm256_setzero_si256(); 2]; 2]);
        for ((&[a0, a1], &[b0, b1]), [k0, k1]) in steps {
            let (a0, a1, b0, b1) = (splat(a0), splat(a1), splat(b0), splat(b1));
            for c in 0..chunks / 2 {
                let at = |k: &[u32; N]| weights(t, k.get(8 * c..).unwrap_or_default());
                let (w0, w1) = (at(k0), at(k1));
                even[0][c] = _mm256_add_epi32(even[0][c], _mm256_madd_epi16(a0, w0));
                odd[0][c] = _mm256_add_epi32(odd[0][c], _mm256_madd_epi16(a1, w1));
                even[1][c] = _mm256_add_epi32(even[1][c], _mm256_madd_epi16(b0, w0));
                odd[1][c] = _mm256_add_epi32(odd[1][c], _mm256_madd_epi16(b1, w1));
            }
        }
        for (y, (even, odd)) in [&mut ya, &mut yb].into_iter().zip(even.iter().zip(&odd)) {
            for c in 0..chunks / 2 {
                y[c] = shift(_mm256_add_epi32(even[c], odd[c]));
                y[chunks - 1 - c] = shift(_mm256_sub_epi32(even[c], odd[c]));
            }
        }
    } else {
        let mut sums = [[round.rnd; 4]; 2];
        for ((&[a0, a1], &[b0, b1]), [k0, k1]) in steps {
            let (a0, a1, b0, b1) = (splat(a0), splat(a1), splat(b0), splat(b1));
            for c in 0..chunks {
                let at = |k: &[u32; N]| weights(t, k.get(8 * c..).unwrap_or_default());
                let (w0, w1) = (at(k0), at(k1));
                let sa = _mm256_add_epi32(_mm256_madd_epi16(a0, w0), _mm256_madd_epi16(a1, w1));
                let sb = _mm256_add_epi32(_mm256_madd_epi16(b0, w0), _mm256_madd_epi16(b1, w1));
                sums[0][c] = _mm256_add_epi32(sums[0][c], sa);
                sums[1][c] = _mm256_add_epi32(sums[1][c], sb);
            }
        }
        for (y, sums) in [&mut ya, &mut yb].into_iter().zip(&sums) {
            for (y, &sum) in y.iter_mut().zip(sums).take(chunks) {
                *y = shift(sum);
            }
        }
    }
    put_row_pairs::<N>(t, (&ya, &yb), split, out);
    true
}

/// Stores the `N / 8` output chunks of rows `ya` and `yb` as column pairs,
/// those of the second half of a `split` row reversed.
#[inline]
#[target_feature(enable = "avx2")]
fn put_row_pairs<const N: usize>(
    t: Avx2,
    (ya, yb): (&[__m256i; 4], &[__m256i; 4]),
    split: bool,
    out: &mut [i32],
) {
    let chunks = N / 8;
    for c in 0..chunks {
        let packed = if split && c >= chunks / 2 {
            let reversed = pair_columns(t, ya[c], yb[c], &INTERLEAVE_REVERSED);
            _mm256_permute4x64_epi64::<0x4E>(reversed)
        } else {
            pair_columns(t, ya[c], yb[c], &INTERLEAVE)
        };
        store8(t, out, 8 * c, packed);
    }
}

/// The coded rows of a 16- or 32-wide block whose inputs are zero past
/// column 8: the first 8 columns of row `r` at `rows[r]`, with the
/// `Round2(x * 2896, 12)` row input rescale when `rescale`.
#[derive(Clone, Copy)]
struct Rows8<'a> {
    rows: &'a [[i32; 8]],
    rescale: bool,
}

impl Rows8<'_> {
    /// The eight clamped `i16` inputs of row `r`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn x8(self, t: Avx2, r: usize) -> __m128i {
        let Some(row) = self.rows.get(r) else {
            return _mm_setzero_si128();
        };
        let mut v = load8(t, row, 0);
        if self.rescale {
            v = rescale(v);
        }
        _mm_packs_epi32(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v))
    }
}

/// [`row_pass`] of a block whose `rows` hold inputs only in their first 8
/// columns, each row on its four input pairs.
#[inline(never)]
#[target_feature(enable = "avx2")]
#[allow(
    clippy::many_single_char_names,
    reason = "rows a and b of a column pair, as in § 7.15.4.1"
)]
fn row_pass8<const N: usize, const H: usize, const KIND: u8>(
    t: Avx2,
    rows: Rows8<'_>,
    (pairs, shift): (&[(u8, u8)], u32),
    kernel: &[[u32; N]; H],
    scratch: &mut [i32],
) -> u32 {
    let round = Round::new(shift);
    let chunks = N / 8;
    let k = |p: usize, c: usize| weights(t, kernel[p].get(8 * c..).unwrap_or_default());
    let row = |x: __m128i| {
        let x = if KIND == SPLIT {
            _mm_shuffle_epi8(x, _mm256_castsi256_si128(bytes(t, &SPLIT_ORDER)))
        } else {
            x
        };
        let x = _mm256_broadcastsi128_si256(x);
        let p = [
            _mm256_shuffle_epi32::<0x00>(x),
            _mm256_shuffle_epi32::<0x55>(x),
            _mm256_shuffle_epi32::<0xAA>(x),
            _mm256_shuffle_epi32::<0xFF>(x),
        ];
        let madd =
            |acc, i: usize, c: usize| _mm256_add_epi32(acc, _mm256_madd_epi16(p[i], k(i, c)));
        let shift = |v| _mm256_sra_epi32(v, round.shift);
        let mut y = [_mm256_setzero_si256(); 4];
        if KIND == SPLIT {
            for c in 0..chunks / 2 {
                let even = madd(madd(round.rnd, 0, c), 2, c);
                let odd = madd(madd(_mm256_setzero_si256(), 1, c), 3, c);
                y[c] = shift(_mm256_add_epi32(even, odd));
                y[chunks - 1 - c] = shift(_mm256_sub_epi32(even, odd));
            }
        } else {
            for (c, y) in y.iter_mut().enumerate().take(chunks) {
                *y = shift((0..4).fold(round.rnd, |acc, i| madd(acc, i, c)));
            }
        }
        y
    };
    let coded = rows.rows.len();
    let mut mask = 0;
    for (q, &(a, b)) in pairs.iter().enumerate() {
        let (a, b) = (usize::from(a), usize::from(b));
        if a >= coded && b >= coded {
            continue;
        }
        let (xa, xb) = (rows.x8(t, a), rows.x8(t, b));
        if is_zero(_mm256_set_m128i(xa, xb)) {
            continue;
        }
        let Some(out) = scratch.get_mut(q * N..q * N + N) else {
            break;
        };
        let ya = row(xa);
        let yb = if b < coded {
            row(xb)
        } else {
            [_mm256_setzero_si256(); 4]
        };
        put_row_pairs::<N>(t, (&ya, &yb), KIND == SPLIT, out);
        mask |= 1 << q;
    }
    mask
}

/// The row pass of a 16- or 32-wide `shape` over `rows` (see [`Rows8`]);
/// no pairs for an identity row pass, which has no secondary transform.
#[inline]
#[target_feature(enable = "avx2")]
fn rows8_pass(t: Avx2, shape: &InverseTransformAdd, rows: Rows8<'_>, scratch: &mut [i32]) -> u32 {
    let p = (
        pairs_of(shape.passes.1, shape.h()),
        u32::from(shape.row_shift),
    );
    match shape.passes.0 {
        Pass::Dct16 => row_pass8::<16, 8, SPLIT>(t, rows, p, &DCT16_SPLIT, scratch),
        Pass::Adst16 => row_pass8::<16, 8, DENSE>(t, rows, p, &ADST16, scratch),
        Pass::Fdst16 => row_pass8::<16, 8, DENSE>(t, rows, p, &FDST16, scratch),
        Pass::Ddtx16 => row_pass8::<16, 8, DENSE>(t, rows, p, &DDTX16, scratch),
        Pass::Fddt16 => row_pass8::<16, 8, DENSE>(t, rows, p, &FDDT16, scratch),
        Pass::Dct32 => row_pass8::<32, 16, SPLIT>(t, rows, p, &DCT32_SPLIT, scratch),
        _ => 0,
    }
}

/// `sum[n] = init + Σ pair[p] * k[p][n]` over the pairs written in `mask`.
#[inline]
#[target_feature(enable = "avx2")]
fn products<const H: usize, const P: usize>(
    v: &[__m256i],
    mask: u32,
    k: &[[u32; H]; P],
    init: __m256i,
) -> [__m256i; H] {
    let mut sum = [init; H];
    for (p, (&v, k)) in v.iter().zip(k).enumerate() {
        if mask & (1 << p) != 0 {
            for (sum, &k) in sum.iter_mut().zip(k) {
                *sum = madd_splat(*sum, v, k);
            }
        }
    }
    sum
}

/// Outputs `e + o` and, mirrored, `e - o`.
#[inline]
#[target_feature(enable = "avx2")]
fn butterfly<const H: usize, const M: usize>(e: &[__m256i; H], o: &[__m256i; H]) -> [__m256i; M] {
    let mut out = [_mm256_setzero_si256(); M];
    for n in 0..H.min(M) {
        out[n] = _mm256_add_epi32(e[n], o[n]);
        if let Some(mirror) = out.get_mut(M - 1 - n) {
            *mirror = _mm256_sub_epi32(e[n], o[n]);
        }
    }
    out
}

#[inline]
#[target_feature(enable = "avx2")]
fn dct4(v: &[__m256i], mask: u32, rnd: __m256i) -> [__m256i; 4] {
    let even = match v.first() {
        Some(&v) if mask & 1 != 0 => [
            madd_splat(rnd, v, pack(64, 64)),
            madd_splat(rnd, v, pack(64, -64)),
        ],
        _ => [rnd; 2],
    };
    let odd = products(
        v.get(1..).unwrap_or_default(),
        mask >> 1,
        &ODD4,
        _mm256_setzero_si256(),
    );
    butterfly::<2, 4>(&even, &odd)
}

#[inline]
#[target_feature(enable = "avx2")]
fn dct8(v: &[__m256i], mask: u32, rnd: __m256i) -> [__m256i; 8] {
    let odd = products(
        v.get(2..).unwrap_or_default(),
        mask >> 2,
        &ODD8,
        _mm256_setzero_si256(),
    );
    butterfly::<4, 8>(&dct4(v, mask, rnd), &odd)
}

#[inline]
#[target_feature(enable = "avx2")]
fn dct16(v: &[__m256i], mask: u32, rnd: __m256i) -> [__m256i; 16] {
    let odd = products(
        v.get(4..).unwrap_or_default(),
        mask >> 4,
        &ODD16,
        _mm256_setzero_si256(),
    );
    butterfly::<8, 16>(&dct8(v, mask, rnd), &odd)
}

/// Column outputs as `i16` rows of a tile, added after the pass.
const TILE: u8 = 0;
/// Column `Clip3` outputs stored as `i32`.
const RESIDUAL: u8 = 1;
/// Column outputs added straight to the 8-bit block (`W <= 8`).
const DIRECT: u8 = 2;

/// Where the column pass puts the shifted output rows of chunk `c`, by
/// `OUT`: as `i16` rows of the `W`-wide `tile` (two samples per word), clamped
/// into `residual`, or added to the 8-bit block at the start of `dst`.
struct Put<'a, const W: usize, const OUT: u8> {
    tile: &'a mut [i32],
    residual: &'a mut [i32],
    dst: &'a mut [u8],
    stride: usize,
    round: Round,
    c: usize,
}

impl<const W: usize, const OUT: u8> Put<'_, W, OUT> {
    /// Puts output rows `a` and `b` before their `Round2`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn rows(&mut self, t: Avx2, (a, b): (usize, usize), va: __m256i, vb: __m256i) {
        let shift = |v| _mm256_sra_epi32(v, self.round.shift);
        let (va, vb) = (shift(va), shift(vb));
        if OUT == RESIDUAL {
            put_residual::<W>(t, (a, b, self.c), va, vb, self.residual);
            return;
        }
        let packed = _mm256_packs_epi32(va, vb);
        if OUT == DIRECT {
            add_rows::<W>(self.dst, (a * self.stride, b * self.stride), packed);
        } else if W >= 8 {
            let packed = _mm256_permute4x64_epi64::<0xD8>(packed);
            store4(
                t,
                self.tile,
                (a * W + 8 * self.c) / 2,
                _mm256_castsi256_si128(packed),
            );
            store4(
                t,
                self.tile,
                (b * W + 8 * self.c) / 2,
                _mm256_extracti128_si256::<1>(packed),
            );
        } else {
            let low = _mm256_castsi256_si128(packed);
            for (row, word) in [
                (a, _mm_cvtsi128_si64(low)),
                (b, _mm_extract_epi64::<1>(low)),
            ] {
                if let Some(out) = self.tile.get_mut(2 * row..2 * row + 2) {
                    out.copy_from_slice(&[word as i32, (word >> 32) as i32]); // splot-copy-ok: one 4-sample row
                }
            }
        }
    }
}

/// The first `2 * j` of `len` pairs that hold every written pair of `mask`.
#[inline]
fn used_pairs(mask: u32, len: usize) -> usize {
    ((u32::BITS - mask.leading_zeros()) as usize)
        .next_multiple_of(2)
        .min(len)
}

/// Puts rows `n` and `2H - 1 - n` of a `DCT` column from its even half `e`
/// and its odd inputs `v`, the first `U` of them non-zero, on the odd pairs
/// `k`.
#[inline]
#[target_feature(enable = "avx2")]
#[allow(
    clippy::many_single_char_names,
    reason = "even half e, odd inputs v, kernel k and odd sum o, as in § 7.15.2.1"
)]
fn odd_rows<const H: usize, const P: usize, const W: usize, const OUT: u8, const U: usize>(
    t: Avx2,
    e: &[__m256i; H],
    v: &[__m256i],
    k: &[[u32; H]; P],
    put: &mut Put<'_, W, OUT>,
) {
    for (n, &e) in e.iter().enumerate() {
        let o = v
            .iter()
            .zip(k)
            .take(U)
            .fold(_mm256_setzero_si256(), |o, (&v, k)| madd_splat(o, v, k[n]));
        put.rows(
            t,
            (n, 2 * H - 1 - n),
            _mm256_add_epi32(e, o),
            _mm256_sub_epi32(e, o),
        );
    }
}

/// [`odd_rows`] over the first `used` odd inputs, the rest being zero.
#[inline]
#[target_feature(enable = "avx2")]
fn odd_rows_used<const H: usize, const P: usize, const W: usize, const OUT: u8>(
    t: Avx2,
    (e, v, used): (&[__m256i; H], &[__m256i], usize),
    k: &[[u32; H]; P],
    put: &mut Put<'_, W, OUT>,
) {
    match used {
        0..=2 => odd_rows::<H, P, W, OUT, 2>(t, e, v, k, put),
        3..=4 => odd_rows::<H, P, W, OUT, 4>(t, e, v, k, put),
        _ => odd_rows::<H, P, W, OUT, 8>(t, e, v, k, put),
    }
}

/// The dense column outputs of the first `U` input pairs `v`, the rest
/// being zero.
#[inline]
#[target_feature(enable = "avx2")]
fn dense_rows<const M: usize, const HM: usize, const W: usize, const OUT: u8, const U: usize>(
    t: Avx2,
    v: &[__m256i; HM],
    k: &[[u32; M]; HM],
    rnd: __m256i,
    put: &mut Put<'_, W, OUT>,
) {
    for m in 0..HM {
        let dot = |n: usize| {
            v.iter()
                .zip(k)
                .take(U)
                .fold(rnd, |sum, (&v, k)| madd_splat(sum, v, k[n]))
        };
        put.rows(t, (2 * m, 2 * m + 1), dot(2 * m), dot(2 * m + 1));
    }
}

/// The column pass of a `W`-wide, `M`-tall block (`HM = M / 2`) over the
/// written pairs `mask` of `pairs` with the `KIND` transform on the pair
/// kernel `k` or the identity `scale`, through `Put` by `OUT`.
/// Each output row is summed with its inputs in registers and put at once.
#[inline(never)]
#[target_feature(enable = "avx2")]
#[allow(
    clippy::many_single_char_names,
    reason = "kernel k, column chunk c and output rows a and b, as in § 7.15.4.1"
)]
fn col_pass<const W: usize, const M: usize, const HM: usize, const KIND: u8, const OUT: u8>(
    t: Avx2,
    (k, scale): (&[[u32; M]; HM], i32),
    pairs: &[i32; 16 * 32],
    (mask, shift): (u32, u32),
    tile: &mut [i32; 512],
    residual: &mut [i32],
) {
    let round = Round::new(shift);
    let mut put = Put::<W, OUT> {
        tile,
        residual,
        dst: &mut [],
        stride: 0,
        round,
        c: 0,
    };
    for c in 0..(W / 8).max(1) {
        put.c = c;
        let mut v = [_mm256_setzero_si256(); HM];
        for (q, v) in v.iter_mut().enumerate() {
            if mask & (1 << q) != 0 {
                *v = if W == 4 {
                    _mm256_broadcastsi128_si256(load4(t, pairs, 4 * q))
                } else {
                    load8(t, pairs, q * W + 8 * c)
                };
            }
        }
        col_rows::<W, M, HM, KIND, OUT>(t, (k, scale), &v, mask, &mut put);
    }
}

/// The `KIND` column transform of one chunk's input pairs `v` (the pairs
/// written in `mask`) on the pair kernel `k` or the identity `scale`, every
/// output row put through `put`.
#[inline]
#[target_feature(enable = "avx2")]
fn col_rows<const W: usize, const M: usize, const HM: usize, const KIND: u8, const OUT: u8>(
    t: Avx2,
    (k, scale): (&[[u32; M]; HM], i32),
    v: &[__m256i; HM],
    mask: u32,
    put: &mut Put<'_, W, OUT>,
) {
    let round = put.round;
    match (KIND, M) {
        (IDENTITY, _) => {
            for (q, &v) in v.iter().enumerate() {
                put.rows(
                    t,
                    (2 * q, 2 * q + 1),
                    madd_splat(round.rnd, v, scale as u32),
                    madd_splat(round.rnd, v, (scale as u32) << 16),
                );
            }
        }
        (SPLIT, 4) => {
            let e = dct4(v, mask, round.rnd);
            put.rows(t, (0, 3), e[0], e[3]);
            put.rows(t, (1, 2), e[1], e[2]);
        }
        (SPLIT, 8) => odd_rows::<4, 2, W, OUT, 2>(
            t,
            &dct4(v, mask, round.rnd),
            v.get(2..4).unwrap_or_default(),
            &ODD8,
            put,
        ),
        (SPLIT, 16) => odd_rows_used(
            t,
            (
                &dct8(v, mask, round.rnd),
                v.get(4..8).unwrap_or_default(),
                used_pairs(mask >> 4, 4),
            ),
            &ODD16,
            put,
        ),
        (SPLIT, _) => odd_rows_used(
            t,
            (
                &dct16(v, mask, round.rnd),
                v.get(8..16).unwrap_or_default(),
                used_pairs(mask >> 8, 8),
            ),
            &ODD32,
            put,
        ),
        _ => match used_pairs(mask, HM) {
            0..=2 => dense_rows::<M, HM, W, OUT, 2>(t, v, k, round.rnd, put),
            3..=4 => dense_rows::<M, HM, W, OUT, 4>(t, v, k, round.rnd, put),
            5..=8 => dense_rows::<M, HM, W, OUT, 8>(t, v, k, round.rnd, put),
            _ => dense_rows::<M, HM, W, OUT, 16>(t, v, k, round.rnd, put),
        },
    }
}

/// Adds the `h` rows of the `W`-wide `i16` `tile` to the 8-bit block at the
/// start of `dst` with the § 7.14.3 `Clip1`, one row at a time.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn add_tile<const W: usize>(t: Avx2, tile: &[i32; 512], h: usize, dst: &mut [u8], stride: usize) {
    for r in 0..h.min(32) {
        let at = r * stride;
        let Some(row) = dst.get_mut(at..at + W) else {
            return;
        };
        let words = r * W / 2;
        if W == 4 {
            let Some(row) = row.first_chunk_mut::<4>() else {
                return;
            };
            let pred = _mm_cvtepu8_epi16(_mm_cvtsi32_si128(i32::from_le_bytes(*row)));
            let res = tile
                .get(words..words + 2)
                .map_or(0, |w| i64::from(w[0] as u32) | i64::from(w[1]) << 32);
            let sum = _mm_adds_epi16(pred, _mm_cvtsi64_si128(res));
            *row = _mm_cvtsi128_si32(_mm_packus_epi16(sum, sum)).to_le_bytes();
        } else if W == 8 {
            let Some(row) = row.first_chunk_mut::<8>() else {
                return;
            };
            let pred = _mm_cvtepu8_epi16(_mm_cvtsi64_si128(i64::from_le_bytes(*row)));
            let sum = _mm_adds_epi16(pred, load4(t, tile, words));
            *row = _mm_cvtsi128_si64(_mm_packus_epi16(sum, sum)).to_le_bytes();
        } else {
            for (half, out) in row.as_chunks_mut::<16>().0.iter_mut().enumerate() {
                let pred: __m128i = fearless_simd::u8x16::load_array_ref(t, out).into();
                let sum =
                    _mm256_adds_epi16(_mm256_cvtepu8_epi16(pred), load8(t, tile, words + 8 * half));
                let sum = _mm256_permute4x64_epi64::<0x08>(_mm256_packus_epi16(sum, sum));
                let sum: fearless_simd::u8x16<Avx2> = _mm256_castsi256_si128(sum).simd_into(t);
                sum.store_array(out);
            }
        }
    }
}

/// Adds the `packs` of rows `a` and `b` of 8 (4 when `W == 4`) columns to
/// the samples at `at_a` and `at_b` of `dst` with the § 7.14.3 `Clip1`.
#[inline]
#[target_feature(enable = "avx2")]
fn add_rows<const W: usize>(dst: &mut [u8], (at_a, at_b): (usize, usize), packed: __m256i) {
    if W >= 8 {
        let read = |dst: &[u8], at: usize| {
            dst.get(at..)
                .and_then(<[u8]>::first_chunk::<8>)
                .map(|row| i64::from_le_bytes(*row))
        };
        let (Some(a), Some(b)) = (read(dst, at_a), read(dst, at_b)) else {
            return;
        };
        let pred = _mm_unpacklo_epi32(_mm_cvtsi64_si128(a), _mm_cvtsi64_si128(b));
        let sum = _mm256_adds_epi16(_mm256_cvtepu8_epi16(pred), packed);
        let sum = _mm256_packus_epi16(sum, sum);
        let rows = _mm_unpacklo_epi32(
            _mm256_castsi256_si128(sum),
            _mm256_extracti128_si256::<1>(sum),
        );
        for (at, row) in [
            (at_a, _mm_cvtsi128_si64(rows)),
            (at_b, _mm_extract_epi64::<1>(rows)),
        ] {
            if let Some(out) = dst.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<8>) {
                *out = row.to_le_bytes();
            }
        }
    } else {
        let read = |dst: &[u8], at: usize| {
            dst.get(at..)
                .and_then(<[u8]>::first_chunk::<4>)
                .map(|row| u32::from_le_bytes(*row))
        };
        let (Some(a), Some(b)) = (read(dst, at_a), read(dst, at_b)) else {
            return;
        };
        let pred = _mm_cvtepu8_epi16(_mm_cvtsi64_si128(
            (u64::from(a) | u64::from(b) << 32) as i64,
        ));
        let sum = _mm_adds_epi16(pred, _mm256_castsi256_si128(packed));
        let sum = _mm_cvtsi128_si64(_mm_packus_epi16(sum, sum)) as u64;
        for (at, row) in [(at_a, sum as u32), (at_b, (sum >> 32) as u32)] {
            if let Some(out) = dst.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<4>) {
                *out = row.to_le_bytes();
            }
        }
    }
}

/// Stores rows `a` and `b` of the column `Clip3` outputs of the 8 (4 when
/// `W == 4`) columns from `8 * c`.
#[inline]
#[target_feature(enable = "avx2")]
fn put_residual<const W: usize>(
    t: Avx2,
    (a, b, c): (usize, usize, usize),
    first: __m256i,
    second: __m256i,
    residual: &mut [i32],
) {
    let (low, high) = (_mm256_set1_epi32(-256), _mm256_set1_epi32(255));
    let first = _mm256_min_epi32(_mm256_max_epi32(first, low), high);
    let second = _mm256_min_epi32(_mm256_max_epi32(second, low), high);
    if W == 4 {
        store4(t, residual, 4 * a, _mm256_castsi256_si128(first));
        store4(t, residual, 4 * b, _mm256_castsi256_si128(second));
    } else {
        store8(t, residual, a * W + 8 * c, first);
        store8(t, residual, b * W + 8 * c, second);
    }
}

/// The constant residual `value` of a `W`-wide, `h`-tall block, added to
/// `dst` with `ADD` or stored in `residual`.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn flat<const W: usize, const ADD: bool>(
    t: Avx2,
    value: i32,
    h: usize,
    (dst, stride): (&mut [u8], usize),
    residual: &mut [i32],
) {
    let v = _mm256_set1_epi32(value);
    for c in 0..(W / 8).max(1) {
        for m in (0..h).step_by(2) {
            if ADD {
                let at = m * stride + 8 * c;
                add_rows::<W>(dst, (at, at + stride), _mm256_packs_epi32(v, v));
            } else {
                put_residual::<W>(t, (m, m + 1, c), v, v, residual);
            }
        }
    }
}

/// The row pairs of the column pass `col` of an `h`-tall block.
fn pairs_of(col: Pass, h: usize) -> &'static [(u8, u8)] {
    let pairs = if col.is_dct() {
        &DCT_PAIRS[(h.trailing_zeros() as usize).saturating_sub(2).min(3)]
    } else {
        &ADJACENT_PAIRS
    };
    pairs.get(..h / 2).unwrap_or_default()
}

/// The row pass of `shape` over its `staged` inputs, returning the written
/// column pairs.
#[inline]
#[target_feature(enable = "avx2")]
fn rows(
    t: Avx2,
    shape: &InverseTransformAdd,
    staged: &[i32],
    coded: usize,
    scratch: &mut [i32],
) -> u32 {
    let p = (
        pairs_of(shape.passes.1, shape.h()),
        u32::from(shape.row_shift),
    );
    macro_rules! pass {
        ($n:literal, $h:literal, $kind:expr, $k:expr, $scale:expr) => {
            row_pass::<$n, $h, { $kind }>(
                t,
                Staged {
                    rows: staged,
                    coded,
                },
                p.0,
                (p.1, $scale),
                $k,
                scratch,
            )
        };
    }
    match shape.passes.0 {
        Pass::Identity16 => pass!(16, 8, IDENTITY, &DCT16, 256),
        Pass::Identity32 => pass!(32, 16, IDENTITY, &DCT32, 362),
        Pass::Dct16 => pass!(16, 8, SPLIT, &DCT16_SPLIT, 0),
        Pass::Adst16 => pass!(16, 8, DENSE, &ADST16, 0),
        Pass::Fdst16 => pass!(16, 8, DENSE, &FDST16, 0),
        Pass::Ddtx16 => pass!(16, 8, DENSE, &DDTX16, 0),
        Pass::Fddt16 => pass!(16, 8, DENSE, &FDDT16, 0),
        Pass::Dct32 => pass!(32, 16, SPLIT, &DCT32_SPLIT, 0),
        _ => 0,
    }
}

/// The row pass of a 4- or 8-wide `shape` straight from `coeffs`,
/// dequantized by `dq` when `quantized`, returning the written column pairs.
#[inline]
#[target_feature(enable = "avx2")]
fn rows_direct(
    t: Avx2,
    shape: &InverseTransformAdd,
    (coeffs, dq, quantized): (&[i32], &FlatDequant, bool),
    scratch: &mut [i32],
) -> u32 {
    let p = (
        pairs_of(shape.passes.1, shape.h()),
        u32::from(shape.row_shift),
    );
    macro_rules! kinds {
        ($dq:literal, $rs:literal) => {{
            let input = Input::<$dq, $rs>::new(coeffs, dq);
            let pass4 = |kind, k: &[[u32; 4]; 2], scale, out: &mut [i32]| match kind {
                IDENTITY => {
                    row_pass_direct::<4, 2, IDENTITY, $dq, $rs>(t, input, p.0, (p.1, scale), k, out)
                }
                _ => row_pass_direct::<4, 2, DENSE, $dq, $rs>(t, input, p.0, (p.1, scale), k, out),
            };
            let pass8 = |kind, k: &[[u32; 8]; 4], scale, out: &mut [i32]| match kind {
                IDENTITY => {
                    row_pass_direct::<8, 4, IDENTITY, $dq, $rs>(t, input, p.0, (p.1, scale), k, out)
                }
                SPLIT => {
                    row_pass_direct::<8, 4, SPLIT, $dq, $rs>(t, input, p.0, (p.1, scale), k, out)
                }
                _ => row_pass_direct::<8, 4, DENSE, $dq, $rs>(t, input, p.0, (p.1, scale), k, out),
            };
            if let Some((kind, k, scale)) = row_spec4(shape.passes.0) {
                pass4(kind, k, scale, scratch)
            } else if let Some((kind, k, scale)) = row_spec8(shape.passes.0) {
                pass8(kind, k, scale, scratch)
            } else {
                0
            }
        }};
    }
    match (quantized, shape.rescale) {
        (true, false) => kinds!(true, false),
        (true, true) => kinds!(true, true),
        (false, false) => kinds!(false, false),
        (false, true) => kinds!(false, true),
    }
}

/// A 4- or 8-point pass of a small block: its kind, pair kernel and
/// identity scale.
type Spec<const N: usize, const H: usize> = (u8, &'static [[u32; N]; H], i32);

/// The row pass `pass` of a 4-wide block.
const fn row_spec4(pass: Pass) -> Option<Spec<4, 2>> {
    Some(match pass {
        Pass::Identity4 => (IDENTITY, &DCT4, 128),
        Pass::Dct4 => (DENSE, &DCT4, 0),
        Pass::Adst4 => (DENSE, &ADST4, 0),
        Pass::Fdst4 => (DENSE, &FDST4, 0),
        _ => return None,
    })
}

/// The row pass `pass` of an 8-wide block.
const fn row_spec8(pass: Pass) -> Option<Spec<8, 4>> {
    Some(match pass {
        Pass::Identity8 => (IDENTITY, &DCT8, 181),
        Pass::Dct8 => (SPLIT, &DCT8_SPLIT, 0),
        Pass::Adst8 => (DENSE, &ADST8, 0),
        Pass::Fdst8 => (DENSE, &FDST8, 0),
        Pass::Ddtx8 => (DENSE, &DDTX8, 0),
        Pass::Fddt8 => (DENSE, &FDDT8, 0),
        _ => return None,
    })
}

/// The column pass `pass` of a 4-tall block.
const fn col_spec4(pass: Pass) -> Option<Spec<4, 2>> {
    Some(match pass {
        Pass::Dct4 => (SPLIT, &DCT4, 0),
        _ => return row_spec4(pass),
    })
}

/// The column pass `pass` of an 8-tall block.
const fn col_spec8(pass: Pass) -> Option<Spec<8, 4>> {
    Some(match pass {
        Pass::Dct8 => (SPLIT, &DCT8, 0),
        _ => return row_spec8(pass),
    })
}

/// The column pass `pass` of a 16-tall block.
const fn col_spec16(pass: Pass) -> Option<Spec<16, 8>> {
    Some(match pass {
        Pass::Identity16 => (IDENTITY, &DCT16, 256),
        Pass::Dct16 => (SPLIT, &DCT16, 0),
        Pass::Adst16 => (DENSE, &ADST16, 0),
        Pass::Fdst16 => (DENSE, &FDST16, 0),
        Pass::Ddtx16 => (DENSE, &DDTX16, 0),
        Pass::Fddt16 => (DENSE, &FDDT16, 0),
        _ => return None,
    })
}

/// The [`Spec`] of every [`Pass`] by `spec`, indexed by the pass.
macro_rules! by_pass {
    ($spec:ident) => {{
        let mut out = [None; Pass::ALL.len()];
        assert!(Pass::Dct32 as usize + 1 == out.len());
        let mut i = 0;
        while i < out.len() {
            assert!(Pass::ALL[i] as usize == i);
            out[i] = $spec(Pass::ALL[i]);
            i += 1;
        }
        out
    }};
}

static ROW_SPEC4: [Option<Spec<4, 2>>; Pass::ALL.len()] = by_pass!(row_spec4);
static ROW_SPEC8: [Option<Spec<8, 4>>; Pass::ALL.len()] = by_pass!(row_spec8);
static COL_SPEC4: [Option<Spec<4, 2>>; Pass::ALL.len()] = by_pass!(col_spec4);
static COL_SPEC8: [Option<Spec<8, 4>>; Pass::ALL.len()] = by_pass!(col_spec8);
static COL_SPEC16: [Option<Spec<16, 8>>; Pass::ALL.len()] = by_pass!(col_spec16);

/// The `W`-sample rows (4 or 8) of a coefficient prefix: row `r` whole,
/// the one partial row zero-padded, zero past the prefix.
#[derive(Clone, Copy)]
struct Rows<'a, const W: usize> {
    full: &'a [[i32; W]],
    tail: __m256i,
}

impl<'a, const W: usize> Rows<'a, W> {
    #[inline]
    #[target_feature(enable = "avx2")]
    fn new<const DQ: bool, const RS: bool>(t: Avx2, input: Input<'a, DQ, RS>) -> Self {
        let (full, rest) = input.coeffs.as_chunks::<W>();
        let at = full.len() * W;
        let tail = match (rest.is_empty(), W) {
            (true, _) => _mm256_setzero_si256(),
            (false, 4) => _mm256_castsi128_si256(input.coeffs4(t, at)),
            _ => input.coeffs8(t, at),
        };
        Self { full, tail }
    }

    /// Row `r` (in the low lane when `W == 4`).
    #[inline]
    #[target_feature(enable = "avx2")]
    fn row(self, t: Avx2, r: usize) -> __m256i {
        match self.full.get(r) {
            Some(row) if W == 4 => _mm256_castsi128_si256(load4(t, row, 0)),
            Some(row) => load8(t, row, 0),
            None if r == self.full.len() => self.tail,
            None => _mm256_setzero_si256(),
        }
    }
}

/// A `W x H` block, 4 or 8 wide and 4 to 16 tall (`WH = W / 2`,
/// `HH = H / 2`), with the
/// `CK` column transform, in registers: the row pass of each coded row pair
/// gives the column pass's input pair at once, and the column outputs are
/// added straight to `dst`.
#[inline(never)]
#[target_feature(enable = "avx2")]
#[allow(
    clippy::many_single_char_names,
    reason = "rows a and b of a column pair, as in § 7.15.4.1"
)]
fn small<
    const W: usize,
    const WH: usize,
    const H: usize,
    const HH: usize,
    const CK: u8,
    const DQ: bool,
    const RS: bool,
>(
    t: Avx2,
    (coeffs, dq): (&[i32], &FlatDequant),
    (row, col): (&Spec<W, WH>, &Spec<H, HH>),
    (row_shift, col_shift): (u32, u32),
    (dst, stride): (&mut [u8], usize),
) {
    let input = Input::<DQ, RS>::new(coeffs, dq);
    let rows = Rows::<W>::new(t, input);
    let pairs = if CK == SPLIT {
        const { dct_pairs(H) }
    } else {
        ADJACENT_PAIRS
    };
    let round = Round::new(row_shift);
    let coded = coeffs.len().div_ceil(W);
    let mut mask = 0;
    let v: [__m256i; HH] = core::array::from_fn(|q| {
        let (a, b) = (usize::from(pairs[q].0), usize::from(pairs[q].1));
        if a >= coded && b >= coded {
            return _mm256_setzero_si256();
        }
        mask |= 1 << q;
        if W == 4 {
            let x = _mm256_permute2x128_si256::<0x20>(rows.row(t, a), rows.row(t, b));
            let x = input.finish(x, a == 0);
            _mm256_broadcastsi128_si256(if row.0 == IDENTITY {
                let v = _mm256_mullo_epi32(clamp16(x), _mm256_set1_epi32(row.2));
                unpack4(round.apply(v))
            } else {
                row4_pairs(t, _mm256_packs_epi32(x, x), row.1.as_flattened(), round)
            })
        } else {
            let xa = input.finish(rows.row(t, a), a == 0);
            let xb = input.finish(rows.row(t, b), b == 0);
            let x = _mm256_permute4x64_epi64::<0xD8>(_mm256_packs_epi32(xa, xb));
            match row.0 {
                IDENTITY => {
                    let scale = _mm256_set1_epi32(row.2);
                    let va = round.apply(_mm256_mullo_epi32(clamp16(xa), scale));
                    let vb = round.apply(_mm256_mullo_epi32(clamp16(xb), scale));
                    pair_columns(t, va, vb, &INTERLEAVE)
                }
                SPLIT => row8_pairs::<SPLIT>(t, x, row.1.as_flattened(), round),
                _ => row8_pairs::<DENSE>(t, x, row.1.as_flattened(), round),
            }
        }
    });
    let mut put = Put::<W, DIRECT> {
        tile: &mut [],
        residual: &mut [],
        dst,
        stride,
        round: Round::new(col_shift),
        c: 0,
    };
    col_rows::<W, H, HH, CK, DIRECT>(t, (col.1, col.2), &v, mask, &mut put);
}

/// Adds a `shape` 4 or 8 wide and 4 to 16 tall over `coeffs` to `dst`
/// through [`small`];
/// `false` for any other shape.
#[inline]
#[target_feature(enable = "avx2")]
fn add_small(
    t: Avx2,
    shape: &InverseTransformAdd,
    (coeffs, dq, quantized): (&[i32], &FlatDequant, bool),
    out: (&mut [u8], usize),
) -> bool {
    let shifts = (u32::from(shape.row_shift), u32::from(shape.col_shift));
    let (row, col) = shape.passes;
    let input = (coeffs, dq);
    macro_rules! run {
        ($rs:literal, $f:ident [$($g:literal),*], $row:expr, $col:expr) => {{
            let (Some(row), Some(col)) = (&$row[row as usize], &$col[col as usize]) else {
                return false;
            };
            let specs = (row, col);
            match (col.0, quantized) {
                (IDENTITY, true) => $f::<$($g,)* IDENTITY, true, $rs>(t, input, specs, shifts, out),
                (IDENTITY, false) => $f::<$($g,)* IDENTITY, false, $rs>(t, input, specs, shifts, out),
                (SPLIT, true) => $f::<$($g,)* SPLIT, true, $rs>(t, input, specs, shifts, out),
                (SPLIT, false) => $f::<$($g,)* SPLIT, false, $rs>(t, input, specs, shifts, out),
                (_, true) => $f::<$($g,)* DENSE, true, $rs>(t, input, specs, shifts, out),
                (_, false) => $f::<$($g,)* DENSE, false, $rs>(t, input, specs, shifts, out),
            }
            true
        }};
    }
    match (shape.w_log2, shape.h_log2, shape.rescale) {
        (2, 2, false) => run!(false, small[4, 2, 4, 2], ROW_SPEC4, COL_SPEC4),
        (2, 3, true) => run!(true, small[4, 2, 8, 4], ROW_SPEC4, COL_SPEC8),
        (3, 2, true) => run!(true, small[8, 4, 4, 2], ROW_SPEC8, COL_SPEC4),
        (3, 3, false) => run!(false, small[8, 4, 8, 4], ROW_SPEC8, COL_SPEC8),
        (2, 4, false) => run!(false, small[4, 2, 16, 8], ROW_SPEC4, COL_SPEC16),
        (3, 4, true) => run!(true, small[8, 4, 16, 8], ROW_SPEC8, COL_SPEC16),
        _ => false,
    }
}

/// The column pass of `shape` over the written `mask` pairs of `pairs`.
#[inline]
#[target_feature(enable = "avx2")]
fn cols<const OUT: u8>(
    t: Avx2,
    shape: &InverseTransformAdd,
    pairs: &[i32; 16 * 32],
    mask: u32,
    out: &mut [i32; 512],
    residual: &mut [i32],
) {
    let passes = (mask, u32::from(shape.col_shift));
    macro_rules! pass {
        ($m:literal, $hm:literal, $kind:expr, $k:expr, $scale:expr) => {
            match shape.w_log2 {
                2 => col_pass::<4, $m, $hm, { $kind }, OUT>(
                    t,
                    ($k, $scale),
                    pairs,
                    passes,
                    out,
                    residual,
                ),
                3 => col_pass::<8, $m, $hm, { $kind }, OUT>(
                    t,
                    ($k, $scale),
                    pairs,
                    passes,
                    out,
                    residual,
                ),
                4 => col_pass::<16, $m, $hm, { $kind }, OUT>(
                    t,
                    ($k, $scale),
                    pairs,
                    passes,
                    out,
                    residual,
                ),
                _ => col_pass::<32, $m, $hm, { $kind }, OUT>(
                    t,
                    ($k, $scale),
                    pairs,
                    passes,
                    out,
                    residual,
                ),
            }
        };
    }
    match shape.passes.1 {
        Pass::Identity4 => pass!(4, 2, IDENTITY, &DCT4, 128),
        Pass::Identity8 => pass!(8, 4, IDENTITY, &DCT8, 181),
        Pass::Identity16 => pass!(16, 8, IDENTITY, &DCT16, 256),
        Pass::Identity32 => pass!(32, 16, IDENTITY, &DCT32, 362),
        Pass::Dct4 => pass!(4, 2, SPLIT, &DCT4, 0),
        Pass::Adst4 => pass!(4, 2, DENSE, &ADST4, 0),
        Pass::Fdst4 => pass!(4, 2, DENSE, &FDST4, 0),
        Pass::Dct8 => pass!(8, 4, SPLIT, &DCT8, 0),
        Pass::Adst8 => pass!(8, 4, DENSE, &ADST8, 0),
        Pass::Fdst8 => pass!(8, 4, DENSE, &FDST8, 0),
        Pass::Ddtx8 => pass!(8, 4, DENSE, &DDTX8, 0),
        Pass::Fddt8 => pass!(8, 4, DENSE, &FDDT8, 0),
        Pass::Dct16 => pass!(16, 8, SPLIT, &DCT16, 0),
        Pass::Adst16 => pass!(16, 8, DENSE, &ADST16, 0),
        Pass::Fdst16 => pass!(16, 8, DENSE, &FDST16, 0),
        Pass::Ddtx16 => pass!(16, 8, DENSE, &DDTX16, 0),
        Pass::Fddt16 => pass!(16, 8, DENSE, &FDDT16, 0),
        Pass::Dct32 => pass!(32, 16, SPLIT, &DCT32, 0),
    }
}

fearless_simd::kernel!(
    /// Adds `shape`, 4 or 8 wide and 4 to 16 tall, over `coeffs`
    /// (dequantized by `dequant` as they load, or already dequantized) to
    /// the 8-bit block at the start of `dst`; `false` with `dst` untouched
    /// for any other shape.
    pub(super) fn add_small_block(
        t: Avx2,
        shape: &InverseTransformAdd,
        dequant: Option<FlatDequant>,
        coeffs: &[i32],
        dst: &mut [u8],
        stride: usize,
    ) -> bool {
        if shape.w_log2 > 3 || shape.h_log2 > 4 {
            return false;
        }
        if let Some(value) = shape.flat_residual(coeffs, dequant, crate::BitDepth::Eight) {
            let out = (dst, stride);
            if shape.w_log2 == 2 {
                flat::<4, true>(t, value, shape.h(), out, &mut []);
            } else {
                flat::<8, true>(t, value, shape.h(), out, &mut []);
            }
            return true;
        }
        let none = FlatDequant {
            dc: 0,
            ac: 0,
            denom_shift: 0,
        };
        let dq = dequant.as_ref().unwrap_or(&none);
        add_small(t, shape, (coeffs, dq, dequant.is_some()), (dst, stride))
    }
);

fearless_simd::kernel!(
    /// Runs `shape` over `coeffs` (dequantized by `dequant` as they load, or
    /// already dequantized) into `sink`, with the 32x32 `scratch` holding the
    /// column pairs. With `rows8`, `shape` is 16 or 32 wide without an
    /// identity row pass and `coeffs` holds only the first 8 columns of its
    /// coded rows, row `r` at `coeffs[8 * r..]`, dequantized.
    pub(super) fn transform(
        t: Avx2,
        shape: &InverseTransformAdd,
        dequant: Option<FlatDequant>,
        coeffs: &[i32],
        rows8: bool,
        sink: Sink<'_>,
        scratch: &mut [i32; 32 * 32],
    ) {
        let (w, h) = (shape.w(), shape.h());
        let (dst, stride, residual) = match sink {
            Sink::Add { dst, stride } => (dst, stride, None),
            Sink::Residual(residual) => (&mut [][..], 0, Some(residual)),
        };
        let mut empty = [];
        if let Some(value) = shape.flat_residual(coeffs, dequant, crate::BitDepth::Eight) {
            let out = (dst, stride);
            match (residual, shape.w_log2) {
                (None, 2) => flat::<4, true>(t, value, h, out, &mut empty),
                (None, 3) => flat::<8, true>(t, value, h, out, &mut empty),
                (None, 4) => flat::<16, true>(t, value, h, out, &mut empty),
                (None, _) => flat::<32, true>(t, value, h, out, &mut empty),
                (Some(residual), 2) => flat::<4, false>(t, value, h, out, residual),
                (Some(residual), 3) => flat::<8, false>(t, value, h, out, residual),
                (Some(residual), 4) => flat::<16, false>(t, value, h, out, residual),
                (Some(residual), _) => flat::<32, false>(t, value, h, out, residual),
            }
            return;
        }
        let none = FlatDequant {
            dc: 0,
            ac: 0,
            denom_shift: 0,
        };
        let dq = dequant.as_ref().unwrap_or(&none);
        let Some((pairs, staged)) = scratch.split_first_chunk_mut::<{ 16 * 32 }>() else {
            return;
        };
        let mask = if w <= 8 {
            rows_direct(t, shape, (coeffs, dq, dequant.is_some()), pairs)
        } else if rows8 {
            let (coeff_rows, _) = coeffs.as_chunks::<8>();
            let rows = Rows8 {
                rows: coeff_rows.get(..h).unwrap_or(coeff_rows),
                rescale: shape.rescale,
            };
            rows8_pass(t, shape, rows, pairs)
        } else {
            let coded = (coeffs.len() + w - 1) >> shape.w_log2;
            let count = coded * w;
            match (dequant.is_some(), shape.rescale) {
                (true, false) => stage::<true, false>(t, coeffs, dq, count, staged),
                (true, true) => stage::<true, true>(t, coeffs, dq, count, staged),
                (false, false) => stage::<false, false>(t, coeffs, dq, count, staged),
                (false, true) => stage::<false, true>(t, coeffs, dq, count, staged),
            }
            rows(t, shape, staged, coded, pairs)
        };
        if mask == 0 {
            if let Some(residual) = residual {
                residual.fill(0);
            }
            return;
        }
        let Some(tile) = staged.first_chunk_mut::<512>() else {
            return;
        };
        let Some(residual) = residual else {
            cols::<TILE>(t, shape, pairs, mask, tile, &mut empty);
            match shape.w_log2 {
                2 => add_tile::<4>(t, tile, h, dst, stride),
                3 => add_tile::<8>(t, tile, h, dst, stride),
                4 => add_tile::<16>(t, tile, h, dst, stride),
                _ => add_tile::<32>(t, tile, h, dst, stride),
            }
            return;
        };
        cols::<RESIDUAL>(t, shape, pairs, mask, tile, residual);
    }
);
