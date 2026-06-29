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
    BitDepth, CurrentFrameWorkspace, IntraCardinalDirection, IntraCardinalEdges,
    IntraRectBlockSize, IntraSmoothMode, PlaneId, ReconSample,
    predict_intra_cardinal_directional_rect_into,
};

use crate::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, SupportedChromaMode, SupportedDirectionalLumaMode,
    reconstruct_general_intra_block_with_prediction,
};

use super::*;

/// Reconstructs one square chroma plane block in decode order into the
/// workspace, dispatching on the resolved § 5.20.5.3 `UVMode`:
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
    log2_side: u32,
    qindex: u32,
    mode: SupportedChromaMode,
    num4_above_right: usize,
    num4_below_left: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    match mode {
        // Chroma never uses the §7.14.4 TCQ dqDenom term (luma DCT_DCT only), so
        // `use_tcq` is false for both DC and SMOOTH chroma reconstruction.
        SupportedChromaMode::Dc => reconstruct_general_intra_block_into(
            workspace, block, plane_id, x, y, log2_side, qindex, false, bit_depth,
        ),
        SupportedChromaMode::Smooth => reconstruct_general_intra_chroma_smooth_into(
            workspace,
            block,
            plane_id,
            x,
            y,
            log2_side,
            qindex,
            num4_above_right,
            bit_depth,
        ),
        // Directional-follow D135 chroma. At the no-neighbour top-left block the
        // §7.13.2.1 edges reduce to the flat fallback and the §7.13.2.8 middle-angle
        // prediction is the `enableIdif == 0` bilinear sample copy
        // ([`reconstruct_general_intra_chroma_directional_first_into`]). For a
        // neighbour-having block the §7.13.2.1 edges are the **real reconstructed**
        // left column / above row; chroma always takes the §7.13.2.8 bilinear branch
        // (`enableIdif = plane == 0` is `0` for U/V), which for D135 (`shift == 0`)
        // is the same sample copy `Edge[base]` as the luma IDIF, bit-exact over a
        // non-flat edge. `num4_above_right` is the top-right sentinel, never read by
        // the D135 above/left projections.
        SupportedChromaMode::D135Follow if x == 0 && y == 0 => {
            reconstruct_general_intra_chroma_directional_first_into(
                workspace, block, plane_id, x, y, log2_side, qindex, bit_depth,
            )
        }
        SupportedChromaMode::D135Follow => {
            reconstruct_general_intra_directional_neighbour_block_into(
                workspace,
                block,
                SupportedDirectionalLumaMode::D135,
                plane_id,
                x,
                y,
                log2_side,
                qindex,
                // Chroma never uses the §7.14.4 TCQ dqDenom term (luma DCT_DCT only).
                false,
                bit_depth,
            )
        }
        // Directional-follow D113 chroma over the real reconstructed §7.13.2.1
        // above row / left column / corner. Chroma takes the `enableIdif == 0`
        // bilinear branch (`enableIdif = plane == 0`, `0` for U/V), which IS the
        // spec-mandated chroma branch, so it is bit-exact. The luma gate
        // guarantees the D113 block is at a row>0, non-first-column position, so
        // the chroma block has the matching real reconstructed neighbours (no
        // top-left no-neighbour D113 chroma path is admitted). The `(true, true)`
        // edge builder supplies the real diagonally-above-left chroma corner.
        SupportedChromaMode::D113Follow => {
            reconstruct_general_intra_directional_neighbour_block_into(
                workspace,
                block,
                SupportedDirectionalLumaMode::D113,
                plane_id,
                x,
                y,
                log2_side,
                qindex,
                // Chroma never uses the §7.14.4 TCQ dqDenom term (luma DCT_DCT only).
                false,
                bit_depth,
            )
        }
        // Directional-follow D157 chroma over a real reconstructed §7.13.2.1 left
        // chroma column. Chroma takes the `enableIdif == 0` bilinear branch
        // (`plane != 0`); over a flat real chroma edge the D157 bilinear projection
        // is bit-exact. The luma gate guarantees the D157 block is at a
        // first-superblock-row, non-first-column position, so the chroma block has
        // the matching real reconstructed left neighbour (no top-left no-neighbour
        // D157 chroma path is admitted).
        SupportedChromaMode::D157Follow => {
            reconstruct_general_intra_directional_neighbour_block_into(
                workspace,
                block,
                SupportedDirectionalLumaMode::D157,
                plane_id,
                x,
                y,
                log2_side,
                qindex,
                // Chroma never uses the §7.14.4 TCQ dqDenom term (luma DCT_DCT only).
                false,
                bit_depth,
            )
        }
        // Cardinal directional-follow V_PRED / H_PRED chroma: a degenerate copy of
        // the real reconstructed §7.13.2.1 above row (V) or left column (H). Chroma
        // uses no IDIF, so the copy is bit-exact over the non-flat real edge. The
        // luma cardinal gate guarantees the chroma block has the matching neighbour
        // (V follows a row>0 luma block, H a non-first-column luma block), and the
        // 4:2:0 chroma block at the half-resolution position has the same neighbour
        // availability. Chroma never uses the §7.14.4 TCQ dqDenom term.
        SupportedChromaMode::VerticalFollow => {
            reconstruct_general_intra_cardinal_neighbour_block_into(
                workspace,
                block,
                IntraCardinalDirection::Vertical,
                plane_id,
                x,
                y,
                log2_side,
                log2_side,
                qindex,
                false,
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
                log2_side,
                log2_side,
                qindex,
                false,
                bit_depth,
            )
        }
        // Non-follow H_PRED chroma at the no-neighbour top-left block: a horizontal
        // copy of the §7.13.2.1 flat no-left fallback. The caller gates this to the
        // no-neighbour block, so the flat-fallback prediction is exact.
        SupportedChromaMode::Horizontal => {
            reconstruct_general_intra_chroma_cardinal_horizontal_first_into(
                workspace, block, plane_id, x, y, log2_side, qindex, bit_depth,
            )
        }
        // Directional-follow D45 chroma (§7.13.2.8 ZONE-1 step 1, pAngle 45) over
        // the real reconstructed §7.13.2.1 chroma above row + above-right. Chroma
        // takes the `enableIdif == 0` bilinear one-sided branch (`enableIdif =
        // plane == 0` is `0` for U/V), which for D45 (`shift == 0`) is the sample
        // copy `AboveRow[base]`. The luma gate guarantees the D45 block is at a
        // row>0, non-first-column, non-rightmost position with a real decoded
        // above-right, so the half-resolution chroma block has the matching real
        // neighbours. Chroma never uses the §7.14.4 TCQ dqDenom term.
        SupportedChromaMode::D45Follow => reconstruct_general_intra_one_sided_neighbour_block_into(
            workspace,
            block,
            // D45-follow (`AngleDeltaUV == AngleDeltaY == 0`): pAngle 45.
            45,
            plane_id,
            x,
            y,
            log2_side,
            qindex,
            num4_above_right,
            false,
            bit_depth,
            // The chroma follow path is the no-edge-filter subset (this sink does
            // not model the §7.13.2.7 chroma `UVSmooth` filter-type state): a
            // default no-op leaves the raw §7.13.2.1 edge unchanged.
            OneSidedEdgeFilter::default(),
        ),
        // Directional-follow D203 chroma (§7.13.2.8 ZONE-3 step 3, pAngle 203) over
        // the real reconstructed §7.13.2.1 chroma left column + below-left. Chroma
        // takes the `enableIdif == 0` bilinear one-sided branch (`enableIdif =
        // plane == 0` is `0` for U/V), the spec-mandated chroma branch. The luma
        // gate guarantees the D203 block is at a first-superblock-row,
        // non-first-column position with a real reconstructed left column, so the
        // half-resolution chroma block has the matching real left neighbour.
        // Chroma never uses the §7.14.4 TCQ dqDenom term.
        SupportedChromaMode::D203Follow => {
            reconstruct_general_intra_one_sided_left_neighbour_block_into(
                workspace,
                block,
                // D203-follow (`AngleDeltaUV == AngleDeltaY == 0`): pAngle 203.
                203,
                plane_id,
                x,
                y,
                log2_side,
                qindex,
                num4_below_left,
                false,
                bit_depth,
                // No-edge-filter chroma follow subset (see D45Follow above).
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
    // §7.13.2.1 no-left fallback: `LeftCol[i] = noneighbour_left` for all rows.
    let left = vec![noneighbour_left::<T>(bit_depth); side];
    let mut prediction = vec![T::default(); side * side];
    predict_intra_cardinal_directional_rect_into(
        bit_depth,
        block_size,
        IntraCardinalDirection::Horizontal,
        IntraCardinalEdges::left(&left),
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
            // Chroma never uses the §7.14.4 TCQ dqDenom term (luma DCT_DCT only).
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
    log2_side: u32,
    qindex: u32,
    num4_above_right: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    // Chroma never uses the §7.14.4 TCQ dqDenom term (luma DCT_DCT only).
    reconstruct_general_intra_smooth_over_edges_into(
        workspace,
        block,
        plane_id,
        x,
        y,
        log2_side,
        qindex,
        IntraSmoothMode::Smooth,
        num4_above_right,
        false,
        bit_depth,
    )
}
