// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.25 block adaptive weighted prediction.

use splot_core::span::ByteOffset;
use splot_recon::{
    CurrentFramePlane, CurrentFrameWorkspace, IntraRectBlockSize, PlaneId, PlaneRect, ReconSample,
};

use super::reference::ReferenceSamples;
use super::{BawpSyntax, MotionVector, PlacedInterBlock};
use crate::Result;
use fearless_simd::{prelude::*, u16x16};

const SHIFT: u32 = 8;

/// Rows below its top-left reference position a § 7.13.3.25 template reaches:
/// [`bawp_template_counts`] caps the template side at 16 luma samples.
const MAX_TEMPLATE_EXTENT: usize = 16;

pub(super) const fn to_fullmv(mv: i32) -> i32 {
    (mv + 3 + if mv >= 0 { 1 } else { 0 }) >> 3
}

/// The luma rows of its reference one § 7.13.3.25 BAWP block's template reads.
///
/// The template samples the reference at the block's full-pel position
/// `refY = lumaY + toFullMv(mvRow)`: one row above it for the above arm, and
/// rows `refY ..= refY + height - 1` for the left arm, where `height` is
/// [`bawp_template_counts`]'s capped block side — at most
/// [`MAX_TEMPLATE_EXTENT`] luma samples, and never more than the block's own
/// height. A chroma arm reads the same window subsampled, whose luma-row
/// requirement `(refY >> subY) + (height >> subY)` shifted back up never passes
/// the luma one, so a single luma-row count covers every plane.
pub(super) fn bawp_reference_luma_rows(luma_y: usize, luma_h: usize, mv_row: i32) -> u32 {
    let top = (luma_y as i64) + i64::from(to_fullmv(mv_row));
    let extent = luma_h.min(MAX_TEMPLATE_EXTENT) as i64;
    top.saturating_add(extent).clamp(0, i64::from(u32::MAX)) as u32
}

#[derive(Default)]
#[cfg_attr(test, derive(Debug, PartialEq))]
struct TemplateStats {
    sum_x: i32,
    sum_y: i32,
    sum_xx: i32,
    sum_xy: i32,
    count: i32,
}

impl TemplateStats {
    fn implicit_alpha(&self) -> i16 {
        let mut alpha = 1i16 << SHIFT;
        if self.count > 0 {
            let nor = self.sum_xy - self.sum_x * self.sum_y / self.count;
            let der = self.sum_xx - self.sum_x * self.sum_x / self.count;
            if der != 0 && nor != 0 {
                alpha = splot_recon::math::resolve_division(nor, der, SHIFT as u8);
                if alpha == 0 {
                    alpha = 1 << SHIFT;
                }
            }
        }
        alpha
    }

    fn beta(&self, alpha: i16) -> i32 {
        if self.count > 0 {
            ((self.sum_y << SHIFT) - self.sum_x * i32::from(alpha)) / self.count
        } else {
            -(1 << (SHIFT - 1))
        }
    }
}

/// The `(reference, current)` samples of a § 7.13.3.25 template, widened to
/// `u16`: the leading 16 samples of both above rows, of which the sums keep
/// the `len_up / num_up` step, and the `num_left <= 16` left pairs, the
/// unused lanes zero. With samples at most 1023, `sumX` and `sumY` stay at
/// most `32 * 1023` and `sumXX` and `sumXY` at most `32 * 1023 * 1023 < 2^31`.
#[derive(Default)]
struct TemplatePairs {
    above: [[u16; 16]; 2],
    left: [[u16; 16]; 2],
    up: (usize, usize),
    count: i32,
}

impl TemplatePairs {
    /// The above arm of `len` samples and `num` pairs from the rows
    /// `reference` and `current`, which hold at least `len` samples.
    #[inline]
    fn add_above<T: ReconSample>(
        &mut self,
        (len, num): (usize, usize),
        [reference, current]: [&[T]; 2],
    ) -> Option<()> {
        let step = len.checked_div(num).filter(|&step| step > 0 && len <= 16)?;
        for (lanes, row) in self.above.iter_mut().zip([reference, current]) {
            if let Some(row) = T::u8_slice(row).and_then(<[u8]>::first_chunk::<16>) {
                *lanes = row.map(u16::from);
            } else if let Some(row) = T::u16_slice(row).and_then(<[u16]>::first_chunk::<16>) {
                *lanes = *row;
            } else {
                let row = row.get(..len)?;
                lanes
                    .iter_mut()
                    .zip(row)
                    .for_each(|(lane, sample)| *lane = sample.to_u16());
            }
        }
        self.up = (len, step);
        self.count = self.count.checked_add(i32::try_from(num).ok()?)?;
        Some(())
    }

    /// The left arm: the pairs `pair(i)` at `i = step >> 1, step + (step >> 1),
    /// ...` below `len`, `step = len / num`.
    fn add_left(
        &mut self,
        (len, num): (usize, usize),
        mut pair: impl FnMut(usize) -> Option<(u16, u16)>,
    ) -> Option<()> {
        let step = len.checked_div(num).filter(|&step| step > 0)?;
        let [reference, current] = &mut self.left;
        for k in 0..num.min(16) {
            (reference[k], current[k]) = pair((step >> 1) + k * step)?;
        }
        self.count = self.count.checked_add(i32::try_from(num).ok()?)?;
        Some(())
    }

    /// The template sums: the above lanes past `len` or off the step
    /// zeroed, `u16` lanes for the sample sums and `u32` lanes for the
    /// products.
    #[inline]
    fn stats(&self) -> TemplateStats {
        const LANE: [u16; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let (len, step) = self.up;
        baseline_simd!(simd => {
            let lane = u16x16::load_array(simd, LANE);
            let keep = lane.simd_lt(u16x16::splat(simd, len as u16))
                & (lane & u16x16::splat(simd, step.wrapping_sub(1) as u16))
                    .simd_eq(u16x16::splat(simd, (step >> 1) as u16));
            let zero = u16x16::splat(simd, 0);
            let above = self.above.map(|lanes| keep.select(u16x16::load_array(simd, lanes), zero));
            let left = self.left.map(|lanes| u16x16::load_array(simd, lanes));
            let (x, y) = ([above[0], left[0]], [above[1], left[1]]);
            let products = |a: [u16x16<_>; 2], b: [u16x16<_>; 2]| {
                let ((low0, high0), (low1, high1)) = (a[0].widening_mul(b[0]), a[1].widening_mul(b[1]));
                (low0 + high0 + low1 + high1).reduce_sum() as i32
            };
            TemplateStats {
                sum_x: i32::from((x[0] + x[1]).reduce_sum()),
                sum_y: i32::from((y[0] + y[1]).reduce_sum()),
                sum_xx: products(x, x),
                sum_xy: products(x, y),
                count: self.count,
            }
        })
    }
}

/// The samples above and left of a block's template in a current-frame
/// plane, clamped to storage like the DC edges: indices past the stored
/// extent repeat the last stored sample.
struct CurrentTemplate<'a, T> {
    above: &'a [T],
    left: &'a [T],
    stride: usize,
    last_col: usize,
    last_row: usize,
}

impl<'a, T: ReconSample> CurrentTemplate<'a, T> {
    /// `None` when an arm the template uses lies outside the frame.
    fn new(
        plane: &'a CurrentFramePlane<T>,
        (x, y): (usize, usize),
        (width, height, num_up, num_left): (usize, usize, usize, usize),
    ) -> Result<Option<Self>> {
        let storage = plane.storage_size();
        let up = usize::from(num_up > 0);
        let (Some(top), Some(stored_cols), Some(stored_rows)) = (
            y.checked_sub(up),
            storage.width().checked_sub(x),
            storage.height().checked_sub(y),
        ) else {
            return Ok(None);
        };
        let (Some(last_col), Some(last_row)) = (
            width.min(stored_cols).checked_sub(1),
            height.min(stored_rows).checked_sub(1),
        ) else {
            return Ok(None);
        };
        if num_left > 0 && x == 0 {
            return Ok(None);
        }
        let (stride, rows) = (plane.stride_samples(), plane.rows(top, y + last_row + 1)?);
        Ok(Some(Self {
            above: rows.get(x..).unwrap_or_default(),
            left: rows
                .get((up * stride + x).saturating_sub(1)..)
                .unwrap_or_default(),
            stride,
            last_col,
            last_row,
        }))
    }

    /// The row above the template from its first column, at least `len`
    /// samples: past the stored extent, `padded` repeats the last stored
    /// sample.
    fn above_row<'b>(&'b self, len: usize, padded: &'b mut [T; 16]) -> Option<&'b [T]> {
        if self.last_col + 1 >= len {
            return self.above.get(..len).map(|_| self.above);
        }
        let stored = self.above.get(..=self.last_col)?;
        let (head, tail) = padded.get_mut(..len)?.split_at_mut(stored.len());
        head.copy_from_slice(stored); // splot-copy-ok: template row padded past the frame edge
        tail.fill(*stored.last()?);
        Some(padded)
    }

    #[inline]
    fn left(&self, i: usize) -> Option<u16> {
        Some(self.left.get(i.min(self.last_row) * self.stride)?.to_u16())
    }
}

/// The § 7.13.3.25 template sums: `above_ref` is the reference row above
/// the block from its first column, `left_ref(i)` the reference sample left
/// of block row `i`.
#[inline]
fn collect_template_stats<T: ReconSample>(
    current: &CurrentTemplate<'_, T>,
    (width, height, num_up, num_left): (usize, usize, usize, usize),
    above_ref: &[T],
    mut left_ref: impl FnMut(usize) -> Option<u16>,
) -> Option<TemplateStats> {
    let mut pairs = TemplatePairs::default();
    if num_up > 0 {
        let mut padded = [T::default(); 16];
        let above = current.above_row(width, &mut padded)?;
        above_ref.get(..width)?;
        pairs.add_above((width, num_up), [above_ref, above])?;
    }
    if num_left > 0 {
        pairs.add_left((height, num_left), |i| {
            Some((left_ref(i)?, current.left(i)?))
        })?;
    }
    Some(pairs.stats())
}

pub(crate) fn apply_bawp<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: ReferenceSamples<'_, T>,
    placed: &PlacedInterBlock,
    bawp: BawpSyntax,
    mv: MotionVector,
    tile_offset: ByteOffset,
) -> Result<()> {
    let mut luma_alpha = 1i16 << SHIFT;
    for (plane, sub_x, sub_y) in super::mc::mc_planes(workspace.info().pixel_format()) {
        if plane != PlaneId::Y && !bawp.chroma {
            break;
        }
        luma_alpha = apply_bawp_plane(
            workspace,
            reference,
            placed,
            bawp,
            mv,
            plane,
            sub_x,
            sub_y,
            luma_alpha,
            tile_offset,
        )?;
    }
    Ok(())
}

pub(crate) fn apply_intrabc_morph_pred<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    target: PlaneRect,
    (avail_up, avail_left): (bool, bool),
    mv: MotionVector,
) -> Result<()> {
    let dy = to_fullmv(mv.row);
    let dx = to_fullmv(mv.col);
    let Some(ref_y) = fullpel_ref_pos(target.y(), dy) else {
        return Ok(());
    };
    let Some(ref_x) = fullpel_ref_pos(target.x(), dx) else {
        return Ok(());
    };
    if ref_x < 1 || ref_y < 1 {
        return Ok(());
    }

    let plane = workspace.plane(PlaneId::Y)?;
    let Some(plane_width) = i32::try_from(plane.storage_size().width()).ok() else {
        return Ok(());
    };
    let Some(plane_height) = i32::try_from(plane.storage_size().height()).ok() else {
        return Ok(());
    };
    let Some(bw) = i32::try_from(target.width()).ok() else {
        return Ok(());
    };
    let Some(bh) = i32::try_from(target.height()).ok() else {
        return Ok(());
    };
    let Some(ref_right) = ref_x.checked_add(bw) else {
        return Ok(());
    };
    let Some(ref_bottom) = ref_y.checked_add(bh) else {
        return Ok(());
    };
    if ref_right > plane_width || ref_bottom > plane_height {
        return Ok(());
    }

    let (width, height, num_up, num_left) =
        bawp_template_counts(target.width(), target.height(), true, avail_up, avail_left);
    let dims = (width, height, num_up, num_left);
    if morph_dimensions_size(width, height).is_none() {
        return Ok(());
    }
    let plane = workspace.plane(PlaneId::Y)?;
    let Some(current) = CurrentTemplate::new(plane, (target.x(), target.y()), dims)? else {
        return Ok(());
    };
    let (Ok(ref_x), Ok(ref_y)) = (usize::try_from(ref_x), usize::try_from(ref_y)) else {
        return Ok(());
    };
    let Some(reference) = CurrentTemplate::new(plane, (ref_x, ref_y), dims)
        .ok()
        .flatten()
    else {
        return Ok(());
    };
    let mut padded = [T::default(); 16];
    let above = reference.above_row(width, &mut padded).unwrap_or_default();
    let Some(stats) = collect_template_stats(&current, dims, above, |i| reference.left(i)) else {
        return Ok(());
    };

    let alpha = stats.implicit_alpha();
    let beta = stats.beta(alpha);

    workspace.apply_bawp_rect(PlaneId::Y, target, alpha, beta)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_bawp_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: ReferenceSamples<'_, T>,
    placed: &PlacedInterBlock,
    bawp: BawpSyntax,
    mv: MotionVector,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    luma_alpha: i16,
    tile_offset: ByteOffset,
) -> Result<i16> {
    let plane_x = placed.luma_x() >> sub_x;
    let plane_y = placed.luma_y() >> sub_y;
    let plane_w = placed.luma_w() >> sub_x;
    let plane_h = placed.luma_h() >> sub_y;
    let last_row = (bawp_reference_luma_rows(placed.luma_y(), placed.luma_h(), mv.row) >> sub_y)
        .saturating_sub(1) as i32;
    let ref_samples = reference.plane_view(plane, last_row, tile_offset)?;
    let (ref_width, ref_height) = (ref_samples.width(), ref_samples.height());
    let dy = to_fullmv(mv.row);
    let dx = to_fullmv(mv.col);
    let Some(ref_y) = fullpel_ref_pos(placed.luma_y(), dy).map(|y| y >> sub_y) else {
        return Ok(luma_alpha);
    };
    let Some(ref_x) = fullpel_ref_pos(placed.luma_x(), dx).map(|x| x >> sub_x) else {
        return Ok(luma_alpha);
    };
    let Some(plane_width) = i32::try_from(ref_width).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_height) = i32::try_from(ref_height).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_x_i32) = i32::try_from(plane_x).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_y_i32) = i32::try_from(plane_y).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_w_i32) = i32::try_from(plane_w).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_h_i32) = i32::try_from(plane_h).ok() else {
        return Ok(luma_alpha);
    };
    let (bw, bh) = (
        plane_w_i32.min(plane_width - plane_x_i32),
        plane_h_i32.min(plane_height - plane_y_i32),
    );
    if bw <= 0 || bh <= 0 {
        return Ok(luma_alpha);
    }
    let Some(ref_right) = ref_x.checked_add(bw) else {
        return Ok(luma_alpha);
    };
    let Some(ref_bottom) = ref_y.checked_add(bh) else {
        return Ok(luma_alpha);
    };
    if ref_x < 1 || ref_y < 1 || ref_right > plane_width || ref_bottom > plane_height {
        return Ok(luma_alpha);
    }

    if !plane_w.is_power_of_two() || !plane_h.is_power_of_two() {
        return Ok(luma_alpha);
    }
    let Some(width_for_counts) = usize::try_from(bw).ok() else {
        return Ok(luma_alpha);
    };
    let Some(height_for_counts) = usize::try_from(bh).ok() else {
        return Ok(luma_alpha);
    };
    let (width, height, num_up, num_left) = bawp_template_counts(
        width_for_counts,
        height_for_counts,
        plane == PlaneId::Y,
        bawp.avail_up,
        bawp.avail_left,
    );
    let dims = (width, height, num_up, num_left);
    if morph_dimensions_size(width, height).is_none() {
        return Ok(luma_alpha);
    }
    let Some(current) = CurrentTemplate::new(workspace.plane(plane)?, (plane_x, plane_y), dims)?
    else {
        return Ok(luma_alpha);
    };
    let (Ok(ref_x), Ok(ref_y)) = (usize::try_from(ref_x), usize::try_from(ref_y)) else {
        return Ok(luma_alpha);
    };
    let above_row = ref_samples.row(ref_y - 1).get(ref_x..).unwrap_or_default();
    let Some(stats) = collect_template_stats(&current, dims, above_row, |i| {
        Some(ref_samples.sample(ref_y + i, ref_x - 1) as u16)
    }) else {
        return Ok(luma_alpha);
    };

    let alpha = if plane != PlaneId::Y {
        if stats.count == 0 {
            1 << SHIFT
        } else {
            luma_alpha
        }
    } else if bawp.explicit {
        let mut scale = i16::from(bawp.list_index) + 1;
        if bawp.ref_dist_gt4 {
            scale += 1;
        }
        if !bawp.explicit_scale_positive {
            scale = -scale;
        }
        256 + 16 * scale
    } else {
        stats.implicit_alpha()
    };
    let beta = stats.beta(alpha);

    workspace.apply_bawp_rect(
        plane,
        PlaneRect::new(plane_x, plane_y, plane_w, plane_h)?,
        alpha,
        beta,
    )?;
    Ok(if plane == PlaneId::Y {
        alpha
    } else {
        luma_alpha
    })
}

fn morph_dimensions_size(width: usize, height: usize) -> Option<IntraRectBlockSize> {
    if !width.is_power_of_two() || !height.is_power_of_two() {
        return None;
    }
    IntraRectBlockSize::new(
        u8::try_from(width.trailing_zeros()).ok()?,
        u8::try_from(height.trailing_zeros()).ok()?,
    )
    .ok()
}

fn fullpel_ref_pos(origin: usize, delta: i32) -> Option<i32> {
    i32::try_from(origin)
        .ok()
        .and_then(|origin| origin.checked_add(delta))
}

fn bawp_template_counts(
    bw: usize,
    bh: usize,
    luma: bool,
    avail_up: bool,
    avail_left: bool,
) -> (usize, usize, usize, usize) {
    let cap = if luma { 16 } else { 8 };
    let bw2 = cap.min(bw);
    let bh2 = cap.min(bh);
    let width = if bw2 == 12 { 8 } else { bw2 };
    let height = if bh2 == 12 { 8 } else { bh2 };
    let (num_up, num_left) = if avail_up && avail_left {
        if width == 16 && height == 16 {
            (16, 16)
        } else if width > 4 && height > 4 {
            (8, 8)
        } else if width < 16 && height < 16 {
            (4, 4)
        } else if width == 16 {
            (16, 0)
        } else {
            (0, 16)
        }
    } else if avail_up {
        (width, 0)
    } else if avail_left {
        (0, height)
    } else {
        (0, 0)
    };
    (width, height, num_up, num_left)
}

#[cfg(test)]
#[path = "bawp_tests.rs"]
mod tests;
