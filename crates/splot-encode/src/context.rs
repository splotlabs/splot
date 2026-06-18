// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Push/pull encoder API surface.
//!
//! The lifecycle is implemented as deterministic state plumbing, but no coded
//! packet production exists yet.

use core::num::NonZeroUsize;
use std::collections::VecDeque;

use splot_parallel::{ThreadCount, WorkerPool};

use crate::config::EncoderConfig;
use crate::error::{Error, Result};
use crate::frame::{Frame, FrameInfo};
use crate::runtime::EncoderRuntimeConfig;

const INPUT_QUEUE_CAPACITY: usize = 1;
const OUTPUT_QUEUE_CAPACITY: usize = 0;

/// An output coded packet.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Coded bytes for one access unit.
    pub data: Vec<u8>,
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
    pool: WorkerPool,
    state: EncoderState,
    input_queue: VecDeque<FrameInfo>,
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
        Ok(Self {
            config,
            runtime,
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
    pub fn send_frame(&mut self, frame: Frame<'_>) -> Result<SendFrameStatus> {
        if self.state != EncoderState::Accepting {
            return Err(self.state_error(EncoderOperation::SendFrame));
        }

        if self.input_queue.len() >= INPUT_QUEUE_CAPACITY {
            return Ok(SendFrameStatus::QueueFull {
                queued_frames: self.input_queue.len(),
                queue_capacity: INPUT_QUEUE_CAPACITY,
            });
        }

        self.input_queue.push_back(frame.info());
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

        if self.input_queue.pop_front().is_some() {
            if self.state == EncoderState::Draining && self.input_queue.is_empty() {
                self.state = EncoderState::Finished;
                return Ok(ReceivePacketStatus::Finished);
            }
            return Ok(ReceivePacketStatus::NeedMoreData);
        }

        if self.state == EncoderState::Draining {
            self.state = EncoderState::Finished;
            return Ok(ReceivePacketStatus::Finished);
        }

        Ok(ReceivePacketStatus::NeedMoreData)
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
    use splot_parallel::ThreadCount;
    use splot_recon::{PlaneRect, PlaneSize};

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

    #[test]
    fn context_exposes_config_and_threads() {
        let runtime = EncoderRuntimeConfig::new(ThreadCount::from(4usize));
        let ctx = Context::new(EncoderConfig::new(1920, 1080), runtime).unwrap();
        assert_eq!(ctx.config().width, 1920);
        assert_eq!(ctx.config().height, 1080);
        assert_eq!(ctx.threads().get(), 4);
        assert_eq!(ctx.requested_threads(), ThreadCount::from(4usize));
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
        assert_eq!(
            ctx.send_frame(frame(&y, &u, &v)).unwrap(),
            SendFrameStatus::Accepted {
                queued_frames: 1,
                queue_capacity: 1,
            }
        );
        assert_eq!(
            ctx.send_frame(frame(&y, &u, &v)).unwrap(),
            SendFrameStatus::QueueFull {
                queued_frames: 1,
                queue_capacity: 1,
            }
        );
        assert_eq!(ctx.state(), EncoderState::Accepting);
        assert_eq!(ctx.queued_input_frames(), 1);
    }

    #[test]
    fn receive_retires_queued_input_without_fake_packet_before_flush() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert!(matches!(
            ctx.send_frame(frame(&y, &u, &v)).unwrap(),
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
            ctx.send_frame(frame(&y, &u, &v)).unwrap(),
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
            ctx.send_frame(frame(&y, &u, &v)).unwrap(),
            SendFrameStatus::Accepted { .. }
        ));
        assert!(matches!(ctx.flush().unwrap(), FlushStatus::Draining { .. }));
        assert!(matches!(
            ctx.send_frame(frame(&y, &u, &v)),
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
            ctx.send_frame(frame(&y, &u, &v)),
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
            ctx.send_frame(frame(&y, &u, &v)),
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
