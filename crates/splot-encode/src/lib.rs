// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-encode` — experimental AV2 encoder API and supported packet emitters.
//!
//! The typed push/pull [`Context`] accepts borrowed 8-bit YUV420 input and emits
//! an AV2 Annex B temporal unit for the supported 64x64 all-128 skip-frame subset.
//! Other accepted frames are retired without a packet. Public fixture emitters
//! cover the explicitly named undivided 64x64 intra traces used by decoder-backed
//! oracles.
//!
//! The [`Context`] owns a [`splot_parallel::WorkerPool`] configured by an
//! [`EncoderRuntimeConfig`] thread-count policy; thread count is a runtime knob
//! and never affects bitstream output.
//! The crate uses `splot-recon`'s validated plane/view geometry for its borrowed
//! input API. Its repository roots are `splot-cli`'s decoder-oracle dev tests and
//! the out-of-workspace fuzz crate; no workspace production crate depends on it.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

mod block_symbol_trace;
mod coefficient_tokenization;
pub mod config;
pub mod context;
pub mod error;
pub mod frame;
mod general_intra_trace;
mod intra_mode_emission;
mod partition_emission;
pub mod runtime;

pub use config::{BitDepth, ChromaSubsampling, DEFAULT_QP, EncoderConfig};
pub use context::{
    Context, EncoderOperation, EncoderState, FlushStatus, Packet, ReceivePacketStatus,
    SendFrameStatus,
};
pub use error::{Error, Result};
pub use frame::{
    Frame, FrameId, FrameInfo, FramePlaneInput, FramePlanesInput, FrameTimestamp, RetainedFrame,
};
pub use general_intra_trace::{
    emit_minimal_intra_2d_ivf, emit_minimal_intra_all_planes_coded_ivf,
    emit_minimal_intra_coded_chroma_ivf, emit_minimal_intra_coded_chroma_v_ivf,
    emit_minimal_intra_coded_dc_ivf, emit_minimal_intra_eob3_ivf, emit_minimal_intra_skip_ivf,
    emit_minimal_intra_two_coeff_ivf, emit_minimal_intra_two_nonzero_ivf,
    emit_minimal_intra_visible_ac_ivf,
};
pub use runtime::{EncoderRuntimeConfig, SpeedPreset, SpeedPresetError};
pub use splot_recon::{PlaneRect, PlaneSize};
