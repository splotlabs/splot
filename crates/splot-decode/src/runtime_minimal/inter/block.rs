// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal inter `mode_info` decode (AV2 § 5.20.7.6) + § 7.11 zero-MV derivation.
//!
//! Walks the § 5.20.3 partition tree to the single 64x64 NONE block, reads the
//! verified inter `mode_info` symbol sequence (`is_inter` / `skip` / `single_mode`)
//! from the tile arithmetic stream, and confirms § 8.2.4 `exit_symbol()`. The MV
//! result is the § 7.11 zero vector for the `GLOBALMV` (identity global motion) case.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::super::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};
use super::{Mv, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV, SPEC_MODE_INFO, unsupported_at};
use crate::tile_payload::{
    DecodeBlockFrontier, DecodeTileWorkUnit, GeneralIntraMultiblockError,
    GeneralIntraTreeWalkError, TileCdfSelector, TileCdfSubset, TilePartitionTraversalError,
    decode_general_intra_multiblock_tree,
};

/// Decodes the single inter block's § 5.20.7.6 `mode_info` and returns its § 7.11
/// motion vector. Runs the real § 5.20.3 partition walk + § 8.2 symbol reads over
/// the tile payload and validates § 8.2.4 `exit_symbol()`.
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_inter_block_and_mv(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: DecodeOptions,
) -> Result<Mv> {
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

    // A single MV captured from the one decoded inter block. The closure runs
    // exactly once for the single 64x64 NONE block; the partition walk records the
    // (intra) joint-mode grid for that block, which inter blocks set to DC_PRED.
    let mut decoded_mv: Option<Mv> = None;
    let limits = options.limits();
    let symbols = decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit, symbols, frontier, _joint_modes, _block_decoded| {
            let mv = decode_one_inter_block(
                work_unit,
                symbols,
                frontier,
                max_drl_bits_minus_1,
                tile_offset,
            )?;
            decoded_mv = Some(mv);
            // AV2 § 5.20.5.3 IntraJointMode for an inter block is DC_PRED (= 0); the
            // partition walk records this grid value but inter neighbours ignore it.
            Ok(0u8)
        },
    )
    .map_err(|error| map_inter_multiblock_error(error, tile_offset))?;

    // §8.2.4 exit_symbol(): the decoded block must consume the whole tile payload
    // (skip == 1 -> no residual). A failure means the symbol reads were not
    // bit-exact, so reject rather than emit wrong output.
    symbols.exit_symbol().map_err(|_| {
        unsupported_at(
            "inter_exit_symbol",
            tile_offset,
            "minimal inter tile payload did not satisfy §8.2.4 exit_symbol() after the decoded inter block",
            SPEC_MODE_INFO,
        )
    })?;

    decoded_mv.ok_or_else(|| {
        unsupported_at(
            "inter_no_decoded_block",
            tile_offset,
            "minimal inter decode expected exactly one decoded inter block",
            SPEC_MODE_INFO,
        )
    })
}

/// Decodes one inter leaf block's § 5.20.7.6 `mode_info` for the verified subset and
/// returns its § 7.11 motion vector. Reads, in § 5.20 order:
/// 1. `is_inter` (§ 5.20.7.3) — `TileIsInterCdf[ctx]`, must decode to 1.
/// 2. `skip` (§ 5.20.5.10) — `TileSkipCdf[ctx]`, must decode to 1 (no residual).
/// 3. `single_mode` (§ 5.20.7.6) — `TileSingleModeCdf[NewMvContext]`, must decode to
///    GLOBALMV (`single_mode == 1`).
///
/// `read_skip_mode` reads no symbol (`skip_mode_present == 0` for this fixture),
/// `read_ref_frames` reads no `single_ref` symbol (`NumTotalRefs == 1`),
/// `read_motion_mode` reads no symbol (no enabled motion modes -> SIMPLE), no DRL is
/// read for GLOBALMV, and `assign_mv` reads no MV delta (GLOBALMV). Any deviation
/// from this exact subset desynchronises the § 8.2 arithmetic decoder and fails the
/// caller's `exit_symbol()` check.
fn decode_one_inter_block(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &DecodeBlockFrontier,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<Mv> {
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
    // ctx == 0. Must decode to skip == 1 (no residual coefficients to read).
    let skip = cdfs
        .read_block_symbol_trace(TileCdfSelector::Skip { ctx: 0 }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    if skip.get() != 1 {
        return Err(unsupported_at(
            "inter_block_not_skip",
            tile_offset,
            "minimal inter decode only supports skip == 1 blocks (no residual); a coded-residual inter block is not yet implemented",
            SPEC_MODE_INFO,
        ));
    }

    // §5.20.7.6 single_mode: TileSingleModeCdf[NewMvContext]. For the no-neighbour
    // block NewMvContext == 0 (§7.11.2: no inter neighbours found, so nearestMatch ==
    // 0 and NewMvCount == 0). YMode = NEARMV + single_mode. The verified subset is the
    // zero-MV NEARMV (single_mode == 0) or GLOBALMV (single_mode == 1) mode; NEWMV
    // (single_mode == 2) reads an MV delta and is not yet supported.
    let single_mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::SingleMode { ctx: 0 }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    let single_mode = single_mode.get();
    if single_mode != SINGLE_MODE_NEARMV && single_mode != SINGLE_MODE_GLOBALMV {
        return Err(unsupported_at(
            "inter_block_unsupported_single_mode",
            tile_offset,
            "minimal inter decode only supports the single-reference zero-MV NEARMV (single_mode == 0) or GLOBALMV (single_mode == 1) mode; NEWMV (single_mode == 2) reads an MV delta and is not yet implemented",
            SPEC_MODE_INFO,
        ));
    }

    // §5.20.7.6 read_motion_mode: the verified subset disables every motion mode
    // (frame_enabled_motion_modes all false), so read_motion_mode returns SIMPLE with
    // no symbol read. (Handled implicitly: no symbol consumed.)

    // §5.20.7.6 / §5.20.7.8 DRL: GLOBALMV reads no DRL; NEARMV (has_nearmv == true)
    // reads `read_drl_idx(0, m)` where `m = max_drl_bits_minus_1 + 1`. The drl_mode
    // CDF is TileDrlModeCdf[Min(idx, 2)][NewMvContext] (§8.3.2); NewMvContext == 0.
    // For the no-neighbour block the §7.10 MV stack yields the zero global candidate,
    // so RefMvIdx selects a zero-MV stack entry regardless of the decoded index.
    if single_mode == SINGLE_MODE_NEARMV {
        read_drl_idx(cdfs, symbols, max_drl_bits_minus_1, tile_offset)?;
    }

    // §7.11 zero MV: GLOBALMV over identity global motion (use_global_motion == 0)
    // gives PredMvs[0] = GlobalMvs[0] = (0, 0); NEARMV over an empty (no-neighbour)
    // §7.10 MV stack gives PredMvs[0] = the zero global candidate. Neither mode reads
    // an MV delta in assign_mv, so BlockMvs[0] = (0, 0). The §8.2.4 exit_symbol()
    // check the caller runs proves the symbol reads (and hence this zero-MV result)
    // were bit-exact.
    Ok(Mv::ZERO)
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
