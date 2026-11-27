// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The push/pull [`Encoder`]: frames go in with [`Encoder::send_frame`], and
//! [`Encoder::receive_packet`] returns one temporal unit per frame, in coding
//! order, which outputs the frames in send order. A call codes only as far as
//! its packet needs, and [`Encoder::progress`] reports how far a call has come.

use std::collections::VecDeque;
use std::fmt;

use splot_core::headers::frame::{FrameHeaderCore, encode_padding_obu};
use splot_decode::encoder_syntax::{FrameStart, TileEnd};
use splot_recon::DecodedFrame;

use crate::cancel::CancelToken;
use crate::config::{Config, RateControlMode};
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::gop::{self, GopFrame, GopPlanner, GopStep};
use crate::intra_frame::IntraFrameCoder;
use crate::lookahead::Lookahead;
use crate::preset::{FrameKind, PresetContext, ResolutionClass};
use crate::progress::Progress;
use crate::rate_control::{
    Cbr, ConstantQp, Crf, DecoderBuffer, FramePlanInfo, FrameType, RateController, Vbr, max_qindex,
};
use crate::stats::{FrameStats, psnr, ssim};
use crate::tpl;

/// One AV2 temporal unit.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Packet {
    /// One AV2 Annex B temporal unit: the hidden frames coded since the last
    /// packet and one output: a shown, implicit-output or show-existing frame.
    pub data: Vec<u8>,
    /// Presentation timestamp of the output frame; not monotonic with implicit output.
    pub pts: u64,
    /// Frame type of the output frame.
    pub frame_type: FrameType,
    /// Statistics of each frame coded in this temporal unit, in coding order;
    /// empty when the unit only shows an earlier hidden frame.
    pub stats: Vec<FrameStats>,
}

/// The header, filtered reconstruction, initial CDFs, final tile CDFs and
/// `ccso_blk` flags of a coded frame.
pub(crate) type CodedReference = (
    FrameHeaderCore,
    DecodedFrame<u16>,
    FrameStart,
    Vec<TileEnd>,
    Vec<Vec<u8>>,
);

/// The output of a [`FrameCoder`] for one frame.
pub(crate) struct CodedFrame {
    /// The frame's OBUs, after a temporal delimiter when the plan opens a
    /// temporal unit; a key frame also repeats the sequence header.
    pub(crate) temporal_unit: Vec<u8>,
    /// Reconstructed Y, U and V planes, packed like the source planes.
    pub(crate) recon: [Vec<u16>; 3],
    /// The frame type the coder used, which can differ from the planned one.
    pub(crate) frame_type: FrameType,
    /// The display index of the frame's plan.
    pub(crate) display_index: u64,
    /// What [`FrameCoder::commit`] stores.
    pub(crate) reference: Option<CodedReference>,
    /// The header of a `TIP_FRAME_AS_OUTPUT` frame, which
    /// [`FrameCoder::commit`] records in place of `reference`.
    pub(crate) tip_output: Option<FrameHeaderCore>,
    /// How the inter frame used warped motion; `None` for key frames and for
    /// `TIP_FRAME_AS_OUTPUT` frames, which code no leaves.
    pub(crate) warp: Option<crate::warp::WarpUse>,
    /// The 8x8 cells the intra leaves of the coded trees cover, which later
    /// frames read from their primary reference; `None` without leaves.
    pub(crate) intra_cells: Option<std::sync::Arc<crate::intra_frame::IntraCells>>,
    /// A key frame whose temporal unit carries a film grain model; the
    /// frames after it apply grain.
    pub(crate) film_grain_model: bool,
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
    /// A temporal delimiter, the metadata OBUs of the output frame
    /// `display_index` and the show-existing frame that outputs the frame in
    /// `slot`, with the explicit `order_hint` when given, as an
    /// `OBU_LEADING_SEF` when `leading`.
    fn show_existing_frame(
        &mut self,
        slot: u32,
        order_hint: Option<u32>,
        leading: bool,
        display_index: u64,
    ) -> Result<Vec<u8>>;
    /// Ends the leading frames of an open-loop key frame before the first
    /// regular frame after them codes (§ 5.18.1); nothing without them.
    fn end_leading_frames(&mut self) -> Result<()>;
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

/// A mini-GOP in coding: [`Encoder::receive_packet`] codes it one wave at a time.
struct MiniGop {
    frames: Vec<SourceFrame>,
    /// The display index of the first of `frames`.
    first: u64,
    steps: Vec<GopStep>,
    /// The number of `steps` done.
    position: usize,
    /// The coded frame type of each of `frames`.
    frame_types: Vec<FrameType>,
    /// The OBUs of the frames coded since the last packet.
    data: Vec<u8>,
    /// The statistics of the frames coded since the last packet.
    stats: Vec<FrameStats>,
}

/// Callback that receives the statistics of every coded frame, in coding
/// order, once the frame is final: after the other frames of its wave are
/// coded and rate control has settled its quantizer. For how far the encoder
/// has come, read [`Progress`].
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
    /// The preset's `global_motion` estimator level.
    global_motion: u8,
    /// The preset's temporal filter frames and strengths of key and base frames.
    temporal_filter: [(u8, u8); 2],
    /// The [`crate::preset::PresetParams::key_qindex_boost`] of the coded size.
    key_boost: i16,
    /// The last frame qindex, which estimates the next ones.
    last_qindex: Option<u16>,
    /// The decoder buffer of [`RateControlMode::CappedCrf`] and [`RateControlMode::Cbr`].
    buffer: Option<DecoderBuffer>,
    /// The mini-GOP whose next wave [`Self::receive_packet`] codes.
    coding: Option<MiniGop>,
    ready: VecDeque<Packet>,
    state: State,
    cancel: CancelToken,
    progress: Progress,
    /// The frames committed so far.
    frames_coded: u64,
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
        let (cancel, progress) = (CancelToken::default(), Progress::default());
        let coder = frame_coder(&config.coded(), cancel.clone(), progress.clone())?;
        Self::with_parts(config, coder, rate_control, (cancel, progress))
    }

    #[cfg(test)]
    pub(crate) fn with_coder(config: Config, coder: Box<dyn FrameCoder + Send>) -> Result<Self> {
        let rate_control = rate_controller(&config)?;
        let handles = (CancelToken::default(), Progress::default());
        Self::with_parts(config, coder, rate_control, handles)
    }

    fn with_parts(
        config: Config,
        coder: Box<dyn FrameCoder + Send>,
        rate_control: Box<dyn RateController + Send>,
        (cancel, progress): (CancelToken, Progress),
    ) -> Result<Self> {
        let params = config.override_preset(config.preset.params())?;
        let mut lookahead = Lookahead::new(&config.coded(), &params, coder.pool());
        lookahead.cancel = cancel.clone();
        let mini_gop_size = match config.rate_control {
            RateControlMode::Vbr { .. } | RateControlMode::Cbr { .. } => {
                params.bitrate_mini_gop_size
            }
            _ => params.mini_gop_size,
        };
        let slots = match params.num_ref_frames {
            0 => 8,
            slots => slots,
        };
        let planner = GopPlanner::new(config.keyint, mini_gop_size)
            .with_ref_slots(slots)
            .with_intra_only(params.intra_only)
            .with_open_loop_key(params.open_loop_key)
            .with_implicit_output(params.implicit_output)
            .with_sef_repeat(params.sef_repeat);
        Ok(Self {
            rate_control,
            stats_sink: None,
            pending: VecDeque::new(),
            last_coded: None,
            planner,
            lookahead,
            lookahead_frames: usize::from(params.lookahead_frames),
            window: Vec::new(),
            tpl_delta_q: params.tpl_delta_q,
            global_motion: params.global_motion,
            temporal_filter: [FrameKind::Key, FrameKind::Base].map(|frame| {
                let coded = config.coded();
                let context = PresetContext {
                    resolution: ResolutionClass::of(coded.width, coded.height),
                    frame,
                    ..PresetContext::default()
                };
                (params.tf_frames.at(context), params.tf_strength.at(context))
            }),
            key_boost: {
                let coded = config.coded();
                let context = PresetContext {
                    resolution: ResolutionClass::of(coded.width, coded.height),
                    frame: FrameKind::Key,
                    ..PresetContext::default()
                };
                i16::from(params.key_qindex_boost.at(context))
            },
            last_qindex: None,
            buffer: DecoderBuffer::new(&config),
            resized: config.resize_denominator.map(|_| config.coded()),
            config,
            coder,
            coding: None,
            ready: VecDeque::new(),
            state: State::Accepting,
            cancel,
            progress,
            frames_coded: 0,
        })
    }

    /// Returns the configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The token that stops this encoder from another thread; see
    /// [`CancelToken`].
    #[must_use]
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// The handle that reads this encoder's progress from another thread; see
    /// [`Progress`].
    #[must_use]
    pub fn progress(&self) -> Progress {
        self.progress.clone()
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
    /// Returns [`Error::Cancelled`] after [`CancelToken::cancel`],
    /// [`Error::SendAfterFlush`] after [`Self::flush`],
    /// [`Error::Failed`] after an earlier coding error, or
    /// [`Error::FrameConfigMismatch`] when the frame was built for another
    /// configuration.
    pub fn send_frame(&mut self, frame: SourceFrame) -> Result<()> {
        self.cancel.check()?;
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
        if self.planner.sef_repeat() {
            let previous = self.pending.back().or_else(|| {
                self.coding
                    .as_ref()
                    .and_then(|mini_gop| mini_gop.frames.last())
                    .or(self.last_coded.as_ref())
            });
            if previous.is_some_and(|previous| previous.planes() == frame.planes()) {
                self.planner.repeat(display_index);
            }
        }
        self.pending.push_back(frame);
        self.cancel.check()
    }

    /// Signals the end of input. Queued frames are still coded.
    ///
    /// # Errors
    /// Returns [`Error::Cancelled`] after [`CancelToken::cancel`], or
    /// [`Error::Failed`] after an earlier coding error.
    pub fn flush(&mut self) -> Result<()> {
        self.cancel.check()?;
        if self.state == State::Failed {
            return Err(Error::Failed);
        }
        self.state = State::Flushed;
        Ok(())
    }

    /// Returns the next temporal unit. A mini-GOP starts once the preset's
    /// `lookahead_frames` frames are queued, or after [`Self::flush`]; a call
    /// codes its frames only until a unit is complete, so the units of a
    /// mini-GOP come out as they are coded. Returns `None` when no unit is
    /// ready: send more frames, or after [`Self::flush`] the stream is
    /// complete.
    ///
    /// # Errors
    /// Returns [`Error::Cancelled`] when [`CancelToken::cancel`] was called
    /// before or during the call, the coding error, after which the encoder
    /// stays failed, or [`Error::Failed`] after an earlier coding error.
    pub fn receive_packet(&mut self) -> Result<Option<Packet>> {
        self.cancel.check()?;
        if self.state == State::Failed {
            return Err(Error::Failed);
        }
        while self.ready.is_empty() {
            let coded = if let Some(mini_gop) = self.coding.take() {
                self.code_wave(mini_gop)
            } else {
                let ended = self.state == State::Flushed;
                let queued = self.pending.len();
                if !ended && queued < self.lookahead_frames {
                    break;
                }
                let Some(len) = self.planner.next_len(queued, ended) else {
                    break;
                };
                self.start_mini_gop(len).map(Some)
            };
            match coded {
                Ok(mini_gop) => self.coding = mini_gop,
                Err(error) => {
                    self.state = State::Failed;
                    self.cancel.check()?;
                    return Err(error);
                }
            }
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
        self.cancel.check()?;
        if let Some(fields) = self.lookahead.fields() {
            self.coder.set_open_loop_motion(fields);
        }
        if self.coder.wants_global_motion() {
            let lambda = crate::rd::Lambda::from_qindex(
                u32::from(self.qindex_estimate()),
                self.config.bit_depth,
            )
            .map_err(crate::intra_frame::coding("global motion lambda"))?;
            let sad_per_bit = lambda.sse_per_bit_q16().isqrt();
            let models = crate::global_motion::estimate(
                &mut self.lookahead,
                &plan,
                (coded_before, self.global_motion, sad_per_bit),
                &sources,
            );
            self.coder.set_global_motion(models);
            self.cancel.check()?;
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
                    qindex_offset: model.qindex_offset
                        - if frame.frame_type == FrameType::Key {
                            self.key_boost
                        } else {
                            0
                        },
                    bit_depth: self.config.bit_depth,
                }
            })
            .collect();
        if blocks.is_some() {
            let (cols, rows) = self.lookahead.grid();
            self.coder.set_block_importance(tpl::BlockImportance {
                grid: (cols, rows, self.lookahead.block_luma()),
                strength: self.tpl_delta_q,
                centred: matches!(self.config.rate_control, RateControlMode::ConstantQp { .. }),
                frames: costs,
            });
        }
        self.rate_control.plan_window(&self.window);
        Ok(())
    }

    /// Plans the next `len` queued frames, the next frames of the plan, as
    /// the mini-GOP that [`Self::code_wave`] codes.
    fn start_mini_gop(&mut self, len: usize) -> Result<MiniGop> {
        self.plan_window()?;
        let frames: Vec<SourceFrame> = self.pending.drain(..len).collect();
        let first = self.planner.next_display();
        if first == 0
            && let Some(frame) = frames.first()
        {
            self.coder.start_sequence(frame)?;
        }
        Ok(MiniGop {
            frame_types: vec![FrameType::Inter; frames.len()],
            steps: self.planner.plan(frames.len())?,
            frames,
            first,
            position: 0,
            data: Vec::new(),
            stats: Vec::new(),
        })
    }

    /// Codes the next wave of `mini_gop` into ready packets and returns the
    /// mini-GOP while it has more waves.
    fn code_wave(&mut self, mut mini_gop: MiniGop) -> Result<Option<MiniGop>> {
        let MiniGop {
            frames,
            first,
            steps,
            position,
            frame_types,
            data,
            stats,
        } = &mut mini_gop;
        let (frames, first) = (frames.as_slice(), *first);
        let source = |display_index: u64| {
            usize::try_from(display_index - first)
                .ok()
                .and_then(|index| frames.get(index))
                .ok_or(Error::Unsupported {
                    what: "a plan outside its mini-GOP",
                })
        };
        let wave = gop::waves(steps.get(*position..).unwrap_or(&[]))
            .next()
            .unwrap_or(&[]);
        let mut planned = Vec::new();
        for step in wave {
            if let GopStep::Code(plan) = *step {
                planned.push((plan, self.frame_qp(&plan)?));
            }
        }
        let filtered = self.temporal_filter(&planned, frames, first);
        self.cancel.check()?;
        let mut jobs = Vec::new();
        for (&(plan, qp), filtered) in planned.iter().zip(&filtered) {
            let frame = match filtered {
                Some(filtered) => filtered,
                None => source(plan.display_index)?,
            };
            jobs.push((frame, plan, qp));
        }
        if jobs.first().is_some_and(|(_, plan, _)| !plan.leading) {
            self.coder.end_leading_frames()?;
        }
        let mut coded = jobs.iter().zip(self.coder.encode_wave(&jobs)?);
        for step in wave {
            *position += 1;
            match *step {
                GopStep::Code(_) => {
                    let (&(coded_source, plan, qp), coded) =
                        coded.next().ok_or(Error::Unsupported {
                            what: "a wave with fewer coded frames than planned",
                        })?;
                    let later = frames_later_in_unit(steps.get(*position - 1..).unwrap_or(&[]));
                    let (qp, mut coded) =
                        self.fit_buffer((coded_source, plan, qp), coded, later)?;
                    let frame = source(plan.display_index)?;
                    stats.push(self.commit(frame, &plan, qp, &mut coded)?);
                    frame_types[(plan.display_index - first) as usize] = coded.frame_type;
                    data.extend(coded.temporal_unit);
                }
                GopStep::ShowExisting {
                    slot,
                    display_index,
                    order_hint,
                    leading,
                } => {
                    let unit =
                        self.coder
                            .show_existing_frame(slot, order_hint, leading, display_index)?;
                    let bits = unit.len() as u64 * 8;
                    if let Some(buffer) = &mut self.buffer {
                        buffer.add(bits);
                    }
                    match order_hint {
                        Some(_) => self.rate_control.update_repeat(bits, display_index),
                        None => self.rate_control.update_show_existing(bits),
                    }
                    data.extend(unit);
                }
            }
            if let Some(display_index) = step.output() {
                if let Some(buffer) = &mut self.buffer {
                    let overflow = buffer.overflow();
                    if self.config.cbr_padding && overflow > 0 {
                        let padding = encode_padding_obu(overflow.div_ceil(8) as usize).map_err(
                            |source| Error::Coding {
                                stage: "padding",
                                source: Box::new(source),
                            },
                        )?;
                        let bits = padding.len() as u64 * 8;
                        buffer.add(bits);
                        self.rate_control.update_padding(bits);
                        data.extend(padding);
                    }
                    buffer.drain();
                }
                self.ready.push_back(Packet {
                    data: core::mem::take(data),
                    pts: source(display_index)?.pts(),
                    frame_type: frame_types[(display_index - first) as usize],
                    stats: core::mem::take(stats),
                });
            }
        }
        self.progress.set_frames(self.frames_coded);
        if *position < steps.len() {
            return Ok(Some(mini_gop));
        }
        self.lookahead
            .retain_from(self.planner.next_display().saturating_sub(1));
        self.last_coded = mini_gop.frames.pop();
        Ok(None)
    }

    /// `coded`, the frame `plan` coded from `source` at `qp`, with its qindex;
    /// while its unit would underflow the [`DecoderBuffer`] below `MaxQ`, the
    /// frame coded again at a coarser qindex ([`DecoderBuffer::coarser`]). A
    /// new coding reruns the frame coder only (mode decision, loop filters
    /// and entropy coding) with the lookahead motion, TPL and filtered source
    /// of the first.
    fn fit_buffer(
        &mut self,
        (source, plan, mut qp): (&SourceFrame, GopFrame, u16),
        mut coded: CodedFrame,
        later: (u64, u64),
    ) -> Result<(u16, CodedFrame)> {
        let Some(buffer) = &self.buffer else {
            return Ok((qp, coded));
        };
        let mut tries = vec![(qp, coded.temporal_unit.len() as u64 * 8)];
        while let Some(coarser) = buffer.coarser(&tries, later, self.config.bit_depth) {
            qp = coarser;
            self.progress.rewind_frame(self.frames_coded);
            coded =
                self.coder
                    .encode_wave(&[(source, plan, qp)])?
                    .pop()
                    .ok_or(Error::Unsupported {
                        what: "a frame coded again without its result",
                    })?;
            tries.push((qp, coded.temporal_unit.len() as u64 * 8));
        }
        if let Some(buffer) = &mut self.buffer {
            buffer.add(coded.temporal_unit.len() as u64 * 8);
        }
        Ok((qp, coded))
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
        let qindex = self.rate_control.frame_qp(&plan).clamp(
            u16::from(!self.config.is_lossless()),
            max_qindex(self.config.bit_depth),
        );
        self.last_qindex = Some(qindex);
        Ok(qindex)
    }

    /// The last frame qindex, or before the first frame the base qindex of
    /// a constant-qp or constant-quality configuration (a mid qindex else).
    fn qindex_estimate(&self) -> u16 {
        self.last_qindex.unwrap_or(match self.config.rate_control {
            RateControlMode::ConstantQp { qp } => qp,
            RateControlMode::Crf { crf } | RateControlMode::CappedCrf { crf, .. } => {
                Crf { crf }.base_qindex(self.config.bit_depth)
            }
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
        self.frames_coded += 1;
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
        RateControlMode::Crf { crf } | RateControlMode::CappedCrf { crf, .. } => {
            Box::new(Crf { crf })
        }
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

/// The hidden and the shown frames coded after `steps[0]` in its temporal
/// unit.
fn frames_later_in_unit(steps: &[GopStep]) -> (u64, u64) {
    let mut later = (0, 0);
    let Some((first, rest)) = steps.split_first() else {
        return later;
    };
    if first.output().is_some() {
        return later;
    }
    for step in rest {
        if let GopStep::Code(frame) = step {
            if frame.show_frame {
                later.1 += 1;
            } else {
                later.0 += 1;
            }
        }
        if step.output().is_some() {
            break;
        }
    }
    later
}

fn frame_coder(
    config: &Config,
    cancel: CancelToken,
    progress: Progress,
) -> Result<Box<dyn FrameCoder + Send>> {
    let mut coder = IntraFrameCoder::new(config)?;
    coder.cancel = cancel;
    coder.progress = progress;
    Ok(Box::new(coder))
}

#[cfg(test)]
#[path = "encoder_tests.rs"]
mod tests;
