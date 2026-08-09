// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode-side coefficient scan traversal helpers.
//!
//! Feature tracking: `DECODE-COEFF-SCAN-WALK`,
//! `DECODE-COEFF-FSC-SCAN-WALK`, and
//! `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER`.

use super::CoeffLoopContextError;
use super::branch::NonZeroCoeffBlockStart;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffScanEntry {
    scan_index: usize,
    pos: usize,
    row: usize,
    col: usize,
}

impl CoeffScanEntry {
    pub(crate) const fn new(scan_index: usize, pos: usize, row: usize, col: usize) -> Self {
        Self {
            scan_index,
            pos,
            row,
            col,
        }
    }

    #[must_use]
    pub(crate) const fn scan_index(self) -> usize {
        self.scan_index
    }

    #[must_use]
    pub(crate) const fn pos(self) -> usize {
        self.pos
    }

    #[must_use]
    pub(crate) const fn row(self) -> usize {
        self.row
    }

    #[must_use]
    pub(crate) const fn col(self) -> usize {
        self.col
    }
}

#[derive(Debug)]
pub(crate) struct NonZeroCoeffScanWalk<'a> {
    scan: &'a [u16],
    width: usize,
}

impl NonZeroCoeffScanWalk<'_> {
    #[must_use]
    pub(crate) fn entries(&self) -> impl ExactSizeIterator<Item = CoeffScanEntry> + '_ {
        (0..self.len()).map(|index| self.entry(index))
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.scan.len()
    }

    fn entry(&self, index: usize) -> CoeffScanEntry {
        let scan_index = self.scan.len() - index - 1;
        let pos = usize::from(self.scan[scan_index]);
        CoeffScanEntry::new(scan_index, pos, pos / self.width, pos % self.width)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FscCoeffScanWalk {
    scan: &'static [u16],
    width: usize,
    bob: usize,
    seg_eob: usize,
}

impl FscCoeffScanWalk {
    #[must_use]
    pub(crate) const fn bob(&self) -> usize {
        self.bob
    }

    #[must_use]
    pub(crate) const fn seg_eob(&self) -> usize {
        self.seg_eob
    }

    #[must_use]
    pub(crate) fn entries(&self) -> impl ExactSizeIterator<Item = CoeffScanEntry> + '_ {
        (0..self.len()).map(|index| self.entry(index))
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.scan.len()
    }

    fn entry(&self, index: usize) -> CoeffScanEntry {
        let pos = usize::from(self.scan[index]);
        CoeffScanEntry::new(self.bob + index, pos, pos / self.width, pos % self.width)
    }
}

pub(crate) fn walk_nonzero_coeff_scan<'a>(
    start: &NonZeroCoeffBlockStart,
    scan: &'a [u16],
) -> Result<NonZeroCoeffScanWalk<'a>, CoeffLoopContextError> {
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
    let coeff_count = block.coeff_count();
    for (scan_index, scan_pos) in scan[..eob].iter().copied().enumerate().rev() {
        let pos = usize::from(scan_pos);
        if pos >= coeff_count {
            return Err(CoeffLoopContextError::ScanWalkPositionOutOfRange {
                scan_index,
                pos,
                coeff_count,
            });
        }
    }

    Ok(NonZeroCoeffScanWalk {
        scan: &scan[..eob],
        width,
    })
}

pub(crate) fn walk_fsc_coeff_scan(
    start: &NonZeroCoeffBlockStart,
    seg_eob: usize,
    scan: &'static [u16],
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

    let bob = seg_eob - eob;
    let block = start.block();
    let width = block.width();
    let coeff_count = block.coeff_count();
    let scan = &scan[bob..seg_eob];
    for (offset, scan_pos) in scan.iter().copied().enumerate() {
        let pos = usize::from(scan_pos);
        if pos >= coeff_count {
            return Err(CoeffLoopContextError::ScanWalkPositionOutOfRange {
                scan_index: bob + offset,
                pos,
                coeff_count,
            });
        }
    }

    Ok(FscCoeffScanWalk {
        scan,
        width,
        bob,
        seg_eob,
    })
}
