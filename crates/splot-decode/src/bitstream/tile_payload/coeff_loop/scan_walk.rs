// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode-side coefficient scan traversal helpers.
//!
//! Feature tracking: `DECODE-COEFF-SCAN-WALK`.

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
