// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode-side coefficient scan traversal helpers.
//!
//! Feature tracking: `DECODE-COEFF-SCAN-WALK` and
//! `DECODE-COEFF-FSC-SCAN-WALK`.

use super::CoeffLoopContextError;
use super::branch::NonZeroCoeffBlockStart;

/// One checked § 5.20.7.27 coefficient scan entry.
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

/// Checked FSC/IDTX coefficient scan window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FscCoeffScanWalk {
    bob: usize,
    seg_eob: usize,
    entries: Vec<CoeffScanEntry>,
}

impl FscCoeffScanWalk {
    /// Begin-of-block scan index `bob = segEob - eob`.
    #[must_use]
    pub(crate) const fn bob(&self) -> usize {
        self.bob
    }

    /// Caller-resolved `segEob` from AV2 § 5.20.7.27.
    #[must_use]
    pub(crate) const fn seg_eob(&self) -> usize {
        self.seg_eob
    }

    /// Entries in visited order: `bob`, ..., `segEob - 1`.
    #[must_use]
    pub(crate) fn entries(&self) -> &[CoeffScanEntry] {
        &self.entries
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

/// Walks the FSC/IDTX § 5.20.7.27 coefficient scan window.
///
/// The caller supplies the already-resolved `scan = get_scan(txSz, txClass)` and
/// `segEob = Min(32, Tx_Width[txSz]) * Min(Tx_Height[txSz], 32)`. This helper
/// checks `bob = segEob - eob` and returns forward entries for the spec's
/// `for (c = bob; c < eob; c++)` loop after the FSC branch assigns
/// `eob = segEob` (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
/// It does not read symbols, update CDFs, write coefficient state, or derive
/// `useFsc`.
pub(crate) fn walk_fsc_coeff_scan(
    start: &NonZeroCoeffBlockStart,
    seg_eob: usize,
    scan: &[u16],
) -> Result<FscCoeffScanWalk, CoeffLoopContextError> {
    let eob = start.eob_read().eob().eob();
    if eob == 0 {
        return Err(CoeffLoopContextError::InvalidScanWalkEob { eob });
    }
    if eob > seg_eob {
        return Err(CoeffLoopContextError::FscScanWalkEobOutOfRange { eob, seg_eob });
    }
    if seg_eob > scan.len() {
        return Err(CoeffLoopContextError::ScanWalkEobOutOfRange {
            eob: seg_eob,
            scan_len: scan.len(),
        });
    }

    let block = start.block();
    let width = block.width();
    let coeff_count = block.level().len();
    let bob = seg_eob - eob;
    let mut entries = Vec::new();
    entries.try_reserve(eob)?;

    for (scan_index, &scan_pos) in scan.iter().enumerate().take(seg_eob).skip(bob) {
        let pos = usize::from(scan_pos);
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

    Ok(FscCoeffScanWalk {
        bob,
        seg_eob,
        entries,
    })
}
