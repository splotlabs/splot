// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-recon` - AV2 reconstruction model primitives.
//!
//! This crate provides the first decoded output frame and plane model shared by
//! future decoder, frame-hash, Y4M, reference-frame storage, and encoder
//! roundtrip work. The model is limited to immutable owned output frames,
//! plane storage invariants, and a safe reference-slot container; it does not
//! implement byte-consuming decode, reconstruction algorithms, deterministic
//! frame hashes, Y4M output, or AV2 reference refresh semantics.
//!
//! Feature tracking: `INFRA-RECON-FRAME-PLANE-TYPES`,
//! `RECON-REFERENCE-FRAME-STORE`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

mod error;
mod format;
mod frame;
mod geometry;
mod plane;
mod reference;

pub use error::{ReconError, Result};
pub use format::{BitDepth, PixelFormat, PlaneId, ReconSample};
pub use frame::{DecodedFrame, DecodedFrameInfo, FramePlanes};
pub use geometry::{OutputIndex, PlaneRect, PlaneSize};
pub use plane::{Plane, VisibleRows};
pub use reference::{
    ReferenceFrameEntries, ReferenceFrameEntry, ReferenceFrameStore, ReferenceSlot,
};
