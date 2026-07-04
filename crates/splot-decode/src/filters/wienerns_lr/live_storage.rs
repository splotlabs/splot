// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{BitDepth, ReconError};

use crate::error::{DecodeError, Result};

use super::{
    LR_RETAINED_FRAME_BUFFERS, WienerNsLrRuntimeStorageRetentionFrontier, WienerNsLrTxSkipGrid,
    decoded_storage_bytes_per_sample, source_read_arithmetic_overflow,
};

pub(crate) const LR_LIVE_FRAME_SAMPLE_STORAGE_BYTES: u64 =
    core::mem::size_of::<Option<u16>>() as u64;
pub(crate) const LR_LIVE_TX_SKIP_STORAGE_BYTES_PER_VALUE: u64 =
    core::mem::size_of::<Option<u8>>() as u64;

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrLiveStorageAllocation {
    bit_depth: BitDepth,
    curr_frame: LiveSlots<u16>,
    cdef_frame: LiveSlots<u16>,
    tx_skip_grid: WienerNsLrLiveTxSkipGrid,
}

#[cfg_attr(not(test), allow(dead_code))]
impl WienerNsLrLiveStorageAllocation {
    pub(crate) fn from_retention_frontier(
        frontier: WienerNsLrRuntimeStorageRetentionFrontier,
    ) -> Result<Self> {
        if frontier.frame_buffer_count != LR_RETAINED_FRAME_BUFFERS {
            return Err(source_read_arithmetic_overflow(
                "wiener ns lr live frame-buffer count",
            ));
        }
        let frame_sample_count =
            live_frame_sample_count(frontier.frame_buffer_bytes, frontier.bit_depth)?;
        let curr_frame = LiveSlots::new(
            frame_sample_count,
            "wiener ns lr live frame-buffer allocation",
        )?;
        let cdef_frame = LiveSlots::new(
            frame_sample_count,
            "wiener ns lr live frame-buffer allocation",
        )?;
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

    pub(crate) const fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    pub(crate) fn frame_sample_count(&self) -> usize {
        self.curr_frame.len()
    }

    pub(crate) fn unpopulated_frame_samples(&self) -> usize {
        self.curr_frame
            .unpopulated()
            .saturating_add(self.cdef_frame.unpopulated())
    }

    pub(crate) fn tx_skip_dimensions(&self) -> (usize, usize) {
        (self.tx_skip_grid.rows, self.tx_skip_grid.cols)
    }

    pub(crate) fn tx_skip_value_count(&self) -> usize {
        self.tx_skip_grid.value_count()
    }

    pub(crate) fn unpopulated_tx_skip_values(&self) -> usize {
        self.tx_skip_grid.unpopulated_values()
    }

    pub(crate) fn populate_tx_skip_grid(&mut self, grid: &WienerNsLrTxSkipGrid) -> Result<()> {
        self.tx_skip_grid.populate_from_retained_grid(grid)
    }

    pub(crate) fn tx_skip_value(&self, row: usize, col: usize) -> Option<u8> {
        self.tx_skip_grid.value(row, col)
    }

    pub(crate) fn is_fully_populated(&self) -> bool {
        self.curr_frame.is_fully_populated()
            && self.cdef_frame.is_fully_populated()
            && self.tx_skip_grid.is_fully_populated()
    }
}

#[derive(Debug, Eq, PartialEq)]
struct WienerNsLrLiveTxSkipGrid {
    rows: usize,
    cols: usize,
    values: LiveSlots<u8>,
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
            values: LiveSlots::new(value_count, "wiener ns lr live tx-skip allocation")?,
        })
    }

    fn value_count(&self) -> usize {
        self.values.len()
    }

    fn unpopulated_values(&self) -> usize {
        self.values.unpopulated()
    }

    fn populate_from_retained_grid(&mut self, grid: &WienerNsLrTxSkipGrid) -> Result<()> {
        if self.rows != grid.rows || self.cols != grid.cols {
            return Err(live_tx_skip_invalid("wiener ns lr live tx-skip dimensions"));
        }
        if self.values.has_population() {
            return Err(live_tx_skip_invalid(
                "wiener ns lr live tx-skip already populated",
            ));
        }
        self.values.populate_from_slice(&grid.values);
        Ok(())
    }

    fn value(&self, row: usize, col: usize) -> Option<u8> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let index = row.checked_mul(self.cols)?.checked_add(col)?;
        self.values.value(index)
    }

    fn is_fully_populated(&self) -> bool {
        self.values.is_fully_populated()
    }
}

#[derive(Debug, Eq, PartialEq)]
struct LiveSlots<T> {
    values: Vec<Option<T>>,
    populated: usize,
}

impl<T: Clone> LiveSlots<T> {
    fn new(len: usize, context: &'static str) -> Result<Self> {
        Ok(Self {
            values: unpopulated_vec(len, context)?,
            populated: 0,
        })
    }
}

impl<T> LiveSlots<T> {
    fn len(&self) -> usize {
        self.values.len()
    }

    fn unpopulated(&self) -> usize {
        self.values.len().saturating_sub(self.populated)
    }

    fn has_population(&self) -> bool {
        self.populated != 0
    }

    fn is_fully_populated(&self) -> bool {
        self.populated == self.values.len()
    }
}

impl<T: Copy> LiveSlots<T> {
    fn populate_from_slice(&mut self, values: &[T]) {
        for (slot, value) in self.values.iter_mut().zip(values.iter().copied()) {
            *slot = Some(value);
        }
        self.populated = self.values.len();
    }

    fn value(&self, index: usize) -> Option<T> {
        self.values.get(index).copied().flatten()
    }
}

fn live_tx_skip_invalid(field: &'static str) -> DecodeError {
    DecodeError::Reconstruction {
        source: ReconError::PcWienerInvalidBounds { field },
    }
}

fn live_frame_sample_count(frame_buffer_bytes: u64, bit_depth: BitDepth) -> Result<usize> {
    let bytes_per_sample = decoded_storage_bytes_per_sample(bit_depth);
    if !frame_buffer_bytes.is_multiple_of(bytes_per_sample) {
        return Err(source_read_arithmetic_overflow(
            "wiener ns lr live frame-buffer byte alignment",
        ));
    }
    storage_u64_to_usize(
        frame_buffer_bytes / bytes_per_sample,
        "wiener ns lr live frame-buffer samples",
    )
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
