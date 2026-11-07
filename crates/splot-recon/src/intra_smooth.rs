// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Smooth intra prediction primitive.
//!
//! Feature tracking: `RECON-INTRA-SMOOTH-PREDICTION`.

#[cfg(target_arch = "x86_64")]
use fearless_simd::u8x16;
use fearless_simd::{Level, Simd, SimdBase, dispatch, u8x32, u16x8, u16x16};

use crate::intra_dc_math::{IntraLaneSample, validate_output_shape, validate_sample_type};
use crate::math::round2_i32;
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

const BLEND_WEIGHT_MAX: i32 = 32;

/// Widest § 7.13.2.13 block side, the largest transform dimension.
const MAX_SMOOTH_SIDE: usize = 64;

/// Smooth intra prediction mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraSmoothMode {
    /// AV2 `SMOOTH_PRED`.
    Smooth,
    /// AV2 `SMOOTH_V_PRED`.
    SmoothVertical,
    /// AV2 `SMOOTH_H_PRED`.
    SmoothHorizontal,
}

impl IntraSmoothMode {
    /// Returns a stable human-readable mode name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Smooth => "smooth",
            Self::SmoothVertical => "smooth_v",
            Self::SmoothHorizontal => "smooth_h",
        }
    }
}

/// Edge identifier for smooth intra prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraSmoothEdge {
    /// Left edge samples `LeftCol[0..h)`.
    Left,
    /// Above edge samples `AboveRow[0..w)`.
    Above,
    /// Bottom-left sentinel sample `LeftCol[h]`.
    BottomLeft,
    /// Top-right sentinel sample `AboveRow[w]`.
    TopRight,
}

impl IntraSmoothEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
            Self::BottomLeft => "bottom-left",
            Self::TopRight => "top-right",
        }
    }
}

/// Caller-provided prepared edge samples for AV2 §7.13.2.13 smooth prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraSmoothEdges<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
}

impl<'a, T: ReconSample> IntraSmoothEdges<'a, T> {
    /// Creates a prepared smooth edge set.
    ///
    /// `left` must contain `LeftCol[0..h]`, including the bottom-left
    /// `LeftCol[h]` sentinel. `above` must contain `AboveRow[0..w]`,
    /// including the top-right `AboveRow[w]` sentinel. Availability and
    /// fallback preparation remain outside this type and are owned by the
    /// broader AV2 §7.13.2.1 intra process.
    pub const fn new(left: &'a [T], above: &'a [T]) -> Self {
        Self { left, above }
    }

    /// Returns prepared left edge samples, including `LeftCol[h]`.
    pub const fn left_samples(self) -> &'a [T] {
        self.left
    }

    /// Returns prepared above edge samples, including `AboveRow[w]`.
    pub const fn above_samples(self) -> &'a [T] {
        self.above
    }
}

/// Writes rectangular AV2 §7.13.2.13 smooth prediction into caller storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples or predictions, a too-small
/// stride, or a too-small output buffer.
pub fn predict_intra_smooth_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    edges: IntraSmoothEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_edges(bit_depth, size, edges)?;
    validate_output_shape(
        size,
        output.len(),
        stride_samples,
        "smooth intra prediction output buffer length",
    )?;

    if bit_depth.max_sample() <= MAX_LANE_SAMPLE {
        let mut lanes = IntraSmoothLanes::new(size);
        let (left, above) = (edges.left_samples(), edges.above_samples());
        lanes.left[..size.height()].copy_from_slice(&left[..size.height()]); // splot-copy-ok: smooth left lanes
        lanes.set_above(&above[..size.width()]);
        lanes.corners = [above[size.width()], left[size.height()]];
        predict_lanes(&lanes, mode, output, stride_samples);
        return Ok(());
    }
    for row in 0..size.height() {
        let row_start = row
            .checked_mul(stride_samples)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "smooth intra prediction output row offset",
            })?;
        for column in 0..size.width() {
            output[row_start + column] =
                predict_smooth_sample(bit_depth, size, mode, edges, row, column)?;
        }
    }

    Ok(())
}

/// Largest sample whose § 7.13.2.13 lane terms fit `u16`: every stage below
/// is a weighted sum of samples whose weights total at most 64, plus a
/// rounding half, so `1023 * 64 + 32` bounds every lane.
const MAX_LANE_SAMPLE: u16 = 1023;
/// Lanes of every per-column and per-row table: a 64-sample side and the
/// eight-row reads of its last group.
const LANE_LEN: usize = MAX_SMOOTH_SIDE + 16;

/// `ROW_INDEX[w]` broadcasts a row term over the `w` lanes of each row in a
/// group of `16 / w` rows: lane `j` reads the `u16` row term `j / w` of its
/// 128-bit block, which holds the group's rows from its first.
const fn row_index(width: usize) -> [u8; 32] {
    let mut index = [0u8; 32];
    let mut byte = 0;
    while byte < 32 {
        index[byte] = (2 * (byte / 2 / width) + byte % 2) as u8;
        byte += 1;
    }
    index
}

const ROW_INDEX_4: [u8; 32] = row_index(4);
const ROW_INDEX_8: [u8; 32] = row_index(8);

/// The § 7.13.2.13 weights of position `j` along an `n = 4 << k` side, for
/// every lane `j` of a table: `FAR[k][j] = n - 1 - j'` and `NEAR[k][j] = j' +
/// 1` with `j' = j mod n`, so sides narrower than the lanes repeat.
static FAR: [[u16; LANE_LEN]; 5] = side_table(true);
static NEAR: [[u16; LANE_LEN]; 5] = side_table(false);

/// `BLEND[scale][k][j]` is the `sLeft`/`sTop` blend weight of position `j mod
/// n` at the block's `scale`.
static BLEND: [[[u16; LANE_LEN]; 5]; 3] = {
    let mut table = [[[0u16; LANE_LEN]; 5]; 3];
    let mut scale = 0;
    while scale < 3 {
        let mut k = 0;
        while k < 5 {
            let mut j = 0;
            while j < LANE_LEN {
                let shift = ((j % (4 << k)) << 1) >> scale;
                table[scale][k][j] = (BLEND_WEIGHT_MAX >> if shift < 6 { shift } else { 6 }) as u16;
                j += 1;
            }
            k += 1;
        }
        scale += 1;
    }
    table
};

const fn side_table(far: bool) -> [[u16; LANE_LEN]; 5] {
    let mut table = [[0u16; LANE_LEN]; 5];
    let mut k = 0;
    while k < 5 {
        let mut j = 0;
        while j < LANE_LEN {
            let n = 4 << k;
            table[k][j] = (if far { n - 1 - j % n } else { j % n + 1 }) as u16;
            j += 1;
        }
        k += 1;
    }
    table
}

/// The block's weight tables: `[FAR, NEAR, BLEND]` of its width, then of its
/// height.
fn smooth_tables(size: IntraRectBlockSize) -> [[&'static [u16; LANE_LEN]; 3]; 2] {
    let scale = round2_i32(size.log2_width() as i32 + size.log2_height() as i32 - 4, 2) as usize;
    [size.log2_width(), size.log2_height()].map(|log2| {
        let k = usize::from(log2 - 2);
        [&FAR[k], &NEAR[k], &BLEND[scale][k]]
    })
}

/// One block's § 7.13.2.13 edges in the prediction kernel's lane layout:
/// `left[i]` is `LeftCol[i]`, `above[j]` is `AboveRow[j mod w]` (repeated for
/// blocks narrower than 16), and `corners` is `[AboveRow[w], LeftCol[h]]`.
///
/// Every sample must lie within the bit depth of the prediction it feeds.
#[derive(Clone, Copy, Debug)]
pub struct IntraSmoothLanes<T: ReconSample> {
    size: IntraRectBlockSize,
    /// `LeftCol[0..h)`, from index 0.
    pub left: [T; LANE_LEN],
    above: [T; LANE_LEN],
    /// `[AboveRow[w], LeftCol[h]]`.
    pub corners: [T; 2],
}

impl<T: ReconSample> IntraSmoothLanes<T> {
    /// Empty lanes for a block of `size`.
    pub fn new(size: IntraRectBlockSize) -> Self {
        Self {
            size,
            left: [T::default(); LANE_LEN],
            above: [T::default(); LANE_LEN],
            corners: [T::default(); 2],
        }
    }

    /// Sets `AboveRow[0..w)` from the first `w` samples of `row`.
    pub fn set_above(&mut self, row: &[T]) {
        fn repeat<T: Copy, const N: usize>(row: &[T], lanes: &mut [T]) {
            if let Some(row) = row.first_chunk::<N>() {
                for lane in lanes.as_chunks_mut::<N>().0 {
                    *lane = *row;
                }
            }
        }
        match self.size.width() {
            4 => repeat::<T, 4>(row, &mut self.above[..16]),
            8 => repeat::<T, 8>(row, &mut self.above[..16]),
            width => {
                if let Some(row) = row.get(..width) {
                    self.above[..width].copy_from_slice(row); // splot-copy-ok: smooth above lanes
                }
            }
        }
    }

    /// Sets every `AboveRow[0..w)` sample to `sample`.
    pub fn fill_above(&mut self, sample: T) {
        self.above.fill(sample);
    }
}

/// Writes the § 7.13.2.13 prediction of `lanes` into caller storage.
///
/// # Errors
/// Returns [`ReconError`] for a sample type that cannot represent `bit_depth`,
/// or a too-small stride or output buffer.
pub fn predict_intra_smooth_lanes_into<T: ReconSample>(
    bit_depth: BitDepth,
    mode: IntraSmoothMode,
    lanes: &IntraSmoothLanes<T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(
        lanes.size,
        output.len(),
        stride_samples,
        "smooth intra prediction output buffer length",
    )?;
    predict_lanes(lanes, mode, output, stride_samples);
    Ok(())
}

fn predict_lanes<T: ReconSample>(
    lanes: &IntraSmoothLanes<T>,
    mode: IntraSmoothMode,
    output: &mut [T],
    stride: usize,
) {
    if let (Some(left), Some(above), Some(corners), Some(target)) = (
        T::u8_slice(&lanes.left),
        T::u8_slice(&lanes.above),
        T::u8_slice(&lanes.corners),
        T::u8_slice_mut(output),
    ) {
        predict_smooth_u8(
            lanes.size,
            mode,
            left,
            above,
            [corners[0], corners[1]],
            target,
            stride,
        );
    } else if let (Some(left), Some(above), Some(corners), Some(target)) = (
        T::u16_slice(&lanes.left),
        T::u16_slice(&lanes.above),
        T::u16_slice(&lanes.corners),
        T::u16_slice_mut(output),
    ) {
        predict_smooth_lanes(
            lanes.size,
            mode,
            left,
            above,
            [corners[0], corners[1]],
            target,
            stride,
        );
    }
}

/// Writes the whole § 7.13.2.13 prediction for samples of at most
/// [`MAX_LANE_SAMPLE`], 16 lanes at a time: 16 columns of one row, or `16 /
/// w` rows of a `w`-wide block.
///
/// With `predV = bottomLeft + Round2((top - bottomLeft) * (h - 1 - i),
/// log2(h))`, adding `bottomLeft * h` inside the `Round2` gives `Round2(top *
/// (h - 1 - i) + bottomLeft * (i + 1), log2(h))`, and each `x + Round2((y - x) *
/// s, 6)` blend is `Round2(x * (64 - s) + y * s, 6)`, so no lane goes negative.
#[inline(never)]
fn predict_smooth_lanes(
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    left: &[u16],
    above: &[u16],
    corners: [u16; 2],
    output: &mut [u16],
    stride: usize,
) {
    let (Ok(left), Ok(above)) = (
        <&[u16; LANE_LEN]>::try_from(left),
        <&[u16; LANE_LEN]>::try_from(above),
    ) else {
        return;
    };
    let lanes = [*left, *above];
    dispatch!(Level::baseline(), simd => smooth_lanes(simd, size, mode, &lanes, corners, output, stride));
}

#[inline]
fn smooth_lanes<S: Simd, P: IntraLaneSample>(
    simd: S,
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    [left, above]: &[[u16; LANE_LEN]; 2],
    [right, bottom]: [u16; 2],
    output: &mut [P],
    stride: usize,
) {
    let (width, height) = (size.width(), size.height());
    let (log2_width, log2_height) = (u32::from(size.log2_width()), u32::from(size.log2_height()));
    let [[h_far, h_near, h_blend], [v_far, v_near, v_blend]] = smooth_tables(size);
    let rows_per_group = (16 / width).max(1);
    let index = match width {
        4 => Some(u8x32::from_slice(simd, &ROW_INDEX_4)),
        8 => Some(u8x32::from_slice(simd, &ROW_INDEX_8)),
        _ => None,
    };
    let rows = |row_terms: &[u16; LANE_LEN], row: usize| match index {
        Some(index) => u16x16::block_splat(u16x8::from_slice(simd, &row_terms[row..row + 8]))
            .swizzle_dyn_within_blocks(index),
        None => u16x16::splat(simd, row_terms[row]),
    };
    let full = u16x16::splat(simd, 64);
    let blend = |pred: u16x16<S>, edge: u16x16<S>, weight: u16x16<S>| {
        (pred * (full - weight) + edge * weight + 32) >> 6
    };
    let (round_h, round_v) = (1u16 << (log2_width - 1), 1u16 << (log2_height - 1));
    for chunk in (0..width).step_by(16) {
        let columns = |column_terms: &[u16; LANE_LEN]| {
            u16x16::from_slice(simd, &column_terms[chunk..chunk + 16])
        };
        let (top, far, s_left) = (columns(above), columns(h_far), columns(h_blend));
        let right_near = columns(h_near) * right + round_h;
        for row in (0..height).step_by(rows_per_group) {
            let vertical = || {
                let bottom_near = rows(v_near, row) * bottom + round_v;
                let pred = (top * rows(v_far, row) + bottom_near) >> log2_height;
                blend(pred, top, rows(v_blend, row))
            };
            let horizontal = || {
                let left = rows(left, row);
                blend((left * far + right_near) >> log2_width, left, s_left)
            };
            let value = match mode {
                IntraSmoothMode::SmoothVertical => vertical(),
                IntraSmoothMode::SmoothHorizontal => horizontal(),
                IntraSmoothMode::Smooth => (vertical() + horizontal() + 1) >> 1,
            };
            let start = row * stride + chunk;
            if width >= 16 {
                if let Some(out) = output.get_mut(start..).and_then(<[P]>::first_chunk_mut) {
                    P::store16(simd, value, out);
                }
                continue;
            }
            let mut lanes = [P::default(); 16];
            P::store16(simd, value, &mut lanes);
            for (r, samples) in lanes.chunks_exact(width).enumerate() {
                let start = start + r * stride;
                if let Some(out) = output.get_mut(start..start + width) {
                    out.copy_from_slice(samples); // splot-copy-ok: publish one smooth row
                }
            }
        }
    }
}

/// 8-bit § 7.13.2.13, through the AVX2 rounding-multiply kernel when the
/// baseline has it.
#[inline(never)]
fn predict_smooth_u8(
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    left: &[u8],
    above: &[u8],
    corners: [u8; 2],
    output: &mut [u8],
    stride: usize,
) {
    let (Ok(left), Ok(above)) = (
        <&[u8; LANE_LEN]>::try_from(left),
        <&[u8; LANE_LEN]>::try_from(above),
    ) else {
        return;
    };
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = Level::baseline().as_avx2() {
        return smooth_u8_avx2(token, size, mode, [left, above], corners, output, stride);
    }
    let lanes = [left.map(u16::from), above.map(u16::from)];
    let corners = corners.map(u16::from);
    dispatch!(Level::baseline(), simd => smooth_lanes(simd, size, mode, &lanes, corners, output, stride));
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// [`smooth_lanes`] for `u8` with `_mm256_mulhrs_epi16`: `mulhrs(d, f <<
    /// (15 - n))` is `Round2(d * f, n)` for `|d| <= 255` and `0 <= f < 2^n`, so
    /// each § 7.13.2.13 `x + Round2((y - x) * f, n)` stage is one rounding
    /// multiply and one add.
    #[inline]
    fn smooth_u8_avx2(
        token: Avx2,
        size: IntraRectBlockSize,
        mode: IntraSmoothMode,
        lanes: [&[u8; LANE_LEN]; 2],
        corners: [u8; 2],
        output: &mut [u8],
        stride: usize,
    ) {
        use core::arch::x86_64::{
            __m256i, _mm_cvtepu8_epi16, _mm_cvtsi32_si128, _mm_packus_epi16, _mm256_add_epi16,
            _mm256_avg_epu16, _mm256_broadcastsi128_si256, _mm256_castsi256_si128,
            _mm256_cvtepu8_epi16, _mm256_extracti128_si256, _mm256_mulhrs_epi16, _mm256_set1_epi16,
            _mm256_shuffle_epi8, _mm256_sll_epi16, _mm256_slli_epi16, _mm256_sub_epi16,
        };
        use fearless_simd::{SimdFrom, u16x8};
        let (width, height) = (size.width(), size.height());
        let [[h_far, _, h_blend], [v_far, _, v_blend]] = smooth_tables(size);
        let h_shift = _mm_cvtsi32_si128(15 - i32::from(size.log2_width()));
        let v_shift = _mm_cvtsi32_si128(15 - i32::from(size.log2_height()));
        let [left, above] = lanes;
        let right = _mm256_set1_epi16(i16::from(corners[0]));
        let bottom = _mm256_set1_epi16(i16::from(corners[1]));
        let rows_per_group = (16 / width).max(1);
        let index: Option<__m256i> = match width {
            4 => Some(u8x32::from_slice(token, &ROW_INDEX_4).into()),
            8 => Some(u8x32::from_slice(token, &ROW_INDEX_8).into()),
            _ => None,
        };
        macro_rules! rows {
            ($terms:expr, $row:expr) => {
                match index {
                    Some(index) => _mm256_shuffle_epi8(
                        _mm256_broadcastsi128_si256(
                            u16x8::from_slice(token, &$terms[$row..$row + 8]).into(),
                        ),
                        index,
                    ),
                    None => _mm256_set1_epi16($terms[$row] as i16),
                }
            };
        }
        macro_rules! left_rows {
            ($row:expr) => {
                match index {
                    Some(index) => _mm256_shuffle_epi8(
                        _mm256_broadcastsi128_si256(_mm_cvtepu8_epi16(
                            u8x16::from_slice(token, &left[$row..$row + 16]).into(),
                        )),
                        index,
                    ),
                    None => _mm256_set1_epi16(i16::from(left[$row])),
                }
            };
        }
        for chunk in (0..width).step_by(16) {
            macro_rules! columns {
                ($terms:expr) => {{
                    let lanes: __m256i =
                        u16x16::from_slice(token, &$terms[chunk..chunk + 16]).into();
                    lanes
                }};
            }
            let top =
                _mm256_cvtepu8_epi16(u8x16::from_slice(token, &above[chunk..chunk + 16]).into());
            let top_gap = _mm256_sub_epi16(top, bottom);
            let far = _mm256_sll_epi16(columns!(h_far), h_shift);
            let s_left = _mm256_slli_epi16::<9>(columns!(h_blend));
            for row in (0..height).step_by(rows_per_group) {
                macro_rules! vertical {
                    () => {{
                        let v_far = _mm256_sll_epi16(rows!(v_far, row), v_shift);
                        let pred = _mm256_add_epi16(bottom, _mm256_mulhrs_epi16(top_gap, v_far));
                        let s_top = _mm256_slli_epi16::<9>(rows!(v_blend, row));
                        _mm256_add_epi16(
                            pred,
                            _mm256_mulhrs_epi16(_mm256_sub_epi16(top, pred), s_top),
                        )
                    }};
                }
                macro_rules! horizontal {
                    () => {{
                        let left = left_rows!(row);
                        let gap = _mm256_sub_epi16(left, right);
                        let pred = _mm256_add_epi16(right, _mm256_mulhrs_epi16(gap, far));
                        _mm256_add_epi16(
                            pred,
                            _mm256_mulhrs_epi16(_mm256_sub_epi16(left, pred), s_left),
                        )
                    }};
                }
                let value = match mode {
                    IntraSmoothMode::SmoothVertical => vertical!(),
                    IntraSmoothMode::SmoothHorizontal => horizontal!(),
                    IntraSmoothMode::Smooth => _mm256_avg_epu16(vertical!(), horizontal!()),
                };
                let packed = u8x16::simd_from(
                    token,
                    _mm_packus_epi16(
                        _mm256_castsi256_si128(value),
                        _mm256_extracti128_si256::<1>(value),
                    ),
                );
                let start = row * stride + chunk;
                if width >= 16 {
                    if let Some(out) = output.get_mut(start..start + 16) {
                        packed.store_slice(out);
                    }
                    continue;
                }
                let lanes: [u8; 16] = packed.into();
                let out = output.get_mut(start..).unwrap_or_default();
                if width == 4 {
                    publish_rows::<4>(&lanes, out, stride);
                } else {
                    publish_rows::<8>(&lanes, out, stride);
                }
            }
        }
    }
);

/// Writes the `16 / N` rows of `N` samples packed in `lanes`, `stride` apart.
#[inline]
fn publish_rows<const N: usize>(lanes: &[u8; 16], output: &mut [u8], stride: usize) {
    for (row, samples) in lanes.as_chunks::<N>().0.iter().enumerate() {
        if let Some(out) = output
            .get_mut(row * stride..)
            .and_then(<[u8]>::first_chunk_mut::<N>)
        {
            *out = *samples;
        }
    }
}

pub(crate) fn predict_smooth_sample<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    edges: IntraSmoothEdges<'_, T>,
    row: usize,
    column: usize,
) -> Result<T> {
    predict_smooth_sample_values(
        bit_depth,
        size,
        mode,
        SmoothSampleEdges {
            left: edges.left[row],
            top: edges.above[column],
            bottom_left: edges.left[size.height()],
            top_right: edges.above[size.width()],
        },
        SmoothSamplePosition { row, column },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SmoothSampleEdges<T: ReconSample> {
    pub(crate) left: T,
    pub(crate) top: T,
    pub(crate) bottom_left: T,
    pub(crate) top_right: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SmoothSamplePosition {
    pub(crate) row: usize,
    pub(crate) column: usize,
}

pub(crate) fn predict_smooth_sample_values<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    samples: SmoothSampleEdges<T>,
    position: SmoothSamplePosition,
) -> Result<T> {
    let top = i32::from(samples.top.to_u16());
    let left = i32::from(samples.left.to_u16());
    let top_right = i32::from(samples.top_right.to_u16());
    let bottom_left = i32::from(samples.bottom_left.to_u16());
    let scale = round2_i32(
        i32::from(size.log2_width()) + i32::from(size.log2_height()) - 4,
        2,
    );
    let scale = u8::try_from(scale).map_err(|_| ReconError::ArithmeticOverflow {
        context: "smooth intra prediction scale",
    })?;

    let top_weight_shift = position
        .row
        .checked_shl(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "smooth intra prediction top weight row shift",
        })?
        >> scale;
    let left_weight_shift =
        position
            .column
            .checked_shl(1)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "smooth intra prediction left weight column shift",
            })?
            >> scale;
    let s_top = BLEND_WEIGHT_MAX >> core::cmp::min(6usize, top_weight_shift);
    let s_left = BLEND_WEIGHT_MAX >> core::cmp::min(6usize, left_weight_shift);
    let h_factor = i32::try_from(size.width() - 1 - position.column).map_err(|_| {
        ReconError::ArithmeticOverflow {
            context: "smooth intra prediction horizontal factor",
        }
    })?;
    let v_factor = i32::try_from(size.height() - 1 - position.row).map_err(|_| {
        ReconError::ArithmeticOverflow {
            context: "smooth intra prediction vertical factor",
        }
    })?;

    let pred_h = top_right
        + round2_i32(
            (left - top_right)
                .checked_mul(h_factor)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction horizontal product",
                })?,
            u32::from(size.log2_width()),
        );
    let pred_v = bottom_left
        + round2_i32(
            (top - bottom_left)
                .checked_mul(v_factor)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction vertical product",
                })?,
            u32::from(size.log2_height()),
        );
    let pred_h2 = pred_h
        + round2_i32(
            (left - pred_h)
                .checked_mul(s_left)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction horizontal blend product",
                })?,
            6,
        );
    let pred_v2 = pred_v
        + round2_i32(
            (top - pred_v)
                .checked_mul(s_top)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction vertical blend product",
                })?,
            6,
        );
    let predicted = match mode {
        IntraSmoothMode::SmoothHorizontal => pred_h2,
        IntraSmoothMode::SmoothVertical => pred_v2,
        IntraSmoothMode::Smooth => round2_i32(
            pred_v2
                .checked_add(pred_h2)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction combined sample",
                })?,
            1,
        ),
    };

    let max = i32::from(bit_depth.max_sample());
    if predicted < 0 || predicted > max {
        return Err(ReconError::IntraSmoothPredictionOutOfRange {
            row: position.row,
            column: position.column,
            value: predicted,
            min: 0,
            max,
        });
    }

    T::try_from_u16(predicted as u16)
}

fn validate_edges<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraSmoothEdges<'_, T>,
) -> Result<()> {
    let expected_left = size
        .height()
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "smooth intra prediction left edge length",
        })?;
    let expected_above = size
        .width()
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "smooth intra prediction above edge length",
        })?;
    if edges.left.len() != expected_left {
        return Err(ReconError::IntraSmoothEdgeLengthMismatch {
            edge: IntraSmoothEdge::Left,
            expected: expected_left,
            actual: edges.left.len(),
        });
    }
    if edges.above.len() != expected_above {
        return Err(ReconError::IntraSmoothEdgeLengthMismatch {
            edge: IntraSmoothEdge::Above,
            expected: expected_above,
            actual: edges.above.len(),
        });
    }

    if T::MAX_VALUE <= bit_depth.max_sample() {
        return Ok(());
    }
    for (sample_index, sample) in edges.left[..size.height()].iter().copied().enumerate() {
        validate_sample(IntraSmoothEdge::Left, sample_index, sample, bit_depth)?;
    }
    validate_sample(
        IntraSmoothEdge::BottomLeft,
        size.height(),
        edges.left[size.height()],
        bit_depth,
    )?;
    for (sample_index, sample) in edges.above[..size.width()].iter().copied().enumerate() {
        validate_sample(IntraSmoothEdge::Above, sample_index, sample, bit_depth)?;
    }
    validate_sample(
        IntraSmoothEdge::TopRight,
        size.width(),
        edges.above[size.width()],
        bit_depth,
    )
}

fn validate_sample<T: ReconSample>(
    edge: IntraSmoothEdge,
    sample_index: usize,
    sample: T,
    bit_depth: BitDepth,
) -> Result<()> {
    let value = sample.to_u16();
    let max = bit_depth.max_sample();
    if value > max {
        Err(ReconError::IntraSmoothSampleOutOfRange {
            edge,
            sample_index,
            value,
            max,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    fn non_uniform_edges() -> ([u8; 5], [u8; 5]) {
        ([20, 40, 60, 80, 100], [10, 30, 50, 70, 90])
    }

    fn assert_smooth_prediction(mode: IntraSmoothMode, expected: [u8; 16]) {
        let (left, above) = non_uniform_edges();
        let mut output = [0u8; 16];

        predict_intra_smooth_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            mode,
            IntraSmoothEdges::new(&left, &above),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(output, expected);
    }

    #[test]
    fn smooth_horizontal_prediction_matches_spec_formula() {
        assert_smooth_prediction(
            IntraSmoothMode::SmoothHorizontal,
            [
                29, 51, 71, 90, 47, 62, 77, 90, 64, 73, 82, 90, 82, 84, 88, 90,
            ],
        );
    }

    #[test]
    fn smooth_vertical_prediction_matches_spec_formula() {
        assert_smooth_prediction(
            IntraSmoothMode::SmoothVertical,
            [
                22, 39, 57, 74, 49, 61, 72, 83, 76, 81, 87, 92, 100, 100, 100, 100,
            ],
        );
    }

    #[test]
    fn smooth_prediction_averages_horizontal_and_vertical_paths() {
        assert_smooth_prediction(
            IntraSmoothMode::Smooth,
            [
                26, 45, 64, 82, 48, 62, 75, 87, 70, 77, 85, 91, 91, 92, 94, 95,
            ],
        );
    }

    #[test]
    fn smooth_prediction_fills_rectangle_with_stride() {
        let (left, above) = non_uniform_edges();
        let mut output = [99u8; 24];

        predict_intra_smooth_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraSmoothMode::SmoothHorizontal,
            IntraSmoothEdges::new(&left, &above),
            &mut output,
            6,
        )
        .unwrap();

        assert_eq!(&output[0..6], &[29, 51, 71, 90, 99, 99]);
        assert_eq!(&output[6..12], &[47, 62, 77, 90, 99, 99]);
        assert_eq!(&output[12..18], &[64, 73, 82, 90, 99, 99]);
        assert_eq!(&output[18..24], &[82, 84, 88, 90, 99, 99]);
    }

    #[test]
    fn smooth_prediction_validates_edge_lengths() {
        let mut output = [9u8; 16];

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&[1; 4], &[1; 5]),
                &mut output,
                4
            ),
            Err(ReconError::IntraSmoothEdgeLengthMismatch {
                edge: IntraSmoothEdge::Left,
                expected: 5,
                actual: 4
            })
        ));
        assert_eq!(output, [9u8; 16]);

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&[1; 5], &[1; 4]),
                &mut output,
                4
            ),
            Err(ReconError::IntraSmoothEdgeLengthMismatch {
                edge: IntraSmoothEdge::Above,
                expected: 5,
                actual: 4
            })
        ));
        assert_eq!(output, [9u8; 16]);
    }

    #[test]
    fn smooth_prediction_validates_edge_samples_against_bit_depth() {
        let mut output = [0u16; 16];

        for (left, above, expected_edge, expected_index) in [
            ([1, 300, 1, 1, 1], [1; 5], IntraSmoothEdge::Left, 1),
            ([1; 5], [1, 300, 1, 1, 1], IntraSmoothEdge::Above, 1),
            ([1, 1, 1, 1, 300], [1; 5], IntraSmoothEdge::BottomLeft, 4),
            ([1; 5], [1, 1, 1, 1, 300], IntraSmoothEdge::TopRight, 4),
        ] {
            assert!(matches!(
                predict_intra_smooth_rect_into(
                    BitDepth::Eight,
                    rect_size(2, 2),
                    IntraSmoothMode::Smooth,
                    IntraSmoothEdges::new(&left, &above),
                    &mut output,
                    4
                ),
                Err(ReconError::IntraSmoothSampleOutOfRange {
                    edge,
                    sample_index,
                    value: 300,
                    max: 255
                }) if edge == expected_edge && sample_index == expected_index
            ));
        }
    }

    #[test]
    fn smooth_prediction_validates_computed_sample_range() {
        assert!(matches!(
            predict_smooth_sample_values(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::SmoothHorizontal,
                SmoothSampleEdges {
                    left: 65535u16,
                    top: 0u16,
                    bottom_left: 0u16,
                    top_right: 65535u16,
                },
                SmoothSamplePosition { row: 0, column: 0 }
            ),
            Err(ReconError::IntraSmoothPredictionOutOfRange {
                row: 0,
                column: 0,
                value,
                min: 0,
                max: 255
            }) if value > 255
        ));
    }

    /// The `u16` lane groups must reproduce the per-sample § 7.13.2.13
    /// reference exactly for every block shape, mode, and bit depth, over
    /// randomized edges and a strided output; `u8` storage must match at 8 bits.
    #[test]
    fn u16_lane_groups_match_the_per_sample_reference() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
            let span = u64::from(bit_depth.max_sample()) + 1;
            for log2_width in 2..=6u8 {
                for log2_height in 2..=6u8 {
                    let size = rect_size(log2_width, log2_height);
                    let left: Vec<u16> = (0..=size.height())
                        .map(|_| (next() % span) as u16)
                        .collect();
                    let above: Vec<u16> =
                        (0..=size.width()).map(|_| (next() % span) as u16).collect();
                    for mode in [
                        IntraSmoothMode::Smooth,
                        IntraSmoothMode::SmoothVertical,
                        IntraSmoothMode::SmoothHorizontal,
                    ] {
                        let stride = size.width() + 3;
                        let mut actual = vec![0u16; stride * size.height()];
                        predict_intra_smooth_rect_into(
                            bit_depth,
                            size,
                            mode,
                            IntraSmoothEdges::new(&left, &above),
                            &mut actual,
                            stride,
                        )
                        .unwrap();
                        for row in 0..size.height() {
                            for column in 0..size.width() {
                                let expected: u16 = predict_smooth_sample(
                                    bit_depth,
                                    size,
                                    mode,
                                    IntraSmoothEdges::new(&left, &above),
                                    row,
                                    column,
                                )
                                .unwrap();
                                assert_eq!(
                                    actual[row * stride + column],
                                    expected,
                                    "{}x{} {mode:?} {bit_depth:?} at ({row}, {column})",
                                    size.width(),
                                    size.height()
                                );
                            }
                        }
                        if bit_depth == BitDepth::Eight {
                            let narrow_left: Vec<u8> = left.iter().map(|&v| v as u8).collect();
                            let narrow_above: Vec<u8> = above.iter().map(|&v| v as u8).collect();
                            let mut narrow = vec![0u8; actual.len()];
                            predict_intra_smooth_rect_into(
                                bit_depth,
                                size,
                                mode,
                                IntraSmoothEdges::new(&narrow_left, &narrow_above),
                                &mut narrow,
                                stride,
                            )
                            .unwrap();
                            assert!(
                                narrow
                                    .iter()
                                    .map(|&v| u16::from(v))
                                    .eq(actual.iter().copied())
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn smooth_prediction_validates_sample_type_against_bit_depth() {
        let mut output = [0u8; 16];

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Ten,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&[1; 5], &[1; 5]),
                &mut output,
                4
            ),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
    }

    #[test]
    fn smooth_prediction_validates_output_stride_and_length() {
        let (left, above) = non_uniform_edges();
        let mut output = [9u8; 15];

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&left, &above),
                &mut output,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));
        assert_eq!(output, [9u8; 15]);

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&left, &above),
                &mut output,
                4
            ),
            Err(ReconError::IntraPredictionOutputTooSmall {
                expected: 16,
                actual: 15
            })
        ));
        assert_eq!(output, [9u8; 15]);
    }

    #[test]
    fn smooth_prediction_rejects_overflowing_output_shape() {
        let mut output = [0u8; 0];

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(6, 6),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&[1; 65], &[1; 65]),
                &mut output,
                usize::MAX
            ),
            Err(ReconError::ArithmeticOverflow {
                context: "smooth intra prediction output buffer length"
            })
        ));
    }
}
