// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded capture of the decisions the encoder makes while it codes.
//!
//! Feature tracking: `ENC-CONTEXT-RECEIVE-PACKET`.
//!
//! [`crate::Encoder::start_decision_trace`] starts a capture and
//! [`crate::Encoder::take_decision_trace`] ends it. A capture never changes
//! the packets: it only copies values the search already has. A value the
//! search does not have is absent, never zero. The encoder outputs a repeat
//! of an earlier frame without a coding: such a frame has no event.
//!
//! # Order
//!
//! Events are in coding order. Each coding of a selected frame gives one
//! [`FrameAttempt`]. The block events of a committed coding follow its
//! [`FrameAttempt`]: superblocks in frame raster order, and for each
//! superblock its [`SuperblockSearch`], its trials in search order, then its
//! [`CommittedNode`]s in syntax order. A coding that the encoder replaced has
//! no block events.
//!
//! # Identity
//!
//! A [`FrameAttempt`] gives the coding and display index of its frame, and a
//! committed one the packet and byte range of the frame's OBUs. A block event
//! names its block with the [`BlockEvidenceId`] that decoder block evidence
//! gives the same node of the same frame. Trial events name the block they
//! tried, which the frame may not code; only [`CommittedNode`]s are in the
//! output.
//!
//! # Truncation
//!
//! The capture keeps at most `max_events` events and at most
//! `max_bytes / size_of::<DecisionEvent>()`. It accepts events in the order
//! above. The first event that does not fit stops the capture: it and every
//! later event are not recorded, and later frames are not traced. The result
//! does not depend on [`crate::Config::threads`]. The budget limits the
//! trace, not the working memory: while a selected frame codes, the encoder
//! holds up to the remaining event budget for each superblock in search and
//! twice that for each frame in coding, in vectors that grow by doubling.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Mutex;

pub use splot_decode::encoder_syntax::{
    BlockEvidenceId, BlockEvidenceTree, Dpcm, InterPrediction, IntraModes, IntrabcModes, Partition,
    TxPartition,
};
use splot_decode::encoder_syntax::{IntraTileSyntax, LeafBlock, PartitionNode};
use splot_recon::PlaneRect;

use crate::rate_control::{FramePlanInfo, FrameType};
use crate::rd::{Lambda, RdCost};
use crate::tile_writer::{LeafChoice, PartitionTree, TileWriterError};

/// A kind of block event. Frame events need no kind: a capture records them
/// for every selected frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecisionKind {
    /// [`PartitionTrial`] events.
    PartitionTrials,
    /// [`ModeTrial`] events.
    ModeTrials,
    /// [`CommittedNode`] events.
    CommittedBlocks,
}

/// Selects the frames, region, event kinds and budgets of one capture.
#[derive(Clone, Debug)]
pub struct DecisionTraceRequest {
    frames: Range<u64>,
    region: Option<PlaneRect>,
    kinds: Kinds,
    max_events: usize,
    max_bytes: usize,
}

/// Whether a capture records each [`DecisionKind`].
#[derive(Clone, Copy, Debug)]
struct Kinds {
    partition_trials: bool,
    mode_trials: bool,
    committed_blocks: bool,
}

impl Kinds {
    const fn slot(&mut self, kind: DecisionKind) -> &mut bool {
        match kind {
            DecisionKind::PartitionTrials => &mut self.partition_trials,
            DecisionKind::ModeTrials => &mut self.mode_trials,
            DecisionKind::CommittedBlocks => &mut self.committed_blocks,
        }
    }

    const fn any(self) -> bool {
        self.partition_trials || self.mode_trials || self.committed_blocks
    }
}

impl DecisionTraceRequest {
    /// Selects the frames whose input (display) index is in `frames`, with
    /// frame events only. See the [module](self) for the budgets.
    #[must_use]
    pub const fn new(frames: Range<u64>, max_events: usize, max_bytes: usize) -> Self {
        Self {
            frames,
            region: None,
            kinds: Kinds {
                partition_trials: false,
                mode_trials: false,
                committed_blocks: false,
            },
            max_events,
            max_bytes,
        }
    }

    /// Keeps only the block events whose block overlaps `region`, in coded
    /// luma samples. Superblocks outside it are not traced.
    #[must_use]
    pub const fn with_region(mut self, region: PlaneRect) -> Self {
        self.region = Some(region);
        self
    }

    /// Adds the block events of `kind`, and the [`SuperblockSearch`] of each
    /// traced superblock.
    #[must_use]
    pub const fn with_kind(mut self, kind: DecisionKind) -> Self {
        *self.kinds.slot(kind) = true;
        self
    }

    fn limit(&self) -> usize {
        self.max_events
            .min(self.max_bytes / size_of::<DecisionEvent>())
    }
}

/// Why a capture did not record every event of its request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecisionTraceStop {
    /// An event did not fit the event or byte budget.
    Budget,
    /// The capture could not store or derive an event.
    CaptureFailed,
    /// The encoder failed or was cancelled. The trace can still have every
    /// selected frame.
    EncodeFailed,
    /// The trace was taken before the encoder committed every selected frame.
    Unfinished,
    /// The encoder coded no selected frame while the capture ran.
    NoFrame,
}

/// The result of a capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecisionTraceStatus {
    /// The trace has every event of the request for the frames coded since
    /// the capture started.
    Complete,
    /// The trace has the events recorded before the stop.
    Partial(DecisionTraceStop),
    /// The trace has no event.
    Dropped(DecisionTraceStop),
}

/// The events of one capture.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DecisionTrace {
    /// Whether the events are all the events of the request.
    pub status: DecisionTraceStatus,
    /// The recorded events, in the [module](self) order.
    pub events: Vec<DecisionEvent>,
    /// The events of traced frame codings that were not recorded. Frames
    /// after the stop are not traced and not counted.
    pub dropped_events: u64,
}

/// One recorded decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecisionEvent {
    /// One coding of a frame.
    Frame(FrameAttempt),
    /// The start of a superblock search.
    Superblock(SuperblockSearch),
    /// A partition shape that a node search considered.
    Partition(PartitionTrial),
    /// A rate-distortion trial of a leaf candidate.
    Mode(ModeTrial),
    /// A node of a partition tree in the output.
    Committed(CommittedNode),
}

/// One coding of a frame at one quantizer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct FrameAttempt {
    /// What rate control knew when it picked the first quantizer: the coding
    /// index, the input (display) index and the lookahead's estimates.
    pub plan: FramePlanInfo,
    /// Presentation timestamp of the source frame.
    pub pts: u64,
    /// Zero for the first coding of the frame, one more for each after it.
    pub attempt: u32,
    /// The frame's `base_q_idx` in this coding.
    pub qindex: u32,
    /// The measured size of this coding in bits, as [`crate::FrameStats::bits`].
    pub bits: u64,
    /// The bits the decoder buffer model had room for; `None` for a rate
    /// control mode without the model.
    pub buffer_room_bits: Option<u64>,
    /// What the encoder did with the coding.
    pub outcome: FrameOutcome,
}

/// What the encoder did with one coding of a frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FrameOutcome {
    /// The coding was over the buffer room. The encoder coded the frame again
    /// at a coarser quantizer.
    Recoded,
    /// The coding is in the output.
    Committed(CommittedFrame),
}

/// Where a committed coding is in the output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CommittedFrame {
    /// The frame type the coder used.
    pub frame_type: FrameType,
    /// The zero-based index of the [`crate::Packet`] that holds the frame's
    /// OBUs, in the order [`crate::Encoder::receive_packet`] returns packets
    /// from the creation of the encoder. A later packet can show the frame.
    pub packet: u64,
    /// The byte offset of the frame's OBUs in [`crate::Packet::data`]. They
    /// take [`FrameAttempt::bits`]` / 8` bytes.
    pub offset: usize,
    /// The frame is a `TIP_FRAME_AS_OUTPUT` frame: it codes no block, and the
    /// trace has no block event for it.
    pub tip_output: bool,
}

/// The quantizer and multiplier of one superblock search. The trials of the
/// superblock follow it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SuperblockSearch {
    /// The superblock.
    pub block: BlockEvidenceId,
    /// The quantizer index of the superblock.
    pub qindex: u32,
    /// The Lagrange multiplier of [`TrialCost::cost`]: squared sample error
    /// per bit, in Q16.
    pub lambda_q16: u64,
    /// The segment the superblock codes in a frame with segmentation.
    pub segment: Option<u8>,
}

/// The rate and distortion of one trial, as the search compared them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TrialCost {
    /// The estimated rate in units of 1/256 bit.
    pub rate: u64,
    /// The sum of squared sample errors. With the preset's `psy_rd` it also
    /// has the AC energy loss of luma.
    pub distortion: u64,
    /// `distortion * 2^24 + lambda_q16 * rate`, with the multiplier of the
    /// superblock's [`SuperblockSearch`].
    pub cost: u128,
}

impl TrialCost {
    const fn new((cost, j): (RdCost, u128)) -> Self {
        Self {
            rate: cost.rate.0,
            distortion: cost.distortion,
            cost: j,
        }
    }
}

/// A partition shape that the search of one node considered. A node can be
/// searched more than once, and the frame may code another shape or another
/// node there. A shape that the preset's block limits or first-pass levels
/// remove before the search has no event, and neither has a search that an
/// earlier bound of its node ends at once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PartitionTrial {
    /// The node.
    pub block: BlockEvidenceId,
    /// The shape.
    pub partition: Partition,
    /// The cost of the node coded with the shape, with the partition symbols;
    /// `None` when the search has none.
    pub cost: Option<TrialCost>,
    /// What the node search did with the shape.
    pub outcome: PartitionOutcome,
}

/// What a node search did with a partition shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PartitionOutcome {
    /// The node search returned the shape.
    Selected,
    /// The search coded the shape and did not return it: another shape cost
    /// less, or the cost was not under the node's budget.
    Rejected,
    /// The search stopped the shape when its cost reached the bound: the
    /// lower of the best cost so far and the node's budget.
    OverBound,
    /// The search coded again the tree that an earlier search of the node
    /// selected; `partition` is the root shape of that tree. A replayed SDP
    /// root codes no partition and has no event.
    Replayed,
    /// The search did not try the shape.
    Pruned(PartitionPruning),
}

/// Why a node search did not try a partition shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PartitionPruning {
    /// The cost that gates an extended shape was missing, or outside the
    /// preset's margin of the best cost.
    ExtendedMargin,
    /// The unsplit cost did not exceed the best split cost (the preset's
    /// `partition_early_exit`).
    EarlyExit,
    /// The unsplit leaf was under the preset's `none_breakout` thresholds.
    NoneBreakout,
}

/// A rate-distortion trial of one candidate of a leaf. A candidate that the
/// search drops before a rate-distortion trial has no event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModeTrial {
    /// The leaf.
    pub block: BlockEvidenceId,
    /// The candidate. The trial decides only its fields for `planes`.
    pub candidate: LeafCandidate,
    /// Whether the trial coded the Y, U and V planes.
    pub planes: [bool; 3],
    /// How the trial quantized.
    pub stage: ModeStage,
    /// The cost over `planes`. Compare costs of one stage and plane set only.
    pub cost: TrialCost,
    /// What the search did with the trial; `None` when it did not record that.
    pub outcome: Option<ModeOutcome>,
}

/// How a mode trial quantized its residual.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ModeStage {
    /// A `DCT_DCT` trial with the deadzone quantizer: an approximate cost.
    Deadzone,
    /// A trial with the preset's transform and quantizer search.
    Full,
}

/// What a comparison of mode trials did with one of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ModeOutcome {
    /// The trial had the least cost of its comparison. A later comparison of
    /// the leaf can replace its candidate.
    Selected,
    /// Another trial of the comparison cost less.
    Rejected,
    /// The deadzone trial ranked inside the preset's `full_trials`: the
    /// candidate got a full trial.
    Advanced,
    /// The deadzone trial ranked outside the preset's `full_trials`: the
    /// candidate got no full trial.
    Pruned,
}

/// The prediction and transform choices of one leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LeafCandidate {
    /// The prediction.
    pub prediction: LeafPrediction,
    /// The luma transform partition.
    pub tx_partition: TxPartition,
    /// `lossless_tx_size` of a leaf of a lossless frame (§ 5.20.6.1).
    pub lossless_tx_size: bool,
}

/// The prediction of one leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LeafPrediction {
    /// An intra leaf.
    Intra {
        /// The intra modes.
        modes: IntraModes,
        /// The DPCM of a leaf of a lossless frame.
        dpcm: Dpcm,
    },
    /// An IntraBC leaf.
    IntraBlockCopy(IntrabcModes),
    /// An inter leaf.
    Inter(InterPrediction),
}

impl From<LeafChoice> for LeafCandidate {
    fn from(choice: LeafChoice) -> Self {
        Self {
            prediction: match (choice.inter, choice.intrabc) {
                (Some(prediction), _) => LeafPrediction::Inter(prediction),
                (None, Some(modes)) => LeafPrediction::IntraBlockCopy(modes),
                (None, None) => LeafPrediction::Intra {
                    modes: choice.modes,
                    dpcm: choice.dpcm,
                },
            },
            tx_partition: choice.tx_partition,
            lossless_tx_size: choice.lossless_tx_size,
        }
    }
}

/// A node of a partition tree that the frame codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CommittedNode {
    /// The node.
    pub block: BlockEvidenceId,
    /// What the node codes.
    pub data: CommittedData,
}

/// What a committed node codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CommittedData {
    /// A partition other than `PARTITION_NONE`.
    Partition(Partition),
    /// A coding block with its choice.
    Block(LeafCandidate),
}

/// What the frame coder collects for the frames it codes next.
#[derive(Clone, Debug)]
pub(crate) struct FrameSpec {
    request: DecisionTraceRequest,
    /// The events the capture has room for.
    cap: usize,
}

/// The block events of one superblock search.
#[derive(Debug)]
pub(crate) struct SearchTrace {
    tile: usize,
    region: Option<PlaneRect>,
    kinds: Kinds,
    cap: usize,
    events: Vec<DecisionEvent>,
    /// The events of the search, stored or not.
    seen: u64,
    failed: bool,
}

impl SearchTrace {
    /// Stores the event of `kind` about `block`; returns its index. A block
    /// without an identity fails the capture.
    fn push(
        &mut self,
        kind: DecisionKind,
        block: Option<BlockEvidenceId>,
        event: impl FnOnce(BlockEvidenceId) -> DecisionEvent,
    ) -> Option<usize> {
        if !*self.kinds.slot(kind) {
            return None;
        }
        let Some(block) = block else {
            self.failed = true;
            return None;
        };
        if self
            .region
            .is_some_and(|region| !overlaps(region, block.coded))
        {
            return None;
        }
        self.store(event(block))
    }

    fn store(&mut self, event: DecisionEvent) -> Option<usize> {
        self.seen += 1;
        if self.failed || self.events.len() >= self.cap {
            return None;
        }
        if self.events.try_reserve(1).is_err() {
            self.failed = true;
            return None;
        }
        self.events.push(event);
        Some(self.events.len() - 1)
    }

    /// Records that the search of `node` considered `partition`, with the
    /// cost and `J` it has; returns the event for [`Self::select_partition`].
    pub(crate) fn partition_trial(
        &mut self,
        node: PartitionNode,
        partition: Partition,
        cost: Option<(RdCost, u128)>,
        outcome: PartitionOutcome,
    ) -> Option<usize> {
        let block = node.block_id(self.tile);
        self.push(DecisionKind::PartitionTrials, block, |block| {
            DecisionEvent::Partition(PartitionTrial {
                block,
                partition,
                cost: cost.map(TrialCost::new),
                outcome,
            })
        })
    }

    /// Marks the trial `index` as the shape its node search returned.
    pub(crate) fn select_partition(&mut self, index: usize) {
        if let Some(DecisionEvent::Partition(trial)) = self.events.get_mut(index) {
            trial.outcome = PartitionOutcome::Selected;
        }
    }

    /// Records a trial of `candidate` over `planes` of `leaf`, a deadzone
    /// trial when `deadzone`, with its cost and `J`.
    pub(crate) fn mode_trial(
        &mut self,
        leaf: &LeafBlock,
        candidate: LeafChoice,
        (planes, deadzone): (Range<usize>, bool),
        cost: (RdCost, u128),
    ) {
        let block = leaf.block_id(self.tile);
        self.push(DecisionKind::ModeTrials, block, |block| {
            DecisionEvent::Mode(ModeTrial {
                block,
                candidate: candidate.into(),
                planes: [0, 1, 2].map(|plane| planes.contains(&plane)),
                stage: if deadzone {
                    ModeStage::Deadzone
                } else {
                    ModeStage::Full
                },
                cost: TrialCost::new(cost),
                outcome: None,
            })
        });
    }

    /// The index of the next stored event.
    pub(crate) const fn len(&self) -> usize {
        self.events.len()
    }

    /// Sets the outcome of the mode trials stored from `from` on, one per
    /// candidate of a comparison in candidate order: `win` for the candidates
    /// `winners`, `lose` for the others.
    pub(crate) fn settle(
        &mut self,
        from: usize,
        winners: impl IntoIterator<Item = usize>,
        (win, lose): (ModeOutcome, ModeOutcome),
    ) {
        for event in self.events.iter_mut().skip(from) {
            if let DecisionEvent::Mode(trial) = event {
                trial.outcome = Some(lose);
            }
        }
        for winner in winners {
            if let Some(DecisionEvent::Mode(trial)) = self.events.get_mut(from + winner) {
                trial.outcome = Some(win);
            }
        }
    }

    /// Records the nodes of `tree`, the searched tree at `node` that the
    /// frame codes.
    pub(crate) fn committed(
        &mut self,
        syntax: &IntraTileSyntax,
        node: PartitionNode,
        tree: &PartitionTree,
    ) {
        if self.walk(syntax, node, tree).is_err() {
            self.failed = true;
        }
    }

    fn walk(
        &mut self,
        syntax: &IntraTileSyntax,
        node: PartitionNode,
        tree: &PartitionTree,
    ) -> Result<(), TileWriterError> {
        if !syntax.in_frame(node) {
            return Ok(());
        }
        if let Some((luma_root, chroma_root)) = syntax.sdp_roots(node) {
            let PartitionTree::Sdp { luma, chroma } = tree else {
                return Err(TileWriterError::Shape("a shared tree at an SDP node"));
            };
            self.walk(syntax, luma_root, luma)?;
            return self.walk(syntax, chroma_root, chroma);
        }
        match tree {
            PartitionTree::Leaf(choice) => {
                self.commit(node, CommittedData::Block((*choice).into()));
            }
            PartitionTree::Split { kind, children } => {
                self.commit(node, CommittedData::Partition(*kind));
                for (child, tree) in syntax.children(node, *kind)?.into_iter().zip(children) {
                    self.walk(syntax, child, tree)?;
                }
            }
            PartitionTree::Region { kind, luma, chroma } => {
                self.commit(node, CommittedData::Partition(*kind));
                let (children, chroma_node) = syntax.region_children(node, *kind)?;
                for (child, tree) in children.into_iter().zip(luma) {
                    self.walk(syntax, child, tree)?;
                }
                self.commit(chroma_node, CommittedData::Block((*chroma).into()));
            }
            PartitionTree::Sdp { .. } => {
                return Err(TileWriterError::Shape("an SDP tree at a shared node"));
            }
        }
        Ok(())
    }

    fn commit(&mut self, node: PartitionNode, data: CommittedData) {
        let block = node.block_id(self.tile);
        self.push(DecisionKind::CommittedBlocks, block, |block| {
            DecisionEvent::Committed(CommittedNode { block, data })
        });
    }
}

fn overlaps(a: PlaneRect, b: PlaneRect) -> bool {
    a.x() < b.x().saturating_add(b.width())
        && b.x() < a.x().saturating_add(a.width())
        && a.y() < b.y().saturating_add(b.height())
        && b.y() < a.y().saturating_add(a.height())
}

/// The block events of one coding of a frame.
#[derive(Debug, Default)]
pub(crate) struct FrameTrace {
    events: Vec<DecisionEvent>,
    /// The block events of the coding, stored or not.
    seen: u64,
    failed: bool,
}

/// Collects the [`SearchTrace`]s of one coding of a frame, whose superblocks
/// can search at the same time.
pub(crate) struct FrameCollector {
    spec: FrameSpec,
    /// By the index of the superblock in frame raster order.
    superblocks: Mutex<BTreeMap<usize, SearchTrace>>,
}

impl FrameCollector {
    /// The collector of the frame with input index `display_index`; `None`
    /// when `spec` selects no block event of it.
    pub(crate) fn new(spec: Option<&FrameSpec>, display_index: u64) -> Option<Self> {
        let spec = spec.filter(|spec| {
            spec.request.frames.contains(&display_index) && spec.request.kinds.any()
        })?;
        Some(Self {
            spec: spec.clone(),
            superblocks: Mutex::default(),
        })
    }

    /// The trace of the search of the superblock at `root` in tile `tile`,
    /// which quantizes at `qindex`, prices with `lambda` and codes `segment`;
    /// `None` outside the region.
    pub(crate) fn superblock(
        &self,
        tile: usize,
        root: PartitionNode,
        (qindex, lambda, segment): (u32, Lambda, Option<u8>),
    ) -> Option<SearchTrace> {
        let request = &self.spec.request;
        let mut trace = SearchTrace {
            tile,
            region: request.region,
            kinds: request.kinds,
            cap: self.spec.cap,
            events: Vec::new(),
            seen: 0,
            failed: false,
        };
        let Some(block) = root.block_id(tile) else {
            trace.failed = true;
            return Some(trace);
        };
        if request
            .region
            .is_some_and(|region| !overlaps(region, block.coded))
        {
            return None;
        }
        trace.store(DecisionEvent::Superblock(SuperblockSearch {
            block,
            qindex,
            lambda_q16: lambda.sse_per_bit_q16(),
            segment,
        }));
        Some(trace)
    }

    /// Takes the finished `trace` of superblock `index`, and frees the events
    /// of every superblock that the events of the superblocks before it
    /// already push out of the budget.
    pub(crate) fn collect(&self, index: usize, trace: SearchTrace) {
        let Ok(mut superblocks) = self.superblocks.lock() else {
            return;
        };
        superblocks.insert(index, trace);
        let mut kept = 0;
        for trace in superblocks.values_mut() {
            if kept >= self.spec.cap {
                trace.events = Vec::new();
            }
            kept += trace.events.len();
        }
    }

    /// The events of the frame within the budget, superblocks in raster
    /// order, up to the first superblock whose trace failed.
    pub(crate) fn finish(self) -> FrameTrace {
        let mut frame = FrameTrace::default();
        let Ok(superblocks) = self.superblocks.into_inner() else {
            frame.failed = true;
            return frame;
        };
        for trace in superblocks.into_values() {
            frame.seen += trace.seen;
            if frame.failed {
                continue;
            }
            let room = self.spec.cap - frame.events.len();
            let take = trace.events.len().min(room);
            frame.failed = trace.failed || frame.events.try_reserve(take).is_err();
            if !frame.failed {
                frame.events.extend(trace.events.into_iter().take(room));
            }
        }
        frame
    }
}

/// The state of one capture, which the encoder feeds in coding order.
#[derive(Debug)]
pub(crate) struct Capture {
    request: DecisionTraceRequest,
    events: Vec<DecisionEvent>,
    dropped: u64,
    stop: Option<DecisionTraceStop>,
    /// The codings recorded of the frame in coding.
    attempts: u32,
}

impl Capture {
    pub(crate) const fn new(request: DecisionTraceRequest) -> Self {
        Self {
            request,
            events: Vec::new(),
            dropped: 0,
            stop: None,
            attempts: 0,
        }
    }

    /// The first input index after the selected frames.
    pub(crate) const fn end(&self) -> u64 {
        self.request.frames.end
    }

    /// What the frame coder collects for the next frames; `None` after a stop.
    pub(crate) fn spec(&self) -> Option<FrameSpec> {
        self.stop.is_none().then(|| FrameSpec {
            request: self.request.clone(),
            cap: self.request.limit() - self.events.len(),
        })
    }

    /// Records the coding `attempt` of a frame and the block events of a
    /// committed one, when the request selects the frame. `attempt.attempt`
    /// is set here. A coding without its record fails the capture.
    pub(crate) fn frame(&mut self, attempt: Option<FrameAttempt>, blocks: Option<FrameTrace>) {
        if self.stop.is_some() {
            return;
        }
        let Some(mut attempt) = attempt else {
            self.stop = Some(DecisionTraceStop::CaptureFailed);
            return;
        };
        if !self.request.frames.contains(&attempt.plan.display_index) {
            return;
        }
        attempt.attempt = self.attempts;
        self.attempts = match attempt.outcome {
            FrameOutcome::Recoded => self.attempts + 1,
            FrameOutcome::Committed(_) => 0,
        };
        let blocks = blocks.unwrap_or_default();
        let total = 1 + blocks.seen;
        let room = self.request.limit() - self.events.len();
        let before = self.events.len();
        if self
            .events
            .try_reserve((1 + blocks.events.len()).min(room))
            .is_ok()
        {
            let events = core::iter::once(DecisionEvent::Frame(attempt)).chain(blocks.events);
            self.events.extend(events.take(room));
        }
        let recorded = (self.events.len() - before) as u64;
        self.dropped += total - recorded;
        if blocks.failed || recorded == 0 && room > 0 {
            self.stop = Some(DecisionTraceStop::CaptureFailed);
        } else if recorded < total {
            self.stop = Some(DecisionTraceStop::Budget);
        }
    }

    /// The trace; `end` says why the encoder has not committed every
    /// selected frame, when it has not.
    pub(crate) fn finish(mut self, end: Option<DecisionTraceStop>) -> DecisionTrace {
        self.events.shrink_to_fit();
        let none = self.events.is_empty();
        let stop = self
            .stop
            .or(end)
            .or(none.then_some(DecisionTraceStop::NoFrame));
        DecisionTrace {
            status: match stop {
                None => DecisionTraceStatus::Complete,
                Some(stop) if none => DecisionTraceStatus::Dropped(stop),
                Some(stop) => DecisionTraceStatus::Partial(stop),
            },
            events: self.events,
            dropped_events: self.dropped,
        }
    }
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
