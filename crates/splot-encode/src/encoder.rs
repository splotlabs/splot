// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The push/pull [`Encoder`]: frames go in with [`Encoder::send_frame`], and
//! [`Encoder::receive_packet`] returns one temporal unit per frame, in coding
//! order, which outputs the frames in send order.

use std::collections::VecDeque;
use std::fmt;

use crate::config::{Config, RateControlMode};
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::gop::{GopFrame, GopPlanner, GopStep};
use crate::intra_frame::IntraFrameCoder;
use crate::rate_control::{ConstantQp, FramePlanInfo, FrameType, RateController};
use crate::stats::{FrameStats, psnr, ssim};

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
}

/// Codes the frames and show-existing frames of the plan, in coding order.
pub(crate) trait FrameCoder {
    /// Codes `source` as `plan` at quantizer `qp`.
    fn encode_frame(&mut self, source: &SourceFrame, plan: &GopFrame, qp: u8)
    -> Result<CodedFrame>;
    /// A temporal delimiter and the show-existing frame that outputs the
    /// hidden frame in `slot`.
    fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>>;
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
    coder: Box<dyn FrameCoder + Send>,
    rate_control: Box<dyn RateController + Send>,
    stats_sink: Option<StatsSink>,
    pending: VecDeque<SourceFrame>,
    planner: GopPlanner,
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
    /// Creates an encoder.
    ///
    /// # Errors
    /// Returns a configuration error from [`Config::validate`], or
    /// [`Error::Unsupported`] because no frame coder is available yet.
    pub fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let coder = frame_coder(&config)?;
        Ok(Self::with_coder(config, coder))
    }

    pub(crate) fn with_coder(config: Config, coder: Box<dyn FrameCoder + Send>) -> Self {
        let rate_control = match config.rate_control {
            RateControlMode::ConstantQp { qp } => Box::new(ConstantQp { qp }),
        };
        let planner = GopPlanner::new(config.keyint, config.preset.params().mini_gop_size);
        Self {
            config,
            coder,
            rate_control,
            stats_sink: None,
            pending: VecDeque::new(),
            planner,
            ready: VecDeque::new(),
            state: State::Accepting,
        }
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

    /// Queues a frame for coding. The encoder keeps every queued frame until
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
            || frame.plane_sizes() != self.config.plane_sizes()?
        {
            return Err(Error::FrameConfigMismatch);
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
    /// `mini_gop_size` frames are queued, or after [`Self::flush`]. Returns
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
        if self.ready.is_empty()
            && let Some(len) = self
                .planner
                .next_len(self.pending.len(), self.state == State::Flushed)
        {
            let frames: Vec<SourceFrame> = self.pending.drain(..len).collect();
            if let Err(error) = self.code_mini_gop(&frames) {
                self.state = State::Failed;
                return Err(error);
            }
        }
        Ok(self.ready.pop_front())
    }

    /// Codes `frames`, the next frames of the plan, into ready packets.
    fn code_mini_gop(&mut self, frames: &[SourceFrame]) -> Result<()> {
        let first = self.planner.next_display();
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
        for step in self.planner.plan(frames.len())? {
            match step {
                GopStep::Code(plan) => {
                    let frame = source(plan.display_index)?;
                    let (coded, frame_stats) = self.code(frame, &plan)?;
                    frame_types[(plan.display_index - first) as usize] = coded.frame_type;
                    data.extend(coded.temporal_unit);
                    stats.push(frame_stats);
                }
                GopStep::ShowExisting { slot, .. } => {
                    data.extend(self.coder.show_existing_frame(slot)?);
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
        Ok(())
    }

    fn code(
        &mut self,
        frame: &SourceFrame,
        gop_frame: &GopFrame,
    ) -> Result<(CodedFrame, FrameStats)> {
        let plan = FramePlanInfo {
            frame_index: gop_frame.frame_index,
            frame_type: gop_frame.frame_type,
        };
        let qp = self.rate_control.frame_qp(&plan);
        let coded = self.coder.encode_frame(frame, gop_frame, qp)?;

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
        Ok((coded, stats))
    }
}

fn frame_coder(config: &Config) -> Result<Box<dyn FrameCoder + Send>> {
    Ok(Box::new(IntraFrameCoder::new(config)?))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::preset::Preset;

    /// Emits the frame index and qp as the frame's OBUs (after `0xd0` when
    /// the frame opens a unit) and a recon that is the source plus one; fails
    /// on the frame index it was told to.
    struct FakeCoder {
        fail_at: Option<u64>,
    }

    impl FrameCoder for FakeCoder {
        fn encode_frame(
            &mut self,
            source: &SourceFrame,
            plan: &GopFrame,
            qp: u8,
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
            temporal_unit.extend([plan.frame_index as u8, qp]);
            Ok(CodedFrame {
                temporal_unit,
                recon,
                frame_type: plan.frame_type,
            })
        }

        fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>> {
            Ok(vec![0xd0, 0x5e, slot as u8])
        }
    }

    fn encoder(fail_at: Option<u64>, preset: u8) -> Encoder {
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

    #[test]
    fn codes_frames_in_order_with_stats() -> Result<()> {
        let mut encoder = encoder(None, Preset::MAX);
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
    /// next output.
    #[test]
    fn packs_hidden_frames_into_the_next_output_unit() -> Result<()> {
        let mut encoder = encoder(None, 6);
        encoder.config.keyint = core::num::NonZeroU32::new(10).unwrap_or(encoder.config.keyint);
        encoder.planner = GopPlanner::new(encoder.config.keyint, 4);
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
        // Coding order 4 2 1 [2] 3 [4]: 4 and 2 are hidden in slots 1 and 2.
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

    #[test]
    fn state_machine_rejects_invalid_operations() -> Result<()> {
        let mut encoder = encoder(Some(0), Preset::MAX);
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
