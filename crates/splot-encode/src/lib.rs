// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-encode` — AV2 encoder API.
//!
//! [`Encoder`] takes owned [`SourceFrame`]s built for a validated [`Config`]
//! and returns one [`Packet`] (an AV2 Annex B temporal unit) per frame, with
//! [`FrameStats`]. [`y4m::Y4mReader`] reads input and [`ivf::IvfWriter`] writes
//! output. Speed presets are rows of [`PRESETS`]; rate control goes through
//! [`RateController`]. The key-frame tile writer takes CDF rows, contexts and
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
mod intra_frame;
pub mod ivf;
mod kernels;
mod loop_filter;
mod mode_decision;
pub mod preset;
pub mod rate_control;
mod rd;
pub mod stats;
#[cfg(test)]
mod test_support;
mod tile_writer;
pub mod y4m;

pub use config::{Config, MAX_FRAME_DIMENSION, RateControlMode};
pub use encoder::{Encoder, Packet, StatsSink};
pub use error::{Error, Result};
pub use frame::SourceFrame;
pub use preset::{PRESETS, Preset, PresetParams};
pub use rate_control::{ConstantQp, FramePlanInfo, FrameType, RateController};
pub use splot_parallel::ThreadCount;
pub use splot_recon::{BitDepth, PixelFormat, PlaneSize};
pub use stats::FrameStats;
