// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode-side coefficient scan traversal helpers.
//!
//! Feature tracking: `DECODE-COEFF-SCAN-WALK`,
//! `DECODE-COEFF-FSC-SCAN-WALK`, and
//! `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER`.

use std::collections::TryReserveError;

use super::CoeffLoopContextError;
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::CoeffTransformClass;

/// One checked § 5.20.7.27 coefficient scan entry.
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

/// Error returned while deriving an AV2 coefficient scan order.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffScanOrderError {
    /// `get_scan(txSz, txClass)` received an unsupported scan extent.
    #[error("coefficient scan order invalid scan shape {width}x{height}")]
    InvalidShape {
        /// Scan width after `Min(Tx_Width[txSz], 32)`.
        width: usize,
        /// Scan height after `Min(Tx_Height[txSz], 32)`.
        height: usize,
    },
    /// Allocation for the derived scan order failed.
    #[error("coefficient scan order allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
}

/// Derives AV2 § 5.20.7.30 `get_scan(txSz, txClass)` output from resolved
/// transform facts (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30`).
pub(crate) fn derive_coeff_scan_order(
    tx_width: usize,
    tx_height: usize,
    tx_class: CoeffTransformClass,
) -> Result<Vec<u16>, CoeffScanOrderError> {
    let width = tx_width.min(32);
    let height = tx_height.min(32);
    if !matches!(width, 4 | 8 | 16 | 32) || !matches!(height, 4 | 8 | 16 | 32) {
        return Err(CoeffScanOrderError::InvalidShape { width, height });
    }

    let coeff_count = width * height;
    let mut out = Vec::new();
    out.try_reserve_exact(coeff_count)?;
    match tx_class {
        CoeffTransformClass::Vertical => {
            for y in 0..height {
                for x in 0..width {
                    out.push((y * width + x) as u16);
                }
            }
        }
        CoeffTransformClass::Horizontal => {
            for x in 0..width {
                for y in 0..height {
                    out.push((y * width + x) as u16);
                }
            }
        }
        CoeffTransformClass::TwoD => {
            let (wi, hi) = (width as i32, height as i32);
            let (mut x, mut y) = (0i32, 0i32);
            for _ in 0..coeff_count {
                out.push((y * wi + x) as u16);
                x += 1;
                y -= 1;
                if y < 0 || x >= wi {
                    x += 1;
                    let s = x.min(hi - 1 - y);
                    x -= s;
                    y += s;
                }
            }
        }
    }
    Ok(out)
}

fn collect_scan_entries<I>(
    start: &NonZeroCoeffBlockStart,
    scan_positions: I,
    capacity: usize,
) -> Result<Vec<CoeffScanEntry>, CoeffLoopContextError>
where
    I: IntoIterator<Item = (usize, u16)>,
{
    let block = start.block();
    let width = block.width();
    let coeff_count = block.level().len();
    let mut entries = Vec::new();
    entries.try_reserve(capacity)?;

    for (scan_index, scan_pos) in scan_positions {
        let pos = usize::from(scan_pos);
        if pos >= coeff_count {
            return Err(CoeffLoopContextError::ScanWalkPositionOutOfRange {
                scan_index,
                pos,
                coeff_count,
            });
        }
        entries.push(CoeffScanEntry::new(
            scan_index,
            pos,
            pos / width,
            pos % width,
        ));
    }

    Ok(entries)
}

/// Walks the ordinary non-FSC AV2 § 5.20.7.27 nonzero coefficient scan window
/// over a caller-supplied scan order
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
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

    let entries = collect_scan_entries(start, scan[..eob].iter().copied().enumerate().rev(), eob)?;

    Ok(NonZeroCoeffScanWalk { entries })
}

/// Walks the FSC/IDTX AV2 § 5.20.7.27 coefficient scan window for a
/// caller-resolved `segEob` and scan order
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
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

    let bob = seg_eob - eob;
    let entries = collect_scan_entries(
        start,
        (bob..seg_eob).zip(scan[bob..seg_eob].iter().copied()),
        eob,
    )?;

    Ok(FscCoeffScanWalk {
        bob,
        seg_eob,
        entries,
    })
}
