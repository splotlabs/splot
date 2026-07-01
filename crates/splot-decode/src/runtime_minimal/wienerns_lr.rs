// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrPlaneParams, TxMode};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader};
use splot_core::span::ByteOffset;
use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{
    BitDepth, DecodedFrame, LoopRestorationSource, LoopRestorationSourceBounds,
    LoopRestorationSourceSample, PcWienerClassifyParams, PcWienerTxSkipLookup, PlaneId, ReconError,
    ReconSample, Result as ReconResult, loop_restoration_source_sample,
    loop_restoration_source_sample_value, pc_wiener_classify,
};

use crate::error::{DecodeError, Result};
use crate::tile_payload::{
    GeneralIntraBlockModeError, GeneralIntraChromaToolConfig, GeneralIntraMultiblockError,
    GeneralIntraResidualError, GeneralIntraTreeWalkError, MinimalRuntimePartitionFrontierError,
    TileCoeffContextState, TilePartitionTraversalError, TilePartitionTraversalUnsupported,
    TransformToolResidualPolicy, decode_general_intra_block_modes_with_fsc_context,
    decode_general_intra_multiblock_tree, decode_general_intra_plane_coeffs, frame_mi_dimensions,
};
use crate::{DecodeLimitName, DecodeLimits, DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

use super::limits::{checked_add, checked_mul, decoded_frame_storage_budget};
use super::{
    AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_FEATURE_ID,
    AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_MATRIX_ROW,
    AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_FEATURE_ID, AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_MATRIX_ROW,
    AC0EJ3_LR_SOURCE_READ_FEATURE_ID, AC0EJ3_LR_SOURCE_READ_MATRIX_ROW,
    AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_FEATURE_ID, AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_MATRIX_ROW,
    derive_tile_plan, effective_allow_screen_content_tools,
    ensure_sequence_chroma_tools_before_tile_decode, unsupported_at, unsupported_feature_at,
};

const LR_MI_SIZE: usize = 4;
const PC_WIENER_LEAD: isize = 1;
const PC_WIENER_LAG: isize = 4;
const PC_WIENER_SOURCE_READS_PER_FEATURE: u64 = 7;
const LR_RETAINED_FRAME_BUFFERS: u64 = 2;
const PC_WIENER_FEATURE_SOURCE_READ_OFFSETS: [(isize, isize); 7] =
    [(0, 0), (0, -1), (0, 1), (1, -1), (-1, 1), (1, 1), (-1, -1)];

mod diagnostics;
pub(in crate::runtime_minimal) mod intrabc_records;
mod intrabc_ref_mv_stack;
mod live_storage;
mod recon;
mod source_read_math;
pub(in crate::runtime_minimal) mod tx_records;

pub(super) use self::recon::reconstruct_ac0ej3_selectable_intra_region;
#[cfg(test)]
pub(super) use self::recon::{FullReconLumaLeaf, WienerNsLrReconSink};
use self::tx_records::WienerNsLrLiveTransformRecordHandoff;
pub(super) use self::tx_records::WienerNsLrTxSkipTransformRecord;

use self::diagnostics::transform_tool_residual_frontier;
pub(super) use self::diagnostics::{
    intra_capped_seq_sb_size, map_wienerns_lr_unit_frontier_error,
    wienerns_lr_live_frame_samples_unpopulated_error,
    wienerns_lr_live_transform_record_handoff_error,
    wienerns_lr_live_transform_record_tool_gate_error, wienerns_lr_runtime_storage_retention_error,
    wienerns_lr_selectable_live_frame_samples_unpopulated_error,
    wienerns_lr_selectable_transform_record_error_reason, wienerns_lr_source_read_runtime_error,
    wienerns_lr_unit_runtime_error,
};
#[cfg(test)]
pub(super) use self::diagnostics::{
    wienerns_lr_classified_wiener_storage_runtime_error, wienerns_lr_live_storage_allocation_error,
    wienerns_lr_tx_mode_select_transform_record_error,
};
use self::diagnostics::{wienerns_lr_mode_literal_reason, wienerns_lr_mode_symbol_reason};
pub(super) use self::live_storage::{
    LR_LIVE_FRAME_SAMPLE_STORAGE_BYTES, LR_LIVE_TX_SKIP_STORAGE_BYTES_PER_VALUE,
    WienerNsLrLiveStorageAllocation,
};
use self::source_read_math::{
    chroma_subsampling, clip_source_read_coordinate, mi_to_luma_end, mi_to_luma_start,
    scale_chroma_source_coordinate, source_read_arithmetic_overflow, source_read_coordinate_add,
    usize_to_source_coordinate, wienerns_lr_source_plane,
};

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct WienerNsLrSourceReadFrontier {
    pub(super) blocks_resolved: usize,
    pub(super) output_samples_resolved: usize,
    pub(super) source_reads_resolved: usize,
    pub(super) curr_frame_source_reads: usize,
    pub(super) cdef_frame_source_reads: usize,
    pub(super) first_sample: Option<WienerNsLrSourceReadSample>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrSourceReadSample {
    pub(super) plane: PlaneId,
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) source: LoopRestorationSource,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct WienerNsLrClassifiedWienerFrontier {
    pub(super) blocks_resolved: usize,
    pub(super) feature_points_resolved: usize,
    pub(super) source_reads_resolved: usize,
    pub(super) curr_frame_source_reads: usize,
    pub(super) cdef_frame_source_reads: usize,
    pub(super) tx_skip_lookups_resolved: usize,
    pub(super) first_sample: Option<WienerNsLrSourceReadSample>,
    pub(super) first_tx_skip_lookup: Option<WienerNsLrTxSkipLookup>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrTxSkipLookup {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) row: usize,
    pub(super) col: usize,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrTxSkipGrid {
    rows: usize,
    cols: usize,
    values: Vec<u8>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl WienerNsLrTxSkipGrid {
    pub(super) fn new(rows: usize, cols: usize, values: Vec<u8>) -> ReconResult<Self> {
        let expected = wienerns_lr_tx_skip_grid_len(rows, cols)?;
        if values.len() != expected {
            return Err(ReconError::BufferLengthMismatch {
                expected,
                actual: values.len(),
            });
        }
        Ok(Self { rows, cols, values })
    }

    pub(super) const fn rows(&self) -> usize {
        self.rows
    }

    pub(super) const fn cols(&self) -> usize {
        self.cols
    }

    pub(super) fn lookup(&self, lookup: WienerNsLrTxSkipLookup) -> ReconResult<i32> {
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

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn derive_wienerns_lr_tx_skip_grid_retention(
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
) -> ReconResult<WienerNsLrTxSkipGrid> {
    let expected = wienerns_lr_tx_skip_grid_len(rows, cols)?;
    let mut values = vec![None; expected];
    let mut populated = 0usize;
    for record in records {
        let value = u8::from(record.skip_flag || record.eob == 0);
        write_wienerns_lr_tx_skip_record(rows, cols, record, value, &mut values, &mut populated)?;
    }
    if populated != expected {
        return Err(ReconError::BufferLengthMismatch {
            expected,
            actual: populated,
        });
    }
    let mut dense = Vec::with_capacity(expected);
    for value in values {
        let Some(value) = value else {
            return Err(ReconError::BufferLengthMismatch {
                expected,
                actual: populated,
            });
        };
        dense.push(value);
    }
    WienerNsLrTxSkipGrid::new(rows, cols, dense)
}

fn write_wienerns_lr_tx_skip_record(
    rows: usize,
    cols: usize,
    record: &WienerNsLrTxSkipTransformRecord,
    value: u8,
    values: &mut [Option<u8>],
    populated: &mut usize,
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
            let Some(slot) = values.get_mut(index) else {
                return Err(ReconError::BufferLengthMismatch {
                    expected: index.saturating_add(1),
                    actual: values.len(),
                });
            };
            match *slot {
                Some(existing) if existing != value => {
                    return Err(ReconError::PcWienerInvalidBounds {
                        field: "LrTxSkip conflicting transform records",
                    });
                }
                Some(_) => {}
                None => {
                    *slot = Some(value);
                    *populated =
                        populated
                            .checked_add(1)
                            .ok_or(ReconError::ArithmeticOverflow {
                                context: "LrTxSkip populated sample count",
                            })?;
                }
            }
        }
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug)]
pub(super) struct WienerNsLrClassifiedWienerStorageInputs<'a, T: ReconSample> {
    pub(super) curr_frame: &'a DecodedFrame<T>,
    pub(super) cdef_frame: &'a DecodedFrame<T>,
    pub(super) tx_skip_grid: &'a WienerNsLrTxSkipGrid,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrRuntimeStorageRetentionFrontier {
    pub(super) bit_depth: BitDepth,
    pub(super) frame_buffer_count: u64,
    pub(super) frame_buffer_bytes: u64,
    pub(super) retained_frame_buffer_bytes: u64,
    pub(super) tx_skip_rows: usize,
    pub(super) tx_skip_cols: usize,
    pub(super) tx_skip_values: u64,
    pub(super) total_storage_bytes: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct WienerNsLrClassifiedWienerValuesFrontier {
    pub(super) blocks_resolved: usize,
    pub(super) source_reads_resolved: usize,
    pub(super) curr_frame_source_reads: usize,
    pub(super) cdef_frame_source_reads: usize,
    pub(super) filter_classes_resolved: usize,
    pub(super) first_sample: Option<WienerNsLrClassifiedWienerValueSourceSample>,
    pub(super) first_filter_class: Option<WienerNsLrFilterClassValue>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrFilterClassValue {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) class: u8,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrClassifiedWienerValueSourceSample {
    pub(super) input_x: isize,
    pub(super) input_y: isize,
    pub(super) bounds: LoopRestorationSourceBounds,
    pub(super) sample: WienerNsLrSourceReadSample,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrSourceReadConfig {
    pub(super) chroma_luma_source_taps: [[bool; WIENER_NS_CHROMA_SOURCE_TAP_COUNT]; 3],
    pub(super) cfl_ds_filter_index: u8,
}

impl WienerNsLrSourceReadConfig {
    pub(super) const CONSERVATIVE: Self = Self {
        chroma_luma_source_taps: [[true; WIENER_NS_CHROMA_SOURCE_TAP_COUNT]; 3],
        cfl_ds_filter_index: 0,
    };

    const fn chroma_luma_source_taps(
        self,
        plane: PlaneId,
    ) -> [bool; WIENER_NS_CHROMA_SOURCE_TAP_COUNT] {
        self.chroma_luma_source_taps[plane.index()]
    }
}

pub(super) const WIENER_NS_CHROMA_SOURCE_TAP_COUNT: usize = 12;
const WIENER_NS_CHROMA_LUMA_COEFF_OFFSET: usize = 6;

const WIENER_NS_LUMA_SOURCE_TAPS: [(isize, isize); 32] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (2, 0),
    (-2, 0),
    (0, 2),
    (0, -2),
    (1, 1),
    (-1, -1),
    (-1, 1),
    (1, -1),
    (2, 1),
    (-2, -1),
    (2, -1),
    (-2, 1),
    (1, 2),
    (-1, -2),
    (1, -2),
    (-1, 2),
    (3, 0),
    (-3, 0),
    (0, 3),
    (0, -3),
    (4, 0),
    (-4, 0),
    (0, 4),
    (0, -4),
    (3, 3),
    (-3, -3),
    (3, -3),
    (-3, 3),
];

const WIENER_NS_CHROMA_SOURCE_TAPS: [(isize, isize); WIENER_NS_CHROMA_SOURCE_TAP_COUNT] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, -1),
    (-1, 1),
    (1, -1),
    (2, 0),
    (-2, 0),
    (0, 2),
    (0, -2),
];

#[allow(clippy::too_many_arguments)]
pub(super) fn ensure_wienerns_lr_unit_runtime_frontier(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    key_candidate: &DecodePlannedObu,
    key_envelope: ObuEnvelope<'_>,
    sequence_offset: ByteOffset,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Result<()> {
    if !has_wienerns_frame_filter_bank(core) {
        return Ok(());
    }
    let lr_frontier = consume_wienerns_lr_unit_frontier(
        bytes,
        options,
        plan,
        key_candidate,
        key_envelope,
        sequence,
        core,
    )?;
    if lr_frontier.all_lr_units_inactive() {
        Ok(())
    } else if !lr_frontier.active_source_blocks().is_empty() {
        let lr_params = core
            .lr_params
            .as_ref()
            .ok_or_else(|| wienerns_lr_unit_runtime_error(key_envelope.offset))?;
        let cfl_ds_filter_index = sequence
            .intra
            .as_ref()
            .map_or(0, |intra| intra.cfl_ds_filter_index);
        let source_read_config =
            wienerns_lr_source_read_config(&lr_params.planes, cfl_ds_filter_index);
        let (classified_frontier, _source_read_frontier) =
            derive_wienerns_lr_runtime_source_frontiers(
                lr_frontier.active_source_blocks(),
                &lr_params.planes,
                sequence.general.chroma_format_idc,
                source_read_config,
                key_envelope.offset,
                options.limits(),
            )?;
        if classified_frontier.is_some() {
            let storage_retention = derive_wienerns_lr_runtime_storage_retention_frontier(
                sequence,
                core,
                key_envelope.offset,
                options.limits(),
            )?;
            let tx_mode = core
                .intra_tail
                .as_ref()
                .map(|tail| tail.tx_mode)
                .ok_or_else(|| {
                    wienerns_lr_live_transform_record_handoff_error(key_envelope.offset)
                })?;
            let transform_handoff = if tx_mode == TxMode::Select {
                tx_records::derive_wienerns_lr_selectable_transform_record_handoff(
                    bytes,
                    options,
                    plan,
                    key_candidate,
                    key_envelope,
                    sequence,
                    core,
                    None,
                )?
            } else if tx_mode == TxMode::Largest {
                ensure_sequence_chroma_tools_before_tile_decode(sequence, sequence_offset)?;
                ensure_fixed_largest_transform_record_tool_gates(sequence, core, sequence_offset)?;
                derive_wienerns_lr_fixed_largest_transform_record_handoff(
                    bytes,
                    options,
                    plan,
                    key_candidate,
                    key_envelope,
                    sequence,
                    core,
                )?
            } else {
                return Err(wienerns_lr_live_transform_record_handoff_error(
                    key_envelope.offset,
                ));
            };
            let mut live_storage = derive_wienerns_lr_live_storage_allocation(storage_retention)?;
            populate_wienerns_lr_live_tx_skip_from_transform_records(
                &mut live_storage,
                transform_handoff.tx_skip_rows,
                transform_handoff.tx_skip_cols,
                &transform_handoff.records,
            )?;
            if tx_mode == TxMode::Select {
                return Err(wienerns_lr_selectable_live_frame_samples_unpopulated_error(
                    key_envelope.offset,
                ));
            }
            return Err(wienerns_lr_live_frame_samples_unpopulated_error(
                key_envelope.offset,
            ));
        }
        Err(wienerns_lr_source_read_runtime_error(key_envelope.offset))
    } else {
        Err(wienerns_lr_unit_runtime_error(key_envelope.offset))
    }
}

#[allow(clippy::too_many_arguments)]
fn consume_wienerns_lr_unit_frontier(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    key_candidate: &DecodePlannedObu,
    key_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Result<crate::tile_payload::TileLoopRestorationRootFrontier> {
    let mut tile_plan = derive_tile_plan(
        plan,
        key_candidate,
        bytes,
        key_envelope,
        sequence,
        core,
        options,
    )?;
    let tile = match tile_plan.work_units_mut() {
        [tile] => tile,
        [] => {
            return Err(unsupported_at(
                "missing_lr_tile_work_unit",
                key_envelope.offset,
                "minimal runtime requires one tile work unit before parsing LR unit syntax",
            ));
        }
        work_units => {
            return Err(unsupported_at(
                "multi_tile_lr_unit_syntax",
                work_units
                    .first()
                    .map_or(key_envelope.offset, |tile| tile.tile_byte_span().start),
                "minimal runtime only consumes ac0ej3 LR unit syntax for one-tile key frames",
            ));
        }
    };
    crate::tile_payload::consume_minimal_runtime_lr_unit_frontier(
        tile,
        sequence,
        core,
        options.limits(),
    )
    .map_err(|err| map_wienerns_lr_unit_frontier_error(err, key_envelope.offset))
}

pub(super) fn derive_wienerns_lr_runtime_storage_retention_frontier(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    offset: ByteOffset,
    limits: DecodeLimits,
) -> Result<WienerNsLrRuntimeStorageRetentionFrontier> {
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_feature_at(
            "unsupported_wienerns_lr_runtime_storage_missing_frame_size",
            offset,
            "minimal runtime cannot retain loop-restoration storage before the parsed frame size is available",
            AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_MATRIX_ROW,
            AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_FEATURE_ID,
            "6.17.4.1",
        )
    })?;
    limits.ensure(DecodeLimitName::MaxFrameWidth, u64::from(frame_size.width))?;
    limits.ensure(
        DecodeLimitName::MaxFrameHeight,
        u64::from(frame_size.height),
    )?;

    let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())?;
    let bytes_per_sample = decoded_storage_bytes_per_sample(bit_depth);
    let budget = decoded_frame_storage_budget(
        frame_size,
        sequence.general.chroma_format_idc,
        bytes_per_sample,
    )?;
    limits.ensure(DecodeLimitName::MaxLumaSamplesPerFrame, budget.luma_samples)?;
    limits.ensure(DecodeLimitName::MaxDecodedFrameBytes, budget.decoded_bytes)?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, budget.luma_samples)?;
    if budget.chroma_samples_per_plane != 0 {
        limits.ensure_allocation_len(
            DecodeLimitName::MaxDecodedFrameBytes,
            budget.chroma_samples_per_plane,
        )?;
    }

    let decoded_sample_count = budget.decoded_bytes / bytes_per_sample;
    let live_frame_buffer_bytes = checked_mul(
        DecodeLimitName::MaxReferenceStoreBytes,
        decoded_sample_count,
        LR_LIVE_FRAME_SAMPLE_STORAGE_BYTES,
    )?;
    let retained_frame_buffer_bytes = checked_mul(
        DecodeLimitName::MaxReferenceStoreBytes,
        live_frame_buffer_bytes,
        LR_RETAINED_FRAME_BUFFERS,
    )?;
    let (tx_skip_rows, tx_skip_cols) = crate::tile_payload::frame_mi_dimensions(core)
        .map_err(|_| wienerns_lr_runtime_storage_retention_error(offset))?;
    let tx_skip_values = checked_mul(
        DecodeLimitName::MaxDecodedFrameBytes,
        usize_to_storage_u64(tx_skip_rows, "LrTxSkip grid rows")?,
        usize_to_storage_u64(tx_skip_cols, "LrTxSkip grid columns")?,
    )?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, tx_skip_values)?;
    let tx_skip_storage_bytes = checked_mul(
        DecodeLimitName::MaxReferenceStoreBytes,
        tx_skip_values,
        LR_LIVE_TX_SKIP_STORAGE_BYTES_PER_VALUE,
    )?;
    let total_storage_bytes = checked_add(
        DecodeLimitName::MaxReferenceStoreBytes,
        retained_frame_buffer_bytes,
        tx_skip_storage_bytes,
    )?;
    limits.ensure(DecodeLimitName::MaxReferenceStoreBytes, total_storage_bytes)?;

    Ok(WienerNsLrRuntimeStorageRetentionFrontier {
        bit_depth,
        frame_buffer_count: LR_RETAINED_FRAME_BUFFERS,
        frame_buffer_bytes: budget.decoded_bytes,
        retained_frame_buffer_bytes,
        tx_skip_rows,
        tx_skip_cols,
        tx_skip_values,
        total_storage_bytes,
    })
}

pub(super) fn derive_wienerns_lr_live_storage_allocation(
    frontier: WienerNsLrRuntimeStorageRetentionFrontier,
) -> Result<WienerNsLrLiveStorageAllocation> {
    WienerNsLrLiveStorageAllocation::from_retention_frontier(frontier)
}

#[allow(clippy::too_many_arguments)]
fn derive_wienerns_lr_fixed_largest_transform_record_handoff(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    key_candidate: &DecodePlannedObu,
    key_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Result<WienerNsLrLiveTransformRecordHandoff> {
    if sequence.general.chroma_format_idc != ChromaFormatIdc::Yuv420 {
        return Err(wienerns_lr_live_transform_record_handoff_error(
            key_envelope.offset,
        ));
    }
    if core
        .delta_q_params
        .as_ref()
        .is_some_and(|delta_q| delta_q.delta_q_present)
    {
        return Err(wienerns_lr_live_transform_record_handoff_error(
            key_envelope.offset,
        ));
    }

    let mut tile_plan = derive_tile_plan(
        plan,
        key_candidate,
        bytes,
        key_envelope,
        sequence,
        core,
        options,
    )?;
    let tile = match tile_plan.work_units_mut() {
        [tile] => tile,
        [] => {
            return Err(wienerns_lr_live_transform_record_handoff_error(
                key_envelope.offset,
            ));
        }
        work_units => {
            return Err(wienerns_lr_live_transform_record_handoff_error(
                work_units
                    .first()
                    .map_or(key_envelope.offset, |tile| tile.tile_byte_span().start),
            ));
        }
    };
    let tile_offset = tile.tile_byte_span().start;
    let (tx_skip_rows, tx_skip_cols) = frame_mi_dimensions(core)
        .map_err(|_| wienerns_lr_live_transform_record_handoff_error(tile_offset))?;
    let mut coeff_ctx = TileCoeffContextState::new(tx_skip_rows, tx_skip_cols)
        .map_err(|_| wienerns_lr_live_transform_record_handoff_error(tile_offset))?;
    let mut records = Vec::new();
    let limits = options.limits();

    let symbols = decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit, symbols, frontier, joint_modes, uses_mrls, fsc_modes, palette_state, is_cfl_ctx, _block_decoded| {
            let n4w = frontier
                .b_size
                .num_4x4_wide()
                .map_err(|_| wienerns_lr_live_transform_record_handoff_error(tile_offset))?;
            let n4h = frontier
                .b_size
                .num_4x4_high()
                .map_err(|_| wienerns_lr_live_transform_record_handoff_error(tile_offset))?;
            if n4w < 2 || n4h < 2 || !frontier.has_chroma {
                return Err(wienerns_lr_live_transform_record_handoff_error(tile_offset));
            }
            if frontier.chroma_offset {
                return Err(wienerns_lr_transform_record_unsupported(
                    WienerNsLrTransformRecordDiagnosticScope::FixedLargest,
                    "unsupported_wienerns_lr_live_transform_record_chroma_offset_leaf",
                    tile_offset,
                    "Fixed-largest Wiener NS LR records need ancestor chroma residual coordinates for chroma-offset leaves.",
                    "5.20.3.1",
                ));
            }
            let use_neighbor_fsc_context =
                core.frame_is_intra == Some(true) || !frontier.is_mixed_region();
            let modes = decode_general_intra_block_modes_with_fsc_context(
                work_unit,
                symbols,
                GeneralIntraChromaToolConfig::disabled()
                    .with_allow_screen_content_tools(effective_allow_screen_content_tools(core)),
                joint_modes,
                uses_mrls,
                fsc_modes,
                use_neighbor_fsc_context,
                palette_state,
                is_cfl_ctx.get(),
                frontier.b_size.index(),
                frontier.r,
                frontier.c,
                n4w,
                n4h,
                frontier.b_size.index(),
                n4w,
                n4h,
                bit_depth_bits(sequence),
            )
            .map_err(|error| {
                wienerns_lr_live_transform_record_mode_error(
                    error,
                    tile_offset,
                    WienerNsLrTransformRecordDiagnosticScope::FixedLargest,
                )
            })?;

            let luma_tx = fixed_largest_tx_size_from_4x4(n4w, n4h)
                .ok_or_else(|| wienerns_lr_live_transform_record_handoff_error(tile_offset))?;
            let luma_x = frontier.c * 4;
            let luma_y = frontier.r * 4;
            let luma = decode_general_intra_plane_coeffs(
                work_unit,
                symbols,
                &mut coeff_ctx,
                0,
                luma_tx,
                luma_x,
                luma_y,
                true,
                None,
                false,
                modes.coeff_uv_mode(),
                0,
                false,
                false,
                false,
                TransformToolResidualPolicy::Allow,
            )
            .map_err(|error| {
                wienerns_lr_live_transform_record_residual_error(
                    error,
                    tile_offset,
                    WienerNsLrTransformRecordDiagnosticScope::FixedLargest,
                )
            })?;
            records
                .try_reserve(1)
                .map_err(|_| wienerns_lr_live_transform_record_handoff_error(tile_offset))?;
            records.push(WienerNsLrTxSkipTransformRecord {
                row: frontier.r,
                col: frontier.c,
                rows: n4h,
                cols: n4w,
                skip_flag: false,
                eob: luma.eob,
                intra_ist: luma.intra_ist,
            });

            let chroma_tx = fixed_largest_420_chroma_tx_size_from_luma_4x4(n4w, n4h)
                .ok_or_else(|| wienerns_lr_live_transform_record_handoff_error(tile_offset))?;
            let chroma_x = frontier.c * 2;
            let chroma_y = frontier.r * 2;
            let angle_delta_uv = if modes.coeff_uv_mode() == modes.y_mode.value() {
                i32::from(modes.angle_delta_y)
            } else {
                0
            };
            let u = decode_general_intra_plane_coeffs(
                work_unit,
                symbols,
                &mut coeff_ctx,
                1,
                chroma_tx,
                chroma_x,
                chroma_y,
                true,
                None,
                false,
                modes.coeff_uv_mode(),
                angle_delta_uv,
                false,
                false,
                modes.fsc_mode != 0,
                TransformToolResidualPolicy::Allow,
            )
            .map_err(|error| {
                wienerns_lr_live_transform_record_residual_error(
                    error,
                    tile_offset,
                    WienerNsLrTransformRecordDiagnosticScope::FixedLargest,
                )
            })?;
            let _v = decode_general_intra_plane_coeffs(
                work_unit,
                symbols,
                &mut coeff_ctx,
                2,
                chroma_tx,
                chroma_x,
                chroma_y,
                true,
                None,
                !u.all_zero,
                modes.coeff_uv_mode(),
                angle_delta_uv,
                false,
                false,
                modes.fsc_mode != 0,
                TransformToolResidualPolicy::Allow,
            )
            .map_err(|error| {
                wienerns_lr_live_transform_record_residual_error(
                    error,
                    tile_offset,
                    WienerNsLrTransformRecordDiagnosticScope::FixedLargest,
                )
            })?;
            Ok(crate::tile_payload::GeneralIntraLeafMode::luma(
                modes.intra_joint_mode,
                modes.y_mode,
                modes.angle_delta_y,
                modes.fsc_mode,
                modes.uses_mrls,
            )
            .with_uv_cfl(modes.is_cfl()))
        },
    )
    .map_err(|error| {
        map_wienerns_lr_transform_record_multiblock_error(
            error,
            tile_offset,
            WienerNsLrTransformRecordDiagnosticScope::FixedLargest,
        )
    })?;

    symbols
        .exit_symbol()
        .map_err(|_| wienerns_lr_live_transform_record_handoff_error(tile_offset))?;
    tile.apply_frame_end_cdf_update();

    Ok(WienerNsLrLiveTransformRecordHandoff {
        tx_skip_rows,
        tx_skip_cols,
        records,
        active_source_blocks: Vec::new(),
        unit_filters: Vec::new(),
        frame_cdfs: tile.frame_cdfs(),
        cdef_grid: None,
        ccso_grid: None,
    })
}

pub(super) fn populate_wienerns_lr_live_tx_skip_from_transform_records(
    live_storage: &mut WienerNsLrLiveStorageAllocation,
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
) -> Result<()> {
    let grid = derive_wienerns_lr_tx_skip_grid_retention(rows, cols, records)?;
    live_storage.populate_tx_skip_grid(&grid)
}

fn ensure_fixed_largest_transform_record_tool_gates(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<()> {
    let unsupported_gates = [
        (
            "tile_grid",
            core.tile_info
                .as_ref()
                .is_none_or(|tile_info| tile_info.tile_cols != 1 || tile_info.tile_rows != 1),
        ),
        (
            "screen_content_tools",
            core.allow_screen_content_tools != Some(false) || core.allow_intrabc != Some(false),
        ),
        (
            "intra_tool",
            sequence.intra.as_ref().is_none_or(|intra| {
                intra.enable_dip
                    || intra.enable_ibp
                    || intra.enable_mrls
                    || intra.enable_intra_edge_filter
            }),
        ),
        (
            "transform_tool",
            sequence.transform_quant_entropy.as_ref().is_none_or(|tq| {
                tq.enable_fsc
                    || tq.enable_cctx
                    || tq.enable_idtx_intra
                    || tq.enable_intra_ist
                    || tq.enable_chroma_dctonly
            }),
        ),
        (
            "frame_tool",
            fixed_largest_frame_tool_gate_is_unsupported(core),
        ),
    ];
    for (tool, unsupported) in unsupported_gates {
        if unsupported {
            return Err(wienerns_lr_live_transform_record_tool_gate_error(
                offset, tool,
            ));
        }
    }
    Ok(())
}

fn fixed_largest_frame_tool_gate_is_unsupported(core: &FrameHeaderCore) -> bool {
    core.segmentation_params
        .as_ref()
        .is_none_or(|seg| seg.segmentation_enabled)
        || core.setup_qm_params.is_none_or(|qm| qm.using_qmatrix)
        || core
            .lossless_info
            .as_ref()
            .is_none_or(|lossless| lossless.coded_lossless)
        || core
            .delta_q_params
            .as_ref()
            .is_none_or(|delta| delta.delta_q_present)
        || core.gdf_params.is_none_or(|gdf| gdf.gdf_frame_enable)
        || core
            .cdef_params
            .as_ref()
            .is_none_or(|cdef| cdef.cdef_frame_enable)
        || core
            .ccso_params
            .as_ref()
            .is_none_or(|ccso| ccso.ccso_frame_flag.is_some() || !ccso.planes.is_empty())
        || core
            .intra_tail
            .is_none_or(|tail| tail.film_grain.apply_grain)
}

fn fixed_largest_tx_size_from_4x4(n4w: usize, n4h: usize) -> Option<usize> {
    if n4w == 0 || n4h == 0 || !n4w.is_power_of_two() || !n4h.is_power_of_two() {
        return None;
    }
    let w_log2 = n4w.trailing_zeros().checked_add(2)?;
    let h_log2 = n4h.trailing_zeros().checked_add(2)?;
    tx_size_from_log2(w_log2, h_log2)
}

fn fixed_largest_420_chroma_tx_size_from_luma_4x4(n4w: usize, n4h: usize) -> Option<usize> {
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

fn map_wienerns_lr_transform_record_multiblock_error(
    error: GeneralIntraMultiblockError<DecodeError>,
    tile_offset: ByteOffset,
    scope: WienerNsLrTransformRecordDiagnosticScope,
) -> DecodeError {
    match error {
        GeneralIntraMultiblockError::Setup(error) => {
            wienerns_lr_transform_record_setup_error(error, tile_offset, scope)
        }
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Leaf(error)) => error,
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Traversal(error)) => {
            wienerns_lr_transform_record_traversal_error(error, tile_offset, scope)
        }
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::MiSize(_)) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_walk_mi_size_state",
                tile_offset,
                "Wiener NS LR transform records need MI-size updates between partition leaves.",
                "5.20.3.1",
            )
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WienerNsLrTransformRecordDiagnosticScope {
    FixedLargest,
    Selectable,
}

impl WienerNsLrTransformRecordDiagnosticScope {
    const fn matrix_row(self) -> &'static str {
        match self {
            Self::FixedLargest => AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_MATRIX_ROW,
            Self::Selectable => AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_MATRIX_ROW,
        }
    }

    const fn feature_id(self) -> &'static str {
        match self {
            Self::FixedLargest => AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_FEATURE_ID,
            Self::Selectable => AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_FEATURE_ID,
        }
    }
}

fn wienerns_lr_transform_record_setup_error(
    error: MinimalRuntimePartitionFrontierError,
    offset: ByteOffset,
    scope: WienerNsLrTransformRecordDiagnosticScope,
) -> DecodeError {
    match error {
        MinimalRuntimePartitionFrontierError::Limit(source)
        | MinimalRuntimePartitionFrontierError::Traversal(TilePartitionTraversalError::Limit(
            source,
        )) => DecodeError::Limit { source },
        MinimalRuntimePartitionFrontierError::Traversal(error) => {
            wienerns_lr_transform_record_traversal_error(error, offset, scope)
        }
        MinimalRuntimePartitionFrontierError::MissingFact { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_missing_fact",
                offset,
                "Wiener NS LR transform records need parser facts that are absent.",
                "5.20.3.1",
            )
        }
        MinimalRuntimePartitionFrontierError::MiSizeState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_mi_size_state",
                offset,
                "Wiener NS LR transform records need MI-size traversal state.",
                "5.20.3.1",
            )
        }
        MinimalRuntimePartitionFrontierError::IntraJointModeState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_intra_joint_mode_state",
                offset,
                "Wiener NS LR transform records need intra joint-mode neighbour state.",
                "8.3.2",
            )
        }
        MinimalRuntimePartitionFrontierError::UsesMrlsState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_uses_mrls_state",
                offset,
                "Wiener NS LR transform records need UsesMrls neighbour state.",
                "8.3.2",
            )
        }
        MinimalRuntimePartitionFrontierError::FscModeState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_fsc_mode_state",
                offset,
                "Wiener NS LR transform records need FscModes neighbour state.",
                "8.3.2",
            )
        }
        MinimalRuntimePartitionFrontierError::UvCflState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_uv_cfl_state",
                offset,
                "Wiener NS LR transform records need UVCfls neighbour state.",
                "8.3.2",
            )
        }
        MinimalRuntimePartitionFrontierError::LumaPaletteState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_luma_palette_state",
                offset,
                "Wiener NS LR transform records need luma palette neighbour state.",
                "5.20.8.1",
            )
        }
        MinimalRuntimePartitionFrontierError::UnexpectedFrontier { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_unexpected_frontier",
                offset,
                "Initial partition frontier shape is outside the Wiener NS LR record subset.",
                "5.20.3.1",
            )
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn wienerns_lr_transform_record_traversal_error(
    error: TilePartitionTraversalError,
    offset: ByteOffset,
    scope: WienerNsLrTransformRecordDiagnosticScope,
) -> DecodeError {
    match error {
        TilePartitionTraversalError::Limit(source) => DecodeError::Limit { source },
        TilePartitionTraversalError::Unsupported(unsupported) => {
            wienerns_lr_transform_record_unsupported_traversal(unsupported, offset, scope)
        }
        TilePartitionTraversalError::BlockDecoded(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_block_decoded_state",
            offset,
            "Partition traversal cannot maintain BlockDecoded state for Wiener NS LR records.",
            "5.20.2.3",
        ),
        TilePartitionTraversalError::IntraYModeState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_intra_ymode_state",
                offset,
                "Partition traversal cannot maintain SDP luma YMode state for Wiener NS LR records.",
                "5.20.5.3",
            )
        }
        TilePartitionTraversalError::UsesMrlsState(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_uses_mrls_state",
            offset,
            "Partition traversal cannot maintain UsesMrls state for Wiener NS LR records.",
            "8.3.2",
        ),
        TilePartitionTraversalError::FscModeState(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_fsc_mode_state",
            offset,
            "Partition traversal cannot maintain FscModes state for Wiener NS LR records.",
            "8.3.2",
        ),
        TilePartitionTraversalError::LumaPaletteState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_luma_palette_state",
                offset,
                "Partition traversal cannot maintain luma palette state for Wiener NS LR records.",
                "5.20.8.1",
            )
        }
        TilePartitionTraversalError::Size(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_size",
            offset,
            "Partition-size lookup failed during Wiener NS LR record traversal.",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::Allowed(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_allowed",
            offset,
            "Allowed partition derivation failed during Wiener NS LR record traversal.",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::Decision(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_decision",
            offset,
            "Partition decision syntax is outside the Wiener NS LR record subset.",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::Symbol(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_symbol",
            offset,
            "Partition symbol decoder setup failed during Wiener NS LR record traversal.",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::Cdf(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_cdf",
            offset,
            "Partition CDF context lookup is outside the supported table shape.",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::CoordinateUnderflow { .. }
        | TilePartitionTraversalError::CoordinateOverflow { .. }
        | TilePartitionTraversalError::CoordinateOffsetOverflow { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_coordinate_math",
                offset,
                "Partition coordinate arithmetic exceeded the checked range.",
                "5.20.3.1",
            )
        }
        TilePartitionTraversalError::InvalidLoopRestorationUnitSize { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_invalid_lr_unit_size",
                offset,
                "Loop-restoration unit size does not map to traversal state.",
                "5.20.10.4",
            )
        }
        TilePartitionTraversalError::InvalidPartitionSubsize { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_invalid_partition_subsize",
                offset,
                "Partition choice produced no valid child block size.",
                "5.20.3.1",
            )
        }
        TilePartitionTraversalError::InvalidRegionType { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_invalid_region_type",
                offset,
                "Extended SDP region type syntax is outside the Wiener NS LR record subset.",
                "5.20.3.1",
            )
        }
        TilePartitionTraversalError::TooManyChildCalls => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_too_many_child_calls",
            offset,
            "Partition traversal produced too many child calls.",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::NoBlockFrontier => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_no_block_frontier",
            offset,
            "Partition traversal reached no in-frame decode_block frontier.",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::MissingIntraLumaModeState { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_missing_intra_luma_mode_state",
                offset,
                "Intra luma/shared leaf is missing YMode state for SDP chroma syntax.",
                "5.20.5.3",
            )
        }
        TilePartitionTraversalError::MissingIntraUsesMrlsState { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_missing_uses_mrls_state",
                offset,
                "Intra luma/shared leaf is missing UsesMrls state for MRL contexts.",
                "5.20.5.3",
            )
        }
        TilePartitionTraversalError::MissingIntraFscModeState { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_missing_fsc_mode_state",
                offset,
                "Intra luma/shared leaf is missing FscModes state for FSC contexts.",
                "5.20.5.3",
            )
        }
    }
}

fn wienerns_lr_transform_record_unsupported_traversal(
    unsupported: TilePartitionTraversalUnsupported,
    offset: ByteOffset,
    scope: WienerNsLrTransformRecordDiagnosticScope,
) -> DecodeError {
    match unsupported {
        TilePartitionTraversalUnsupported::ExtendedSdp => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_extended_sdp",
            offset,
            "Extended SDP region signaling is outside the Wiener NS LR record subset.",
            "5.20.3.1",
        ),
        TilePartitionTraversalUnsupported::ReadLoopRestoration => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_read_loop_restoration",
                offset,
                "Root read_lr syntax is outside transform-record traversal.",
                "5.20.10.4",
            )
        }
        TilePartitionTraversalUnsupported::BruOrBridge => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_bru_or_bridge",
            offset,
            "BRU, bridge, and inactive partition behavior are outside the Wiener NS LR record subset.",
            "5.20.3.1",
        ),
    }
}

fn wienerns_lr_transform_record_unsupported(
    scope: WienerNsLrTransformRecordDiagnosticScope,
    reason: &'static str,
    offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    unsupported_feature_at(
        reason,
        offset,
        message,
        scope.matrix_row(),
        scope.feature_id(),
        spec_section,
    )
}

#[allow(clippy::needless_pass_by_value)]
fn wienerns_lr_live_transform_record_mode_error(
    error: GeneralIntraBlockModeError,
    offset: ByteOffset,
    scope: WienerNsLrTransformRecordDiagnosticScope,
) -> DecodeError {
    match error {
        GeneralIntraBlockModeError::SymbolRead { reason, .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                wienerns_lr_mode_symbol_reason(reason),
                offset,
                "Mode-info CDF symbol read is outside the supported intra subset.",
                "5.20.5.3",
            )
        }
        GeneralIntraBlockModeError::Literal { reason, .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                wienerns_lr_mode_literal_reason(reason),
                offset,
                "Mode-info escape literal read is outside the supported intra subset.",
                "5.20.5.3",
            )
        }
        GeneralIntraBlockModeError::UnsupportedYMode { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_y_mode",
                offset,
                "Luma intra mode is outside the Wiener NS LR record subset.",
                "5.20.5.3",
            )
        }
        GeneralIntraBlockModeError::InvalidUvMode { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_uv_mode",
                offset,
                "Chroma intra mode is outside the Wiener NS LR record subset.",
                "5.20.5.6",
            )
        }
        GeneralIntraBlockModeError::InvalidFscBlockSizeIndex { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_fsc_bsize_group",
                offset,
                "Block size does not map through Fsc_Bsize_Groups for fsc_mode.",
                "8.3.2",
            )
        }
        GeneralIntraBlockModeError::InvalidCflMhDirBlockSizeIndex { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_cfl_mh_dir_size_group",
                offset,
                "Block size does not map through Size_Group for cfl_mh_dir.",
                "8.3.2",
            )
        }
        GeneralIntraBlockModeError::UnsupportedMhccpMode => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_mhccp_mode",
                offset,
                "Active MHCCP chroma prediction is outside the Wiener NS LR record subset.",
                "5.20.5.6",
            )
        }
        GeneralIntraBlockModeError::InvalidPaletteYSize { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "invalid_wienerns_lr_live_transform_record_palette_y_size",
                offset,
                "Decoded luma palette size is outside the valid range.",
                "5.20.8.1",
            )
        }
        GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_directional_neighbour_reorder",
                offset,
                "Y-mode selection needs directional-neighbour reordering.",
                "5.20.5.3",
            )
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn wienerns_lr_live_transform_record_residual_error(
    error: GeneralIntraResidualError,
    offset: ByteOffset,
    scope: WienerNsLrTransformRecordDiagnosticScope,
) -> DecodeError {
    match error {
        GeneralIntraResidualError::UnsupportedTransformToolResidual { reason } => {
            let (message, matrix_row, feature_id, spec_section) =
                transform_tool_residual_frontier(reason);
            unsupported_feature_at(
                reason,
                offset,
                message,
                matrix_row,
                feature_id,
                spec_section,
            )
        }
        _ => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_residual_parse",
            offset,
            "Coefficient syntax is outside the Wiener NS LR transform-record subset.",
            "5.20.7.27",
        ),
    }
}

const fn decoded_storage_bytes_per_sample(bit_depth: BitDepth) -> u64 {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
    }
}

fn usize_to_storage_u64(value: usize, context: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| source_read_arithmetic_overflow(context))
}

fn increment_wienerns_lr_counter(value: &mut usize, context: &'static str) -> Result<()> {
    *value = value
        .checked_add(1)
        .ok_or_else(|| source_read_arithmetic_overflow(context))?;
    Ok(())
}

fn increment_wienerns_lr_recon_counter(
    value: &mut usize,
    context: &'static str,
) -> ReconResult<()> {
    *value = value
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow { context })?;
    Ok(())
}

fn next_wienerns_lr_counter(value: usize, context: &'static str) -> Result<usize> {
    value
        .checked_add(1)
        .ok_or_else(|| source_read_arithmetic_overflow(context))
}

fn wienerns_lr_source_read_sample(
    plane: PlaneId,
    sample: LoopRestorationSourceSample,
) -> WienerNsLrSourceReadSample {
    WienerNsLrSourceReadSample {
        plane,
        x: sample.x,
        y: sample.y,
        source: sample.source,
    }
}

fn observe_wienerns_lr_source_sample(
    first_sample: &mut Option<WienerNsLrSourceReadSample>,
    curr_frame_source_reads: &mut usize,
    cdef_frame_source_reads: &mut usize,
    plane: PlaneId,
    sample: LoopRestorationSourceSample,
    curr_context: &'static str,
    cdef_context: &'static str,
) -> Result<()> {
    if first_sample.is_none() {
        *first_sample = Some(wienerns_lr_source_read_sample(plane, sample));
    }
    match sample.source {
        LoopRestorationSource::CurrFrame => {
            increment_wienerns_lr_counter(curr_frame_source_reads, curr_context)
        }
        LoopRestorationSource::CdefFrame => {
            increment_wienerns_lr_counter(cdef_frame_source_reads, cdef_context)
        }
    }
}

fn observe_wienerns_lr_recon_source_sample(
    curr_frame_source_reads: &mut usize,
    cdef_frame_source_reads: &mut usize,
    sample: LoopRestorationSourceSample,
    curr_context: &'static str,
    cdef_context: &'static str,
) -> ReconResult<()> {
    match sample.source {
        LoopRestorationSource::CurrFrame => {
            increment_wienerns_lr_recon_counter(curr_frame_source_reads, curr_context)
        }
        LoopRestorationSource::CdefFrame => {
            increment_wienerns_lr_recon_counter(cdef_frame_source_reads, cdef_context)
        }
    }
}

pub(super) fn wienerns_lr_source_read_config(
    planes: &[LrPlaneParams],
    cfl_ds_filter_index: u8,
) -> WienerNsLrSourceReadConfig {
    let mut config = WienerNsLrSourceReadConfig::CONSERVATIVE;
    config.cfl_ds_filter_index = cfl_ds_filter_index;
    for plane in [PlaneId::U, PlaneId::V] {
        let Some(plane_params) = planes.get(plane.index()) else {
            continue;
        };
        if !plane_params.frame_filters_on {
            continue;
        }
        let Some(bank) = &plane_params.frame_filter_bank else {
            continue;
        };
        let Some(class) = bank.classes.first() else {
            continue;
        };
        for (tap_index, enabled) in config.chroma_luma_source_taps[plane.index()]
            .iter_mut()
            .enumerate()
        {
            *enabled = class
                .coeffs
                .get(WIENER_NS_CHROMA_LUMA_COEFF_OFFSET + tap_index)
                .is_none_or(|coefficient| *coefficient != 0);
        }
    }
    config
}

pub(super) fn derive_wienerns_lr_runtime_source_frontiers(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    planes: &[LrPlaneParams],
    chroma_format: ChromaFormatIdc,
    config: WienerNsLrSourceReadConfig,
    offset: ByteOffset,
    limits: DecodeLimits,
) -> Result<(
    Option<WienerNsLrClassifiedWienerFrontier>,
    WienerNsLrSourceReadFrontier,
)> {
    ensure_wienerns_lr_source_read_preconditions(active_source_blocks, planes, offset)?;
    let classified_source_reads =
        count_wienerns_lr_classified_wiener_source_reads(active_source_blocks, planes)?;
    let source_reads =
        count_wienerns_lr_source_reads(active_source_blocks, chroma_format, config, offset)?;
    let total_source_reads = classified_source_reads
        .checked_add(source_reads)
        .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr source-read count"))?;
    limits.ensure(
        DecodeLimitName::MaxLoopRestorationSourceReads,
        total_source_reads,
    )?;
    let classified_frontier = if classified_source_reads == 0 {
        None
    } else {
        Some(
            derive_wienerns_lr_classified_wiener_frontier_after_preflight(
                active_source_blocks,
                planes,
            )?,
        )
    };
    let source_read_frontier = derive_wienerns_lr_source_read_frontier_after_preflight(
        active_source_blocks,
        chroma_format,
        config,
        offset,
    )?;
    Ok((classified_frontier, source_read_frontier))
}

fn ensure_wienerns_lr_source_read_preconditions(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    planes: &[LrPlaneParams],
    offset: ByteOffset,
) -> Result<()> {
    for block in active_source_blocks {
        let plane = match block.plane {
            1 | 2 => block.plane,
            _ => continue,
        };
        if planes.get(plane).is_some_and(|params| {
            params.restoration_type == FrameRestorationType::WienerNonsep
                && !params.frame_filters_on
        }) {
            return Err(unsupported_feature_at(
                "unsupported_wienerns_lr_unit_chroma_filter_values",
                offset,
                "Per-unit chroma Wiener NS coefficients are not retained for luma-source tap selection.",
                AC0EJ3_LR_SOURCE_READ_MATRIX_ROW,
                AC0EJ3_LR_SOURCE_READ_FEATURE_ID,
                "5.20.10.6",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn derive_wienerns_lr_classified_wiener_frontier(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    planes: &[LrPlaneParams],
    limits: DecodeLimits,
) -> Result<Option<WienerNsLrClassifiedWienerFrontier>> {
    let source_reads =
        count_wienerns_lr_classified_wiener_source_reads(active_source_blocks, planes)?;
    limits.ensure(DecodeLimitName::MaxLoopRestorationSourceReads, source_reads)?;
    if source_reads == 0 {
        return Ok(None);
    }
    derive_wienerns_lr_classified_wiener_frontier_after_preflight(active_source_blocks, planes)
        .map(Some)
}

fn count_wienerns_lr_classified_wiener_source_reads(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    planes: &[LrPlaneParams],
) -> Result<u64> {
    if !wienerns_lr_uses_classified_luma(active_source_blocks, planes) {
        return Ok(0);
    }
    let luma_blocks = active_source_blocks
        .iter()
        .filter(|block| block.plane == 0)
        .count();
    let luma_blocks = u64::try_from(luma_blocks)
        .map_err(|_| source_read_arithmetic_overflow("wiener ns lr classified block count"))?;
    let reads_per_block = pc_wiener_feature_window_points()?
        .checked_mul(PC_WIENER_SOURCE_READS_PER_FEATURE)
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener classified source reads"))?;
    luma_blocks
        .checked_mul(reads_per_block)
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener classified source reads"))
}

fn wienerns_lr_uses_classified_luma(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    planes: &[LrPlaneParams],
) -> bool {
    active_source_blocks.iter().any(|block| block.plane == 0)
        && planes.first().is_some_and(|plane| {
            plane.frame_filters_on && plane.num_filter_classes.unwrap_or(1) > 1
        })
}

fn pc_wiener_feature_window_points() -> Result<u64> {
    let side = PC_WIENER_LEAD
        .checked_add(PC_WIENER_LAG)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener feature window side"))?;
    let side = u64::try_from(side)
        .map_err(|_| source_read_arithmetic_overflow("pc wiener feature window side"))?;
    side.checked_mul(side)
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener feature window points"))
}

fn derive_wienerns_lr_classified_wiener_frontier_after_preflight(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    planes: &[LrPlaneParams],
) -> Result<WienerNsLrClassifiedWienerFrontier> {
    let mut summary = WienerNsLrClassifiedWienerFrontier::default();
    if !wienerns_lr_uses_classified_luma(active_source_blocks, planes) {
        return Ok(summary);
    }

    for block in active_source_blocks
        .iter()
        .filter(|block| block.plane == PlaneId::Y.index())
    {
        let bounds = wienerns_lr_classified_luma_source_bounds(block);
        let block_start_x = (block.x >> 6) << 6;
        let block_end_x = pc_wiener_block_end_x(block, block_start_x)?;
        let x = usize_to_source_coordinate(block.x, "pc wiener classified block x")?;
        let y = usize_to_source_coordinate(block.y, "pc wiener classified block y")?;
        derive_pc_wiener_box_features_source_reads(
            &mut summary,
            block,
            &bounds,
            block_start_x,
            block_end_x,
            x,
            y,
        )?;
        increment_wienerns_lr_counter(
            &mut summary.blocks_resolved,
            "pc wiener classified block count",
        )?;
    }
    Ok(summary)
}

fn wienerns_lr_classified_luma_source_bounds(
    block: &crate::tile_payload::WienerNsLrSourceBlock,
) -> LoopRestorationSourceBounds {
    wienerns_lr_source_block_bounds(block, 0, 0)
}

fn wienerns_lr_source_block_bounds(
    block: &crate::tile_payload::WienerNsLrSourceBlock,
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

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn derive_wienerns_lr_classified_wiener_values_frontier<T, FS, FT>(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    planes: &[LrPlaneParams],
    bit_depth: BitDepth,
    base_q_idx: u32,
    limits: DecodeLimits,
    mut source_sample: FS,
    mut tx_skip: FT,
) -> Result<Option<WienerNsLrClassifiedWienerValuesFrontier>>
where
    T: ReconSample,
    FS: FnMut(WienerNsLrClassifiedWienerValueSourceSample) -> ReconResult<T>,
    FT: FnMut(WienerNsLrTxSkipLookup) -> ReconResult<i32>,
{
    let source_reads =
        count_wienerns_lr_classified_wiener_source_reads(active_source_blocks, planes)?;
    limits.ensure(DecodeLimitName::MaxLoopRestorationSourceReads, source_reads)?;
    if source_reads == 0 {
        return Ok(None);
    }

    let mut summary = WienerNsLrClassifiedWienerValuesFrontier::default();
    for block in active_source_blocks
        .iter()
        .filter(|block| block.plane == PlaneId::Y.index())
    {
        let bounds = wienerns_lr_classified_luma_source_bounds(block);
        let block_start_x = (block.x >> 6) << 6;
        let block_end_x = pc_wiener_block_end_x(block, block_start_x)?;
        let params = PcWienerClassifyParams {
            x: usize_to_source_coordinate(block.x, "pc wiener classified block x")?,
            y: usize_to_source_coordinate(block.y, "pc wiener classified block y")?,
            bit_depth,
            base_q_idx,
            block_start_x,
            block_end_x,
            luma_stripe_start_y: block.luma_stripe_start_y,
            luma_stripe_end_y: block.luma_stripe_end_y,
            tile_start_y: mi_to_luma_start(
                block.tile_mi_row_start,
                "pc wiener classified tile start y",
            )?,
            tile_end_y: mi_to_luma_end(block.tile_mi_row_end, "pc wiener classified tile end y")?,
        };
        let classification = pc_wiener_classify::<T, _, _>(
            &params,
            |x, y| {
                read_wienerns_lr_classified_wiener_value_source_sample(
                    &mut summary,
                    &mut source_sample,
                    x,
                    y,
                    &bounds,
                )
            },
            |lookup| tx_skip(wienerns_lr_tx_skip_lookup_from_pc(lookup)),
        )?;
        record_wienerns_lr_filter_class(&mut summary, block, classification.class)?;
    }
    Ok(Some(summary))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn derive_wienerns_lr_classified_wiener_storage_frontier<T>(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    planes: &[LrPlaneParams],
    bit_depth: BitDepth,
    base_q_idx: u32,
    limits: DecodeLimits,
    storage: WienerNsLrClassifiedWienerStorageInputs<'_, T>,
) -> Result<Option<WienerNsLrClassifiedWienerValuesFrontier>>
where
    T: ReconSample,
{
    let curr_frame = storage.curr_frame.as_frame_ref();
    let cdef_frame = storage.cdef_frame.as_frame_ref();
    derive_wienerns_lr_classified_wiener_values_frontier(
        active_source_blocks,
        planes,
        bit_depth,
        base_q_idx,
        limits,
        |read| {
            loop_restoration_source_sample_value(
                PlaneId::Y,
                read.input_x,
                read.input_y,
                &read.bounds,
                curr_frame,
                cdef_frame,
            )
            .map(|sample| sample.value)
        },
        |lookup| storage.tx_skip_grid.lookup(lookup),
    )
}

fn read_wienerns_lr_classified_wiener_value_source_sample<T, FS>(
    summary: &mut WienerNsLrClassifiedWienerValuesFrontier,
    source_sample: &mut FS,
    input_x: isize,
    input_y: isize,
    bounds: &LoopRestorationSourceBounds,
) -> ReconResult<T>
where
    T: ReconSample,
    FS: FnMut(WienerNsLrClassifiedWienerValueSourceSample) -> ReconResult<T>,
{
    let sample = loop_restoration_source_sample(PlaneId::Y, input_x, input_y, bounds)?;
    let read = record_wienerns_lr_classified_wiener_value_source_read(
        summary, input_x, input_y, *bounds, sample,
    )?;
    source_sample(read)
}

fn record_wienerns_lr_classified_wiener_value_source_read(
    summary: &mut WienerNsLrClassifiedWienerValuesFrontier,
    input_x: isize,
    input_y: isize,
    bounds: LoopRestorationSourceBounds,
    sample: LoopRestorationSourceSample,
) -> ReconResult<WienerNsLrClassifiedWienerValueSourceSample> {
    let read = WienerNsLrClassifiedWienerValueSourceSample {
        input_x,
        input_y,
        bounds,
        sample: wienerns_lr_source_read_sample(PlaneId::Y, sample),
    };
    if summary.first_sample.is_none() {
        summary.first_sample = Some(read);
    }
    increment_wienerns_lr_recon_counter(
        &mut summary.source_reads_resolved,
        "pc wiener classified value source-read count",
    )?;
    observe_wienerns_lr_recon_source_sample(
        &mut summary.curr_frame_source_reads,
        &mut summary.cdef_frame_source_reads,
        sample,
        "pc wiener classified value curr-frame source-read count",
        "pc wiener classified value cdef-frame source-read count",
    )?;
    Ok(read)
}

fn record_wienerns_lr_filter_class(
    summary: &mut WienerNsLrClassifiedWienerValuesFrontier,
    block: &crate::tile_payload::WienerNsLrSourceBlock,
    class: u8,
) -> Result<()> {
    let filter_class = WienerNsLrFilterClassValue {
        x: block.x,
        y: block.y,
        row: block.y >> 2,
        col: block.x >> 2,
        class,
    };
    if summary.first_filter_class.is_none() {
        summary.first_filter_class = Some(filter_class);
    }
    increment_wienerns_lr_counter(
        &mut summary.blocks_resolved,
        "pc wiener classified value block count",
    )?;
    increment_wienerns_lr_counter(
        &mut summary.filter_classes_resolved,
        "pc wiener classified filter-class count",
    )?;
    Ok(())
}

const fn wienerns_lr_tx_skip_lookup_from_pc(
    lookup: PcWienerTxSkipLookup,
) -> WienerNsLrTxSkipLookup {
    WienerNsLrTxSkipLookup {
        x: lookup.x,
        y: lookup.y,
        row: lookup.row,
        col: lookup.col,
    }
}

fn pc_wiener_block_end_x(
    block: &crate::tile_payload::WienerNsLrSourceBlock,
    block_start_x: usize,
) -> Result<usize> {
    let tile_end_x = mi_to_luma_end(block.tile_mi_col_end, "pc wiener classified tile end x")?;
    let block_end_x = block_start_x
        .checked_add(63)
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener classified block end x"))?;
    Ok(tile_end_x.min(block_end_x))
}

fn derive_pc_wiener_box_features_source_reads(
    summary: &mut WienerNsLrClassifiedWienerFrontier,
    block: &crate::tile_payload::WienerNsLrSourceBlock,
    bounds: &LoopRestorationSourceBounds,
    block_start_x: usize,
    block_end_x: usize,
    x: isize,
    y: isize,
) -> Result<()> {
    for dy in -PC_WIENER_LEAD..=PC_WIENER_LAG {
        for dx in -PC_WIENER_LEAD..=PC_WIENER_LAG {
            let feature_x = source_read_coordinate_add(x, dx, "pc wiener feature x")?;
            let feature_y = source_read_coordinate_add(y, dy, "pc wiener feature y")?;
            derive_pc_wiener_feature_source_reads(
                summary,
                block,
                bounds,
                block_start_x,
                block_end_x,
                feature_x,
                feature_y,
            )?;
        }
    }
    Ok(())
}

fn derive_pc_wiener_feature_source_reads(
    summary: &mut WienerNsLrClassifiedWienerFrontier,
    block: &crate::tile_payload::WienerNsLrSourceBlock,
    bounds: &LoopRestorationSourceBounds,
    block_start_x: usize,
    block_end_x: usize,
    x: isize,
    y: isize,
) -> Result<()> {
    let block_end_x_plus_two = block_end_x
        .checked_add(2)
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener classified block end x"))?;
    let block_end_x_plus_two =
        usize_to_source_coordinate(block_end_x_plus_two, "pc wiener classified block end x")?;
    let x = x.min(block_end_x_plus_two);

    for (dx, dy) in PC_WIENER_FEATURE_SOURCE_READ_OFFSETS {
        record_wienerns_lr_classified_source_read(
            summary,
            pc_wiener_feature_source_read_coordinate(
                x,
                dx,
                "pc wiener feature left x",
                "pc wiener feature right x",
            )?,
            pc_wiener_feature_source_read_coordinate(
                y,
                dy,
                "pc wiener feature up y",
                "pc wiener feature down y",
            )?,
            bounds,
        )?;
    }
    record_wienerns_lr_classified_tx_skip_lookup(summary, block, block_start_x, block_end_x, x, y)?;
    increment_wienerns_lr_counter(
        &mut summary.feature_points_resolved,
        "pc wiener classified feature point count",
    )?;
    Ok(())
}

fn pc_wiener_feature_source_read_coordinate(
    coordinate: isize,
    delta: isize,
    negative_context: &'static str,
    positive_context: &'static str,
) -> Result<isize> {
    match delta.cmp(&0) {
        std::cmp::Ordering::Less => source_read_coordinate_add(coordinate, delta, negative_context),
        std::cmp::Ordering::Equal => Ok(coordinate),
        std::cmp::Ordering::Greater => {
            source_read_coordinate_add(coordinate, delta, positive_context)
        }
    }
}

fn record_wienerns_lr_classified_source_read(
    summary: &mut WienerNsLrClassifiedWienerFrontier,
    x: isize,
    y: isize,
    bounds: &LoopRestorationSourceBounds,
) -> Result<()> {
    let next_reads = next_wienerns_lr_counter(
        summary.source_reads_resolved,
        "pc wiener classified source-read count",
    )?;
    let sample = loop_restoration_source_sample(PlaneId::Y, x, y, bounds)?;
    observe_wienerns_lr_source_sample(
        &mut summary.first_sample,
        &mut summary.curr_frame_source_reads,
        &mut summary.cdef_frame_source_reads,
        PlaneId::Y,
        sample,
        "pc wiener curr-frame source-read count",
        "pc wiener cdef-frame source-read count",
    )?;
    summary.source_reads_resolved = next_reads;
    Ok(())
}

fn record_wienerns_lr_classified_tx_skip_lookup(
    summary: &mut WienerNsLrClassifiedWienerFrontier,
    block: &crate::tile_payload::WienerNsLrSourceBlock,
    block_start_x: usize,
    block_end_x: usize,
    x: isize,
    y: isize,
) -> Result<()> {
    let x = clip_source_read_coordinate(
        x,
        block_start_x,
        block_end_x,
        "pc wiener tx-skip x coordinate",
    )?;
    let y = clip_source_read_coordinate(
        y,
        block.luma_stripe_start_y,
        block.luma_stripe_end_y,
        "pc wiener tx-skip stripe y coordinate",
    )?;
    let tile_start_y = mi_to_luma_start(block.tile_mi_row_start, "pc wiener tx-skip tile start y")?;
    let tile_end_y = mi_to_luma_end(block.tile_mi_row_end, "pc wiener tx-skip tile end y")?;
    if tile_start_y > tile_end_y {
        return Err(source_read_arithmetic_overflow(
            "pc wiener tx-skip tile y range",
        ));
    }
    let y = y.clamp(tile_start_y, tile_end_y);
    let lookup = WienerNsLrTxSkipLookup {
        x,
        y,
        row: y >> 2,
        col: x >> 2,
    };
    if summary.first_tx_skip_lookup.is_none() {
        summary.first_tx_skip_lookup = Some(lookup);
    }
    increment_wienerns_lr_counter(
        &mut summary.tx_skip_lookups_resolved,
        "pc wiener tx-skip lookup count",
    )?;
    Ok(())
}

#[cfg(test)]
pub(super) fn derive_wienerns_lr_source_read_frontier(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    chroma_format: ChromaFormatIdc,
    config: WienerNsLrSourceReadConfig,
    offset: ByteOffset,
    limits: DecodeLimits,
) -> Result<WienerNsLrSourceReadFrontier> {
    let source_read_count =
        count_wienerns_lr_source_reads(active_source_blocks, chroma_format, config, offset)?;
    limits.ensure(
        DecodeLimitName::MaxLoopRestorationSourceReads,
        source_read_count,
    )?;
    derive_wienerns_lr_source_read_frontier_after_preflight(
        active_source_blocks,
        chroma_format,
        config,
        offset,
    )
}

fn derive_wienerns_lr_source_read_frontier_after_preflight(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    chroma_format: ChromaFormatIdc,
    config: WienerNsLrSourceReadConfig,
    offset: ByteOffset,
) -> Result<WienerNsLrSourceReadFrontier> {
    let (subsampling_x, subsampling_y) = chroma_subsampling(chroma_format);
    let mut summary = WienerNsLrSourceReadFrontier::default();

    for block in active_source_blocks {
        let plane = wienerns_lr_source_plane(block.plane, chroma_format, offset)?;
        let bounds = wienerns_lr_source_block_bounds(block, subsampling_x, subsampling_y);
        for y_offset in 0..block.height {
            let y = wienerns_lr_block_sample_coordinate(
                block.y,
                y_offset,
                "wiener ns lr source y coordinate",
            )?;
            for x_offset in 0..block.width {
                let x = wienerns_lr_block_sample_coordinate(
                    block.x,
                    x_offset,
                    "wiener ns lr source x coordinate",
                )?;
                derive_wienerns_lr_output_sample_source_reads(
                    &mut summary,
                    config,
                    plane,
                    x,
                    y,
                    &bounds,
                    block.frame_luma_end_y,
                )?;
            }
        }
        increment_wienerns_lr_counter(
            &mut summary.blocks_resolved,
            "wiener ns lr source block count",
        )?;
    }
    Ok(summary)
}

fn wienerns_lr_block_sample_coordinate(
    origin: usize,
    offset: usize,
    context: &'static str,
) -> Result<isize> {
    let coordinate = origin
        .checked_add(offset)
        .ok_or_else(|| source_read_arithmetic_overflow(context))?;
    usize_to_source_coordinate(coordinate, context)
}

fn count_wienerns_lr_source_reads(
    active_source_blocks: &[crate::tile_payload::WienerNsLrSourceBlock],
    chroma_format: ChromaFormatIdc,
    config: WienerNsLrSourceReadConfig,
    offset: ByteOffset,
) -> Result<u64> {
    let (subsampling_x, subsampling_y) = chroma_subsampling(chroma_format);
    let luma_reads_per_chroma_sample =
        wienerns_lr_luma_reads_per_chroma_sample(subsampling_x, subsampling_y, config);
    let mut total = 0u64;
    for block in active_source_blocks {
        let plane = wienerns_lr_source_plane(block.plane, chroma_format, offset)?;
        let output_samples = block
            .width
            .checked_mul(block.height)
            .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr output sample count"))?;
        let output_samples = u64::try_from(output_samples)
            .map_err(|_| source_read_arithmetic_overflow("wiener ns lr output sample count"))?;
        let reads_per_sample =
            wienerns_lr_source_reads_per_sample(plane, config, luma_reads_per_chroma_sample)?;
        let block_reads = output_samples
            .checked_mul(reads_per_sample)
            .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr source-read count"))?;
        total = total
            .checked_add(block_reads)
            .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr source-read count"))?;
    }
    Ok(total)
}

const fn wienerns_lr_luma_reads_per_chroma_sample(
    subsampling_x: u8,
    subsampling_y: u8,
    config: WienerNsLrSourceReadConfig,
) -> u64 {
    if subsampling_x == 1 && subsampling_y == 1 && config.cfl_ds_filter_index != 2 {
        4
    } else {
        1
    }
}

fn wienerns_lr_source_reads_per_sample(
    plane: PlaneId,
    config: WienerNsLrSourceReadConfig,
    luma_reads_per_chroma_sample: u64,
) -> Result<u64> {
    match plane {
        PlaneId::Y => Ok(1 + WIENER_NS_LUMA_SOURCE_TAPS.len() as u64),
        PlaneId::U | PlaneId::V => {
            let active_luma_taps = config
                .chroma_luma_source_taps(plane)
                .iter()
                .filter(|enabled| **enabled)
                .count();
            let active_luma_taps = u64::try_from(active_luma_taps).map_err(|_| {
                source_read_arithmetic_overflow("wiener ns lr chroma luma source tap count")
            })?;
            let luma_reads = active_luma_taps
                .checked_add(1)
                .and_then(|reads| reads.checked_mul(luma_reads_per_chroma_sample))
                .ok_or_else(|| {
                    source_read_arithmetic_overflow("wiener ns lr chroma luma source-read count")
                })?;
            (1 + WIENER_NS_CHROMA_SOURCE_TAPS.len() as u64)
                .checked_add(luma_reads)
                .ok_or_else(|| {
                    source_read_arithmetic_overflow("wiener ns lr chroma source-read count")
                })
        }
    }
}

fn derive_wienerns_lr_output_sample_source_reads(
    summary: &mut WienerNsLrSourceReadFrontier,
    config: WienerNsLrSourceReadConfig,
    plane: PlaneId,
    x: isize,
    y: isize,
    bounds: &LoopRestorationSourceBounds,
    frame_luma_end_y: usize,
) -> Result<()> {
    record_wienerns_lr_source_read(summary, plane, x, y, bounds)?;
    match plane {
        PlaneId::Y => {
            record_wienerns_lr_tap_source_reads(
                summary,
                plane,
                (x, y),
                bounds,
                &WIENER_NS_LUMA_SOURCE_TAPS,
                ("wiener ns lr luma tap x", "wiener ns lr luma tap y"),
            )?;
        }
        PlaneId::U | PlaneId::V => {
            record_wienerns_lr_tap_source_reads(
                summary,
                plane,
                (x, y),
                bounds,
                &WIENER_NS_CHROMA_SOURCE_TAPS,
                ("wiener ns lr chroma tap x", "wiener ns lr chroma tap y"),
            )?;
            record_wienerns_lr_chroma_luma_source_reads(
                summary,
                config,
                x,
                y,
                bounds,
                frame_luma_end_y,
            )?;
            for ((dy, dx), luma_tap_enabled) in WIENER_NS_CHROMA_SOURCE_TAPS
                .into_iter()
                .zip(config.chroma_luma_source_taps(plane))
            {
                if !luma_tap_enabled {
                    continue;
                }
                let tap_x = source_read_coordinate_add(x, dx, "wiener ns lr chroma luma tap x")?;
                let tap_y = source_read_coordinate_add(y, dy, "wiener ns lr chroma luma tap y")?;
                record_wienerns_lr_chroma_luma_source_reads(
                    summary,
                    config,
                    tap_x,
                    tap_y,
                    bounds,
                    frame_luma_end_y,
                )?;
            }
        }
    }
    increment_wienerns_lr_counter(
        &mut summary.output_samples_resolved,
        "wiener ns lr output sample count",
    )?;
    Ok(())
}

fn record_wienerns_lr_tap_source_reads(
    summary: &mut WienerNsLrSourceReadFrontier,
    plane: PlaneId,
    origin: (isize, isize),
    bounds: &LoopRestorationSourceBounds,
    taps_yx: &[(isize, isize)],
    contexts: (&'static str, &'static str),
) -> Result<()> {
    let (x, y) = origin;
    let (x_context, y_context) = contexts;
    for &(dy, dx) in taps_yx {
        let tap_x = source_read_coordinate_add(x, dx, x_context)?;
        let tap_y = source_read_coordinate_add(y, dy, y_context)?;
        record_wienerns_lr_source_read(summary, plane, tap_x, tap_y, bounds)?;
    }
    Ok(())
}

fn record_wienerns_lr_source_read(
    summary: &mut WienerNsLrSourceReadFrontier,
    plane: PlaneId,
    x: isize,
    y: isize,
    bounds: &LoopRestorationSourceBounds,
) -> Result<()> {
    let next_reads = next_wienerns_lr_counter(
        summary.source_reads_resolved,
        "wiener ns lr source-read count",
    )?;
    let sample = loop_restoration_source_sample(plane, x, y, bounds)?;
    observe_wienerns_lr_source_sample(
        &mut summary.first_sample,
        &mut summary.curr_frame_source_reads,
        &mut summary.cdef_frame_source_reads,
        plane,
        sample,
        "wiener ns lr curr-frame source-read count",
        "wiener ns lr cdef-frame source-read count",
    )?;
    summary.source_reads_resolved = next_reads;
    Ok(())
}

pub(super) fn record_wienerns_lr_chroma_luma_source_reads(
    summary: &mut WienerNsLrSourceReadFrontier,
    config: WienerNsLrSourceReadConfig,
    chroma_x: isize,
    chroma_y: isize,
    bounds: &LoopRestorationSourceBounds,
    frame_luma_end_y: usize,
) -> Result<()> {
    let sub_x = usize::from(bounds.subsampling_x);
    let sub_y = usize::from(bounds.subsampling_y);
    let luma_x =
        scale_chroma_source_coordinate(chroma_x, sub_x, "wiener ns lr chroma luma x coordinate")?;
    let luma_y =
        scale_chroma_source_coordinate(chroma_y, sub_y, "wiener ns lr chroma luma y coordinate")?;
    let last_x = bounds
        .luma_end_x
        .checked_sub(sub_x)
        .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr luma source last x"))?;
    let last_y = frame_luma_end_y
        .checked_sub(sub_y)
        .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr luma source last y"))?;
    let luma_x = clip_source_read_coordinate(
        luma_x,
        bounds.luma_start_x,
        last_x,
        "wiener ns lr clipped luma source x",
    )?;
    let luma_y =
        clip_source_read_coordinate(luma_y, 0, last_y, "wiener ns lr clipped luma source y")?;

    if bounds.subsampling_x == 1 && bounds.subsampling_y == 1 && config.cfl_ds_filter_index != 2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let read_x = luma_x.checked_add(dx).ok_or_else(|| {
                    source_read_arithmetic_overflow("wiener ns lr 420 luma source x")
                })?;
                let read_y = luma_y.checked_add(dy).ok_or_else(|| {
                    source_read_arithmetic_overflow("wiener ns lr 420 luma source y")
                })?;
                record_wienerns_lr_source_read(
                    summary,
                    PlaneId::Y,
                    usize_to_source_coordinate(read_x, "wiener ns lr 420 luma source x")?,
                    usize_to_source_coordinate(read_y, "wiener ns lr 420 luma source y")?,
                    bounds,
                )?;
            }
        }
    } else {
        record_wienerns_lr_source_read(
            summary,
            PlaneId::Y,
            usize_to_source_coordinate(luma_x, "wiener ns lr luma source x")?,
            usize_to_source_coordinate(luma_y, "wiener ns lr luma source y")?,
            bounds,
        )?;
    }
    Ok(())
}

fn has_wienerns_frame_filter_bank(core: &FrameHeaderCore) -> bool {
    core.lr_params.as_ref().is_some_and(|lr| {
        lr.planes
            .iter()
            .any(|plane| plane.frame_filter_bank.is_some())
    })
}

fn bit_depth_bits(sequence: &SequenceHeader) -> u32 {
    match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight => 8,
        BitDepthIdc::Ten => 10,
    }
}
