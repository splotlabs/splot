// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Push/pull encoder API surface.
//!
//! The lifecycle is deterministic state plumbing. [`Context::receive_packet`] now produces a real
//! [`Packet`] — one coded access unit (the AV2 Annex B temporal unit, not a container file) — for
//! the input subset the minimal encoder can encode losslessly: a 64x64 frame whose every visible
//! sample is the 128 no-neighbour DC predictor, encoded as the skip frame, whose flat-128
//! reconstruction equals the input. A consumer muxes packets into a container (e.g. IVF) to store
//! or decode them. Any other frame is retired without a packet; broader input handling (forward
//! quantization, larger sizes, real residual) is future work.

use core::num::NonZeroUsize;
use std::collections::VecDeque;

use splot_parallel::{ThreadCount, WorkerPool};

use crate::config::EncoderConfig;
use crate::decide::{ConstantQp, RateController};
use crate::error::{Error, Result};
use crate::frame::{Frame, FrameInfo};
use crate::runtime::{EncoderRuntimeConfig, SpeedPreset};

const INPUT_QUEUE_CAPACITY: usize = 1;
const OUTPUT_QUEUE_CAPACITY: usize = 0;

/// An output coded packet: the bytes of one coded access unit (an AV2 Annex B temporal
/// unit). A consumer muxes packets into a container (e.g. IVF) to store or decode them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Coded bytes for one access unit.
    pub data: Vec<u8>,
}

/// The AV2 § 7.13.2 no-neighbour DC predictor sample for 8-bit content: `128`. A frame whose
/// every visible sample equals this value has zero residual against the predictor, so a skip
/// (all-zero residual) frame reconstructs it bit-exactly.
const NO_NEIGHBOUR_DC_PREDICTOR_8BIT: u8 = 128;

/// The frozen square dimension the minimal skip emitter
/// ([`crate::general_intra_trace::emit_minimal_intra_skip_ivf`]) produces: 64x64.
const SUPPORTED_FRAME_DIMENSION: usize = 64;

/// The `base_q_idx` range the minimal skip path can honestly emit: `1..=90`. Within this
/// range the decoder's coefficient CDF q-context is `0` (matching the skip tile's coding);
/// the skip block's all-zero residual makes the flat-128 reconstruction independent of the
/// exact `base_q_idx`. Above 90 the q-context derivation is not yet modeled, and `0`
/// (lossless) is excluded from this minimal tier.
//
// TODO(spec: ENC-CONFIG-QP-FIELD): widen once the `get_qctx` mapping lands (co-evolving
// with the decoder's currently-placeholder q-context).
const SUPPORTED_SKIP_QP: core::ops::RangeInclusive<u8> = 1..=90;

/// A queued input frame with its visible samples owned, so [`Context::receive_packet`] can
/// inspect the pixels after the borrowed [`Frame`] has gone out of scope.
#[derive(Debug)]
struct QueuedFrame {
    info: FrameInfo,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

/// Current encoder context lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderState {
    /// The context can accept more input frames.
    Accepting,
    /// The caller has flushed input and the context is draining accepted frames.
    Draining,
    /// The context reached end of stream.
    Finished,
    /// The context entered a terminal failed state.
    Failed,
}

/// Encoder lifecycle operation used in typed state errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderOperation {
    /// `Context::send_frame`.
    SendFrame,
    /// `Context::receive_packet`.
    ReceivePacket,
    /// `Context::flush`.
    Flush,
}

/// Status returned by [`Context::send_frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFrameStatus {
    /// The frame metadata was accepted into the bounded input queue.
    Accepted {
        /// Number of queued input frames after the operation.
        queued_frames: usize,
        /// Current input queue capacity in frames.
        queue_capacity: usize,
    },
    /// The input queue is full; the caller should pull packets or retry later.
    QueueFull {
        /// Number of queued input frames at the time of the operation.
        queued_frames: usize,
        /// Current input queue capacity in frames.
        queue_capacity: usize,
    },
}

/// Status returned by [`Context::receive_packet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceivePacketStatus {
    /// A coded packet is available.
    Packet(Packet),
    /// No packet is ready and the context needs more input or more work.
    NeedMoreData,
    /// The context reached end of stream.
    Finished,
}

/// Status returned by [`Context::flush`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushStatus {
    /// The context is draining queued input.
    Draining {
        /// Number of input frames still queued for no-output draining.
        queued_frames: usize,
    },
    /// The context is already at end of stream.
    Finished,
}

/// The encoder context. Holds configuration and runtime parameters.
#[derive(Debug)]
pub struct Context {
    config: EncoderConfig,
    runtime: EncoderRuntimeConfig,
    /// The quantizer-decision seam. The minimal encoder installs a constant-QP controller
    /// built from [`EncoderConfig::qp`]; a rate-controlled implementation swaps in here.
    rate_controller: ConstantQp,
    pool: WorkerPool,
    state: EncoderState,
    input_queue: VecDeque<QueuedFrame>,
    output_queue: VecDeque<Packet>,
}

impl Context {
    /// Creates an encoder context from a bitstream configuration and a runtime
    /// configuration, building the context's single owned [`WorkerPool`].
    ///
    /// # Errors
    /// Returns [`crate::Error::Pool`] if the worker pool cannot be constructed.
    pub fn new(config: EncoderConfig, runtime: EncoderRuntimeConfig) -> Result<Self> {
        let pool = WorkerPool::new(runtime.thread_count)?;
        let rate_controller = ConstantQp::new(config.qp);
        Ok(Self {
            config,
            runtime,
            rate_controller,
            pool,
            state: EncoderState::Accepting,
            input_queue: VecDeque::with_capacity(INPUT_QUEUE_CAPACITY),
            output_queue: VecDeque::with_capacity(OUTPUT_QUEUE_CAPACITY),
        })
    }

    /// Returns the bitstream-affecting configuration.
    #[must_use]
    pub fn config(&self) -> &EncoderConfig {
        &self.config
    }

    /// The runtime (non-bitstream) configuration.
    #[must_use]
    pub fn runtime(&self) -> &EncoderRuntimeConfig {
        &self.runtime
    }

    /// The originally requested (unresolved) thread-count policy.
    #[must_use]
    pub fn requested_threads(&self) -> ThreadCount {
        self.runtime.thread_count
    }

    /// The configured runtime speed preset.
    #[must_use]
    pub fn speed_preset(&self) -> SpeedPreset {
        self.runtime.speed_preset()
    }

    /// The resolved, non-zero worker-thread count.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.pool.threads()
    }

    /// The context's single owned worker pool.
    #[must_use]
    pub fn pool(&self) -> &WorkerPool {
        &self.pool
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub fn state(&self) -> EncoderState {
        self.state
    }

    /// Returns the number of input frames currently queued.
    #[must_use]
    pub fn queued_input_frames(&self) -> usize {
        self.input_queue.len()
    }

    /// Returns the current input queue capacity in frames.
    #[must_use]
    pub const fn input_queue_capacity(&self) -> usize {
        INPUT_QUEUE_CAPACITY
    }

    /// Returns the number of coded packets currently queued.
    #[must_use]
    pub fn queued_output_packets(&self) -> usize {
        self.output_queue.len()
    }

    /// Returns the current output queue capacity in packets.
    #[must_use]
    pub const fn output_queue_capacity(&self) -> usize {
        OUTPUT_QUEUE_CAPACITY
    }

    /// Submits a frame to the encoder.
    ///
    /// # Errors
    /// Returns [`Error::State`] if the context is draining, finished, or failed.
    pub fn send_frame(&mut self, frame: &Frame<'_>) -> Result<SendFrameStatus> {
        if self.state != EncoderState::Accepting {
            return Err(self.state_error(EncoderOperation::SendFrame));
        }

        if self.input_queue.len() >= INPUT_QUEUE_CAPACITY {
            return Ok(SendFrameStatus::QueueFull {
                queued_frames: self.input_queue.len(),
                queue_capacity: INPUT_QUEUE_CAPACITY,
            });
        }

        let info = frame.info();
        self.validate_frame_info(info)?;
        // splot-copy-ok: retain the visible input samples as owned buffers — the borrowed `Frame`
        // does not outlive `send_frame`, so `receive_packet` must own a copy to inspect the pixels
        // (and choose/produce the encode) after the borrow ends. This is the deliberate
        // input-materialization boundary of the push/pull encoder.
        self.input_queue.push_back(QueuedFrame {
            info,
            y: frame.y().visible_rows().flatten().copied().collect(),
            u: frame.u().visible_rows().flatten().copied().collect(),
            v: frame.v().visible_rows().flatten().copied().collect(),
        });
        Ok(SendFrameStatus::Accepted {
            queued_frames: self.input_queue.len(),
            queue_capacity: INPUT_QUEUE_CAPACITY,
        })
    }

    /// Retrieves the next coded packet.
    ///
    /// # Errors
    /// Returns [`Error::State`] if the context is failed.
    pub fn receive_packet(&mut self) -> Result<ReceivePacketStatus> {
        if self.state == EncoderState::Failed {
            return Err(self.state_error(EncoderOperation::ReceivePacket));
        }

        if self.state == EncoderState::Finished {
            return Ok(ReceivePacketStatus::Finished);
        }

        if let Some(packet) = self.output_queue.pop_front() {
            return Ok(ReceivePacketStatus::Packet(packet));
        }

        if let Some(frame) = self.input_queue.pop_front() {
            match self.try_emit_supported_packet(&frame) {
                Ok(Some(packet)) => return Ok(ReceivePacketStatus::Packet(packet)),
                Ok(None) => {
                    // The input is outside the subset the minimal encoder can honestly encode
                    // (64x64, every visible sample == the 128 DC predictor). Retire it without
                    // a packet — never a canned packet that ignores the input.
                    if self.state == EncoderState::Draining && self.input_queue.is_empty() {
                        self.state = EncoderState::Finished;
                        return Ok(ReceivePacketStatus::Finished);
                    }
                    return Ok(ReceivePacketStatus::NeedMoreData);
                }
                Err(error) => {
                    self.state = EncoderState::Failed;
                    self.input_queue.clear();
                    return Err(error);
                }
            }
        }

        if self.state == EncoderState::Draining {
            self.state = EncoderState::Finished;
            return Ok(ReceivePacketStatus::Finished);
        }

        Ok(ReceivePacketStatus::NeedMoreData)
    }

    /// Produces a coded packet for the input subset the minimal encoder can encode losslessly:
    /// a 64x64 frame whose every visible Y/U/V sample is the 128 no-neighbour DC predictor. Such
    /// a frame has zero residual, so the skip frame's flat-128 reconstruction equals the input.
    /// The frame is muxed at the configured fixed quantizer [`EncoderConfig::qp`](crate::EncoderConfig::qp).
    ///
    /// Returns `Ok(None)` for any other frame (a different size or any non-128 sample), or when
    /// the configured `qp` is outside the range whose coefficient CDF q-context the encoder
    /// currently models (see [`SUPPORTED_SKIP_QP`]) — the encoder cannot yet honestly encode it,
    /// so the caller retires it without a packet rather than emit output that would not decode.
    ///
    /// # Errors
    /// Returns the underlying [`Error`] if the skip emitter fails to assemble the container.
    fn try_emit_supported_packet(&self, frame: &QueuedFrame) -> Result<Option<Packet>> {
        let size = frame.info.visible_luma_size();
        if size.width() != SUPPORTED_FRAME_DIMENSION || size.height() != SUPPORTED_FRAME_DIMENSION {
            return Ok(None);
        }
        // The quantizer comes from the RateController seam (a constant-QP controller for the
        // minimal encoder); a rate-controlled implementation would choose it per frame here.
        let base_q_idx = self.rate_controller.frame_base_q_idx();
        if !SUPPORTED_SKIP_QP.contains(&base_q_idx) {
            return Ok(None);
        }
        let is_flat_predictor =
            |plane: &[u8]| plane.iter().all(|&s| s == NO_NEIGHBOUR_DC_PREDICTOR_8BIT);
        if !is_flat_predictor(&frame.y)
            || !is_flat_predictor(&frame.u)
            || !is_flat_predictor(&frame.v)
        {
            return Ok(None);
        }
        // The packet carries one coded access unit (the Annex B temporal unit), not a full IVF
        // file — so a consumer can mux multiple packets into a stream.
        let data =
            crate::general_intra_trace::emit_minimal_intra_skip_temporal_unit_with_base_q_idx(
                base_q_idx,
            )?;
        Ok(Some(Packet { data }))
    }

    /// Flushes the encoder, signalling end of input.
    ///
    /// # Errors
    /// Returns [`Error::State`] if the context is failed.
    pub fn flush(&mut self) -> Result<FlushStatus> {
        match self.state {
            EncoderState::Accepting => {
                if self.input_queue.is_empty() && self.output_queue.is_empty() {
                    self.state = EncoderState::Finished;
                    Ok(FlushStatus::Finished)
                } else {
                    self.state = EncoderState::Draining;
                    Ok(FlushStatus::Draining {
                        queued_frames: self.input_queue.len(),
                    })
                }
            }
            EncoderState::Draining => Ok(FlushStatus::Draining {
                queued_frames: self.input_queue.len(),
            }),
            EncoderState::Finished => Ok(FlushStatus::Finished),
            EncoderState::Failed => Err(self.state_error(EncoderOperation::Flush)),
        }
    }

    fn state_error(&self, operation: EncoderOperation) -> Error {
        Error::State {
            operation,
            state: self.state,
        }
    }

    fn validate_frame_info(&self, info: FrameInfo) -> Result<()> {
        let visible_luma_size = info.visible_luma_size();
        if (self.config.width != 0 || self.config.height != 0)
            && (visible_luma_size.width() != self.config.width as usize
                || visible_luma_size.height() != self.config.height as usize)
        {
            return Err(Error::InputFrameSizeMismatch {
                expected_width: self.config.width,
                expected_height: self.config.height,
                actual: visible_luma_size,
            });
        }
        if info.bit_depth() != self.config.bit_depth {
            return Err(Error::InputFrameBitDepthMismatch {
                expected: self.config.bit_depth,
                actual: info.bit_depth(),
            });
        }
        if info.chroma_subsampling() != self.config.chroma_subsampling {
            return Err(Error::InputFrameChromaSubsamplingMismatch {
                expected: self.config.chroma_subsampling,
                actual: info.chroma_subsampling(),
            });
        }
        Ok(())
    }

    #[cfg(test)]
    fn enter_failed_for_test(&mut self) {
        self.state = EncoderState::Failed;
        self.input_queue.clear();
        self.output_queue.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::config::{BitDepth, ChromaSubsampling};
    use splot_parallel::ThreadCount;
    use splot_recon::{PlaneId, PlaneRect, PlaneSize};

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(0, 0, width, height).unwrap()
    }

    fn frame<'a>(y: &'a [u8], u: &'a [u8], v: &'a [u8]) -> Frame<'a> {
        crate::frame::Frame::from_planes(
            crate::frame::FrameInfo::yuv420_8bit(crate::frame::FrameId::new(0), size(2, 2)),
            crate::frame::FramePlanesInput::yuv(
                crate::frame::FramePlaneInput::new(y, 2, rect(2, 2)),
                crate::frame::FramePlaneInput::new(u, 1, rect(1, 1)),
                crate::frame::FramePlaneInput::new(v, 1, rect(1, 1)),
            ),
        )
        .unwrap()
    }

    fn frame_64x64<'a>(y: &'a [u8], u: &'a [u8], v: &'a [u8]) -> Frame<'a> {
        crate::frame::Frame::from_planes(
            crate::frame::FrameInfo::yuv420_8bit(crate::frame::FrameId::new(0), size(64, 64)),
            crate::frame::FramePlanesInput::yuv(
                crate::frame::FramePlaneInput::new(y, 64, rect(64, 64)),
                crate::frame::FramePlaneInput::new(u, 32, rect(32, 32)),
                crate::frame::FramePlaneInput::new(v, 32, rect(32, 32)),
            ),
        )
        .unwrap()
    }

    #[test]
    fn receive_packet_encodes_a_64x64_all_128_frame_to_the_skip_packet() {
        let mut ctx =
            Context::new(EncoderConfig::new(64, 64), EncoderRuntimeConfig::default()).unwrap();
        let y = [128_u8; 64 * 64];
        let u = [128_u8; 32 * 32];
        let v = [128_u8; 32 * 32];
        assert!(matches!(
            ctx.send_frame(&frame_64x64(&y, &u, &v)).unwrap(),
            SendFrameStatus::Accepted { .. }
        ));
        assert!(matches!(ctx.flush().unwrap(), FlushStatus::Draining { .. }));
        // The public lifecycle now yields a REAL packet — one coded access unit (the Annex B
        // temporal unit of the decode-proven minimal skip frame), not a full IVF file.
        let status = ctx.receive_packet().unwrap();
        assert!(
            matches!(&status, ReceivePacketStatus::Packet(packet)
                if !packet.data.is_empty()
                    && packet.data
                        == crate::general_intra_trace::emit_minimal_intra_skip_temporal_unit()
                            .unwrap()),
            "expected the minimal skip access unit, got {status:?}"
        );
        // The single frame is consumed; the next pull finishes the stream.
        assert_eq!(ctx.receive_packet().unwrap(), ReceivePacketStatus::Finished);
        assert_eq!(ctx.state(), EncoderState::Finished);
    }

    #[test]
    fn receive_packet_retires_a_non_flat_64x64_frame_without_a_packet() {
        let mut ctx =
            Context::new(EncoderConfig::new(64, 64), EncoderRuntimeConfig::default()).unwrap();
        let mut y = [128_u8; 64 * 64];
        y[0] = 100; // a single non-predictor sample: not honestly encodable yet.
        let u = [128_u8; 32 * 32];
        let v = [128_u8; 32 * 32];
        assert!(matches!(
            ctx.send_frame(&frame_64x64(&y, &u, &v)).unwrap(),
            SendFrameStatus::Accepted { .. }
        ));
        assert!(matches!(ctx.flush().unwrap(), FlushStatus::Draining { .. }));
        // No canned packet: the unsupported frame is retired and the stream finishes.
        assert_eq!(ctx.receive_packet().unwrap(), ReceivePacketStatus::Finished);
        assert_eq!(ctx.queued_output_packets(), 0);
    }

    #[test]
    fn context_exposes_config_and_threads() {
        let speed = SpeedPreset::try_from_u8(3).unwrap();
        let runtime = EncoderRuntimeConfig::new(ThreadCount::from(4usize)).with_speed_preset(speed);
        let ctx = Context::new(EncoderConfig::new(1920, 1080), runtime).unwrap();
        assert_eq!(ctx.config().width, 1920);
        assert_eq!(ctx.config().height, 1080);
        assert_eq!(ctx.threads().get(), 4);
        assert_eq!(ctx.requested_threads(), ThreadCount::from(4usize));
        assert_eq!(ctx.speed_preset(), speed);
        assert_eq!(ctx.state(), EncoderState::Accepting);
        assert_eq!(ctx.queued_input_frames(), 0);
        assert_eq!(ctx.queued_output_packets(), 0);
        assert_eq!(ctx.input_queue_capacity(), 1);
        assert_eq!(ctx.output_queue_capacity(), 0);
    }

    #[test]
    fn receive_before_input_needs_more_data() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        assert_eq!(
            ctx.receive_packet().unwrap(),
            ReceivePacketStatus::NeedMoreData
        );
        assert_eq!(ctx.state(), EncoderState::Accepting);
        assert_eq!(ctx.queued_input_frames(), 0);
    }

    #[test]
    fn send_frame_accepts_until_bounded_input_queue_is_full() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)).unwrap(),
            SendFrameStatus::Accepted {
                queued_frames: 1,
                queue_capacity: 1,
            }
        ));
        let retry = frame(&y, &u, &v);
        assert_eq!(
            ctx.send_frame(&retry).unwrap(),
            SendFrameStatus::QueueFull {
                queued_frames: 1,
                queue_capacity: 1,
            }
        );
        assert_eq!(ctx.state(), EncoderState::Accepting);
        assert_eq!(ctx.queued_input_frames(), 1);
        assert_eq!(
            ctx.receive_packet().unwrap(),
            ReceivePacketStatus::NeedMoreData
        );
        assert!(matches!(
            ctx.send_frame(&retry).unwrap(),
            SendFrameStatus::Accepted { .. }
        ));
    }

    #[test]
    fn send_frame_rejects_config_size_mismatch_without_queueing() {
        let mut ctx = Context::new(
            EncoderConfig::new(1920, 1080),
            EncoderRuntimeConfig::default(),
        )
        .unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)),
            Err(Error::InputFrameSizeMismatch {
                expected_width: 1920,
                expected_height: 1080,
                actual,
            }) if actual == size(2, 2)
        ));
        assert_eq!(ctx.state(), EncoderState::Accepting);
        assert_eq!(ctx.queued_input_frames(), 0);
    }

    #[test]
    fn send_frame_rejects_config_format_mismatch_without_queueing() {
        let config = EncoderConfig {
            bit_depth: BitDepth::Ten,
            ..EncoderConfig::default()
        };
        let mut ctx = Context::new(config, EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)),
            Err(Error::InputFrameBitDepthMismatch {
                expected: BitDepth::Ten,
                actual: BitDepth::Eight,
            })
        ));
        assert_eq!(ctx.queued_input_frames(), 0);

        let config = EncoderConfig {
            chroma_subsampling: ChromaSubsampling::Yuv444,
            ..EncoderConfig::default()
        };
        let mut ctx = Context::new(config, EncoderRuntimeConfig::default()).unwrap();
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)),
            Err(Error::InputFrameChromaSubsamplingMismatch {
                expected: ChromaSubsampling::Yuv444,
                actual: ChromaSubsampling::Yuv420,
            })
        ));
        assert_eq!(ctx.queued_input_frames(), 0);
    }

    #[test]
    fn receive_retires_queued_input_without_fake_packet_before_flush() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)).unwrap(),
            SendFrameStatus::Accepted { .. }
        ));
        assert_eq!(
            ctx.receive_packet().unwrap(),
            ReceivePacketStatus::NeedMoreData
        );
        assert_eq!(ctx.state(), EncoderState::Accepting);
        assert_eq!(ctx.queued_input_frames(), 0);
        assert_eq!(ctx.queued_output_packets(), 0);
    }

    #[test]
    fn syntax_and_header_planning_do_not_enable_packet_output() {
        let mut ctx = Context::new(
            EncoderConfig::new(2, 2),
            EncoderRuntimeConfig::default()
                .with_speed_preset(SpeedPreset::try_from_u8(10).unwrap()),
        )
        .unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        let input = frame(&y, &u, &v);

        assert!(crate::header_plan::MinimalHeaderPlan::new(ctx.config(), input.info()).is_ok());
        let residual = crate::residual::ResidualBlock::from_plane_prediction(
            PlaneId::Y,
            input.y(),
            rect(2, 2),
            &[0; 4],
            2,
        )
        .unwrap();
        assert_eq!(residual.samples(), &[0, 0, 0, 0]);
        let transformed = crate::forward_transform::ForwardTransformBlock::dct_dct_4x4_dc_only(
            PlaneId::Y,
            rect(4, 4),
            &[0; 16],
        )
        .unwrap();
        assert_eq!(transformed.coefficients(), &[0; 16]);
        let quantized = crate::quantization::QuantizedTransformBlock::dct_dct_4x4_dc_only(
            &transformed,
            crate::quantization::FixedQuantizationParams::new(splot_recon::BitDepth::Eight, 0)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(quantized.quantized(), &[0; 16]);
        assert_eq!(quantized.dequantized(), &[0; 16]);
        let tokenized =
            crate::coefficient_tokenization::tokenize_quantized_4x4_dct_dct_dc_only(&quantized)
                .unwrap();
        assert_eq!(tokenized.eob(), 0);
        assert_eq!(tokenized.tokens().len(), 1);

        assert!(matches!(
            ctx.send_frame(&input).unwrap(),
            SendFrameStatus::Accepted { .. }
        ));
        assert_eq!(ctx.queued_output_packets(), 0);
        assert_eq!(
            ctx.receive_packet().unwrap(),
            ReceivePacketStatus::NeedMoreData
        );
        assert_eq!(ctx.queued_output_packets(), 0);
    }

    #[test]
    fn flush_without_input_finishes_and_is_repeatable() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        assert_eq!(ctx.flush().unwrap(), FlushStatus::Finished);
        assert_eq!(ctx.state(), EncoderState::Finished);
        assert_eq!(ctx.flush().unwrap(), FlushStatus::Finished);
        assert_eq!(ctx.receive_packet().unwrap(), ReceivePacketStatus::Finished);
    }

    #[test]
    fn flush_drains_queued_input_to_finished_without_packets() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)).unwrap(),
            SendFrameStatus::Accepted { .. }
        ));
        assert_eq!(
            ctx.flush().unwrap(),
            FlushStatus::Draining { queued_frames: 1 }
        );
        assert_eq!(ctx.state(), EncoderState::Draining);
        assert_eq!(
            ctx.flush().unwrap(),
            FlushStatus::Draining { queued_frames: 1 }
        );
        assert_eq!(ctx.receive_packet().unwrap(), ReceivePacketStatus::Finished);
        assert_eq!(ctx.state(), EncoderState::Finished);
        assert_eq!(ctx.queued_input_frames(), 0);
        assert_eq!(ctx.queued_output_packets(), 0);
    }

    #[test]
    fn send_after_flush_is_typed_state_error() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)).unwrap(),
            SendFrameStatus::Accepted { .. }
        ));
        assert!(matches!(ctx.flush().unwrap(), FlushStatus::Draining { .. }));
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)),
            Err(Error::State {
                operation: EncoderOperation::SendFrame,
                state: EncoderState::Draining,
            })
        ));
    }

    #[test]
    fn send_after_finished_is_typed_state_error() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert_eq!(ctx.flush().unwrap(), FlushStatus::Finished);
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)),
            Err(Error::State {
                operation: EncoderOperation::SendFrame,
                state: EncoderState::Finished,
            })
        ));
    }

    #[test]
    fn failed_state_rejects_all_lifecycle_operations() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        ctx.enter_failed_for_test();
        assert_eq!(ctx.state(), EncoderState::Failed);
        assert!(matches!(
            ctx.send_frame(&frame(&y, &u, &v)),
            Err(Error::State {
                operation: EncoderOperation::SendFrame,
                state: EncoderState::Failed,
            })
        ));
        assert!(matches!(
            ctx.receive_packet(),
            Err(Error::State {
                operation: EncoderOperation::ReceivePacket,
                state: EncoderState::Failed,
            })
        ));
        assert!(matches!(
            ctx.flush(),
            Err(Error::State {
                operation: EncoderOperation::Flush,
                state: EncoderState::Failed,
            })
        ));
    }

    #[test]
    fn default_runtime_config_is_auto() {
        assert_eq!(
            EncoderRuntimeConfig::default().thread_count,
            ThreadCount::Auto
        );
    }

    #[test]
    fn zero_threads_maps_to_auto() {
        let runtime = EncoderRuntimeConfig::new(ThreadCount::from(0usize));
        assert_eq!(runtime.thread_count, ThreadCount::Auto);
        let ctx = Context::new(EncoderConfig::default(), runtime).unwrap();
        assert!(ctx.threads().get() >= 1);
    }

    #[test]
    fn thread_count_does_not_change_bitstream_config() {
        let a = Context::new(
            EncoderConfig::new(640, 480),
            EncoderRuntimeConfig::new(ThreadCount::from(1usize)),
        )
        .unwrap();
        let b = Context::new(
            EncoderConfig::new(640, 480),
            EncoderRuntimeConfig::new(ThreadCount::from(4usize)),
        )
        .unwrap();
        assert_eq!(a.config(), b.config());
    }

    #[test]
    fn context_pool_runs_parallel_iterators_via_prelude_deterministically() {
        // splot-encode has no direct `rayon` dependency: parallel iteration is
        // reached only through `splot_parallel::prelude`, driven on the context's
        // own `WorkerPool` (not Rayon's global pool). Indexed map + ordered collect
        // is deterministic regardless of worker count.
        use splot_parallel::prelude::*;

        let ctx = Context::new(
            EncoderConfig::default(),
            EncoderRuntimeConfig::new(ThreadCount::from(4usize)),
        )
        .unwrap();
        let doubled: Vec<u64> = ctx
            .pool()
            .install(|| (0..16u64).into_par_iter().map(|x| x * 2).collect());
        assert_eq!(doubled, (0..16u64).map(|x| x * 2).collect::<Vec<_>>());
    }
}
