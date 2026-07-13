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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffLevelState {
    eob_read: NonZeroCoeffEobSymbolRead,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffLevelState {
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }

    pub(crate) fn into_parts(self) -> (NonZeroCoeffEobSymbolRead, TransformCoeffBlockState) {
        (self.eob_read, self.block)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffLevelStateWriteError {
    #[error("coefficient level write read count {reads} does not match scan entries {entries}")]
    ReadCountMismatch { reads: usize, entries: usize },
    #[error(
        "coefficient level write read {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    ScanEntryMismatch {
        index: usize,
        expected: CoeffScanEntry,
        actual: CoeffScanEntry,
    },
    #[error("coefficient level write state error: {0}")]
    State(#[from] TileCoeffStateError),
}

pub(crate) fn apply_nonzero_coeff_base_levels(
    start: NonZeroCoeffBlockStart,
    walk: &NonZeroCoeffScanWalk<'_>,
    reads: &[CoeffBaseSymbolRead],
) -> Result<NonZeroCoeffLevelState, CoeffLevelStateWriteError> {
    let (eob_read, mut block) = start.into_parts();
    if reads.len() != walk.len() {
        return Err(CoeffLevelStateWriteError::ReadCountMismatch {
            reads: reads.len(),
            entries: walk.len(),
        });
    }

    preflight_level_writes(&block, walk, reads)?;
    for (entry, read) in walk.entries().zip(reads.iter().copied()) {
        block.set_level(entry.row(), entry.col(), read.level())?;
    }

    Ok(NonZeroCoeffLevelState { eob_read, block })
}

fn preflight_level_writes(
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    reads: &[CoeffBaseSymbolRead],
) -> Result<(), CoeffLevelStateWriteError> {
    for (index, (entry, read)) in walk.entries().zip(reads.iter().copied()).enumerate() {
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
