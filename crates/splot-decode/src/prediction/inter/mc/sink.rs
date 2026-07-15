// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Prediction and residual writes for the ordered inter reconstruction walk.

use splot_recon::{PlaneId, PlaneRect, ReconError, ReconSample};

pub(crate) use splot_recon::CurrentFrameSurface as WorkspaceSink;

const SUBPEL_ROW_CAPACITY: usize = 128;

pub(super) fn write_u16_rect<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    plane: PlaneId,
    rect: PlaneRect,
    samples: &[u16],
    row_stride_samples: usize,
) -> splot_recon::Result<()> {
    let storage = sink.plane_storage_size(plane)?;
    if rect.x() >= storage.width() || rect.y() >= storage.height() {
        return Err(ReconError::WorkspaceRectOutOfBounds {
            plane,
            storage,
            rect,
        });
    }
    let write_rect = PlaneRect::new(
        rect.x(),
        rect.y(),
        rect.width().min(storage.width() - rect.x()),
        rect.height().min(storage.height() - rect.y()),
    )?;
    sink.rect_rows(plane, write_rect)?;
    if row_stride_samples < write_rect.width() {
        return Err(ReconError::WorkspaceWriteStrideTooSmall {
            plane,
            stride_samples: row_stride_samples,
            width: write_rect.width(),
        });
    }
    let expected = (write_rect.height() - 1)
        .checked_mul(row_stride_samples)
        .and_then(|offset| offset.checked_add(write_rect.width()))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "subpel prediction source sample span",
        })?;
    if samples.len() < expected {
        return Err(ReconError::WorkspaceWriteLengthMismatch {
            plane,
            expected,
            actual: samples.len(),
        });
    }
    let max = sink.info().bit_depth().max_sample();
    for (sample_index, &sample) in samples.iter().enumerate() {
        T::try_from_u16(sample)?;
        if sample > max {
            return Err(ReconError::SampleOutOfRange {
                plane,
                sample_index,
                value: sample,
                max,
            });
        }
    }

    let mut row_samples = [T::default(); SUBPEL_ROW_CAPACITY];
    let row_samples =
        row_samples
            .get_mut(..write_rect.width())
            .ok_or(ReconError::BufferLengthMismatch {
                expected: write_rect.width(),
                actual: SUBPEL_ROW_CAPACITY,
            })?;
    for row in 0..write_rect.height() {
        let source_start =
            row.checked_mul(row_stride_samples)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "subpel prediction source row offset",
                })?;
        let source = samples
            .get(source_start..source_start + write_rect.width())
            .ok_or(ReconError::BufferLengthMismatch {
                expected,
                actual: samples.len(),
            })?;
        for (output, &sample) in row_samples.iter_mut().zip(source) {
            *output = T::try_from_u16(sample)?;
        }
        let row_rect = PlaneRect::new(write_rect.x(), write_rect.y() + row, write_rect.width(), 1)?;
        sink.write_rect(plane, row_rect, row_samples, write_rect.width())?;
    }
    Ok(())
}
