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

use splot_recon::{
    CurrentFrameWorkspace, DecodedFrameInfo, IntraRectBlockSize, PlaneId, PlaneRect, PlaneSize,
    ReconError, ReconSample, WorkspaceRectRows,
};

use super::{McBlockRect, mc_planes};
use crate::Result;

#[derive(Debug)]
struct WindowPlane<T: ReconSample> {
    rect: PlaneRect,
    storage: PlaneSize,
    buf: Vec<T>,
    written: bool,
}

/// Block-local render target with frame-identical coordinates and bounds.
#[derive(Debug)]
pub(crate) struct BlockReconWindow<T: ReconSample> {
    info: DecodedFrameInfo,
    planes: [Option<WindowPlane<T>>; 3],
}

const PLANE_IDS: [PlaneId; 3] = [PlaneId::Y, PlaneId::U, PlaneId::V];

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
        for (plane, sub_x, sub_y) in mc_planes(info.pixel_format()) {
            let Ok(workspace_plane) = workspace.plane(plane) else {
                continue;
            };
            let storage = workspace_plane.storage_size();
            let (x, y, w, h) = rect.plane_rect(plane, sub_x, sub_y);
            let w = w.min(storage.width().saturating_sub(x));
            let h = h.min(storage.height().saturating_sub(y));
            let plane_rect = PlaneRect::new(x, y, w, h)?;
            planes[plane.index()] = Some(WindowPlane {
                rect: plane_rect,
                storage,
                buf: vec![T::default(); w * h],
                written: false,
            });
        }
        Ok(Self { info, planes })
    }

    fn plane(&self, plane: PlaneId) -> splot_recon::Result<&WindowPlane<T>> {
        self.planes[plane.index()]
            .as_ref()
            .ok_or(ReconError::MissingWorkspacePlane { plane })
    }

    fn plane_mut(&mut self, plane: PlaneId) -> splot_recon::Result<&mut WindowPlane<T>> {
        self.planes[plane.index()]
            .as_mut()
            .ok_or(ReconError::MissingWorkspacePlane { plane })
    }

    /// Publishes every written plane rect into the shared workspace.
    ///
    /// # Errors
    /// Propagates workspace write errors.
    pub(crate) fn publish(&self, workspace: &mut CurrentFrameWorkspace<T>) -> Result<()> {
        for plane in PLANE_IDS {
            let Some(window_plane) = &self.planes[plane.index()] else {
                continue;
            };
            if !window_plane.written {
                continue;
            }
            workspace.write_rect(
                plane,
                window_plane.rect,
                &window_plane.buf,
                window_plane.rect.width(),
            )?;
        }
        Ok(())
    }
}

impl<T: ReconSample> WindowPlane<T> {
    fn checked_offsets(
        &self,
        plane: PlaneId,
        rect: PlaneRect,
    ) -> splot_recon::Result<(usize, usize)> {
        let in_window = rect.x() >= self.rect.x()
            && rect.y() >= self.rect.y()
            && rect.x() + rect.width() <= self.rect.x() + self.rect.width()
            && rect.y() + rect.height() <= self.rect.y() + self.rect.height();
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
                let target = window.plane_mut(plane)?;
                let (x0, y0) = target.checked_offsets(plane, rect)?;
                if row_stride_samples < rect.width() {
                    return Err(ReconError::WorkspaceWriteStrideTooSmall {
                        plane,
                        stride_samples: row_stride_samples,
                        width: rect.width(),
                    });
                }
                let window_stride = target.rect.width();
                for row in 0..rect.height() {
                    let source = samples
                        .get(row * row_stride_samples..)
                        .and_then(|tail| tail.get(..rect.width()))
                        .ok_or(ReconError::BufferLengthMismatch {
                            expected: (rect.height() - 1) * row_stride_samples + rect.width(),
                            actual: samples.len(),
                        })?;
                    for (column, &value) in source.iter().enumerate() {
                        if value.to_u16() > max {
                            return Err(ReconError::SampleOutOfRange {
                                plane,
                                sample_index: column,
                                value: value.to_u16(),
                                max,
                            });
                        }
                    }
                    let start = (y0 + row) * window_stride + x0;
                    target.buf[start..start + rect.width()].copy_from_slice(source);
                }
                target.written = true;
                Ok(())
            }
        }
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
                let (x0, y0) = source.checked_offsets(plane, rect)?;
                Ok(SinkRectRows::Window {
                    buf: &source.buf,
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
