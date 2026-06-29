// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal traced reconstruction handoff for the documented runtime tier.
//!
//! Feature tracking: `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER`.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, IntraCardinalDirection,
    IntraCardinalEdges, IntraDirectionalAngle, IntraDirectionalAngleEdges,
    IntraDirectionalAngleIdifEdges, IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraMiddleDirectionalAngleIdifEdges, IntraPaethEdges, IntraRectBlockSize, IntraSmoothEdges,
    IntraSmoothMode, IntraSquareBlockSize, OutputIndex, PixelFormat, PlaneId, PlaneRect, PlaneSize,
    ReconSample, apply_intra_ibp_dc_rect, predict_intra_cardinal_directional_rect_into,
    predict_intra_dc_rect_value, predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_into,
    predict_intra_middle_directional_angle_rect_idif_into,
    predict_intra_middle_directional_angle_rect_into, predict_intra_paeth_rect_into,
    predict_intra_smooth_rect_into,
};

use crate::Result;
use crate::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, MinimalRuntimeReconstructionTrace,
    SupportedDirectionalLumaMode, SupportedNonDcLumaMode, reconstruct_general_intra_block,
    reconstruct_general_intra_block_rect_with_prediction,
    reconstruct_general_intra_block_with_prediction,
};

mod chroma_directional;

pub(crate) use chroma_directional::reconstruct_general_intra_chroma_block_into;

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
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let dc = predict_intra_dc_rect_value(bit_depth, block_size, edges.as_dc_edges())?;
    let out = if block.all_zero {
        vec![dc; side * side]
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
    // §7.13.2.10 produces a flat DC prediction; when the §7.13.2.12 IBP DC gate
    // (`enable_ibp && useDip == 0 && DC_PRED && !(w == 4 && h == 4) && (plane 0 ||
    // UVMode != CfL)`) holds, blend the edge rows/columns toward the reconstructed
    // neighbours BEFORE adding the residual. The blend is a validated no-op over
    // uniform neighbours (the value blends toward itself), so a block whose left /
    // above column is flat reconstructs identically with or without it.
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
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if block.all_zero {
        // §5.20.7.27 all_zero == 1: no residual, the MC prediction is the
        // reconstruction. Leave the workspace prediction in place.
        return Ok(());
    }
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let rect = PlaneRect::new(x, y, side, side)?;
    // Gather the §7.13.3.18 motion-compensated prediction for this block.
    let mut prediction = Vec::with_capacity(side * side);
    for row in workspace.rect_rows(plane_id, rect)? {
        prediction.extend_from_slice(row);
    }
    let out = reconstruct_general_intra_block_with_prediction(
        &block.quant,
        &prediction,
        qindex,
        plane_id,
        log2_side,
        block.plane_tx_type,
        use_tcq,
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
        // §5.20.7.27 all_zero == 1: no residual, the displaced copy IS the
        // reconstruction. Leave the workspace prediction in place.
        return Ok(());
    }
    let rect = PlaneRect::new(x, y, width, height)?;
    // Gather the §7.13.3.18 displaced-copy prediction for this transform.
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
        qindex,
        smooth_mode,
        num4_above_right,
        use_tcq,
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
    log2_side: u32,
    qindex: u32,
    smooth_mode: IntraSmoothMode,
    num4_above_right: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
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
        bit_depth,
    );
    let smooth_edges = IntraSmoothEdges::new(&left, &above);
    let mut prediction = vec![T::default(); side * side];
    predict_intra_smooth_rect_into(
        bit_depth,
        block_size,
        smooth_mode,
        smooth_edges,
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
fn build_smooth_edges<T: ReconSample>(
    left_neighbour: Option<&[T]>,
    above_neighbour: Option<&[T]>,
    have_left: bool,
    have_above: bool,
    side: usize,
    above_right_sentinel: Option<T>,
    bit_depth: BitDepth,
) -> (Vec<T>, Vec<T>) {
    let edge_len = side + 1;
    // §7.13.2.1 `LeftCol[i]`: reconstructed left column when haveLeft; else when
    // haveAbove, the above neighbour's first sample; else the no-left fallback.
    // The bottom-left sentinel `LeftCol[h]` keeps the clamped last left sample:
    // in raster decode order a full-superblock block's below-left is never
    // decoded yet (`num4BelowLeft == 0`), so the spec value
    // `CurrFrame[plane][Min(maxY, y+h)][x-1]` equals the clamped last sample.
    let left = match (have_left, left_neighbour) {
        (true, Some(samples)) => fill_edge_from_neighbour(samples, edge_len, bit_depth),
        _ if have_above => {
            let seed = above_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(noneighbour_left::<T>(bit_depth));
            vec![seed; edge_len]
        }
        _ => vec![noneighbour_left::<T>(bit_depth); edge_len],
    };
    // §7.13.2.1 `AboveRow[i]`: reconstructed above row when haveAbove; else when
    // haveLeft, the left neighbour's first sample; else the no-above fallback.
    let mut above = match (have_above, above_neighbour) {
        (true, Some(samples)) => fill_edge_from_neighbour(samples, edge_len, bit_depth),
        _ if have_left => {
            let seed = left_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(noneighbour_above::<T>(bit_depth));
            vec![seed; edge_len]
        }
        _ => vec![noneighbour_above::<T>(bit_depth); edge_len],
    };
    // §7.13.2.1 top-right sentinel `AboveRow[w]` (index `side`): overwrite the
    // clamped last in-block sample with the real reconstructed above-right sample
    // when the caller resolved one (above-right decoded, in-frame).
    if let Some(sentinel) = above_right_sentinel
        && let Some(slot) = above.get_mut(side)
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
    if !have_above || num4_above_right == 0 {
        return Ok(None);
    }
    // §7.13.2.1: maxX = ((MiCols * MI_SIZE) >> SubsamplingX) - 1, i.e. the chroma
    // frame right column. The chroma workspace plane storage width equals the
    // chroma frame width for these multiple-of-64 frames, so its last column is
    // `maxX`.
    let plane = workspace.plane(plane_id)?;
    let storage_width = plane.storage_size().width();
    let Some(max_x) = storage_width.checked_sub(1) else {
        return Ok(None);
    };
    let Some(above_row) = y.checked_sub(1) else {
        return Ok(None);
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
    let sentinel = workspace.reconstructed_sample(plane_id, sentinel_col, above_row)?;
    Ok(Some(sentinel))
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
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
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
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
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
    // Logical `AboveRow[-1..w)` and `LeftCol[-1..h)` (length side + 1): index 0
    // is the corner; index `k + 1` is logical `k`.
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
    // §7.13.2.1 `haveLeft` / `haveAbove`: a reconstructed neighbour exists exactly
    // when the block is not at the frame edge, which `intra_dc_edges_for_rect`
    // reports as a present left/above edge.
    let have_left = edges.left_samples().is_some();
    let have_above = edges.above_samples().is_some();
    // §7.13.2.1 corner AboveRow[-1] == LeftCol[-1] for the `haveLeft && haveAbove`
    // arm is the real reconstructed diagonally-above-left sample
    // CurrFrame[plane][y-1][x-1] (aboveMrlIndex == 0 at the superblock boundary,
    // MrlIndex == 0). It lies outside the block's immediate above row / left column,
    // so it is read explicitly here; `intra_dc_edges_for_rect` does not return it.
    // The other arms ignore `above_corner` (they derive the corner from the above row
    // / left column), so only read it when both neighbours are present.
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
    // §7.13.2.8 `enableIdif = plane == 0`: luma uses the IDIF 4-tap (which for
    // D135 `shift == 0` is the same sample copy as bilinear, but for D157 `shift
    // != 0` genuinely interpolates); chroma uses the `enableIdif == 0` bilinear
    // branch. The chroma callers only pass D135 (shift == 0), so both branches
    // agree there, but the plane dispatch keeps the spec contract exact.
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

/// Reconstructs one neighbour-having § 7.13.2.2 PAETH (`PAETH_PRED`,
/// non-directional) luma block over the § 7.13.2.1 above row / left column / corner
/// read from the partially-built frame's **real reconstructed neighbours**, writing
/// the bare prediction (this helper is gated by the caller to an `all_zero` leaf —
/// a residual-bearing PAETH stays deferred).
///
/// § 7.13.2.2 generates `pred[i][j]` from `LeftCol[i]`, `AboveRow[j]`, and the
/// shared corner `AboveRow[-1]` (the Paeth predictor: pick whichever of left /
/// above / corner is closest to `AboveRow[j] + LeftCol[i] - AboveRow[-1]`). It is
/// rectangular (independent `w` x `h`) and needs no IDIF / edge-filter / above-right
/// / bottom-left synthesis.
///
/// The caller admits ONLY the § 7.13.2.1 `haveAbove == 1 && haveLeft == 1` config,
/// so:
/// * `AboveRow[0..w)` is the real reconstructed above row
///   (`intra_dc_edges_for_rect` above edge);
/// * `LeftCol[0..h)` is the real reconstructed left column (its left edge);
/// * the corner `AboveRow[-1] = CurrFrame[plane][y-1][x-1]` (the diagonal
///   above-left sample, `MrlIndex == 0`, `aboveMrlIndex == 0`), read explicitly
///   here exactly as the § 7.13.2.8 directional-middle neighbour path reads it
///   (`intra_dc_edges_for_rect` does not return the corner). No single-sided
///   § 7.13.2.1 fallback is taken — those PAETH configs DEFER upstream.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_paeth_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    // The caller's `paeth_neighbours_reconstructed` gate guarantees BOTH the above
    // row and the left column are real reconstructed neighbours; treat a missing
    // edge as a gate violation rather than synthesizing a fallback (PAETH only ever
    // reaches here in the two-sided config). Reuse the directional "missing real
    // above-neighbour edge / §7.13.2.1 corner" guard — the same defensive class.
    let (Some(above_neighbour), Some(left_neighbour)) =
        (edges.above_samples(), edges.left_samples())
    else {
        return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
    };
    // §7.13.2.1 corner `AboveRow[-1] = CurrFrame[plane][y-1][x-1]` for the
    // `haveAbove && haveLeft` arm — the real reconstructed diagonal above-left
    // sample, read exactly as the §7.13.2.8 middle-directional neighbour path does.
    let (Some(corner_x), Some(corner_y)) = (x.checked_sub(1), y.checked_sub(1)) else {
        return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
    };
    let top_left = workspace.reconstructed_sample(plane_id, corner_x, corner_y)?;
    // §7.13.2.2 reads exactly `AboveRow[0..w)` and `LeftCol[0..h)`; copy the
    // reconstructed neighbour samples into width / height-sized edges.
    let above: Vec<T> = above_neighbour.iter().take(width).copied().collect();
    let left: Vec<T> = left_neighbour.iter().take(height).copied().collect();
    if above.len() != width || left.len() != height {
        return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
    }
    // The caller gates this helper to an `all_zero` leaf (zero residual), so the
    // bare §7.13.2.2 prediction IS the reconstruction. A non-`all_zero` PAETH
    // residual (which would need the §5.20.7.29 tx-type / IST coupling) stays
    // deferred upstream, so no dequant / inverse-transform runs here.
    let mut prediction = vec![T::default(); width * height];
    predict_intra_paeth_rect_into(
        bit_depth,
        block_size,
        IntraPaethEdges::new(&left, &above, top_left),
        &mut prediction,
        width,
    )?;
    workspace.write_rect_block(plane_id, x, y, block_size, &prediction)?;
    Ok(())
}

/// Reconstructs one neighbour-having ZONE-1 ONE-SIDED directional block — D45
/// (§ 7.13.2.8 step 1, pAngle 45) — over the § 7.13.2.1 above row PLUS the real
/// reconstructed above-right read from the partially-built frame, adds the
/// decoded residual (or writes the bare prediction for an `all_zero` block), and
/// stores the result so later blocks read it as a neighbour.
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
    log2_side: u32,
    qindex: u32,
    num4_above_right: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    // §7.13.2.1 above row + above-right; the corner is the real diagonally-above-
    // left sample. The zone-1 block is gated to a row>0, non-first-column position
    // (`haveLeft && haveAbove`), so both the above row and the corner are real.
    let above_idif =
        build_one_sided_above_idif_edge(workspace, plane_id, x, y, side, num4_above_right)?;
    let mut prediction = vec![T::default(); side * side];
    // §7.13.2.8 `enableIdif = plane == 0`: luma uses the IDIF 4-tap; chroma uses
    // the `enableIdif == 0` bilinear one-sided branch. For D45 (`shift == 0`) both
    // reduce to the same sample copy `AboveRow[base]`, so the result is bit-exact
    // either way, but the plane dispatch keeps the spec contract exact.
    if matches!(plane_id, PlaneId::Y) {
        predict_intra_directional_angle_rect_one_sided_idif_into(
            bit_depth,
            block_size,
            IntraDirectionalAngle::try_from_p_angle(p_angle)?,
            IntraDirectionalAngleIdifEdges::above(&above_idif),
            &mut prediction,
            side,
        )?;
    } else {
        // The chroma bilinear one-sided predictor reads the logical above edge
        // `AboveRow[0..w+h)` (length `w + h`); drop the IDIF `-2`/`-1` corner
        // prefix (slice indices 0,1) to recover that view.
        let above_bilinear = &above_idif[2..2 + side + side];
        predict_intra_directional_angle_rect_into(
            bit_depth,
            block_size,
            IntraDirectionalAngle::try_from_p_angle(p_angle)?,
            IntraDirectionalAngleEdges::above(above_bilinear),
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

/// Builds the § 7.13.2.8 ZONE-1 IDIF above edge `AboveRow[-2 ..= w + h + 1]`
/// (length `w + h + 4` for `MrlIndex == 0`, `slice[0]` = logical `-2`) for a
/// neighbour-having (`haveLeft && haveAbove`) one-sided block.
///
/// Per § 7.13.2.1: the in-row samples `AboveRow[i]` for `i in 0..w + h` are
/// `CurrFrame[plane][y - 1][Min(aboveLimit, x + i)]`, the corner `AboveRow[-1]` is
/// `CurrFrame[plane][y - 1][x - 1]`. Per § 7.13.2.8 the edge is then extended:
/// `AboveRow[minBase - 1] = AboveRow[minBase]` (here `AboveRow[-2] = AboveRow[-1]`)
/// and `AboveRow[maxBase + 1] = AboveRow[maxBase + 2] = AboveRow[maxBase]` (the
/// two trailing samples repeat the clamped last in-row sample). `aboveLimit =
/// Min(maxX, x + w + 4 * num4AboveRight - 1)`.
fn build_one_sided_above_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    side: usize,
    num4_above_right: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let above_row = y
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let corner_col = x
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    // §7.13.2.1 maxX = ((MiCols * MI_SIZE) >> SubsamplingX) - 1, i.e. the plane's
    // last reconstructed column. The plane storage width equals the plane frame
    // width for these multiple-of-64 frames.
    let plane = workspace.plane(plane_id)?;
    let storage_width = plane.storage_size().width();
    let max_x = storage_width
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    // aboveLimit = Min(maxX, x + w + 4 * num4AboveRight - 1).
    let above_right_extent = side
        .checked_add(num4_above_right.saturating_mul(4))
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| x.checked_add(v));
    let above_limit = above_right_extent.map_or(max_x, |limit| limit.min(max_x));

    // maxBaseX = w + h - 1 (mrlIndex 0). The IDIF logical range is -2..=maxBaseX+2
    // (slice length maxBaseX + 5 = w + h + 4): logical -2 (slice 0), -1 corner
    // (slice 1), 0..=maxBaseX (slices 2..=maxBaseX+2), and the two trailing
    // extension samples maxBaseX+1 (slice maxBaseX+3), maxBaseX+2 (slice maxBaseX+4).
    let max_base_x = side
        .checked_add(side)
        .and_then(|v| v.checked_sub(1))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let edge_len = max_base_x
        .checked_add(5)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mut above = vec![T::default(); edge_len];
    // Logical -1 corner -> slice index 1; -2 -> slice index 0.
    let corner = workspace.reconstructed_sample(plane_id, corner_col, above_row)?;
    above[0] = corner; // logical -2 = AboveRow[-1] (spec extension)
    above[1] = corner; // logical -1
    // In-row samples logical 0..=maxBaseX -> slice indices 2..=maxBaseX+2.
    for i in 0..=max_base_x {
        let column = x.saturating_add(i).min(above_limit);
        let sample = workspace.reconstructed_sample(plane_id, column, above_row)?;
        let slot = i
            .checked_add(2)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        above[slot] = sample;
    }
    // §7.13.2.8 trailing extension: AboveRow[maxBaseX+1] = AboveRow[maxBaseX+2] =
    // AboveRow[maxBaseX]; copy the clamped last in-row sample into both trailing
    // slots (maxBaseX+3, maxBaseX+4).
    let last_in_row = above[max_base_x + 2];
    if let Some(slot) = above.get_mut(max_base_x + 3) {
        *slot = last_in_row;
    }
    if let Some(slot) = above.get_mut(max_base_x + 4) {
        *slot = last_in_row;
    }
    Ok(above)
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
    log2_side: u32,
    qindex: u32,
    num4_below_left: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
    // §7.13.2.1 left column + below-left; at the gated first-superblock-row,
    // non-first-column position (`haveAbove == 0 && haveLeft == 1`) the corner is
    // `CurrFrame[plane][y][x-1]` and the left column is the real reconstructed
    // right column of the already-decoded left superblock.
    let left_idif =
        build_one_sided_left_idif_edge(workspace, plane_id, x, y, side, num4_below_left)?;
    let mut prediction = vec![T::default(); side * side];
    // §7.13.2.8 `enableIdif = plane == 0`: luma uses the zone-3 IDIF 4-tap; chroma
    // uses the `enableIdif == 0` bilinear one-sided branch (the spec-mandated
    // chroma branch). Both read the same prepared left edge.
    if matches!(plane_id, PlaneId::Y) {
        predict_intra_directional_angle_rect_one_sided_idif_into(
            bit_depth,
            block_size,
            angle,
            IntraDirectionalAngleIdifEdges::left(&left_idif),
            &mut prediction,
            side,
        )?;
    } else {
        // The chroma bilinear one-sided predictor reads the logical left edge
        // `LeftCol[0..w+h)` (length `w + h`); drop the IDIF `-2`/`-1` corner
        // prefix (slice indices 0,1) to recover that view.
        let left_bilinear = &left_idif[2..2 + side + side];
        predict_intra_directional_angle_rect_into(
            bit_depth,
            block_size,
            angle,
            IntraDirectionalAngleEdges::left(left_bilinear),
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

/// Builds the § 7.13.2.8 ZONE-3 IDIF left edge `LeftCol[-2 ..= w + h + 1]`
/// (length `w + h + 4` for `MrlIndex == 0`, `slice[0]` = logical `-2`) for a
/// first-superblock-row, non-first-column (`haveAbove == 0 && haveLeft == 1`)
/// one-sided block.
///
/// Per § 7.13.2.1 (the `haveLeft == 1` left-column branch): the in-column samples
/// `LeftCol[i]` for `i in 0..w + h` are `CurrFrame[plane][Min(leftLimit, y + i)]
/// [x - 1]` with `leftLimit = Min(maxY, y + h + 4 * num4BelowLeft - 1)`. At
/// `haveAbove == 0 && haveLeft == 1` the corner `LeftCol[-1] = AboveRow[-1] =
/// CurrFrame[plane][y][x - 1]` (the top of the left column). Per § 7.13.2.8 the
/// edge is then extended: `LeftCol[minBase - 1] = LeftCol[minBase]` (here
/// `LeftCol[-2] = LeftCol[-1]`) and `LeftCol[maxBase + 1] = LeftCol[maxBase + 2]
/// = LeftCol[maxBase]` (the two trailing samples repeat the clamped last in-column
/// sample).
fn build_one_sided_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    side: usize,
    num4_below_left: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let left_col = x
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    // §7.13.2.1 maxY = ((MiRows * MI_SIZE) >> SubsamplingY) - 1, i.e. the plane's
    // last reconstructed row. The plane storage height equals the plane frame
    // height for these multiple-of-64 frames.
    let plane = workspace.plane(plane_id)?;
    let storage_height = plane.storage_size().height();
    let max_y = storage_height
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    // leftLimit = Min(maxY, y + h + 4 * num4BelowLeft - 1).
    let below_left_extent = side
        .checked_add(num4_below_left.saturating_mul(4))
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| y.checked_add(v));
    let left_limit = below_left_extent.map_or(max_y, |limit| limit.min(max_y));

    // maxBaseY = w + h - 1 (mrlIndex 0). The IDIF logical range is -2..=maxBaseY+2
    // (slice length maxBaseY + 5 = w + h + 4): logical -2 (slice 0), -1 corner
    // (slice 1), 0..=maxBaseY (slices 2..=maxBaseY+2), and the two trailing
    // extension samples maxBaseY+1 (slice maxBaseY+3), maxBaseY+2 (slice maxBaseY+4).
    let max_base_y = side
        .checked_add(side)
        .and_then(|v| v.checked_sub(1))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let edge_len = max_base_y
        .checked_add(5)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mut left = vec![T::default(); edge_len];
    // §7.13.2.1 (haveAbove == 0 && haveLeft == 1): corner LeftCol[-1] =
    // CurrFrame[plane][y][x-1]. Logical -1 -> slice index 1; -2 -> slice index 0.
    let corner = workspace.reconstructed_sample(plane_id, left_col, y)?;
    left[0] = corner; // logical -2 = LeftCol[-1] (spec extension)
    left[1] = corner; // logical -1
    // In-column samples logical 0..=maxBaseY -> slice indices 2..=maxBaseY+2.
    for i in 0..=max_base_y {
        let row = y.saturating_add(i).min(left_limit);
        let sample = workspace.reconstructed_sample(plane_id, left_col, row)?;
        let slot = i
            .checked_add(2)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        left[slot] = sample;
    }
    // §7.13.2.8 trailing extension: LeftCol[maxBaseY+1] = LeftCol[maxBaseY+2] =
    // LeftCol[maxBaseY]; copy the clamped last in-column sample into both trailing
    // slots (maxBaseY+3, maxBaseY+4).
    let last_in_col = left[max_base_y + 2];
    if let Some(slot) = left.get_mut(max_base_y + 3) {
        *slot = last_in_col;
    }
    if let Some(slot) = left.get_mut(max_base_y + 4) {
        *slot = last_in_col;
    }
    Ok(left)
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
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    // §7.13.2.8 step 4/5 read ONLY the above row (V) or the left column (H). Build
    // exactly that rectangular edge from the real reconstructed neighbour
    // (`intra_dc_edges_for_rect` returns the W-wide above row and the H-tall left
    // column). `predict_intra_cardinal_directional_rect_into` is fully rectangular:
    // V copies the W-wide above row into every one of the H rows; H fills each of
    // the H rows with one of the H left samples.
    //
    // When the required real edge is absent at a frame/tile boundary but the
    // ORTHOGONAL neighbour exists, §7.13.2.1 synthesizes the missing edge as a flat
    // repeat of one orthogonal sample (`MrlIndex == 0`):
    // * V_PRED, `haveAbove == 0 && haveLeft == 1` (lines 5286-5287):
    //   `AboveRow[i] = CurrFrame[plane][y][x-1]` — the block's top-left left
    //   neighbour, which is `left[0]` of the H-tall reconstructed left column.
    // * H_PRED, `haveLeft == 0 && haveAbove == 1` (lines 5272-5273):
    //   `LeftCol[i] = CurrFrame[plane][y-1][x]` — the block's top-left above
    //   neighbour, which is `above[0]` of the W-wide reconstructed above row.
    // The cardinal copy of a flat synthesized edge is a flat block (the §7.13.2.1
    // `!haveAbove`/`!haveLeft` no-neighbour midpoint fallback is a SEPARATE path: it
    // applies only when the orthogonal neighbour is ALSO absent, which the admission
    // gate excludes). The synthesized edge is owned here so it outlives the borrow.
    let synthesized_edge;
    let cardinal_edges = match direction {
        IntraCardinalDirection::Vertical => {
            if let Some(above) = edges.above_samples() {
                IntraCardinalEdges::above(above)
            } else {
                let left = edges
                    .left_samples()
                    .ok_or(GeneralIntraResidualError::MissingCardinalEdge)?;
                let fill = *left
                    .first()
                    .ok_or(GeneralIntraResidualError::MissingCardinalEdge)?;
                synthesized_edge = vec![fill; width];
                IntraCardinalEdges::above(&synthesized_edge)
            }
        }
        IntraCardinalDirection::Horizontal => {
            if let Some(left) = edges.left_samples() {
                IntraCardinalEdges::left(left)
            } else {
                let above = edges
                    .above_samples()
                    .ok_or(GeneralIntraResidualError::MissingCardinalEdge)?;
                let fill = *above
                    .first()
                    .ok_or(GeneralIntraResidualError::MissingCardinalEdge)?;
                synthesized_edge = vec![fill; height];
                IntraCardinalEdges::left(&synthesized_edge)
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
    let edge_len = side + 1;
    // §7.13.2.1 LeftCol[i] (logical 0..side-1 -> slice index 1..=side) and the
    // corner LeftCol[-1] (slice index 0).
    let mut left = Vec::with_capacity(edge_len);
    let mut above = Vec::with_capacity(edge_len);
    match (have_left, have_above) {
        (true, true) => {
            // §7.13.2.1 haveLeft && haveAbove (aboveMrlIndex == 0 at the superblock
            // boundary, MrlIndex == 0): LeftCol[i] = CurrFrame[plane][y+i][x-1] (the
            // real reconstructed left column), AboveRow[i] =
            // CurrFrame[plane][y-1][x+i] (the real reconstructed above row), and the
            // corner AboveRow[-1] = LeftCol[-1] = CurrFrame[plane][y-1][x-1] (the real
            // diagonally-above-left sample, supplied by the caller). D135 reads the
            // corner on its main diagonal (above_base == -1, shift == 0).
            let Some(corner) = above_corner else {
                return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
            };
            let left_samples = left_neighbour.unwrap_or(&[]);
            let above_samples = above_neighbour.unwrap_or(&[]);
            left.push(corner);
            above.push(corner);
            for i in 0..side {
                left.push(sample_or_last(
                    left_samples,
                    i,
                    noneighbour_left::<T>(bit_depth),
                ));
                above.push(sample_or_last(
                    above_samples,
                    i,
                    noneighbour_above::<T>(bit_depth),
                ));
            }
        }
        (false, true) => {
            // §7.13.2.1 !haveLeft && haveAbove (aboveMrlIndex == 0, MrlIndex == 0):
            // AboveRow[i] = CurrFrame[plane][y-1][x+i] (the real reconstructed above
            // row), LeftCol[i] = CurrFrame[plane][y-1][x] (the first above sample,
            // repeated), and the corner AboveRow[-1] = LeftCol[-1] =
            // CurrFrame[plane][y-1][x] = AboveRow[0] (the first above sample). No
            // separate corner read is needed.
            let above_samples = above_neighbour.unwrap_or(&[]);
            let seed = above_samples
                .first()
                .copied()
                .unwrap_or(noneighbour_above::<T>(bit_depth));
            left.push(seed);
            above.push(seed);
            for i in 0..side {
                left.push(seed);
                above.push(sample_or_last(
                    above_samples,
                    i,
                    noneighbour_above::<T>(bit_depth),
                ));
            }
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
                .unwrap_or(noneighbour_left::<T>(bit_depth));
            left.push(seed);
            above.push(seed);
            for i in 0..side {
                left.push(sample_or_last(
                    left_samples,
                    i,
                    noneighbour_left::<T>(bit_depth),
                ));
                above.push(seed);
            }
        }
        (false, false) => {
            // §7.13.2.1 no-neighbour fallbacks (handled by the first-block path, but
            // kept total here): AboveRow 127, LeftCol 129, shared corner 128.
            left.push(noneighbour_corner::<T>(bit_depth));
            above.push(noneighbour_corner::<T>(bit_depth));
            for _ in 0..side {
                left.push(noneighbour_left::<T>(bit_depth));
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
        // The cardinal copy modes and the one-sided angles (zone-1 D45,
        // zone-3 D203) use dedicated predictors and never reach the middle-angle
        // path; return an error (defensive: the dispatch routes them away first).
        SupportedDirectionalLumaMode::Vertical
        | SupportedDirectionalLumaMode::Horizontal
        | SupportedDirectionalLumaMode::D45
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
    // `edge` is `Edge[-1..side)`: index 0 = `-1` (corner), index `k + 1` = k.
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

    /// An `all_zero` (`txb_skip`) luma block: reconstruction writes the bare
    /// §7.13.2 prediction (zero residual), the only kind these cardinal
    /// rect/transpose guards exercise.
    fn all_zero_luma_block() -> LumaCoeffBlock {
        LumaCoeffBlock {
            all_zero: true,
            eob: 0,
            quant: Vec::new(),
            intra_ist: None,
            // §3 `DCT_DCT` index 0; a skip block inverts no residual.
            plane_tx_type: 0,
        }
    }

    /// Lays an `above_row` pattern (length `width`) so that workspace row `edge_y` is
    /// that pattern over `x[0, width)`. Writes a `width x 4` block at `(0, edge_y-3)`
    /// whose every row carries the pattern (so its bottom row `edge_y` does too).
    fn lay_above_row(
        ws: &mut CurrentFrameWorkspace<u8>,
        edge_y: usize,
        log2_w: u8,
        pattern: &[u8],
    ) {
        let width = 1usize << log2_w;
        let samples: Vec<u8> = (0..4).flat_map(|_| pattern.iter().copied()).collect();
        let size = IntraRectBlockSize::new(log2_w, 2).unwrap();
        ws.write_rect_block(PlaneId::Y, 0, edge_y - 3, size, &samples)
            .unwrap();
        debug_assert_eq!(width, pattern.len());
    }

    /// Lays a `left_col` pattern (length `height`) so that workspace column `edge_x`
    /// is that pattern over `y[0, height)`. Writes a `4 x height` block at
    /// `(edge_x-3, 0)` whose every column carries the pattern (so its rightmost
    /// column `edge_x` does too).
    fn lay_left_col(ws: &mut CurrentFrameWorkspace<u8>, edge_x: usize, log2_h: u8, pattern: &[u8]) {
        let height = 1usize << log2_h;
        let mut samples = vec![0u8; 4 * height];
        for (row, &v) in pattern.iter().enumerate() {
            for col in 0..4 {
                samples[row * 4 + col] = v;
            }
        }
        let size = IntraRectBlockSize::new(2, log2_h).unwrap();
        ws.write_rect_block(PlaneId::Y, edge_x - 3, 0, size, &samples)
            .unwrap();
    }

    /// STRIDE/TRANSPOSE GUARD — V_PRED over a NON-SQUARE 64x32 (`W == 64`,
    /// `H == 32`) block with a REAL, NON-FLAT above row. §7.13.2.8 V_PRED copies the
    /// 64-wide above row into every one of the 32 rows; a width/height swap or a
    /// `stride == height`-instead-of-`width` bug would corrupt the layout and fail.
    /// The asymmetric edge is the key: a flat block (the ac0ej3 all-68 oracle) would
    /// MASK a transpose.
    #[test]
    fn rect_cardinal_vertical_64x32_copies_wide_above_row_per_row() {
        // Workspace tall/wide enough: the block sits at y=64 so it has a real above
        // row at y=63, x[0,64). Build a non-flat above row (x + 100, distinct per x).
        let mut ws = new_general_intra_workspace::<u8>(64, 128, BitDepth::Eight).unwrap();
        let above_row: Vec<u8> = (0..64).map(|x| 100 + x as u8).collect();
        lay_above_row(&mut ws, 63, 6, &above_row);

        reconstruct_general_intra_cardinal_neighbour_block_into(
            &mut ws,
            &all_zero_luma_block(),
            IntraCardinalDirection::Vertical,
            PlaneId::Y,
            0,
            64,
            6, // log2_width = 6 -> 64
            5, // log2_height = 5 -> 32
            0,
            false,
            BitDepth::Eight,
        )
        .unwrap();

        for row in 0..32 {
            for col in 0..64 {
                assert_eq!(
                    ws.reconstructed_sample(PlaneId::Y, col, 64 + row).unwrap(),
                    100 + col as u8,
                    "V_PRED 64x32 sample ({col},{}) must copy above_row[{col}]",
                    64 + row,
                );
            }
        }
    }

    /// STRIDE/TRANSPOSE GUARD — H_PRED over a NON-SQUARE 32x64 (`W == 32`,
    /// `H == 64`) block with a REAL, NON-FLAT left column. §7.13.2.8 H_PRED fills
    /// each of the 64 rows with one of the 64 left samples; a width/height swap would
    /// read past the 64-tall left column or mis-stride and fail.
    #[test]
    fn rect_cardinal_horizontal_32x64_fills_each_row_from_tall_left_column() {
        let mut ws = new_general_intra_workspace::<u8>(128, 64, BitDepth::Eight).unwrap();
        // Block at x=64, y=0: real left column at x=63, y[0,64) (non-flat per row).
        let left_col: Vec<u8> = (0..64).map(|y| 50 + y as u8).collect();
        lay_left_col(&mut ws, 63, 6, &left_col);

        reconstruct_general_intra_cardinal_neighbour_block_into(
            &mut ws,
            &all_zero_luma_block(),
            IntraCardinalDirection::Horizontal,
            PlaneId::Y,
            64,
            0,
            5, // log2_width = 5 -> 32
            6, // log2_height = 6 -> 64
            0,
            false,
            BitDepth::Eight,
        )
        .unwrap();

        for row in 0..64 {
            for col in 0..32 {
                assert_eq!(
                    ws.reconstructed_sample(PlaneId::Y, 64 + col, row).unwrap(),
                    50 + row as u8,
                    "H_PRED 32x64 sample ({},{row}) must fill row from left_col[{row}]",
                    64 + col,
                );
            }
        }
    }

    /// §7.13.2.1 NO-ABOVE FALLBACK GUARD — the ac0ej3 MI(64,0) case: a NON-SQUARE
    /// 64x32 V_PRED block at the frame TOP (`y == 0`, `haveAbove == 0`) with a
    /// NON-FLAT reconstructed left column. §7.13.2.1 synthesizes
    /// `AboveRow[i] = CurrFrame[plane][y][x-1]` — the block's top-left left neighbour
    /// (`left[0]`), repeated across the whole synthesized above row — so the V_PRED
    /// copy is a FLAT block equal to `left[0]`, NOT `left[i]`. A non-flat left column
    /// proves the fallback reads ONLY `left[0]` (a bug reading `left[i]` row-wise
    /// would produce a vertical gradient and fail).
    #[test]
    fn rect_cardinal_vertical_64x32_no_above_fallback_is_flat_left_corner() {
        let mut ws = new_general_intra_workspace::<u8>(128, 64, BitDepth::Eight).unwrap();
        // Block at x=64, y=0 (frame top): non-flat left column at x=63, y[0,32).
        let left_col: Vec<u8> = (0..32).map(|y| 70 + y as u8).collect();
        lay_left_col(&mut ws, 63, 5, &left_col);

        reconstruct_general_intra_cardinal_neighbour_block_into(
            &mut ws,
            &all_zero_luma_block(),
            IntraCardinalDirection::Vertical,
            PlaneId::Y,
            64,
            0,
            6, // log2_width = 6 -> 64
            5, // log2_height = 5 -> 32
            0,
            false,
            BitDepth::Eight,
        )
        .unwrap();

        // Every sample equals left[0] (= CurrFrame[0][63] = 70), flat over 64x32.
        for row in 0..32 {
            for col in 0..64 {
                assert_eq!(
                    ws.reconstructed_sample(PlaneId::Y, 64 + col, row).unwrap(),
                    70,
                    "no-above V_PRED 64x32 sample ({},{row}) must be the flat left corner left[0]=70",
                    64 + col,
                );
            }
        }
    }

    /// §7.13.2.1 NO-LEFT FALLBACK GUARD — the symmetric H_PRED case at the frame
    /// LEFT edge (`x == 0`, `haveLeft == 0`) with a NON-FLAT reconstructed above row.
    /// §7.13.2.1 synthesizes `LeftCol[i] = CurrFrame[plane][y-1][x]` (`above[0]`),
    /// so the H_PRED copy is FLAT equal to `above[0]`, NOT `above[j]`.
    #[test]
    fn rect_cardinal_horizontal_32x64_no_left_fallback_is_flat_above_corner() {
        let mut ws = new_general_intra_workspace::<u8>(64, 128, BitDepth::Eight).unwrap();
        // Block at x=0, y=64 (frame left edge): non-flat above row at y=63, x[0,32).
        let above_row: Vec<u8> = (0..32).map(|x| 80 + x as u8).collect();
        lay_above_row(&mut ws, 63, 5, &above_row);

        reconstruct_general_intra_cardinal_neighbour_block_into(
            &mut ws,
            &all_zero_luma_block(),
            IntraCardinalDirection::Horizontal,
            PlaneId::Y,
            0,
            64,
            5, // log2_width = 5 -> 32
            6, // log2_height = 6 -> 64
            0,
            false,
            BitDepth::Eight,
        )
        .unwrap();

        // Every sample equals above[0] (= CurrFrame[63][0] = 80), flat over 32x64.
        for row in 0..64 {
            for col in 0..32 {
                assert_eq!(
                    ws.reconstructed_sample(PlaneId::Y, col, 64 + row).unwrap(),
                    80,
                    "no-left H_PRED 32x64 sample ({col},{}) must be the flat above corner above[0]=80",
                    64 + row,
                );
            }
        }
    }

    /// Reference §7.13.2.2 Paeth sample (independent of the splot-recon primitive):
    /// pick whichever of `left` / `above` / `top_left` is closest to
    /// `above + left - top_left`, ties favouring left then above.
    fn ref_paeth(left: i32, above: i32, top_left: i32) -> u8 {
        let base = above + left - top_left;
        let p_left = (base - left).abs();
        let p_top = (base - above).abs();
        let p_top_left = (base - top_left).abs();
        let v = if p_left <= p_top && p_left <= p_top_left {
            left
        } else if p_top <= p_top_left {
            above
        } else {
            top_left
        };
        u8::try_from(v).unwrap()
    }

    /// STRIDE / CORNER GUARD — §7.13.2.2 PAETH over a NON-SQUARE 8x16 (`W == 8`,
    /// `H == 16`) block with a REAL, NON-FLAT above row, a REAL, NON-FLAT left
    /// column, AND a DISTINCT corner `AboveRow[-1] = CurrFrame[plane][y-1][x-1]`.
    /// Paeth genuinely depends on all three (`base = AboveRow[j] + LeftCol[i] -
    /// AboveRow[-1]`), so a width/height swap, a wrong stride, or reading the corner
    /// from the above row / left column instead of `CurrFrame[y-1][x-1]` would
    /// corrupt the output and fail. The asymmetric edges are the key: the flat
    /// ac0ej3 oracle (all `68`) would MASK every one of those mix-ups.
    #[test]
    fn rect_paeth_8x16_uses_above_left_and_distinct_corner() {
        // Block at (16, 16): real above row at y=15 over x[16,24), real left column
        // at x=15 over y[16,32), and corner at (15, 15). Lay the above-NEIGHBOUR
        // block at (16, 12) so its bottom row y=15 carries `above[j]`, and the
        // left-NEIGHBOUR block at (12, 15) so its right column x=15 carries the
        // corner (row 15) and `left[i]` (rows 16..32). Build a 64x64 workspace.
        let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();

        // Above row: 8 distinct values over x[16,24).
        let above: Vec<u8> = (0..8).map(|j| 30 + 7 * j as u8).collect();
        let above_block: Vec<u8> = (0..4).flat_map(|_| above.iter().copied()).collect();
        ws.write_rect_block(
            PlaneId::Y,
            16,
            12,
            IntraRectBlockSize::new(3, 2).unwrap(),
            &above_block,
        )
        .unwrap();

        // Left column at x=15: a DISTINCT corner at y=15 plus 16 distinct left
        // samples over y[16,32). Write a 4-wide x 32-tall block at (12, 0) whose
        // rightmost column x=15 carries: rows 0..15 arbitrary, row 15 = corner 200,
        // rows 16..32 = left[i]. (Lower rows are overwritten by the above block only
        // over x[16,24); x=15 stays as written here.)
        let corner: u8 = 200;
        let left: Vec<u8> = (0..16).map(|i| 40 + 5 * i as u8).collect();
        let mut left_block = vec![0u8; 4 * 32];
        for col in 0..4 {
            left_block[15 * 4 + col] = corner;
            for (i, &v) in left.iter().enumerate() {
                left_block[(16 + i) * 4 + col] = v;
            }
        }
        ws.write_rect_block(
            PlaneId::Y,
            12,
            0,
            IntraRectBlockSize::new(2, 5).unwrap(),
            &left_block,
        )
        .unwrap();

        // Sanity: the laid neighbours read back as intended.
        assert_eq!(ws.reconstructed_sample(PlaneId::Y, 15, 15).unwrap(), corner);
        assert_eq!(
            ws.reconstructed_sample(PlaneId::Y, 16, 15).unwrap(),
            above[0]
        );
        assert_eq!(
            ws.reconstructed_sample(PlaneId::Y, 15, 16).unwrap(),
            left[0]
        );

        reconstruct_general_intra_luma_paeth_neighbour_block_into(
            &mut ws,
            PlaneId::Y,
            16,
            16,
            3, // log2_width = 3 -> 8
            4, // log2_height = 4 -> 16
            BitDepth::Eight,
        )
        .unwrap();

        for (i, &left_i) in left.iter().enumerate() {
            for (j, &above_j) in above.iter().enumerate() {
                let want = ref_paeth(i32::from(left_i), i32::from(above_j), i32::from(corner));
                assert_eq!(
                    ws.reconstructed_sample(PlaneId::Y, 16 + j, 16 + i).unwrap(),
                    want,
                    "PAETH 8x16 sample (col {j}, row {i}) must be Paeth(left[{i}], above[{j}], corner)"
                );
            }
        }
    }
}
