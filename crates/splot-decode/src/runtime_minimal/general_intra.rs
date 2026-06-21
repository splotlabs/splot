// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General minimal-tool intra decode frontier for the shared minimal-tier
//! runtime.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

use super::*;

/// Routes a parsed frame to the general intra decode frontier.
///
/// The frozen minimal hash tier owns exactly the committed
/// `base_q_idx == 255` fixture (see [`validate_frame_core`]); any other
/// general minimal-tool intra key frame routes to
/// [`decode_general_minimal_intra_frame`]. Frames that are not minimal-tool
/// intra (segmentation, quant matrices, delta-Q, in-loop filters, CCSO, GDF,
/// film grain, screen-content/palette, DIP, or SDP enabled) fall through to the
/// frozen gate so its precise diagnostics are preserved.
///
/// `enable_dip` (§ 5.20.5.3 `dip_mode_info`) and `enable_sdp` (luma-only key
/// partitions that omit the `uv_mode` read) are checked here at the sequence
/// level — not in [`validate_sequence`] — because the frozen hash fixture is
/// itself an `enable_sdp` stream whose hand-traced symbol path handles it; only
/// the general mode decode cannot yet.
pub(super) fn route_general_minimal_intra(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> bool {
    core.quantization_params
        .is_some_and(|quant| quant.base_q_idx != FROZEN_MINIMAL_BASE_Q_IDX)
        // Reconstruction passes zero quantizer deltas, so admit only frames whose
        // §5.18.6.1 per-plane DeltaQ values are all zero. (Base*DeltaQ is forced
        // to zero by the `equal_ac_dc_q` admission below.)
        && core.quantization_params.is_some_and(|quant| {
            quant.delta_q_y_dc == 0
                && quant.delta_q_u_dc == 0
                && quant.delta_q_u_ac == 0
                && quant.delta_q_v_dc == 0
                && quant.delta_q_v_ac == 0
        })
        && sequence
            .intra
            .as_ref()
            // §7.13.2 (lines 5355-5365): with `enable_ibp == 1` a non-4x4
            // `DC_PRED` block runs the §7.13.2.12 IBP DC process, which modifies
            // the prediction using the available left/above neighbours. This path
            // applies only the plain §7.13.2.4 DC predictor, so a neighbour-having
            // DC block (any non-first superblock / split block) would reconstruct
            // wrong pixels under IBP. Reject `enable_ibp` until the IBP DC process
            // is modelled (all committed fixtures are encoded with enable_ibp = 0).
            .is_some_and(|intra| !intra.enable_dip && !intra.enable_ibp)
        && sequence
            .partition
            .is_some_and(|partition| !partition.enable_sdp)
        // §5.20 / §5.20.7.27: FSC, CCTX, IDTX, and IST all add transform-type or
        // cross-component syntax the general path does not yet read; `equal_ac_dc_q`
        // forces every derived Base*DeltaQ to zero (§5.4.8).
        && sequence.transform_quant_entropy.is_some_and(|tq| {
            tq.equal_ac_dc_q
                && !tq.enable_fsc
                && !tq.enable_cctx
                && !tq.enable_idtx_intra
                && !tq.enable_intra_ist
                // §5.4.8: equal_ac_dc_q forces BaseYDcDeltaQ to zero, but the
                // chroma base offsets BaseUVDcDeltaQ / BaseUVAcDeltaQ are derived
                // independently. Reconstruction passes zero deltas, so require
                // both to resolve to zero as well.
                && i32::from(tq.base_uv_dc_delta_q) + GENERAL_INTRA_DELTA_DCQUANT_MIN == 0
                && i32::from(tq.base_uv_ac_delta_q) + GENERAL_INTRA_DELTA_DCQUANT_MIN == 0
        })
        // §5.20.6.1: TX_MODE_SELECT inserts read_tx_partition() before coeffs();
        // only the fixed-largest 64x64 transform is handled.
        && core
            .intra_tail
            .is_some_and(|tail| tail.tx_mode == TxMode::Largest)
        && is_general_minimal_intra(core)
}

/// Returns whether `core` is a single-tile 8-bit intra key frame whose width and
/// height are positive multiples of 64 forming a (possibly 2-D) grid of 64x64
/// superblocks, with no segmentation, quant matrices, delta-Q, in-loop filters,
/// CCSO, GDF, or film grain — the general intra subset the frontier admits. This
/// mirrors [`validate_frame_core`] but accepts any `base_q_idx`, so blocks can
/// carry a real (nonzero) residual.
fn is_general_minimal_intra(core: &FrameHeaderCore) -> bool {
    core.status == FrameHeaderParseStatus::IntraHeaderComplete
        && core.cur_mfh_id.is_zero()
        && core.show_existing_frame == Some(false)
        && core.frame_is_intra == Some(true)
        && core.is_key_frame
        && core.immediate_output_frame == Some(true)
        && core.implicit_output_frame == Some(false)
        // §5.18.3 / §5.20.2.1 / §7.13.2.1: the general intra path tiles the frame
        // into 64x64 superblocks, so width and height must be positive multiples
        // of the superblock side (64), and the §5.20.2.1 raster loop iterates them
        // (`clear_left_context()` per superblock row) with later superblocks
        // predicting from already-reconstructed left/above neighbours. A full 2-D
        // grid is admitted: a non-rightmost row>0 superblock's full-superblock
        // §7.13.2.13 SMOOTH chroma block has a decoded above-right neighbour
        // (`clear_block_decoded_flags` (§5.20.2.3) marks `BlockDecoded[-1][x] = 1`
        // up to `(MiColEnd - c) >> subX`, which exceeds the superblock width), so
        // the §7.13.2.1 `AboveRow[w]` sentinel reads the real reconstructed
        // `CurrFrame[plane][y-1][Min(aboveLimit, x+w)]` sample (see
        // `reconstruct_general_intra_chroma_smooth_into`), no longer the edge-clamp.
        && core.frame_size.is_some_and(|size| {
            size.width != 0
                && size.height != 0
                && size.width % MINIMAL_WIDTH == 0
                && size.height % MINIMAL_HEIGHT == 0
        })
        && core
            .tile_info
            .as_ref()
            .is_some_and(|tile_info| tile_info.tile_cols == 1 && tile_info.tile_rows == 1)
        && core.quantization_params.is_some()
        && core
            .segmentation_params
            .as_ref()
            .is_some_and(|seg| !seg.segmentation_enabled)
        && core.setup_qm_params.is_some_and(|qm| !qm.using_qmatrix)
        && core
            .delta_q_params
            .is_some_and(|delta| !delta.delta_q_present)
        && core
            .lossless_info
            .as_ref()
            .is_some_and(|lossless| !lossless.coded_lossless)
        && core
            .deblocking_filter_params
            .is_some_and(|filter| filter.apply_deblocking_filter == [false; 4])
        && core.gdf_params.is_some_and(|gdf| !gdf.gdf_frame_enable)
        && core
            .cdef_params
            .as_ref()
            .is_some_and(|cdef| !cdef.cdef_frame_enable)
        && core.lr_params.as_ref().is_some_and(|lr| !lr.uses_lr)
        && core
            .ccso_params
            .as_ref()
            .is_some_and(|ccso| ccso.ccso_frame_flag.is_none() && ccso.planes.is_empty())
        && core
            .intra_tail
            .is_some_and(|tail| !tail.film_grain.apply_grain)
        // Screen-content tools enable §5.20.8.1 palette_mode_info() after uv_mode,
        // adding mode symbols the general mode decode does not yet read.
        && core.allow_screen_content_tools != Some(true)
}

/// Decodes a general minimal-tool intra key frame as far as the current
/// frontier reaches.
///
/// This runs the real AV2 § 5.20.3.1 partition traversal over the single tile,
/// confirms the root partition frontier, decodes the § 5.20.5.3 block mode info,
/// decodes the § 5.20.7.27 luma and chroma transform-block coefficients,
/// dequantizes / inverse-transforms / residual-adds each plane over a
/// no-neighbour DC prediction, validates `exit_symbol()`, and returns the
/// reconstructed frame. It never mutates the frozen minimal hash tier.
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_general_minimal_intra_frame(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: DecodeOptions,
    header: IvfHeader,
) -> Result<MinimalRuntimeFrame> {
    let mut tile_plan = derive_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        core,
        options,
    )?;
    let tile = match tile_plan.work_units_mut() {
        [tile] => tile,
        [] => {
            return Err(general_intra_unsupported(
                "general_intra_missing_tile_work_unit",
                None,
                "general intra decode requires one tile work unit",
                GENERAL_INTRA_TILE_SPEC_SECTION,
            ));
        }
        work_units => {
            return Err(general_intra_unsupported(
                "general_intra_unexpected_tile_work_units",
                work_units.first().map(|tile| tile.tile_byte_span().start),
                "general intra decode currently supports exactly one tile work unit",
                GENERAL_INTRA_TILE_SPEC_SECTION,
            ));
        }
    };
    let tile_offset = tile.tile_byte_span().start;

    // §7.14.2 quantizer index == base_q_idx for the minimal-tool frame (no
    // segmentation or delta-Q). The §7.14.4 TCQ dqDenom term applies to the luma
    // DCT_DCT (TX_CLASS_2D) non-lossless non-FSC block only.
    let qindex = core
        .quantization_params
        .map(|quant| quant.base_q_idx)
        .ok_or_else(|| {
            general_intra_unsupported(
                "general_intra_missing_base_q",
                Some(tile_offset),
                "general intra decode requires a parsed base_q_idx",
                GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
            )
        })?;
    let luma_use_tcq = tile.coeff_frame_facts().allow_tcq();
    let (mi_rows, mi_cols) = crate::tile_payload::frame_mi_dimensions(core)
        .map_err(|error| general_intra_partition_frontier_error(error, tile_offset))?;

    // §5.18.3 frame dimensions: `is_general_minimal_intra` already gated these to
    // positive multiples of 64, so the workspace and decode limits are sized to
    // the real frame size (not the 64x64 single-superblock constant) so that
    // multi-superblock frames (e.g. 128x64) reconstruct into the full plane.
    let frame_size = core.frame_size.ok_or_else(|| {
        general_intra_unsupported(
            "general_intra_missing_frame_size",
            Some(tile_offset),
            "general intra decode requires a parsed frame size",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        )
    })?;
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;

    // Enforce the configured decode limits before allocating reconstruction
    // buffers, matching the frozen minimal path's ordering.
    let tile_size = tile.tile_size();
    let limits = options.limits();
    ensure_runtime_limits(limits, frame_width, frame_height, tile_size)?;

    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace(
        frame_width as usize,
        frame_height as usize,
    )?;
    let mut coeff_ctx =
        crate::tile_payload::TileCoeffContextState::new(mi_rows, mi_cols).map_err(|source| {
            general_intra_residual_error(
                GeneralIntraResidualError::CoeffContextState { source },
                tile_offset,
            )
        })?;

    // Walk the full §5.20.3.1 partition tree, decoding each leaf block's
    // §5.20.5.3 mode info and §5.20.7.27 Y/U/V coefficients and reconstructing it
    // into the workspace in decode order (so later blocks DC-predict from the
    // already-reconstructed neighbours).
    let symbols = crate::tile_payload::decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit, symbols, frontier, joint_modes| {
            decode_one_general_intra_block(
                work_unit,
                symbols,
                frontier,
                joint_modes,
                &mut workspace,
                &mut coeff_ctx,
                qindex,
                luma_use_tcq,
                mi_cols,
                tile_offset,
            )
        },
    )
    .map_err(|error| map_general_intra_multiblock_error(error, tile_offset))?;

    // The decoded blocks consume the entire tile payload, so §8.2.4
    // exit_symbol() must hold; a failure means the decode was not bit-exact.
    symbols.exit_symbol().map_err(|_| {
        general_intra_unsupported(
            "general_intra_exit_symbol",
            Some(tile_offset),
            "general intra tile payload did not satisfy §8.2.4 exit_symbol() after the decoded blocks",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        )
    })?;

    let frame = workspace.freeze()?;
    Ok(MinimalRuntimeFrame {
        frame,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

/// Decodes one general intra leaf block (mode info + Y/U/V coefficients) and
/// reconstructs it into `workspace` in decode order. Gated to square DC_PRED
/// blocks: the no-neighbour-aware §7.13.2 DC prediction is read from the
/// partially-built frame, so non-DC modes and non-square partitions are
/// rejected. Chroma is 4:2:0 (half-resolution).
///
/// Returns the block's AV2 § 5.20.5.3 `IntraJointMode` (`= modeDelta`) so the
/// caller can record it into the `IntraJointModes` grid for later blocks'
/// § 8.3.2 `y_mode_index` neighbour context; `joint_modes` supplies that grid
/// (read-only here) for this block's own `y_mode_index` context.
#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_block(
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::tile_payload::DecodeBlockFrontier,
    joint_modes: &crate::tile_payload::TileIntraJointModeState,
    workspace: &mut splot_recon::CurrentFrameWorkspace<u8>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    qindex: u32,
    luma_use_tcq: bool,
    mi_cols: usize,
    tile_offset: ByteOffset,
) -> Result<u8> {
    // Resolve the block geometry and gate the handled subset BEFORE reading the
    // §5.20.5.3 mode info: `uv_mode` is only coded when the block has chroma, and
    // sub-8x8 luma leaves use a different (deferred 4x4) chroma sizing that this
    // path does not model. Reading modes first for those cases would consume a
    // `uv_mode` symbol that is not present and desynchronise the decoder.
    let geometry_error = || {
        general_intra_unsupported(
            "general_intra_block_geometry",
            Some(tile_offset),
            "general intra block geometry lookup failed",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        )
    };
    let n4w = frontier
        .b_size
        .num_4x4_wide()
        .map_err(|_| geometry_error())?;
    let n4h = frontier
        .b_size
        .num_4x4_high()
        .map_err(|_| geometry_error())?;
    if n4w != n4h {
        return Err(general_intra_unsupported(
            "general_intra_non_square_block",
            Some(tile_offset),
            "general intra decode only supports square partition blocks; rectangular partitions are not yet implemented",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    // 4:2:0 sub-8x8 luma leaves defer chroma to the bottom-right 4x4 (a 4x4 chroma
    // transform over the 8x8 region, not luma_log2 - 1), and the other three are
    // luma-only; neither chroma sizing/position is modelled yet.
    if n4w < 2 {
        return Err(general_intra_unsupported(
            "general_intra_sub_8x8_block",
            Some(tile_offset),
            "general intra decode does not yet support sub-8x8 luma blocks (deferred 4:2:0 chroma sizing)",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    if !frontier.has_chroma {
        return Err(general_intra_unsupported(
            "general_intra_luma_only_block",
            Some(tile_offset),
            "general intra decode does not yet support luma-only (no-chroma) blocks",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }

    // The block's § 8.3.2 `y_mode_index` context is derived from the already-
    // decoded left/above neighbours' stored `IntraJointMode` (§ 5.20.5.3). A
    // directional neighbour raises `ctx` to `1` or `2`, selecting the
    // `TileYModeIndexCdf[ctx]` row; the non-directional luma decode over that row
    // is now handled (only the `y_mode_offset` escape's §5.20.5.3
    // directional-neighbour reorder is still deferred inside the mode decode).
    let modes = crate::tile_payload::decode_general_intra_block_modes(
        work_unit,
        symbols,
        joint_modes,
        frontier.r,
        frontier.c,
        n4w,
        n4h,
    )
    .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
    // Chroma is reconstructed with DC prediction; with § 7.13.2.13 smooth
    // prediction over § 7.13.2.1 neighbour edges read from the partially-built
    // frame when the decoded `uv_mode` resolves (via § 5.20.5.3
    // `get_intra_uv_mode_set`) to `SMOOTH_PRED`; or with § 7.13.2.8 D135
    // directional-follow prediction when `uv_mode == 0` over a directional luma
    // makes the spec return `YMode == D135_PRED` (`AngleDeltaUV = AngleDeltaY ==
    // 0`). Other non-DC chroma modes (PAETH, SMOOTH_V/H, other directional
    // angles) need their own § 7.13 predictors and are deferred.
    let Some(supported_chroma) = modes.supported_chroma_mode() else {
        return Err(general_intra_unsupported(
            "general_intra_non_dc_chroma_mode",
            Some(tile_offset),
            "general intra reconstruction only supports DC, SMOOTH, and D135 directional-follow chroma prediction; other non-DC chroma (uv_mode) modes are not yet implemented",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    };
    // The directional-follow D135 chroma predictor is verified bit-exact only for
    // the top-left no-neighbour block, where the § 7.13.2.1 edges reduce to the
    // flat fallback so the § 7.13.2.8 prediction is constant. Chroma directional
    // prediction always takes the bilinear branch — § 7.13.2.8 sets
    // `enableIdif = (plane == 0)`, so `enableIdif == 0` for U/V and the IDIF 4-tap
    // is luma-only — but over a real reconstructed NON-FLAT neighbour edge that
    // bilinear prediction reads the real edge samples, which is not yet verified
    // (a separate brick), so gate it to the no-neighbour top-left full superblock.
    let chroma_is_top_left = frontier.r == 0 && frontier.c == 0;
    if supported_chroma == crate::tile_payload::SupportedChromaMode::D135Follow
        && !(chroma_is_top_left && n4w == 16)
    {
        return Err(general_intra_unsupported(
            "general_intra_directional_chroma_neighbour",
            Some(tile_offset),
            "general intra directional-follow (D135) chroma prediction is only supported for the top-left (no-neighbour) 64x64 superblock block, where the §7.13.2.1 edges are the flat fallback; chroma directional prediction uses the §7.13.2.8 bilinear branch (enableIdif = plane == 0, so enableIdif == 0 for U/V), and over a real reconstructed non-flat neighbour edge that bilinear prediction is not yet verified",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // §7.13.2.1: the SMOOTH chroma path builds the §7.13.2.13 bottom-left
    // (`LeftCol[h]`) sentinel by edge-clamping (repeating the last in-block
    // neighbour sample). In raster decode order a full-superblock block's
    // below-left chroma is never decoded yet (`num4BelowLeft == 0`), so the spec
    // value `CurrFrame[Min(maxY, y+h)][x-1]` equals the clamped last left sample.
    // The top-right (`AboveRow[w]`) sentinel, however, reads the real
    // reconstructed `CurrFrame[plane][y-1][Min(aboveLimit, x+w)]` when the
    // above-right is decoded (`num4AboveRight > 0`): for a non-rightmost row>0
    // superblock `clear_block_decoded_flags` (§5.20.2.3) marks the above row
    // decoded out to `(MiColEnd - c) >> subX`, exceeding the superblock width.
    //
    // SMOOTH chroma is still gated to full-superblock blocks (`n4w == 16`): a
    // sub-partitioned (split) block needs the §5.20.2.3 per-block `BlockDecoded`
    // update (so an intra-superblock above-right / below-left split child is read
    // correctly), which is not yet modelled. A 64x64 superblock is 16 4x4 MI
    // units wide.
    const FULL_SB_N4: usize = 16;
    if supported_chroma == crate::tile_payload::SupportedChromaMode::Smooth && n4w != FULL_SB_N4 {
        return Err(general_intra_unsupported(
            "general_intra_smooth_chroma_subblock",
            Some(tile_offset),
            "general intra SMOOTH chroma is only supported for full 64x64 superblock blocks; sub-partitioned SMOOTH chroma needs the §7.13.2.1 above-right / below-left sentinel neighbours from the per-block §5.20.2.3 BlockDecoded update, which is not yet modelled",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // Luma is DC, a supported non-DC mode (§ 7.13.2.13 SMOOTH_V / SMOOTH_H), or
    // the supported directional mode (§ 7.13.2.8 D135_PRED).
    //
    // SMOOTH_V / SMOOTH_H is reconstructed both for the top-left (no-neighbour)
    // block (§ 7.13.2.1 pure fallback edges) AND for a neighbour-having
    // full-superblock block, where § 7.13.2.1 supplies the **real reconstructed**
    // left column / above row of the already-decoded neighbour. Smooth prediction
    // is linear interpolation over those edges (no `enable_intra_edge_filter` /
    // IDIF / upsample edge synthesis is involved), so the neighbour edge can be
    // non-flat and the result is still bit-exact against the AVM/dav2d oracle.
    //
    // The directional mode (§ 7.13.2.8 D135_PRED) is still gated to the top-left
    // no-neighbour block: over a real (non-flat) neighbour edge its
    // `enableIdif == 0` bilinear reduction no longer equals the spec IDIF 4-tap
    // interpolation, so it needs the real § 7.13.2.8 IDIF (a separate brick).
    //
    // Non-DC smooth: gated to >= 32x32 (`n4w >= 8`), where § 5.20.8.2
    // `get_tx_set` returns TX_SET_DCTONLY (square intra `txSzSqrUp >= TX_32X32`
    // -> forced DCT_DCT, no `intra_tx_type`); the neighbour-edge subset is gated
    // tighter to the full 64x64 superblock (`n4w == 16`), because a sub-superblock
    // split block needs the per-block § 5.20.2.3 `BlockDecoded` update (for the
    // intra-superblock above-right / below-left split neighbours) that is not yet
    // modelled. Directional D135: gated to the verified 64x64 superblock
    // (`n4w == 16`, TX_64X64 -> TX_SET_DCTONLY); the 32x32 / smaller directional
    // blocks (which may signal a mode-dependent non-DCT TxType) and other angles /
    // non-zero angle deltas are deferred.
    const NON_DC_MIN_N4: usize = 8;
    const FULL_SB_N4_LUMA: usize = 16;
    let supported_nondc_luma = modes.supported_nondc_luma();
    let supported_directional_luma = modes.supported_directional_luma();
    let is_top_left = frontier.r == 0 && frontier.c == 0;
    // True when a non-DC SMOOTH_V/H luma block reads a **real** reconstructed
    // neighbour edge (any superblock position other than the no-neighbour
    // top-left). In the first superblock row (`frontier.r == 0`, `haveAbove == 0`)
    // only the left column is real (the § 7.13.2.1 above row / top-right sentinel
    // are the no-neighbour fallback); for a row>0 block § 7.13.2.1 supplies the
    // **real reconstructed above row** (`CurrFrame[plane][y-1][...]`) and, when an
    // already-decoded above-right superblock is in frame, the real above-right
    // sentinel (`num4AboveRight > 0`), exactly as the SMOOTH chroma grid path does.
    // [`reconstruct_general_intra_luma_nondc_neighbour_block_into`] delegates to the
    // same plane-general edge builder + above-right resolver, so both cases share
    // one bit-exact path.
    let nondc_luma_has_neighbour = supported_nondc_luma.is_some() && !is_top_left;
    if !modes.luma_is_dc() {
        match (supported_nondc_luma, supported_directional_luma) {
            // SMOOTH_V / SMOOTH_H at the no-neighbour top-left block.
            (Some(_), _) if is_top_left && n4w >= NON_DC_MIN_N4 => {}
            // SMOOTH_V / SMOOTH_H neighbour-having full-superblock block at ANY
            // superblock position in the 2-D grid. First superblock row
            // (`haveAbove == 0`): reads the real § 7.13.2.1 reconstructed LEFT
            // column (above row / above-right are the no-neighbour fallback). A
            // row>0 block (`haveAbove == 1`): reads the **real reconstructed above
            // row** and, when an already-decoded above-right superblock is in frame
            // (`num4AboveRight > 0`, derived by `full_sb_num4_above_right` over the
            // § 5.20.2.3 `BlockDecoded` state), the real § 7.13.2.1 above-right
            // sentinel — the same machinery the SMOOTH chroma grid path already
            // uses. Smooth prediction is linear interpolation over those edges (no
            // `enable_intra_edge_filter` / IDIF / upsample edge synthesis), so the
            // neighbour edge can be non-flat and the result is still bit-exact
            // against the AVM/dav2d oracle.
            //
            // SMOOTH_V at ANY 2-D grid position: its § 7.13.2.13 predictor reads
            // the above ROW and the bottom-left but never the above-right sentinel
            // VALUE (`AboveRow[w]`), so the row>0 above-row path is exactly what
            // the committed `syn-vgrid` fixture oracle-verifies.
            (Some(SupportedNonDcLumaMode::SmoothVertical), _) if n4w == FULL_SB_N4_LUMA => {}
            // SMOOTH_H reads the above-right sentinel VALUE (`AboveRow[w]`, the
            // top-right). In the first superblock row (`haveAbove == 0`) that is
            // the § 7.13.2.1 no-neighbour fallback (the shared edge builder is
            // verified and the value is the fallback). At row>0 it would be the
            // **real reconstructed** above-right of a decoded neighbour — a luma
            // (`sub_x == 0`) above-right VALUE path no oracle fixture has exercised
            // yet (the SMOOTH chroma grid verifies only `sub_x == 1`, and SMOOTH_V
            // row>0 ignores the value), so it is deferred until a SMOOTH_H luma
            // grid fixture pins it.
            (Some(SupportedNonDcLumaMode::SmoothHorizontal), _)
                if n4w == FULL_SB_N4_LUMA && frontier.r == 0 => {}
            (Some(SupportedNonDcLumaMode::SmoothHorizontal), _) if n4w == FULL_SB_N4_LUMA => {
                return Err(general_intra_unsupported(
                    "general_intra_smooth_h_above_right_unverified",
                    Some(tile_offset),
                    "general intra SMOOTH_H luma at superblock row > 0 reads the §7.13.2.1 real reconstructed above-right sentinel value (AboveRow[w]); that luma (sub_x=0) above-right value path is not yet covered by an oracle fixture (only SMOOTH_V row>0, which ignores the above-right value, and the sub_x=1 SMOOTH chroma grid are verified), so it is deferred to a dedicated SMOOTH_H luma grid fixture",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            (Some(_), _) if is_top_left => {
                return Err(general_intra_unsupported(
                    "general_intra_non_dc_non_dctonly_size",
                    Some(tile_offset),
                    "general intra non-DC luma prediction is only supported for 32x32-or-larger (TX_SET_DCTONLY) blocks; smaller non-DC blocks can signal a mode-dependent transform type that is not yet decoded",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            (Some(_), _) => {
                return Err(general_intra_unsupported(
                    "general_intra_multiblock_non_dc_subblock",
                    Some(tile_offset),
                    "general intra multi-block non-DC (SMOOTH_V / SMOOTH_H) luma prediction over a reconstructed neighbour is only supported for full 64x64 superblock blocks; sub-partitioned non-DC blocks need the §5.20.2.3 per-block BlockDecoded update for the §7.13.2.1 above-right / below-left neighbours, which is not yet modelled",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // Directional D135: top-left no-neighbour full superblock only.
            (_, Some(_)) if is_top_left && n4w == FULL_SB_N4_LUMA => {}
            (_, Some(_)) if !is_top_left => {
                return Err(general_intra_unsupported(
                    "general_intra_multiblock_directional_luma",
                    Some(tile_offset),
                    "general intra directional (D135) luma prediction is only supported for the top-left (no-neighbour) block; over a real reconstructed neighbour edge it needs the §7.13.2.8 IDIF 4-tap interpolation (bilinear equals IDIF only for a flat edge), which is not yet implemented",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            (_, Some(_)) => {
                return Err(general_intra_unsupported(
                    "general_intra_directional_non_dctonly_size",
                    Some(tile_offset),
                    "general intra directional (D135) luma prediction is only supported for the verified 64x64 (TX_SET_DCTONLY) superblock block; smaller directional blocks can signal a mode-dependent transform type that is not yet decoded",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            (None, None) => {
                return Err(general_intra_unsupported(
                    "general_intra_unsupported_luma_mode",
                    Some(tile_offset),
                    "general intra reconstruction only supports DC, SMOOTH_V / SMOOTH_H, and D135 (pAngle 135) luma prediction; SMOOTH, PAETH, other directional modes, and non-zero angle deltas are not yet implemented",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
        }
    }

    let uv_mode = usize::from(modes.uv_mode);
    let luma_log2 = n4w.trailing_zeros() + 2;
    let luma_tx = (luma_log2 - 2) as usize;
    let luma_x = frontier.c * 4;
    let luma_y = frontier.r * 4;
    let luma = crate::tile_payload::decode_general_intra_plane_coeffs(
        work_unit, symbols, coeff_ctx, 0, luma_tx, luma_x, luma_y, false, uv_mode,
    )
    .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    match (supported_nondc_luma, supported_directional_luma) {
        (Some(mode), _) if nondc_luma_has_neighbour => {
            // §7.13.2.1 `num4AboveRight` for the full-superblock luma block (the
            // gate restricts the neighbour-edge non-DC path to `n4w == 16`), from
            // §5.20.7.25 `count_top_right_avail`. Luma is not subsampled
            // (`sub_x == 0`). It only matters for SMOOTH_H / SMOOTH (the top-right
            // sentinel `AboveRow[w]`); SMOOTH_V's output never reads it.
            let num4_above_right =
                full_sb_num4_above_right(frontier.c, n4w, mi_cols, FRAME_LUMA_SUBSAMPLING_X);
            crate::runtime_minimal_recon::reconstruct_general_intra_luma_nondc_neighbour_block_into(
                workspace,
                &luma,
                mode,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                luma_use_tcq,
                num4_above_right,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        (Some(mode), _) => {
            crate::runtime_minimal_recon::reconstruct_general_intra_luma_nondc_first_block_into(
                workspace,
                &luma,
                mode,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                luma_use_tcq,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        (None, Some(mode)) => {
            crate::runtime_minimal_recon::reconstruct_general_intra_luma_directional_first_block_into(
                workspace,
                &luma,
                mode,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                luma_use_tcq,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        (None, None) => crate::runtime_minimal_recon::reconstruct_general_intra_block_into(
            workspace,
            &luma,
            PlaneId::Y,
            luma_x,
            luma_y,
            luma_log2,
            qindex,
            luma_use_tcq,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?,
    }

    if frontier.has_chroma {
        // 4:2:0: chroma is half-resolution, so the chroma transform/log2 is one
        // smaller and the chroma plane position is the luma position >> 1.
        let chroma_log2 = luma_log2 - 1;
        let chroma_tx = (chroma_log2 - 2) as usize;
        let chroma_x = frontier.c * 2;
        let chroma_y = frontier.r * 2;
        // §7.13.2.1 `num4AboveRight` for the full-superblock chroma block, from
        // §5.20.7.25 `count_top_right_avail` over the §5.20.2.3 `BlockDecoded`
        // state. SMOOTH chroma is gated to full-superblock blocks above, so the
        // block is the whole 64x64 superblock; the §7.13.2.13 top-right sentinel
        // needs the real reconstructed above-right sample when an in-frame,
        // already-decoded superblock sits to this superblock's upper-right.
        let num4_above_right =
            full_sb_num4_above_right(frontier.c, n4w, mi_cols, FRAME_420_SUBSAMPLING_X);
        let u = crate::tile_payload::decode_general_intra_plane_coeffs(
            work_unit, symbols, coeff_ctx, 1, chroma_tx, chroma_x, chroma_y, false, uv_mode,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        crate::runtime_minimal_recon::reconstruct_general_intra_chroma_block_into(
            workspace,
            &u,
            PlaneId::U,
            chroma_x,
            chroma_y,
            chroma_log2,
            qindex,
            supported_chroma,
            num4_above_right,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        let v = crate::tile_payload::decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            2,
            chroma_tx,
            chroma_x,
            chroma_y,
            !u.all_zero,
            uv_mode,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        crate::runtime_minimal_recon::reconstruct_general_intra_chroma_block_into(
            workspace,
            &v,
            PlaneId::V,
            chroma_x,
            chroma_y,
            chroma_log2,
            qindex,
            supported_chroma,
            num4_above_right,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    }
    // AV2 § 5.20.5.3: this block's IntraJointMode (the reorder index modeDelta)
    // is recorded into the IntraJointModes grid by the partition walk so later
    // blocks' § 8.3.2 `y_mode_index` context can read it as a neighbour.
    Ok(modes.intra_joint_mode)
}

/// 4:2:0 chroma horizontal subsampling (`SubsamplingX == 1`).
const FRAME_420_SUBSAMPLING_X: usize = 1;

/// Luma horizontal subsampling (`SubsamplingX == 0`).
const FRAME_LUMA_SUBSAMPLING_X: usize = 0;

/// Derives AV2 § 7.13.2.1 `num4AboveRight` (in plane 4x4 units) for a
/// full-superblock transform block, faithfully to § 5.20.7.25
/// `count_top_right_avail` over the § 5.20.2.3 `BlockDecoded` state. The plane is
/// selected by `sub_x` (`0` for luma, `1` for 4:2:0 chroma).
///
/// For a full 64x64 superblock the block coincides with the superblock, so its
/// sub-block MI position within the superblock is `(0, 0)` and its width in plane
/// 4x4 units is `w4 = n4w >> SubsamplingX` (the luma `n4w` 4x4 units subsampled).
/// `count_top_right_avail(plane, 0, 0, w4)` scans `BlockDecoded[plane][-1][w4 + i]`
/// for `i in 0..w4`; `clear_block_decoded_flags` (§ 5.20.2.3) marks the above row
/// decoded for plane columns `x < (MiColEnd - c) >> SubsamplingX` (a single
/// full-frame tile has `MiColEnd == MiCols`), so a column `w4 + i` is decoded while
/// `w4 + i < (MiCols - c) >> SubsamplingX`. The count stops at the first
/// undecoded column (or at `w4`), matching the spec loop's `break`.
pub(super) fn full_sb_num4_above_right(
    c: usize,
    n4w: usize,
    mi_cols: usize,
    sub_x: usize,
) -> usize {
    let w4 = n4w >> sub_x;
    // Chroma above-row decoded extent (in chroma 4x4 columns) for this
    // superblock, from `clear_block_decoded_flags` `sbWidth4 = (MiColEnd - c) >> subX`.
    let above_decoded_cols = mi_cols.saturating_sub(c) >> sub_x;
    let mut num_top_right = 0;
    for i in 0..w4 {
        if w4 + i < above_decoded_cols {
            num_top_right = i + 1;
        } else {
            break;
        }
    }
    num_top_right
}

/// Maps a general intra multi-block tree-walk error to a decode diagnostic. The
/// leaf-block error is already a structured `DecodeError`; setup, traversal, and
/// MI-size failures collapse to an unsupported-partition diagnostic.
fn map_general_intra_multiblock_error(
    error: crate::tile_payload::GeneralIntraMultiblockError<DecodeError>,
    tile_offset: ByteOffset,
) -> DecodeError {
    use crate::tile_payload::{GeneralIntraMultiblockError, GeneralIntraTreeWalkError};
    match error {
        GeneralIntraMultiblockError::Setup(error) => {
            general_intra_partition_frontier_error(error, tile_offset)
        }
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Leaf(error)) => error,
        // Preserve the resource-limit contract: a partition-step (or other)
        // limit must report as DecodeError::Limit, not unsupported-feature.
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Traversal(
            TilePartitionTraversalError::Limit(source),
        )) => DecodeError::Limit { source },
        GeneralIntraMultiblockError::Walk(_) => general_intra_unsupported(
            "general_intra_partition_walk",
            Some(tile_offset),
            "general intra partition tree walk reached an unsupported path",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ),
    }
}

fn general_intra_residual_error(
    error: GeneralIntraResidualError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        GeneralIntraResidualError::AllZeroRead { .. }
        | GeneralIntraResidualError::NonZeroPass { .. } => general_intra_unsupported(
            "general_intra_luma_coeff_parse",
            Some(offset),
            "general intra luma transform-block coefficient syntax could not be parsed from the tile payload",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::CoeffContextState { .. } => general_intra_unsupported(
            "general_intra_luma_coeff_state",
            Some(offset),
            "general intra luma coefficient context state could not be derived from the tile work unit",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::UnexpectedBranch => general_intra_unsupported(
            "general_intra_luma_coeff_unexpected_branch",
            Some(offset),
            "general intra luma coefficient decode produced an unexpected branch result",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::QuantLength { .. }
        | GeneralIntraResidualError::PredictionLength { .. }
        | GeneralIntraResidualError::Reconstruct { .. } => general_intra_unsupported(
            "general_intra_luma_reconstruct",
            Some(offset),
            "general intra luma transform-block reconstruction could not be composed from the decoded coefficients",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
    }
}

fn general_intra_block_mode_error(
    error: GeneralIntraBlockModeError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        GeneralIntraBlockModeError::SymbolRead { .. }
        | GeneralIntraBlockModeError::Literal { .. } => general_intra_unsupported(
            "general_intra_block_mode_parse",
            Some(offset),
            "general intra block mode-info syntax could not be parsed from the tile payload",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::UnsupportedYMode { .. } => general_intra_unsupported(
            "general_intra_unsupported_y_mode",
            Some(offset),
            "general intra decode currently reconstructs only the non-directional luma intra mode subset",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::InvalidUvMode { .. } => general_intra_unsupported(
            "general_intra_invalid_uv_mode",
            Some(offset),
            "general intra decode rejected an out-of-range chroma uv_mode index",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder { .. } => {
            general_intra_unsupported(
                "general_intra_directional_neighbour_reorder",
                Some(offset),
                "general intra y_mode_offset escape over a directional joint-mode neighbour needs the §5.20.5.3 directional-neighbour mode reorder (resolving to a directional mode that needs the deferred §7.13.2.8 luma IDIF)",
                GENERAL_INTRA_MODE_SPEC_SECTION,
            )
        }
    }
}

fn general_intra_partition_frontier_error(
    error: MinimalRuntimePartitionFrontierError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        MinimalRuntimePartitionFrontierError::Limit(source)
        | MinimalRuntimePartitionFrontierError::Traversal(TilePartitionTraversalError::Limit(
            source,
        )) => DecodeError::Limit { source },
        MinimalRuntimePartitionFrontierError::MissingFact { .. }
        | MinimalRuntimePartitionFrontierError::MiSizeState(_)
        | MinimalRuntimePartitionFrontierError::IntraJointModeState(_)
        | MinimalRuntimePartitionFrontierError::Traversal(_)
        | MinimalRuntimePartitionFrontierError::UnexpectedFrontier { .. } => {
            general_intra_unsupported(
                "general_intra_partition_frontier",
                Some(offset),
                "general intra decode could not reach a supported AV2 §5.20.3.1 single-block root partition frontier",
                GENERAL_INTRA_PARTITION_SPEC_SECTION,
            )
        }
    }
}

fn general_intra_unsupported(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            GENERAL_INTRA_TIER_ID,
            GENERAL_INTRA_MATRIX_ROW,
            GENERAL_INTRA_FEATURE_ID,
            spec_section,
            message,
            GENERAL_INTRA_REMEDIATION,
            byte_offset,
        )),
    }
}
