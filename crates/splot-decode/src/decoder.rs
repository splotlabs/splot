// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A streaming decoder: send temporal units, take pictures that share the
//! decoder's sample storage.
//!
//! Feature tracking: `DECODE-STREAMING-API`, `DECODE-ZERO-COPY-PICTURE`.

mod composition;
pub use composition::{AtlasComposite, AtlasCompositionError, AtlasCompositor};

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};
use splot_core::headers::content_interpretation::ContentInterpretation;
use splot_core::headers::frame::FrameType;
use splot_core::headers::metadata::MetadataUnit;
use splot_core::ivf::IvfHeader;
use splot_core::simd::{ResolvedBackend, SelectedBackend, SimdBackend, SimdPolicy, SimdScope};
use splot_core::span::ByteOffset;
use splot_core::types::TemporalLayerId;
use splot_parallel::{FrameDelay, ThreadCount};
use splot_recon::{
    BitDepth, DecodedFrame, DecodedFrameInfo, FramePlaneSamples, PixelFormat, PlaneId, PlanePool,
    PlaneRef, ReconSample, SharedFrame,
};

use crate::bitstream::byte_stream::{SourceUnit, UnitBytes, UnitSource, fill_buffer};
#[cfg(feature = "block-evidence")]
use crate::block_evidence::{BlockEvidenceCapture, BlockEvidenceRequest, BlockEvidenceState};
use crate::error::{DecodeError, DecoderStateError, Result};
use crate::pipeline::frame_store::{CodedFrameObserver, DecodeObserver};
use crate::pipeline::{ActiveFilmGrain, DecodeSession, PipelineDecodedFrame, PipelineFrame};
use crate::{
    DecodeLimitName, DecodeLimits, DecodeOptions, EmbeddedLayerId, EmbeddedLayerSelection,
    ExtendedLayerId, ExtendedLayerSelection, OperatingPointSelection, TemporalLayerSelection,
};

/// How a [`Decoder`] runs.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DecoderSettings {
    /// The threads the decoder runs.
    pub threads: ThreadCount,
    /// Frames the decoder may decode in parallel; also how many temporal
    /// units it holds before [`Decoder::send_data`] reports
    /// [`SendStatus::Full`].
    pub frame_delay: FrameDelay,
    /// Resource limits. [`DecodeLimitName::MaxInputBytes`] counts every byte
    /// sent since the decoder opened or [`Decoder::flush`] ran.
    pub limits: DecodeLimits,
    /// Leaves film grain out of the pictures; [`Picture::apply_film_grain`]
    /// adds it to a picture later.
    pub skip_film_grain: bool,
    /// Startup SIMD backend policy. Defaults to [`SimdPolicy::Auto`].
    pub simd_policy: SimdPolicy,
    /// Temporal enhancement layers to decode and emit. Defaults to [`TemporalLayerSelection::All`].
    pub temporal_layer_selection: TemporalLayerSelection,
    /// Embedded enhancement layers to decode and emit. Defaults to [`EmbeddedLayerSelection::All`].
    pub embedded_layer_selection: EmbeddedLayerSelection,
    /// Extended enhancement layers to decode and emit. Defaults to [`ExtendedLayerSelection::All`].
    pub extended_layer_selection: ExtendedLayerSelection,
    /// Operating-point selection within an `OBU_OPERATING_POINT_SET`. Defaults to
    /// [`OperatingPointSelection::None`].
    pub operating_point_selection: OperatingPointSelection,
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

    /// Returns a copy that leaves film grain out of the pictures (`true`) or
    /// applies it (`false`, the default).
    #[must_use]
    pub const fn with_skip_film_grain(mut self, skip: bool) -> Self {
        self.skip_film_grain = skip;
        self
    }

    /// Returns a copy with the startup SIMD backend policy replaced.
    #[must_use]
    pub const fn with_simd_policy(mut self, simd_policy: SimdPolicy) -> Self {
        self.simd_policy = simd_policy;
        self
    }

    /// Returns a copy with the temporal-layer selection policy replaced.
    #[must_use]
    pub const fn with_temporal_layer_selection(
        mut self,
        temporal_layer_selection: TemporalLayerSelection,
    ) -> Self {
        self.temporal_layer_selection = temporal_layer_selection;
        self
    }

    /// Returns a copy with the embedded-layer selection policy replaced.
    #[must_use]
    pub const fn with_embedded_layer_selection(
        mut self,
        embedded_layer_selection: EmbeddedLayerSelection,
    ) -> Self {
        self.embedded_layer_selection = embedded_layer_selection;
        self
    }

    /// Returns a copy with the extended-layer selection policy replaced.
    #[must_use]
    pub const fn with_extended_layer_selection(
        mut self,
        extended_layer_selection: ExtendedLayerSelection,
    ) -> Self {
        self.extended_layer_selection = extended_layer_selection;
        self
    }

    /// Returns a copy with the operating-point selection policy replaced.
    #[must_use]
    pub const fn with_operating_point_selection(
        mut self,
        operating_point_selection: OperatingPointSelection,
    ) -> Self {
        self.operating_point_selection = operating_point_selection;
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

/// The location of an OBU in a temporal unit submitted with [`Decoder::send_data`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ObuLocation {
    /// The [`DataProps::user_data`] value of the unit.
    pub user_data: u64,
    /// The offset from the start of the unit to the OBU header after its length prefix.
    pub offset: ByteOffset,
}

/// Where a coded frame obtains its base quantizer index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BaseQuantizerOrigin {
    /// Read from `quantization_params()` (AV2 § 5.18.6.1).
    Signaled,
    /// Computed from TIP reference quantizers (AV2 § 5.18.2).
    Derived,
    /// Copied from the bridge or inactive BRU reference (AV2 § 5.18.2).
    Inherited,
}

/// The coded frame's base quantizer index and its source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BaseQuantizer {
    /// The value of `base_q_idx` before segment or block deltas.
    pub index: u32,
    /// How the decoder obtained the value.
    pub origin: BaseQuantizerOrigin,
}

/// A coded frame or one of its continuation OBUs admitted by the decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CodedFrameEvent {
    /// An admitted frame, including a frame that produces no picture.
    #[non_exhaustive]
    Frame {
        /// The first coded OBU, also used by [`PictureProvenance::coded`].
        coded: ObuLocation,
        /// The active sequence header.
        sequence_header: ObuLocation,
        /// The coded extended layer.
        extended_layer_id: ExtendedLayerId,
        /// The coded embedded layer.
        embedded_layer_id: EmbeddedLayerId,
        /// The coded temporal layer.
        temporal_layer_id: TemporalLayerId,
        /// The known base quantizer, if available.
        base_quantizer: Option<BaseQuantizer>,
    },
    /// A tile group that continues a frame reported by [`Self::Frame`].
    #[non_exhaustive]
    Continuation {
        /// The first coded OBU of the frame.
        coded: ObuLocation,
        /// The continuation OBU.
        obu: ObuLocation,
    },
}

/// The AV2 input locations for a decoded picture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PictureProvenance {
    /// The frame OBU that coded the samples. This is the first tile group for a tiled frame.
    pub coded: ObuLocation,
    /// The frame OBU that caused display. This is `coded` unless a show-existing OBU caused display.
    pub display: ObuLocation,
    /// The sequence header in effect when the decoder decoded the coded frame.
    pub sequence_header: ObuLocation,
}

impl PictureProvenance {
    pub(crate) const fn direct(coded: ObuLocation, sequence_header: ObuLocation) -> Self {
        Self {
            coded,
            display: coded,
            sequence_header,
        }
    }

    #[cfg(test)]
    pub(crate) const TEST: Self = Self::direct(
        ObuLocation {
            user_data: 0,
            offset: ByteOffset::new(0),
        },
        ObuLocation {
            user_data: 0,
            offset: ByteOffset::new(0),
        },
    );
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
/// the storage back to the decoder, which reuses frame-sized planes for later
/// pictures and frees smaller ones. Film grain, when the stream signals it and
/// [`DecoderSettings::skip_film_grain`] is not set, is applied into planes from
/// that same storage, with the same layout guarantees.
#[derive(Debug)]
pub struct Picture {
    samples: Option<Samples>,
    info: DecodedFrameInfo,
    props: DataProps,
    provenance: PictureProvenance,
    content_interpretation: Option<ContentInterpretation>,
    metadata: Option<Arc<[MetadataUnit]>>,
    atlas: Vec<crate::AtlasAssociation>,
    pool: Option<Arc<PlanePool>>,
    has_film_grain: bool,
    /// The grain the stream signals for this picture and the samples lack.
    pending_grain: Option<ActiveFilmGrain>,
    frame_type: Option<FrameType>,
    extended_layer_id: ExtendedLayerId,
    embedded_layer_id: EmbeddedLayerId,
    temporal_layer_id: TemporalLayerId,
    backend: splot_core::simd::ResolvedBackend,
}

#[derive(Debug)]
enum Samples {
    Eight(SharedFrame<u8>),
    Ten(SharedFrame<u16>),
}

impl Samples {
    fn info(&self) -> DecodedFrameInfo {
        match self {
            Self::Eight(frame) => frame.info(),
            Self::Ten(frame) => frame.info(),
        }
    }
}

/// A decoded frame borrowed from a [`Picture`].
#[derive(Clone, Copy, Debug)]
pub enum PictureFrame<'a> {
    /// Eight-bit decoded samples and their geometry.
    Eight(&'a DecodedFrame<u8>),
    /// Ten-bit decoded samples and their geometry.
    Ten(&'a DecodedFrame<u16>),
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
    fn new(
        frame: &PipelineFrame,
        pool: Option<Arc<PlanePool>>,
        options: &DecodeOptions,
        backend: impl SimdBackend,
        resolved: ResolvedBackend,
    ) -> Result<Self> {
        let grain = frame.output_grain(options);
        let samples = match frame.ready_frame()? {
            PipelineDecodedFrame::Eight(frame) => {
                Samples::Eight(display(backend, frame, grain, pool.as_ref())?)
            }
            PipelineDecodedFrame::Ten(frame) => {
                Samples::Ten(display(backend, frame, grain, pool.as_ref())?)
            }
        };
        let metadata = &frame.output_effects.metadata;
        Ok(Self {
            frame_type: frame
                .ccso_header
                .as_ref()
                .and_then(|header| header.frame_type),
            extended_layer_id: frame.extended_layer_id,
            embedded_layer_id: frame.embedded_layer_id,
            temporal_layer_id: frame.temporal_layer_id,
            backend: resolved,
            info: samples.info(),
            samples: Some(samples),
            props: frame.props,
            provenance: frame.provenance,
            content_interpretation: frame.output_effects.content_interpretation,
            atlas: frame.output_effects.atlas.clone(),
            metadata: (!metadata.is_empty()).then(|| Arc::from(metadata.as_slice())),
            pool,
            has_film_grain: frame.display_grain.is_some(),
            pending_grain: match grain {
                Some(_) => None,
                None => frame.display_grain.clone(),
            },
        })
    }

    /// Whether the stream signals film grain for this picture (AV2 § 7.21.1),
    /// whether or not [`DecoderSettings::skip_film_grain`] left it out.
    #[must_use]
    pub const fn has_film_grain(&self) -> bool {
        self.has_film_grain
    }

    /// This picture with the film grain of AV2 § 7.21.7 applied.
    ///
    /// The grain goes into planes of its own. A picture that has its grain,
    /// or whose stream signals none, shares its samples with the result.
    ///
    /// # Errors
    /// Returns [`DecodeError::Reconstruction`] when the synthesis fails.
    pub fn apply_film_grain(&self) -> Result<Self> {
        struct Apply<'a>(&'a Picture);
        impl SimdScope<Result<Picture>> for Apply<'_> {
            fn run<B: SimdBackend>(self, backend: B) -> Result<Picture> {
                self.0.apply_film_grain_with(backend)
            }
        }
        self.backend.enter(Apply(self))
    }

    fn apply_film_grain_with(&self, backend: impl SimdBackend) -> Result<Self> {
        let grain = self.pending_grain.as_ref();
        let pool = self.pool.as_ref();
        let samples = match &self.samples {
            Some(Samples::Eight(frame)) => Some(Samples::Eight(display(
                backend,
                frame.share(),
                grain,
                pool,
            )?)),
            Some(Samples::Ten(frame)) => {
                Some(Samples::Ten(display(backend, frame.share(), grain, pool)?))
            }
            None => None,
        };
        Ok(Self {
            info: samples.as_ref().map_or(self.info, Samples::info),
            samples,
            props: self.props,
            provenance: self.provenance,
            content_interpretation: self.content_interpretation,
            metadata: self.metadata.clone(),
            atlas: self.atlas.clone(),
            pool: self.pool.clone(),
            has_film_grain: self.has_film_grain,
            pending_grain: None,
            frame_type: self.frame_type,
            extended_layer_id: self.extended_layer_id,
            embedded_layer_id: self.embedded_layer_id,
            temporal_layer_id: self.temporal_layer_id,
            backend: self.backend,
        })
    }

    /// The `FrameType` of the frame this picture shows (AV2 § 5.18.2):
    /// [`FrameType::Key`] for a closed-loop or open-loop key frame. A
    /// show-existing picture has the type of the frame it shows again.
    #[must_use]
    pub const fn frame_type(&self) -> Option<FrameType> {
        self.frame_type
    }

    /// Extended layer id (`obu_xlayer_id`, AV2 § 6.2.2) of the coded or
    /// show-existing frame that produced this picture.
    #[must_use]
    pub const fn extended_layer_id(&self) -> ExtendedLayerId {
        self.extended_layer_id
    }

    /// Embedded layer ID (`obu_mlayer_id`, AV2 § 6.2.2) of this output picture.
    /// A show-existing frame with a derived order hint uses the reference layer ID.
    #[must_use]
    pub const fn embedded_layer_id(&self) -> EmbeddedLayerId {
        self.embedded_layer_id
    }

    /// Temporal layer ID (`obu_tlayer_id`, AV2 § 6.2.2) of this output picture.
    /// A show-existing frame with a derived order hint uses the reference layer ID.
    #[must_use]
    pub const fn temporal_layer_id(&self) -> TemporalLayerId {
        self.temporal_layer_id
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

    /// Borrows the decoded frame without a copy. The frame gives the coded
    /// and storage sizes and the visible rectangle. It contains the same
    /// samples as [`Self::plane`], with or without film grain.
    ///
    /// Use the order from [`Decoder::get_picture`] for display order.
    #[must_use]
    pub fn decoded_frame(&self) -> Option<PictureFrame<'_>> {
        Some(match self.samples.as_ref()? {
            Samples::Eight(frame) => PictureFrame::Eight(frame.get()),
            Samples::Ten(frame) => PictureFrame::Ten(frame.get()),
        })
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

    /// The coded, display, and sequence header locations for this picture.
    /// Submit a distinct [`DataProps::user_data`] value for each temporal unit
    /// to distinguish locations in different units.
    #[must_use]
    pub const fn provenance(&self) -> PictureProvenance {
        self.provenance
    }

    /// The content interpretation (color description, timing) in effect for
    /// this picture, from AV2 § 5.15.
    #[must_use]
    pub const fn content_interpretation(&self) -> Option<&ContentInterpretation> {
        self.content_interpretation.as_ref()
    }

    /// Applicable global and local atlas associations in source order by scope (global first).
    #[must_use]
    pub fn atlas_associations(&self) -> &[crate::AtlasAssociation] {
        &self.atlas
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

/// The displayed form of `frame`: grain goes into planes from the decoder's
/// pool, which the picture gives them back to when the caller drops it.
fn display<T: ReconSample>(
    backend: impl SimdBackend,
    frame: SharedFrame<T>,
    grain: Option<&ActiveFilmGrain>,
    pool: Option<&Arc<PlanePool>>,
) -> Result<SharedFrame<T>> {
    let Some(grain) = grain else {
        return Ok(frame);
    };
    let spare = FramePlaneSamples::default().with_pool(pool);
    let grained = crate::output::film_grain::apply(backend, frame.get(), grain, spare)?;
    Ok(SharedFrame::new(grained))
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
/// take the rest. The decoder runs on its own threads.
///
/// An error ends the session: the call that sees it returns it, and later
/// calls return [`DecoderStateError::Failed`] until [`Self::flush`] starts a
/// new session on the same threads.
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
    selected_backend: SelectedBackend,
    /// Joined by [`Decoder::drop`], which waits for the driver to stop.
    driver: Option<std::thread::JoinHandle<()>>,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    /// Wakes a caller that waits for a picture or for the driver to stop.
    caller: Condvar,
    /// Wakes the driver when the caller sent a unit, took a picture, drained
    /// or closed.
    driver: Condvar,
}

#[derive(Debug)]
struct State {
    input: VecDeque<SourceUnit>,
    input_capacity: usize,
    buffers: Vec<Arc<Vec<u8>>>,
    output: VecDeque<Picture>,
    output_capacity: usize,
    sent_bytes: u64,
    draining: bool,
    closing: bool,
    /// [`Decoder::flush`] waits for the driver to drop the session.
    flushing: bool,
    /// The driver thread is gone.
    stopped: bool,
    settle: Settle,
    outcome: Outcome,
}

/// Where a [`Decoder::wait_picture`] request for the frames still decoding is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Settle {
    Idle,
    Requested,
    /// The driver hands the frames over.
    Running,
    /// The driver holds no unit and no frame that could give a picture now.
    Done,
}

#[derive(Debug)]
enum Outcome {
    Running,
    Finished,
    Failed(Option<DecodeError>),
}

impl Shared {
    /// Wakes a waiting driver so it looks at the state again.
    fn bump(&self, state: parking_lot::MutexGuard<'_, State>) {
        drop(state);
        self.driver.notify_all();
    }

    /// Records the outcome of one session, then waits for a flush or the
    /// close. Returns `true` when a flush starts the next session.
    fn park(&self, result: Result<()>) -> bool {
        let mut state = self.state.lock();
        if !state.flushing && matches!(state.outcome, Outcome::Running) {
            state.outcome = match result {
                Ok(()) => Outcome::Finished,
                Err(error) => Outcome::Failed(Some(error)),
            };
        }
        state.input.clear();
        self.caller.notify_all();
        while !state.flushing && !state.closing {
            self.driver.wait(&mut state);
        }
        if state.closing {
            return false;
        }
        state.output.clear();
        state.sent_bytes = 0;
        state.draining = false;
        state.flushing = false;
        state.settle = Settle::Idle;
        state.outcome = Outcome::Running;
        drop(state);
        self.caller.notify_all();
        true
    }
}

/// Marks the driver as gone, and fails the session if it unwound.
struct DriverGuard(Arc<Shared>);

impl Drop for DriverGuard {
    fn drop(&mut self) {
        let mut state = self.0.state.lock();
        if matches!(state.outcome, Outcome::Running) {
            state.outcome = Outcome::Failed(Some(DecoderStateError::Failed.into()));
        }
        state.stopped = true;
        drop(state);
        self.0.caller.notify_all();
    }
}

struct DriverLaunch {
    observer: Option<Box<dyn DecodeObserver>>,
    #[cfg(feature = "block-evidence")]
    block_evidence: Option<Arc<BlockEvidenceState>>,
    atlases: crate::ExternalAtlases,
    shared: Arc<Shared>,
    limits: DecodeLimits,
    skip_film_grain: bool,
    temporal_layer_selection: TemporalLayerSelection,
    embedded_layer_selection: EmbeddedLayerSelection,
    extended_layer_selection: ExtendedLayerSelection,
    operating_point_selection: OperatingPointSelection,
    frame_delay: core::num::NonZeroUsize,
    rows: Option<Arc<crate::pipeline::DecoderRowScheduler>>,
    resolved: ResolvedBackend,
}

impl SimdScope<Result<std::thread::JoinHandle<()>>> for DriverLaunch {
    fn run<B: SimdBackend>(self, backend: B) -> Result<std::thread::JoinHandle<()>> {
        splot_parallel::spawn_thread("splot-driver", move || {
            run_driver_session(backend, self);
        })
        .map_err(Into::into)
    }
}

fn run_driver_session(backend: impl SimdBackend, launch: DriverLaunch) {
    let options = DecodeOptions::new(launch.limits)
        .with_skip_film_grain(launch.skip_film_grain)
        .with_temporal_layer_selection(launch.temporal_layer_selection)
        .with_embedded_layer_selection(launch.embedded_layer_selection)
        .with_extended_layer_selection(launch.extended_layer_selection)
        .with_operating_point_selection(launch.operating_point_selection);
    let guard = DriverGuard(launch.shared);
    let mut session =
        DecodeSession::detaching_held(launch.frame_delay, launch.rows, launch.resolved);
    session.external_atlases = launch.atlases;
    session.observer = launch.observer;
    #[cfg(feature = "block-evidence")]
    {
        session.block_evidence = launch.block_evidence;
    }
    loop {
        let mut units = ChannelUnits(Arc::clone(&guard.0));
        let result = crate::pipeline::emit_frames(
            backend.narrow(),
            &mut units,
            &options,
            &session,
            |_| Ok(()),
            |frame| {
                guard.0.push_picture(Picture::new(
                    frame,
                    session.planes(),
                    &options,
                    backend,
                    session.backend(),
                )?);
                Ok(())
            },
        );
        #[cfg(feature = "block-evidence")]
        if let Some(evidence) = &session.block_evidence {
            evidence.finish(result.is_ok());
        }
        if !guard.0.park(result) {
            break;
        }
    }
}

impl Decoder {
    /// Opens a decoder, validates its settings and SIMD policy, and starts its
    /// driver and row workers.
    ///
    /// # Errors
    /// Returns [`DecodeError::InvalidTemporalSelection`] if
    /// `settings.temporal_layer_selection` is invalid,
    /// [`DecodeError::InvalidEmbeddedSelection`] if
    /// `settings.embedded_layer_selection` is invalid,
    /// [`DecodeError::InvalidExtendedSelection`] if
    /// `settings.extended_layer_selection` is invalid,
    /// [`DecodeError::InvalidOperatingPointSelection`] if
    /// `settings.operating_point_selection` is invalid or conflicts with layer selections,
    /// [`DecodeError::UnsupportedBackend`] if the requested SIMD policy cannot
    /// run on this host, or [`DecodeError::Pool`] if a thread cannot start.
    pub fn new(settings: DecoderSettings) -> Result<Self> {
        Self::new_with_external_atlases(settings, crate::ExternalAtlases::default())
    }

    /// Opens a decoder with caller-supplied atlas metadata. In-band OBUs can replace those atlases.
    /// Each flush restores the supplied metadata for the next decode.
    ///
    /// # Errors
    /// Returns the errors from [`Self::new`].
    pub fn new_with_external_atlases(
        settings: DecoderSettings,
        atlases: crate::ExternalAtlases,
    ) -> Result<Self> {
        Self::open(
            &settings,
            atlases,
            None,
            #[cfg(feature = "block-evidence")]
            None,
        )
    }

    /// Opens a decoder with an observer for frames admitted on its decode path.
    ///
    /// The driver calls the observer in coded order, before the corresponding
    /// picture can be returned. An event can precede a later decode error.
    /// Skipped frames and show-existing OBUs do not generate coded-frame events.
    /// The observer remains installed across [`Self::flush`].
    /// Return promptly from the observer. Do not call this decoder from it.
    /// Do not panic in the observer. A panic that unwinds stops the driver permanently.
    /// Assign a distinct [`DataProps::user_data`] value to each input unit if the
    /// observer must distinguish locations in different units.
    ///
    /// # Errors
    /// Returns the errors from [`Self::new`].
    pub fn new_with_coded_frame_observer(
        settings: DecoderSettings,
        observer: impl Fn(CodedFrameEvent) + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::open(
            &settings,
            crate::ExternalAtlases::default(),
            Some(Box::new(CodedFrameObserver(observer))),
            #[cfg(feature = "block-evidence")]
            None,
        )
    }

    /// Opens a decoder with an observer for admitted coded frames' reference states.
    ///
    /// The driver calls the observer in coded order. An event can precede a later
    /// decode error. Skipped frames and show-existing output do not generate events.
    /// The decoder uses in-band reference pictures. Missing prerequisites still
    /// produce the usual decode errors.
    ///
    /// The observer remains installed across [`Self::flush`]. Return promptly.
    /// Do not call this decoder from the observer. Do not panic in the observer.
    /// A panic that unwinds stops the driver permanently. Assign distinct
    /// [`DataProps::user_data`] values to distinguish locations in different units.
    ///
    /// # Errors
    /// Returns the errors from [`Self::new`].
    pub fn new_with_reference_observer(
        settings: DecoderSettings,
        observer: impl Fn(crate::ReferenceFrameEvent) + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::open(
            &settings,
            crate::ExternalAtlases::default(),
            Some(Box::new(
                crate::reference::observation::CaptureReferences::new(observer),
            )),
            #[cfg(feature = "block-evidence")]
            None,
        )
    }

    /// Opens a decoder with a bounded capture for partitions and coding blocks.
    /// The capture does not skip prerequisite decoding.
    /// The driver waits for a selected frame to settle before it admits another frame.
    /// Assign distinct [`DataProps::user_data`] values to distinguish input units.
    /// A flush ends the capture and resets the decoder.
    /// Enable the `block-evidence` Cargo feature to use this method.
    ///
    /// # Errors
    /// Returns the errors from [`Self::new`].
    #[cfg(feature = "block-evidence")]
    pub fn new_with_block_evidence(
        settings: DecoderSettings,
        request: BlockEvidenceRequest,
    ) -> Result<(Self, BlockEvidenceCapture)> {
        let evidence = Arc::new(BlockEvidenceState::new(request));
        let decoder = Self::open(
            &settings,
            crate::ExternalAtlases::default(),
            None,
            Some(Arc::clone(&evidence)),
        )?;
        Ok((decoder, BlockEvidenceCapture(evidence)))
    }

    fn open(
        settings: &DecoderSettings,
        atlases: crate::ExternalAtlases,
        observer: Option<Box<dyn DecodeObserver>>,
        #[cfg(feature = "block-evidence")] block_evidence: Option<Arc<BlockEvidenceState>>,
    ) -> Result<Self> {
        settings.temporal_layer_selection.validate()?;
        settings.embedded_layer_selection.validate()?;
        settings.extended_layer_selection.validate()?;
        settings.operating_point_selection.validate_with(
            settings.temporal_layer_selection,
            settings.embedded_layer_selection,
            settings.extended_layer_selection,
        )?;
        let resolved = splot_core::simd::resolve_backend(settings.simd_policy)?;
        let selected_backend = resolved.kind();
        let threads = settings.threads.resolve();
        let frame_delay = settings.frame_delay.resolve(threads);
        let capacity = frame_delay.get().max(2);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                input: VecDeque::with_capacity(capacity),
                input_capacity: capacity,
                buffers: Vec::new(),
                output: VecDeque::with_capacity(capacity + MAX_OUTPUTS_PER_UNIT),
                output_capacity: capacity,
                sent_bytes: 0,
                draining: false,
                closing: false,
                flushing: false,
                stopped: false,
                settle: Settle::Idle,
                outcome: Outcome::Running,
            }),
            caller: Condvar::new(),
            driver: Condvar::new(),
        });
        let driver = Arc::clone(&shared);
        let rows = crate::pipeline::row_scheduler(threads, resolved)?;
        let driver = resolved.enter(DriverLaunch {
            observer,
            #[cfg(feature = "block-evidence")]
            block_evidence,
            atlases,
            shared: driver,
            limits: settings.limits,
            skip_film_grain: settings.skip_film_grain,
            temporal_layer_selection: settings.temporal_layer_selection,
            embedded_layer_selection: settings.embedded_layer_selection,
            extended_layer_selection: settings.extended_layer_selection,
            operating_point_selection: settings.operating_point_selection,
            frame_delay,
            rows,
            resolved,
        })?;
        Ok(Self {
            shared,
            limits: settings.limits,
            selected_backend,
            driver: Some(driver),
        })
    }

    /// The concrete SIMD backend selected when this decoder opened.
    #[must_use]
    pub const fn selected_backend(&self) -> SelectedBackend {
        self.selected_backend
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
        state.settle = Settle::Idle;
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

    /// Takes the next picture in display order, waiting for the units the
    /// decoder holds.
    ///
    /// Returns `None` once every unit sent was decoded and no picture is
    /// ready: a unit can give no picture, when its frame is shown later.
    /// After [`Self::drain`] this is [`Self::get_picture`].
    ///
    /// # Errors
    /// Returns the decode error that ended the session, then
    /// [`DecoderStateError::Failed`] on later calls.
    pub fn wait_picture(&mut self) -> Result<Option<Picture>> {
        let mut state = self.shared.state.lock();
        state.settle = Settle::Requested;
        self.shared.driver.notify_all();
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
            if state.settle == Settle::Done {
                state.settle = Settle::Idle;
                return Ok(None);
            }
            self.shared.caller.wait(&mut state);
        }
    }

    /// Drops every unit and picture the decoder holds and starts a new
    /// session on the same threads and sample storage, for a seek or after an
    /// error. The next unit sent must be a random access point
    /// ([`splot_core::stream_reader::is_random_access_point`]).
    ///
    /// Waits for the frames in decode, which are not interrupted. Pictures
    /// taken earlier stay valid. [`DecodeLimitName::MaxInputBytes`] counts
    /// from zero again.
    pub fn flush(&mut self) {
        let mut state = self.shared.state.lock();
        state.flushing = true;
        self.shared.driver.notify_all();
        while state.flushing && !state.stopped {
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
    /// Stops the driver and joins it, so no decoder code runs after the
    /// decoder is gone.
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.closing = true;
        state.output.clear();
        self.shared.bump(state);
        if let Some(driver) = self.driver.take() {
            let _ = driver.join();
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
        if !state.closing && !state.flushing {
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
        let mut state = shared.state.lock();
        loop {
            if state.closing || state.flushing {
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
            shared.driver.wait(&mut state);
        }
    }

    fn ivf_header(&self) -> Option<IvfHeader> {
        None
    }

    fn await_input(&mut self) -> bool {
        let shared = &self.0;
        let mut state = shared.state.lock();
        loop {
            let ready = state.output.len() < state.output_capacity && !state.input.is_empty();
            if ready || state.closing || state.flushing || state.draining {
                return false;
            }
            if state.input.is_empty() {
                match state.settle {
                    Settle::Requested => {
                        state.settle = Settle::Running;
                        return true;
                    }
                    Settle::Running => {
                        state.settle = Settle::Done;
                        shared.caller.notify_all();
                    }
                    Settle::Idle | Settle::Done => {}
                }
            }
            shared.driver.wait(&mut state);
        }
    }
}
