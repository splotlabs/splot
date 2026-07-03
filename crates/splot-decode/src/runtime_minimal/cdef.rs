// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::FrameHeaderCore;
use splot_recon::{
    BitDepth, CDEF_DIRECTIONS, CDEF_UV_DIR, CdefSampleTaps, CdefTap, CurrentFrameWorkspace,
    PlaneId, PlaneRect, ReconSample, cdef_direction, cdef_filter_sample,
};

const MI_SIZE: usize = 4;
const MI_SIZE_LOG2: u32 = 2;
const CDEF_UNIT_MI: usize = 16;
const STEP4: usize = 2;

const UNAVAILABLE_TAP: CdefTap = CdefTap {
    value: 0,
    available: false,
};

/// Parsed CDEF strengths for the admitted single-strength-set subset.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CdefFrameParams {
    /// Luma primary strength.
    pub(crate) y_pri: i32,
    /// Luma secondary strength.
    pub(crate) y_sec: i32,
    /// Chroma primary strength.
    pub(crate) uv_pri: i32,
    /// Chroma secondary strength.
    pub(crate) uv_sec: i32,
    /// CDEF damping.
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

/// Parsed CDEF strength index grid, one cell per 64x64 luma filter block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdefUnitGrid {
    rows: usize,
    cols: usize,
    values: Vec<Option<usize>>,
}

impl CdefUnitGrid {
    /// Creates a row-major CDEF unit grid.
    pub(crate) fn new(
        rows: usize,
        cols: usize,
        values: Vec<Option<usize>>,
    ) -> Result<Self, CdefError> {
        validate_grid_len(rows, cols, values.len())?;
        Ok(Self { rows, cols, values })
    }

    fn constant(mi_rows: usize, mi_cols: usize, value: usize) -> Result<Self, CdefError> {
        let rows = mi_rows.div_ceil(CDEF_UNIT_MI);
        let cols = mi_cols.div_ceil(CDEF_UNIT_MI);
        let values_len = rows.checked_mul(cols).ok_or(CdefError::Geometry)?;
        Ok(Self {
            rows,
            cols,
            values: vec![Some(value); values_len],
        })
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

/// Parsed CDEF skip decisions, one cell per luma 4x4 block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdefSkipGrid {
    rows: usize,
    cols: usize,
    values: Vec<bool>,
}

impl CdefSkipGrid {
    /// Creates a row-major CDEF skip grid.
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
    samples: Vec<i32>,
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
            samples.extend(row.iter().map(|value| i32::from(value.to_u16())));
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
        self.samples.get(y * self.width + x).copied()
    }
}

/// Applies AV2 § 7.18 CDEF in place.
pub(crate) fn cdef_general_intra_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    params: CdefFrameParams,
    mi_rows: usize,
    mi_cols: usize,
    bit_depth: BitDepth,
) -> Result<(), CdefError> {
    let grid = CdefUnitGrid::constant(mi_rows, mi_cols, 0)?;
    cdef_general_intra_frame_indexed(
        workspace,
        &[params],
        &grid,
        None,
        mi_rows,
        mi_cols,
        bit_depth,
    )
}

/// Applies AV2 § 7.18 CDEF using the parsed per-unit strength index grid.
pub(crate) fn cdef_general_intra_frame_indexed<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    strengths: &[CdefFrameParams],
    grid: &CdefUnitGrid,
    skip_grid: Option<&CdefSkipGrid>,
    mi_rows: usize,
    mi_cols: usize,
    bit_depth: BitDepth,
) -> Result<(), CdefError> {
    let coeff_shift = u32::from(bit_depth.bits()) - 8;
    let max_sample = i32::from(bit_depth.max_sample());

    let sub_x = 1usize;
    let sub_y = 1usize;

    let luma_size = workspace
        .plane(PlaneId::Y)
        .map_err(|_| CdefError::Workspace)?
        .storage_size();
    let u_size = workspace
        .plane(PlaneId::U)
        .map_err(|_| CdefError::Workspace)?
        .storage_size();
    let v_size = workspace
        .plane(PlaneId::V)
        .map_err(|_| CdefError::Workspace)?
        .storage_size();

    let luma_snap =
        PlaneSnapshot::capture(workspace, PlaneId::Y, luma_size.width(), luma_size.height())?;
    let u_snap = PlaneSnapshot::capture(workspace, PlaneId::U, u_size.width(), u_size.height())?;
    let v_snap = PlaneSnapshot::capture(workspace, PlaneId::V, v_size.width(), v_size.height())?;

    let mut r = 0usize;
    while r < mi_rows {
        let mut c = 0usize;
        while c < mi_cols {
            let Some(strength_index) = grid.strength_for_mi(r, c)? else {
                c += STEP4;
                continue;
            };
            if let Some(skip_grid) = skip_grid
                && skip_grid.all_skipped_8x8(r, c, mi_rows, mi_cols)?
            {
                c += STEP4;
                continue;
            }
            let params = *strengths.get(strength_index).ok_or(CdefError::Geometry)?;
            cdef_block(
                workspace,
                &CdefBlockCtx {
                    r,
                    c,
                    params,
                    coeff_shift,
                    max_sample,
                    mi_rows,
                    mi_cols,
                    sub_x,
                    sub_y,
                },
                &luma_snap,
                &u_snap,
                &v_snap,
            )?;
            c += STEP4;
        }
        r += STEP4;
    }

    Ok(())
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

fn cdef_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    ctx: &CdefBlockCtx,
    luma_snap: &PlaneSnapshot,
    u_snap: &PlaneSnapshot,
    v_snap: &PlaneSnapshot,
) -> Result<(), CdefError> {
    let x0 = ctx.c << MI_SIZE_LOG2;
    let y0 = ctx.r << MI_SIZE_LOG2;
    let block_w = 8.min(luma_snap.width.saturating_sub(x0));
    let block_h = 8.min(luma_snap.height.saturating_sub(y0));
    if block_w == 0 || block_h == 0 {
        return Ok(());
    }
    let mut block = [[0i32; 8]; 8];
    for (i, row) in block.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            let sample = luma_snap.get(
                (x0 + j.min(block_w - 1)) as isize,
                (y0 + i.min(block_h - 1)) as isize,
            );
            let sample = sample.ok_or(CdefError::Geometry)?;
            *cell = (sample >> ctx.coeff_shift) - 128;
        }
    }
    let (y_dir, var) = cdef_direction(&block);

    let pri_str = ctx.params.y_pri << ctx.coeff_shift;
    let sec_str = ctx.params.y_sec << ctx.coeff_shift;
    let dir = if pri_str == 0 { 0 } else { y_dir };
    let var_str = if var >> 6 != 0 {
        floor_log2_i64(var >> 6).min(12)
    } else {
        0
    };
    let pri_str = if var != 0 {
        (pri_str * (4 + var_str) + 8) >> 4
    } else {
        0
    };
    let damping = ctx.params.damping + ctx.coeff_shift as i32;
    let y_filter = ctx.filter_ctx(pri_str, sec_str, damping, dir, 0);
    cdef_filter_plane(workspace, PlaneId::Y, luma_snap, &y_filter)?;

    let uv_pri = ctx.params.uv_pri << ctx.coeff_shift;
    let uv_sec = ctx.params.uv_sec << ctx.coeff_shift;
    let uv_dir = if uv_pri == 0 {
        0
    } else {
        CDEF_UV_DIR[ctx.sub_x][ctx.sub_y][y_dir]
    };
    let uv_damping = ctx.params.damping + ctx.coeff_shift as i32 - 1;
    let uv_filter = ctx.filter_ctx(uv_pri, uv_sec, uv_damping, uv_dir, 1);
    for (plane, snap) in [(PlaneId::U, u_snap), (PlaneId::V, v_snap)] {
        cdef_filter_plane(workspace, plane, snap, &uv_filter)?;
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

fn cdef_filter_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    snap: &PlaneSnapshot,
    ctx: &CdefFilterCtx,
) -> Result<(), CdefError> {
    let sub_x = if ctx.sub > 0 { ctx.frame_sub_x } else { 0 };
    let sub_y = if ctx.sub > 0 { ctx.frame_sub_y } else { 0 };
    let x0 = (ctx.c * MI_SIZE) >> sub_x;
    let y0 = (ctx.r * MI_SIZE) >> sub_y;
    let w = (8 >> sub_x).min(snap.width.saturating_sub(x0));
    let h = (8 >> sub_y).min(snap.height.saturating_sub(y0));
    if w == 0 || h == 0 {
        return Ok(());
    }

    // § 7.18 CdefInside reduces to one plane-coordinate rectangle: the mi grid
    // covers x < (MiCols * MI_SIZE) >> sub_x and y < (MiRows * MI_SIZE) >> sub_y.
    let inside_x = ((ctx.mi_cols * MI_SIZE) >> sub_x).min(snap.width);
    let inside_y = ((ctx.mi_rows * MI_SIZE) >> sub_y).min(snap.height);

    let mut filtered_block = Vec::new();
    filtered_block
        .try_reserve_exact(w.checked_mul(h).ok_or(CdefError::Geometry)?)
        .map_err(|_| CdefError::Geometry)?;
    for i in 0..h {
        for j in 0..w {
            let center = snap
                .get((x0 + j) as isize, (y0 + i) as isize)
                .ok_or(CdefError::Geometry)?;
            let taps = gather_taps(snap, ctx, x0 + j, y0 + i, inside_x, inside_y, center);
            let filtered = cdef_filter_sample(
                &taps,
                ctx.pri_str,
                ctx.sec_str,
                ctx.damping,
                ctx.coeff_shift,
            );
            let clipped = filtered.clamp(0, ctx.max_sample);
            let value = T::try_from_u16(u16::try_from(clipped).map_err(|_| CdefError::Geometry)?)
                .map_err(|_| CdefError::Workspace)?;
            filtered_block.push(value);
        }
    }
    let rect = PlaneRect::new(x0, y0, w, h).map_err(|_| CdefError::Geometry)?;
    workspace
        .write_rect(plane, rect, &filtered_block, w)
        .map_err(|_| CdefError::Workspace)
}

fn gather_taps(
    snap: &PlaneSnapshot,
    ctx: &CdefFilterCtx,
    x: usize,
    y: usize,
    inside_x: usize,
    inside_y: usize,
    center: i32,
) -> CdefSampleTaps {
    let fetch = |dir: usize, k: usize, sign: i32| -> CdefTap {
        let y = y as isize + sign as isize * CDEF_DIRECTIONS[dir][k][0] as isize;
        let x = x as isize + sign as isize * CDEF_DIRECTIONS[dir][k][1] as isize;
        if x >= 0 && y >= 0 && (x as usize) < inside_x && (y as usize) < inside_y {
            match snap.samples.get(y as usize * snap.width + x as usize) {
                Some(&value) => CdefTap {
                    value,
                    available: true,
                },
                None => UNAVAILABLE_TAP,
            }
        } else {
            UNAVAILABLE_TAP
        }
    };

    let sign_for = |index: usize| if index == 0 { -1 } else { 1 };
    let mut primary = [[UNAVAILABLE_TAP; 2]; 2];
    let mut secondary = [[[UNAVAILABLE_TAP; 2]; 2]; 2];
    for k in 0..2 {
        for sign_index in 0..2 {
            let sign = sign_for(sign_index);
            primary[k][sign_index] = fetch(ctx.dir, k, sign);
            for (dir_off_index, dir_off) in [-2i32, 2].into_iter().enumerate() {
                let sdir = ((ctx.dir as i32 + dir_off) & 7) as usize;
                secondary[k][sign_index][dir_off_index] = fetch(sdir, k, sign);
            }
        }
    }

    CdefSampleTaps {
        center,
        primary,
        secondary,
    }
}

const fn floor_log2_i64(x: i64) -> i32 {
    if x <= 0 {
        0
    } else {
        63 - x.leading_zeros() as i32
    }
}

/// Errors from CDEF orchestration.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CdefError {
    /// Geometry or indexing went out of range.
    #[error("CDEF geometry computation went out of range")]
    Geometry,
    /// Workspace sample access went out of range.
    #[error("CDEF workspace sample access went out of bounds")]
    Workspace,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::test_support::yuv420_workspace as workspace_8bit;
    use super::*;

    #[test]
    fn flat_frame_is_unchanged() {
        let mut ws = workspace_8bit(64, 64, 100);
        cdef_general_intra_frame(
            &mut ws,
            CdefFrameParams {
                y_pri: 4,
                y_sec: 4,
                uv_pri: 0,
                uv_sec: 0,
                damping: 4,
            },
            16,
            16,
            BitDepth::Eight,
        )
        .unwrap();
        assert!(
            ws.samples(PlaneId::Y).unwrap().iter().all(|&s| s == 100),
            "flat luma unchanged"
        );
        assert!(
            ws.samples(PlaneId::U).unwrap().iter().all(|&s| s == 100),
            "flat chroma unchanged"
        );
    }

    #[test]
    fn partial_coded_edge_blocks_do_not_exceed_plane_bounds() {
        let width = 18usize;
        let height = 10usize;
        let mut ws = workspace_8bit(width, height, 100);
        cdef_general_intra_frame(
            &mut ws,
            CdefFrameParams {
                y_pri: 4,
                y_sec: 4,
                uv_pri: 2,
                uv_sec: 4,
                damping: 4,
            },
            height.div_ceil(MI_SIZE),
            width.div_ceil(MI_SIZE),
            BitDepth::Eight,
        )
        .unwrap();
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            assert!(
                ws.samples(plane).unwrap().iter().all(|&s| s == 100),
                "flat partial-edge plane remains unchanged"
            );
        }
    }

    #[test]
    fn small_ringing_step_is_deringed_within_bounds() {
        let mut ws = workspace_8bit(64, 64, 100);
        seed_luma_ripple(&mut ws);
        let before = luma_8x8(&ws);
        cdef_general_intra_frame(&mut ws, cdef_ripple_params(), 16, 16, BitDepth::Eight).unwrap();
        let after = luma_8x8(&ws);
        assert_ne!(before, after, "the ripple block must be filtered (changed)");
        assert!(
            after.iter().all(|&s| (97..=103).contains(&s)),
            "deringed samples stay within the original [97, 103] band: {after:?}"
        );
        assert_eq!(
            ws.reconstructed_sample(PlaneId::Y, 40, 40).unwrap(),
            100,
            "far flat region untouched"
        );
    }

    #[test]
    fn skip_grid_leaves_all_skipped_8x8_unfiltered() {
        let (before, after) = run_skip_grid_ripple(vec![true; 16 * 16]);
        assert_eq!(before, after, "all-skipped CDEF block bypasses filtering");
    }

    #[test]
    fn skip_grid_filters_mixed_8x8() {
        let mut skip_values = vec![true; 16 * 16];
        skip_values[0] = false;
        let (before, after) = run_skip_grid_ripple(skip_values);
        assert_ne!(before, after, "mixed CDEF block still filters");
    }

    fn run_skip_grid_ripple(skip_values: Vec<bool>) -> (Vec<u8>, Vec<u8>) {
        let mut ws = workspace_8bit(64, 64, 100);
        seed_luma_ripple(&mut ws);
        let before = luma_8x8(&ws);
        let grid = CdefUnitGrid::constant(16, 16, 0).unwrap();
        let skip = CdefSkipGrid::new(16, 16, skip_values).unwrap();
        cdef_general_intra_frame_indexed(
            &mut ws,
            &[cdef_ripple_params()],
            &grid,
            Some(&skip),
            16,
            16,
            BitDepth::Eight,
        )
        .unwrap();
        (before, luma_8x8(&ws))
    }

    fn seed_luma_ripple(ws: &mut CurrentFrameWorkspace<u8>) {
        for y in 0..8 {
            for x in 0..8 {
                let v = if (x + y) % 2 == 0 { 103 } else { 97 };
                ws.set_reconstructed_sample(PlaneId::Y, x, y, v).unwrap();
            }
        }
    }

    fn luma_8x8(ws: &CurrentFrameWorkspace<u8>) -> Vec<u8> {
        (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .map(|(x, y)| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap())
            .collect()
    }

    const fn cdef_ripple_params() -> CdefFrameParams {
        CdefFrameParams {
            y_pri: 4,
            y_sec: 4,
            uv_pri: 0,
            uv_sec: 0,
            damping: 4,
        }
    }

    #[test]
    fn snapshot_get_bounds() {
        let ws = workspace_8bit(16, 16, 50);
        let snap = PlaneSnapshot::capture(&ws, PlaneId::Y, 16, 16).unwrap();
        assert_eq!(snap.get(0, 0), Some(50));
        assert_eq!(snap.get(15, 15), Some(50));
        assert_eq!(snap.get(-1, 0), None, "negative x off-frame");
        assert_eq!(snap.get(16, 0), None, "x past width off-frame");
        assert_eq!(snap.get(0, 16), None, "y past height off-frame");
    }

    fn workspace_chroma_ripple(row_varying: bool) -> CurrentFrameWorkspace<u8> {
        let mut ws = workspace_8bit(64, 64, 128);
        for plane in [PlaneId::U, PlaneId::V] {
            for y in 0..32usize {
                for x in 0..32usize {
                    let v = if y % 2 == 0 { 130 } else { 126 };
                    ws.set_reconstructed_sample(plane, x, y, v).unwrap();
                }
            }
        }
        for y in 0..8usize {
            for x in 0..8usize {
                let g = if row_varying { y } else { x } as i32;
                let v = (100 + g * 6).clamp(0, 255) as u8;
                ws.set_reconstructed_sample(PlaneId::Y, x, y, v).unwrap();
            }
        }
        ws
    }

    fn chroma_top_left_4x4(ws: &CurrentFrameWorkspace<u8>, plane: PlaneId) -> Vec<u8> {
        (0..4)
            .flat_map(|y| (0..4).map(move |x| (x, y)))
            .map(|(x, y)| ws.reconstructed_sample(plane, x, y).unwrap())
            .collect()
    }

    fn run_cdef(ws: &mut CurrentFrameWorkspace<u8>, uv_pri: i32, uv_sec: i32) {
        cdef_general_intra_frame(
            ws,
            CdefFrameParams {
                y_pri: 0,
                y_sec: 0,
                uv_pri,
                uv_sec,
                damping: 4,
            },
            16,
            16,
            BitDepth::Eight,
        )
        .unwrap();
    }

    #[test]
    fn zero_uv_strengths_leave_chroma_untouched() {
        let before = workspace_chroma_ripple(true);
        let mut after = workspace_chroma_ripple(true);
        run_cdef(&mut after, 0, 0);
        for plane in [PlaneId::U, PlaneId::V] {
            assert_eq!(
                before.samples(plane).unwrap(),
                after.samples(plane).unwrap(),
                "uv strengths 0 -> chroma unchanged",
            );
        }
    }

    #[test]
    fn nonzero_uv_strengths_dering_chroma_only() {
        let before = workspace_chroma_ripple(true);
        let mut after = workspace_chroma_ripple(true);
        run_cdef(&mut after, 2, 4);
        for plane in [PlaneId::U, PlaneId::V] {
            assert_ne!(
                before.samples(plane).unwrap(),
                after.samples(plane).unwrap(),
                "nonzero uv -> chroma derings (changes)",
            );
            assert!(
                after
                    .samples(plane)
                    .unwrap()
                    .iter()
                    .all(|&s| (126..=130).contains(&s)),
                "deringed chroma stays within the original [126, 130] band",
            );
        }
        assert_eq!(
            before.samples(PlaneId::Y).unwrap(),
            after.samples(PlaneId::Y).unwrap(),
            "uv strengths are chroma-only: luma untouched",
        );
    }

    #[test]
    fn uv_dir_selection_tracks_luma_direction_only_when_uv_pri_nonzero() {
        let mut row_block = [[0i32; 8]; 8];
        let mut col_block = [[0i32; 8]; 8];
        for i in 0..8 {
            for j in 0..8 {
                row_block[i][j] = (100 + i as i32 * 6) - 128;
                col_block[i][j] = (100 + j as i32 * 6) - 128;
            }
        }
        let (row_dir, _) = cdef_direction(&row_block);
        let (col_dir, _) = cdef_direction(&col_block);
        assert_ne!(
            row_dir, col_dir,
            "the two luma blocks must select different yDirs to drive Cdef_Uv_Dir",
        );

        let mut horiz = workspace_chroma_ripple(true);
        let mut vert = workspace_chroma_ripple(false);
        run_cdef(&mut horiz, 2, 4);
        run_cdef(&mut vert, 2, 4);
        assert_ne!(
            chroma_top_left_4x4(&horiz, PlaneId::U),
            chroma_top_left_4x4(&vert, PlaneId::U),
            "uv_pri != 0: Cdef_Uv_Dir maps yDir to a primary chroma direction, so the \
             chroma output depends on the luma direction",
        );

        let mut horiz0 = workspace_chroma_ripple(true);
        let mut vert0 = workspace_chroma_ripple(false);
        run_cdef(&mut horiz0, 0, 4);
        run_cdef(&mut vert0, 0, 4);
        assert_eq!(
            chroma_top_left_4x4(&horiz0, PlaneId::U),
            chroma_top_left_4x4(&vert0, PlaneId::U),
            "uv_pri == 0: direction is forced to 0, so the luma direction is ignored",
        );
    }
}
