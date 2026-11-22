// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{LoopRestorationSourceBounds, ReconError, Result as ReconResult};

const MI_SIZE: usize = 4;

/// Grid sentinel for a cell no transform record has covered yet. Cell values are
/// `u8::from(bool)`, so this cannot collide with a written 0 or 1.
const WIENERNS_LR_TX_SKIP_UNWRITTEN: u8 = u8::MAX;

mod diagnostics;
pub(crate) mod intrabc_mv_valid;
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

#[derive(Clone, Copy)]
pub(crate) struct FilterRecordNeeds {
    pub(crate) deblock: bool,
    pub(crate) tx_skip: bool,
}

impl FilterRecordNeeds {
    pub(crate) const ALL: Self = Self {
        deblock: true,
        tx_skip: true,
    };

    pub(crate) fn from_frame(core: &splot_core::headers::frame::FrameHeaderCore) -> Self {
        Self {
            deblock: core
                .deblocking_filter_params
                .as_ref()
                .is_some_and(|filter| filter.apply_deblocking_filter != [false; 4])
                || core
                    .lossless_info
                    .as_ref()
                    .is_some_and(|info| info.has_lossless_segment),
            tx_skip: cdef_uses_tx_skip(core) || lr_uses_tx_skip(core),
        }
    }

    pub(crate) const fn any(self) -> bool {
        self.deblock || self.tx_skip
    }

    pub(crate) fn record_luma(
        self,
        blocks: &mut Vec<crate::filters::deblock::DeblockBlock>,
        skips: &mut Vec<WienerNsLrTxSkipTransformRecord>,
        block: crate::filters::deblock::DeblockBlock,
        eob: usize,
    ) {
        if self.tx_skip {
            skips.push(WienerNsLrTxSkipTransformRecord::from_luma(&block, eob));
        }
        if self.deblock {
            blocks.push(block);
        }
    }
}

impl Default for FilterRecordNeeds {
    fn default() -> Self {
        Self::ALL
    }
}

fn cdef_uses_tx_skip(core: &splot_core::headers::frame::FrameHeaderCore) -> bool {
    core.cdef_params
        .as_ref()
        .is_some_and(|cdef| cdef.cdef_on_skip_txfm_frame_enable == Some(false))
}

fn lr_uses_tx_skip(core: &splot_core::headers::frame::FrameHeaderCore) -> bool {
    use splot_core::headers::frame::FrameRestorationType;
    core.lr_params
        .as_ref()
        .and_then(|lr| lr.planes.first())
        .is_some_and(|plane| {
            matches!(
                plane.restoration_type,
                FrameRestorationType::PcWiener | FrameRestorationType::Switchable
            ) || (plane.restoration_type == FrameRestorationType::WienerNonsep
                && plane.frame_filters_on
                && plane.num_filter_classes.unwrap_or(1) > 1)
        })
}

#[derive(Default)]
pub(crate) struct FrameFilterRecords {
    pub(crate) deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    pub(crate) chroma_deblock_blocks: crate::filters::deblock::ChromaDeblockRecords,
    pub(crate) tx_skip_records: Vec<WienerNsLrTxSkipTransformRecord>,
    pub(crate) lr_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    pub(crate) lr_unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    /// The per-stripe lists the filter phase borrows and hands back.
    pub(crate) stripes: FilterStripeLists,
    /// The deblock grid vectors, borrowed and handed back the same way.
    pub(crate) deblock_grids: crate::filters::deblock::DeblockGridStorage,
    /// The decode's reusable storage, which the next frame's workspace is taken
    /// from and returns to.
    pub(crate) buffers: Option<std::sync::Arc<crate::support::decode_buffers::DecodeBuffers>>,
    pub(crate) cdef_grid_values: Vec<Option<usize>>,
    pub(crate) cdef_strengths: Vec<crate::filters::cdef::CdefFrameParams>,
    pub(crate) tx_skip_grid_values: Vec<u8>,
    pub(crate) cdef_skip_grid_values: Vec<u8>,
    pub(crate) lossless_cells: [Vec<bool>; 2],
    /// Spent stripe input windows, for the next filter phase to fill.
    pub(crate) filter_windows: Vec<splot_recon::RetiredFramePlanes>,
    /// The frame's `BruModes`, `None` when `use_bru == 0`.
    pub(crate) bru_modes: Option<std::sync::Arc<crate::filters::bru::BruModeMap>>,
}

/// The per-stripe lists one frame's filter phase works through.
///
/// The phase borrows them from the records and hands them back when it
/// finishes, so they ride the same channel to the decoder's scratch and a
/// steady-state frame opens its filter setup without allocating.
#[derive(Default)]
pub(crate) struct FilterStripeLists {
    pub(crate) ranges: Vec<(usize, usize)>,
    pub(crate) lifecycles: Vec<recon::StripeLifecycle>,
}

/// The capacities one tile's spent frame-sized record lists reached.
///
/// A tile publishes its units into a fresh set, so without this the lists climb
/// the growth ladder again for a tile the decode has already sized. The deblock
/// lists are left out: the frontier drains them as the parse fills them.
#[derive(Clone, Copy, Default)]
pub(crate) struct FrameFilterRecordCapacities {
    tx_skip_records: usize,
    lr_source_blocks: usize,
    lr_unit_filters: usize,
}

impl FrameFilterRecordCapacities {
    /// Grows every hint to cover `other` as well.
    pub(crate) fn cover(&mut self, other: Self) {
        self.tx_skip_records = self.tx_skip_records.max(other.tx_skip_records);
        self.lr_source_blocks = self.lr_source_blocks.max(other.lr_source_blocks);
        self.lr_unit_filters = self.lr_unit_filters.max(other.lr_unit_filters);
    }
}

impl FrameFilterRecords {
    /// The capacities these lists are holding.
    pub(crate) fn capacities(&self) -> FrameFilterRecordCapacities {
        FrameFilterRecordCapacities {
            tx_skip_records: self.tx_skip_records.capacity(),
            lr_source_blocks: self.lr_source_blocks.capacity(),
            lr_unit_filters: self.lr_unit_filters.capacity(),
        }
    }

    /// Sizes empty lists for the tile a spent set already covered.
    pub(crate) fn reserve_from(&mut self, hint: FrameFilterRecordCapacities) {
        let _ = self.tx_skip_records.try_reserve_exact(hint.tx_skip_records);
        let _ = self
            .lr_source_blocks
            .try_reserve_exact(hint.lr_source_blocks);
        let _ = self.lr_unit_filters.try_reserve_exact(hint.lr_unit_filters);
    }

    /// Moves every record out of `other` and onto the end of these lists.
    pub(crate) fn append(&mut self, other: &mut Self) {
        self.deblock_blocks
            .reserve_exact(other.deblock_blocks.len());
        self.tx_skip_records
            .reserve_exact(other.tx_skip_records.len());
        self.lr_source_blocks
            .reserve_exact(other.lr_source_blocks.len());
        self.lr_unit_filters
            .reserve_exact(other.lr_unit_filters.len());
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
    frame_sb_size4, selectable_missing_quantization_error, selectable_symbol_read_error,
};

#[cfg(test)]
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

    #[cfg(test)]
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

    /// Row `row` from column `col` to its end, checked to hold `len` values.
    pub(crate) fn run(&self, row: usize, col: usize, len: usize) -> ReconResult<&[u8]> {
        col.checked_add(len)
            .filter(|&end| row < self.rows && end <= self.cols)
            .and_then(|_| {
                let start = wienerns_lr_tx_skip_grid_index(row, col, self.cols).ok()?;
                self.values.get(start..start + (self.cols - col))
            })
            .ok_or(ReconError::PcWienerInvalidBounds {
                field: "LrTxSkip grid lookup",
            })
    }

    pub(crate) fn into_values(self) -> Vec<u8> {
        self.values
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
    values: Vec<u8>,
) -> ReconResult<crate::filters::cdef::CdefSkipGrid> {
    let cells =
        derive_wienerns_lr_skip_grid(rows, cols, records, values, |record| record.skip_flag)?;
    crate::filters::cdef::CdefSkipGrid::new(rows, cols, cells.into_values()).map_err(|_| {
        ReconError::PcWienerInvalidBounds {
            field: "CDEF skip grid dimensions",
        }
    })
}

pub(crate) fn derive_wienerns_lr_tx_skip_grid_reusing(
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
    values: Vec<u8>,
) -> ReconResult<WienerNsLrTxSkipGrid> {
    derive_wienerns_lr_skip_grid(rows, cols, records, values, |record| {
        record.skip_flag || record.empty
    })
}

#[cfg(test)]
pub(crate) fn derive_wienerns_lr_tx_skip_grid_retention(
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
) -> ReconResult<WienerNsLrTxSkipGrid> {
    derive_wienerns_lr_tx_skip_grid_reusing(rows, cols, records, Vec::new())
}

fn derive_wienerns_lr_skip_grid(
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
    mut values: Vec<u8>,
    value_of: impl Fn(WienerNsLrTxSkipTransformRecord) -> bool,
) -> ReconResult<WienerNsLrTxSkipGrid> {
    let expected = wienerns_lr_tx_skip_grid_len(rows, cols)?;
    values.clear();
    values
        .try_reserve(expected)
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: splot_recon::PlaneId::Y,
            context: "WienerNS LR transform-skip grid",
        })?;
    values.resize(expected, WIENERNS_LR_TX_SKIP_UNWRITTEN);
    let mut populated = 0usize;
    for &record in records {
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

#[expect(clippy::inline_always, reason = "one call per transform record")]
#[inline(always)]
fn write_wienerns_lr_tx_skip_record(
    rows: usize,
    cols: usize,
    record: WienerNsLrTxSkipTransformRecord,
    value: u8,
    values: &mut [u8],
    populated: &mut usize,
) -> ReconResult<()> {
    let (start, end_row, width) = wienerns_lr_tx_skip_record_span(rows, cols, record)?;
    let run = (start, cols, end_row - usize::from(record.row));
    match width {
        1 => write_tx_skip_rows::<1>(values, run, value, populated),
        2 => write_tx_skip_rows::<2>(values, run, value, populated),
        4 => write_tx_skip_rows::<4>(values, run, value, populated),
        8 => write_tx_skip_rows::<8>(values, run, value, populated),
        16 => write_tx_skip_rows::<16>(values, run, value, populated),
        _ => {
            let mut at = start;
            for _ in 0..run.2 {
                let actual = values.len();
                let slots =
                    values
                        .get_mut(at..at + width)
                        .ok_or(ReconError::BufferLengthMismatch {
                            expected: at.max(actual).saturating_add(1),
                            actual,
                        })?;
                write_tx_skip_slots(slots, value, populated)?;
                at += cols;
            }
            Ok(())
        }
    }
}

/// Writes `value` over `N` slots from `start` in each of `count` rows `cols`
/// apart.
#[expect(clippy::inline_always, reason = "one fixed-width run per row")]
#[inline(always)]
fn write_tx_skip_rows<const N: usize>(
    values: &mut [u8],
    (start, cols, count): (usize, usize, usize),
    value: u8,
    populated: &mut usize,
) -> ReconResult<()> {
    let mut at = start;
    for _ in 0..count {
        let actual = values.len();
        let slots = values
            .get_mut(at..)
            .and_then(<[u8]>::first_chunk_mut::<N>)
            .ok_or_else(|| ReconError::BufferLengthMismatch {
                expected: at.max(actual).saturating_add(1),
                actual,
            })?;
        if *slots == [WIENERNS_LR_TX_SKIP_UNWRITTEN; N] {
            *slots = [value; N];
            *populated += N;
        } else {
            write_tx_skip_slots(slots, value, populated)?;
        }
        at += cols;
    }
    Ok(())
}

/// Writes `value` over the unwritten `slots`, failing on a slot another
/// record wrote differently.
#[expect(clippy::inline_always, reason = "one call per transform record row")]
#[inline(always)]
fn write_tx_skip_slots(slots: &mut [u8], value: u8, populated: &mut usize) -> ReconResult<()> {
    if mi_run_is(slots, WIENERNS_LR_TX_SKIP_UNWRITTEN) {
        crate::support::fill_mi_run(slots, value);
        *populated += slots.len();
        return Ok(());
    }
    for slot in slots {
        if *slot == WIENERNS_LR_TX_SKIP_UNWRITTEN {
            *slot = value;
            *populated += 1;
        } else if *slot != value {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LrTxSkip conflicting transform records",
            });
        }
    }
    Ok(())
}

/// Whether every slot of `run` is `value`, read like
/// [`crate::support::fill_mi_run`] writes: at most two overlapping windows.
fn mi_run_is(run: &[u8], value: u8) -> bool {
    fn ends<const N: usize>(run: &[u8], value: u8) -> bool {
        run[..N] == [value; N] && run[run.len() - N..] == [value; N]
    }
    match run.len() {
        4..=7 => ends::<4>(run, value),
        8..=15 => ends::<8>(run, value),
        16..=32 => ends::<16>(run, value),
        _ => run.iter().all(|&slot| slot == value),
    }
}

/// The first grid index, the clipped end row and the clipped width of
/// `record`.
fn wienerns_lr_tx_skip_record_span(
    rows: usize,
    cols: usize,
    record: WienerNsLrTxSkipTransformRecord,
) -> ReconResult<(usize, usize, usize)> {
    if record.rows == 0 || record.cols == 0 {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip transform record dimensions",
        });
    }
    let (row, col) = (usize::from(record.row), usize::from(record.col));
    if row >= rows || col >= cols {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip transform record bounds",
        });
    }
    let end_row = (row + usize::from(record.rows)).min(rows);
    let width = usize::from(record.cols).min(cols - col);
    let start = wienerns_lr_tx_skip_grid_index(row, col, cols)?;
    Ok((start, end_row, width))
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

#[cfg(test)]
mod tests {
    #[test]
    fn mi_run_is_checks_every_slot_of_every_run_length() {
        for len in 0..=40 {
            let mut run = vec![7u8; len];
            assert!(super::mi_run_is(&run, 7), "len {len}");
            for index in 0..len {
                run[index] = 6;
                assert!(!super::mi_run_is(&run, 7), "len {len} index {index}");
                run[index] = 7;
            }
        }
    }
}
