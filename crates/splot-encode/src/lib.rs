// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-encode` — API placeholder for the future AV2 encoder.
//!
//! This crate fixes the *shape* of the encoder API (configuration and a push/pull
//! [`Context`]) without implementing any encoding. Every encoding operation
//! returns [`splot_core::Error::Unimplemented`]. Nothing in the repository depends
//! on this crate except `splot-cli`.
//!
//! The [`Context`] now owns a [`splot_parallel::WorkerPool`] configured by an
//! [`EncoderRuntimeConfig`] thread-count policy; thread count is a runtime knob
//! and never affects bitstream output.
//! The crate also has a private `splot-recon` boundary marker so later encoder
//! phases can reuse reconstruction primitives without creating a public API
//! promise in this placeholder phase.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

pub mod config;
pub mod context;
pub mod error;
mod recon_boundary;
pub mod runtime;

const _: fn() -> usize = recon_boundary::dependency_marker;

pub use config::{BitDepth, ChromaSubsampling, EncoderConfig};
pub use context::{Context, EncoderStatus, Frame, Packet};
pub use error::{Error, Result};
pub use runtime::EncoderRuntimeConfig;
