// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal traced reconstruction handoff for the documented runtime tier.
//!
//! Feature tracking: `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER`.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, IntraCardinalDirection,
    IntraCardinalEdges, IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraRectBlockSize, IntraSmoothEdges, IntraSmoothMode, IntraSquareBlockSize, OutputIndex,
    PixelFormat, PlaneId, PlaneRect, PlaneSize, predict_intra_cardinal_directional_rect_into,
    predict_intra_dc_rect_value, predict_intra_middle_directional_angle_rect_into,
    predict_intra_smooth_rect_into,
};

use crate::Result;
use crate::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, MinimalRuntimeReconstructionTrace,
    SupportedChromaMode, SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
    reconstruct_general_intra_block, reconstruct_general_intra_block_with_prediction,
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

    // AV2 §7.13.2.1 uses (1 << (BitDepth - 1)) + 1 for LeftCol when no
    // neighbor is available. The traced top-left chroma blocks use H_PRED
    // (pAngle 180 via §7.13.2.8 and §9.2), so prepare that left edge
    // explicitly for this narrow minimal tier instead of claiming broad edge
    // preparation.
    let chroma_block = IntraRectBlockSize::new(MINIMAL_CHROMA_LOG2_SIZE, MINIMAL_CHROMA_LOG2_SIZE)?;
    let chroma_left = [TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE; MINIMAL_CHROMA_HEIGHT];
    let mut chroma_prediction = [0u8; MINIMAL_CHROMA_WIDTH * MINIMAL_CHROMA_HEIGHT];
    predict_intra_cardinal_directional_rect_into(
        BitDepth::Eight,
        chroma_block,
        IntraCardinalDirection::Horizontal,
        IntraCardinalEdges::left(&chroma_left),
        &mut chroma_prediction,
        MINIMAL_CHROMA_WIDTH,
    )?;
    workspace.write_rect_block(PlaneId::U, 0, 0, chroma_block, &chroma_prediction)?;
    workspace.write_rect_block(PlaneId::V, 0, 0, chroma_block, &chroma_prediction)?;

    Ok(workspace.freeze()?)
}

/// Creates an empty decoded 8-bit 4:2:0 frame workspace sized to the actual
/// `luma_width` x `luma_height` (a positive multiple of 64) for incremental
/// per-block reconstruction on the general intra multi-block path. Chroma is
/// 4:2:0 (half-resolution), so the chroma plane is `luma_width / 2` x
/// `luma_height / 2`, derived internally by [`PixelFormat::Yuv420`].
pub(crate) fn new_general_intra_workspace(
    luma_width: usize,
    luma_height: usize,
) -> Result<CurrentFrameWorkspace<u8>> {
    let luma_size = PlaneSize::new(luma_width, luma_height)?;
    let luma_rect = PlaneRect::new(0, 0, luma_width, luma_height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        luma_rect,
    )?;
    Ok(CurrentFrameWorkspace::<u8>::new(info, 0)?)
}

/// Reconstructs one square plane block in decode order into the workspace: the
/// § 7.13.2 DC prediction is read from the partially-built frame's neighbours
/// (`128` fallback when none); an `all_zero` block writes the flat prediction,
/// otherwise the dequant / inverse-transform / residual-add reconstruction is
/// added; the result is written back so later blocks read it as a neighbour.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2).map_err(recon_err)?;
    let edges = workspace
        .intra_dc_edges_for_rect(plane_id, x, y, block_size)
        .map_err(recon_err)?;
    let dc = predict_intra_dc_rect_value(BitDepth::Eight, block_size, edges.as_dc_edges())
        .map_err(recon_err)?;
    let out = if block.all_zero {
        vec![dc; side * side]
    } else {
        reconstruct_general_intra_block(&block.quant, dc, qindex, plane_id, log2_side, use_tcq)?
    };
    workspace
        .write_rect_block(plane_id, x, y, block_size, &out)
        .map_err(recon_err)?;
    Ok(())
}

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
/// the clamped last in-block above sample.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_block_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    mode: SupportedChromaMode,
    num4_above_right: usize,
) -> core::result::Result<(), GeneralIntraResidualError> {
    match mode {
        // Chroma never uses the §7.14.4 TCQ dqDenom term (luma DCT_DCT only), so
        // `use_tcq` is false for both DC and SMOOTH chroma reconstruction.
        SupportedChromaMode::Dc => reconstruct_general_intra_block_into(
            workspace, block, plane_id, x, y, log2_side, qindex, false,
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
                workspace, block, plane_id, x, y, log2_side, qindex,
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
fn reconstruct_general_intra_chroma_directional_first_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2).map_err(recon_err)?;
    let prediction =
        predict_directional_noneighbour(SupportedDirectionalLumaMode::D135, block_size, side)?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_side,
            false,
        )?
    };
    workspace
        .write_rect_block(plane_id, x, y, block_size, &out)
        .map_err(recon_err)?;
    Ok(())
}

/// Reconstructs one § 7.13.2.13 `SMOOTH_PRED` chroma block over § 7.13.2.1 edges
/// read from the partially-built frame.
#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_chroma_smooth_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    num4_above_right: usize,
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
    )
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
pub(crate) fn reconstruct_general_intra_luma_nondc_neighbour_block_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    mode: SupportedNonDcLumaMode,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    num4_above_right: usize,
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
        qindex,
        smooth_mode,
        num4_above_right,
        use_tcq,
    )
}

/// Reconstructs one § 7.13.2.13 smooth block (`SMOOTH_PRED` / `SMOOTH_V_PRED` /
/// `SMOOTH_H_PRED`) over § 7.13.2.1 edges read from the partially-built frame's
/// reconstructed neighbours, for any plane. The § 7.13.2.1 edge derivation is
/// plane-independent (it reads the workspace neighbour samples and applies the
/// no-above / no-left / no-neighbour fallbacks); the caller selects the smooth
/// mode and whether luma TCQ dequant applies.
#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_smooth_over_edges_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    smooth_mode: IntraSmoothMode,
    num4_above_right: usize,
    use_tcq: bool,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2).map_err(recon_err)?;
    let edges = workspace
        .intra_dc_edges_for_rect(plane_id, x, y, block_size)
        .map_err(recon_err)?;
    // §7.13.2.1 `haveLeft` / `haveAbove`: in this single-tile minimal path a
    // reconstructed neighbour exists exactly when the block is not at the frame
    // edge, which `intra_dc_edges_for_rect` reports as a present left/above edge.
    let have_left = edges.left_samples().is_some();
    let have_above = edges.above_samples().is_some();
    // §7.13.2.1 top-right sentinel `AboveRow[w]`: when haveAbove and the
    // above-right is decoded (`num4AboveRight > 0`), read the real reconstructed
    // `CurrFrame[plane][y-1][Min(aboveLimit, x+w)]`; otherwise the no-above
    // fallback / clamped last in-block sample is used (built below).
    let above_right_sentinel = resolve_smooth_above_right_sentinel(
        workspace,
        plane_id,
        x,
        y,
        side,
        have_above,
        num4_above_right,
    )?;
    let (left, above) = build_smooth_edges(
        edges.left_samples(),
        edges.above_samples(),
        have_left,
        have_above,
        side,
        above_right_sentinel,
    )?;
    let smooth_edges = IntraSmoothEdges::new(&left, &above);
    let mut prediction = vec![0u8; side * side];
    predict_intra_smooth_rect_into(
        BitDepth::Eight,
        block_size,
        smooth_mode,
        smooth_edges,
        &mut prediction,
        side,
    )
    .map_err(recon_err)?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_side,
            use_tcq,
        )?
    };
    workspace
        .write_rect_block(plane_id, x, y, block_size, &out)
        .map_err(recon_err)?;
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
fn build_smooth_edges(
    left_neighbour: Option<&[u8]>,
    above_neighbour: Option<&[u8]>,
    have_left: bool,
    have_above: bool,
    side: usize,
    above_right_sentinel: Option<u8>,
) -> core::result::Result<(Vec<u8>, Vec<u8>), GeneralIntraResidualError> {
    let edge_len = side + 1;
    // §7.13.2.1 `LeftCol[i]`: reconstructed left column when haveLeft; else when
    // haveAbove, the above neighbour's first sample; else the no-left fallback.
    // The bottom-left sentinel `LeftCol[h]` keeps the clamped last left sample:
    // in raster decode order a full-superblock block's below-left is never
    // decoded yet (`num4BelowLeft == 0`), so the spec value
    // `CurrFrame[plane][Min(maxY, y+h)][x-1]` equals the clamped last sample.
    let left = match (have_left, left_neighbour) {
        (true, Some(samples)) => fill_edge_from_neighbour(samples, edge_len),
        _ if have_above => {
            let seed = above_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(NONEIGHBOUR_LEFT_8BIT);
            vec![seed; edge_len]
        }
        _ => vec![NONEIGHBOUR_LEFT_8BIT; edge_len],
    };
    // §7.13.2.1 `AboveRow[i]`: reconstructed above row when haveAbove; else when
    // haveLeft, the left neighbour's first sample; else the no-above fallback.
    let mut above = match (have_above, above_neighbour) {
        (true, Some(samples)) => fill_edge_from_neighbour(samples, edge_len),
        _ if have_left => {
            let seed = left_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(NONEIGHBOUR_ABOVE_8BIT);
            vec![seed; edge_len]
        }
        _ => vec![NONEIGHBOUR_ABOVE_8BIT; edge_len],
    };
    // §7.13.2.1 top-right sentinel `AboveRow[w]` (index `side`): overwrite the
    // clamped last in-block sample with the real reconstructed above-right sample
    // when the caller resolved one (above-right decoded, in-frame).
    if let Some(sentinel) = above_right_sentinel
        && let Some(slot) = above.get_mut(side)
    {
        *slot = sentinel;
    }
    Ok((left, above))
}

/// Resolves the AV2 § 7.13.2.1 top-right sentinel `AboveRow[w]` for a SMOOTH
/// chroma block in this single-tile minimal path.
///
/// Per § 7.13.2.1, when `haveAbove == 1` the sentinel is
/// `CurrFrame[plane][y - 1][Min(aboveLimit, x + w)]` with
/// `aboveLimit = Min(maxX, x + w + 4 * num4AboveRight - 1)` (8-bit, `MrlIndex == 0`,
/// `aboveMrlIndex == 0`). When `num4AboveRight == 0` (no decoded above-right) or
/// the block already touches the chroma frame right edge (`x + w > maxX`), this
/// reduces to the clamped last in-block above sample, so `None` is returned to
/// keep the [`build_smooth_edges`] clamp. When `haveAbove == 0` the
/// sentinel is not read from the above-right at all, so `None` is returned.
fn resolve_smooth_above_right_sentinel(
    workspace: &CurrentFrameWorkspace<u8>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    side: usize,
    have_above: bool,
    num4_above_right: usize,
) -> core::result::Result<Option<u8>, GeneralIntraResidualError> {
    if !have_above || num4_above_right == 0 {
        return Ok(None);
    }
    // §7.13.2.1: maxX = ((MiCols * MI_SIZE) >> SubsamplingX) - 1, i.e. the chroma
    // frame right column. The chroma workspace plane storage width equals the
    // chroma frame width for these multiple-of-64 frames, so its last column is
    // `maxX`.
    let plane = workspace.plane(plane_id).map_err(recon_err)?;
    let storage_width = plane.storage_size().width();
    let max_x = match storage_width.checked_sub(1) {
        Some(value) => value,
        None => return Ok(None),
    };
    let above_row = match y.checked_sub(1) {
        Some(value) => value,
        None => return Ok(None),
    };
    let x_plus_w = x.saturating_add(side);
    // aboveLimit = Min(maxX, x + w + 4 * num4AboveRight - 1). Since
    // num4AboveRight >= 1, `x + w + 4*num4AboveRight - 1 >= x + w`, so the
    // sentinel column `Min(aboveLimit, x + w)` simplifies to `Min(maxX, x + w)`.
    let sentinel_col = x_plus_w.min(max_x);
    // When the block already touches the frame right edge (`x + w > maxX`) the
    // sentinel collapses to the clamped last in-block sample (`maxX` would be the
    // block's own last column), which the clamp already supplies; leave it.
    if x_plus_w > max_x {
        return Ok(None);
    }
    let sentinel = workspace
        .reconstructed_sample(plane_id, sentinel_col, above_row)
        .map_err(recon_err)?;
    Ok(Some(sentinel))
}

/// Copies `samples` into a length-`edge_len` edge, repeating the last sample to
/// fill the trailing § 7.13.2.13 sentinel slot(s) (§ 7.13.2.1 edge extension).
fn fill_edge_from_neighbour(samples: &[u8], edge_len: usize) -> Vec<u8> {
    let mut edge = Vec::with_capacity(edge_len);
    for i in 0..edge_len {
        let sample = samples
            .get(i)
            .or_else(|| samples.last())
            .copied()
            .unwrap_or(NONEIGHBOUR_LEFT_8BIT);
        edge.push(sample);
    }
    edge
}

/// AV2 § 7.13.2.1 no-neighbour fallback (8-bit, `haveAbove == 0 && haveLeft == 0`):
/// every `AboveRow` sample is `(1 << (BitDepth - 1)) - 1` and every `LeftCol`
/// sample is `(1 << (BitDepth - 1)) + 1`.
const NONEIGHBOUR_ABOVE_8BIT: u8 = (1 << 7) - 1;
const NONEIGHBOUR_LEFT_8BIT: u8 = (1 << 7) + 1;

/// Reconstructs one no-neighbour (top-left) non-DC luma block: builds the
/// § 7.13.2.13 smooth prediction over the § 7.13.2.1 no-neighbour fallback edges,
/// adds the decoded AC residual (or writes the bare prediction for an all-zero
/// block), and stores the result into the workspace.
///
/// This path is gated to the top-left block (no above/left neighbours), so the
/// edges are pure § 7.13.2.1 fallbacks; multi-block non-DC prediction (which
/// reads reconstructed neighbours) is a future increment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_nondc_first_block_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    mode: SupportedNonDcLumaMode,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2).map_err(recon_err)?;
    let prediction = predict_nondc_noneighbour_smooth(mode, block_size, side)?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_side,
            use_tcq,
        )?
    };
    workspace
        .write_rect_block(PlaneId::Y, x, y, block_size, &out)
        .map_err(recon_err)?;
    Ok(())
}

/// Builds the § 7.13.2.13 smooth prediction for a no-neighbour square block over
/// the § 7.13.2.1 fallback edges (above `127`, left `129`; the smooth sentinels
/// `above[w]` / `left[h]` share those fallbacks).
fn predict_nondc_noneighbour_smooth(
    mode: SupportedNonDcLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
) -> core::result::Result<Vec<u8>, GeneralIntraResidualError> {
    let smooth_mode = match mode {
        SupportedNonDcLumaMode::Smooth => IntraSmoothMode::Smooth,
        SupportedNonDcLumaMode::SmoothVertical => IntraSmoothMode::SmoothVertical,
        SupportedNonDcLumaMode::SmoothHorizontal => IntraSmoothMode::SmoothHorizontal,
    };
    let above = vec![NONEIGHBOUR_ABOVE_8BIT; side + 1];
    let left = vec![NONEIGHBOUR_LEFT_8BIT; side + 1];
    let edges = IntraSmoothEdges::new(&left, &above);
    let mut out = vec![0u8; side * side];
    predict_intra_smooth_rect_into(
        BitDepth::Eight,
        block_size,
        smooth_mode,
        edges,
        &mut out,
        side,
    )
    .map_err(recon_err)?;
    Ok(out)
}

/// AV2 § 7.13.2.1 no-neighbour top-left corner sample
/// (`AboveRow[-1] == LeftCol[-1] == 1 << (BitDepth - 1)`), 8-bit.
const NONEIGHBOUR_CORNER_8BIT: u8 = 1 << 7;

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
pub(crate) fn reconstruct_general_intra_luma_directional_first_block_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    mode: SupportedDirectionalLumaMode,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2).map_err(recon_err)?;
    let prediction = predict_directional_noneighbour(mode, block_size, side)?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_side,
            use_tcq,
        )?
    };
    workspace
        .write_rect_block(PlaneId::Y, x, y, block_size, &out)
        .map_err(recon_err)?;
    Ok(())
}

/// Builds the § 7.13.2.8 directional prediction for a no-neighbour square block
/// over the § 7.13.2.1 fallback edges. The middle-angle predictor takes logical
/// edges whose index 0 is the `-1` sample: `above_with_minus_one[0]` /
/// `left_with_minus_one[0]` are the shared corner `128`, the remaining above
/// samples are `127` and left samples are `129`.
fn predict_directional_noneighbour(
    mode: SupportedDirectionalLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
) -> core::result::Result<Vec<u8>, GeneralIntraResidualError> {
    let angle = match mode {
        SupportedDirectionalLumaMode::D135 => IntraMiddleDirectionalAngle::D135,
    };
    // Logical `AboveRow[-1..w)` and `LeftCol[-1..h)` (length side + 1): index 0
    // is the corner; index `k + 1` is logical `k`.
    let mut above = vec![NONEIGHBOUR_ABOVE_8BIT; side + 1];
    let mut left = vec![NONEIGHBOUR_LEFT_8BIT; side + 1];
    above[0] = NONEIGHBOUR_CORNER_8BIT;
    left[0] = NONEIGHBOUR_CORNER_8BIT;
    let edges = IntraMiddleDirectionalAngleEdges::both(&left, &above);
    let mut out = vec![0u8; side * side];
    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        block_size,
        angle,
        edges,
        &mut out,
        side,
    )
    .map_err(recon_err)?;
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
pub(crate) fn reconstruct_general_intra_directional_neighbour_block_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: &LumaCoeffBlock,
    mode: SupportedDirectionalLumaMode,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2).map_err(recon_err)?;
    let edges = workspace
        .intra_dc_edges_for_rect(plane_id, x, y, block_size)
        .map_err(recon_err)?;
    // §7.13.2.1 `haveLeft` / `haveAbove`: a reconstructed neighbour exists exactly
    // when the block is not at the frame edge, which `intra_dc_edges_for_rect`
    // reports as a present left/above edge.
    let have_left = edges.left_samples().is_some();
    let have_above = edges.above_samples().is_some();
    let (left, above) =
        build_directional_middle_edges(edges.left_samples(), have_left, have_above, side)?;
    let angle = match mode {
        SupportedDirectionalLumaMode::D135 => IntraMiddleDirectionalAngle::D135,
    };
    let middle_edges = IntraMiddleDirectionalAngleEdges::both(&left, &above);
    let mut prediction = vec![0u8; side * side];
    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        block_size,
        angle,
        middle_edges,
        &mut prediction,
        side,
    )
    .map_err(recon_err)?;
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_side,
            use_tcq,
        )?
    };
    workspace
        .write_rect_block(plane_id, x, y, block_size, &out)
        .map_err(recon_err)?;
    Ok(())
}

/// Builds the AV2 § 7.13.2.1 logical `LeftCol[-1..h)` and `AboveRow[-1..w)` edges
/// (8-bit, `MrlIndex == 0`, no DIP/MRL/edge-filter) for § 7.13.2.8 middle-angle
/// directional prediction over reconstructed neighbours, for any plane. Each
/// returned slice has length `side + 1`: index `0` is the logical `-1` (corner)
/// sample and index `k + 1` is logical index `k`.
///
/// The corner `AboveRow[-1] == LeftCol[-1]` and the `AboveRow[i]` / `LeftCol[i]`
/// fills follow § 7.13.2.1 exactly for the minimal-tool subset:
/// - `haveAbove` (either `haveLeft && haveAbove` or `!haveLeft && haveAbove`):
///   NOT supported, returns [`GeneralIntraResidualError::UnsupportedDirectionalAboveEdge`].
///   § 7.13.2.8 D135 DOES read the § 7.13.2.1 corner `AboveRow[-1] == LeftCol[-1]` on
///   its main diagonal (`column == row` gives `above_base == -1`, `shift == 0`), so a
///   real above neighbour needs the real corner `CurrFrame[plane][y-1][x-1]`, which
///   `intra_dc_edges_for_rect` does not return. These arms are unreachable today (the
///   row>0 directional path is gated out, `general_intra_multirow_directional_luma`);
///   they reject rather than build a wrong-but-plausible corner, so relaxing the gate
///   must first reconstruct the real corner.
/// - `haveLeft && !haveAbove` (the committed fixture's row-0 D135 block): the only
///   reachable neighbour path. `LeftCol[i]` is the reconstructed left column (clamped
///   at the bottom-left sentinel, `num4BelowLeft == 0` in raster order), `AboveRow[i]`
///   is the repeated first left sample `CurrFrame[plane][y][x-1]`, and the corner
///   `AboveRow[-1] = LeftCol[-1] = CurrFrame[plane][y][x-1]` (the first left sample) —
///   the exact § 7.13.2.1 `haveLeft && !haveAbove` value, so the corner read on the
///   diagonal is correct and the committed fixture is bit-exact.
/// - neither: the § 7.13.2.1 flat fallbacks (`AboveRow 127`, `LeftCol 129`, corner
///   `128`) — the no-neighbour path handled by `predict_directional_noneighbour`.
fn build_directional_middle_edges(
    left_neighbour: Option<&[u8]>,
    have_left: bool,
    have_above: bool,
    side: usize,
) -> core::result::Result<(Vec<u8>, Vec<u8>), GeneralIntraResidualError> {
    let edge_len = side + 1;
    // §7.13.2.1 LeftCol[i] (logical 0..side-1 -> slice index 1..=side) and the
    // corner LeftCol[-1] (slice index 0).
    let mut left = Vec::with_capacity(edge_len);
    let mut above = Vec::with_capacity(edge_len);
    match (have_left, have_above) {
        (true, true) => {
            // A real reconstructed ABOVE neighbour (`haveAbove == 1`) only arises for
            // a row>0 directional block, which the admission gate rejects
            // (`general_intra_multirow_directional_luma`) BEFORE reconstruction, so
            // this arm is currently unreachable. It is NOT a no-corner case: §7.13.2.8
            // D135 reads the §7.13.2.1 corner `AboveRow[-1] == LeftCol[-1]` on its main
            // diagonal (`column == row` gives `above_base == -1`, `shift == 0`, so the
            // predictor copies the corner). The real corner is
            // `CurrFrame[plane][y-1][x-1]`, which `intra_dc_edges_for_rect` does not
            // return. Reject rather than build a wrong-but-plausible corner, so
            // relaxing the row>0 gate must first reconstruct the real corner.
            return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
        }
        (true, false) => {
            // §7.13.2.1 haveLeft && !haveAbove: AboveRow[i] = CurrFrame[plane][y][x-1]
            // (the first left sample, repeated), corner AboveRow[-1] = LeftCol[-1] =
            // CurrFrame[plane][y][x-1] (also the first left sample). LeftCol[i] is the
            // reconstructed left column.
            let left_samples = left_neighbour.unwrap_or(&[]);
            let seed = left_samples
                .first()
                .copied()
                .unwrap_or(NONEIGHBOUR_LEFT_8BIT);
            left.push(seed);
            above.push(seed);
            for i in 0..side {
                left.push(sample_or_last(left_samples, i, NONEIGHBOUR_LEFT_8BIT));
                above.push(seed);
            }
        }
        (false, true) => {
            // Same as `(true, true)`: a real ABOVE neighbour needs the §7.13.2.1
            // corner `CurrFrame[plane][y-1][x-1]` that D135 reads on its diagonal;
            // unreachable today (row>0 directional is gated out) and rejected here so
            // it cannot silently produce a wrong corner.
            return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
        }
        (false, false) => {
            // §7.13.2.1 no-neighbour fallbacks (handled by the first-block path, but
            // kept total here): AboveRow 127, LeftCol 129, shared corner 128.
            left.push(NONEIGHBOUR_CORNER_8BIT);
            above.push(NONEIGHBOUR_CORNER_8BIT);
            for _ in 0..side {
                left.push(NONEIGHBOUR_LEFT_8BIT);
                above.push(NONEIGHBOUR_ABOVE_8BIT);
            }
        }
    }
    Ok((left, above))
}

/// Returns `samples[index]`, falling back to the last sample (§ 7.13.2.1 bottom-left
/// / right-most edge clamp) and then to `fallback` for an empty slice.
fn sample_or_last(samples: &[u8], index: usize, fallback: u8) -> u8 {
    samples
        .get(index)
        .or_else(|| samples.last())
        .copied()
        .unwrap_or(fallback)
}

fn recon_err(source: splot_recon::ReconError) -> GeneralIntraResidualError {
    GeneralIntraResidualError::Reconstruct { source }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_recon::DecodedFrameHashInput;

    use super::*;

    const EXPECTED_DIGEST: &str =
        "dd244844938e78b226240de27e9c0acd39fc7ec2c1631319d13250fbe5f08496";

    fn reconstruct() -> DecodedFrame<u8> {
        reconstruct_minimal_traced_frame(
            MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64,
        )
        .unwrap()
    }

    #[test]
    fn traced_luma_dc_chroma_h_pred_reconstruction_predicts_visible_samples() {
        let frame = reconstruct();

        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());
        assert_eq!(
            frame.u().unwrap().visible_size(),
            PlaneSize::new(32, 32).unwrap()
        );
        assert_eq!(
            frame.v().unwrap().visible_size(),
            PlaneSize::new(32, 32).unwrap()
        );
        assert!(frame.y().samples().iter().all(|sample| *sample == 128));
        assert!(
            frame
                .u()
                .unwrap()
                .samples()
                .iter()
                .all(|sample| *sample == TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE)
        );
        assert!(
            frame
                .v()
                .unwrap()
                .samples()
                .iter()
                .all(|sample| *sample == TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE)
        );
        assert!(!frame.y().samples().contains(&0));
        assert!(!frame.u().unwrap().samples().contains(&0));
        assert!(!frame.v().unwrap().samples().contains(&0));
    }

    #[test]
    fn traced_luma_dc_chroma_h_pred_reconstruction_hash_matches_minimal_contract() {
        let frame = reconstruct();
        let hash = DecodedFrameHashInput::new(&frame).compute_hash();

        assert_eq!(hash.to_hex(), EXPECTED_DIGEST);
    }
}
