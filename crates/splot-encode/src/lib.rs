// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-encode` — API surface for the future AV2 encoder.
//!
//! This crate fixes the *shape* of the encoder API (configuration, borrowed frame
//! input views, explicit retained input sharing, and a push/pull [`Context`])
//! without implementing coded packet production. The push/pull lifecycle is
//! deterministic and typed, but [`Context::receive_packet`] cannot yet return a
//! real AV2 packet. Nothing in the repository depends on this crate except
//! `splot-cli`.
//!
//! The [`Context`] now owns a [`splot_parallel::WorkerPool`] configured by an
//! [`EncoderRuntimeConfig`] thread-count policy; thread count is a runtime knob
//! and never affects bitstream output.
//! The crate uses `splot-recon`'s validated plane/view geometry for its borrowed
//! input API. Closed-loop reconstruction, lookahead materialization, and coded
//! packet production remain future work.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

pub mod config;
pub mod context;
mod core_boundary;
pub mod error;
pub mod frame;
mod recon_boundary;
pub mod runtime;
mod syntax_ir;

const _: fn() -> usize = core_boundary::dependency_marker;
const _: fn() -> usize = recon_boundary::dependency_marker;

pub use config::{BitDepth, ChromaSubsampling, EncoderConfig};
pub use context::{
    Context, EncoderOperation, EncoderState, FlushStatus, Packet, ReceivePacketStatus,
    SendFrameStatus,
};
pub use error::{Error, Result};
pub use frame::{
    Frame, FrameId, FrameInfo, FramePlaneInput, FramePlanesInput, FrameTimestamp, RetainedFrame,
};
pub use runtime::EncoderRuntimeConfig;
pub use splot_recon::{PlaneRect, PlaneSize};
