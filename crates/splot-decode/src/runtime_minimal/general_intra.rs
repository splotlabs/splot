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
        && core.quantization_params.is_some_and(|quant| {
            quant.delta_q_y_dc == 0
                && quant.delta_q_u_dc == 0
                && quant.delta_q_u_ac == 0
                && quant.delta_q_v_dc == 0
                && quant.delta_q_v_ac == 0
        })
        && sequence.intra.as_ref().is_some_and(|intra| {
            !intra.enable_dip
                && !intra.enable_ibp
                && !intra.enable_mrls
                && !intra.enable_intra_edge_filter
        })
        && sequence
            .partition
            .is_some_and(|partition| !partition.enable_sdp)
        && sequence.transform_quant_entropy.is_some_and(|tq| {
            tq.equal_ac_dc_q
                && !tq.enable_fsc
                && !tq.enable_cctx
                && !tq.enable_idtx_intra
                && !tq.enable_intra_ist
                && i32::from(tq.base_uv_dc_delta_q) + GENERAL_INTRA_DELTA_DCQUANT_MIN == 0
                && i32::from(tq.base_uv_ac_delta_q) + GENERAL_INTRA_DELTA_DCQUANT_MIN == 0
        })
        && core
            .intra_tail
            .is_some_and(|tail| tail.tx_mode == TxMode::Largest)
        && core.deblocking_filter_params.is_some_and(|filter| {
            filter.apply_deblocking_filter == [false; 4]
                || matches!(sequence.general.bit_depth_idc, BitDepthIdc::Eight)
        })
        && core.cdef_params.as_ref().is_some_and(|cdef| {
            !cdef.cdef_frame_enable || matches!(sequence.general.bit_depth_idc, BitDepthIdc::Eight)
        })
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
            .is_some_and(|filter| filter.df_delta_q == [0; 4])
        && core.gdf_params.is_some_and(|gdf| !gdf.gdf_frame_enable)
        && core.cdef_params.as_ref().is_some_and(|cdef| {
            !cdef.cdef_frame_enable
                || (cdef.cdef_strengths == Some(1)
                    && cdef.cdef_on_skip_txfm_frame_enable == Some(true)
                    && cdef.cdef_damping.is_some()
                    && !cdef.strengths.is_empty())
        })
        && core.lr_params.as_ref().is_some_and(|lr| !lr.uses_lr)
        && core
            .ccso_params
            .as_ref()
            .is_some_and(|ccso| ccso.ccso_frame_flag.is_none() && ccso.planes.is_empty())
        && core
            .intra_tail
            .is_some_and(|tail| !tail.film_grain.apply_grain)
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

    let tile_size = tile.tile_size();
    let limits = options.limits();

    let bit_depth = match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight => BitDepth::Eight,
        BitDepthIdc::Ten => BitDepth::Ten,
    };

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

    let mut deblock_blocks: Vec<super::deblock::DeblockBlock> = Vec::new();

    let symbols = crate::tile_payload::decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit,
         symbols,
         frontier,
         joint_modes,
         uses_mrls,
         fsc_modes,
         _is_cfl_ctx,
         block_decoded| {
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
                &mut deblock_blocks,
                qindex,
                luma_use_tcq,
                mi_cols,
                mi_rows,
                bit_depth,
                tile_offset,
            )
        },
    )
    .map_err(|error| map_general_intra_multiblock_error(error, tile_offset))?;

    symbols.exit_symbol().map_err(|_| {
        general_intra_unsupported(
            "general_intra_exit_symbol",
            Some(tile_offset),
            "general intra tile payload did not satisfy §8.2.4 exit_symbol() after the decoded blocks",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        )
    })?;

    let apply = core
        .deblocking_filter_params
        .map_or([false; 4], |filter| filter.apply_deblocking_filter);
    super::deblock::deblock_general_intra_frame(
        &mut workspace,
        &deblock_blocks,
        mi_rows,
        mi_cols,
        apply,
        qindex,
        bit_depth,
    )
    .map_err(|error| general_intra_deblock_error(error, tile_offset))?;

    if let Some(params) = cdef_frame_params(core) {
        super::cdef::cdef_general_intra_frame(&mut workspace, params, mi_rows, mi_cols, bit_depth)
            .map_err(|error| general_intra_cdef_error(error, tile_offset))?;
    }

    Ok(workspace.freeze()?)
}

/// Builds the §7.18.1 single-strength-set CDEF parameters from the parsed frame
/// header, or `None` when CDEF is frame-disabled (the pass is then skipped). The
/// route gate guarantees `CdefStrengths == 1` and a present damping / strength set
/// whenever `cdef_frame_enable` is true, so this maps the verified subset only.
fn cdef_frame_params(core: &FrameHeaderCore) -> Option<super::cdef::CdefFrameParams> {
    let cdef = core.cdef_params.as_ref()?;
    if !cdef.cdef_frame_enable {
        return None;
    }
    let damping = i32::from(cdef.cdef_damping?);
    let set = cdef.strengths.first()?;
    Some(super::cdef::CdefFrameParams {
        y_pri: i32::from(set.y_pri_strength),
        y_sec: i32::from(set.y_sec_strength),
        uv_pri: i32::from(set.uv_pri_strength),
        uv_sec: i32::from(set.uv_sec_strength),
        damping,
    })
}

/// Maps a §7.18 CDEF-orchestration error to a decode diagnostic. The per-block
/// primitives are total for valid inputs, so any error here signals an internal
/// inconsistency (a geometry or workspace access out of bounds) and surfaces as an
/// `unsupported-feature` diagnostic rather than a silent wrong-pixel output.
fn general_intra_cdef_error(_error: super::cdef::CdefError, offset: ByteOffset) -> DecodeError {
    general_intra_unsupported(
        "general_intra_cdef",
        Some(offset),
        "general intra §7.18 CDEF orchestration reached an unsupported or inconsistent block configuration",
        "7.18",
    )
}

/// Maps a §7.17 deblocking-orchestration error to a decode diagnostic. The
/// per-edge primitives are total for valid inputs, so any error here signals an
/// internal inconsistency (an uncovered MI, a missing transform size, or a
/// workspace access out of bounds) and surfaces as an `unsupported-feature`
/// diagnostic rather than a silent wrong-pixel output.
fn general_intra_deblock_error(
    _error: super::deblock::DeblockError,
    offset: ByteOffset,
) -> DecodeError {
    general_intra_unsupported(
        "general_intra_deblock",
        Some(offset),
        "general intra §7.17 deblocking-filter orchestration reached an unsupported or inconsistent edge configuration",
        "7.17",
    )
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
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    qindex: u32,
    luma_use_tcq: bool,
    mi_cols: usize,
    mi_rows: usize,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<crate::tile_payload::GeneralIntraLeafMode> {
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

    let modes = crate::tile_payload::decode_general_intra_block_modes(
        work_unit,
        symbols,
        crate::tile_payload::GeneralIntraChromaToolConfig::disabled(),
        joint_modes,
        uses_mrls,
        fsc_modes,
        0,
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

    let single_sb_frame = mi_cols == FULL_SB_N4_LUMA && mi_rows == FULL_SB_N4_LUMA;
    let chroma_admitted_10bit = match modes.supported_chroma_mode() {
        Some(crate::tile_payload::SupportedChromaMode::Dc) => true,
        Some(crate::tile_payload::SupportedChromaMode::Smooth) => {
            single_sb_frame && frontier.r == 0 && frontier.c == 0
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

    if bit_depth != BitDepth::Eight && (n4w != FULL_SB_N4_LUMA || n4h != FULL_SB_N4_LUMA) {
        return Err(general_intra_unsupported(
            "unsupported_10bit_non_64x64_leaf",
            Some(tile_offset),
            "general intra 10-bit reconstruction is only oracle-verified for full 64x64 square DC leaves; a 10-bit non-64x64 partition leaf (rectangular, or a split 32x32 / 16x16 square sub-block) is deferred until a 10-bit oracle fixture pins it",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }

    if n4w != n4h {
        return decode_one_general_intra_rect_block::<T>(
            work_unit,
            symbols,
            frontier,
            &modes,
            workspace,
            coeff_ctx,
            deblock_blocks,
            qindex,
            n4w,
            n4h,
            bit_depth,
            tile_offset,
        );
    }

    let Some(supported_chroma) = modes.supported_chroma_mode() else {
        return Err(general_intra_unsupported(
            "general_intra_non_dc_chroma_mode",
            Some(tile_offset),
            "general intra reconstruction only supports DC, SMOOTH, the cardinal V/H directional-follow, and the D135 / D157 directional-follow chroma prediction; other non-DC chroma (uv_mode) modes are not yet implemented",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    };
    let chroma_is_top_left = frontier.r == 0 && frontier.c == 0;
    #[allow(clippy::items_after_statements)]
    const FULL_SB_N4_CHROMA_GATE: usize = 16;
    let chroma_first_row_neighbour_ok = frontier.r == 0 && n4w == FULL_SB_N4_CHROMA_GATE;
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
    #[allow(clippy::items_after_statements)]
    const FULL_SB_N4: usize = 16;
    if supported_chroma == crate::tile_payload::SupportedChromaMode::Smooth && n4w != FULL_SB_N4 {
        return Err(general_intra_unsupported(
            "general_intra_smooth_chroma_subblock",
            Some(tile_offset),
            "general intra SMOOTH chroma is only supported for full 64x64 superblock blocks; sub-partitioned SMOOTH chroma needs the §7.13.2.1 above-right / below-left sentinel neighbours from the per-block §5.20.2.3 BlockDecoded update, which is not yet modelled",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    #[allow(clippy::items_after_statements)]
    const NON_DC_MIN_N4: usize = 8;
    #[allow(clippy::items_after_statements)]
    const FULL_SB_N4_LUMA: usize = 16;
    let supported_nondc_luma = modes.supported_nondc_luma();
    let supported_directional_luma = modes.supported_directional_luma();
    let is_top_left = frontier.r == 0 && frontier.c == 0;
    let nondc_luma_has_neighbour = supported_nondc_luma.is_some() && !is_top_left;
    let directional_luma_has_neighbour = supported_directional_luma.is_some() && !is_top_left;
    if !modes.luma_is_dc() {
        #[allow(clippy::match_same_arms)]
        match (supported_nondc_luma, supported_directional_luma) {
            (Some(SupportedNonDcLumaMode::Smooth), _) if is_top_left && n4w == FULL_SB_N4_LUMA => {}
            (
                Some(
                    SupportedNonDcLumaMode::SmoothVertical
                    | SupportedNonDcLumaMode::SmoothHorizontal,
                ),
                _,
            ) if is_top_left && n4w >= NON_DC_MIN_N4 => {}
            (Some(SupportedNonDcLumaMode::SmoothVertical), _) if n4w == FULL_SB_N4_LUMA => {}
            (Some(SupportedNonDcLumaMode::SmoothHorizontal), _) if n4w == FULL_SB_N4_LUMA => {}
            (Some(_), _) if is_top_left => {
                return Err(general_intra_unsupported(
                    "general_intra_non_dc_non_dctonly_size",
                    Some(tile_offset),
                    "general intra non-DC luma prediction is only supported for 32x32-or-larger (TX_SET_DCTONLY) blocks; smaller non-DC blocks can signal a mode-dependent transform type that is not yet decoded",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
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
            (_, Some(_)) if is_top_left && n4w == FULL_SB_N4_LUMA => {}
            (_, Some(_)) if frontier.r == 0 && n4w == FULL_SB_N4_LUMA => {}
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
            .map_err(|error| general_intra_residual_error(error, tile_offset))?;
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
            .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        }
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
                luma_log2,
                qindex,
                luma_use_tcq,
                bit_depth,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        }
        (None, Some(SupportedDirectionalLumaMode::D45)) if directional_luma_has_neighbour => {
            let num4_above_right = full_sb_num4_above_right(frontier.c, n4w, mi_cols, 0);
            crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_neighbour_block_into(
                workspace,
                &luma,
                45,
                PlaneId::Y,
                luma_x,
                luma_y,
                luma_log2,
                luma_log2,
                qindex,
                num4_above_right,
                luma_use_tcq,
                bit_depth,
                crate::runtime_minimal_recon::OneSidedEdgeFilter::default(),
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        }
        (None, Some(SupportedDirectionalLumaMode::D203)) if directional_luma_has_neighbour => {
            let num4_below_left = full_sb_num4_below_left(frontier.r, n4h, 0);
            crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_left_neighbour_block_into(
                workspace,
                &luma,
                203,
                PlaneId::Y,
                luma_x,
                luma_y,
                luma_log2,
                luma_log2,
                qindex,
                num4_below_left,
                false, // have_above: first-SB-row no-above leaf, corner `CurrFrame[y][x-1]`
                luma_use_tcq,
                bit_depth,
                crate::runtime_minimal_recon::OneSidedEdgeFilter::default(),
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        }
        (None, Some(mode)) if directional_luma_has_neighbour => {
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
            .map_err(|error| general_intra_residual_error(error, tile_offset))?;
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
            .map_err(|error| general_intra_residual_error(error, tile_offset))?;
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
        let chroma_log2 = luma_log2 - 1;
        let chroma_tx = (chroma_log2 - 2) as usize;
        let chroma_x = frontier.c * 2;
        let chroma_y = frontier.r * 2;
        let num4_above_right =
            full_sb_num4_above_right(frontier.c, n4w, mi_cols, FRAME_420_SUBSAMPLING_X);
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
    let chroma_tx = if frontier.has_chroma {
        Some((luma_log2 - 1 - 2) as usize)
    } else {
        None
    };
    deblock_blocks.push(super::deblock::DeblockBlock {
        r: frontier.r,
        c: frontier.c,
        n4w,
        n4h,
        luma_tx,
        chroma_tx,
    });

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
    deblock_blocks: &mut Vec<super::deblock::DeblockBlock>,
    qindex: u32,
    n4w: usize,
    n4h: usize,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<crate::tile_payload::GeneralIntraLeafMode> {
    if (n4w, n4h) != (16, 8) {
        return Err(general_intra_unsupported(
            "general_intra_rect_unverified_geometry",
            Some(tile_offset),
            "general intra rectangular (non-square) partition leaves are only oracle-verified for the 64x32 PARTITION_HORZ geometry; other rectangular sizes are decodable by the same path but not yet fixtured",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    if !modes.luma_is_dc() {
        return Err(general_intra_unsupported(
            "general_intra_rect_non_dc_luma",
            Some(tile_offset),
            "general intra rectangular (non-square) partition leaves are only reconstructed for DC_PRED luma; non-DC (SMOOTH / directional) rectangular luma prediction is not yet modelled",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    if modes.supported_chroma_mode() != Some(crate::tile_payload::SupportedChromaMode::Dc) {
        return Err(general_intra_unsupported(
            "general_intra_rect_non_dc_chroma",
            Some(tile_offset),
            "general intra rectangular (non-square) partition leaves are only reconstructed for DC chroma; non-DC rectangular chroma prediction is not yet modelled",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }

    let uv_mode = modes.coeff_uv_mode();
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
        false,
        bit_depth,
    )
    .map_err(|error| general_intra_residual_error(error, tile_offset))?;

    if frontier.has_chroma {
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
            false,
            bit_depth,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    }
    let chroma_tx = if frontier.has_chroma {
        rect_tx_size_from_log2(luma_w_log2 - 1, luma_h_log2 - 1)
    } else {
        None
    };
    deblock_blocks.push(super::deblock::DeblockBlock {
        r: frontier.r,
        c: frontier.c,
        n4w,
        n4h,
        luma_tx,
        chroma_tx,
    });
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

#[allow(clippy::needless_pass_by_value)]
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

#[allow(clippy::needless_pass_by_value)]
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

#[allow(clippy::needless_pass_by_value)]
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
        | MinimalRuntimePartitionFrontierError::UvCflState(_)
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
