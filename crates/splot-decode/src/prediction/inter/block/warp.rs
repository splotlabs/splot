// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 5.20.7.13/14 warp motion syntax and the § 7.13.3.23/24 model
//! derivations: is_warp/warp_mv/extend/local reads, the shared
//! DRL+precision+MVD tail, least-squares and extension estimation, and the
//! warp-delta parameter reads.

use super::super::find_mv_stack::reduce_warp_model;
#[allow(clippy::wildcard_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_warp_inter_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    allow_warpmv_mode: bool,
    force_integer_mv: bool,
    n4w: usize,
    n4h: usize,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<Option<WarpInterMode>> {
    if !allow_warpmv_mode || n4w < 2 || n4h < 2 {
        return Ok(None);
    }
    let is_warp = cdfs
        .read_block_symbol_trace(TileCdfSelector::IsWarp { ctx }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    if is_warp.get() == 0 {
        return Ok(None);
    }
    if force_integer_mv {
        return Ok(Some(WarpInterMode::Warpmv));
    }
    let warp_mv = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpMv, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?;
    Ok(Some(if warp_mv.get() == 0 {
        WarpInterMode::WarpNewmv
    } else {
        WarpInterMode::Warpmv
    }))
}

pub(crate) fn read_warp_newmv_motion_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    warp_sample_found: bool,
    tile_offset: ByteOffset,
) -> Result<MotionMode> {
    let frame_modes = core
        .inter
        .as_ref()
        .and_then(|inter| inter.frame_enabled_motion_modes)
        .unwrap_or([false; splot_core::headers::frame::MOTION_MODES]);
    let mut read_flag = |selector: TileCdfSelector| -> Result<bool> {
        let flag = cdfs
            .read_block_symbol_trace(selector, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?;
        Ok(flag.get() != 0)
    };
    if warp_sample_found
        && frame_modes[splot_core::headers::frame::EXTENDWARP]
        && read_flag(TileCdfSelector::UseExtendWarp {
            ctx: neighbour_ctx.use_extend_warp_ctx(),
        })?
    {
        return Ok(MotionMode::ExtendWarp);
    }
    if warp_sample_found
        && frame_modes[splot_core::headers::frame::LOCALWARP]
        && read_flag(TileCdfSelector::UseLocalWarp {
            ctx: neighbour_ctx.use_local_warp_ctx(),
        })?
    {
        return Ok(MotionMode::LocalWarp);
    }
    Ok(MotionMode::DeltaWarp)
}

fn warp_round2(value: i64, n: u32, tile_offset: ByteOffset) -> Result<i32> {
    let rounded = if n == 0 {
        value
    } else {
        (value + (1i64 << (n - 1))) >> n
    };
    i32::try_from(rounded).map_err(|_| warp_model_error(tile_offset))
}

const LS_MV_MAX: i32 = 256;

const fn ls_product(a: i32, b: i32) -> i32 {
    ((a * b) >> 2) + (a + b)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn local_warp_estimation(
    samples: &[[i32; 4]],
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<[i32; 6]> {
    let geometry_error = || warp_model_error(tile_offset);
    let mid_y = i32::try_from(
        n4h.checked_mul(2)
            .and_then(|half| mi_row.checked_mul(4)?.checked_add(half))
            .ok_or_else(geometry_error)?,
    )
    .map_err(|_| geometry_error())?
        - 1;
    let mid_x = i32::try_from(
        n4w.checked_mul(2)
            .and_then(|half| mi_col.checked_mul(4)?.checked_add(half))
            .ok_or_else(geometry_error)?,
    )
    .map_err(|_| geometry_error())?
        - 1;
    let suy = mid_y * 8;
    let sux = mid_x * 8;
    let duy = suy + mv.row;
    let dux = sux + mv.col;
    let mut a = [[0i32; 2]; 2];
    let mut bx = [0i32; 2];
    let mut by = [0i32; 2];
    for sample in samples {
        let sy = sample[0] - suy;
        let sx = sample[1] - sux;
        let dy = sample[2] - duy;
        let dx = sample[3] - dux;
        if (sx - dx).abs() < LS_MV_MAX && (sy - dy).abs() < LS_MV_MAX {
            a[0][0] += ls_product(sx, sx) + 8;
            a[0][1] += ls_product(sx, sy) + 4;
            a[1][1] += ls_product(sy, sy) + 8;
            bx[0] += ls_product(sx, dx) + 8;
            bx[1] += ls_product(sy, dx) + 4;
            by[0] += ls_product(sx, dy) + 4;
            by[1] += ls_product(sy, dy) + 8;
        }
    }
    let det = i64::from(a[0][0]) * i64::from(a[1][1]) - i64::from(a[0][1]) * i64::from(a[0][1]);
    let mut params = IDENTITY_WARP_PARAMS;
    if det == 0 {
        set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
        return Ok(params);
    }
    let (raw_shift, factor) =
        splot_recon::resolve_divisor(det.unsigned_abs()).map_err(|_| geometry_error())?;
    let div_factor = if det < 0 {
        -i32::from(factor)
    } else {
        i32::from(factor)
    };
    let mut div_shift = i32::from(raw_shift) - WARPEDMODEL_PREC_BITS as i32;
    let mut div_factor = div_factor;
    if div_shift < 0 {
        div_factor = div_factor
            .checked_shl((-div_shift) as u32)
            .ok_or_else(geometry_error)?;
        div_shift = 0;
    }
    let shift = u32::try_from(div_shift).map_err(|_| geometry_error())?;
    let diag = |v: i64| -> i32 {
        let product = i128::from(v) * i128::from(div_factor);
        let magnitude = product.unsigned_abs();
        let rounded = if shift == 0 {
            magnitude
        } else {
            (magnitude + (1u128 << (shift - 1))) >> shift
        };
        let signed = if product < 0 {
            -i128::try_from(rounded).unwrap_or(i128::MAX)
        } else {
            i128::try_from(rounded).unwrap_or(i128::MAX)
        };
        signed.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32
    };
    params[2] = diag(i64::from(a[1][1]) * i64::from(bx[0]) - i64::from(a[0][1]) * i64::from(bx[1]));
    params[3] =
        diag(-i64::from(a[0][1]) * i64::from(bx[0]) + i64::from(a[0][0]) * i64::from(bx[1]));
    params[4] = diag(i64::from(a[1][1]) * i64::from(by[0]) - i64::from(a[0][1]) * i64::from(by[1]));
    params[5] =
        diag(-i64::from(a[0][1]) * i64::from(by[0]) + i64::from(a[0][0]) * i64::from(by[1]));
    reduce_warp_model(&mut params);
    set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(params)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn extend_warp_estimation(
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    extend_delta: Option<(i32, i32)>,
    stack: &super::super::find_mv_stack::MvStack,
    ref_mv_idx: usize,
    mv: Mv,
    tile_offset: ByteOffset,
) -> Result<[i32; 6]> {
    let (mi_row, mi_col, n4w, n4h) = (
        block_ctx.mi_row,
        block_ctx.mi_col,
        block_ctx.bw4,
        block_ctx.bh4,
    );
    let Some((delta_row, delta_col)) = extend_warp_base_position(
        mv_grid,
        block_ctx,
        stack.candidate_offsets(ref_mv_idx),
        extend_delta,
    ) else {
        return Err(inter_cap!(
            "inter_warp_extend_base_missing",
            tile_offset,
            "inter.warp_extend.base_position",
            "7.13.3.24"
        ));
    };
    let Some(params) = super::super::find_mv_stack::extend_warp_neighbour_params(
        mv_grid, block_ctx, delta_row, delta_col,
    ) else {
        return Err(inter_cap!(
            "inter_warp_extend_neighbour_missing",
            tile_offset,
            "inter.warp_extend.base_position",
            "7.13.3.24"
        ));
    };
    let geometry_error = || warp_model_error(tile_offset);
    let mid_y = i32::try_from(
        n4h.checked_mul(2)
            .and_then(|half| mi_row.checked_mul(4)?.checked_add(half))
            .ok_or_else(geometry_error)?,
    )
    .map_err(|_| geometry_error())?
        - 1;
    let mid_x = i32::try_from(
        n4w.checked_mul(2)
            .and_then(|half| mi_col.checked_mul(4)?.checked_add(half))
            .ok_or_else(geometry_error)?,
    )
    .map_err(|_| geometry_error())?
        - 1;
    let proj_mid_x = (i64::from(mid_x) << WARPEDMODEL_PREC_BITS)
        + (i64::from(mv.col) << (WARPEDMODEL_PREC_BITS - 3));
    let proj_mid_y = (i64::from(mid_y) << WARPEDMODEL_PREC_BITS)
        + (i64::from(mv.row) << (WARPEDMODEL_PREC_BITS - 3));
    let mut extended = IDENTITY_WARP_PARAMS;
    extended[0] = 0;
    extended[1] = 0;
    let neighbour_is_above = delta_row == -1 && delta_col >= 0;
    if neighbour_is_above {
        extended[2] = params[2];
        extended[4] = params[4];
        let above_x = mid_x;
        let above_y = i32::try_from(mi_row.checked_mul(4).ok_or_else(geometry_error)?)
            .map_err(|_| geometry_error())?
            - 1;
        let proj_above_x = i64::from(params[2]) * i64::from(above_x)
            + i64::from(params[3]) * i64::from(above_y)
            + i64::from(params[0]);
        let proj_above_y = i64::from(params[4]) * i64::from(above_x)
            + i64::from(params[5]) * i64::from(above_y)
            + i64::from(params[1]);
        let shift = n4h.trailing_zeros() + MI_SIZE_LOG2 - 1;
        extended[3] = warp_round2(proj_mid_x - proj_above_x, shift, tile_offset)?;
        extended[5] = warp_round2(proj_mid_y - proj_above_y, shift, tile_offset)?;
    } else {
        extended[3] = params[3];
        extended[5] = params[5];
        let left_x = i32::try_from(mi_col.checked_mul(4).ok_or_else(geometry_error)?)
            .map_err(|_| geometry_error())?
            - 1;
        let left_y = mid_y;
        let proj_left_x = i64::from(params[2]) * i64::from(left_x)
            + i64::from(params[3]) * i64::from(left_y)
            + i64::from(params[0]);
        let proj_left_y = i64::from(params[4]) * i64::from(left_x)
            + i64::from(params[5]) * i64::from(left_y)
            + i64::from(params[1]);
        let shift = n4w.trailing_zeros() + MI_SIZE_LOG2 - 1;
        extended[2] = warp_round2(proj_mid_x - proj_left_x, shift, tile_offset)?;
        extended[4] = warp_round2(proj_mid_y - proj_left_y, shift, tile_offset)?;
    }
    reduce_warp_model(&mut extended);
    set_warp_translation(&mut extended, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(extended)
}

/// AVM `get_extend_base_pos` rejects TIP spatial bases, although that guard is
/// absent from the mirrored AV2 § 7.13.3.24 pseudocode.
pub(super) fn extend_warp_base_position(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    candidate: (i32, i32),
    fallback: Option<(i32, i32)>,
) -> Option<(i32, i32)> {
    let (row, col) = candidate;
    if (row == -1 || col == -1)
        && grid.is_non_tip_at(block.mi_row as i32 + row, block.mi_col as i32 + col)
    {
        return Some(candidate);
    }
    fallback
}

/// § 5.20.7.13 warp syntax for one leaf: the model source plus the indices,
/// precision and MVD the § 7.13.3 derivations consume.
pub(super) struct ParsedWarpSyntax {
    pub(super) source: WarpModelSource,
    pub(super) ref_mv_idx: usize,
    pub(super) ref_warp_idx: usize,
    pub(super) mvd: Option<Mv>,
    pub(super) precision: BlockPrecisionRecord,
}

/// § 5.20.7.13 DRL index, per-block precision and MVD read by both WARP_NEWMV
/// forms once the warp reference index (if any) is known.
struct WarpNewmvTail {
    ref_mv_idx: usize,
    precision: BlockPrecisionRecord,
    mvd: Mv,
}

#[allow(clippy::too_many_arguments)]
fn read_warp_newmv_tail(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    mv_config: MvReadConfig,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<WarpNewmvTail> {
    let ref_mv_idx = read_drl_idx(
        cdfs,
        symbols,
        new_mv_context,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let precision = read_block_mv_precision_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        neighbour_ctx,
        mv_config.precision(),
        true,
        false,
        tile_offset,
    )?;
    let block_config = MvReadConfig::inter(precision.mv_precision);
    let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, block_config)?;
    let mvd = apply_inter_mvd_signs(magnitude, symbols, tile_offset, block_config, false, 1)?;
    Ok(WarpNewmvTail {
        ref_mv_idx,
        precision,
        mvd,
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WarpInterIntraSyntax {
    pub(crate) enabled: bool,
    pub(crate) mode: Option<u8>,
    pub(crate) use_wedge: bool,
    pub(crate) wedge_index: Option<u8>,
}

pub(crate) fn inter_mv_read_config(
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<MvReadConfig> {
    let precision = core
        .inter
        .as_ref()
        .and_then(|inter| inter.mv_precision)
        .ok_or_else(|| {
            inter_missing!(
                "inter_mv_precision",
                tile_offset,
                "inter.mv_precision",
                SPEC_MODE_INFO
            )
        })?;
    let precision = mv_precision_code(precision).ok_or_else(|| {
        inter_cap!(
            "inter_mv_precision_unsupported",
            tile_offset,
            "inter.mv_precision unsupported",
            SPEC_MODE_INFO
        )
    })?;
    Ok(MvReadConfig::inter(precision))
}

pub(crate) const fn mv_precision_code(precision: MvPrecision) -> Option<u8> {
    Some(match precision {
        MvPrecision::OnePel => MV_PRECISION_ONE_PEL,
        MvPrecision::HalfPel => MV_PRECISION_HALF_PEL,
        MvPrecision::QuarterPel => MV_PRECISION_QUARTER_PEL,
        MvPrecision::EighthPel => MV_PRECISION_EIGHTH_PEL,
        _ => return None,
    })
}

pub(crate) fn inter_mvd_sign_derivation_allowed(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    single_mode: u8,
    use_amvd: bool,
    frame_config: MvReadConfig,
    config: MvReadConfig,
) -> bool {
    if single_mode != SINGLE_MODE_NEWMV || use_amvd {
        return false;
    }
    if !sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_mvd_sign_derive)
    {
        return false;
    }
    if effective_allow_screen_content_tools(core) {
        return false;
    }
    frame_config.precision() <= MV_PRECISION_QUARTER_PEL
        && config.precision() < MV_PRECISION_QUARTER_PEL
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_warp_extend_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    mv_config: MvReadConfig,
    motion_mode: MotionMode,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpSyntax> {
    let tail = read_warp_newmv_tail(
        cdfs,
        symbols,
        sequence,
        core,
        neighbour_ctx,
        mv_config,
        new_mv_context,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    Ok(ParsedWarpSyntax {
        source: if motion_mode == MotionMode::LocalWarp {
            WarpModelSource::LocalSamples
        } else {
            WarpModelSource::Extended
        },
        ref_mv_idx: tail.ref_mv_idx,
        ref_warp_idx: 0,
        mvd: Some(tail.mvd),
        precision: tail.precision,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_warp_newmv_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    mv_config: MvReadConfig,
    b_size: usize,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpSyntax> {
    let ref_warp_idx = read_warp_ref_idx(cdfs, symbols, MAX_WARP_REF_CANDIDATES, tile_offset)?;
    let tail = read_warp_newmv_tail(
        cdfs,
        symbols,
        sequence,
        core,
        neighbour_ctx,
        mv_config,
        new_mv_context,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let delta = read_warp_delta_syntax(cdfs, symbols, sequence, b_size, ref_warp_idx, tile_offset)?;
    Ok(ParsedWarpSyntax {
        source: WarpModelSource::Delta(delta),
        ref_mv_idx: tail.ref_mv_idx,
        ref_warp_idx,
        mvd: Some(tail.mvd),
        precision: tail.precision,
    })
}

pub(crate) fn read_warpmv_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    mv_config: MvReadConfig,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpSyntax> {
    let ref_warp_idx = read_warp_ref_idx(cdfs, symbols, MAX_WARP_REF_CANDIDATES, tile_offset)?;
    let warpmv_with_mvd = if ref_warp_idx < 2 {
        read_warpmv_with_mvd_flag(cdfs, symbols, tile_offset)?
    } else {
        false
    };
    let mvd = if warpmv_with_mvd {
        let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, mv_config)?;
        Some(apply_inter_mvd_signs(
            magnitude,
            symbols,
            tile_offset,
            mv_config,
            false,
            1,
        )?)
    } else {
        None
    };
    Ok(ParsedWarpSyntax {
        source: WarpModelSource::Candidate,
        ref_mv_idx: 0,
        ref_warp_idx,
        mvd,
        precision: BlockPrecisionRecord::most_probable(mv_config.precision()),
    })
}

fn read_warpmv_with_mvd_flag(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let flag = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpWithMvd, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    Ok(flag != 0)
}

fn read_warp_ref_idx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    max_num_warp_candidates: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    if max_num_warp_candidates <= 1 {
        return Ok(0);
    }
    let mut ref_warp_idx = 0usize;
    for bit_idx in 0..max_num_warp_candidates.saturating_sub(1) {
        let warp_idx = cdfs
            .read_block_symbol_trace(TileCdfSelector::WarpIdx { ctx: bit_idx }, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        ref_warp_idx = bit_idx + usize::from(warp_idx);
        if warp_idx == 0 {
            break;
        }
    }
    Ok(ref_warp_idx)
}

const WEDGE_ANGLE_DIST_TO_INDEX: [[i8; 4]; 20] = [
    [-1, 0, 1, 2],
    [3, 4, 5, 6],
    [7, 8, 9, 10],
    [11, 12, 13, 14],
    [15, 16, 17, 18],
    [-1, 19, 20, 21],
    [22, 23, 24, 25],
    [26, 27, 28, 29],
    [30, 31, 32, 33],
    [34, 35, 36, 37],
    [-1, 38, 39, 40],
    [-1, 41, 42, 43],
    [-1, 44, 45, 46],
    [-1, 47, 48, 49],
    [-1, 50, 51, 52],
    [-1, 53, 54, 55],
    [-1, 56, 57, 58],
    [-1, 59, 60, 61],
    [-1, 62, 63, 64],
    [-1, 65, 66, 67],
];

pub(crate) fn interintra_prediction_mode(
    syntax: WarpInterIntraSyntax,
    tile_offset: ByteOffset,
) -> Result<Option<InterIntraPrediction>> {
    if !syntax.enabled {
        return Ok(None);
    }
    let mode = match syntax.mode {
        Some(0) => InterIntraMode::Dc,
        Some(1) => InterIntraMode::Vertical,
        Some(2) => InterIntraMode::Horizontal,
        Some(3) => InterIntraMode::Smooth,
        _ => Err(inter_cap!(
            "inter_interintra_mode_missing",
            tile_offset,
            "inter.interintra.mode",
            "5.20.7.15"
        ))?,
    };
    Ok(Some(if syntax.use_wedge {
        InterIntraPrediction::WedgeMask {
            mode,
            wedge_index: syntax.wedge_index.ok_or_else(|| {
                inter_cap!(
                    "inter_wedge_interintra_index_missing",
                    tile_offset,
                    "inter.interintra.wedge_index",
                    "5.20.7.15"
                )
            })?,
        }
    } else {
        InterIntraPrediction::SmoothMask { mode }
    }))
}

/// Reads the AV2 section 5.20.7.15 inter-intra tail shared by the plain and warp
/// readers: the inter-intra mode, the wedge flag, and the wedge index. Each caller
/// keeps its own eligibility test and enable symbol and delegates once the block is
/// known to be inter-intra coded.
pub(super) fn read_active_inter_intra_tail(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    bsize_group: usize,
    b_size: usize,
    tile_offset: ByteOffset,
) -> Result<WarpInterIntraSyntax> {
    let mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::InterIntraMode { bsize_group }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();

    let use_wedge = if WEDGE_USED_BY_BSIZE.get(b_size).copied().unwrap_or(false) {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::WedgeInterIntra, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        symbol != 0
    } else {
        false
    };
    let wedge_index = if use_wedge {
        Some(read_wedge_mode_syntax(cdfs, symbols, tile_offset)?)
    } else {
        None
    };

    Ok(WarpInterIntraSyntax {
        enabled: true,
        mode: Some(mode),
        use_wedge,
        wedge_index,
    })
}

pub(crate) fn read_warp_inter_intra_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    b_size: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<WarpInterIntraSyntax> {
    if n4w < 2 || n4h < 2 || n4w.max(n4h) > CHUNK_64_N4 {
        return Ok(WarpInterIntraSyntax::default());
    }
    let bsize_group = *SIZE_GROUP_LOOKUP.get(b_size).ok_or_else(|| {
        inter_cap!(
            "inter_warp_interintra_bsize_group",
            tile_offset,
            "inter.warp_inter_intra block size out of range",
            SPEC_MODE_INFO
        )
    })?;
    let enabled = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpInterIntra { bsize_group }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if enabled == 0 {
        return Ok(WarpInterIntraSyntax::default());
    }

    read_active_inter_intra_tail(cdfs, symbols, bsize_group, b_size, tile_offset)
}

pub(crate) fn read_wedge_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<u8> {
    let quad = cdfs
        .read_block_symbol_trace(TileCdfSelector::WedgeQuad, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    let angle_in_quad = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::WedgeAngle {
                quad: usize::from(quad),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    let angle = quad
        .checked_mul(QUAD_WEDGE_ANGLES)
        .and_then(|base| base.checked_add(angle_in_quad))
        .ok_or_else(|| warp_model_error(tile_offset))?;
    let use_dist2 = angle >= H_WEDGE_ANGLES || matches!(angle, WEDGE_90 | WEDGE_0);
    let dist = if use_dist2 {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::WedgeDist2, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        symbol + 1
    } else {
        cdfs.read_block_symbol_trace(TileCdfSelector::WedgeDist1, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get()
    };
    let index = WEDGE_ANGLE_DIST_TO_INDEX
        .get(usize::from(angle))
        .and_then(|row| row.get(usize::from(dist)))
        .copied()
        .filter(|&index| index >= 0)
        .ok_or_else(|| {
            inter_cap!(
                "inter_wedge_mode_index",
                tile_offset,
                "inter.wedge_angle_dist index out of range",
                SPEC_MODE_INFO
            )
        })?;
    u8::try_from(index).map_err(|_| warp_model_error(tile_offset))
}

fn read_warp_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    b_size: usize,
    ref_warp_idx: usize,
    tile_offset: ByteOffset,
) -> Result<WarpDeltaSyntax> {
    let six_param = sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_six_param_warp_delta)
        && ref_warp_idx == 1;
    if !six_param && ref_warp_idx != 0 {
        return Ok(WarpDeltaSyntax {
            deltas: None,
            six_param,
        });
    }
    let precision_idx = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::WarpPrecision { block_size: b_size },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    let high = precision_idx != 0;
    let mut deltas = [0i32; 4];
    deltas[0] = read_warp_delta_param(cdfs, symbols, WarpDeltaParam::Two, high, tile_offset)?;
    deltas[1] = read_warp_delta_param(cdfs, symbols, WarpDeltaParam::Three, high, tile_offset)?;
    if six_param {
        deltas[2] = read_warp_delta_param(cdfs, symbols, WarpDeltaParam::Four, high, tile_offset)?;
        deltas[3] = read_warp_delta_param(cdfs, symbols, WarpDeltaParam::Five, high, tile_offset)?;
    }
    Ok(WarpDeltaSyntax {
        deltas: Some(deltas),
        six_param,
    })
}

/// AV2 § 7.13.3.25: the parsed warp deltas applied to the § 7.12 warp
/// candidate, reduced and re-centred on the block's motion vector.
pub(super) fn apply_warp_delta(
    mut params: [i32; 6],
    delta: WarpDeltaSyntax,
    mv: Mv,
    block_ctx: &MvBlockContext,
    tile_offset: ByteOffset,
) -> Result<[i32; 6]> {
    if let Some(deltas) = delta.deltas {
        params[0] = 0;
        params[1] = 0;
        params[2] += deltas[0];
        params[3] += deltas[1];
        if delta.six_param {
            params[4] += deltas[2];
            params[5] += deltas[3];
        } else {
            params[4] = -params[3];
            params[5] = params[2];
        }
    }
    reduce_warp_model(&mut params);
    set_warp_translation(
        &mut params,
        mv,
        block_ctx.mi_row,
        block_ctx.mi_col,
        block_ctx.bw4,
        block_ctx.bh4,
        tile_offset,
    )?;
    Ok(params)
}

/// The four AV2 section 5.20.5.9 `warp_delta_param` positions. Their CDF context is
/// shared pairwise, so the enum keeps the index-to-context mapping total.
#[derive(Clone, Copy)]
enum WarpDeltaParam {
    Two,
    Three,
    Four,
    Five,
}

impl WarpDeltaParam {
    const fn index_type(self) -> usize {
        match self {
            Self::Two | Self::Five => 0,
            Self::Three | Self::Four => 1,
        }
    }
}

fn read_warp_delta_param(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    param: WarpDeltaParam,
    high_precision: bool,
    tile_offset: ByteOffset,
) -> Result<i32> {
    let index_type = param.index_type();
    let mut value = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamLow { index_type }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if high_precision && value == WARP_DELTA_NUM_SYMBOLS_LOW - 1 {
        let high = cdfs
            .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamHigh { index_type }, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        value = value
            .checked_add(high)
            .ok_or_else(|| warp_model_error(tile_offset))?;
    }
    let mut signed = i32::from(value);
    if signed != 0 {
        let sign = cdfs
            .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamSign, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if sign != 0 {
            signed = -signed;
        }
    }
    let step_bits = WARP_DELTA_STEP_BITS + 1 - u32::from(high_precision);
    signed
        .checked_shl(step_bits)
        .ok_or_else(|| warp_model_error(tile_offset))
}

pub(super) fn set_warp_translation(
    params: &mut [i32; 6],
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    let center_x = mi_col
        .checked_mul(MI_SIZE)
        .and_then(|value| value.checked_add(n4w.checked_mul(2)?))
        .and_then(|value| value.checked_sub(1))
        .ok_or_else(|| warp_model_error(tile_offset))?;
    let center_y = mi_row
        .checked_mul(MI_SIZE)
        .and_then(|value| value.checked_add(n4h.checked_mul(2)?))
        .and_then(|value| value.checked_sub(1))
        .ok_or_else(|| warp_model_error(tile_offset))?;
    let one = 1i128 << WARPEDMODEL_PREC_BITS;
    let mv_scale = 1i128 << (WARPEDMODEL_PREC_BITS - 3);
    let wmmat0 = i128::from(mv.col) * mv_scale
        - (center_x as i128 * (i128::from(params[2]) - one)
            + center_y as i128 * i128::from(params[3]));
    let wmmat1 = i128::from(mv.row) * mv_scale
        - (center_x as i128 * i128::from(params[4])
            + center_y as i128 * (i128::from(params[5]) - one));
    let high = WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS);
    params[0] = clamp_i128_to_i32(wmmat0, -WARPEDMODEL_TRANS_CLAMP, high);
    params[1] = clamp_i128_to_i32(wmmat1, -WARPEDMODEL_TRANS_CLAMP, high);
    Ok(())
}

fn clamp_i128_to_i32(value: i128, low: i32, high: i32) -> i32 {
    value.clamp(i128::from(low), i128::from(high)) as i32
}

fn warp_model_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_cap!(
        "inter_warp_model_overflow",
        tile_offset,
        "inter.warp_model arithmetic overflow",
        SPEC_MODE_INFO
    )
}
