// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! SHELL-coded motion-vector CDF rows for AV2 § 5.20.7.20.

use splot_core::tables::cdf::{
    DEFAULT_AMVD_INDICES_CDF, DEFAULT_COL_MV_GREATER_CDF, DEFAULT_COL_MV_INDEX_CDF,
    DEFAULT_JOINT_SHELL_LAST_TWO_CLASSES_CDF, DEFAULT_JOINT_SHELL_SET_CDF,
    DEFAULT_JOINT_SHELL0_CLASS0_CDF, DEFAULT_JOINT_SHELL0_CLASS1_CDF,
    DEFAULT_JOINT_SHELL1_CLASS0_CDF, DEFAULT_JOINT_SHELL1_CLASS1_CDF,
    DEFAULT_JOINT_SHELL3_CLASS0_CDF, DEFAULT_JOINT_SHELL3_CLASS1_CDF,
    DEFAULT_JOINT_SHELL4_CLASS0_CDF, DEFAULT_JOINT_SHELL4_CLASS1_CDF,
    DEFAULT_JOINT_SHELL5_CLASS0_CDF, DEFAULT_JOINT_SHELL5_CLASS1_CDF,
    DEFAULT_JOINT_SHELL6_CLASS0_CDF, DEFAULT_JOINT_SHELL6_CLASS1_CDF,
    DEFAULT_MV_JOINT_ADAPTIVE_CDF, DEFAULT_SHELL_OFFSET_CLASS2_CDF,
    DEFAULT_SHELL_OFFSET_LOW_CLASS_CDF, DEFAULT_SHELL_OFFSET_OTHER_CLASS_CDF,
};

use splot_core::symbol::SymbolDecoder;

use super::super::block_read::{read_row, row_at};
#[cfg(any(debug_assertions, fuzzing, test))]
use super::super::cdf_store_valid;
use super::super::util::checked_context;
use super::super::{
    CDF_ROW_LEN, TileCdfArray, TileCdfError, avg_cdf_store, blend_cdf_store, scale_cdf_store,
};

const MV_CONTEXTS: usize = 2;
const SHELL_OFFSET_LOW_CLASS_BANKS: usize = 2;
const COL_MV_GREATER_BANKS: usize = 2;
const COL_MV_INDEX_BANKS: usize = 4;
const SHELL_OFFSET_OTHER_CLASS_BANKS: usize = 16;

/// A § 5.20.7.20 `read_mv` CDF row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MvCdfSelector {
    /// `joint_shell_set`.
    JointShellSet {
        /// `MvCtx`.
        mv_ctx: usize,
    },
    /// `joint_shell_class` of a `shell_set` at `precision`.
    JointShellClass {
        /// `MvPrecision`.
        precision: usize,
        /// `shell_set`.
        shell_set: usize,
        /// `MvCtx`.
        mv_ctx: usize,
    },
    /// `joint_shell_last_two_classes`.
    JointShellLastTwo {
        /// `MvCtx`.
        mv_ctx: usize,
    },
    /// `shell_offset_low_class` of class `0` or `1`.
    ShellOffsetLowClass {
        /// `MvCtx`.
        mv_ctx: usize,
        /// `shellClass`.
        shell_class: usize,
    },
    /// The first `shell_offset_class2` symbol.
    ShellOffsetClass2 {
        /// `MvCtx`.
        mv_ctx: usize,
    },
    /// `shell_offset_other_class` bit `i`.
    ShellOffsetOtherClass {
        /// `MvCtx`.
        mv_ctx: usize,
        /// The bit index.
        i: usize,
    },
    /// `col_mv_greater_flags` step `i`.
    ColMvGreater {
        /// `MvCtx`.
        mv_ctx: usize,
        /// The step index.
        i: usize,
    },
    /// `col_mv_index`.
    ColMvIndex {
        /// `MvCtx`.
        mv_ctx: usize,
        /// The class context.
        ctx: usize,
    },
    /// `amvd_joint`.
    AmvdJoint,
    /// `amvd_index` of component `comp`.
    AmvdIndex {
        /// `0` row, `1` column.
        comp: usize,
    },
}

cdf_rows! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct MvCdfRows[MvCdfElement]() {
        joint_shell_set[MV_CONTEXTS]: CDF_ROW_LEN = [DEFAULT_JOINT_SHELL_SET_CDF; MV_CONTEXTS],
        joint_shell0_class0[MV_CONTEXTS]: 6 = [DEFAULT_JOINT_SHELL0_CLASS0_CDF; MV_CONTEXTS],
        joint_shell0_class1[MV_CONTEXTS]: 7 = [DEFAULT_JOINT_SHELL0_CLASS1_CDF; MV_CONTEXTS],
        joint_shell1_class0[MV_CONTEXTS]: 7 = [DEFAULT_JOINT_SHELL1_CLASS0_CDF; MV_CONTEXTS],
        joint_shell1_class1[MV_CONTEXTS]: 7 = [DEFAULT_JOINT_SHELL1_CLASS1_CDF; MV_CONTEXTS],
        joint_shell3_class0[MV_CONTEXTS]: 8 = [DEFAULT_JOINT_SHELL3_CLASS0_CDF; MV_CONTEXTS],
        joint_shell3_class1[MV_CONTEXTS]: 8 = [DEFAULT_JOINT_SHELL3_CLASS1_CDF; MV_CONTEXTS],
        joint_shell4_class0[MV_CONTEXTS]: 8 = [DEFAULT_JOINT_SHELL4_CLASS0_CDF; MV_CONTEXTS],
        joint_shell4_class1[MV_CONTEXTS]: 9 = [DEFAULT_JOINT_SHELL4_CLASS1_CDF; MV_CONTEXTS],
        joint_shell5_class0[MV_CONTEXTS]: 9 = [DEFAULT_JOINT_SHELL5_CLASS0_CDF; MV_CONTEXTS],
        joint_shell5_class1[MV_CONTEXTS]: 9 = [DEFAULT_JOINT_SHELL5_CLASS1_CDF; MV_CONTEXTS],
        joint_shell6_class0[MV_CONTEXTS]: 9 = [DEFAULT_JOINT_SHELL6_CLASS0_CDF; MV_CONTEXTS],
        joint_shell6_class1[MV_CONTEXTS]: 9 = [DEFAULT_JOINT_SHELL6_CLASS1_CDF; MV_CONTEXTS],
        joint_shell_last_two[MV_CONTEXTS]: CDF_ROW_LEN = [DEFAULT_JOINT_SHELL_LAST_TWO_CLASSES_CDF; MV_CONTEXTS],
        shell_offset_low_class[MV_CONTEXTS][SHELL_OFFSET_LOW_CLASS_BANKS]: CDF_ROW_LEN = [DEFAULT_SHELL_OFFSET_LOW_CLASS_CDF; MV_CONTEXTS],
        shell_offset_class2[MV_CONTEXTS]: CDF_ROW_LEN = [DEFAULT_SHELL_OFFSET_CLASS2_CDF; MV_CONTEXTS],
        shell_offset_other_class[MV_CONTEXTS][SHELL_OFFSET_OTHER_CLASS_BANKS]: CDF_ROW_LEN = [DEFAULT_SHELL_OFFSET_OTHER_CLASS_CDF; MV_CONTEXTS],
        col_mv_greater[MV_CONTEXTS][COL_MV_GREATER_BANKS]: CDF_ROW_LEN = [DEFAULT_COL_MV_GREATER_CDF; MV_CONTEXTS],
        col_mv_index[MV_CONTEXTS][COL_MV_INDEX_BANKS]: CDF_ROW_LEN = [DEFAULT_COL_MV_INDEX_CDF; MV_CONTEXTS],
        amvd_joint[1]: 5 = [DEFAULT_MV_JOINT_ADAPTIVE_CDF],
        amvd_index[2]: 9 = DEFAULT_AMVD_INDICES_CDF,
        ..
    }
}

impl MvCdfRows {
    #[expect(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn row_mut(&mut self, selector: MvCdfSelector) -> Result<&mut [u16], TileCdfError> {
        match selector {
            MvCdfSelector::JointShellSet { mv_ctx } => checked_cdf_row_mut(
                self.joint_shell_set(),
                mv_ctx,
                "mv_ctx",
                TileCdfArray::JointShell6Class,
            ),
            MvCdfSelector::JointShellClass {
                precision,
                shell_set,
                mv_ctx,
            } => self.joint_shell_class_row_mut(precision, shell_set, mv_ctx),
            MvCdfSelector::JointShellLastTwo { mv_ctx } => checked_cdf_row_mut(
                self.joint_shell_last_two(),
                mv_ctx,
                "mv_ctx",
                TileCdfArray::JointShell6Class,
            ),
            MvCdfSelector::ShellOffsetLowClass {
                mv_ctx,
                shell_class,
            } => checked_cdf_bank_row_mut(
                self.shell_offset_low_class(),
                mv_ctx,
                "mv_ctx",
                shell_class,
                "shell_class",
                TileCdfArray::ShellOffsetLowClass,
            ),
            MvCdfSelector::ShellOffsetClass2 { mv_ctx } => checked_cdf_row_mut(
                self.shell_offset_class2(),
                mv_ctx,
                "mv_ctx",
                TileCdfArray::ShellOffsetLowClass,
            ),
            MvCdfSelector::ShellOffsetOtherClass { mv_ctx, i } => checked_cdf_bank_row_mut(
                self.shell_offset_other_class(),
                mv_ctx,
                "mv_ctx",
                i,
                "i",
                TileCdfArray::ShellOffsetOtherClass,
            ),
            MvCdfSelector::ColMvGreater { mv_ctx, i } => checked_cdf_bank_row_mut(
                self.col_mv_greater(),
                mv_ctx,
                "mv_ctx",
                i,
                "i",
                TileCdfArray::ColMvGreater,
            ),
            MvCdfSelector::ColMvIndex { mv_ctx, ctx } => checked_cdf_bank_row_mut(
                self.col_mv_index(),
                mv_ctx,
                "mv_ctx",
                ctx,
                "ctx",
                TileCdfArray::ColMvIndex,
            ),
            MvCdfSelector::AmvdJoint => Ok(self.amvd_joint()[0].as_mut_slice()),
            MvCdfSelector::AmvdIndex { comp } => {
                checked_cdf_row_mut(self.amvd_index(), comp, "comp", TileCdfArray::AmvdIndex)
            }
        }
    }

    /// The trusted read of the row of `selector`; `None` for a shell-class
    /// precision without a row.
    #[expect(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn read(
        &mut self,
        symbols: &mut SymbolDecoder<'_>,
        selector: MvCdfSelector,
    ) -> Option<u8> {
        let symbol = match selector {
            MvCdfSelector::JointShellSet { mv_ctx } => {
                read_row(symbols, row_at(self.joint_shell_set(), mv_ctx))
            }
            MvCdfSelector::JointShellClass {
                precision,
                shell_set,
                mv_ctx,
            } => match (precision, shell_set) {
                (0, 0) => read_row(symbols, row_at(self.joint_shell0_class0(), mv_ctx)),
                (0, _) => read_row(symbols, row_at(self.joint_shell0_class1(), mv_ctx)),
                (1, 0) => read_row(symbols, row_at(self.joint_shell1_class0(), mv_ctx)),
                (1, _) => read_row(symbols, row_at(self.joint_shell1_class1(), mv_ctx)),
                (3, 0) => read_row(symbols, row_at(self.joint_shell3_class0(), mv_ctx)),
                (3, _) => read_row(symbols, row_at(self.joint_shell3_class1(), mv_ctx)),
                (4, 0) => read_row(symbols, row_at(self.joint_shell4_class0(), mv_ctx)),
                (4, _) => read_row(symbols, row_at(self.joint_shell4_class1(), mv_ctx)),
                (5, 0) => read_row(symbols, row_at(self.joint_shell5_class0(), mv_ctx)),
                (5, _) => read_row(symbols, row_at(self.joint_shell5_class1(), mv_ctx)),
                (6, 0) => read_row(symbols, row_at(self.joint_shell6_class0(), mv_ctx)),
                (6, _) => read_row(symbols, row_at(self.joint_shell6_class1(), mv_ctx)),
                _ => return None,
            },
            MvCdfSelector::JointShellLastTwo { mv_ctx } => {
                read_row(symbols, row_at(self.joint_shell_last_two(), mv_ctx))
            }
            MvCdfSelector::ShellOffsetLowClass {
                mv_ctx,
                shell_class,
            } => read_row(
                symbols,
                row_at(row_at(self.shell_offset_low_class(), mv_ctx), shell_class),
            ),
            MvCdfSelector::ShellOffsetClass2 { mv_ctx } => {
                read_row(symbols, row_at(self.shell_offset_class2(), mv_ctx))
            }
            MvCdfSelector::ShellOffsetOtherClass { mv_ctx, i } => read_row(
                symbols,
                row_at(row_at(self.shell_offset_other_class(), mv_ctx), i),
            ),
            MvCdfSelector::ColMvGreater { mv_ctx, i } => {
                read_row(symbols, row_at(row_at(self.col_mv_greater(), mv_ctx), i))
            }
            MvCdfSelector::ColMvIndex { mv_ctx, ctx } => {
                read_row(symbols, row_at(row_at(self.col_mv_index(), mv_ctx), ctx))
            }
            MvCdfSelector::AmvdJoint => read_row(symbols, &mut self.amvd_joint()[0]),
            MvCdfSelector::AmvdIndex { comp } => read_row(symbols, row_at(self.amvd_index(), comp)),
        };
        Some(symbol as u8)
    }

    pub(crate) fn average_from_tile(&mut self, tile: &Self, tile_num: u32, num_log2: u8) {
        avg_cdf_store(
            &mut self.store,
            &tile.store,
            &Self::BOUNDS,
            tile_num,
            num_log2,
        );
    }

    pub(crate) fn blend_from_saved(&mut self, saved: &Self) {
        blend_cdf_store(&mut self.store, &saved.store, &Self::BOUNDS);
    }

    #[cfg(any(debug_assertions, fuzzing, test))]
    pub(crate) fn rows_valid(&self) -> bool {
        cdf_store_valid(&self.store, &Self::BOUNDS)
    }

    pub(crate) fn scale_counts(&mut self) {
        scale_cdf_store(&mut self.store, &Self::BOUNDS);
    }

    fn joint_shell_class_row_mut(
        &mut self,
        precision: usize,
        shell_set: usize,
        mv_ctx: usize,
    ) -> Result<&mut [u16], TileCdfError> {
        checked_shell_class_axes(precision, shell_set, mv_ctx)?;
        match (precision, shell_set) {
            (0, 0) => Ok(self.joint_shell0_class0()[mv_ctx].as_mut_slice()),
            (0, 1) => Ok(self.joint_shell0_class1()[mv_ctx].as_mut_slice()),
            (1, 0) => Ok(self.joint_shell1_class0()[mv_ctx].as_mut_slice()),
            (1, 1) => Ok(self.joint_shell1_class1()[mv_ctx].as_mut_slice()),
            (3, 0) => Ok(self.joint_shell3_class0()[mv_ctx].as_mut_slice()),
            (3, 1) => Ok(self.joint_shell3_class1()[mv_ctx].as_mut_slice()),
            (4, 0) => Ok(self.joint_shell4_class0()[mv_ctx].as_mut_slice()),
            (4, 1) => Ok(self.joint_shell4_class1()[mv_ctx].as_mut_slice()),
            (5, 0) => Ok(self.joint_shell5_class0()[mv_ctx].as_mut_slice()),
            (5, 1) => Ok(self.joint_shell5_class1()[mv_ctx].as_mut_slice()),
            (6, 0) => Ok(self.joint_shell6_class0()[mv_ctx].as_mut_slice()),
            (6, 1) => Ok(self.joint_shell6_class1()[mv_ctx].as_mut_slice()),
            _ => Err(precision_error(precision)),
        }
    }
}

fn checked_shell_class_axes(
    precision: usize,
    shell_set: usize,
    mv_ctx: usize,
) -> Result<(), TileCdfError> {
    checked_context(TileCdfArray::JointShell6Class, "shell_set", shell_set, 2)?;
    checked_context(
        TileCdfArray::JointShell6Class,
        "mv_ctx",
        mv_ctx,
        MV_CONTEXTS,
    )?;
    if matches!(precision, 0 | 1 | 3..=6) {
        Ok(())
    } else {
        Err(precision_error(precision))
    }
}

fn checked_row_mut<'a, T, const N: usize>(
    rows: &'a mut [T; N],
    index: usize,
    index_name: &'static str,
    array: TileCdfArray,
) -> Result<&'a mut T, TileCdfError> {
    rows.get_mut(index).ok_or(TileCdfError::SelectorOutOfRange {
        array,
        index_name,
        actual: index,
        max_exclusive: N,
    })
}

fn checked_cdf_row_mut<'a, const ROW_LEN: usize, const N: usize>(
    rows: &'a mut [[u16; ROW_LEN]; N],
    index: usize,
    index_name: &'static str,
    array: TileCdfArray,
) -> Result<&'a mut [u16], TileCdfError> {
    Ok(checked_row_mut(rows, index, index_name, array)?.as_mut_slice())
}

fn checked_cdf_bank_row_mut<'a, const ROW_LEN: usize, const OUTER: usize, const INNER: usize>(
    rows: &'a mut [[[u16; ROW_LEN]; INNER]; OUTER],
    outer: usize,
    outer_name: &'static str,
    inner: usize,
    inner_name: &'static str,
    array: TileCdfArray,
) -> Result<&'a mut [u16], TileCdfError> {
    let bank = checked_row_mut(rows, outer, outer_name, array)?;
    checked_cdf_row_mut(bank, inner, inner_name, array)
}

fn precision_error(precision: usize) -> TileCdfError {
    TileCdfError::SelectorOutOfRange {
        array: TileCdfArray::JointShell6Class,
        index_name: "precision",
        actual: precision,
        max_exclusive: 7,
    }
}
