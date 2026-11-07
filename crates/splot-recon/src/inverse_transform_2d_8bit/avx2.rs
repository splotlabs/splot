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
use splot_tables::tables::transform_1d::{DCT_KERNEL4, DCT_KERNEL8, DCT_KERNEL16, DCT_KERNEL32};

use super::{
    DCT4, DCT8, DCT8_SPLIT, DCT16, DCT16_SPLIT, DCT32, DCT32_SPLIT, FlatDequant, PairKernel,
    SPLIT_ORDER,
};

/// The row transform of one [`Plan`].
#[derive(Clone, Copy)]
pub(super) enum Row {
    /// § 7.15.2.3 identity with its scale.
    Identity(i32),
    /// The split `DCT` of a row of 8, 16 or 32.
    Dct,
    /// A paired kernel product.
    Dense(PairKernel),
}

/// The column transform of one [`Plan`].
#[derive(Clone, Copy)]
pub(super) enum Col {
    /// § 7.15.2.3 identity with its scale.
    Identity(i32),
    /// The factored `DCT`.
    Dct,
    /// A paired kernel product.
    Dense(PairKernel),
}

/// One validated 8-bit transform.
#[derive(Clone, Copy)]
pub(super) struct Plan {
    pub w: usize,
    pub h: usize,
    pub row: Row,
    pub col: Col,
    pub rescale: bool,
    pub row_shift: u32,
    pub col_shift: u32,
    pub dequant: Option<FlatDequant>,
    /// The constant residual of a block whose transform is flat.
    pub flat: Option<i32>,
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
    let (dc, ac) = (dq.dc as i32, dq.ac as i32);
    Input::<DQ, RS> {
        coeffs,
        dc: _mm256_setr_epi32(dc, ac, ac, ac, ac, ac, ac, ac),
        ac: _mm256_set1_epi32(ac),
        shift: _mm_cvtsi32_si128(3 + dq.denom_shift as i32),
    }
    .stage(t, count, staged);
}

impl<const DQ: bool, const RS: bool> Input<'_, DQ, RS> {
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
            let scaled = _mm256_mullo_epi32(v, _mm256_set1_epi32(2896));
            v = _mm256_srai_epi32::<12>(_mm256_add_epi32(scaled, _mm256_set1_epi32(1 << 11)));
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

    /// The four inputs of row `r` (`N == 4`).
    #[inline]
    #[target_feature(enable = "avx2")]
    fn x4(self, r: usize) -> __m128i {
        let at = r * 2;
        match self.rows.get(at..at + 2) {
            Some(&[low, high]) if r < self.coded => {
                _mm_cvtsi64_si128(i64::from(low as u32) | i64::from(high) << 32)
            }
            _ => _mm_setzero_si128(),
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

/// The row pass of an `N`-wide block (`H = N / 2`) over the column `pairs`
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
        let written = match (KIND, N) {
            (IDENTITY, 4) => {
                let x = _mm256_set_m128i(rows.x4(b), rows.x4(a));
                let v = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(_mm256_permute4x64_epi64::<
                    0x08,
                >(x)));
                let v = round.apply(_mm256_mullo_epi32(v, _mm256_set1_epi32(scale)));
                store4(t, out, 0, unpack4(v));
                !is_zero(x)
            }
            (IDENTITY, _) => {
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
            }
            (_, 4) => row4(
                t,
                _mm256_set_m128i(rows.x4(b), rows.x4(a)),
                kernel.as_flattened(),
                round,
                out,
            ),
            (_, 8) => {
                let x = _mm256_set_m128i(rows.x8(t, b, 0), rows.x8(t, a, 0));
                row8::<KIND>(t, x, kernel.as_flattened(), round, out)
            }
            _ => row_wide::<N, H>(t, rows, (a, b), kernel, KIND == SPLIT, round, out),
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
    let sum = _mm256_add_epi32(
        _mm256_madd_epi16(_mm256_shuffle_epi32::<0x00>(x), weights128(t, k)),
        _mm256_madd_epi16(
            _mm256_shuffle_epi32::<0x55>(x),
            weights128(t, k.get(4..).unwrap_or_default()),
        ),
    );
    store4(t, out, 0, unpack4(round.apply(sum)));
    true
}

/// One pair of 8-sample rows, `x` holding row `a` in its low and row `b` in
/// its high lane: the split `DCT` on `DCT8_SPLIT` or the paired kernel `k`.
#[inline]
#[target_feature(enable = "avx2")]
fn row8<const KIND: u8>(t: Avx2, x: __m256i, k: &[u32], round: Round, out: &mut [i32]) -> bool {
    if is_zero(x) {
        return false;
    }
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
    store8(t, out, 0, _mm256_shuffle_epi8(packed, bytes(t, order)));
    true
}

/// One pair of `N`-sample rows (`H = N / 2`) on the pair `kernel`, split
/// into even and odd inputs with `split`.
#[inline]
#[target_feature(enable = "avx2")]
#[allow(
    clippy::many_single_char_names,
    reason = "rows a and b of a column pair, as in § 7.15.4.1"
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
    let mut outputs = [[_mm256_setzero_si256(); 4]; 2];
    let mut any = false;
    for (slot, r) in outputs.iter_mut().zip([a, b]) {
        let mut pairs = [0u32; 16];
        let mut nonzero = 0u32;
        for (j, chunk) in pairs
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
        if nonzero == 0 {
            continue;
        }
        any = true;
        let used = (u32::BITS - nonzero.leading_zeros()) as usize;
        let (pairs, kernel) = (
            pairs.get(..used).unwrap_or_default(),
            kernel.get(..used).unwrap_or_default(),
        );
        if split {
            let mut sums = [[round.rnd; 2], [_mm256_setzero_si256(); 2]];
            for (p, (&pair, k)) in pairs.iter().zip(kernel).enumerate() {
                let x = _mm256_set1_epi32(pair as i32);
                for (c, sum) in sums[p % 2].iter_mut().take(chunks / 2).enumerate() {
                    let k = k.get(8 * c..).unwrap_or_default();
                    *sum = _mm256_add_epi32(*sum, _mm256_madd_epi16(x, weights(t, k)));
                }
            }
            let [even, odd] = sums;
            for c in 0..chunks / 2 {
                slot[c] = _mm256_sra_epi32(_mm256_add_epi32(even[c], odd[c]), round.shift);
                slot[chunks - 1 - c] =
                    _mm256_sra_epi32(_mm256_sub_epi32(even[c], odd[c]), round.shift);
            }
        } else {
            for (&pair, k) in pairs.iter().zip(kernel) {
                let x = _mm256_set1_epi32(pair as i32);
                for (c, sum) in slot.iter_mut().take(chunks).enumerate() {
                    let k = k.get(8 * c..).unwrap_or_default();
                    *sum = _mm256_add_epi32(*sum, _mm256_madd_epi16(x, weights(t, k)));
                }
            }
            for sum in slot.iter_mut().take(chunks) {
                *sum = round.apply(*sum);
            }
        }
    }
    if !any {
        return false;
    }
    let [a, b] = outputs;
    for c in 0..chunks {
        let packed = if split && c >= chunks / 2 {
            let reversed = pair_columns(t, a[c], b[c], &INTERLEAVE_REVERSED);
            _mm256_permute4x64_epi64::<0x4E>(reversed)
        } else {
            pair_columns(t, a[c], b[c], &INTERLEAVE)
        };
        store8(t, out, 8 * c, packed);
    }
    true
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

#[inline]
#[target_feature(enable = "avx2")]
fn dct32(v: &[__m256i], mask: u32, rnd: __m256i) -> [__m256i; 32] {
    let odd = products(
        v.get(8..).unwrap_or_default(),
        mask >> 8,
        &ODD32,
        _mm256_setzero_si256(),
    );
    butterfly::<16, 32>(&dct16(v, mask, rnd), &odd)
}

/// The first `B` entries of `x`, zero past `A`.
#[inline]
#[target_feature(enable = "avx2")]
fn fit<const A: usize, const B: usize>(x: [__m256i; A]) -> [__m256i; B] {
    let mut out = [_mm256_setzero_si256(); B];
    for (out, x) in out.iter_mut().zip(x) {
        *out = x;
    }
    out
}

/// The column pass of a `w`-wide, `M`-tall block (`HM = M / 2`) over the
/// written pairs `mask` of `scratch` with the `KIND` transform on the pair
/// kernel `k` or the identity `scale`, added to `dst` with `ADD` or stored in
/// `residual`.
#[inline(never)]
#[target_feature(enable = "avx2")]
#[allow(clippy::too_many_arguments)]
fn col_pass<const M: usize, const HM: usize, const KIND: u8, const ADD: bool>(
    t: Avx2,
    w: usize,
    (k, scale): (&[[u32; M]; HM], i32),
    scratch: &[i32],
    mask: u32,
    shift: u32,
    (dst, stride): (&mut [u8], usize),
    residual: &mut [i32],
) {
    let round = Round::new(shift);
    for c in 0..(w / 8).max(1) {
        let mut v = [_mm256_setzero_si256(); HM];
        for (q, v) in v.iter_mut().enumerate() {
            if mask & (1 << q) != 0 {
                *v = if w == 4 {
                    _mm256_broadcastsi128_si256(load4(t, scratch, 4 * q))
                } else {
                    load8(t, scratch, q * w + 8 * c)
                };
            }
        }
        let out: [__m256i; M] = match (KIND, M) {
            (IDENTITY, _) => {
                let mut out = [round.rnd; M];
                for (q, (&v, out)) in v.iter().zip(out.as_chunks_mut::<2>().0).enumerate() {
                    if mask & (1 << q) != 0 {
                        out[0] = madd_splat(out[0], v, scale as u32);
                        out[1] = madd_splat(out[1], v, (scale as u32) << 16);
                    }
                }
                out
            }
            (SPLIT, 4) => fit(dct4(&v, mask, round.rnd)),
            (SPLIT, 8) => fit(dct8(&v, mask, round.rnd)),
            (SPLIT, 16) => fit(dct16(&v, mask, round.rnd)),
            (SPLIT, _) => fit(dct32(&v, mask, round.rnd)),
            _ => products(&v, mask, k, round.rnd),
        };
        let shift = |v| _mm256_sra_epi32(v, round.shift);
        if ADD {
            let width = w.min(8);
            for (m, rows) in out.as_chunks::<2>().0.iter().enumerate() {
                let at = 2 * m * stride + 8 * c;
                let Some((first, second)) = dst
                    .get_mut(at..at + stride + width)
                    .and_then(|span| span.split_at_mut_checked(stride))
                else {
                    break;
                };
                put_add(first, second, shift(rows[0]), shift(rows[1]));
            }
        } else {
            for (m, rows) in out.as_chunks::<2>().0.iter().enumerate() {
                put_residual(t, w, (2 * m, c), shift(rows[0]), shift(rows[1]), residual);
            }
        }
    }
}

/// Adds the rows `a` and `b` of 8 (4 when `w == 4`) columns to the samples
/// `first` and `second` with the § 7.14.3 `Clip1`.
#[inline]
#[target_feature(enable = "avx2")]
fn put_add(first: &mut [u8], second: &mut [u8], a: __m256i, b: __m256i) {
    let packed = _mm256_packs_epi32(a, b);
    if let (Some(first), Some(second)) =
        (first.first_chunk_mut::<8>(), second.first_chunk_mut::<8>())
    {
        let pred = _mm_unpacklo_epi32(
            _mm_cvtsi64_si128(i64::from_le_bytes(*first)),
            _mm_cvtsi64_si128(i64::from_le_bytes(*second)),
        );
        let sum = _mm256_adds_epi16(_mm256_cvtepu8_epi16(pred), packed);
        let sum = _mm256_packus_epi16(sum, sum);
        let rows = _mm_unpacklo_epi32(
            _mm256_castsi256_si128(sum),
            _mm256_extracti128_si256::<1>(sum),
        );
        *first = _mm_cvtsi128_si64(rows).to_le_bytes();
        *second = _mm_extract_epi64::<1>(rows).to_le_bytes();
    } else if let (Some(first), Some(second)) =
        (first.first_chunk_mut::<4>(), second.first_chunk_mut::<4>())
    {
        let pred =
            u64::from(u32::from_le_bytes(*first)) | u64::from(u32::from_le_bytes(*second)) << 32;
        let pred = _mm_cvtepu8_epi16(_mm_cvtsi64_si128(pred as i64));
        let sum = _mm_adds_epi16(pred, _mm256_castsi256_si128(packed));
        let sum = _mm_cvtsi128_si64(_mm_packus_epi16(sum, sum)) as u64;
        *first = (sum as u32).to_le_bytes();
        *second = ((sum >> 32) as u32).to_le_bytes();
    }
}

/// Stores rows `m` and `m + 1` of the column `Clip3` outputs of the 8 (4
/// when `w == 4`) columns from `8 * c`.
#[inline]
#[target_feature(enable = "avx2")]
fn put_residual(
    t: Avx2,
    w: usize,
    (m, c): (usize, usize),
    first: __m256i,
    second: __m256i,
    residual: &mut [i32],
) {
    let (low, high) = (_mm256_set1_epi32(-256), _mm256_set1_epi32(255));
    let first = _mm256_min_epi32(_mm256_max_epi32(first, low), high);
    let second = _mm256_min_epi32(_mm256_max_epi32(second, low), high);
    if w == 4 {
        store4(t, residual, 4 * m, _mm256_castsi256_si128(first));
        store4(t, residual, 4 * m + 4, _mm256_castsi256_si128(second));
    } else {
        store8(t, residual, m * w + 8 * c, first);
        store8(t, residual, (m + 1) * w + 8 * c, second);
    }
}

/// The row pairs of the column transform of `plan`.
fn pairs_of(plan: &Plan) -> &'static [(u8, u8)] {
    let pairs = match plan.col {
        Col::Dct => &DCT_PAIRS[(plan.h.trailing_zeros() as usize).saturating_sub(2).min(3)],
        Col::Identity(_) | Col::Dense(_) => &ADJACENT_PAIRS,
    };
    pairs.get(..plan.h / 2).unwrap_or_default()
}

/// The row pass of `plan` over its `staged` inputs, returning the written
/// column pairs.
#[inline]
#[target_feature(enable = "avx2")]
fn rows(t: Avx2, plan: &Plan, staged: &[i32], coded: usize, scratch: &mut [i32]) -> u32 {
    let p = (pairs_of(plan), plan.row_shift);
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
    match (plan.row, plan.w) {
        (Row::Identity(s), 4) => pass!(4, 2, IDENTITY, &DCT4, s),
        (Row::Identity(s), 8) => pass!(8, 4, IDENTITY, &DCT8, s),
        (Row::Identity(s), 16) => pass!(16, 8, IDENTITY, &DCT16, s),
        (Row::Identity(s), _) => pass!(32, 16, IDENTITY, &DCT32, s),
        (Row::Dense(PairKernel::N4(k)), _) => pass!(4, 2, DENSE, k, 0),
        (Row::Dense(PairKernel::N8(k)), _) => pass!(8, 4, DENSE, k, 0),
        (Row::Dct, 8) => pass!(8, 4, SPLIT, &DCT8_SPLIT, 0),
        (Row::Dense(PairKernel::N16(k)), _) => pass!(16, 8, DENSE, k, 0),
        (Row::Dct, 16) => pass!(16, 8, SPLIT, &DCT16_SPLIT, 0),
        (Row::Dct | Row::Dense(PairKernel::N32(_)), _) => pass!(32, 16, SPLIT, &DCT32_SPLIT, 0),
    }
}

fearless_simd::kernel!(
    /// Runs `plan` over the whole-row or quantized prefix `coeffs` into
    /// `sink`, with the 32x32 `scratch` holding the column pairs.
    pub(super) fn transform(
        t: Avx2,
        plan: &Plan,
        coeffs: &[i32],
        sink: Sink<'_>,
        scratch: &mut [i32; 32 * 32],
    ) {
        let (w, h) = (plan.w, plan.h);
        let (dst, stride, mut residual) = match sink {
            Sink::Add { dst, stride } => (dst, stride, None),
            Sink::Residual(residual) => (&mut [][..], 0, Some(residual)),
        };
        let mut empty = [];
        if let Some(value) = plan.flat {
            let v = _mm256_set1_epi32(value);
            match residual {
                None => {
                    for c in 0..(w / 8).max(1) {
                        for m in (0..h).step_by(2) {
                            let at = m * stride + 8 * c;
                            if let Some((first, second)) = dst
                                .get_mut(at..at + stride + w.min(8))
                                .and_then(|span| span.split_at_mut_checked(stride))
                            {
                                put_add(first, second, v, v);
                            }
                        }
                    }
                }
                Some(ref mut residual) => {
                    for c in 0..(w / 8).max(1) {
                        for m in (0..h).step_by(2) {
                            put_residual(t, w, (m, c), v, v, residual);
                        }
                    }
                }
            }
            return;
        }
        let dq = plan.dequant.unwrap_or(FlatDequant {
            dc: 0,
            ac: 0,
            denom_shift: 0,
            bound: 0,
        });
        let coded = (coeffs.len() + w - 1) >> w.trailing_zeros();
        let (pairs, staged) = scratch.split_at_mut(16 * 32);
        let count = coded * w;
        match (plan.dequant.is_some(), plan.rescale) {
            (true, false) => stage::<true, false>(t, coeffs, &dq, count, staged),
            (true, true) => stage::<true, true>(t, coeffs, &dq, count, staged),
            (false, false) => stage::<false, false>(t, coeffs, &dq, count, staged),
            (false, true) => stage::<false, true>(t, coeffs, &dq, count, staged),
        }
        let mask = rows(t, plan, staged, coded, pairs);
        if mask == 0 {
            if let Some(residual) = residual {
                residual.fill(0);
            }
            return;
        }
        let round = plan.col_shift;
        macro_rules! cols {
            ($add:literal, $out:expr, $residual:expr) => {
                match (plan.col, h) {
                    (Col::Identity(s), 4) => col_pass::<4, 2, IDENTITY, $add>(
                        t,
                        w,
                        (&DCT4, s),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Identity(s), 8) => col_pass::<8, 4, IDENTITY, $add>(
                        t,
                        w,
                        (&DCT8, s),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Identity(s), 16) => col_pass::<16, 8, IDENTITY, $add>(
                        t,
                        w,
                        (&DCT16, s),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Identity(s), _) => col_pass::<32, 16, IDENTITY, $add>(
                        t,
                        w,
                        (&DCT32, s),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Dct, 4) => col_pass::<4, 2, SPLIT, $add>(
                        t,
                        w,
                        (&DCT4, 0),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Dct, 8) => col_pass::<8, 4, SPLIT, $add>(
                        t,
                        w,
                        (&DCT8, 0),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Dct, 16) => col_pass::<16, 8, SPLIT, $add>(
                        t,
                        w,
                        (&DCT16, 0),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Dct, _) => col_pass::<32, 16, SPLIT, $add>(
                        t,
                        w,
                        (&DCT32, 0),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Dense(PairKernel::N4(k)), _) => col_pass::<4, 2, DENSE, $add>(
                        t,
                        w,
                        (k, 0),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Dense(PairKernel::N8(k)), _) => col_pass::<8, 4, DENSE, $add>(
                        t,
                        w,
                        (k, 0),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Dense(PairKernel::N16(k)), _) => col_pass::<16, 8, DENSE, $add>(
                        t,
                        w,
                        (k, 0),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                    (Col::Dense(PairKernel::N32(k)), _) => col_pass::<32, 16, DENSE, $add>(
                        t,
                        w,
                        (k, 0),
                        pairs,
                        mask,
                        round,
                        $out,
                        $residual,
                    ),
                }
            };
        }
        match residual {
            None => cols!(true, (dst, stride), &mut empty),
            Some(residual) => cols!(false, (dst, 0), residual),
        }
    }
);
