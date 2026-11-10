// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.29 intra mode variant mask and § 7.13.3.30 interintra blend.

use super::{CurrentFramePlane, CurrentFrameWorkspace, checked_sample_block_rect};
#[cfg(target_arch = "x86_64")]
use crate::intra_smooth::publish_rows;
use crate::math::round2_i32;
use crate::{IntraRectBlockSize, PlaneId, ReconError, ReconSample, Result};
use fearless_simd::{Simd, SimdBase, i32x4, i32x8, i32x16};

/// AV2 § 7.13.3.29 `Ii_Weights_1d[128]` smooth-interintra blending weights.
const II_WEIGHTS_1D: [u16; 128] = [
    60, 58, 56, 54, 52, 50, 48, 47, 45, 44, 42, 41, 39, 38, 37, 35, 34, 33, 32, 31, 30, 29, 28, 27,
    26, 25, 24, 23, 22, 22, 21, 20, 19, 19, 18, 18, 17, 16, 16, 15, 15, 14, 14, 13, 13, 12, 12, 12,
    11, 11, 10, 10, 10, 9, 9, 9, 8, 8, 8, 8, 7, 7, 7, 7, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 4, 4, 4, 4,
    4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];
const MASK_MASTER_SIZE: usize = 128;
const MAX_WEDGE_TYPES: usize = 68;
const WEDGE_BOUNDARY_SHARP: usize = 0;
const WEDGE_BOUNDARY_SMOOTH: usize = 1;
const WEDGE_COS_LUT_ALL: [[i32; 20]; 2] = [
    [
        32, 32, 32, 16, 16, 0, -16, -16, -32, -32, -32, -32, -32, -16, -16, 0, 16, 16, 32, 32,
    ],
    [
        16, 16, 16, 8, 8, 0, -8, -8, -16, -16, -16, -16, -16, -8, -8, 0, 8, 8, 16, 16,
    ],
];
const WEDGE_SIN_LUT_ALL: [[i32; 20]; 2] = [
    [
        0, -8, -16, -16, -32, -32, -32, -16, -16, -8, 0, 8, 16, 16, 32, 32, 32, 16, 16, 8,
    ],
    [
        0, -4, -8, -8, -16, -16, -16, -8, -8, -4, 0, 4, 8, 8, 16, 16, 16, 8, 8, 4,
    ],
];
const POS_DIST_2_BLD_WEIGHT: [u16; 128] = [
    8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16,
];
const NEG_DIST_2_BLD_WEIGHT: [u16; 128] = [
    8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
const WEDGE_CODEBOOK: [(usize, usize, usize); MAX_WEDGE_TYPES] = [
    (0, 5, 4),
    (0, 6, 4),
    (0, 7, 4),
    (1, 4, 4),
    (1, 5, 4),
    (1, 6, 4),
    (1, 7, 4),
    (2, 4, 4),
    (2, 5, 4),
    (2, 6, 4),
    (2, 7, 4),
    (3, 4, 4),
    (3, 5, 4),
    (3, 6, 4),
    (3, 7, 4),
    (4, 4, 4),
    (4, 4, 3),
    (4, 4, 2),
    (4, 4, 1),
    (5, 4, 3),
    (5, 4, 2),
    (5, 4, 1),
    (6, 4, 4),
    (6, 4, 3),
    (6, 4, 2),
    (6, 4, 1),
    (7, 4, 4),
    (7, 3, 4),
    (7, 2, 4),
    (7, 1, 4),
    (8, 4, 4),
    (8, 3, 4),
    (8, 2, 4),
    (8, 1, 4),
    (9, 4, 4),
    (9, 3, 4),
    (9, 2, 4),
    (9, 1, 4),
    (10, 3, 4),
    (10, 2, 4),
    (10, 1, 4),
    (11, 3, 4),
    (11, 2, 4),
    (11, 1, 4),
    (12, 3, 4),
    (12, 2, 4),
    (12, 1, 4),
    (13, 3, 4),
    (13, 2, 4),
    (13, 1, 4),
    (14, 4, 5),
    (14, 4, 6),
    (14, 4, 7),
    (15, 4, 5),
    (15, 4, 6),
    (15, 4, 7),
    (16, 4, 5),
    (16, 4, 6),
    (16, 4, 7),
    (17, 5, 4),
    (17, 6, 4),
    (17, 7, 4),
    (18, 5, 4),
    (18, 6, 4),
    (18, 7, 4),
    (19, 5, 4),
    (19, 6, 4),
    (19, 7, 4),
];

/// The § 5.20.7.15 `interintra_mode` selecting the § 7.13.3.29 mask variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterIntraMode {
    /// `II_DC_PRED`: constant weight 32.
    Dc,
    /// `II_V_PRED`: weights decay downward from the above edge.
    Vertical,
    /// `II_H_PRED`: weights decay rightward from the left edge.
    Horizontal,
    /// `II_SMOOTH_PRED`: weights decay along `Min(i, j)`.
    Smooth,
}

/// Returns the § 7.13.3.29 `Mask[i][j]` blending weight for one sample.
fn intra_mode_variant_weight(mode: InterIntraMode, i: usize, j: usize, size_scale: usize) -> u16 {
    let index = match mode {
        InterIntraMode::Dc => return 32,
        InterIntraMode::Vertical => i * size_scale,
        InterIntraMode::Horizontal => j * size_scale,
        InterIntraMode::Smooth => i.min(j) * size_scale,
    };
    II_WEIGHTS_1D[index.min(II_WEIGHTS_1D.len() - 1)]
}

fn wedge_boundary_index(luma_width: usize, luma_height: usize) -> usize {
    if luma_width <= 16 && luma_height <= 16 {
        WEDGE_BOUNDARY_SHARP
    } else {
        WEDGE_BOUNDARY_SMOOTH
    }
}

fn wedge_mask_luma_sample(
    luma_width: usize,
    luma_height: usize,
    wedge_index: usize,
    sign: bool,
    x: usize,
    y: usize,
) -> Result<u16> {
    let (direction, x_offset, y_offset) =
        WEDGE_CODEBOOK
            .get(wedge_index)
            .copied()
            .ok_or(ReconError::ArithmeticOverflow {
                context: "interintra wedge index",
            })?;
    let boundary = wedge_boundary_index(luma_width, luma_height);
    let master_x = MASK_MASTER_SIZE / 2 - ((x_offset * luma_width) >> 3) + x;
    let master_y = MASK_MASTER_SIZE / 2 - ((y_offset * luma_height) >> 3) + y;
    let centered_x =
        i32::try_from((master_x << 1) + 1).map_err(|_| ReconError::ArithmeticOverflow {
            context: "interintra wedge mask x coordinate",
        })? - i32::try_from(MASK_MASTER_SIZE).map_err(|_| ReconError::ArithmeticOverflow {
            context: "interintra wedge mask master size",
        })?;
    let centered_y =
        i32::try_from((master_y << 1) + 1).map_err(|_| ReconError::ArithmeticOverflow {
            context: "interintra wedge mask y coordinate",
        })? - i32::try_from(MASK_MASTER_SIZE).map_err(|_| ReconError::ArithmeticOverflow {
            context: "interintra wedge mask master size",
        })?;
    let d = centered_x * WEDGE_COS_LUT_ALL[boundary][direction]
        + centered_y * WEDGE_SIN_LUT_ALL[boundary][direction];
    let clamp_d = d.clamp(-127, 127);
    let base = if clamp_d >= 0 {
        POS_DIST_2_BLD_WEIGHT[usize::try_from(clamp_d).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "interintra wedge positive distance",
            }
        })?]
    } else {
        NEG_DIST_2_BLD_WEIGHT[usize::try_from(-clamp_d).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "interintra wedge negative distance",
            }
        })?]
    } << 2;
    Ok(if sign { 64 - base } else { base })
}

/// Returns one AV2 wedge-mask weight for a luma or chroma plane sample.
///
/// `sign` selects the inverse interinter wedge mask; interintra uses the base
/// mask with `sign == false`.
///
/// # Errors
/// Returns [`ReconError`] when the luma block size or wedge index does not
/// select a valid AV2 wedge mask.
#[allow(clippy::too_many_arguments)]
pub fn wedge_mask_plane_sample(
    luma_width: usize,
    luma_height: usize,
    wedge_index: usize,
    sign: bool,
    sub_x: u32,
    sub_y: u32,
    x: usize,
    y: usize,
) -> Result<u16> {
    if sub_x == 0 {
        return wedge_mask_luma_sample(luma_width, luma_height, wedge_index, sign, x, y);
    }
    let x0 = x << sub_x;
    let y0 = y << sub_y;
    if sub_y == 0 {
        let sum = i32::from(wedge_mask_luma_sample(
            luma_width,
            luma_height,
            wedge_index,
            sign,
            x0,
            y0,
        )?) + i32::from(wedge_mask_luma_sample(
            luma_width,
            luma_height,
            wedge_index,
            sign,
            x0 + 1,
            y0,
        )?);
        return Ok(round2_i32(sum, 1) as u16);
    }
    let sum = i32::from(wedge_mask_luma_sample(
        luma_width,
        luma_height,
        wedge_index,
        sign,
        x0,
        y0,
    )?) + i32::from(wedge_mask_luma_sample(
        luma_width,
        luma_height,
        wedge_index,
        sign,
        x0 + 1,
        y0,
    )?) + i32::from(wedge_mask_luma_sample(
        luma_width,
        luma_height,
        wedge_index,
        sign,
        x0,
        y0 + 1,
    )?) + i32::from(wedge_mask_luma_sample(
        luma_width,
        luma_height,
        wedge_index,
        sign,
        x0 + 1,
        y0 + 1,
    )?);
    Ok(round2_i32(sum, 2) as u16)
}

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Blends caller-supplied intra prediction samples over the in-storage
    /// inter prediction with the AV2 § 7.13.3.29 smooth-interintra mask:
    /// `CurrFrame = Round2(m * IntraPred + (64 - m) * CurrFrame, 6)`
    /// (§ 7.13.3.30 interintra arm; the mask is regenerated at this plane's
    /// dimensions, so no chroma subsampling applies).
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target rectangle
    /// is out of bounds, `intra.len()` is not the rectangular sample count, or
    /// a blended sample cannot be stored.
    pub fn blend_smooth_interintra_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        mode: InterIntraMode,
        intra: &[T],
    ) -> Result<()> {
        let (rect, target) = self.interintra_target_rect(plane, x, y, size, intra.len())?;
        target.blend_smooth_interintra_rect(rect, size, mode, intra)
    }

    /// Blends caller-supplied intra prediction samples over the in-storage inter
    /// prediction with the AV2 § 7.13.3.27 wedge mask and § 7.13.3.30
    /// interintra blend. Chroma planes average the luma mask when subsampled.
    ///
    /// # Errors
    /// Returns [`ReconError`] when geometry is invalid, the wedge index is
    /// outside the AVM codebook, or a blended sample cannot be stored.
    #[allow(clippy::too_many_arguments)]
    pub fn blend_wedge_interintra_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        luma_width: usize,
        luma_height: usize,
        wedge_index: usize,
        sub_x: u32,
        sub_y: u32,
        intra: &[T],
    ) -> Result<()> {
        let (rect, target) = self.interintra_target_rect(plane, x, y, size, intra.len())?;
        target.blend_wedge_interintra_rect(
            rect,
            size,
            luma_width,
            luma_height,
            wedge_index,
            sub_x,
            sub_y,
            intra,
        )
    }

    fn interintra_target_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        intra_len: usize,
    ) -> Result<(crate::PlaneRect, &mut CurrentFramePlane<T>)> {
        let rect = checked_sample_block_rect(plane, x, y, size, intra_len)?;
        let target = self.plane_mut(plane)?;
        let rect = target.clamp_rect_to_storage(rect)?;
        Ok((rect, target))
    }
}

impl<T: ReconSample> CurrentFramePlane<T> {
    fn blend_smooth_interintra_rect(
        &mut self,
        rect: crate::PlaneRect,
        size: IntraRectBlockSize,
        mode: InterIntraMode,
        intra: &[T],
    ) -> Result<()> {
        let size_scale = 128 / size.width().max(size.height());
        self.blend_interintra_rect_by_mask(
            rect,
            size,
            intra,
            "interintra blend sample storage",
            |i, j| Ok(intra_mode_variant_weight(mode, i, j, size_scale)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn blend_wedge_interintra_rect(
        &mut self,
        rect: crate::PlaneRect,
        size: IntraRectBlockSize,
        luma_width: usize,
        luma_height: usize,
        wedge_index: usize,
        sub_x: u32,
        sub_y: u32,
        intra: &[T],
    ) -> Result<()> {
        self.blend_interintra_rect_by_mask(
            rect,
            size,
            intra,
            "interintra wedge blend sample storage",
            |i, j| {
                wedge_mask_plane_sample(
                    luma_width,
                    luma_height,
                    wedge_index,
                    false,
                    sub_x,
                    sub_y,
                    j,
                    i,
                )
            },
        )
    }

    fn blend_interintra_rect_by_mask(
        &mut self,
        rect: crate::PlaneRect,
        size: IntraRectBlockSize,
        intra: &[T],
        overflow_context: &'static str,
        mut mask_at: impl FnMut(usize, usize) -> Result<u16>,
    ) -> Result<()> {
        self.ensure_rect(rect)?;
        for i in 0..rect.height() {
            let row_start = self.sample_index(rect.x(), rect.y() + i)?;
            for j in 0..rect.width() {
                let m = mask_at(i, j)?;
                let pred0 = u32::from(intra[i * size.width() + j].to_u16());
                let pred1 = u32::from(self.samples[row_start + j].to_u16());
                let blended = (u32::from(m) * pred0 + (64 - u32::from(m)) * pred1 + 32) >> 6;
                let stored = u16::try_from(blended)
                    .ok()
                    .and_then(|value| T::try_from_u16(value).ok())
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: overflow_context,
                    })?;
                self.samples[row_start + j] = stored;
            }
        }
        Ok(())
    }
}

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// AV2 § 7.13.3.25 block adaptive weighted prediction application:
    /// `CurrFrame = Clip1((orig * alpha + beta) >> 8)` in place over the
    /// rect, dropping frame-edge overhang like the reconstruction writes. A
    /// rect whose origin lies outside storage scales nothing.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent or a scaled sample
    /// cannot be stored.
    pub fn apply_bawp_rect(
        &mut self,
        plane: PlaneId,
        rect: crate::PlaneRect,
        alpha: i16,
        beta: i32,
    ) -> Result<()> {
        let max_sample = i32::from(self.info().bit_depth().max_sample());
        let plane = self.plane_mut(plane)?;
        let storage = plane.storage_size();
        if rect.x() >= storage.width() || rect.y() >= storage.height() {
            return Ok(());
        }
        plane.apply_bawp_rect(rect, alpha, beta, max_sample)
    }
}

impl<T: ReconSample> CurrentFramePlane<T> {
    /// Scales the clamped-to-storage rectangle in place: motion compensation
    /// drops frame-edge overhang on write, so the BAWP scale covers the same
    /// clamped samples.
    fn apply_bawp_rect(
        &mut self,
        rect: crate::PlaneRect,
        alpha: i16,
        beta: i32,
        max_sample: i32,
    ) -> Result<()> {
        let rect = self.clamp_rect_to_storage(rect)?;
        let (stride, width, height) = (self.stride_samples(), rect.width(), rect.height());
        let short = || ReconError::ArithmeticOverflow {
            context: "bawp rectangle rows",
        };
        let first = self.band_row(rect.y(), height)? * stride + rect.x();
        let rows = self
            .samples
            .get_mut(first..first + (height - 1) * stride + width)
            .ok_or_else(short)?;
        macro_rules! scale_lanes {
            ($lanes:expr) => {{
                let lanes = $lanes;
                let orig = lanes.map(i32::from);
                let scaled =
                    baseline_simd!(simd => bawp_scale(simd, orig, alpha, beta, max_sample));
                *lanes = scaled.map(|value| value as _);
            }};
        }
        macro_rules! scale_rows {
            ($rows:expr, $lanes:literal) => {
                for row in $rows.chunks_mut(stride) {
                    let row = row.get_mut(..width).unwrap_or_default();
                    for lanes in row.as_chunks_mut::<$lanes>().0 {
                        scale_lanes!(lanes);
                    }
                }
            };
        }
        macro_rules! scale_by_width {
            ($rows:expr) => {
                match width % 16 {
                    0 => scale_rows!($rows, 16),
                    8 => scale_rows!($rows, 8),
                    _ => scale_rows!($rows, 4),
                }
            };
        }
        if width.is_multiple_of(4) {
            if let Some(rows) = T::u8_slice_mut(rows) {
                #[cfg(target_arch = "x86_64")]
                if let Some(token) = fearless_simd::Level::baseline().as_avx2()
                    && max_sample == 0xff
                    && (width.is_multiple_of(16)
                        || matches!(width, 4 | 8) && height.is_multiple_of(16 / width))
                {
                    bawp_u8_avx2(token, rows, stride, width, alpha, beta);
                    return Ok(());
                }
                scale_by_width!(rows);
                return Ok(());
            }
            if let Some(rows) = T::u16_slice_mut(rows) {
                scale_by_width!(rows);
                return Ok(());
            }
        }
        for row in rows.chunks_mut(stride) {
            let row = row.get_mut(..width).unwrap_or_default();
            for sample in row {
                let orig = i32::from(sample.to_u16());
                let scaled = ((orig * i32::from(alpha) + beta) >> 8).clamp(0, max_sample);
                *sample = u16::try_from(scaled)
                    .ok()
                    .and_then(|value| T::try_from_u16(value).ok())
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "bawp scaled sample storage",
                    })?;
            }
        }
        Ok(())
    }
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// [`bawp_scale`] for 8-bit rows: `madd` of the zero-extended samples
    /// with `(alpha, 0)` pairs is `orig * alpha`, and the saturating
    /// `packs`/`packus` narrowing is the `Clip1`. Rows 4 and 8 wide scale
    /// `16 / w` at a time, so `rows` holds whole groups of them.
    #[inline]
    fn bawp_u8_avx2(
        token: Avx2,
        rows: &mut [u8],
        stride: usize,
        width: usize,
        alpha: i16,
        beta: i32,
    ) {
        use core::arch::x86_64::{
            __m128i, _mm_packus_epi16, _mm_set_epi64x, _mm_setr_epi32, _mm_srli_si128,
            _mm256_add_epi32, _mm256_castsi256_si128, _mm256_cvtepu8_epi32,
            _mm256_extracti128_si256, _mm256_madd_epi16, _mm256_packs_epi32,
            _mm256_permute4x64_epi64, _mm256_set1_epi32, _mm256_srai_epi32,
        };
        use fearless_simd::{SimdFrom, u8x16};
        let alpha = _mm256_set1_epi32(i32::from(alpha as u16));
        let beta = _mm256_set1_epi32(beta);
        let scale = |samples: __m128i| -> [u8; 16] {
            let half = |samples: __m128i| {
                let product = _mm256_madd_epi16(_mm256_cvtepu8_epi32(samples), alpha);
                _mm256_srai_epi32::<8>(_mm256_add_epi32(product, beta))
            };
            let words = _mm256_permute4x64_epi64::<0b11_01_10_00>(_mm256_packs_epi32(
                half(samples),
                half(_mm_srli_si128::<8>(samples)),
            ));
            u8x16::simd_from(
                token,
                _mm_packus_epi16(
                    _mm256_castsi256_si128(words),
                    _mm256_extracti128_si256::<1>(words),
                ),
            )
            .into()
        };
        let mut rest = rows;
        match width {
            4 => {
                while !rest.is_empty() {
                    let lane = |k| {
                        rest.get(k * stride..)
                            .and_then(<[u8]>::first_chunk::<4>)
                            .map_or(0, |&samples| i32::from_le_bytes(samples))
                    };
                    let lanes = scale(_mm_setr_epi32(lane(0), lane(1), lane(2), lane(3)));
                    rest = publish_rows::<u8, 4>(&lanes, rest, stride);
                }
            }
            8 => {
                while !rest.is_empty() {
                    let lane = |k| {
                        rest.get(k * stride..)
                            .and_then(<[u8]>::first_chunk::<8>)
                            .map_or(0, |&samples| i64::from_le_bytes(samples))
                    };
                    let lanes = scale(_mm_set_epi64x(lane(1), lane(0)));
                    rest = publish_rows::<u8, 8>(&lanes, rest, stride);
                }
            }
            _ => {
                for line in rest.chunks_mut(stride) {
                    let line = line.get_mut(..width).unwrap_or_default();
                    for lanes in line.as_chunks_mut::<16>().0 {
                        *lanes = scale(u8x16::from_slice(token, lanes).into());
                    }
                }
            }
        }
    }
);

/// `Clip3(0, max, (orig * alpha + beta) >> 8)` lane-wise, the BAWP scale.
#[inline]
fn bawp_scale<S: Simd, const N: usize>(
    simd: S,
    orig: [i32; N],
    alpha: i16,
    beta: i32,
    max: i32,
) -> [i32; N] {
    let scale8 = |orig: &[i32; 8]| {
        let lanes = i32x8::load_array_ref(simd, orig);
        ((lanes * i32::from(alpha) + beta) >> 8)
            .max(0)
            .min(max)
            .to_array()
    };
    let mut out = [0; N];
    if let (Some(orig), Some(slot)) = (orig.first_chunk::<16>(), out.first_chunk_mut::<16>()) {
        let lanes = i32x16::load_array_ref(simd, orig);
        *slot = ((lanes * i32::from(alpha) + beta) >> 8)
            .max(0)
            .min(max)
            .to_array();
    } else if let (Some(orig), Some(slot)) = (orig.first_chunk::<8>(), out.first_chunk_mut::<8>()) {
        *slot = scale8(orig);
    } else if let (Some(orig), Some(slot)) = (orig.first_chunk::<4>(), out.first_chunk_mut::<4>()) {
        let lanes = i32x4::load_array_ref(simd, orig);
        *slot = ((lanes * i32::from(alpha) + beta) >> 8)
            .max(0)
            .min(max)
            .to_array();
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{BitDepth, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize};

    #[test]
    fn ii_weights_table_matches_the_spec_end_points() {
        assert_eq!(II_WEIGHTS_1D[0], 60);
        assert_eq!(II_WEIGHTS_1D[18], 32);
        assert_eq!(II_WEIGHTS_1D[127], 1);
        assert_eq!(II_WEIGHTS_1D.len(), 128);
    }

    #[test]
    fn dc_mask_is_constant_32() {
        for (i, j) in [(0, 0), (3, 7), (63, 63)] {
            assert_eq!(intra_mode_variant_weight(InterIntraMode::Dc, i, j, 2), 32);
        }
    }

    #[test]
    fn directional_masks_index_the_expected_axis() {
        assert_eq!(
            intra_mode_variant_weight(InterIntraMode::Vertical, 4, 0, 2),
            II_WEIGHTS_1D[8]
        );
        assert_eq!(
            intra_mode_variant_weight(InterIntraMode::Horizontal, 0, 4, 2),
            II_WEIGHTS_1D[8]
        );
        assert_eq!(
            intra_mode_variant_weight(InterIntraMode::Smooth, 9, 4, 2),
            II_WEIGHTS_1D[8]
        );
    }

    #[test]
    fn interintra_blend_clips_frame_edge_overhang() {
        let storage = PlaneSize::new(8, 6).unwrap();
        let visible = PlaneRect::new(0, 0, 8, 6).unwrap();
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Monochrome,
            storage,
            visible,
        )
        .unwrap();
        let mut workspace = CurrentFrameWorkspace::new(info, 0u8).unwrap();
        let size = IntraRectBlockSize::new(2, 2).unwrap();

        workspace
            .blend_smooth_interintra_rect(PlaneId::Y, 4, 4, size, InterIntraMode::Dc, &[100; 16])
            .unwrap();

        for y in 0..6 {
            for x in 0..8 {
                let expected = if y >= 4 && x >= 4 { 50 } else { 0 };
                assert_eq!(
                    workspace.reconstructed_sample(PlaneId::Y, x, y).unwrap(),
                    expected
                );
            }
        }
    }
}
