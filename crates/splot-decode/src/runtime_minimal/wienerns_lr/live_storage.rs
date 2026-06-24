// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Live Wiener NS loop-restoration storage allocation shells.

use splot_recon::{BitDepth, ReconError};

use crate::error::{DecodeError, Result};

use super::{
    LR_RETAINED_FRAME_BUFFERS, WienerNsLrRuntimeStorageRetentionFrontier, WienerNsLrTxSkipGrid,
    decoded_storage_bytes_per_sample, source_read_arithmetic_overflow,
};

pub(in crate::runtime_minimal) const LR_LIVE_FRAME_SAMPLE_STORAGE_BYTES: u64 =
    core::mem::size_of::<Option<u16>>() as u64;
pub(in crate::runtime_minimal) const LR_LIVE_TX_SKIP_STORAGE_BYTES_PER_VALUE: u64 =
    core::mem::size_of::<Option<u8>>() as u64;

// This proof shell keeps explicit "missing" state while the runtime still fails
// closed before tile reconstruction populates samples. `Option<u16>` is
// intentionally bit-depth-agnostic: it can carry 8/10/12-bit decoded samples,
// and the retention frontier charges the current slot size before allocation.
// Before this storage becomes populated retained state, replace these slots with
// packed presence masks plus typed value buffers.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private live storage-allocation proof waits for tile reconstruction to populate values"
    )
)]
#[derive(Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct WienerNsLrLiveStorageAllocation {
    bit_depth: BitDepth,
    curr_frame: WienerNsLrLiveFrameBuffer,
    cdef_frame: WienerNsLrLiveFrameBuffer,
    tx_skip_grid: WienerNsLrLiveTxSkipGrid,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private live storage-allocation proof waits for tile reconstruction to populate values"
    )
)]
impl WienerNsLrLiveStorageAllocation {
    pub(in crate::runtime_minimal) fn from_retention_frontier(
        frontier: WienerNsLrRuntimeStorageRetentionFrontier,
    ) -> Result<Self> {
        if frontier.frame_buffer_count != LR_RETAINED_FRAME_BUFFERS {
            return Err(source_read_arithmetic_overflow(
                "wiener ns lr live frame-buffer count",
            ));
        }
        let bytes_per_sample = decoded_storage_bytes_per_sample(frontier.bit_depth);
        if !frontier.frame_buffer_bytes.is_multiple_of(bytes_per_sample) {
            return Err(source_read_arithmetic_overflow(
                "wiener ns lr live frame-buffer byte alignment",
            ));
        }
        let frame_sample_count = storage_u64_to_usize(
            frontier.frame_buffer_bytes / bytes_per_sample,
            "wiener ns lr live frame-buffer samples",
        )?;
        let curr_frame = WienerNsLrLiveFrameBuffer::new(frontier.bit_depth, frame_sample_count)?;
        let cdef_frame = WienerNsLrLiveFrameBuffer::new(frontier.bit_depth, frame_sample_count)?;
        let tx_skip_grid = WienerNsLrLiveTxSkipGrid::new(
            frontier.tx_skip_rows,
            frontier.tx_skip_cols,
            storage_u64_to_usize(frontier.tx_skip_values, "wiener ns lr live tx-skip values")?,
        )?;

        Ok(Self {
            bit_depth: frontier.bit_depth,
            curr_frame,
            cdef_frame,
            tx_skip_grid,
        })
    }

    pub(in crate::runtime_minimal) const fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    pub(in crate::runtime_minimal) fn frame_sample_count(&self) -> usize {
        self.curr_frame.sample_count()
    }

    pub(in crate::runtime_minimal) fn unpopulated_frame_samples(&self) -> usize {
        self.curr_frame
            .unpopulated_samples()
            .saturating_add(self.cdef_frame.unpopulated_samples())
    }

    pub(in crate::runtime_minimal) fn tx_skip_dimensions(&self) -> (usize, usize) {
        (self.tx_skip_grid.rows, self.tx_skip_grid.cols)
    }

    pub(in crate::runtime_minimal) fn tx_skip_value_count(&self) -> usize {
        self.tx_skip_grid.value_count()
    }

    pub(in crate::runtime_minimal) fn unpopulated_tx_skip_values(&self) -> usize {
        self.tx_skip_grid.unpopulated_values()
    }

    pub(in crate::runtime_minimal) fn populate_tx_skip_grid(
        &mut self,
        grid: &WienerNsLrTxSkipGrid,
    ) -> Result<()> {
        self.tx_skip_grid.populate_from_retained_grid(grid)
    }

    pub(in crate::runtime_minimal) fn tx_skip_value(&self, row: usize, col: usize) -> Option<u8> {
        self.tx_skip_grid.value(row, col)
    }

    pub(in crate::runtime_minimal) fn is_fully_populated(&self) -> bool {
        self.curr_frame.is_fully_populated()
            && self.cdef_frame.is_fully_populated()
            && self.tx_skip_grid.is_fully_populated()
    }
}

#[derive(Debug, Eq, PartialEq)]
struct WienerNsLrLiveFrameBuffer {
    samples: Vec<Option<u16>>,
    populated: usize,
}

impl WienerNsLrLiveFrameBuffer {
    fn new(bit_depth: BitDepth, sample_count: usize) -> Result<Self> {
        let _ = bit_depth;
        Ok(Self {
            samples: unpopulated_vec(sample_count, "wiener ns lr live frame-buffer allocation")?,
            populated: 0,
        })
    }

    fn sample_count(&self) -> usize {
        self.samples.len()
    }

    fn unpopulated_samples(&self) -> usize {
        self.samples.len().saturating_sub(self.populated)
    }

    fn is_fully_populated(&self) -> bool {
        self.populated == self.samples.len()
    }
}

#[derive(Debug, Eq, PartialEq)]
struct WienerNsLrLiveTxSkipGrid {
    rows: usize,
    cols: usize,
    values: Vec<Option<u8>>,
    populated: usize,
}

impl WienerNsLrLiveTxSkipGrid {
    fn new(rows: usize, cols: usize, value_count: usize) -> Result<Self> {
        if rows == 0 || cols == 0 {
            return Err(source_read_arithmetic_overflow(
                "wiener ns lr live tx-skip dimensions",
            ));
        }
        let expected = rows.checked_mul(cols).ok_or_else(|| {
            source_read_arithmetic_overflow("wiener ns lr live tx-skip sample count")
        })?;
        if value_count != expected {
            return Err(source_read_arithmetic_overflow(
                "wiener ns lr live tx-skip value count",
            ));
        }
        Ok(Self {
            rows,
            cols,
            values: unpopulated_vec(value_count, "wiener ns lr live tx-skip allocation")?,
            populated: 0,
        })
    }

    fn value_count(&self) -> usize {
        self.values.len()
    }

    fn unpopulated_values(&self) -> usize {
        self.values.len().saturating_sub(self.populated)
    }

    fn populate_from_retained_grid(&mut self, grid: &WienerNsLrTxSkipGrid) -> Result<()> {
        if self.rows != grid.rows || self.cols != grid.cols {
            return Err(live_tx_skip_invalid("wiener ns lr live tx-skip dimensions"));
        }
        if self.populated != 0 {
            return Err(live_tx_skip_invalid(
                "wiener ns lr live tx-skip already populated",
            ));
        }
        if self.values.len() != grid.values.len() {
            return Err(live_tx_skip_invalid(
                "wiener ns lr live tx-skip value count",
            ));
        }
        for (slot, value) in self.values.iter_mut().zip(grid.values.iter().copied()) {
            *slot = Some(value);
        }
        self.populated = self.values.len();
        Ok(())
    }

    fn value(&self, row: usize, col: usize) -> Option<u8> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let index = row.checked_mul(self.cols)?.checked_add(col)?;
        self.values.get(index).copied().flatten()
    }

    fn is_fully_populated(&self) -> bool {
        self.populated == self.values.len()
    }
}

fn live_tx_skip_invalid(field: &'static str) -> DecodeError {
    DecodeError::Reconstruction {
        source: ReconError::PcWienerInvalidBounds { field },
    }
}

fn storage_u64_to_usize(value: u64, context: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| source_read_arithmetic_overflow(context))
}

fn unpopulated_vec<T: Clone>(len: usize, context: &'static str) -> Result<Vec<Option<T>>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(len)
        .map_err(|_| source_read_arithmetic_overflow(context))?;
    values.resize(len, None);
    Ok(values)
}
