// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{Mv, SPEC_MV, unsupported_at};
use crate::Result;
use crate::bitstream::tile_payload::{MvCdfSelector, TileCdfSelector, TileCdfSubset};

const MAX_COL_TRUNCATED_UNARY_VAL: usize = 2;
const NUM_CTX_COL_MV_INDEX: usize = 4;
const MV_LOW: i32 = -(1 << 16);
const MV_UPP: i32 = 1 << 16;
const AMVD_INDEX_TO_MVD: [i32; 9] = [0, 2, 4, 6, 8, 16, 32, 64, 128];

pub(crate) const MV_PRECISION_EIGHT_PEL: u8 = 0;
pub(crate) const MV_PRECISION_FOUR_PEL: u8 = 1;
pub(crate) const MV_PRECISION_TWO_PEL: u8 = 2;
pub(crate) const MV_PRECISION_ONE_PEL: u8 = 3;
pub(crate) const MV_PRECISION_HALF_PEL: u8 = 4;
pub(crate) const MV_PRECISION_QUARTER_PEL: u8 = 5;
pub(crate) const MV_PRECISION_EIGHTH_PEL: u8 = 6;

pub(crate) const MV_INTRABC_CONTEXT: usize = 1;
const INTER_MV_CONTEXT: usize = 0;
const MV_CONTEXTS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MvReadConfig {
    precision: u8,
    mv_ctx: usize,
}

impl MvReadConfig {
    pub(crate) const fn inter(precision: u8) -> Self {
        Self {
            precision,
            mv_ctx: INTER_MV_CONTEXT,
        }
    }

    pub(crate) const fn intrabc(precision: u8) -> Self {
        Self {
            precision,
            mv_ctx: MV_INTRABC_CONTEXT,
        }
    }

    pub(crate) const fn precision(self) -> u8 {
        self.precision
    }
}

pub(crate) const fn mv_clamp_to_integer(v: i32) -> i32 {
    if v < MV_LOW + 1 {
        MV_LOW + 8
    } else if v > MV_UPP - 1 {
        MV_UPP - 8
    } else {
        v
    }
}

pub(crate) fn lower_mv_precision(precision: u8, mv: Mv) -> Mv {
    if precision >= MV_PRECISION_EIGHTH_PEL {
        return mv;
    }
    let bits = u32::from(MV_PRECISION_EIGHTH_PEL - precision);
    let radix = 1i32 << bits;
    let round = |component: i32| -> i32 {
        let a = component.abs();
        let a_int = (a - 1 + (radix >> 1)) >> bits;
        let mut rounded = if component >= 0 {
            a_int << bits
        } else {
            -(a_int << bits)
        };
        if (a_int << bits) != a {
            rounded = rounded.clamp(MV_LOW + radix, MV_UPP - radix);
        }
        rounded
    };
    Mv {
        row: round(mv.row),
        col: round(mv.col),
    }
}

pub(crate) fn read_newmv_block_mvd_magnitude(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
    config: MvReadConfig,
) -> Result<Mv> {
    read_newmv_block_mvd_magnitude_with_config(cdfs, symbols, tile_offset, config)
}

pub(crate) fn read_newmv_amvd_block_mvd(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<Mv> {
    let joint = read_symbol(cdfs, symbols, MvCdfSelector::AmvdJoint, tile_offset)?;
    let row = if matches!(joint, 2 | 3) {
        read_amvd_component(cdfs, symbols, 0, tile_offset)?
    } else {
        0
    };
    let col = if matches!(joint, 1 | 3) {
        read_amvd_component(cdfs, symbols, 1, tile_offset)?
    } else {
        0
    };
    Ok(Mv { row, col })
}

pub(crate) fn read_newmv_block_mvd_with_config(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
    config: MvReadConfig,
) -> Result<Mv> {
    let diff = read_newmv_block_mvd_magnitude_with_config(cdfs, symbols, tile_offset, config)?;
    apply_inter_mvd_signs(diff, symbols, tile_offset, config, false, 1)
}

pub(crate) fn read_newmv_block_mvd_magnitude_with_config(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
    config: MvReadConfig,
) -> Result<Mv> {
    validate_config(config, tile_offset)?;
    let (diff_row, diff_col) = read_shell_diff(cdfs, symbols, tile_offset, config)?;
    Ok(Mv {
        row: diff_row,
        col: diff_col,
    })
}

pub(crate) fn apply_inter_mvd_signs(
    magnitude: Mv,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
    config: MvReadConfig,
    derive_last_sign: bool,
    derive_threshold: usize,
) -> Result<Mv> {
    let [signed] = apply_inter_mvd_sign_vector_set(
        [magnitude],
        symbols,
        tile_offset,
        config,
        derive_last_sign,
        derive_threshold,
    )?;
    Ok(signed)
}

pub(crate) fn apply_inter_mvd_sign_pair(
    first: Mv,
    second: Mv,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
    config: MvReadConfig,
    derive_last_sign: bool,
    derive_threshold: usize,
) -> Result<(Mv, Mv)> {
    let [first, second] = apply_inter_mvd_sign_vector_set(
        [first, second],
        symbols,
        tile_offset,
        config,
        derive_last_sign,
        derive_threshold,
    )?;
    Ok((first, second))
}

fn apply_inter_mvd_sign_vector_set<const N: usize>(
    mut vectors: [Mv; N],
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
    config: MvReadConfig,
    derive_last_sign: bool,
    derive_threshold: usize,
) -> Result<[Mv; N]> {
    let shift = u32::from(MV_PRECISION_EIGHTH_PEL - config.precision());
    let mut nonzero_count = 0usize;
    let mut sum = 0i32;
    let mut last_nonzero = None;
    for (vector_idx, vector) in vectors.iter().enumerate() {
        for component_idx in 0..2 {
            let component = if component_idx == 0 {
                vector.row
            } else {
                vector.col
            };
            if component < 0 {
                return Err(mv_overflow(tile_offset));
            }
            if component != 0 {
                nonzero_count += 1;
                sum += component >> shift;
                last_nonzero = Some((vector_idx, component_idx));
            }
        }
    }

    let derive = derive_last_sign && nonzero_count >= derive_threshold;
    for (vector_idx, vector) in vectors.iter_mut().enumerate() {
        for component_idx in 0..2 {
            let component = if component_idx == 0 {
                vector.row
            } else {
                vector.col
            };
            if component == 0 {
                continue;
            }
            let sign = if derive && last_nonzero == Some((vector_idx, component_idx)) {
                u8::try_from(sum & 1).map_err(|_| mv_overflow(tile_offset))?
            } else {
                read_bypass_bit(symbols, tile_offset)?
            };
            if sign != 0 {
                if component_idx == 0 {
                    vector.row = -component;
                } else {
                    vector.col = -component;
                }
            }
        }
    }
    Ok(vectors)
}

fn read_shell_diff(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
    config: MvReadConfig,
) -> Result<(i32, i32)> {
    let mv_ctx = config.mv_ctx;
    let precision = config.precision;
    let shell_set = read_symbol(
        cdfs,
        symbols,
        MvCdfSelector::JointShellSet { mv_ctx },
        tile_offset,
    )?;

    let raw_shell_class = read_symbol(
        cdfs,
        symbols,
        MvCdfSelector::JointShellClass {
            precision: usize::from(precision),
            shell_set: usize::from(shell_set),
            mv_ctx,
        },
        tile_offset,
    )?;
    let mut shell_class = raw_shell_class as i64;
    if shell_set != 0 {
        shell_class += i64::from((11 + precision) >> 1);
        if precision == MV_PRECISION_EIGHTH_PEL && raw_shell_class == 7 {
            let last_two = read_symbol(
                cdfs,
                symbols,
                MvCdfSelector::JointShellLastTwo { mv_ctx },
                tile_offset,
            )?;
            shell_class += i64::from(last_two);
        }
    }

    let shell_class_offset =
        read_shell_class_offset(cdfs, symbols, shell_class, tile_offset, config)?;

    let shell_class_base_index: i64 = if shell_class == 0 {
        0
    } else {
        1 << shell_class
    };
    let shell_index = shell_class_base_index + shell_class_offset;

    if shell_index <= 0 {
        return Ok((0, 0));
    }

    let diff_col = read_col_split(cdfs, symbols, shell_index, shell_class, tile_offset, config)?;
    let diff_row = shell_index - diff_col;

    let shift = u32::from(MV_PRECISION_EIGHTH_PEL - precision);
    let diff_row = diff_row
        .checked_shl(shift)
        .ok_or_else(|| mv_overflow(tile_offset))?;
    let diff_col = diff_col
        .checked_shl(shift)
        .ok_or_else(|| mv_overflow(tile_offset))?;
    let diff_row = i32::try_from(diff_row).map_err(|_| mv_overflow(tile_offset))?;
    let diff_col = i32::try_from(diff_col).map_err(|_| mv_overflow(tile_offset))?;
    Ok((diff_row, diff_col))
}

fn read_amvd_component(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    comp: usize,
    tile_offset: ByteOffset,
) -> Result<i32> {
    let symbol = read_symbol(
        cdfs,
        symbols,
        MvCdfSelector::AmvdIndex { comp },
        tile_offset,
    )?;
    let index = usize::from(symbol) + 1;
    amvd_index_to_mvd(index, tile_offset)
}

fn amvd_index_to_mvd(index: usize, tile_offset: ByteOffset) -> Result<i32> {
    AMVD_INDEX_TO_MVD
        .get(index)
        .copied()
        .ok_or_else(|| mv_symbol_error(tile_offset))
}

fn read_shell_class_offset(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    shell_class: i64,
    tile_offset: ByteOffset,
    config: MvReadConfig,
) -> Result<i64> {
    let mv_ctx = config.mv_ctx;
    if shell_class < 2 {
        let offset = read_symbol(
            cdfs,
            symbols,
            MvCdfSelector::ShellOffsetLowClass {
                mv_ctx,
                shell_class: shell_class as usize,
            },
            tile_offset,
        )?;
        return Ok(i64::from(offset));
    }
    if shell_class == 2 {
        let mut shell_class_offset: i64 = 0;
        for i in 0..3 {
            if i == 0 {
                shell_class_offset = i64::from(read_symbol(
                    cdfs,
                    symbols,
                    MvCdfSelector::ShellOffsetClass2 { mv_ctx },
                    tile_offset,
                )?);
            } else {
                let high = read_bypass_bit(symbols, tile_offset)?;
                shell_class_offset = i64::from(high) + i as i64;
            }
            if shell_class_offset == i as i64 {
                break;
            }
        }
        return Ok(shell_class_offset);
    }
    let mut shell_class_offset: i64 = 0;
    for i in 0..shell_class {
        let bit = read_symbol(
            cdfs,
            symbols,
            MvCdfSelector::ShellOffsetOtherClass {
                mv_ctx,
                i: i as usize,
            },
            tile_offset,
        )?;
        shell_class_offset |= i64::from(bit) << i;
    }
    Ok(shell_class_offset)
}

fn read_col_split(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    shell_index: i64,
    shell_class: i64,
    tile_offset: ByteOffset,
    config: MvReadConfig,
) -> Result<i64> {
    let mv_ctx = config.mv_ctx;
    let mut col: i64 = 0;
    let maximum_pair_index = shell_index >> 1;
    if maximum_pair_index > 0 {
        let max_idx_bits = maximum_pair_index.min(MAX_COL_TRUNCATED_UNARY_VAL as i64);
        for i in 0..max_idx_bits {
            let greater = read_symbol(
                cdfs,
                symbols,
                MvCdfSelector::ColMvGreater {
                    mv_ctx,
                    i: i as usize,
                },
                tile_offset,
            )?;
            col = i + i64::from(greater);
            if greater == 0 {
                break;
            }
        }
        if maximum_pair_index > MAX_COL_TRUNCATED_UNARY_VAL as i64
            && col == MAX_COL_TRUNCATED_UNARY_VAL as i64
        {
            let n = maximum_pair_index - 1;
            let remainder = read_ns(symbols, n, tile_offset)?;
            col = remainder + MAX_COL_TRUNCATED_UNARY_VAL as i64;
        }
    }

    let skip_coding_col_bit = col == maximum_pair_index && (shell_index & 1) == 0;
    if skip_coding_col_bit {
        return Ok(maximum_pair_index);
    }

    let ctx = (shell_class as usize).min(NUM_CTX_COL_MV_INDEX - 1);
    let col_mv_index = read_symbol(
        cdfs,
        symbols,
        MvCdfSelector::ColMvIndex { mv_ctx, ctx },
        tile_offset,
    )?;
    if col_mv_index == 0 {
        Ok(col)
    } else {
        Ok(shell_index - col)
    }
}

fn validate_config(config: MvReadConfig, tile_offset: ByteOffset) -> Result<()> {
    if config.mv_ctx >= MV_CONTEXTS {
        return Err(mv_overflow(tile_offset));
    }
    match config.precision {
        MV_PRECISION_EIGHT_PEL
        | MV_PRECISION_FOUR_PEL
        | MV_PRECISION_ONE_PEL
        | MV_PRECISION_HALF_PEL
        | MV_PRECISION_QUARTER_PEL
        | MV_PRECISION_EIGHTH_PEL => Ok(()),
        _ => Err(mv_overflow(tile_offset)),
    }
}

fn read_ns(symbols: &mut SymbolDecoder<'_>, n: i64, tile_offset: ByteOffset) -> Result<i64> {
    if n <= 1 {
        return Ok(0);
    }
    let n_u = u64::try_from(n).map_err(|_| mv_overflow(tile_offset))?;
    let floor_log2 = n_u.ilog2();
    let w = floor_log2 + 1;
    let v = read_literal(symbols, w - 1, tile_offset)?;
    let m = (1u64 << w) - n_u;
    if u64::from(v) < m {
        return Ok(i64::from(v));
    }
    let extra = read_bypass_bit(symbols, tile_offset)?;
    let result = (u64::from(v) << 1) - m + u64::from(extra);
    i64::try_from(result).map_err(|_| mv_overflow(tile_offset))
}

fn read_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: MvCdfSelector,
    tile_offset: ByteOffset,
) -> Result<u8> {
    cdfs.read_block_symbol_trace(TileCdfSelector::ReadMv(selector), symbols)
        .map(splot_core::symbol::Symbol::get)
        .map_err(|_| mv_symbol_error(tile_offset))
}

fn read_bypass_bit(symbols: &mut SymbolDecoder<'_>, tile_offset: ByteOffset) -> Result<u8> {
    let value = symbols
        .read_bool()
        .map(u8::from)
        .map_err(|_| mv_symbol_error(tile_offset))?;
    Ok(value)
}

fn read_literal(symbols: &mut SymbolDecoder<'_>, n: u32, tile_offset: ByteOffset) -> Result<u32> {
    if n == 0 {
        return Ok(0);
    }
    let value = symbols
        .read_literal(n)
        .map_err(|_| mv_symbol_error(tile_offset))?;
    Ok(value)
}

fn mv_symbol_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    unsupported_at(
        "inter_mv_symbol_parse",
        tile_offset,
        "minimal inter decode could not parse a §5.20.7.20 read_mv symbol from the tile payload",
        SPEC_MV,
    )
}

fn mv_overflow(tile_offset: ByteOffset) -> crate::error::DecodeError {
    unsupported_at(
        "inter_mv_overflow",
        tile_offset,
        "minimal inter decode read_mv magnitude overflowed the supported motion-vector range",
        SPEC_MV,
    )
}

#[cfg(test)]
mod tests;
