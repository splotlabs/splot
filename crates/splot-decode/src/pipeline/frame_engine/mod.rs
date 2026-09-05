// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The unified generic AV2 §7 frame-decode engine.
//!
//! [`walk_frame`] is the single entry for every frame: it runs the entropy walk
//! and reconstruction and leaves the filter phase owed as a
//! [`finish::WalkedFrame`], which the driver hands to the admission scheduler
//! (see [`crate::pipeline::inflight`]). [`FrameSetup`] carries
//! the frame-level branch between key (intra) and inter frames — the genuinely
//! frame-level divergence (references, CDF load, order-hint history, warp /
//! temporal-MV banks, skip-mode, segmentation, CfL enable). Below the setup, the
//! partition walk, per-block engine ([`crate::prediction::inter::block`]), coeff /
//! residual decode, inverse transform, loop filters, and output are shared.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_recon::{BitDepth, ReconSample};

use crate::prediction::inter::{self, InterReferenceState};
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};

pub(crate) mod finish;
pub(crate) mod intra;

use self::finish::FrameWalk;

pub(crate) enum FrameSetup<'a, T: ReconSample> {
    Intra,
    Inter(&'a InterReferenceState<T>),
}

/// Runs one frame's walk phase: the entropy walk and reconstruction, up to the
/// end-of-walk artifacts the filter phase and the driver each consume.
///
/// # Errors
///
/// Returns the walk's own diagnostic when the header, tile plan, or block walk
/// fails.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_frame<T: ReconSample>(
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
) -> Result<FrameWalk<T>> {
    match *setup {
        FrameSetup::Inter(reference) => inter::walk_inter_frame(
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
        FrameSetup::Intra => intra::walk_intra_frame::<T>(
            scratch,
            plan,
            candidate,
            bytes,
            frame_envelope,
            core,
            sequence,
            options,
            bit_depth,
        ),
    }
}
