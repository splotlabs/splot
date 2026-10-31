// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rate control: picks the quantizer of each frame before it is coded.
//!
//! [`ConstantQp`] codes every frame at one `base_q_idx`. [`Crf`] starts from a
//! quality level and adds each frame's lookahead offset, so frames that later
//! frames predict from get a finer quantizer: the offsets spread the
//! window over key frames, mini-GOP base frames and the layers between them
//! and the leaves. [`Vbr`] and [`Cbr`] pick a base
//! qindex per lookahead window so that the predicted bits of the window meet
//! the budget, and add the same offsets. Their frame-size model is
//! `bits = k * complexity / step(qindex)` with one `k` for key frames and one
//! per mini-GOP layer (the deepest shared), learnt from the coded bits. All
//! decisions are integer arithmetic.

use std::collections::VecDeque;

use splot_recon::{BitDepth, max_quantizer_index, quantizer_value};

use crate::error::{Error, Result};
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
    /// best SATD on the downscaled luma, each at least a floor.
    pub complexity: u64,
    /// The complexity of a frame whose blocks all cost the floor. A frame
    /// below twice this is flat: its size is mostly headers and says little
    /// about the size of detailed frames.
    pub min_complexity: u64,
    /// The lookahead's suggested change to the base qindex: negative for
    /// frames that later frames predict from.
    pub qindex_offset: i16,
    /// Sample bit depth; it sets the qindex range `1..=`[`max_qindex`].
    pub bit_depth: BitDepth,
}

/// Picks per-frame quantizers and learns from the coded result.
///
/// The encoder codes the input as a sequence of steps: a key frame alone, or
/// a mini-GOP. Before each step it calls [`Self::plan_window`] once. Then it
/// codes the frames of that step in waves of frames that read the same
/// references: [`Self::frame_qp`] for each frame of the wave in coding order,
/// then [`Self::update`] for each frame and [`Self::update_show_existing`] for
/// each show-existing frame, in coding order. Then the next step starts with
/// the next `plan_window` call.
pub trait RateController {
    /// Receives the planned frames of the lookahead window in coding order:
    /// the frames of the next step first, then the frames planned after them,
    /// up to the preset's `lookahead_frames` input frames (fewer at the end of
    /// the input). The default ignores them.
    fn plan_window(&mut self, _window: &[FramePlanInfo]) {}
    /// Returns the `base_q_idx` for the planned frame. The encoder clamps it
    /// to `1..=`[`max_qindex`] of the bit depth.
    fn frame_qp(&mut self, info: &FramePlanInfo) -> u16;
    /// Records the statistics of the frame just coded.
    fn update(&mut self, stats: &FrameStats);
    /// Records the size in bits of a show-existing temporal unit, which codes
    /// no frame. The default ignores it.
    fn update_show_existing(&mut self, _bits: u64) {}
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

/// The largest qindex decrease [`frame_offsets`] gives a frame for its boost.
pub(crate) const MAX_FRAME_BOOST: i32 = 64;
/// The qindex above the base qindex of [`Crf`] at which the leaves of a
/// mini-GOP sit, per layer of its depth.
pub(crate) const LAYER_QINDEX_STEP: i32 = 3;

/// A planned frame of a lookahead window as [`frame_offsets`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowFrame {
    pub(crate) display_index: u64,
    pub(crate) frame_type: FrameType,
    pub(crate) layer: u8,
    /// The temporal model's qindex decrease: 12 per octave of importance.
    pub(crate) boost: i32,
    /// The display indices of the past and future frames it predicts from.
    pub(crate) references: [Option<u64>; 2],
}

/// The qindex offset of each frame of `frames`, in coding order, from the
/// base qindex of [`Crf`]. The leaves of a mini-GOP of depth `d` sit at the
/// CRF level, [`LAYER_QINDEX_STEP`] per layer above the base qindex; a key
/// frame or a frame at layer 0 moves below the base qindex by its boost, at
/// most [`MAX_FRAME_BOOST`]; a frame at layer `l` moves from its base
/// frame's offset to the CRF level by `l / d`, and never below a frame it
/// predicts from.
pub(crate) fn frame_offsets(frames: &[WindowFrame]) -> Vec<i16> {
    let mut offsets: Vec<i32> = Vec::with_capacity(frames.len());
    let (mut base, mut depth) = (0, 0);
    for (index, frame) in frames.iter().enumerate() {
        if frame.layer == 0 {
            depth = frames[index + 1..]
                .iter()
                .take_while(|next| next.layer > 0)
                .map(|next| i32::from(next.layer))
                .max()
                .unwrap_or(0);
            base = -frame.boost.clamp(0, MAX_FRAME_BOOST);
            offsets.push(base);
            continue;
        }
        let layer = i32::from(frame.layer).min(depth);
        let level = LAYER_QINDEX_STEP * depth;
        let interpolated = base + (level - base) * layer / depth.max(1);
        let references = frame.references.iter().flatten().filter_map(|display| {
            let coded = frames[..index]
                .iter()
                .position(|f| f.display_index == *display);
            coded.map(|position| offsets[position])
        });
        offsets.push(references.fold(interpolated, i32::max));
    }
    offsets.into_iter().map(|offset| offset as i16).collect()
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

/// Frames over which VBR spreads its overspend or underspend; VBR remembers
/// at most this many frame budgets of underspend.
const VBR_HORIZON_FRAMES: i64 = 8;
/// Frame budgets of overspend VBR remembers: a key frame can cost more than
/// [`VBR_HORIZON_FRAMES`] budgets, and its key-frame group repays it.
const VBR_DEBT_FRAMES: i64 = 32;
/// Largest frame budget in bits; it keeps the window sums far from overflow.
const MAX_FRAME_BITS: u64 = 1 << 40;
/// Largest bitrate of [`Vbr`] and [`Cbr`]: 1 Gbit/s.
pub const MAX_BITRATE_KBPS: u32 = 1_000_000;
/// Largest [`Cbr`] buffer: ten minutes.
pub const MAX_BUFFER_MS: u32 = 600_000;
/// Initial `k` of key and inter frames in Q8, from 160x90 and 720p encodes.
const INITIAL_MODEL_Q8: [u64; CLASSES] = [40 << 8, 40 << 8, 32 << 8, 24 << 8];
/// Model classes: key frames, then inter layers `0`, `1` and `2` or deeper.
const CLASSES: usize = 4;

/// Whether the lookahead sees a frame as flat: see
/// [`FramePlanInfo::min_complexity`].
fn is_flat(info: &FramePlanInfo) -> bool {
    info.complexity < info.min_complexity.saturating_mul(2)
}

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

/// Checks the settings of a bitrate mode; `buffer_ms` only for CBR.
pub(crate) fn check_bitrate(
    bitrate_kbps: u32,
    buffer_ms: Option<u32>,
    (numerator, denominator): (u32, u32),
) -> Result<()> {
    if numerator == 0 || denominator == 0 {
        return Err(Error::InvalidFrameRate {
            numerator,
            denominator,
        });
    }
    let limits = [
        ("bitrate_kbps", Some(bitrate_kbps), MAX_BITRATE_KBPS),
        ("buffer_ms", buffer_ms, MAX_BUFFER_MS),
    ];
    for (what, value, max) in limits {
        if let Some(value) = value.filter(|value| !(1..=max).contains(value)) {
            return Err(Error::InvalidBitrate { what, value, max });
        }
    }
    Ok(())
}

/// The budget of one frame: `bitrate_kbps * 1000 / frame_rate` bits.
fn frame_bits(bitrate_kbps: u32, (numerator, denominator): (u32, u32)) -> u64 {
    let bits =
        u128::from(bitrate_kbps) * 1000 * u128::from(denominator) / u128::from(numerator.max(1));
    bits.clamp(1, u128::from(MAX_FRAME_BITS)) as u64
}

/// Variable bitrate: the average bitrate meets the target over a few
/// windows. [`Cbr`] uses the same model.
#[derive(Debug, Clone)]
pub struct Vbr {
    frame_bits: u64,
    /// Smallest number of frames that repay the drift.
    horizon: i64,
    /// Largest underspend and overspend the drift holds.
    drift_limits: (i64, i64),
    model_q8: [u64; CLASSES],
    learnt: [bool; CLASSES],
    /// The mean coded size of the flat frames of each class, once one is
    /// coded: mostly headers, whatever the qindex.
    flat_bits: [Option<u64>; CLASSES],
    /// Bits spent minus bits budgeted so far, within `drift_limits`.
    drift: i64,
    base: u16,
    /// The most frames a window has held.
    longest_window: usize,
    /// Frame index, model class, complexity (`0` for a flat frame) and bit
    /// depth of the planned frames.
    planned: VecDeque<(u64, usize, u64, BitDepth)>,
}

impl Vbr {
    /// A VBR controller for `bitrate_kbps` at `frame_rate` frames per second.
    ///
    /// # Errors
    /// Returns [`Error::InvalidBitrate`] for a bitrate outside
    /// `1..=`[`MAX_BITRATE_KBPS`] and [`Error::InvalidFrameRate`] for a zero
    /// frame-rate term.
    pub fn new(bitrate_kbps: u32, frame_rate: (u32, u32)) -> Result<Self> {
        check_bitrate(bitrate_kbps, None, frame_rate)?;
        let frame_bits = frame_bits(bitrate_kbps, frame_rate);
        let limit = |frames: i64| frames.saturating_mul(frame_bits as i64);
        Ok(Self::with_limits(
            frame_bits,
            VBR_HORIZON_FRAMES,
            (limit(VBR_HORIZON_FRAMES), limit(VBR_DEBT_FRAMES)),
        ))
    }

    fn with_limits(frame_bits: u64, horizon: i64, drift_limits: (i64, i64)) -> Self {
        Self {
            frame_bits,
            horizon,
            drift_limits,
            model_q8: INITIAL_MODEL_Q8,
            learnt: [false; CLASSES],
            flat_bits: [None; CLASSES],
            drift: 0,
            base: 128,
            longest_window: 0,
            planned: VecDeque::new(),
        }
    }

    fn predict(&self, info: &FramePlanInfo, qindex: u16) -> u64 {
        if let Some(bits) = self.flat_bits[class(info)].filter(|_| is_flat(info)) {
            return bits;
        }
        let model = u128::from(self.model_q8[class(info)]);
        let bits = (model * u128::from(info.complexity.max(1))
            / u128::from(step(qindex, info.bit_depth)))
            >> 8;
        u64::try_from(bits).unwrap_or(u64::MAX)
    }

    /// Adds `bits` spent against `budget` to the drift.
    fn spend(&mut self, bits: u64, budget: u64) {
        let bits = i64::try_from(bits).unwrap_or(i64::MAX);
        self.drift = self
            .drift
            .saturating_add(bits)
            .saturating_sub(budget as i64)
            .clamp(-self.drift_limits.0, self.drift_limits.1);
    }
}

impl RateController for Vbr {
    /// Picks the smallest base qindex whose predicted window bits fit the
    /// budget: the window's target minus the drift spread over at least
    /// `horizon` frames, within a quarter and twice the target. A window
    /// shorter than an earlier one holds the last frames of the input, which
    /// repay the whole drift.
    fn plan_window(&mut self, window: &[FramePlanInfo]) {
        let ending = window.len() < self.longest_window;
        self.longest_window = self.longest_window.max(window.len());
        let frames = i64::try_from(window.len()).unwrap_or(i64::MAX);
        let target = (self.frame_bits as i64).saturating_mul(frames);
        let horizon = if ending {
            frames
        } else {
            frames.max(self.horizon)
        };
        let repaid = i128::from(self.drift) * i128::from(frames) / i128::from(horizon.max(1));
        let budget = (target - repaid as i64).clamp(target / 4, target.saturating_mul(2));
        let predicted = |base: u16| {
            window.iter().fold(0u64, |sum, info| {
                let qindex = with_offset(base, info.qindex_offset, info.bit_depth);
                sum.saturating_add(self.predict(info, qindex))
            })
        };
        let max = window
            .first()
            .map_or(u16::MAX, |info| max_qindex(info.bit_depth));
        let b = (1..=max)
            .find(|&base| predicted(base) <= budget as u64)
            .unwrap_or(max);
        self.base = b;
    }

    fn frame_qp(&mut self, info: &FramePlanInfo) -> u16 {
        if self.planned.len() >= 64 {
            self.planned.pop_front();
        }
        let flat = is_flat(info);
        self.planned.push_back((
            info.frame_index,
            class(info),
            if flat { 0 } else { info.complexity },
            info.bit_depth,
        ));
        with_offset(self.base, info.qindex_offset, info.bit_depth)
    }

    /// Adds the frame to the drift, and learns its class's `k`, or the mean
    /// size of the class's flat frames for a flat frame.
    fn update(&mut self, stats: &FrameStats) {
        self.spend(stats.bits, self.frame_bits);
        let Some(&(_, class, complexity, bit_depth)) = self
            .planned
            .iter()
            .find(|planned| planned.0 == stats.frame_index)
        else {
            return;
        };
        if complexity == 0 {
            let mean = self.flat_bits[class].map_or(stats.bits, |bits| {
                (bits.saturating_mul(3).saturating_add(stats.bits)) / 4
            });
            self.flat_bits[class] = Some(mean);
            return;
        }
        let observed = ((u128::from(stats.bits) * u128::from(step(stats.qp, bit_depth))) << 8)
            / u128::from(complexity.max(1));
        let model = u128::from(self.model_q8[class]);
        let blended = if self.learnt[class] {
            (3 * model + observed) / 4
        } else {
            observed
        };
        self.model_q8[class] = u64::try_from(blended).unwrap_or(u64::MAX);
        self.learnt[class] = true;
    }

    fn update_show_existing(&mut self, bits: u64) {
        self.spend(bits, 0);
    }
}

/// Constant bitrate: every window meets the target, and no frame overflows a
/// buffer that starts half full. The drift is the buffer level minus half the
/// buffer, so it stays within half the buffer either way.
#[derive(Debug, Clone)]
pub struct Cbr {
    vbr: Vbr,
    buffer_bits: u64,
    /// The predicted drift of the frames of this wave that have a qindex.
    pending: i64,
}

impl Cbr {
    /// A CBR controller for `bitrate_kbps` with a buffer of `buffer_ms` at
    /// that bitrate, at `frame_rate` frames per second.
    ///
    /// # Errors
    /// Returns [`Error::InvalidBitrate`] for a bitrate outside
    /// `1..=`[`MAX_BITRATE_KBPS`] or a buffer outside `1..=`[`MAX_BUFFER_MS`],
    /// and [`Error::InvalidFrameRate`] for a zero frame-rate term.
    pub fn new(bitrate_kbps: u32, buffer_ms: u32, frame_rate: (u32, u32)) -> Result<Self> {
        check_bitrate(bitrate_kbps, Some(buffer_ms), frame_rate)?;
        let buffer_bits = u64::from(bitrate_kbps) * u64::from(buffer_ms);
        let frame_bits = frame_bits(bitrate_kbps, frame_rate);
        Ok(Self {
            vbr: Vbr::with_limits(
                frame_bits,
                1,
                (buffer_bits as i64 / 2, buffer_bits as i64 / 2),
            ),
            buffer_bits,
            pending: 0,
        })
    }
}

impl RateController for Cbr {
    fn plan_window(&mut self, window: &[FramePlanInfo]) {
        self.vbr.plan_window(window);
    }

    /// The VBR qindex, raised until the predicted frame fits the buffer and
    /// the room left after one frame budget drains.
    fn frame_qp(&mut self, info: &FramePlanInfo) -> u16 {
        let mut qindex = self.vbr.frame_qp(info);
        let level = (self.buffer_bits as i64 / 2)
            .saturating_add(self.vbr.drift)
            .saturating_add(self.pending)
            .max(0) as u64;
        let room = (self.buffer_bits + self.vbr.frame_bits)
            .saturating_sub(level)
            .min(self.buffer_bits);
        while qindex < max_qindex(info.bit_depth) && self.vbr.predict(info, qindex) > room {
            qindex += 1;
        }
        let predicted = i64::try_from(self.vbr.predict(info, qindex)).unwrap_or(i64::MAX);
        self.pending = self
            .pending
            .saturating_add(predicted.saturating_sub(self.vbr.frame_bits as i64));
        qindex
    }

    fn update(&mut self, stats: &FrameStats) {
        self.pending = 0;
        self.vbr.update(stats);
    }

    fn update_show_existing(&mut self, bits: u64) {
        self.vbr.update_show_existing(bits);
    }
}

#[cfg(test)]
#[path = "rate_control_tests.rs"]
mod tests;
