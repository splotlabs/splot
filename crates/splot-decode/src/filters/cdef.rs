// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{
    Mask, Simd, SimdElement, cmp::SimdOrd, cmp::SimdPartialEq, num::SimdUint, simd_swizzle,
};

use splot_core::headers::frame::FrameHeaderCore;
use splot_recon::{
    BitDepth, CDEF_DIRECTIONS, CDEF_PADDED_AREA, CDEF_PADDED_SIDE, CDEF_PAIR_OUTPUT,
    CDEF_PAIR_SEGMENT_BLOCK_AREA, CDEF_PAIR_SEGMENT_STRIDE, CDEF_PAIR_STRIDE,
    CDEF_SEGMENT_BLOCK_AREA, CDEF_SEGMENT_BLOCKS, CDEF_SEGMENT_STRIDE, CDEF_UNAVAILABLE,
    CDEF_UV_DIR, CdefBlockFilter, CdefSampleTaps, CdefTap, PlaneId, PlaneRect, ReconSample,
    cdef_direction_padded, cdef_direction_segment, cdef_filter_block_boundary_to_valid_stride,
    cdef_filter_block_chroma_pair, cdef_filter_block_chroma_pair_segment,
    cdef_filter_block_interior_to_valid_stride, cdef_filter_block_segment, cdef_filter_sample,
};

use super::source::{DeblockedPlanes, FramePlane, StripeInitialization, StripePlane};

const MI_SIZE: usize = 4;
const MI_SIZE_LOG2: u32 = 2;
const CDEF_UNIT_MI: usize = 16;
const STEP4: usize = 2;
const CHROMA_PAIR_SIDE: usize = 4;
const CHROMA_PAIR_SPAN: usize = CHROMA_PAIR_SIDE + 2 * CDEF_TAP_REACH;
const SEGMENT_MI: usize = STEP4 * CDEF_SEGMENT_BLOCKS;
const LUMA_SEGMENT_AREA: usize = 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_SEGMENT_BLOCK_AREA;
const PAIR_SEGMENT_AREA: usize = 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_PAIR_SEGMENT_BLOCK_AREA;

/// Resolves the MI span of the tile containing `pos`.
///
/// `starts` carries an end sentinel, so consecutive pairs bound one tile. It is
/// `None` unless the frame sets `disable_loopfilters_across_tiles`, in which
/// case AV2 keeps CDEF inside the frame and the span is the whole picture.
pub(crate) fn tile_span(starts: Option<&[u32]>, pos: usize, frame_end: usize) -> (usize, usize) {
    let Some(starts) = starts else {
        return (0, frame_end);
    };
    starts
        .windows(2)
        .find_map(|w| {
            let (start, end) = (w[0] as usize, w[1] as usize);
            (start <= pos && pos < end).then_some((start, end.min(frame_end)))
        })
        .unwrap_or((0, frame_end))
}

const UNAVAILABLE_TAP: CdefTap = CdefTap {
    value: 0,
    available: false,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct CdefFrameParams {
    pub(crate) y_pri: i32,
    pub(crate) y_sec: i32,
    pub(crate) uv_pri: i32,
    pub(crate) uv_sec: i32,
    pub(crate) damping: i32,
}

pub(crate) fn cdef_frame_strengths(
    core: &FrameHeaderCore,
    strengths: &mut Vec<CdefFrameParams>,
) -> Option<()> {
    strengths.clear();
    let cdef = core.cdef_params.as_ref()?;
    if !cdef.cdef_frame_enable {
        return None;
    }
    let damping = i32::from(cdef.cdef_damping?);
    strengths.reserve(cdef.strengths.len());
    for set in cdef.strengths.as_slice() {
        strengths.push(CdefFrameParams {
            y_pri: i32::from(set.y_pri_strength),
            y_sec: i32::from(set.y_sec_strength),
            uv_pri: i32::from(set.uv_pri_strength),
            uv_sec: i32::from(set.uv_sec_strength),
            damping,
        });
    }
    Some(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdefUnitGrid {
    rows: usize,
    cols: usize,
    values: Vec<Option<usize>>,
}

impl CdefUnitGrid {
    pub(crate) fn new(
        rows: usize,
        cols: usize,
        values: Vec<Option<usize>>,
    ) -> Result<Self, CdefError> {
        validate_grid_len(rows, cols, values.len())?;
        Ok(Self { rows, cols, values })
    }

    pub(crate) fn into_values(self) -> Vec<Option<usize>> {
        self.values
    }

    fn strength_for_mi(&self, mi_row: usize, mi_col: usize) -> Result<Option<usize>, CdefError> {
        let row = mi_row / CDEF_UNIT_MI;
        let col = mi_col / CDEF_UNIT_MI;
        if row >= self.rows || col >= self.cols {
            return Ok(None);
        }
        self.values
            .get(row * self.cols + col)
            .copied()
            .ok_or(CdefError::Geometry)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdefSkipGrid {
    rows: usize,
    cols: usize,
    values: Vec<bool>,
    skipped_block_prefix: Vec<usize>,
}

impl CdefSkipGrid {
    pub(crate) fn new(rows: usize, cols: usize, values: Vec<bool>) -> Result<Self, CdefError> {
        validate_grid_len(rows, cols, values.len())?;
        let mut skipped_block_prefix = Vec::with_capacity(rows.div_ceil(STEP4) + 1);
        skipped_block_prefix.push(0usize);
        for row in (0..rows).step_by(STEP4) {
            let mut skipped = 0usize;
            for col in (0..cols).step_by(STEP4) {
                let row_end = row.saturating_add(STEP4).min(rows);
                let col_end = col.saturating_add(STEP4).min(cols);
                if (row..row_end).all(|row| (col..col_end).all(|col| values[row * cols + col])) {
                    skipped += 1;
                }
            }
            skipped_block_prefix.push(
                skipped_block_prefix
                    .last()
                    .copied()
                    .unwrap_or_default()
                    .checked_add(skipped)
                    .ok_or(CdefError::Geometry)?,
            );
        }
        Ok(Self {
            rows,
            cols,
            values,
            skipped_block_prefix,
        })
    }

    fn has_all_skipped_8x8(
        &self,
        r_start: usize,
        r_end: usize,
        mi_rows: usize,
        mi_cols: usize,
    ) -> Result<bool, CdefError> {
        if self.rows != mi_rows
            || self.cols != mi_cols
            || r_start > r_end
            || r_end > mi_rows
            || !r_start.is_multiple_of(STEP4)
        {
            return Err(CdefError::Geometry);
        }
        let start = r_start / STEP4;
        let end = r_end.div_ceil(STEP4);
        let before = self
            .skipped_block_prefix
            .get(start)
            .copied()
            .ok_or(CdefError::Geometry)?;
        let after = self
            .skipped_block_prefix
            .get(end)
            .copied()
            .ok_or(CdefError::Geometry)?;
        Ok(after != before)
    }

    fn all_skipped_8x8(
        &self,
        mi_row: usize,
        mi_col: usize,
        mi_rows: usize,
        mi_cols: usize,
    ) -> Result<bool, CdefError> {
        let row_end = mi_row.saturating_add(STEP4).min(mi_rows);
        let col_end = mi_col.saturating_add(STEP4).min(mi_cols);
        if row_end <= mi_row || col_end <= mi_col {
            return Ok(false);
        }
        for row in mi_row..row_end {
            for col in mi_col..col_end {
                if row >= self.rows || col >= self.cols {
                    return Err(CdefError::Geometry);
                }
                let skipped = self
                    .values
                    .get(row * self.cols + col)
                    .copied()
                    .ok_or(CdefError::Geometry)?;
                if !skipped {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

fn validate_grid_len(rows: usize, cols: usize, len: usize) -> Result<(), CdefError> {
    if len == rows.checked_mul(cols).ok_or(CdefError::Geometry)? {
        Ok(())
    } else {
        Err(CdefError::Geometry)
    }
}

pub(crate) struct CdefFrame<'a, T> {
    pub(crate) deblocked_y: FramePlane<'a, T>,
    pub(crate) deblocked_u: Option<FramePlane<'a, T>>,
    pub(crate) deblocked_v: Option<FramePlane<'a, T>>,
    pub(crate) filtered_y: StripePlane,
    pub(crate) filtered_u: Option<StripePlane>,
    pub(crate) filtered_v: Option<StripePlane>,
}

struct CdefBlockLookup<'a> {
    strengths: &'a [CdefFrameParams],
    grid: &'a CdefUnitGrid,
    tile_row_starts: Option<&'a [u32]>,
    tile_col_starts: Option<&'a [u32]>,
    skip_grid: Option<&'a CdefSkipGrid>,
    lossless_grid: Option<&'a crate::filters::lossless::LosslessBlockGrid>,
    mi_rows: usize,
    mi_cols: usize,
    sub_x: usize,
    sub_y: usize,
    has_chroma: bool,
    coeff_shift: u32,
    max_sample: i32,
}

#[derive(Clone, Copy)]
struct CdefPlaneGeometry {
    width: usize,
    frame_height: usize,
    origin_y: usize,
    end_y: usize,
}

impl CdefPlaneGeometry {
    fn block_count(
        self,
        r_start: usize,
        r_end: usize,
        mi_cols: usize,
        sub_x: usize,
        sub_y: usize,
    ) -> Option<usize> {
        let block_width = 8usize.checked_shr(u32::try_from(sub_x).ok()?)?;
        let block_height = 8usize.checked_shr(u32::try_from(sub_y).ok()?)?;
        let columns = mi_cols.div_ceil(STEP4);
        let rows = r_end.checked_sub(r_start)?.div_ceil(STEP4);
        let expected_origin = r_start.checked_mul(MI_SIZE)?.checked_shr(sub_y as u32)?;
        let expected_end = expected_origin
            .checked_add(rows.checked_mul(block_height)?)?
            .min(self.frame_height);
        (self.width > 0
            && self.origin_y == expected_origin
            && self.end_y == expected_end
            && columns == self.width.div_ceil(block_width)
            && rows == (self.end_y - self.origin_y).div_ceil(block_height))
        .then(|| rows.checked_mul(columns))?
    }
}

fn cdef_params_guarantee_write(params: CdefFrameParams, plane: PlaneId) -> bool {
    match plane {
        PlaneId::Y => params.y_sec != 0,
        PlaneId::U | PlaneId::V => params.uv_pri != 0 || params.uv_sec != 0,
    }
}

impl CdefBlockLookup<'_> {
    /// The context of the 8x8 block at `(r, c)`, given its 64x64 unit's
    /// strength set (`None` when the unit's strength index is out of range)
    /// and the MI span of its tile row.
    #[allow(clippy::inline_always, reason = "measured CDEF per-block hot path")]
    #[inline(always)]
    fn at(
        &self,
        r: usize,
        c: usize,
        params: Option<CdefFrameParams>,
        (mi_row_start, mi_rows): (usize, usize),
    ) -> Result<Option<CdefBlockCtx>, CdefError> {
        if let Some(skip_grid) = self.skip_grid
            && skip_grid.all_skipped_8x8(r, c, self.mi_rows, self.mi_cols)?
        {
            return Ok(None);
        }
        let luma_lossless = self
            .lossless_grid
            .is_some_and(|grid| grid.cdef_luma_lossless(r, c));
        let chroma_lossless = self.lossless_grid.is_some_and(|grid| {
            grid.cdef_chroma_lossless(PlaneId::U, r, c)
                && grid.cdef_chroma_lossless(PlaneId::V, r, c)
        });
        if luma_lossless && (!self.has_chroma || chroma_lossless) {
            return Ok(None);
        }
        let params = params.ok_or(CdefError::Geometry)?;
        let (mi_col_start, mi_cols) = tile_span(self.tile_col_starts, c, self.mi_cols);
        Ok(Some(CdefBlockCtx {
            r,
            c,
            mi_row_start,
            mi_col_start,
            params,
            coeff_shift: self.coeff_shift,
            max_sample: self.max_sample,
            mi_rows,
            mi_cols,
            sub_x: self.sub_x,
            sub_y: self.sub_y,
            luma_lossless,
            chroma_lossless,
        }))
    }
}

fn cdef_initializations(
    lookup: Option<&CdefBlockLookup<'_>>,
    target: Option<&crate::pipeline::frame_progress::DirectStripeTarget>,
    geometry: [Option<CdefPlaneGeometry>; 3],
    luma_rows: (usize, usize),
) -> Result<[StripeInitialization; 3], CdefError> {
    let Some(lookup) = lookup else {
        return Ok([StripeInitialization::CopyAll; 3]);
    };
    let r_start = luma_rows.0 / MI_SIZE;
    let r_end = luma_rows.1.div_ceil(MI_SIZE).min(lookup.mi_rows);
    let mut remaining: [Option<usize>; 3] = core::array::from_fn(|index| {
        let plane = [PlaneId::Y, PlaneId::U, PlaneId::V][index];
        target
            .and_then(|target| target.get(plane))
            .filter(|target| target.is_u16())
            .and(geometry[index])
            .and_then(|geometry| {
                let (sub_x, sub_y) = if plane == PlaneId::Y {
                    (0, 0)
                } else {
                    (lookup.sub_x, lookup.sub_y)
                };
                geometry.block_count(r_start, r_end, lookup.mi_cols, sub_x, sub_y)
            })
    });
    if remaining.iter().all(Option::is_none) {
        return Ok([StripeInitialization::CopyAll; 3]);
    }

    let unit_row_start = r_start / CDEF_UNIT_MI;
    let unit_row_end = r_end.div_ceil(CDEF_UNIT_MI);
    let unit_cols = lookup.mi_cols.div_ceil(CDEF_UNIT_MI);
    'units: for unit_row in unit_row_start..unit_row_end {
        for unit_col in 0..unit_cols {
            let params = lookup
                .grid
                .strength_for_mi(unit_row * CDEF_UNIT_MI, unit_col * CDEF_UNIT_MI)?
                .and_then(|index| lookup.strengths.get(index))
                .copied();
            for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
                if remaining[plane.index()].is_some()
                    && !params.is_some_and(|params| cdef_params_guarantee_write(params, plane))
                {
                    remaining[plane.index()] = None;
                }
            }
            if remaining.iter().all(Option::is_none) {
                break 'units;
            }
        }
    }

    if lookup.skip_grid.is_some_and(|skip_grid| {
        skip_grid
            .has_all_skipped_8x8(r_start, r_end, lookup.mi_rows, lookup.mi_cols)
            .unwrap_or(true)
    }) {
        return Ok([StripeInitialization::CopyAll; 3]);
    }
    if let Some(lossless_grid) = lookup.lossless_grid {
        let mut r = r_start;
        'lossless: while r < r_end {
            let mut c = 0;
            while c < lookup.mi_cols {
                if remaining[PlaneId::Y.index()].is_some() && lossless_grid.cdef_luma_lossless(r, c)
                {
                    remaining[PlaneId::Y.index()] = None;
                }
                for plane in [PlaneId::U, PlaneId::V] {
                    if remaining[plane.index()].is_some()
                        && lossless_grid.cdef_chroma_lossless(plane, r, c)
                    {
                        remaining[plane.index()] = None;
                    }
                }
                if remaining.iter().all(Option::is_none) {
                    break 'lossless;
                }
                c += STEP4;
            }
            r += STEP4;
        }
    }
    Ok(remaining.map(|remaining| {
        if remaining.is_some() {
            StripeInitialization::FullyOverwritten
        } else {
            StripeInitialization::CopyAll
        }
    }))
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn cdef_stripe<'a, T: ReconSample>(
    deblocked: DeblockedPlanes<'a, T>,
    strengths: Option<&[CdefFrameParams]>,
    grid: Option<&CdefUnitGrid>,
    skip_grid: Option<&CdefSkipGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    mi_size: (usize, usize),
    subsampling: (usize, usize),
    bit_depth: BitDepth,
    tile_starts: Option<(&[u32], &[u32])>,
    luma_start: usize,
    luma_end: usize,
) -> Result<CdefFrame<'a, T>, CdefError> {
    cdef_stripe_into(
        deblocked,
        strengths,
        grid,
        skip_grid,
        lossless_grid,
        mi_size,
        subsampling,
        bit_depth,
        tile_starts,
        luma_start,
        luma_end,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cdef_stripe_into<'a, T: ReconSample>(
    deblocked: DeblockedPlanes<'a, T>,
    strengths: Option<&[CdefFrameParams]>,
    grid: Option<&CdefUnitGrid>,
    skip_grid: Option<&CdefSkipGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    mi_size: (usize, usize),
    subsampling: (usize, usize),
    bit_depth: BitDepth,
    tile_starts: Option<(&[u32], &[u32])>,
    luma_start: usize,
    luma_end: usize,
    mut target: Option<crate::pipeline::frame_progress::DirectStripeTarget>,
) -> Result<CdefFrame<'a, T>, CdefError> {
    let (mi_rows, mi_cols) = mi_size;
    if luma_start >= luma_end || !luma_start.is_multiple_of(STEP4 * MI_SIZE) {
        return Err(CdefError::Geometry);
    }
    let coeff_shift = u32::from(bit_depth.bits()) - 8;
    let max_sample = i32::from(bit_depth.max_sample());
    let (sub_x, sub_y) = subsampling;
    let has_chroma = deblocked.u.is_some();
    let deblocked_y = deblocked.y;
    let (deblocked_u, deblocked_v) = if has_chroma {
        (
            Some(deblocked.u.ok_or(CdefError::Workspace)?),
            Some(deblocked.v.ok_or(CdefError::Workspace)?),
        )
    } else {
        (None, None)
    };
    let chroma_start = luma_start >> sub_y;
    let chroma_end = luma_end.div_ceil(1usize << sub_y);
    let lookup = if let (Some(strengths), Some(grid)) = (strengths, grid) {
        Some(CdefBlockLookup {
            strengths,
            grid,
            tile_row_starts: tile_starts.map(|(rows, _)| rows),
            tile_col_starts: tile_starts.map(|(_, cols)| cols),
            skip_grid,
            lossless_grid,
            mi_rows,
            mi_cols,
            sub_x,
            sub_y,
            has_chroma,
            coeff_shift,
            max_sample,
        })
    } else {
        None
    };
    let geometry = [
        Some(CdefPlaneGeometry {
            width: deblocked_y.width(),
            frame_height: deblocked_y.frame_height(),
            origin_y: luma_start,
            end_y: luma_end,
        }),
        deblocked_u.map(|plane| CdefPlaneGeometry {
            width: plane.width(),
            frame_height: plane.frame_height(),
            origin_y: chroma_start,
            end_y: chroma_end,
        }),
        deblocked_v.map(|plane| CdefPlaneGeometry {
            width: plane.width(),
            frame_height: plane.frame_height(),
            origin_y: chroma_start,
            end_y: chroma_end,
        }),
    ];
    let mut initializations = cdef_initializations(
        lookup.as_ref(),
        target.as_ref(),
        geometry,
        (luma_start, luma_end),
    )?;
    let mut fill_flat = initializations.map(|init| init == StripeInitialization::FullyOverwritten);
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        if target
            .as_ref()
            .and_then(|target| target.get(plane))
            .is_some_and(|target| target.is_u16() && target.holds_deblocked())
        {
            initializations[plane.index()] = StripeInitialization::FullyOverwritten;
            fill_flat[plane.index()] = false;
        }
    }
    StripePlane::preflight_copy_from_into(
        deblocked_y,
        luma_start,
        luma_end,
        target.as_ref().and_then(|target| target.get(PlaneId::Y)),
        initializations[PlaneId::Y.index()],
    )
    .map_err(CdefError::from)?;
    if let (Some(u), Some(v)) = (deblocked_u, deblocked_v) {
        StripePlane::preflight_copy_from_into(
            u,
            chroma_start,
            chroma_end,
            target.as_ref().and_then(|target| target.get(PlaneId::U)),
            initializations[PlaneId::U.index()],
        )
        .map_err(|error| CdefError::from(error.for_plane(PlaneId::U)))?;
        StripePlane::preflight_copy_from_into(
            v,
            chroma_start,
            chroma_end,
            target.as_ref().and_then(|target| target.get(PlaneId::V)),
            initializations[PlaneId::V.index()],
        )
        .map_err(|error| CdefError::from(error.for_plane(PlaneId::V)))?;
    }
    let filtered_y = StripePlane::copy_from_into_mode(
        deblocked_y,
        luma_start,
        luma_end,
        target.as_mut().and_then(|target| target.take(PlaneId::Y)),
        initializations[PlaneId::Y.index()],
    )
    .map_err(CdefError::from)?;
    let filtered_u = deblocked_u
        .map(|u| {
            StripePlane::copy_from_into_mode(
                u,
                chroma_start,
                chroma_end,
                target.as_mut().and_then(|target| target.take(PlaneId::U)),
                initializations[PlaneId::U.index()],
            )
            .map_err(|error| CdefError::from(error.for_plane(PlaneId::U)))
        })
        .transpose()?;
    let filtered_v = deblocked_v
        .map(|v| {
            StripePlane::copy_from_into_mode(
                v,
                chroma_start,
                chroma_end,
                target.as_mut().and_then(|target| target.take(PlaneId::V)),
                initializations[PlaneId::V.index()],
            )
            .map_err(|error| CdefError::from(error.for_plane(PlaneId::V)))
        })
        .transpose()?;
    let mut frame = CdefFrame {
        deblocked_y,
        deblocked_u,
        deblocked_v,
        filtered_y,
        filtered_u,
        filtered_v,
    };
    if let Some(lookup) = lookup.as_ref() {
        let mut r = luma_start / MI_SIZE;
        let r_end = luma_end.div_ceil(MI_SIZE).min(mi_rows);
        let mut pad = [0u16; CDEF_PADDED_AREA];
        let mut segment = CdefSegmentScratch {
            luma: [0; LUMA_SEGMENT_AREA],
            pair: [0; PAIR_SEGMENT_AREA],
        };
        let whole_y = frame.deblocked_y;
        let whole_u = frame.deblocked_u;
        let whole_v = frame.deblocked_v;
        let skip_grid_fits = lookup
            .skip_grid
            .is_none_or(|grid| (grid.rows, grid.cols) == (mi_rows, mi_cols));
        while r < r_end {
            let row_span = tile_span(lookup.tile_row_starts, r, mi_rows);
            let mut c = 0;
            while c < mi_cols {
                let unit_end = (c / CDEF_UNIT_MI + 1) * CDEF_UNIT_MI;
                let Some(strength_index) = lookup.grid.strength_for_mi(r, c)? else {
                    c = unit_end;
                    continue;
                };
                let params = lookup.strengths.get(strength_index).copied();
                if let Some(params) = params
                    && skip_grid_fits
                    && unit_end <= mi_cols
                    && let Some(values) =
                        flat_segment(lookup, params, (r, c), row_span, whole_y, whole_u, whole_v)
                {
                    fill_flat_segment(&mut frame, lookup, (r, c), values, fill_flat)?;
                    c = unit_end;
                    continue;
                }
                while c < unit_end.min(mi_cols) {
                    if c.is_multiple_of(SEGMENT_MI)
                        && let Some(params) = params
                        && cdef_segment(lookup, params, (r, c), row_span, &mut segment, &mut frame)?
                    {
                        c += SEGMENT_MI;
                        continue;
                    }
                    if let Some(ctx) = lookup.at(r, c, params, row_span)? {
                        compute_cdef_block::<T>(
                            &ctx,
                            &mut pad,
                            whole_y,
                            whole_u,
                            whole_v,
                            &mut frame.filtered_y,
                            frame.filtered_u.as_mut(),
                            frame.filtered_v.as_mut(),
                        )?;
                    }
                    c += STEP4;
                }
            }
            r += STEP4;
        }
    }
    Ok(frame)
}

/// The value each plane the unit's strengths filter holds over the 64x8 luma
/// segment at `(r, c)` and its tap reach, or `None` unless each such plane is
/// interior and flat. § 7.18.3 filters a flat interior block to itself, since
/// `constrain(0)` is 0 for every strength and direction.
#[inline(never)]
fn flat_segment<S: ReconSample>(
    lookup: &CdefBlockLookup<'_>,
    params: CdefFrameParams,
    (r, c): (usize, usize),
    (row_start, row_end): (usize, usize),
    y_plane: FramePlane<'_, S>,
    u_plane: Option<FramePlane<'_, S>>,
    v_plane: Option<FramePlane<'_, S>>,
) -> Option<[Option<u16>; 3]> {
    let (col_start, col_end) = tile_span(lookup.tile_col_starts, c, lookup.mi_cols);
    let start = (col_start * MI_SIZE, row_start * MI_SIZE);
    let end = (col_end * MI_SIZE, row_end * MI_SIZE);
    let origin = (c * MI_SIZE, r * MI_SIZE);
    let mut values = [None; 3];
    if params.y_pri != 0 || params.y_sec != 0 {
        values[0] = Some(flat_window::<S, 64, 8>(y_plane, origin, start, end)?);
    }
    if (params.uv_pri != 0 || params.uv_sec != 0)
        && let (Some(u_plane), Some(v_plane)) = (u_plane, v_plane)
    {
        if (lookup.sub_x, lookup.sub_y) != (1, 1) {
            return None;
        }
        let half = |(x, y): (usize, usize)| (x >> 1, y >> 1);
        let (origin, start, end) = (half(origin), half(start), half(end));
        values[1] = Some(flat_window::<S, 32, 4>(u_plane, origin, start, end)?);
        values[2] = Some(flat_window::<S, 32, 4>(v_plane, origin, start, end)?);
    }
    Some(values)
}

/// The top-left sample of the `w`x`h` region at `(x, y)` with its tap reach,
/// provided that reach lies inside the tile bounds `start..end`, the plane
/// and its deblocked window.
fn reach_origin<S: ReconSample>(
    plane: FramePlane<'_, S>,
    (x, y): (usize, usize),
    (w, h): (usize, usize),
    start: (usize, usize),
    end: (usize, usize),
) -> Option<(usize, usize)> {
    (x >= start.0 + CDEF_TAP_REACH
        && y >= start.1 + CDEF_TAP_REACH
        && y >= plane.origin_y() + CDEF_TAP_REACH
        && x + w + CDEF_TAP_REACH <= end.0.min(plane.width())
        && y + h + CDEF_TAP_REACH <= end.1.min(plane.frame_height()).min(plane.end_y()))
    .then(|| (x - CDEF_TAP_REACH, y - CDEF_TAP_REACH))
}

/// The one value `plane` holds over the `W`x`H` region at `origin` and its tap
/// reach, provided [`reach_origin`] admits that reach.
fn flat_window<S: ReconSample, const W: usize, const H: usize>(
    plane: FramePlane<'_, S>,
    origin: (usize, usize),
    start: (usize, usize),
    end: (usize, usize),
) -> Option<u16> {
    let (left, top) = reach_origin(plane, origin, (W, H), start, end)?;
    let value = plane.row(top)?.get(left)?.to_u16();
    for row in top..top + H + 2 * CDEF_TAP_REACH {
        let samples = plane.row(row)?.get(left..)?.get(..W + 2 * CDEF_TAP_REACH)?;
        let flat = match (S::u16_slice(samples), S::u8_slice(samples)) {
            (Some(samples), _) => lanes_repeat(samples, value),
            (_, Some(samples)) => lanes_repeat(samples, u8::try_from(value).ok()?),
            _ => false,
        };
        if !flat {
            return None;
        }
    }
    Some(value)
}

/// Whether every sample of `samples`, at least 16 long, equals `value`.
fn lanes_repeat<T: SimdElement>(samples: &[T], value: T) -> bool
where
    Simd<T, 16>: SimdPartialEq<Mask = Mask<T::Mask, 16>>,
{
    let value = Simd::splat(value);
    let last = samples.len().saturating_sub(16);
    (0..last).step_by(16).chain([last]).all(|start| {
        samples
            .get(start..start + 16)
            .is_some_and(|lanes| Simd::from_slice(lanes).simd_eq(value).all())
    })
}

/// Writes a flat segment's values into the planes `fill` marks, whose
/// `FullyOverwritten` stripes hold no source samples yet.
fn fill_flat_segment<T>(
    frame: &mut CdefFrame<'_, T>,
    lookup: &CdefBlockLookup<'_>,
    (r, c): (usize, usize),
    values: [Option<u16>; 3],
    fill: [bool; 3],
) -> Result<(), CdefError> {
    let planes = [
        Some(&mut frame.filtered_y),
        frame.filtered_u.as_mut(),
        frame.filtered_v.as_mut(),
    ];
    for (index, plane) in planes.into_iter().enumerate() {
        let (Some(value), true, Some(plane)) = (values[index], fill[index], plane) else {
            continue;
        };
        let (sx, sy) = if index == 0 {
            (0, 0)
        } else {
            (lookup.sub_x, lookup.sub_y)
        };
        let width = CDEF_UNIT_MI * MI_SIZE;
        fill_rect(
            plane,
            (c * MI_SIZE) >> sx,
            (r * MI_SIZE) >> sy,
            width >> sx,
            8 >> sy,
            value,
        )?;
    }
    Ok(())
}

/// Whether each of the `PLANES` interleaved planes of the `ROWS` x `WIDTH`-lane
/// window of `pad`, rows `STRIDE` lanes apart, holds one value. A flat block
/// filters to itself (§ 7.18.3), and on a flat luma block every § 7.18.2
/// direction cost is equal, so the search gives direction 0 and variance 0.
/// Callers check only high-bit-depth blocks: on 8-bit streams the check cost
/// more than it saved.
fn window_flat<
    const STRIDE: usize,
    const ROWS: usize,
    const WIDTH: usize,
    const PLANES: usize,
    const AREA: usize,
>(
    pad: &[u16; AREA],
) -> bool {
    let last = (ROWS - 1) * STRIDE;
    if pad[last + WIDTH - PLANES..last + WIDTH] != pad[..PLANES]
        || pad[last..last + PLANES] != pad[WIDTH - PLANES..WIDTH]
    {
        return false;
    }
    let lanes = if PLANES == 1 {
        Simd::splat(pad[0])
    } else {
        simd_swizzle!(
            Simd::from_array([pad[0], pad[1]]),
            [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]
        )
    };
    let diff = (0..ROWS).fold(Simd::splat(0), |diff, row| {
        diff | (Simd::<u16, 16>::from_slice(&pad[row * STRIDE..row * STRIDE + 16]) ^ lanes)
    });
    let width = Simd::from_array(core::array::from_fn(|lane| u16::from(lane < WIDTH)));
    (diff & (width * Simd::splat(u16::MAX))) == Simd::splat(0)
}

fn fill_rect(
    plane: &mut StripePlane,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    value: u16,
) -> Result<(), CdefError> {
    let rect = PlaneRect::new(x, y, w, h).map_err(|_| CdefError::Geometry)?;
    let (samples, stride) = plane.rect_mut(rect).ok_or(CdefError::Workspace)?;
    for row in samples.chunks_mut(stride) {
        row.get_mut(..w).ok_or(CdefError::Workspace)?.fill(value);
    }
    Ok(())
}

/// One row segment's gathered luma taps and interleaved chroma-pair taps.
struct CdefSegmentScratch {
    luma: [u16; LUMA_SEGMENT_AREA],
    pair: [u16; PAIR_SEGMENT_AREA],
}

/// CDEF over the `CDEF_SEGMENT_BLOCKS` blocks of the row segment at `(r, c)`
/// with one gather per plane. Returns `false`, having written nothing, when a
/// plane's tap reach is not interior, the planes are not `u16`, or chroma is
/// not 4:2:0; the caller then takes the per-block path.
#[inline(never)]
fn cdef_segment<S: ReconSample>(
    lookup: &CdefBlockLookup<'_>,
    params: CdefFrameParams,
    (r, c): (usize, usize),
    row_span: (usize, usize),
    scratch: &mut CdefSegmentScratch,
    frame: &mut CdefFrame<'_, S>,
) -> Result<bool, CdefError> {
    let luma_used = params.y_pri != 0 || params.y_sec != 0 || params.uv_pri != 0;
    let chroma_used = params.uv_pri != 0 || params.uv_sec != 0;
    let y_plane = frame.deblocked_y;
    let Some(y_samples) = S::u16_slice(y_plane.samples()) else {
        return Ok(false);
    };
    let (col_start, col_end) = tile_span(lookup.tile_col_starts, c, lookup.mi_cols);
    let start = (col_start * MI_SIZE, row_span.0 * MI_SIZE);
    let end = (col_end * MI_SIZE, row_span.1 * MI_SIZE);
    let origin = (c * MI_SIZE, r * MI_SIZE);
    let width = 8 * CDEF_SEGMENT_BLOCKS;
    let Some((left, top)) = reach_origin(y_plane, origin, (width, 8), start, end) else {
        return Ok(false);
    };
    let chroma = match (frame.deblocked_u, frame.deblocked_v) {
        (Some(u_plane), Some(v_plane)) if chroma_used => {
            let half = |(x, y): (usize, usize)| (x >> 1, y >> 1);
            let window = reach_origin(
                u_plane,
                half(origin),
                (width / 2, 4),
                half(start),
                half(end),
            );
            let (Some(u), Some(v), (1, 1), true, Some((left, top))) = (
                S::u16_slice(u_plane.samples()),
                S::u16_slice(v_plane.samples()),
                (lookup.sub_x, lookup.sub_y),
                u_plane.stride() == v_plane.stride(),
                window,
            ) else {
                return Ok(false);
            };
            let base = (top - u_plane.origin_y()) * u_plane.stride() + left;
            Some((u, v, base, u_plane.stride()))
        }
        _ => None,
    };
    let luma_ranges = if luma_used {
        Some(gather_luma_segment(
            y_samples,
            y_plane.width(),
            y_plane.stride(),
            &mut scratch.luma,
            (
                left + CDEF_TAP_REACH,
                top + CDEF_TAP_REACH - y_plane.origin_y(),
            ),
        )?)
    } else {
        None
    };
    if let Some((u, v, base, stride)) = chroma {
        gather_pair_segment(u, v, base, stride, &mut scratch.pair)?;
    }
    for block in 0..CDEF_SEGMENT_BLOCKS {
        if let Some(ctx) = lookup.at(r, c + STEP4 * block, Some(params), row_span)? {
            let luma_range = luma_ranges.map(|ranges| ranges[block]);
            compute_cdef_segment_block(&ctx, block, luma_range, scratch, chroma.is_some(), frame)?;
        }
    }
    Ok(true)
}

/// [`compute_cdef_block`] for block `block` of a gathered interior segment,
/// whose luma window holds samples in `luma_range` when that is known.
fn compute_cdef_segment_block<S>(
    ctx: &CdefBlockCtx,
    block: usize,
    luma_range: Option<[u16; 2]>,
    scratch: &CdefSegmentScratch,
    chroma: bool,
    frame: &mut CdefFrame<'_, S>,
) -> Result<(), CdefError> {
    let luma = scratch
        .luma
        .get(8 * block..)
        .and_then(<[u16]>::first_chunk::<CDEF_SEGMENT_BLOCK_AREA>)
        .ok_or(CdefError::Workspace)?;
    let (x0, y0) = (ctx.c * MI_SIZE, ctx.r * MI_SIZE);
    let pri_base = ctx.params.y_pri << ctx.coeff_shift;
    let uv_pri = ctx.params.uv_pri << ctx.coeff_shift;
    let range = luma_range.filter(|_| {
        !ctx.luma_lossless && (ctx.params.y_sec != 0 || pri_base != 0) && ctx.coeff_shift > 0
    });
    let luma_flat = range.is_some_and(|[min, max]| min == max);
    let (y_dir, var) = if (pri_base == 0 && uv_pri == 0) || luma_flat {
        (0, 0)
    } else {
        cdef_direction_segment(luma, ctx.coeff_shift)
    };
    let [y_filter, uv_filter] = block_filters(ctx, y_dir, var);
    if !((y_filter.pri_str == 0 && y_filter.sec_str == 0) || ctx.luma_lossless) {
        if luma_flat {
            fill_rect(&mut frame.filtered_y, x0, y0, 8, 8, luma[0])?;
        } else {
            let rect = PlaneRect::new(x0, y0, 8, 8).map_err(|_| CdefError::Geometry)?;
            let (output, stride) = frame
                .filtered_y
                .rect_mut(rect)
                .ok_or(CdefError::Workspace)?;
            let window_min = range.and_then(|[min, max]| (max - min <= 255).then_some(min));
            if !cdef_filter_block_segment(luma, &y_filter, window_min, output, stride) {
                return Err(CdefError::Workspace);
            }
        }
    }
    if !chroma || (uv_filter.pri_str == 0 && uv_filter.sec_str == 0) || ctx.chroma_lossless {
        return Ok(());
    }
    let pair = scratch
        .pair
        .get(8 * block..)
        .and_then(<[u16]>::first_chunk::<CDEF_PAIR_SEGMENT_BLOCK_AREA>)
        .ok_or(CdefError::Workspace)?;
    let (Some(filtered_u), Some(filtered_v)) =
        (frame.filtered_u.as_mut(), frame.filtered_v.as_mut())
    else {
        return Err(CdefError::Workspace);
    };
    write_chroma_pair::<CDEF_PAIR_SEGMENT_STRIDE, CDEF_PAIR_SEGMENT_BLOCK_AREA>(
        pair,
        ctx.coeff_shift > 0,
        |output| cdef_filter_block_chroma_pair_segment(pair, &uv_filter, output),
        (filtered_u, filtered_v),
        (x0 >> 1, y0 >> 1),
    )
}

/// The § 7.18.1 luma and chroma filters of the block at `ctx`, given its
/// § 7.18.2 direction and variance.
fn block_filters(ctx: &CdefBlockCtx, y_dir: usize, var: i32) -> [CdefBlockFilter; 2] {
    let shift = ctx.coeff_shift;
    let pri_base = ctx.params.y_pri << shift;
    let uv_pri = ctx.params.uv_pri << shift;
    let var_str = (var >> 6).checked_ilog2().unwrap_or(0).min(12) as i32;
    let damping = ctx.params.damping + shift as i32;
    [
        CdefBlockFilter {
            pri_str: if var != 0 {
                (pri_base * (4 + var_str) + 8) >> 4
            } else {
                0
            },
            sec_str: ctx.params.y_sec << shift,
            damping,
            dir: if pri_base == 0 { 0 } else { y_dir },
            coeff_shift: shift,
        },
        CdefBlockFilter {
            pri_str: uv_pri,
            sec_str: ctx.params.uv_sec << shift,
            damping: damping - 1,
            dir: if uv_pri == 0 {
                0
            } else {
                CDEF_UV_DIR[ctx.sub_x][ctx.sub_y][y_dir]
            },
            coeff_shift: shift,
        },
    ]
}

/// Copies the luma segment's tap rows into `pad` and returns the least and
/// greatest sample of each block's 12x12 window; out of line so that the
/// segment loop keeps its values in registers.
#[inline(never)]
fn gather_luma_segment(
    samples: &[u16],
    width: usize,
    stride: usize,
    pad: &mut [u16; LUMA_SEGMENT_AREA],
    (x0, y0): (usize, usize),
) -> Result<[[u16; 2]; CDEF_SEGMENT_BLOCKS], CdefError> {
    const STARTS: [usize; 5] = [0, 8, 16, 24, CDEF_SEGMENT_STRIDE - 8];
    const { assert!(CDEF_SEGMENT_BLOCKS == 4 && CDEF_SEGMENT_STRIDE == 36) };
    let left = x0.checked_sub(CDEF_TAP_REACH).ok_or(CdefError::Workspace)?;
    if left + CDEF_SEGMENT_STRIDE > width {
        return Err(CdefError::Workspace);
    }
    let mut base = y0
        .checked_sub(CDEF_TAP_REACH)
        .and_then(|top| top.checked_mul(stride))
        .and_then(|row| row.checked_add(left))
        .ok_or(CdefError::Workspace)?;
    let mut min = [Simd::<u16, 8>::splat(u16::MAX); 5];
    let mut max = [Simd::<u16, 8>::splat(0); 5];
    for row in 0..12 {
        let src = samples
            .get(base..)
            .and_then(<[u16]>::first_chunk::<CDEF_SEGMENT_STRIDE>)
            .ok_or(CdefError::Workspace)?;
        let dst = pad
            .get_mut(row * CDEF_SEGMENT_STRIDE..)
            .and_then(<[u16]>::first_chunk_mut::<CDEF_SEGMENT_STRIDE>)
            .ok_or(CdefError::Workspace)?;
        for (chunk, start) in STARTS.into_iter().enumerate() {
            let lanes = Simd::<u16, 8>::from_slice(&src[start..start + 8]);
            lanes.copy_to_slice(&mut dst[start..start + 8]);
            min[chunk] = min[chunk].simd_min(lanes);
            max[chunk] = max[chunk].simd_max(lanes);
        }
        base += stride;
    }
    let tail = |chunks: &[Simd<u16, 8>; 5], block: usize| {
        if block + 1 < CDEF_SEGMENT_BLOCKS {
            simd_swizzle!(chunks[block + 1], [0, 1, 2, 3, 0, 1, 2, 3])
        } else {
            chunks[4]
        }
    };
    Ok(core::array::from_fn(|block| {
        [
            min[block].simd_min(tail(&min, block)).reduce_min(),
            max[block].simd_max(tail(&max, block)).reduce_max(),
        ]
    }))
}

/// Interleaves the chroma segment's tap rows, half a `CDEF_PAIR_SEGMENT_STRIDE`
/// of each plane from `base` on, into `pad` at `CDEF_PAIR_SEGMENT_STRIDE`
/// lanes per row.
#[inline(never)]
fn gather_pair_segment(
    u_samples: &[u16],
    v_samples: &[u16],
    mut base: usize,
    stride: usize,
    pad: &mut [u16; PAIR_SEGMENT_AREA],
) -> Result<(), CdefError> {
    const SPAN: usize = CDEF_PAIR_SEGMENT_STRIDE / 2;
    for row in 0..CHROMA_PAIR_SIDE + 2 * CDEF_TAP_REACH {
        let u_row = u_samples
            .get(base..)
            .and_then(<[u16]>::first_chunk::<SPAN>)
            .ok_or(CdefError::Workspace)?;
        let v_row = v_samples
            .get(base..)
            .and_then(<[u16]>::first_chunk::<SPAN>)
            .ok_or(CdefError::Workspace)?;
        let lanes = pad
            .get_mut(row * CDEF_PAIR_SEGMENT_STRIDE..)
            .and_then(<[u16]>::first_chunk_mut::<CDEF_PAIR_SEGMENT_STRIDE>)
            .ok_or(CdefError::Workspace)?;
        for start in [0, 8, SPAN - 8] {
            let (low, high) = Simd::<u16, 8>::from_slice(&u_row[start..start + 8])
                .interleave(Simd::from_slice(&v_row[start..start + 8]));
            low.copy_to_slice(&mut lanes[2 * start..2 * start + 8]);
            high.copy_to_slice(&mut lanes[2 * start + 8..2 * start + 16]);
        }
        base += stride;
    }
    Ok(())
}

struct CdefBlockCtx {
    r: usize,
    c: usize,
    mi_row_start: usize,
    mi_col_start: usize,
    params: CdefFrameParams,
    coeff_shift: u32,
    max_sample: i32,
    mi_rows: usize,
    mi_cols: usize,
    sub_x: usize,
    sub_y: usize,
    luma_lossless: bool,
    chroma_lossless: bool,
}

#[allow(clippy::too_many_arguments)]
fn compute_cdef_block<S: ReconSample>(
    ctx: &CdefBlockCtx,
    pad: &mut [u16; CDEF_PADDED_AREA],
    luma_snap: FramePlane<'_, S>,
    u_snap: Option<FramePlane<'_, S>>,
    v_snap: Option<FramePlane<'_, S>>,
    filtered_y: &mut StripePlane,
    filtered_u: Option<&mut StripePlane>,
    filtered_v: Option<&mut StripePlane>,
) -> Result<(), CdefError> {
    let x0 = ctx.c << MI_SIZE_LOG2;
    let y0 = ctx.r << MI_SIZE_LOG2;
    let block_w = 8.min(luma_snap.width().saturating_sub(x0));
    let block_h = 8.min(luma_snap.frame_height().saturating_sub(y0));
    if block_w == 0 || block_h == 0 {
        return Ok(());
    }
    let pri_base = ctx.params.y_pri << ctx.coeff_shift;
    let sec_str = ctx.params.y_sec << ctx.coeff_shift;
    let uv_pri = ctx.params.uv_pri << ctx.coeff_shift;
    let uv_sec = ctx.params.uv_sec << ctx.coeff_shift;

    let luma_inside_x = (ctx.mi_cols * MI_SIZE).min(luma_snap.width());
    let luma_inside_y = (ctx.mi_rows * MI_SIZE).min(luma_snap.frame_height());
    let luma_start_x = ctx.mi_col_start * MI_SIZE;
    let luma_start_y = ctx.mi_row_start * MI_SIZE;
    let luma_interior = x0 >= luma_start_x + CDEF_TAP_REACH
        && y0 >= luma_start_y + CDEF_TAP_REACH
        && x0 + block_w - 1 + CDEF_TAP_REACH < luma_inside_x
        && y0 + block_h - 1 + CDEF_TAP_REACH < luma_inside_y;
    let luma_pad_ready = luma_interior && !ctx.luma_lossless && (sec_str != 0 || pri_base != 0);
    let mut luma_flat = false;
    if luma_pad_ready {
        gather_interior_pad(luma_snap, pad, x0, y0, block_w, block_h)?;
        luma_flat = S::MAX_VALUE > u16::from(u8::MAX)
            && ctx.coeff_shift > 0
            && window_flat::<CDEF_PADDED_SIDE, 12, 12, 1, CDEF_PADDED_AREA>(pad);
    }

    let (y_dir, var) = if (pri_base == 0 && uv_pri == 0) || luma_flat {
        (0, 0)
    } else if luma_pad_ready {
        cdef_direction_padded(pad, ctx.coeff_shift)
    } else {
        for i in 0..8 {
            let src = luma_snap
                .row(y0 + i.min(block_h - 1))
                .and_then(|row| row.get(x0..x0 + block_w))
                .ok_or(CdefError::Geometry)?;
            let start = (i + 2) * CDEF_PADDED_SIDE + 2;
            let dst = pad.get_mut(start..start + 8).ok_or(CdefError::Workspace)?;
            for (j, cell) in dst.iter_mut().enumerate() {
                *cell = src[j.min(block_w - 1)].to_u16();
            }
        }
        cdef_direction_padded(pad, ctx.coeff_shift)
    };
    let [y_filter, uv_filter] = block_filters(ctx, y_dir, var);
    let y_zero = y_filter.pri_str == 0 && sec_str == 0;
    let uv_zero = uv_pri == 0 && uv_sec == 0;
    if !(y_zero || ctx.luma_lossless) {
        if luma_flat {
            fill_rect(filtered_y, x0, y0, block_w, block_h, pad[0])?;
        } else if luma_pad_ready {
            filter_pad_into(filtered_y, pad, x0, y0, block_w, block_h, &y_filter, false)?;
        } else {
            compute_cdef_filter_plane::<S>(luma_snap, ctx, false, &y_filter, pad, filtered_y)?;
        }
    }
    if uv_zero || ctx.chroma_lossless {
        return Ok(());
    }
    match (u_snap, v_snap, filtered_u, filtered_v) {
        (None, None, _, _) => Ok(()),
        (Some(u_snap), Some(v_snap), Some(filtered_u), Some(filtered_v)) => {
            if compute_cdef_chroma_pair::<S>(
                u_snap, v_snap, ctx, &uv_filter, pad, filtered_u, filtered_v,
            )? {
                return Ok(());
            }
            compute_cdef_filter_plane::<S>(u_snap, ctx, true, &uv_filter, pad, filtered_u)?;
            compute_cdef_filter_plane::<S>(v_snap, ctx, true, &uv_filter, pad, filtered_v)
        }
        _ => Err(CdefError::Workspace),
    }
}

/// AV2 § 7.18.3 CDEF over one interior 4x4 chroma block of both planes at once.
///
/// The two chroma planes share the block's geometry and every filter parameter,
/// so one interleaved 16-lane pass replaces two 8-lane passes plus a second
/// geometry derivation and a second tap gather. Returns `false` when the block
/// is not the interior `4x4` case the interleaved scratch covers, which leaves
/// the caller on the per-plane path.
fn compute_cdef_chroma_pair<S: ReconSample>(
    u_snap: FramePlane<'_, S>,
    v_snap: FramePlane<'_, S>,
    ctx: &CdefBlockCtx,
    filter: &CdefBlockFilter,
    pad: &mut [u16; CDEF_PADDED_AREA],
    filtered_u: &mut StripePlane,
    filtered_v: &mut StripePlane,
) -> Result<bool, CdefError> {
    let x0 = (ctx.c * MI_SIZE) >> ctx.sub_x;
    let y0 = (ctx.r * MI_SIZE) >> ctx.sub_y;
    let (w, h) = ((8 >> ctx.sub_x), (8 >> ctx.sub_y));
    if (w, h) != (CHROMA_PAIR_SIDE, CHROMA_PAIR_SIDE) {
        return Ok(false);
    }
    let inside_x = ((ctx.mi_cols * MI_SIZE) >> ctx.sub_x).min(u_snap.width());
    let inside_y = ((ctx.mi_rows * MI_SIZE) >> ctx.sub_y).min(u_snap.frame_height());
    let start_x = (ctx.mi_col_start * MI_SIZE) >> ctx.sub_x;
    let start_y = (ctx.mi_row_start * MI_SIZE) >> ctx.sub_y;
    if !(x0 >= start_x + CDEF_TAP_REACH
        && y0 >= start_y + CDEF_TAP_REACH
        && x0 + w - 1 + CDEF_TAP_REACH < inside_x
        && y0 + h - 1 + CDEF_TAP_REACH < inside_y
        && y0 >= u_snap.origin_y() + CDEF_TAP_REACH
        && y0 + h + CDEF_TAP_REACH <= u_snap.end_y())
    {
        return Ok(false);
    }
    let span = w + 2 * CDEF_TAP_REACH;
    let left = x0 - CDEF_TAP_REACH;
    if left + span > u_snap.width() || u_snap.stride() != v_snap.stride() {
        return Ok(false);
    }
    let base = (y0 - u_snap.origin_y() - CDEF_TAP_REACH) * u_snap.stride() + left;
    let (u_samples, v_samples) = (u_snap.samples(), v_snap.samples());
    if let (Some(u), Some(v)) = (S::u16_slice(u_samples), S::u16_slice(v_samples)) {
        gather_chroma_pair(u, v, base, u_snap.stride(), pad)?;
    } else if let (Some(u), Some(v)) = (S::u8_slice(u_samples), S::u8_slice(v_samples)) {
        gather_chroma_pair(u, v, base, u_snap.stride(), pad)?;
    } else {
        return Ok(false);
    }
    write_chroma_pair::<CDEF_PAIR_STRIDE, CDEF_PADDED_AREA>(
        pad,
        S::MAX_VALUE > u16::from(u8::MAX) && filter.coeff_shift > 0,
        |output| cdef_filter_block_chroma_pair(pad, h, filter, output),
        (filtered_u, filtered_v),
        (x0, y0),
    )?;
    Ok(true)
}

/// Writes one interior chroma pair from its gathered interleaved taps in
/// `pad`: the flat values when `check_flat` finds both planes flat, else the
/// output of `filter_pair`.
#[allow(
    clippy::inline_always,
    reason = "measured: out of line it slowed 8-bit CDEF"
)]
#[inline(always)]
fn write_chroma_pair<const STRIDE: usize, const AREA: usize>(
    pad: &[u16; AREA],
    check_flat: bool,
    filter_pair: impl FnOnce(&mut [u16; CDEF_PAIR_OUTPUT]) -> bool,
    (filtered_u, filtered_v): (&mut StripePlane, &mut StripePlane),
    (x0, y0): (usize, usize),
) -> Result<(), CdefError> {
    if check_flat && window_flat::<STRIDE, 8, 16, 2, AREA>(pad) {
        fill_rect(
            filtered_u,
            x0,
            y0,
            CHROMA_PAIR_SIDE,
            CHROMA_PAIR_SIDE,
            pad[0],
        )?;
        return fill_rect(
            filtered_v,
            x0,
            y0,
            CHROMA_PAIR_SIDE,
            CHROMA_PAIR_SIDE,
            pad[1],
        );
    }
    let mut output = [0u16; CDEF_PAIR_OUTPUT];
    if !filter_pair(&mut output) {
        return Err(CdefError::Workspace);
    }
    let (u_out, u_stride) = stripe_rows_from(filtered_u, x0, y0).ok_or(CdefError::Workspace)?;
    let (v_out, v_stride) = stripe_rows_from(filtered_v, x0, y0).ok_or(CdefError::Workspace)?;
    for (row, lanes) in output.chunks_exact(2 * CHROMA_PAIR_SIDE).enumerate() {
        let planes = simd_swizzle!(
            Simd::<u16, CHROMA_PAIR_SPAN>::from_slice(lanes),
            [0, 2, 4, 6, 1, 3, 5, 7]
        );
        let (u_lanes, v_lanes) = planes.as_array().split_at(CHROMA_PAIR_SIDE);
        u_out
            .get_mut(row * u_stride..)
            .and_then(|row| row.get_mut(..CHROMA_PAIR_SIDE))
            .ok_or(CdefError::Workspace)?
            .copy_from_slice(u_lanes); // splot-copy-ok: publish the pair's U samples
        v_out
            .get_mut(row * v_stride..)
            .and_then(|row| row.get_mut(..CHROMA_PAIR_SIDE))
            .ok_or(CdefError::Workspace)?
            .copy_from_slice(v_lanes); // splot-copy-ok: publish the pair's V samples
    }
    Ok(())
}

/// `plane`'s samples from `(x, y)` on, and its row stride, provided rows
/// keep `CHROMA_PAIR_SIDE` samples from `x` on.
fn stripe_rows_from(plane: &mut StripePlane, x: usize, y: usize) -> Option<(&mut [u16], usize)> {
    let stride = plane.width();
    if x.checked_add(CHROMA_PAIR_SIDE)? > stride {
        return None;
    }
    let start = y
        .checked_sub(plane.origin_y())?
        .checked_mul(stride)?
        .checked_add(x)?;
    Some((plane.samples_mut().get_mut(start..)?, stride))
}

/// Interleaves the chroma pair's tap rows, `CHROMA_PAIR_SPAN` samples of each
/// plane from `base` on, into `pad` at `CDEF_PAIR_STRIDE` lanes per row.
///
/// Kept out of line: inlined into the stripe loop, it reloaded its plane
/// bases, lengths and stride from the stack for every row.
#[inline(never)]
fn gather_chroma_pair<T: Copy>(
    u_samples: &[T],
    v_samples: &[T],
    mut base: usize,
    stride: usize,
    pad: &mut [u16; CDEF_PADDED_AREA],
) -> Result<(), CdefError>
where
    u16: From<T>,
{
    for row in 0..CHROMA_PAIR_SIDE + 2 * CDEF_TAP_REACH {
        let u_row = u_samples
            .get(base..base + CHROMA_PAIR_SPAN)
            .ok_or(CdefError::Workspace)?;
        let v_row = v_samples
            .get(base..base + CHROMA_PAIR_SPAN)
            .ok_or(CdefError::Workspace)?;
        let widen = |row: &[T]| {
            let mut lanes = [0u16; CHROMA_PAIR_SPAN];
            for (lane, &sample) in lanes.iter_mut().zip(row) {
                *lane = u16::from(sample);
            }
            Simd::from_array(lanes)
        };
        let (low, high) = widen(u_row).interleave(widen(v_row));
        let lanes = pad
            .get_mut(row * CDEF_PAIR_STRIDE..)
            .and_then(<[u16]>::first_chunk_mut::<{ 2 * CHROMA_PAIR_SPAN }>)
            .ok_or(CdefError::Workspace)?;
        lanes[..CHROMA_PAIR_SPAN].copy_from_slice(low.as_array()); // splot-copy-ok: interleave the chroma pair's taps
        lanes[CHROMA_PAIR_SPAN..].copy_from_slice(high.as_array()); // splot-copy-ok: interleave the chroma pair's taps
        base += stride;
    }
    Ok(())
}

fn compute_cdef_filter_plane<S: ReconSample>(
    snap: FramePlane<'_, S>,
    ctx: &CdefBlockCtx,
    chroma: bool,
    filter: &CdefBlockFilter,
    pad: &mut [u16; CDEF_PADDED_AREA],
    filtered: &mut StripePlane,
) -> Result<(), CdefError> {
    let (sub_x, sub_y) = if chroma {
        (ctx.sub_x, ctx.sub_y)
    } else {
        (0, 0)
    };
    let x0 = (ctx.c * MI_SIZE) >> sub_x;
    let y0 = (ctx.r * MI_SIZE) >> sub_y;
    let w = (8 >> sub_x).min(snap.width().saturating_sub(x0));
    let h = (8 >> sub_y).min(snap.frame_height().saturating_sub(y0));
    if w == 0 || h == 0 {
        return Ok(());
    }

    let inside_x = ((ctx.mi_cols * MI_SIZE) >> sub_x).min(snap.width());
    let inside_y = ((ctx.mi_rows * MI_SIZE) >> sub_y).min(snap.frame_height());
    let start_x = (ctx.mi_col_start * MI_SIZE) >> sub_x;
    let start_y = (ctx.mi_row_start * MI_SIZE) >> sub_y;
    let interior = x0 >= start_x + CDEF_TAP_REACH
        && y0 >= start_y + CDEF_TAP_REACH
        && x0 + w - 1 + CDEF_TAP_REACH < inside_x
        && y0 + h - 1 + CDEF_TAP_REACH < inside_y;

    if interior {
        gather_interior_pad(snap, pad, x0, y0, w, h)?;
        return filter_pad_into(filtered, pad, x0, y0, w, h, filter, false);
    }

    if matches!(w, 4 | 8) {
        gather_boundary_pad(
            snap, pad, x0, y0, w, h, start_x, start_y, inside_x, inside_y,
        )?;
        return filter_pad_into(filtered, pad, x0, y0, w, h, filter, true);
    }
    let mut filtered_block = [0u16; 64];
    let offsets = CdefTapOffsets::for_direction(filter.dir);
    for i in 0..h {
        for j in 0..w {
            let center = snap
                .get((x0 + j) as isize, (y0 + i) as isize)
                .ok_or(CdefError::Geometry)?;
            let taps = gather_taps(
                snap,
                &offsets,
                x0 + j,
                y0 + i,
                start_x,
                start_y,
                inside_x,
                inside_y,
                center,
            );
            let filtered = cdef_filter_sample(
                &taps,
                filter.pri_str,
                filter.sec_str,
                filter.damping,
                filter.coeff_shift,
            );
            filtered_block[i * w + j] = storage_sample(filtered, ctx.max_sample)?;
        }
    }
    let rect = PlaneRect::new(x0, y0, w, h).map_err(|_| CdefError::Geometry)?;
    filtered
        .write_rect(rect, &filtered_block, w)
        .ok_or(CdefError::Workspace)
}

/// Gathers the `w`x`h` block plus a two-sample border from `snap` into `pad`, in the
/// `CDEF_PADDED_SIDE`-wide layout [`cdef_filter_block_interior`] indexes. The caller
/// guarantees the bordered region is inside the filter region (interior block), so the
/// gather covers exactly the samples the kernel reads and `pad` needs no re-zeroing.
fn gather_interior_pad<S: ReconSample>(
    snap: FramePlane<'_, S>,
    pad: &mut [u16; CDEF_PADDED_AREA],
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
) -> Result<(), CdefError> {
    let inside = y0 >= snap.origin_y() + CDEF_TAP_REACH && y0 + h + CDEF_TAP_REACH <= snap.end_y();
    let window_y = inside.then(|| y0 - snap.origin_y());
    let (width, stride) = (snap.width(), snap.stride());
    let windowed = window_y.and_then(|y0| {
        let samples = snap.samples();
        match (S::u16_slice(samples), S::u8_slice(samples)) {
            (Some(samples), _) => gather_interior_window(samples, width, stride, pad, x0, y0, w, h),
            (_, Some(samples)) => gather_interior_window(samples, width, stride, pad, x0, y0, w, h),
            _ => None,
        }
    });
    if let Some(gathered) = windowed {
        return gathered;
    }
    for r in 0..h + 2 * CDEF_TAP_REACH {
        let src = snap
            .row(y0 - CDEF_TAP_REACH + r)
            .and_then(|row| row.get(x0 - CDEF_TAP_REACH..x0 + w + CDEF_TAP_REACH))
            .ok_or(CdefError::Workspace)?;
        let dst_start = r * CDEF_PADDED_SIDE;
        let dst = pad
            .get_mut(dst_start..dst_start + src.len())
            .ok_or(CdefError::Workspace)?;
        if let Some(src) = S::u16_slice(src) {
            dst.copy_from_slice(src);
        } else {
            for (dst, src) in dst.iter_mut().zip(src) {
                *dst = src.to_u16();
            }
        }
    }
    Ok(())
}

/// [`gather_interior_rows`] for the block sizes CDEF filters, or `None` for
/// any other size.
#[allow(clippy::too_many_arguments)]
fn gather_interior_window<T: Copy>(
    samples: &[T],
    width: usize,
    stride: usize,
    pad: &mut [u16; CDEF_PADDED_AREA],
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
) -> Option<Result<(), CdefError>>
where
    u16: From<T>,
{
    Some(match (w, h) {
        (8, 8) => gather_interior_rows::<12, 12, CDEF_PADDED_SIDE, CDEF_PADDED_AREA, _>(
            samples, width, stride, pad, x0, y0,
        ),
        (4, 4) => gather_interior_rows::<8, 8, CDEF_PADDED_SIDE, CDEF_PADDED_AREA, _>(
            samples, width, stride, pad, x0, y0,
        ),
        (4, 8) => gather_interior_rows::<8, 12, CDEF_PADDED_SIDE, CDEF_PADDED_AREA, _>(
            samples, width, stride, pad, x0, y0,
        ),
        _ => return None,
    })
}

/// [`gather_interior_pad`] for `u16` or `u8` plane storage, widening `SPAN`
/// samples per row of `ROWS` rows off one hoisted row base.
fn gather_interior_rows<
    const SPAN: usize,
    const ROWS: usize,
    const DST_STRIDE: usize,
    const AREA: usize,
    T: Copy,
>(
    samples: &[T],
    width: usize,
    stride: usize,
    pad: &mut [u16; AREA],
    x0: usize,
    y0: usize,
) -> Result<(), CdefError>
where
    u16: From<T>,
{
    let left = x0.checked_sub(CDEF_TAP_REACH).ok_or(CdefError::Workspace)?;
    if left + SPAN > width {
        return Err(CdefError::Workspace);
    }
    let mut base = y0
        .checked_sub(CDEF_TAP_REACH)
        .and_then(|top| top.checked_mul(stride))
        .and_then(|row| row.checked_add(left))
        .ok_or(CdefError::Workspace)?;
    for r in 0..ROWS {
        let src = samples
            .get(base..)
            .and_then(<[T]>::first_chunk::<SPAN>)
            .ok_or(CdefError::Workspace)?;
        let dst = pad
            .get_mut(r * DST_STRIDE..)
            .and_then(<[u16]>::first_chunk_mut::<SPAN>)
            .ok_or(CdefError::Workspace)?;
        for (dst, &src) in dst.iter_mut().zip(src) {
            *dst = u16::from(src);
        }
        base += stride;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn gather_boundary_pad<S: ReconSample>(
    snap: FramePlane<'_, S>,
    pad: &mut [u16; CDEF_PADDED_AREA],
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    start_x: usize,
    start_y: usize,
    inside_x: usize,
    inside_y: usize,
) -> Result<(), CdefError> {
    pad[..CDEF_PADDED_SIDE * CDEF_PADDED_SIDE].fill(CDEF_UNAVAILABLE);
    let source_x_start = x0.saturating_sub(CDEF_TAP_REACH).max(start_x);
    let source_x_end = x0
        .saturating_add(w)
        .saturating_add(CDEF_TAP_REACH)
        .min(inside_x);
    let destination_x = (source_x_start as isize - x0 as isize + CDEF_TAP_REACH as isize) as usize;
    for pad_row in 0..h + 2 * CDEF_TAP_REACH {
        let source_y = y0 as isize + pad_row as isize - CDEF_TAP_REACH as isize;
        if !(start_y as isize..inside_y as isize).contains(&source_y) {
            continue;
        }
        let source = snap
            .row(source_y as usize)
            .and_then(|row| row.get(source_x_start..source_x_end))
            .ok_or(CdefError::Workspace)?;
        let start = pad_row * CDEF_PADDED_SIDE + destination_x;
        let destination = pad
            .get_mut(start..start + source.len())
            .ok_or(CdefError::Workspace)?;
        for (destination, source) in destination.iter_mut().zip(source) {
            *destination = source.to_u16();
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn filter_pad_into(
    filtered: &mut StripePlane,
    pad: &[u16; CDEF_PADDED_AREA],
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    has_unavailable: bool,
) -> Result<(), CdefError> {
    let rect = PlaneRect::new(x0, y0, w, h).map_err(|_| CdefError::Geometry)?;
    let (output, stride) = filtered.rect_mut(rect).ok_or(CdefError::Workspace)?;
    let filtered = if has_unavailable {
        cdef_filter_block_boundary_to_valid_stride(pad, w, h, filter, output, stride)
    } else {
        cdef_filter_block_interior_to_valid_stride(pad, w, h, filter, output, stride)
    };
    if filtered {
        Ok(())
    } else {
        Err(CdefError::Workspace)
    }
}

fn storage_sample(filtered: i32, max_sample: i32) -> Result<u16, CdefError> {
    let clipped = filtered.clamp(0, max_sample);
    u16::try_from(clipped).map_err(|_| CdefError::Geometry)
}

const CDEF_TAP_REACH: usize = 2;

struct CdefTapOffsets {
    primary: [[(isize, isize); 2]; 2],
    secondary: [[[(isize, isize); 2]; 2]; 2],
}

impl CdefTapOffsets {
    fn for_direction(dir: usize) -> Self {
        let offset = |dir: usize, k: usize, sign: isize| -> (isize, isize) {
            (
                sign * CDEF_DIRECTIONS[dir & 7][k][0] as isize,
                sign * CDEF_DIRECTIONS[dir & 7][k][1] as isize,
            )
        };
        let mut primary = [[(0isize, 0isize); 2]; 2];
        let mut secondary = [[[(0isize, 0isize); 2]; 2]; 2];
        for k in 0..2 {
            for (sign_index, sign) in [-1isize, 1].into_iter().enumerate() {
                primary[k][sign_index] = offset(dir, k, sign);
                for (dir_off_index, dir_off) in [6usize, 2].into_iter().enumerate() {
                    secondary[k][sign_index][dir_off_index] = offset(dir + dir_off, k, sign);
                }
            }
        }
        Self { primary, secondary }
    }
}

#[allow(clippy::too_many_arguments)]
fn gather_taps<T: ReconSample>(
    snap: FramePlane<'_, T>,
    offsets: &CdefTapOffsets,
    x: usize,
    y: usize,
    start_x: usize,
    start_y: usize,
    inside_x: usize,
    inside_y: usize,
    center: i32,
) -> CdefSampleTaps {
    let fetch = |(dy, dx): (isize, isize)| -> CdefTap {
        let y = y as isize + dy;
        let x = x as isize + dx;
        if x >= start_x as isize
            && y >= start_y as isize
            && (x as usize) < inside_x
            && (y as usize) < inside_y
        {
            match snap.row(y as usize).and_then(|row| row.get(x as usize)) {
                Some(value) => CdefTap {
                    value: i32::from(value.to_u16()),
                    available: true,
                },
                None => UNAVAILABLE_TAP,
            }
        } else {
            UNAVAILABLE_TAP
        }
    };

    let mut primary = [[UNAVAILABLE_TAP; 2]; 2];
    let mut secondary = [[[UNAVAILABLE_TAP; 2]; 2]; 2];
    for k in 0..2 {
        for sign_index in 0..2 {
            primary[k][sign_index] = fetch(offsets.primary[k][sign_index]);
            for (dir_off_index, tap) in secondary[k][sign_index].iter_mut().enumerate() {
                *tap = fetch(offsets.secondary[k][sign_index][dir_off_index]);
            }
        }
    }

    CdefSampleTaps {
        center,
        primary,
        secondary,
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CdefError {
    #[error("CDEF geometry computation went out of range")]
    Geometry,
    #[error("CDEF workspace sample access went out of bounds")]
    Workspace,
    #[error("CDEF stripe output storage could not be reserved")]
    Allocation(splot_recon::PlaneId),
}

impl From<crate::filters::source::StripeCopyError> for CdefError {
    fn from(error: crate::filters::source::StripeCopyError) -> Self {
        match error {
            crate::filters::source::StripeCopyError::Allocation(plane) => Self::Allocation(plane),
            crate::filters::source::StripeCopyError::Geometry => Self::Geometry,
        }
    }
}

#[cfg(test)]
#[path = "cdef_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cdef_direct_tests.rs"]
mod direct_tests;
