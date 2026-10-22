// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 5.20.7.20 `read_mv` writer: the joint shell coding of one motion vector
//! difference and its § 5.20.7.21 sign bits, inverting the decoder's reader
//! with the facade's CDF rows. Adaptive MVD and sign derivation stay off.

use splot_core::symbol_encoder::SymbolSink;
use splot_decode::encoder_syntax::{
    IntraTileSyntax, MV_PRECISION_EIGHTH_PEL, MotionVector, MvCdfSelector,
};

use super::coeffs::write_symbol;
use super::{Result, TileWriterError};

/// The inter `MvCtx`.
const MV_CTX: usize = 0;
/// `MAX_COL_TRUNCATED_UNARY_VAL`.
const MAX_COL_UNARY: u32 = 2;
/// `NUM_CTX_COL_MV_INDEX`.
const COL_MV_INDEX_CONTEXTS: u32 = 4;

/// Writes the `NEWMV` difference `mvd` (in `1/8` samples, a multiple of the
/// precision step) with the rows of `syntax`. Eighth-sample precision, which
/// codes `joint_shell_last_two_classes`, is not supported.
pub(crate) fn write_mvd(
    syntax: &mut IntraTileSyntax,
    sink: &mut impl SymbolSink,
    mvd: MotionVector,
) -> Result<()> {
    let precision = syntax.mv_precision();
    if precision >= MV_PRECISION_EIGHTH_PEL {
        return Err(TileWriterError::Unsupported("eighth-pel motion vectors"));
    }
    let shift = u32::from(MV_PRECISION_EIGHTH_PEL - precision);
    let magnitude = |v: i32| -> Result<u32> {
        let a = v.unsigned_abs();
        if a & ((1 << shift) - 1) != 0 {
            return Err(TileWriterError::Unsupported(
                "a motion vector off the precision lattice",
            ));
        }
        Ok(a >> shift)
    };
    let (row, col) = (magnitude(mvd.row)?, magnitude(mvd.col)?);
    let shell = row + col;
    let class = if shell < 2 { 0 } else { shell.ilog2() };
    let base = if class == 0 { 0 } else { 1 << class };
    let set0_classes = u32::from((11 + precision) >> 1);
    let shell_set = u32::from(class >= set0_classes);
    write_symbol(
        sink,
        syntax.mv_cdf(MvCdfSelector::JointShellSet { mv_ctx: MV_CTX })?,
        shell_set as usize,
    )?;
    write_symbol(
        sink,
        syntax.mv_cdf(MvCdfSelector::JointShellClass {
            precision: usize::from(precision),
            shell_set: shell_set as usize,
            mv_ctx: MV_CTX,
        })?,
        (class - shell_set * set0_classes) as usize,
    )?;
    write_class_offset(syntax, sink, class, shell - base)?;
    if shell > 0 {
        write_col(syntax, sink, shell, class, col)?;
    }
    for value in [mvd.row, mvd.col] {
        if value != 0 {
            sink.literal(u32::from(value < 0), 1)?;
        }
    }
    Ok(())
}

/// `shell_class_offset`: one symbol for classes `0` and `1`, a truncated
/// unary of up to three steps for class `2`, else one bit per class bit.
fn write_class_offset(
    syntax: &mut IntraTileSyntax,
    sink: &mut impl SymbolSink,
    class: u32,
    offset: u32,
) -> Result<()> {
    match class {
        0 | 1 => write_symbol(
            sink,
            syntax.mv_cdf(MvCdfSelector::ShellOffsetLowClass {
                mv_ctx: MV_CTX,
                shell_class: class as usize,
            })?,
            offset as usize,
        ),
        2 => {
            write_symbol(
                sink,
                syntax.mv_cdf(MvCdfSelector::ShellOffsetClass2 { mv_ctx: MV_CTX })?,
                usize::from(offset > 0),
            )?;
            if offset > 0 {
                for step in 1..=2 {
                    sink.literal(u32::from(offset > step), 1)?;
                    if offset <= step {
                        break;
                    }
                }
            }
            Ok(())
        }
        _ => {
            for bit in 0..class {
                write_symbol(
                    sink,
                    syntax.mv_cdf(MvCdfSelector::ShellOffsetOtherClass {
                        mv_ctx: MV_CTX,
                        i: bit as usize,
                    })?,
                    ((offset >> bit) & 1) as usize,
                )?;
            }
            Ok(())
        }
    }
}

/// The column split of a nonzero `shell`: the smaller part as a truncated
/// unary plus a `ns` remainder, then which part the column takes.
fn write_col(
    syntax: &mut IntraTileSyntax,
    sink: &mut impl SymbolSink,
    shell: u32,
    class: u32,
    col: u32,
) -> Result<()> {
    let max_pair = shell >> 1;
    let low = col.min(shell - col);
    if max_pair > 0 {
        let steps = max_pair.min(MAX_COL_UNARY);
        for i in 0..steps {
            let greater = low > i;
            write_symbol(
                sink,
                syntax.mv_cdf(MvCdfSelector::ColMvGreater {
                    mv_ctx: MV_CTX,
                    i: i as usize,
                })?,
                usize::from(greater),
            )?;
            if !greater {
                break;
            }
        }
        if max_pair > MAX_COL_UNARY && low >= MAX_COL_UNARY {
            write_ns(sink, max_pair - 1, low - MAX_COL_UNARY)?;
        }
    }
    if low == max_pair && shell & 1 == 0 {
        return Ok(());
    }
    write_symbol(
        sink,
        syntax.mv_cdf(MvCdfSelector::ColMvIndex {
            mv_ctx: MV_CTX,
            ctx: class.min(COL_MV_INDEX_CONTEXTS - 1) as usize,
        })?,
        usize::from(col != low),
    )
}

/// `ns(n)` (§ 4.10.10) of `value < n`.
fn write_ns(sink: &mut impl SymbolSink, n: u32, value: u32) -> Result<()> {
    if n <= 1 {
        return Ok(());
    }
    let w = n.ilog2() + 1;
    let m = (1 << w) - n;
    if value < m {
        sink.literal(value, w - 1)?;
    } else {
        sink.literal((value + m) >> 1, w - 1)?;
        sink.literal((value + m) & 1, 1)?;
    }
    Ok(())
}
