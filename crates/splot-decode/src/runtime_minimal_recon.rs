// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal traced reconstruction handoff for the documented runtime tier.
//!
//! Feature tracking: `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER`.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, IntraCardinalDirection,
    IntraDirectionalAngle, IntraDirectionalAngleEdges, IntraDirectionalAngleIdifEdges,
    IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraMiddleDirectionalAngleIdifEdges, IntraMiddleDirectionalAngleIdifMrlEdges, IntraPaethEdges,
    IntraRectBlockSize, IntraSmoothEdges, IntraSmoothMode, IntraSquareBlockSize, OutputIndex,
    PixelFormat, PlaneId, PlaneRect, PlaneSize, ReconSample, apply_ibp_dr_blend_rect,
    apply_intra_edge_filter, apply_intra_ibp_dc_rect, filter_intra_edge_corner,
    predict_intra_cardinal_directional_rect_into, predict_intra_dc_rect_value,
    predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_into,
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into,
    predict_intra_middle_directional_angle_rect_idif_into,
    predict_intra_middle_directional_angle_rect_idif_mrl_into,
    predict_intra_middle_directional_angle_rect_into, predict_intra_paeth_rect_into,
    predict_intra_smooth_rect_into,
};

use crate::Result;
use crate::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, LumaPalette, LumaTransformTypeContext,
    MinimalRuntimeReconstructionTrace, SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
    reconstruct_general_intra_block, reconstruct_general_intra_block_rect_with_prediction,
    reconstruct_general_intra_block_rect_with_prediction_and_ddt,
    reconstruct_general_intra_block_with_prediction,
    reconstruct_general_intra_luma_block_rect_with_prediction_and_ist,
};

mod cfl;
mod chroma_directional;
mod mhccp;

pub(crate) use cfl::reconstruct_general_intra_chroma_cfl_block_into;
pub(crate) use chroma_directional::{
    reconstruct_general_intra_chroma_block_into,
    reconstruct_general_intra_chroma_smooth_available_edges_into,
};
pub(crate) use mhccp::{
    MHCCP_BITS, MHCCP_PARAM_COUNT, MhccpRefs, derive_mhccp_params, mul_fixed32_adapt,
};

const MINIMAL_LUMA_WIDTH: usize = 64;
const MINIMAL_LUMA_HEIGHT: usize = 64;
const MINIMAL_CHROMA_WIDTH: usize = 32;
const MINIMAL_CHROMA_HEIGHT: usize = 32;
const MINIMAL_LUMA_LOG2_SIZE: u8 = 6;
const MINIMAL_CHROMA_LOG2_SIZE: u8 = 5;
const TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE: u8 = 129;

/// Reconstructs the current traced minimal runtime frame.
pub(crate) fn reconstruct_minimal_traced_frame(
    trace: MinimalRuntimeReconstructionTrace,
) -> Result<DecodedFrame<u8>> {
    match trace {
        MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64 => {
            reconstruct_luma_dc_chroma_h_pred_8bit420_64x64()
        }
    }
}

fn reconstruct_luma_dc_chroma_h_pred_8bit420_64x64() -> Result<DecodedFrame<u8>> {
    let luma_size = PlaneSize::new(MINIMAL_LUMA_WIDTH, MINIMAL_LUMA_HEIGHT)?;
    let luma_rect = PlaneRect::new(0, 0, MINIMAL_LUMA_WIDTH, MINIMAL_LUMA_HEIGHT)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        luma_rect,
    )?;

    let mut workspace = CurrentFrameWorkspace::<u8>::new(info, 0)?;
    let luma_block = IntraSquareBlockSize::new(MINIMAL_LUMA_LOG2_SIZE)?;
    workspace.predict_intra_dc_square(PlaneId::Y, 0, 0, luma_block)?;

    let chroma_block = IntraRectBlockSize::new(MINIMAL_CHROMA_LOG2_SIZE, MINIMAL_CHROMA_LOG2_SIZE)?;
    let chroma_left = [TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE; MINIMAL_CHROMA_HEIGHT];
    let mut chroma_prediction = [0u8; MINIMAL_CHROMA_WIDTH * MINIMAL_CHROMA_HEIGHT];
    predict_intra_cardinal_directional_rect_into(
        BitDepth::Eight,
        chroma_block,
        IntraCardinalDirection::Horizontal,
        IntraDirectionalAngleEdges::left(&chroma_left),
        &mut chroma_prediction,
        MINIMAL_CHROMA_WIDTH,
    )?;
    workspace.write_rect_block(PlaneId::U, 0, 0, chroma_block, &chroma_prediction)?;
    workspace.write_rect_block(PlaneId::V, 0, 0, chroma_block, &chroma_prediction)?;

    Ok(workspace.freeze()?)
}

/// Creates an empty decoded 4:2:0 frame workspace sized to the actual
/// `luma_width` x `luma_height` (a positive multiple of 64) for incremental
/// per-block reconstruction on the general intra multi-block path. Chroma is
/// 4:2:0 (half-resolution), so the chroma plane is `luma_width / 2` x
/// `luma_height / 2`, derived internally by [`PixelFormat::Yuv420`]. The sample
/// storage type `T` matches the active sequence `bit_depth` (§ 6.4.1): `u8` for
/// 8-bit, `u16` for 10-bit.
pub(crate) fn new_general_intra_workspace<T: ReconSample>(
    luma_width: usize,
    luma_height: usize,
    bit_depth: BitDepth,
) -> Result<CurrentFrameWorkspace<T>> {
    let luma_size = PlaneSize::new(luma_width, luma_height)?;
    let luma_rect = PlaneRect::new(0, 0, luma_width, luma_height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        PixelFormat::Yuv420,
        luma_size,
        luma_rect,
    )?;
    Ok(CurrentFrameWorkspace::<T>::new(info, T::default())?)
}

/// Reconstructs one square plane block in decode order into the workspace: the
/// § 7.13.2 DC prediction is read from the partially-built frame's neighbours
/// (`128` fallback when none); an `all_zero` block writes the flat prediction,
/// otherwise the dequant / inverse-transform / residual-add reconstruction is
/// added; the result is written back so later blocks read it as a neighbour.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    ibp_dc: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let dc = predict_intra_dc_rect_value(bit_depth, block_size, edges.as_dc_edges())?;
    let mut prediction = vec![dc; side * side];
    if ibp_dc {
        apply_intra_ibp_dc_rect(
            bit_depth,
            block_size,
            edges.as_dc_edges(),
            &mut prediction,
            side,
        )?;
    }
    let out = if block.all_zero {
        prediction
    } else if ibp_dc {
        reconstruct_general_intra_block_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_side,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    } else {
        reconstruct_general_intra_block(
            &block.quant,
            dc,
            qindex,
            plane_id,
            log2_side,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Reconstructs one **rectangular** DC_PRED plane block in decode order into the
/// workspace, the rectangular generalisation of [`reconstruct_general_intra_block_into`].
///
/// `log2_width` and `log2_height` are the block's §7.15.4 transform dimensions
/// and may differ (e.g. a 64x32 `TX_64X32` luma block has `log2_width == 6`,
/// `log2_height == 5`; its 4:2:0 chroma is a 32x16 `TX_32X16` block). The §7.13.2
/// DC prediction is read from the partially-built frame's neighbours over the
/// rectangular block (`intra_dc_edges_for_rect` / `predict_intra_dc_rect_value`
/// already accept a rectangular [`IntraRectBlockSize`]); an `all_zero` block
/// writes the flat prediction, otherwise the §7.14.4 / §7.15.4 / §7.14.3
/// dequant + inverse-transform + residual reconstruction is added (with the
/// §7.15.4.1 √2 rescale when the log2 ratio is odd). The result is written back
/// so later blocks read it as a neighbour. Chroma never uses the §7.14.4 TCQ
/// `dqDenom` term (luma DCT_DCT only).
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_rect_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    ibp_dc: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let dc = predict_intra_dc_rect_value(bit_depth, block_size, edges.as_dc_edges())?;
    let prediction = if ibp_dc {
        let mut pred = vec![dc; width * height];
        apply_intra_ibp_dc_rect(bit_depth, block_size, edges.as_dc_edges(), &mut pred, width)?;
        pred
    } else {
        vec![dc; width * height]
    };
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_dc_rect_block_with_ist_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    ibp_dc: bool,
    bit_depth: BitDepth,
    luma_context: LumaTransformTypeContext,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(PlaneId::Y, x, y, block_size)?;
    let dc = predict_intra_dc_rect_value(bit_depth, block_size, edges.as_dc_edges())?;
    let prediction = if ibp_dc {
        let mut pred = vec![dc; width * height];
        apply_intra_ibp_dc_rect(bit_depth, block_size, edges.as_dc_edges(), &mut pred, width)?;
        pred
    } else {
        vec![dc; width * height]
    };
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_palette_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    palette: LumaPalette,
    color_map: &[u8],
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    if color_map.len() != width.saturating_mul(height) {
        return Err(GeneralIntraResidualError::PredictionLength {
            expected: width.saturating_mul(height),
            actual: color_map.len(),
        });
    }
    let mut prediction = Vec::with_capacity(color_map.len());
    for &color_index in color_map {
        let sample =
            palette
                .sample(color_index)
                .ok_or(GeneralIntraResidualError::PaletteColorIndex {
                    color_index: usize::from(color_index),
                    palette_size: palette.size(),
                })?;
        prediction.push(T::try_from_u16(sample)?);
    }
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

/// Adds a decoded § 5.20.7.27 inter residual onto the § 7.13.3.18
/// motion-compensated prediction already written into the workspace plane.
///
/// `workspace` already holds the inter predictor for the whole plane (the
/// `motion_compensate_inter_block` output frozen into a workspace, or written
/// per-plane). This reads the predicted samples of the square block at `(x, y)`
/// (side `1 << log2_side`), composes the § 7.14.4 dequantization, § 7.15.4
/// inverse transform, and § 7.14.3 residual addition over them (via
/// [`reconstruct_general_intra_block_with_prediction`], which is the §7.14.3
/// reconstruction over an arbitrary per-sample prediction — identical for inter
/// and intra), then writes the reconstructed block back. An `all_zero` block
/// leaves the prediction untouched (the residual is zero), so this is a no-op
/// for the skipped-transform case. `qindex == base_q_idx` for the minimal-tool
/// frame; `use_tcq` adds the § 7.14.4 TCQ `dqDenom` term (luma DCT_DCT only).
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_inter_block_residual_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if block.all_zero {
        return Ok(());
    }
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let rect = PlaneRect::new(x, y, side, side)?;
    let mut prediction = Vec::with_capacity(side * side);
    for row in workspace.rect_rows(plane_id, rect)? {
        prediction.extend_from_slice(row);
    }
    let out = reconstruct_general_intra_block_rect_with_prediction_and_ddt(
        &block.quant,
        &prediction,
        qindex,
        plane_id,
        log2_side,
        log2_side,
        block.plane_tx_type,
        use_tcq,
        use_ddt,
        bit_depth,
    )?;
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Adds one rectangular inter residual transform onto an existing
/// motion-compensated prediction block.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_inter_block_residual_rect_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if log2_width == log2_height {
        return reconstruct_inter_block_residual_into(
            workspace, block, plane_id, x, y, log2_width, qindex, use_tcq, use_ddt, bit_depth,
        );
    }
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    if block.all_zero {
        return Ok(());
    }
    let rect = PlaneRect::new(x, y, width, height)?;
    let mut prediction = Vec::with_capacity(width * height);
    for row in workspace.rect_rows(plane_id, rect)? {
        prediction.extend_from_slice(row);
    }
    let out = reconstruct_general_intra_block_rect_with_prediction_and_ddt(
        &block.quant,
        &prediction,
        qindex,
        plane_id,
        log2_width,
        log2_height,
        block.plane_tx_type,
        use_tcq,
        use_ddt,
        bit_depth,
    )?;
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Adds a decoded §5.20.7.27 IntrABC residual onto the §7.13.3.18 displaced-copy
/// prediction already written into the workspace plane, the **rectangular**
/// generalisation of [`reconstruct_inter_block_residual_into`].
///
/// An IntrABC leaf's predictor is the displaced `CurrFrame` copy
/// ([`crate::runtime_minimal::wienerns_lr::WienerNsLrReconSink::reconstruct_intrabc_block`]
/// wrote it into `workspace` over the whole block before this per-transform leaf
/// runs), NOT a §7.13.2 intra prediction — IntrABC reads no intra Y mode. This
/// reads the predicted samples of the `1<<log2_width` x `1<<log2_height` transform
/// at `(x, y)`, composes the §7.14.4 dequantization, §7.15.4 inverse transform, and
/// §7.14.3 residual addition over them (via
/// [`reconstruct_general_intra_block_rect_with_prediction`] — the §7.14.3
/// reconstruction over an arbitrary per-sample prediction, identical for the
/// displaced IntrABC predictor), then writes the reconstructed block back. An
/// `all_zero` transform leaves the copied predictor untouched (the residual is
/// zero). `use_tcq` adds the §7.14.4 TCQ `dqDenom` term (luma `DCT_DCT` only);
/// IntrABC is an inter (`is_inter == 1`) leaf, so it never carries it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_intrabc_block_residual_rect_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    if block.all_zero {
        return Ok(());
    }
    let rect = PlaneRect::new(x, y, width, height)?;
    let mut prediction = Vec::with_capacity(width * height);
    for row in workspace.rect_rows(plane_id, rect)? {
        prediction.extend_from_slice(row);
    }
    let out = reconstruct_general_intra_block_rect_with_prediction(
        &block.quant,
        &prediction,
        qindex,
        plane_id,
        log2_width,
        log2_height,
        block.plane_tx_type,
        use_tcq,
        bit_depth,
    )?;
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Reconstructs one non-DC luma block whose mode is `SMOOTH_V_PRED` /
/// `SMOOTH_H_PRED` (§ 7.13.2.13) over the § 7.13.2.1 edges read from the
/// partially-built frame's **real reconstructed neighbours**, adds the decoded
/// residual (or writes the bare prediction for an `all_zero` block), and stores
/// the result so later blocks read it as a neighbour.
///
/// Unlike [`reconstruct_general_intra_luma_nondc_first_block_into`] (which is
/// gated to the no-neighbour top-left block and uses the § 7.13.2.1 flat
/// fallbacks), this reads the genuine reconstructed left column / above row of an
/// already-decoded neighbour at **any** superblock position in the 2-D grid.
/// First superblock row (a left neighbour but no above neighbour): § 7.13.2.1
/// sets `LeftCol[i]` to the reconstructed left column (clamping the bottom-left
/// sentinel `LeftCol[h]` to the last in-block sample, since `num4BelowLeft == 0`
/// in raster order) and `AboveRow[i]` to the repeated first left sample
/// (`CurrFrame[plane][y][x-1]`). Row > 0 (an above neighbour, and a left
/// neighbour when not at the frame left edge): § 7.13.2.1 sets `AboveRow[i]` to
/// the **real reconstructed above row** (`CurrFrame[plane][y-1][...]`) and, when
/// an already-decoded above-right block is in frame (`num4AboveRight > 0`,
/// supplied by the caller), the top-right sentinel `AboveRow[w]` to the real
/// reconstructed above-right sample. For `SMOOTH_V_PRED` the § 7.13.2.13 output
/// depends only on `AboveRow[j]` and the bottom-left sentinel `LeftCol[h]`; for
/// `SMOOTH_H_PRED` it depends only on `LeftCol[i]` and the top-right sentinel
/// `AboveRow[w]`. No directional / IDIF edge synthesis is involved (smooth
/// prediction is linear interpolation, not an angle copy), so this path is
/// bit-exact against the AVM/dav2d oracle for the verified subset.
///
/// `num4_above_right` is the § 7.13.2.1 `num4AboveRight` (in 4x4 units) for this
/// luma transform block; it selects the § 7.13.2.13 top-right sentinel
/// `AboveRow[w]` between the real reconstructed above-right sample and the clamped
/// last in-block above sample (only material for `SMOOTH_H_PRED` / `SMOOTH_PRED`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_nondc_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedNonDcLumaMode,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    num4_above_right: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let smooth_mode = match mode {
        SupportedNonDcLumaMode::Smooth => IntraSmoothMode::Smooth,
        SupportedNonDcLumaMode::SmoothVertical => IntraSmoothMode::SmoothVertical,
        SupportedNonDcLumaMode::SmoothHorizontal => IntraSmoothMode::SmoothHorizontal,
    };
    reconstruct_general_intra_smooth_over_edges_into(
        workspace,
        block,
        PlaneId::Y,
        x,
        y,
        log2_side,
        log2_side,
        qindex,
        smooth_mode,
        num4_above_right,
        0,
        use_tcq,
        None,
        bit_depth,
    )
}

/// Reconstructs one rectangular luma smooth block over § 7.13.2.1 edges.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_smooth_rect_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedNonDcLumaMode,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    num4_above_right: usize,
    num4_below_left: usize,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let smooth_mode = match mode {
        SupportedNonDcLumaMode::Smooth => IntraSmoothMode::Smooth,
        SupportedNonDcLumaMode::SmoothVertical => IntraSmoothMode::SmoothVertical,
        SupportedNonDcLumaMode::SmoothHorizontal => IntraSmoothMode::SmoothHorizontal,
    };
    reconstruct_general_intra_smooth_over_edges_into(
        workspace,
        block,
        PlaneId::Y,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        smooth_mode,
        num4_above_right,
        num4_below_left,
        use_tcq,
        luma_context,
        bit_depth,
    )
}

/// Reconstructs one § 7.13.2.13 smooth block (`SMOOTH_PRED` / `SMOOTH_V_PRED` /
/// `SMOOTH_H_PRED`) over § 7.13.2.1 edges read from the partially-built frame's
/// reconstructed neighbours, for any plane. The § 7.13.2.1 edge derivation is
/// plane-independent (it reads the workspace neighbour samples and applies the
/// no-above / no-left / no-neighbour fallbacks); the caller selects the smooth
/// mode and whether luma TCQ dequant applies.
#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_smooth_over_edges_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    smooth_mode: IntraSmoothMode,
    num4_above_right: usize,
    num4_below_left: usize,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    reconstruct_general_intra_smooth_over_available_edges_into(
        workspace,
        block,
        plane_id,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        smooth_mode,
        None,
        None,
        num4_above_right,
        num4_below_left,
        use_tcq,
        luma_context,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_smooth_over_available_edges_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    smooth_mode: IntraSmoothMode,
    available_left_samples: Option<usize>,
    available_above_samples: Option<usize>,
    num4_above_right: usize,
    num4_below_left: usize,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let left_len =
        available_left_samples.unwrap_or_else(|| edges.left_samples().map_or(0, <[T]>::len));
    let above_len =
        available_above_samples.unwrap_or_else(|| edges.above_samples().map_or(0, <[T]>::len));
    let left = edges
        .left_samples()
        .map(|samples| &samples[..left_len.min(samples.len())]);
    let above = edges
        .above_samples()
        .map(|samples| &samples[..above_len.min(samples.len())]);
    let have_left = left.is_some_and(|samples| !samples.is_empty());
    let have_above = above.is_some_and(|samples| !samples.is_empty());
    let above_right_sentinel = if above_len >= width {
        resolve_smooth_above_right_sentinel(
            workspace,
            plane_id,
            x,
            y,
            width,
            have_above,
            num4_above_right,
        )?
    } else {
        None
    };
    let bottom_left_sentinel = if left_len >= height {
        resolve_smooth_bottom_left_sentinel(
            workspace,
            plane_id,
            x,
            y,
            height,
            have_left,
            num4_below_left,
        )?
    } else {
        None
    };
    let (left, above) = build_smooth_edges(
        left,
        above,
        have_left,
        have_above,
        width,
        height,
        above_right_sentinel,
        bottom_left_sentinel,
        bit_depth,
    );
    let smooth_edges = IntraSmoothEdges::new(&left, &above);
    let mut prediction = vec![T::default(); width * height];
    predict_intra_smooth_rect_into(
        bit_depth,
        block_size,
        smooth_mode,
        smooth_edges,
        &mut prediction,
        width,
    )?;
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Builds the AV2 § 7.13.2.1 `LeftCol[0..=side]` and `AboveRow[0..=side]` edges
/// (8-bit, `MrlIndex == 0`, no DIP) for § 7.13.2.13 smooth prediction (luma or
/// chroma — the edge derivation is plane-independent), from the reconstructed
/// left/above neighbours. The `[side]` entries are the smooth-process bottom-left
/// / top-right sentinels.
///
/// `above_right_sentinel` is the caller-resolved § 7.13.2.1 top-right sentinel
/// `AboveRow[w]` (the real reconstructed above-right sample when decoded, or
/// `None` to keep the clamped last in-block above sample / no-above fallback).
/// `bottom_left_sentinel` is the symmetric `LeftCol[h]`.
#[allow(clippy::too_many_arguments)]
fn build_smooth_edges<T: ReconSample>(
    left_neighbour: Option<&[T]>,
    above_neighbour: Option<&[T]>,
    have_left: bool,
    have_above: bool,
    width: usize,
    height: usize,
    above_right_sentinel: Option<T>,
    bottom_left_sentinel: Option<T>,
    bit_depth: BitDepth,
) -> (Vec<T>, Vec<T>) {
    let left_len = height + 1;
    let above_len = width + 1;
    let left = match (have_left, left_neighbour) {
        (true, Some(samples)) => fill_edge_from_neighbour(samples, left_len, bit_depth),
        _ if have_above => {
            let seed = above_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(noneighbour_left::<T>(bit_depth));
            vec![seed; left_len]
        }
        _ => vec![noneighbour_left::<T>(bit_depth); left_len],
    };
    let mut above = match (have_above, above_neighbour) {
        (true, Some(samples)) => fill_edge_from_neighbour(samples, above_len, bit_depth),
        _ if have_left => {
            let seed = left_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(noneighbour_above::<T>(bit_depth));
            vec![seed; above_len]
        }
        _ => vec![noneighbour_above::<T>(bit_depth); above_len],
    };
    if let Some(sentinel) = above_right_sentinel
        && let Some(slot) = above.get_mut(width)
    {
        *slot = sentinel;
    }
    let mut left = left;
    if let Some(sentinel) = bottom_left_sentinel
        && let Some(slot) = left.get_mut(height)
    {
        *slot = sentinel;
    }
    (left, above)
}

/// Resolves the AV2 § 7.13.2.1 top-right sentinel `AboveRow[w]` for a SMOOTH
/// block (luma or chroma — the derivation is plane-independent) in this
/// single-tile minimal path.
///
/// Per § 7.13.2.1, when `haveAbove == 1` the sentinel is
/// `CurrFrame[plane][y - 1][Min(aboveLimit, x + w)]` with
/// `aboveLimit = Min(maxX, x + w + 4 * num4AboveRight - 1)` (8-bit, `MrlIndex == 0`,
/// `aboveMrlIndex == 0`). When `num4AboveRight == 0` (no decoded above-right) or
/// the block already touches the plane's frame right edge (`x + w > maxX`), this
/// reduces to the clamped last in-block above sample, so `None` is returned to
/// keep the [`build_smooth_edges`] clamp. When `haveAbove == 0` the
/// sentinel is not read from the above-right at all, so `None` is returned.
///
/// For a luma (`sub_x == 0`) `SMOOTH_H_PRED` full-superblock block at superblock
/// row > 0 this reads the real reconstructed bottom row of the already-decoded
/// diagonally-above-right superblock (the `syn-shgrid` fixture pins it bit-exact).
fn resolve_smooth_above_right_sentinel<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    side: usize,
    have_above: bool,
    num4_above_right: usize,
) -> core::result::Result<Option<T>, GeneralIntraResidualError> {
    resolve_smooth_sentinel(
        workspace,
        SmoothSentinelRequest {
            plane_id,
            x,
            y,
            side,
            have_edge: have_above,
            extension: num4_above_right,
            kind: SmoothSentinelKind::AboveRight,
        },
    )
}

/// Resolves the AV2 § 7.13.2.1 bottom-left sentinel `LeftCol[h]` for a SMOOTH
/// block, symmetric to [`resolve_smooth_above_right_sentinel`].
fn resolve_smooth_bottom_left_sentinel<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    height: usize,
    have_left: bool,
    num4_below_left: usize,
) -> core::result::Result<Option<T>, GeneralIntraResidualError> {
    resolve_smooth_sentinel(
        workspace,
        SmoothSentinelRequest {
            plane_id,
            x,
            y,
            side: height,
            have_edge: have_left,
            extension: num4_below_left,
            kind: SmoothSentinelKind::BottomLeft,
        },
    )
}

#[derive(Clone, Copy)]
enum SmoothSentinelKind {
    AboveRight,
    BottomLeft,
}

#[derive(Clone, Copy)]
struct SmoothSentinelRequest {
    plane_id: PlaneId,
    x: usize,
    y: usize,
    side: usize,
    have_edge: bool,
    extension: usize,
    kind: SmoothSentinelKind,
}

fn resolve_smooth_sentinel<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    request: SmoothSentinelRequest,
) -> core::result::Result<Option<T>, GeneralIntraResidualError> {
    let SmoothSentinelRequest {
        plane_id,
        x,
        y,
        side,
        have_edge,
        extension,
        kind,
    } = request;
    if !have_edge || extension == 0 {
        return Ok(None);
    }
    let storage = workspace.plane(plane_id)?.storage_size();
    match kind {
        SmoothSentinelKind::AboveRight => {
            let Some(max_x) = storage.width().checked_sub(1) else {
                return Ok(None);
            };
            let Some(sample_y) = y.checked_sub(1) else {
                return Ok(None);
            };
            let sample_x = x.saturating_add(side);
            if sample_x > max_x {
                return Ok(None);
            }
            Ok(Some(
                workspace.reconstructed_sample(plane_id, sample_x, sample_y)?,
            ))
        }
        SmoothSentinelKind::BottomLeft => {
            let Some(max_y) = storage.height().checked_sub(1) else {
                return Ok(None);
            };
            let Some(sample_x) = x.checked_sub(1) else {
                return Ok(None);
            };
            let sample_y = y.saturating_add(side);
            if sample_y > max_y {
                return Ok(None);
            }
            Ok(Some(
                workspace.reconstructed_sample(plane_id, sample_x, sample_y)?,
            ))
        }
    }
}

/// Copies `samples` into a length-`edge_len` edge, repeating the last sample to
/// fill the trailing § 7.13.2.13 sentinel slot(s) (§ 7.13.2.1 edge extension).
fn fill_edge_from_neighbour<T: ReconSample>(
    samples: &[T],
    edge_len: usize,
    bit_depth: BitDepth,
) -> Vec<T> {
    let mut edge = Vec::with_capacity(edge_len);
    for i in 0..edge_len {
        let sample = samples
            .get(i)
            .or_else(|| samples.last())
            .copied()
            .unwrap_or(noneighbour_left::<T>(bit_depth));
        edge.push(sample);
    }
    edge
}

/// AV2 § 7.13.2.1 no-neighbour fallback (`haveAbove == 0 && haveLeft == 0`):
/// every `AboveRow` sample is `(1 << (BitDepth - 1)) - 1`, every `LeftCol` sample
/// is `(1 << (BitDepth - 1)) + 1`, and the shared corner `AboveRow[-1] ==
/// LeftCol[-1]` is `1 << (BitDepth - 1)`. For 8-bit these are `127` / `129` /
/// `128`; for 10-bit `511` / `513` / `512`. The values are derived from
/// `bit_depth` and converted into the sample storage type `T` (`T::from(i32)` via
/// the `ReconSample` conversion), so 8-bit storage stays byte-identical.
fn noneighbour_above<T: ReconSample>(bit_depth: BitDepth) -> T {
    let half = 1u16 << (bit_depth.bits() - 1);
    noneighbour_sample::<T>(half - 1)
}

fn noneighbour_left<T: ReconSample>(bit_depth: BitDepth) -> T {
    let half = 1u16 << (bit_depth.bits() - 1);
    noneighbour_sample::<T>(half + 1)
}

fn noneighbour_corner<T: ReconSample>(bit_depth: BitDepth) -> T {
    let half = 1u16 << (bit_depth.bits() - 1);
    noneighbour_sample::<T>(half)
}

/// Converts an in-range § 7.13.2.1 fallback `value` into the sample storage type
/// `T`. The conversion is infallible by the dispatch invariant: `T` is bound to
/// the active `bit_depth` (`u8` <-> `BitDepth::Eight`, `u16` <-> `BitDepth::Ten`,
/// see [`decode_general_minimal_intra_frame`]), so a fallback derived from
/// `bit_depth` always fits `T`. CEILING: if a future change decouples `T` from
/// `bit_depth` (e.g. admits `u8` storage for a 10-bit stream), a `> 255` value
/// would not fit `u8` and `unwrap_or_default()` would silently yield `0`. UPGRADE
/// PATH: thread `Result` through the § 7.13.2.1 edge builders (which already
/// return [`GeneralIntraResidualError`]) and propagate the conversion error. The
/// `debug_assert` fails loud under test / debug builds if the invariant is ever
/// broken, instead of emitting a `0`-valued fallback edge.
fn noneighbour_sample<T: ReconSample>(value: u16) -> T {
    debug_assert!(
        T::try_from_u16(value).is_ok(),
        "§7.13.2.1 no-neighbour fallback {value} does not fit the sample storage type for the active bit depth",
    );
    T::try_from_u16(value).unwrap_or_default()
}

/// Shared tail for the no-neighbour (top-left) luma first-block reconstructors:
/// given the already-built § 7.13.2 prediction, adds the decoded AC residual (or
/// writes the bare prediction for an `all_zero` block) and stores the result into
/// the workspace.
#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_luma_first_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    prediction: Vec<T>,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_side,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

fn luma_square_prediction_geometry(
    log2_side: u32,
) -> core::result::Result<(usize, IntraRectBlockSize), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    Ok((side, IntraRectBlockSize::new(log2, log2)?))
}

/// Reconstructs one no-neighbour (top-left) non-DC luma block: builds the
/// § 7.13.2.13 smooth prediction over the § 7.13.2.1 no-neighbour fallback edges,
/// adds the decoded AC residual (or writes the bare prediction for an all-zero
/// block), and stores the result into the workspace.
///
/// This path is gated to the top-left block (no above/left neighbours), so the
/// edges are pure § 7.13.2.1 fallbacks; multi-block non-DC prediction (which
/// reads reconstructed neighbours) is a future increment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_nondc_first_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedNonDcLumaMode,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (side, block_size) = luma_square_prediction_geometry(log2_side)?;
    let prediction = predict_nondc_noneighbour_smooth(mode, block_size, side, bit_depth)?;
    reconstruct_general_intra_luma_first_block_into(
        workspace, block, prediction, x, y, log2_side, qindex, use_tcq, bit_depth,
    )
}

/// Builds the § 7.13.2.13 smooth prediction for a no-neighbour square block over
/// the § 7.13.2.1 fallback edges (above `127`, left `129`; the smooth sentinels
/// `above[w]` / `left[h]` share those fallbacks).
fn predict_nondc_noneighbour_smooth<T: ReconSample>(
    mode: SupportedNonDcLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
    bit_depth: BitDepth,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let smooth_mode = match mode {
        SupportedNonDcLumaMode::Smooth => IntraSmoothMode::Smooth,
        SupportedNonDcLumaMode::SmoothVertical => IntraSmoothMode::SmoothVertical,
        SupportedNonDcLumaMode::SmoothHorizontal => IntraSmoothMode::SmoothHorizontal,
    };
    let above = vec![noneighbour_above::<T>(bit_depth); side + 1];
    let left = vec![noneighbour_left::<T>(bit_depth); side + 1];
    let edges = IntraSmoothEdges::new(&left, &above);
    let mut out = vec![T::default(); side * side];
    predict_intra_smooth_rect_into(bit_depth, block_size, smooth_mode, edges, &mut out, side)?;
    Ok(out)
}

/// Reconstructs one no-neighbour (top-left) directional-angle luma block:
/// builds the § 7.13.2.8 prediction over the § 7.13.2.1 no-neighbour fallback
/// edges, adds the decoded residual (or writes the bare prediction for an
/// all-zero block), and stores the result into the workspace.
///
/// This path is gated (by the caller) to the top-left no-neighbour block at
/// pAngle 135 (`D135_PRED`, `AngleDeltaY == 0`). For that case
/// `enable_intra_edge_filter == 0`, `MrlIndex == 0`, and no above/left neighbour
/// exist, so the § 7.13.2.x edge-filter / corner-filter / upsample step is a
/// no-op and the prediction edges reduce to the § 7.13.2.1 flat fallbacks:
/// `AboveRow[k] = 127`, `LeftCol[k] = 129`, and the shared corner
/// `AboveRow[-1] = LeftCol[-1] = 128`. pAngle 135 is a § 7.13.2.8 "middle" angle
/// (`90 < pAngle < 180`); its derivatives are `dx = dy = Dr_Intra_Derivative[45]
/// = 64`, so every projection lands on an integer sample (`shift == 0`) and the
/// luma IDIF 4-tap `Dr_Interp_Filter` reduces to a sample copy — bit-identical to
/// the `enableIdif == 0` bilinear `predict_intra_middle_directional_angle_rect_into`
/// for this angle. (Verified bit-exact against avmdec/dav2d.) Other angles, where
/// `shift != 0` and luma IDIF genuinely differs from bilinear, are deferred.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_directional_first_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedDirectionalLumaMode,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (side, block_size) = luma_square_prediction_geometry(log2_side)?;
    let prediction = predict_directional_noneighbour(mode, block_size, side, bit_depth)?;
    reconstruct_general_intra_luma_first_block_into(
        workspace, block, prediction, x, y, log2_side, qindex, use_tcq, bit_depth,
    )
}

/// Builds the § 7.13.2.8 directional prediction for a no-neighbour square block
/// over the § 7.13.2.1 fallback edges. The middle-angle predictor takes logical
/// edges whose index 0 is the `-1` sample: `above_with_minus_one[0]` /
/// `left_with_minus_one[0]` are the shared corner `128`, the remaining above
/// samples are `127` and left samples are `129`.
fn predict_directional_noneighbour<T: ReconSample>(
    mode: SupportedDirectionalLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
    bit_depth: BitDepth,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let angle = middle_directional_angle(mode)?;
    let mut above = vec![noneighbour_above::<T>(bit_depth); side + 1];
    let mut left = vec![noneighbour_left::<T>(bit_depth); side + 1];
    above[0] = noneighbour_corner::<T>(bit_depth);
    left[0] = noneighbour_corner::<T>(bit_depth);
    let edges = IntraMiddleDirectionalAngleEdges::both(&left, &above);
    let mut out = vec![T::default(); side * side];
    predict_intra_middle_directional_angle_rect_into(
        bit_depth, block_size, angle, edges, &mut out, side,
    )?;
    Ok(out)
}

/// Reconstructs one neighbour-having directional-angle block (luma D135 with the
/// § 7.13.2.8 IDIF, or the directional-follow D135 chroma with the bilinear
/// branch) over the § 7.13.2.1 edges read from the partially-built frame's **real
/// reconstructed neighbours**, adds the decoded residual (or writes the bare
/// prediction for an `all_zero` block), and stores the result so later blocks read
/// it as a neighbour.
///
/// This is the neighbour-having companion of
/// [`reconstruct_general_intra_luma_directional_first_block_into`] /
/// [`reconstruct_general_intra_chroma_directional_first_into`] (which are gated to
/// the no-neighbour top-left block and use the § 7.13.2.1 flat fallbacks). It
/// reads the genuine reconstructed left column / above row of an already-decoded
/// neighbour, building the logical `AboveRow[-1..w)` / `LeftCol[-1..h)` edges per
/// § 7.13.2.1.
///
/// pAngle 135 is a § 7.13.2.8 "middle" angle (`90 < pAngle < 180`) whose
/// derivatives are `dx = dy = Dr_Intra_Derivative[45] = 64`, so every projection
/// lands on an integer sample (`shift == 0`). At `shift == 0` the § 7.13.2.8 IDIF
/// 4-tap (`enableIdif == 1` for luma) collapses to `Dr_Interp_Filter[0] =
/// {0, 128, 0, 0}`, i.e. `Clip1(Round2(128 * Edge[base], 7)) = Edge[base]`, which
/// is bit-identical to the `enableIdif == 0` bilinear branch
/// (`Round2(Edge[base] * 32 + Edge[base + 1] * 0, 5) = Edge[base]`) **even over a
/// non-flat reconstructed edge**. So both the luma IDIF and the chroma bilinear
/// branch reduce to the same sample copy `Edge[base]` for D135, and the shared
/// [`predict_intra_middle_directional_angle_rect_into`] (bilinear) is bit-exact
/// for this angle in either plane. (Other angles, where `shift != 0`, genuinely
/// differ between IDIF and bilinear and are deferred.)
///
/// `enable_intra_edge_filter == 0`, `MrlIndex == 0`, and no upsample apply in the
/// minimal-tool subset, so no § 7.13.2.x edge-filter / corner-filter / upsample
/// synthesis runs over the edges. Directional D135 never uses the above-right
/// sentinel value (its above/left reads stay within `AboveRow[0..w)` /
/// `LeftCol[0..h)`), so the above-right resolver is not needed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_directional_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedDirectionalLumaMode,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let have_left = edges.left_samples().is_some();
    let have_above = edges.above_samples().is_some();
    let above_corner = if have_left && have_above {
        match (x.checked_sub(1), y.checked_sub(1)) {
            (Some(cx), Some(cy)) => Some(workspace.reconstructed_sample(plane_id, cx, cy)?),
            _ => None,
        }
    } else {
        None
    };
    let (left, above) = build_directional_middle_edges(
        edges.left_samples(),
        edges.above_samples(),
        above_corner,
        have_left,
        have_above,
        side,
        bit_depth,
    )?;
    let angle = middle_directional_angle(mode)?;
    let mut prediction = vec![T::default(); side * side];
    if matches!(plane_id, PlaneId::Y) {
        let (left_idif, above_idif) =
            extend_directional_middle_idif_edges(&left, &above, bit_depth);
        predict_intra_middle_directional_angle_rect_idif_into(
            bit_depth,
            block_size,
            angle,
            IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
            &mut prediction,
            side,
        )?;
    } else {
        predict_intra_middle_directional_angle_rect_into(
            bit_depth,
            block_size,
            angle,
            IntraMiddleDirectionalAngleEdges::both(&left, &above),
            &mut prediction,
            side,
        )?;
    }
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_side,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_middle_neighbour_rect_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
    filters: TwoSidedMiddleEdgeFilters,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let mut prediction = vec![T::default(); width * height];

    if edges.left_samples().is_some() && edges.above_samples().is_some() {
        let above_row = y
            .checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let left_col = x
            .checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let corner = workspace.reconstructed_sample(plane_id, left_col, above_row)?;
        let above_idif = build_two_sided_middle_idif_edge(width, filters.above, corner, |i| {
            workspace.reconstructed_sample(plane_id, x.saturating_add(i), above_row)
        })?;
        let left_idif = build_two_sided_middle_idif_edge(height, filters.left, corner, |i| {
            workspace.reconstructed_sample(plane_id, left_col, y.saturating_add(i))
        })?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_middle_directional_angle_rect_idif_into(
                bit_depth,
                block_size,
                angle,
                IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
                &mut prediction,
                width,
            )?;
        } else {
            let left = &left_idif[1..height + 2];
            let above = &above_idif[1..width + 2];
            predict_intra_middle_directional_angle_rect_into(
                bit_depth,
                block_size,
                angle,
                IntraMiddleDirectionalAngleEdges::both(left, above),
                &mut prediction,
                width,
            )?;
        }
    } else {
        let (left, above) = build_directional_middle_rect_edges(
            edges.left_samples(),
            edges.above_samples(),
            None,
            edges.left_samples().is_some(),
            edges.above_samples().is_some(),
            width,
            height,
            bit_depth,
        )?;
        if matches!(plane_id, PlaneId::Y) {
            let (left_idif, above_idif) =
                extend_directional_middle_idif_edges(&left, &above, bit_depth);
            predict_intra_middle_directional_angle_rect_idif_into(
                bit_depth,
                block_size,
                angle,
                IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
                &mut prediction,
                width,
            )?;
        } else {
            predict_intra_middle_directional_angle_rect_into(
                bit_depth,
                block_size,
                angle,
                IntraMiddleDirectionalAngleEdges::both(&left, &above),
                &mut prediction,
                width,
            )?;
        }
    }
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Builds the § 7.13.2.1 PAETH reference edges (`AboveRow[0..w)`, `LeftCol[0..h)`,
/// and the shared corner `AboveRow[-1]`) for a block at `(x, y)`, applying the
/// spec's `haveAbove` / `haveLeft` availability fallback.
///
/// `above_in` / `left_in` are the workspace's in-storage edges (`Some` when the
/// block has a decoded above row / left column; `None` only at the `y == 0` /
/// `x == 0` frame edge, which equals `haveAbove == 0` / `haveLeft == 0` for the
/// single-tile decode-order walk). The § 7.13.2.1 reference build is:
/// * above row — real above when `haveAbove`; else `CurrFrame[plane][y][x-1]` (the
///   left column's top sample, `left_in[0]`) when `haveLeft`; else the
///   `(1 << (BitDepth-1)) - 1` no-above constant;
/// * left column — real left when `haveLeft`; else `CurrFrame[plane][y-1][x]` (the
///   above row's first sample, `above_in[0]`) when `haveAbove`; else the
///   `(1 << (BitDepth-1)) + 1` no-left constant;
/// * corner `AboveRow[-1]` — `CurrFrame[plane][y-1][x-1]` when both neighbours
///   exist; else the available edge's first sample (`above_in[0]` / `left_in[0]`);
///   else the `1 << (BitDepth-1)` no-neighbour constant.
///
/// This mirrors AVM `av2_build_intra_predictors_high`
/// (`av2/common/reconintra.c`): `above_row[i] = left_ref[0]` / `left_col[i] =
/// above_ref[0]` for the single-missing edge and `above_row[-1] = left_ref[0]` /
/// `above_ref[0]` for the corner. `MrlIndex == 0` (PAETH is dispatched only for the
/// immediate edge), so `aboveMrlIndex == 0`.
#[allow(clippy::too_many_arguments)]
fn paeth_reference_edges<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    bit_depth: BitDepth,
    above_in: Option<&[T]>,
    left_in: Option<&[T]>,
) -> core::result::Result<(Vec<T>, Vec<T>, T), GeneralIntraResidualError> {
    let above = match (above_in, left_in) {
        (Some(above), _) => above.iter().take(width).copied().collect::<Vec<T>>(),
        (None, Some(left)) => {
            vec![*left.first().unwrap_or(&noneighbour_above::<T>(bit_depth)); width]
        }
        (None, None) => vec![noneighbour_above::<T>(bit_depth); width],
    };
    let left = match (left_in, above_in) {
        (Some(left), _) => left.iter().take(height).copied().collect::<Vec<T>>(),
        (None, Some(above)) => {
            vec![*above.first().unwrap_or(&noneighbour_left::<T>(bit_depth)); height]
        }
        (None, None) => vec![noneighbour_left::<T>(bit_depth); height],
    };
    if above.len() != width || left.len() != height {
        return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
    }
    let top_left = match (above_in, left_in) {
        (Some(_), Some(_)) => {
            let (Some(corner_x), Some(corner_y)) = (x.checked_sub(1), y.checked_sub(1)) else {
                return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
            };
            workspace.reconstructed_sample(plane_id, corner_x, corner_y)?
        }
        (Some(above), None) => *above.first().unwrap_or(&noneighbour_corner::<T>(bit_depth)),
        (None, Some(left)) => *left.first().unwrap_or(&noneighbour_corner::<T>(bit_depth)),
        (None, None) => noneighbour_corner::<T>(bit_depth),
    };
    Ok((above, left, top_left))
}

/// Reconstructs one neighbour-having § 7.13.2.2 PAETH (`PAETH_PRED`,
/// non-directional) luma block over the § 7.13.2.1 above row / left column / corner
/// read from the partially-built frame's **real reconstructed neighbours**. For an
/// `all_zero` leaf this writes the bare § 7.13.2.2 prediction; for a residual-bearing
/// leaf it adds the § 5.20.7.27 decoded residual onto the prediction via the standard
/// § 7.14.3 `Clip1(pred + inverse-transform(residual))` reconstruction (the same
/// `reconstruct_general_intra_block_rect_with_prediction` the directional paths use).
/// PAETH is non-directional, so the residual add is plane-independent of the
/// predictor: no IDIF / edge-filter / above-right synthesis interacts with it.
///
/// § 7.13.2.2 generates `pred[i][j]` from `LeftCol[i]`, `AboveRow[j]`, and the
/// shared corner `AboveRow[-1]` (the Paeth predictor: pick whichever of left /
/// above / corner is closest to `AboveRow[j] + LeftCol[i] - AboveRow[-1]`). It is
/// rectangular (independent `w` x `h`) and needs no IDIF / edge-filter / above-right
/// / bottom-left synthesis.
///
/// The § 7.13.2.1 reference edges (above row, left column, shared corner) are built
/// by [`paeth_reference_edges`], which applies the spec's full `haveAbove` /
/// `haveLeft` fallback: when a frame-edge block has no above row (or no left column)
/// the missing edge is synthesized from the available perpendicular edge's first
/// sample (or the § 7.13.2.1 mid-grey constant when neither neighbour exists), so a
/// top-row / left-column PAETH leaf reconstructs instead of deferring.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_paeth_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let (above, left, top_left) = paeth_reference_edges(
        workspace,
        plane_id,
        x,
        y,
        width,
        height,
        bit_depth,
        edges.above_samples(),
        edges.left_samples(),
    )?;
    let mut prediction = vec![T::default(); width * height];
    predict_intra_paeth_rect_into(
        bit_depth,
        block_size,
        IntraPaethEdges::new(&left, &above, top_left),
        &mut prediction,
        width,
    )?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// The § 7.13.2.7 step-1 edge-filter / corner-filter inputs the caller resolved
/// for a one-sided IDIF leaf (the gate derived them from the neighbour modes and
/// the § 7.13.2.17 strength selection). Threaded into the edge builder so the raw
/// § 7.13.2.1 reference edge is rewritten in place before the § 7.13.2.8
/// prediction reads it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OneSidedEdgeFilter {
    /// § 7.13.2.17 `intra_edge_filter_strength_selection` result (`0..=3`) for the
    /// read edge (`angleAbove` zone-1 / `angleLeft` zone-3, with the neighbour
    /// `filterType`). `0` is a § 7.13.2.18 no-op.
    pub strength: u8,
    /// § 7.13.2.7 `numPx` for the read edge: `Min(w, maxX - x + 1) + (needRight ?
    /// h : 0) + 1` (zone-1) / `Min(h, maxY - y + 1) + (needBottom ? w : 0) + 1`
    /// (zone-3). The § 7.13.2.18 filter runs over the corner + this many samples.
    pub num_px: usize,
    /// `Some(oppositeEdge[0])` when the § 7.13.2.14 corner filter fires (`needAbove
    /// && needLeft && (w + h) >= 24`): the reconstructed OPPOSITE-edge `[0]` sample
    /// the corner blend reads (`LeftCol[0]` for zone-1, `AboveRow[0]` for zone-3).
    /// `None` when the corner filter does not fire (`AboveRow[-1]` / `LeftCol[-1]`
    /// stay the raw § 7.13.2.1 corner).
    pub corner_opposite: Option<u16>,
}

/// The §7.13.2.1 / §5.20.5.5 MULTI-REFERENCE-LINE offsets for a ZONE-1 one-sided
/// above-reading leaf. The §7.13.2.8 projection geometry (`maxBase = w + h - 1 +
/// (mrlIndex << 1)`, `idx = (i + 1 + mrlIndex) * dx`) keys on the full `mrl_index`,
/// but the above ROW is read from `CurrFrame[y - 1 - aboveMrlIndex]` where
/// `aboveMrlIndex == sbBoundary ? 0 : mrlIndex` (AVM `above_ref_1st = ref -
/// ref_stride * (above_mrl_idx + 1)` with `above_mrl_idx = is_sb_boundary ? 0 :
/// mrl_index`). At the immediate edge both are `0`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OneSidedAboveMrl {
    /// §5.20.5.5 `MrlIndex` — the projection-geometry reference-line distance.
    pub mrl_index: usize,
    /// `aboveMrlIndex == sbBoundary ? 0 : MrlIndex` — the above-row read offset.
    pub above_mrl_index: usize,
}

/// Reconstructs one neighbour-having ZONE-1 ONE-SIDED directional luma/chroma
/// block (any `pAngle < 90`, e.g. D45 § 7.13.2.8 step 1) over the § 7.13.2.1 above
/// row PLUS the real reconstructed above-right read from the partially-built
/// frame, adds the decoded residual (or writes the bare prediction for an
/// `all_zero` block), and stores the result so later blocks read it as a
/// neighbour.
///
/// SQUARE OR NON-SQUARE: `log2_width`/`log2_height` are the independent § 5.20.5.3
/// `Tx_Width`/`Tx_Height` log2s. The § 7.13.2.8 projection, the `maxBaseX == w + h
/// - 1` edge, and the `aboveLimit == x + w + 4*num4AboveRight - 1` above-right
/// extent all key on the real `(w, h)`; the IDIF predictor iterates `h` rows x `w`
/// columns. A square block is the `log2_width == log2_height` case.
///
/// Zone-1 (`pAngle < 90`, `needRight`) projects UP-AND-RIGHT into the above-right:
/// `pred[i][j]` reads `AboveRow[base]` with `base = (i + 1 + j)` (D45,
/// `dx = Dr_Intra_Derivative[45] = 64`, shift always `0`), up to
/// `base == maxBaseX == w + h - 1`. So unlike the middle angles (which stay
/// within `AboveRow[0..w)`), this reads `h` real reconstructed above-right
/// samples. § 7.13.2.1 fills `AboveRow[i] = CurrFrame[plane][y - 1][Min(aboveLimit,
/// x + i)]` for `i in 0..w + h`, with `aboveLimit = Min(maxX, x + w +
/// 4 * num4AboveRight - 1)` (8-bit, `MrlIndex == 0`, `aboveMrlIndex == 0`); the
/// `num4_above_right` (in plane 4x4 units, from § 5.20.7.25 `count_top_right_avail`
/// over the § 5.20.2.3 `BlockDecoded` state) bounds how far the real above-right
/// extends before the spec clamps to `CurrFrame[plane][y - 1][aboveLimit]`. The
/// corner `AboveRow[-1] = CurrFrame[plane][y - 1][x - 1]` is read directly.
///
/// The block is gated (by the caller) to a row > 0, non-first-column,
/// non-rightmost full 64x64 superblock (`haveLeft && haveAbove`, a real decoded
/// above-right superblock in frame). `enable_intra_edge_filter == 0` /
/// `MrlIndex == 0` keep the § 7.13.2.7 edge-filter / corner-filter / upsample
/// synthesis a no-op, and `enable_ibp == 0` keeps `useIBP == 0` (§ 7.13.2.7 gates
/// `useIBP` on `pAngle < 90`, so the IBP secondary blend is skipped only when
/// `enable_ibp` is off). Luma uses the § 7.13.2.8 IDIF 4-tap
/// (`enableIdif = plane == 0`); for D45 every `shift == 0`, so the IDIF reduces to
/// the sample copy `AboveRow[base]`, bit-identical to the chroma bilinear branch
/// over the same real reconstructed above-right.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_one_sided_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    num4_above_right: usize,
    mrl: OneSidedAboveMrl,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let prediction = if matches!(plane_id, PlaneId::Y) {
        predict_general_intra_luma_one_sided_above_mrl(
            workspace,
            p_angle,
            x,
            y,
            log2_width,
            log2_height,
            num4_above_right,
            mrl,
            bit_depth,
            edge_filter,
        )?
    } else {
        if mrl.mrl_index != 0 {
            return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
        }
        let above_idif = build_one_sided_above_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            num4_above_right,
            mrl.mrl_index,
            mrl.above_mrl_index,
            edge_filter,
        )?;
        let mut prediction = vec![T::default(); width * height];
        let above_bilinear = &above_idif[2..2 + width + height];
        predict_intra_directional_angle_rect_into(
            bit_depth,
            block_size,
            IntraDirectionalAngle::try_from_p_angle(p_angle)?,
            IntraDirectionalAngleEdges::above(above_bilinear),
            &mut prediction,
            width,
        )?;
        prediction
    };
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_general_intra_luma_one_sided_above_mrl<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    num4_above_right: usize,
    mrl: OneSidedAboveMrl,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let above_idif = build_one_sided_above_idif_edge(
        workspace,
        PlaneId::Y,
        x,
        y,
        width,
        height,
        num4_above_right,
        mrl.mrl_index,
        mrl.above_mrl_index,
        edge_filter,
    )?;
    let mut prediction = vec![T::default(); width * height];
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into(
        bit_depth,
        block_size,
        IntraDirectionalAngle::try_from_p_angle(p_angle)?,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        mrl.mrl_index,
        &mut prediction,
        width,
    )?;
    Ok(prediction)
}

fn average_luma_prediction_with<T: ReconSample>(
    prediction: &mut [T],
    secondary: Vec<T>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    for (primary, secondary) in prediction.iter_mut().zip(secondary) {
        let average = (u32::from(primary.to_u16()) + u32::from(secondary.to_u16()) + 1) >> 1;
        let average = u16::try_from(average)
            .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        *primary = T::try_from_u16(average)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_luma_prediction_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    prediction: Vec<T>,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_mrl_secondary_above_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    num4_above_right: usize,
    primary_mrl: OneSidedAboveMrl,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let mut prediction = predict_general_intra_luma_one_sided_above_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        num4_above_right,
        primary_mrl,
        bit_depth,
        OneSidedEdgeFilter::default(),
    )?;
    let secondary = predict_general_intra_luma_one_sided_above_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        num4_above_right,
        OneSidedAboveMrl::default(),
        bit_depth,
        OneSidedEdgeFilter::default(),
    )?;
    average_luma_prediction_with(&mut prediction, secondary)?;
    write_luma_prediction_block(
        workspace,
        block,
        prediction,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        use_tcq,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_cardinal_mrl_luma_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    direction: IntraCardinalDirection,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    mrl_index: usize,
    above_mrl_index: usize,
    secondary_mrl: bool,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let mut prediction = cardinal_mrl_luma_prediction(
        workspace,
        direction,
        x,
        y,
        width,
        height,
        mrl_index,
        above_mrl_index,
    )?;
    if secondary_mrl {
        let secondary =
            cardinal_mrl_luma_prediction(workspace, direction, x, y, width, height, 0, 0)?;
        average_luma_prediction_with(&mut prediction, secondary)?;
    }
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cardinal_mrl_luma_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    direction: IntraCardinalDirection,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    mrl_index: usize,
    above_mrl_index: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let mut prediction = vec![T::default(); width * height];
    match direction {
        IntraCardinalDirection::Vertical => {
            let above_row = y
                .checked_sub(1)
                .and_then(|row| row.checked_sub(above_mrl_index))
                .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            for column in 0..width {
                let sample = workspace.reconstructed_sample(
                    PlaneId::Y,
                    x.saturating_add(column),
                    above_row,
                )?;
                for row in 0..height {
                    prediction[row * width + column] = sample;
                }
            }
        }
        IntraCardinalDirection::Horizontal => {
            let left_col = x
                .checked_sub(1)
                .and_then(|col| col.checked_sub(mrl_index))
                .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            for row in 0..height {
                let sample =
                    workspace.reconstructed_sample(PlaneId::Y, left_col, y.saturating_add(row))?;
                for column in 0..width {
                    prediction[row * width + column] = sample;
                }
            }
        }
    }
    Ok(prediction)
}

/// Builds the § 7.13.2.8 ZONE-1 IDIF above edge `AboveRow[-2 ..= w + h + 1]`
/// (length `w + h + 4` for `MrlIndex == 0`, `slice[0]` = logical `-2`) for a
/// neighbour-having (`haveLeft && haveAbove`) one-sided block.
///
/// Per § 7.13.2.1: the in-row samples `AboveRow[i]` for `i in 0..w + h` are
/// `CurrFrame[plane][y - 1][Min(aboveLimit, x + i)]`, the corner `AboveRow[-1]` is
/// `CurrFrame[plane][y - 1][x - 1]`. When `edge_filter` is non-default the
/// § 7.13.2.7 step 1 corner / edge filter rewrites the raw edge IN PLACE before
/// the spec extension: the § 7.13.2.14 corner blend rewrites `AboveRow[-1]`
/// (slice 1), then the § 7.13.2.18 edge filter sweeps the corner + `numPx`
/// samples. Per § 7.13.2.8 the edge is then extended FROM the filtered values:
/// `AboveRow[minBase - 1] = AboveRow[minBase]` (here `AboveRow[-2] = AboveRow[-1]`)
/// and `AboveRow[maxBase + 1] = AboveRow[maxBase + 2] = AboveRow[maxBase]` (the
/// two trailing samples repeat the clamped last in-row sample). `aboveLimit =
/// Min(maxX, x + w + 4 * num4AboveRight - 1)`.
///
/// For `MrlIndex > 0` the above row is read `aboveMrlIndex` lines further up (AVM
/// `above_ref_1st = ref - ref_stride * (aboveMrlIndex + 1)`, with `aboveMrlIndex ==
/// sbBoundary ? 0 : MrlIndex` resolved by the caller), and the projection geometry
/// widens by `mrl_index` (`maxBase += mrl_index << 1`).
#[allow(clippy::too_many_arguments)]
fn build_one_sided_above_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    num4_above_right: usize,
    mrl_index: usize,
    above_mrl_index: usize,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    if y == 0 {
        let fallback_col = x
            .checked_sub(mrl_index + 1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let fallback = workspace.reconstructed_sample(plane_id, fallback_col, y)?;
        return build_one_sided_idif_edge(
            width,
            height,
            mrl_index,
            edge_filter,
            || Ok(fallback),
            |_| Ok(fallback),
        );
    }
    let above_row = y
        .checked_sub(1)
        .and_then(|row| row.checked_sub(above_mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_x = workspace
        .plane(plane_id)?
        .storage_size()
        .width()
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let above_limit = width
        .checked_add(num4_above_right.saturating_mul(4))
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| x.checked_add(v))
        .map_or(max_x, |limit| limit.min(max_x));
    let corner_col = x.checked_sub(1).unwrap_or(x);
    let edge = build_one_sided_idif_edge(
        width,
        height,
        mrl_index,
        edge_filter,
        || workspace.reconstructed_sample(plane_id, corner_col, above_row),
        |i| {
            let column = x.saturating_add(i).min(above_limit);
            workspace.reconstructed_sample(plane_id, column, above_row)
        },
    )?;
    Ok(edge)
}

/// Builds the §7.13.2.8 one-sided IDIF read edge `Edge[-2 ..= w + h + 1]` (length
/// `w + h + 4` for `MrlIndex == 0`) shared by the zone-1 above and zone-3 left
/// builders, which differ only in the per-axis sample fetch. `corner` returns the
/// logical `[-1]` corner; `in_edge(i)` returns the logical `[i]` in-edge sample
/// (already §7.13.2.1 frame-edge / above-right / below-left clamped). The §7.13.2.7
/// corner / edge filter then runs over the raw edge, followed by the §7.13.2.8 `-2`
/// / trailing extension ([`finalize_one_sided_idif_edge`]).
///
/// maxBase = w + h - 1 (AVM `max_base = (bw + bh) - 1`). The slice layout: logical
/// -2 (slice 0), -1 corner (slice 1), 0..=maxBase (slices 2..=maxBase+2), and the
/// two trailing extension samples maxBase+1 / maxBase+2.
fn build_one_sided_idif_edge<T: ReconSample>(
    width: usize,
    height: usize,
    mrl_index: usize,
    edge_filter: OneSidedEdgeFilter,
    corner: impl FnOnce() -> core::result::Result<T, splot_recon::ReconError>,
    in_edge: impl Fn(usize) -> core::result::Result<T, splot_recon::ReconError>,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let max_base = width
        .checked_add(height)
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| v.checked_add(mrl_index << 1))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let edge_len = max_base
        .checked_add(5)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mut edge = vec![T::default(); edge_len];
    edge[1] = corner()?;
    for i in 0..=max_base {
        let slot = i
            .checked_add(2)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        edge[slot] = in_edge(i)?;
    }
    finalize_one_sided_idif_edge(&mut edge, max_base, edge_filter)?;
    Ok(edge)
}

/// Finalizes a one-sided IDIF read edge slice (zone-1 above edge or zone-3 left
/// edge — the slice layout is identical: `slice[1]` is the logical `[-1]` corner,
/// `slice[2]` the logical `[0]`, `slice[2 + maxBase]` the logical `maxBase`):
/// applies the § 7.13.2.7 step-1 corner / edge filter, then the § 7.13.2.8 `-2` /
/// trailing extension FROM the (possibly filtered) edge. Shared by the zone-1 and
/// zone-3 builders so the symmetric tail is written once.
///
/// First the § 7.13.2.14 corner filter (when `corner_opposite` is `Some`): the
/// blend `s = LeftCol[0] * 5 + AboveRow[-1] * 6 + AboveRow[0] * 5`, `Round2(s, 4)`
/// is symmetric in its two `*5` terms, so passing `(corner_opposite, slice[1],
/// slice[2])` reproduces it for BOTH zones (zone-1 `LeftCol[0]` / zone-3
/// `AboveRow[0]` is the opposite sample). The result is written into the shared
/// corner `slice[1]` (= both `AboveRow[-1]` and `LeftCol[-1]`).
///
/// Then the § 7.13.2.18 edge filter over the corner + `num_px` samples
/// (`slice[1..]`, so `edge[0]` is the just-rewritten corner, never overwritten):
/// a `strength == 0` no-op leaves the raw edge unchanged.
///
/// Finally the § 7.13.2.8 extension: `slice[0]` (logical `-2`) = `slice[1]`
/// (logical `-1`), and the two trailing samples `slice[maxBase + 3]` /
/// `slice[maxBase + 4]` (logical `maxBase + 1` / `maxBase + 2`) repeat the clamped
/// last in-edge sample `slice[maxBase + 2]`.
fn finalize_one_sided_idif_edge<T: ReconSample>(
    edge: &mut [T],
    max_base: usize,
    filter: OneSidedEdgeFilter,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if let Some(opposite) = filter.corner_opposite {
        let corner = edge
            .get(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            .to_u16();
        let own0 = edge
            .get(2)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            .to_u16();
        let filtered = filter_intra_edge_corner(opposite, corner, own0);
        edge[1] = T::try_from_u16(filtered)?;
    }
    if filter.num_px > 0
        && let Some(window) = edge.get_mut(1..)
    {
        apply_intra_edge_filter(window, filter.num_px, filter.strength)?;
    }
    edge[0] = edge[1];
    let last_in_edge = edge[max_base + 2];
    if let Some(slot) = edge.get_mut(max_base + 3) {
        *slot = last_in_edge;
    }
    if let Some(slot) = edge.get_mut(max_base + 4) {
        *slot = last_in_edge;
    }
    Ok(())
}

/// Reconstructs one neighbour-having ZONE-3 ONE-SIDED directional block — D203
/// (§ 7.13.2.8 step 3, pAngle 203) — over the § 7.13.2.1 left column PLUS the
/// real reconstructed below-left read from the partially-built frame, adds the
/// decoded residual (or writes the bare prediction for an `all_zero` block), and
/// stores the result so later blocks read it as a neighbour.
///
/// Zone-3 (`pAngle > 180`, `needBottom`) is the symmetric mirror of zone-1 D45:
/// `dy = Dr_Intra_Derivative[270 - 203] = Dr_Intra_Derivative[67] = 24`,
/// `idx = (j + 1) * dy`, `base = (idx >> 6) + i`, projecting DOWN-AND-LEFT into
/// the below-left up to `base == maxBaseY == w + h - 1`. So unlike the middle
/// angles (which stay within `LeftCol[0..h)`), this reads `w` below-left samples.
/// § 7.13.2.1 fills `LeftCol[i] = CurrFrame[plane][Min(leftLimit, y + i)][x - 1]`
/// for `i in 0..w + h`, with `leftLimit = Min(maxY, y + h + 4 * num4BelowLeft - 1)`
/// (8-bit, `MrlIndex == 0`); the `num4_below_left` (in plane 4x4 units, from
/// § 5.20.7.25 `count_bottom_left_avail` over the § 5.20.2.3 `BlockDecoded` state)
/// bounds how far the real below-left extends before the spec clamps to
/// `CurrFrame[plane][leftLimit][x - 1]`. In raster order `num4_below_left == 0`
/// for the gated position (no block below-left is decoded yet), so the below-left
/// samples are the clamped repeat of the last in-block left sample.
///
/// The block is gated (by the caller) to a first-superblock-row
/// (`frontier.r == 0`, `haveAbove == 0`), non-first-column (`haveLeft == 1`) full
/// 64x64 superblock, so the real reconstructed left column is the right column of
/// the already-decoded left superblock. At `haveAbove == 0 && haveLeft == 1`,
/// § 7.13.2.1 sets the corner `LeftCol[-1] = AboveRow[-1] =
/// CurrFrame[plane][y][x - 1]` (the top-left sample of the left column).
/// `enable_intra_edge_filter == 0` / `MrlIndex == 0` keep the § 7.13.2.7
/// edge-filter / corner-filter / upsample synthesis a no-op, and `enable_ibp == 0`
/// keeps `useIBP == 0` (§ 7.13.2.7 gates `useIBP` on `pAngle > 180`). Luma uses
/// the § 7.13.2.8 IDIF 4-tap (`enableIdif = plane == 0`); D203's `dy == 24` lands
/// most projections on a nonzero `shift`, so the IDIF genuinely interpolates over
/// the real reconstructed left column. Chroma uses the spec-mandated bilinear
/// one-sided branch (`enableIdif == 0` for U/V) over the same prepared left edge.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_one_sided_left_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let prediction = if matches!(plane_id, PlaneId::Y) {
        predict_general_intra_luma_one_sided_left_mrl(
            workspace,
            p_angle,
            x,
            y,
            log2_width,
            log2_height,
            num4_below_left,
            have_above,
            mrl_index,
            bit_depth,
            edge_filter,
        )?
    } else {
        if mrl_index != 0 {
            return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
        }
        let left_idif = build_one_sided_left_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            num4_below_left,
            have_above,
            mrl_index,
            edge_filter,
        )?;
        let mut prediction = vec![T::default(); width * height];
        let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
        let left_bilinear = &left_idif[2..2 + width + height];
        predict_intra_directional_angle_rect_into(
            bit_depth,
            block_size,
            angle,
            IntraDirectionalAngleEdges::left(left_bilinear),
            &mut prediction,
            width,
        )?;
        prediction
    };
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_general_intra_luma_one_sided_left_mrl<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let left_idif = build_one_sided_left_idif_edge(
        workspace,
        PlaneId::Y,
        x,
        y,
        width,
        height,
        num4_below_left,
        have_above,
        mrl_index,
        edge_filter,
    )?;
    let mut prediction = vec![T::default(); width * height];
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into(
        bit_depth,
        block_size,
        IntraDirectionalAngle::try_from_p_angle(p_angle)?,
        IntraDirectionalAngleIdifEdges::left(&left_idif),
        mrl_index,
        &mut prediction,
        width,
    )?;
    Ok(prediction)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_mrl_secondary_left_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let mut prediction = predict_general_intra_luma_one_sided_left_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        num4_below_left,
        have_above,
        mrl_index,
        bit_depth,
        OneSidedEdgeFilter::default(),
    )?;
    let secondary = predict_general_intra_luma_one_sided_left_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        num4_below_left,
        have_above,
        0,
        bit_depth,
        OneSidedEdgeFilter::default(),
    )?;
    average_luma_prediction_with(&mut prediction, secondary)?;
    write_luma_prediction_block(
        workspace,
        block,
        prediction,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        use_tcq,
        bit_depth,
    )
}

/// Builds the § 7.13.2.8 ZONE-3 IDIF left edge `LeftCol[-2 ..= w + h + 1]`
/// (length `w + h + 4` for `MrlIndex == 0`, `slice[0]` = logical `-2`) for a
/// first-superblock-row, non-first-column (`haveAbove == 0 && haveLeft == 1`)
/// one-sided block.
///
/// Per § 7.13.2.1 (the `haveLeft == 1` left-column branch): the in-column samples
/// `LeftCol[i]` for `i in 0..w + h` are `CurrFrame[plane][Min(leftLimit, y + i)]
/// [x - 1]` with `leftLimit = Min(maxY, y + h + 4 * num4BelowLeft - 1)`. The corner
/// `LeftCol[-1] = AboveRow[-1]` follows § 7.13.2.1 / AVM `need_above_left`: when the
/// above row is available (`have_above`, AVM `n_top_px > 0 && n_left_px > 0`) it is
/// the DIAGONAL above-left `CurrFrame[plane][y - 1][x - 1]`; when the above row is
/// off-grid (`!have_above`, the frame-top `n_top_px == 0` case) it is the TOP of the
/// left column `CurrFrame[plane][y][x - 1]` (AVM `left_col[-i] = left_ref[0]`). Per
/// § 7.13.2.8 the edge is then extended: `LeftCol[minBase - 1] = LeftCol[minBase]`
/// (here `LeftCol[-2] = LeftCol[-1]`) and `LeftCol[maxBase + 1] = LeftCol[maxBase +
/// 2] = LeftCol[maxBase]` (the two trailing samples repeat the clamped last in-column
/// sample).
///
/// For `MrlIndex > 0` the left column is read `MrlIndex` columns further left (AVM
/// `left_ref_1st = ref - 1 - MrlIndex`; the left axis has no sbBoundary special
/// case), and the projection widens by `mrl_index` (`maxBase += mrl_index << 1`).
#[allow(clippy::too_many_arguments)]
fn build_one_sided_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let left_col = x
        .checked_sub(1)
        .and_then(|col| col.checked_sub(mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_y = workspace
        .plane(plane_id)?
        .storage_size()
        .height()
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let left_limit = height
        .checked_add(num4_below_left.saturating_mul(4))
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| y.checked_add(v))
        .map_or(max_y, |limit| limit.min(max_y));
    let corner_row = if have_above {
        y.checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
    } else {
        y
    };
    build_one_sided_idif_edge(
        width,
        height,
        mrl_index,
        edge_filter,
        || workspace.reconstructed_sample(plane_id, left_col, corner_row),
        |i| {
            let row = y.saturating_add(i).min(left_limit);
            workspace.reconstructed_sample(plane_id, left_col, row)
        },
    )
}

/// The §7.13.2.7/§7.13.2.9 inputs for the IBP SECONDARY one-sided prediction: the
/// `secondAngle` (`pAngle ± 180`), the opposite-edge §7.13.2.7 filter, and the
/// opposite-edge far-extension count (the §5.20.7.25 below-left / above-right 4x4
/// units the secondary projection walks into).
#[derive(Clone, Copy, Debug)]
pub(crate) struct IbpSecondary {
    /// `secondAngle = pAngle + 180` (zone-1 primary) / `pAngle - 180` (zone-3),
    /// in the opposite one-sided zone — its §7.13.2.8 prediction reads the opposite
    /// edge.
    pub second_angle: u16,
    /// The §7.13.2.7 step-1 corner/edge filter for the secondary (opposite) edge.
    pub edge_filter: OneSidedEdgeFilter,
    /// The §5.20.7.25 `num4` for the secondary edge's far extension: below-left 4x4
    /// units (zone-1 secondary reads the left column) / above-right 4x4 units
    /// (zone-3 secondary reads the above row).
    pub num4_far: usize,
}

/// Reconstructs one §7.13.2.9 `useIBP` one-sided directional LUMA leaf: the
/// §7.13.2.8 primary prediction at `p_angle` (zone-1 above / zone-3 left) blended
/// with the secondary §7.13.2.8 prediction at `secondary.second_angle` (the
/// OPPOSITE edge), then the decoded residual added (or the bare blended prediction
/// for an `all_zero` block).
///
/// The blend weights and the `cShift`/`rShift` indexing come from the §7.13.2.9
/// IBP weights process; [`apply_ibp_dr_blend_rect`] is a validated no-op when the
/// leaf's mode is not in the `is_ibp_enabled` set (so the bare primary survives).
/// The caller has already verified BOTH the primary edge (above + above-right /
/// left + below-left) AND the secondary/opposite edge are reconstructed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_one_sided_ibp_luma_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    primary_num4_far: usize,
    primary_edge_filter: OneSidedEdgeFilter,
    secondary: IbpSecondary,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let zone1 = p_angle < 90;
    let mut primary = vec![T::default(); width * height];
    if zone1 {
        let above_idif = build_one_sided_above_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            primary_num4_far,
            0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
            0, // above_mrl_index
            primary_edge_filter,
        )?;
        let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                block_size,
                angle,
                IntraDirectionalAngleIdifEdges::above(&above_idif),
                &mut primary,
                width,
            )?;
        } else {
            let above = &above_idif[2..2 + width + height];
            predict_intra_directional_angle_rect_into(
                bit_depth,
                block_size,
                angle,
                IntraDirectionalAngleEdges::above(above),
                &mut primary,
                width,
            )?;
        }
    } else {
        let left_idif = build_one_sided_left_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            primary_num4_far,
            true,
            0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
            primary_edge_filter,
        )?;
        let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                block_size,
                angle,
                IntraDirectionalAngleIdifEdges::left(&left_idif),
                &mut primary,
                width,
            )?;
        } else {
            let left = &left_idif[2..2 + width + height];
            predict_intra_directional_angle_rect_into(
                bit_depth,
                block_size,
                angle,
                IntraDirectionalAngleEdges::left(left),
                &mut primary,
                width,
            )?;
        }
    }
    let mut second = vec![T::default(); width * height];
    let second_angle = IntraDirectionalAngle::try_from_p_angle(secondary.second_angle)?;
    if zone1 {
        let left_idif = build_one_sided_left_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            secondary.num4_far,
            true,
            0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
            secondary.edge_filter,
        )?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                block_size,
                second_angle,
                IntraDirectionalAngleIdifEdges::left(&left_idif),
                &mut second,
                width,
            )?;
        } else {
            let left = &left_idif[2..2 + width + height];
            predict_intra_directional_angle_rect_into(
                bit_depth,
                block_size,
                second_angle,
                IntraDirectionalAngleEdges::left(left),
                &mut second,
                width,
            )?;
        }
    } else {
        let above_idif = build_one_sided_above_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            secondary.num4_far,
            0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
            0, // above_mrl_index
            secondary.edge_filter,
        )?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                block_size,
                second_angle,
                IntraDirectionalAngleIdifEdges::above(&above_idif),
                &mut second,
                width,
            )?;
        } else {
            let above = &above_idif[2..2 + width + height];
            predict_intra_directional_angle_rect_into(
                bit_depth,
                block_size,
                second_angle,
                IntraDirectionalAngleEdges::above(above),
                &mut second,
                width,
            )?;
        }
    }
    apply_ibp_dr_blend_rect(block_size, p_angle, &mut primary, &second)?;
    let out = if block.all_zero {
        primary
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &primary,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &primary,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// The § 7.13.2.7 step-1 filters for the TWO edges of a § 7.13.2.8 ZONE-2 (middle,
/// `90 < pAngle < 180`) leaf: the above edge (filtered with `angleAbove = pAngle -
/// 90`) and the left edge (filtered with `angleLeft = pAngle - 180`). Both carry the
/// SHARED § 7.13.2.14 corner blend in `corner_opposite` (the above edge's opposite is
/// `LeftCol[0]`, the left edge's is `AboveRow[0]`; the blend is symmetric in the two
/// `*5` terms, so both reproduce the same rewritten corner). For zone-2 AVM sets
/// `need_right == need_bottom == 0`, so neither edge filter spans a far extent — the
/// above filter sweeps `n_top_px + 1` samples, the left filter `n_left_px + 1`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TwoSidedMiddleEdgeFilters {
    /// § 7.13.2.7 filter for the above edge (`angleAbove`, `filterTypeAbove`).
    pub above: OneSidedEdgeFilter,
    /// § 7.13.2.7 filter for the left edge (`angleLeft`, `filterTypeLeft`).
    pub left: OneSidedEdgeFilter,
}

/// Reconstructs one neighbour-having § 7.13.2.8 ZONE-2 (middle, `90 < pAngle < 180`)
/// directional LUMA leaf over BOTH the above row and the left column read from the
/// partially-built frame, adds the decoded residual (or writes the bare prediction
/// for an `all_zero` block), and stores the result for later neighbours.
///
/// Zone-2 (`needAbove && needLeft && needAboveLeft`) reads the in-block above row
/// `AboveRow[0..w)`, the in-block left column `LeftCol[0..h)`, and the shared corner
/// `AboveRow[-1] = LeftCol[-1] = CurrFrame[plane][y - 1][x - 1]`. The z2 IDIF
/// projection stays within those edges (`base_x in [-1, w-1]`, `base_y in [-1, h-1]`),
/// so NO above-right / below-left far samples are read (unlike zone-1/zone-3). The
/// caller has verified the whole above row, left column, and corner are reconstructed.
///
/// The § 7.13.2.7 step-1 filter rewrites the edges in place before the § 7.13.2.8
/// prediction: the § 7.13.2.14 corner blend (`(w + h) >= 24`) rewrites the shared
/// corner, then the § 7.13.2.18 edge filter sweeps each edge with its per-edge
/// § 7.13.2.17 strength. The edges are then spec-extended (`Edge[-2] = Edge[-1]`,
/// `Edge[side] = Edge[side + 1] = Edge[side - 1]`) into the IDIF logical range
/// `[-2 ..= side + 1]` and the generalized middle IDIF runs at the leaf's `pAngle`.
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_two_sided_middle_luma_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
    filters: TwoSidedMiddleEdgeFilters,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle)?;

    let above_row = y
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let left_col = x
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let corner = workspace.reconstructed_sample(PlaneId::Y, left_col, above_row)?;

    let above_idif = build_two_sided_middle_idif_edge(width, filters.above, corner, |i| {
        workspace.reconstructed_sample(PlaneId::Y, x.saturating_add(i), above_row)
    })?;
    let left_idif = build_two_sided_middle_idif_edge(height, filters.left, corner, |i| {
        workspace.reconstructed_sample(PlaneId::Y, left_col, y.saturating_add(i))
    })?;

    let mut prediction = vec![T::default(); width * height];
    predict_intra_middle_directional_angle_rect_idif_into(
        bit_depth,
        block_size,
        angle,
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        &mut prediction,
        width,
    )?;
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_two_sided_middle_luma_mrl_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    mrl_index: usize,
    above_mrl_index: usize,
    is_sb_boundary: bool,
    secondary_mrl: bool,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let mut prediction = predict_two_sided_middle_luma_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        mrl_index,
        above_mrl_index,
        is_sb_boundary,
        bit_depth,
    )?;
    if secondary_mrl {
        let secondary = predict_two_sided_middle_luma_mrl(
            workspace,
            p_angle,
            x,
            y,
            log2_width,
            log2_height,
            0,
            0,
            false,
            bit_depth,
        )?;
        average_luma_prediction_with(&mut prediction, secondary)?;
    }
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_two_sided_middle_luma_mrl<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    mrl_index: usize,
    above_mrl_index: usize,
    is_sb_boundary: bool,
    bit_depth: BitDepth,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle)?;
    if y == 0 {
        let above_idif =
            build_top_row_left_only_middle_mrl_above_idif_edge(workspace, x, y, width, mrl_index)?;
        let left_idif =
            build_top_row_left_only_middle_mrl_left_idif_edge(workspace, x, y, height, mrl_index)?;
        let mut prediction = vec![T::default(); width * height];
        predict_intra_middle_directional_angle_rect_idif_mrl_into(
            bit_depth,
            block_size,
            angle,
            IntraMiddleDirectionalAngleIdifMrlEdges::both(&left_idif, &above_idif),
            mrl_index,
            &mut prediction,
            width,
        )?;
        return Ok(prediction);
    }
    let above_idif = build_two_sided_middle_mrl_above_idif_edge(
        workspace,
        x,
        y,
        width,
        mrl_index,
        above_mrl_index,
    )?;
    let left_idif = build_two_sided_middle_mrl_left_idif_edge(
        workspace,
        x,
        y,
        height,
        mrl_index,
        is_sb_boundary,
    )?;
    let mut prediction = vec![T::default(); width * height];
    predict_intra_middle_directional_angle_rect_idif_mrl_into(
        bit_depth,
        block_size,
        angle,
        IntraMiddleDirectionalAngleIdifMrlEdges::both(&left_idif, &above_idif),
        mrl_index,
        &mut prediction,
        width,
    )?;
    Ok(prediction)
}

fn build_top_row_left_only_middle_mrl_above_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    mrl_index: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let left_col = x
        .checked_sub(1)
        .and_then(|col| col.checked_sub(mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let seed = workspace.reconstructed_sample(PlaneId::Y, left_col, y)?;
    let len = width
        .checked_add(mrl_index)
        .and_then(|v| v.checked_add(4))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    Ok(vec![seed; len])
}

fn build_top_row_left_only_middle_mrl_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    height: usize,
    mrl_index: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let left_col = x
        .checked_sub(1)
        .and_then(|col| col.checked_sub(mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let seed = workspace.reconstructed_sample(PlaneId::Y, left_col, y)?;
    let max_logical = i64::try_from(height)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        + 1;
    build_two_sided_middle_mrl_idif_edge(height, mrl_index, max_logical, |logical| {
        if logical < 0 {
            return Ok(seed);
        }
        let logical = usize::try_from(logical)
            .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let row = y.saturating_add(logical.min(height.saturating_sub(1)));
        Ok(workspace.reconstructed_sample(PlaneId::Y, left_col, row)?)
    })
}

fn build_two_sided_middle_mrl_above_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    mrl_index: usize,
    above_mrl_index: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let above_row = y
        .checked_sub(1)
        .and_then(|row| row.checked_sub(above_mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_logical = i64::try_from(width)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        + 1;
    build_two_sided_middle_mrl_idif_edge(width, mrl_index, max_logical, |logical| {
        let column = if logical < 0 {
            let back = usize::try_from(-logical)
                .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            x.checked_sub(back)
                .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        } else {
            let logical = usize::try_from(logical)
                .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            x.saturating_add(logical.min(width.saturating_sub(1)))
        };
        Ok(workspace.reconstructed_sample(PlaneId::Y, column, above_row)?)
    })
}

fn build_two_sided_middle_mrl_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    height: usize,
    mrl_index: usize,
    is_sb_boundary: bool,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let left_col = x
        .checked_sub(1)
        .and_then(|col| col.checked_sub(mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_logical = i64::try_from(height)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        + 1;
    build_two_sided_middle_mrl_idif_edge(height, mrl_index, max_logical, |logical| {
        let row = if logical < 0 {
            if is_sb_boundary {
                y.checked_sub(1)
                    .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            } else {
                let back = usize::try_from(-logical)
                    .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
                y.checked_sub(back)
                    .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            }
        } else {
            let logical = usize::try_from(logical)
                .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            y.saturating_add(logical.min(height.saturating_sub(1)))
        };
        Ok(workspace.reconstructed_sample(PlaneId::Y, left_col, row)?)
    })
}

fn build_two_sided_middle_mrl_idif_edge<T: ReconSample>(
    side: usize,
    mrl_index: usize,
    max_logical: i64,
    sample: impl Fn(i64) -> core::result::Result<T, GeneralIntraResidualError>,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let len = side
        .checked_add(mrl_index)
        .and_then(|v| v.checked_add(4))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mrl = i64::try_from(mrl_index)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let min_base = -1 - mrl;
    let mut edge = vec![T::default(); len];
    for logical in min_base..=max_logical {
        let offset = usize::try_from(logical + mrl + 2)
            .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        edge[offset] = sample(logical)?;
    }
    edge[0] = edge[1];
    Ok(edge)
}

/// Builds one § 7.13.2.8 ZONE-2 IDIF edge `Edge[-2 ..= side + 1]` (length `side + 4`,
/// `slice[0]` = logical `-2`, `slice[1]` the `-1` corner, `slice[i + 2]` logical `i`).
/// The in-block samples `Edge[0..side)` come from `in_edge`, the corner from the
/// caller's shared `corner`. The § 7.13.2.7 corner / edge filter then rewrites the
/// edge in place ([`finalize_one_sided_idif_edge`] handles the § 7.13.2.14 corner
/// blend, the § 7.13.2.18 sweep, and the § 7.13.2.8 `-2` / trailing extension — its
/// `max_base == side - 1` for zone-2, so the trailing slots `side + 2` / `side + 3`
/// repeat `Edge[side - 1]`, matching AVM `above[bw] = above[bw - 1]`).
fn build_two_sided_middle_idif_edge<T: ReconSample>(
    side: usize,
    edge_filter: OneSidedEdgeFilter,
    corner: T,
    in_edge: impl Fn(usize) -> core::result::Result<T, splot_recon::ReconError>,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let max_base = side
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let edge_len = side
        .checked_add(4)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mut edge = vec![T::default(); edge_len];
    edge[1] = corner;
    for i in 0..side {
        edge[i + 2] = in_edge(i)?;
    }
    finalize_one_sided_idif_edge(&mut edge, max_base, edge_filter)?;
    Ok(edge)
}

/// Reconstructs one neighbour-having CARDINAL directional luma block — `V_PRED`
/// (§ 7.13.2.8 step 4, pAngle 90) or `H_PRED` (step 5, pAngle 180) — over the
/// § 7.13.2.1 edge read from the partially-built frame's **real reconstructed
/// neighbour**, adds the decoded residual (or writes the bare prediction for an
/// `all_zero` block), and stores the result so later blocks read it as a
/// neighbour.
///
/// `log2_width` and `log2_height` are the block's §7.15.4 transform dimensions
/// and may differ (e.g. a 64x32 `TX_64X32` luma block has `log2_width == 6`,
/// `log2_height == 5`). The §7.13.2.8 cardinal copy is fully rectangular:
/// `predict_intra_cardinal_directional_rect_into` takes independent width/height,
/// `intra_dc_edges_for_rect` returns the W-wide above row / H-tall left column,
/// and `reconstruct_general_intra_block_rect_with_prediction` adds the §7.14.4 /
/// §7.15.4 / §7.14.3 rectangular residual (with the §7.15.4.1 √2 rescale when the
/// log2 ratio is odd). A square block is the `log2_width == log2_height` case.
///
/// The cardinal cases are a degenerate sample copy with NO IDIF, NO corner, NO
/// edge synthesis and NO `useIBP` (which § 7.13.2.7 gates on
/// `pAngle < 90 || pAngle > 180`):
/// - `V_PRED` (pAngle 90): `pred[i][j] = AboveRow[j]` — every one of the H rows is
///   a copy of the real reconstructed W-wide above row
///   (`CurrFrame[plane][y-1][x..x+w)`). It reads ONLY the above row, so it needs
///   `haveAbove == 1` (a real above neighbour; target a superblock row > 0).
/// - `H_PRED` (pAngle 180): `pred[i][j] = LeftCol[i]` — every one of the W columns
///   is a copy of the real reconstructed H-tall left column
///   (`CurrFrame[plane][y..y+h)][x-1]`). It reads ONLY the left column, so it
///   needs `haveLeft == 1` (a real left neighbour; target a non-first superblock
///   column).
///
/// Unlike the § 7.13.2.8 "middle" angles (D135), the cardinal copy is bit-exact
/// over a NON-flat reconstructed edge without any interpolation, so it does not
/// need the corner / IDIF that [`build_directional_middle_edges`] guards. The
/// `enable_intra_edge_filter == 0` / `MrlIndex == 0` minimal-tool subset keeps the
/// § 7.13.2.7 edge-filter / corner-filter step a no-op (and § 7.13.2.7 skips it
/// entirely for `pAngle == 90 || pAngle == 180`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_cardinal_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    direction: IntraCardinalDirection,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let synthesized_edge;
    let cardinal_edges = match direction {
        IntraCardinalDirection::Vertical => {
            if let Some(above) = edges.above_samples() {
                IntraDirectionalAngleEdges::above(above)
            } else {
                let left = edges
                    .left_samples()
                    .ok_or(GeneralIntraResidualError::MissingCardinalEdge)?;
                let fill = *left
                    .first()
                    .ok_or(GeneralIntraResidualError::MissingCardinalEdge)?;
                synthesized_edge = vec![fill; width];
                IntraDirectionalAngleEdges::above(&synthesized_edge)
            }
        }
        IntraCardinalDirection::Horizontal => {
            if let Some(left) = edges.left_samples() {
                IntraDirectionalAngleEdges::left(left)
            } else {
                let above = edges
                    .above_samples()
                    .ok_or(GeneralIntraResidualError::MissingCardinalEdge)?;
                let fill = *above
                    .first()
                    .ok_or(GeneralIntraResidualError::MissingCardinalEdge)?;
                synthesized_edge = vec![fill; height];
                IntraDirectionalAngleEdges::left(&synthesized_edge)
            }
        }
    };
    let mut prediction = vec![T::default(); width * height];
    predict_intra_cardinal_directional_rect_into(
        bit_depth,
        block_size,
        direction,
        cardinal_edges,
        &mut prediction,
        width,
    )?;
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Builds the AV2 § 7.13.2.1 logical `LeftCol[-1..h)` and `AboveRow[-1..w)` edges
/// (8-bit, `MrlIndex == 0`, no DIP/MRL/edge-filter) for § 7.13.2.8 middle-angle
/// directional prediction over reconstructed neighbours, for any plane. Each
/// returned slice has length `side + 1`: index `0` is the logical `-1` (corner)
/// sample and index `k + 1` is logical index `k`.
///
/// The corner `AboveRow[-1] == LeftCol[-1]` and the `AboveRow[i]` / `LeftCol[i]`
/// fills follow § 7.13.2.1 exactly for the minimal-tool subset (`MrlIndex == 0`,
/// `aboveMrlIndex == 0` at the superblock boundary):
/// - `haveLeft && haveAbove` (the `above_corner` argument carries the real corner):
///   `LeftCol[i]` is the reconstructed left column (`left_neighbour`, clamped at the
///   bottom-left sentinel, `num4BelowLeft == 0` in raster order), `AboveRow[i]` is the
///   reconstructed above row (`above_neighbour`), and the corner
///   `AboveRow[-1] = LeftCol[-1] = CurrFrame[plane][y-1][x-1]` (`above_corner`, the real
///   reconstructed diagonally-above-left sample, read by the caller via
///   `reconstructed_sample`). § 7.13.2.8 D135 reads this corner on its main diagonal
///   (`column == row` gives `above_base == -1`, `shift == 0`, the predictor copies it).
/// - `!haveLeft && haveAbove`: `AboveRow[i]` is the reconstructed above row
///   (`above_neighbour`); § 7.13.2.1 sets `LeftCol[i] = CurrFrame[plane][y-1][x]` (the
///   first above sample, repeated) and the corner
///   `AboveRow[-1] = LeftCol[-1] = CurrFrame[plane][y-1][x]` = `AboveRow[0]`. No extra
///   corner read is needed (it is the first above sample).
/// - `haveLeft && !haveAbove` (the committed row-0 D135 block): `LeftCol[i]` is the
///   reconstructed left column, `AboveRow[i]` is the repeated first left sample
///   `CurrFrame[plane][y][x-1]`, and the corner
///   `AboveRow[-1] = LeftCol[-1] = CurrFrame[plane][y][x-1]` (the first left sample).
/// - neither: the § 7.13.2.1 flat fallbacks (`AboveRow 127`, `LeftCol 129`, corner
///   `128`) — the no-neighbour path handled by `predict_directional_noneighbour`.
///
/// `above_corner` MUST be `Some(CurrFrame[plane][y-1][x-1])` for the `haveLeft &&
/// haveAbove` arm (the only arm needing a corner sample outside `left_neighbour` /
/// `above_neighbour`); the other arms ignore it. When it is absent for that arm the
/// builder returns [`GeneralIntraResidualError::UnsupportedDirectionalAboveEdge`]
/// rather than fabricate a corner.
fn build_directional_middle_edges<T: ReconSample>(
    left_neighbour: Option<&[T]>,
    above_neighbour: Option<&[T]>,
    above_corner: Option<T>,
    have_left: bool,
    have_above: bool,
    side: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(Vec<T>, Vec<T>), GeneralIntraResidualError> {
    build_directional_middle_rect_edges(
        left_neighbour,
        above_neighbour,
        above_corner,
        have_left,
        have_above,
        side,
        side,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_directional_middle_rect_edges<T: ReconSample>(
    left_neighbour: Option<&[T]>,
    above_neighbour: Option<&[T]>,
    above_corner: Option<T>,
    have_left: bool,
    have_above: bool,
    width: usize,
    height: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(Vec<T>, Vec<T>), GeneralIntraResidualError> {
    let mut left = Vec::with_capacity(height + 1);
    let mut above = Vec::with_capacity(width + 1);
    match (have_left, have_above) {
        (true, true) => {
            let Some(corner) = above_corner else {
                return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
            };
            let left_samples = left_neighbour.unwrap_or(&[]);
            let above_samples = above_neighbour.unwrap_or(&[]);
            left.push(corner);
            above.push(corner);
            for i in 0..height {
                left.push(sample_or_last(
                    left_samples,
                    i,
                    noneighbour_left::<T>(bit_depth),
                ));
            }
            for i in 0..width {
                above.push(sample_or_last(
                    above_samples,
                    i,
                    noneighbour_above::<T>(bit_depth),
                ));
            }
        }
        (false, true) => {
            let above_samples = above_neighbour.unwrap_or(&[]);
            let seed = above_samples
                .first()
                .copied()
                .unwrap_or(noneighbour_above::<T>(bit_depth));
            left.push(seed);
            above.push(seed);
            for _ in 0..height {
                left.push(seed);
            }
            for i in 0..width {
                above.push(sample_or_last(
                    above_samples,
                    i,
                    noneighbour_above::<T>(bit_depth),
                ));
            }
        }
        (true, false) => {
            let left_samples = left_neighbour.unwrap_or(&[]);
            let seed = left_samples
                .first()
                .copied()
                .unwrap_or(noneighbour_left::<T>(bit_depth));
            left.push(seed);
            above.push(seed);
            for i in 0..height {
                left.push(sample_or_last(
                    left_samples,
                    i,
                    noneighbour_left::<T>(bit_depth),
                ));
            }
            for _ in 0..width {
                above.push(seed);
            }
        }
        (false, false) => {
            left.push(noneighbour_corner::<T>(bit_depth));
            above.push(noneighbour_corner::<T>(bit_depth));
            for _ in 0..height {
                left.push(noneighbour_left::<T>(bit_depth));
            }
            for _ in 0..width {
                above.push(noneighbour_above::<T>(bit_depth));
            }
        }
    }
    Ok((left, above))
}

/// Returns `samples[index]`, falling back to the last sample (§ 7.13.2.1 bottom-left
/// / right-most edge clamp) and then to `fallback` for an empty slice.
fn sample_or_last<T: ReconSample>(samples: &[T], index: usize, fallback: T) -> T {
    samples
        .get(index)
        .or_else(|| samples.last())
        .copied()
        .unwrap_or(fallback)
}

/// Maps a supported directional luma mode to its § 7.13.2.8 "middle" angle
/// (`90 < pAngle < 180`) for the bilinear/IDIF middle-angle predictor. Only
/// `D135` is a middle angle; the cardinal `Vertical` (pAngle 90) / `Horizontal`
/// (pAngle 180) modes use the dedicated copy predictor
/// [`reconstruct_general_intra_cardinal_neighbour_block_into`] and never reach the
/// middle-angle paths, so they return an error here (defensive: the dispatch
/// routes them away before these functions are called).
fn middle_directional_angle(
    mode: SupportedDirectionalLumaMode,
) -> core::result::Result<IntraMiddleDirectionalAngle, GeneralIntraResidualError> {
    match mode {
        SupportedDirectionalLumaMode::D113 => Ok(IntraMiddleDirectionalAngle::D113),
        SupportedDirectionalLumaMode::D135 => Ok(IntraMiddleDirectionalAngle::D135),
        SupportedDirectionalLumaMode::D157 => Ok(IntraMiddleDirectionalAngle::D157),
        SupportedDirectionalLumaMode::Vertical
        | SupportedDirectionalLumaMode::Horizontal
        | SupportedDirectionalLumaMode::D45
        | SupportedDirectionalLumaMode::D67
        | SupportedDirectionalLumaMode::D203 => {
            Err(GeneralIntraResidualError::CardinalModeInMiddleAnglePath)
        }
    }
}

/// Extends the §7.13.2.1 logical `LeftCol[-1..h)` / `AboveRow[-1..w)` edges
/// (length `side + 1`, index 0 = the `-1` corner) to the wider IDIF logical
/// range `Edge[-2..=side+1]` (length `side + 4`, index 0 = logical `-2`) for the
/// §7.13.2.8 luma IDIF 4-tap, which reads `Edge[base - 1 ..= base + 2]`. The
/// extension follows §7.13.2.8: `Edge[minBase - 1] = Edge[minBase]`
/// (`Edge[-2] = Edge[-1]`, the repeated corner) and, for the middle branch
/// (`90 < pAngle < 180`), `Edge[side] = Edge[side + 1] = Edge[side - 1]` (the
/// repeated last in-block edge sample).
fn extend_directional_middle_idif_edges<T: ReconSample>(
    left: &[T],
    above: &[T],
    bit_depth: BitDepth,
) -> (Vec<T>, Vec<T>) {
    (
        extend_one_middle_idif_edge(left, bit_depth),
        extend_one_middle_idif_edge(above, bit_depth),
    )
}

fn extend_one_middle_idif_edge<T: ReconSample>(edge: &[T], bit_depth: BitDepth) -> Vec<T> {
    let corner = edge
        .first()
        .copied()
        .unwrap_or(noneighbour_corner::<T>(bit_depth));
    let last = edge.last().copied().unwrap_or(corner);
    let mut out = Vec::with_capacity(edge.len() + 3);
    out.push(corner); // logical -2 == Edge[-1]
    out.extend_from_slice(edge); // logical -1..side-1
    out.push(last); // logical side == Edge[side - 1]
    out.push(last); // logical side + 1 == Edge[side - 1]
    out
}

#[cfg(test)]
#[path = "runtime_minimal_recon/tests.rs"]
mod tests;
