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
use splot_core::ivf::IvfHeader;
use splot_recon::{BitDepth, DecodedFrame, ReconSample};

use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::pipeline::unsupported_at;
use crate::prediction::inter::{self, InterReferenceState};
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};

/// The frame-level branch of the unified decode engine.
///
/// `Inter` carries the per-slot reference state a coded inter frame reads; `Intra`
/// (key / intra-only frames) is near-empty because its frame-level setup is
/// self-contained. Intra frames join `decode_frame` in a later step; the variant
/// is defined now so the branch shape is stable.
pub(crate) enum FrameSetup<'a, T: ReconSample> {
    #[allow(dead_code)]
    Intra,
    Inter(&'a InterReferenceState<'a, T>),
}

/// Decodes one frame through the unified engine.
///
/// Returns the reconstructed frame, its (possibly-updated) header core, and the
/// end-of-frame CDF subset the reference bookkeeping retains.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_frame<T: ReconSample>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    header: IvfHeader,
    setup: &FrameSetup<'_, T>,
    bit_depth: BitDepth,
) -> Result<(DecodedFrame<T>, FrameHeaderCore, FrameCdfSubset)> {
    match *setup {
        FrameSetup::Inter(reference) => inter::decode_inter_frame(
            plan,
            candidate,
            bytes,
            frame_envelope,
            core,
            sequence,
            options,
            header,
            reference,
            bit_depth,
        ),
        FrameSetup::Intra => Err(unsupported_at(
            "frame_engine_intra_unrouted",
            frame_envelope.offset,
            "intra frame decode is not yet routed through the unified engine",
        )),
    }
}
