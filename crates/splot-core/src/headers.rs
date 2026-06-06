// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <contact@splotlabs.io>

//! Placeholder sequence- and frame-header types.
//!
//! Only fields citable from AV2 v1.0.0 are modeled; the full syntax is not yet
//! implemented. Do not add fields that are not backed by the spec — use a
//! `TODO(spec)` marker instead.

/// AV2 sequence header (`sequence_header_obu()`, AV2 v1.0.0 § 5.4). Not yet modeled.
// TODO(spec): model sequence_header_obu() fields (AV2 v1.0.0 § 5.4).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceHeader {}

/// AV2 frame header (`frame_header()`). Not yet modeled.
// TODO(spec): model frame header syntax (AV2 v1.0.0 § 5 frame header OBUs).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameHeader {}
