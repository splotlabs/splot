// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_parallel::prelude::*;
use splot_recon::{
    BitDepth, CDEF_DIRECTIONS, CDEF_PADDED_AREA, CDEF_PADDED_SIDE, CDEF_UV_DIR, CdefBlockFilter,
    CdefSampleTaps, CdefTap, CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample,
    cdef_direction, cdef_filter_block_interior, cdef_filter_sample,
};

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

struct PlaneSnapshot {
    width: usize,
    height: usize,
    samples: Vec<u16>,
}

impl PlaneSnapshot {
    fn capture<T: ReconSample>(
        workspace: &CurrentFrameWorkspace<T>,
        plane: PlaneId,
        width: usize,
        height: usize,
    ) -> Result<Self, CdefError> {
        let source = workspace.plane(plane).map_err(|_| CdefError::Workspace)?;
        let stride = source.stride_samples();
        let backing = source.samples();
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(width.checked_mul(height).ok_or(CdefError::Geometry)?)
            .map_err(|_| CdefError::Geometry)?;
        for y in 0..height {
            let start = y.checked_mul(stride).ok_or(CdefError::Geometry)?;
            let end = start.checked_add(width).ok_or(CdefError::Geometry)?;
            let row = backing.get(start..end).ok_or(CdefError::Workspace)?;
            samples.extend(row.iter().map(|value| value.to_u16()));
        }
        Ok(Self {
            width,
            height,
            samples,
        })
    }

    fn get(&self, x: isize, y: isize) -> Option<i32> {
        if x < 0 || y < 0 {
            return None;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.width || y >= self.height {
            return None;
        }
        self.samples
            .get(y * self.width + x)
            .map(|&value| i32::from(value))
    }
}

struct CdefBandView<'a, T: ReconSample> {
    samples: &'a mut [T],
    stride: usize,
    top_row: usize,
}

struct CdefRowBands<'a, T: ReconSample> {
    band_mi_rows: usize,
    bands: Vec<(
        CdefBandView<'a, T>,
        CdefBandView<'a, T>,
        CdefBandView<'a, T>,
    )>,
}

impl<'a, T: ReconSample> CdefRowBands<'a, T> {
    fn split(
        workspace: &'a mut CurrentFrameWorkspace<T>,
        mi_rows: usize,
        chroma_sub_y: usize,
    ) -> Option<Self> {
        let workers = splot_parallel::current_pool_width();
        let cdef_rows = mi_rows.div_ceil(STEP4);
        let rows_per_band = cdef_rows.div_ceil(workers.checked_mul(4)?).max(1);
        let band_mi_rows = rows_per_band.checked_mul(STEP4)?;
        let luma_band_rows = band_mi_rows.checked_mul(MI_SIZE)?;
        let chroma_band_rows = luma_band_rows >> chroma_sub_y;
        let needed = cdef_rows.div_ceil(rows_per_band);

        let (y, u, v) = workspace.as_frame_mut().into_planes();
        let (u, v) = (u?, v?);
        let y_stride = y.stride_samples();
        let u_stride = u.stride_samples();
        let v_stride = v.stride_samples();
        let y_chunk = luma_band_rows.checked_mul(y_stride)?;
        let u_chunk = chroma_band_rows.checked_mul(u_stride)?;
        let v_chunk = chroma_band_rows.checked_mul(v_stride)?;
        if y_chunk == 0 || u_chunk == 0 || v_chunk == 0 {
            return None;
        }

        let bands: Vec<_> = y
            .into_samples()
            .chunks_mut(y_chunk)
            .zip(u.into_samples().chunks_mut(u_chunk))
            .zip(v.into_samples().chunks_mut(v_chunk))
            .enumerate()
            .map(|(band, ((y_band, u_band), v_band))| {
                (
                    CdefBandView {
                        samples: y_band,
                        stride: y_stride,
                        top_row: band * luma_band_rows,
                    },
                    CdefBandView {
                        samples: u_band,
                        stride: u_stride,
                        top_row: band * chroma_band_rows,
                    },
                    CdefBandView {
                        samples: v_band,
                        stride: v_stride,
                        top_row: band * chroma_band_rows,
                    },
                )
            })
            .collect();
        if bands.len() < needed {
            return None;
        }
        Some(Self {
            band_mi_rows,
            bands,
        })
    }
}

pub(crate) fn cdef_general_intra_frame_indexed<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    strengths: &[CdefFrameParams],
    grid: &CdefUnitGrid,
    skip_grid: Option<&CdefSkipGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    mi_size: (usize, usize),
    bit_depth: BitDepth,
) -> Result<(), CdefError> {
    let (mi_rows, mi_cols) = mi_size;
    let coeff_shift = u32::from(bit_depth.bits()) - 8;
    let max_sample = i32::from(bit_depth.max_sample());

    let pixel_format = workspace.info().pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let has_chroma = !pixel_format.is_monochrome();

    let luma_size = workspace
        .plane(PlaneId::Y)
        .map_err(|_| CdefError::Workspace)?
        .storage_size();

    let luma_snap =
        PlaneSnapshot::capture(workspace, PlaneId::Y, luma_size.width(), luma_size.height())?;
    let (u_snap, v_snap) = if has_chroma {
        let u_size = workspace
            .plane(PlaneId::U)
            .map_err(|_| CdefError::Workspace)?
            .storage_size();
        let v_size = workspace
            .plane(PlaneId::V)
            .map_err(|_| CdefError::Workspace)?
            .storage_size();
        (
            Some(PlaneSnapshot::capture(
                workspace,
                PlaneId::U,
                u_size.width(),
                u_size.height(),
            )?),
            Some(PlaneSnapshot::capture(
                workspace,
                PlaneId::V,
                v_size.width(),
                v_size.height(),
            )?),
        )
    } else {
        (None, None)
    };

    let block_ctx_at = |r: usize, c: usize| -> Result<Option<CdefBlockCtx>, CdefError> {
        let Some(strength_index) = grid.strength_for_mi(r, c)? else {
            return Ok(None);
        };
        if let Some(skip_grid) = skip_grid
            && skip_grid.all_skipped_8x8(r, c, mi_rows, mi_cols)?
        {
            return Ok(None);
        }
        let luma_lossless = lossless_grid.is_some_and(|grid| grid.cdef_luma_lossless(r, c));
        let chroma_lossless = lossless_grid.is_some_and(|grid| {
            grid.cdef_chroma_lossless(PlaneId::U, r, c)
                && grid.cdef_chroma_lossless(PlaneId::V, r, c)
        });
        if luma_lossless && (!has_chroma || chroma_lossless) {
            return Ok(None);
        }
        let params = *strengths.get(strength_index).ok_or(CdefError::Geometry)?;
        Ok(Some(CdefBlockCtx {
            r,
            c,
            params,
            coeff_shift,
            max_sample,
            mi_rows,
            mi_cols,
            sub_x,
            sub_y,
            luma_lossless,
            chroma_lossless,
        }))
    };

    if splot_parallel::on_worker_pool()
        && let Some(bands) = CdefRowBands::split(workspace, mi_rows, sub_y)
    {
        let timer = crate::timing::start();
        let tally = crate::timing::WorkerTally::new();
        let workers = splot_parallel::current_pool_width();
        let band_mi_rows = bands.band_mi_rows;
        let band_count = bands.bands.len();
        let result = bands.bands.into_par_iter().enumerate().try_for_each(
            |(band, (mut y_band, mut u_band, mut v_band))| {
                tally.note_worker();
                let mut r = band * band_mi_rows;
                let r_end = r.saturating_add(band_mi_rows).min(mi_rows);
                while r < r_end {
                    let mut c = 0usize;
                    while c < mi_cols {
                        if let Some(ctx) = block_ctx_at(r, c)? {
                            let output = compute_cdef_block::<T>(
                                &ctx,
                                &luma_snap,
                                u_snap.as_ref(),
                                v_snap.as_ref(),
                            )?;
                            for (plane, rect, samples, width) in output.into_iter().flatten() {
                                let band_view = match plane {
                                    PlaneId::Y => &mut y_band,
                                    PlaneId::U => &mut u_band,
                                    PlaneId::V => &mut v_band,
                                };
                                crate::tile::plane_bands::write_rect_into_band(
                                    band_view.samples,
                                    band_view.stride,
                                    band_view.top_row,
                                    rect,
                                    &samples,
                                    width,
                                )
                                .ok_or(CdefError::Workspace)?;
                            }
                        }
                        c += STEP4;
                    }
                    r += STEP4;
                }
                Ok(())
            },
        );
        crate::timing::report_detail(
            "cdef_bands",
            timer,
            &format!(
                "units={band_count} threads={workers} workers_used={}",
                tally.workers_used()
            ),
        );
        return result;
    }

    let mut r = 0usize;
    while r < mi_rows {
        let mut c = 0usize;
        while c < mi_cols {
            if let Some(ctx) = block_ctx_at(r, c)? {
                let output =
                    compute_cdef_block::<T>(&ctx, &luma_snap, u_snap.as_ref(), v_snap.as_ref())?;
                for (plane, rect, samples, width) in output.into_iter().flatten() {
                    workspace
                        .write_rect(plane, rect, &samples, width)
                        .map_err(|_| CdefError::Workspace)?;
                }
            }
            c += STEP4;
        }
        r += STEP4;
    }

    Ok(())
}

type CdefPlaneOutput<T> = (PlaneId, PlaneRect, [T; 64], usize);

type CdefBlockOutput<T> = [Option<CdefPlaneOutput<T>>; 3];

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

fn compute_cdef_block<T: ReconSample>(
    ctx: &CdefBlockCtx,
    luma_snap: &PlaneSnapshot,
    u_snap: Option<&PlaneSnapshot>,
    v_snap: Option<&PlaneSnapshot>,
) -> Result<CdefBlockOutput<T>, CdefError> {
    let x0 = ctx.c << MI_SIZE_LOG2;
    let y0 = ctx.r << MI_SIZE_LOG2;
    let block_w = 8.min(luma_snap.width.saturating_sub(x0));
    let block_h = 8.min(luma_snap.height.saturating_sub(y0));
    if block_w == 0 || block_h == 0 {
        return Ok([None, None, None]);
    }
    let pri_base = ctx.params.y_pri << ctx.coeff_shift;
    let sec_str = ctx.params.y_sec << ctx.coeff_shift;
    let uv_pri = ctx.params.uv_pri << ctx.coeff_shift;
    let uv_sec = ctx.params.uv_sec << ctx.coeff_shift;

    let (y_dir, var) = if pri_base == 0 && uv_pri == 0 {
        (0, 0)
    } else {
        let mut block = [[0i32; 8]; 8];
        for (i, row) in block.iter_mut().enumerate() {
            let start = (y0 + i.min(block_h - 1)) * luma_snap.width + x0;
            let src = luma_snap
                .samples
                .get(start..start + block_w)
                .ok_or(CdefError::Geometry)?;
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (i32::from(src[j.min(block_w - 1)]) >> ctx.coeff_shift) - 128;
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

    let plane_out = |identity: bool, plane, snap, filter: &CdefFilterCtx| {
        if identity {
            Ok(None)
        } else {
            compute_cdef_filter_plane::<T>(plane, snap, filter)
        }
    };
    let y_zero = pri_str == 0 && sec_str == 0;
    let uv_zero = uv_pri == 0 && uv_sec == 0;
    let output = [
        plane_out(
            y_zero || ctx.luma_lossless,
            PlaneId::Y,
            luma_snap,
            &y_filter,
        )?,
        u_snap
            .map(|snap| plane_out(uv_zero || ctx.chroma_lossless, PlaneId::U, snap, &uv_filter))
            .transpose()?
            .flatten(),
        v_snap
            .map(|snap| plane_out(uv_zero || ctx.chroma_lossless, PlaneId::V, snap, &uv_filter))
            .transpose()?
            .flatten(),
    ];
    Ok(output)
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

fn compute_cdef_filter_plane<T: ReconSample>(
    plane: PlaneId,
    snap: &PlaneSnapshot,
    ctx: &CdefFilterCtx,
) -> Result<Option<CdefPlaneOutput<T>>, CdefError> {
    let sub_x = if ctx.sub > 0 { ctx.frame_sub_x } else { 0 };
    let sub_y = if ctx.sub > 0 { ctx.frame_sub_y } else { 0 };
    let x0 = (ctx.c * MI_SIZE) >> sub_x;
    let y0 = (ctx.r * MI_SIZE) >> sub_y;
    let w = (8 >> sub_x).min(snap.width.saturating_sub(x0));
    let h = (8 >> sub_y).min(snap.height.saturating_sub(y0));
    if w == 0 || h == 0 {
        return Ok(None);
    }

    let inside_x = ((ctx.mi_cols * MI_SIZE) >> sub_x).min(snap.width);
    let inside_y = ((ctx.mi_rows * MI_SIZE) >> sub_y).min(snap.height);
    let interior = x0 >= CDEF_TAP_REACH
        && y0 >= CDEF_TAP_REACH
        && x0 + w - 1 + CDEF_TAP_REACH < inside_x
        && y0 + h - 1 + CDEF_TAP_REACH < inside_y;

    let mut filtered_block = [T::default(); 64];
    if interior {
        let mut pad = [0i32; CDEF_PADDED_AREA];
        for r in 0..h + 2 * CDEF_TAP_REACH {
            let start = (y0 - CDEF_TAP_REACH + r) * snap.width + (x0 - CDEF_TAP_REACH);
            let src = snap
                .samples
                .get(start..start + w + 2 * CDEF_TAP_REACH)
                .ok_or(CdefError::Workspace)?;
            for (dst, &value) in pad[r * CDEF_PADDED_SIDE..].iter_mut().zip(src) {
                *dst = i32::from(value);
            }
        }
        let filter = CdefBlockFilter {
            pri_str: ctx.pri_str,
            sec_str: ctx.sec_str,
            damping: ctx.damping,
            dir: ctx.dir,
            coeff_shift: ctx.coeff_shift,
        };
        let mut out = [0i32; 64];
        cdef_filter_block_interior(&pad, w, h, &filter, &mut out);
        for (dst, &filtered) in filtered_block.iter_mut().zip(&out).take(w * h) {
            *dst = storage_sample::<T>(filtered, ctx.max_sample)?;
        }
    } else {
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
                filtered_block[i * w + j] = storage_sample::<T>(filtered, ctx.max_sample)?;
            }
        }
    }
    let rect = PlaneRect::new(x0, y0, w, h).map_err(|_| CdefError::Geometry)?;
    Ok(Some((plane, rect, filtered_block, w)))
}

fn storage_sample<T: ReconSample>(filtered: i32, max_sample: i32) -> Result<T, CdefError> {
    let clipped = filtered.clamp(0, max_sample);
    T::try_from_u16(u16::try_from(clipped).map_err(|_| CdefError::Geometry)?)
        .map_err(|_| CdefError::Workspace)
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

fn gather_taps(
    snap: &PlaneSnapshot,
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
            match snap.samples.get(y as usize * snap.width + x as usize) {
                Some(&value) => CdefTap {
                    value: i32::from(value),
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
