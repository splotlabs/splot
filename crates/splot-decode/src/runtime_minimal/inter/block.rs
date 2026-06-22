// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal inter `mode_info` decode (AV2 § 5.20.7.6) + § 7.11 MV derivation.
//!
//! Walks the § 5.20.3 partition tree to the single 64x64 NONE block, reads the
//! verified inter `mode_info` symbol sequence (`is_inter` / `skip` / `single_mode`
//! / DRL / `read_mv` / `interp_filter`) from the tile arithmetic stream, and
//! confirms § 8.2.4 `exit_symbol()`. The MV result is the § 7.11 zero vector for
//! the zero-MV `NEARMV` / `GLOBALMV` modes, or the § 5.20.7.20 SHELL-coded
//! `read_mv` delta for the `NEWMV` mode (over the zero no-neighbour predictor).

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::frame::InterpolationFilter as FrameInterpolationFilter;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_recon::InterpolationFilter as ReconInterpolationFilter;

use super::super::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};
use super::read_mv::read_newmv_block_mv;
use super::{
    InterBlock, InterResidual, Mv, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV, SINGLE_MODE_NEWMV,
    SPEC_MODE_INFO, unsupported_at,
};
use crate::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, GeneralIntraMultiblockError,
    GeneralIntraTreeWalkError, TileCdfSelector, TileCdfSubset, TileCoeffContextState,
    TilePartitionTraversalError, decode_general_intra_multiblock_tree,
    decode_general_intra_plane_coeffs, frame_mi_dimensions,
};

/// AV2 § 8.3.2 `interp_filter` context for the verified single-reference
/// no-neighbour block: `is_inter_ref_frame(RefFrame[1]) * 4 == 0` (single ref) and
/// `NNum == 0` gives `leftType == aboveType == 3`, so `ctx == 3`.
const INTERP_FILTER_CTX_NO_NEIGHBOUR: usize = 3;

/// Decodes the single inter block's § 5.20.7.6 `mode_info` and returns its § 7.11
/// motion vector + the § 5.20.7.6 block interpolation filter. Runs the real
/// § 5.20.3 partition walk + § 8.2 symbol reads over the tile payload and validates
/// § 8.2.4 `exit_symbol()`.
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_inter_block_and_mv(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: DecodeOptions,
    frame_interpolation_filter: FrameInterpolationFilter,
) -> Result<InterBlock> {
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

    // A single decoded inter block captured from the one closure invocation. The
    // closure runs exactly once for the single 64x64 NONE block; the partition walk
    // records the (intra) joint-mode grid for that block, which inter blocks set to
    // DC_PRED.
    let mut decoded_block: Option<InterBlock> = None;
    let limits = options.limits();
    let symbols = decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit, symbols, frontier, _joint_modes, _block_decoded| {
            let block = decode_one_inter_block(
                work_unit,
                symbols,
                frontier,
                &mut coeff_ctx,
                max_drl_bits_minus_1,
                frame_interpolation_filter,
                residual_tools_present,
                tile_offset,
            )?;
            decoded_block = Some(block);
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

    decoded_block.ok_or_else(|| {
        unsupported_at(
            "inter_no_decoded_block",
            tile_offset,
            "minimal inter decode expected exactly one decoded inter block",
            SPEC_MODE_INFO,
        )
    })
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
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tools_present: bool,
    tile_offset: ByteOffset,
) -> Result<InterBlock> {
    // Gate to the single full-superblock 64x64 NONE block at the top-left, no
    // neighbours. n4w == n4h == 16 (a 64x64 block in 4x4 MI units).
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
    const FULL_SB_N4: usize = 16;
    if frontier.r != 0 || frontier.c != 0 || n4w != FULL_SB_N4 || n4h != FULL_SB_N4 {
        return Err(unsupported_at(
            "inter_unsupported_block_geometry",
            tile_offset,
            "minimal inter decode only supports a single top-left 64x64 (NONE-partition) inter block",
            SPEC_MODE_INFO,
        ));
    }

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    // §5.20.7.3 read_is_inter: TileIsInterCdf[ctx]. For the top-left no-neighbour
    // block NNumBuf == 0 -> ctx == 0. Must decode to is_inter == 1.
    let is_inter = cdfs
        .read_block_symbol_trace(TileCdfSelector::IsInter { ctx: 0 }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    if is_inter.get() != 1 {
        return Err(unsupported_at(
            "inter_block_is_intra",
            tile_offset,
            "minimal inter decode only supports an inter (is_inter == 1) block",
            SPEC_MODE_INFO,
        ));
    }

    // §5.20.5.10 read_skip: TileSkipCdf[ctx]. NNumBuf == 0 and skip_mode == 0 ->
    // ctx == 0. The verified subset admits skip == 1 (no residual, the §7.13.3.18
    // MC prediction is the reconstruction) and skip == 0 (a coded §5.20.7.27
    // residual added over the MC prediction). Any other value desyncs the §8.2
    // decoder and fails the caller's exit_symbol() backstop.
    let skip = cdfs
        .read_block_symbol_trace(TileCdfSelector::Skip { ctx: 0 }, symbols)
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

    // §5.20.7.6 single_mode: TileSingleModeCdf[NewMvContext]. For the no-neighbour
    // block NewMvContext == 0 (§7.11.2: no inter neighbours found, so nearestMatch ==
    // 0 and NewMvCount == 0). YMode = NEARMV + single_mode. The verified subset is the
    // zero-MV NEARMV (single_mode == 0) / GLOBALMV (single_mode == 1) mode or NEWMV
    // (single_mode == 2), which reads a §5.20.7.20 SHELL MV delta.
    let single_mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::SingleMode { ctx: 0 }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    let single_mode = single_mode.get();
    if single_mode != SINGLE_MODE_NEARMV
        && single_mode != SINGLE_MODE_GLOBALMV
        && single_mode != SINGLE_MODE_NEWMV
    {
        return Err(unsupported_at(
            "inter_block_unsupported_single_mode",
            tile_offset,
            "minimal inter decode only supports the single-reference NEARMV (0) / GLOBALMV (1) zero-MV modes or NEWMV (2); a compound or other single-reference mode is not yet implemented",
            SPEC_MODE_INFO,
        ));
    }

    // §5.20.7.6 read_motion_mode: the verified subset disables every motion mode
    // (frame_enabled_motion_modes all false), so read_motion_mode returns SIMPLE with
    // no symbol read. (Handled implicitly: no symbol consumed.)

    // §5.20.7.6 / §5.20.7.8 DRL: GLOBALMV reads no DRL; NEARMV (has_nearmv) and NEWMV
    // (has_newmv) read `read_drl_idx(0, m)` where `m = max_drl_bits_minus_1 + 1`. The
    // drl_mode CDF is TileDrlModeCdf[Min(idx, 2)][NewMvContext] (§8.3.2);
    // NewMvContext == 0. For the no-neighbour block the §7.10 MV stack yields the zero
    // global candidate, so RefMvIdx selects a zero-MV stack entry regardless of the
    // decoded index.
    if single_mode == SINGLE_MODE_NEARMV || single_mode == SINGLE_MODE_NEWMV {
        read_drl_idx(cdfs, symbols, max_drl_bits_minus_1, tile_offset)?;
    }

    // §5.20.7.6 MvPrecision derivation: the verified subset disables flexible MV
    // resolution and adaptive MVD (enable_flex_mvres == 0, enable_adaptive_mvd == 0),
    // so MvPrecision == FrameMvPrecision (EighthPel here) and no precision symbol is
    // read. The inter header gate rejects any enable_flex_mvres frame.

    // §5.20.7.6 / §5.20.7.20 assign_mv: GLOBALMV (identity global motion) and the
    // no-neighbour NEARMV both give the zero block MV (PredMvs[0] == GlobalMvs[0] ==
    // the zero global candidate, no MV delta read). NEWMV reads the SHELL-coded MV
    // delta over that zero predictor.
    let mv = if single_mode == SINGLE_MODE_NEWMV {
        read_newmv_block_mv(cdfs, symbols, tile_offset)?
    } else {
        Mv::ZERO
    };

    // §5.20.7.17 read_refinemv / §5.20.7.16 read_compound_type: single-reference
    // (isCompound == 0) with inter_intra == 0 reads no symbol.

    // §5.20.7.6 interp_filter: when the frame interpolation_filter is SWITCHABLE and
    // needs_interp_filter() is 1, read the per-block `interp_filter` symbol
    // (TileInterpFilterCdf[ctx], §8.3.2 ctx == 3 for the no-neighbour single-ref
    // block). needs_interp_filter() is 0 only for a large (>= 8x8) GLOBALMV /
    // GLOBAL_GLOBALMV block; NEARMV / NEWMV always need it. A fixed frame filter
    // supplies the filter with no block symbol.
    let interp = resolve_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        single_mode,
        tile_offset,
    )?;

    // §5.20.7 decode_block: after mode_info, read_block_tx_size() reads no symbol
    // under TX_MODE_LARGEST (read_tx_size returns 0 -> TxSize = maxRectTxSize =
    // TX_64X64 for the 64x64 block). Then for skip == 0 residual() reads the
    // §5.20.7.27 coefficients per plane (Y, U, V). skip == 1 reads none.
    let residual = if skip == 0 {
        // A skip == 0 block reads the §5.20.7.27 residual, whose transform-type /
        // coefficient path the verified subset only models for the DCT-only,
        // no-IST / no-DDT / no-CCTX / no-FSC / no-IDTX-intra case. Reject otherwise
        // (a skip == 1 block never reaches here, so a sub-pel skip == 1 fixture with
        // those tools enabled still decodes).
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

    Ok(InterBlock {
        mv,
        interp,
        residual,
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
/// no `sec_tx_type` symbol is read either. The chroma TX_32X32 transform also
/// falls in the DCT-only set, so this reuses the intra DCT_DCT coefficient loop
/// with `is_inter == true` (the only inter-specific contexts are the § 8.3.2
/// `TileTxbSkipCdf[is_inter]` bank and the `eobCtx = is_inter` luma EOB context,
/// both threaded through `decode_general_intra_plane_coeffs`).
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

/// AV2 § 5.20.7.8 `read_drl_idx(0, m)` for the verified no-neighbour single-reference
/// NEARMV block: reads `drl_mode` symbols from `TileDrlModeCdf[Min(idx, 2)][0]`
/// (NewMvContext == 0) until one decodes to 0 or `idx` reaches `m`. The decoded
/// `RefMvIdx` selects an MV-stack entry; for the no-neighbour block every candidate
/// is the zero global MV, so the resulting MV is (0, 0) regardless of the index. The
/// symbol reads still matter: they advance the § 8.2 arithmetic decoder and are
/// validated bit-exactly by the caller's `exit_symbol()`.
fn read_drl_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<()> {
    let m = max_drl_bits_minus_1.saturating_add(1) as usize;
    for idx in 0..m {
        let bank = idx.min(2);
        let drl_mode = cdfs
            .read_block_symbol_trace(TileCdfSelector::DrlMode { idx: bank, ctx: 0 }, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?;
        if drl_mode.get() == 0 {
            break;
        }
    }
    Ok(())
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
