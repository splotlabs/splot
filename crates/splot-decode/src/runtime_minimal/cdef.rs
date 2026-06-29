// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.18 CDEF (Constrained Directional Enhancement Filter) orchestration for
//! the general intra decode path.
//!
//! This is the scheduler over the `splot-recon` per-block CDEF primitives
//! ([`cdef_direction`], [`cdef_filter_sample`], [`CDEF_DIRECTIONS`], [`CDEF_UV_DIR`]):
//! it iterates the § 7.18 64x64-unit → 8x8-block grid, derives the § 7.18.2 direction
//! / variance from the deblocked luma block, derives the § 7.18.1 primary / secondary
//! strengths and damping, fetches each output sample's primary / secondary directional
//! taps with the § 5.20.9.3 `is_inside_filter_region` (single-tile → `is_inside_frame`)
//! availability check from a pre-CDEF snapshot of `CurrFrame`, and writes the deringed
//! `CdefFrame` samples back into the [`CurrentFrameWorkspace`] IN PLACE after deblocking
//! and before `workspace.freeze()`.
//!
//! Reading from a snapshot is load-bearing: § 7.18 filters `CurrFrame` (the deblocked
//! frame) into `CdefFrame`, so every tap must read the pre-CDEF sample even after an
//! earlier 8x8 block in raster order has written its output.
//!
//! Verified subset (everything else is rejected by the general intra route gate before
//! any caller-visible output): an 8-bit 4:2:0 intra key frame with
//! `cdef_frame_enable == 1`, `CdefStrengths == 1` (so § 5.20.10.1 `read_cdef` reads NO
//! per-block symbol — `cdef_idx[r][c]` is `0` for the whole frame), and
//! `cdef_on_skip_txfm_frame_enable == 1` (so the § 7.18.1 `skip` is `0` with no `Skips`
//! lookup). Segmentation is disabled and no segment is lossless (so the § 7.18.1
//! `LosslessArray` skip terms are `0`). Both luma and chroma (uv) strengths are applied
//! and oracle-pinned: the § 7.18.1 chroma steps 9-14 — the
//! `Cdef_Uv_Dir[SubsamplingX][SubsamplingY][yDir]` direction selection (engaged when
//! `uv_pri != 0`), the 4:2:0 subsampled 4x4 chroma tap addressing, and the
//! `CdefDamping - 1` chroma damping — are bit-exact vs avmdec (and dav2d) on a
//! nonzero-uv fixture, so a sample-changing chroma CDEF output is hashed against the
//! oracle rather than over-rejected upstream.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-CDEF`.

use splot_recon::{
    BitDepth, CDEF_DIRECTIONS, CDEF_UV_DIR, CdefSampleTaps, CdefTap, CurrentFrameWorkspace,
    PlaneId, ReconSample, cdef_direction, cdef_filter_sample,
};

/// AV2 § 3 `MI_SIZE`: the side of one mode-info unit in luma samples.
const MI_SIZE: usize = 4;
/// AV2 § 3 `MI_SIZE_LOG2`.
const MI_SIZE_LOG2: u32 = 2;
/// `Num_4x4_Blocks_Wide[BLOCK_8X8]`: the § 7.18 `step4` (8x8 block stride in MI units).
const STEP4: usize = 2;

/// One frame's parsed § 5.18.7.10 CDEF parameters for the admitted single-strength-set
/// subset: the strengths the § 7.18.1 process applies (`cdef_idx` is `0` everywhere).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CdefFrameParams {
    /// `cdef_y_pri_strength[0]`.
    pub(crate) y_pri: i32,
    /// `cdef_y_sec_strength[0]`.
    pub(crate) y_sec: i32,
    /// `cdef_uv_pri_strength[0]`.
    pub(crate) uv_pri: i32,
    /// `cdef_uv_sec_strength[0]`.
    pub(crate) uv_sec: i32,
    /// `CdefDamping`.
    pub(crate) damping: i32,
}

/// A pre-CDEF snapshot of one plane's reconstructed (`CurrFrame`) samples, addressed
/// in plane sample coordinates. Out-of-frame reads return `None` so the caller can set
/// `CdefAvailable = 0`.
struct PlaneSnapshot {
    width: usize,
    height: usize,
    samples: Vec<i32>,
}

impl PlaneSnapshot {
    /// Snapshots the visible region of `plane` from `workspace`.
    fn capture<T: ReconSample>(
        workspace: &CurrentFrameWorkspace<T>,
        plane: PlaneId,
        width: usize,
        height: usize,
    ) -> Result<Self, CdefError> {
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(width.checked_mul(height).ok_or(CdefError::Geometry)?)
            .map_err(|_| CdefError::Geometry)?;
        for y in 0..height {
            for x in 0..width {
                let value = workspace
                    .reconstructed_sample(plane, x, y)
                    .map_err(|_| CdefError::Workspace)?;
                samples.push(i32::from(value.to_u16()));
            }
        }
        Ok(Self {
            width,
            height,
            samples,
        })
    }

    /// The sample at `(x, y)`, or `None` when off-frame.
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

/// AV2 § 7.18 CDEF orchestration over the decoded general intra frame, applied in
/// place to `workspace`.
///
/// `params` are the single-strength-set CDEF parameters (§ 7.18.1); `mi_rows` /
/// `mi_cols` are the frame MI dimensions; `bit_depth` is the active decoded bit depth.
/// 4:2:0 chroma (the admitted subset) is assumed (`SubsamplingX == SubsamplingY == 1`,
/// `NumPlanes == 3`).
///
/// Returns `Err` only on an internal inconsistency (a workspace access out of bounds or
/// a geometry overflow); for the verified subset it is total.
pub(crate) fn cdef_general_intra_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    params: CdefFrameParams,
    mi_rows: usize,
    mi_cols: usize,
    bit_depth: BitDepth,
) -> Result<(), CdefError> {
    let coeff_shift = u32::from(bit_depth.bits()) - 8;
    let max_sample = i32::from(bit_depth.max_sample());

    let sub_x = 1usize;
    let sub_y = 1usize;

    let luma_w = mi_cols * MI_SIZE;
    let luma_h = mi_rows * MI_SIZE;
    let chroma_w = luma_w >> sub_x;
    let chroma_h = luma_h >> sub_y;

    let luma_snap = PlaneSnapshot::capture(workspace, PlaneId::Y, luma_w, luma_h)?;
    let u_snap = PlaneSnapshot::capture(workspace, PlaneId::U, chroma_w, chroma_h)?;
    let v_snap = PlaneSnapshot::capture(workspace, PlaneId::V, chroma_w, chroma_h)?;

    let mut r = 0usize;
    while r < mi_rows {
        let mut c = 0usize;
        while c < mi_cols {
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

/// Inputs to the § 7.18.1 CDEF block process for one 8x8 block.
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

/// AV2 § 7.18.1 CDEF block process for one 8x8 block (`idx == 0`, `skip == 0`).
fn cdef_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    ctx: &CdefBlockCtx,
    luma_snap: &PlaneSnapshot,
    u_snap: &PlaneSnapshot,
    v_snap: &PlaneSnapshot,
) -> Result<(), CdefError> {
    let x0 = ctx.c << MI_SIZE_LOG2;
    let y0 = ctx.r << MI_SIZE_LOG2;
    let mut block = [[0i32; 8]; 8];
    for (i, row) in block.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            let sample = luma_snap
                .get((x0 + j) as isize, (y0 + i) as isize)
                .ok_or(CdefError::Geometry)?;
            *cell = (sample >> ctx.coeff_shift) - 128;
        }
    }
    let (y_dir, var) = cdef_direction(&block);

    #[cfg(debug_assertions)]
    if std::env::var_os("SPLOT_CDEF_TRACE").is_some() {
        eprintln!(
            "CDEF unit r={} c={} yDir={} var={} y_pri={} y_sec={} damp={}",
            ctx.r, ctx.c, y_dir, var, ctx.params.y_pri, ctx.params.y_sec, ctx.params.damping
        );
    }

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
    cdef_filter_plane(
        workspace,
        PlaneId::Y,
        luma_snap,
        &CdefFilterCtx {
            r: ctx.r,
            c: ctx.c,
            pri_str,
            sec_str,
            damping,
            dir,
            sub: 0,
            coeff_shift: ctx.coeff_shift,
            max_sample: ctx.max_sample,
            mi_rows: ctx.mi_rows,
            mi_cols: ctx.mi_cols,
            frame_sub_x: ctx.sub_x,
            frame_sub_y: ctx.sub_y,
        },
    )?;

    let uv_pri = ctx.params.uv_pri << ctx.coeff_shift;
    let uv_sec = ctx.params.uv_sec << ctx.coeff_shift;
    let uv_dir = if uv_pri == 0 {
        0
    } else {
        CDEF_UV_DIR[ctx.sub_x][ctx.sub_y][y_dir]
    };
    let uv_damping = ctx.params.damping + ctx.coeff_shift as i32 - 1;
    for (plane, snap) in [(PlaneId::U, u_snap), (PlaneId::V, v_snap)] {
        cdef_filter_plane(
            workspace,
            plane,
            snap,
            &CdefFilterCtx {
                r: ctx.r,
                c: ctx.c,
                pri_str: uv_pri,
                sec_str: uv_sec,
                damping: uv_damping,
                dir: uv_dir,
                sub: 1,
                coeff_shift: ctx.coeff_shift,
                max_sample: ctx.max_sample,
                mi_rows: ctx.mi_rows,
                mi_cols: ctx.mi_cols,
                frame_sub_x: ctx.sub_x,
                frame_sub_y: ctx.sub_y,
            },
        )?;
    }
    Ok(())
}

/// Inputs to the § 7.18.3 CDEF filter process for one plane of one 8x8 block.
struct CdefFilterCtx {
    r: usize,
    c: usize,
    pri_str: i32,
    sec_str: i32,
    damping: i32,
    dir: usize,
    /// `1` for a subsampled chroma plane, `0` for luma (selects `w`/`h` and `subX`/`subY`).
    sub: usize,
    coeff_shift: u32,
    max_sample: i32,
    mi_rows: usize,
    mi_cols: usize,
    frame_sub_x: usize,
    frame_sub_y: usize,
}

/// AV2 § 7.18.3 CDEF filter process for one plane of one 8x8 block.
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
    let w = 8 >> sub_x;
    let h = 8 >> sub_y;

    for i in 0..h {
        for j in 0..w {
            let center = snap
                .get((x0 + j) as isize, (y0 + i) as isize)
                .ok_or(CdefError::Geometry)?;
            let taps = gather_taps(snap, ctx, x0, y0, i, j, sub_x, sub_y, center);
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
            workspace
                .set_reconstructed_sample(plane, x0 + j, y0 + i, value)
                .map_err(|_| CdefError::Workspace)?;
        }
    }
    Ok(())
}

/// AV2 § 7.18.3 `cdef_get_at` for all six directional taps of one output sample.
#[allow(clippy::too_many_arguments)]
fn gather_taps(
    snap: &PlaneSnapshot,
    ctx: &CdefFilterCtx,
    x0: usize,
    y0: usize,
    i: usize,
    j: usize,
    sub_x: usize,
    sub_y: usize,
    center: i32,
) -> CdefSampleTaps {
    let fetch = |dir: usize, k: usize, sign: i32| -> CdefTap {
        let y = (y0 + i) as isize + sign as isize * CDEF_DIRECTIONS[dir][k][0] as isize;
        let x = (x0 + j) as isize + sign as isize * CDEF_DIRECTIONS[dir][k][1] as isize;
        let candidate_r = (y << sub_y) >> MI_SIZE_LOG2;
        let candidate_c = (x << sub_x) >> MI_SIZE_LOG2;
        let inside = candidate_r >= 0
            && candidate_c >= 0
            && (candidate_r as usize) < ctx.mi_rows
            && (candidate_c as usize) < ctx.mi_cols;
        if inside {
            match snap.get(x, y) {
                Some(value) => CdefTap {
                    value,
                    available: true,
                },
                None => CdefTap {
                    value: 0,
                    available: false,
                },
            }
        } else {
            CdefTap {
                value: 0,
                available: false,
            }
        }
    };

    let sign_for = |index: usize| if index == 0 { -1 } else { 1 };
    let mut primary = [[CdefTap {
        value: 0,
        available: false,
    }; 2]; 2];
    let mut secondary = [[[CdefTap {
        value: 0,
        available: false,
    }; 2]; 2]; 2];
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

/// AV2 § 4.7 `FloorLog2` for the i64 `var >> 6` (guarded nonzero by the caller).
const fn floor_log2_i64(x: i64) -> i32 {
    if x <= 0 {
        0
    } else {
        63 - x.leading_zeros() as i32
    }
}

/// Errors from the CDEF orchestration. These signal an internal inconsistency (the
/// per-block primitives are total for valid inputs), so the caller maps them to an
/// `unsupported-feature` decode diagnostic rather than a silent wrong-pixel output.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CdefError {
    /// A geometry / index computation went out of range.
    #[error("CDEF geometry computation went out of range")]
    Geometry,
    /// A workspace read/write went out of bounds or produced an out-of-range sample.
    #[error("CDEF workspace sample access went out of bounds")]
    Workspace,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use splot_recon::{DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize};

    fn workspace_8bit(width: usize, height: usize, fill: u8) -> CurrentFrameWorkspace<u8> {
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            PlaneSize::new(width, height).unwrap(),
            PlaneRect::new(0, 0, width, height).unwrap(),
        )
        .unwrap();
        CurrentFrameWorkspace::<u8>::new(info, fill).unwrap()
    }

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
    fn small_ringing_step_is_deringed_within_bounds() {
        let mut ws = workspace_8bit(64, 64, 100);
        for y in 0..8 {
            for x in 0..8 {
                let v = if (x + y) % 2 == 0 { 103 } else { 97 };
                ws.set_reconstructed_sample(PlaneId::Y, x, y, v).unwrap();
            }
        }
        let before: Vec<u8> = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .map(|(x, y)| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap())
            .collect();
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
        let after: Vec<u8> = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .map(|(x, y)| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap())
            .collect();
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
    fn snapshot_get_bounds() {
        let ws = workspace_8bit(16, 16, 50);
        let snap = PlaneSnapshot::capture(&ws, PlaneId::Y, 16, 16).unwrap();
        assert_eq!(snap.get(0, 0), Some(50));
        assert_eq!(snap.get(15, 15), Some(50));
        assert_eq!(snap.get(-1, 0), None, "negative x off-frame");
        assert_eq!(snap.get(16, 0), None, "x past width off-frame");
        assert_eq!(snap.get(0, 16), None, "y past height off-frame");
    }

    /// Builds a 64x64 4:2:0 frame (mi 16x16, chroma 32x32) whose top-left luma 8x8
    /// carries a directional gradient (`row_varying` selects the §7.18.2 yDir) and
    /// whose chroma planes carry a row-alternating ±2 ripple on a flat 128 field —
    /// a small ringing pattern §7.18 CDEF derings. (The amplitude is kept low: a
    /// larger diff is rejected by the §7.18.3 `constrain` rampdown under the
    /// `CdefDamping - 1` chroma damping, leaving the chroma untouched.)
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
                y_pri: 0, // luma unfiltered: isolate the chroma path (yDir is still
                y_sec: 0, // derived from the luma block content for Cdef_Uv_Dir).
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

        // uv_pri nonzero: the top-left chroma 4x4 differs between the two luma
        // directions, because Cdef_Uv_Dir picks a different primary chroma direction.
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
