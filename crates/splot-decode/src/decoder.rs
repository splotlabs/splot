// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A streaming decoder: send temporal units, take pictures that share the
//! decoder's sample storage.
//!
//! Feature tracking: `DECODE-STREAMING-API`, `DECODE-ZERO-COPY-PICTURE`.

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};
use splot_core::headers::content_interpretation::ContentInterpretation;
use splot_core::headers::metadata::MetadataUnit;
use splot_core::ivf::IvfHeader;
use splot_parallel::{FrameDelay, ThreadCount, WatermarkCell, WorkerPool};
use splot_recon::{
    BitDepth, DecodedFrameInfo, PixelFormat, PlaneId, PlanePool, PlaneRef, ReconSample, SharedFrame,
};

use crate::bitstream::byte_stream::{SourceUnit, UnitBytes, UnitSource, fill_buffer};
use crate::error::{DecodeError, DecoderStateError, Result};
use crate::output::film_grain::{DisplayFrame, frame_for_output};
use crate::pipeline::{DecodeSession, PipelineDecodedFrame, PipelineFrame};
use crate::{DecodeLimitName, DecodeLimits, DecodeOptions};

/// How a [`Decoder`] runs.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DecoderSettings {
    /// Worker threads the decoder's own pool runs.
    pub threads: ThreadCount,
    /// Frames the decoder may decode in parallel; also how many temporal
    /// units it holds before [`Decoder::send_data`] reports
    /// [`SendStatus::Full`].
    pub frame_delay: FrameDelay,
    /// Resource limits. [`DecodeLimitName::MaxInputBytes`] counts every byte
    /// sent to the decoder.
    pub limits: DecodeLimits,
}

impl DecoderSettings {
    /// Returns a copy with the worker-thread count replaced.
    #[must_use]
    pub const fn with_threads(mut self, threads: ThreadCount) -> Self {
        self.threads = threads;
        self
    }

    /// Returns a copy with the frame delay replaced.
    #[must_use]
    pub const fn with_frame_delay(mut self, frame_delay: FrameDelay) -> Self {
        self.frame_delay = frame_delay;
        self
    }

    /// Returns a copy with the resource limits replaced.
    #[must_use]
    pub const fn with_limits(mut self, limits: DecodeLimits) -> Self {
        self.limits = limits;
        self
    }
}

/// Caller data that goes with one temporal unit to the pictures it codes.
///
/// A picture gets the data of the temporal unit that coded it; a
/// show-existing picture gets the data of the unit that showed it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct DataProps {
    /// Presentation timestamp, in the caller's time base.
    pub pts: i64,
    /// Duration, in the caller's time base.
    pub duration: i64,
    /// An opaque caller value.
    pub user_data: u64,
}

impl DataProps {
    /// Returns a copy with the presentation timestamp replaced.
    #[must_use]
    pub const fn with_pts(mut self, pts: i64) -> Self {
        self.pts = pts;
        self
    }

    /// Returns a copy with the duration replaced.
    #[must_use]
    pub const fn with_duration(mut self, duration: i64) -> Self {
        self.duration = duration;
        self
    }

    /// Returns a copy with the opaque caller value replaced.
    #[must_use]
    pub const fn with_user_data(mut self, user_data: u64) -> Self {
        self.user_data = user_data;
        self
    }
}

/// What [`Decoder::send_data`] did with a temporal unit.
#[must_use]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendStatus {
    /// The decoder took a copy of the unit.
    Accepted,
    /// The decoder holds as many units as it takes. Take pictures with
    /// [`Decoder::get_picture`], then send the same unit again.
    Full,
}

/// A decoded picture.
///
/// The planes are the decoder's own sample storage, shared without a copy.
/// Each plane's storage starts on a [`PLANE_ALIGNMENT`](crate::PLANE_ALIGNMENT)-byte
/// boundary and its row stride is a multiple of that many bytes; the visible
/// area may start inside it (see [`PlaneRef::visible_rect`]). The decoder never
/// writes the planes again; dropping the last picture that holds them gives
/// the storage back to the decoder. Film grain, when the stream signals it, is
/// applied to a new buffer with the same layout guarantees.
#[derive(Debug)]
pub struct Picture {
    samples: Option<Samples>,
    info: DecodedFrameInfo,
    props: DataProps,
    content_interpretation: Option<ContentInterpretation>,
    metadata: Option<Arc<[MetadataUnit]>>,
    pool: Option<Arc<PlanePool>>,
}

#[derive(Debug)]
enum Samples {
    Eight(SharedFrame<u8>),
    Ten(SharedFrame<u16>),
}

/// The samples of one picture plane, cropped to the visible area.
#[derive(Clone, Copy, Debug)]
pub enum PictureSamples<'a> {
    /// Eight-bit samples.
    Eight(PlaneRef<'a, u8>),
    /// Ten-bit samples, one per `u16`, in the low bits.
    Ten(PlaneRef<'a, u16>),
}

impl Picture {
    fn new(frame: &PipelineFrame, pool: Option<Arc<PlanePool>>) -> Result<Self> {
        let grain = frame.display_grain.as_ref();
        let samples = match frame.ready_frame()? {
            PipelineDecodedFrame::Eight(frame) => Samples::Eight(display(frame, grain)?),
            PipelineDecodedFrame::Ten(frame) => Samples::Ten(display(frame, grain)?),
        };
        let info = match &samples {
            Samples::Eight(frame) => frame.info(),
            Samples::Ten(frame) => frame.info(),
        };
        let metadata = &frame.output_effects.metadata;
        Ok(Self {
            samples: Some(samples),
            info,
            props: frame.props,
            content_interpretation: frame.output_effects.content_interpretation,
            metadata: (!metadata.is_empty()).then(|| Arc::from(metadata.as_slice())),
            pool,
        })
    }

    /// Visible width in luma samples.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.info.visible_luma_rect().width()
    }

    /// Visible height in luma samples.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.info.visible_luma_rect().height()
    }

    /// Sample bit depth.
    #[must_use]
    pub const fn bit_depth(&self) -> BitDepth {
        self.info.bit_depth()
    }

    /// Chroma subsampling, or monochrome.
    #[must_use]
    pub const fn pixel_format(&self) -> PixelFormat {
        self.info.pixel_format()
    }

    /// The visible samples of `plane`, or `None` for a chroma plane of a
    /// monochrome picture.
    #[must_use]
    pub fn plane(&self, plane: PlaneId) -> Option<PictureSamples<'_>> {
        Some(match self.samples.as_ref()? {
            Samples::Eight(frame) => PictureSamples::Eight(frame.plane(plane)?.as_plane_ref()),
            Samples::Ten(frame) => PictureSamples::Ten(frame.plane(plane)?.as_plane_ref()),
        })
    }

    /// The caller data of the temporal unit that coded or showed this picture.
    #[must_use]
    pub const fn props(&self) -> DataProps {
        self.props
    }

    /// The content interpretation (color description, timing) in effect for
    /// this picture, from AV2 § 5.15.
    #[must_use]
    pub const fn content_interpretation(&self) -> Option<&ContentInterpretation> {
        self.content_interpretation.as_ref()
    }

    /// The metadata units (HDR, ITU-T T.35 and others) attached to this
    /// picture, from AV2 § 5.17.
    #[must_use]
    pub fn metadata(&self) -> &[MetadataUnit] {
        self.metadata.as_deref().unwrap_or_default()
    }
}

impl Drop for Picture {
    fn drop(&mut self) {
        let (Some(samples), Some(pool)) = (self.samples.take(), self.pool.as_ref()) else {
            return;
        };
        match samples {
            Samples::Eight(frame) => recycle(frame, pool),
            Samples::Ten(frame) => recycle(frame, pool),
        }
    }
}

fn display<T: ReconSample>(
    frame: SharedFrame<T>,
    grain: Option<&crate::pipeline::ActiveFilmGrain>,
) -> Result<SharedFrame<T>> {
    if grain.is_none() {
        return Ok(frame);
    }
    match frame_for_output(frame.get(), grain)? {
        DisplayFrame::Owned(grained) => Ok(SharedFrame::new(*grained)),
        DisplayFrame::Borrowed(_) => Ok(frame),
    }
}

fn recycle<T: ReconSample>(frame: SharedFrame<T>, pool: &Arc<PlanePool>) {
    if let Some(frame) = frame.into_inner() {
        frame.into_plane_samples().with_pool(Some(pool)).release();
    }
}

/// A streaming AV2 decoder.
///
/// Send one temporal unit (the OBUs of one IVF frame record, MP4 sample or
/// Matroska block) with [`Self::send_data`], take the pictures it produced
/// with [`Self::get_picture`], and call [`Self::drain`] at end of stream to
/// take the rest. The decoder runs on its own worker pool.
///
/// An error ends the session: the call that sees it returns it, and later
/// calls return [`DecoderStateError::Failed`].
///
/// ```no_run
/// # fn run(units: &[&[u8]]) -> splot_decode::Result<()> {
/// use splot_decode::{DataProps, Decoder, DecoderSettings, PlaneId, SendStatus};
///
/// let mut decoder = Decoder::new(DecoderSettings::default())?;
/// for (pts, unit) in (0..).zip(units) {
///     while decoder.send_data(unit, DataProps::default().with_pts(pts))? == SendStatus::Full {
///         while let Some(picture) = decoder.get_picture()? {
///             let _luma = picture.plane(PlaneId::Y);
///         }
///     }
///     while let Some(picture) = decoder.get_picture()? {
///         let _luma = picture.plane(PlaneId::Y);
///     }
/// }
/// decoder.drain();
/// while let Some(picture) = decoder.get_picture()? {
///     let _luma = picture.plane(PlaneId::Y);
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Decoder {
    shared: Arc<Shared>,
    limits: DecodeLimits,
    /// Dropped after [`Decoder::drop`] has waited for the driver.
    _pool: WorkerPool,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    /// Wakes a caller that waits for a picture or for the driver to stop.
    caller: Condvar,
    /// Counts caller events; the driver waits for it to change.
    events: WatermarkCell,
}

#[derive(Debug)]
struct State {
    input: VecDeque<SourceUnit>,
    input_capacity: usize,
    buffers: Vec<Arc<Vec<u8>>>,
    output: VecDeque<Picture>,
    output_capacity: usize,
    sent_bytes: u64,
    events: usize,
    draining: bool,
    closing: bool,
    outcome: Outcome,
}

#[derive(Debug)]
enum Outcome {
    Running,
    Finished,
    Failed(Option<DecodeError>),
}

impl Shared {
    /// Counts one caller event, so a waiting driver looks at the state again.
    fn bump(&self, mut state: parking_lot::MutexGuard<'_, State>) {
        state.events += 1;
        let events = state.events;
        drop(state);
        self.events.publish(events);
    }

    /// Records the driver's outcome and wakes the caller.
    fn finish(&self, result: Result<()>) {
        let mut state = self.state.lock();
        if matches!(state.outcome, Outcome::Running) {
            state.outcome = match result {
                Ok(()) => Outcome::Finished,
                Err(error) => Outcome::Failed(Some(error)),
            };
        }
        state.input.clear();
        drop(state);
        self.caller.notify_all();
    }
}

/// Fails the session if the driver unwinds before it records an outcome.
struct DriverGuard(Arc<Shared>);

impl Drop for DriverGuard {
    fn drop(&mut self) {
        self.0.finish(Err(DecodeError::State {
            source: DecoderStateError::Failed,
        }));
    }
}

impl Decoder {
    /// Opens a decoder and starts its worker pool.
    ///
    /// # Errors
    /// Returns [`DecodeError::Pool`] if the worker pool cannot be built.
    pub fn new(settings: DecoderSettings) -> Result<Self> {
        let pool = WorkerPool::new(settings.threads)?;
        let frame_delay = settings.frame_delay.resolve(pool.threads());
        let capacity = frame_delay.get().max(2);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                input: VecDeque::with_capacity(capacity),
                input_capacity: capacity,
                buffers: Vec::new(),
                output: VecDeque::with_capacity(capacity + MAX_OUTPUTS_PER_UNIT),
                output_capacity: capacity,
                sent_bytes: 0,
                events: 0,
                draining: false,
                closing: false,
                outcome: Outcome::Running,
            }),
            caller: Condvar::new(),
            events: WatermarkCell::new(),
        });
        let driver = Arc::clone(&shared);
        let options = DecodeOptions::new(settings.limits);
        pool.spawn(move || {
            let guard = DriverGuard(driver);
            let session = DecodeSession::detaching_held(frame_delay);
            let mut units = ChannelUnits(Arc::clone(&guard.0));
            let result = crate::pipeline::emit_frames(
                &mut units,
                &options,
                &session,
                |_| Ok(()),
                |frame| {
                    guard.0.push_picture(Picture::new(frame, session.planes())?);
                    Ok(())
                },
            );
            guard.0.finish(result);
        });
        Ok(Self {
            shared,
            limits: settings.limits,
            _pool: pool,
        })
    }

    /// Sends one whole temporal unit. The decoder copies it.
    ///
    /// # Errors
    /// Returns the decode error that ended the session,
    /// [`DecoderStateError::Failed`] after that error was returned,
    /// [`DecoderStateError::Draining`] after [`Self::drain`], or
    /// [`DecodeError::Limit`] when the unit takes the input past
    /// [`DecodeLimitName::MaxInputBytes`].
    pub fn send_data(&mut self, data: &[u8], props: DataProps) -> Result<SendStatus> {
        let mut state = self.shared.state.lock();
        state.take_failure()?;
        if state.draining || !matches!(state.outcome, Outcome::Running) {
            return Err(DecoderStateError::Draining.into());
        }
        if state.input.len() >= state.input_capacity {
            return Ok(SendStatus::Full);
        }
        if data.is_empty() {
            return Ok(SendStatus::Accepted);
        }
        let base = state.sent_bytes;
        let sent = base.saturating_add(data.len() as u64);
        self.limits.ensure(DecodeLimitName::MaxInputBytes, sent)?;
        state.sent_bytes = sent;
        let bytes = UnitBytes::new(fill_buffer(&mut state.buffers, data), base);
        state.input.push_back(SourceUnit {
            bytes,
            ivf_frame: None,
            props,
        });
        self.shared.bump(state);
        Ok(SendStatus::Accepted)
    }

    /// Takes the next picture in display order.
    ///
    /// Returns `None` when the decoder needs more data to make progress, or,
    /// after [`Self::drain`], when every picture was taken. Blocks while the
    /// decoder holds as many units as it takes, or drains, until a picture is
    /// ready.
    ///
    /// # Errors
    /// Returns the decode error that ended the session, then
    /// [`DecoderStateError::Failed`] on later calls.
    pub fn get_picture(&mut self) -> Result<Option<Picture>> {
        let mut state = self.shared.state.lock();
        loop {
            if let Some(picture) = state.output.pop_front() {
                self.shared.bump(state);
                return Ok(Some(picture));
            }
            match state.outcome {
                Outcome::Running => {}
                Outcome::Finished => return Ok(None),
                Outcome::Failed(_) => return state.take_failure().map(|()| None),
            }
            if !state.draining && state.input.len() < state.input_capacity {
                return Ok(None);
            }
            self.shared.caller.wait(&mut state);
        }
    }

    /// Ends the input. The decoder decodes what it holds, and
    /// [`Self::get_picture`] returns the remaining pictures.
    pub fn drain(&mut self) {
        let mut state = self.shared.state.lock();
        state.draining = true;
        self.shared.bump(state);
    }
}

impl Drop for Decoder {
    /// Stops the driver and waits for it, so no decoder code runs on the
    /// pool after the decoder is gone.
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.closing = true;
        state.output.clear();
        self.shared.bump(state);
        let mut state = self.shared.state.lock();
        while matches!(state.outcome, Outcome::Running) {
            self.shared.caller.wait(&mut state);
        }
    }
}

impl State {
    /// Returns the session's error once, then [`DecoderStateError::Failed`].
    fn take_failure(&mut self) -> Result<()> {
        match &mut self.outcome {
            Outcome::Failed(error) => Err(error.take().unwrap_or(DecoderStateError::Failed.into())),
            Outcome::Running | Outcome::Finished => Ok(()),
        }
    }
}

/// The most pictures one temporal unit can release at once: a new sequence
/// flushes every pending and referenced frame.
const MAX_OUTPUTS_PER_UNIT: usize = 2 * splot_core::headers::sequence::MAX_REF_FRAMES + 2;

impl Shared {
    fn push_picture(&self, picture: Picture) {
        let mut state = self.state.lock();
        if !state.closing {
            state.output.push_back(picture);
        }
        drop(state);
        self.caller.notify_all();
    }
}

/// The temporal units a caller sent, read by the decode loop on the pool.
struct ChannelUnits(Arc<Shared>);

impl UnitSource for ChannelUnits {
    fn next_unit(&mut self, _: &mut Vec<Arc<Vec<u8>>>) -> Result<Option<SourceUnit>> {
        let shared = &self.0;
        loop {
            let snapshot = splot_parallel::pool_progress_snapshot();
            let _ = shared.events.current();
            let mut state = shared.state.lock();
            if state.closing {
                return Ok(None);
            }
            if state.output.len() < state.output_capacity {
                if let Some(unit) = state.input.pop_front() {
                    drop(state);
                    shared.caller.notify_all();
                    return Ok(Some(unit));
                }
                if state.draining {
                    return Ok(None);
                }
            }
            drop(state);
            splot_parallel::assist_pool_or_park(&snapshot);
        }
    }

    fn ivf_header(&self) -> Option<IvfHeader> {
        None
    }
}
