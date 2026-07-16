// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The unified generic AV2 §7 frame-decode engine.
//!
//! [`decode_frame`] is the single entry for every frame. [`FrameSetup`] carries
//! the frame-level branch between key (intra) and inter frames — the genuinely
//! frame-level divergence (references, CDF load, order-hint history, warp /
//! temporal-MV banks, skip-mode, segmentation, CfL enable). Below the setup, the
//! partition walk, per-block engine ([`crate::prediction::inter::block`]), coeff /
//! residual decode, inverse transform, loop filters, and output are shared.
//!
//! The migration is a strangler fig: inter frames route through `decode_frame`
//! first (this module), then intra frames join, then the tiered "minimal"
//! allowlist dispatch is retired clause by clause behind op-local markers.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_recon::{BitDepth, DecodedFrame, ReconSample};

use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::prediction::inter::{self, InterReferenceState, TemporalMotionField};
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};

pub(crate) mod intra;

type FrameDecodeOutput<T> = (
    DecodedFrame<T>,
    FrameHeaderCore,
    FrameCdfSubset,
    Option<crate::filters::ccso::CcsoUnitGrid>,
    TemporalMotionField,
);

pub(crate) enum FrameSetup<'a, T: ReconSample> {
    Intra,
    Inter(&'a InterReferenceState<'a, T>),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_frame<T: ReconSample>(
    scratch: &mut inter::InterDecodeScratch<T>,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    setup: &FrameSetup<'_, T>,
    bit_depth: BitDepth,
) -> Result<FrameDecodeOutput<T>> {
    match *setup {
        FrameSetup::Inter(reference) => inter::decode_inter_frame(
            scratch,
            plan,
            candidate,
            bytes,
            frame_envelope,
            core,
            sequence,
            options,
            reference,
            bit_depth,
        ),
        FrameSetup::Intra => {
            let (frame, frame_cdfs, ccso_grid) = intra::decode_intra_frame::<T>(
                scratch,
                plan,
                candidate,
                bytes,
                frame_envelope,
                &core,
                sequence,
                options,
                bit_depth,
            )?;
            Ok((
                frame,
                core,
                frame_cdfs,
                ccso_grid,
                TemporalMotionField::empty(),
            ))
        }
    }
}
