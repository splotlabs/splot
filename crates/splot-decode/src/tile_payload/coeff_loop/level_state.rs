// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient level state writes.
//!
//! Feature tracking: `DECODE-COEFF-LEVEL-STATE-WRITE`.

use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEobSymbolRead;
use super::base_symbol::CoeffBaseSymbolRead;
use super::branch::NonZeroCoeffBlockStart;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

/// Local coefficient state after applying decoded ordinary non-FSC levels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffLevelState {
    eob_read: NonZeroCoeffEobSymbolRead,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffLevelState {
    /// Decoded nonzero EOB syntax result carried forward from block start.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    /// Local transform coefficient state after `Level[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }

    /// Consumes the level state into its carried EOB facts and local block.
    pub(crate) fn into_parts(self) -> (NonZeroCoeffEobSymbolRead, TransformCoeffBlockState) {
        (self.eob_read, self.block)
    }
}

/// Error returned by the coefficient level state-write boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffLevelStateWriteError {
    /// The number of decoded level records did not match the checked scan walk.
    #[error("coefficient level write read count {reads} does not match scan entries {entries}")]
    ReadCountMismatch {
        /// Decoded level record count.
        reads: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// One decoded level record was not paired with the matching checked scan entry.
    #[error(
        "coefficient level write read {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    ScanEntryMismatch {
        /// Read index.
        index: usize,
        /// Expected checked scan entry.
        expected: CoeffScanEntry,
        /// Actual decoded read entry.
        actual: CoeffScanEntry,
    },
    /// The local transform-block state rejected a checked coordinate.
    #[error("coefficient level write state error: {0}")]
    State(#[from] TileCoeffStateError),
}

/// Applies decoded ordinary non-FSC §5.20.7.27 levels to local `Level[]` state.
///
/// This starts after nonzero EOB, checked scan traversal, and
/// base/base-range symbol reads. It validates the decoded read records against
/// the checked scan walk, preflights every target coordinate, and then writes
/// `Level[row][col] = level`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). It does not
/// read signs, write `QuantSign[]` or `Quant[]`, run `read_quant`, update tile
/// context lines, or invoke reconstruction.
pub(crate) fn apply_nonzero_coeff_base_levels(
    start: NonZeroCoeffBlockStart,
    walk: &NonZeroCoeffScanWalk,
    reads: &[CoeffBaseSymbolRead],
) -> Result<NonZeroCoeffLevelState, CoeffLevelStateWriteError> {
    let (eob_read, mut block) = start.into_parts();
    let entries = walk.entries();
    if reads.len() != entries.len() {
        return Err(CoeffLevelStateWriteError::ReadCountMismatch {
            reads: reads.len(),
            entries: entries.len(),
        });
    }

    preflight_level_writes(&block, entries, reads)?;
    for read in reads {
        let entry = read.entry();
        block.set_level(entry.row(), entry.col(), read.level())?;
    }

    Ok(NonZeroCoeffLevelState { eob_read, block })
}

fn preflight_level_writes(
    block: &TransformCoeffBlockState,
    entries: &[CoeffScanEntry],
    reads: &[CoeffBaseSymbolRead],
) -> Result<(), CoeffLevelStateWriteError> {
    for (index, (entry, read)) in entries
        .iter()
        .copied()
        .zip(reads.iter().copied())
        .enumerate()
    {
        let actual = read.entry();
        if actual != entry {
            return Err(CoeffLevelStateWriteError::ScanEntryMismatch {
                index,
                expected: entry,
                actual,
            });
        }
        block.level_at(entry.row(), entry.col())?;
    }
    Ok(())
}
