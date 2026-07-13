// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Prediction/residual write target for the inter block engine.
//!
//! [`WorkspaceSink`] lets one reconstruction body target either the shared
//! frame workspace (the ordered walk) or a block-local window buffer
//! ([`BlockReconWindow`]) that a deferred parallel job renders into before the
//! walk thread publishes it. Window accesses keep frame coordinates and frame
//! storage bounds so clamping arithmetic matches the workspace exactly; an
//! access outside the window is an error, which the deferred scheduler treats
//! as "re-run this block inline against the frame workspace".

use core::ops::Range;

use splot_recon::{
    CurrentFrameWorkspace, DecodedFrameInfo, IntraRectBlockSize, PlaneId, PlaneRect, PlaneSize,
    ReconError, ReconSample, WorkspaceRectRows,
};

use super::{McBlockRect, mc_planes};
use crate::Result;

#[derive(Debug)]
struct WindowPlane {
    rect: PlaneRect,
    storage: PlaneSize,
    range: Range<usize>,
    written: bool,
}

/// Block-local render target with frame-identical coordinates and bounds.
#[derive(Debug)]
pub(crate) struct BlockReconWindow<T: ReconSample> {
    info: DecodedFrameInfo,
    planes: [Option<WindowPlane>; 3],
    samples: Vec<T>,
}

const PLANE_IDS: [PlaneId; 3] = [PlaneId::Y, PlaneId::U, PlaneId::V];
const SUBPEL_ROW_CAPACITY: usize = 128;

impl<T: ReconSample> BlockReconWindow<T> {
    /// Builds a window covering the block's per-plane rects, clamped to the
    /// workspace storage the way in-frame reads are.
    ///
    /// # Errors
    /// Returns the geometry error when a clamped plane rect is empty.
    pub(crate) fn for_block(
        workspace: &CurrentFrameWorkspace<T>,
        rect: McBlockRect,
    ) -> splot_recon::Result<Self> {
        let info = workspace.info();
        let mut planes = [None, None, None];
        let mut sample_count = 0usize;
        for (plane, sub_x, sub_y) in mc_planes(info.pixel_format()) {
            let Ok(workspace_plane) = workspace.plane(plane) else {
                continue;
            };
            let storage = workspace_plane.storage_size();
            let (x, y, w, h) = rect.plane_rect(plane, sub_x, sub_y);
            let w = w.min(storage.width().saturating_sub(x));
            let h = h.min(storage.height().saturating_sub(y));
            let plane_rect = PlaneRect::new(x, y, w, h)?;
            let plane_sample_count = w.checked_mul(h).ok_or(ReconError::ArithmeticOverflow {
                context: "block reconstruction window plane sample count",
            })?;
            let end = sample_count.checked_add(plane_sample_count).ok_or(
                ReconError::ArithmeticOverflow {
                    context: "block reconstruction window sample count",
                },
            )?;
            planes[plane.index()] = Some(WindowPlane {
                rect: plane_rect,
                storage,
                range: sample_count..end,
                written: false,
            });
            sample_count = end;
        }
        Ok(Self {
            info,
            planes,
            samples: vec![T::default(); sample_count],
        })
    }

    fn plane(&self, plane: PlaneId) -> splot_recon::Result<&WindowPlane> {
        self.planes[plane.index()]
            .as_ref()
            .ok_or(ReconError::MissingWorkspacePlane { plane })
    }

    /// Publishes every written plane rect into the shared workspace.
    ///
    /// # Errors
    /// Propagates workspace write errors.
    pub(crate) fn publish(&self, workspace: &mut CurrentFrameWorkspace<T>) -> Result<()> {
        for plane in PLANE_IDS {
            let Some(window_plane) = self.planes[plane.index()].as_ref() else {
                continue;
            };
            if !window_plane.written {
                continue;
            }
            let samples = window_plane.samples(&self.samples)?;
            workspace.write_rect(plane, window_plane.rect, samples, window_plane.rect.width())?;
        }
        Ok(())
    }
}

impl WindowPlane {
    fn samples<'a, T>(&self, samples: &'a [T]) -> splot_recon::Result<&'a [T]> {
        samples
            .get(self.range.clone())
            .ok_or(ReconError::BufferLengthMismatch {
                expected: self.range.end,
                actual: samples.len(),
            })
    }

    fn samples_mut<'a, T>(&self, samples: &'a mut [T]) -> splot_recon::Result<&'a mut [T]> {
        let actual = samples.len();
        samples
            .get_mut(self.range.clone())
            .ok_or(ReconError::BufferLengthMismatch {
                expected: self.range.end,
                actual,
            })
    }

    fn checked_offsets(
        &self,
        plane: PlaneId,
        rect: PlaneRect,
    ) -> splot_recon::Result<(usize, usize)> {
        let rect_right =
            rect.x()
                .checked_add(rect.width())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "block reconstruction window rectangle right edge",
                })?;
        let rect_bottom =
            rect.y()
                .checked_add(rect.height())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "block reconstruction window rectangle bottom edge",
                })?;
        let window_right =
            self.rect
                .x()
                .checked_add(self.rect.width())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "block reconstruction window right edge",
                })?;
        let window_bottom = self.rect.y().checked_add(self.rect.height()).ok_or(
            ReconError::ArithmeticOverflow {
                context: "block reconstruction window bottom edge",
            },
        )?;
        let in_window = rect.x() >= self.rect.x()
            && rect.y() >= self.rect.y()
            && rect_right <= window_right
            && rect_bottom <= window_bottom;
        if !in_window {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane,
                storage: self.storage,
                rect,
            });
        }
        Ok((rect.x() - self.rect.x(), rect.y() - self.rect.y()))
    }
}

/// Write/read target shared by the ordered walk and deferred block jobs.
#[derive(Debug)]
pub(crate) enum WorkspaceSink<'a, T: ReconSample> {
    /// The shared frame workspace (ordered raster reconstruction).
    Frame(&'a mut CurrentFrameWorkspace<T>),
    /// A block-local window rendered by a deferred job.
    Window(&'a mut BlockReconWindow<T>),
}

impl<T: ReconSample> WorkspaceSink<'_, T> {
    pub(crate) fn info(&self) -> DecodedFrameInfo {
        match self {
            Self::Frame(workspace) => workspace.info(),
            Self::Window(window) => window.info,
        }
    }

    pub(crate) fn plane_storage_size(&self, plane: PlaneId) -> splot_recon::Result<PlaneSize> {
        match self {
            Self::Frame(workspace) => Ok(workspace.plane(plane)?.storage_size()),
            Self::Window(window) => Ok(window.plane(plane)?.storage),
        }
    }

    pub(crate) fn write_rect(
        &mut self,
        plane: PlaneId,
        rect: PlaneRect,
        samples: &[T],
        row_stride_samples: usize,
    ) -> splot_recon::Result<()> {
        match self {
            Self::Frame(workspace) => {
                workspace.write_rect(plane, rect, samples, row_stride_samples)
            }
            Self::Window(window) => {
                let max = window.info.bit_depth().max_sample();
                let target = window.plane(plane)?;
                let (x0, y0) = target.checked_offsets(plane, rect)?;
                if row_stride_samples < rect.width() {
                    return Err(ReconError::WorkspaceWriteStrideTooSmall {
                        plane,
                        stride_samples: row_stride_samples,
                        width: rect.width(),
                    });
                }
                let window_stride = target.rect.width();
                let expected = (rect.height() - 1)
                    .checked_mul(row_stride_samples)
                    .and_then(|offset| offset.checked_add(rect.width()))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "block reconstruction window source sample span",
                    })?;
                if samples.len() < expected {
                    return Err(ReconError::BufferLengthMismatch {
                        expected,
                        actual: samples.len(),
                    });
                }
                for row in 0..rect.height() {
                    let source_start = row * row_stride_samples;
                    let source = &samples[source_start..source_start + rect.width()];
                    for (column, &value) in source.iter().enumerate() {
                        let value = value.to_u16();
                        if value > max {
                            return Err(ReconError::SampleOutOfRange {
                                plane,
                                sample_index: source_start + column,
                                value,
                                max,
                            });
                        }
                    }
                }
                let target = window.planes[plane.index()]
                    .as_mut()
                    .ok_or(ReconError::MissingWorkspacePlane { plane })?;
                let target_samples = target.samples_mut(&mut window.samples)?;
                for row in 0..rect.height() {
                    let source_start = row * row_stride_samples;
                    let source = &samples[source_start..source_start + rect.width()];
                    let start = (y0 + row) * window_stride + x0;
                    target_samples[start..start + rect.width()].copy_from_slice(source);
                }
                target.written = true;
                Ok(())
            }
        }
    }

    pub(super) fn write_u16_rect(
        &mut self,
        plane: PlaneId,
        rect: PlaneRect,
        samples: &[u16],
        row_stride_samples: usize,
    ) -> splot_recon::Result<()> {
        let write_rect = match self {
            Self::Frame(workspace) => {
                let storage = workspace.plane(plane)?.storage_size();
                if rect.x() >= storage.width() || rect.y() >= storage.height() {
                    return Err(ReconError::WorkspaceRectOutOfBounds {
                        plane,
                        storage,
                        rect,
                    });
                }
                PlaneRect::new(
                    rect.x(),
                    rect.y(),
                    rect.width().min(storage.width() - rect.x()),
                    rect.height().min(storage.height() - rect.y()),
                )?
            }
            Self::Window(window) => {
                window.plane(plane)?.checked_offsets(plane, rect)?;
                rect
            }
        };
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
        let max = self.info().bit_depth().max_sample();
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
            let row_rect =
                PlaneRect::new(write_rect.x(), write_rect.y() + row, write_rect.width(), 1)?;
            self.write_rect(plane, row_rect, row_samples, write_rect.width())?;
        }
        Ok(())
    }

    pub(crate) fn write_rect_block(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        samples: &[T],
    ) -> splot_recon::Result<()> {
        match self {
            Self::Frame(workspace) => workspace.write_rect_block(plane, x, y, size, samples),
            Self::Window(_) => {
                if samples.len() != size.sample_count() {
                    return Err(ReconError::WorkspaceWriteLengthMismatch {
                        plane,
                        expected: size.sample_count(),
                        actual: samples.len(),
                    });
                }
                let rect = PlaneRect::new(x, y, size.width(), size.height())?;
                self.write_rect(plane, rect, samples, size.width())
            }
        }
    }

    pub(crate) fn rect_rows(
        &self,
        plane: PlaneId,
        rect: PlaneRect,
    ) -> splot_recon::Result<SinkRectRows<'_, T>> {
        match self {
            Self::Frame(workspace) => Ok(SinkRectRows::Frame(workspace.rect_rows(plane, rect)?)),
            Self::Window(window) => {
                let source = window.plane(plane)?;
                let samples = source.samples(&window.samples)?;
                let (x0, y0) = source.checked_offsets(plane, rect)?;
                Ok(SinkRectRows::Window {
                    buf: samples,
                    stride: source.rect.width(),
                    start: y0 * source.rect.width() + x0,
                    width: rect.width(),
                    rows: rect.height(),
                    next: 0,
                })
            }
        }
    }
}

/// Row iterator over either sink variant.
pub(crate) enum SinkRectRows<'a, T: ReconSample> {
    Frame(WorkspaceRectRows<'a, T>),
    Window {
        buf: &'a [T],
        stride: usize,
        start: usize,
        width: usize,
        rows: usize,
        next: usize,
    },
}

impl<'a, T: ReconSample> Iterator for SinkRectRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Frame(rows) => rows.next(),
            Self::Window {
                buf,
                stride,
                start,
                width,
                rows,
                next,
            } => {
                if *next >= *rows {
                    return None;
                }
                let offset = *start + *next * *stride;
                *next += 1;
                Some(&buf[offset..offset + *width])
            }
        }
    }
}
