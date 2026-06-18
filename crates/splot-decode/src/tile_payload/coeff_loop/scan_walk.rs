// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode-side coefficient scan traversal helpers.
//!
//! Feature tracking: `DECODE-COEFF-SCAN-WALK`.

use super::CoeffLoopContextError;
use super::branch::NonZeroCoeffBlockStart;

/// One checked ordinary non-FSC § 5.20.7.27 coefficient scan entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffScanEntry {
    scan_index: usize,
    pos: usize,
    row: usize,
    col: usize,
}

impl CoeffScanEntry {
    /// Scan index `c` from the § 5.20.7.27 loop.
    #[must_use]
    pub(crate) const fn scan_index(self) -> usize {
        self.scan_index
    }

    /// Raster coefficient position `scan[c]`.
    #[must_use]
    pub(crate) const fn pos(self) -> usize {
        self.pos
    }

    /// Row derived by the spec's `get_tx_row_col` operation.
    #[must_use]
    pub(crate) const fn row(self) -> usize {
        self.row
    }

    /// Column derived by the spec's `get_tx_row_col` operation.
    #[must_use]
    pub(crate) const fn col(self) -> usize {
        self.col
    }

    #[cfg(test)]
    pub(crate) const fn for_test(scan_index: usize, pos: usize, row: usize, col: usize) -> Self {
        Self {
            scan_index,
            pos,
            row,
            col,
        }
    }
}

/// Checked ordinary non-FSC coefficient scan window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffScanWalk {
    entries: Vec<CoeffScanEntry>,
}

impl NonZeroCoeffScanWalk {
    /// Entries in visited order: `eob - 1`, ..., `0`.
    #[must_use]
    pub(crate) fn entries(&self) -> &[CoeffScanEntry] {
        &self.entries
    }

    #[cfg(test)]
    pub(crate) fn from_entries_for_test(entries: Vec<CoeffScanEntry>) -> Self {
        Self { entries }
    }
}

/// Walks the ordinary non-FSC § 5.20.7.27 nonzero coefficient scan window.
///
/// The caller supplies the already-resolved `scan = get_scan(txSz, txClass)`
/// table. This helper only checks the decode-side consumption boundary and maps
/// `scan[c]` raster positions to row/column facts using the initialized adjusted
/// block extent (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
/// It does not derive scan order, read symbols, update CDFs, or write
/// coefficients.
pub(crate) fn walk_nonzero_coeff_scan(
    start: &NonZeroCoeffBlockStart,
    scan: &[u16],
) -> Result<NonZeroCoeffScanWalk, CoeffLoopContextError> {
    let eob = start.eob_read().eob().eob();
    if eob == 0 {
        return Err(CoeffLoopContextError::InvalidScanWalkEob { eob });
    }
    if eob > scan.len() {
        return Err(CoeffLoopContextError::ScanWalkEobOutOfRange {
            eob,
            scan_len: scan.len(),
        });
    }

    let block = start.block();
    let width = block.width();
    let coeff_count = block.level().len();
    let mut entries = Vec::new();
    entries.try_reserve(eob)?;

    for scan_index in (0..eob).rev() {
        let pos = usize::from(scan[scan_index]);
        if pos >= coeff_count {
            return Err(CoeffLoopContextError::ScanWalkPositionOutOfRange {
                scan_index,
                pos,
                coeff_count,
            });
        }
        entries.push(CoeffScanEntry {
            scan_index,
            pos,
            row: pos / width,
            col: pos % width,
        });
    }

    Ok(NonZeroCoeffScanWalk { entries })
}
