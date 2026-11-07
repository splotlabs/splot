// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.5/§ 7.13.6 chroma-from-luma reconstruction helpers.
//!
//! § 7.13.5 applies 64-sample clamping to `L`; `lumaAvg` and `implicitAlpha` use
//! transform-local reference lines (docs/spec/av2/1.0.0/07-decoding-process.md).
//!
//! A block resolves its luma rows, neighbour sums and per-plane models once,
//! before either chroma plane is written, then predicts each plane straight
//! into the frame when it codes no residual.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`,
//! `DECODE-GENERAL-INTRA-CFL-NON-420`,
//! `DECODE-MHCCP-FRAME-EDGE-REFERENCE-EXTENT`.

use fearless_simd::{Bytes as _, Select as _, Simd, SimdBase, i32x8, u16x16, u32x8};

use splot_recon::math::{approx_divide, resolve_division, round2_signed_i32};
use splot_recon::mhccp::{MhccpPredictor, MhccpRefs, derive_mhccp_params};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraRectBlockSize, PixelFormat, PlaneId, ReconSample,
};

#[cfg(target_arch = "x86_64")]
use super::cfl_luma::{
    Luma420Block, cfl_alpha_fits_i16, cfl_predict_u8, cfl_predict_u16, luma_420,
};
use crate::bitstream::tile_payload::{
    CflMultiDirection, CflParams, CoeffBlock, GeneralIntraResidualError,
};
use crate::pipeline::reconstruct::{commit_intra_prediction, predict_intra_in_frame};
use crate::tile::block_context::NeighbourAvailability;

const MI_SIZE: usize = 4;
/// `Cfl_Filters_420[filterIdx][dy + 1][dx + 1]` (§ 7.13.5): the luma taps
/// of a 4:2:0 CfL sample, `dy` and `dx` in `-1..=1`, summing to `8`.
pub const CFL_FILTERS_420: [[[i32; 3]; 3]; 3] = [
    [[0, 0, 0], [0, 2, 2], [0, 2, 2]],
    [[0, 0, 0], [1, 2, 1], [1, 2, 1]],
    [[0, 1, 0], [1, 4, 1], [0, 1, 0]],
];
/// `Cfl_Filters_422[filterIdx][dx + 1]` (§ 7.13.5): the luma taps of a 4:2:2
/// CfL sample, summing to `8`.
pub const CFL_FILTERS_422: [[i32; 3]; 3] = [[0, 4, 4], [2, 4, 2], [0, 8, 0]];
const CFL_ALPHA_SHIFT: u32 = 11;
const CFL_ALPHA_SCALE: i32 = 32;
const CFL_DERIVED_ALPHA_SHIFT: u8 = 8;
const NUM_REF_SAM_CFL: usize = 8;
/// The widest CfL block axis.
const MAX_CFL_AXIS: usize = 64;
/// [`CflLuma`] filter selector for weights only known at run time.
const RUNTIME_WEIGHTS: usize = 3;

/// One chroma plane of a CfL block: its id, coefficients and neighbours.
type CflPlane<'c> = (PlaneId, CoeffBlock<'c>, NeighbourAvailability);

/// The chroma transform block every plane of a CfL block shares.
#[derive(Clone, Copy)]
struct CflBlock {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    size: IntraRectBlockSize,
    qindex: u32,
    sb_mib: usize,
    bit_depth: BitDepth,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cfl_block_into<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let geometry = CflBlock::new(x, y, log2_width, log2_height, qindex, sb_mib, bit_depth)?;
    reconstruct_cfl_planes(
        scratch,
        workspace,
        &[(plane_id, block, neighbours)],
        geometry,
        cfl_params,
        cfl_ds_filter_index,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cfl_pair_into<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    u_block: CoeffBlock<'_>,
    v_block: CoeffBlock<'_>,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    u_neighbours: NeighbourAvailability,
    v_neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let geometry = CflBlock::new(x, y, log2_width, log2_height, qindex, sb_mib, bit_depth)?;
    let u = (PlaneId::U, u_block, u_neighbours);
    let v = (PlaneId::V, v_block, v_neighbours);
    if matches!(cfl_params, CflParams::Multi { .. }) && u_neighbours != v_neighbours {
        for plane in [u, v] {
            reconstruct_cfl_planes(
                scratch,
                workspace,
                &[plane],
                geometry,
                cfl_params,
                cfl_ds_filter_index,
            )?;
        }
        return Ok(());
    }
    reconstruct_cfl_planes(
        scratch,
        workspace,
        &[u, v],
        geometry,
        cfl_params,
        cfl_ds_filter_index,
    )
}

impl CflBlock {
    fn new(
        x: usize,
        y: usize,
        log2_width: u32,
        log2_height: u32,
        qindex: u32,
        sb_mib: usize,
        bit_depth: BitDepth,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        let size = IntraRectBlockSize::new(
            u8::try_from(log2_width).unwrap_or(u8::MAX),
            u8::try_from(log2_height).unwrap_or(u8::MAX),
        )?;
        Ok(Self {
            x,
            y,
            width: size.width(),
            height: size.height(),
            size,
            qindex,
            sb_mib,
            bit_depth,
        })
    }
}

/// The per-plane outcome of a block's analysis: a § 7.13.5 CfL `(alpha, dc)`
/// or a § 7.13.6 MHCCP predictor.
#[derive(Clone, Copy)]
enum CflModel {
    Linear {
        alpha_q3: i32,
        dc: i32,
        average_q3: i32,
    },
    Multi(MhccpPredictor),
}

/// Reconstructs `planes` (one, or U then V with the same geometry): the
/// luma side is analysed once for all planes before any plane is written.
fn reconstruct_cfl_planes<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    planes: &[CflPlane<'_>],
    block: CflBlock,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let multi = match cfl_params {
        CflParams::Multi { direction } => Some(direction),
        _ => None,
    };
    let Some(filter) = cfl_filter_index(cfl_ds_filter_index) else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: if multi.is_some() {
                "MHCCP filter"
            } else {
                "CfL filter"
            },
        });
    };
    let crate::pipeline::general_intra::GeneralIntraReconScratch {
        cfl_luma_ac,
        cfl_prediction,
        mhccp_refs,
        ..
    } = scratch;
    let mut models = [CflModel::Linear {
        alpha_q3: 0,
        dc: 0,
        average_q3: 0,
    }; 2];
    let Some(models) = models.get_mut(..planes.len()) else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL plane count",
        });
    };
    let mut mhccp_geometry = None;
    {
        let luma_plane = workspace.plane(PlaneId::Y)?;
        let specialised = if workspace.info().pixel_format() == PixelFormat::Yuv420 {
            filter
        } else {
            RUNTIME_WEIGHTS
        };
        macro_rules! analyse {
            ($samples:expr, $filter:literal) => {
                analyse_cfl_block(
                    workspace,
                    &CflLuma::<_, $filter>::new(workspace, $samples, filter)?,
                    planes,
                    block,
                    cfl_params,
                    multi,
                    cfl_luma_ac,
                    mhccp_refs,
                    models,
                    &mut mhccp_geometry,
                )?
            };
            ($samples:expr) => {
                match specialised {
                    0 => analyse!($samples, 0),
                    1 => analyse!($samples, 1),
                    2 => analyse!($samples, 2),
                    _ => analyse!($samples, 3),
                }
            };
        }
        if let Some(samples) = T::u8_slice(luma_plane.samples()) {
            analyse!(samples);
        } else if let Some(samples) = T::u16_slice(luma_plane.samples()) {
            analyse!(samples);
        }
    }
    let max = i32::from(block.bit_depth.max_sample());
    let (width, height) = (block.width, block.height);
    for (&(plane_id, coeffs, _), &model) in planes.iter().zip(models.iter()) {
        let predict = |out: &mut [T], stride: usize| match model {
            CflModel::Linear {
                alpha_q3,
                dc,
                average_q3,
            } => {
                let Some(luma) = cfl_luma_ac.get(..width * height) else {
                    return Err(GeneralIntraResidualError::InvalidReconstructionState {
                        context: "CfL luma length",
                    });
                };
                let model = [dc, alpha_q3, average_q3, max];
                cfl_predict_block(model, luma, width, out, stride)
            }
            CflModel::Multi(predictor) => {
                let Some((geometry, direction)) = mhccp_geometry else {
                    return Err(GeneralIntraResidualError::InvalidReconstructionState {
                        context: "MHCCP references",
                    });
                };
                let refs = geometry.refs(&mhccp_refs[0]);
                if predictor.predict_block(&refs, direction, width, height, out, stride) {
                    Ok(())
                } else {
                    Err(GeneralIntraResidualError::InvalidReconstructionState {
                        context: "MHCCP reference geometry",
                    })
                }
            }
        };
        if predict_intra_in_frame(
            workspace,
            coeffs,
            plane_id,
            block.x,
            block.y,
            block.size,
            block.qindex,
            false,
            None,
            None,
            block.bit_depth,
            predict,
        )? {
            continue;
        }
        cfl_prediction.clear();
        cfl_prediction.resize(width * height, T::default());
        predict(cfl_prediction.as_mut_slice(), width)?;
        commit_intra_prediction(
            workspace,
            coeffs,
            cfl_prediction,
            plane_id,
            block.x,
            block.y,
            block.size,
            block.qindex,
            false,
            None,
            None,
            block.bit_depth,
        )?;
    }
    Ok(())
}

/// Builds every plane's model of a block from the luma and chroma
/// neighbours, plus the shared luma AC (CfL) or luma references (MHCCP).
#[allow(clippy::too_many_arguments)]
fn analyse_cfl_block<T: ReconSample, P: CflLaneSample, const F: usize>(
    workspace: &CurrentFrameWorkspace<T>,
    luma: &CflLuma<'_, P, F>,
    planes: &[CflPlane<'_>],
    block: CflBlock,
    cfl_params: CflParams,
    multi: Option<CflMultiDirection>,
    luma_ac: &mut Vec<i32>,
    mhccp_refs: &mut [Vec<u16>; 3],
    models: &mut [CflModel],
    mhccp_geometry: &mut Option<(MhccpGeometry, u8)>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let Some(&(_, _, neighbours)) = planes.first() else {
        return Ok(());
    };
    if let Some(direction) = multi {
        let mh_dir = direction.value();
        let geometry = MhccpGeometry::new(workspace, planes[0].0, block, neighbours)?;
        let [luma_refs, chroma_refs @ ..] = mhccp_refs;
        baseline_simd!(simd => geometry.gather_luma(simd, luma, block, mh_dir, luma_refs, luma_ac))?;
        for (&(plane_id, _, _), refs) in planes.iter().zip(chroma_refs.iter_mut()) {
            geometry.gather_chroma(workspace, plane_id, block, refs)?;
        }
        let refs = geometry.refs(luma_refs);
        let [u, v] = &*chroma_refs;
        let multi = |params| CflModel::Multi(MhccpPredictor::new(params, block.bit_depth));
        if let [u_model, v_model] = models {
            let [u_params, v_params] =
                derive_mhccp_params(&refs, [u.as_slice(), v.as_slice()], mh_dir, block.bit_depth);
            (*u_model, *v_model) = (multi(u_params), multi(v_params));
        } else {
            let [params] = derive_mhccp_params(&refs, [u.as_slice()], mh_dir, block.bit_depth);
            models.fill(multi(params));
        }
        *mhccp_geometry = Some((geometry, mh_dir));
        return Ok(());
    }
    let mut edges = CflLumaEdges::default();
    let average_q3 = baseline_simd!(simd => {
        edges.gather_above(simd, luma, planes, block)?;
        let left = (block.x > 0 && planes.iter().any(|plane| plane.2.has_left()))
            .then_some(&mut edges.left[..block.height]);
        luma.fill_luma(simd, block, left, luma_ac)?;
        edges.average_q3(block, neighbours)
    })?;
    for (&(plane_id, _, neighbours), model) in planes.iter().zip(models.iter_mut()) {
        let chroma = PlaneSamples::of(workspace, plane_id)?;
        let (dc, implicit_alpha) = edges.chroma_model(
            &chroma,
            block,
            neighbours,
            cfl_params == CflParams::DerivedAlpha,
        )?;
        let alpha_q3 = match cfl_params {
            CflParams::Explicit { alpha_u, alpha_v } => {
                let alpha = match plane_id {
                    PlaneId::U => alpha_u,
                    PlaneId::V => alpha_v,
                    PlaneId::Y => 0,
                };
                i32::from(alpha) * CFL_ALPHA_SCALE
            }
            CflParams::DerivedAlpha => implicit_alpha,
            CflParams::Multi { .. } => 0,
        };
        *model = CflModel::Linear {
            alpha_q3,
            dc,
            average_q3,
        };
    }
    Ok(())
}

/// Writes the § 7.13.5 CfL prediction `[dc, alpha, lumaAvg, max]` of a
/// `width`-wide block of luma `L` into `out`, row `r` at `out[r * stride..]`.
fn cfl_predict_block<T: ReconSample>(
    model: [i32; 4],
    luma: &[i32],
    width: usize,
    out: &mut [T],
    stride: usize,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let height = luma.len() / width.max(1);
    if width == 0 || (height.max(1) - 1) * stride + width > out.len() {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL prediction target",
        });
    }
    #[cfg(target_arch = "x86_64")]
    if cfl_alpha_fits_i16(model[1])
        && luma.len().is_multiple_of(16)
        && let Some(token) = fearless_simd::Level::baseline().as_avx2()
    {
        if let Some(out) = T::u16_slice_mut(out) {
            cfl_predict_u16(token, model, luma, width, out, stride);
            return Ok(());
        }
        if model[3] == i32::from(u8::MAX)
            && let Some(out) = T::u8_slice_mut(out)
        {
            cfl_predict_u8(token, model, luma, width, out, stride);
            return Ok(());
        }
    }
    if let Some(out) = T::u16_slice_mut(out) {
        baseline_simd!(simd => cfl_predict_rows(simd, model, luma, width, out, stride));
    } else if model[3] <= i32::from(u8::MAX)
        && let Some(out) = T::u8_slice_mut(out)
    {
        baseline_simd!(simd => cfl_predict_rows(simd, model, luma, width, out, stride));
    } else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL sample storage",
        });
    }
    Ok(())
}

/// One § 7.13.5 CfL sample: `Clip1(dc + Round2Signed(alpha * (L - lumaAvg), 11))`.
#[inline]
fn cfl_predict([dc, alpha_q3, average_q3, max]: [i32; 4], luma: i32) -> i32 {
    (dc + round2_signed_i32(alpha_q3 * (luma - average_q3), CFL_ALPHA_SHIFT)).clamp(0, max)
}

/// [`cfl_predict`] over the rows of `luma` in 8-lane groups.
/// `Round2Signed` rounds the magnitude, so each lane rounds `|alpha * L|` and
/// restores the sign.
#[inline]
fn cfl_predict_rows<S: Simd, P: CflLaneSample>(
    simd: S,
    model: [i32; 4],
    luma: &[i32],
    width: usize,
    out: &mut [P],
    stride: usize,
) {
    const LANES: usize = 8;
    let [dc, alpha_q3, average_q3, max] = model;
    let zero = i32x8::splat(simd, 0);
    let predict = |luma: &[i32]| {
        let scaled = (i32x8::from_slice(simd, luma) - average_q3) * alpha_q3;
        let rounded = (scaled.abs() + (1 << (CFL_ALPHA_SHIFT - 1))) >> CFL_ALPHA_SHIFT;
        let signed = scaled.simd_lt(zero).select(zero - rounded, rounded);
        (signed + dc).max(0).min(max)
    };
    if width == LANES / 2 && luma.len().is_multiple_of(LANES) {
        for (pair, luma) in luma.chunks_exact(LANES).enumerate() {
            let mut rows = [P::from_clipped(0); LANES];
            P::store8(simd, &mut rows, predict(luma));
            let start = 2 * pair * stride;
            out[start..start + width].copy_from_slice(&rows[..width]); // splot-copy-ok: publish a 4-wide CfL row
            out[start + stride..start + stride + width].copy_from_slice(&rows[width..]); // splot-copy-ok: publish a 4-wide CfL row
        }
        return;
    }
    for (row, luma) in luma.chunks_exact(width).enumerate() {
        let out = &mut out[row * stride..row * stride + width];
        let mut groups = out.chunks_exact_mut(LANES);
        let mut lumas = luma.chunks_exact(LANES);
        for (out, luma) in groups.by_ref().zip(lumas.by_ref()) {
            P::store8(simd, out, predict(luma));
        }
        for (slot, &luma) in groups.into_remainder().iter_mut().zip(lumas.remainder()) {
            *slot = P::from_clipped(cfl_predict(model, luma));
        }
    }
}

/// The § 7.13.5 downsampled luma above and left of a CfL block (`t`, in
/// Q3), each sample gathered once for `lumaAvg` and `implicitAlpha`.
struct CflLumaEdges {
    above: [i32; MAX_CFL_AXIS],
    left: [i32; MAX_CFL_AXIS],
}

impl Default for CflLumaEdges {
    fn default() -> Self {
        Self {
            above: [0; MAX_CFL_AXIS],
            left: [0; MAX_CFL_AXIS],
        }
    }
}

impl CflLumaEdges {
    /// Gathers the above edge when any plane in `planes` reads it; the left
    /// edge comes with the block's luma from [`CflLuma::fill_luma`].
    fn gather_above<S: Simd, P: CflLaneSample, const F: usize>(
        &mut self,
        simd: S,
        luma: &CflLuma<'_, P, F>,
        planes: &[CflPlane<'_>],
        block: CflBlock,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let CflBlock { x, y, width, .. } = block;
        if width > MAX_CFL_AXIS || block.height > MAX_CFL_AXIS {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL block size",
            });
        }
        if y > 0 && planes.iter().any(|plane| plane.2.has_above()) {
            let min_luma_ref_y = cfl_above_min_luma_ref_y(y, block.sb_mib, luma.pixel_format);
            let above = &mut self.above[..width];
            let below_sb_top = min_luma_ref_y.is_none_or(|min_y| min_y <= 2 * y as isize - 3);
            if !(below_sb_top
                && luma.block_luma_420(x, y - 1, width, [false, false, true], &mut None, above))
            {
                let rows = luma.row(y - 1, false, min_luma_ref_y)?;
                luma.q3_run(simd, &rows, x, true, above);
            }
        }
        Ok(())
    }

    /// § 7.13.5 `lumaAvg`.
    fn average_q3(
        &self,
        block: CflBlock,
        neighbours: NeighbourAvailability,
    ) -> core::result::Result<i32, GeneralIntraResidualError> {
        let step_w = if block.width > 32 { 2 } else { 1 };
        let step_h = if block.height > 32 { 2 } else { 1 };
        let mut sum = 0u32;
        let mut count = 0u32;
        if neighbours.has_above() && block.y > 0 {
            for &q3 in self.above[..block.width].iter().step_by(step_w) {
                sum = sum.saturating_add(q3 as u32);
                count += 1;
            }
        }
        if neighbours.has_left() && block.x > 0 {
            for &q3 in self.left[..block.height].iter().step_by(step_h) {
                sum = sum.saturating_add(q3 as u32);
                count += 1;
            }
        }
        let bits = block.bit_depth.bits();
        if count == 0 {
            return Ok(i32::from(8u16 << (bits - 1)));
        }
        let max = (8u16 << bits).saturating_sub(1);
        Ok(i32::from(approx_divide(sum, count)?.min(max)))
    }

    /// The § 7.13.5 DC prediction a CfL plane scales its luma around and,
    /// when `derive`, its `implicitAlpha`, from one read of its edges.
    fn chroma_model<T: ReconSample>(
        &self,
        chroma: &PlaneSamples<'_, T>,
        block: CflBlock,
        neighbours: NeighbourAvailability,
        derive: bool,
    ) -> core::result::Result<(i32, i32), GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            ..
        } = block;
        let have_above = neighbours.has_above() && y > 0;
        let have_left = neighbours.has_left() && x > 0;
        let (max_x, max_y) = (chroma.width - 1, chroma.height - 1);
        let above = if have_above {
            chroma.row_or_err(y - 1)?
        } else {
            &[]
        };
        let above_at = |col: usize| i32::from(above[(x + col).min(max_x)].to_u16());
        let left = if have_left {
            let rows = height.min(chroma.height - y.min(max_y));
            chroma.column(x - 1, y, rows)?
        } else {
            (&[][..], 0, 0)
        };
        let left_at = |row: usize| {
            let (samples, stride, rows) = left;
            i32::from(samples[row.min(rows.saturating_sub(1)) * stride].to_u16())
        };
        let left_sum = have_left.then(|| every(height, dc_step(height), &left_at));
        let above_sum = have_above.then(|| every(width, dc_step(width), &above_at));
        let dc: T = splot_recon::predict_intra_dc_value_from_edge_sums(
            block.bit_depth,
            block.size,
            left_sum.map(|sum| sum as u32),
            above_sum.map(|sum| sum as u32),
        )?;
        let dc = i32::from(dc.to_u16());
        if !derive {
            return Ok((dc, 0));
        }
        let (num_above, num_left) = if neighbours.has_above() && neighbours.has_left() {
            if width > height.saturating_mul(2) {
                (NUM_REF_SAM_CFL, 0)
            } else if height > width.saturating_mul(2) {
                (0, NUM_REF_SAM_CFL)
            } else {
                (NUM_REF_SAM_CFL >> 1, NUM_REF_SAM_CFL >> 1)
            }
        } else {
            (
                if neighbours.has_above() {
                    NUM_REF_SAM_CFL
                } else {
                    0
                },
                if neighbours.has_left() {
                    NUM_REF_SAM_CFL
                } else {
                    0
                },
            )
        };
        let (num_above, num_left) = (num_above.min(width), num_left.min(height));
        if num_above == 0 && num_left == 0 {
            return Ok((dc, 0));
        }
        if (num_above > 0 && !have_above) || (num_left > 0 && !have_left) {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL implicit alpha edge",
            });
        }
        let mut sums = [0i32; 5];
        add_alpha_samples(&mut sums, width, num_above, &self.above, above_at);
        add_alpha_samples(&mut sums, height, num_left, &self.left, left_at);
        let [sum_x, sum_y, sum_xy, sum_xx, count] = sums;
        let der = sum_xx - (sum_x * sum_x) / count;
        let nor = sum_xy - (sum_x * sum_y) / count;
        if der == 0 || nor == 0 {
            return Ok((dc, 0));
        }
        Ok((
            dc,
            i32::from(resolve_division(nor, der, CFL_DERIVED_ALPHA_SHIFT)),
        ))
    }
}

/// Adds the § 7.13.5 `implicitAlpha` sums of `num` evenly spaced samples of
/// an edge `len` long: `luma_q3` and the chroma `chroma_at` beside it.
#[inline]
fn add_alpha_samples(
    sums: &mut [i32; 5],
    len: usize,
    num: usize,
    luma_q3: &[i32],
    chroma_at: impl Fn(usize) -> i32,
) {
    if num == 0 {
        return;
    }
    let step = (len / num).max(1);
    let mut at = if step == 1 { 0 } else { step >> 1 };
    while at < len {
        let (luma, chroma) = (luma_q3[at] >> 3, chroma_at(at));
        for (sum, term) in sums
            .iter_mut()
            .zip([luma, chroma, luma * chroma, luma * luma, 1])
        {
            *sum += term;
        }
        at += step;
    }
}

/// The § 7.13.2.10 edge sampling step: every second sample past 32.
const fn dc_step(len: usize) -> usize {
    if len > 32 { 2 } else { 1 }
}

/// The sum of `at(i)` over `i` below `len` in steps of `step`.
#[inline]
fn every(len: usize, step: usize, at: &impl Fn(usize) -> i32) -> i32 {
    let (mut total, mut i) = (0, 0);
    while i < len {
        total += at(i);
        i += step;
    }
    total
}

fn cfl_above_min_luma_ref_y(
    chroma_y: usize,
    sb_mib: usize,
    pixel_format: PixelFormat,
) -> Option<isize> {
    let luma_y = chroma_y << pixel_format.subsampling_y();
    let luma_mi_row = luma_y / MI_SIZE;
    let sb_height_luma = sb_mib.saturating_mul(MI_SIZE);
    let sb_start_luma_y = if sb_mib == 0 {
        0
    } else {
        luma_mi_row
            .checked_div(sb_mib)
            .map_or(0, |sb_row| sb_row.saturating_mul(sb_height_luma))
    };
    isize::try_from(sb_start_luma_y)
        .ok()
        .and_then(|sb_y| sb_y.checked_sub(1))
}

/// Where a block's § 7.13.6 `CflRef` buffers sit around it. Both chroma
/// planes of a block with the same neighbours share it.
#[derive(Clone, Copy)]
struct MhccpGeometry {
    ref_width: usize,
    ref_height: usize,
    reference_width: usize,
    reference_height: usize,
    above: usize,
    left: usize,
    min_chroma_ref_y: usize,
    min_luma_ref_y: Option<isize>,
}

impl MhccpGeometry {
    fn new<T: ReconSample>(
        workspace: &CurrentFrameWorkspace<T>,
        plane_id: PlaneId,
        block: CflBlock,
        neighbours: NeighbourAvailability,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            sb_mib,
            ..
        } = block;
        let pixel_format = workspace.info().pixel_format();
        let sub_x = usize::from(pixel_format.subsampling_x());
        let sub_y = usize::from(pixel_format.subsampling_y());
        let have_above = neighbours.has_above();
        let have_left = neighbours.has_left();
        let above = if have_above { y.min(2) } else { 0 };
        let left = if have_left { x.min(2) } else { 0 };
        let luma_mi_row = (y << sub_y) / MI_SIZE;
        let sb_start_luma_y = if sb_mib == 0 {
            0
        } else {
            luma_mi_row.checked_div(sb_mib).map_or(0, |sb_row| {
                sb_row.saturating_mul(sb_mib.saturating_mul(MI_SIZE))
            })
        };
        let extra_right = if have_above && width << sub_x > MI_SIZE {
            neighbours
                .num_above_right()
                .saturating_mul(MI_SIZE)
                .min(width)
        } else {
            0
        };
        let extra_bottom = if have_left && height << sub_y > MI_SIZE {
            neighbours
                .num_below_left()
                .saturating_mul(MI_SIZE)
                .min(height)
        } else {
            0
        };
        let chroma_size = workspace.plane(plane_id)?.storage_size();
        let frame_right = chroma_size.width().saturating_sub(x);
        let frame_bottom = chroma_size.height().saturating_sub(y);
        if frame_right == 0 || frame_bottom == 0 {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "MHCCP reference geometry",
            });
        }
        let reference_width = (left + width + extra_right)
            .min(128 >> sub_x)
            .min(left + frame_right);
        let reference_height = (above + height + extra_bottom)
            .min(128 >> sub_y)
            .min(above + frame_bottom);
        Ok(Self {
            ref_width: reference_width.max(left + width),
            ref_height: reference_height.max(above + height),
            reference_width,
            reference_height,
            above,
            left,
            min_chroma_ref_y: (sb_start_luma_y >> sub_y).saturating_sub(1),
            min_luma_ref_y: isize::try_from(sb_start_luma_y)
                .ok()
                .and_then(|sb_y| sb_y.checked_sub(1)),
        })
    }

    fn refs<'a>(&self, luma: &'a [u16]) -> MhccpRefs<'a> {
        MhccpRefs {
            width: self.ref_width,
            height: self.ref_height,
            reference_width: self.reference_width,
            reference_height: self.reference_height,
            above: self.above,
            left: self.left,
            luma,
        }
    }

    /// Captures the § 7.13.6 luma `CflRef[0]`. § 7.13.7 reads neither row 0
    /// nor column 0 of the edge as a centre: luma only as `v[0]` of
    /// direction 1 (row 0) or 2 (column 0), or as block samples when the
    /// edge is absent.
    fn gather_luma<S: Simd, P: CflLaneSample, const F: usize>(
        &self,
        simd: S,
        luma_plane: &CflLuma<'_, P, F>,
        block: CflBlock,
        mh_dir: u8,
        luma: &mut Vec<u16>,
        scratch: &mut Vec<i32>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let (above, left, reference_width) = (self.above, self.left, self.reference_width);
        luma.clear();
        luma.resize(self.ref_width * self.ref_height, 0);
        let first_luma_row = usize::from(above > 0 && mh_dir != 1);
        let first_luma_col = usize::from(left > 0 && mh_dir != 2);
        let block_cols = left + block.width - first_luma_col;
        let block_rows = above..above + block.height;
        scratch.clear();
        scratch.resize(block_cols * block.height, 0);
        let in_block = block_rows.end <= self.reference_height
            && left + block.width <= reference_width
            && luma_plane.block_luma_420(
                block.x - left + first_luma_col,
                block.y,
                block_cols,
                [false, above == 0, first_luma_col == 0],
                &mut None,
                scratch,
            );
        if in_block {
            for (row, q3) in block_rows.clone().zip(scratch.chunks_exact(block_cols)) {
                let start = row * self.ref_width + first_luma_col;
                for (dst, &sample) in luma[start..start + block_cols].iter_mut().zip(q3) {
                    *dst = (sample >> 3) as u16;
                }
            }
        }
        let mut q3 = [0i32; 128];
        for row in first_luma_row..self.reference_height {
            if in_block && block_rows.contains(&row) {
                continue;
            }
            let luma_cols = if row < above {
                reference_width
            } else if row < above + block.height {
                (left + block.width).min(reference_width)
            } else {
                left.min(reference_width)
            };
            if luma_cols <= first_luma_col {
                continue;
            }
            let rows = luma_plane.row(block.y + row - above, row == 0, self.min_luma_ref_y)?;
            let q3 = &mut q3[first_luma_col..luma_cols];
            luma_plane.q3_run(
                simd,
                &rows,
                block.x - left + first_luma_col,
                first_luma_col == 0,
                q3,
            );
            let start = row * self.ref_width + first_luma_col;
            for (dst, &sample) in luma[start..].iter_mut().zip(q3.iter()) {
                *dst = (sample >> 3) as u16;
            }
        }
        Ok(())
    }

    /// Captures the § 7.13.6 chroma `CflRef[1]` of `plane_id`: only the edge
    /// samples § 7.13.7 reads, so the rest of `chroma` is left as it was.
    fn gather_chroma<T: ReconSample>(
        &self,
        workspace: &CurrentFrameWorkspace<T>,
        plane_id: PlaneId,
        block: CflBlock,
        chroma: &mut Vec<u16>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let samples = self.ref_width * self.ref_height;
        if chroma.len() < samples {
            chroma.resize(samples, 0);
        }
        let plane = PlaneSamples::of(workspace, plane_id)?;
        let last_y = plane.height - 1;
        for row in 1..self.above.min(self.reference_height) {
            if self.reference_width < 2 {
                break;
            }
            let ref_chroma_y = (block.y + row - self.above).max(self.min_chroma_ref_y);
            let source_row = plane.row_or_err(ref_chroma_y.min(last_y))?;
            let fill = source_row[source_row.len() - 1].to_u16();
            let start = row * self.ref_width;
            let run = &mut chroma[start + 1..start + self.reference_width];
            let source = source_row
                .get(block.x + 1 - self.left..)
                .unwrap_or_default();
            let copied = source.len().min(run.len());
            for (slot, sample) in run.iter_mut().zip(&source[..copied]) {
                *slot = sample.to_u16();
            }
            run[copied..].fill(fill);
        }
        let first = self.above.max(1);
        if self.left.min(self.reference_width) >= 2 && first < self.reference_height {
            let start_y = (block.y + first - self.above).min(last_y);
            let end_y = (block.y + self.reference_height - 1 - self.above).min(last_y);
            let (column, stride, _) = plane.column(block.x - 1, start_y, end_y - start_y + 1)?;
            for row in first..self.reference_height {
                let y = (block.y + row - self.above).min(last_y) - start_y;
                chroma[row * self.ref_width + 1] = column[y * stride].to_u16();
            }
        }
        Ok(())
    }
}

/// One plane of the current frame, resolved once for many row reads.
struct PlaneSamples<'a, T> {
    samples: &'a [T],
    origin_y: usize,
    stored_rows: usize,
    width: usize,
    stride: usize,
    height: usize,
}

impl<'a, T> PlaneSamples<'a, T> {
    fn new<W: ReconSample>(
        workspace: &'a CurrentFrameWorkspace<W>,
        samples: &'a [T],
        plane_id: PlaneId,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        let plane = workspace.plane(plane_id)?;
        let size = plane.storage_size();
        let stride = plane.stride_samples().max(1);
        if size.width() == 0 || size.height() == 0 {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL plane size",
            });
        }
        Ok(Self {
            samples,
            origin_y: plane.origin_y(),
            stored_rows: samples.len() / stride,
            width: size.width(),
            stride,
            height: size.height(),
        })
    }

    /// Stored row `y`, which must be inside the plane.
    #[inline]
    fn row_or_err(&self, y: usize) -> core::result::Result<&'a [T], GeneralIntraResidualError> {
        const UNSTORED: GeneralIntraResidualError =
            GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL reference row",
            };
        let Some(local) = y.checked_sub(self.origin_y) else {
            return Err(UNSTORED);
        };
        if y >= self.height || local >= self.stored_rows {
            return Err(UNSTORED);
        }
        let start = local * self.stride;
        match self.samples.get(start..start + self.width) {
            Some(row) => Ok(row),
            None => Err(UNSTORED),
        }
    }

    /// Column `x` of the `rows` stored rows from `y`: the samples from
    /// `(x, y)` on, the row stride, and `rows`, which must be at least one.
    #[inline]
    fn column(
        &self,
        x: usize,
        y: usize,
        rows: usize,
    ) -> core::result::Result<(&'a [T], usize, usize), GeneralIntraResidualError> {
        const UNSTORED: GeneralIntraResidualError =
            GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL reference column",
            };
        let Some(local) = y.checked_sub(self.origin_y) else {
            return Err(UNSTORED);
        };
        if rows == 0 || x >= self.width || y + rows > self.height || local + rows > self.stored_rows
        {
            return Err(UNSTORED);
        }
        let start = local * self.stride + x;
        match self
            .samples
            .get(start..start + (rows - 1) * self.stride + 1)
        {
            Some(column) => Ok((column, self.stride, rows)),
            None => Err(UNSTORED),
        }
    }
}

impl<'a, T: ReconSample> PlaneSamples<'a, T> {
    fn of(
        workspace: &'a CurrentFrameWorkspace<T>,
        plane_id: PlaneId,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        Self::new(workspace, workspace.plane(plane_id)?.samples(), plane_id)
    }
}

/// The luma plane of one CfL block and its § 7.13.5 downsampling filter:
/// the 4:2:0 filter `F`, or [`RUNTIME_WEIGHTS`] for any format.
struct CflLuma<'a, P, const F: usize> {
    plane: PlaneSamples<'a, P>,
    pixel_format: PixelFormat,
    sub_x: usize,
    sub_y: usize,
    /// `weights[dy + 1][dx + 1]`, zero off the subsampled axes.
    weights: [[i32; 3]; 3],
}

impl<'a, P: CflLaneSample, const F: usize> CflLuma<'a, P, F> {
    fn new<T: ReconSample>(
        workspace: &'a CurrentFrameWorkspace<T>,
        samples: &'a [P],
        filter: usize,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        let pixel_format = workspace.info().pixel_format();
        let sub_x = usize::from(pixel_format.subsampling_x());
        let sub_y = usize::from(pixel_format.subsampling_y());
        let weights = match (sub_x, sub_y) {
            (1, 1) => CFL_FILTERS_420[filter],
            (1, _) => [[0; 3], CFL_FILTERS_422[filter], [0; 3]],
            _ => [[0; 3], [0, 8, 0], [0; 3]],
        };
        if F != RUNTIME_WEIGHTS && (sub_x, sub_y, F) != (1, 1, filter) {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL filter specialisation",
            });
        }
        Ok(Self {
            plane: PlaneSamples::new(workspace, samples, PlaneId::Y)?,
            pixel_format,
            sub_x,
            sub_y,
            weights,
        })
    }

    /// The taps, constant for a 4:2:0 filter.
    #[inline]
    fn weights(&self) -> [[i32; 3]; 3] {
        if F < RUNTIME_WEIGHTS {
            CFL_FILTERS_420[F]
        } else {
            self.weights
        }
    }

    /// The luma rows the taps of chroma row `chroma_y` read; a row with no
    /// tap aliases the middle row.
    #[inline]
    fn row(
        &self,
        chroma_y: usize,
        clamp_y: bool,
        min_luma_ref_y: Option<isize>,
    ) -> core::result::Result<[&'a [P]; 3], GeneralIntraResidualError> {
        let max_y = self.plane.height - 1;
        let luma_y = clamped_cfl_luma_coordinate(chroma_y, self.sub_y, max_y) as isize;
        let tap_row = |dy: isize| {
            let mut sy = luma_y + if clamp_y { dy.max(0) } else { dy };
            if let Some(min_y) = min_luma_ref_y {
                sy = sy.max(min_y);
            }
            self.plane.row_or_err(sy.clamp(0, max_y as isize) as usize)
        };
        let middle = tap_row(0)?;
        let weights = self.weights();
        let edge = |dy: isize, weights: [i32; 3]| {
            if weights == [0; 3] {
                Ok(middle)
            } else {
                tap_row(dy)
            }
        };
        Ok([edge(-1, weights[0])?, middle, edge(1, weights[2])?])
    }

    /// The § 7.13.5 downsampled luma sample at `chroma_x` of `rows`, in Q3.
    #[inline]
    fn q3(&self, rows: &[&[P]; 3], chroma_x: usize, clamp_x: bool) -> i32 {
        let max_x = self.plane.width - 1;
        let luma_x = clamped_cfl_luma_coordinate(chroma_x, self.sub_x, max_x);
        let columns = [
            if clamp_x {
                luma_x
            } else {
                luma_x.saturating_sub(1)
            },
            luma_x,
            (luma_x + 1).min(max_x),
        ];
        let mut total = 0;
        for (row, weights) in rows.iter().zip(&self.weights()) {
            for (&column, &weight) in columns.iter().zip(weights) {
                if weight != 0 {
                    total += weight * i32::from(row[column].into());
                }
            }
        }
        total
    }

    /// `out[i] = q3(chroma_x0 + i)`, where only the first sample
    /// clamps, when `clamp_first`.
    #[inline]
    fn q3_run<S: Simd>(
        &self,
        simd: S,
        rows: &[&[P]; 3],
        chroma_x0: usize,
        clamp_first: bool,
        out: &mut [i32],
    ) {
        let done = self.q3_lanes(simd, rows, chroma_x0, out);
        for (i, slot) in out.iter_mut().enumerate().skip(done) {
            *slot = self.q3(rows, chroma_x0 + i, clamp_first && i == 0);
        }
        if clamp_first && done > 0 {
            out[0] = self.q3(rows, chroma_x0, true);
        }
    }

    /// The unclamped 8-lane groups of [`Self::q3_run`] from its start while
    /// every tap is inside the row; returns how many samples they wrote.
    #[inline]
    fn q3_lanes<S: Simd>(
        &self,
        simd: S,
        rows: &[&[P]; 3],
        chroma_x0: usize,
        out: &mut [i32],
    ) -> usize {
        const LANES: usize = 8;
        if self.sub_x != 1 || chroma_x0 == 0 {
            return 0;
        }
        let weights = self.weights();
        let mut done = 0;
        while done < out.len() && 2 * (chroma_x0 + done) + 2 * LANES < self.plane.width {
            let luma_x = 2 * (chroma_x0 + done);
            let acc = cfl_420_row_taps(simd, rows[0], luma_x, weights[0])
                + cfl_420_row_taps(simd, rows[1], luma_x, weights[1])
                + cfl_420_row_taps(simd, rows[2], luma_x, weights[2]);
            let take = (out.len() - done).min(LANES);
            if take == LANES {
                acc.store_slice(&mut out[done..done + LANES]);
            } else {
                let lanes: [i32; LANES] = acc.into();
                out[done..done + take].copy_from_slice(&lanes[..take]); // splot-copy-ok: publish CfL luma samples
            }
            done += take;
        }
        done
    }

    /// The 8-bit 4:2:0 downsampled luma of the chroma rectangle at `(x, y)`,
    /// `width` wide and `out.len() / width` tall, in one AVX2 pass when every
    /// tap lies in the stored plane: `[periodic, clamp_row0, clamp_col0]` as
    /// in [`Luma420Block`]. `false` leaves `out` and `left` unwritten.
    #[inline]
    fn block_luma_420(
        &self,
        x: usize,
        y: usize,
        width: usize,
        clamps: [bool; 3],
        left: &mut Option<&mut [i32]>,
        out: &mut [i32],
    ) -> bool {
        let height = out.len() / width.max(1);
        #[cfg(target_arch = "x86_64")]
        if F < RUNTIME_WEIGHTS
            && width > 0
            && height > 0
            && let Some(token) = fearless_simd::Level::baseline().as_avx2()
            && let Some(samples) = P::as_u8(self.plane.samples)
            && 2 * (x + width) <= self.plane.width
            && 2 * (y + height) <= self.plane.height
            && let Some(local_y) = (2 * y).checked_sub(self.plane.origin_y)
        {
            let [periodic, clamp_row0, clamp_col0] = clamps;
            let block = Luma420Block {
                origin: local_y * self.plane.stride + 2 * x,
                stride: self.plane.stride,
                luma_x: 2 * x,
                luma_y: 2 * y,
                width,
                periodic,
                clamp_row0,
                clamp_col0,
            };
            if block.fits(height, samples.len()) {
                return luma_420::<F>(token, samples, block, left.as_deref_mut(), out);
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = (x, y, height, clamps, left, out);
        false
    }

    /// The § 7.13.5 `L` of a block into `luma`, and the downsampled luma left
    /// of each block row into `left` when given.
    fn fill_luma<S: Simd>(
        &self,
        simd: S,
        block: CflBlock,
        mut left: Option<&mut [i32]>,
        luma: &mut Vec<i32>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            ..
        } = block;
        luma.clear();
        luma.resize(width * height, 0);
        if self.block_luma_420(x, y, width, [true; 3], &mut left, luma) {
            return Ok(());
        }
        let max_x = self.plane.width - 1;
        let max_y = self.plane.height - 1;
        let edge = max_x >> self.sub_x;
        let period = 64 >> self.sub_x;
        let edge_clamps = (edge << self.sub_x).is_multiple_of(64);
        let top_taps = self.weights()[0] != [0; 3];
        for (row, out) in luma.chunks_exact_mut(width).enumerate() {
            let chroma_y = y + row;
            let luma_y = clamped_cfl_luma_coordinate(chroma_y, self.sub_y, max_y);
            let clamp_y = row == 0 || luma_y.is_multiple_of(64);
            let rows = self.row(chroma_y, clamp_y, None)?;
            if let Some(left) = left.as_deref_mut() {
                left[row] = if clamp_y && row > 0 && top_taps {
                    self.q3(&self.row(chroma_y, false, None)?, x - 1, false)
                } else {
                    self.q3(&rows, x - 1, false)
                };
            }
            self.q3_run(simd, &rows, x, true, out);
            let mut chroma_x = (x / period + 1) * period;
            while chroma_x < x + width && chroma_x <= edge {
                out[chroma_x - x] = self.q3(&rows, chroma_x, true);
                chroma_x += period;
            }
            if edge_clamps {
                for chroma_x in (edge + 1).max(x + 1)..x + width {
                    out[chroma_x - x] = self.q3(&rows, chroma_x, true);
                }
            }
        }
        Ok(())
    }
}

/// A luma sample type the CfL lane forms load as `u16` lanes.
trait CflLaneSample: Copy + Into<u16> {
    /// Loads `row[start..start + 16]`.
    fn load16<S: Simd>(simd: S, row: &[Self], start: usize) -> u16x16<S>;
    /// Stores eight samples already clipped to the sample range.
    fn store8<S: Simd>(simd: S, out: &mut [Self], v: i32x8<S>);
    fn from_clipped(v: i32) -> Self;
    /// `samples` as bytes, for 8-bit planes.
    fn as_u8(samples: &[Self]) -> Option<&[u8]>;
}

impl CflLaneSample for u8 {
    #[inline]
    fn load16<S: Simd>(simd: S, row: &[Self], start: usize) -> u16x16<S> {
        let (low, high) = simd.widen_u8x16(fearless_simd::u8x16::from_slice(
            simd,
            &row[start..start + 16],
        ));
        simd.combine_u16x8(low, high)
    }

    #[inline]
    fn store8<S: Simd>(simd: S, out: &mut [Self], v: i32x8<S>) {
        let (low, high) = simd.split_u32x8(v.bitcast());
        let words = simd.relaxed_narrow_u32x4(low, high);
        let bytes: [u8; 16] = simd.relaxed_narrow_u16x8(words, words).into();
        out[..8].copy_from_slice(&bytes[..8]); // splot-copy-ok: publish eight CfL samples
    }

    #[inline]
    fn from_clipped(v: i32) -> Self {
        v as u8
    }

    #[inline]
    fn as_u8(samples: &[Self]) -> Option<&[u8]> {
        Some(samples)
    }
}

impl CflLaneSample for u16 {
    #[inline]
    fn load16<S: Simd>(simd: S, row: &[Self], start: usize) -> u16x16<S> {
        u16x16::from_slice(simd, &row[start..start + 16])
    }

    #[inline]
    fn store8<S: Simd>(simd: S, out: &mut [Self], v: i32x8<S>) {
        let (low, high) = simd.split_u32x8(v.bitcast());
        simd.relaxed_narrow_u32x4(low, high)
            .store_slice(&mut out[..8]);
    }

    #[inline]
    fn from_clipped(v: i32) -> Self {
        v as u16
    }

    #[inline]
    fn as_u8(_samples: &[Self]) -> Option<&[u8]> {
        None
    }
}

/// One luma row's left, centre and right taps of eight § 7.13.5 4:2:0 Q3
/// samples from luma column `luma_x` (at least 1, with 17 more samples in the
/// row). Each `u32` lane of the row read from `luma_x - 1` holds the left and
/// centre taps, and one from `luma_x` holds the centre and right taps.
#[allow(clippy::inline_always, reason = "measured: folds the constant weights")]
#[inline(always)]
fn cfl_420_row_taps<S: Simd, P: CflLaneSample>(
    simd: S,
    row: &[P],
    luma_x: usize,
    weights: [i32; 3],
) -> i32x8<S> {
    let pairs = |start: usize| {
        let pairs = P::load16(simd, row, start).bitcast::<u32x8<S>>();
        (
            (pairs & 0xffff).bitcast::<i32x8<S>>(),
            (pairs >> 16).bitcast::<i32x8<S>>(),
        )
    };
    let mut acc = i32x8::splat(simd, 0);
    if weights[1] != 0 || weights[2] != 0 {
        let (center, right) = pairs(luma_x);
        acc = center * weights[1] + right * weights[2];
    }
    if weights[0] != 0 {
        acc += pairs(luma_x - 1).0 * weights[0];
    }
    acc
}

fn clamped_cfl_luma_coordinate(
    chroma_coordinate: usize,
    subsampling: usize,
    max_luma_coordinate: usize,
) -> usize {
    chroma_coordinate.min(max_luma_coordinate >> subsampling) << subsampling
}

const fn cfl_filter_index(cfl_ds_filter_index: u8) -> Option<usize> {
    match cfl_ds_filter_index {
        0 | 3 => Some(0),
        1 => Some(1),
        2 => Some(2),
        _ => None,
    }
}

#[cfg(test)]
#[path = "cfl_tests.rs"]
mod tests;
