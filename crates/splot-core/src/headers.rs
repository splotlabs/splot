// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Placeholder sequence- and frame-header types.
//!
//! Only fields citable from AV2 v1.0.0 are modeled; the full syntax is not yet
//! implemented. Do not add fields that are not backed by the spec — leave a spec
//! TODO that names the implementation-matrix feature id instead (see AGENTS.md).

pub mod sequence;

pub use sequence::{SequenceHeader, SequenceHeaderGeneral};

/// AV2 frame header (`frame_header()`). Not yet modeled.
// TODO(spec: AV2-5.18-FRAME-HEADER): model frame header syntax (AV2 v1.0.0 § 5.18).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameHeader {}
