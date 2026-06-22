// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.20 SHELL-coded `read_mv()` + § 5.20.7.13 `assign_mv()` sign pass
//! for the verified single-reference EighthPel NEWMV inter block.
//!
//! The motion vector difference is coded as a *shell* (the L1 magnitude
//! `shellIndex = |row| + |col|`) plus a split of that shell into its two
//! components (AV2 § 5.20.7.20):
//!
//! 1. `shell_set` — selects the high/low half of the shell-class range.
//! 2. `shell_class` — the shell magnitude class (with the EighthPel
//!    `joint_shell_last_two_classes` refinement when `shell_set` and the raw
//!    class is 7).
//! 3. `shell_offset_*` — the offset within the class, giving `shellIndex`.
//! 4. `col_mv_greater` / `col_mv_index` — split `shellIndex` into the column
//!    magnitude `diffMv[1]`; the row magnitude is `diffMv[0] = shellIndex -
//!    diffMv[1]`.
//! 5. one `mv_sign` L(1) bypass bit per nonzero component (the § 5.20.7.13 sign
//!    pass; sign derivation is disabled for EighthPel because `MvPrecision >=
//!    MV_PRECISION_QUARTER_PEL`).
//!
//! For the no-neighbour single 64x64 block the § 7.10 MV stack yields the zero
//! predictor (`PredMvs[0] == (0, 0)`), so the decoded block MV equals this read
//! delta after the § 5.20.7.13 `mv_clamp_to_integer` (a no-op for the small
//! shell magnitudes this subset produces).
//!
//! Only the EighthPel (`MvPrecision == MV_PRECISION_EIGHTH_PEL`, P == 6) case is
//! wired; the caller rejects any other precision before reaching this module, so
//! the `shift` (`MV_PRECISION_EIGHTH_PEL - MvPrecision`) is always 0 and only the
//! P == 6 `shell_class` CDF bank is consumed. Every read is a real § 8.2 symbol
//! or bypass bit; a wrong read desynchronises the arithmetic decoder and fails
//! the caller's § 8.2.4 `exit_symbol()`.

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{Mv, SPEC_MV, unsupported_at};
use crate::Result;
use crate::tile_payload::{TileCdfSelector, TileCdfSubset};

/// AV2 § 3 `MAX_COL_TRUNCATED_UNARY_VAL`: the maximum number of `col_mv_greater`
/// truncated-unary symbols.
const MAX_COL_TRUNCATED_UNARY_VAL: usize = 2;

/// AV2 § 3 `NUM_CTX_COL_MV_INDEX`: the `col_mv_index` context count.
const NUM_CTX_COL_MV_INDEX: usize = 4;

/// AV2 § 3 `MV_IN_USE_BITS` derived bounds for § 5.20.7.13 `mv_clamp_to_integer`.
const MV_LOW: i32 = -(1 << 16);
const MV_UPP: i32 = 1 << 16;

/// AV2 § 5.20.7.13 `mv_clamp_to_integer(v)`. A no-op for the small shell
/// magnitudes the verified subset produces, but applied faithfully.
const fn mv_clamp_to_integer(v: i32) -> i32 {
    if v < MV_LOW + 1 {
        MV_LOW + 8
    } else if v > MV_UPP - 1 {
        MV_UPP - 8
    } else {
        v
    }
}

/// Reads the § 5.20.7.20 SHELL-coded MV delta and applies the § 5.20.7.13 sign
/// pass for the verified single-reference EighthPel NEWMV block, returning the
/// signed `diffMv = (row, col)` in eighth-pel units. `PredMvs[0]` is the zero
/// no-neighbour predictor, so the caller adds (0, 0) and clamps; this returns the
/// clamped block MV directly.
pub(super) fn read_newmv_block_mv(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<Mv> {
    // §5.20.7.20: read the shell magnitude split into the two unsigned component
    // magnitudes (diff_row, diff_col), each already left-shifted by `shift` (0 for
    // EighthPel).
    let (diff_row, diff_col) = read_shell_diff(cdfs, symbols, tile_offset)?;

    // §5.20.7.13 assign_mv sign pass. EighthPel disables MVD sign derivation
    // (`MvPrecision >= MV_PRECISION_QUARTER_PEL`), so each nonzero component reads
    // an explicit `mv_sign` L(1) bypass bit. Spec order: i == 0 (single ref), comp
    // 0 (row) then comp 1 (col).
    let row = apply_sign(diff_row, symbols, tile_offset)?;
    let col = apply_sign(diff_col, symbols, tile_offset)?;

    // §5.20.7.13: BlockMvs[0][comp] = mv_clamp_to_integer(PredMvs[0][comp] +
    // diffMvs[0][comp]); PredMvs[0] == (0, 0) for the no-neighbour block.
    Ok(Mv {
        row: mv_clamp_to_integer(row),
        col: mv_clamp_to_integer(col),
    })
}

/// AV2 § 5.20.7.20 `read_mv()` magnitude path for EighthPel: returns the unsigned
/// `(diffMv[0], diffMv[1]) == (row_magnitude, col_magnitude)` (shift == 0).
fn read_shell_diff(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<(i32, i32)> {
    // shell_set  S()  -> TileJointShellSetCdf[MvCtx == 0].
    let shell_set = read_symbol(cdfs, symbols, TileCdfSelector::JointShellSet, tile_offset)?;

    // shell_class  S()  -> TileJointShell6ClassCdf[shell_set] (P == 6, EighthPel).
    let raw_shell_class = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::JointShell6Class {
            shell_set: shell_set as usize,
        },
        tile_offset,
    )?;
    let mut shell_class = raw_shell_class as i64;
    if shell_set != 0 {
        // (11 + MvPrecision) >> 1 == (11 + 6) >> 1 == 8 for EighthPel.
        shell_class += 8;
        // EighthPel: when the raw shell_class symbol is 7, read the last-two-classes
        // refinement.
        if raw_shell_class == 7 {
            let last_two = read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::JointShellLastTwo,
                tile_offset,
            )?;
            shell_class += i64::from(last_two);
        }
    }

    // shellClassOffset derivation.
    let shell_class_offset = read_shell_class_offset(cdfs, symbols, shell_class, tile_offset)?;

    let shell_class_base_index: i64 = if shell_class == 0 {
        0
    } else {
        1 << shell_class
    };
    let shell_index = shell_class_base_index + shell_class_offset;

    if shell_index <= 0 {
        // §5.20.7.20: diffMv stays (0, 0) when shellIndex == 0.
        return Ok((0, 0));
    }

    // §5.20.7.20: split shellIndex into the column magnitude `col`, then derive the
    // row magnitude as `shellIndex - col`. `shift == 0` for EighthPel so no scaling.
    let diff_col = read_col_split(cdfs, symbols, shell_index, shell_class, tile_offset)?;
    let diff_row = shell_index - diff_col;

    let diff_row = i32::try_from(diff_row).map_err(|_| mv_overflow(tile_offset))?;
    let diff_col = i32::try_from(diff_col).map_err(|_| mv_overflow(tile_offset))?;
    Ok((diff_row, diff_col))
}

/// AV2 § 5.20.7.20 `shellClassOffset` derivation (the `shell_offset_*` reads).
fn read_shell_class_offset(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    shell_class: i64,
    tile_offset: ByteOffset,
) -> Result<i64> {
    if shell_class < 2 {
        // shell_offset_low_class  S()  -> TileShellOffsetLowClassCdf[shellClass].
        let offset = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::ShellOffsetLowClass {
                shell_class: shell_class as usize,
            },
            tile_offset,
        )?;
        return Ok(i64::from(offset));
    }
    if shell_class == 2 {
        // §5.20.7.20: the class-2 loop reads shell_offset_class2 S() then up to two
        // shell_offset_class2_high L(1) bits, stopping when shellClassOffset == i.
        let mut shell_class_offset: i64 = 0;
        for i in 0..3 {
            if i == 0 {
                let v = read_symbol(
                    cdfs,
                    symbols,
                    TileCdfSelector::ShellOffsetClass2,
                    tile_offset,
                )?;
                shell_class_offset = i64::from(v);
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
    // shell_class > 2: read `shellClass` bits, each shell_offset_other_class S()
    // from bank `i`, building shellClassOffset |= bit << i.
    let mut shell_class_offset: i64 = 0;
    for i in 0..shell_class {
        let bit = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::ShellOffsetOtherClass { i: i as usize },
            tile_offset,
        )?;
        shell_class_offset |= i64::from(bit) << i;
    }
    Ok(shell_class_offset)
}

/// AV2 § 5.20.7.20 column-magnitude split: derives `diffMv[1]` (the column
/// magnitude) from `shellIndex` via `col_mv_greater` / `col_remainder` /
/// `col_mv_index`.
fn read_col_split(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    shell_index: i64,
    shell_class: i64,
    tile_offset: ByteOffset,
) -> Result<i64> {
    let mut col: i64 = 0;
    let maximum_pair_index = shell_index >> 1;
    if maximum_pair_index > 0 {
        let max_idx_bits = maximum_pair_index.min(MAX_COL_TRUNCATED_UNARY_VAL as i64);
        for i in 0..max_idx_bits {
            // col_mv_greater  S()  -> TileColMvGreaterCdf[MvCtx][i].
            let greater = read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::ColMvGreater { i: i as usize },
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
            // col_remainder  NS(n)  with n = maximumPairIndex - 1, coded
            // arithmetically (§4.11.13 over the §8.2 bypass primitives).
            let n = maximum_pair_index - 1;
            let remainder = read_ns(symbols, n, tile_offset)?;
            col = remainder + MAX_COL_TRUNCATED_UNARY_VAL as i64;
        }
    }

    let skip_coding_col_bit = col == maximum_pair_index && (shell_index & 1) == 0;
    if skip_coding_col_bit {
        return Ok(maximum_pair_index);
    }

    // col_mv_index  S()  -> TileColMvIndexCdf[MvCtx][Min(shellClass, NUM_CTX - 1)].
    let ctx = (shell_class as usize).min(NUM_CTX_COL_MV_INDEX - 1);
    let col_mv_index = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::ColMvIndex { ctx },
        tile_offset,
    )?;
    if col_mv_index == 0 {
        Ok(col)
    } else {
        Ok(shell_index - col)
    }
}

/// AV2 § 4.11.13 `NS(n)`: arithmetic-coded non-symmetric literal in `0..n`,
/// composed from the § 8.2 bypass primitives (`L(w-1)` then maybe `L(1)`).
fn read_ns(symbols: &mut SymbolDecoder<'_>, n: i64, tile_offset: ByteOffset) -> Result<i64> {
    if n <= 1 {
        return Ok(0);
    }
    // §4.11.13: w = FloorLog2(n) + 1; m = (1 << w) - n; v = L(w - 1).
    let n_u = u64::try_from(n).map_err(|_| mv_overflow(tile_offset))?;
    let floor_log2 = 63 - n_u.leading_zeros(); // FloorLog2(n) for n_u >= 1.
    let w = floor_log2 + 1;
    let v = read_literal(symbols, w - 1, tile_offset)?; // L(w - 1)
    let m = (1u64 << w) - n_u;
    if u64::from(v) < m {
        return Ok(i64::from(v));
    }
    // extra_bit L(1); return (v << 1) - m + extra_bit.
    let extra = read_bypass_bit(symbols, tile_offset)?;
    let result = (u64::from(v) << 1) - m + u64::from(extra);
    i64::try_from(result).map_err(|_| mv_overflow(tile_offset))
}

/// Applies the § 5.20.7.13 explicit `mv_sign` L(1) read to a nonzero component
/// magnitude; a zero magnitude reads no sign bit and stays 0.
fn apply_sign(
    magnitude: i32,
    symbols: &mut SymbolDecoder<'_>,
    tile_offset: ByteOffset,
) -> Result<i32> {
    if magnitude == 0 {
        return Ok(0);
    }
    let sign = read_bypass_bit(symbols, tile_offset)?;
    Ok(if sign != 0 { -magnitude } else { magnitude })
}

/// Reads one § 8.2.6 arithmetic symbol from the selected CDF row, mapping a
/// failure to a typed inter-decode error.
fn read_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
) -> Result<u8> {
    cdfs.read_block_symbol_trace(selector, symbols)
        .map(|symbol| symbol.get())
        .map_err(|_| mv_symbol_error(tile_offset))
}

/// Reads one § 8.2.3 pseudo-raw bypass bit (an `L(1)` value).
fn read_bypass_bit(symbols: &mut SymbolDecoder<'_>, tile_offset: ByteOffset) -> Result<u8> {
    symbols
        .read_bool()
        .map(u8::from)
        .map_err(|_| mv_symbol_error(tile_offset))
}

/// Reads an `L(n)` bypass literal (MSB-first), mapping a failure to a typed error.
fn read_literal(symbols: &mut SymbolDecoder<'_>, n: u32, tile_offset: ByteOffset) -> Result<u32> {
    if n == 0 {
        return Ok(0);
    }
    symbols
        .read_literal(n)
        .map_err(|_| mv_symbol_error(tile_offset))
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
