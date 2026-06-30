// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Final loop-filter application for the ac0ej3 reconstruction sink.

use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrPlaneParams};
use splot_core::span::ByteOffset;
use splot_recon::{
    LoopRestorationSource, LoopRestorationSourceBounds, PcWienerClassifyParams, PlaneId, PlaneRect,
    ReconError, ReconSample, Result as ReconResult, WIENER_NS_CHROMA_COEFFS, WIENER_NS_LUMA_COEFFS,
    WienerNsChromaFilter, WienerNsLumaFilter, loop_restoration_source_sample, pc_wiener_classify,
    pc_wiener_filter_set_index, pc_wiener_subclass, wiener_ns_filter_chroma_block,
    wiener_ns_filter_luma_block,
};

use super::{MI_SIZE, WienerNsLrReconSink};
use crate::Result;
use crate::runtime_minimal::cdef::CdefSkipGrid;
use crate::runtime_minimal::wienerns_lr::WienerNsLrTxSkipLookup;
use crate::runtime_minimal::wienerns_lr::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use crate::tile_payload::{WienerNsLrSourceBlock, WienerNsLrUnitFilter};

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

#[allow(clippy::too_many_arguments)]
fn lr_plane_source_sample<T: ReconSample>(
    plane: PlaneId,
    curr_plane: &[u16],
    cdef_plane: &[u16],
    plane_width: usize,
    plane_height: usize,
    bounds: &LoopRestorationSourceBounds,
    x: isize,
    y: isize,
) -> ReconResult<T> {
    let sample = loop_restoration_source_sample(plane, x, y, bounds)?;
    if sample.x >= plane_width || sample.y >= plane_height {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "LR source frame bounds",
        });
    }
    let index = sample
        .y
        .checked_mul(plane_width)
        .and_then(|row| row.checked_add(sample.x))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "LR source sample index",
        })?;
    let source = match sample.source {
        LoopRestorationSource::CurrFrame => curr_plane,
        LoopRestorationSource::CdefFrame => cdef_plane,
    };
    let Some(&value) = source.get(index) else {
        return Err(ReconError::BufferLengthMismatch {
            expected: index.saturating_add(1),
            actual: source.len(),
        });
    };
    T::try_from_u16(value)
}

#[allow(clippy::too_many_arguments)]
fn copy_lr_block<T: ReconSample>(
    lr_plane: &mut [T],
    plane_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    output: &[T],
    offset: ByteOffset,
) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(luma_lr_filter_error(offset));
    }
    let expected = width
        .checked_mul(height)
        .ok_or_else(|| luma_lr_filter_error(offset))?;
    if output.len() != expected {
        return Err(luma_lr_filter_error(offset));
    }
    let end_x = x
        .checked_add(width)
        .ok_or_else(|| luma_lr_filter_error(offset))?;
    if end_x > plane_width {
        return Err(luma_lr_filter_error(offset));
    }
    for row in 0..height {
        let dst_start = y
            .checked_add(row)
            .and_then(|target_y| target_y.checked_mul(plane_width))
            .and_then(|row_start| row_start.checked_add(x))
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let dst_end = dst_start
            .checked_add(width)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let src_start = row
            .checked_mul(width)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let src_end = src_start
            .checked_add(width)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let Some(dst) = lr_plane.get_mut(dst_start..dst_end) else {
            return Err(luma_lr_filter_error(offset));
        };
        let Some(src) = output.get(src_start..src_end) else {
            return Err(luma_lr_filter_error(offset));
        };
        // splot-copy-ok: commit a validated fail-atomic LR block into luma scratch.
        dst.copy_from_slice(src);
    }
    Ok(())
}

fn usize_to_isize_recon(value: usize, context: &'static str) -> ReconResult<isize> {
    isize::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })
}

fn source_coordinate_add_recon(
    value: isize,
    delta: isize,
    context: &'static str,
) -> ReconResult<isize> {
    value
        .checked_add(delta)
        .ok_or(ReconError::ArithmeticOverflow { context })
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
    pub(super) fn cdef_skip_grid(
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

    pub(super) fn apply_luma_lr(
        &mut self,
        core: &FrameHeaderCore,
        offset: ByteOffset,
        lr_source_blocks: &[WienerNsLrSourceBlock],
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
        if plane.restoration_type != FrameRestorationType::WienerNonsep
            || !lr_source_blocks
                .iter()
                .any(|block| block.plane == PlaneId::Y.index())
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
        let frame_coeffs = if plane.frame_filters_on {
            let num_classes = usize::from(plane.num_filter_classes.unwrap_or(1));
            Some((
                luma_lr_frame_coeffs(plane, num_classes, offset)?,
                num_classes,
                pc_wiener_filter_set_index(qindex),
            ))
        } else {
            None
        };
        let mut lr_luma = Vec::with_capacity(cdef_luma.len());
        for &sample in cdef_luma {
            lr_luma.push(T::try_from_u16(sample).map_err(|_| luma_lr_filter_error(offset))?);
        }

        for block in lr_source_blocks
            .iter()
            .filter(|block| block.plane == PlaneId::Y.index())
        {
            if let Some((coeffs, num_classes, filter_set_index)) = frame_coeffs.as_ref() {
                self.apply_luma_lr_block(
                    offset,
                    block,
                    curr_luma,
                    cdef_luma,
                    qindex,
                    *num_classes,
                    *filter_set_index,
                    coeffs,
                    &mut lr_luma,
                )?;
            } else {
                let coeffs = [luma_lr_unit_coeffs(lr_unit_filters, block, offset)?];
                self.apply_luma_lr_block(
                    offset,
                    block,
                    curr_luma,
                    cdef_luma,
                    qindex,
                    1,
                    0,
                    &coeffs,
                    &mut lr_luma,
                )?;
            }
        }

        let rect = PlaneRect::new(0, 0, self.luma_width, self.luma_height)
            .map_err(|_| luma_lr_filter_error(offset))?;
        self.workspace
            .write_rect(PlaneId::Y, rect, &lr_luma, self.luma_width)
            .map_err(|_| luma_lr_filter_error(offset))
    }

    fn plane_dimensions(&self, plane_id: PlaneId) -> (usize, usize) {
        match plane_id {
            PlaneId::Y => (self.luma_width, self.luma_height),
            PlaneId::U | PlaneId::V => (self.luma_width.div_ceil(2), self.luma_height.div_ceil(2)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_chroma_lr(
        &mut self,
        core: &FrameHeaderCore,
        offset: ByteOffset,
        plane_id: PlaneId,
        lr_source_blocks: &[WienerNsLrSourceBlock],
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
        if plane.restoration_type != FrameRestorationType::WienerNonsep
            || !lr_source_blocks
                .iter()
                .any(|block| block.plane == plane_index)
        {
            return Ok(());
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
        let mut lr_chroma = Vec::with_capacity(cdef_chroma.len());
        for &sample in cdef_chroma {
            lr_chroma.push(T::try_from_u16(sample).map_err(|_| luma_lr_filter_error(offset))?);
        }

        for block in lr_source_blocks
            .iter()
            .filter(|block| block.plane == plane_index)
        {
            self.apply_chroma_lr_block(
                offset,
                plane_id,
                block,
                lr_unit_filters,
                frame_coeffs.as_ref(),
                curr_chroma,
                cdef_chroma,
                curr_luma,
                cdef_luma,
                &mut lr_chroma,
            )?;
        }

        let rect = PlaneRect::new(0, 0, plane_width, plane_height)
            .map_err(|_| luma_lr_filter_error(offset))?;
        self.workspace
            .write_rect(plane_id, rect, &lr_chroma, plane_width)
            .map_err(|_| luma_lr_filter_error(offset))
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_chroma_lr_block(
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
        lr_chroma: &mut [T],
    ) -> Result<()> {
        let (plane_width, plane_height) = self.plane_dimensions(plane_id);
        let coeffs = match frame_coeffs {
            Some(coeffs) => *coeffs,
            None => chroma_lr_unit_coeffs(lr_unit_filters, block, offset)?,
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
        let chroma_bounds = super::super::wienerns_lr_source_block_bounds(block, 1, 1);
        let luma_bounds = super::super::wienerns_lr_source_block_bounds(block, 1, 1);
        let source_error = core::cell::RefCell::new(None::<ReconError>);
        wiener_ns_filter_chroma_block(
            &mut output,
            &params,
            |x, y| {
                let sample = lr_plane_source_sample::<T>(
                    plane_id,
                    curr_chroma,
                    cdef_chroma,
                    plane_width,
                    plane_height,
                    &chroma_bounds,
                    x,
                    y,
                );
                match sample {
                    Ok(sample) => sample,
                    Err(error) => {
                        if source_error.borrow().is_none() {
                            *source_error.borrow_mut() = Some(error);
                        }
                        T::default()
                    }
                }
            },
            |x, y| {
                let sample = lr_plane_source_sample::<T>(
                    PlaneId::Y,
                    curr_luma,
                    cdef_luma,
                    self.luma_width,
                    self.luma_height,
                    &luma_bounds,
                    x,
                    y,
                );
                match sample {
                    Ok(sample) => sample,
                    Err(error) => {
                        if source_error.borrow().is_none() {
                            *source_error.borrow_mut() = Some(error);
                        }
                        T::default()
                    }
                }
            },
        )
        .map_err(|_| luma_lr_filter_error(offset))?;
        if source_error.into_inner().is_some() {
            return Err(luma_lr_filter_error(offset));
        }
        copy_lr_block(
            lr_chroma,
            plane_width,
            block.x,
            block.y,
            block.width,
            block.height,
            &output,
            offset,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_luma_lr_block(
        &self,
        offset: ByteOffset,
        block: &WienerNsLrSourceBlock,
        curr_luma: &[u16],
        cdef_luma: &[u16],
        qindex: u32,
        num_classes: usize,
        filter_set_index: usize,
        coeffs: &[[i16; WIENER_NS_LUMA_COEFFS]],
        lr_luma: &mut [T],
    ) -> Result<()> {
        let sample_count = block
            .width
            .checked_mul(block.height)
            .ok_or_else(|| luma_lr_filter_error(offset))?;
        let subclasses = if num_classes > 1 {
            Some(self.luma_lr_subclasses(
                offset,
                block,
                curr_luma,
                cdef_luma,
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
        let bounds = super::super::wienerns_lr_source_block_bounds(block, 0, 0);
        let block_x = usize_to_isize_recon(block.x, "luma LR block x")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let block_y = usize_to_isize_recon(block.y, "luma LR block y")
            .map_err(|_| luma_lr_filter_error(offset))?;
        let source_error = core::cell::RefCell::new(None::<ReconError>);
        wiener_ns_filter_luma_block(&mut output, &params, |dx, dy| {
            let x = source_coordinate_add_recon(block_x, dx, "luma LR source x");
            let y = source_coordinate_add_recon(block_y, dy, "luma LR source y");
            let sample = match (x, y) {
                (Ok(x), Ok(y)) => lr_plane_source_sample::<T>(
                    PlaneId::Y,
                    curr_luma,
                    cdef_luma,
                    self.luma_width,
                    self.luma_height,
                    &bounds,
                    x,
                    y,
                ),
                (Err(error), _) | (_, Err(error)) => Err(error),
            };
            match sample {
                Ok(sample) => sample,
                Err(error) => {
                    if source_error.borrow().is_none() {
                        *source_error.borrow_mut() = Some(error);
                    }
                    T::default()
                }
            }
        })
        .map_err(|_| luma_lr_filter_error(offset))?;
        if source_error.into_inner().is_some() {
            return Err(luma_lr_filter_error(offset));
        }
        copy_lr_block(
            lr_luma,
            self.luma_width,
            block.x,
            block.y,
            block.width,
            block.height,
            &output,
            offset,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn luma_lr_subclasses(
        &self,
        offset: ByteOffset,
        block: &WienerNsLrSourceBlock,
        curr_luma: &[u16],
        cdef_luma: &[u16],
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
        let mut cell_subclasses = vec![None; cell_cols * cell_rows];
        let mut subclasses = Vec::with_capacity(sample_count);
        for row in 0..block.height {
            for col in 0..block.width {
                let cell_row = row / MI_SIZE;
                let cell_col = col / MI_SIZE;
                let cell_index = cell_row
                    .checked_mul(cell_cols)
                    .and_then(|start| start.checked_add(cell_col))
                    .ok_or_else(|| luma_lr_filter_error(offset))?;
                let subclass =
                    if let Some(subclass) = cell_subclasses.get(cell_index).copied().flatten() {
                        subclass
                    } else {
                        let class_x = block
                            .x
                            .checked_add(cell_col.saturating_mul(MI_SIZE))
                            .ok_or_else(|| luma_lr_filter_error(offset))?;
                        let class_y = block
                            .y
                            .checked_add(cell_row.saturating_mul(MI_SIZE))
                            .ok_or_else(|| luma_lr_filter_error(offset))?;
                        let subclass = self.luma_lr_subclass_at(
                            offset,
                            block,
                            curr_luma,
                            cdef_luma,
                            qindex,
                            num_classes,
                            filter_set_index,
                            class_x,
                            class_y,
                        )?;
                        let Some(slot) = cell_subclasses.get_mut(cell_index) else {
                            return Err(luma_lr_filter_error(offset));
                        };
                        *slot = Some(subclass);
                        subclass
                    };
                subclasses.push(subclass);
            }
        }
        Ok(subclasses)
    }

    #[allow(clippy::too_many_arguments)]
    fn luma_lr_subclass_at(
        &self,
        offset: ByteOffset,
        block: &WienerNsLrSourceBlock,
        curr_luma: &[u16],
        cdef_luma: &[u16],
        qindex: u32,
        num_classes: usize,
        filter_set_index: usize,
        class_x: usize,
        class_y: usize,
    ) -> Result<usize> {
        let Some(tx_skip_grid) = self.tx_skip_grid.as_ref() else {
            return Err(luma_lr_filter_error(offset));
        };
        let bounds = super::super::wienerns_lr_source_block_bounds(block, 0, 0);
        let block_start_x = (block.x >> 6) << 6;
        let block_end_x = super::super::pc_wiener_block_end_x(block, block_start_x)
            .map_err(|_| luma_lr_filter_error(offset))?;
        let params = PcWienerClassifyParams {
            x: usize_to_isize_recon(class_x, "luma LR PC-Wiener x")
                .map_err(|_| luma_lr_filter_error(offset))?,
            y: usize_to_isize_recon(class_y, "luma LR PC-Wiener y")
                .map_err(|_| luma_lr_filter_error(offset))?,
            bit_depth: self.bit_depth,
            base_q_idx: qindex,
            block_start_x,
            block_end_x,
            luma_stripe_start_y: block.luma_stripe_start_y,
            luma_stripe_end_y: block.luma_stripe_end_y,
            tile_start_y: mi_to_luma_start_recon(block.tile_mi_row_start, "luma LR tile start y")
                .map_err(|_| luma_lr_filter_error(offset))?,
            tile_end_y: mi_to_luma_end_recon(block.tile_mi_row_end, "luma LR tile end y")
                .map_err(|_| luma_lr_filter_error(offset))?,
        };
        let classification = pc_wiener_classify::<T, _, _>(
            &params,
            |x, y| {
                lr_plane_source_sample(
                    PlaneId::Y,
                    curr_luma,
                    cdef_luma,
                    self.luma_width,
                    self.luma_height,
                    &bounds,
                    x,
                    y,
                )
            },
            |lookup| tx_skip_grid.lookup(super::super::wienerns_lr_tx_skip_lookup_from_pc(lookup)),
        )
        .map_err(|_| luma_lr_filter_error(offset))?;
        pc_wiener_subclass(num_classes, filter_set_index, classification.class)
            .map_err(|_| luma_lr_filter_error(offset))
    }
}
