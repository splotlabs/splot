// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Prediction and residual reconstruction handoffs for decoded frames.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

mod first_block;
mod ibp;
mod middle;
mod one_sided;
mod sink;
mod smooth;

#[cfg(test)]
use crate::bitstream::tile_payload::{
    LumaCoeffBlock, SupportedDirectionalLumaMode,
    reconstruct_general_intra_coeff_block_rect_with_prediction,
};
#[cfg(test)]
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraCardinalDirection, IntraRectBlockSize, PixelFormat,
    PlaneId,
};

pub(crate) use crate::prediction::chroma::cfl::reconstruct_general_intra_chroma_cfl_block_into;
pub(crate) use crate::prediction::chroma::directional::reconstruct_general_intra_chroma_block_into;
pub(crate) use first_block::*;
pub(crate) use ibp::*;
pub(crate) use middle::*;
pub(crate) use one_sided::*;
pub(crate) use sink::*;
pub(crate) use smooth::*;

#[cfg(test)]
#[path = "reconstruct_tests.rs"]
mod tests;
