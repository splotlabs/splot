// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Wiener NS loop-restoration runtime frontier helpers.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrPlaneParams};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_core::span::ByteOffset;
use splot_recon::{
    BitDepth, LoopRestorationSource, LoopRestorationSourceBounds, PcWienerClassifyParams,
    PcWienerTxSkipLookup, PlaneId, ReconSample, loop_restoration_source_sample, pc_wiener_classify,
};

use crate::error::{DecodeError, Result};
use crate::tile_payload::{MinimalRuntimePartitionFrontierError, TilePartitionTraversalError};
use crate::{DecodeLimitName, DecodeLimits, DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

use super::{
    AC0EJ3_LR_CLASSIFIED_WIENER_VALUES_FEATURE_ID, AC0EJ3_LR_CLASSIFIED_WIENER_VALUES_MATRIX_ROW,
    AC0EJ3_LR_SOURCE_READ_FEATURE_ID, AC0EJ3_LR_SOURCE_READ_MATRIX_ROW,
    AC0EJ3_LR_UNIT_SELECTIONS_FEATURE_ID, AC0EJ3_LR_UNIT_SELECTIONS_MATRIX_ROW, derive_tile_plan,
    unsupported_at, unsupported_feature_at,
};

/// AV2 §3 `MI_SIZE`: smallest mode-info block size in luma samples.
const LR_MI_SIZE: usize = 4;
/// AV2 §3 `PC_WIENER_LEAD`.
const PC_WIENER_LEAD: isize = 1;
/// AV2 §3 `PC_WIENER_LAG`.
const PC_WIENER_LAG: isize = 4;
/// AV2 §7.20.4 `get_features`: m/up/down/upright/downleft/downright/upleft.
const PC_WIENER_SOURCE_READS_PER_FEATURE: u64 = 7;
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
        reason = "private classified-Wiener value frontier proof state waits for real runtime storage"
    )
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WienerNsLrClassifiedWienerValuesFrontier {
    pub(super) blocks_resolved: usize,
    pub(super) filter_classes_resolved: usize,
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

// AV2 §7.20.3 `Wiener_Ns_Config_Y`, stored as (dy, dx) source offsets.
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

// AV2 §7.20.3 `Wiener_Ns_Config_Uv`, stored as (dy, dx) source offsets.
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
            return Err(wienerns_lr_classified_wiener_values_runtime_error(
                key_envelope.offset,
            ));
        }
        Err(wienerns_lr_source_read_runtime_error(key_envelope.offset))
    } else {
        Err(wienerns_lr_unit_runtime_error(key_envelope.offset))
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
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: block.luma_start_x,
            luma_end_x: block.luma_end_x,
            luma_start_y: block.luma_start_y,
            luma_end_y: block.luma_end_y,
            luma_stripe_start_y: block.luma_stripe_start_y,
            luma_stripe_end_y: block.luma_stripe_end_y,
            subsampling_x: 0,
            subsampling_y: 0,
        };
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
    mut source_sample: FS,
    mut tx_skip: FT,
) -> Result<Option<WienerNsLrClassifiedWienerValuesFrontier>>
where
    T: ReconSample,
    FS: FnMut(isize, isize) -> T,
    FT: FnMut(WienerNsLrTxSkipLookup) -> i32,
{
    if !wienerns_lr_uses_classified_luma(active_source_blocks, planes) {
        return Ok(None);
    }

    let mut summary = WienerNsLrClassifiedWienerValuesFrontier {
        blocks_resolved: 0,
        filter_classes_resolved: 0,
        first_filter_class: None,
    };
    for block in active_source_blocks
        .iter()
        .filter(|block| block.plane == PlaneId::Y.index())
    {
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
        let classification =
            pc_wiener_classify::<T, _, _>(&params, &mut source_sample, |lookup| {
                tx_skip(wienerns_lr_tx_skip_lookup_from_pc(lookup))
            })?;
        record_wienerns_lr_filter_class(&mut summary, block, classification.class)?;
    }
    Ok(Some(summary))
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

    // AV2 §7.20.3 `get_luma_sample`: 4:2:0 filter indexes 0, 1, and 3 read
    // the 2x2 luma footprint; filter index 2 reads only the scaled luma sample.
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

fn source_read_arithmetic_overflow(context: &'static str) -> DecodeError {
    DecodeError::Reconstruction {
        source: splot_recon::ReconError::ArithmeticOverflow { context },
    }
}

pub(super) fn map_wienerns_lr_unit_frontier_error(
    err: MinimalRuntimePartitionFrontierError,
    offset: ByteOffset,
) -> DecodeError {
    match err {
        MinimalRuntimePartitionFrontierError::Limit(source)
        | MinimalRuntimePartitionFrontierError::Traversal(TilePartitionTraversalError::Limit(
            source,
        )) => DecodeError::Limit { source },
        _ => wienerns_lr_unit_runtime_error(offset),
    }
}

fn has_wienerns_frame_filter_bank(core: &FrameHeaderCore) -> bool {
    core.lr_params.as_ref().is_some_and(|lr| {
        lr.planes
            .iter()
            .any(|plane| plane.frame_filter_bank.is_some())
    })
}

fn wienerns_lr_unit_runtime_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_active_wienerns_lr_units",
        offset,
        "minimal runtime consumed the supported AV2 §5.20.10.4/§5.20.10.5 frame-level Wiener NS LR unit syntax, retained per-unit selection state, and found at least one unit selecting RESTORE_WIENER_NONSEP, but does not yet apply active loop-restoration reconstruction before output",
        AC0EJ3_LR_UNIT_SELECTIONS_MATRIX_ROW,
        AC0EJ3_LR_UNIT_SELECTIONS_FEATURE_ID,
        "5.20.10.5",
    )
}

pub(super) fn wienerns_lr_source_read_runtime_error(offset: ByteOffset) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_source_read",
        offset,
        "minimal runtime consumed active AV2 §5.20.10.4/§5.20.10.5 frame-level Wiener NS LR unit syntax, retained per-unit selection state, derived active §7.20.1 loop-restoration source-bound facts, and resolved §7.20.2 source-read state for output, Wiener tap, and chroma luma-source coordinates, but does not yet read source sample values or apply §7.20.3 filtering before output",
        AC0EJ3_LR_SOURCE_READ_MATRIX_ROW,
        AC0EJ3_LR_SOURCE_READ_FEATURE_ID,
        "7.20.2",
    )
}

pub(super) fn wienerns_lr_classified_wiener_values_runtime_error(
    offset: ByteOffset,
) -> DecodeError {
    unsupported_feature_at(
        "unsupported_wienerns_lr_classified_wiener_storage",
        offset,
        "minimal runtime consumed active AV2 frame-level Wiener NS LR unit syntax, retained §7.20.1 source-bound and tile-bound facts, resolved §7.20.4 skip-filter classified-luma source-read and LrTxSkip lookup coordinates, resolved the later §7.20.3 source-read state, and has value-capable FilterClass derivation for caller-supplied source samples and LrTxSkip values, but the live ac0ej3 path reaches loop restoration before decoded 10-bit CurrFrame/CdefFrame storage and before an LrTxSkip grid are retained; no real source sample values or LrTxSkip values are read, and loop-restoration filtering/output is not applied",
        AC0EJ3_LR_CLASSIFIED_WIENER_VALUES_MATRIX_ROW,
        AC0EJ3_LR_CLASSIFIED_WIENER_VALUES_FEATURE_ID,
        "7.20.4",
    )
}
