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
use super::find_mv_stack::{
    MvBlockContext, NeighbourMvGrid, NeighbourYMode, block_neighbour_ctx, find_mode_ctx,
    find_mv_stack,
};
use super::read_mv::{mv_clamp_to_integer, read_newmv_block_mvd};
use super::{
    InterBlock, InterResidual, Mv, PlacedInterBlock, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV,
    SINGLE_MODE_NEWMV, SPEC_MODE_INFO, unsupported_at,
};
use crate::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, GeneralIntraMultiblockError,
    GeneralIntraTreeWalkError, TileCdfSelector, TileCdfSubset, TileCoeffContextState,
    TilePartitionTraversalError, decode_general_intra_multiblock_tree,
    decode_general_intra_plane_coeffs, frame_mi_dimensions,
};

/// AV2 § 8.3.2 `interp_filter` context for the verified single-reference block
/// (single ref -> `is_inter_ref_frame(RefFrame[1]) * 4 == 0`). With no decoded
/// neighbours `NNum == 0` gives `ctx == 3`; the verified subset only reads the
/// per-block `interp_filter` symbol when the frame filter is SWITCHABLE, which
/// the fixture's fixed frame filter is not, so this no-neighbour context is the
/// only one exercised.
const INTERP_FILTER_CTX_NO_NEIGHBOUR: usize = 3;

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
    let tile = match tile_plan.work_units_mut() {
        [tile] => tile,
        _ => {
            return Err(unsupported_at(
                "inter_unexpected_tile_work_units",
                offset,
                "minimal inter decode requires exactly one tile work unit",
                SPEC_MODE_INFO,
            ));
        }
    };
    let tile_offset = tile.tile_byte_span().start;

    // §5.20.7.6 / §5.20.7.8 DRL bound: m = max_drl_bits_minus_1 + 1 from the parsed
    // inter control region.
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

    // §5.20.7.27 coefficient neighbour-context state for the inter residual
    // (skip == 0). A skip == 1 block reads no coefficients and never touches it;
    // it is sized to the frame MI grid like the intra path.
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

    // §7.11/§7.12 spatial-neighbour MV grid: each decoded block's IsInters /
    // RefFrames[0] / YModes / Mvs[0] are recorded so a later block in decode order
    // predicts its MV from the already-decoded left/above neighbours.
    let mut mv_grid = NeighbourMvGrid::new(mi_rows, mi_cols).ok_or_else(|| {
        unsupported_at(
            "inter_mv_grid",
            offset,
            "minimal inter decode could not allocate the §7.11/§7.12 neighbour MV grid",
            SPEC_MODE_INFO,
        )
    })?;
    // §7.12.2 step 15 superblock height for the `isSbBorder` derivation: the
    // verified subset is sb_size 64 (Num_4x4_Blocks_High[SbSize] == 16). Derived
    // from the parsed sequence partition config to avoid a hard-coded assumption.
    let sb_h4 = superblock_h4(sequence, core).ok_or_else(|| {
        unsupported_at(
            "inter_sb_size",
            offset,
            "minimal inter decode requires the parsed superblock size",
            SPEC_MODE_INFO,
        )
    })?;

    // §5.20.8.2 transform_type() / §5.20.7.27 coeffs(): the verified skip == 0
    // residual is read as a single DCT_DCT TX_64X64 luma + TX_32X32 chroma transform
    // block (§5.20.8.3 get_tx_set returns TX_SET_DCTONLY for those sizes, so no
    // inter_tx_type symbol). `enable_inter_ist` would make transform_type() read a
    // `sec_tx_type` symbol (eob > 3, DCT_DCT, >= 16x16); `enable_inter_ddt` adds an
    // inter data-driven transform type; `enable_cctx` adds a cross-chroma-component
    // transform symbol; `enable_fsc` / `enable_idtx_intra` change the §8.3.2
    // `txb_skip` / IDTX coefficient path. The residual decode reads none of these,
    // so a skip == 0 block whose sequence enables any of them is rejected (a skip ==
    // 1 block reads no residual and is unaffected — the existing skip == 1 sub-pel
    // fixture enables enable_inter_ist/inter_ddt and must still decode).
    let residual_tools_present = sequence.transform_quant_entropy.is_none_or(|tq| {
        tq.enable_inter_ist
            || tq.enable_inter_ddt
            || tq.enable_cctx
            || tq.enable_fsc
            || tq.enable_idtx_intra
    });

    // §7.12.2 find_mv_stack subset: the spatial single-prediction kernel models
    // neither the temporal scan (§7.12.2.7/§7.12.2.8, gated on use_ref_frame_mvs
    // from enable_ref_frame_mvs), the reference MV bank (§7.12.2.21,
    // enable_refmvbank), nor the DRL reorder sort (§7.12.2.19, DrlReorder !=
    // Disabled). For a NO-NEIGHBOUR block these steps are provable no-ops (the first
    // inter frame after a key frame has an empty motion field; the per-tile ref-MV
    // bank starts empty; the sort needs numNearest >= 4 neighbours), so a
    // single-block inter frame that enables them still decodes correctly. But a
    // block WITH a decoded neighbour would build a different RefStackMv from the
    // kernel, so the per-block decode rejects the deferred tools once a neighbour
    // exists (the §8.2.4 exit_symbol() bit-count check would not catch a wrong MV
    // value). Captured here from the sequence config.
    let mv_stack_tools_present = sequence.inter.as_ref().is_none_or(|seq_inter| {
        seq_inter.enable_ref_frame_mvs
            || seq_inter.enable_refmvbank
            || seq_inter.drl_reorder != splot_core::headers::sequence::DrlReorder::Disabled
    });

    // Each decoded leaf inter block in decode (DFS) order. The partition walk
    // invokes the closure per §5.20.3 PARTITION_NONE leaf; each block decodes its
    // §5.20.7.6 mode_info over the spatial-neighbour-derived contexts + MV stack,
    // records itself into the grid, and yields a placed block for the caller's MC.
    let mut decoded_blocks: Vec<PlacedInterBlock> = Vec::new();
    let limits = options.limits();
    let symbols = decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit, symbols, frontier, _joint_modes, _block_decoded| {
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
                mv_stack_tools_present,
                tile_offset,
            )?;
            decoded_blocks.push(placed);
            // AV2 § 5.20.5.3 IntraJointMode for an inter block is DC_PRED (= 0); the
            // partition walk records this grid value but inter neighbours ignore it.
            Ok(0u8)
        },
    )
    .map_err(|error| map_inter_multiblock_error(error, tile_offset))?;

    // §8.2.4 exit_symbol(): the decoded block must consume the whole tile payload
    // (mode info, and for skip == 0 the §5.20.7.27 residual coefficients). A
    // failure means the symbol reads were not bit-exact, so reject rather than
    // emit wrong output. This is the backstop that proves every §5.20.7.6 /
    // §5.20.7.20 / §5.20.7.27 symbol read (mode, DRL, shell MV, interp_filter,
    // residual coeffs) was bit-exact.
    symbols.exit_symbol().map_err(|_| {
        unsupported_at(
            "inter_exit_symbol",
            tile_offset,
            "minimal inter tile payload did not satisfy §8.2.4 exit_symbol() after the decoded inter block",
            SPEC_MODE_INFO,
        )
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
/// (the verified fixture is a single 64x64 superblock per tile).
fn superblock_h4(sequence: &SequenceHeader, core: &FrameHeaderCore) -> Option<usize> {
    let partition = sequence.partition?;
    let frame_is_intra = core.frame_is_intra?;
    // §5.20.2.1: an inter frame uses the sequence superblock size directly.
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
#[allow(clippy::too_many_arguments)]
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
    mv_stack_tools_present: bool,
    tile_offset: ByteOffset,
) -> Result<PlacedInterBlock> {
    // The block's geometry in 4x4 MI units + luma samples. The single-block gate is
    // lifted: each §5.20.3 leaf inter block is decoded at its own position/size, so a
    // partition split into multiple inter blocks is admitted.
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

    // §7.11/§7.12 block context for the spatial neighbour scan.
    let block_ctx = MvBlockContext {
        mi_row,
        mi_col,
        bw4: n4w,
        bh4: n4h,
        sb_h4,
        ref_frame0: SINGLE_REF_FRAME0,
        mi_rows,
        mi_cols,
    };

    // §5.20.7.2 neighbour-buffer contexts (is_inter / skip) + §7.11.2 mode context
    // (NewMvContext for single_mode / DRL). Derived from the already-decoded
    // neighbours in the grid BEFORE reading this block's symbols, exactly as the
    // §8.3.2 context derivation requires.
    let neighbour_ctx = block_neighbour_ctx(mv_grid, &block_ctx);
    let mode_ctx = find_mode_ctx(mv_grid, &block_ctx);

    // §7.12.2 find_mv_stack subset gate: the temporal scan / ref-MV bank / DRL
    // reorder steps are deferred. They are provable no-ops for a NO-NEIGHBOUR block
    // (empty motion field on the first inter frame, empty per-tile bank, no sort
    // without >= 4 neighbours), so a no-neighbour block decodes correctly even when
    // the sequence enables them. But a block WITH a decoded neighbour would build a
    // different RefStackMv than this kernel, so reject the deferred tools here once a
    // neighbour exists — the §8.2.4 exit_symbol() bit-count check cannot catch a
    // wrong MV value, only a wrong bit count.
    if mv_stack_tools_present && neighbour_ctx.has_neighbour {
        return Err(unsupported_at(
            "inter_block_mv_stack_tools_with_neighbour",
            tile_offset,
            "minimal inter decode requires the spatial-only §7.12.2 MV-stack subset (no temporal ref-frame-mvs, no reference MV bank, no DRL reorder) once a block has a decoded neighbour",
            super::SPEC_MV,
        ));
    }

    // §7.12.2.5 scan_col (find_mv_stack step 16, deltaCol = -3) is deferred. The
    // §7.12.2.6 guard fires when `MiColBase[MiCol-3] != MiColBase[MiCol-1]` — i.e.
    // when a base-column boundary falls between `MiCol-3` and `MiCol-1`. Creating
    // such a boundary requires a sub-32x32 block somewhere in that 2-MI span, and by
    // §5.20.3 DFS decode order that sub-32x32 block (which has its own decoded
    // neighbour) is reached EARLIER and rejected by this same gate — so any frame
    // that could make scan_col append a distinct candidate is rejected before a
    // >= 32x32 leaf reaches here. (The narrower "the left block is also >= 32x32"
    // claim is not independently true; DFS ordering is what makes the gate sound.)
    // For a sub-32x32 leaf WITH a decoded neighbour scan_col can fire and append a
    // stack candidate this kernel omits, so a DRL index that selects it resolves a
    // WRONG MV — and scan_col reads no symbol, so §8.2.4 exit_symbol() (bit-count
    // only) cannot catch it. Reject a sub-32x32 inter leaf once a neighbour exists
    // (a no-neighbour leaf stays a no-op and remains admitted).
    // 32x32 == 8 4x4-MI units.
    const SCAN_COL_NOOP_MIN_N4: usize = 8;
    if neighbour_ctx.has_neighbour && (n4w < SCAN_COL_NOOP_MIN_N4 || n4h < SCAN_COL_NOOP_MIN_N4) {
        return Err(unsupported_at(
            "inter_block_subblock_scan_col_with_neighbour",
            tile_offset,
            "minimal inter decode defers §7.12.2.5 scan_col, a no-op only for >= 32x32 leaves; a sub-32x32 inter leaf with a decoded neighbour is rejected",
            super::SPEC_MV,
        ));
    }

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    // §5.20.7.3 read_is_inter: TileIsInterCdf[ctx], ctx from §5.20.7.2 / §8.3.2
    // (NNumBuf + NIntra). Must decode to is_inter == 1.
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

    // §5.20.5.10 read_skip: TileSkipCdf[ctx], ctx = sum of neighbour Skips[]
    // (§8.3.2; skip_mode == 0). The verified subset admits skip == 1 (no residual,
    // the §7.13.3.18 MC prediction is the reconstruction) and skip == 0 (a coded
    // §5.20.7.27 residual added over the MC prediction). Any other value desyncs the
    // §8.2 decoder and fails the caller's exit_symbol() backstop.
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

    // §5.20.7.6 single_mode: TileSingleModeCdf[NewMvContext]. NewMvContext is the
    // §7.11.2 find-mode-context output: 0 for a no-neighbour block, or up to 3 when
    // inter NEW-MV neighbours are present. YMode = NEARMV + single_mode. The verified
    // subset is the NEARMV (single_mode == 0) / GLOBALMV (1) / NEWMV (2) modes.
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

    // §5.20.7.6 read_motion_mode: the verified subset disables every motion mode
    // (frame_enabled_motion_modes all false), so read_motion_mode returns SIMPLE with
    // no symbol read. (Handled implicitly: no symbol consumed.)

    // §7.12.2 find_mv_stack: build the spatial-neighbour MV candidate stack BEFORE
    // the DRL read (the DRL index selects a stack entry). GlobalMvs[0] is the zero
    // vector (the inter header gate rejects use_global_motion).
    let stack = find_mv_stack(mv_grid, &block_ctx, Mv::ZERO);

    // §5.20.7.6 / §5.20.7.8 DRL: GLOBALMV reads no DRL; NEARMV (has_nearmv) and NEWMV
    // (has_newmv) read `read_drl_idx(0, m)` where `m = max_drl_bits_minus_1 + 1`. The
    // drl_mode CDF is TileDrlModeCdf[Min(idx, 2)][NewMvContext] (§8.3.2). The decoded
    // RefMvIdx selects the MV-stack predictor candidate.
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

    // §5.20.7.6 MvPrecision derivation: the verified subset disables flexible MV
    // resolution and adaptive MVD (enable_flex_mvres == 0, enable_adaptive_mvd == 0),
    // so MvPrecision == FrameMvPrecision (EighthPel here) and no precision symbol is
    // read. The inter header gate rejects any enable_flex_mvres frame.

    // §5.20.7.6 / §5.20.7.20 / §5.20.7.13 assign_mv (single prediction):
    //  - GLOBALMV: PredMvs[0] = GlobalMvs[0] (zero), diffMv = 0 -> MV = (0, 0).
    //  - NEARMV: PredMvs[0] = RefStackMv[RefMvIdx][0], diffMv = 0 -> MV = the stack
    //    predictor (a neighbour's MV when one was found).
    //  - NEWMV: PredMvs[0] = RefStackMv[RefMvIdx][0], MV = clamp(PredMvs[0] +
    //    read_mv() delta). EighthPel makes the lower_mv_precision step a no-op.
    // §5.20.7.6: RefMvIdx must index a candidate in the §7.12.2 stack (the spec's
    // global-MV fallback guarantees NumMvFound >= 1). A DRL index past the stack
    // means the stream is malformed for this single-prediction subset; reject it
    // rather than predict from a clamped fallback that the oracle would not match.
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
        // SINGLE_MODE_NEWMV
        _ => {
            let diff = read_newmv_block_mvd(cdfs, symbols, tile_offset)?;
            Mv {
                row: mv_clamp_to_integer(pred_mv.row + diff.row),
                col: mv_clamp_to_integer(pred_mv.col + diff.col),
            }
        }
    };

    // §5.20.7.17 read_refinemv / §5.20.7.16 read_compound_type: single-reference
    // (isCompound == 0) with inter_intra == 0 reads no symbol.

    // §5.20.7.6 interp_filter: when the frame interpolation_filter is SWITCHABLE and
    // needs_interp_filter() is 1, read the per-block `interp_filter` symbol. A fixed
    // frame filter supplies the filter with no block symbol. needs_interp_filter() is
    // 0 only for a large (>= 8x8) GLOBALMV block; NEARMV / NEWMV always need it.
    let interp = resolve_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        single_mode,
        neighbour_ctx.has_neighbour,
        tile_offset,
    )?;

    // §5.20.7 decode_block: after mode_info, read_block_tx_size() reads no symbol
    // under TX_MODE_LARGEST. Then for skip == 0 residual() reads the §5.20.7.27
    // coefficients per plane (Y, U, V). skip == 1 reads none.
    let residual = if skip == 0 {
        // A skip == 0 block reads the §5.20.7.27 residual, whose transform-type /
        // coefficient path the verified subset only models for the DCT-only,
        // no-IST / no-DDT / no-CCTX / no-FSC / no-IDTX-intra case. The residual is
        // additionally only modelled for the single full-superblock 64x64 block
        // (TX_64X64 luma / TX_32X32 chroma); a multi-block skip == 0 residual needs
        // per-block TX sizes (a future brick), so reject it here.
        // TODO(spec: DECODE-INTER-MVSTACK-SPATIAL): per-block (sub-64x64) skip == 0
        // residual transform sizes.
        const FULL_SB_N4: usize = 16;
        if n4w != FULL_SB_N4 || n4h != FULL_SB_N4 || mi_row != 0 || mi_col != 0 {
            return Err(unsupported_at(
                "inter_block_multiblock_residual",
                tile_offset,
                "minimal inter decode only models a skip == 0 residual for the single top-left 64x64 block; a multi-block residual needs per-block transform sizes",
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

    // §5.20.4.1 decode_block records the block's mode info into the per-MI grid so a
    // later block in decode order predicts from it. §5.20.7.6 YMode = NEARMV +
    // single_mode; only NEWMV (single_mode == 2) is a NEW MV for the §7.11.3 NewMvCount.
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
        SINGLE_REF_FRAME0,
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
            mv,
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
/// `decode_general_intra_plane_coeffs`). The verified fixture's chroma is
/// `all_zero` (inter chroma == flat key chroma), so this luma-residual + chroma-
/// skip path is oracle-exact; a coded *chroma* inter residual is not yet exercised.
fn read_inter_residual(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    tile_offset: ByteOffset,
) -> Result<InterResidual> {
    // The single 64x64 block at the tile origin: luma plane-sample (0, 0),
    // 4:2:0 chroma plane-sample (0, 0). (Gated to the top-left 64x64 block
    // above.)
    let luma = decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        0,
        TX_64X64,
        0,
        0,
        false,
        INTER_UV_MODE_DC,
        true,
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
        false,
        INTER_UV_MODE_DC,
        true,
    )
    .map_err(|_| residual_read_error(tile_offset))?;
    // §5.20.7.27 v_txb_skip uses EobU != 0 (the U plane's eob); pass !u.all_zero.
    let v = decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        2,
        TX_32X32,
        0,
        0,
        !u.all_zero,
        INTER_UV_MODE_DC,
        true,
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

/// AV2 § 5.20.7.6 `interp_filter` resolution for the verified single-reference
/// no-neighbour block, mapped to the recon-side § 7.13.3.18 filter.
///
/// A fixed frame filter supplies the block filter directly (no symbol). A
/// SWITCHABLE frame filter reads the per-block `interp_filter` symbol
/// (`TileInterpFilterCdf[ctx]`) when `needs_interp_filter()` is 1. For the verified
/// `motion_mode == SIMPLE` block `needs_interp_filter()` returns 0 only for a large
/// (>= 8x8) GLOBALMV block (which then uses EIGHTTAP); NEARMV / NEWMV always read
/// the symbol.
fn resolve_interp_filter(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_interpolation_filter: FrameInterpolationFilter,
    single_mode: u8,
    has_neighbour: bool,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    match frame_interpolation_filter {
        FrameInterpolationFilter::Eighttap => Ok(ReconInterpolationFilter::EightTap),
        FrameInterpolationFilter::EighttapSmooth => Ok(ReconInterpolationFilter::EightTapSmooth),
        FrameInterpolationFilter::EighttapSharp => Ok(ReconInterpolationFilter::EightTapSharp),
        FrameInterpolationFilter::Bilinear => Ok(ReconInterpolationFilter::Bilinear),
        FrameInterpolationFilter::Switchable => {
            // §5.20.7.6 needs_interp_filter(): the 64x64 block is large, so a GLOBALMV
            // block returns 0 (EIGHTTAP, no symbol); NEARMV / NEWMV return 1 and read
            // the per-block symbol.
            if single_mode == SINGLE_MODE_GLOBALMV {
                return Ok(ReconInterpolationFilter::EightTap);
            }
            // §8.3.2 interp_filter ctx is neighbour-dependent (it folds in
            // InterpFilters[neighbour] for a matching-reference neighbour); this kernel
            // models only the no-neighbour ctx == 3. A NEARMV/NEWMV block WITH a decoded
            // neighbour would read the wrong CDF row, so reject a SWITCHABLE filter once
            // a neighbour exists (the no-neighbour single-block sub-pel fixture still
            // decodes; the multi-block fixture uses a fixed frame filter, so neither is
            // affected). A wrong CDF row usually shifts the bit count, but a coincidental
            // same-length decode would pass §8.2.4 exit_symbol() with a wrong filter.
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
                        ctx: INTERP_FILTER_CTX_NO_NEIGHBOUR,
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
