// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-encode` — API surface for the future AV2 encoder.
//!
//! This crate fixes the *shape* of the encoder API (configuration, borrowed frame
//! input views, explicit retained input sharing, and a push/pull [`Context`]).
//! The push/pull lifecycle is deterministic and typed, and
//! [`Context::receive_packet`] now returns a real coded access unit (an AV2
//! Annex B temporal unit) for the input subset the minimal encoder can encode
//! losslessly (a 64x64 all-128 frame — the skip frame, which decodes back to
//! the input once muxed into a container). Nothing in the repository depends on
//! this crate except `splot-cli`.
//!
//! The [`Context`] owns a [`splot_parallel::WorkerPool`] configured by an
//! [`EncoderRuntimeConfig`] thread-count policy; thread count is a runtime knob
//! and never affects bitstream output.
//! The crate uses `splot-recon`'s validated plane/view geometry for its borrowed
//! input API. Forward quantization of arbitrary input, closed-loop reconstruction,
//! lookahead materialization, and rate control remain future work.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

mod block_symbol_trace;
mod closed_loop;
mod coefficient_tokenization;
pub mod config;
pub mod context;
mod core_boundary;
mod decide;
pub mod error;
mod forward_transform;
mod forward_transform_16x16;
pub mod frame;
mod general_intra_trace;
mod header_plan;
mod intra_mode_emission;
mod partition_emission;
mod quantization;
mod quantization_16x16;
#[cfg(test)]
mod quantization_test_support;
mod recon_boundary;
mod residual;
pub mod runtime;
mod syntax_ir;

const _: fn() -> usize = core_boundary::dependency_marker;
const _: fn() -> usize = recon_boundary::dependency_marker;

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
