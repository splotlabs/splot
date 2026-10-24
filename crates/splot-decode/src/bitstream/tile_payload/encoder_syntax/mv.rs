// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Motion vector differences of the encoder facade (§ 5.20.7.20): the
//! decoder's own `read_mv` reads each difference while [`MvWriter`] answers
//! its symbols for the target, then the facade writes the answers.

use splot_core::span::ByteOffset;
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;

use super::super::cdf::{MvCdfSelector, TileCdfSelector, TileCdfSubset};
use super::inter::{MV_PRECISION_EIGHTH_PEL, MotionVector};
use super::{CdfJournal, CdfRow, EncoderSyntaxError, IntraTileSyntax, Result, rule};
use crate::prediction::inter::read_mv::{
    AMVD_INDEX_TO_MVD, MvReadConfig, MvSymbols, apply_inter_mvd_sign_pair, apply_inter_mvd_signs,
    read_newmv_amvd_block_mvd, read_newmv_block_mvd_magnitude_with_config, shell_set0_classes,
};
use crate::prediction::inter::{Mv, MvdCoding};

/// One answered `read_mv` symbol.
#[derive(Clone, Copy, Debug)]
enum MvStep {
    Symbol(MvCdfSelector, u8),
    /// An `L(n)` literal: `(value, n)`.
    Literal(u32, u32),
}

/// Answers each `read_mv` symbol for the differences `targets` and records it.
struct MvWriter {
    targets: [Mv; 2],
    /// The difference whose magnitude is read.
    vector: usize,
    precision: u8,
    /// The `col_mv_greater` symbols answered `1` for the current magnitude.
    greater: i32,
    steps: Vec<MvStep>,
}

/// The last `shell_class` symbol value; at eighth-pel, shell set `1` codes
/// the last two classes with it and `joint_shell_last_two_classes`.
const LAST_RAW_SHELL_CLASS: u32 = 7;

/// The joint shell coding of one magnitude: `(class, offset, column, low)`,
/// where `low` is the smaller part of the shell split.
fn shell(target: Mv, precision: u8) -> (u32, u32, u32, u32) {
    let shift = u32::from(MV_PRECISION_EIGHTH_PEL.saturating_sub(precision));
    let (row, col) = (
        target.row.unsigned_abs() >> shift,
        target.col.unsigned_abs() >> shift,
    );
    let shell = row + col;
    let class = if shell < 2 { 0 } else { shell.ilog2() };
    let base = if class == 0 { 0 } else { 1 << class };
    (class, shell - base, col, col.min(row))
}

impl MvWriter {
    fn bit(&mut self, value: bool) -> u8 {
        self.steps.push(MvStep::Literal(u32::from(value), 1));
        u8::from(value)
    }
}

impl MvSymbols for MvWriter {
    fn symbol(
        &mut self,
        _cdfs: &mut TileCdfSubset,
        selector: MvCdfSelector,
        _tile_offset: ByteOffset,
    ) -> crate::Result<u8> {
        let (class, offset, col, low) = shell(self.targets[self.vector], self.precision);
        let set0 = shell_set0_classes(self.precision).unsigned_abs();
        let value = match selector {
            MvCdfSelector::JointShellSet { .. } => u32::from(class >= set0),
            MvCdfSelector::JointShellClass { shell_set, .. } => class
                .saturating_sub(set0 * shell_set as u32)
                .min(LAST_RAW_SHELL_CLASS),
            MvCdfSelector::JointShellLastTwo { .. } => {
                class.saturating_sub(set0 + LAST_RAW_SHELL_CLASS)
            }
            MvCdfSelector::ShellOffsetLowClass { .. } => offset,
            MvCdfSelector::ShellOffsetClass2 { .. } => u32::from(offset > 0),
            MvCdfSelector::ShellOffsetOtherClass { i, .. } => (offset >> i) & 1,
            MvCdfSelector::ColMvGreater { i, .. } => {
                let greater = low > i as u32;
                self.greater += i32::from(greater);
                u32::from(greater)
            }
            MvCdfSelector::ColMvIndex { .. } => u32::from(col != low),
            MvCdfSelector::AmvdJoint => {
                let target = self.targets[self.vector];
                2 * u32::from(target.row != 0) + u32::from(target.col != 0)
            }
            MvCdfSelector::AmvdIndex { comp } => {
                let target = self.targets[self.vector];
                let magnitude = if comp == 0 { target.row } else { target.col }.abs();
                AMVD_INDEX_TO_MVD
                    .iter()
                    .position(|&step| step == magnitude)
                    .and_then(|index| (index as u32).checked_sub(1))
                    .unwrap_or(u32::MAX)
            }
        };
        let value = u8::try_from(value).unwrap_or(u8::MAX);
        self.steps.push(MvStep::Symbol(selector, value));
        Ok(value)
    }

    fn class2_bit(&mut self, i: i32, _tile_offset: ByteOffset) -> crate::Result<u8> {
        let (_, offset, _, _) = shell(self.targets[self.vector], self.precision);
        Ok(self.bit(i64::from(offset) > i64::from(i)))
    }

    /// `ns(n)` (§ 4.10.10) of the part of `low` the greater symbols left.
    fn ns(&mut self, n: i32, _tile_offset: ByteOffset) -> crate::Result<i32> {
        let (_, _, _, low) = shell(self.targets[self.vector], self.precision);
        let value = i32::try_from(low).unwrap_or(i32::MAX) - self.greater;
        if n > 1 {
            let (n, v) = (n.unsigned_abs(), value.unsigned_abs());
            let w = n.ilog2() + 1;
            let m = (1 << w) - n;
            if v < m {
                self.steps.push(MvStep::Literal(v, w - 1));
            } else {
                self.steps.push(MvStep::Literal((v + m) >> 1, w - 1));
                self.steps.push(MvStep::Literal((v + m) & 1, 1));
            }
        }
        Ok(value)
    }

    fn sign(
        &mut self,
        vector: usize,
        component: usize,
        _tile_offset: ByteOffset,
    ) -> crate::Result<u8> {
        let target = self.targets.get(vector).copied().unwrap_or_default();
        let value = if component == 0 {
            target.row
        } else {
            target.col
        };
        Ok(self.bit(value < 0))
    }
}

/// Writes the differences `mvds` (one or two) read with `coding` into `cdfs`
/// and `sink` as `read_mv` reads them: every magnitude, then every sign bit
/// the decoder does not derive. `journal` logs each CDF row before it adapts.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Unsupported`] for differences `read_mv`
/// cannot code, and [`EncoderSyntaxError::Rule`] for a failed read or write.
pub(super) fn write_mvds(
    (cdfs, journal): (&mut TileCdfSubset, &mut CdfJournal),
    coding: MvdCoding,
    mvds: &[MotionVector],
    sink: &mut impl SymbolSink,
) -> Result<()> {
    if mvds.is_empty() {
        return Ok(());
    }
    if mvds.len() > 2 {
        return Err(EncoderSyntaxError::Unsupported(
            "more than two motion vector differences",
        ));
    }
    let mut targets = [Mv::default(); 2];
    for (target, &mvd) in targets.iter_mut().zip(mvds) {
        *target = mvd.into();
    }
    let amvd = |d: i32| d == 0 || AMVD_INDEX_TO_MVD.contains(&d.abs());
    if coding.use_amvd && !targets.iter().all(|t| amvd(t.row) && amvd(t.col)) {
        return Err(EncoderSyntaxError::Unsupported(
            "an adaptive-MVD difference off the AMVD steps",
        ));
    }
    let precision = coding.precision;
    let mut writer = MvWriter {
        targets,
        vector: 0,
        precision,
        greater: 0,
        steps: Vec::new(),
    };
    let config = MvReadConfig::inter(precision);
    let offset = ByteOffset::new(0);
    let mut magnitudes = [Mv::default(); 2];
    for (vector, magnitude) in magnitudes.iter_mut().take(mvds.len()).enumerate() {
        writer.vector = vector;
        writer.greater = 0;
        *magnitude = if coding.use_amvd {
            read_newmv_amvd_block_mvd(cdfs, &mut writer, offset)
        } else {
            read_newmv_block_mvd_magnitude_with_config(cdfs, &mut writer, offset, config)
        }
        .map_err(rule("motion vector difference"))?;
    }
    let derive = (coding.derive_sign, coding.derive_threshold);
    let read = if mvds.len() == 1 {
        [
            apply_inter_mvd_signs(
                magnitudes[0],
                &mut writer,
                offset,
                config,
                derive.0,
                derive.1,
            ),
            Ok(Mv::default()),
        ]
    } else {
        let pair = apply_inter_mvd_sign_pair(
            magnitudes[0],
            magnitudes[1],
            &mut writer,
            offset,
            config,
            derive.0,
            derive.1,
        );
        match pair {
            Ok((first, second)) => [Ok(first), Ok(second)],
            Err(error) => return Err(rule("motion vector sign")(error)),
        }
    };
    for (read, target) in read.into_iter().zip(targets) {
        if read.map_err(rule("motion vector sign"))? != target {
            return Err(EncoderSyntaxError::Unsupported(
                "a motion vector difference read_mv cannot code",
            ));
        }
    }
    let (base, _) = cdfs.rows_span();
    for step in writer.steps {
        match step {
            MvStep::Symbol(selector, value) => {
                let selector = TileCdfSelector::ReadMv(selector);
                let row = cdfs.row_mut(selector).map_err(rule("CDF row"))?;
                journal.log(base, CdfRow::Tile(selector), row);
                sink.symbol(row, Symbol::new(value))
            }
            MvStep::Literal(value, bits) => sink.literal(value, bits),
        }
        .map_err(rule("motion vector write"))?;
    }
    Ok(())
}

impl IntraTileSyntax {
    /// Writes the motion vector differences `mvds` of the planned inter leaf
    /// in coding order, as [`Self::write_inter_modes`] returns them.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for differences `read_mv` cannot code,
    /// including a derived sign the target does not have, or a write failure.
    pub fn write_mvds(&mut self, mvds: &[MotionVector], sink: &mut impl SymbolSink) -> Result<()> {
        let state = self.inter_ref()?;
        let motion = state
            .planned_motion()
            .ok_or(EncoderSyntaxError::Unsupported("no planned inter leaf"))?;
        let coding = state
            .tile
            .mvd_coding(motion)
            .map_err(rule("motion vector difference"))?;
        write_mvds((&mut self.cdfs, &mut self.journal), coding, mvds, sink)
    }
}

#[cfg(test)]
#[path = "mv_tests.rs"]
mod tests;
