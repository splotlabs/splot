// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded observations of the streaming decoder's partition and block walk.
//!
//! Feature tracking: `DECODE-STREAMING-API`.

mod prediction;
pub(crate) mod stages;
mod transform;
pub use prediction::*;
pub use stages::{DecodeStage, StageEvidence, StageEvidenceStatus};
pub(crate) use transform::DecodedTransformUnit;
pub use transform::{BlockTransformDetails, BlockTransformEvidence, BlockTransformId};

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use splot_core::obu::ObuHeader;
use splot_recon::{BitDepth, PlaneId, PlaneRect};

use crate::bitstream::tile_payload::{DecodeBlockFrontier, PartitionTreeType, TilePartitionCall};
use crate::{EmbeddedLayerId, ExtendedLayerId, ObuLocation, PictureProvenance, TemporalLayerId};

pub use crate::encoder_syntax::Partition as BlockPartition;

/// The structural records to collect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockEvidenceKind {
    /// Partition decisions only.
    Partitions,
    /// Coding blocks and their prediction and motion facts.
    Blocks,
    /// Partition decisions, coding blocks and motion facts.
    PartitionsAndBlocks,
    /// Transform geometry only.
    Transforms,
    /// Samples at selected reconstruction stages. Use `with_stage_samples` to set the region.
    Stages,
    /// Partition decisions, coding blocks, motion and transform geometry.
    All,
}

impl BlockEvidenceKind {
    fn partitions(self) -> bool {
        matches!(
            self,
            Self::Partitions | Self::PartitionsAndBlocks | Self::All
        )
    }

    fn blocks(self) -> bool {
        matches!(self, Self::Blocks | Self::PartitionsAndBlocks | Self::All)
    }

    fn transforms(self) -> bool {
        matches!(self, Self::Transforms | Self::All)
    }
}

/// Selects coded frames for one decode session.
pub struct BlockEvidenceRequest {
    matches_coded: Box<dyn Fn(ObuLocation) -> bool + Send + Sync>,
    region: Option<PlaneRect>,
    max_events: usize,
    max_bytes: usize,
    frame_limit: NonZeroUsize,
    kind: BlockEvidenceKind,
    transform_details: Option<BlockTransformId>,
    stage_samples: Option<(PlaneId, PlaneRect)>,
    cancelled: Option<Box<dyn Fn() -> bool + Send + Sync>>,
}

impl BlockEvidenceRequest {
    /// Selects the first matching coded OBU and a region in coded luma coordinates.
    /// Each budget limits the same fixed-size records. The byte budget counts
    /// record storage, including spare capacity. Zero stops before the first record.
    ///
    /// Return promptly from the predicate. Do not call this decoder or panic.
    #[must_use]
    pub fn new(
        matches_coded: impl Fn(ObuLocation) -> bool + Send + Sync + 'static,
        region: Option<PlaneRect>,
        max_events: usize,
        max_bytes: usize,
    ) -> Self {
        Self {
            matches_coded: Box::new(matches_coded),
            region,
            max_events,
            max_bytes,
            frame_limit: NonZeroUsize::MIN,
            kind: BlockEvidenceKind::PartitionsAndBlocks,
            transform_details: None,
            stage_samples: None,
            cancelled: None,
        }
    }

    /// Selects at most this number of matching coded frames, in decode order.
    /// The record and byte budgets apply to the full range. A range stops when
    /// a selected frame exceeds a budget. The frame limit also bounds report metadata.
    #[must_use]
    pub const fn with_frame_limit(mut self, frames: NonZeroUsize) -> Self {
        self.frame_limit = frames;
        self
    }

    /// Selects which records consume the budget.
    #[must_use]
    pub fn with_kind(mut self, kind: BlockEvidenceKind) -> Self {
        self.kind = kind;
        self.transform_details = None;
        self.stage_samples = None;
        self
    }

    /// Selects one transform and includes its details.
    #[must_use]
    pub fn with_transform_details(mut self, transform: BlockTransformId) -> Self {
        self.transform_details = Some(transform);
        self.stage_samples = None;
        self.kind = BlockEvidenceKind::Transforms;
        self
    }

    /// Captures sample stages for one plane. The region uses native plane coordinates.
    /// The decoder intersects the region with the visible plane. The byte budget
    /// counts allocated sample storage. The event budget applies to structural records.
    #[must_use]
    pub fn with_stage_samples(mut self, plane: PlaneId, region: PlaneRect) -> Self {
        self.stage_samples = Some((plane, region));
        self.transform_details = None;
        self.kind = BlockEvidenceKind::Stages;
        self
    }

    /// Cancels capture when the predicate returns true.
    /// The decoder continues normal decoding. The predicate may run on a worker.
    /// Return promptly from the predicate. Do not call this decoder or panic.
    #[must_use]
    pub fn with_cancellation(
        mut self,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        self.cancelled = Some(Box::new(cancelled));
        self
    }
}

/// The result of a capture request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockEvidenceStatus {
    /// The decoder collected every applicable record in the request.
    Complete,
    /// The record or byte budget prevented capture of an applicable record.
    Truncated,
    /// The cancellation predicate stopped capture.
    Cancelled,
    /// The decode session ended before it admitted a matching coded frame.
    FrameNotFound,
    /// The matching frame does not use the coding block walk.
    Unavailable,
    /// Decode failed. Records, if present, may be incomplete.
    DecodeFailed,
    /// Record storage allocation failed.
    AllocationFailed,
}

/// Identifies a captured coded frame within the decoder's input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockEvidenceFrame {
    /// The coded OBU and its sequence header. Pictures identify display events separately.
    pub provenance: PictureProvenance,
    /// The frame's extended layer.
    pub extended_layer_id: ExtendedLayerId,
    /// The frame's embedded layer.
    pub embedded_layer_id: EmbeddedLayerId,
    /// The frame's temporal layer.
    pub temporal_layer_id: TemporalLayerId,
}

/// The partition tree that owns a node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum BlockEvidenceTree {
    /// The shared tree.
    Shared,
    /// The separate luma tree.
    Luma,
    /// The separate chroma tree.
    Chroma,
}

impl From<PartitionTreeType> for BlockEvidenceTree {
    fn from(tree: PartitionTreeType) -> Self {
        match tree {
            PartitionTreeType::Shared => Self::Shared,
            PartitionTreeType::LumaPart => Self::Luma,
            PartitionTreeType::ChromaPart => Self::Chroma,
        }
    }
}

/// Identifies a node within one captured coded frame.
/// Keep its report's frame and input source identity to compare it across captures.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct BlockEvidenceId {
    /// Zero-based tile index in raster order.
    pub tile: usize,
    /// The partition tree.
    pub tree: BlockEvidenceTree,
    /// The partition rectangle in coded luma samples, including edge padding.
    pub coded: PlaneRect,
}

impl BlockEvidenceId {
    /// Identifies a node by tile, tree and coded luma rectangle.
    #[must_use]
    pub const fn new(tile: usize, tree: BlockEvidenceTree, coded: PlaneRect) -> Self {
        Self { tile, tree, coded }
    }
}

/// Geometry and facts for one event from the block walk.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlockEvidenceRecord {
    /// The node the record describes.
    pub id: BlockEvidenceId,
    /// The containing partition, if this record describes a child node.
    /// Supplemental motion records omit the parent.
    pub parent: Option<BlockEvidenceId>,
    /// The intersection with the visible picture, in coded luma coordinates.
    pub visible: Option<PlaneRect>,
    /// The facts for this event.
    pub data: BlockEvidenceData,
}

/// The facts associated with a partition or coding block.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum BlockEvidenceData {
    /// A partition decision. In a partition-only capture, `None` marks a leaf.
    Partition(BlockPartition),
    /// A coding block.
    Block(BlockEvidenceFacts),
    /// Supplemental resolved motion for the coding block with the same identity.
    Motion(BlockMotion),
    /// A transform unit parsed within the coding block with the same identity.
    Transform(BlockTransformEvidence),
}

/// Facts from the coding block parser.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlockEvidenceFacts {
    /// The block codes chroma.
    pub has_chroma: bool,
    /// The effective segment identifier.
    pub segment_id: u8,
    /// The effective block quantizer index before plane-specific adjustments.
    pub qindex: u32,
    /// The prediction facts available for the block.
    pub prediction: BlockPrediction,
}

/// A bounded batch of records. The order of records across tiles can vary.
#[derive(Debug)]
#[non_exhaustive]
pub struct BlockEvidenceReport {
    /// The capture result.
    pub status: BlockEvidenceStatus,
    /// The selected frame, when the decoder admitted it.
    pub frame: Option<BlockEvidenceFrame>,
    /// Records that satisfied the selection before capture stopped.
    pub records: Vec<BlockEvidenceRecord>,
    /// Sample stages during a region capture.
    pub stages: Vec<StageEvidence>,
}

/// The receive end of an evidence capture.
pub struct BlockEvidenceCapture(pub(crate) Arc<BlockEvidenceState>);

impl BlockEvidenceCapture {
    /// Reports if capture ended. Unread reports remain available through [`Self::take`].
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.0.state.lock().finished
    }

    /// Takes the next available report in coded order. Returns `None` when
    /// no report is ready. Drain the decoder to finish a range that remains open.
    #[must_use]
    pub fn take(&self) -> Option<BlockEvidenceReport> {
        let mut state = self.0.state.lock();
        state
            .reports
            .pop_front()
            .or_else(|| state.last_report.take())
    }
}

#[derive(Default)]
struct CaptureState {
    frame: Option<BlockEvidenceFrame>,
    frames_selected: usize,
    remaining_events: usize,
    remaining_bytes: usize,
    walking: bool,
    stopped: Option<BlockEvidenceStatus>,
    finished: bool,
    records: Vec<BlockEvidenceRecord>,
    stages: Option<stages::StageSamples>,
    stage_depth: Option<BitDepth>,
    reports: VecDeque<BlockEvidenceReport>,
    last_report: Option<BlockEvidenceReport>,
}

pub(crate) struct BlockEvidenceState {
    request: BlockEvidenceRequest,
    state: Mutex<CaptureState>,
    stopped: AtomicBool,
}

impl BlockEvidenceState {
    pub(crate) fn new(request: BlockEvidenceRequest) -> Self {
        let state = CaptureState {
            remaining_events: request.max_events,
            remaining_bytes: request.max_bytes,
            ..CaptureState::default()
        };
        Self {
            request,
            state: Mutex::new(state),
            stopped: AtomicBool::new(false),
        }
    }

    fn cancelled(&self) -> bool {
        self.request
            .cancelled
            .as_ref()
            .is_some_and(|cancelled| cancelled())
    }

    pub(crate) fn select(
        self: &Arc<Self>,
        provenance: PictureProvenance,
        header: ObuHeader,
    ) -> Option<Arc<Self>> {
        let between_frames = {
            let state = self.state.lock();
            if state.finished || state.frame.is_some() {
                return None;
            }
            state.frames_selected > 0
        };
        if between_frames && self.cancelled() {
            self.complete(true, true, true);
            return None;
        }
        if !(self.request.matches_coded)(provenance.coded) {
            return None;
        }
        let state = &mut *self.state.lock();
        state.frames_selected += 1;
        self.stopped.store(false, Ordering::Relaxed);
        state.frame = Some(BlockEvidenceFrame {
            provenance,
            extended_layer_id: header.extended_layer_id,
            embedded_layer_id: header.embedded_layer_id,
            temporal_layer_id: header.temporal_layer_id,
        });
        Some(Arc::clone(self))
    }

    pub(crate) fn start_walk(
        self: &Arc<Self>,
        visible: PlaneRect,
        subsampling: (bool, bool),
    ) -> BlockEvidenceRecorder {
        let mut state = self.state.lock();
        state.walking = true;
        if let Some((plane, region)) = self.request.stage_samples {
            let sx = 1 << usize::from(plane != PlaneId::Y && subsampling.0);
            let sy = 1 << usize::from(plane != PlaneId::Y && subsampling.1);
            let x = visible.x() / sx;
            let y = visible.y() / sy;
            if let Ok(bounds) = PlaneRect::new(
                x,
                y,
                (visible.x() + visible.width()).div_ceil(sx) - x,
                (visible.y() + visible.height()).div_ceil(sy) - y,
            ) {
                state.stages = stages::intersect(bounds, region)
                    .map(|region| stages::StageSamples::new(plane, region, bounds));
            }
        }
        BlockEvidenceRecorder {
            state: Arc::clone(self),
            frame_number: state.frames_selected,
            visible,
            subsampling,
        }
    }

    pub(crate) fn finish(&self, decoded: bool) {
        if self.state.lock().finished {
            return;
        }
        self.complete(decoded, true, self.cancelled());
    }

    fn complete(&self, decoded: bool, session_ended: bool, cancelled: bool) {
        let state = &mut *self.state.lock();
        if state.finished {
            return;
        }
        self.stopped.store(true, Ordering::Relaxed);
        let frame = state.frame.take();
        if session_ended && frame.is_none() && state.frames_selected > 0 && decoded && !cancelled {
            state.finished = true;
            return;
        }
        let stopped = state.stopped.take();
        let walking = core::mem::take(&mut state.walking);
        let status = if !decoded {
            BlockEvidenceStatus::DecodeFailed
        } else if let Some(stopped) = stopped {
            stopped
        } else if cancelled {
            BlockEvidenceStatus::Cancelled
        } else if walking {
            BlockEvidenceStatus::Complete
        } else if frame.is_some() {
            BlockEvidenceStatus::Unavailable
        } else {
            BlockEvidenceStatus::FrameNotFound
        };
        let records = core::mem::take(&mut state.records);
        state.remaining_events -= records.len();
        state.remaining_bytes -= records.capacity() * size_of::<BlockEvidenceRecord>();
        let stages =
            if let (Some(stages), Some(depth)) = (state.stages.take(), state.stage_depth.take()) {
                state.remaining_bytes -= stages.bytes();
                stages.finish(depth)
            } else {
                Vec::new()
            };
        let mut report = BlockEvidenceReport {
            status,
            frame,
            records,
            stages,
        };
        state.finished = session_ended
            || state.frames_selected == self.request.frame_limit.get()
            || !matches!(
                status,
                BlockEvidenceStatus::Complete | BlockEvidenceStatus::Unavailable
            );
        if !state.finished {
            if state.reports.try_reserve(1).is_ok() {
                state.reports.push_back(report);
                return;
            }
            state.finished = true;
            report.status = BlockEvidenceStatus::AllocationFailed;
        }
        state.last_report = Some(report);
    }

    pub(crate) fn finish_frame(&self, frame: &crate::pipeline::PipelineFrame) {
        let matches = {
            let state = self.state.lock();
            !state.finished
                && state
                    .frame
                    .is_some_and(|selected| selected.provenance == frame.provenance)
        };
        if matches {
            let decoded = if self.request.stage_samples.is_some() {
                match frame.wait_ready_frame() {
                    Ok(crate::pipeline::PipelineDecodedFrame::Eight(frame)) => {
                        stages::reference(self, &frame);
                        true
                    }
                    Ok(crate::pipeline::PipelineDecodedFrame::Ten(frame)) => {
                        stages::reference(self, &frame);
                        true
                    }
                    Err(_) => false,
                }
            } else {
                frame.wait_settled().is_ok()
            };
            self.complete(decoded, false, self.cancelled());
        }
    }

    pub(crate) fn captures_stages(&self) -> bool {
        self.request.stage_samples.is_some()
    }

    pub(crate) fn stage_capture(self: &Arc<Self>) -> Option<Arc<Self>> {
        self.captures_stages().then(|| Arc::clone(self))
    }

    pub(crate) fn stage_region(&self) -> Option<(PlaneId, PlaneRect)> {
        if self.stopped.load(Ordering::Relaxed) {
            return None;
        }
        let state = self.state.lock();
        state
            .stages
            .as_ref()
            .map(|samples| (samples.plane, samples.region))
    }

    pub(crate) fn record_stage(
        &self,
        stage: DecodeStage,
        plane: PlaneId,
        source: PlaneRect,
        applied: bool,
        sample: impl FnMut(usize, usize) -> Option<i32>,
    ) {
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        let cancelled = self.cancelled();
        let state = &mut *self.state.lock();
        if state.finished || state.stopped.is_some() {
            return;
        }
        let Some(stages) = state.stages.as_mut().filter(|stages| stages.plane == plane) else {
            return;
        };
        let stop = if cancelled {
            Some(BlockEvidenceStatus::Cancelled)
        } else {
            stages
                .record(stage, source, applied, state.remaining_bytes, sample)
                .err()
        };
        if let Some(stop) = stop {
            state.stopped = Some(stop);
            self.stopped.store(true, Ordering::Relaxed);
        }
    }

    fn prepare_reference_stage(&self, plane: PlaneId, bounds: PlaneRect, depth: BitDepth) {
        let state = &mut *self.state.lock();
        state.stage_depth = Some(depth);
        if state.stages.is_none()
            && let Some((requested, region)) = self.request.stage_samples
            && requested == plane
            && let Some(region) = stages::intersect(bounds, region)
        {
            state.stages = Some(stages::StageSamples::new(plane, region, bounds));
        }
    }

    fn push(&self, frame_number: usize, record: &BlockEvidenceRecord) {
        let cancelled = self.cancelled();
        let state = &mut *self.state.lock();
        if state.finished || state.stopped.is_some() || state.frames_selected != frame_number {
            return;
        }
        let limit = state
            .remaining_events
            .min(state.remaining_bytes / size_of::<BlockEvidenceRecord>());
        let stop = if cancelled {
            BlockEvidenceStatus::Cancelled
        } else if state.records.len() == limit {
            BlockEvidenceStatus::Truncated
        } else {
            let current = state.records.capacity();
            let first_capacity = if self.request.frame_limit == NonZeroUsize::MIN {
                32
            } else {
                1
            };
            let capacity = current.saturating_mul(2).max(first_capacity).min(limit);
            if state.records.len() != current
                || state.records.try_reserve_exact(capacity - current).is_ok()
            {
                state.records.push(*record);
                return;
            }
            BlockEvidenceStatus::AllocationFailed
        };
        state.stopped = Some(stop);
        self.stopped.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub(crate) struct BlockEvidenceRecorder {
    state: Arc<BlockEvidenceState>,
    frame_number: usize,
    visible: PlaneRect,
    subsampling: (bool, bool),
}

impl BlockEvidenceRecorder {
    pub(crate) fn tile(&self, tile: usize) -> Option<BlockEvidenceTile<'_>> {
        (!self.state.stopped.load(Ordering::Relaxed)).then_some(BlockEvidenceTile {
            recorder: self,
            tile,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BlockEvidenceTile<'a> {
    recorder: &'a BlockEvidenceRecorder,
    tile: usize,
}

impl BlockEvidenceTile<'_> {
    fn node_id(self, call: TilePartitionCall) -> Option<BlockEvidenceId> {
        let [h, w] = call.b_size.num_4x4();
        self.id(
            call.tree_type().into(),
            call.c() * 4,
            call.r() * 4,
            w * 4,
            h * 4,
        )
    }

    pub(crate) fn record_partition(
        self,
        call: TilePartitionCall,
        parent: Option<TilePartitionCall>,
        partition: BlockPartition,
    ) {
        if self.recorder.state.request.kind.partitions()
            && let Some(id) = self.node_id(call)
        {
            self.record(
                id,
                parent,
                id.coded,
                BlockEvidenceData::Partition(partition),
            );
        }
    }

    pub(crate) fn record_terminal_partition(
        self,
        call: TilePartitionCall,
        parent: Option<TilePartitionCall>,
    ) {
        if self.recorder.state.request.kind == BlockEvidenceKind::Partitions {
            self.record_partition(call, parent, BlockPartition::None);
        }
    }

    pub(crate) fn record_block(
        self,
        frontier: &DecodeBlockFrontier,
        parent: Option<TilePartitionCall>,
        segment_id: u8,
        qindex: u32,
        prediction: BlockPrediction,
    ) {
        if self.wants_blocks()
            && let Some(id) = self.block_id(frontier)
        {
            self.record(
                id,
                parent,
                id.coded,
                BlockEvidenceData::Block(BlockEvidenceFacts {
                    has_chroma: frontier.has_chroma,
                    segment_id,
                    qindex,
                    prediction,
                }),
            );
        }
    }

    pub(crate) fn wants_blocks(self) -> bool {
        self.recorder.state.request.kind.blocks()
    }

    pub(crate) fn wants_transforms(self) -> bool {
        self.recorder.state.request.kind.transforms()
    }

    fn block_id(self, frontier: &DecodeBlockFrontier) -> Option<BlockEvidenceId> {
        let tree = if frontier.is_chroma_part() {
            BlockEvidenceTree::Chroma
        } else if frontier.is_luma_part() {
            BlockEvidenceTree::Luma
        } else {
            BlockEvidenceTree::Shared
        };
        let [h, w] = frontier.b_size.num_4x4();
        self.id(tree, frontier.c * 4, frontier.r * 4, w * 4, h * 4)
    }

    pub(crate) fn record_motion(
        self,
        tree: BlockEvidenceTree,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        motion: BlockMotion,
    ) {
        if self.wants_blocks()
            && let Some(id) = self.id(tree, x, y, width, height)
        {
            self.record(id, None, id.coded, BlockEvidenceData::Motion(motion));
        }
    }

    pub(crate) fn record_transform(
        self,
        frontier: &DecodeBlockFrontier,
        unit: &DecodedTransformUnit<'_>,
    ) {
        if let Some(id) = self.block_id(frontier)
            && let Some(mut transform) = unit.to_evidence(id, self.recorder.subsampling)
        {
            if let Some(selected) = self.recorder.state.request.transform_details {
                if selected != transform.id {
                    return;
                }
                transform.details = Some(unit.details());
            }
            self.record(
                id,
                None,
                transform.luma_coded,
                BlockEvidenceData::Transform(transform),
            );
        }
    }

    fn id(
        self,
        tree: BlockEvidenceTree,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Option<BlockEvidenceId> {
        let coded = PlaneRect::new(x, y, width, height).ok()?;
        Some(BlockEvidenceId::new(self.tile, tree, coded))
    }

    fn record(
        self,
        id: BlockEvidenceId,
        parent: Option<TilePartitionCall>,
        geometry: PlaneRect,
        data: BlockEvidenceData,
    ) {
        if self
            .recorder
            .state
            .request
            .region
            .is_some_and(|region| intersection(geometry, region).is_none())
        {
            return;
        }
        self.recorder.state.push(
            self.recorder.frame_number,
            &BlockEvidenceRecord {
                id,
                parent: parent.and_then(|parent| self.node_id(parent)),
                visible: intersection(geometry, self.recorder.visible),
                data,
            },
        );
    }
}

pub(crate) struct BlockIntraEvidence<'a> {
    pub(crate) tile: BlockEvidenceTile<'a>,
    pub(crate) frontier: &'a DecodeBlockFrontier,
    pub(crate) parent: Option<TilePartitionCall>,
}

impl BlockIntraEvidence<'_> {
    pub(crate) fn record(&self, prediction: BlockIntraPrediction, segment_id: u8, qindex: u32) {
        self.tile.record_block(
            self.frontier,
            self.parent,
            segment_id,
            qindex,
            BlockPrediction::Intra(prediction),
        );
    }
}

fn intersection(first: PlaneRect, second: PlaneRect) -> Option<PlaneRect> {
    let x = first.x().max(second.x());
    let y = first.y().max(second.y());
    let right = first
        .x()
        .saturating_add(first.width())
        .min(second.x().saturating_add(second.width()));
    let bottom = first
        .y()
        .saturating_add(first.height())
        .min(second.y().saturating_add(second.height()));
    PlaneRect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_storage_stays_within_the_byte_budget() {
        let limit = 33;
        let state = Arc::new(BlockEvidenceState::new(BlockEvidenceRequest::new(
            |_| true,
            None,
            usize::MAX,
            limit * size_of::<BlockEvidenceRecord>(),
        )));
        let area = PlaneRect::new(0, 0, 4, 4).unwrap();
        let recorder = state.start_walk(area, (false, false));
        let record = BlockEvidenceRecord {
            id: BlockEvidenceId {
                tile: 0,
                tree: BlockEvidenceTree::Shared,
                coded: area,
            },
            parent: None,
            visible: Some(area),
            data: BlockEvidenceData::Partition(BlockPartition::None),
        };
        for _ in 0..limit {
            state.push(recorder.frame_number, &record);
            assert!(state.state.lock().records.capacity() <= limit);
        }
        assert!(recorder.tile(0).is_some());
        state.push(recorder.frame_number, &record);
        assert!(recorder.tile(0).is_none());
        state.finish(true);
        let report = BlockEvidenceCapture(state).take().unwrap();
        assert_eq!(report.status, BlockEvidenceStatus::Truncated);
        assert_eq!(report.records.len(), limit);
    }
}
