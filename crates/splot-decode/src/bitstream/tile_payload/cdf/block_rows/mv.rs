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

use super::super::{
    CDF_ROW_LEN, TileCdfArray, TileCdfError, avg_cdf_rows, blend_cdf_rows, scale_cdf_rows,
};

const MV_CONTEXTS: usize = 2;
const SHELL_OFFSET_LOW_CLASS_BANKS: usize = 2;
const COL_MV_GREATER_BANKS: usize = 2;
const COL_MV_INDEX_BANKS: usize = 4;
const SHELL_OFFSET_OTHER_CLASS_BANKS: usize = 16;

type JointShellSetCdfRows = [[u16; CDF_ROW_LEN]; MV_CONTEXTS];
type JointShell0Class0CdfRows = [[u16; 6]; MV_CONTEXTS];
type JointShell0Class1CdfRows = [[u16; 7]; MV_CONTEXTS];
type JointShell1ClassCdfRows = [[u16; 7]; MV_CONTEXTS];
type JointShell3ClassCdfRows = [[u16; 8]; MV_CONTEXTS];
type JointShell4Class0CdfRows = [[u16; 8]; MV_CONTEXTS];
type JointShell4Class1CdfRows = [[u16; 9]; MV_CONTEXTS];
type JointShell5ClassCdfRows = [[u16; 9]; MV_CONTEXTS];
type JointShell6ClassCdfRows = [[u16; 9]; MV_CONTEXTS];
type JointShellLastTwoCdfRows = [[u16; CDF_ROW_LEN]; MV_CONTEXTS];
type ShellOffsetLowClassCdfRows = [[[u16; CDF_ROW_LEN]; SHELL_OFFSET_LOW_CLASS_BANKS]; MV_CONTEXTS];
type ShellOffsetClass2CdfRows = [[u16; CDF_ROW_LEN]; MV_CONTEXTS];
type ShellOffsetOtherClassCdfRows =
    [[[u16; CDF_ROW_LEN]; SHELL_OFFSET_OTHER_CLASS_BANKS]; MV_CONTEXTS];
type ColMvGreaterCdfRows = [[[u16; CDF_ROW_LEN]; COL_MV_GREATER_BANKS]; MV_CONTEXTS];
type ColMvIndexCdfRows = [[[u16; CDF_ROW_LEN]; COL_MV_INDEX_BANKS]; MV_CONTEXTS];
type AmvdJointCdfRows = [[u16; 5]; 1];
type AmvdIndexCdfRows = [[u16; 9]; 2];

macro_rules! visit_mv_cdf_rows {
    ($visit:ident) => {
        $visit!(joint_shell_set);
        $visit!(joint_shell0_class0);
        $visit!(joint_shell0_class1);
        $visit!(joint_shell1_class0);
        $visit!(joint_shell1_class1);
        $visit!(joint_shell3_class0);
        $visit!(joint_shell3_class1);
        $visit!(joint_shell4_class0);
        $visit!(joint_shell4_class1);
        $visit!(joint_shell5_class0);
        $visit!(joint_shell5_class1);
        $visit!(joint_shell6_class0);
        $visit!(joint_shell6_class1);
        $visit!(joint_shell_last_two);
        $visit!(shell_offset_low_class.flatten());
        $visit!(shell_offset_class2);
        $visit!(shell_offset_other_class.flatten());
        $visit!(col_mv_greater.flatten());
        $visit!(col_mv_index.flatten());
        $visit!(amvd_joint);
        $visit!(amvd_index);
    };
}

macro_rules! joint_shell_class_row_match {
    ($rows:expr, $precision:expr, $shell_set:expr, $mv_ctx:expr, $slice:ident) => {
        match ($precision, $shell_set) {
            (0, 0) => Ok($rows.joint_shell0_class0[$mv_ctx].$slice()),
            (0, 1) => Ok($rows.joint_shell0_class1[$mv_ctx].$slice()),
            (1, 0) => Ok($rows.joint_shell1_class0[$mv_ctx].$slice()),
            (1, 1) => Ok($rows.joint_shell1_class1[$mv_ctx].$slice()),
            (3, 0) => Ok($rows.joint_shell3_class0[$mv_ctx].$slice()),
            (3, 1) => Ok($rows.joint_shell3_class1[$mv_ctx].$slice()),
            (4, 0) => Ok($rows.joint_shell4_class0[$mv_ctx].$slice()),
            (4, 1) => Ok($rows.joint_shell4_class1[$mv_ctx].$slice()),
            (5, 0) => Ok($rows.joint_shell5_class0[$mv_ctx].$slice()),
            (5, 1) => Ok($rows.joint_shell5_class1[$mv_ctx].$slice()),
            (6, 0) => Ok($rows.joint_shell6_class0[$mv_ctx].$slice()),
            (6, 1) => Ok($rows.joint_shell6_class1[$mv_ctx].$slice()),
            _ => Err(precision_error($precision)),
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MvCdfSelector {
    JointShellSet {
        mv_ctx: usize,
    },
    JointShellClass {
        precision: usize,
        shell_set: usize,
        mv_ctx: usize,
    },
    JointShellLastTwo {
        mv_ctx: usize,
    },
    ShellOffsetLowClass {
        mv_ctx: usize,
        shell_class: usize,
    },
    ShellOffsetClass2 {
        mv_ctx: usize,
    },
    ShellOffsetOtherClass {
        mv_ctx: usize,
        i: usize,
    },
    ColMvGreater {
        mv_ctx: usize,
        i: usize,
    },
    ColMvIndex {
        mv_ctx: usize,
        ctx: usize,
    },
    AmvdJoint,
    AmvdIndex {
        comp: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MvCdfRows {
    joint_shell_set: JointShellSetCdfRows,
    joint_shell0_class0: JointShell0Class0CdfRows,
    joint_shell0_class1: JointShell0Class1CdfRows,
    joint_shell1_class0: JointShell1ClassCdfRows,
    joint_shell1_class1: JointShell1ClassCdfRows,
    joint_shell3_class0: JointShell3ClassCdfRows,
    joint_shell3_class1: JointShell3ClassCdfRows,
    joint_shell4_class0: JointShell4Class0CdfRows,
    joint_shell4_class1: JointShell4Class1CdfRows,
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
    amvd_joint: AmvdJointCdfRows,
    amvd_index: AmvdIndexCdfRows,
}

impl MvCdfRows {
    pub(crate) fn from_defaults() -> Self {
        Self {
            joint_shell_set: [DEFAULT_JOINT_SHELL_SET_CDF; MV_CONTEXTS],
            joint_shell0_class0: [DEFAULT_JOINT_SHELL0_CLASS0_CDF; MV_CONTEXTS],
            joint_shell0_class1: [DEFAULT_JOINT_SHELL0_CLASS1_CDF; MV_CONTEXTS],
            joint_shell1_class0: [DEFAULT_JOINT_SHELL1_CLASS0_CDF; MV_CONTEXTS],
            joint_shell1_class1: [DEFAULT_JOINT_SHELL1_CLASS1_CDF; MV_CONTEXTS],
            joint_shell3_class0: [DEFAULT_JOINT_SHELL3_CLASS0_CDF; MV_CONTEXTS],
            joint_shell3_class1: [DEFAULT_JOINT_SHELL3_CLASS1_CDF; MV_CONTEXTS],
            joint_shell4_class0: [DEFAULT_JOINT_SHELL4_CLASS0_CDF; MV_CONTEXTS],
            joint_shell4_class1: [DEFAULT_JOINT_SHELL4_CLASS1_CDF; MV_CONTEXTS],
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
            amvd_joint: [DEFAULT_MV_JOINT_ADAPTIVE_CDF],
            amvd_index: DEFAULT_AMVD_INDICES_CDF,
        }
    }

    pub(crate) fn row_mut(&mut self, selector: MvCdfSelector) -> Result<&mut [u16], TileCdfError> {
        match selector {
            MvCdfSelector::JointShellSet { mv_ctx } => checked_cdf_row_mut(
                &mut self.joint_shell_set,
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
                &mut self.joint_shell_last_two,
                mv_ctx,
                "mv_ctx",
                TileCdfArray::JointShell6Class,
            ),
            MvCdfSelector::ShellOffsetLowClass {
                mv_ctx,
                shell_class,
            } => checked_cdf_bank_row_mut(
                &mut self.shell_offset_low_class,
                mv_ctx,
                "mv_ctx",
                shell_class,
                "shell_class",
                TileCdfArray::ShellOffsetLowClass,
            ),
            MvCdfSelector::ShellOffsetClass2 { mv_ctx } => checked_cdf_row_mut(
                &mut self.shell_offset_class2,
                mv_ctx,
                "mv_ctx",
                TileCdfArray::ShellOffsetLowClass,
            ),
            MvCdfSelector::ShellOffsetOtherClass { mv_ctx, i } => checked_cdf_bank_row_mut(
                &mut self.shell_offset_other_class,
                mv_ctx,
                "mv_ctx",
                i,
                "i",
                TileCdfArray::ShellOffsetOtherClass,
            ),
            MvCdfSelector::ColMvGreater { mv_ctx, i } => checked_cdf_bank_row_mut(
                &mut self.col_mv_greater,
                mv_ctx,
                "mv_ctx",
                i,
                "i",
                TileCdfArray::ColMvGreater,
            ),
            MvCdfSelector::ColMvIndex { mv_ctx, ctx } => checked_cdf_bank_row_mut(
                &mut self.col_mv_index,
                mv_ctx,
                "mv_ctx",
                ctx,
                "ctx",
                TileCdfArray::ColMvIndex,
            ),
            MvCdfSelector::AmvdJoint => Ok(self.amvd_joint[0].as_mut_slice()),
            MvCdfSelector::AmvdIndex { comp } => {
                checked_cdf_row_mut(&mut self.amvd_index, comp, "comp", TileCdfArray::AmvdIndex)
            }
        }
    }

    pub(crate) fn average_from_tile(&mut self, tile: &Self, tile_num: u32, num_log2: u8) {
        macro_rules! avg_rows {
            ($field:ident $(. $flatten:ident())*) => {
                avg_cdf_rows(
                    flat_cdf_rows_mut!(self.$field $(, $flatten)*),
                    flat_cdf_rows!(tile.$field $(, $flatten)*),
                    tile_num,
                    num_log2,
                );
            };
        }

        visit_mv_cdf_rows!(avg_rows);
    }

    pub(crate) fn blend_from_saved(&mut self, saved: &Self) {
        macro_rules! blend_rows {
            ($field:ident $(. $flatten:ident())*) => {
                blend_cdf_rows(
                    flat_cdf_rows_mut!(self.$field $(, $flatten)*),
                    flat_cdf_rows!(saved.$field $(, $flatten)*),
                );
            };
        }

        visit_mv_cdf_rows!(blend_rows);
    }

    pub(crate) fn scale_counts(&mut self) {
        macro_rules! scale_rows {
            ($field:ident $(. $flatten:ident())*) => {
                scale_cdf_rows(flat_cdf_rows_mut!(self.$field $(, $flatten)*));
            };
        }

        visit_mv_cdf_rows!(scale_rows);
    }

    fn joint_shell_class_row_mut(
        &mut self,
        precision: usize,
        shell_set: usize,
        mv_ctx: usize,
    ) -> Result<&mut [u16], TileCdfError> {
        checked_shell_class_axes(precision, shell_set, mv_ctx)?;
        joint_shell_class_row_match!(self, precision, shell_set, mv_ctx, as_mut_slice)
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
