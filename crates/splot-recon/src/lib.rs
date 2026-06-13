// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-recon` - AV2 reconstruction model primitives.
//!
//! This crate provides the first decoded output frame and plane model shared by
//! future decoder, frame-hash, Y4M, and encoder roundtrip work. The model is
//! limited to immutable owned output frames and plane storage invariants; it
//! does not implement byte-consuming decode, reconstruction algorithms,
//! deterministic frame hashes, Y4M output, or reference-frame storage.
//!
//! Feature tracking: `INFRA-RECON-FRAME-PLANE-TYPES`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

mod error;
mod format;
mod frame;
mod geometry;
mod plane;

pub use error::{ReconError, Result};
pub use format::{BitDepth, PixelFormat, PlaneId, ReconSample};
pub use frame::{DecodedFrame, DecodedFrameInfo, FramePlanes};
pub use geometry::{OutputIndex, PlaneRect, PlaneSize};
pub use plane::{Plane, VisibleRows};
