// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.29 intra mode variant mask and § 7.13.3.30 interintra blend.

use super::{CurrentFramePlane, CurrentFrameWorkspace, checked_sample_block_rect};
#[cfg(target_arch = "x86_64")]
use crate::intra_smooth::publish_rows;
use crate::{IntraRectBlockSize, PlaneId, ReconError, ReconSample, Result};
use fearless_simd::{Simd, i16x16, i32x4, i32x8, i32x16, prelude::*, u8x16, u16x8, u16x16};
use std::sync::OnceLock;

/// AV2 § 7.13.3.29 `Ii_Weights_1d[128]` smooth-interintra blending weights.
const II_WEIGHTS_1D: [u8; 128] = [
    60, 58, 56, 54, 52, 50, 48, 47, 45, 44, 42, 41, 39, 38, 37, 35, 34, 33, 32, 31, 30, 29, 28, 27,
    26, 25, 24, 23, 22, 22, 21, 20, 19, 19, 18, 18, 17, 16, 16, 15, 15, 14, 14, 13, 13, 12, 12, 12,
    11, 11, 10, 10, 10, 9, 9, 9, 8, 8, 8, 8, 7, 7, 7, 7, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 4, 4, 4, 4,
    4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];

/// `Ii_Weights_1d[j * sizeScale]` of § 7.13.3.29 at `j` in `0..64`, for
/// `sizeScale = 128 / Max(w, h)` of `Max(w, h) = 64 >> k` at row `k`.
const II_SCALED_WEIGHTS: [[u8; 64]; 5] = {
    let mut rows = [[0; 64]; 5];
    let mut k = 0;
    while k < 5 {
        let mut j = 0;
        while j < 64 {
            let index = j << (k + 1);
            rows[k][j] = II_WEIGHTS_1D[if index < 127 { index } else { 127 }];
            j += 1;
        }
        k += 1;
    }
    rows
};
const MASK_MASTER_SIZE: usize = 128;
const MAX_WEDGE_TYPES: usize = 68;
const WEDGE_ANGLES: usize = 20;
const WEDGE_BOUNDARY_SHARP: usize = 0;
const WEDGE_BOUNDARY_SMOOTH: usize = 1;
const WEDGE_COS_LUT_ALL: [[i32; WEDGE_ANGLES]; 2] = [
    [
        32, 32, 32, 16, 16, 0, -16, -16, -32, -32, -32, -32, -32, -16, -16, 0, 16, 16, 32, 32,
    ],
    [
        16, 16, 16, 8, 8, 0, -8, -8, -16, -16, -16, -16, -16, -8, -8, 0, 8, 8, 16, 16,
    ],
];
const WEDGE_SIN_LUT_ALL: [[i32; WEDGE_ANGLES]; 2] = [
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
#[cfg(test)]
fn intra_mode_variant_weight(mode: InterIntraMode, i: usize, j: usize, size_scale: usize) -> u16 {
    let index = match mode {
        InterIntraMode::Dc => return 32,
        InterIntraMode::Vertical => i * size_scale,
        InterIntraMode::Horizontal => j * size_scale,
        InterIntraMode::Smooth => i.min(j) * size_scale,
    };
    u16::from(II_WEIGHTS_1D[index.min(II_WEIGHTS_1D.len() - 1)])
}

fn wedge_boundary_index(luma_width: usize, luma_height: usize) -> usize {
    if luma_width <= 16 && luma_height <= 16 {
        WEDGE_BOUNDARY_SHARP
    } else {
        WEDGE_BOUNDARY_SMOOTH
    }
}

/// The § 7.13.3.27 `MasterMask[boundary][direction][n][m]` (AVM values).
fn master_weight(boundary: usize, direction: usize, m: usize, n: usize) -> u16 {
    let centered = |v: usize| 2 * v as i32 + 1 - MASK_MASTER_SIZE as i32;
    let d = centered(m) * WEDGE_COS_LUT_ALL[boundary][direction]
        + centered(n) * WEDGE_SIN_LUT_ALL[boundary][direction];
    let d = d.clamp(-127, 127);
    let weight = if d >= 0 {
        POS_DIST_2_BLD_WEIGHT[d as usize]
    } else {
        NEG_DIST_2_BLD_WEIGHT[d.unsigned_abs() as usize]
    };
    weight << 2
}

/// The § 7.13.3.27 `MasterMask` of each boundary and angle, 16 KiB each,
/// built at first use.
static MASTER_MASKS: [OnceLock<Box<[u8]>>; 2 * WEDGE_ANGLES] =
    [const { OnceLock::new() }; 2 * WEDGE_ANGLES];

/// The § 7.13.3.27 `WedgeMasks[MiSize][sign][wedge_index]` of a luma block:
/// its rows start `MASK_MASTER_SIZE` apart in one master mask. The inverse
/// mask `64 - MasterMask[angle]` is `MasterMask[angle + 10]`, because both
/// luts negate there and `Pos_Dist_2_Bld_Weight[k] + Neg_Dist_2_Bld_Weight[k]`
/// is 16.
fn wedge_mask_window(
    luma_width: usize,
    luma_height: usize,
    wedge_index: usize,
    sign: bool,
) -> Result<&'static [u8]> {
    let overflow = |context| ReconError::ArithmeticOverflow { context };
    let &(direction, x_offset, y_offset) = WEDGE_CODEBOOK
        .get(wedge_index)
        .ok_or(overflow("interintra wedge index"))?;
    let boundary = wedge_boundary_index(luma_width, luma_height);
    let direction = (direction + usize::from(sign) * WEDGE_ANGLES / 2) % WEDGE_ANGLES;
    let start = |offset: usize, size: usize| {
        (MASK_MASTER_SIZE / 2)
            .checked_sub((offset * size) >> 3)
            .filter(|&start| start + size <= MASK_MASTER_SIZE)
            .ok_or(overflow("wedge mask block size"))
    };
    let start = start(y_offset, luma_height)? * MASK_MASTER_SIZE + start(x_offset, luma_width)?;
    let master = MASTER_MASKS[boundary * WEDGE_ANGLES + direction].get_or_init(|| {
        let side = MASK_MASTER_SIZE;
        let weight = |i| master_weight(boundary, direction, i % side, i / side) as u8;
        (0..side * side).map(weight).collect()
    });
    let len = luma_height.saturating_sub(1) * MASK_MASTER_SIZE + luma_width;
    Ok(master.get(start..start + len).unwrap_or_default())
}

/// The rows of the § 7.13.3.27 wedge mask of a `luma_width x luma_height`
/// block, `luma_width` weights each.
///
/// # Errors
/// Returns [`ReconError`] when the block size or the wedge index does not
/// select an AV2 wedge mask.
pub fn wedge_mask_rows(
    luma_width: usize,
    luma_height: usize,
    wedge_index: usize,
    sign: bool,
) -> Result<impl Iterator<Item = &'static [u8]>> {
    let window = wedge_mask_window(luma_width, luma_height, wedge_index, sign)?;
    let rows = window.chunks(MASK_MASTER_SIZE).take(luma_height);
    Ok(rows.map(move |row| row.get(..luma_width).unwrap_or_default()))
}

/// The 8x8 unit at luma `(x, y)` of the § 7.13.3.27 wedge mask of a
/// `luma_width x luma_height` block, row by row.
///
/// # Errors
/// Returns [`ReconError`] when the block size or the wedge index does not
/// select an AV2 wedge mask, or when the unit leaves the block.
pub fn wedge_mask_unit(
    luma_width: usize,
    luma_height: usize,
    (wedge_index, sign): (usize, bool),
    (x, y): (usize, usize),
) -> Result<[u8; 64]> {
    let window = wedge_mask_window(luma_width, luma_height, wedge_index, sign)?;
    let outside = || ReconError::ArithmeticOverflow {
        context: "wedge mask unit position",
    };
    if x + 8 > luma_width || y + 8 > luma_height {
        return Err(outside());
    }
    let mut unit = [0; 64];
    for (row, lanes) in unit.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        let start = (y + row) * MASK_MASTER_SIZE + x;
        *lanes = *window
            .get(start..)
            .and_then(<[u8]>::first_chunk)
            .ok_or_else(outside)?;
    }
    Ok(unit)
}

/// Writes the wedge mask of a `luma` block for a `w x h` plane block, as
/// § 7.13.3.30 subsamples it by `(sub_x, sub_y)`, into the packed `mask`.
/// The mask does not depend on the bit depth.
///
/// # Errors
/// Returns [`ReconError`] when the block size or the wedge index does not
/// select an AV2 wedge mask, when the plane block is larger than the
/// subsampled luma block or, when subsampled, not a multiple of 4 wide, or
/// when `mask` is shorter than `w * h`.
pub fn wedge_mask_into(
    mask: &mut [u8],
    (w, h): (usize, usize),
    (luma_width, luma_height): (usize, usize),
    (wedge_index, sign): (usize, bool),
    (sub_x, sub_y): (u32, u32),
) -> Result<()> {
    let window = wedge_mask_window(luma_width, luma_height, wedge_index, sign)?;
    let sub_y = if sub_x == 0 { 0 } else { sub_y };
    let fits = (w << sub_x) <= luma_width
        && (h << sub_y) <= luma_height
        && (sub_x == 0 || w.is_multiple_of(4));
    let mask =
        mask.get_mut(..w * h)
            .filter(|_| fits && w > 0)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "wedge plane mask size",
            })?;
    if sub_x == 0 {
        for (row, source) in mask
            .chunks_exact_mut(w)
            .zip(window.chunks(MASK_MASTER_SIZE))
        {
            row.copy_from_slice(&source[..w]); // splot-copy-ok: wedge mask row from the master mask
        }
        return Ok(());
    }
    crate::blend_mask::subsample_mask(window, MASK_MASTER_SIZE, mask, w, sub_y);
    Ok(())
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
    /// is out of bounds or not stored, or `intra.len()` is not the
    /// rectangular sample count.
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
    /// outside the AVM codebook, or the target rows are not stored.
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
    /// Row `i` of the § 7.13.3.29 mask is `Max(base[j], floor(i))`: the
    /// weights fall as their index grows, so `Ii_Weights_1d[Min(i, j) * s]`
    /// is the larger of the row and column weights.
    fn blend_smooth_interintra_rect(
        &mut self,
        rect: crate::PlaneRect,
        size: IntraRectBlockSize,
        mode: InterIntraMode,
        intra: &[T],
    ) -> Result<()> {
        let log2_max = size.log2_width().max(size.log2_height());
        let weights = II_SCALED_WEIGHTS
            .get(usize::from(6u8.saturating_sub(log2_max)))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "interintra mask block size",
            })?;
        let (base, column): (&[u8; 64], bool) = match mode {
            InterIntraMode::Dc => (&[32; 64], false),
            InterIntraMode::Vertical => (&[0; 64], true),
            InterIntraMode::Horizontal => (weights, false),
            InterIntraMode::Smooth => (weights, true),
        };
        let masks = weights
            .iter()
            .map(move |&weight| (&base[..], if column { weight } else { 0 }));
        self.blend_interintra(rect, size.width(), intra, masks)
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
        let short = || ReconError::ArithmeticOverflow {
            context: "interintra wedge mask size",
        };
        let (w, h) = (size.width(), size.height());
        let wedge = (wedge_index, false);
        if sub_x == 0 {
            if w > luma_width || h > luma_height {
                return Err(short());
            }
            let rows = wedge_mask_rows(luma_width, luma_height, wedge_index, false)?;
            return self.blend_interintra(rect, w, intra, rows.map(|row| (row, 0)));
        }
        let mut mask = [0; 32 * 64];
        let mask = mask.get_mut(..w * h).ok_or_else(short)?;
        wedge_mask_into(
            mask,
            (w, h),
            (luma_width, luma_height),
            wedge,
            (sub_x, sub_y),
        )?;
        self.blend_interintra(rect, w, intra, mask.chunks_exact(w).map(|row| (row, 0)))
    }

    /// The § 7.13.3.30 interintra blend
    /// `CurrFrame = Round2(m * IntraPred + (64 - m) * CurrFrame, 6)` over the
    /// storage-clamped `rect`, with `m = Max(mask[j], floor)` of the
    /// `(mask, floor)` of each row. `intra` rows start `intra_stride` apart.
    fn blend_interintra<'m>(
        &mut self,
        rect: crate::PlaneRect,
        intra_stride: usize,
        intra: &[T],
        masks: impl Iterator<Item = (&'m [u8], u8)>,
    ) -> Result<()> {
        let short = || ReconError::ArithmeticOverflow {
            context: "interintra blend rows",
        };
        let (stride, width, height) = (self.stride_samples(), rect.width(), rect.height());
        let first = self.band_row(rect.y(), height)? * stride + rect.x();
        let rows = self
            .samples
            .get_mut(first..first + (height - 1) * stride + width)
            .ok_or_else(short)?;
        let intra = intra
            .get(..(height - 1) * intra_stride + width)
            .filter(|_| intra_stride >= width)
            .ok_or_else(short)?;
        if let (Some(rows), Some(intra)) = (T::u8_slice_mut(rows), T::u8_slice(intra)) {
            baseline_simd!(simd => blend_interintra_rows::<_, 16>(
                (rows, stride), (intra, intra_stride), width, masks,
                |row, intra, mask| blend_interintra_u8(simd, row, intra, mask),
            ));
        } else if let (Some(rows), Some(intra)) = (T::u16_slice_mut(rows), T::u16_slice(intra)) {
            baseline_simd!(simd => blend_interintra_rows::<_, 8>(
                (rows, stride), (intra, intra_stride), width, masks,
                |row, intra, mask| blend_interintra_u16(simd, row, intra, mask),
            ));
        }
        Ok(())
    }
}

/// Runs `chunk` on the `N`-lane chunks of each `width`-sample row, the last
/// chunk of a row zero-padded, with the row's mask lanes raised to its floor.
#[inline]
fn blend_interintra_rows<'m, P: Copy + Default, const N: usize>(
    (rows, stride): (&mut [P], usize),
    (intra, intra_stride): (&[P], usize),
    width: usize,
    masks: impl Iterator<Item = (&'m [u8], u8)>,
    chunk: impl Fn(&mut [P; N], &[P; N], ([u8; N], u8)),
) {
    let rows = rows.chunks_mut(stride).zip(intra.chunks(intra_stride));
    for ((row, intra), (mask, floor)) in rows.zip(masks) {
        let (Some(row), Some(intra), Some(mask)) =
            (row.get_mut(..width), intra.get(..width), mask.get(..width))
        else {
            return;
        };
        let (rows, row_tail) = row.as_chunks_mut::<N>();
        let (intras, intra_tail) = intra.as_chunks::<N>();
        let (masks, mask_tail) = mask.as_chunks::<N>();
        for ((row, intra), mask) in rows.iter_mut().zip(intras).zip(masks) {
            chunk(row, intra, (*mask, floor));
        }
        let tail = row_tail.len();
        if tail != 0 {
            let (mut row, mut intra, mut mask) = ([P::default(); N], [P::default(); N], [0; N]);
            row[..tail].copy_from_slice(row_tail); // splot-copy-ok: zero-padded last chunk of an interintra row
            intra[..tail].copy_from_slice(intra_tail); // splot-copy-ok: zero-padded last chunk of an interintra row
            mask[..tail].copy_from_slice(mask_tail); // splot-copy-ok: zero-padded last chunk of an interintra mask row
            chunk(&mut row, &intra, (mask, floor));
            row_tail.copy_from_slice(&row[..tail]); // splot-copy-ok: blended last chunk back into the frame row
        }
    }
}

/// [`blend_interintra_rows`] lanes of eight-bit samples: `u8 x u8` products
/// widen to `u16`, where `64 * 255 + 32` fits.
#[inline]
fn blend_interintra_u8<S: Simd>(
    simd: S,
    row: &mut [u8; 16],
    intra: &[u8; 16],
    (mask, floor): ([u8; 16], u8),
) {
    let mask = u8x16::load_array(simd, mask).max(u8x16::splat(simd, floor));
    let (intra_low, intra_high) = mask.widening_mul(u8x16::load_array_ref(simd, intra));
    let inverse = u8x16::splat(simd, 64) - mask;
    let (inter_low, inter_high) = inverse.widening_mul(u8x16::load_array_ref(simd, row));
    let round = |intra: u16x8<S>, inter: u16x8<S>| (intra + inter + 32) >> 6;
    *row = round(intra_low, inter_low)
        .narrow(round(intra_high, inter_high))
        .to_array();
}

/// [`blend_interintra_rows`] lanes of samples up to ten bits in `u16`:
/// `m * intra + (64 - m) * inter + 32 <= 64 * 1023 + 32` fits.
#[inline]
fn blend_interintra_u16<S: Simd>(
    simd: S,
    row: &mut [u16; 8],
    intra: &[u16; 8],
    (mask, floor): ([u8; 8], u8),
) {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&mask); // splot-copy-ok: eight mask bytes into one vector
    let (mask, _) = u8x16::load_array(simd, bytes)
        .max(u8x16::splat(simd, floor))
        .widen();
    let intra = u16x8::load_array_ref(simd, intra);
    let inter = u16x8::load_array_ref(simd, row);
    let blended = (mask * intra + (u16x8::splat(simd, 64) - mask) * inter + 32) >> 6;
    *row = blended.to_array();
}

/// The above-arm lanes a § 7.13.3.25 template keeps, by `log2(width) - 2`
/// and `upStep - 1`: lanes `upStep >> 1, upStep + (upStep >> 1), ...` below
/// `width`.
const UP_LANES: [[[u16; 16]; 2]; 3] = {
    let mut masks = [[[0; 16]; 2]; 3];
    let mut size = 0;
    while size < 3 {
        let mut step = 1;
        while step <= 2 {
            let mut lane = step >> 1;
            while lane < 4 << size {
                masks[size][step - 1][lane] = u16::MAX;
                lane += step;
            }
            step += 1;
        }
        size += 1;
    }
    masks
};

/// One frame's side of a § 7.13.3.25 template: the row above it from its
/// first column, the column left of it from its first row (`stride` apart),
/// and its last stored `(column, row)`, past which reads repeat the last
/// stored sample like the DC edges.
struct TemplateSource<'a, T> {
    above: &'a [T],
    left: &'a [T],
    stride: usize,
    last: (usize, usize),
}

/// The `((width, numUp), (height, numLeft))` arms of a § 7.13.3.25 template.
type TemplateArms = ((usize, usize), (usize, usize));

/// `(alpha, beta, max)` of a § 7.13.3.25 scale.
type BawpScale = (i16, i32, i16);

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// AV2 § 7.13.3.25 for one plane: sums the template of the block at
    /// `block` against the reference template at `at` — in `reference`, or
    /// in this plane for morphological prediction when `None` — lets `fit`
    /// turn `[sumX, sumY, sumXX, sumXY]` into `(alpha, beta)`, and scales the
    /// `size` block at `block` like [`Self::apply_bawp_rect`]. `arms` are
    /// `((width, numUp), (height, numLeft))` with sides of 4, 8 or 16 and
    /// power-of-two counts that divide them; `sumXX` and `sumXY` stay zero
    /// unless `products`. Returns the alpha, or `None` with nothing scaled
    /// when an arm the template uses lies outside the frame or the
    /// reference.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, its template or
    /// block rows are not stored, or the storage-clamped block width is not
    /// a multiple of 4.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_bawp_template(
        &mut self,
        plane: PlaneId,
        reference: Option<&crate::ReferencePlaneView<'_, T>>,
        (block, at): ((usize, usize), (usize, usize)),
        arms: TemplateArms,
        products: bool,
        size: (usize, usize),
        fit: impl FnOnce([i32; 4]) -> (i16, i32),
    ) -> Result<Option<i16>> {
        let max = self.info().bit_depth().max_sample() as i16;
        let target = self.plane_mut(plane)?;
        let Some(current) = target.bawp_template(block, arms)? else {
            return Ok(None);
        };
        let reference = match reference {
            Some(view) => view_template(view, at, arms),
            None => target.bawp_template(at, arms).ok().flatten(),
        };
        let Some(sums) = reference.and_then(|reference| {
            baseline_simd!(simd => template_sums(simd, [&reference, &current], arms, products))
        }) else {
            return Ok(None);
        };
        let (alpha, beta) = fit(sums);
        target.apply_bawp_rect(block, size, (alpha, beta, max))?;
        Ok(Some(alpha))
    }

    /// AV2 § 7.13.3.25 block adaptive weighted prediction application:
    /// `CurrFrame = Clip1((orig * alpha + beta) >> 8)` in place over the
    /// rect, dropping frame-edge overhang like the reconstruction writes. A
    /// rect whose origin lies outside storage scales nothing.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the rect rows are
    /// not stored, or the storage-clamped width is not a multiple of 4: AV2
    /// plane storage is `MiCols * 4 >> subX` wide with an even `MiCols`
    /// (§ 5.18.4.4 `MiCols = 2 * ((FrameWidth + 7) >> 3)`) and BAWP blocks
    /// start on 4-sample columns, so a decoded block never clamps to one.
    pub fn apply_bawp_rect(
        &mut self,
        plane: PlaneId,
        rect: crate::PlaneRect,
        alpha: i16,
        beta: i32,
    ) -> Result<()> {
        let max = self.info().bit_depth().max_sample() as i16;
        self.plane_mut(plane)?.apply_bawp_rect(
            (rect.x(), rect.y()),
            (rect.width(), rect.height()),
            (alpha, beta, max),
        )
    }
}

impl<T: ReconSample> CurrentFramePlane<T> {
    /// The current-frame side of a template at `(x, y)`; `None` when an arm
    /// the template uses lies outside the frame.
    #[inline]
    fn bawp_template(
        &self,
        (x, y): (usize, usize),
        ((width, num_up), (height, num_left)): TemplateArms,
    ) -> Result<Option<TemplateSource<'_, T>>> {
        let storage = self.storage_size();
        let up = usize::from(num_up > 0);
        let (Some(top), Some(cols), Some(rows)) = (
            y.checked_sub(up),
            storage.width().checked_sub(x),
            storage.height().checked_sub(y),
        ) else {
            return Ok(None);
        };
        let (Some(last_col), Some(last_row)) = (
            width.min(cols).checked_sub(1),
            height.min(rows).checked_sub(1),
        ) else {
            return Ok(None);
        };
        if num_left > 0 && x == 0 {
            return Ok(None);
        }
        let stride = self.stride_samples();
        let first = self.band_row(top, up + last_row + 1)? * stride;
        let rows = self.samples.get(first..).unwrap_or_default();
        Ok(Some(TemplateSource {
            above: rows.get(x..).unwrap_or_default(),
            left: rows
                .get((up * stride + x).saturating_sub(1)..)
                .unwrap_or_default(),
            stride,
            last: (last_col, last_row),
        }))
    }

    /// Scales the `(width, height)` rectangle at `(x, y)` clamped to storage
    /// in place: motion compensation drops frame-edge overhang on write, so
    /// the BAWP scale covers the same clamped samples.
    #[inline]
    fn apply_bawp_rect(
        &mut self,
        (x, y): (usize, usize),
        (width, height): (usize, usize),
        scale: BawpScale,
    ) -> Result<()> {
        let storage = self.storage_size();
        let (Some(cols), Some(rows)) = (
            storage.width().checked_sub(x).filter(|&cols| cols > 0),
            storage.height().checked_sub(y).filter(|&rows| rows > 0),
        ) else {
            return Ok(());
        };
        let (width, height) = (width.min(cols), height.min(rows));
        if !width.is_multiple_of(4) {
            return Err(ReconError::ArithmeticOverflow {
                context: "bawp rectangle width",
            });
        }
        let Some(last) = height.checked_sub(1) else {
            return Ok(());
        };
        let stride = self.stride_samples();
        let first = self.band_row(y, height)? * stride + x;
        let rows = self
            .samples
            .get_mut(first..first + last * stride + width)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "bawp rectangle rows",
            })?;
        let (alpha, beta, max_sample) = (scale.0, scale.1, i32::from(scale.2));
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
        } else if let Some(rows) = T::u16_slice_mut(rows) {
            scale_by_width!(rows);
        }
        Ok(())
    }
}

/// The reference side of a template at `(x, y)` of `view`, every sample of
/// it inside the view's width and readable rows; `None` otherwise.
#[inline]
fn view_template<'a, T: ReconSample>(
    view: &crate::ReferencePlaneView<'a, T>,
    (x, y): (usize, usize),
    ((width, _), (height, _)): TemplateArms,
) -> Option<TemplateSource<'a, T>> {
    let [stride, view_width, readable_rows, _] = view.geometry();
    if x == 0 || y == 0 || x + width > view_width || y + height > readable_rows {
        return None;
    }
    let (rows, _) = view.rows_from(y - 1)?;
    Some(TemplateSource {
        above: rows.get(x..)?,
        left: rows.get(stride + x - 1..)?,
        stride,
        last: (width.saturating_sub(1), height.saturating_sub(1)),
    })
}

/// `samples` widened to `u16` lanes.
#[allow(clippy::inline_always, reason = "measured BAWP hot path")]
#[inline(always)]
fn sample_lanes<S: Simd, T: ReconSample>(simd: S, samples: &[T; 16]) -> u16x16<S> {
    if let Some(bytes) = T::u8_slice(samples).and_then(<[u8]>::first_chunk::<16>) {
        widen_u8(simd, u8x16::load_array_ref(simd, bytes))
    } else if let Some(words) = T::u16_slice(samples).and_then(<[u16]>::first_chunk::<16>) {
        u16x16::load_array_ref(simd, words)
    } else {
        u16x16::splat(simd, 0)
    }
}

#[allow(clippy::inline_always, reason = "measured BAWP hot path")]
#[inline(always)]
fn widen_u8<S: Simd>(simd: S, lanes: u8x16<S>) -> u16x16<S> {
    simd.widen_u8x32(simd.combine_u8x16(lanes, lanes)).0
}

/// The above arm of `source`: one 16-lane load of the row, padded from a
/// copy only where storage ends within it, lanes past the last stored
/// column repeating it.
#[allow(clippy::inline_always, reason = "measured BAWP hot path")]
#[inline(always)]
fn above_lanes<S: Simd, T: ReconSample>(
    simd: S,
    source: &TemplateSource<'_, T>,
    width: usize,
) -> Option<u16x16<S>> {
    let stored = width.min(source.last.0 + 1);
    let row = source.above.get(..stored)?;
    let lanes = if let Some(samples) = source.above.first_chunk::<16>() {
        sample_lanes(simd, samples)
    } else {
        let mut padded = [T::default(); 16];
        padded.get_mut(..stored)?.copy_from_slice(row); // splot-copy-ok: template row padded where storage ends
        sample_lanes(simd, &padded)
    };
    if stored == width {
        return Some(lanes);
    }
    let lane = u16x16::load_array(simd, LANES);
    let last = u16x16::splat(simd, row.last()?.to_u16());
    Some(lane.simd_lt(stored as u16).select(lanes, last))
}

/// Lane indices of a 16-lane template vector.
const LANES: [u16; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// The `N` samples `pitch` apart from the start of `column`, after one
/// bounds check.
#[allow(clippy::inline_always, reason = "measured BAWP hot path")]
#[inline(always)]
fn gather<T: Copy + Default, const N: usize>(column: &[T], pitch: usize) -> Option<[T; 16]> {
    let column = column.get(..=(N - 1) * pitch)?;
    let mut lanes = [T::default(); 16];
    for (k, lane) in lanes.iter_mut().take(N).enumerate() {
        *lane = *column.get(k * pitch)?;
    }
    Some(lanes)
}

/// The left arm of `source`: `num` samples at step `height / num` from row
/// `step >> 1`, gathered after one bounds check; at a storage edge the rows
/// past the last stored one repeat it.
#[allow(clippy::inline_always, reason = "measured BAWP hot path")]
#[inline(always)]
fn left_lanes<S: Simd, T: ReconSample>(
    simd: S,
    source: &TemplateSource<'_, T>,
    (height, num): (usize, usize),
) -> Option<u16x16<S>> {
    let step = height >> num.trailing_zeros();
    let (first, last_row) = (step >> 1, source.last.1);
    let pitch = source.stride * step;
    let column = source.left.get(first * source.stride..).unwrap_or_default();
    if step > 0 && first + (num - 1) * step <= last_row {
        let lanes = match num {
            4 => gather::<T, 4>(column, pitch)?,
            8 => gather::<T, 8>(column, pitch)?,
            16 => gather::<T, 16>(column, pitch)?,
            _ => return None,
        };
        return Some(sample_lanes(simd, &lanes));
    }
    let stored = if step == 0 || first > last_row {
        0
    } else {
        ((last_row - first) / step + 1).min(num)
    };
    let mut lanes = [T::default(); 16];
    for (slot, &sample) in lanes
        .iter_mut()
        .zip(column.iter().step_by(pitch.max(1)))
        .take(stored)
    {
        *slot = sample;
    }
    let lane = u16x16::load_array(simd, LANES);
    let last = source.left.get(last_row * source.stride)?.to_u16();
    let tail = lane
        .simd_lt(num as u16)
        .select(u16x16::splat(simd, last), u16x16::splat(simd, 0));
    Some(
        lane.simd_lt(stored as u16)
            .select(sample_lanes(simd, &lanes), tail),
    )
}

/// The § 7.13.3.25 sums over the `[reference, current]` templates, summed
/// in registers as the arms load. Samples are at most 1023 and there are at
/// most 32 pairs, so `sumX` and `sumY` fit `u16` lanes and the products'
/// pair sums `i32` lanes.
#[allow(clippy::inline_always, reason = "measured BAWP hot path")]
#[inline(always)]
fn template_sums<S: Simd, T: ReconSample>(
    simd: S,
    sources: [&TemplateSource<'_, T>; 2],
    ((width, num_up), left): TemplateArms,
    products: bool,
) -> Option<[i32; 4]> {
    let zero = u16x16::splat(simd, 0);
    let mut arms = [[zero; 2]; 2];
    if num_up > 0 {
        let step = width >> num_up.trailing_zeros();
        let keep = UP_LANES
            .get((width.trailing_zeros() as usize).wrapping_sub(2))?
            .get(step.wrapping_sub(1))?;
        let keep = u16x16::load_array_ref(simd, keep);
        for (slot, source) in arms[0].iter_mut().zip(sources) {
            *slot = above_lanes(simd, source, width)? & keep;
        }
    }
    if left.1 > 0 {
        for (slot, source) in arms[1].iter_mut().zip(sources) {
            *slot = left_lanes(simd, source, left)?;
        }
    }
    let [[x_up, y_up], [x_left, y_left]] = arms;
    let sum = |lanes: u16x16<S>| i32::from(lanes.reduce_sum());
    let pairs = |a: u16x16<S>, b: u16x16<S>| {
        a.bitcast::<i16x16<S>>()
            .pairwise_dot_product(b.bitcast::<i16x16<S>>())
    };
    let (xx, xy) = if products {
        (
            (pairs(x_up, x_up) + pairs(x_left, x_left)).reduce_sum(),
            (pairs(x_up, y_up) + pairs(x_left, y_left)).reduce_sum(),
        )
    } else {
        (0, 0)
    };
    Some([sum(x_up + x_left), sum(y_up + y_left), xx, xy])
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

    fn wedge_mask_luma_sample(
        luma_width: usize,
        luma_height: usize,
        wedge_index: usize,
        sign: bool,
        x: usize,
        y: usize,
    ) -> Result<u16> {
        let overflow = |context| ReconError::ArithmeticOverflow { context };
        let &(direction, x_offset, y_offset) = WEDGE_CODEBOOK
            .get(wedge_index)
            .ok_or(overflow("interintra wedge index"))?;
        let master = |offset: usize, size: usize, at: usize| {
            (MASK_MASTER_SIZE / 2 + at)
                .checked_sub((offset * size) >> 3)
                .ok_or(overflow("interintra wedge mask coordinate"))
        };
        let (m, n) = (
            master(x_offset, luma_width, x)?,
            master(y_offset, luma_height, y)?,
        );
        let base = master_weight(
            wedge_boundary_index(luma_width, luma_height),
            direction,
            m,
            n,
        );
        Ok(if sign { 64 - base } else { base })
    }

    /// One § 7.13.3.27 wedge weight of a luma plane sample, or of a chroma
    /// sample as § 7.13.3.30 subsamples it by `(sub_x, sub_y)`, sample by sample.
    #[allow(clippy::too_many_arguments)]
    fn wedge_mask_plane_sample(
        lw: usize,
        lh: usize,
        index: usize,
        sign: bool,
        sub_x: u32,
        sub_y: u32,
        x: usize,
        y: usize,
    ) -> Result<u16> {
        let sub_y = sub_y.min(sub_x);
        let mut sum = 0;
        for (dx, dy) in
            (0..=sub_y as usize).flat_map(|dy| (0..=sub_x as usize).map(move |dx| (dx, dy)))
        {
            let (x, y) = ((x << sub_x) + dx, (y << sub_y) + dy);
            sum += i32::from(wedge_mask_luma_sample(lw, lh, index, sign, x, y)?);
        }
        Ok(crate::math::round2_i32(sum, sub_x + sub_y) as u16)
    }

    #[test]
    fn ii_weights_table_matches_the_spec_end_points() {
        assert_eq!(II_WEIGHTS_1D[0], 60);
        assert_eq!(II_WEIGHTS_1D[18], 32);
        assert_eq!(II_WEIGHTS_1D[127], 1);
        assert_eq!(II_WEIGHTS_1D.len(), 128);
    }

    /// Every block size with `Wedge_Bits[MiSize] > 0` (§ 9.2), every
    /// `WEDGE_TYPES` index (§ 3), both signs and every subsampling: the
    /// table masks equal the sample-by-sample mask.
    #[test]
    fn wedge_masks_match_the_sample_oracle() {
        let sizes = [
            (8, 8),
            (8, 16),
            (16, 8),
            (16, 16),
            (16, 32),
            (32, 16),
            (32, 32),
            (32, 64),
            (64, 32),
            (64, 64),
            (8, 32),
            (32, 8),
            (16, 64),
            (64, 16),
            (8, 64),
            (64, 8),
        ];
        let mut mask = vec![0; 64 * 64];
        for (lw, lh) in sizes {
            for index in 0..MAX_WEDGE_TYPES {
                for sign in [false, true] {
                    let rows: Vec<&[u8]> = wedge_mask_rows(lw, lh, index, sign).unwrap().collect();
                    assert_eq!(rows.len(), lh);
                    for (sub_x, sub_y) in [(0, 0), (1, 0), (1, 1)] {
                        let (w, h) = (lw >> sub_x, lh >> sub_y);
                        let wedge = (index, sign);
                        wedge_mask_into(&mut mask, (w, h), (lw, lh), wedge, (sub_x, sub_y))
                            .unwrap();
                        for (y, x) in (0..h).flat_map(|y| (0..w).map(move |x| (y, x))) {
                            let want =
                                wedge_mask_plane_sample(lw, lh, index, sign, sub_x, sub_y, x, y);
                            let at = format!("{lw}x{lh} {index} {sign} {sub_x}{sub_y} {x},{y}");
                            assert_eq!(u16::from(mask[y * w + x]), want.unwrap(), "{at}");
                            if sub_x == 0 {
                                assert_eq!(rows[y][x], mask[y * w + x], "{at}");
                            }
                        }
                    }
                }
            }
        }
        assert!(wedge_mask_rows(8, 8, MAX_WEDGE_TYPES, false).is_err());
        assert!(wedge_mask_rows(128, 128, 0, false).is_err());
        assert!(wedge_mask_into(&mut mask[..63], (8, 8), (8, 8), (0, false), (0, 0)).is_err());
        assert!(wedge_mask_into(&mut mask, (8, 8), (8, 8), (0, false), (1, 1)).is_err());
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
            u16::from(II_WEIGHTS_1D[8])
        );
        assert_eq!(
            intra_mode_variant_weight(InterIntraMode::Horizontal, 0, 4, 2),
            u16::from(II_WEIGHTS_1D[8])
        );
        assert_eq!(
            intra_mode_variant_weight(InterIntraMode::Smooth, 9, 4, 2),
            u16::from(II_WEIGHTS_1D[8])
        );
    }

    /// The blend has no `Clip1`: its non-negative weights sum to 64, so
    /// extreme 10-bit inputs under every mask stay at most 1023.
    #[test]
    fn interintra_blend_of_extreme_samples_stays_in_range() {
        let size_px = PlaneSize::new(32, 32).unwrap();
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Ten,
            PixelFormat::Monochrome,
            size_px,
            PlaneRect::new(0, 0, 32, 32).unwrap(),
        )
        .unwrap();
        let size = IntraRectBlockSize::new(5, 5).unwrap();
        let intra: Vec<u16> = (0..32 * 32)
            .map(|i| if i % 3 == 0 { 0 } else { 1023 })
            .collect();
        let modes = [
            InterIntraMode::Dc,
            InterIntraMode::Vertical,
            InterIntraMode::Horizontal,
            InterIntraMode::Smooth,
        ];
        for fill in [0, 1023] {
            for blend in 0..modes.len() + MAX_WEDGE_TYPES {
                let mut workspace = CurrentFrameWorkspace::new(info, fill).unwrap();
                if let Some(&mode) = modes.get(blend) {
                    workspace
                        .blend_smooth_interintra_rect(PlaneId::Y, 0, 0, size, mode, &intra)
                        .unwrap();
                } else {
                    let wedge = blend - modes.len();
                    workspace
                        .blend_wedge_interintra_rect(
                            PlaneId::Y,
                            0,
                            0,
                            size,
                            32,
                            32,
                            wedge,
                            0,
                            0,
                            &intra,
                        )
                        .unwrap();
                }
                for (x, y) in (0..32).flat_map(|y| (0..32).map(move |x| (x, y))) {
                    assert!(workspace.reconstructed_sample(PlaneId::Y, x, y).unwrap() <= 1023);
                }
            }
        }
    }

    /// A monochrome `width x height` workspace of asymmetric samples up to
    /// `max`, and the same samples as a row-major `u16` copy.
    fn noisy_workspace<T: ReconSample>(
        bit_depth: BitDepth,
        (width, height): (usize, usize),
        seed: u32,
    ) -> (CurrentFrameWorkspace<T>, Vec<u16>) {
        let size = PlaneSize::new(width, height).unwrap();
        let rect = PlaneRect::new(0, 0, width, height).unwrap();
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            bit_depth,
            PixelFormat::Monochrome,
            size,
            rect,
        )
        .unwrap();
        let mut workspace = CurrentFrameWorkspace::new(info, T::default()).unwrap();
        let max = u32::from(bit_depth.max_sample());
        let samples: Vec<u16> = (0..(width * height) as u32)
            .map(|i| match (i * 7 + seed) % 11 {
                0 => 0,
                1 => max as u16,
                _ => {
                    ((i.wrapping_mul(2_654_435_761) ^ (seed * 40_503)) >> 7) as u16
                        % (max as u16 + 1)
                }
            })
            .collect();
        for (i, &sample) in samples.iter().enumerate() {
            let value = T::try_from_u16(sample).unwrap();
            workspace
                .set_reconstructed_sample(PlaneId::Y, i % width, i / width, value)
                .unwrap();
        }
        (workspace, samples)
    }

    /// The per-sample § 7.13.3.30 interintra blend of the oracle.
    fn oracle_interintra(
        frame: &mut [u16],
        frame_width: usize,
        (left, top, width, height): (usize, usize, usize, usize),
        intra: &[u16],
        intra_stride: usize,
        mask: impl Fn(usize, usize) -> u16,
    ) {
        for i in 0..height {
            for j in 0..width {
                let weight = u32::from(mask(i, j));
                let at = (top + i) * frame_width + left + j;
                let (pred0, pred1) = (u32::from(intra[i * intra_stride + j]), u32::from(frame[at]));
                frame[at] = ((weight * pred0 + (64 - weight) * pred1 + 32) >> 6) as u16;
            }
        }
    }

    fn check_interintra_blends<T: ReconSample>(bit_depth: BitDepth) {
        let max = bit_depth.max_sample();
        let modes = [
            InterIntraMode::Dc,
            InterIntraMode::Vertical,
            InterIntraMode::Horizontal,
            InterIntraMode::Smooth,
        ];
        let wedge_sizes = [8, 16, 32, 64];
        for (log2_w, log2_h) in (2..=6).flat_map(|w| (2..=6).map(move |h| (w, h))) {
            let size = IntraRectBlockSize::new(log2_w, log2_h).unwrap();
            let (w, h) = (size.width(), size.height());
            let intra: Vec<u16> = (0..w * h)
                .map(|i| [max, 0, (i as u16).wrapping_mul(37) % (max + 1)][i % 3])
                .collect();
            let intra_t: Vec<T> = intra.iter().map(|&s| T::try_from_u16(s).unwrap()).collect();
            let wedges = (0..MAX_WEDGE_TYPES).map(Some);
            let blends = modes.iter().copied().map(Ok).chain(wedges.map(Err));
            for (n, blend) in blends.enumerate() {
                let (sub_x, sub_y) = [(0, 0), (1, 0), (1, 1)][n % 3];
                let (lw, lh) = (w << sub_x, h << sub_y);
                if blend.is_err() && !(wedge_sizes.contains(&lw) && wedge_sizes.contains(&lh)) {
                    continue;
                }
                for (frame, x, y) in [((w + 5, h + 3), 3, 2), ((w + 2, h - 1), 3, 1)] {
                    let (mut workspace, mut want) =
                        noisy_workspace::<T>(bit_depth, frame, n as u32);
                    let clipped = (w.min(frame.0 - x), h.min(frame.1 - y));
                    let rect = (x, y, clipped.0, clipped.1);
                    match blend {
                        Ok(mode) => {
                            let scale = 128 / w.max(h);
                            workspace
                                .blend_smooth_interintra_rect(
                                    PlaneId::Y,
                                    x,
                                    y,
                                    size,
                                    mode,
                                    &intra_t,
                                )
                                .unwrap();
                            oracle_interintra(&mut want, frame.0, rect, &intra, w, |i, j| {
                                intra_mode_variant_weight(mode, i, j, scale)
                            });
                        }
                        Err(index) => {
                            let index = index.unwrap();
                            let wedge = |i, j| {
                                wedge_mask_plane_sample(lw, lh, index, false, sub_x, sub_y, j, i)
                                    .unwrap()
                            };
                            workspace
                                .blend_wedge_interintra_rect(
                                    PlaneId::Y,
                                    x,
                                    y,
                                    size,
                                    lw,
                                    lh,
                                    index,
                                    sub_x,
                                    sub_y,
                                    &intra_t,
                                )
                                .unwrap();
                            oracle_interintra(&mut want, frame.0, rect, &intra, w, wedge);
                        }
                    }
                    for (i, &want) in want.iter().enumerate() {
                        let got = workspace
                            .reconstructed_sample(PlaneId::Y, i % frame.0, i / frame.0)
                            .unwrap();
                        assert_eq!(got.to_u16(), want, "{w}x{h} {blend:?} {frame:?} {i}");
                    }
                }
            }
        }
    }

    /// Every § 7.13.3.29 mode and § 7.13.3.27 wedge at every block size,
    /// both bit depths, inside storage and clipped at its edge: the row
    /// kernels equal the per-sample oracle.
    #[test]
    fn interintra_blends_match_the_sample_oracle() {
        assert!(II_WEIGHTS_1D.windows(2).all(|pair| pair[0] >= pair[1]));
        check_interintra_blends::<u8>(BitDepth::Eight);
        check_interintra_blends::<u16>(BitDepth::Ten);
    }

    fn check_bawp_scales<T: ReconSample>(bit_depth: BitDepth) {
        let max = i32::from(bit_depth.max_sample());
        for (w, h) in [
            (2, 4),
            (4, 4),
            (6, 8),
            (8, 4),
            (10, 2),
            (16, 8),
            (18, 3),
            (32, 2),
        ] {
            for (alpha, beta) in [(256, -128), (320, -9000), (-48, 70_000), (511, 3)] {
                let frame = (w + 3, h + 1);
                let (mut workspace, mut want) = noisy_workspace::<T>(bit_depth, frame, w as u32);
                let rect = PlaneRect::new(3, 1, w + 4, h).unwrap();
                let scaled = workspace.apply_bawp_rect(PlaneId::Y, rect, alpha, beta);
                assert_eq!(scaled.is_ok(), w % 4 == 0, "{w}x{h}");
                for (i, sample) in want.iter_mut().enumerate() {
                    if w % 4 != 0 {
                        break;
                    }
                    if i % frame.0 >= 3 && i / frame.0 >= 1 {
                        let scaled = (i32::from(*sample) * i32::from(alpha) + beta) >> 8;
                        *sample = scaled.clamp(0, max) as u16;
                    }
                }
                for (i, &want) in want.iter().enumerate() {
                    let got = workspace
                        .reconstructed_sample(PlaneId::Y, i % frame.0, i / frame.0)
                        .unwrap();
                    assert_eq!(got.to_u16(), want, "{w}x{h} {alpha} {beta} {i}");
                }
            }
        }
    }

    /// § 7.13.3.25 scales of rects clipped at the storage edge, against the
    /// per-sample `Clip1`; a clamped width that is not a multiple of 4 is an
    /// error and writes nothing.
    #[test]
    fn bawp_scale_of_clipped_rects_matches_the_sample_oracle() {
        check_bawp_scales::<u8>(BitDepth::Eight);
        check_bawp_scales::<u16>(BitDepth::Ten);
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
