// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{
    LoopRestorationSourceBounds, PcWienerTxSkipLookup, ReconError, Result as ReconResult,
};

const MI_SIZE: usize = 4;

/// Grid sentinel for a cell no transform record has covered yet. Cell values are
/// `u8::from(bool)`, so this cannot collide with a written 0 or 1.
const WIENERNS_LR_TX_SKIP_UNWRITTEN: u8 = u8::MAX;

mod diagnostics;
pub(crate) mod intrabc_records;
pub(crate) mod intrabc_ref_mv_stack;
pub(crate) mod recon;
pub(crate) mod tx_records;

pub(crate) use self::recon::chroma_transform_deblock_block;

pub(crate) fn recon_final_filter_sink<T: splot_recon::ReconSample>(
    workspace: splot_recon::CurrentFrameWorkspace<T>,
    luma_width: usize,
    luma_height: usize,
    bit_depth: splot_recon::BitDepth,
) -> recon::WienerNsLrReconSink<T> {
    recon::WienerNsLrReconSink::for_final_filtering(workspace, luma_width, luma_height, bit_depth)
}
pub(crate) use self::tx_records::WienerNsLrTxSkipTransformRecord;

#[derive(Default)]
pub(crate) struct FrameFilterRecords {
    pub(crate) deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    pub(crate) chroma_deblock_blocks: crate::filters::deblock::ChromaDeblockRecords,
    pub(crate) tx_skip_records: Vec<WienerNsLrTxSkipTransformRecord>,
    pub(crate) lr_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    pub(crate) lr_unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
}

impl FrameFilterRecords {
    /// Moves every record out of `other` and onto the end of these lists.
    pub(crate) fn append(&mut self, other: &mut Self) {
        self.deblock_blocks.append(&mut other.deblock_blocks);
        self.chroma_deblock_blocks
            .append(&mut other.chroma_deblock_blocks);
        self.tx_skip_records.append(&mut other.tx_skip_records);
        self.lr_source_blocks.append(&mut other.lr_source_blocks);
        self.lr_unit_filters.append(&mut other.lr_unit_filters);
    }

    pub(crate) fn clear(&mut self) {
        self.deblock_blocks.clear();
        self.chroma_deblock_blocks.clear();
        self.tx_skip_records.clear();
        self.lr_source_blocks.clear();
        self.lr_unit_filters.clear();
        debug_assert!(
            self.deblock_blocks.is_empty()
                && self.chroma_deblock_blocks.is_empty()
                && self.tx_skip_records.is_empty()
                && self.lr_source_blocks.is_empty()
                && self.lr_unit_filters.is_empty()
        );
    }
}

pub(crate) use self::diagnostics::{
    intra_capped_seq_sb_size, selectable_missing_quantization_error, selectable_symbol_read_error,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrTxSkipLookup {
    pub(crate) row: usize,
    pub(crate) col: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrTxSkipGrid {
    rows: usize,
    cols: usize,
    values: Vec<u8>,
}

impl WienerNsLrTxSkipGrid {
    pub(crate) fn new(rows: usize, cols: usize, values: Vec<u8>) -> ReconResult<Self> {
        let expected = wienerns_lr_tx_skip_grid_len(rows, cols)?;
        if values.len() != expected {
            return Err(ReconError::BufferLengthMismatch {
                expected,
                actual: values.len(),
            });
        }
        Ok(Self { rows, cols, values })
    }

    pub(crate) fn lookup(&self, lookup: WienerNsLrTxSkipLookup) -> ReconResult<i32> {
        if lookup.row >= self.rows || lookup.col >= self.cols {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LrTxSkip grid lookup",
            });
        }
        let index = wienerns_lr_tx_skip_grid_index(lookup.row, lookup.col, self.cols)?;
        let Some(value) = self.values.get(index) else {
            return Err(ReconError::BufferLengthMismatch {
                expected: index.saturating_add(1),
                actual: self.values.len(),
            });
        };
        Ok(i32::from(*value))
    }
}

fn wienerns_lr_tx_skip_grid_len(rows: usize, cols: usize) -> ReconResult<usize> {
    if rows == 0 || cols == 0 {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip grid dimensions",
        });
    }
    rows.checked_mul(cols)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "LrTxSkip grid sample count",
        })
}

fn wienerns_lr_tx_skip_grid_index(row: usize, col: usize, cols: usize) -> ReconResult<usize> {
    let row_start = row
        .checked_mul(cols)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "LrTxSkip grid row offset",
        })?;
    row_start
        .checked_add(col)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "LrTxSkip grid sample offset",
        })
}

/// § 7.18.1 CDEF skip grid: the `Skips` array, which is `skip_txfm` alone.
///
/// The CDEF block process reads `Skips[r][c] && Skips[r + 1][c] &&
/// Skips[r][c + 1] && Skips[r + 1][c + 1]` when `cdef_on_skip_txfm_frame_enable`
/// is 0. That is a different array from loop restoration's `LrTxSkip`, which
/// additionally treats an empty transform (`eob == 0`) as skipped. Feeding the
/// loop-restoration predicate to CDEF marks extra 8x8 units as fully skipped and
/// drops filtering the spec applies.
pub(crate) fn derive_cdef_skip_grid(
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
) -> ReconResult<crate::filters::cdef::CdefSkipGrid> {
    let expected = wienerns_lr_tx_skip_grid_len(rows, cols)?;
    let mut values = vec![false; expected];
    let mut written = vec![false; expected];
    let mut populated = 0usize;
    for record in records {
        write_cdef_skip_record(
            rows,
            cols,
            record,
            &mut values,
            &mut written,
            &mut populated,
        )?;
    }
    if populated != expected {
        return Err(ReconError::BufferLengthMismatch {
            expected,
            actual: populated,
        });
    }
    crate::filters::cdef::CdefSkipGrid::new(rows, cols, values).map_err(|_| {
        ReconError::PcWienerInvalidBounds {
            field: "CDEF skip grid dimensions",
        }
    })
}

fn write_cdef_skip_record(
    rows: usize,
    cols: usize,
    record: &WienerNsLrTxSkipTransformRecord,
    values: &mut [bool],
    written: &mut [bool],
    populated: &mut usize,
) -> ReconResult<()> {
    for_each_wienerns_lr_tx_skip_record_cell(rows, cols, record, |index| {
        let actual = values.len().min(written.len());
        let Some((value, was_written)) = values.get_mut(index).zip(written.get_mut(index)) else {
            return Err(ReconError::BufferLengthMismatch {
                expected: index.saturating_add(1),
                actual,
            });
        };
        if !*was_written {
            *value = record.skip_flag;
            *was_written = true;
            *populated = populated
                .checked_add(1)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "CDEF skip populated sample count",
                })?;
        } else if *value != record.skip_flag {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LrTxSkip conflicting transform records",
            });
        }
        Ok(())
    })
}

pub(crate) fn derive_wienerns_lr_tx_skip_grid_retention(
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
) -> ReconResult<WienerNsLrTxSkipGrid> {
    derive_wienerns_lr_skip_grid(rows, cols, records, |record| {
        record.skip_flag || record.eob == 0
    })
}

fn derive_wienerns_lr_skip_grid(
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
    value_of: impl Fn(&WienerNsLrTxSkipTransformRecord) -> bool,
) -> ReconResult<WienerNsLrTxSkipGrid> {
    let expected = wienerns_lr_tx_skip_grid_len(rows, cols)?;
    let mut values = vec![WIENERNS_LR_TX_SKIP_UNWRITTEN; expected];
    let mut populated = 0usize;
    for record in records {
        let value = u8::from(value_of(record));
        write_wienerns_lr_tx_skip_record(rows, cols, record, value, &mut values, &mut populated)?;
    }
    if populated != expected {
        return Err(ReconError::BufferLengthMismatch {
            expected,
            actual: populated,
        });
    }
    WienerNsLrTxSkipGrid::new(rows, cols, values)
}

fn write_wienerns_lr_tx_skip_record(
    rows: usize,
    cols: usize,
    record: &WienerNsLrTxSkipTransformRecord,
    value: u8,
    values: &mut [u8],
    populated: &mut usize,
) -> ReconResult<()> {
    for_each_wienerns_lr_tx_skip_record_cell(rows, cols, record, |index| {
        let actual = values.len();
        let Some(slot) = values.get_mut(index) else {
            return Err(ReconError::BufferLengthMismatch {
                expected: index.saturating_add(1),
                actual,
            });
        };
        if *slot == WIENERNS_LR_TX_SKIP_UNWRITTEN {
            *slot = value;
            *populated = populated
                .checked_add(1)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "LrTxSkip populated sample count",
                })?;
        } else if *slot != value {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LrTxSkip conflicting transform records",
            });
        }
        Ok(())
    })
}

fn for_each_wienerns_lr_tx_skip_record_cell(
    rows: usize,
    cols: usize,
    record: &WienerNsLrTxSkipTransformRecord,
    mut visit: impl FnMut(usize) -> ReconResult<()>,
) -> ReconResult<()> {
    if record.rows == 0 || record.cols == 0 {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip transform record dimensions",
        });
    }
    let nominal_end_row =
        record
            .row
            .checked_add(record.rows)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "LrTxSkip transform record row extent",
            })?;
    let nominal_end_col =
        record
            .col
            .checked_add(record.cols)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "LrTxSkip transform record column extent",
            })?;
    if record.row >= rows || record.col >= cols {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip transform record bounds",
        });
    }
    let end_row = nominal_end_row.min(rows);
    let end_col = nominal_end_col.min(cols);

    for row in record.row..end_row {
        for col in record.col..end_col {
            let index = wienerns_lr_tx_skip_grid_index(row, col, cols)?;
            visit(index)?;
        }
    }
    Ok(())
}

pub(crate) fn fixed_largest_420_chroma_tx_size_from_luma_4x4(
    n4w: usize,
    n4h: usize,
) -> Option<usize> {
    if n4w < 2 || n4h < 2 || !n4w.is_power_of_two() || !n4h.is_power_of_two() {
        return None;
    }
    let luma_w_log2 = n4w.trailing_zeros().checked_add(2)?;
    let luma_h_log2 = n4h.trailing_zeros().checked_add(2)?;
    tx_size_from_log2(luma_w_log2.checked_sub(1)?, luma_h_log2.checked_sub(1)?)
}

fn tx_size_from_log2(w_log2: u32, h_log2: u32) -> Option<usize> {
    let w = i32::try_from(w_log2).ok()?;
    let h = i32::try_from(h_log2).ok()?;
    TX_WIDTH_LOG2.iter().enumerate().find_map(|(tx_size, &tw)| {
        (tw == w && TX_HEIGHT_LOG2.get(tx_size).copied() == Some(h)).then_some(tx_size)
    })
}

pub(crate) fn wienerns_lr_source_block_bounds(
    block: &crate::bitstream::tile_payload::WienerNsLrSourceBlock,
    subsampling_x: u8,
    subsampling_y: u8,
) -> LoopRestorationSourceBounds {
    LoopRestorationSourceBounds {
        luma_start_x: block.luma_start_x,
        luma_end_x: block.luma_end_x,
        luma_start_y: block.luma_start_y,
        luma_end_y: block.luma_end_y,
        luma_stripe_start_y: block.luma_stripe_start_y,
        luma_stripe_end_y: block.luma_stripe_end_y,
        subsampling_x,
        subsampling_y,
    }
}

pub(crate) const fn wienerns_lr_tx_skip_lookup_from_pc(
    lookup: PcWienerTxSkipLookup,
) -> WienerNsLrTxSkipLookup {
    WienerNsLrTxSkipLookup {
        row: lookup.row,
        col: lookup.col,
    }
}

fn pc_wiener_block_end_x(
    block: &crate::bitstream::tile_payload::WienerNsLrSourceBlock,
    block_start_x: usize,
) -> ReconResult<usize> {
    let tile_end_x = block
        .tile_mi_col_end
        .checked_mul(MI_SIZE)
        .and_then(|end| end.checked_sub(1))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "pc wiener classified tile end x",
        })?;
    let block_end_x = block_start_x
        .checked_add(63)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "pc wiener classified block end x",
        })?;
    Ok(tile_end_x.min(block_end_x))
}
