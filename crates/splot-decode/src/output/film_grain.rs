// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.21.7 display film-grain output adapter.

use splot_recon::{DecodedFrame, ReconSample};

use crate::Result;
use crate::pipeline::ActiveFilmGrain;

pub(crate) enum DisplayFrame<'a, T: ReconSample> {
    Borrowed(&'a DecodedFrame<T>),
    Owned(Box<DecodedFrame<T>>),
}

impl<T: ReconSample> DisplayFrame<'_, T> {
    pub(crate) fn as_ref(&self) -> &DecodedFrame<T> {
        match self {
            Self::Borrowed(frame) => frame,
            Self::Owned(frame) => frame.as_ref(),
        }
    }
}

pub(crate) fn frame_for_output<'a, T: ReconSample>(
    frame: &'a DecodedFrame<T>,
    grain: Option<&ActiveFilmGrain>,
) -> Result<DisplayFrame<'a, T>> {
    let Some(grain) = grain else {
        return Ok(DisplayFrame::Borrowed(frame));
    };
    let frame = splot_recon::apply_film_grain(frame, &grain.model, grain.grain_seed)?;
    Ok(DisplayFrame::Owned(Box::new(frame)))
}
