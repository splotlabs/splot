// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_recon::{
    BitDepth, CDEF_DIRECTIONS, CDEF_PADDED_AREA, CDEF_PADDED_SIDE, CDEF_UV_DIR, CdefBlockFilter,
    CdefSampleTaps, CdefTap, CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample,
    cdef_direction, cdef_direction_padded, cdef_filter_block_interior_to_valid_stride,
    cdef_filter_sample,
};

use super::source::{FramePlane, StripePlane};

const MI_SIZE: usize = 4;
const MI_SIZE_LOG2: u32 = 2;
const CDEF_UNIT_MI: usize = 16;
const STEP4: usize = 2;

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

pub(crate) fn cdef_frame_strengths(core: &FrameHeaderCore) -> Option<Vec<CdefFrameParams>> {
    let cdef = core.cdef_params.as_ref()?;
    if !cdef.cdef_frame_enable {
        return None;
    }
    let damping = i32::from(cdef.cdef_damping?);
    let mut strengths = Vec::with_capacity(cdef.strengths.len());
    for set in &cdef.strengths {
        strengths.push(CdefFrameParams {
            y_pri: i32::from(set.y_pri_strength),
            y_sec: i32::from(set.y_sec_strength),
            uv_pri: i32::from(set.uv_pri_strength),
            uv_sec: i32::from(set.uv_sec_strength),
            damping,
        });
    }
    Some(strengths)
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
}

impl CdefSkipGrid {
    pub(crate) fn new(rows: usize, cols: usize, values: Vec<bool>) -> Result<Self, CdefError> {
        validate_grid_len(rows, cols, values.len())?;
        Ok(Self { rows, cols, values })
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

impl<'a, T: ReconSample> CdefFrame<'a, T> {
    pub(crate) fn deblocked(&self, plane: PlaneId) -> Option<FramePlane<'a, T>> {
        match plane {
            PlaneId::Y => Some(self.deblocked_y),
            PlaneId::U => self.deblocked_u,
            PlaneId::V => self.deblocked_v,
        }
    }
}

struct CdefBlockLookup<'a> {
    strengths: &'a [CdefFrameParams],
    grid: &'a CdefUnitGrid,
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

impl CdefBlockLookup<'_> {
    fn at(&self, r: usize, c: usize) -> Result<Option<CdefBlockCtx>, CdefError> {
        let Some(strength_index) = self.grid.strength_for_mi(r, c)? else {
            return Ok(None);
        };
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
        let params = *self
            .strengths
            .get(strength_index)
            .ok_or(CdefError::Geometry)?;
        Ok(Some(CdefBlockCtx {
            r,
            c,
            params,
            coeff_shift: self.coeff_shift,
            max_sample: self.max_sample,
            mi_rows: self.mi_rows,
            mi_cols: self.mi_cols,
            sub_x: self.sub_x,
            sub_y: self.sub_y,
            luma_lossless,
            chroma_lossless,
        }))
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cdef_stripe<'a, T: ReconSample>(
    workspace: &'a CurrentFrameWorkspace<T>,
    strengths: Option<&[CdefFrameParams]>,
    grid: Option<&CdefUnitGrid>,
    skip_grid: Option<&CdefSkipGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    mi_size: (usize, usize),
    bit_depth: BitDepth,
    luma_start: usize,
    luma_end: usize,
) -> Result<CdefFrame<'a, T>, CdefError> {
    let (mi_rows, mi_cols) = mi_size;
    if luma_start >= luma_end || !luma_start.is_multiple_of(STEP4 * MI_SIZE) {
        return Err(CdefError::Geometry);
    }
    let coeff_shift = u32::from(bit_depth.bits()) - 8;
    let max_sample = i32::from(bit_depth.max_sample());
    let pixel_format = workspace.info().pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let has_chroma = !pixel_format.is_monochrome();

    let deblocked_y = FramePlane::new(workspace, PlaneId::Y).ok_or(CdefError::Workspace)?;
    let filtered_y =
        StripePlane::copy_from(deblocked_y, luma_start, luma_end).ok_or(CdefError::Geometry)?;
    let chroma_start = luma_start >> sub_y;
    let chroma_end = luma_end.div_ceil(1usize << sub_y);
    let (deblocked_u, deblocked_v, filtered_u, filtered_v) = if has_chroma {
        let u = FramePlane::new(workspace, PlaneId::U).ok_or(CdefError::Workspace)?;
        let v = FramePlane::new(workspace, PlaneId::V).ok_or(CdefError::Workspace)?;
        let filtered_u =
            StripePlane::copy_from(u, chroma_start, chroma_end).ok_or(CdefError::Geometry)?;
        let filtered_v =
            StripePlane::copy_from(v, chroma_start, chroma_end).ok_or(CdefError::Geometry)?;
        (Some(u), Some(v), Some(filtered_u), Some(filtered_v))
    } else {
        (None, None, None, None)
    };
    let mut frame = CdefFrame {
        deblocked_y,
        deblocked_u,
        deblocked_v,
        filtered_y,
        filtered_u,
        filtered_v,
    };
    if let (Some(strengths), Some(grid)) = (strengths, grid) {
        let lookup = CdefBlockLookup {
            strengths,
            grid,
            skip_grid,
            lossless_grid,
            mi_rows,
            mi_cols,
            sub_x,
            sub_y,
            has_chroma,
            coeff_shift,
            max_sample,
        };

        let mut r = luma_start / MI_SIZE;
        let r_end = luma_end.div_ceil(MI_SIZE).min(mi_rows);
        // Each interior block's gather overwrites exactly the padded region the
        // kernel reads, so one stripe-scoped scratch is reused without re-zeroing.
        let mut pad = [0u16; CDEF_PADDED_AREA];
        while r < r_end {
            let mut c = 0;
            while c < mi_cols {
                if let Some(ctx) = lookup.at(r, c)? {
                    compute_cdef_block::<T>(
                        &ctx,
                        &mut pad,
                        frame.deblocked_y,
                        frame.deblocked_u,
                        frame.deblocked_v,
                        &mut frame.filtered_y,
                        frame.filtered_u.as_mut(),
                        frame.filtered_v.as_mut(),
                    )?;
                }
                c += STEP4;
            }
            r += STEP4;
        }
    }
    Ok(frame)
}

struct CdefBlockCtx {
    r: usize,
    c: usize,
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

impl CdefBlockCtx {
    fn filter_ctx(
        &self,
        pri_str: i32,
        sec_str: i32,
        damping: i32,
        dir: usize,
        sub: usize,
    ) -> CdefFilterCtx {
        CdefFilterCtx {
            r: self.r,
            c: self.c,
            pri_str,
            sec_str,
            damping,
            dir,
            sub,
            coeff_shift: self.coeff_shift,
            max_sample: self.max_sample,
            mi_rows: self.mi_rows,
            mi_cols: self.mi_cols,
            frame_sub_x: self.sub_x,
            frame_sub_y: self.sub_y,
        }
    }
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
    let luma_interior = x0 >= CDEF_TAP_REACH
        && y0 >= CDEF_TAP_REACH
        && x0 + block_w - 1 + CDEF_TAP_REACH < luma_inside_x
        && y0 + block_h - 1 + CDEF_TAP_REACH < luma_inside_y;
    // Gather the luma pad up front when the luma plane will be filtered: that same
    // padded neighbourhood feeds both the direction search and the kernel, removing
    // the separate direction-only read of the 8x8 block from the source plane.
    let luma_pad_ready = luma_interior && !ctx.luma_lossless && (sec_str != 0 || pri_base != 0);
    if luma_pad_ready {
        gather_interior_pad(luma_snap, pad, x0, y0, block_w, block_h)?;
    }

    let (y_dir, var) = if pri_base == 0 && uv_pri == 0 {
        (0, 0)
    } else if luma_pad_ready {
        cdef_direction_padded(pad, ctx.coeff_shift)
    } else {
        let mut block = [[0i32; 8]; 8];
        for (i, row) in block.iter_mut().enumerate() {
            let src = luma_snap
                .row(y0 + i.min(block_h - 1))
                .and_then(|row| row.get(x0..x0 + block_w))
                .ok_or(CdefError::Geometry)?;
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (i32::from(src[j.min(block_w - 1)].to_u16()) >> ctx.coeff_shift) - 128;
            }
        }
        cdef_direction(&block)
    };
    let dir = if pri_base == 0 { 0 } else { y_dir };
    let var_str = (var >> 6).checked_ilog2().unwrap_or(0).min(12) as i32;
    let pri_str = if var != 0 {
        (pri_base * (4 + var_str) + 8) >> 4
    } else {
        0
    };
    let damping = ctx.params.damping + ctx.coeff_shift as i32;
    let y_filter = ctx.filter_ctx(pri_str, sec_str, damping, dir, 0);

    let uv_dir = if uv_pri == 0 {
        0
    } else {
        CDEF_UV_DIR[ctx.sub_x][ctx.sub_y][y_dir]
    };
    let uv_damping = ctx.params.damping + ctx.coeff_shift as i32 - 1;
    let uv_filter = ctx.filter_ctx(uv_pri, uv_sec, uv_damping, uv_dir, 1);

    let y_zero = pri_str == 0 && sec_str == 0;
    let uv_zero = uv_pri == 0 && uv_sec == 0;
    if !(y_zero || ctx.luma_lossless) {
        if luma_pad_ready {
            filter_interior_pad_into(filtered_y, pad, x0, y0, block_w, block_h, &y_filter)?;
        } else {
            compute_cdef_filter_plane::<S>(luma_snap, &y_filter, pad, filtered_y)?;
        }
    }
    match (u_snap, filtered_u) {
        (Some(snap), Some(filtered)) if !(uv_zero || ctx.chroma_lossless) => {
            compute_cdef_filter_plane::<S>(snap, &uv_filter, pad, filtered)?;
        }
        (Some(_), None) if !(uv_zero || ctx.chroma_lossless) => {
            return Err(CdefError::Workspace);
        }
        _ => {}
    }
    match (v_snap, filtered_v) {
        (Some(snap), Some(filtered)) if !(uv_zero || ctx.chroma_lossless) => {
            compute_cdef_filter_plane::<S>(snap, &uv_filter, pad, filtered)?;
        }
        (Some(_), None) if !(uv_zero || ctx.chroma_lossless) => {
            return Err(CdefError::Workspace);
        }
        _ => {}
    }
    Ok(())
}

struct CdefFilterCtx {
    r: usize,
    c: usize,
    pri_str: i32,
    sec_str: i32,
    damping: i32,
    dir: usize,
    sub: usize,
    coeff_shift: u32,
    max_sample: i32,
    mi_rows: usize,
    mi_cols: usize,
    frame_sub_x: usize,
    frame_sub_y: usize,
}

fn compute_cdef_filter_plane<S: ReconSample>(
    snap: FramePlane<'_, S>,
    ctx: &CdefFilterCtx,
    pad: &mut [u16; CDEF_PADDED_AREA],
    filtered: &mut StripePlane,
) -> Result<(), CdefError> {
    let sub_x = if ctx.sub > 0 { ctx.frame_sub_x } else { 0 };
    let sub_y = if ctx.sub > 0 { ctx.frame_sub_y } else { 0 };
    let x0 = (ctx.c * MI_SIZE) >> sub_x;
    let y0 = (ctx.r * MI_SIZE) >> sub_y;
    let w = (8 >> sub_x).min(snap.width().saturating_sub(x0));
    let h = (8 >> sub_y).min(snap.frame_height().saturating_sub(y0));
    if w == 0 || h == 0 {
        return Ok(());
    }

    let inside_x = ((ctx.mi_cols * MI_SIZE) >> sub_x).min(snap.width());
    let inside_y = ((ctx.mi_rows * MI_SIZE) >> sub_y).min(snap.frame_height());
    let interior = x0 >= CDEF_TAP_REACH
        && y0 >= CDEF_TAP_REACH
        && x0 + w - 1 + CDEF_TAP_REACH < inside_x
        && y0 + h - 1 + CDEF_TAP_REACH < inside_y;

    if interior {
        gather_interior_pad(snap, pad, x0, y0, w, h)?;
        return filter_interior_pad_into(filtered, pad, x0, y0, w, h, ctx);
    }

    let mut filtered_block = [0u16; 64];
    let offsets = CdefTapOffsets::for_direction(ctx.dir);
    for i in 0..h {
        for j in 0..w {
            let center = snap
                .get((x0 + j) as isize, (y0 + i) as isize)
                .ok_or(CdefError::Geometry)?;
            let taps = gather_taps(snap, &offsets, x0 + j, y0 + i, inside_x, inside_y, center);
            let filtered = cdef_filter_sample(
                &taps,
                ctx.pri_str,
                ctx.sec_str,
                ctx.damping,
                ctx.coeff_shift,
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
    for r in 0..h + 2 * CDEF_TAP_REACH {
        let src = snap
            .row(y0 - CDEF_TAP_REACH + r)
            .and_then(|row| row.get(x0 - CDEF_TAP_REACH..x0 + w + CDEF_TAP_REACH))
            .ok_or(CdefError::Workspace)?;
        let dst_start = r * CDEF_PADDED_SIDE;
        let dst = pad
            .get_mut(dst_start..dst_start + src.len())
            .ok_or(CdefError::Workspace)?;
        for (dst, src) in dst.iter_mut().zip(src) {
            *dst = src.to_u16();
        }
    }
    Ok(())
}

fn filter_interior_pad_into(
    filtered: &mut StripePlane,
    pad: &[u16; CDEF_PADDED_AREA],
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    ctx: &CdefFilterCtx,
) -> Result<(), CdefError> {
    let filter = CdefBlockFilter {
        pri_str: ctx.pri_str,
        sec_str: ctx.sec_str,
        damping: ctx.damping,
        dir: ctx.dir,
        coeff_shift: ctx.coeff_shift,
    };
    let rect = PlaneRect::new(x0, y0, w, h).map_err(|_| CdefError::Geometry)?;
    let (output, stride) = filtered.rect_mut(rect).ok_or(CdefError::Workspace)?;
    if cdef_filter_block_interior_to_valid_stride(pad, w, h, &filter, output, stride) {
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

fn gather_taps<T: ReconSample>(
    snap: FramePlane<'_, T>,
    offsets: &CdefTapOffsets,
    x: usize,
    y: usize,
    inside_x: usize,
    inside_y: usize,
    center: i32,
) -> CdefSampleTaps {
    let fetch = |(dy, dx): (isize, isize)| -> CdefTap {
        let y = y as isize + dy;
        let x = x as isize + dx;
        if x >= 0 && y >= 0 && (x as usize) < inside_x && (y as usize) < inside_y {
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
}

#[cfg(test)]
#[path = "cdef_tests.rs"]
mod tests;
