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

/// § 5.20.7.14 `read_motion_mode` WARP_NEWMV branch: the `use_extend_warp`
/// and `use_local_warp` reads, gated on § 7.11.4 `WarpSampleFound[ 0 ]` and
/// the frame-enabled motion modes. LOCALWARP prediction is beyond the
/// frontier, so that flag defers; otherwise the selected mode is returned.
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

/// § 4.8 `Round2` over a signed § 7.13.3.24 projection difference: the
/// spec's integer form `(x + (1 << (n - 1))) >> n` with an arithmetic shift
/// (AVM `ROUND_POWER_OF_TWO_64`), NOT the sign-magnitude `Round2Signed`.
const fn warp_round2(value: i64, n: u32) -> i64 {
    if n == 0 {
        return value;
    }
    (value + (1i64 << (n - 1))) >> n
}

/// AV2 § 3 `LS_MV_MAX`.
const LS_MV_MAX: i64 = 256;

/// § 7.13.3.23 `ls_product`.
const fn ls_product(a: i64, b: i64) -> i64 {
    ((a * b) >> 2) + (a + b)
}

/// § 7.13.3.23 warp estimation: integer least-squares fit over the § 7.12.3
/// warp samples, falling back to a pure translation when the determinant is
/// zero.
#[allow(clippy::too_many_arguments)]
fn local_warp_estimation(
    samples: &[[i64; 4]],
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<[i64; 6]> {
    let geometry_error = || warp_model_error(tile_offset);
    let mid_y = i64::try_from(mi_row * 4 + n4h * 2).map_err(|_| geometry_error())? - 1;
    let mid_x = i64::try_from(mi_col * 4 + n4w * 2).map_err(|_| geometry_error())? - 1;
    let suy = mid_y * 8;
    let sux = mid_x * 8;
    let duy = suy + i64::from(mv.row);
    let dux = sux + i64::from(mv.col);
    let mut a = [[0i64; 2]; 2];
    let mut bx = [0i64; 2];
    let mut by = [0i64; 2];
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
    let det = i128::from(a[0][0]) * i128::from(a[1][1]) - i128::from(a[0][1]) * i128::from(a[0][1]);
    let det = i64::try_from(det).map_err(|_| geometry_error())?;
    let mut params = IDENTITY_WARP_PARAMS;
    if det == 0 {
        set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
        return Ok(params);
    }
    let (raw_shift, factor) =
        splot_recon::resolve_divisor(det.unsigned_abs()).map_err(|_| geometry_error())?;
    let div_factor = if det < 0 {
        -i64::from(factor)
    } else {
        i64::from(factor)
    };
    let mut div_shift = i64::from(raw_shift) - i64::from(WARPEDMODEL_PREC_BITS);
    let mut div_factor = div_factor;
    if div_shift < 0 {
        div_factor <<= -div_shift;
        div_shift = 0;
    }
    let shift = u32::try_from(div_shift).map_err(|_| geometry_error())?;
    let diag = |v: i64| -> i64 {
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
        i64::try_from(signed.clamp(i128::from(i32::MIN), i128::from(i32::MAX))).unwrap_or_default()
    };
    params[2] = diag(a[1][1] * bx[0] - a[0][1] * bx[1]);
    params[3] = diag(-a[0][1] * bx[0] + a[0][0] * bx[1]);
    params[4] = diag(a[1][1] * by[0] - a[0][1] * by[1]);
    params[5] = diag(-a[0][1] * by[0] + a[0][0] * by[1]);
    reduce_warp_model(&mut params);
    set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(params)
}

/// § 7.13.3.24 extend-warp estimation: extends the base neighbour's warp
/// model through the block's signalled MV. The global-motion `params` arm is
/// statically unreachable (global-motion frames defer at the frame gate).
#[allow(clippy::too_many_arguments)]
fn extend_warp_estimation(
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    mode_ctx: &ModeContext,
    stack: &super::super::find_mv_stack::MvStack,
    ref_mv_idx: usize,
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<[i64; 6]> {
    let (mut delta_row, mut delta_col) = stack.candidate_offsets(ref_mv_idx);
    if delta_row != -1 && delta_col != -1 {
        let Some((fallback_row, fallback_col)) = mode_ctx.extend_delta else {
            return Err(inter_cap!(
                "inter_warp_extend_base_missing",
                tile_offset,
                "inter.warp_extend.base_position",
                "7.13.3.24"
            ));
        };
        delta_row = fallback_row;
        delta_col = fallback_col;
    }
    let params = match super::super::find_mv_stack::extend_warp_neighbour_params(
        mv_grid, block_ctx, delta_row, delta_col,
    ) {
        super::super::find_mv_stack::ExtendWarpNeighbour::Params(params) => params,
        super::super::find_mv_stack::ExtendWarpNeighbour::List1MvUnretained => {
            return Err(inter_cap!(
                "inter_warp_extend_list1_mv_unretained",
                tile_offset,
                "inter.warp_extend.second_list_neighbour_mv",
                "7.13.3.24"
            ));
        }
        super::super::find_mv_stack::ExtendWarpNeighbour::Missing => {
            return Err(inter_cap!(
                "inter_warp_extend_neighbour_missing",
                tile_offset,
                "inter.warp_extend.base_position",
                "7.13.3.24"
            ));
        }
    };
    let geometry_error = || warp_model_error(tile_offset);
    let mid_y = i64::try_from(mi_row * 4 + n4h * 2).map_err(|_| geometry_error())? - 1;
    let mid_x = i64::try_from(mi_col * 4 + n4w * 2).map_err(|_| geometry_error())? - 1;
    let proj_mid_x =
        (mid_x << WARPEDMODEL_PREC_BITS) + (i64::from(mv.col) << (WARPEDMODEL_PREC_BITS - 3));
    let proj_mid_y =
        (mid_y << WARPEDMODEL_PREC_BITS) + (i64::from(mv.row) << (WARPEDMODEL_PREC_BITS - 3));
    let mut extended = IDENTITY_WARP_PARAMS;
    extended[0] = 0;
    extended[1] = 0;
    let neighbour_is_above = delta_row == -1 && delta_col >= 0;
    if neighbour_is_above {
        extended[2] = params[2];
        extended[4] = params[4];
        let above_x = mid_x;
        let above_y = i64::try_from(mi_row * 4).map_err(|_| geometry_error())? - 1;
        let proj_above_x = params[2] * above_x + params[3] * above_y + params[0];
        let proj_above_y = params[4] * above_x + params[5] * above_y + params[1];
        let shift = n4h.trailing_zeros() + MI_SIZE_LOG2 - 1;
        extended[3] = warp_round2(proj_mid_x - proj_above_x, shift);
        extended[5] = warp_round2(proj_mid_y - proj_above_y, shift);
    } else {
        extended[3] = params[3];
        extended[5] = params[5];
        let left_x = i64::try_from(mi_col * 4).map_err(|_| geometry_error())? - 1;
        let left_y = mid_y;
        let proj_left_x = params[2] * left_x + params[3] * left_y + params[0];
        let proj_left_y = params[4] * left_x + params[5] * left_y + params[1];
        let shift = n4w.trailing_zeros() + MI_SIZE_LOG2 - 1;
        extended[2] = warp_round2(proj_mid_x - proj_left_x, shift);
        extended[4] = warp_round2(proj_mid_y - proj_left_y, shift);
    }
    reduce_warp_model(&mut extended);
    set_warp_translation(&mut extended, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(extended)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParsedWarpNewmv {
    pub(crate) mv: Mv,
    pub(crate) warp_params: [i64; 6],
    pub(crate) ref_mv_idx: usize,
    pub(crate) ref_warp_idx: usize,
    pub(crate) precision_idx: u8,
    pub(crate) warpmv_with_mvd: bool,
    pub(crate) block_precision: BlockPrecisionRecord,
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

/// § 5.20.7.13 EXTENDWARP / LOCALWARP tail: DRL, block precision, and the
/// NEWMV MVD — no `warp_idx` loop and no `read_warp_delta` — then the
/// mode's model derivation (§ 7.13.3.24 extension or § 7.13.3.23 least
/// squares over the § 7.12.3 warp samples).
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_warp_extend_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    neighbour_ctx: &BlockNeighbourContext,
    mv_config: MvReadConfig,
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    mode_ctx: &ModeContext,
    motion_mode: MotionMode,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    stack: &super::super::find_mv_stack::MvStack,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpNewmv> {
    let ref_mv_idx = read_drl_idx(
        cdfs,
        symbols,
        new_mv_context,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let block_precision = read_block_mv_precision_syntax(
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
    let block_config = MvReadConfig::inter(block_precision.mv_precision);
    let pred_mv = lowered_pred_mv(block_precision, stack.candidate(ref_mv_idx));
    let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, block_config)?;
    let diff = apply_inter_mvd_signs(magnitude, symbols, tile_offset, block_config, false, 1)?;
    let mv = Mv {
        row: mv_clamp_to_integer(pred_mv.row + diff.row),
        col: mv_clamp_to_integer(pred_mv.col + diff.col),
    };
    let warp_params = if motion_mode == MotionMode::LocalWarp {
        match super::super::find_mv_stack::find_warp_samples(mv_grid, block_ctx) {
            super::super::find_mv_stack::WarpSampleCollection::Samples(samples) => {
                local_warp_estimation(&samples, mv, mi_row, mi_col, n4w, n4h, tile_offset)?
            }
            super::super::find_mv_stack::WarpSampleCollection::List1MvUnretained => {
                return Err(inter_cap!(
                    "inter_warp_sample_list1_mv_unretained",
                    tile_offset,
                    "inter.local_warp.second_list_neighbour_mv",
                    "7.12.3.2"
                ));
            }
        }
    } else {
        extend_warp_estimation(
            mv_grid,
            block_ctx,
            mode_ctx,
            stack,
            ref_mv_idx,
            mv,
            mi_row,
            mi_col,
            n4w,
            n4h,
            tile_offset,
        )?
    };
    Ok(ParsedWarpNewmv {
        mv,
        warp_params,
        ref_mv_idx,
        ref_warp_idx: 0,
        precision_idx: 0,
        warpmv_with_mvd: false,
        block_precision,
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
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    stack: &super::super::find_mv_stack::MvStack,
    new_mv_context: usize,
    max_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpNewmv> {
    let ref_warp_idx = read_warp_ref_idx(cdfs, symbols, MAX_WARP_REF_CANDIDATES, tile_offset)?;
    let ref_mv_idx = read_drl_idx(
        cdfs,
        symbols,
        new_mv_context,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let block_precision = read_block_mv_precision_syntax(
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
    let block_config = MvReadConfig::inter(block_precision.mv_precision);
    let pred_mv = lowered_pred_mv(block_precision, stack.candidate(ref_mv_idx));
    let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, block_config)?;
    let diff = apply_inter_mvd_signs(magnitude, symbols, tile_offset, block_config, false, 1)?;
    let mv = Mv {
        row: mv_clamp_to_integer(pred_mv.row + diff.row),
        col: mv_clamp_to_integer(pred_mv.col + diff.col),
    };
    let (warp_params, precision_idx) = read_warp_delta_syntax(
        cdfs,
        symbols,
        sequence,
        b_size,
        ref_warp_idx,
        mv,
        mi_row,
        mi_col,
        n4w,
        n4h,
        stack,
        tile_offset,
    )?;
    Ok(ParsedWarpNewmv {
        mv,
        warp_params,
        ref_mv_idx,
        ref_warp_idx,
        precision_idx,
        warpmv_with_mvd: false,
        block_precision,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_warpmv_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    mv_config: MvReadConfig,
    _b_size: usize,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    stack: &super::super::find_mv_stack::MvStack,
    tile_offset: ByteOffset,
) -> Result<ParsedWarpNewmv> {
    let ref_warp_idx = read_warp_ref_idx(cdfs, symbols, MAX_WARP_REF_CANDIDATES, tile_offset)?;
    let warpmv_with_mvd = if ref_warp_idx < 2 {
        read_warpmv_with_mvd_flag(cdfs, symbols, tile_offset)?
    } else {
        false
    };
    let base_precision = if warpmv_with_mvd {
        mv_config.precision()
    } else {
        MV_PRECISION_EIGHTH_PEL
    };
    let base_mv = stack.warp_predicted_mv(ref_warp_idx, base_precision);
    let mv = if warpmv_with_mvd {
        let magnitude = read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, mv_config)?;
        let diff = apply_inter_mvd_signs(magnitude, symbols, tile_offset, mv_config, false, 1)?;
        Mv {
            row: mv_clamp_to_integer(base_mv.row + diff.row),
            col: mv_clamp_to_integer(base_mv.col + diff.col),
        }
    } else {
        base_mv
    };
    let mut warp_params = stack.warp_candidate(ref_warp_idx);
    reduce_warp_model(&mut warp_params);
    set_warp_translation(&mut warp_params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok(ParsedWarpNewmv {
        mv,
        warp_params,
        ref_mv_idx: 0,
        ref_warp_idx,
        precision_idx: 0,
        warpmv_with_mvd,
        block_precision: BlockPrecisionRecord::most_probable(mv_config.precision()),
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
    if flag > 1 {
        return Err(inter_cap!(
            "inter_warpmv_with_mvd_symbol",
            tile_offset,
            "inter.warpmv_with_mvd_flag symbol out of range",
            SPEC_MODE_INFO
        ));
    }
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
        if warp_idx > 1 {
            return Err(inter_cap!(
                "inter_warp_ref_idx_symbol",
                tile_offset,
                "inter.warp_ref_idx symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        ref_warp_idx = bit_idx + usize::from(warp_idx);
        if warp_idx == 0 {
            break;
        }
    }
    Ok(ref_warp_idx)
}

/// Maps a parsed § 5.20.7.15 interintra read onto the supported prediction
/// subset: smooth-mask II_DC/II_V/II_H. Wedge interintra (§ 7.13.3.27 tables)
/// and II_SMOOTH stay fail-closed defers after the bit-exact parse.
pub(crate) fn interintra_prediction_mode(
    syntax: WarpInterIntraSyntax,
    tile_offset: ByteOffset,
) -> Result<Option<InterIntraMode>> {
    if !syntax.enabled {
        return Ok(None);
    }
    if syntax.use_wedge {
        return Err(inter_cap!(
            "inter_wedge_interintra_prediction_unimplemented",
            tile_offset,
            "inter.interintra.wedge_mask",
            "7.13.3.27"
        ));
    }
    match syntax.mode {
        Some(0) => Ok(Some(InterIntraMode::Dc)),
        Some(1) => Ok(Some(InterIntraMode::Vertical)),
        Some(2) => Ok(Some(InterIntraMode::Horizontal)),
        Some(3) => Err(inter_cap!(
            "inter_interintra_smooth_unimplemented",
            tile_offset,
            "inter.interintra.ii_smooth",
            "7.13.3.29"
        )),
        _ => Err(inter_cap!(
            "inter_interintra_mode_missing",
            tile_offset,
            "inter.interintra.mode",
            "5.20.7.15"
        )),
    }
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
    if enabled > 1 {
        return Err(inter_cap!(
            "inter_warp_interintra_symbol",
            tile_offset,
            "inter.warp_inter_intra symbol out of range",
            SPEC_MODE_INFO
        ));
    }
    if enabled == 0 {
        return Ok(WarpInterIntraSyntax::default());
    }

    let mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::InterIntraMode { bsize_group }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if mode >= INTERINTRA_MODES {
        return Err(inter_cap!(
            "inter_warp_interintra_mode_symbol",
            tile_offset,
            "inter.interintra_mode symbol out of range",
            SPEC_MODE_INFO
        ));
    }

    let use_wedge = if WEDGE_USED_BY_BSIZE.get(b_size).copied().unwrap_or(false) {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::WedgeInterIntra, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if symbol > 1 {
            return Err(inter_cap!(
                "inter_wedge_interintra_symbol",
                tile_offset,
                "inter.use_wedge_interintra symbol out of range",
                SPEC_MODE_INFO
            ));
        }
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

pub(crate) fn read_wedge_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<u8> {
    let quad = cdfs
        .read_block_symbol_trace(TileCdfSelector::WedgeQuad, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if quad >= WEDGE_QUADS {
        return Err(inter_cap!(
            "inter_wedge_quad_symbol",
            tile_offset,
            "inter.wedge_quad symbol out of range",
            SPEC_MODE_INFO
        ));
    }
    let angle_in_quad = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::WedgeAngle {
                quad: usize::from(quad),
            },
            symbols,
        )
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if angle_in_quad >= QUAD_WEDGE_ANGLES {
        return Err(inter_cap!(
            "inter_wedge_angle_symbol",
            tile_offset,
            "inter.wedge_angle symbol out of range",
            SPEC_MODE_INFO
        ));
    }
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
        if symbol >= NUM_WEDGE_DIST - 1 {
            return Err(inter_cap!(
                "inter_wedge_dist2_symbol",
                tile_offset,
                "inter.wedge_dist_cdf2 symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        symbol + 1
    } else {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::WedgeDist1, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if symbol >= NUM_WEDGE_DIST {
            return Err(inter_cap!(
                "inter_wedge_dist1_symbol",
                tile_offset,
                "inter.wedge_dist_cdf symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        symbol
    };
    Ok(angle.saturating_mul(NUM_WEDGE_DIST).saturating_add(dist))
}

#[allow(clippy::too_many_arguments)]
fn read_warp_delta_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    b_size: usize,
    ref_warp_idx: usize,
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    stack: &super::super::find_mv_stack::MvStack,
    tile_offset: ByteOffset,
) -> Result<([i64; 6], u8)> {
    let mut params = stack.warp_candidate(ref_warp_idx);
    let use_six_param = sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_six_param_warp_delta)
        && ref_warp_idx == 1;
    let mut precision_idx = 0u8;

    if use_six_param || ref_warp_idx == 0 {
        precision_idx = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::WarpPrecision { block_size: b_size },
                symbols,
            )
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if precision_idx > 1 {
            return Err(inter_cap!(
                "inter_warp_precision_symbol",
                tile_offset,
                "inter.warp_delta_precision symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        let high_precision = precision_idx != 0;
        params[0] = 0;
        params[1] = 0;
        params[2] += read_warp_delta_param(cdfs, symbols, 2, high_precision, tile_offset)?;
        params[3] += read_warp_delta_param(cdfs, symbols, 3, high_precision, tile_offset)?;
        if use_six_param {
            params[4] += read_warp_delta_param(cdfs, symbols, 4, high_precision, tile_offset)?;
            params[5] += read_warp_delta_param(cdfs, symbols, 5, high_precision, tile_offset)?;
        } else {
            params[4] = -params[3];
            params[5] = params[2];
        }
    }

    reduce_warp_model(&mut params);
    set_warp_translation(&mut params, mv, mi_row, mi_col, n4w, n4h, tile_offset)?;
    Ok((params, precision_idx))
}

fn read_warp_delta_param(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    index: usize,
    high_precision: bool,
    tile_offset: ByteOffset,
) -> Result<i64> {
    let index_type = match index {
        2 | 5 => 0,
        3 | 4 => 1,
        _ => {
            return Err(inter_cap!(
                "inter_warp_delta_param_index",
                tile_offset,
                "inter.warp_delta_param index out of range",
                SPEC_MODE_INFO
            ));
        }
    };
    let mut value = cdfs
        .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamLow { index_type }, symbols)
        .map_err(|_| symbol_read_error(tile_offset))?
        .get();
    if high_precision && value == WARP_DELTA_NUM_SYMBOLS_LOW - 1 {
        let high = cdfs
            .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamHigh { index_type }, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if high >= WARP_DELTA_NUM_SYMBOLS_HIGH {
            return Err(inter_cap!(
                "inter_warp_delta_param_high_symbol",
                tile_offset,
                "inter.warp_delta_param_high symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        value = value
            .checked_add(high)
            .ok_or_else(|| warp_model_error(tile_offset))?;
    }
    let mut signed = i64::from(value);
    if signed != 0 {
        let sign = cdfs
            .read_block_symbol_trace(TileCdfSelector::WarpDeltaParamSign, symbols)
            .map_err(|_| symbol_read_error(tile_offset))?
            .get();
        if sign > 1 {
            return Err(inter_cap!(
                "inter_warp_delta_param_sign_symbol",
                tile_offset,
                "inter.warp_delta_param_sign symbol out of range",
                SPEC_MODE_INFO
            ));
        }
        if sign != 0 {
            signed = -signed;
        }
    }
    let step_bits = WARP_DELTA_STEP_BITS + 1 - u32::from(high_precision);
    signed
        .checked_shl(step_bits)
        .ok_or_else(|| warp_model_error(tile_offset))
}

fn set_warp_translation(
    params: &mut [i64; 6],
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
    params[0] = clamp_i128_to_i64(wmmat0, -WARPEDMODEL_TRANS_CLAMP, high);
    params[1] = clamp_i128_to_i64(wmmat1, -WARPEDMODEL_TRANS_CLAMP, high);
    Ok(())
}

fn clamp_i128_to_i64(value: i128, low: i64, high: i64) -> i64 {
    value.clamp(i128::from(low), i128::from(high)) as i64
}

fn warp_model_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_cap!(
        "inter_warp_model_overflow",
        tile_offset,
        "inter.warp_model arithmetic overflow",
        SPEC_MODE_INFO
    )
}
