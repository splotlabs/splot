// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Prediction and residual writes for the ordered inter reconstruction walk.

use splot_recon::{PlaneId, PlaneRect, ReconSample};

pub(crate) use splot_recon::CurrentFrameSurface as WorkspaceSink;

pub(super) fn write_u16_rect<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    plane: PlaneId,
    rect: PlaneRect,
    samples: &[u16],
    row_stride_samples: usize,
) -> splot_recon::Result<()> {
    sink.write_u16_rect(plane, rect, samples, row_stride_samples)
}
