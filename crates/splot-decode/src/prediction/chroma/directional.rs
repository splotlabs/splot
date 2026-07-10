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
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DpcmDirection, IntraCardinalDirection,
    IntraDirectionalAngleEdges, IntraRectBlockSize, IntraSmoothMode, PlaneId, ReconSample,
    predict_intra_cardinal_directional_rect_into,
};

use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, SupportedChromaMode, SupportedDirectionalLumaMode,
    reconstruct_general_intra_coeff_block_rect_with_prediction,
    reconstruct_general_intra_coeff_block_with_prediction,
};

use crate::pipeline::reconstruct::*;

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
    dpcm: Option<DpcmDirection>,
    num4_above_right: usize,
    num4_below_left: usize,
    ibp_dc: bool,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    match mode {
        SupportedChromaMode::Dc => reconstruct_general_intra_block_rect_with_availability_into(
            workspace,
            block,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            false,
            ibp_dc,
            None,
            availability,
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
            availability,
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
            availability,
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
            availability,
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
            availability,
            bit_depth,
        ),
        SupportedChromaMode::D135Follow | SupportedChromaMode::D135 if x == 0 && y == 0 => {
            reconstruct_general_intra_chroma_directional_first_into(
                workspace,
                block,
                SupportedDirectionalLumaMode::D135,
                plane_id,
                x,
                y,
                log2_width,
                qindex,
                bit_depth,
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
                None,
                bit_depth,
                MiddleEdgeAvailability {
                    above: availability.above,
                    left: availability.left,
                },
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
                None,
                bit_depth,
                MiddleEdgeAvailability {
                    above: availability.above,
                    left: availability.left,
                },
            )
        }
        SupportedChromaMode::D157 if x == 0 && y == 0 => {
            reconstruct_general_intra_chroma_directional_first_into(
                workspace,
                block,
                SupportedDirectionalLumaMode::D157,
                plane_id,
                x,
                y,
                log2_width,
                qindex,
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
                None,
                bit_depth,
                MiddleEdgeAvailability {
                    above: availability.above,
                    left: availability.left,
                },
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
                dpcm,
                availability,
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
                dpcm,
                availability,
                bit_depth,
            )
        }
        SupportedChromaMode::Horizontal if x == 0 && y == 0 => {
            reconstruct_general_intra_chroma_cardinal_horizontal_first_into(
                workspace,
                block,
                plane_id,
                x,
                y,
                log2_width,
                log2_height,
                qindex,
                dpcm,
                bit_depth,
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
            dpcm,
            availability,
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
            dpcm,
            availability,
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
                dpcm,
                availability,
                bit_depth,
                OneSidedEdgeFilter::default(),
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_chroma_directional_first_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedDirectionalLumaMode,
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
    let prediction = predict_directional_noneighbour(mode, block_size, side, bit_depth)?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_coeff_block_with_prediction(
            block,
            &prediction,
            qindex,
            plane_id,
            log2_side,
            false,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_chroma_cardinal_horizontal_first_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let left = vec![noneighbour_left::<T>(bit_depth); height];
    let mut prediction = vec![T::default(); width * height];
    predict_intra_cardinal_directional_rect_into(
        bit_depth,
        block_size,
        IntraCardinalDirection::Horizontal,
        IntraDirectionalAngleEdges::left(&left),
        &mut prediction,
        width,
    )?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_coeff_block_rect_with_prediction(
            block,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            false,
            dpcm,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

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
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (available_left_samples, available_above_samples) = availability.available_sample_limits();
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
        available_left_samples,
        available_above_samples,
        num4_above_right,
        num4_below_left,
        false,
        None,
        bit_depth,
    )
}
