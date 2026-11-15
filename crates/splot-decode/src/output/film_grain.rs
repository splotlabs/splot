// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.21.7 display film-grain output adapter.

use splot_recon::{DecodedFrame, FramePlaneSamples, ReconSample};

use crate::Result;
use crate::pipeline::ActiveFilmGrain;

/// The grained planes of the last displayed frame, kept for the next one.
#[derive(Default)]
pub(crate) struct GrainPlanes {
    pub(crate) eight: FramePlaneSamples<u8>,
    pub(crate) ten: FramePlaneSamples<u16>,
}

/// Gives `write` the displayed form of `frame`: the frame itself, or a grained
/// copy in the `spare` planes, which keep its storage for the next frame.
pub(crate) fn write_display<T: ReconSample>(
    frame: &DecodedFrame<T>,
    grain: Option<&ActiveFilmGrain>,
    spare: &mut FramePlaneSamples<T>,
    write: impl FnOnce(&DecodedFrame<T>) -> Result<()>,
) -> Result<()> {
    let Some(grain) = grain else {
        return write(frame);
    };
    let grained = splot_recon::apply_film_grain_into(
        frame,
        &grain.model,
        grain.grain_seed,
        core::mem::take(spare),
    )?;
    let written = write(&grained);
    *spare = grained.into_plane_samples();
    written
}
