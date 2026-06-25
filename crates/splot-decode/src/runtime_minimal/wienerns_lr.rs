// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Wiener NS loop-restoration runtime frontier helpers.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrPlaneParams, TxMode};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
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
    TransformToolResidualPolicy, decode_general_intra_block_modes,
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
    derive_tile_plan, ensure_sequence_chroma_tools_before_tile_decode, unsupported_at,
    unsupported_feature_at,
};

const LR_MI_SIZE: usize = 4;
const PC_WIENER_LEAD: isize = 1;
const PC_WIENER_LAG: isize = 4;
const PC_WIENER_SOURCE_READS_PER_FEATURE: u64 = 7;
const LR_RETAINED_FRAME_BUFFERS: u64 = 2;

mod diagnostics;
mod live_storage;
mod tx_records;

use self::tx_records::WienerNsLrLiveTransformRecordHandoff;
pub(super) use self::tx_records::WienerNsLrTxSkipTransformRecord;

use self::diagnostics::transform_tool_residual_frontier;
pub(super) use self::diagnostics::{
    map_wienerns_lr_unit_frontier_error, wienerns_lr_live_frame_samples_unpopulated_error,
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

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private source-read frontier proof state is consumed by tests until filtering consumes it"
    )
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrSourceReadFrontier {
    pub(super) blocks_resolved: usize,
    pub(super) output_samples_resolved: usize,
    pub(super) source_reads_resolved: usize,
    pub(super) curr_frame_source_reads: usize,
    pub(super) cdef_frame_source_reads: usize,
    pub(super) first_sample: Option<WienerNsLrSourceReadSample>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private source-read frontier proof state is consumed by tests until filtering consumes it"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrSourceReadSample {
    pub(super) plane: PlaneId,
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) source: LoopRestorationSource,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener frontier proof state is consumed by tests until filtering consumes it"
    )
)]
#[derive(Clone, Debug, Eq, PartialEq)]
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

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener frontier proof state is consumed by tests until filtering consumes it"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrTxSkipLookup {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) row: usize,
    pub(super) col: usize,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener storage proof waits for live tx-skip retention"
    )
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrTxSkipGrid {
    rows: usize,
    cols: usize,
    values: Vec<u8>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener storage proof waits for live tx-skip retention"
    )
)]
impl WienerNsLrTxSkipGrid {
    pub(super) fn new(rows: usize, cols: usize, values: Vec<u8>) -> ReconResult<Self> {
        if rows == 0 || cols == 0 {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LrTxSkip grid dimensions",
            });
        }
        let expected = rows
            .checked_mul(cols)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "LrTxSkip grid sample count",
            })?;
        if values.len() != expected {
            return Err(ReconError::BufferLengthMismatch {
                expected,
                actual: values.len(),
            });
        }
        Ok(Self { rows, cols, values })
    }

    pub(super) fn lookup(&self, lookup: WienerNsLrTxSkipLookup) -> ReconResult<i32> {
        if lookup.row >= self.rows || lookup.col >= self.cols {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LrTxSkip grid lookup",
            });
        }
        let row_start =
            lookup
                .row
                .checked_mul(self.cols)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "LrTxSkip grid row offset",
                })?;
        let index = row_start
            .checked_add(lookup.col)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "LrTxSkip grid sample offset",
            })?;
        let Some(value) = self.values.get(index) else {
            return Err(ReconError::BufferLengthMismatch {
                expected: index.saturating_add(1),
                actual: self.values.len(),
            });
        };
        Ok(i32::from(*value))
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private tx-skip retention proof waits for live transform-record handoff"
    )
)]
pub(super) fn derive_wienerns_lr_tx_skip_grid_retention(
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
) -> ReconResult<WienerNsLrTxSkipGrid> {
    if rows == 0 || cols == 0 {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip grid dimensions",
        });
    }
    let expected = rows
        .checked_mul(cols)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "LrTxSkip grid sample count",
        })?;
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
    let end_row = record
        .row
        .checked_add(record.rows)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "LrTxSkip transform record row extent",
        })?;
    let end_col = record
        .col
        .checked_add(record.cols)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "LrTxSkip transform record column extent",
        })?;
    if end_row > rows || end_col > cols {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "LrTxSkip transform record bounds",
        });
    }

    for row in record.row..end_row {
        let row_start = row
            .checked_mul(cols)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "LrTxSkip grid row offset",
            })?;
        for col in record.col..end_col {
            let index = row_start
                .checked_add(col)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "LrTxSkip grid sample offset",
                })?;
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

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener storage proof waits for live frame and tx-skip retention"
    )
)]
#[derive(Clone, Copy, Debug)]
pub(super) struct WienerNsLrClassifiedWienerStorageInputs<'a, T: ReconSample> {
    pub(super) curr_frame: &'a DecodedFrame<T>,
    pub(super) cdef_frame: &'a DecodedFrame<T>,
    pub(super) tx_skip_grid: &'a WienerNsLrTxSkipGrid,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private runtime storage-retention frontier is consumed by tests until filtering consumes it"
    )
)]
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

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener value frontier proof state waits for real runtime storage"
    )
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrClassifiedWienerValuesFrontier {
    pub(super) blocks_resolved: usize,
    pub(super) source_reads_resolved: usize,
    pub(super) curr_frame_source_reads: usize,
    pub(super) cdef_frame_source_reads: usize,
    pub(super) filter_classes_resolved: usize,
    pub(super) first_sample: Option<WienerNsLrClassifiedWienerValueSourceSample>,
    pub(super) first_filter_class: Option<WienerNsLrFilterClassValue>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener value frontier proof state waits for real runtime storage"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrFilterClassValue {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) class: u8,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener value frontier proof state waits for real runtime storage"
    )
)]
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
    options: DecodeOptions,
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
    let lr_frontier = crate::tile_payload::consume_minimal_runtime_lr_unit_frontier(
        tile,
        sequence,
        core,
        options.limits(),
    )
    .map_err(|err| map_wienerns_lr_unit_frontier_error(err, key_envelope.offset))?;
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

pub(super) fn derive_wienerns_lr_runtime_storage_retention_frontier(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    offset: ByteOffset,
    limits: DecodeLimits,
) -> Result<WienerNsLrRuntimeStorageRetentionFrontier> {
    // `FrameSize` carries the derived §6.17.4.1 dimensions used for storage.
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

    let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())
        .map_err(|source| DecodeError::Reconstruction { source })?;
    let budget = decoded_frame_storage_budget(
        frame_size,
        sequence.general.chroma_format_idc,
        decoded_storage_bytes_per_sample(bit_depth),
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

    let bytes_per_sample = decoded_storage_bytes_per_sample(bit_depth);
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
    // Charge private fail-closed storage by current slot sizes, not compact AV2 bytes.
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
    options: DecodeOptions,
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
        |work_unit, symbols, frontier, joint_modes, uses_mrls, _block_decoded| {
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
                    "minimal runtime reached active Wiener NS LR transform-record derivation, but a chroma-offset leaf would require carrying ancestor chroma residual coordinates before deriving fixed-largest chroma transform records",
                    "5.20.3.1",
                ));
            }
            let modes = decode_general_intra_block_modes(
                work_unit,
                symbols,
                GeneralIntraChromaToolConfig::disabled(),
                joint_modes,
                uses_mrls,
                frontier.b_size.index(),
                frontier.r,
                frontier.c,
                n4w,
                n4h,
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
                false,
                modes.coeff_uv_mode(),
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
            let u = decode_general_intra_plane_coeffs(
                work_unit,
                symbols,
                &mut coeff_ctx,
                1,
                chroma_tx,
                chroma_x,
                chroma_y,
                true,
                false,
                modes.coeff_uv_mode(),
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
            let _v = decode_general_intra_plane_coeffs(
                work_unit,
                symbols,
                &mut coeff_ctx,
                2,
                chroma_tx,
                chroma_x,
                chroma_y,
                true,
                !u.all_zero,
                modes.coeff_uv_mode(),
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
            Ok(crate::tile_payload::GeneralIntraLeafMode::luma(
                modes.intra_joint_mode,
                modes.y_mode,
                modes.uses_mrls,
            ))
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

    Ok(WienerNsLrLiveTransformRecordHandoff {
        tx_skip_rows,
        tx_skip_cols,
        records,
    })
}

pub(super) fn populate_wienerns_lr_live_tx_skip_from_transform_records(
    live_storage: &mut WienerNsLrLiveStorageAllocation,
    rows: usize,
    cols: usize,
    records: &[WienerNsLrTxSkipTransformRecord],
) -> Result<()> {
    let grid = derive_wienerns_lr_tx_skip_grid_retention(rows, cols, records)
        .map_err(|source| DecodeError::Reconstruction { source })?;
    live_storage.populate_tx_skip_grid(&grid)
}

fn ensure_fixed_largest_transform_record_tool_gates(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<()> {
    if core
        .tile_info
        .as_ref()
        .is_none_or(|tile_info| tile_info.tile_cols != 1 || tile_info.tile_rows != 1)
    {
        return Err(wienerns_lr_live_transform_record_tool_gate_error(
            offset,
            "tile_grid",
        ));
    }
    if core.allow_screen_content_tools != Some(false) || core.allow_intrabc != Some(false) {
        return Err(wienerns_lr_live_transform_record_tool_gate_error(
            offset,
            "screen_content_tools",
        ));
    }
    if sequence.intra.as_ref().is_none_or(|intra| {
        intra.enable_dip || intra.enable_ibp || intra.enable_mrls || intra.enable_intra_edge_filter
    }) {
        return Err(wienerns_lr_live_transform_record_tool_gate_error(
            offset,
            "intra_tool",
        ));
    }
    if sequence.transform_quant_entropy.as_ref().is_none_or(|tq| {
        tq.enable_fsc
            || tq.enable_cctx
            || tq.enable_idtx_intra
            || tq.enable_intra_ist
            || tq.enable_chroma_dctonly
    }) {
        return Err(wienerns_lr_live_transform_record_tool_gate_error(
            offset,
            "transform_tool",
        ));
    }
    if core
        .segmentation_params
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
    {
        return Err(wienerns_lr_live_transform_record_tool_gate_error(
            offset,
            "frame_tool",
        ));
    }
    Ok(())
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
    TX_WIDTH_LOG2
        .iter()
        .zip(TX_HEIGHT_LOG2.iter())
        .position(|(&tw, &th)| tw == w && th == h)
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
                "minimal runtime reached active Wiener NS LR transform-record derivation, but the full partition-tree walk could not update MI-size state for the next leaf block",
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
                "minimal runtime reached active Wiener NS LR transform-record derivation, but a parser fact required to seed the partition-tree walk is absent",
                "5.20.3.1",
            )
        }
        MinimalRuntimePartitionFrontierError::MiSizeState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_mi_size_state",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but MI-size state allocation for the partition-tree walk is outside the supported subset",
                "5.20.3.1",
            )
        }
        MinimalRuntimePartitionFrontierError::IntraJointModeState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_intra_joint_mode_state",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but intra joint-mode neighbour state allocation for the partition-tree walk is outside the supported subset",
                "8.3.2",
            )
        }
        MinimalRuntimePartitionFrontierError::UsesMrlsState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_setup_uses_mrls_state",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but UsesMrls neighbour state allocation for the partition-tree walk is outside the supported subset",
                "8.3.2",
            )
        }
        MinimalRuntimePartitionFrontierError::UnexpectedFrontier { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_unexpected_frontier",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but the initial partition frontier shape is outside the supported transform-record handoff subset",
                "5.20.3.1",
            )
        }
    }
}

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
            "minimal runtime reached active Wiener NS LR transform-record derivation, but the partition-tree walk could not maintain BlockDecoded state",
            "5.20.2.3",
        ),
        TilePartitionTraversalError::IntraYModeState(_) => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_intra_ymode_state",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but the partition-tree walk could not maintain SDP luma YMode state",
                "5.20.5.3",
            )
        }
        TilePartitionTraversalError::UsesMrlsState(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_uses_mrls_state",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but the partition-tree walk could not maintain UsesMrls state for MRL contexts",
            "8.3.2",
        ),
        TilePartitionTraversalError::Size(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_size",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but a partition-size lookup used by the full partition-tree walk failed",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::Allowed(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_allowed",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but the full partition-tree walk could not derive the allowed partition set",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::Decision(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_decision",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but partition decision syntax is outside the currently supported handoff subset",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::Symbol(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_symbol",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but symbol decoder initialization for the partition-tree walk failed",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::Cdf(_) => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_partition_cdf",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but a partition-tree CDF context lookup is outside the supported table shape",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::CoordinateUnderflow { .. }
        | TilePartitionTraversalError::CoordinateOverflow { .. }
        | TilePartitionTraversalError::CoordinateOffsetOverflow { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_coordinate_math",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but partition-tree coordinate arithmetic exceeded the supported checked range",
                "5.20.3.1",
            )
        }
        TilePartitionTraversalError::InvalidLoopRestorationUnitSize { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_invalid_lr_unit_size",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but a loop-restoration unit size did not map to the supported traversal state",
                "5.20.10.4",
            )
        }
        TilePartitionTraversalError::InvalidPartitionSubsize { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_invalid_partition_subsize",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but the selected partition produced no valid child block size",
                "5.20.3.1",
            )
        }
        TilePartitionTraversalError::TooManyChildCalls => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_too_many_child_calls",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but the partition-tree walk produced more child calls than the supported stack shape",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::NoBlockFrontier => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_no_block_frontier",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but the partition-tree walk reached no in-frame decode_block frontier",
            "5.20.3.1",
        ),
        TilePartitionTraversalError::MissingIntraLumaModeState { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_missing_intra_luma_mode_state",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but an intra luma/shared leaf did not provide YMode state for subsequent SDP chroma syntax",
                "5.20.5.3",
            )
        }
        TilePartitionTraversalError::MissingIntraUsesMrlsState { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_missing_uses_mrls_state",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but an intra luma/shared leaf did not provide UsesMrls state for subsequent MRL contexts",
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
        TilePartitionTraversalUnsupported::Sdp => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_sdp",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but SDP partition side effects are outside the supported partition-tree walk",
            "5.20.3.1",
        ),
        TilePartitionTraversalUnsupported::ExtendedSdp => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_extended_sdp",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but extended SDP region signaling is outside the supported partition-tree walk",
            "5.20.3.1",
        ),
        TilePartitionTraversalUnsupported::ReadLoopRestoration => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_read_loop_restoration",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but root read_lr syntax appeared in the partition traversal path instead of the earlier LR-unit frontier",
                "5.20.10.4",
            )
        }
        TilePartitionTraversalUnsupported::BruOrBridge => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_bru_or_bridge",
            offset,
            "minimal runtime reached active Wiener NS LR transform-record derivation, but BRU/bridge/inactive partition behavior is outside the supported handoff subset",
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
                "minimal runtime reached active Wiener NS LR transform-record derivation, but a mode-info CDF symbol read is outside the currently supported intra subset",
                "5.20.5.3",
            )
        }
        GeneralIntraBlockModeError::Literal { reason, .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                wienerns_lr_mode_literal_reason(reason),
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but a mode-info escape literal read is outside the currently supported intra subset",
                "5.20.5.3",
            )
        }
        GeneralIntraBlockModeError::UnsupportedYMode { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_y_mode",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but the block selected a luma intra mode outside the supported transform-record handoff subset",
                "5.20.5.3",
            )
        }
        GeneralIntraBlockModeError::InvalidUvMode { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_uv_mode",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but the block selected a chroma intra mode outside the supported transform-record handoff subset",
                "5.20.5.6",
            )
        }
        GeneralIntraBlockModeError::UnsupportedFscMode => wienerns_lr_transform_record_unsupported(
            scope,
            "unsupported_wienerns_lr_live_transform_record_fsc_mode",
            offset,
            "minimal runtime reached active Wiener NS LR and consumed mode-info syntax, but the block selected active FSC coefficient mode; deriving live LrTxSkip records for FSC blocks is outside this handoff subset",
            "5.20.5.3",
        ),
        GeneralIntraBlockModeError::InvalidFscBlockSizeIndex { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_fsc_bsize_group",
                offset,
                "minimal runtime reached active Wiener NS LR and consumed mode-info syntax, but the block size could not be mapped through Fsc_Bsize_Groups for fsc_mode",
                "8.3.2",
            )
        }
        GeneralIntraBlockModeError::InvalidCflMhDirBlockSizeIndex { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_cfl_mh_dir_size_group",
                offset,
                "minimal runtime reached active Wiener NS LR and consumed active CfL mode syntax, but the block size could not be mapped through Size_Group for cfl_mh_dir",
                "8.3.2",
            )
        }
        GeneralIntraBlockModeError::UnsupportedMhccpMode => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_mhccp_mode",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but the block selected active MHCCP chroma prediction",
                "5.20.5.6",
            )
        }
        GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder { .. } => {
            wienerns_lr_transform_record_unsupported(
                scope,
                "unsupported_wienerns_lr_live_transform_record_directional_neighbour_reorder",
                offset,
                "minimal runtime reached active Wiener NS LR transform-record derivation, but y_mode selection needs the directional-neighbour reorder path",
                "5.20.5.3",
            )
        }
    }
}

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
            "minimal runtime reached active Wiener NS LR and tried to derive live LrTxSkip records from key-tile transform coefficients, but the coefficient syntax is outside the transform-record handoff subset; live samples, filtering, output, and reference refresh are not applied",
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
                "minimal runtime retained an active chroma Wiener NS LR unit whose coefficients are coded per unit, but per-unit filter coefficients are not retained for §7.20.3 chroma luma-source tap selection before source-read derivation",
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
    let mut summary = WienerNsLrClassifiedWienerFrontier {
        blocks_resolved: 0,
        feature_points_resolved: 0,
        source_reads_resolved: 0,
        curr_frame_source_reads: 0,
        cdef_frame_source_reads: 0,
        tx_skip_lookups_resolved: 0,
        first_sample: None,
        first_tx_skip_lookup: None,
    };
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
        summary.blocks_resolved = summary
            .blocks_resolved
            .checked_add(1)
            .ok_or_else(|| source_read_arithmetic_overflow("pc wiener classified block count"))?;
    }
    Ok(summary)
}

fn wienerns_lr_classified_luma_source_bounds(
    block: &crate::tile_payload::WienerNsLrSourceBlock,
) -> LoopRestorationSourceBounds {
    LoopRestorationSourceBounds {
        luma_start_x: block.luma_start_x,
        luma_end_x: block.luma_end_x,
        luma_start_y: block.luma_start_y,
        luma_end_y: block.luma_end_y,
        luma_stripe_start_y: block.luma_stripe_start_y,
        luma_stripe_end_y: block.luma_stripe_end_y,
        subsampling_x: 0,
        subsampling_y: 0,
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener value frontier waits for decoded frame and tx-skip storage"
    )
)]
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

    let mut summary = WienerNsLrClassifiedWienerValuesFrontier {
        blocks_resolved: 0,
        source_reads_resolved: 0,
        curr_frame_source_reads: 0,
        cdef_frame_source_reads: 0,
        filter_classes_resolved: 0,
        first_sample: None,
        first_filter_class: None,
    };
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

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "private classified-Wiener storage proof waits for live frame and tx-skip retention"
    )
)]
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
        sample: WienerNsLrSourceReadSample {
            plane: PlaneId::Y,
            x: sample.x,
            y: sample.y,
            source: sample.source,
        },
    };
    if summary.first_sample.is_none() {
        summary.first_sample = Some(read);
    }
    summary.source_reads_resolved =
        summary
            .source_reads_resolved
            .checked_add(1)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "pc wiener classified value source-read count",
            })?;
    match sample.source {
        LoopRestorationSource::CurrFrame => {
            summary.curr_frame_source_reads = summary
                .curr_frame_source_reads
                .checked_add(1)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "pc wiener classified value curr-frame source-read count",
                })?;
        }
        LoopRestorationSource::CdefFrame => {
            summary.cdef_frame_source_reads = summary
                .cdef_frame_source_reads
                .checked_add(1)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "pc wiener classified value cdef-frame source-read count",
                })?;
        }
    }
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
    summary.blocks_resolved = summary
        .blocks_resolved
        .checked_add(1)
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener classified value block count"))?;
    summary.filter_classes_resolved =
        summary
            .filter_classes_resolved
            .checked_add(1)
            .ok_or_else(|| {
                source_read_arithmetic_overflow("pc wiener classified filter-class count")
            })?;
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

    record_wienerns_lr_classified_source_read(summary, x, y, bounds)?;
    record_wienerns_lr_classified_source_read(
        summary,
        x,
        source_read_coordinate_add(y, -1, "pc wiener feature up y")?,
        bounds,
    )?;
    record_wienerns_lr_classified_source_read(
        summary,
        x,
        source_read_coordinate_add(y, 1, "pc wiener feature down y")?,
        bounds,
    )?;
    record_wienerns_lr_classified_source_read(
        summary,
        source_read_coordinate_add(x, 1, "pc wiener feature right x")?,
        source_read_coordinate_add(y, -1, "pc wiener feature up y")?,
        bounds,
    )?;
    record_wienerns_lr_classified_source_read(
        summary,
        source_read_coordinate_add(x, -1, "pc wiener feature left x")?,
        source_read_coordinate_add(y, 1, "pc wiener feature down y")?,
        bounds,
    )?;
    record_wienerns_lr_classified_source_read(
        summary,
        source_read_coordinate_add(x, 1, "pc wiener feature right x")?,
        source_read_coordinate_add(y, 1, "pc wiener feature down y")?,
        bounds,
    )?;
    record_wienerns_lr_classified_source_read(
        summary,
        source_read_coordinate_add(x, -1, "pc wiener feature left x")?,
        source_read_coordinate_add(y, -1, "pc wiener feature up y")?,
        bounds,
    )?;
    record_wienerns_lr_classified_tx_skip_lookup(summary, block, block_start_x, block_end_x, x, y)?;
    summary.feature_points_resolved =
        summary
            .feature_points_resolved
            .checked_add(1)
            .ok_or_else(|| {
                source_read_arithmetic_overflow("pc wiener classified feature point count")
            })?;
    Ok(())
}

fn record_wienerns_lr_classified_source_read(
    summary: &mut WienerNsLrClassifiedWienerFrontier,
    x: isize,
    y: isize,
    bounds: &LoopRestorationSourceBounds,
) -> Result<()> {
    let next_reads = summary
        .source_reads_resolved
        .checked_add(1)
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener classified source-read count"))?;
    let sample = loop_restoration_source_sample(PlaneId::Y, x, y, bounds)?;
    if summary.first_sample.is_none() {
        summary.first_sample = Some(WienerNsLrSourceReadSample {
            plane: PlaneId::Y,
            x: sample.x,
            y: sample.y,
            source: sample.source,
        });
    }
    match sample.source {
        LoopRestorationSource::CurrFrame => {
            summary.curr_frame_source_reads = summary
                .curr_frame_source_reads
                .checked_add(1)
                .ok_or_else(|| {
                    source_read_arithmetic_overflow("pc wiener curr-frame source-read count")
                })?;
        }
        LoopRestorationSource::CdefFrame => {
            summary.cdef_frame_source_reads = summary
                .cdef_frame_source_reads
                .checked_add(1)
                .ok_or_else(|| {
                    source_read_arithmetic_overflow("pc wiener cdef-frame source-read count")
                })?;
        }
    }
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
    summary.tx_skip_lookups_resolved = summary
        .tx_skip_lookups_resolved
        .checked_add(1)
        .ok_or_else(|| source_read_arithmetic_overflow("pc wiener tx-skip lookup count"))?;
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
    let mut summary = WienerNsLrSourceReadFrontier {
        blocks_resolved: 0,
        output_samples_resolved: 0,
        source_reads_resolved: 0,
        curr_frame_source_reads: 0,
        cdef_frame_source_reads: 0,
        first_sample: None,
    };

    for block in active_source_blocks {
        let plane = wienerns_lr_source_plane(block.plane, chroma_format, offset)?;
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: block.luma_start_x,
            luma_end_x: block.luma_end_x,
            luma_start_y: block.luma_start_y,
            luma_end_y: block.luma_end_y,
            luma_stripe_start_y: block.luma_stripe_start_y,
            luma_stripe_end_y: block.luma_stripe_end_y,
            subsampling_x,
            subsampling_y,
        };
        for y_offset in 0..block.height {
            let y = block.y.checked_add(y_offset).ok_or_else(|| {
                source_read_arithmetic_overflow("wiener ns lr source y coordinate")
            })?;
            let y = isize::try_from(y)
                .map_err(|_| source_read_arithmetic_overflow("wiener ns lr source y coordinate"))?;
            for x_offset in 0..block.width {
                let x = block.x.checked_add(x_offset).ok_or_else(|| {
                    source_read_arithmetic_overflow("wiener ns lr source x coordinate")
                })?;
                let x = isize::try_from(x).map_err(|_| {
                    source_read_arithmetic_overflow("wiener ns lr source x coordinate")
                })?;
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
        summary.blocks_resolved = summary
            .blocks_resolved
            .checked_add(1)
            .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr source block count"))?;
    }
    Ok(summary)
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
            for (dy, dx) in WIENER_NS_LUMA_SOURCE_TAPS {
                let tap_x = source_read_coordinate_add(x, dx, "wiener ns lr luma tap x")?;
                let tap_y = source_read_coordinate_add(y, dy, "wiener ns lr luma tap y")?;
                record_wienerns_lr_source_read(summary, plane, tap_x, tap_y, bounds)?;
            }
        }
        PlaneId::U | PlaneId::V => {
            for (dy, dx) in WIENER_NS_CHROMA_SOURCE_TAPS {
                let tap_x = source_read_coordinate_add(x, dx, "wiener ns lr chroma tap x")?;
                let tap_y = source_read_coordinate_add(y, dy, "wiener ns lr chroma tap y")?;
                record_wienerns_lr_source_read(summary, plane, tap_x, tap_y, bounds)?;
            }
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
    summary.output_samples_resolved = summary
        .output_samples_resolved
        .checked_add(1)
        .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr output sample count"))?;
    Ok(())
}

fn record_wienerns_lr_source_read(
    summary: &mut WienerNsLrSourceReadFrontier,
    plane: PlaneId,
    x: isize,
    y: isize,
    bounds: &LoopRestorationSourceBounds,
) -> Result<()> {
    let next_reads = summary
        .source_reads_resolved
        .checked_add(1)
        .ok_or_else(|| source_read_arithmetic_overflow("wiener ns lr source-read count"))?;
    let sample = loop_restoration_source_sample(plane, x, y, bounds)?;
    if summary.first_sample.is_none() {
        summary.first_sample = Some(WienerNsLrSourceReadSample {
            plane,
            x: sample.x,
            y: sample.y,
            source: sample.source,
        });
    }
    match sample.source {
        LoopRestorationSource::CurrFrame => {
            summary.curr_frame_source_reads = summary
                .curr_frame_source_reads
                .checked_add(1)
                .ok_or_else(|| {
                    source_read_arithmetic_overflow("wiener ns lr curr-frame source-read count")
                })?;
        }
        LoopRestorationSource::CdefFrame => {
            summary.cdef_frame_source_reads = summary
                .cdef_frame_source_reads
                .checked_add(1)
                .ok_or_else(|| {
                    source_read_arithmetic_overflow("wiener ns lr cdef-frame source-read count")
                })?;
        }
    }
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

fn source_read_coordinate_add(value: isize, delta: isize, context: &'static str) -> Result<isize> {
    value
        .checked_add(delta)
        .ok_or_else(|| source_read_arithmetic_overflow(context))
}

fn scale_chroma_source_coordinate(
    value: isize,
    subsampling: usize,
    context: &'static str,
) -> Result<isize> {
    match subsampling {
        0 => Ok(value),
        1 => value
            .checked_mul(2)
            .ok_or_else(|| source_read_arithmetic_overflow(context)),
        _ => Err(source_read_arithmetic_overflow(context)),
    }
}

fn clip_source_read_coordinate(
    value: isize,
    minimum: usize,
    maximum: usize,
    context: &'static str,
) -> Result<usize> {
    let minimum = isize::try_from(minimum).map_err(|_| source_read_arithmetic_overflow(context))?;
    let maximum = isize::try_from(maximum).map_err(|_| source_read_arithmetic_overflow(context))?;
    if minimum > maximum {
        return Err(source_read_arithmetic_overflow(context));
    }
    usize::try_from(value.clamp(minimum, maximum))
        .map_err(|_| source_read_arithmetic_overflow(context))
}

fn mi_to_luma_start(mi: usize, context: &'static str) -> Result<usize> {
    mi.checked_mul(LR_MI_SIZE)
        .ok_or_else(|| source_read_arithmetic_overflow(context))
}
fn mi_to_luma_end(mi_end: usize, context: &'static str) -> Result<usize> {
    mi_to_luma_start(mi_end, context)?
        .checked_sub(1)
        .ok_or_else(|| source_read_arithmetic_overflow(context))
}
fn usize_to_source_coordinate(value: usize, context: &'static str) -> Result<isize> {
    isize::try_from(value).map_err(|_| source_read_arithmetic_overflow(context))
}
const fn chroma_subsampling(chroma_format: ChromaFormatIdc) -> (u8, u8) {
    match chroma_format {
        ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => (1, 1),
        ChromaFormatIdc::Yuv444 => (0, 0),
        ChromaFormatIdc::Yuv422 => (1, 0),
    }
}
fn wienerns_lr_source_plane(
    plane: usize,
    chroma_format: ChromaFormatIdc,
    offset: ByteOffset,
) -> Result<PlaneId> {
    match plane {
        0 => Ok(PlaneId::Y),
        1 if chroma_format != ChromaFormatIdc::Monochrome => Ok(PlaneId::U),
        2 if chroma_format != ChromaFormatIdc::Monochrome => Ok(PlaneId::V),
        1 | 2 => Err(unsupported_feature_at(
            "unsupported_wienerns_lr_source_chroma_plane",
            offset,
            "minimal runtime reached a Wiener NS LR source-read request for a chroma plane in a monochrome sequence",
            AC0EJ3_LR_SOURCE_READ_MATRIX_ROW,
            AC0EJ3_LR_SOURCE_READ_FEATURE_ID,
            "7.20.2",
        )),
        _ => Err(unsupported_feature_at(
            "unsupported_wienerns_lr_source_plane",
            offset,
            "minimal runtime reached a Wiener NS LR source-read request for an unsupported plane index",
            AC0EJ3_LR_SOURCE_READ_MATRIX_ROW,
            AC0EJ3_LR_SOURCE_READ_FEATURE_ID,
            "7.20.2",
        )),
    }
}
fn has_wienerns_frame_filter_bank(core: &FrameHeaderCore) -> bool {
    core.lr_params.as_ref().is_some_and(|lr| {
        lr.planes
            .iter()
            .any(|plane| plane.frame_filter_bank.is_some())
    })
}

fn source_read_arithmetic_overflow(context: &'static str) -> DecodeError {
    DecodeError::Reconstruction {
        source: splot_recon::ReconError::ArithmeticOverflow { context },
    }
}
