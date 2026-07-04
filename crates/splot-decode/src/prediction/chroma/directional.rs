// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Chroma directional / cardinal / smooth intra reconstructors for the runtime
//! tier.
//!
//! This is the chroma half of the general-intra reconstruction handoff: it
//! dispatches the resolved § 5.20.5.3 `UVMode` and reconstructs each chroma
//! transform block over the § 7.13.2.1 prediction edges read from the
//! partially-built frame. The luma reconstructors and shared edge / fallback
//! helpers stay in the parent [`super`] module and are reached through
//! `use super::*`.
//!
//! Feature tracking: `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER`.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraCardinalDirection, IntraDirectionalAngleEdges,
    IntraRectBlockSize, IntraSmoothMode, PlaneId, ReconSample,
    predict_intra_cardinal_directional_rect_into,
};

use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, SupportedChromaMode, SupportedDirectionalLumaMode,
    reconstruct_general_intra_block_with_prediction,
};

use crate::pipeline::reconstruct::*;

/// Reconstructs one chroma plane block in decode order into the workspace,
/// dispatching on the resolved § 5.20.5.3 `UVMode`:
///
/// - [`SupportedChromaMode::Dc`] delegates to the § 7.13.2.4 DC reconstruction
///   ([`reconstruct_general_intra_block_into`]).
/// - [`SupportedChromaMode::Smooth`] builds the § 7.13.2.1 `AboveRow` / `LeftCol`
///   edges from the partially-built frame's reconstructed neighbours (applying
///   the no-above / no-left / no-neighbour fallbacks), runs § 7.13.2.13 smooth
///   prediction, and adds the decoded residual (or writes the bare prediction
///   for an `all_zero` block).
///
/// `num4_above_right` is the § 7.13.2.1 `num4AboveRight` (in 4x4 units) for this
/// transform block, derived by the caller from § 5.20.7.25 `count_top_right_avail`
/// over the § 5.20.2.3 `BlockDecoded` state; it selects the SMOOTH top-right
/// sentinel `AboveRow[w]` between the real reconstructed above-right sample and
/// the clamped last in-block above sample. `num4_below_left` is the symmetric
/// § 7.13.2.1 `num4BelowLeft` (§ 5.20.7.25 `count_bottom_left_avail`) bounding the
/// real below-left for the D203-follow zone-3 chroma (`0` in raster order).
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    mode: SupportedChromaMode,
    num4_above_right: usize,
    num4_below_left: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    match mode {
        SupportedChromaMode::Dc => reconstruct_general_intra_block_rect_into(
            workspace,
            block,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            false,
            false,
            bit_depth,
        ),
        SupportedChromaMode::Smooth => reconstruct_general_intra_chroma_smooth_into(
            workspace,
            block,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            IntraSmoothMode::Smooth,
            num4_above_right,
            num4_below_left,
            bit_depth,
        ),
        SupportedChromaMode::SmoothVertical => reconstruct_general_intra_chroma_smooth_into(
            workspace,
            block,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            IntraSmoothMode::SmoothVertical,
            num4_above_right,
            num4_below_left,
            bit_depth,
        ),
        SupportedChromaMode::SmoothHorizontal => reconstruct_general_intra_chroma_smooth_into(
            workspace,
            block,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            IntraSmoothMode::SmoothHorizontal,
            num4_above_right,
            num4_below_left,
            bit_depth,
        ),
        SupportedChromaMode::Paeth => reconstruct_general_intra_luma_paeth_neighbour_block_into(
            workspace,
            block,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            false,
            bit_depth,
        ),
        SupportedChromaMode::D135Follow | SupportedChromaMode::D135 if x == 0 && y == 0 => {
            reconstruct_general_intra_chroma_directional_first_into(
                workspace, block, plane_id, x, y, log2_width, qindex, bit_depth,
            )
        }
        SupportedChromaMode::D135Follow | SupportedChromaMode::D135 => {
            reconstruct_general_intra_directional_neighbour_block_into(
                workspace,
                block,
                SupportedDirectionalLumaMode::D135,
                plane_id,
                x,
                y,
                log2_width,
                qindex,
                false,
                bit_depth,
            )
        }
        SupportedChromaMode::D113Follow | SupportedChromaMode::D113 => {
            reconstruct_general_intra_directional_neighbour_block_into(
                workspace,
                block,
                SupportedDirectionalLumaMode::D113,
                plane_id,
                x,
                y,
                log2_width,
                qindex,
                false,
                bit_depth,
            )
        }
        SupportedChromaMode::D157Follow | SupportedChromaMode::D157 => {
            reconstruct_general_intra_directional_neighbour_block_into(
                workspace,
                block,
                SupportedDirectionalLumaMode::D157,
                plane_id,
                x,
                y,
                log2_width,
                qindex,
                false,
                bit_depth,
            )
        }
        SupportedChromaMode::VerticalFollow | SupportedChromaMode::Vertical => {
            reconstruct_general_intra_cardinal_neighbour_block_into(
                workspace,
                block,
                IntraCardinalDirection::Vertical,
                plane_id,
                x,
                y,
                log2_width,
                log2_height,
                qindex,
                false,
                None,
                bit_depth,
            )
        }
        SupportedChromaMode::HorizontalFollow => {
            reconstruct_general_intra_cardinal_neighbour_block_into(
                workspace,
                block,
                IntraCardinalDirection::Horizontal,
                plane_id,
                x,
                y,
                log2_width,
                log2_height,
                qindex,
                false,
                None,
                bit_depth,
            )
        }
        SupportedChromaMode::Horizontal if x == 0 && y == 0 => {
            reconstruct_general_intra_chroma_cardinal_horizontal_first_into(
                workspace, block, plane_id, x, y, log2_width, qindex, bit_depth,
            )
        }
        SupportedChromaMode::Horizontal => reconstruct_general_intra_cardinal_neighbour_block_into(
            workspace,
            block,
            IntraCardinalDirection::Horizontal,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            false,
            None,
            bit_depth,
        ),
        SupportedChromaMode::D45Follow
        | SupportedChromaMode::D45
        | SupportedChromaMode::D67Follow
        | SupportedChromaMode::D67 => reconstruct_general_intra_one_sided_neighbour_block_into(
            workspace,
            block,
            match mode {
                SupportedChromaMode::D45Follow | SupportedChromaMode::D45 => 45,
                SupportedChromaMode::D67Follow | SupportedChromaMode::D67 => 67,
                _ => unreachable!(),
            },
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            num4_above_right,
            OneSidedAboveMrl::default(),
            false,
            None,
            bit_depth,
            OneSidedEdgeFilter::default(),
        ),
        SupportedChromaMode::D203Follow | SupportedChromaMode::D203 => {
            reconstruct_general_intra_one_sided_left_neighbour_block_into(
                workspace,
                block,
                203,
                plane_id,
                x,
                y,
                log2_width,
                log2_height,
                qindex,
                num4_below_left,
                false, // have_above: unchanged chroma §7.13.2.1 corner `CurrFrame[y][x-1]`
                0,     // mrl_index: chroma follow uses the immediate reference line
                false,
                None,
                bit_depth,
                OneSidedEdgeFilter::default(),
            )
        }
    }
}

/// Reconstructs one no-neighbour (top-left) directional-follow D135 chroma block
/// (§ 7.13.2.8 pAngle 135, `AngleDeltaUV == 0`) over the § 7.13.2.1 no-neighbour
/// fallback edges, adds the decoded residual (or writes the bare prediction for an
/// all-zero block), and stores the result into the workspace.
///
/// This is the chroma companion of
/// [`reconstruct_general_intra_luma_directional_first_block_into`]: the caller
/// gates it to the top-left no-neighbour block, where the chroma plane has no
/// above/left neighbour, so the § 7.13.2.1 prediction edges reduce to the flat
/// fallbacks (`AboveRow[k] = 127`, `LeftCol[k] = 129`, corner `128`) and the
/// `enable_intra_edge_filter` / IDIF / upsample edge synthesis is a no-op. pAngle
/// 135 has `dx == dy == Dr_Intra_Derivative[45] == 64`, so every projection lands
/// on an integer sample (`shift == 0`) and the § 7.13.2.8 bilinear middle-angle
/// predictor (`enableIdif == 0` for chroma, since `enableIdif = plane == 0`, so the
/// IDIF 4-tap is luma-only) is a sample copy of the flat fallback edge for this
/// angle (verified bit-exact against avmdec/dav2d). Chroma never uses the
/// § 7.14.4 TCQ dqDenom term (luma DCT_DCT only), so `use_tcq` is `false`.
#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_chroma_directional_first_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let prediction = predict_directional_noneighbour(
        SupportedDirectionalLumaMode::D135,
        block_size,
        side,
        bit_depth,
    )?;
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
            false,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Reconstructs one no-neighbour (top-left) cardinal H_PRED chroma block
/// (§ 7.13.2.8 step 5, pAngle 180) over the § 7.13.2.1 no-neighbour fallback left
/// column, adds the decoded residual (or writes the bare prediction for an
/// `all_zero` block), and stores the result.
///
/// At the no-neighbour top-left block § 7.13.2.1 has neither a real left nor a
/// real above neighbour, so `LeftCol[i]` is the flat no-left fallback
/// (`noneighbour_left`, `(1 << (BitDepth - 1)) + 1` — `129` for 8-bit,
/// `513` for 10-bit). The § 7.13.2.8 horizontal copy
/// `pred[i][j] = LeftCol[i]` therefore writes a flat prediction. The cardinal copy
/// has no IDIF, no corner, and no `useIBP` (§ 7.13.2.7 skips the edge filter for
/// `pAngle == 180`), so the flat-fallback prediction is exact; the caller gates
/// this path to the no-neighbour block (verified bit-exact against avmdec/dav2d).
#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_chroma_cardinal_horizontal_first_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let left = vec![noneighbour_left::<T>(bit_depth); side];
    let mut prediction = vec![T::default(); side * side];
    predict_intra_cardinal_directional_rect_into(
        bit_depth,
        block_size,
        IntraCardinalDirection::Horizontal,
        IntraDirectionalAngleEdges::left(&left),
        &mut prediction,
        side,
    )?;
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
            false,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

/// Reconstructs one § 7.13.2.13 `SMOOTH_PRED` chroma block over § 7.13.2.1 edges
/// read from the partially-built frame.
#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_chroma_smooth_into<T: ReconSample>(
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
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    reconstruct_general_intra_smooth_over_edges_into(
        workspace,
        block,
        plane_id,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        smooth_mode,
        num4_above_right,
        num4_below_left,
        false,
        None,
        bit_depth,
    )
}

/// Reconstructs one rectangular § 7.13.2.13 `SMOOTH_PRED` chroma block using
/// caller-provided § 7.13.2.1 availability counts for the already reconstructed
/// left and above edges.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_smooth_available_edges_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    smooth_mode: IntraSmoothMode,
    available_left_samples: usize,
    available_above_samples: usize,
    num4_above_right: usize,
    num4_below_left: usize,
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
        Some(available_left_samples),
        Some(available_above_samples),
        num4_above_right,
        num4_below_left,
        false,
        None,
        bit_depth,
    )
}
