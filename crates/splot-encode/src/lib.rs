// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-encode` — AV2 encoder API.
//!
//! [`Encoder`] takes owned [`SourceFrame`]s built for a validated [`Config`]
//! and returns one [`Packet`] (an AV2 Annex B temporal unit) per frame, with
//! [`FrameStats`]. [`y4m::Y4mReader`] reads input and [`ivf::IvfWriter`] writes
//! output. Speed presets are rows of [`PRESETS`]; rate control goes through
//! [`RateController`], fed by a lookahead that detects scene cuts and models
//! how much later frames reuse each frame. The key-frame tile writer takes CDF rows, contexts and
//! partition rules from `splot_decode::encoder_syntax`; every coded frame is
//! tested against the decoders' output.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

#![feature(portable_simd)]

pub mod config;
pub mod encoder;
pub mod error;
pub mod frame;
mod gop;
mod intra_frame;
pub mod ivf;
mod kernels;
mod lookahead;
mod loop_filter;
mod mode_decision;
mod motion_search;
pub mod preset;
pub mod rate_control;
mod rd;
pub mod stats;
#[cfg(test)]
mod test_support;
mod tile_writer;
mod tpl;
mod tx_choice;
pub mod y4m;

pub use config::{Config, MAX_FRAME_DIMENSION, RateControlMode, TileLayout};
pub use encoder::{Encoder, Packet, StatsSink};
pub use error::{Error, Result};
pub use frame::SourceFrame;
pub use preset::{PRESETS, Preset, PresetParams};
pub use rate_control::{
    Cbr, ConstantQp, Crf, FramePlanInfo, FrameType, RateController, Vbr, max_qindex,
};
pub use splot_parallel::ThreadCount;
pub use splot_recon::{BitDepth, PixelFormat, PlaneSize};
pub use stats::FrameStats;
