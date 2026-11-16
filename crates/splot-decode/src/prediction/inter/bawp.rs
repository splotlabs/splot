// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.25 block adaptive weighted prediction.

use splot_core::span::ByteOffset;
use splot_recon::{CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample};

use super::reference::ReferenceSamples;
use super::{BawpSyntax, Mv, PlacedInterBlock};
use crate::Result;

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

/// The § 7.13.3.25 template sums of `count` pairs. `count` is `numUp +
/// numLeft`, a power of two: every arm count of the table is 4, 8, 16, the
/// template `width`, or its `height`, and [`template_arms`] admits only
/// widths and heights of 4, 8 or 16, so a two-arm template holds 8, 16 or
/// 32 pairs and a one-arm template 4, 8 or 16. The spec's divisions by
/// `count` are therefore shifts by `log2(count)`.
#[cfg_attr(test, derive(Debug, PartialEq))]
struct TemplateStats {
    sum_x: i32,
    sum_y: i32,
    sum_xx: i32,
    sum_xy: i32,
    log2_count: Option<u32>,
}

impl TemplateStats {
    fn new([sum_x, sum_y, sum_xx, sum_xy]: [i32; 4], count: usize) -> Self {
        Self {
            sum_x,
            sum_y,
            sum_xx,
            sum_xy,
            log2_count: (count > 0).then(|| count.trailing_zeros()),
        }
    }

    /// `nor` and `der` divide non-negative products, where `/` and `>>`
    /// agree.
    fn implicit_alpha(&self) -> i16 {
        let mut alpha = 1i16 << SHIFT;
        if let Some(log2) = self.log2_count {
            let nor = self.sum_xy - ((self.sum_x * self.sum_y) >> log2);
            let der = self.sum_xx - ((self.sum_x * self.sum_x) >> log2);
            if der != 0 && nor != 0 {
                alpha = splot_recon::math::resolve_division(nor, der, SHIFT as u8);
                if alpha == 0 {
                    alpha = 1 << SHIFT;
                }
            }
        }
        alpha
    }

    /// `((sumY << 8) - sumX * alpha) / count` with the § 4 `/` that
    /// truncates toward zero: a negative numerator rounds up by adding
    /// `count - 1` before the shift.
    fn beta(&self, alpha: i16) -> i32 {
        match self.log2_count {
            Some(log2) => {
                let numerator = (self.sum_y << SHIFT) - self.sum_x * i32::from(alpha);
                (numerator + ((numerator >> 31) & ((1 << log2) - 1))) >> log2
            }
            None => -(1 << (SHIFT - 1)),
        }
    }
}

/// Out of line: inlined, it grew the reconstruction of every inter block.
#[inline(never)]
pub(crate) fn apply_bawp<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: ReferenceSamples<'_, T>,
    placed: &PlacedInterBlock,
    bawp: BawpSyntax,
    mv: Mv,
    tile_offset: ByteOffset,
) -> Result<()> {
    let block = BawpBlock {
        luma: [
            placed.luma_x(),
            placed.luma_y(),
            placed.luma_w(),
            placed.luma_h(),
        ],
        reference: fullpel_ref_pos(placed.luma_x(), to_fullmv(mv.col))
            .zip(fullpel_ref_pos(placed.luma_y(), to_fullmv(mv.row))),
        luma_rows: bawp_reference_luma_rows(placed.luma_y(), placed.luma_h(), mv.row),
    };
    let mut luma_alpha = 1i16 << SHIFT;
    for (plane, sub_x, sub_y) in super::mc::mc_planes(workspace.info().pixel_format()) {
        if plane != PlaneId::Y && !bawp.chroma {
            break;
        }
        let view = reference.plane_view(
            plane,
            (block.luma_rows >> sub_y).saturating_sub(1) as i32,
            tile_offset,
        )?;
        let Some(geometry) = block.plane(plane, (sub_x, sub_y), bawp, &view) else {
            continue;
        };
        let luma = plane == PlaneId::Y;
        let explicit = luma.then_some(bawp).filter(|bawp| bawp.explicit);
        let count = geometry.arms.0.1 + geometry.arms.1.1;
        let fit = |sums| {
            let stats = TemplateStats::new(sums, count);
            let alpha = if let Some(bawp) = explicit {
                let mut scale = i16::from(bawp.list_index) + 1;
                if bawp.ref_dist_gt4 {
                    scale += 1;
                }
                if !bawp.explicit_scale_positive {
                    scale = -scale;
                }
                256 + 16 * scale
            } else if luma {
                stats.implicit_alpha()
            } else if count == 0 {
                1 << SHIFT
            } else {
                luma_alpha
            };
            (alpha, stats.beta(alpha))
        };
        let alpha = workspace.apply_bawp_template(
            plane,
            Some(&view),
            geometry.at,
            geometry.arms,
            luma && explicit.is_none(),
            geometry.size,
            fit,
        )?;
        if let Some(alpha) = alpha.filter(|_| luma) {
            luma_alpha = alpha;
        }
    }
    Ok(())
}

/// The block-constant § 7.13.3.25 geometry: the luma block
/// `[x, y, w, h]`, the full-pel luma reference position
/// `(MiCol * MI_SIZE + dx, MiRow * MI_SIZE + dy)`, and the reference luma
/// rows the template reaches.
struct BawpBlock {
    luma: [usize; 4],
    reference: Option<(i32, i32)>,
    luma_rows: u32,
}

/// One plane's § 7.13.3.25 template: the block and reference positions, the
/// arms, and the block size the scale covers.
struct PlaneTemplate {
    at: ((usize, usize), (usize, usize)),
    arms: ((usize, usize), (usize, usize)),
    size: (usize, usize),
}

impl BawpBlock {
    /// `None` when § 7.13.3.25 leaves the plane unscaled here: the
    /// reference block outside the bitstream-conformance bounds
    /// `refX >= 1`, `refY >= 1`, `refX + bw <= planeWidth`,
    /// `refY + bh <= planeHeight` of the reference `view`, the block
    /// outside the plane, or a template side other than 4, 8 or 16.
    fn plane<T: ReconSample>(
        &self,
        plane: PlaneId,
        (sub_x, sub_y): (u32, u32),
        bawp: BawpSyntax,
        view: &splot_recon::ReferencePlaneView<'_, T>,
    ) -> Option<PlaneTemplate> {
        let [x, y, w, h] = [
            self.luma[0] >> sub_x,
            self.luma[1] >> sub_y,
            self.luma[2] >> sub_x,
            self.luma[3] >> sub_y,
        ];
        let (ref_x, ref_y) = self.reference?;
        let (ref_x, ref_y) = (ref_x >> sub_x, ref_y >> sub_y);
        if ref_x < 1 || ref_y < 1 || !w.is_power_of_two() || !h.is_power_of_two() {
            return None;
        }
        let (ref_x, ref_y) = (ref_x as usize, ref_y as usize);
        let bw = view.width().checked_sub(x)?.min(w);
        let bh = view.height().checked_sub(y)?.min(h);
        if bw == 0 || bh == 0 || ref_x + bw > view.width() || ref_y + bh > view.height() {
            return None;
        }
        let arms = template_arms(bw, bh, plane == PlaneId::Y, bawp.avail_up, bawp.avail_left)?;
        Some(PlaneTemplate {
            at: ((x, y), (ref_x, ref_y)),
            arms,
            size: (w, h),
        })
    }
}

pub(crate) fn apply_intrabc_morph_pred<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    target: PlaneRect,
    (avail_up, avail_left): (bool, bool),
    mv: Mv,
) -> Result<()> {
    let (Some(ref_x), Some(ref_y)) = (
        fullpel_ref_pos(target.x(), to_fullmv(mv.col)),
        fullpel_ref_pos(target.y(), to_fullmv(mv.row)),
    ) else {
        return Ok(());
    };
    if ref_x < 1 || ref_y < 1 {
        return Ok(());
    }
    let (ref_x, ref_y) = (ref_x as usize, ref_y as usize);
    let storage = workspace.plane(PlaneId::Y)?.storage_size();
    if ref_x + target.width() > storage.width() || ref_y + target.height() > storage.height() {
        return Ok(());
    }
    let Some(arms) = template_arms(target.width(), target.height(), true, avail_up, avail_left)
    else {
        return Ok(());
    };
    let count = arms.0.1 + arms.1.1;
    workspace.apply_bawp_template(
        PlaneId::Y,
        None,
        ((target.x(), target.y()), (ref_x, ref_y)),
        arms,
        true,
        (target.width(), target.height()),
        |sums| {
            let stats = TemplateStats::new(sums, count);
            let alpha = stats.implicit_alpha();
            (alpha, stats.beta(alpha))
        },
    )?;
    Ok(())
}

/// The `((width, numUp), (height, numLeft))` arms of
/// [`bawp_template_counts`] when both template sides are 4, 8 or 16.
fn template_arms(
    bw: usize,
    bh: usize,
    luma: bool,
    avail_up: bool,
    avail_left: bool,
) -> Option<((usize, usize), (usize, usize))> {
    let (width, height, num_up, num_left) =
        bawp_template_counts(bw, bh, luma, avail_up, avail_left);
    (matches!(width, 4 | 8 | 16) && matches!(height, 4 | 8 | 16))
        .then_some(((width, num_up), (height, num_left)))
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
