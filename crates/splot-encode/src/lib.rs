// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-encode` — AV2 encoder API.
//!
//! [`Encoder`] takes owned [`SourceFrame`]s built for a validated [`Config`]
//! and returns one [`Packet`] (an AV2 Annex B temporal unit) per frame, with
//! [`FrameStats`]. [`y4m::Y4mReader`] reads input and [`ivf::IvfWriter`] writes
//! output. Speed presets are rows of [`PRESETS`]; rate control goes through
//! [`RateController`]. The fixture emitters cover named undivided 64x64 intra
//! traces used by decoder-backed oracles.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

#![feature(portable_simd)]

mod block_symbol_trace;
mod coefficient_tokenization;
pub mod config;
pub mod encoder;
pub mod error;
pub mod frame;
mod general_intra_trace;
mod intra_mode_emission;
pub mod ivf;
mod kernels;
mod partition_emission;
pub mod preset;
pub mod rate_control;
pub mod stats;
pub mod y4m;

pub use config::{Config, MAX_FRAME_DIMENSION, RateControlMode};
pub use encoder::{Encoder, Packet, StatsSink};
pub use error::{Error, Result};
pub use frame::SourceFrame;
pub use general_intra_trace::{
    emit_minimal_intra_2d_ivf, emit_minimal_intra_all_planes_coded_ivf,
    emit_minimal_intra_coded_chroma_ivf, emit_minimal_intra_coded_chroma_v_ivf,
    emit_minimal_intra_coded_dc_ivf, emit_minimal_intra_eob3_ivf, emit_minimal_intra_skip_ivf,
    emit_minimal_intra_two_coeff_ivf, emit_minimal_intra_two_nonzero_ivf,
    emit_minimal_intra_visible_ac_ivf,
};
pub use preset::{PRESETS, Preset, PresetParams};
pub use rate_control::{ConstantQp, FramePlanInfo, FrameType, RateController};
pub use splot_parallel::ThreadCount;
pub use splot_recon::{BitDepth, PixelFormat, PlaneSize};
pub use stats::FrameStats;
