// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-recon` - AV2 reconstruction model primitives.
//!
//! This crate provides the first decoded output frame and plane model shared by
//! future decoder, frame-hash, Y4M, reference-frame storage, and encoder
//! roundtrip work. The model is limited to immutable owned output frames,
//! plane storage invariants, a safe reference-slot container, and deterministic
//! frame-hash input serialization and digest computation, plus source-backed
//! Y4M writing for caller-supplied decoded frames, plus square DC,
//! rectangular DC, subsampled DC, basic/PAETH, smooth, and H/V cardinal
//! directional intra prediction primitives and a mutable current-frame
//! workspace; it does not implement byte-consuming decode, full reconstruction,
//! runtime CLI Y4M output, or AV2 reference refresh semantics.
//!
//! The ownership model is view-first ([`docs/ZERO_COPY.md`](../../../docs/ZERO_COPY.md)):
//! owned plane/frame/workspace storage hands out borrowed [`PlaneRef`]/[`PlaneMut`]
//! and [`FrameRef`]/[`FrameMut`] views without copying, immutable frames are shared
//! without copying pixels via [`SharedFrame`], and no media-storage type implements
//! `Clone`.
//!
//! Feature tracking: `INFRA-RECON-FRAME-PLANE-TYPES`,
//! `INFRA-ZERO-COPY-MEDIA-POLICY`,
//! `RECON-REFERENCE-FRAME-STORE`, `RECON-HASH-INPUT-SERIALIZATION`,
//! `RECON-FRAME-HASH-DIGEST`, `RECON-Y4M-OUTPUT-WRITER`,
//! `RECON-INTRA-DC-SQUARE-PREDICTION`,
//! `RECON-INTRA-DC-RECTANGULAR-PREDICTION`,
//! `RECON-INTRA-DC-SUBSAMPLED-PREDICTION`,
//! `RECON-INTRA-BASIC-PAETH-PREDICTION`,
//! `RECON-INTRA-SMOOTH-PREDICTION`,
//! `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION`,
//! `RECON-CURRENT-FRAME-WORKSPACE`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

mod error;
mod format;
mod frame;
mod geometry;
mod hash_input;
mod intra;
mod intra_basic;
mod intra_dc_math;
mod intra_dc_subsampled;
mod intra_directional;
mod intra_smooth;
mod plane;
mod reference;
mod views;
mod workspace;
mod y4m;

pub use error::{ReconError, Result};
pub use format::{BitDepth, PixelFormat, PlaneId, ReconSample};
pub use frame::{DecodedFrame, DecodedFrameInfo, FramePlanes, SharedFrame};
pub use geometry::{OutputIndex, PlaneRect, PlaneSize};
pub use hash_input::{DecodedFrameHash, DecodedFrameHashInput};
pub use intra::{
    IntraDcEdge, IntraDcEdges, IntraRectBlockSize, IntraSquareBlockSize,
    SquareIntraPredictionBlock, SquareIntraPredictionRows, predict_intra_dc_rect_into,
    predict_intra_dc_rect_value, predict_intra_dc_square, predict_intra_dc_square_into,
    predict_intra_dc_square_value,
};
pub use intra_basic::{IntraPaethEdge, IntraPaethEdges, predict_intra_paeth_rect_into};
pub use intra_dc_subsampled::{
    predict_intra_dc_subsampled_rect_into, predict_intra_dc_subsampled_rect_value,
};
pub use intra_directional::{
    IntraCardinalDirection, IntraCardinalEdge, IntraCardinalEdges,
    predict_intra_cardinal_directional_rect_into,
};
pub use intra_smooth::{
    IntraSmoothEdge, IntraSmoothEdges, IntraSmoothMode, predict_intra_smooth_rect_into,
};
pub use plane::{Plane, VisibleRows};
pub use reference::{
    ReferenceFrameEntries, ReferenceFrameEntry, ReferenceFrameStore, ReferenceSlot,
};
pub use views::{FrameMut, FrameRef, PlaneMut, PlaneMutRows, PlaneRef, PlaneRefRows};
pub use workspace::{
    CurrentFrameIntraEdges, CurrentFramePlane, CurrentFrameWorkspace, WorkspaceRectRows,
};
pub use y4m::{
    Y4mChromaTag, Y4mError, Y4mFrameFormat, Y4mFrameHeader, Y4mFrameRate, Y4mResult,
    Y4mStreamHeader, Y4mWriter,
};
