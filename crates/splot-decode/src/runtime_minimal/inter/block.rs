// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal inter `mode_info` decode (AV2 § 5.20.7.6) + § 7.11/§ 7.12 MV
//! derivation.
//!
//! Walks the § 5.20.3 partition tree over every leaf inter block, reads the
//! verified inter `mode_info` symbol sequence (`is_inter` / `skip` / `single_mode`
//! / DRL / `read_mv` / `interp_filter`) from the tile arithmetic stream per block,
//! and confirms § 8.2.4 `exit_symbol()`. The § 8.3.2 `single_mode` / DRL contexts
//! are derived from the spatial neighbours via the § 7.11.2 find-mode-context
//! kernel, and the MV is predicted from the spatial-neighbour MV stack via the
//! § 7.12.2 find-mv-stack kernel: a later block's NEARMV / NEARESTMV mode
//! reconstructs a neighbour block's MV from the stack, while NEWMV reads the
//! § 5.20.7.20 SHELL-coded delta over the stack-selected predictor.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::frame::InterpolationFilter as FrameInterpolationFilter;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_recon::InterpolationFilter as ReconInterpolationFilter;

use super::super::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};
use super::compound::{CompoundParseInput, read_compound_average_syntax};
use super::find_mv_stack::{
    MvBlockContext, NeighbourMvGrid, NeighbourYMode, block_neighbour_ctx, find_mode_ctx,
    find_mv_stack,
};
use super::read_mv::{mv_clamp_to_integer, read_newmv_block_mvd};
use super::{
    InterBlock, InterResidual, Mv, PlacedInterBlock, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV,
    SINGLE_MODE_NEWMV, SPEC_MODE_INFO, effective_quantizer_deltas_are_zero, unsupported_at,
    unsupported_compound_at,
};
use crate::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, GeneralIntraMultiblockError,
    GeneralIntraTreeWalkError, TileCdfSelector, TileCdfSubset, TileCoeffContextState,
    TilePartitionTraversalError, TransformToolResidualPolicy, decode_general_intra_multiblock_tree,
    decode_general_intra_plane_coeffs, frame_mi_dimensions,
};

/// AV2 § 8.3.2 no-neighbour `interp_filter` context base: `leftType` and
/// `aboveType` both default to 3 before any `RefFrame[1]` compound-reference offset.
const INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE: usize = 3;

/// AV2 § 8.3.2 `interp_filter` context offset for compound prediction:
/// `is_inter_ref_frame(RefFrame[1]) * 4`.
const INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET: usize = 4;

/// `RefFrame[0]` for the verified single-reference subset: `read_ref_frames`
/// returns `RefFrame[0] == 0` (LAST_FRAME) when `NumTotalRefs == 1`.
const SINGLE_REF_FRAME0: i8 = 0;

/// Decodes every § 5.20.3 leaf inter block's § 5.20.7.6 `mode_info` and returns
/// each placed block (luma-space rect + § 7.11/§ 7.12 motion vector + § 5.20.7.6
/// interpolation filter + optional residual), in decode (DFS) order. Runs the
/// real § 5.20.3 partition walk + § 8.2 symbol reads over the tile payload,
/// threading the spatial-neighbour MV grid through `find_mode_ctx` / `find_mv_stack`
/// so a later block predicts an earlier block's MV, and validates § 8.2.4
/// `exit_symbol()`.
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_inter_blocks(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: DecodeOptions,
    frame_interpolation_filter: FrameInterpolationFilter,
    num_total_refs: usize,
    reference_select: bool,
    compound_is_joint_ctx: Option<usize>,
    num_same_ref_compound: u8,
) -> Result<Vec<PlacedInterBlock>> {
    let offset = frame_envelope.offset;
    let mut tile_plan = super::super::derive_inter_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        core,
        options,
    )?;
    let [tile] = tile_plan.work_units_mut() else {
        return Err(unsupported_at(
            "inter_unexpected_tile_work_units",
            offset,
            "minimal inter decode requires exactly one tile work unit",
            SPEC_MODE_INFO,
        ));
    };
    let tile_offset = tile.tile_byte_span().start;

    let max_drl_bits_minus_1 = core
        .inter
        .as_ref()
        .and_then(|inter| inter.max_drl_bits_minus_1)
        .ok_or_else(|| {
            unsupported_at(
                "inter_missing_max_drl_bits",
                offset,
                "minimal inter decode requires the parsed max_drl_bits_minus_1",
                SPEC_MODE_INFO,
            )
        })?;

    let (mi_rows, mi_cols) = frame_mi_dimensions(core).map_err(|_| {
        unsupported_at(
            "inter_mi_dimensions",
            offset,
            "minimal inter decode requires the frame MI dimensions for the residual context",
            SPEC_MODE_INFO,
        )
    })?;
    let mut coeff_ctx = TileCoeffContextState::new(mi_rows, mi_cols).map_err(|_| {
        unsupported_at(
            "inter_coeff_context_state",
            offset,
            "minimal inter decode could not allocate the §5.20.7.27 residual context",
            SPEC_MODE_INFO,
        )
    })?;

    let mut mv_grid = NeighbourMvGrid::new(mi_rows, mi_cols).ok_or_else(|| {
        unsupported_at(
            "inter_mv_grid",
            offset,
            "minimal inter decode could not allocate the §7.11/§7.12 neighbour MV grid",
            SPEC_MODE_INFO,
        )
    })?;
    let sb_h4 = superblock_h4(sequence, core).ok_or_else(|| {
        unsupported_at(
            "inter_sb_size",
            offset,
            "minimal inter decode requires the parsed superblock size",
            SPEC_MODE_INFO,
        )
    })?;

    let residual_tools_present = sequence.transform_quant_entropy.is_none_or(|tq| {
        tq.enable_inter_ist
            || tq.enable_inter_ddt
            || tq.enable_cctx
            || tq.enable_fsc
            || tq.enable_idtx_intra
    });
    let residual_quantizer_deltas_are_zero = core
        .quantization_params
        .as_ref()
        .is_some_and(|quant| effective_quantizer_deltas_are_zero(sequence, quant));

    let mv_stack_tools_present = sequence.inter.as_ref().is_none_or(|seq_inter| {
        seq_inter.enable_ref_frame_mvs
            || seq_inter.enable_refmvbank
            || seq_inter.drl_reorder != splot_core::headers::sequence::DrlReorder::Disabled
    });

    let mut decoded_blocks: Vec<PlacedInterBlock> = Vec::new();
    let limits = options.limits();
    let symbols = decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit,
         symbols,
         frontier,
         _joint_modes,
         _uses_mrls,
         _fsc_modes,
         _is_cfl_ctx,
         _block_decoded| {
            let placed = decode_one_inter_block(
                work_unit,
                symbols,
                frontier,
                &mut coeff_ctx,
                &mut mv_grid,
                sb_h4,
                mi_rows,
                mi_cols,
                max_drl_bits_minus_1,
                frame_interpolation_filter,
                residual_tools_present,
                residual_quantizer_deltas_are_zero,
                mv_stack_tools_present,
                num_total_refs,
                reference_select,
                compound_is_joint_ctx,
                num_same_ref_compound,
                tile_offset,
            )?;
            decoded_blocks.push(placed);
            Ok(crate::tile_payload::GeneralIntraLeafMode::no_luma_mode())
        },
    )
    .map_err(|error| map_inter_multiblock_error(error, tile_offset))?;

    symbols.exit_symbol().map_err(|_| {
        if reference_select {
            unsupported_compound_at(
                "compound_exit_symbol",
                tile_offset,
                "minimal compound-average tile payload did not satisfy §8.2.4 exit_symbol() after the decoded compound inter block",
                SPEC_MODE_INFO,
            )
        } else {
            unsupported_at(
                "inter_exit_symbol",
                tile_offset,
                "minimal inter tile payload did not satisfy §8.2.4 exit_symbol() after the decoded inter block",
                SPEC_MODE_INFO,
            )
        }
    })?;

    if decoded_blocks.is_empty() {
        return Err(unsupported_at(
            "inter_no_decoded_block",
            tile_offset,
            "minimal inter decode expected at least one decoded inter block",
            SPEC_MODE_INFO,
        ));
    }
    Ok(decoded_blocks)
}

/// AV2 § 5.20.2.1 superblock height in 4x4 MI units (`Num_4x4_Blocks_High[SbSize]`)
/// for the `isSbBorder` derivation. The verified subset is sb_size 64
/// (`sb_h4 == 16`); a sb_size-128 frame is rejected by the inter frame-header gate
/// (the supported case is a single 64x64 superblock per tile).
fn superblock_h4(sequence: &SequenceHeader, core: &FrameHeaderCore) -> Option<usize> {
    let partition = sequence.partition?;
    let frame_is_intra = core.frame_is_intra?;
    let _ = frame_is_intra;
    match partition.seq_sb_size() {
        splot_core::headers::sequence::SuperblockSize::Block64x64 => Some(16),
        splot_core::headers::sequence::SuperblockSize::Block128x128 => Some(32),
        splot_core::headers::sequence::SuperblockSize::Block256x256 => Some(64),
    }
}

/// Decodes one inter leaf block's § 5.20.7.6 `mode_info` for the verified subset and
/// returns its § 7.11 motion vector + § 5.20.7.6 interpolation filter. Reads, in
/// § 5.20 order:
/// 1. `is_inter` (§ 5.20.7.3) — `TileIsInterCdf[ctx]`, must decode to 1.
/// 2. `skip` (§ 5.20.5.10) — `TileSkipCdf[ctx]`, must decode to 1 (no residual).
/// 3. `single_mode` (§ 5.20.7.6) — `TileSingleModeCdf[NewMvContext]`; NEARMV (0) /
///    GLOBALMV (1) are the zero-MV modes, NEWMV (2) reads a SHELL MV delta.
/// 4. DRL (§ 5.20.7.8) — `read_drl_idx(0, m)` for NEARMV / NEWMV (`has_nearmv` /
///    `has_newmv`); GLOBALMV reads none.
/// 5. `read_mv` (§ 5.20.7.20) — the SHELL-coded MV delta, NEWMV only.
/// 6. `interp_filter` (§ 5.20.7.6) — `TileInterpFilterCdf[ctx]`, read only when the
///    frame filter is SWITCHABLE and `needs_interp_filter()` is 1 (NEARMV / NEWMV;
///    GLOBALMV at >= 8x8 returns 0).
///
/// `read_skip_mode` reads no symbol (`skip_mode_present == 0` for this fixture),
/// `read_ref_frames` reads no `single_ref` symbol (`NumTotalRefs == 1`),
/// `read_motion_mode` reads no symbol (no enabled motion modes -> SIMPLE),
/// `read_refinemv` / `read_compound_type` read no symbol (single reference,
/// `inter_intra == 0`), and the MvPrecision derivation reads no symbol
/// (`enable_flex_mvres == 0` -> `MvPrecision == FrameMvPrecision`). Any deviation
/// from this exact subset desynchronises the § 8.2 arithmetic decoder and fails the
/// caller's `exit_symbol()` check.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn decode_one_inter_block(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    coeff_ctx: &mut TileCoeffContextState,
    mv_grid: &mut NeighbourMvGrid,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tools_present: bool,
    residual_quantizer_deltas_are_zero: bool,
    mv_stack_tools_present: bool,
    num_total_refs: usize,
    reference_select: bool,
    compound_is_joint_ctx: Option<usize>,
    num_same_ref_compound: u8,
    tile_offset: ByteOffset,
) -> Result<PlacedInterBlock> {
    let n4w = frontier.b_size.num_4x4_wide().map_err(|_| {
        unsupported_at(
            "inter_block_geometry",
            tile_offset,
            "minimal inter block geometry lookup failed",
            SPEC_MODE_INFO,
        )
    })?;
    let n4h = frontier.b_size.num_4x4_high().map_err(|_| {
        unsupported_at(
            "inter_block_geometry",
            tile_offset,
            "minimal inter block geometry lookup failed",
            SPEC_MODE_INFO,
        )
    })?;
    let mi_row = frontier.r;
    let mi_col = frontier.c;

    let mut block_ctx = MvBlockContext {
        mi_row,
        mi_col,
        bw4: n4w,
        bh4: n4h,
        sb_h4,
        ref_frame0: SINGLE_REF_FRAME0,
        mi_rows,
        mi_cols,
    };

    let neighbour_ctx = block_neighbour_ctx(mv_grid, &block_ctx);
    let mode_ctx = find_mode_ctx(mv_grid, &block_ctx);

    if mv_stack_tools_present && neighbour_ctx.has_neighbour {
        return Err(unsupported_at(
            "inter_block_mv_stack_tools_with_neighbour",
            tile_offset,
            "minimal inter decode requires the spatial-only §7.12.2 MV-stack subset (no temporal ref-frame-mvs, no reference MV bank, no DRL reorder) once a block has a decoded neighbour",
            super::SPEC_MV,
        ));
    }

    #[allow(clippy::items_after_statements)]
    const MIN_INTER_LEAF_N4: usize = 8;
    if n4w < MIN_INTER_LEAF_N4 || n4h < MIN_INTER_LEAF_N4 {
        return Err(unsupported_at(
            "inter_block_subblock_unverified_size",
            tile_offset,
            "minimal inter decode is verified only for >= 32x32 leaves; a sub-32x32 inter leaf is rejected (it carries §5.20.7.6 needs_interp_filter / §5.20.7.3 shared-tree is_inter / §7.12.2.5 scan_col special cases this kernel does not model)",
            super::SPEC_MV,
        ));
    }

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    let is_inter = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::IsInter {
                ctx: neighbour_ctx.is_inter_ctx,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    if is_inter.get() != 1 {
        return Err(unsupported_at(
            "inter_block_is_intra",
            tile_offset,
            "minimal inter decode only supports an inter (is_inter == 1) block",
            SPEC_MODE_INFO,
        ));
    }

    let skip = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::Skip {
                ctx: neighbour_ctx.skip_ctx,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    let skip = skip.get();
    if skip != 0 && skip != 1 {
        return Err(unsupported_at(
            "inter_block_unexpected_skip",
            tile_offset,
            "minimal inter decode read an out-of-range skip value",
            SPEC_MODE_INFO,
        ));
    }

    if reference_select {
        let is_joint_ctx = compound_is_joint_ctx.ok_or_else(|| {
            unsupported_compound_at(
                "compound_missing_is_joint_context",
                tile_offset,
                "minimal compound-average decode requires the frame-level §8.3.2 is_joint context",
                SPEC_MODE_INFO,
            )
        })?;
        let compound = read_compound_average_syntax(
            cdfs,
            symbols,
            CompoundParseInput {
                num_total_refs,
                num_same_ref_compound,
                has_neighbour: neighbour_ctx.has_neighbour,
                new_mv_context: mode_ctx.new_mv_context,
                is_joint_ctx,
                skip,
                n4w,
                n4h,
                mi_row,
                mi_col,
                mi_rows,
                mi_cols,
            },
            tile_offset,
        )?;
        let ref_mv_idx0 = read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?;
        let ref_mv_idx1 = read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?;
        if ref_mv_idx0 != 0 || ref_mv_idx1 != 0 {
            return Err(unsupported_compound_at(
                "compound_block_drl_idx",
                tile_offset,
                "minimal compound-average decode only supports the no-neighbour NEAR_NEARMV DRL indices RefMvIdx0 == 0 and RefMvIdx1 == 0",
                SPEC_MODE_INFO,
            ));
        }
        let interp = resolve_interp_filter(
            cdfs,
            symbols,
            frame_interpolation_filter,
            SINGLE_MODE_NEARMV,
            true,
            neighbour_ctx.has_neighbour,
            tile_offset,
        )?;
        mv_grid.record_block(
            mi_row,
            mi_col,
            n4w,
            n4h,
            true,
            compound.ref_frame0,
            NeighbourYMode::Other,
            compound.mv0,
            true,
        );
        return Ok(PlacedInterBlock {
            luma_x: mi_col * 4,
            luma_y: mi_row * 4,
            luma_w: n4w * 4,
            luma_h: n4h * 4,
            block: InterBlock {
                ref_frame0: compound.ref_frame0,
                ref_frame1: Some(compound.ref_frame1),
                mv: compound.mv0,
                mv1: compound.mv1,
                interp,
                residual: None,
            },
        });
    }

    let ref_frame0: i8 = if num_total_refs >= 2 {
        if neighbour_ctx.has_neighbour {
            return Err(unsupported_at(
                "inter_block_single_ref_with_neighbour",
                tile_offset,
                "minimal inter decode reads single_ref only for a no-neighbour block (the §8.3.2 ctx is provably 1); a neighbour-having NumTotalRefs == 2 block is not yet fixtured",
                SPEC_MODE_INFO,
            ));
        }
        let ctx = neighbour_ctx
            .single_ref_ctx(0, num_total_refs)
            .ok_or_else(|| {
                unsupported_at(
                    "inter_block_single_ref_ctx",
                    tile_offset,
                    "minimal inter decode could not derive the §8.3.2 single_ref context",
                    SPEC_MODE_INFO,
                )
            })?;
        let contexts = [ctx];
        let selected = super::single_ref::read_single_ref(cdfs, symbols, num_total_refs, &contexts)
            .map_err(|_| {
                unsupported_at(
                    "inter_block_single_ref_read",
                    tile_offset,
                    "minimal inter decode could not read the §5.20.7.12 single_ref symbol",
                    SPEC_MODE_INFO,
                )
            })?;
        i8::try_from(selected).map_err(|_| {
            unsupported_at(
                "inter_block_single_ref_value",
                tile_offset,
                "minimal inter decode read an out-of-range single_ref selection",
                SPEC_MODE_INFO,
            )
        })?
    } else {
        SINGLE_REF_FRAME0
    };
    block_ctx.ref_frame0 = ref_frame0;

    let single_mode = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::SingleMode {
                ctx: mode_ctx.new_mv_context,
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?;
    let single_mode = single_mode.get();
    if single_mode != SINGLE_MODE_NEARMV
        && single_mode != SINGLE_MODE_GLOBALMV
        && single_mode != SINGLE_MODE_NEWMV
    {
        return Err(unsupported_at(
            "inter_block_unsupported_single_mode",
            tile_offset,
            "minimal inter decode only supports the single-reference NEARMV (0) / GLOBALMV (1) / NEWMV (2) modes; a compound or other single-reference mode is not yet implemented",
            SPEC_MODE_INFO,
        ));
    }

    let stack = find_mv_stack(mv_grid, &block_ctx, Mv::ZERO);

    let ref_mv_idx = if single_mode == SINGLE_MODE_NEARMV || single_mode == SINGLE_MODE_NEWMV {
        read_drl_idx(
            cdfs,
            symbols,
            mode_ctx.new_mv_context,
            max_drl_bits_minus_1,
            tile_offset,
        )?
    } else {
        0
    };

    if (single_mode == SINGLE_MODE_NEARMV || single_mode == SINGLE_MODE_NEWMV)
        && ref_mv_idx >= stack.num_mv_found()
    {
        return Err(unsupported_at(
            "inter_block_drl_idx_out_of_range",
            tile_offset,
            "minimal inter decode read a DRL RefMvIdx past the §7.12.2 MV stack",
            SPEC_MODE_INFO,
        ));
    }
    let pred_mv = stack.candidate(ref_mv_idx);
    let mv = match single_mode {
        SINGLE_MODE_GLOBALMV => Mv::ZERO,
        SINGLE_MODE_NEARMV => pred_mv,
        _ => {
            let diff = read_newmv_block_mvd(cdfs, symbols, tile_offset)?;
            Mv {
                row: mv_clamp_to_integer(pred_mv.row + diff.row),
                col: mv_clamp_to_integer(pred_mv.col + diff.col),
            }
        }
    };

    let interp = resolve_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        single_mode,
        false,
        neighbour_ctx.has_neighbour,
        tile_offset,
    )?;

    let residual = if skip == 0 {
        // TODO(spec: DECODE-INTER-MVSTACK-SPATIAL): per-block (sub-64x64) and
        if !residual_quantizer_deltas_are_zero {
            return Err(unsupported_at(
                "inter_block_residual_quantizer_delta",
                tile_offset,
                "minimal inter residual decode requires zero effective quantizer deltas (DeltaQ* + Base*DeltaQ) before using the verified zero-delta dequantization subset",
                SPEC_MODE_INFO,
            ));
        }
        #[allow(clippy::items_after_statements)]
        const FULL_SB_N4: usize = 16;
        if n4w != FULL_SB_N4
            || n4h != FULL_SB_N4
            || mi_row != 0
            || mi_col != 0
            || mi_rows > FULL_SB_N4
            || mi_cols > FULL_SB_N4
        {
            return Err(unsupported_at(
                "inter_block_multiblock_residual",
                tile_offset,
                "minimal inter decode only models a skip == 0 residual for the single top-left 64x64 block of a single-superblock frame; a multi-block or multi-superblock residual needs per-block transform sizes",
                SPEC_MODE_INFO,
            ));
        }
        if residual_tools_present {
            return Err(unsupported_at(
                "inter_block_residual_tools",
                tile_offset,
                "minimal inter residual decode requires the DCT-only transform subset (no inter-IST / inter-DDT / CCTX / FSC / IDTX-intra)",
                SPEC_MODE_INFO,
            ));
        }
        Some(read_inter_residual(
            work_unit,
            symbols,
            coeff_ctx,
            tile_offset,
        )?)
    } else {
        None
    };

    let y_mode = if single_mode == SINGLE_MODE_NEWMV {
        NeighbourYMode::NewMv
    } else {
        NeighbourYMode::Other
    };
    mv_grid.record_block(
        mi_row,
        mi_col,
        n4w,
        n4h,
        true,
        ref_frame0,
        y_mode,
        mv,
        skip == 1,
    );

    Ok(PlacedInterBlock {
        luma_x: mi_col * 4,
        luma_y: mi_row * 4,
        luma_w: n4w * 4,
        luma_h: n4h * 4,
        block: InterBlock {
            ref_frame0,
            ref_frame1: None,
            mv,
            mv1: Mv::ZERO,
            interp,
            residual,
        },
    })
}

/// AV2 § 9.2 `TX_SIZES_ALL` index of `TX_64X64` (the single luma transform for
/// the 64x64 inter block under TX_MODE_LARGEST; § 5.20.6.1 read_tx_size returns 0
/// and TxSize = maxRectTxSize = TX_64X64) and `TX_32X32` (its 4:2:0 chroma).
const TX_64X64: usize = 4;
const TX_32X32: usize = 3;
/// `UV_DC_PRED` (= 0): inter blocks have no UV intra mode, so the §5.20.7.27
/// nonzero coefficient pass reads no chroma intra-mode-dependent transform type;
/// DCT_DCT is forced for the 64x64/32x32 DCT-only inter transform set.
const INTER_UV_MODE_DC: usize = 0;

/// Reads the AV2 § 5.20.7.27 residual coefficients for the single 64x64 inter
/// block (`skip == 0`): the luma TX_64X64 transform block, then the U and V
/// TX_32X32 chroma transform blocks, all `is_inter == 1`.
///
/// § 5.20.8.3 `get_tx_set(TX_64X64, 0)` returns `TX_SET_DCTONLY` (txSzSqrUp >
/// TX_32X32 && txSzSqr >= TX_32X32), so § 5.20.8.2 `transform_type()` reads NO
/// `inter_tx_type` symbol (`set == 0`) and `PlaneTxType = DCT_DCT`. The caller's
/// `residual_tools_present` gate rejects `enable_inter_ist` before this runs, so
/// no `sec_tx_type` symbol is read either. The chroma tx type is *derived*, not
/// signalled — § 5.20.8.2 reads no chroma `tx_type` symbol (for an inter TX_32X32
/// chroma block § 5.20.8.3 `get_tx_set` is `TX_SET_DCT_IDTX`, not the DCT-only
/// branch, but chroma forces `DCT_DCT`), so this reuses the intra DCT_DCT
/// coefficient loop with `is_inter == true` (the only inter-specific contexts are
/// the § 8.3.2 `TileTxbSkipCdf[is_inter || fsc_mode]` bank and the `eobCtx =
/// is_inter` luma EOB context, both threaded through
/// `decode_general_intra_plane_coeffs`). The supported case's chroma is
/// `all_zero` (inter chroma == flat key chroma), so this luma-residual + chroma-
/// skip path is oracle-exact; a coded *chroma* inter residual is not yet exercised.
fn read_inter_residual(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    tile_offset: ByteOffset,
) -> Result<InterResidual> {
    let luma = decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        0,
        TX_64X64,
        0,
        0,
        true,
        false,
        INTER_UV_MODE_DC,
        true,
        false,
        TransformToolResidualPolicy::Allow,
    )
    .map_err(|_| residual_read_error(tile_offset))?;
    let u = decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        1,
        TX_32X32,
        0,
        0,
        true,
        false,
        INTER_UV_MODE_DC,
        true,
        false,
        TransformToolResidualPolicy::Allow,
    )
    .map_err(|_| residual_read_error(tile_offset))?;
    let v = decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        2,
        TX_32X32,
        0,
        0,
        true,
        !u.all_zero,
        INTER_UV_MODE_DC,
        true,
        false,
        TransformToolResidualPolicy::Allow,
    )
    .map_err(|_| residual_read_error(tile_offset))?;
    Ok(InterResidual { luma, u, v })
}

fn residual_read_error(tile_offset: ByteOffset) -> super::super::DecodeError {
    unsupported_at(
        "inter_block_residual_parse",
        tile_offset,
        "minimal inter block §5.20.7.27 residual coefficients could not be parsed from the tile payload",
        SPEC_MODE_INFO,
    )
}

/// AV2 § 5.20.7.6 `interp_filter` resolution for the verified no-neighbour
/// inter block, mapped to the recon-side § 7.13.3.18 filter.
///
/// A fixed frame filter supplies the block filter directly (no symbol). A
/// SWITCHABLE frame filter reads the per-block `interp_filter` symbol
/// (`TileInterpFilterCdf[ctx]`) when `needs_interp_filter()` is 1. For the verified
/// `motion_mode == SIMPLE` block `needs_interp_filter()` returns 0 only for a large
/// (>= 8x8) GLOBALMV block (which then uses EIGHTTAP); NEARMV / NEWMV and the
/// verified compound NEAR_NEARMV path always read the symbol.
fn resolve_interp_filter(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_interpolation_filter: FrameInterpolationFilter,
    mode_for_needs_interp_filter: u8,
    ref_frame1_is_inter: bool,
    has_neighbour: bool,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    match frame_interpolation_filter {
        FrameInterpolationFilter::Eighttap => Ok(ReconInterpolationFilter::EightTap),
        FrameInterpolationFilter::EighttapSmooth => Ok(ReconInterpolationFilter::EightTapSmooth),
        FrameInterpolationFilter::EighttapSharp => Ok(ReconInterpolationFilter::EightTapSharp),
        FrameInterpolationFilter::Bilinear => Ok(ReconInterpolationFilter::Bilinear),
        FrameInterpolationFilter::Switchable => {
            if mode_for_needs_interp_filter == SINGLE_MODE_GLOBALMV {
                return Ok(ReconInterpolationFilter::EightTap);
            }
            if has_neighbour {
                return Err(unsupported_at(
                    "inter_block_interp_filter_neighbour_ctx",
                    tile_offset,
                    "minimal inter decode models only the no-neighbour §8.3.2 interp_filter context; a SWITCHABLE frame filter with a decoded neighbour is rejected",
                    SPEC_MODE_INFO,
                ));
            }
            let symbol = cdfs
                .read_block_symbol_trace(
                    TileCdfSelector::InterpFilter {
                        ctx: interp_filter_no_neighbour_ctx(ref_frame1_is_inter),
                    },
                    symbols,
                )
                .map_err(|_| symbol_read_error(tile_offset))?;
            interp_filter_from_symbol(symbol.get(), tile_offset)
        }
        _ => Err(unsupported_at(
            "inter_unsupported_interpolation_filter",
            tile_offset,
            "minimal inter decode encountered an unsupported frame interpolation_filter",
            SPEC_MODE_INFO,
        )),
    }
}

pub(super) fn interp_filter_no_neighbour_ctx(ref_frame1_is_inter: bool) -> usize {
    INTERP_FILTER_CTX_NO_NEIGHBOUR_BASE
        + usize::from(ref_frame1_is_inter) * INTERP_FILTER_CTX_SECOND_REF_INTER_OFFSET
}

/// Maps a decoded `interp_filter` symbol (`0..3`) to the recon § 7.13.3.18 filter.
fn interp_filter_from_symbol(
    symbol: u8,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    match symbol {
        0 => Ok(ReconInterpolationFilter::EightTap),
        1 => Ok(ReconInterpolationFilter::EightTapSmooth),
        2 => Ok(ReconInterpolationFilter::EightTapSharp),
        3 => Ok(ReconInterpolationFilter::Bilinear),
        _ => Err(unsupported_at(
            "inter_invalid_interp_filter_symbol",
            tile_offset,
            "minimal inter decode read an out-of-range interp_filter symbol",
            SPEC_MODE_INFO,
        )),
    }
}

/// AV2 § 5.20.7.8 `read_drl_idx(0, m)` for the single-reference NEARMV / NEWMV
/// block: reads `drl_mode` symbols from `TileDrlModeCdf[Min(idx, 2)][NewMvContext]`
/// (§ 8.3.2) until one decodes to 0 or `idx` reaches `m`, returning the decoded
/// `RefMvIdx`. The returned index selects the § 7.12.2 MV-stack predictor
/// candidate. The symbol reads advance the § 8.2 arithmetic decoder and are
/// validated bit-exactly by the caller's `exit_symbol()`.
fn read_drl_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let m = max_drl_bits_minus_1.saturating_add(1) as usize;
    for idx in 0..m {
        let bank = idx.min(2);
        let drl_mode = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::DrlMode {
                    idx: bank,
                    ctx: new_mv_context,
                },
                symbols,
            )
            .map_err(|_| symbol_read_error(tile_offset))?;
        if drl_mode.get() == 0 {
            return Ok(idx);
        }
    }
    Ok(m)
}

fn symbol_read_error(tile_offset: ByteOffset) -> super::super::DecodeError {
    unsupported_at(
        "inter_block_mode_parse",
        tile_offset,
        "minimal inter block mode-info syntax could not be parsed from the tile payload",
        SPEC_MODE_INFO,
    )
}

fn map_inter_multiblock_error(
    error: GeneralIntraMultiblockError<super::super::DecodeError>,
    tile_offset: ByteOffset,
) -> super::super::DecodeError {
    match error {
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Leaf(error)) => error,
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Traversal(
            TilePartitionTraversalError::Limit(source),
        )) => super::super::DecodeError::Limit { source },
        _ => unsupported_at(
            "inter_partition_walk",
            tile_offset,
            "minimal inter decode could not reach a supported §5.20.3.1 single-block partition frontier",
            SPEC_MODE_INFO,
        ),
    }
}
