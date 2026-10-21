// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The push/pull [`Encoder`]: frames go in with [`Encoder::send_frame`], one
//! packet per frame comes out of [`Encoder::receive_packet`] in send order.

use std::collections::VecDeque;
use std::fmt;

use crate::config::{Config, RateControlMode};
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::gop::{self, GopFrame};
use crate::intra_frame::IntraFrameCoder;
use crate::rate_control::{ConstantQp, FramePlanInfo, FrameType, RateController};
use crate::stats::{FrameStats, psnr, ssim};

/// One coded frame.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Packet {
    /// One AV2 Annex B temporal unit.
    pub data: Vec<u8>,
    /// Presentation timestamp of the source frame.
    pub pts: u64,
    /// Frame type.
    pub frame_type: FrameType,
    /// Statistics of the frame.
    pub stats: FrameStats,
}

/// The output of a [`FrameCoder`] for one frame.
pub(crate) struct CodedFrame {
    /// One AV2 Annex B temporal unit.
    pub(crate) temporal_unit: Vec<u8>,
    /// Reconstructed Y, U and V planes, packed like the source planes.
    pub(crate) recon: [Vec<u16>; 3],
    /// The frame type the coder used, which can differ from the planned one.
    pub(crate) frame_type: FrameType,
}

/// Codes one frame at the quantizer and plan the encoder picked.
pub(crate) trait FrameCoder {
    fn encode_frame(&mut self, source: &SourceFrame, plan: &GopFrame, qp: u8)
    -> Result<CodedFrame>;
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
    next_frame_index: u64,
    state: State,
}

impl fmt::Debug for Encoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Encoder")
            .field("config", &self.config)
            .field("pending", &self.pending.len())
            .field("next_frame_index", &self.next_frame_index)
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

    fn with_coder(config: Config, coder: Box<dyn FrameCoder + Send>) -> Self {
        let rate_control = match config.rate_control {
            RateControlMode::ConstantQp { qp } => Box::new(ConstantQp { qp }),
        };
        Self {
            config,
            coder,
            rate_control,
            stats_sink: None,
            pending: VecDeque::new(),
            next_frame_index: 0,
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

    /// Codes the oldest queued frame. Returns `None` when no frame is queued:
    /// send more frames, or after [`Self::flush`] the stream is complete.
    ///
    /// # Errors
    /// Returns the coding error, after which the encoder stays failed, or
    /// [`Error::Failed`] after an earlier coding error.
    pub fn receive_packet(&mut self) -> Result<Option<Packet>> {
        if self.state == State::Failed {
            return Err(Error::Failed);
        }
        let Some(frame) = self.pending.pop_front() else {
            return Ok(None);
        };
        let packet = self.code(&frame);
        if packet.is_err() {
            self.state = State::Failed;
        }
        packet.map(Some)
    }

    fn code(&mut self, frame: &SourceFrame) -> Result<Packet> {
        let frame_index = self.next_frame_index;
        let gop_frame = gop::plan(frame_index, self.config.keyint);
        let plan = FramePlanInfo {
            frame_index,
            frame_type: gop_frame.frame_type,
        };
        let qp = self.rate_control.frame_qp(&plan);
        let coded = self.coder.encode_frame(frame, &gop_frame, qp)?;

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
            frame_index,
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
        self.next_frame_index += 1;
        Ok(Packet {
            data: coded.temporal_unit,
            pts: frame.pts(),
            frame_type: coded.frame_type,
            stats,
        })
    }
}

fn frame_coder(config: &Config) -> Result<Box<dyn FrameCoder + Send>> {
    Ok(Box::new(IntraFrameCoder::new(config)?))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Emits the frame index and qp as the packet and a recon that is the
    /// source plus one; fails on the frame index it was told to.
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
            Ok(CodedFrame {
                temporal_unit: vec![plan.frame_index as u8, qp],
                recon,
                frame_type: plan.frame_type,
            })
        }
    }

    fn encoder(fail_at: Option<u64>) -> Encoder {
        let mut config = Config::new(4, 4);
        config.keyint = core::num::NonZeroU32::MIN.saturating_add(1);
        config.rate_control = RateControlMode::ConstantQp { qp: 40 };
        Encoder::with_coder(config, Box::new(FakeCoder { fail_at }))
    }

    fn frame(encoder: &Encoder, pts: u64) -> Result<SourceFrame> {
        SourceFrame::new(
            encoder.config(),
            pts,
            [vec![10; 16], vec![20; 4], vec![30; 4]],
        )
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
        let mut packets = Vec::new();
        while let Some(packet) = encoder.receive_packet()? {
            packets.push(packet);
        }
        assert_eq!(packets.len(), 2);
        for (pts, packet) in packets.iter().enumerate() {
            assert_eq!(packet.pts, pts as u64);
            assert!(packet.stats.psnr_y > 40.0, "{:?}", packet.stats);
        }
        let types: Vec<_> = packets.iter().map(|packet| packet.frame_type).collect();
        assert_eq!(types, [FrameType::Key, FrameType::Inter]);
        Ok(())
    }

    #[test]
    fn codes_frames_in_order_with_stats() -> Result<()> {
        let mut encoder = encoder(None);
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
        let mut packets = Vec::new();
        while let Some(packet) = encoder.receive_packet()? {
            packets.push(packet);
        }
        let types: Vec<_> = packets.iter().map(|p| (p.pts, p.frame_type)).collect();
        assert_eq!(
            types,
            [
                (5, FrameType::Key),
                (6, FrameType::Inter),
                (7, FrameType::Key)
            ]
        );
        let stats = packets[1].stats;
        assert_eq!((stats.frame_index, stats.qp, stats.bits), (1, 40, 16));
        assert!((stats.psnr_y - 48.130_803_608).abs() < 1e-6);
        assert_eq!(packets[2].data, [2, 40]);
        assert_eq!(seen.lock().map(|s| s.clone()).ok(), Some(vec![0, 1, 2]));
        Ok(())
    }

    #[test]
    fn state_machine_rejects_invalid_operations() -> Result<()> {
        let mut encoder = encoder(Some(0));
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
