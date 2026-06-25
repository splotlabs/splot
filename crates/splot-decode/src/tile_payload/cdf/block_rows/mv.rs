// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! SHELL-coded motion-vector CDF rows for AV2 § 5.20.7.20.

use splot_core::tables::cdf::{
    DEFAULT_COL_MV_GREATER_CDF, DEFAULT_COL_MV_INDEX_CDF, DEFAULT_JOINT_SHELL_LAST_TWO_CLASSES_CDF,
    DEFAULT_JOINT_SHELL_SET_CDF, DEFAULT_JOINT_SHELL3_CLASS0_CDF, DEFAULT_JOINT_SHELL3_CLASS1_CDF,
    DEFAULT_JOINT_SHELL5_CLASS0_CDF, DEFAULT_JOINT_SHELL5_CLASS1_CDF,
    DEFAULT_JOINT_SHELL6_CLASS0_CDF, DEFAULT_JOINT_SHELL6_CLASS1_CDF,
    DEFAULT_SHELL_OFFSET_CLASS2_CDF, DEFAULT_SHELL_OFFSET_LOW_CLASS_CDF,
    DEFAULT_SHELL_OFFSET_OTHER_CLASS_CDF,
};

use super::super::{CDF_ROW_LEN, TileCdfArray, TileCdfError, avg_cdf_row, scale_cdf_count};

const MV_CONTEXTS: usize = 2;
const SHELL_OFFSET_LOW_CLASS_BANKS: usize = 2;
const COL_MV_GREATER_BANKS: usize = 2;
const COL_MV_INDEX_BANKS: usize = 4;
const SHELL_OFFSET_OTHER_CLASS_BANKS: usize = 16;

type JointShellSetCdfRows = [[i32; CDF_ROW_LEN]; MV_CONTEXTS];
type JointShell3ClassCdfRows = [[i32; 8]; MV_CONTEXTS];
type JointShell5ClassCdfRows = [[i32; 9]; MV_CONTEXTS];
type JointShell6ClassCdfRows = [[i32; 9]; MV_CONTEXTS];
type JointShellLastTwoCdfRows = [[i32; CDF_ROW_LEN]; MV_CONTEXTS];
type ShellOffsetLowClassCdfRows = [[[i32; CDF_ROW_LEN]; SHELL_OFFSET_LOW_CLASS_BANKS]; MV_CONTEXTS];
type ShellOffsetClass2CdfRows = [[i32; CDF_ROW_LEN]; MV_CONTEXTS];
type ShellOffsetOtherClassCdfRows =
    [[[i32; CDF_ROW_LEN]; SHELL_OFFSET_OTHER_CLASS_BANKS]; MV_CONTEXTS];
type ColMvGreaterCdfRows = [[[i32; CDF_ROW_LEN]; COL_MV_GREATER_BANKS]; MV_CONTEXTS];
type ColMvIndexCdfRows = [[[i32; CDF_ROW_LEN]; COL_MV_INDEX_BANKS]; MV_CONTEXTS];

/// CDF selector for the AV2 § 5.20.7.20 `read_mv()` row family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MvCdfSelector {
    /// `TileJointShellSetCdf[MvCtx]`.
    JointShellSet {
        /// §5.20.7.20 `MvCtx`.
        mv_ctx: usize,
    },
    /// `TileJointShellPClassQCdf[MvCtx]`.
    JointShellClass {
        /// `P == MvPrecision` (`3`, `5`, or `6` in the supported subset).
        precision: usize,
        /// `Q == shell_set`.
        shell_set: usize,
        /// §5.20.7.20 `MvCtx`.
        mv_ctx: usize,
    },
    /// `TileJointShellLastTwoClassesCdf[MvCtx]`.
    JointShellLastTwo {
        /// §5.20.7.20 `MvCtx`.
        mv_ctx: usize,
    },
    /// `TileShellOffsetLowClassCdf[MvCtx][shellClass]`.
    ShellOffsetLowClass {
        /// §5.20.7.20 `MvCtx`.
        mv_ctx: usize,
        /// `shellClass` for the low-class offset rows.
        shell_class: usize,
    },
    /// `TileShellOffsetClass2Cdf[MvCtx]`.
    ShellOffsetClass2 {
        /// §5.20.7.20 `MvCtx`.
        mv_ctx: usize,
    },
    /// `TileShellOffsetOtherClassCdf[MvCtx][i]`.
    ShellOffsetOtherClass {
        /// §5.20.7.20 `MvCtx`.
        mv_ctx: usize,
        /// The §5.20.7.20 loop counter `i`.
        i: usize,
    },
    /// `TileColMvGreaterCdf[MvCtx][i]`.
    ColMvGreater {
        /// §5.20.7.20 `MvCtx`.
        mv_ctx: usize,
        /// The §5.20.7.20 loop counter `i`.
        i: usize,
    },
    /// `TileColMvIndexCdf[MvCtx][Min(shellClass, NUM_CTX_COL_MV_INDEX - 1)]`.
    ColMvIndex {
        /// §5.20.7.20 `MvCtx`.
        mv_ctx: usize,
        /// `Min(shellClass, NUM_CTX_COL_MV_INDEX - 1)`.
        ctx: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MvCdfRows {
    joint_shell_set: JointShellSetCdfRows,
    joint_shell3_class0: JointShell3ClassCdfRows,
    joint_shell3_class1: JointShell3ClassCdfRows,
    joint_shell5_class0: JointShell5ClassCdfRows,
    joint_shell5_class1: JointShell5ClassCdfRows,
    joint_shell6_class0: JointShell6ClassCdfRows,
    joint_shell6_class1: JointShell6ClassCdfRows,
    joint_shell_last_two: JointShellLastTwoCdfRows,
    shell_offset_low_class: ShellOffsetLowClassCdfRows,
    shell_offset_class2: ShellOffsetClass2CdfRows,
    shell_offset_other_class: ShellOffsetOtherClassCdfRows,
    col_mv_greater: ColMvGreaterCdfRows,
    col_mv_index: ColMvIndexCdfRows,
}

impl MvCdfRows {
    pub(super) fn from_defaults() -> Self {
        Self {
            joint_shell_set: [DEFAULT_JOINT_SHELL_SET_CDF; MV_CONTEXTS],
            joint_shell3_class0: [DEFAULT_JOINT_SHELL3_CLASS0_CDF; MV_CONTEXTS],
            joint_shell3_class1: [DEFAULT_JOINT_SHELL3_CLASS1_CDF; MV_CONTEXTS],
            joint_shell5_class0: [DEFAULT_JOINT_SHELL5_CLASS0_CDF; MV_CONTEXTS],
            joint_shell5_class1: [DEFAULT_JOINT_SHELL5_CLASS1_CDF; MV_CONTEXTS],
            joint_shell6_class0: [DEFAULT_JOINT_SHELL6_CLASS0_CDF; MV_CONTEXTS],
            joint_shell6_class1: [DEFAULT_JOINT_SHELL6_CLASS1_CDF; MV_CONTEXTS],
            joint_shell_last_two: [DEFAULT_JOINT_SHELL_LAST_TWO_CLASSES_CDF; MV_CONTEXTS],
            shell_offset_low_class: [DEFAULT_SHELL_OFFSET_LOW_CLASS_CDF; MV_CONTEXTS],
            shell_offset_class2: [DEFAULT_SHELL_OFFSET_CLASS2_CDF; MV_CONTEXTS],
            shell_offset_other_class: [DEFAULT_SHELL_OFFSET_OTHER_CLASS_CDF; MV_CONTEXTS],
            col_mv_greater: [DEFAULT_COL_MV_GREATER_CDF; MV_CONTEXTS],
            col_mv_index: [DEFAULT_COL_MV_INDEX_CDF; MV_CONTEXTS],
        }
    }

    pub(super) fn row(&self, selector: MvCdfSelector) -> Result<&[i32], TileCdfError> {
        match selector {
            MvCdfSelector::JointShellSet { mv_ctx } => Ok(checked_row(
                &self.joint_shell_set,
                mv_ctx,
                "mv_ctx",
                TileCdfArray::JointShell6Class,
            )?
            .as_slice()),
            MvCdfSelector::JointShellClass {
                precision,
                shell_set,
                mv_ctx,
            } => self.joint_shell_class_row(precision, shell_set, mv_ctx),
            MvCdfSelector::JointShellLastTwo { mv_ctx } => Ok(checked_row(
                &self.joint_shell_last_two,
                mv_ctx,
                "mv_ctx",
                TileCdfArray::JointShell6Class,
            )?
            .as_slice()),
            MvCdfSelector::ShellOffsetLowClass {
                mv_ctx,
                shell_class,
            } => {
                let bank = checked_row(
                    &self.shell_offset_low_class,
                    mv_ctx,
                    "mv_ctx",
                    TileCdfArray::ShellOffsetLowClass,
                )?;
                Ok(checked_row(
                    bank,
                    shell_class,
                    "shell_class",
                    TileCdfArray::ShellOffsetLowClass,
                )?
                .as_slice())
            }
            MvCdfSelector::ShellOffsetClass2 { mv_ctx } => Ok(checked_row(
                &self.shell_offset_class2,
                mv_ctx,
                "mv_ctx",
                TileCdfArray::ShellOffsetLowClass,
            )?
            .as_slice()),
            MvCdfSelector::ShellOffsetOtherClass { mv_ctx, i } => {
                let bank = checked_row(
                    &self.shell_offset_other_class,
                    mv_ctx,
                    "mv_ctx",
                    TileCdfArray::ShellOffsetOtherClass,
                )?;
                Ok(checked_row(bank, i, "i", TileCdfArray::ShellOffsetOtherClass)?.as_slice())
            }
            MvCdfSelector::ColMvGreater { mv_ctx, i } => {
                let bank = checked_row(
                    &self.col_mv_greater,
                    mv_ctx,
                    "mv_ctx",
                    TileCdfArray::ColMvGreater,
                )?;
                Ok(checked_row(bank, i, "i", TileCdfArray::ColMvGreater)?.as_slice())
            }
            MvCdfSelector::ColMvIndex { mv_ctx, ctx } => {
                let bank = checked_row(
                    &self.col_mv_index,
                    mv_ctx,
                    "mv_ctx",
                    TileCdfArray::ColMvIndex,
                )?;
                Ok(checked_row(bank, ctx, "ctx", TileCdfArray::ColMvIndex)?.as_slice())
            }
        }
    }

    pub(super) fn row_mut(&mut self, selector: MvCdfSelector) -> Result<&mut [i32], TileCdfError> {
        match selector {
            MvCdfSelector::JointShellSet { mv_ctx } => Ok(checked_row_mut(
                &mut self.joint_shell_set,
                mv_ctx,
                "mv_ctx",
                TileCdfArray::JointShell6Class,
            )?
            .as_mut_slice()),
            MvCdfSelector::JointShellClass {
                precision,
                shell_set,
                mv_ctx,
            } => self.joint_shell_class_row_mut(precision, shell_set, mv_ctx),
            MvCdfSelector::JointShellLastTwo { mv_ctx } => Ok(checked_row_mut(
                &mut self.joint_shell_last_two,
                mv_ctx,
                "mv_ctx",
                TileCdfArray::JointShell6Class,
            )?
            .as_mut_slice()),
            MvCdfSelector::ShellOffsetLowClass {
                mv_ctx,
                shell_class,
            } => {
                let bank = checked_row_mut(
                    &mut self.shell_offset_low_class,
                    mv_ctx,
                    "mv_ctx",
                    TileCdfArray::ShellOffsetLowClass,
                )?;
                Ok(checked_row_mut(
                    bank,
                    shell_class,
                    "shell_class",
                    TileCdfArray::ShellOffsetLowClass,
                )?
                .as_mut_slice())
            }
            MvCdfSelector::ShellOffsetClass2 { mv_ctx } => Ok(checked_row_mut(
                &mut self.shell_offset_class2,
                mv_ctx,
                "mv_ctx",
                TileCdfArray::ShellOffsetLowClass,
            )?
            .as_mut_slice()),
            MvCdfSelector::ShellOffsetOtherClass { mv_ctx, i } => {
                let bank = checked_row_mut(
                    &mut self.shell_offset_other_class,
                    mv_ctx,
                    "mv_ctx",
                    TileCdfArray::ShellOffsetOtherClass,
                )?;
                Ok(
                    checked_row_mut(bank, i, "i", TileCdfArray::ShellOffsetOtherClass)?
                        .as_mut_slice(),
                )
            }
            MvCdfSelector::ColMvGreater { mv_ctx, i } => {
                let bank = checked_row_mut(
                    &mut self.col_mv_greater,
                    mv_ctx,
                    "mv_ctx",
                    TileCdfArray::ColMvGreater,
                )?;
                Ok(checked_row_mut(bank, i, "i", TileCdfArray::ColMvGreater)?.as_mut_slice())
            }
            MvCdfSelector::ColMvIndex { mv_ctx, ctx } => {
                let bank = checked_row_mut(
                    &mut self.col_mv_index,
                    mv_ctx,
                    "mv_ctx",
                    TileCdfArray::ColMvIndex,
                )?;
                Ok(checked_row_mut(bank, ctx, "ctx", TileCdfArray::ColMvIndex)?.as_mut_slice())
            }
        }
    }

    pub(super) fn average_from_tile(&mut self, tile: &Self, tile_num: u32, num_log2: u8) {
        for mv_ctx in 0..MV_CONTEXTS {
            avg_cdf_row(
                &mut self.joint_shell_set[mv_ctx],
                &tile.joint_shell_set[mv_ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.joint_shell3_class0[mv_ctx],
                &tile.joint_shell3_class0[mv_ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.joint_shell3_class1[mv_ctx],
                &tile.joint_shell3_class1[mv_ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.joint_shell5_class0[mv_ctx],
                &tile.joint_shell5_class0[mv_ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.joint_shell5_class1[mv_ctx],
                &tile.joint_shell5_class1[mv_ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.joint_shell6_class0[mv_ctx],
                &tile.joint_shell6_class0[mv_ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.joint_shell6_class1[mv_ctx],
                &tile.joint_shell6_class1[mv_ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.joint_shell_last_two[mv_ctx],
                &tile.joint_shell_last_two[mv_ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.shell_offset_class2[mv_ctx],
                &tile.shell_offset_class2[mv_ctx],
                tile_num,
                num_log2,
            );
        }
        for mv_ctx in 0..MV_CONTEXTS {
            for bank in 0..SHELL_OFFSET_LOW_CLASS_BANKS {
                avg_cdf_row(
                    &mut self.shell_offset_low_class[mv_ctx][bank],
                    &tile.shell_offset_low_class[mv_ctx][bank],
                    tile_num,
                    num_log2,
                );
            }
            for bank in 0..SHELL_OFFSET_OTHER_CLASS_BANKS {
                avg_cdf_row(
                    &mut self.shell_offset_other_class[mv_ctx][bank],
                    &tile.shell_offset_other_class[mv_ctx][bank],
                    tile_num,
                    num_log2,
                );
            }
            for bank in 0..COL_MV_GREATER_BANKS {
                avg_cdf_row(
                    &mut self.col_mv_greater[mv_ctx][bank],
                    &tile.col_mv_greater[mv_ctx][bank],
                    tile_num,
                    num_log2,
                );
            }
            for bank in 0..COL_MV_INDEX_BANKS {
                avg_cdf_row(
                    &mut self.col_mv_index[mv_ctx][bank],
                    &tile.col_mv_index[mv_ctx][bank],
                    tile_num,
                    num_log2,
                );
            }
        }
    }

    pub(super) fn scale_counts(&mut self) {
        for mv_ctx in 0..MV_CONTEXTS {
            scale_cdf_count(&mut self.joint_shell_set[mv_ctx]);
            scale_cdf_count(&mut self.joint_shell3_class0[mv_ctx]);
            scale_cdf_count(&mut self.joint_shell3_class1[mv_ctx]);
            scale_cdf_count(&mut self.joint_shell5_class0[mv_ctx]);
            scale_cdf_count(&mut self.joint_shell5_class1[mv_ctx]);
            scale_cdf_count(&mut self.joint_shell6_class0[mv_ctx]);
            scale_cdf_count(&mut self.joint_shell6_class1[mv_ctx]);
            scale_cdf_count(&mut self.joint_shell_last_two[mv_ctx]);
            scale_cdf_count(&mut self.shell_offset_class2[mv_ctx]);
            for bank in 0..SHELL_OFFSET_LOW_CLASS_BANKS {
                scale_cdf_count(&mut self.shell_offset_low_class[mv_ctx][bank]);
            }
            for bank in 0..SHELL_OFFSET_OTHER_CLASS_BANKS {
                scale_cdf_count(&mut self.shell_offset_other_class[mv_ctx][bank]);
            }
            for bank in 0..COL_MV_GREATER_BANKS {
                scale_cdf_count(&mut self.col_mv_greater[mv_ctx][bank]);
            }
            for bank in 0..COL_MV_INDEX_BANKS {
                scale_cdf_count(&mut self.col_mv_index[mv_ctx][bank]);
            }
        }
    }

    fn joint_shell_class_row(
        &self,
        precision: usize,
        shell_set: usize,
        mv_ctx: usize,
    ) -> Result<&[i32], TileCdfError> {
        checked_shell_class_axes(precision, shell_set, mv_ctx)?;
        match (precision, shell_set) {
            (3, 0) => Ok(self.joint_shell3_class0[mv_ctx].as_slice()),
            (3, 1) => Ok(self.joint_shell3_class1[mv_ctx].as_slice()),
            (5, 0) => Ok(self.joint_shell5_class0[mv_ctx].as_slice()),
            (5, 1) => Ok(self.joint_shell5_class1[mv_ctx].as_slice()),
            (6, 0) => Ok(self.joint_shell6_class0[mv_ctx].as_slice()),
            (6, 1) => Ok(self.joint_shell6_class1[mv_ctx].as_slice()),
            _ => Err(precision_error(precision)),
        }
    }

    fn joint_shell_class_row_mut(
        &mut self,
        precision: usize,
        shell_set: usize,
        mv_ctx: usize,
    ) -> Result<&mut [i32], TileCdfError> {
        checked_shell_class_axes(precision, shell_set, mv_ctx)?;
        match (precision, shell_set) {
            (3, 0) => Ok(self.joint_shell3_class0[mv_ctx].as_mut_slice()),
            (3, 1) => Ok(self.joint_shell3_class1[mv_ctx].as_mut_slice()),
            (5, 0) => Ok(self.joint_shell5_class0[mv_ctx].as_mut_slice()),
            (5, 1) => Ok(self.joint_shell5_class1[mv_ctx].as_mut_slice()),
            (6, 0) => Ok(self.joint_shell6_class0[mv_ctx].as_mut_slice()),
            (6, 1) => Ok(self.joint_shell6_class1[mv_ctx].as_mut_slice()),
            _ => Err(precision_error(precision)),
        }
    }
}

fn checked_shell_class_axes(
    precision: usize,
    shell_set: usize,
    mv_ctx: usize,
) -> Result<(), TileCdfError> {
    if shell_set >= 2 {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::JointShell6Class,
            index_name: "shell_set",
            actual: shell_set,
            max_exclusive: 2,
        });
    }
    if mv_ctx >= MV_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::JointShell6Class,
            index_name: "mv_ctx",
            actual: mv_ctx,
            max_exclusive: MV_CONTEXTS,
        });
    }
    if matches!(precision, 3 | 5 | 6) {
        Ok(())
    } else {
        Err(precision_error(precision))
    }
}

fn checked_row<'a, T, const N: usize>(
    rows: &'a [T; N],
    index: usize,
    index_name: &'static str,
    array: TileCdfArray,
) -> Result<&'a T, TileCdfError> {
    rows.get(index).ok_or(TileCdfError::SelectorOutOfRange {
        array,
        index_name,
        actual: index,
        max_exclusive: N,
    })
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

fn precision_error(precision: usize) -> TileCdfError {
    TileCdfError::SelectorOutOfRange {
        array: TileCdfArray::JointShell6Class,
        index_name: "precision",
        actual: precision,
        max_exclusive: 7,
    }
}
