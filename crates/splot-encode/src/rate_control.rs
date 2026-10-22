// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rate control: picks the quantizer of each frame before it is coded.
//!
//! [`ConstantQp`] codes every frame at one `base_q_idx`. [`Crf`] starts from a
//! quality level and adds each frame's lookahead offset, so frames that later
//! frames predict from get a finer quantizer. [`BitrateController`] (VBR and
//! CBR) picks a base qindex per lookahead window so that the predicted bits of
//! the window meet the budget, and adds the same offsets. Its frame-size model
//! is `bits = k * complexity / step(qindex)` with one `k` for key frames and
//! one per mini-GOP layer (the deepest shared), learnt from the coded bits. All decisions are integer
//! arithmetic.

use std::collections::VecDeque;

use splot_recon::{BitDepth, max_quantizer_index, quantizer_value};

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
    /// Zero-based input (display) index.
    pub display_index: u64,
    /// Frame type.
    pub frame_type: FrameType,
    /// Depth in the mini-GOP hierarchy; `0` for key frames and the last frame
    /// of each mini-GOP.
    pub layer: u8,
    /// Lookahead estimate of the frame's coding cost: the sum of its blocks'
    /// best SATD on the downscaled luma.
    pub complexity: u64,
    /// The lookahead's suggested change to the base qindex: negative for
    /// frames that later frames predict from.
    pub qindex_offset: i16,
    /// Sample bit depth; it sets the qindex range `1..=`[`max_qindex`].
    pub bit_depth: BitDepth,
}

/// Picks per-frame quantizers and learns from the coded result.
pub trait RateController {
    /// Receives the planned frames of the lookahead window in coding order
    /// before the encoder codes the first of them. The default ignores them.
    fn plan_window(&mut self, _window: &[FramePlanInfo]) {}
    /// Returns the `base_q_idx` for the planned frame. The encoder clamps it
    /// to `1..=`[`max_qindex`] of the bit depth.
    fn frame_qp(&mut self, info: &FramePlanInfo) -> u16;
    /// Records the statistics of the frame just coded.
    fn update(&mut self, stats: &FrameStats);
}

/// Uses the same quantizer for every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstantQp {
    /// The `base_q_idx` of every frame.
    pub qp: u16,
}

impl RateController for ConstantQp {
    fn frame_qp(&mut self, _info: &FramePlanInfo) -> u16 {
        self.qp
    }

    fn update(&mut self, _stats: &FrameStats) {}
}

/// The largest `base_q_idx` at `bit_depth`: `MaxQ` of § 6.4.1 Table 6.3, 255
/// at 8 bits and 303 at 10 bits.
#[must_use]
pub fn max_qindex(bit_depth: BitDepth) -> u16 {
    max_quantizer_index(bit_depth) as u16
}

/// `base` moved by `offset`, clamped to `1..=max_qindex(bit_depth)`.
pub(crate) fn with_offset(base: u16, offset: i16, bit_depth: BitDepth) -> u16 {
    (i32::from(base) + i32::from(offset)).clamp(1, i32::from(max_qindex(bit_depth))) as u16
}

/// Constant quality: the base qindex is `4 * crf` at 8 bits, plus each
/// frame's offset.
///
/// The § 7.14.2 quantizer step doubles every 24 qindex, and 10-bit samples
/// are 4 times 8-bit ones, so 10 bits adds 48 (`MaxQ` 303 - 255): the same
/// `crf` gives the same step relative to the sample range, and about the same
/// quality, at both bit depths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crf {
    /// The rate factor, `1..=Crf::MAX`; higher means fewer bits.
    pub crf: u8,
}

impl Crf {
    /// The largest rate factor.
    pub const MAX: u8 = 63;

    /// The base qindex of `crf` at `bit_depth`.
    #[must_use]
    pub fn base_qindex(self, bit_depth: BitDepth) -> u16 {
        let extra = max_qindex(bit_depth) - max_qindex(BitDepth::Eight);
        (4 * u16::from(self.crf) + extra).min(max_qindex(bit_depth))
    }
}

impl RateController for Crf {
    fn frame_qp(&mut self, info: &FramePlanInfo) -> u16 {
        with_offset(
            self.base_qindex(info.bit_depth),
            info.qindex_offset,
            info.bit_depth,
        )
    }

    fn update(&mut self, _stats: &FrameStats) {}
}

/// Frames over which VBR spreads its overspend or underspend.
const VBR_HORIZON_FRAMES: i64 = 8;
/// Initial `k` of key and inter frames in Q8, from 160x90 and 720p encodes.
const INITIAL_MODEL_Q8: [u64; CLASSES] = [40 << 8, 40 << 8, 32 << 8, 24 << 8];
/// Model classes: key frames, then inter layers `0`, `1` and `2` or deeper.
const CLASSES: usize = 4;

/// The model class of a frame.
fn class(info: &FramePlanInfo) -> usize {
    match info.frame_type {
        FrameType::Key => 0,
        FrameType::Inter => 1 + usize::from(info.layer).min(CLASSES - 2),
    }
}

/// The § 7.14.2 luma AC quantizer step of `qindex`.
fn step(qindex: u16, bit_depth: BitDepth) -> u64 {
    u64::from(quantizer_value(u32::from(qindex), 0, bit_depth))
}

/// Variable or constant bitrate.
#[derive(Debug, Clone)]
pub struct BitrateController {
    frame_bits: u64,
    buffer_bits: Option<u64>,
    model_q8: [u64; CLASSES],
    learnt: [bool; CLASSES],
    /// Bits spent minus bits budgeted so far.
    drift: i64,
    base: u16,
    /// Frame index, model class, complexity and bit depth of the planned frames.
    planned: VecDeque<(u64, usize, u64, BitDepth)>,
}

impl BitrateController {
    /// VBR: the average bitrate meets `bitrate_kbps` over a few seconds.
    #[must_use]
    pub fn vbr(bitrate_kbps: u32, frame_rate: (u32, u32)) -> Self {
        Self::new(bitrate_kbps, frame_rate, None)
    }

    /// CBR: every window meets `bitrate_kbps`, and no frame overflows a
    /// buffer of `buffer_ms` that starts half full.
    #[must_use]
    pub fn cbr(bitrate_kbps: u32, buffer_ms: u32, frame_rate: (u32, u32)) -> Self {
        let buffer = u64::from(bitrate_kbps) * u64::from(buffer_ms);
        Self::new(bitrate_kbps, frame_rate, Some(buffer))
    }

    fn new(bitrate_kbps: u32, (num, den): (u32, u32), buffer_bits: Option<u64>) -> Self {
        Self {
            frame_bits: (u64::from(bitrate_kbps) * 1000 * u64::from(den) / u64::from(num.max(1)))
                .max(1),
            buffer_bits,
            model_q8: INITIAL_MODEL_Q8,
            learnt: [false; CLASSES],
            drift: 0,
            base: 128,
            planned: VecDeque::new(),
        }
    }

    fn predict(&self, info: &FramePlanInfo, qindex: u16) -> u64 {
        let model = self.model_q8[class(info)];
        (model * info.complexity.max(1) / step(qindex, info.bit_depth)) >> 8
    }
}

impl RateController for BitrateController {
    fn plan_window(&mut self, window: &[FramePlanInfo]) {
        let frames = window.len() as i64;
        let horizon = if self.buffer_bits.is_some() {
            frames
        } else {
            frames.max(VBR_HORIZON_FRAMES)
        };
        let target = self.frame_bits as i64 * frames;
        let budget = (target - self.drift * frames / horizon.max(1)).max(target / 4);
        let predicted = |base: u16| {
            window
                .iter()
                .map(|info| {
                    let qindex = with_offset(base, info.qindex_offset, info.bit_depth);
                    self.predict(info, qindex)
                })
                .sum::<u64>()
        };
        let max = window
            .first()
            .map_or(u16::MAX, |info| max_qindex(info.bit_depth));
        self.base = (1..=max)
            .find(|&base| predicted(base) as i64 <= budget)
            .unwrap_or(max);
    }

    fn frame_qp(&mut self, info: &FramePlanInfo) -> u16 {
        let mut qindex = with_offset(self.base, info.qindex_offset, info.bit_depth);
        if let Some(buffer) = self.buffer_bits {
            let level = (buffer as i64 / 2 + self.drift).max(0) as u64;
            let room = (buffer + self.frame_bits).saturating_sub(level);
            while qindex < max_qindex(info.bit_depth) && self.predict(info, qindex) > room {
                qindex += 1;
            }
        }
        if self.planned.len() >= 64 {
            self.planned.pop_front();
        }
        self.planned.push_back((
            info.frame_index,
            class(info),
            info.complexity,
            info.bit_depth,
        ));
        qindex
    }

    fn update(&mut self, stats: &FrameStats) {
        self.drift += stats.bits as i64 - self.frame_bits as i64;
        let Some(&(_, class, complexity, bit_depth)) = self
            .planned
            .iter()
            .find(|planned| planned.0 == stats.frame_index)
        else {
            return;
        };
        let observed = ((stats.bits * step(stats.qp, bit_depth)) << 8) / complexity.max(1);
        self.model_q8[class] = if self.learnt[class] {
            (3 * self.model_q8[class] + observed) / 4
        } else {
            observed
        };
        self.learnt[class] = true;
    }
}

#[cfg(test)]
#[path = "rate_control_tests.rs"]
mod tests;
