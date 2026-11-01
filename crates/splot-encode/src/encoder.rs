// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The push/pull [`Encoder`]: frames go in with [`Encoder::send_frame`], and
//! [`Encoder::receive_packet`] returns one temporal unit per frame, in coding
//! order, which outputs the frames in send order.

use std::collections::VecDeque;
use std::fmt;

use splot_core::headers::frame::FrameHeaderCore;
use splot_decode::encoder_syntax::{FrameStart, TileEnd};
use splot_recon::DecodedFrame;

use crate::config::{Config, RateControlMode};
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::gop::{self, GopFrame, GopPlanner, GopStep};
use crate::intra_frame::IntraFrameCoder;
use crate::lookahead::Lookahead;
use crate::preset::{FrameKind, PresetContext, ResolutionClass};
use crate::rate_control::{
    Cbr, ConstantQp, Crf, FramePlanInfo, FrameType, RateController, Vbr, max_qindex,
};
use crate::stats::{FrameStats, psnr, ssim};
use crate::tpl;

/// One AV2 temporal unit.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Packet {
    /// One AV2 Annex B temporal unit: the hidden frames coded since the last
    /// packet and one output, a shown frame or a show-existing frame.
    pub data: Vec<u8>,
    /// Presentation timestamp of the output frame.
    pub pts: u64,
    /// Frame type of the output frame.
    pub frame_type: FrameType,
    /// Statistics of each frame coded in this temporal unit, in coding order;
    /// empty when the unit only shows an earlier hidden frame.
    pub stats: Vec<FrameStats>,
}

/// The output of a [`FrameCoder`] for one frame.
pub(crate) struct CodedFrame {
    /// The frame's OBUs, after a temporal delimiter when the plan opens a
    /// temporal unit; a key frame also repeats the sequence header.
    pub(crate) temporal_unit: Vec<u8>,
    /// Reconstructed Y, U and V planes, packed like the source planes.
    pub(crate) recon: [Vec<u16>; 3],
    /// The frame type the coder used, which can differ from the planned one.
    pub(crate) frame_type: FrameType,
    /// The header, filtered reconstruction, initial CDFs and final tile CDFs
    /// that [`FrameCoder::commit`] stores.
    pub(crate) reference: Option<(FrameHeaderCore, DecodedFrame<u16>, FrameStart, Vec<TileEnd>)>,
    /// The header of a `TIP_FRAME_AS_OUTPUT` frame, which
    /// [`FrameCoder::commit`] records in place of `reference`.
    pub(crate) tip_output: Option<FrameHeaderCore>,
    /// How the inter frame used warped motion; `None` for key frames and for
    /// `TIP_FRAME_AS_OUTPUT` frames, which code no leaves.
    pub(crate) warp: Option<crate::warp::WarpUse>,
    /// The 8x8 cells the intra leaves of the coded trees cover, which later
    /// frames read from their primary reference; `None` without leaves.
    pub(crate) intra_cells: Option<std::sync::Arc<crate::intra_frame::IntraCells>>,
    /// The `GLOBALMV` and `GLOBAL_GLOBALMV` leaves of the coded trees.
    #[cfg(test)]
    pub(crate) global_leaves: usize,
}

/// Codes the frames and show-existing frames of the plan, in coding order.
pub(crate) trait FrameCoder {
    /// Codes each `(source, plan, qp)` of one wave against the committed
    /// references; the frames of a wave can code at the same time.
    fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>>;
    /// Stores a frame of [`Self::encode_wave`] in the references, in coding order.
    fn commit(&mut self, frame: &mut CodedFrame) -> Result<()>;
    /// A temporal delimiter and the show-existing frame that outputs the
    /// hidden frame in `slot`.
    fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>>;
    /// Fits the sequence to `first`, the first frame in display order,
    /// before any frame is coded.
    fn start_sequence(&mut self, _first: &SourceFrame) -> Result<()> {
        Ok(())
    }
    /// Receives the block importances of the next frames to code, which
    /// replace the previous ones.
    fn set_block_importance(&mut self, _importance: tpl::BlockImportance) {}
    /// Whether the coder codes global motion models.
    fn wants_global_motion(&self) -> bool {
        false
    }
    /// Receives the global motion models of the next frames to code, which
    /// replace the previous ones.
    fn set_global_motion(&mut self, _models: crate::global_motion::GlobalModels) {}
    /// Receives the open-loop motion of the next frames to code, which
    /// replaces the previous one.
    fn set_open_loop_motion(&mut self, _motion: crate::lookahead::OpenLoopMotion) {}
    /// The worker pool the coder runs on, which the lookahead shares.
    fn pool(&self) -> Option<splot_parallel::WorkerPool> {
        None
    }
}

/// Callback that receives the statistics of every coded frame.
pub type StatsSink = Box<dyn FnMut(&FrameStats) + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Accepting,
    Flushed,
    Failed,
}

/// The AV2 encoder. Output depends only on the input frames and the [`Config`];
/// [`Config::threads`] never changes it.
pub struct Encoder {
    config: Config,
    /// [`Config::coded`] when the configuration resizes its frames.
    resized: Option<Config>,
    coder: Box<dyn FrameCoder + Send>,
    rate_control: Box<dyn RateController + Send>,
    stats_sink: Option<StatsSink>,
    pending: VecDeque<SourceFrame>,
    /// The last frame of the last coded mini-GOP, the past reference that the
    /// lookahead's motion search of the next window reads.
    last_coded: Option<SourceFrame>,
    planner: GopPlanner,
    lookahead: Lookahead,
    lookahead_frames: usize,
    /// The planned frames of the current lookahead window.
    window: Vec<FramePlanInfo>,
    /// The preset's `tpl_delta_q` strength in percent.
    tpl_delta_q: u8,
    /// The preset's temporal filter frames and strengths of key and base frames.
    temporal_filter: [(u8, u8); 2],
    /// The last frame qindex, which estimates the next ones.
    last_qindex: Option<u16>,
    ready: VecDeque<Packet>,
    state: State,
}

impl fmt::Debug for Encoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Encoder")
            .field("config", &self.config)
            .field("pending", &self.pending.len())
            .field("planner", &self.planner)
            .field("ready", &self.ready.len())
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Encoder {
    /// Creates an encoder with the rate controller of `config.rate_control`.
    ///
    /// # Errors
    /// Returns a configuration error from [`Config::validate`], or a coding
    /// error when the frame coder cannot be set up for `config`.
    pub fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let rate_control = rate_controller(&config)?;
        Self::with_rate_controller(config, rate_control)
    }

    /// Creates an encoder whose quantizers come from `rate_control` instead
    /// of the controller that `config.rate_control` names. [`RateController`]
    /// says when the encoder calls it; the encoder clamps its qindex to
    /// `1..=`[`max_qindex`] of the bit depth.
    ///
    /// # Errors
    /// Returns a configuration error from [`Config::validate`], or a coding
    /// error when the frame coder cannot be set up for `config`.
    pub fn with_rate_controller(
        config: Config,
        rate_control: Box<dyn RateController + Send>,
    ) -> Result<Self> {
        config.validate()?;
        let coder = frame_coder(&config.coded())?;
        Self::with_parts(config, coder, rate_control)
    }

    #[cfg(test)]
    pub(crate) fn with_coder(config: Config, coder: Box<dyn FrameCoder + Send>) -> Result<Self> {
        let rate_control = rate_controller(&config)?;
        Self::with_parts(config, coder, rate_control)
    }

    fn with_parts(
        config: Config,
        coder: Box<dyn FrameCoder + Send>,
        rate_control: Box<dyn RateController + Send>,
    ) -> Result<Self> {
        let params = config.override_preset(config.preset.params())?;
        let mini_gop_size = match config.rate_control {
            RateControlMode::Vbr { .. } | RateControlMode::Cbr { .. } => {
                params.bitrate_mini_gop_size
            }
            _ => params.mini_gop_size,
        };
        let planner = GopPlanner::new(config.keyint, mini_gop_size);
        Ok(Self {
            rate_control,
            stats_sink: None,
            pending: VecDeque::new(),
            last_coded: None,
            planner,
            lookahead: Lookahead::new(&config.coded(), &params, coder.pool()),
            lookahead_frames: usize::from(params.lookahead_frames),
            window: Vec::new(),
            tpl_delta_q: params.tpl_delta_q,
            temporal_filter: [FrameKind::Key, FrameKind::Base].map(|frame| {
                let coded = config.coded();
                let context = PresetContext {
                    resolution: ResolutionClass::of(coded.width, coded.height),
                    frame,
                    ..PresetContext::default()
                };
                (params.tf_frames.at(context), params.tf_strength.at(context))
            }),
            last_qindex: None,
            resized: config.resize_denominator.map(|_| config.coded()),
            config,
            coder,
            ready: VecDeque::new(),
            state: State::Accepting,
        })
    }

    /// Returns the configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Calls `sink` with the statistics of every frame coded from now on.
    pub fn set_stats_sink(&mut self, sink: StatsSink) {
        self.stats_sink = Some(sink);
    }

    /// Queues a frame for coding and analyses it in the lookahead; a scene cut
    /// makes it a key frame. A frame of the input size is resized to the
    /// [`Config::coded`] size first. The encoder keeps every queued frame until
    /// [`Self::receive_packet`] codes it.
    ///
    /// # Errors
    /// Returns [`Error::SendAfterFlush`] after [`Self::flush`],
    /// [`Error::Failed`] after an earlier coding error, or
    /// [`Error::FrameConfigMismatch`] when the frame was built for another
    /// configuration.
    pub fn send_frame(&mut self, frame: SourceFrame) -> Result<()> {
        match self.state {
            State::Accepting => {}
            State::Flushed => return Err(Error::SendAfterFlush),
            State::Failed => return Err(Error::Failed),
        }
        if frame.bit_depth() != self.config.bit_depth
            || frame.plane_sizes() != self.config.plane_sizes()?.as_slice()
        {
            return Err(Error::FrameConfigMismatch);
        }
        let frame = match &self.resized {
            Some(coded) => frame.resized(coded)?,
            None => frame,
        };
        let display_index = self.planner.next_display() + self.pending.len() as u64;
        if self.lookahead.push(&frame, display_index) {
            self.planner.force_key(display_index);
        }
        self.pending.push_back(frame);
        Ok(())
    }

    /// Signals the end of input. Queued frames are still coded.
    ///
    /// # Errors
    /// Returns [`Error::Failed`] after an earlier coding error.
    pub fn flush(&mut self) -> Result<()> {
        if self.state == State::Failed {
            return Err(Error::Failed);
        }
        self.state = State::Flushed;
        Ok(())
    }

    /// Returns the next temporal unit. A mini-GOP is coded once the preset's
    /// `lookahead_frames` frames are queued, or after [`Self::flush`]. Returns
    /// `None` when no unit is ready: send more frames, or after
    /// [`Self::flush`] the stream is complete.
    ///
    /// # Errors
    /// Returns the coding error, after which the encoder stays failed, or
    /// [`Error::Failed`] after an earlier coding error.
    pub fn receive_packet(&mut self) -> Result<Option<Packet>> {
        if self.state == State::Failed {
            return Err(Error::Failed);
        }
        let ended = self.state == State::Flushed;
        let queued = self.pending.len();
        if self.ready.is_empty()
            && (ended || queued >= self.lookahead_frames)
            && let Some(len) = self.planner.next_len(queued, ended)
            && let Err(error) = self.code_mini_gop(len)
        {
            self.state = State::Failed;
            return Err(error);
        }
        Ok(self.ready.pop_front())
    }

    /// Plans the first `lookahead_frames` queued frames, runs the temporal
    /// model over the plan, and hands the window to the rate controller. Coding
    /// starts only with a full window or after the flush, so the window does not
    /// depend on how the caller interleaves sends and receives.
    fn plan_window(&mut self) -> Result<()> {
        let mut planner = self.planner.clone();
        let mut remaining = self.pending.len().min(self.lookahead_frames);
        let mut plan = Vec::new();
        let mut first_step = None;
        while let Some(len) = planner.next_len(remaining, true) {
            plan.extend(
                planner
                    .plan(len)?
                    .into_iter()
                    .filter_map(|step| match step {
                        GopStep::Code(frame) => Some(frame),
                        GopStep::ShowExisting { .. } => None,
                    }),
            );
            remaining -= len;
            first_step.get_or_insert(plan.len());
        }
        let coded_before = self.planner.next_display().checked_sub(1);
        let blocks = (self.tpl_delta_q > 0).then(|| tpl::BlockModel {
            frames: first_step.unwrap_or(0) + 1,
            noise: tpl::recon_noise(
                self.qindex_estimate(),
                self.config.bit_depth,
                self.lookahead.block_luma(),
            ),
        });
        let next = self.planner.next_display();
        let (pending, last_coded) = (&self.pending, &self.last_coded);
        let sources = |display: u64| match display.checked_sub(next) {
            Some(index) => pending.get(usize::try_from(index).ok()?),
            None => last_coded.as_ref().filter(|_| display + 1 == next),
        };
        let model = tpl::analyze(&mut self.lookahead, &plan, coded_before, blocks, &sources);
        if let Some(fields) = self.lookahead.fields() {
            self.coder.set_open_loop_motion(fields);
        }
        if self.coder.wants_global_motion() {
            let models =
                crate::global_motion::estimate(&mut self.lookahead, &plan, coded_before, &sources);
            self.coder.set_global_motion(models);
        }
        let mut costs = std::collections::BTreeMap::new();
        self.window = plan
            .iter()
            .zip(model)
            .map(|(frame, model)| {
                if !model.block_costs.is_empty() {
                    costs.insert(frame.display_index, model.block_costs);
                }
                FramePlanInfo {
                    frame_index: frame.frame_index,
                    display_index: frame.display_index,
                    frame_type: frame.frame_type,
                    layer: frame.layer,
                    complexity: model.complexity,
                    min_complexity: model.min_complexity,
                    qindex_offset: model.qindex_offset,
                    bit_depth: self.config.bit_depth,
                }
            })
            .collect();
        if blocks.is_some() {
            let (cols, rows) = self.lookahead.grid();
            self.coder.set_block_importance(tpl::BlockImportance {
                grid: (cols, rows, self.lookahead.block_luma()),
                strength: self.tpl_delta_q,
                frames: costs,
            });
        }
        self.rate_control.plan_window(&self.window);
        Ok(())
    }

    /// Codes the next `len` queued frames, the next frames of the plan, into
    /// ready packets.
    fn code_mini_gop(&mut self, len: usize) -> Result<()> {
        self.plan_window()?;
        let frames: Vec<SourceFrame> = self.pending.drain(..len).collect();
        let first = self.planner.next_display();
        if first == 0
            && let Some(frame) = frames.first()
        {
            self.coder.start_sequence(frame)?;
        }
        let source = |display_index: u64| {
            usize::try_from(display_index - first)
                .ok()
                .and_then(|index| frames.get(index))
                .ok_or(Error::Unsupported {
                    what: "a plan outside its mini-GOP",
                })
        };
        let mut frame_types = vec![FrameType::Inter; frames.len()];
        let (mut data, mut stats) = (Vec::new(), Vec::new());
        let steps = self.planner.plan(frames.len())?;
        for wave in gop::waves(&steps) {
            let mut planned = Vec::new();
            for step in wave {
                if let GopStep::Code(plan) = *step {
                    planned.push((plan, self.frame_qp(&plan)?));
                }
            }
            let filtered = self.temporal_filter(&planned, &frames, first);
            let mut jobs = Vec::new();
            for (&(plan, qp), filtered) in planned.iter().zip(&filtered) {
                let frame = match filtered {
                    Some(filtered) => filtered,
                    None => source(plan.display_index)?,
                };
                jobs.push((frame, plan, qp));
            }
            let mut coded = jobs.iter().zip(self.coder.encode_wave(&jobs)?);
            for step in wave {
                match *step {
                    GopStep::Code(_) => {
                        let (&(_, plan, qp), mut coded) =
                            coded.next().ok_or(Error::Unsupported {
                                what: "a wave with fewer coded frames than planned",
                            })?;
                        let frame = source(plan.display_index)?;
                        stats.push(self.commit(frame, &plan, qp, &mut coded)?);
                        frame_types[(plan.display_index - first) as usize] = coded.frame_type;
                        data.extend(coded.temporal_unit);
                    }
                    GopStep::ShowExisting { slot, .. } => {
                        let unit = self.coder.show_existing_frame(slot)?;
                        self.rate_control
                            .update_show_existing(unit.len() as u64 * 8);
                        data.extend(unit);
                    }
                }
                if let Some(display_index) = step.output() {
                    self.ready.push_back(Packet {
                        data: core::mem::take(&mut data),
                        pts: source(display_index)?.pts(),
                        frame_type: frame_types[(display_index - first) as usize],
                        stats: core::mem::take(&mut stats),
                    });
                }
            }
        }
        self.lookahead
            .retain_from(self.planner.next_display().saturating_sub(1));
        self.last_coded = frames.into_iter().last();
        Ok(())
    }

    /// The temporally filtered source of each `(plan, qp)` of a wave of the
    /// mini-GOP `frames`, whose first display index is `first`. The filter
    /// reads the frames of the lookahead window before the next planned key
    /// frame, so the output does not depend on how the caller interleaves
    /// sends and receives, nor does a frame read across a scene cut.
    fn temporal_filter(
        &self,
        planned: &[(GopFrame, u16)],
        frames: &[SourceFrame],
        first: u64,
    ) -> Vec<Option<SourceFrame>> {
        let (pending, last_coded) = (&self.pending, &self.last_coded);
        let end = (first + self.lookahead_frames as u64).min(self.planner.next_key());
        let sources = |display: u64| match display.checked_sub(first) {
            Some(index) if display < end => {
                let index = usize::try_from(index).ok()?;
                frames
                    .get(index)
                    .or_else(|| pending.get(index.checked_sub(frames.len())?))
            }
            Some(_) => None,
            None => last_coded.as_ref().filter(|_| display + 1 == first),
        };
        let subsampling = [
            self.config.chroma.subsampling_x(),
            self.config.chroma.subsampling_y(),
        ];
        let pool = self.coder.pool();
        planned
            .iter()
            .map(|(plan, qp)| {
                crate::temporal_filter::filter_frame(
                    &self.lookahead,
                    &sources,
                    plan,
                    *qp,
                    self.temporal_filter,
                    subsampling,
                    pool.as_ref(),
                )
            })
            .collect()
    }

    fn frame_qp(&mut self, gop_frame: &GopFrame) -> Result<u16> {
        let plan = self
            .window
            .iter()
            .find(|info| info.display_index == gop_frame.display_index)
            .copied()
            .ok_or(Error::Unsupported {
                what: "a frame outside its lookahead window",
            })?;
        let qindex = self
            .rate_control
            .frame_qp(&plan)
            .clamp(1, max_qindex(self.config.bit_depth));
        self.last_qindex = Some(qindex);
        Ok(qindex)
    }

    /// The last frame qindex, or before the first frame the base qindex of
    /// a constant-qp or constant-quality configuration (a mid qindex else).
    fn qindex_estimate(&self) -> u16 {
        self.last_qindex.unwrap_or(match self.config.rate_control {
            RateControlMode::ConstantQp { qp } => qp,
            RateControlMode::Crf { crf } => Crf { crf }.base_qindex(self.config.bit_depth),
            _ => max_qindex(self.config.bit_depth) / 2,
        })
    }

    /// Commits `coded`, the frame `frame` coded as `gop_frame` at `qp`, and
    /// reports its statistics to the rate controller and the stats sink.
    fn commit(
        &mut self,
        frame: &SourceFrame,
        gop_frame: &GopFrame,
        qp: u16,
        coded: &mut CodedFrame,
    ) -> Result<FrameStats> {
        self.coder.commit(coded)?;
        let bit_depth = frame.bit_depth();
        let [source_y, source_u, source_v] = frame.planes();
        let [recon_y, recon_u, recon_v] = &coded.recon;
        if [recon_y.len(), recon_u.len(), recon_v.len()]
            != [source_y.len(), source_u.len(), source_v.len()]
        {
            return Err(Error::ReconSizeMismatch);
        }
        let luma_width = frame.plane_sizes()[0].width();
        let stats = FrameStats {
            frame_index: gop_frame.frame_index,
            pts: frame.pts(),
            qp,
            bits: coded.temporal_unit.len() as u64 * 8,
            psnr_y: psnr(source_y, recon_y, bit_depth),
            psnr_u: psnr(source_u, recon_u, bit_depth),
            psnr_v: psnr(source_v, recon_v, bit_depth),
            ssim_y: Some(ssim(source_y, recon_y, luma_width, bit_depth)),
        };
        self.rate_control.update(&stats);
        if let Some(sink) = &mut self.stats_sink {
            sink(&stats);
        }
        Ok(stats)
    }
}

fn rate_controller(config: &Config) -> Result<Box<dyn RateController + Send>> {
    Ok(match config.rate_control {
        RateControlMode::ConstantQp { qp } => Box::new(ConstantQp { qp }),
        RateControlMode::Crf { crf } => Box::new(Crf { crf }),
        RateControlMode::Vbr { bitrate_kbps } => {
            Box::new(Vbr::new(bitrate_kbps, config.frame_rate)?.with_first_pass(&config.first_pass))
        }
        RateControlMode::Cbr {
            bitrate_kbps,
            buffer_ms,
            initial_ms,
        } => Box::new(Cbr::new(
            bitrate_kbps,
            (buffer_ms, initial_ms),
            config.frame_rate,
        )?),
    })
}

fn frame_coder(config: &Config) -> Result<Box<dyn FrameCoder + Send>> {
    Ok(Box::new(IntraFrameCoder::new(config)?))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::config::TileLayout;
    use crate::preset::{Preset, PresetError};

    /// Emits the frame index and qp as the frame's OBUs (after `0xd0` when
    /// the frame opens a unit) and a recon that is the source plus one; fails
    /// on the frame index it was told to.
    struct FakeCoder {
        fail_at: Option<u64>,
    }

    impl FrameCoder for FakeCoder {
        fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>> {
            wave.iter()
                .map(|&(source, plan, qp)| self.encode_frame(source, &plan, qp))
                .collect()
        }

        fn commit(&mut self, _frame: &mut CodedFrame) -> Result<()> {
            Ok(())
        }

        fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>> {
            Ok(vec![0xd0, 0x5e, slot as u8])
        }
    }

    impl FakeCoder {
        fn encode_frame(
            &self,
            source: &SourceFrame,
            plan: &GopFrame,
            qp: u16,
        ) -> Result<CodedFrame> {
            if self.fail_at == Some(plan.frame_index) {
                return Err(Error::Unsupported { what: "test" });
            }
            let recon = source
                .planes()
                .clone()
                .map(|plane| plane.into_iter().map(|s| s + 1).collect());
            let mut temporal_unit = if plan.opens_temporal_unit {
                vec![0xd0]
            } else {
                Vec::new()
            };
            temporal_unit.extend([plan.frame_index as u8, qp as u8]);
            Ok(CodedFrame {
                temporal_unit,
                recon,
                frame_type: plan.frame_type,
                reference: None,
                tip_output: None,
                warp: None,
                intra_cells: None,
                global_leaves: 0,
            })
        }
    }

    fn encoder(fail_at: Option<u64>, preset: u8) -> Result<Encoder> {
        let mut config = Config::new(4, 4);
        config.keyint = core::num::NonZeroU32::MIN.saturating_add(1);
        config.rate_control = RateControlMode::ConstantQp { qp: 40 };
        config.preset = Preset::new(preset).unwrap_or_default();
        Encoder::with_coder(config, Box::new(FakeCoder { fail_at }))
    }

    fn frame(encoder: &Encoder, pts: u64) -> Result<SourceFrame> {
        SourceFrame::new(
            encoder.config(),
            pts,
            [vec![10; 16], vec![20; 4], vec![30; 4]],
        )
    }

    fn drain(encoder: &mut Encoder) -> Result<Vec<Packet>> {
        let mut packets = Vec::new();
        while let Some(packet) = encoder.receive_packet()? {
            packets.push(packet);
        }
        Ok(packets)
    }

    #[test]
    fn encodes_flat_frames_through_the_public_api() -> Result<()> {
        assert!(matches!(
            Encoder::new(Config::new(0, 64)),
            Err(Error::InvalidDimensions { .. })
        ));
        let config = Config::new(40, 24);
        let mut encoder = Encoder::new(config.clone())?;
        for pts in 0..2 {
            let planes = [vec![90; 40 * 24], vec![128; 20 * 12], vec![160; 20 * 12]];
            encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
        }
        encoder.flush()?;
        let packets = drain(&mut encoder)?;
        assert_eq!(packets.len(), 2);
        for (pts, packet) in packets.iter().enumerate() {
            assert_eq!(packet.pts, pts as u64);
            assert!(packet.stats[0].psnr_y > 40.0, "{:?}", packet.stats);
        }
        let types: Vec<_> = packets.iter().map(|packet| packet.frame_type).collect();
        assert_eq!(types, [FrameType::Key, FrameType::Inter]);
        Ok(())
    }

    /// The low-delay overrides: every frame is coded and shown as soon as it
    /// is sent, in display order.
    #[test]
    fn mini_gop_and_lookahead_of_one_emit_each_frame_at_once() -> Result<()> {
        let mut config = encoder(None, 6)?.config().clone();
        config.keyint = Config::DEFAULT_KEYINT;
        config.preset_overrides = vec![
            ("mini_gop_size".into(), "1".into()),
            ("bitrate_mini_gop_size".into(), "1".into()),
            ("lookahead_frames".into(), "1".into()),
        ];
        let mut encoder = Encoder::with_coder(config, Box::new(FakeCoder { fail_at: None }))?;
        for pts in 0..6 {
            encoder.send_frame(frame(&encoder, pts)?)?;
            let packet = encoder.receive_packet()?.ok_or(Error::Failed)?;
            assert_eq!((packet.pts, packet.data[1]), (pts, pts as u8));
            assert!(encoder.receive_packet()?.is_none());
        }
        Ok(())
    }

    /// A resized stream takes input-size frames, codes them at 8/16 of the
    /// size with every loop filter, and decodes to the coded size; the stats
    /// compare the recon with the resized source.
    #[test]
    fn resized_streams_code_input_frames_at_the_scaled_size() -> Result<()> {
        let mut config = Config::new(67, 48);
        config.resize_denominator = Some(16);
        config.keyint = core::num::NonZeroU32::MIN.saturating_add(1);
        let coded = config.coded();
        assert_eq!((coded.width, coded.height), (34, 24));
        let mut encoder = Encoder::new(config.clone())?;
        let small = SourceFrame::new(
            &coded,
            0,
            [vec![0; 34 * 24], vec![0; 17 * 12], vec![0; 17 * 12]],
        )?;
        assert!(matches!(
            encoder.send_frame(small),
            Err(Error::FrameConfigMismatch)
        ));
        let sizes = config.plane_sizes()?;
        for pts in 0..3u16 {
            let planes = core::array::from_fn(|plane| {
                let (w, h) = (sizes[plane].width(), sizes[plane].height());
                (0..w * h)
                    .map(|i| (60 + i % w + (i / w) * 2 + usize::from(pts) * 5) as u16)
                    .collect()
            });
            encoder.send_frame(SourceFrame::new(&config, u64::from(pts), planes)?)?;
        }
        encoder.flush()?;
        let packets = drain(&mut encoder)?;
        let stream: Vec<u8> = packets.iter().flat_map(|p| p.data.clone()).collect();
        assert_eq!(
            crate::test_support::decode(&stream).len(),
            3 * (34 * 24 + 2 * 17 * 12)
        );
        assert!(
            packets.iter().all(|p| p.stats[0].psnr_y > 35.0),
            "{packets:?}"
        );
        Ok(())
    }

    /// The stream of three flat 40x24 frames at preset 6 with `overrides`.
    fn stream_with(overrides: &[(&str, &str)]) -> Result<Vec<u8>> {
        let mut config = Config::new(40, 24);
        config.preset_overrides = overrides
            .iter()
            .map(|&(field, value)| (field.into(), value.into()))
            .collect();
        let mut encoder = Encoder::new(config.clone())?;
        for pts in 0..3 {
            let planes = [
                vec![90 + pts as u16; 40 * 24],
                vec![128; 20 * 12],
                vec![160; 20 * 12],
            ];
            encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
        }
        encoder.flush()?;
        Ok(drain(&mut encoder)?
            .into_iter()
            .flat_map(|packet| packet.data)
            .collect())
    }

    #[test]
    fn preset_overrides_change_the_stream() -> Result<()> {
        let plain = stream_with(&[])?;
        assert_eq!(
            stream_with(&[("max_block_log2", "7"), ("mini_gop_size", "4")])?,
            plain
        );
        assert_ne!(stream_with(&[("max_block_log2", "6")])?, plain);
        let ippp = [("mini_gop_size", "1"), ("bitrate_mini_gop_size", "1")];
        assert_ne!(stream_with(&ippp)?, plain);
        assert!(matches!(
            stream_with(&[("mini_gop", "1")]),
            Err(Error::Preset(PresetError::UnknownField { .. }))
        ));
        assert!(matches!(
            stream_with(&[("mini_gop_size", "x")]),
            Err(Error::Preset(PresetError::InvalidValue { .. }))
        ));
        Ok(())
    }

    #[test]
    fn codes_frames_in_order_with_stats() -> Result<()> {
        let mut encoder = encoder(None, Preset::MAX)?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = Arc::clone(&seen);
        encoder.set_stats_sink(Box::new(move |stats| {
            if let Ok(mut seen) = sink_seen.lock() {
                seen.push(stats.frame_index);
            }
        }));
        assert_eq!(encoder.receive_packet()?, None);
        for pts in [5, 6, 7] {
            let frame = frame(&encoder, pts)?;
            encoder.send_frame(frame)?;
        }
        encoder.flush()?;
        let packets = drain(&mut encoder)?;
        let types: Vec<_> = packets.iter().map(|p| (p.pts, p.frame_type)).collect();
        assert_eq!(
            types,
            [
                (5, FrameType::Key),
                (6, FrameType::Inter),
                (7, FrameType::Key)
            ]
        );
        let stats = packets[1].stats[0];
        assert_eq!((stats.frame_index, stats.pts), (1, 6));
        assert_eq!((stats.qp, stats.bits), (40, 24));
        assert!((stats.psnr_y - 48.130_803_608).abs() < 1e-6);
        assert_eq!(packets[2].data, [0xd0, 2, 40]);
        assert_eq!(seen.lock().map(|s| s.clone()).ok(), Some(vec![0, 1, 2]));
        Ok(())
    }

    /// A mini-GOP of 4 waits for its frames and packs hidden frames with the
    /// next output: coding order 4 2 1 [2] 3 [4], with 4 and 2 hidden in slots 1 and 2.
    #[test]
    fn packs_hidden_frames_into_the_next_output_unit() -> Result<()> {
        let mut encoder = encoder(None, 6)?;
        encoder.config.keyint = core::num::NonZeroU32::new(10).unwrap_or(encoder.config.keyint);
        encoder.planner = GopPlanner::new(encoder.config.keyint, 4);
        encoder.lookahead_frames = 4;
        for pts in 0..4 {
            let frame = frame(&encoder, pts)?;
            encoder.send_frame(frame)?;
        }
        let key = drain(&mut encoder)?;
        assert_eq!(key.len(), 1, "the mini-GOP after the key frame waits");
        let frame = frame(&encoder, 4)?;
        encoder.send_frame(frame)?;
        let packets = drain(&mut encoder)?;
        let data: Vec<_> = packets.iter().map(|p| p.data.clone()).collect();
        assert_eq!(
            data,
            [
                vec![0xd0, 1, 40, 2, 40, 3, 40],
                vec![0xd0, 0x5e, 2],
                vec![0xd0, 4, 40],
                vec![0xd0, 0x5e, 1],
            ]
        );
        let pts: Vec<_> = packets.iter().map(|p| p.pts).collect();
        assert_eq!(pts, [1, 2, 3, 4]);
        let stats: Vec<Vec<u64>> = packets
            .iter()
            .map(|p| p.stats.iter().map(|s| s.pts).collect())
            .collect();
        assert_eq!(stats, [vec![4, 2, 1], vec![], vec![3], vec![]]);
        Ok(())
    }

    /// Codes `frames` frames of a 64x64 texture panning by `shift` samples a
    /// frame, with a new texture from frame `cut`, through the fake coder.
    fn textured(rate_control: RateControlMode, shift: usize, cut: u64) -> Result<Vec<Packet>> {
        let mut config = Config::new(64, 64);
        config.rate_control = rate_control;
        let mut encoder =
            Encoder::with_coder(config.clone(), Box::new(FakeCoder { fail_at: None }))?;
        for pts in 0..12 {
            let scene = u32::from(pts >= cut);
            let frame = crate::lookahead::tests::frame(&config, pts, shift * pts as usize, scene);
            encoder.send_frame(frame)?;
        }
        encoder.flush()?;
        drain(&mut encoder)
    }

    #[test]
    fn a_scene_cut_starts_a_key_frame() -> Result<()> {
        let packets = textured(RateControlMode::ConstantQp { qp: 40 }, 2, 9)?;
        let keys: Vec<u64> = packets
            .iter()
            .filter(|packet| packet.frame_type == FrameType::Key)
            .map(|packet| packet.pts)
            .collect();
        assert_eq!(keys, [0, 9]);
        Ok(())
    }

    /// Fresh noise in every frame predicts badly from the frame before, like
    /// a cut, but no worse than the frame before did: only frame 0 is a key.
    #[test]
    fn noise_makes_no_scene_cuts() -> Result<()> {
        let config = Config::new(64, 64);
        let mut encoder =
            Encoder::with_coder(config.clone(), Box::new(FakeCoder { fail_at: None }))?;
        let mut random = xorshift(5);
        for pts in 0..12 {
            let sizes = config.plane_sizes()?;
            let planes = core::array::from_fn(|plane| {
                (0..sizes[plane].width() * sizes[plane].height())
                    .map(|_| 16 + 37 * random() as u16)
                    .collect()
            });
            encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
        }
        encoder.flush()?;
        let keys = drain(&mut encoder)?
            .iter()
            .filter(|packet| packet.frame_type == FrameType::Key)
            .count();
        assert_eq!(keys, 1);
        Ok(())
    }

    /// On static content the alt-ref of each mini-GOP of 8 gets a finer
    /// quantizer than its middle frame, that one than the quarter frames, and
    /// those than the leaves.
    #[test]
    fn crf_gives_referenced_frames_lower_qindex_on_static_content() -> Result<()> {
        let packets = textured(RateControlMode::Crf { crf: 30 }, 0, u64::MAX)?;
        let qp: BTreeMap<u64, u16> = packets
            .iter()
            .flat_map(|packet| &packet.stats)
            .map(|stats| (stats.pts, stats.qp))
            .collect();
        let layers = [8, 4, 2, 1].map(|display| qp[&display]);
        assert!(layers.is_sorted_by(|a, b| a < b), "{qp:?}");
        assert_eq!(qp[&1], qp[&3], "{qp:?}");
        assert!(qp[&0] < qp[&2], "{qp:?}");
        Ok(())
    }

    /// The motion search of a window reads the last coded frame, the past
    /// reference of the window's first coded frame. A window of six frames
    /// over mini-GOPs of four plans frame 11 against frame 8 only after 8 is
    /// coded; on a pan frame 11 then costs far less than intra.
    #[test]
    fn a_window_searches_against_the_last_coded_frame() -> Result<()> {
        let mut config = Config::new(64, 64);
        config.preset_overrides = [("lookahead_frames", "6"), ("mini_gop_size", "4")]
            .map(|(field, value)| (field.to_owned(), value.to_owned()))
            .into();
        let mut encoder =
            Encoder::with_coder(config.clone(), Box::new(FakeCoder { fail_at: None }))?;
        for pts in 0..12 {
            encoder.send_frame(crate::lookahead::tests::frame(
                &config,
                pts,
                2 * pts as usize,
                0,
            ))?;
        }
        encoder.flush()?;
        drain(&mut encoder)?;
        let last = encoder
            .window
            .iter()
            .max_by_key(|info| info.display_index)
            .copied();
        let last = last.ok_or(Error::Failed)?;
        assert_eq!(
            (last.display_index, last.frame_type),
            (11, FrameType::Inter)
        );
        let intra = encoder.lookahead.intra(11).ok_or(Error::Failed)?;
        let intra: u64 = intra.iter().map(|&cost| u64::from(cost.max(128))).sum();
        assert!(
            last.complexity * 2 < intra,
            "{} vs intra {intra}",
            last.complexity
        );
        Ok(())
    }

    /// Encodes 20 panning frames `width` wide in `tiles` with a cut at 11,
    /// sending them `sends` at a time and then receiving up to `receives`
    /// packets, until the input ends.
    fn interleaved(
        (rate_control, width, tiles): (RateControlMode, u32, TileLayout),
        mut sends: impl FnMut() -> usize,
        mut receives: impl FnMut() -> usize,
    ) -> Result<Vec<u8>> {
        let mut config = Config::new(width, 16);
        config.rate_control = rate_control;
        config.tiles = tiles;
        config.preset = Preset::new(9).unwrap_or_default();
        let mut encoder = Encoder::new(config.clone())?;
        let mut bytes = Vec::new();
        let mut next = 0;
        while next < 20 {
            for _ in 0..sends() {
                if next < 20 {
                    let scene = u32::from(next >= 11);
                    let frame =
                        crate::lookahead::tests::frame(&config, next, 2 * next as usize, scene);
                    encoder.send_frame(frame)?;
                    next += 1;
                }
            }
            for _ in 0..receives() {
                let Some(packet) = encoder.receive_packet()? else {
                    break;
                };
                bytes.extend(packet.data);
            }
        }
        encoder.flush()?;
        for packet in drain(&mut encoder)? {
            bytes.extend(packet.data);
        }
        Ok(bytes)
    }

    /// Pseudo-random counts in `0..6`.
    fn xorshift(mut state: u32) -> impl FnMut() -> usize {
        move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % 6) as usize
        }
    }

    /// One send and a full drain per frame, every send before the flush, and
    /// random interleavings all give the same bytes; the VBR case codes two
    /// tiles in parallel.
    #[test]
    fn output_does_not_depend_on_the_call_pattern() -> Result<()> {
        let one = TileLayout::default();
        let two = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        let cbr = RateControlMode::Cbr {
            bitrate_kbps: 60,
            buffer_ms: 1000,
            initial_ms: 500,
        };
        for mode in [
            (RateControlMode::Crf { crf: 30 }, 48, one),
            (RateControlMode::Vbr { bitrate_kbps: 90 }, 72, two),
            (cbr, 48, one),
        ] {
            let per_frame = interleaved(mode, || 1, || usize::MAX)?;
            assert!(!per_frame.is_empty());
            assert!(per_frame == interleaved(mode, || 20, || 0)?, "{mode:?}");
            for seed in [1, 7, 12_345] {
                let random = interleaved(mode, xorshift(seed), xorshift(seed + 99))?;
                assert!(per_frame == random, "{mode:?} seed {seed}");
            }
        }
        Ok(())
    }

    /// Asks for a qindex above `MaxQ` and records the show-existing bits.
    struct Recorder(Arc<Mutex<Vec<u64>>>);

    impl RateController for Recorder {
        fn frame_qp(&mut self, _info: &FramePlanInfo) -> u16 {
            u16::MAX
        }

        fn update(&mut self, _stats: &FrameStats) {}

        fn update_show_existing(&mut self, bits: u64) {
            if let Ok(mut seen) = self.0.lock() {
                seen.push(bits);
            }
        }
    }

    /// A caller's rate controller gets every show-existing unit's bits, and
    /// its qindex is clamped to `MaxQ`.
    #[test]
    fn custom_rate_controller_sees_show_existing_bits() -> Result<()> {
        let mut config = Config::new(16, 16);
        config.preset = Preset::new(6)?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut encoder =
            Encoder::with_rate_controller(config.clone(), Box::new(Recorder(Arc::clone(&seen))))?;
        for pts in 0..5 {
            let planes = [vec![90; 256], vec![128; 64], vec![160; 64]];
            encoder.send_frame(SourceFrame::new(&config, pts, planes)?)?;
        }
        encoder.flush()?;
        let packets = drain(&mut encoder)?;
        let shown: Vec<u64> = packets
            .iter()
            .filter(|packet| packet.stats.is_empty())
            .map(|packet| packet.data.len() as u64 * 8)
            .collect();
        assert_eq!(shown.len(), 2);
        assert_eq!(seen.lock().map(|seen| seen.clone()).ok(), Some(shown));
        assert!(packets.iter().flat_map(|p| &p.stats).all(|s| s.qp == 255));
        Ok(())
    }

    #[test]
    fn state_machine_rejects_invalid_operations() -> Result<()> {
        let mut encoder = encoder(Some(0), Preset::MAX)?;
        let mut other = Config::new(8, 4);
        other.rate_control = RateControlMode::ConstantQp { qp: 40 };
        let wrong = SourceFrame::new(&other, 0, [vec![0; 32], vec![0; 8], vec![0; 8]])?;
        assert!(matches!(
            encoder.send_frame(wrong),
            Err(Error::FrameConfigMismatch)
        ));
        let first = frame(&encoder, 0)?;
        encoder.send_frame(first)?;
        encoder.flush()?;
        let late = frame(&encoder, 1)?;
        assert!(matches!(
            encoder.send_frame(late),
            Err(Error::SendAfterFlush)
        ));
        assert!(matches!(
            encoder.receive_packet(),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(encoder.receive_packet(), Err(Error::Failed)));
        assert!(matches!(encoder.flush(), Err(Error::Failed)));
        Ok(())
    }
}
