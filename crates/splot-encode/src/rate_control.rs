// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rate control: picks the quantizer of each frame before it is coded.

use crate::stats::FrameStats;

/// The kind of frame the encoder will code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FrameType {
    /// A key frame: intra only, resets references.
    Key,
    /// An inter frame that predicts from earlier frames.
    Inter,
}

/// What the encoder has decided about a frame before it picks its quantizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FramePlanInfo {
    /// Zero-based coding-order index.
    pub frame_index: u64,
    /// Frame type.
    pub frame_type: FrameType,
}

/// Picks per-frame quantizers and learns from the coded result.
pub trait RateController {
    /// Returns the `base_q_idx` for the planned frame, `1..=255`.
    fn frame_qp(&mut self, info: &FramePlanInfo) -> u8;
    /// Records the statistics of the frame just coded.
    fn update(&mut self, stats: &FrameStats);
}

/// Uses the same quantizer for every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstantQp {
    /// The `base_q_idx` of every frame.
    pub qp: u8,
}

impl RateController for ConstantQp {
    fn frame_qp(&mut self, _info: &FramePlanInfo) -> u8 {
        self.qp
    }

    fn update(&mut self, _stats: &FrameStats) {}
}
