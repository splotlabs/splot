// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General minimal-tool intra decode frontier for the shared minimal-tier
//! runtime.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

use splot_core::headers::sequence::BitDepthIdc;
use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{BitDepth, CurrentFrameWorkspace, PlaneId, ReconSample};

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
            //
            // §5.20.5.3 `read_intra_y_mode` reads `mrl_index` (an `S()` arithmetic
            // symbol; conditionally `mrl_sec_index`) when
            // `enable_mrls && is_directional_mode(YMode)`. The general path reads no
            // MRL symbol, so an `enable_mrls` stream with a directional (D135) block
            // would desync the §8.2 arithmetic decoder at the skipped read. §7.13.2
            // runs the §7.13.2.18 intra edge filter on the prediction edges before
            // directional prediction when `enable_intra_edge_filter == 1 && MrlIndex
            // == 0`; the general path applies no edge filter, so over a real
            // (non-flat) neighbour edge a directional block would reconstruct wrong
            // pixels. Reject both until the MRL syntax and the §7.13.2.18 edge filter
            // are modelled (all committed general fixtures are encoded with
            // `enable_mrls == enable_intra_edge_filter == 0`; the frozen minimal-tier
            // fixture carries `enable_intra_edge_filter == 1` but routes here as false
            // via its `base_q_idx` and never reaches directional prediction).
            .is_some_and(|intra| {
                !intra.enable_dip
                    && !intra.enable_ibp
                    && !intra.enable_mrls
                    && !intra.enable_intra_edge_filter
            })
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

    // §6.4.1: the sequence bit depth selects the reconstruction sample storage
    // type (Eight -> `u8`, Ten -> `u16`). The whole reconstruction graph is
    // generic over `T: ReconSample`; dispatch to the matching specialization and
    // wrap the frozen frame in the matching output-carrier arm. 10-bit is
    // admitted only for the verified DC subset (gated per-block inside
    // `decode_one_general_intra_block`); a 10-bit non-DC / non-64x64-leaf shape
    // rejects before any sample write.
    let bit_depth = match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight => BitDepth::Eight,
        BitDepthIdc::Ten => BitDepth::Ten,
    };

    // Enforce the configured decode limits before allocating reconstruction
    // buffers; the byte budget charges the active bit depth (a 10-bit frame
    // allocates two bytes per sample) so an over-limit 10-bit frame fails before
    // the `DecodedFrame<u16>` workspace is allocated.
    ensure_runtime_limits(limits, frame_width, frame_height, tile_size, bit_depth)?;

    let frame = match bit_depth {
        BitDepth::Eight => {
            MinimalRuntimeDecodedFrame::Eight(decode_general_intra_frame_into::<u8>(
                tile,
                sequence,
                core,
                limits,
                frame_width as usize,
                frame_height as usize,
                mi_rows,
                mi_cols,
                qindex,
                luma_use_tcq,
                bit_depth,
                tile_offset,
            )?)
        }
        BitDepth::Ten => MinimalRuntimeDecodedFrame::Ten(decode_general_intra_frame_into::<u16>(
            tile,
            sequence,
            core,
            limits,
            frame_width as usize,
            frame_height as usize,
            mi_rows,
            mi_cols,
            qindex,
            luma_use_tcq,
            bit_depth,
            tile_offset,
        )?),
    };
    Ok(MinimalRuntimeFrame {
        frame,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

/// Reconstructs the general intra key frame into a `DecodedFrame<T>` for the
/// sample storage type `T` selected by the active sequence `bit_depth` (§ 6.4.1).
///
/// This is the storage-typed body of [`decode_general_minimal_intra_frame`]: it
/// builds the typed `CurrentFrameWorkspace<T>`, walks the real AV2 § 5.20.3.1
/// partition tree decoding each leaf's § 5.20.5.3 mode info and § 5.20.7.27
/// Y/U/V coefficients, reconstructs each block into the workspace in decode order
/// (so later blocks predict from the already-reconstructed neighbours),
/// validates § 8.2.4 `exit_symbol()`, and freezes the workspace.
#[allow(clippy::too_many_arguments)]
fn decode_general_intra_frame_into<T: ReconSample>(
    tile: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: crate::DecodeLimits,
    frame_width: usize,
    frame_height: usize,
    mi_rows: usize,
    mi_cols: usize,
    qindex: u32,
    luma_use_tcq: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<DecodedFrame<T>> {
    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace::<T>(
        frame_width,
        frame_height,
        bit_depth,
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
        |work_unit, symbols, frontier, joint_modes, uses_mrls, fsc_modes, block_decoded| {
            decode_one_general_intra_block::<T>(
                work_unit,
                symbols,
                frontier,
                joint_modes,
                uses_mrls,
                fsc_modes,
                block_decoded,
                &mut workspace,
                &mut coeff_ctx,
                qindex,
                luma_use_tcq,
                mi_cols,
                bit_depth,
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

    Ok(workspace.freeze()?)
}

/// Decodes one general intra leaf block (mode info + Y/U/V coefficients) and
/// reconstructs it into `workspace` in decode order. Gated to square DC_PRED
/// blocks: the no-neighbour-aware §7.13.2 DC prediction is read from the
/// partially-built frame, so non-DC modes and non-square partitions are
/// rejected. Chroma is 4:2:0 (half-resolution).
///
/// Returns the block's AV2 § 5.20.5.3 luma mode state so the caller can record
/// it into the `IntraJointModes` / `YModes` grids for later blocks' contexts;
/// `joint_modes` supplies that grid (read-only here) for this block's own
/// `y_mode_index` context.
#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_block<T: ReconSample>(
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::tile_payload::DecodeBlockFrontier,
    joint_modes: &crate::tile_payload::TileIntraJointModeState,
    uses_mrls: &crate::tile_payload::TileUsesMrlsState,
    fsc_modes: &crate::tile_payload::TileFscModeState,
    block_decoded: &crate::tile_payload::TileBlockDecodedState,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    qindex: u32,
    luma_use_tcq: bool,
    mi_cols: usize,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<crate::tile_payload::GeneralIntraLeafMode> {
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
    // 4:2:0 sub-8x8 luma leaves defer chroma to the bottom-right 4x4 (a 4x4 chroma
    // transform over the 8x8 region, not luma_log2 - 1), and the other three are
    // luma-only; neither chroma sizing/position is modelled yet. This applies to
    // both square and rectangular leaves (the smaller dimension is checked).
    if n4w < 2 || n4h < 2 {
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
        crate::tile_payload::GeneralIntraChromaToolConfig::disabled(),
        joint_modes,
        uses_mrls,
        fsc_modes,
        frontier.b_size.index(),
        frontier.r,
        frontier.c,
        n4w,
        n4h,
    )
    .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
    if modes.uses_active_mrl() {
        return Err(general_intra_unsupported(
            "general_intra_unsupported_mrl_mode",
            Some(tile_offset),
            "general intra decode can retain active MRL mode-info but does not support §7.13.2 MRL prediction",
            "7.13.2",
        ));
    }
    if modes.uses_active_fsc() {
        return Err(general_intra_unsupported(
            "general_intra_unsupported_fsc_mode",
            Some(tile_offset),
            "general intra decode can retain active FSC mode-info but does not support FSC/IDTX reconstruction",
            "5.20.7.27",
        ));
    }

    // VERIFIED-SUBSET DISCIPLINE (§6.4.1 10-bit): the 8-bit general intra path
    // reconstructs DC, SMOOTH, directional, cardinal, and one-sided modes, all
    // oracle-verified against committed 8-bit fixtures. At 10-bit, the
    // oracle-verified subset is the DC_PRED-luma SQUARE-leaf shape with either:
    //   - DC chroma — single 64x64 (`syn-flat-intra-64x64-10bit-q80.ivf` flat DC,
    //     `syn-cos-intra-64x64-10bit-q180.ivf` AC residual) and multi-64x64-
    //     superblock (`syn-2sb-intra-128x64-10bit-q80.ivf`); or
    //   - §7.13.2.13 SMOOTH chroma over the §7.13.2.1 NO-NEIGHBOUR fallback edges
    //     at the top-left block (`frontier.r == 0 && frontier.c == 0`), pinned by
    //     `syn-smchroma-intra-64x64-10bit-q160.ivf`.
    // Each is byte-exact vs avmdec AND dav2d. The recon math is bit-depth-generic
    // so other 10-bit shapes WOULD reconstruct, but they are not yet pinned by a
    // 10-bit oracle fixture: a 10-bit non-DC LUMA block, a non-DC / non-(top-left
    // SMOOTH) CHROMA block, or a neighbour-having SMOOTH chroma block (which reads
    // real reconstructed 10-bit edges no fixture pins) is rejected before any
    // coefficient read or sample write. (8-bit is unaffected: this guard never
    // fires for `BitDepth::Eight`.) A confident-but-unverified 10-bit hash is the
    // cardinal sin; over-rejecting is safe.
    let chroma_admitted_10bit = match modes.supported_chroma_mode() {
        Some(crate::tile_payload::SupportedChromaMode::Dc) => true,
        Some(crate::tile_payload::SupportedChromaMode::Smooth) => {
            frontier.r == 0 && frontier.c == 0
        }
        _ => false,
    };
    if bit_depth != BitDepth::Eight && (!modes.luma_is_dc() || !chroma_admitted_10bit) {
        return Err(general_intra_unsupported(
            "unsupported_10bit_non_dc_intra",
            Some(tile_offset),
            "general intra 10-bit reconstruction is only oracle-verified for DC_PRED luma with DC chroma, or no-neighbour top-left SMOOTH chroma; a 10-bit non-DC luma, other non-DC chroma, or neighbour-having SMOOTH chroma block is deferred until a 10-bit oracle fixture pins it",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }

    // 10-bit reconstruction is oracle-verified ONLY for the FULL 64x64 square leaf
    // (n4w == n4h == FULL_SB_N4_LUMA): the committed 10-bit fixtures are single
    // 64x64 and multi-64x64-superblock frames, every leaf a PARTITION_NONE 64x64
    // block. Any non-64x64 leaf — a rectangular 64x32 PARTITION_HORZ child
    // (n4w != n4h) OR a split square 32x32 / 16x16 sub-block (n4w == n4h < 16) —
    // has NO committed 10-bit oracle fixture. The §7.14.4/§7.15.4 reconstruction is
    // bit-depth-generic so such a leaf WOULD reconstruct, but admitting it unpinned
    // risks a confident-but-unverified 10-bit hash (the cardinal sin), so reject it
    // before the rectangular dispatch and the coefficient loop. (8-bit is
    // unaffected: this guard never fires for `BitDepth::Eight`.) `FULL_SB_N4_LUMA`
    // (== 16, the 64x64 superblock width in 4x4 units) is defined below in this fn.
    if bit_depth != BitDepth::Eight && (n4w != FULL_SB_N4_LUMA || n4h != FULL_SB_N4_LUMA) {
        return Err(general_intra_unsupported(
            "unsupported_10bit_non_64x64_leaf",
            Some(tile_offset),
            "general intra 10-bit reconstruction is only oracle-verified for full 64x64 square DC leaves; a 10-bit non-64x64 partition leaf (rectangular, or a split 32x32 / 16x16 square sub-block) is deferred until a 10-bit oracle fixture pins it",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }

    // RECTANGULAR PARTITION LEAF (`n4w != n4h`, e.g. a 64x32 PARTITION_HORZ child
    // or a 32x64 PARTITION_VERT child). The §7.13.2.4 DC predictor reads only the
    // immediate in-frame left column / above row from the partially-built frame
    // (no §7.13.2.1 above-right / below-left sentinels, so no §5.20.2.3
    // BlockDecoded state is needed), and the §5.20.7.27 coefficient loop +
    // §7.14.4/§7.15.4 reconstruction already read transform width and height
    // independently from the §9.2 conversion tables (incl. the §7.15.4.1 √2 rescale
    // for an odd log2 ratio). So a DC_PRED luma + DC chroma rectangular leaf is
    // reconstructed bit-exact via the dedicated rectangular path; any non-DC luma
    // or non-DC chroma rectangular mode (which would need rectangular §7.13.2.8 /
    // §7.13.2.13 prediction edges, not yet modelled) is rejected, keeping every
    // square mode path below unchanged.
    if n4w != n4h {
        return decode_one_general_intra_rect_block::<T>(
            work_unit,
            symbols,
            frontier,
            &modes,
            workspace,
            coeff_ctx,
            qindex,
            n4w,
            n4h,
            bit_depth,
            tile_offset,
        );
    }

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
            "general intra reconstruction only supports DC, SMOOTH, the cardinal V/H directional-follow, and the D135 / D157 directional-follow chroma prediction; other non-DC chroma (uv_mode) modes are not yet implemented",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    };
    // The directional-follow D135 chroma predictor is verified bit-exact for the
    // top-left no-neighbour 64x64 superblock (the §7.13.2.1 flat-fallback edges)
    // AND for a first-superblock-row (`frontier.r == 0`, `haveAbove == 0`),
    // non-top-left full-superblock block whose §7.13.2.1 chroma edges are the
    // **real reconstructed** left column of the already-decoded left neighbour.
    // Chroma directional prediction always takes the §7.13.2.8 bilinear branch
    // (`enableIdif = plane == 0`, so `enableIdif == 0` for U/V); for D135
    // (`shift == 0`) that bilinear branch is the sample copy `Edge[base]`, which is
    // bit-identical to the luma IDIF even over the non-flat real chroma edge
    // (verified against avmdec/dav2d). It couples with the neighbour-having D135
    // luma block (`uv_mode == 0` directional-follow). The first-superblock-row
    // (`frontier.r == 0`) and a row>0 non-first-column (`frontier.r != 0 &&
    // frontier.c != 0`) full-superblock block are supported: the latter reads the
    // real reconstructed §7.13.2.1 above chroma row, left chroma column, AND
    // diagonally-above-left chroma corner via the same plane-general
    // [`build_directional_middle_edges`] `(true, true)` arm the luma uses. (Chroma is
    // 4:2:0, so the chroma block is the half-resolution image of the 64x64 luma
    // superblock and is itself a `haveLeft && haveAbove` block at this position.)
    let chroma_is_top_left = frontier.r == 0 && frontier.c == 0;
    const FULL_SB_N4_CHROMA_GATE: usize = 16;
    let chroma_first_row_neighbour_ok = frontier.r == 0 && n4w == FULL_SB_N4_CHROMA_GATE;
    // Row>0 non-first-column full-superblock (`haveLeft && haveAbove`): the fixtured
    // row>0 D135-follow chroma position. The row>0 FIRST-column (`!haveLeft &&
    // haveAbove`) position is not yet fixtured and stays rejected.
    let chroma_row_gt0_neighbour_ok =
        frontier.r != 0 && frontier.c != 0 && n4w == FULL_SB_N4_CHROMA_GATE;
    if supported_chroma == crate::tile_payload::SupportedChromaMode::D135Follow
        && !((chroma_is_top_left && n4w == FULL_SB_N4_CHROMA_GATE)
            || chroma_first_row_neighbour_ok
            || chroma_row_gt0_neighbour_ok)
    {
        return Err(general_intra_unsupported(
            "general_intra_directional_chroma_neighbour",
            Some(tile_offset),
            "general intra directional-follow (D135) chroma prediction is supported for the top-left (no-neighbour) 64x64 superblock block, a first-superblock-row neighbour-having full 64x64 superblock block, and a row > 0 non-first-column full 64x64 superblock block (real reconstructed above row + left column + diagonally-above-left corner); a row > 0 FIRST-column (!haveLeft && haveAbove) or sub-partitioned D135-follow chroma block is deferred until an oracle fixture pins it",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // Directional-follow D113 chroma: only the neighbour-having row>0,
    // non-first-column position is fixtured (it couples with the D113 luma block,
    // which is gated identically). Chroma takes the `enableIdif == 0` bilinear
    // branch — the spec-mandated chroma branch (`enableIdif = plane == 0`) — over
    // the real reconstructed §7.13.2.1 above row + left column + diagonally-above
    // -left corner, so it is bit-exact against avmdec/dav2d.
    if supported_chroma == crate::tile_payload::SupportedChromaMode::D113Follow
        && !(frontier.r != 0 && frontier.c != 0 && n4w == FULL_SB_N4_CHROMA_GATE)
    {
        return Err(general_intra_unsupported(
            "general_intra_directional_d113_chroma_neighbour",
            Some(tile_offset),
            "general intra directional-follow (D113) chroma prediction is only supported for a row>0, non-first-column neighbour-having full 64x64 superblock block reading the real reconstructed §7.13.2.1 above row + left column + diagonally-above-left corner; the top-left, first-row, first-column, sub-partitioned, and non-64x64 D113-follow chroma positions are deferred until an oracle fixture pins them",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // Directional-follow D157 chroma: only the neighbour-having
    // `frontier.r == 0 && frontier.c != 0` position is fixtured (it couples with
    // the D157 luma block, which is gated identically). Chroma takes the
    // `enableIdif == 0` bilinear branch over the real reconstructed §7.13.2.1 left
    // chroma column; over a flat real chroma edge that projection is bit-exact.
    if supported_chroma == crate::tile_payload::SupportedChromaMode::D157Follow
        && !(frontier.r == 0 && frontier.c != 0 && n4w == FULL_SB_N4_CHROMA_GATE)
    {
        return Err(general_intra_unsupported(
            "general_intra_directional_d157_chroma_neighbour",
            Some(tile_offset),
            "general intra directional-follow (D157) chroma prediction is only supported for a first-superblock-row, non-first-column neighbour-having full 64x64 superblock block reading the real reconstructed §7.13.2.1 left chroma column; the top-left, first-column, sub-partitioned, and row>0 D157-follow chroma positions are deferred until an oracle fixture pins them",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // Directional-follow D45 chroma (§7.13.2.8 ZONE-1 one-sided): only the
    // neighbour-having row>0, non-first-column, NON-rightmost position is fixtured
    // (it couples with the D45 luma block, gated identically). Chroma takes the
    // `enableIdif == 0` bilinear one-sided branch over the real reconstructed
    // §7.13.2.1 above row + above-right; for D45 (`shift == 0`) it is the sample
    // copy `AboveRow[base]`, bit-exact. The chroma above-right availability is
    // derived for the chroma plane (`sub_x == 1`) so the half-resolution block's
    // decoded above-right is counted; a rightmost superblock (no decoded
    // above-right) is rejected.
    if supported_chroma == crate::tile_payload::SupportedChromaMode::D45Follow
        && !(frontier.r != 0
            && frontier.c != 0
            && n4w == FULL_SB_N4_CHROMA_GATE
            && full_sb_num4_above_right(frontier.c, n4w, mi_cols, 1) > 0)
    {
        return Err(general_intra_unsupported(
            "general_intra_directional_d45_chroma_neighbour",
            Some(tile_offset),
            "general intra directional-follow (D45) chroma prediction is only supported for a row>0, non-first-column, non-rightmost neighbour-having full 64x64 superblock block reading the real reconstructed §7.13.2.1 above row + above-right; the top-left, first-row, first-column, rightmost, sub-partitioned, and non-64x64 D45-follow chroma positions are deferred until an oracle fixture pins them",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // Cardinal directional-follow V_PRED / H_PRED chroma: a degenerate §7.13.2.8
    // copy of the real reconstructed §7.13.2.1 above row (V, chroma `frontier.r !=
    // 0`) or left column (H, chroma `frontier.c != 0`). It couples with the
    // matching neighbour-having cardinal luma block (`uv_mode == 0` follow). Gated
    // to the full 64x64 superblock (`n4w == 16`), mirroring the luma gate; a chroma
    // block on the first superblock row (V) / first superblock column (H) would
    // read the §7.13.2.1 no-neighbour fallback, which is not what the luma gate
    // admits, so reject it.
    if supported_chroma == crate::tile_payload::SupportedChromaMode::VerticalFollow
        && !(frontier.r != 0 && n4w == FULL_SB_N4_CHROMA_GATE)
    {
        return Err(general_intra_unsupported(
            "general_intra_cardinal_vertical_chroma",
            Some(tile_offset),
            "general intra directional-follow V_PRED (pAngle 90) chroma prediction is only supported for a row>0 full 64x64 superblock block reading the real reconstructed §7.13.2.1 above row; a first-superblock-row or sub-partitioned block is not yet covered by an oracle fixture",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    if supported_chroma == crate::tile_payload::SupportedChromaMode::HorizontalFollow
        && !(frontier.c != 0 && n4w == FULL_SB_N4_CHROMA_GATE)
    {
        return Err(general_intra_unsupported(
            "general_intra_cardinal_horizontal_chroma",
            Some(tile_offset),
            "general intra directional-follow H_PRED (pAngle 180) chroma prediction is only supported for a non-first-column full 64x64 superblock block reading the real reconstructed §7.13.2.1 left column; a first-superblock-column or sub-partitioned block is not yet covered by an oracle fixture",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // Non-follow H_PRED chroma (explicit uv_mode over a non-directional luma):
    // §7.13.2.8 pAngle 180 reads ONLY the §7.13.2.1 left column. Supported only at
    // the no-neighbour top-left full 64x64 superblock block, where the left column
    // is the flat fallback, so the horizontal copy is bit-exact. A neighbour-having
    // position would read the real reconstructed left column over a possibly
    // non-flat edge and is deferred until an oracle fixture pins it.
    if supported_chroma == crate::tile_payload::SupportedChromaMode::Horizontal
        && !(chroma_is_top_left && n4w == FULL_SB_N4_CHROMA_GATE)
    {
        return Err(general_intra_unsupported(
            "general_intra_horizontal_chroma_position",
            Some(tile_offset),
            "general intra non-follow H_PRED (pAngle 180) chroma prediction is only supported at the no-neighbour top-left full 64x64 superblock block reading the §7.13.2.1 flat fallback left column; a neighbour-having or sub-partitioned position is not yet covered by an oracle fixture",
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
    // True when a directional (D135) luma block reads a **real** reconstructed
    // neighbour edge. The gate above admits this only for a first-superblock-row
    // (`frontier.r == 0`, `haveAbove == 0`) full-superblock block, whose left
    // neighbour is non-directional (`ctx == 0`) and supplies the real §7.13.2.1
    // reconstructed left column; D135 (`shift == 0`) is a sample copy over that
    // non-flat edge, bit-identical for the luma IDIF and the chroma bilinear branch.
    let directional_luma_has_neighbour = supported_directional_luma.is_some() && !is_top_left;
    if !modes.luma_is_dc() {
        match (supported_nondc_luma, supported_directional_luma) {
            // Plain SMOOTH at the no-neighbour top-left block. Its § 7.13.2.13
            // predictor reads BOTH the above-right sentinel `AboveRow[w]` (the
            // top-right) AND the below-left sentinel `LeftCol[h]`; at the top-left
            // block both are the § 7.13.2.1 no-neighbour fallback (8-bit `127` /
            // `129`), so the 2-D interpolation is fully deterministic over the
            // verified fallback edges. Gated to the verified 64x64 superblock
            // (`n4w == 16`, TX_64X64 -> TX_SET_DCTONLY): a sub-64x64 plain-SMOOTH
            // block can signal a mode-dependent (non-DCT) transform type that is
            // not yet decoded, and the neighbour-having plain-SMOOTH above-right /
            // below-left sentinel paths are not yet covered by an oracle fixture,
            // so they are rejected below.
            (Some(SupportedNonDcLumaMode::Smooth), _) if is_top_left && n4w == FULL_SB_N4_LUMA => {}
            // SMOOTH_V / SMOOTH_H at the no-neighbour top-left block.
            (
                Some(
                    SupportedNonDcLumaMode::SmoothVertical
                    | SupportedNonDcLumaMode::SmoothHorizontal,
                ),
                _,
            ) if is_top_left && n4w >= NON_DC_MIN_N4 => {}
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
            // SMOOTH_H full-superblock block reads the above-right sentinel VALUE
            // (`AboveRow[w]`, the top-right) at ANY 2-D grid position. In the first
            // superblock row (`haveAbove == 0`) that is the § 7.13.2.1 no-neighbour
            // fallback (the shared edge builder is verified and the value is the
            // fallback). At a row>0 superblock the § 7.13.2.1 top-right sentinel is
            // the **real reconstructed** bottom row of the already-decoded
            // diagonally-above-right superblock, resolved by
            // `luma_num4_above_right_from_block_decoded` (§ 5.20.7.25
            // `count_top_right_avail` over the § 5.20.2.3 `BlockDecoded` state) and
            // `resolve_smooth_above_right_sentinel` — the same plane-general
            // machinery the SMOOTH chroma grid path already uses for `sub_x == 1`,
            // now exercised for the luma (`sub_x == 0`) above-right VALUE. Smooth
            // prediction is linear interpolation over those edges (no
            // `enable_intra_edge_filter` / IDIF / upsample synthesis), so the
            // non-flat real above-right reconstructs bit-exact against the
            // AVM/dav2d oracle (the `syn-shgrid` fixture pins it).
            (Some(SupportedNonDcLumaMode::SmoothHorizontal), _) if n4w == FULL_SB_N4_LUMA => {}
            (Some(_), _) if is_top_left => {
                return Err(general_intra_unsupported(
                    "general_intra_non_dc_non_dctonly_size",
                    Some(tile_offset),
                    "general intra non-DC luma prediction is only supported for 32x32-or-larger (TX_SET_DCTONLY) blocks; smaller non-DC blocks can signal a mode-dependent transform type that is not yet decoded",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // SMOOTH_H sub-partitioned (SPLIT-child) block, 32x32-or-larger
            // (TX_SET_DCTONLY), reading the real § 7.13.2.1 above-right sentinel
            // VALUE (`AboveRow[w]`) from an already-decoded intra-superblock sibling
            // via § 5.20.7.25 `count_top_right_avail` over the real § 5.20.2.3
            // `BlockDecoded` state — but ONLY a WITHIN-superblock above-right: a
            // child at superblock-relative MI row > 0 (`frontier.r % FULL_SB_N4_LUMA
            // != 0`, e.g. the bottom-left 32x32 of a SPLIT 64x64 superblock reading
            // the already-decoded top-right 32x32 sibling's bottom row). That is the
            // case the `syn-shsplit` fixture oracle-verifies, and it is independent
            // of the superblock's frame row. A child at superblock-relative row 0
            // (`frontier.r % FULL_SB_N4_LUMA == 0`) instead reads its above-right
            // from the superblock ABOVE (a cross-superblock, row>0 neighbour) — the
            // SAME luma (`sub_x == 0`) above-right VALUE path the full-superblock arm
            // defers (`general_intra_smooth_h_above_right_unverified`), so it is
            // rejected here too until a multi-superblock-row SMOOTH_H luma fixture
            // pins it. SMOOTH_V is NOT admitted here (its § 7.13.2.13 predictor reads
            // the below-left sentinel `LeftCol[h]`, whose `num4BelowLeft` over a
            // SPLIT child is a separate, not-yet-fixtured path).
            (Some(SupportedNonDcLumaMode::SmoothHorizontal), _)
                if n4w >= NON_DC_MIN_N4 && !frontier.r.is_multiple_of(FULL_SB_N4_LUMA) => {}
            (Some(SupportedNonDcLumaMode::SmoothHorizontal), _) if n4w >= NON_DC_MIN_N4 => {
                return Err(general_intra_unsupported(
                    "general_intra_smooth_h_above_right_unverified",
                    Some(tile_offset),
                    "general intra SMOOTH_H sub-partitioned luma at superblock-relative row 0 reads the §7.13.2.1 above-right sentinel value (AboveRow[w]) from a cross-superblock (row>0) decoded neighbour — the same luma (sub_x=0) above-right value path the full-superblock arm defers; only the within-superblock above-right sibling is oracle-verified, so it is deferred to a multi-superblock-row SMOOTH_H luma fixture",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            (Some(_), _) => {
                return Err(general_intra_unsupported(
                    "general_intra_multiblock_non_dc_subblock",
                    Some(tile_offset),
                    "general intra multi-block SMOOTH_V luma prediction over a reconstructed neighbour is only supported for full 64x64 superblock blocks; a sub-partitioned SMOOTH_V block reads the §7.13.2.1 below-left sentinel (LeftCol[h], §5.20.7.25 count_bottom_left_avail), which is not yet covered by an oracle fixture",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // Cardinal V_PRED (§7.13.2.8 step 4, pAngle 90): a pure VERTICAL copy
            // of the §7.13.2.1 above row (`pred[i][j] = AboveRow[j]`). It reads ONLY
            // the real reconstructed above row, so it needs a row>0 full-superblock
            // block (`frontier.r != 0`, `haveAbove == 1`); `intra_dc_edges_for_rect`
            // returns `CurrFrame[0][y-1][x..x+w)`. The escape/first-set decode was
            // resolved with a non-directional joint-mode neighbour (`ctx == 0`; a
            // directional neighbour would have been rejected earlier as the
            // unmodelled §5.20.5.3 directional-neighbour reorder). The cardinal copy
            // has NO IDIF, NO corner, NO `useIBP` (§7.13.2.7 gates `useIBP` on
            // `pAngle < 90 || pAngle > 180`), so it is bit-exact over the non-flat
            // real above row; `enable_intra_edge_filter == 0` / `MrlIndex == 0` keep
            // the §7.13.2.7 edge-filter step a no-op (and §7.13.2.7 skips it entirely
            // for `pAngle == 90`). Gated to the full 64x64 superblock (`n4w == 16`,
            // TX_64X64 -> TX_SET_DCTONLY); a row 0 V_PRED block reads the §7.13.2.1
            // no-neighbour above fallback and is not covered by an oracle fixture, so
            // it is rejected by the generic directional arm below.
            (_, Some(SupportedDirectionalLumaMode::Vertical))
                if frontier.r != 0 && n4w == FULL_SB_N4_LUMA => {}
            (_, Some(SupportedDirectionalLumaMode::Vertical)) => {
                return Err(general_intra_unsupported(
                    "general_intra_cardinal_vertical_unverified",
                    Some(tile_offset),
                    "general intra cardinal V_PRED (pAngle 90) luma prediction is only verified for a row>0 full 64x64 superblock block reading the real reconstructed §7.13.2.1 above row; a first-superblock-row (haveAbove == 0) or sub-partitioned V_PRED block is not yet covered by an oracle fixture",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // Cardinal H_PRED (§7.13.2.8 step 5, pAngle 180): a pure HORIZONTAL copy
            // of the §7.13.2.1 left column (`pred[i][j] = LeftCol[i]`). It reads ONLY
            // the real reconstructed left column, so it needs a non-first-column
            // full-superblock block (`frontier.c != 0`, `haveLeft == 1`);
            // `intra_dc_edges_for_rect` returns `CurrFrame[0][y..y+h)][x-1]`. Same
            // ctx == 0 / no-IDIF / no-useIBP argument as V_PRED above (§7.13.2.7 skips
            // the edge filter for `pAngle == 180` too). Gated to the full 64x64
            // superblock (`n4w == 16`); a first-superblock-column H_PRED block reads
            // the §7.13.2.1 no-neighbour left fallback and is rejected below.
            (_, Some(SupportedDirectionalLumaMode::Horizontal))
                if frontier.c != 0 && n4w == FULL_SB_N4_LUMA => {}
            (_, Some(SupportedDirectionalLumaMode::Horizontal)) => {
                return Err(general_intra_unsupported(
                    "general_intra_cardinal_horizontal_unverified",
                    Some(tile_offset),
                    "general intra cardinal H_PRED (pAngle 180) luma prediction is only verified for a non-first-column full 64x64 superblock block reading the real reconstructed §7.13.2.1 left column; a first-superblock-column (haveLeft == 0) or sub-partitioned H_PRED block is not yet covered by an oracle fixture",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // Directional D157 (§7.13.2.8 middle angle, pAngle 157) at a
            // first-superblock-row, NON-first-column full-superblock block
            // (`frontier.r == 0 && frontier.c != 0`, `haveLeft && !haveAbove`),
            // reading the **real reconstructed** §7.13.2.1 left column. Unlike
            // D135, D157's projections (`dx == Dr_Intra_Derivative[23] == 170`,
            // `dy == Dr_Intra_Derivative[67] == 24`) mostly have `shift != 0`, so
            // the luma IDIF 4-tap `Dr_Interp_Filter` genuinely interpolates and
            // differs from the bilinear branch — this is the brick that
            // oracle-verifies the real luma IDIF kernel. At `haveLeft &&
            // !haveAbove` the §7.13.2.1 corner `AboveRow[-1] == LeftCol[-1]` is the
            // repeated first left sample (`CurrFrame[plane][y][x-1]`), so the
            // few above-branch corner reads (`above_base == -1`) are correct and
            // the deferred real-above corner is NOT a blocker. The D157 escape was
            // decoded with a non-directional joint-mode neighbour (`ctx == 0`).
            // `enable_intra_edge_filter == 0` / `MrlIndex == 0` keep the
            // §7.13.2.x edge-filter / upsample synthesis a no-op.
            //
            // D157 is gated tighter than D135 (no top-left no-neighbour block, no
            // first-column block): only the `frontier.c != 0` first-row position is
            // fixtured. The top-left and row>0 D157 positions read the §7.13.2.1
            // no-neighbour / real-above corner that no oracle fixture pins yet.
            (_, Some(SupportedDirectionalLumaMode::D157))
                if frontier.r == 0 && frontier.c != 0 && n4w == FULL_SB_N4_LUMA => {}
            (_, Some(SupportedDirectionalLumaMode::D157)) => {
                return Err(general_intra_unsupported(
                    "general_intra_d157_unverified_position",
                    Some(tile_offset),
                    "general intra directional D157 (pAngle 157) luma IDIF prediction is only verified for a first-superblock-row, non-first-column full 64x64 superblock block (haveLeft && !haveAbove, real reconstructed §7.13.2.1 left column); the top-left no-neighbour, first-column, sub-partitioned, and row>0 D157 positions read the §7.13.2.1 corner / above row that no oracle fixture pins yet, so they are deferred",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // Directional D113 (§7.13.2.8 middle angle, pAngle 113) at a row>0,
            // NON-first-column full-superblock block (`frontier.r != 0 &&
            // frontier.c != 0`, `haveLeft && haveAbove`), reading the **real
            // reconstructed** §7.13.2.1 above row, left column, AND the
            // diagonally-above-left corner `CurrFrame[plane][y-1][x-1]`. D113 is
            // vertical-leaning (`dx == Dr_Intra_Derivative[180 - 113] ==
            // Dr_Intra_Derivative[67] == 24`, `dy == Dr_Intra_Derivative[113 - 90]
            // == Dr_Intra_Derivative[23] == 170`): most projections take the above
            // branch (`base >= -(1 + mrlIndex)`) and land on a NONZERO `shift`, so
            // the §7.13.2.8 luma IDIF 4-tap `Dr_Interp_Filter` genuinely
            // interpolates over the real above row + corner (unlike D135, whose
            // `shift == 0` reduces the IDIF to a copy). The corner read
            // `AboveRow[-1] == LeftCol[-1] == CurrFrame[plane][y-1][x-1]` is
            // supplied by [`build_directional_middle_edges`]'s `(true, true)` arm
            // via `reconstructed_sample`. The D113 escape (`y_mode_offset == 2`,
            // §5.20.5.3 modeIdx 9 -> modeDelta 29 -> Reordered_Y_Mode[8] == D113,
            // AngleDeltaY 0) is decoded with a non-directional joint-mode neighbour
            // (`ctx == 0`; a directional neighbour reorder is rejected earlier).
            // `enable_intra_edge_filter == 0` / `MrlIndex == 0` keep the §7.13.2.x
            // edge-filter / upsample synthesis a no-op.
            //
            // Gated to the row>0 non-first-column position (`haveLeft &&
            // haveAbove`): a first-superblock-row D113 block (`haveAbove == 0`)
            // would read the §7.13.2.1 above fallback that no oracle fixture pins,
            // and D113 reads the real above row (so first-row is degenerate). The
            // first-column, top-left, sub-partitioned, and non-64x64 D113 positions
            // are rejected below.
            (_, Some(SupportedDirectionalLumaMode::D113))
                if frontier.r != 0 && frontier.c != 0 && n4w == FULL_SB_N4_LUMA => {}
            (_, Some(SupportedDirectionalLumaMode::D113)) => {
                return Err(general_intra_unsupported(
                    "general_intra_d113_unverified_position",
                    Some(tile_offset),
                    "general intra directional D113 (pAngle 113) luma IDIF prediction is only verified for a row>0, non-first-column full 64x64 superblock block (haveLeft && haveAbove, real reconstructed §7.13.2.1 above row + left column + diagonally-above-left corner); the top-left no-neighbour, first-row, first-column, sub-partitioned, and non-64x64 D113 positions read the §7.13.2.1 above row / corner that no oracle fixture pins yet, so they are deferred",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // Directional D45 (§7.13.2.8 ZONE-1 one-sided angle, pAngle 45) at a
            // row>0, NON-first-column, NON-rightmost full-superblock block
            // (`frontier.r != 0 && frontier.c != 0`, `haveLeft && haveAbove`, with
            // a real already-decoded above-right superblock in frame). Unlike the
            // §7.13.2.8 "middle" angles (which read `AboveRow[0..w)`), D45 projects
            // UP-AND-RIGHT into the above-right (`base = i + 1 + j`, up to
            // `maxBaseX == w + h - 1`), so it reads `h` real reconstructed
            // above-right samples (the bottom row of the diagonally-above-right
            // superblock) supplied by [`build_one_sided_above_idif_edge`] via
            // §7.13.2.1 `AboveRow[i] = CurrFrame[plane][y-1][Min(aboveLimit, x+i)]`
            // with `aboveLimit` bounded by `num4AboveRight` (§5.20.7.25
            // `count_top_right_avail` over the §5.20.2.3 `BlockDecoded` state). The
            // corner `AboveRow[-1] = CurrFrame[plane][y-1][x-1]` is read directly.
            //
            // D45 is decoded via the §5.20.5.3 `y_mode_offset` escape
            // (`y_mode_offset == 0` -> modeIdx 7 -> modeDelta 8 ->
            // Reordered_Y_Mode[5] == D45_PRED == canonical mode 3, §8.3.2 ctx == 0,
            // AngleDeltaY 0). Every D45 projection has `shift == 0`
            // (`(i+1) * Dr_Intra_Derivative[45] == (i+1) * 64`, `(idx >> 1) & 0x1F
            // == 0`), so the §7.13.2.8 luma IDIF 4-tap reduces to the sample copy
            // `AboveRow[base]` — but it still reads far into the REAL reconstructed
            // above-right, the one-sided zone the middle angles never touch.
            // `enable_ibp == 0` keeps `useIBP == 0` (§7.13.2.7 gates `useIBP` on
            // `pAngle < 90`), and `enable_intra_edge_filter == 0` / `MrlIndex == 0`
            // keep the §7.13.2.x edge-filter / upsample synthesis a no-op.
            //
            // Gated to the row>0 non-first-column NON-rightmost position
            // (`full_sb_num4_above_right(c, n4w, mi_cols, 0) > 0`): the rightmost
            // column has no decoded above-right superblock, so its above-right
            // would clamp (degenerate) and is deferred; the top-left, first-row,
            // first-column, sub-partitioned, and non-64x64 D45 positions are
            // rejected below.
            (_, Some(SupportedDirectionalLumaMode::D45))
                if frontier.r != 0
                    && frontier.c != 0
                    && n4w == FULL_SB_N4_LUMA
                    && full_sb_num4_above_right(frontier.c, n4w, mi_cols, 0) > 0 => {}
            (_, Some(SupportedDirectionalLumaMode::D45)) => {
                return Err(general_intra_unsupported(
                    "general_intra_d45_unverified_position",
                    Some(tile_offset),
                    "general intra directional D45 (pAngle 45, §7.13.2.8 zone-1 one-sided) luma prediction is only verified for a row>0, non-first-column, non-rightmost full 64x64 superblock block (haveLeft && haveAbove, with a real decoded above-right superblock supplying the §7.13.2.1 above-right CurrFrame[plane][y-1][x+i]); the top-left no-neighbour, first-row, first-column, rightmost (no decoded above-right), sub-partitioned, and non-64x64 D45 positions read the §7.13.2.1 above-right that no oracle fixture pins yet, so they are deferred",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // Directional D203 (§7.13.2.8 ZONE-3 one-sided angle, pAngle 203) at a
            // FIRST-superblock-row, NON-first-column full-superblock block
            // (`frontier.r == 0 && frontier.c != 0`, `haveAbove == 0 &&
            // haveLeft == 1`). The symmetric mirror of D45: unlike the §7.13.2.8
            // "middle" angles (which read `LeftCol[0..h)`), D203 projects
            // DOWN-AND-LEFT into the below-left (`idx = (j + 1) * dy`,
            // `base = (idx >> 6) + i`, up to `maxBaseY == w + h - 1`), reading the
            // real reconstructed left column (the right column of the already-decoded
            // left superblock) supplied by [`build_one_sided_left_idif_edge`] via
            // §7.13.2.1 `LeftCol[i] = CurrFrame[plane][Min(leftLimit, y+i)][x-1]`
            // with `leftLimit` bounded by `num4BelowLeft` (§5.20.7.25
            // `count_bottom_left_avail` over the §5.20.2.3 `BlockDecoded` state). In
            // raster order `num4BelowLeft == 0` for this position (no block
            // below-left is decoded yet), so the below-left clamps to the last left
            // sample. At `haveAbove == 0` the corner is `CurrFrame[plane][y][x-1]`.
            //
            // D203 is decoded via the §5.20.5.3 `y_mode_offset` escape
            // (`y_mode_offset == 7` -> modeIdx 7 -> modeDelta 8+... ->
            // Reordered_Y_Mode == D203_PRED == canonical mode 7, §8.3.2 ctx == 0,
            // AngleDeltaY 0). D203's `dy == Dr_Intra_Derivative[270-203] ==
            // Dr_Intra_Derivative[67] == 24` makes most projections land on a
            // nonzero `shift`, so the §7.13.2.8 luma IDIF 4-tap genuinely
            // interpolates over the real reconstructed left column. `enable_ibp == 0`
            // keeps `useIBP == 0` (§7.13.2.7 gates `useIBP` on `pAngle > 180`), and
            // `enable_intra_edge_filter == 0` / `MrlIndex == 0` keep the §7.13.2.x
            // edge-filter / upsample synthesis a no-op.
            //
            // Gated to the first-superblock-row non-first-column position: the
            // top-left no-neighbour, row>0, first-column (no real left column),
            // sub-partitioned, and non-64x64 D203 positions read the §7.13.2.1 left
            // column / below-left / corner that no oracle fixture pins yet, so they
            // are rejected below.
            (_, Some(SupportedDirectionalLumaMode::D203))
                if frontier.r == 0 && frontier.c != 0 && n4w == FULL_SB_N4_LUMA => {}
            (_, Some(SupportedDirectionalLumaMode::D203)) => {
                return Err(general_intra_unsupported(
                    "general_intra_d203_unverified_position",
                    Some(tile_offset),
                    "general intra directional D203 (pAngle 203, §7.13.2.8 zone-3 one-sided) luma prediction is only verified for a first-superblock-row, non-first-column full 64x64 superblock block (haveAbove == 0 && haveLeft == 1, with a real reconstructed left column supplying the §7.13.2.1 left column CurrFrame[plane][Min(leftLimit, y+i)][x-1]); the top-left no-neighbour, first-column (no real left column), row>0, sub-partitioned, and non-64x64 D203 positions read the §7.13.2.1 left column / below-left / corner that no oracle fixture pins yet, so they are deferred",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            // Directional D135: top-left no-neighbour full superblock.
            (_, Some(_)) if is_top_left && n4w == FULL_SB_N4_LUMA => {}
            // Directional D135 at a first-superblock-row (`frontier.r == 0`),
            // non-top-left full-superblock block, reading a **real reconstructed**
            // neighbour edge. The D135 escape was decoded with a non-directional
            // joint-mode neighbour (`ctx == 0`; a directional neighbour would have
            // been rejected earlier as the unmodelled §5.20.5.3 directional-neighbour
            // reorder), so the left neighbour is DC / SMOOTH and supplies the real
            // §7.13.2.1 reconstructed left column. At `frontier.r == 0`,
            // `haveAbove == 0`, so §7.13.2.1 fills `AboveRow` with the repeated first
            // left sample (`CurrFrame[plane][y][x-1]`) and `LeftCol` with the real
            // left column. pAngle 135 has `dx == dy == Dr_Intra_Derivative[45] == 64`,
            // so every projection has `shift == 0`: the §7.13.2.8 IDIF 4-tap
            // (`enableIdif == 1` for luma) reduces to `Dr_Interp_Filter[0] =
            // {0, 128, 0, 0}`, i.e. `Edge[base]`, bit-identical to the bilinear branch
            // **even over the non-flat real left column** (verified bit-exact against
            // avmdec/dav2d). `enable_intra_edge_filter == 0` / `MrlIndex == 0` keep
            // the edge-filter / upsample synthesis a no-op.
            //
            // Gated to `frontier.r == 0`: a row>0 D135 block reads the real above row
            // (the §7.13.2.1 `haveAbove == 1` corner path), which is bit-exact by the
            // same `shift == 0` argument but is not yet covered by an oracle fixture,
            // so it is deferred to a dedicated row>0 D135 grid fixture.
            (_, Some(_)) if frontier.r == 0 && n4w == FULL_SB_N4_LUMA => {}
            // Directional D135 at a row>0, non-first-column full-superblock block
            // (`frontier.r != 0 && frontier.c != 0`, `haveLeft && haveAbove`), reading
            // the **real reconstructed** §7.13.2.1 above row, left column, AND the
            // diagonally-above-left corner `CurrFrame[plane][y-1][x-1]`. §7.13.2.8 D135
            // reads that corner on its main diagonal (`above_base == -1`, `shift == 0`,
            // a sample copy), so the row>0 `haveAbove == 1` path needs the real corner —
            // which [`build_directional_middle_edges`]'s `(true, true)` arm now supplies
            // via `reconstructed_sample`. pAngle 135's `shift == 0` makes the §7.13.2.8
            // luma IDIF 4-tap (`Dr_Interp_Filter[0] = {0,128,0,0}`) a sample copy
            // `Edge[base]`, bit-identical to the bilinear branch over the non-flat real
            // edge (verified bit-exact against avmdec/dav2d). `enable_intra_edge_filter
            // == 0` / `MrlIndex == 0` keep the edge-filter / upsample synthesis a no-op,
            // and D135 never reads the above-right sentinel (`AboveRow[w]`).
            //
            // Gated tighter than the first-row arm: only the `frontier.c != 0`
            // (`haveLeft && haveAbove`) row>0 position is fixtured. The `frontier.c == 0`
            // (`!haveLeft && haveAbove`) first-column row>0 position is rejected below.
            (_, Some(SupportedDirectionalLumaMode::D135))
                if frontier.r != 0 && frontier.c != 0 && n4w == FULL_SB_N4_LUMA => {}
            (_, Some(_)) if !is_top_left && frontier.r != 0 => {
                return Err(general_intra_unsupported(
                    "general_intra_multirow_directional_luma",
                    Some(tile_offset),
                    "general intra directional (D135) luma prediction over a real reconstructed neighbour is verified for the first superblock row (haveAbove == 0, real left column) and for a row > 0 non-first-column full-superblock block (haveLeft && haveAbove, real above row + left column + diagonally-above-left corner); a row > 0 FIRST-column (!haveLeft && haveAbove) or sub-partitioned D135 block, and any row > 0 D157 block, are not yet covered by an oracle fixture, so they are deferred",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            (_, Some(_)) if !is_top_left => {
                return Err(general_intra_unsupported(
                    "general_intra_multiblock_directional_subblock",
                    Some(tile_offset),
                    "general intra multi-block directional (D135) luma prediction over a reconstructed neighbour is only supported for full 64x64 superblock blocks; sub-partitioned directional blocks need the §5.20.2.3 per-block BlockDecoded update for the §7.13.2.1 neighbours and the mode-dependent transform type, which is not yet modelled",
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

    let uv_mode = modes.coeff_uv_mode();
    let luma_log2 = n4w.trailing_zeros() + 2;
    let luma_tx = (luma_log2 - 2) as usize;
    let luma_x = frontier.c * 4;
    let luma_y = frontier.r * 4;
    let luma = crate::tile_payload::decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        0,
        luma_tx,
        luma_x,
        luma_y,
        true,
        false,
        uv_mode,
        false,
        false,
        TransformToolResidualPolicy::Allow,
    )
    .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    match (supported_nondc_luma, supported_directional_luma) {
        (Some(mode), _) if nondc_luma_has_neighbour => {
            // §7.13.2.1 `num4AboveRight` from §5.20.7.25 `count_top_right_avail`
            // over the real §5.20.2.3 `BlockDecoded` state (luma plane, not
            // subsampled). This is the spec-faithful sub-block derivation: a
            // full-superblock block coincides with the superblock so it counts the
            // `clear_block_decoded_flags` above-row marking (identical to
            // `full_sb_num4_above_right`), while a SPLIT child (e.g. the bottom-left
            // 32x32) counts its already-decoded intra-superblock sibling (the
            // top-right 32x32). It only matters for SMOOTH_H / SMOOTH (the top-right
            // sentinel `AboveRow[w]`); SMOOTH_V's output never reads it.
            let num4_above_right = luma_num4_above_right_from_block_decoded(
                block_decoded,
                frontier.r,
                frontier.c,
                n4w,
            );
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
                bit_depth,
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
                bit_depth,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        // Neighbour-having CARDINAL V_PRED / H_PRED luma: a degenerate §7.13.2.8
        // copy of the real reconstructed §7.13.2.1 above row (V, pAngle 90) or left
        // column (H, pAngle 180). No IDIF, no corner, no useIBP, bit-exact over the
        // non-flat real edge.
        (
            None,
            Some(SupportedDirectionalLumaMode::Vertical | SupportedDirectionalLumaMode::Horizontal),
        ) if directional_luma_has_neighbour => {
            let direction = match supported_directional_luma {
                Some(SupportedDirectionalLumaMode::Vertical) => IntraCardinalDirection::Vertical,
                _ => IntraCardinalDirection::Horizontal,
            };
            crate::runtime_minimal_recon::reconstruct_general_intra_cardinal_neighbour_block_into(
                workspace,
                &luma,
                direction,
                PlaneId::Y,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                luma_use_tcq,
                bit_depth,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        // Neighbour-having ZONE-1 one-sided D45 luma over the real §7.13.2.1
        // reconstructed above row + above-right (the §7.13.2.8 step-1 IDIF, which
        // for D45 `shift == 0` is the sample copy `AboveRow[base]` reading far into
        // the real reconstructed above-right). `num4AboveRight` (§5.20.7.25
        // `count_top_right_avail` over the §5.20.2.3 `BlockDecoded` state) bounds
        // the real above-right extent; the luma gate guarantees it is nonzero.
        (None, Some(SupportedDirectionalLumaMode::D45)) if directional_luma_has_neighbour => {
            let num4_above_right =
                full_sb_num4_above_right(frontier.c, n4w, mi_cols, 0);
            crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_neighbour_block_into(
                workspace,
                &luma,
                SupportedDirectionalLumaMode::D45,
                PlaneId::Y,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                num4_above_right,
                luma_use_tcq,
                bit_depth,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        // Neighbour-having ZONE-3 one-sided D203 luma over the real §7.13.2.1
        // reconstructed left column + below-left (the §7.13.2.8 step-3 IDIF). D203's
        // `dy == 24` makes most projections land on a nonzero `shift`, so the luma
        // IDIF 4-tap genuinely interpolates over the real reconstructed left column.
        // `num4BelowLeft` (§5.20.7.25 `count_bottom_left_avail` over the §5.20.2.3
        // `BlockDecoded` state) is `0` in raster order for this position, so the
        // below-left clamps to the last in-block left sample.
        (None, Some(SupportedDirectionalLumaMode::D203)) if directional_luma_has_neighbour => {
            let num4_below_left = full_sb_num4_below_left(frontier.r, n4h, 0);
            crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_left_neighbour_block_into(
                workspace,
                &luma,
                PlaneId::Y,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                num4_below_left,
                luma_use_tcq,
                bit_depth,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        (None, Some(mode)) if directional_luma_has_neighbour => {
            // Neighbour-having D135 luma over the real §7.13.2.1 reconstructed left
            // column (the §7.13.2.8 IDIF, which for D135 `shift == 0` is the same
            // sample copy as bilinear, bit-exact over the non-flat edge).
            crate::runtime_minimal_recon::reconstruct_general_intra_directional_neighbour_block_into(
                workspace,
                &luma,
                mode,
                PlaneId::Y,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                luma_use_tcq,
                bit_depth,
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
                bit_depth,
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
            bit_depth,
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
        // §7.13.2.1 `num4BelowLeft` for the full-superblock chroma block, from
        // §5.20.7.25 `count_bottom_left_avail` over the §5.20.2.3 `BlockDecoded`
        // state. The D203-follow zone-3 chroma reads the chroma below-left; in
        // raster order it is `0` for this first-superblock-row position.
        let num4_below_left = full_sb_num4_below_left(frontier.r, n4h, FRAME_420_SUBSAMPLING_Y);
        let u = crate::tile_payload::decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            1,
            chroma_tx,
            chroma_x,
            chroma_y,
            true,
            false,
            uv_mode,
            false,
            false,
            TransformToolResidualPolicy::Allow,
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
            num4_below_left,
            bit_depth,
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
            true,
            !u.all_zero,
            uv_mode,
            false,
            false,
            TransformToolResidualPolicy::Allow,
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
            num4_below_left,
            bit_depth,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    }
    // AV2 § 5.20.5.3: this block's IntraJointMode (the reorder index modeDelta)
    // is recorded into the IntraJointModes grid by the partition walk so later
    // blocks' § 8.3.2 `y_mode_index` context can read it as a neighbour.
    Ok(crate::tile_payload::GeneralIntraLeafMode::luma(
        modes.intra_joint_mode,
        modes.y_mode,
        modes.fsc_mode,
        modes.uses_mrls,
    ))
}

/// Decodes and reconstructs one **rectangular** general intra leaf block
/// (`n4w != n4h`, e.g. a 64x32 PARTITION_HORZ child or a 32x64 PARTITION_VERT
/// child), gated to the verified DC_PRED luma + DC chroma subset.
///
/// The §7.13.2.4 DC predictor reads only the immediate in-frame left column /
/// above row, so a rectangular DC leaf reconstructs correctly at any superblock
/// position with NO §5.20.2.3 BlockDecoded sentinel state. Under TX_MODE_LARGEST
/// the luma transform is the single rectangular transform spanning the block
/// (`Max_Tx_Size_Rect`, derived here from the §9.2 conversion tables by the
/// block's width/height log2), and the 4:2:0 chroma transform is one log2 smaller
/// in each dimension; both resolve to `DCT_DCT` (§5.20.8.2 `get_tx_set` returns
/// TX_SET_DCTONLY for `txSzSqrUp >= TX_32X32`). The §5.20.7.27 coefficient loop
/// and §7.14.4/§7.15.4 reconstruction already read width and height
/// independently. Any non-DC luma or non-DC chroma mode is rejected (a
/// rectangular §7.13.2.8 / §7.13.2.13 predictor is not yet modelled), keeping the
/// verified subset tight.
#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_rect_block<T: ReconSample>(
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::tile_payload::DecodeBlockFrontier,
    modes: &crate::tile_payload::GeneralIntraBlockModes,
    workspace: &mut CurrentFrameWorkspace<T>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    qindex: u32,
    n4w: usize,
    n4h: usize,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<crate::tile_payload::GeneralIntraLeafMode> {
    // VERIFIED-SUBSET DISCIPLINE: only the oracle-verified 64x32 (`n4w == 16`,
    // `n4h == 8`, PARTITION_HORZ) rectangular geometry is admitted — that is the
    // single geometry the committed fixture (`syn-hrect-intra-64x64-q120`) proves
    // bit-exact against avmdec + dav2d. Every other rectangular size/ratio (32x64,
    // 32x16, 16x8, the §5.20.3 HORZ4/VERT4 4:1 shapes whose *even* log2 ratio takes
    // a different §7.15.4 path than the verified 2:1 √2-rescale case, …) is
    // reconstructed by the same general code but is not yet oracle-fixtured, so it
    // is rejected here rather than emit a confident-but-unverified sample. Later
    // bricks widen this set, each with its own conformance vector.
    if (n4w, n4h) != (16, 8) {
        return Err(general_intra_unsupported(
            "general_intra_rect_unverified_geometry",
            Some(tile_offset),
            "general intra rectangular (non-square) partition leaves are only oracle-verified for the 64x32 PARTITION_HORZ geometry; other rectangular sizes are decodable by the same path but not yet fixtured",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    // Only DC_PRED luma is reconstructed for a rectangular leaf: a non-DC mode
    // (SMOOTH / directional) would need a rectangular §7.13.2.8 / §7.13.2.13
    // predictor that is not yet modelled.
    if !modes.luma_is_dc() {
        return Err(general_intra_unsupported(
            "general_intra_rect_non_dc_luma",
            Some(tile_offset),
            "general intra rectangular (non-square) partition leaves are only reconstructed for DC_PRED luma; non-DC (SMOOTH / directional) rectangular luma prediction is not yet modelled",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // Only DC chroma is reconstructed for a rectangular leaf, for the same reason.
    if modes.supported_chroma_mode() != Some(crate::tile_payload::SupportedChromaMode::Dc) {
        return Err(general_intra_unsupported(
            "general_intra_rect_non_dc_chroma",
            Some(tile_offset),
            "general intra rectangular (non-square) partition leaves are only reconstructed for DC chroma; non-DC rectangular chroma prediction is not yet modelled",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }

    let uv_mode = modes.coeff_uv_mode();
    // §7.15.4 transform dimensions: the block's width / height log2 (4x4 MI units
    // -> log2 pels is `trailing_zeros + 2`). Under TX_MODE_LARGEST the single
    // transform spans the whole block (capped at 64), so its width/height log2
    // equal the block's.
    let luma_w_log2 = n4w.trailing_zeros() + 2;
    let luma_h_log2 = n4h.trailing_zeros() + 2;
    let luma_tx = rect_tx_size_from_log2(luma_w_log2, luma_h_log2).ok_or_else(|| {
        general_intra_unsupported(
            "general_intra_rect_tx_size",
            Some(tile_offset),
            "general intra rectangular leaf could not resolve a §9.2 transform size for its width/height",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        )
    })?;
    let luma_x = frontier.c * 4;
    let luma_y = frontier.r * 4;
    let luma = crate::tile_payload::decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        0,
        luma_tx,
        luma_x,
        luma_y,
        true,
        false,
        uv_mode,
        false,
        false,
        TransformToolResidualPolicy::Allow,
    )
    .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    // §7.14.4 TCQ dqDenom applies to the luma DCT_DCT (TX_CLASS_2D) non-lossless
    // non-FSC block; use the frame-level allowance.
    let luma_use_tcq = work_unit.coeff_frame_facts().allow_tcq();
    crate::runtime_minimal_recon::reconstruct_general_intra_block_rect_into(
        workspace,
        &luma,
        PlaneId::Y,
        luma_x,
        luma_y,
        luma_w_log2,
        luma_h_log2,
        qindex,
        luma_use_tcq,
        bit_depth,
    )
    .map_err(|error| general_intra_residual_error(error, tile_offset))?;

    if frontier.has_chroma {
        // 4:2:0: chroma is half-resolution in each dimension, so the chroma
        // transform/log2 is one smaller per axis and the chroma plane position is
        // the luma position >> 1.
        let chroma_w_log2 = luma_w_log2 - 1;
        let chroma_h_log2 = luma_h_log2 - 1;
        let chroma_tx = rect_tx_size_from_log2(chroma_w_log2, chroma_h_log2).ok_or_else(|| {
            general_intra_unsupported(
                "general_intra_rect_chroma_tx_size",
                Some(tile_offset),
                "general intra rectangular leaf could not resolve a §9.2 chroma transform size",
                GENERAL_INTRA_PARTITION_SPEC_SECTION,
            )
        })?;
        let chroma_x = frontier.c * 2;
        let chroma_y = frontier.r * 2;
        let u = crate::tile_payload::decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            1,
            chroma_tx,
            chroma_x,
            chroma_y,
            true,
            false,
            uv_mode,
            false,
            false,
            TransformToolResidualPolicy::Allow,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        crate::runtime_minimal_recon::reconstruct_general_intra_block_rect_into(
            workspace,
            &u,
            PlaneId::U,
            chroma_x,
            chroma_y,
            chroma_w_log2,
            chroma_h_log2,
            qindex,
            false,
            bit_depth,
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
            true,
            !u.all_zero,
            uv_mode,
            false,
            false,
            TransformToolResidualPolicy::Allow,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        crate::runtime_minimal_recon::reconstruct_general_intra_block_rect_into(
            workspace,
            &v,
            PlaneId::V,
            chroma_x,
            chroma_y,
            chroma_w_log2,
            chroma_h_log2,
            qindex,
            false,
            bit_depth,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    }
    Ok(crate::tile_payload::GeneralIntraLeafMode::luma(
        modes.intra_joint_mode,
        modes.y_mode,
        modes.fsc_mode,
        modes.uses_mrls,
    ))
}

/// Resolves the AV2 § 9.2 `TX_SIZES_ALL` index whose `Tx_Width_Log2` /
/// `Tx_Height_Log2` match `(w_log2, h_log2)`, scanning the generated conversion
/// tables (no invented constant). Used to map a rectangular block's transform
/// dimensions to its `txSz` for the §5.20.7.27 coefficient loop and §7.15.4
/// reconstruction. Returns `None` when no §9.2 transform has those dimensions.
fn rect_tx_size_from_log2(w_log2: u32, h_log2: u32) -> Option<usize> {
    let w = i32::try_from(w_log2).ok()?;
    let h = i32::try_from(h_log2).ok()?;
    TX_WIDTH_LOG2
        .iter()
        .zip(TX_HEIGHT_LOG2.iter())
        .position(|(&tw, &th)| tw == w && th == h)
}

/// 4:2:0 chroma horizontal subsampling (`SubsamplingX == 1`).
const FRAME_420_SUBSAMPLING_X: usize = 1;

/// 4:2:0 chroma vertical subsampling (`SubsamplingY == 1`).
const FRAME_420_SUBSAMPLING_Y: usize = 1;

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

/// Derives AV2 § 7.13.2.1 `num4BelowLeft` (in plane 4x4 units) for a
/// full-superblock transform block, faithfully to § 5.20.7.25
/// `count_bottom_left_avail` over the § 5.20.2.3 `BlockDecoded` state. The plane
/// is selected by `sub_y` (`0` for luma, `1` for 4:2:0 chroma).
///
/// `count_bottom_left_avail(plane, x4, y4, h4)` scans
/// `BlockDecoded[plane][y4 + h4 + i][x4 - 1]` for `i in 0..h4`. For a full 64x64
/// superblock the block coincides with the superblock, so its sub-block MI
/// position within the superblock is `(0, 0)`: it scans
/// `BlockDecoded[plane][h4 + i][-1]`, the column to the left of the superblock at
/// rows BELOW the superblock. In raster decode order those below-left rows belong
/// to superblocks that have not been decoded yet (a first-superblock-row block has
/// no decoded superblock below it), and `clear_block_decoded_flags` (§ 5.20.2.3)
/// does not mark the below-left rows decoded, so the count is `0`. This matches
/// the spec loop's first-iteration `break`.
fn full_sb_num4_below_left(_r: usize, _n4h: usize, _sub_y: usize) -> usize {
    // For the verified first-superblock-row, non-first-column position the
    // below-left rows are not yet decoded in raster order (no decoded superblock
    // below this one), so §5.20.7.25 `count_bottom_left_avail` returns 0 and the
    // §7.13.2.1 below-left clamps to the last in-block left sample.
    0
}

/// Derives AV2 § 7.13.2.1 `num4AboveRight` (in luma 4x4 units) for an arbitrary
/// (full-superblock or sub-partitioned) luma transform block, faithfully to
/// § 5.20.7.25 `count_top_right_avail` over the real § 5.20.2.3 `BlockDecoded`
/// state. Unlike [`full_sb_num4_above_right`] (which special-cases the
/// full-superblock `(0, 0)` sub-block whose above-right is read directly from the
/// `clear_block_decoded_flags` above-row marking), this reads the genuine
/// per-block decoded grid, so a SPLIT child's above-right sibling (e.g. the
/// bottom-left 32x32 reading the already-decoded top-right 32x32) is counted.
///
/// `r` / `c` are the block's luma MI position; `n4w` is its width in luma 4x4
/// units. The superblock-relative sub-block position is `(r & sbMask, c & sbMask)`
/// (`sbMask = sbSize4 - 1`); luma is not subsampled, so `x4 = subBlockMiCol`,
/// `y4 = subBlockMiRow`, `w4 = n4w`.
fn luma_num4_above_right_from_block_decoded(
    block_decoded: &crate::tile_payload::TileBlockDecodedState,
    r: usize,
    c: usize,
    n4w: usize,
) -> usize {
    let sb_mask = block_decoded.sb_size4().saturating_sub(1);
    let x4 = c & sb_mask;
    let y4 = r & sb_mask;
    // Luma plane (0); not subsampled.
    block_decoded.count_top_right_avail(0, x4, y4, n4w)
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
        | GeneralIntraResidualError::NonZeroPass { .. }
        | GeneralIntraResidualError::NonZeroStart { .. }
        | GeneralIntraResidualError::StagedNonZeroPass { .. }
        | GeneralIntraResidualError::StagedFscPass { .. }
        | GeneralIntraResidualError::TransformTypeRead { .. } => general_intra_unsupported(
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
        GeneralIntraResidualError::UnsupportedTransformToolResidual { .. } => {
            general_intra_unsupported(
                "general_intra_transform_tool_residual",
                Some(offset),
                "general intra residual decode consumed the all_zero decision, but a nonzero residual would require transform-tool syntax outside the supported subset",
                GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
            )
        }
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
        GeneralIntraResidualError::UnsupportedDirectionalAboveEdge => general_intra_unsupported(
            "general_intra_directional_above_edge",
            Some(offset),
            "general intra directional prediction over a real reconstructed above-neighbour edge needs the §7.13.2.1 corner sample CurrFrame[plane][y-1][x-1] (D135 reads the corner on its main diagonal), which is not yet modelled",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::MissingCardinalEdge => general_intra_unsupported(
            "general_intra_cardinal_missing_edge",
            Some(offset),
            "general intra cardinal (V_PRED / H_PRED) prediction is missing its required reconstructed neighbour edge (V_PRED needs the §7.13.2.1 above row, H_PRED needs the left column)",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::CardinalModeInMiddleAnglePath => general_intra_unsupported(
            "general_intra_cardinal_in_middle_angle_path",
            Some(offset),
            "general intra cardinal (V_PRED / H_PRED) mode reached the §7.13.2.8 middle-angle path (which only covers D135); cardinal modes must be dispatched to the cardinal copy reconstruction",
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
            "general intra decode reached a luma intra mode outside the currently supported reconstruction subset",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::InvalidUvMode { .. } => general_intra_unsupported(
            "general_intra_invalid_uv_mode",
            Some(offset),
            "general intra decode rejected an out-of-range chroma uv_mode index",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::InvalidFscBlockSizeIndex { .. } => general_intra_unsupported(
            "general_intra_invalid_fsc_block_size_index",
            Some(offset),
            "general intra decode could not map MiSize through Fsc_Bsize_Groups",
            "8.3.2",
        ),
        GeneralIntraBlockModeError::InvalidCflMhDirBlockSizeIndex { .. } => {
            general_intra_unsupported(
                "general_intra_invalid_cfl_mh_dir_size_group",
                Some(offset),
                "general intra decode could not map MiSize through Size_Group for cfl_mh_dir",
                "8.3.2",
            )
        }
        GeneralIntraBlockModeError::UnsupportedMhccpMode => general_intra_unsupported(
            "general_intra_unsupported_mhccp_mode",
            Some(offset),
            "general intra decode can skip a false MHCCP-enabled is_cfl decision but does not support active MHCCP chroma prediction",
            "5.20.5.6",
        ),
        GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder { .. } => {
            general_intra_unsupported(
                "general_intra_directional_neighbour_reorder",
                Some(offset),
                "general intra luma mode syntax over a directional joint-mode neighbour needs the §5.20.5.5 directional-neighbour mode reorder",
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
        | MinimalRuntimePartitionFrontierError::UsesMrlsState(_)
        | MinimalRuntimePartitionFrontierError::FscModeState(_)
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
