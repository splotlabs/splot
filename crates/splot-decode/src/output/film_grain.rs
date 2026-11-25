// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.21.7 display film-grain output adapter.

use splot_core::simd::{ResolvedBackend, SimdBackend, SimdScope};
use splot_recon::{DecodedFrame, FramePlaneSamples, ReconSample};

use crate::Result;
use crate::pipeline::ActiveFilmGrain;

/// The grained planes of the last displayed frame, kept for the next one.
#[derive(Default)]
pub(crate) struct GrainPlanes {
    pub(crate) eight: FramePlaneSamples<u8>,
    pub(crate) ten: FramePlaneSamples<u16>,
}

/// Applies `grain` to `frame` into `spare` using `backend`.
pub(crate) fn apply<T: ReconSample>(
    backend: ResolvedBackend,
    frame: &DecodedFrame<T>,
    grain: &ActiveFilmGrain,
    spare: FramePlaneSamples<T>,
) -> Result<DecodedFrame<T>> {
    backend.enter(Apply {
        frame,
        grain,
        spare,
    })
}

struct Apply<'a, T: ReconSample> {
    frame: &'a DecodedFrame<T>,
    grain: &'a ActiveFilmGrain,
    spare: FramePlaneSamples<T>,
}

impl<T: ReconSample> SimdScope<Result<DecodedFrame<T>>> for Apply<'_, T> {
    fn run<B: SimdBackend>(self, backend: B) -> Result<DecodedFrame<T>> {
        Ok(splot_recon::apply_film_grain_into_with(
            backend,
            self.frame,
            &self.grain.model,
            self.grain.grain_seed,
            self.spare,
        )?)
    }
}

/// Gives `write` the displayed form of `frame`: the frame itself, or a grained
/// copy in the `spare` planes, which keep its storage for the next frame.
pub(crate) fn write_display<T: ReconSample>(
    backend: ResolvedBackend,
    frame: &DecodedFrame<T>,
    grain: Option<&ActiveFilmGrain>,
    spare: &mut FramePlaneSamples<T>,
    write: impl FnOnce(&DecodedFrame<T>) -> Result<()>,
) -> Result<()> {
    let Some(grain) = grain else {
        return write(frame);
    };
    let grained = apply(backend, frame, grain, core::mem::take(spare))?;
    let written = write(&grained);
    *spare = grained.into_plane_samples();
    written
}
