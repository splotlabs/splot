// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Final loop-filter application for the reconstruction sink.

use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrPlaneParams};
use splot_core::span::ByteOffset;
use splot_parallel::prelude::*;
use splot_recon::{
    LoopRestorationSource, LoopRestorationSourceBounds, PC_WIENER_CLASSIFY_READ_RADIUS,
    PC_WIENER_FILTER_TAP_RADIUS, PC_WIENER_FULL_CLASSES, PcWienerClassifyPaddedSource,
    PcWienerClassifyParams, PcWienerClassifyScratch, PcWienerFilter, PcWienerPaddedSource, PlaneId,
    PlaneRect, ReconError, ReconSample, Result as ReconResult, WIENER_NS_CHROMA_COEFFS,
    WIENER_NS_CHROMA_TAP_RADIUS, WIENER_NS_LUMA_COEFFS, WIENER_NS_LUMA_TAP_RADIUS,
    WienerNsChromaFilter, WienerNsLumaFilter, WienerNsLumaPaddedSource,
    loop_restoration_source_sample, pc_wiener_classify_grid_padded_into,
    pc_wiener_filter_block_padded, pc_wiener_filter_set_index, pc_wiener_subclass,
    wiener_ns_filter_chroma_block, wiener_ns_filter_luma_block_padded,
};

use super::{MI_SIZE, WienerNsLrReconSink};
use crate::Result;
use crate::bitstream::tile_payload::{
    LrUnitRestorationType, WienerNsLrSourceBlock, WienerNsLrUnitFilter,
};
use crate::filters::cdef::CdefSkipGrid;
use crate::filters::wienerns_lr::WienerNsLrTxSkipLookup;
use crate::filters::wienerns_lr::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use crate::support::reusable_scratch::with_reusable_scratch;

thread_local! {
    static PC_WIENER_CLASSIFY_SCRATCH: std::cell::RefCell<PcWienerClassifyScratch> =
        std::cell::RefCell::new(PcWienerClassifyScratch::default());
}

fn luma_lr_filter_error(offset: ByteOffset) -> crate::error::DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(
        offset,
        "unsupported_wienerns_lr_selectable_transform_records_luma_lr_filter",
    )
}

fn luma_lr_frame_coeffs(
    plane: &LrPlaneParams,
    num_classes: usize,
    offset: ByteOffset,
) -> Result<Vec<[i16; WIENER_NS_LUMA_COEFFS]>> {
    if num_classes == 0 {
        return Err(luma_lr_filter_error(offset));
    }
    let Some(bank) = plane.frame_filter_bank.as_ref() else {
        return Err(luma_lr_filter_error(offset));
    };
    if bank.classes.len() != num_classes {
        return Err(luma_lr_filter_error(offset));
    }
    let mut coeffs = Vec::with_capacity(num_classes);
    for class in &bank.classes {
        let coeff: [i16; WIENER_NS_LUMA_COEFFS] = class
            .coeffs
            .as_slice()
            .try_into()
            .map_err(|_| luma_lr_filter_error(offset))?;
        coeffs.push(coeff);
    }
    Ok(coeffs)
}

fn luma_lr_unit_coeffs(
    filters: &[WienerNsLrUnitFilter],
    block: &WienerNsLrSourceBlock,
    offset: ByteOffset,
) -> Result<[i16; WIENER_NS_LUMA_COEFFS]> {
    let filter = lr_unit_filter_for_block(filters, block, offset)?;
    if filter.coeff_count != WIENER_NS_LUMA_COEFFS {
        return Err(luma_lr_filter_error(offset));
    }
    let mut coeffs = [0i16; WIENER_NS_LUMA_COEFFS];
    coeffs.copy_from_slice(&filter.coeffs[..WIENER_NS_LUMA_COEFFS]);
    Ok(coeffs)
}

fn chroma_lr_frame_coeffs(
    plane: &LrPlaneParams,
    offset: ByteOffset,
) -> Result<[i16; WIENER_NS_CHROMA_COEFFS]> {
    let Some(bank) = plane.frame_filter_bank.as_ref() else {
        return Err(luma_lr_filter_error(offset));
    };
    let [class] = bank.classes.as_slice() else {
        return Err(luma_lr_filter_error(offset));
    };
    class
        .coeffs
        .as_slice()
        .try_into()
        .map_err(|_| luma_lr_filter_error(offset))
}

fn chroma_lr_unit_coeffs(
    filters: &[WienerNsLrUnitFilter],
    block: &WienerNsLrSourceBlock,
    offset: ByteOffset,
) -> Result<[i16; WIENER_NS_CHROMA_COEFFS]> {
    let filter = lr_unit_filter_for_block(filters, block, offset)?;
    if filter.coeff_count != WIENER_NS_CHROMA_COEFFS {
        return Err(luma_lr_filter_error(offset));
    }
    Ok(filter.coeffs)
}

fn lr_unit_filter_for_block<'a>(
    filters: &'a [WienerNsLrUnitFilter],
    block: &WienerNsLrSourceBlock,
    offset: ByteOffset,
) -> Result<&'a WienerNsLrUnitFilter> {
    filters
        .iter()
        .find(|filter| {
            filter.plane == block.plane
                && filter.unit_row == block.unit_row
                && filter.unit_col == block.unit_col
        })
        .ok_or_else(|| luma_lr_filter_error(offset))
}

#[cfg(test)]
fn coalesced_lr_source_rows(
    lr_source_blocks: &[WienerNsLrSourceBlock],
    plane_index: usize,
) -> Vec<WienerNsLrSourceBlock> {
    let (blocks, [y_end, u_end]) = coalesced_lr_source_rows_all(lr_source_blocks.to_vec());
    let starts = [0, y_end, u_end];
    let ends = [y_end, u_end, blocks.len()];
    blocks[starts[plane_index]..ends[plane_index]].to_vec()
}

pub(crate) fn coalesced_lr_source_rows_all(
    mut blocks: Vec<WienerNsLrSourceBlock>,
) -> (Vec<WienerNsLrSourceBlock>, [usize; 2]) {
    blocks.retain(|block| block.plane < 3);
    blocks.sort_unstable_by_key(|block| (block.plane, block.y, block.x));
    blocks.dedup_by(|next, run| {
        let Some(width) = run.merged_width_with(next) else {
            return false;
        };
        run.width = width;
        true
    });

    blocks.sort_unstable_by_key(|block| (block.vertical_merge_key(), block.y));
    blocks.dedup_by(|next, rectangle| {
        let Some(height) = rectangle.merged_height_with(next) else {
            return false;
        };
        rectangle.height = height;
        true
    });
    blocks.sort_unstable_by_key(|block| (block.plane, block.y, block.x));

    let y_end = blocks.partition_point(|block| block.plane < 1);
    let u_end = blocks.partition_point(|block| block.plane < 2);
    (blocks, [y_end, u_end])
}

fn clipped_lr_source_block(
    block: &WienerNsLrSourceBlock,
    plane_width: usize,
    plane_height: usize,
    luma_width: usize,
    luma_height: usize,
    offset: ByteOffset,
) -> Result<WienerNsLrSourceBlock> {
    let mut clipped = *block;
    let remaining_width = plane_width
        .checked_sub(block.x)
        .ok_or_else(|| luma_lr_filter_error(offset))?;
    let remaining_height = plane_height
        .checked_sub(block.y)
        .ok_or_else(|| luma_lr_filter_error(offset))?;
    clipped.width = block.width.min(remaining_width);
    clipped.height = block.height.min(remaining_height);
    if clipped.width == 0 || clipped.height == 0 || luma_width == 0 || luma_height == 0 {
        return Err(luma_lr_filter_error(offset));
    }

    let luma_end_x = luma_width - 1;
    let luma_end_y = luma_height - 1;
    clipped.luma_end_x = clipped.luma_end_x.min(luma_end_x);
    clipped.luma_end_y = clipped.luma_end_y.min(luma_end_y);
    clipped.frame_luma_end_y = clipped.frame_luma_end_y.min(luma_end_y);
    clipped.luma_stripe_end_y = clipped.luma_stripe_end_y.min(luma_end_y);
    if clipped.luma_start_x > clipped.luma_end_x
        || clipped.luma_start_y > clipped.luma_end_y
        || clipped.luma_stripe_start_y > clipped.luma_stripe_end_y
    {
        return Err(luma_lr_filter_error(offset));
    }
    Ok(clipped)
}

struct LrSourceWindow<T> {
    samples: Vec<T>,
    stride: usize,
    origin_x: isize,
    origin_y: isize,
}

impl<T: ReconSample> LrSourceWindow<T> {
    #[allow(clippy::too_many_arguments)]
    fn materialize(
        plane: PlaneId,
        curr_plane: &[u16],
        cdef_plane: &[u16],
        plane_width: usize,
        plane_height: usize,
        bounds: &LoopRestorationSourceBounds,
        block_x: isize,
        block_y: isize,
        width: usize,
        height: usize,
        radius: usize,
    ) -> ReconResult<Self> {
        let stride = width
            .checked_add(radius.checked_mul(2).ok_or(OVERFLOW_WINDOW)?)
            .ok_or(OVERFLOW_WINDOW)?;
        let rows = height
            .checked_add(radius.checked_mul(2).ok_or(OVERFLOW_WINDOW)?)
            .ok_or(OVERFLOW_WINDOW)?;
        let radius = isize::try_from(radius).map_err(|_| OVERFLOW_WINDOW)?;
        let mut samples = Vec::with_capacity(stride.checked_mul(rows).ok_or(OVERFLOW_WINDOW)?);
        for row_index in 0..rows {
            let y = block_y
                .checked_sub(radius)
                .and_then(|top| top.checked_add(isize::try_from(row_index).ok()?))
                .ok_or(OVERFLOW_WINDOW)?;
            let left = loop_restoration_source_sample(plane, isize::MIN, y, bounds)?;
            let right = loop_restoration_source_sample(plane, isize::MAX, y, bounds)?;
            if right.x >= plane_width || left.y >= plane_height {
                return Err(ReconError::PcWienerInvalidBounds {
                    field: "LR source frame bounds",
                });
            }
            let source = match left.source {
                LoopRestorationSource::CurrFrame => curr_plane,
                LoopRestorationSource::CdefFrame => cdef_plane,
            };
            let row_start =
                left.y
                    .checked_mul(plane_width)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "LR source sample index",
                    })?;
            let row_end =
                row_start
                    .checked_add(plane_width)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "LR source sample index",
                    })?;
            let source_row =
                source
                    .get(row_start..row_end)
                    .ok_or(ReconError::BufferLengthMismatch {
                        expected: row_end,
                        actual: source.len(),
                    })?;
            let min_x = isize::try_from(left.x).map_err(|_| OVERFLOW_WINDOW)?;
            let max_x = isize::try_from(right.x).map_err(|_| OVERFLOW_WINDOW)?;
            let x0 = block_x.checked_sub(radius).ok_or(OVERFLOW_WINDOW)?;
            let stride_i = isize::try_from(stride).map_err(|_| OVERFLOW_WINDOW)?;
            let pre = min_x
                .checked_sub(x0)
                .ok_or(OVERFLOW_WINDOW)?
                .clamp(0, stride_i) as usize;
            let post = x0
                .checked_add(stride_i)
                .and_then(|end| end.checked_sub(1))
                .and_then(|last| last.checked_sub(max_x))
                .ok_or(OVERFLOW_WINDOW)?
                .clamp(
                    0,
                    stride_i.checked_sub(pre as isize).ok_or(OVERFLOW_WINDOW)?,
                ) as usize;
            let mid = stride - pre - post;
            let left_value = T::try_from_u16(*source_row.get(left.x).ok_or(
                ReconError::BufferLengthMismatch {
                    expected: left.x.saturating_add(1),
                    actual: source_row.len(),
                },
            )?)?;
            samples.resize(samples.len().saturating_add(pre), left_value);
            if mid > 0 {
                let mid_start = (x0 + pre as isize) as usize;
                let mid_slice = source_row.get(mid_start..mid_start + mid).ok_or(
                    ReconError::BufferLengthMismatch {
                        expected: mid_start.saturating_add(mid),
                        actual: source_row.len(),
                    },
                )?;
                for &value in mid_slice {
                    samples.push(T::try_from_u16(value)?);
                }
            }
            let right_value = T::try_from_u16(*source_row.get(right.x).ok_or(
                ReconError::BufferLengthMismatch {
                    expected: right.x.saturating_add(1),
                    actual: source_row.len(),
                },
            )?)?;
            samples.resize(samples.len().saturating_add(post), right_value);
        }
        Ok(Self {
            samples,
            stride,
            origin_x: block_x.checked_sub(radius).ok_or(OVERFLOW_WINDOW)?,
            origin_y: block_y.checked_sub(radius).ok_or(OVERFLOW_WINDOW)?,
        })
    }

    fn tail_from(&self, x: isize, y: isize) -> Option<(&[T], usize)> {
        let col = usize::try_from(x.checked_sub(self.origin_x)?).ok()?;
        let row = usize::try_from(y.checked_sub(self.origin_y)?).ok()?;
        if col >= self.stride {
            return None;
        }
        let start = row.checked_mul(self.stride)?.checked_add(col)?;
        self.samples.get(start..).map(|tail| (tail, self.stride))
    }

    fn get_abs(&self, x: isize, y: isize) -> T {
        let col = x.saturating_sub(self.origin_x);
        let row = y.saturating_sub(self.origin_y);
        if col < 0 || row < 0 || col as usize >= self.stride {
            return T::default();
        }
        self.samples
            .get(
                (row as usize)
                    .saturating_mul(self.stride)
                    .saturating_add(col as usize),
            )
            .copied()
            .unwrap_or_default()
    }
}

const OVERFLOW_WINDOW: ReconError = ReconError::ArithmeticOverflow {
    context: "LR source window geometry",
};

fn usize_to_isize_recon(value: usize, context: &'static str) -> ReconResult<isize> {
    isize::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })
}

fn mi_to_luma_start_recon(mi: usize, context: &'static str) -> ReconResult<usize> {
    mi.checked_mul(MI_SIZE)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn mi_to_luma_end_recon(mi_end: usize, context: &'static str) -> ReconResult<usize> {
    mi_to_luma_start_recon(mi_end, context)?
        .checked_sub(1)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

impl<T: ReconSample> WienerNsLrReconSink<T> {
    pub(crate) fn cdef_skip_grid(
        &self,
        core: &FrameHeaderCore,
        mi_rows: usize,
        mi_cols: usize,
        offset: ByteOffset,
    ) -> Result<Option<CdefSkipGrid>> {
        let Some(cdef) = core.cdef_params.as_ref() else {
            return Ok(None);
        };
        if cdef.cdef_on_skip_txfm_frame_enable != Some(false) {
            return Ok(None);
        }
        let Some(tx_skip_grid) = self.tx_skip_grid.as_ref() else {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_skip_grid",
            ));
        };
        if tx_skip_grid.rows() < mi_rows || tx_skip_grid.cols() < mi_cols {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_skip_grid",
            ));
        }
        let count = mi_rows.checked_mul(mi_cols).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_skip_grid",
            )
        })?;
        let mut values = Vec::with_capacity(count);
        for row in 0..mi_rows {
            for col in 0..mi_cols {
                let skip = tx_skip_grid
                    .lookup(WienerNsLrTxSkipLookup {
                        x: col.saturating_mul(MI_SIZE),
                        y: row.saturating_mul(MI_SIZE),
                        row,
                        col,
                    })
                    .map_err(|_| {
                        wienerns_lr_selectable_transform_record_error_reason(
                            offset,
                            "unsupported_wienerns_lr_selectable_transform_records_cdef_skip_grid",
                        )
                    })?;
                if !(0..=1).contains(&skip) {
                    return Err(wienerns_lr_selectable_transform_record_error_reason(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_cdef_skip_grid",
                    ));
                }
                values.push(skip != 0);
            }
        }
        CdefSkipGrid::new(mi_rows, mi_cols, values)
            .map(Some)
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_cdef_skip_grid",
                )
            })
    }

    pub(crate) fn apply_luma_lr_runs(
        &mut self,
        core: &FrameHeaderCore,
        offset: ByteOffset,
        y_blocks: &[WienerNsLrSourceBlock],
        lr_unit_filters: &[WienerNsLrUnitFilter],
        curr_luma: &[u16],
        cdef_luma: &[u16],
    ) -> Result<()> {
        let Some(lr_params) = core.lr_params.as_ref() else {
            return Ok(());
        };
        let Some(plane) = lr_params.planes.first() else {
            return Ok(());
        };
        if !matches!(
            plane.restoration_type,
            FrameRestorationType::WienerNonsep
                | FrameRestorationType::PcWiener
                | FrameRestorationType::Switchable
        ) || y_blocks.is_empty()
        {
            return Ok(());
        }

        let qindex = core
            .quantization_params
            .as_ref()
            .ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_missing_quantization",
                )
            })?
            .base_q_idx;
        let filter_set_index = pc_wiener_filter_set_index(qindex);
        let frame_coeffs = if matches!(
            plane.restoration_type,
            FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
        ) && plane.frame_filters_on
        {
            let num_classes = usize::from(plane.num_filter_classes.unwrap_or(1));
            Some((
                luma_lr_frame_coeffs(plane, num_classes, offset)?,
                num_classes,
            ))
        } else {
            None
        };
        let compute = |block: &WienerNsLrSourceBlock| match block.restoration_type {
            LrUnitRestorationType::PcWiener => self.compute_pc_wiener_block(
                offset,
                block,
                curr_luma,
                cdef_luma,
                qindex,
                filter_set_index,
            ),
            LrUnitRestorationType::WienerNonsep => {
                if let Some((coeffs, num_classes)) = frame_coeffs.as_ref() {
                    self.compute_luma_lr_block(
                        offset,
                        block,
                        curr_luma,
                        cdef_luma,
                        qindex,
                        *num_classes,
                        filter_set_index,
                        coeffs,
                    )
                } else {
                    let coeffs = [luma_lr_unit_coeffs(lr_unit_filters, block, offset)?];
                    self.compute_luma_lr_block(
                        offset, block, curr_luma, cdef_luma, qindex, 1, 0, &coeffs,
                    )
                }
            }
            LrUnitRestorationType::None => Err(luma_lr_filter_error(offset)),
        };
        let filtered: Vec<(WienerNsLrSourceBlock, Vec<T>)> = if splot_parallel::on_worker_pool() {
            let timer = crate::timing::start();
            let tally = crate::timing::WorkerTally::new();
            let outputs = y_blocks
                .par_iter()
                .map(|block| {
                    tally.note_worker();
                    compute(block)
                })
                .collect::<Result<_>>()?;
            crate::timing::report_detail(
                "lr_luma_blocks",
                timer,
                &format!(
                    "units={} threads={} workers_used={}",
                    y_blocks.len(),
                    splot_parallel::current_pool_width(),
                    tally.workers_used()
                ),
            );
            outputs
        } else {
            y_blocks.iter().map(&compute).collect::<Result<_>>()?
        };
        self.publish_lr_outputs(PlaneId::Y, filtered, offset)
    }

    fn compute_pc_wiener_block(
        &self,
        offset: ByteOffset,
        block: &WienerNsLrSourceBlock,
        curr_luma: &[u16],
        cdef_luma: &[u16],
        qindex: u32,
        filter_set_index: usize,
    ) -> Result<(WienerNsLrSourceBlock, Vec<T>)> {
        let block = clipped_lr_source_block(
            block,
            self.luma_width,
            self.luma_height,
            self.luma_width,
            self.luma_height,
            offset,
        )?;
        let sample_count = block
            .width
            .checked_mul(block.height)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let bounds = crate::filters::wienerns_lr::wienerns_lr_source_block_bounds(&block, 0, 0);
        let block_x = usize_to_isize_recon(block.x, "PC-Wiener block x")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let block_y = usize_to_isize_recon(block.y, "PC-Wiener block y")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let window = LrSourceWindow::<T>::materialize(
            PlaneId::Y,
            curr_luma,
            cdef_luma,
            self.luma_width,
            self.luma_height,
            &bounds,
            block_x,
            block_y,
            block.width,
            block.height,
            PC_WIENER_CLASSIFY_READ_RADIUS.max(PC_WIENER_FILTER_TAP_RADIUS),
        )
        .map_err(|_| luma_lr_filter_error(offset))?;
        let subclasses = self.luma_lr_subclasses(
            offset,
            &block,
            &window,
            qindex,
            PC_WIENER_FULL_CLASSES,
            filter_set_index,
            sample_count,
        )?;
        let mut output = vec![T::default(); sample_count];
        let params = PcWienerFilter {
            width: block.width,
            height: block.height,
            output_stride: block.width,
            bit_depth: self.bit_depth,
            filter_set_index,
            subclasses: &subclasses,
        };
        let tap_radius = isize::try_from(PC_WIENER_FILTER_TAP_RADIUS)
            .map_err(|_| luma_lr_filter_error(offset))?;
        let (padded, padded_stride) = window
            .tail_from(
                block_x.saturating_sub(tap_radius),
                block_y.saturating_sub(tap_radius),
            )
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let padded_source =
            PcWienerPaddedSource::new(padded, padded_stride, block.width, block.height)
                .map_err(|_| luma_lr_filter_error(offset))?;
        pc_wiener_filter_block_padded(&mut output, &params, &padded_source)
            .map_err(|_| luma_lr_filter_error(offset))?;
        self.preserve_lossless_lr_samples(PlaneId::Y, &block, curr_luma, &mut output, offset)?;
        Ok((block, output))
    }

    fn publish_lr_outputs(
        &mut self,
        plane_id: PlaneId,
        filtered: Vec<(WienerNsLrSourceBlock, Vec<T>)>,
        offset: ByteOffset,
    ) -> Result<()> {
        let timer = crate::timing::start();
        let mut runs = Vec::with_capacity(filtered.len());
        for (block, output) in filtered {
            let rect = PlaneRect::new(block.x, block.y, block.width, block.height)
                .map_err(|_| luma_lr_filter_error(offset))?;
            runs.push((rect, output, block.width));
        }
        let result = self.publish_lr_runs(plane_id, &runs, offset);
        crate::timing::report_detail("lr_publish", timer, &format!("plane={}", plane_id.index()));
        result
    }

    fn publish_lr_runs(
        &mut self,
        plane_id: PlaneId,
        runs: &[crate::tile::plane_bands::RectRun<T>],
        offset: ByteOffset,
    ) -> Result<()> {
        if splot_parallel::on_multiworker_pool() {
            let mut frame = self.workspace.as_frame_mut();
            let view = frame
                .plane_mut(plane_id)
                .ok_or_else(|| luma_lr_filter_error(offset))?;
            let stride = view.stride_samples();
            if crate::tile::plane_bands::publish_rect_runs_parallel(
                view.samples_mut(),
                stride,
                runs,
            )
            .is_some()
            {
                return Ok(());
            }
        }
        for (rect, output, row_stride) in runs {
            self.workspace
                .write_rect(plane_id, *rect, output, *row_stride)
                .map_err(|_| luma_lr_filter_error(offset))?;
        }
        Ok(())
    }

    fn plane_dimensions(&self, plane_id: PlaneId) -> (usize, usize) {
        match plane_id {
            PlaneId::Y => (self.luma_width, self.luma_height),
            PlaneId::U | PlaneId::V => (self.luma_width.div_ceil(2), self.luma_height.div_ceil(2)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_chroma_lr_runs(
        &mut self,
        core: &FrameHeaderCore,
        offset: ByteOffset,
        plane_id: PlaneId,
        plane_blocks: &[WienerNsLrSourceBlock],
        lr_unit_filters: &[WienerNsLrUnitFilter],
        curr_chroma: &[u16],
        cdef_chroma: &[u16],
        curr_luma: &[u16],
        cdef_luma: &[u16],
    ) -> Result<()> {
        let Some(lr_params) = core.lr_params.as_ref() else {
            return Ok(());
        };
        let Some(plane) = lr_params.planes.get(plane_id.index()) else {
            return Ok(());
        };
        let plane_index = plane_id.index();
        if plane.restoration_type != FrameRestorationType::WienerNonsep || plane_blocks.is_empty() {
            return Ok(());
        }
        if plane_blocks
            .iter()
            .any(|block| block.restoration_type != LrUnitRestorationType::WienerNonsep)
        {
            return Err(luma_lr_filter_error(offset));
        }
        let frame_coeffs = if plane.frame_filters_on {
            Some(chroma_lr_frame_coeffs(plane, offset)?)
        } else {
            None
        };

        let (plane_width, plane_height) = self.plane_dimensions(plane_id);
        let expected_samples = plane_width
            .checked_mul(plane_height)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        if curr_chroma.len() != expected_samples || cdef_chroma.len() != expected_samples {
            return Err(luma_lr_filter_error(offset));
        }
        let compute = |block: &WienerNsLrSourceBlock| {
            self.compute_chroma_lr_block(
                offset,
                plane_id,
                block,
                lr_unit_filters,
                frame_coeffs.as_ref(),
                curr_chroma,
                cdef_chroma,
                curr_luma,
                cdef_luma,
            )
        };
        let filtered: Vec<(WienerNsLrSourceBlock, Vec<T>)> = if splot_parallel::on_worker_pool() {
            let timer = crate::timing::start();
            let tally = crate::timing::WorkerTally::new();
            let outputs = plane_blocks
                .par_iter()
                .map(|block| {
                    tally.note_worker();
                    compute(block)
                })
                .collect::<Result<_>>()?;
            crate::timing::report_detail(
                "lr_chroma_blocks",
                timer,
                &format!(
                    "plane={} units={} threads={} workers_used={}",
                    plane_index,
                    plane_blocks.len(),
                    splot_parallel::current_pool_width(),
                    tally.workers_used()
                ),
            );
            outputs
        } else {
            plane_blocks.iter().map(&compute).collect::<Result<_>>()?
        };
        self.publish_lr_outputs(plane_id, filtered, offset)
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_chroma_lr_block(
        &self,
        offset: ByteOffset,
        plane_id: PlaneId,
        block: &WienerNsLrSourceBlock,
        lr_unit_filters: &[WienerNsLrUnitFilter],
        frame_coeffs: Option<&[i16; WIENER_NS_CHROMA_COEFFS]>,
        curr_chroma: &[u16],
        cdef_chroma: &[u16],
        curr_luma: &[u16],
        cdef_luma: &[u16],
    ) -> Result<(WienerNsLrSourceBlock, Vec<T>)> {
        let (plane_width, plane_height) = self.plane_dimensions(plane_id);
        let block = clipped_lr_source_block(
            block,
            plane_width,
            plane_height,
            self.luma_width,
            self.luma_height,
            offset,
        )?;
        let coeffs = match frame_coeffs {
            Some(coeffs) => *coeffs,
            None => chroma_lr_unit_coeffs(lr_unit_filters, &block, offset)?,
        };
        let sample_count = block
            .width
            .checked_mul(block.height)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let mut output = vec![T::default(); sample_count];
        let params = WienerNsChromaFilter {
            x: block.x,
            y: block.y,
            width: block.width,
            height: block.height,
            output_stride: block.width,
            bit_depth: self.bit_depth,
            coeffs: &coeffs,
            subsampling_x: 1,
            subsampling_y: 1,
            luma_start_x: block.luma_start_x,
            luma_end_x: block.luma_end_x,
            mi_rows: self.luma_height.div_ceil(MI_SIZE),
            cfl_ds_filter_index: self.cfl_ds_filter_index,
        };
        let chroma_bounds =
            crate::filters::wienerns_lr::wienerns_lr_source_block_bounds(&block, 1, 1);
        let luma_bounds =
            crate::filters::wienerns_lr::wienerns_lr_source_block_bounds(&block, 1, 1);
        let block_x = usize_to_isize_recon(block.x, "chroma LR block x")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let block_y = usize_to_isize_recon(block.y, "chroma LR block y")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let chroma_window = LrSourceWindow::<T>::materialize(
            plane_id,
            curr_chroma,
            cdef_chroma,
            plane_width,
            plane_height,
            &chroma_bounds,
            block_x,
            block_y,
            block.width,
            block.height,
            WIENER_NS_CHROMA_TAP_RADIUS,
        )
        .map_err(|_| luma_lr_filter_error(offset))?;
        let luma_window = LrSourceWindow::<T>::materialize(
            PlaneId::Y,
            curr_luma,
            cdef_luma,
            self.luma_width,
            self.luma_height,
            &luma_bounds,
            block_x.saturating_mul(2),
            block_y.saturating_mul(2),
            block.width.saturating_mul(2),
            block.height.saturating_mul(2),
            WIENER_NS_CHROMA_TAP_RADIUS * 2,
        )
        .map_err(|_| luma_lr_filter_error(offset))?;
        wiener_ns_filter_chroma_block(
            &mut output,
            &params,
            |x, y| chroma_window.get_abs(x, y),
            |x, y| luma_window.get_abs(x, y),
        )
        .map_err(|_| luma_lr_filter_error(offset))?;
        self.preserve_lossless_lr_samples(plane_id, &block, curr_chroma, &mut output, offset)?;
        Ok((block, output))
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_luma_lr_block(
        &self,
        offset: ByteOffset,
        block: &WienerNsLrSourceBlock,
        curr_luma: &[u16],
        cdef_luma: &[u16],
        qindex: u32,
        num_classes: usize,
        filter_set_index: usize,
        coeffs: &[[i16; WIENER_NS_LUMA_COEFFS]],
    ) -> Result<(WienerNsLrSourceBlock, Vec<T>)> {
        let block = clipped_lr_source_block(
            block,
            self.luma_width,
            self.luma_height,
            self.luma_width,
            self.luma_height,
            offset,
        )?;
        let sample_count = block
            .width
            .checked_mul(block.height)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let bounds = crate::filters::wienerns_lr::wienerns_lr_source_block_bounds(&block, 0, 0);
        let block_x = usize_to_isize_recon(block.x, "luma LR block x")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let block_y = usize_to_isize_recon(block.y, "luma LR block y")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let window = LrSourceWindow::<T>::materialize(
            PlaneId::Y,
            curr_luma,
            cdef_luma,
            self.luma_width,
            self.luma_height,
            &bounds,
            block_x,
            block_y,
            block.width,
            block.height,
            WIENER_NS_LUMA_TAP_RADIUS.max(PC_WIENER_CLASSIFY_READ_RADIUS),
        )
        .map_err(|_| luma_lr_filter_error(offset))?;
        let subclasses = if num_classes > 1 {
            Some(self.luma_lr_subclasses(
                offset,
                &block,
                &window,
                qindex,
                num_classes,
                filter_set_index,
                sample_count,
            )?)
        } else {
            None
        };
        let mut output = vec![T::default(); sample_count];
        let params = WienerNsLumaFilter {
            width: block.width,
            height: block.height,
            output_stride: block.width,
            bit_depth: self.bit_depth,
            coeffs_by_class: coeffs,
            subclasses: subclasses.as_deref(),
        };
        let tap_radius =
            isize::try_from(WIENER_NS_LUMA_TAP_RADIUS).map_err(|_| luma_lr_filter_error(offset))?;
        let (padded, padded_stride) = window
            .tail_from(
                block_x.saturating_sub(tap_radius),
                block_y.saturating_sub(tap_radius),
            )
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let padded_source =
            WienerNsLumaPaddedSource::new(padded, padded_stride, block.width, block.height)
                .map_err(|_| luma_lr_filter_error(offset))?;
        wiener_ns_filter_luma_block_padded(&mut output, &params, &padded_source)
            .map_err(|_| luma_lr_filter_error(offset))?;
        self.preserve_lossless_lr_samples(PlaneId::Y, &block, curr_luma, &mut output, offset)?;
        Ok((block, output))
    }

    fn preserve_lossless_lr_samples(
        &self,
        plane_id: PlaneId,
        block: &WienerNsLrSourceBlock,
        curr_plane: &[u16],
        output: &mut [T],
        offset: ByteOffset,
    ) -> Result<()> {
        let Some(lossless_grid) = self.lossless_grid.as_ref() else {
            return Ok(());
        };
        let (plane_width, plane_height) = self.plane_dimensions(plane_id);
        let expected_samples = plane_width
            .checked_mul(plane_height)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        if curr_plane.len() != expected_samples {
            return Err(luma_lr_filter_error(offset));
        }
        let (sub_x, sub_y) = self.plane_subsampling(plane_id);
        for row in 0..block.height {
            for col in 0..block.width {
                let x = block
                    .x
                    .checked_add(col)
                    .ok_or_else(|| luma_lr_filter_error(offset))?;
                let y = block
                    .y
                    .checked_add(row)
                    .ok_or_else(|| luma_lr_filter_error(offset))?;
                if !lossless_grid.plane_sample_lossless(plane_id, x, y, sub_x, sub_y) {
                    continue;
                }
                let source_index = y
                    .checked_mul(plane_width)
                    .and_then(|start| start.checked_add(x))
                    .ok_or_else(|| luma_lr_filter_error(offset))?;
                let output_index = row
                    .checked_mul(block.width)
                    .and_then(|start| start.checked_add(col))
                    .ok_or_else(|| luma_lr_filter_error(offset))?;
                let sample = *curr_plane
                    .get(source_index)
                    .ok_or_else(|| luma_lr_filter_error(offset))?;
                let output_sample =
                    T::try_from_u16(sample).map_err(|_| luma_lr_filter_error(offset))?;
                let Some(slot) = output.get_mut(output_index) else {
                    return Err(luma_lr_filter_error(offset));
                };
                *slot = output_sample;
            }
        }
        Ok(())
    }

    fn plane_subsampling(&self, plane_id: PlaneId) -> (usize, usize) {
        match plane_id {
            PlaneId::Y => (0, 0),
            PlaneId::U | PlaneId::V => {
                let format = self.workspace.info().pixel_format();
                (
                    usize::from(format.subsampling_x()),
                    usize::from(format.subsampling_y()),
                )
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn luma_lr_subclasses(
        &self,
        offset: ByteOffset,
        block: &WienerNsLrSourceBlock,
        window: &LrSourceWindow<T>,
        qindex: u32,
        num_classes: usize,
        filter_set_index: usize,
        sample_count: usize,
    ) -> Result<Vec<usize>> {
        if sample_count
            != block
                .width
                .checked_mul(block.height)
                .ok_or_else(|| luma_lr_filter_error(offset))?
        {
            return Err(luma_lr_filter_error(offset));
        }
        let cell_cols = block.width.div_ceil(MI_SIZE).max(1);
        let cell_rows = block.height.div_ceil(MI_SIZE).max(1);
        let Some(tx_skip_grid) = self.tx_skip_grid.as_ref() else {
            return Err(luma_lr_filter_error(offset));
        };
        let tile_start_y = mi_to_luma_start_recon(block.tile_mi_row_start, "luma LR tile start y")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let tile_end_y = mi_to_luma_end_recon(block.tile_mi_row_end, "luma LR tile end y")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let mut cell_subclasses = vec![0; cell_cols * cell_rows];
        let mut group_start = 0;
        while group_start < cell_cols {
            let class_x = block
                .x
                .checked_add(group_start.saturating_mul(MI_SIZE))
                .ok_or_else(|| luma_lr_filter_error(offset))?;
            let block_start_x = (class_x >> 6) << 6;
            let mut group_end = group_start + 1;
            while group_end < cell_cols {
                let next_x = block
                    .x
                    .checked_add(group_end.saturating_mul(MI_SIZE))
                    .ok_or_else(|| luma_lr_filter_error(offset))?;
                if ((next_x >> 6) << 6) != block_start_x {
                    break;
                }
                group_end += 1;
            }
            let block_end_x = super::super::pc_wiener_block_end_x(block, block_start_x)
                .map_err(|_| luma_lr_filter_error(offset))?;
            let params = PcWienerClassifyParams {
                x: usize_to_isize_recon(class_x, "luma LR PC-Wiener x")
                    .map_err(|_| luma_lr_filter_error(offset))?,
                y: usize_to_isize_recon(block.y, "luma LR PC-Wiener y")
                    .map_err(|_| luma_lr_filter_error(offset))?,
                bit_depth: self.bit_depth,
                base_q_idx: qindex,
                block_start_x,
                block_end_x,
                luma_stripe_start_y: block.luma_stripe_start_y,
                luma_stripe_end_y: block.luma_stripe_end_y,
                tile_start_y,
                tile_end_y,
            };
            let group_cols = group_end - group_start;
            let padded_source = PcWienerClassifyPaddedSource::new(
                &window.samples,
                window.stride,
                window.origin_x,
                window.origin_y,
            );
            with_reusable_scratch(&PC_WIENER_CLASSIFY_SCRATCH, |scratch| {
                let classifications = pc_wiener_classify_grid_padded_into::<T, _>(
                    &params,
                    group_cols,
                    cell_rows,
                    &padded_source,
                    |lookup| {
                        tx_skip_grid.lookup(
                            crate::filters::wienerns_lr::wienerns_lr_tx_skip_lookup_from_pc(lookup),
                        )
                    },
                    scratch,
                )
                .map_err(|_| luma_lr_filter_error(offset))?;
                for (index, classification) in classifications.iter().enumerate() {
                    let cell_row = index / group_cols;
                    let cell_col = group_start + index % group_cols;
                    let cell_index = cell_row
                        .checked_mul(cell_cols)
                        .and_then(|start| start.checked_add(cell_col))
                        .ok_or_else(|| luma_lr_filter_error(offset))?;
                    let subclass =
                        pc_wiener_subclass(num_classes, filter_set_index, classification.class)
                            .map_err(|_| luma_lr_filter_error(offset))?;
                    let Some(slot) = cell_subclasses.get_mut(cell_index) else {
                        return Err(luma_lr_filter_error(offset));
                    };
                    *slot = subclass;
                }
                Ok(())
            })?;
            group_start = group_end;
        }

        let mut subclasses = Vec::with_capacity(sample_count);
        for row in 0..block.height {
            let cell_row = row / MI_SIZE;
            for cell_col in 0..cell_cols {
                let cell_index = cell_row
                    .checked_mul(cell_cols)
                    .and_then(|start| start.checked_add(cell_col))
                    .ok_or_else(|| luma_lr_filter_error(offset))?;
                let Some(&subclass) = cell_subclasses.get(cell_index) else {
                    return Err(luma_lr_filter_error(offset));
                };
                let cell_start = cell_col.saturating_mul(MI_SIZE);
                let cell_width = MI_SIZE.min(block.width.saturating_sub(cell_start));
                subclasses.extend(core::iter::repeat_n(subclass, cell_width));
            }
        }
        Ok(subclasses)
    }
}

#[cfg(test)]
#[path = "final_filters_tests.rs"]
mod tests;
