// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! In-flight frames as row-scheduled stages: a [`SplitFrame`] per single-tile
//! inter frame, a [`TipFrame`] per TIP output frame, and a [`FinishFrame`]
//! for the filter phase of a frame the driver walked itself.
//!
//! A [`SplitFrame`] runs on the [`RowScheduler`] as seven stages: parse (the
//! whole tile, publishing a unit count as it goes), prepare (the temporal
//! prelude and the per-tile reconstruction state), attach (the filter state
//! the parse settles last), resolve (one temporal band at a time), recon (one
//! batch of superblocks at a time, in order, straight into the frame
//! workspace), frontier (one superblock row at a time: in-place deblock behind
//! the lead, then the filter stripes that row releases), and output (the
//! freeze). Readiness is read from the frame's own counters and from the
//! cells its references publish; nothing registers a waiter.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::{Mutex, RwLock};
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_parallel::{Claim, FrameWork, ParallelStage, RowScheduler, RowWake, Stage};
use splot_recon::BitDepth;

use super::inflight::{PendingFinish, PipelineFrameSlot, RefFrameSlot, SpareFramePlanes};
use crate::bitstream::byte_stream::UnitBytes;
use crate::bitstream::tile_payload::FrameQuantizerSnapshot;
use crate::error::DecodeError;
use crate::filters::wienerns_lr::FrameFilterRecords;
use crate::prediction::inter::{self, ScheduledTileRecon, ScheduledTileWorkspace};
use crate::reference::buffer::RefSlots;
use crate::{DecodeOptions, DecodePlannedObu, Result};
use splot_core::span::ByteOffset;

const PARSE: usize = 0;
const PREPARE: usize = 1;
const ATTACH: usize = 2;
const RESOLVE: usize = 3;
const COMMIT: usize = 4;
const FRONTIER: usize = 5;
const OUTPUT: usize = 6;
const PRECOMPUTE: usize = 7;
const FILTER: usize = 8;

/// A sample type the row-scheduled frame works are typed for.
pub(crate) trait SplitSample: SpareFramePlanes + Send + 'static {
    const BIT_DEPTH: BitDepth;
    fn erase(slot: RefFrameSlot<Self>) -> PipelineFrameSlot;
    fn tip_pool(pools: &mut RowPools) -> &mut WorkPool<TipFrame<Self>>;
    fn tip_spares(pools: &RowPools) -> &Arc<inter::TipSpares<Self>>;
    fn finish_pool(pools: &mut RowPools) -> &mut WorkPool<FinishFrame<Self>>;
}

impl SplitSample for u8 {
    const BIT_DEPTH: BitDepth = BitDepth::Eight;
    fn erase(slot: RefFrameSlot<Self>) -> PipelineFrameSlot {
        PipelineFrameSlot::Eight(slot)
    }
    fn tip_pool(pools: &mut RowPools) -> &mut WorkPool<TipFrame<Self>> {
        &mut pools.tip_eight
    }
    fn tip_spares(pools: &RowPools) -> &Arc<inter::TipSpares<Self>> {
        &pools.tip_spares_eight
    }
    fn finish_pool(pools: &mut RowPools) -> &mut WorkPool<FinishFrame<Self>> {
        &mut pools.finish_eight
    }
}

impl SplitSample for u16 {
    const BIT_DEPTH: BitDepth = BitDepth::Ten;
    fn erase(slot: RefFrameSlot<Self>) -> PipelineFrameSlot {
        PipelineFrameSlot::Ten(slot)
    }
    fn tip_pool(pools: &mut RowPools) -> &mut WorkPool<TipFrame<Self>> {
        &mut pools.tip_ten
    }
    fn tip_spares(pools: &RowPools) -> &Arc<inter::TipSpares<Self>> {
        &pools.tip_spares_ten
    }
    fn finish_pool(pools: &mut RowPools) -> &mut WorkPool<FinishFrame<Self>> {
        &mut pools.finish_ten
    }
}

/// What the driver settles for one frame before its parse starts.
pub(crate) struct SplitFrameInputs<T: SplitSample> {
    pub(crate) records: FrameFilterRecords,
    pub(crate) candidate: DecodePlannedObu,
    pub(crate) core: Arc<FrameHeaderCore>,
    pub(crate) sequence: Arc<SequenceHeader>,
    pub(crate) reference: inter::InterReferenceState<T>,
    pub(crate) bit_depth: BitDepth,
    pub(crate) geometry: inter::FrameDecodeGeometry,
    pub(crate) motion: inter::MotionFieldHandle,
    pub(crate) parse_progress: Arc<inter::ParseProgress>,
    pub(crate) quantizer: FrameQuantizerSnapshot,
    pub(crate) products: inter::FrameProductWriters,
    pub(crate) unit: UnitBytes,
    pub(crate) entropy: inter::EntropyDependencies,
    pub(crate) motion_deps: RefSlots<Option<inter::MotionFieldHandle>>,
    pub(crate) finish: PendingFinish<T>,
}

/// One inter frame's row-scheduled state, reused across frames by its slot.
pub(crate) struct SplitFrame<T: SplitSample> {
    wake: RowWake,
    options: DecodeOptions,
    stages: [Stage; 7],
    precompute: ParallelStage,
    filter: ParallelStage,
    filter_jobs: Mutex<Vec<Option<crate::filters::wienerns_lr::recon::OwnedFilterJob<T>>>>,
    filter_error: Mutex<Option<DecodeError>>,
    early_ready: AtomicBool,
    parse_done: AtomicBool,
    prepared: AtomicBool,
    attached: AtomicBool,
    resolved_batches: AtomicUsize,
    awaiting_units: AtomicUsize,
    closed_rows: AtomicUsize,
    output_ready: AtomicBool,
    failed: AtomicBool,
    inputs: Mutex<Option<SplitFrameInputs<T>>>,
    motion: Mutex<Option<inter::MotionFieldHandle>>,
    entropy: OnceLock<inter::EntropyDependencies>,
    motion_deps: OnceLock<RefSlots<Option<inter::MotionFieldHandle>>>,
    parse_progress: OnceLock<Arc<inter::ParseProgress>>,
    early: Mutex<Option<inter::InterWalkEarly<T>>>,
    deferred: Mutex<Option<inter::DeferredInterWalk<T>>>,
    pending_filters: Mutex<Option<inter::PendingFilterAttach<T>>>,
    finish: Mutex<Option<PendingFinish<T>>>,
    output_filter: Mutex<Option<crate::filters::wienerns_lr::recon::OwnedFilterFinish<T>>>,
    recon: OnceLock<ScheduledTileRecon<T>>,
    workspace: Mutex<ScheduledTileWorkspace<T>>,
    temporal: Mutex<Arc<inter::TemporalMvContext>>,
    workers: Arc<inter::InterReconScratchPool<T>>,
    scratch: Mutex<Option<inter::InterDecodeScratch<T>>>,
    filter_shell: Mutex<Option<crate::filters::wienerns_lr::recon::OwnedFilterShell<T>>>,
}

impl<T: SplitSample> SplitFrame<T> {
    fn new(
        wake: RowWake,
        options: DecodeOptions,
        workers: Arc<inter::InterReconScratchPool<T>>,
    ) -> Self {
        Self {
            wake,
            options,
            stages: core::array::from_fn(|_| Stage::new(0)),
            precompute: ParallelStage::new(),
            filter: ParallelStage::new(),
            filter_jobs: Mutex::new(Vec::new()),
            filter_error: Mutex::new(None),
            early_ready: AtomicBool::new(false),
            parse_done: AtomicBool::new(false),
            prepared: AtomicBool::new(false),
            attached: AtomicBool::new(false),
            resolved_batches: AtomicUsize::new(0),
            awaiting_units: AtomicUsize::new(0),
            closed_rows: AtomicUsize::new(0),
            output_ready: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            inputs: Mutex::new(None),
            motion: Mutex::new(None),
            entropy: OnceLock::new(),
            motion_deps: OnceLock::new(),
            parse_progress: OnceLock::new(),
            early: Mutex::new(None),
            deferred: Mutex::new(None),
            pending_filters: Mutex::new(None),
            finish: Mutex::new(None),
            output_filter: Mutex::new(None),
            recon: OnceLock::new(),
            workspace: Mutex::new(ScheduledTileWorkspace::default()),
            temporal: Mutex::new(Arc::new(inter::TemporalMvContext::empty())),
            workers,
            scratch: Mutex::new(None),
            filter_shell: Mutex::new(None),
        }
    }

    /// Returns the slot's storage to its idle state, or `false` while a
    /// reader of the last frame still holds part of it.
    fn reset(&mut self) -> bool {
        if let Some(recon) = self.recon.take() {
            *self.workspace.get_mut() = recon.retire();
        }
        if Arc::get_mut(self.temporal.get_mut()).is_none() {
            return false;
        }
        let workspace = self.workspace.get_mut();
        if !workspace.producer_storage_reusable() || !workspace.retire_reference_handles() {
            return false;
        }
        for stage in &mut self.stages {
            stage.reset(0);
        }
        self.precompute.set_rows(0);
        self.filter.set_rows(0);
        self.filter_jobs.get_mut().clear();
        self.filter_error.get_mut().take();
        *self.early_ready.get_mut() = false;
        *self.parse_done.get_mut() = false;
        *self.prepared.get_mut() = false;
        *self.attached.get_mut() = false;
        *self.resolved_batches.get_mut() = 0;
        *self.awaiting_units.get_mut() = 0;
        *self.closed_rows.get_mut() = 0;
        *self.output_ready.get_mut() = false;
        *self.failed.get_mut() = false;
        self.inputs.get_mut().take();
        self.motion.get_mut().take();
        self.entropy.take();
        self.motion_deps.take();
        self.parse_progress.take();
        self.early.get_mut().take();
        self.deferred.get_mut().take();
        self.pending_filters.get_mut().take();
        self.finish.get_mut().take();
        self.output_filter.get_mut().take();
        true
    }

    /// Loads one frame into an idle slot. The parse stage becomes claimable.
    fn arm(&mut self, mut inputs: SplitFrameInputs<T>) {
        *self.motion.get_mut() = Some(inputs.motion.clone());
        let _ = self.entropy.set(core::mem::take(&mut inputs.entropy));
        let _ = self
            .motion_deps
            .set(core::mem::take(&mut inputs.motion_deps));
        let _ = self.parse_progress.set(Arc::clone(&inputs.parse_progress));
        self.stages[PARSE].reset(1);
        self.stages[PREPARE].reset(1);
        self.stages[ATTACH].reset(1);
        self.stages[OUTPUT].reset(1);
        *self.inputs.get_mut() = Some(inputs);
    }

    fn notify(&self) {
        self.wake.notify();
    }

    fn recon_state(&self) -> Result<&ScheduledTileRecon<T>> {
        self.recon
            .get()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)
    }

    fn parse(&self) -> Result<()> {
        let inputs = self
            .inputs
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let SplitFrameInputs {
            records,
            candidate,
            core,
            sequence,
            reference,
            bit_depth,
            geometry,
            motion,
            parse_progress,
            quantizer,
            products,
            unit,
            entropy: _,
            motion_deps: _,
            finish,
        } = inputs;
        *self.finish.lock() = Some(finish);
        let start = inter::InterFrameStart {
            records,
            candidate,
            core,
            sequence,
            options: &self.options,
            reference,
            bit_depth,
            geometry,
            motion,
            parse_progress,
            quantizer,
            products,
        };
        let (early, mut pending) = {
            let mut workspace = self.workspace.lock();
            start.run(unit.view(), &mut workspace)?
        };
        *self.early.lock() = Some(early);
        self.early_ready.store(true, Ordering::Release);
        self.notify();
        while !pending.parse_units(core::num::NonZeroUsize::MAX)? {}
        let mut deferred = pending.finish()?;
        deferred.publish_products()?;
        *self.deferred.lock() = Some(deferred);
        self.parse_done.store(true, Ordering::Release);
        Ok(())
    }

    fn prepare(&self) -> Result<()> {
        let early = self
            .early
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let finish = self.finish.lock();
        let progress = finish
            .as_ref()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?
            .progress_handle();
        drop(finish);
        let mut scratch = self.scratch.lock().take().unwrap_or_default();
        if let Some(buffers) = progress.buffers() {
            scratch.set_decode_buffers(buffers);
        }
        let (scheduled, pending_filters) = early.prepare_scheduled(
            scratch,
            &mut self.workspace.lock(),
            &mut self.temporal.lock(),
            Arc::clone(&self.workers),
            progress,
        )?;
        if scheduled.resolve_len() == 0 {
            self.resolved_batches
                .store(scheduled.len(), Ordering::Release);
        }
        self.stages[RESOLVE].set_rows(scheduled.resolve_len());
        self.precompute.set_rows(scheduled.len());
        self.stages[COMMIT].set_rows(scheduled.len());
        self.stages[FRONTIER].set_rows(scheduled.frontier_len());
        self.filter.set_rows(scheduled.filter_count());
        self.filter_jobs
            .lock()
            .resize_with(scheduled.filter_count(), || None);
        *self.pending_filters.lock() = Some(pending_filters);
        if self.recon.set(scheduled).is_err() {
            return Err(inter::invalid_inter_tile_scheduling_state());
        }
        self.prepared.store(true, Ordering::Release);
        Ok(())
    }

    fn attach(&self) -> Result<()> {
        let deferred = self
            .deferred
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let pending = self
            .pending_filters
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let shell = self
            .filter_shell
            .lock()
            .take()
            .unwrap_or_else(|| Arc::new(None));
        deferred.attach_filters(pending, self.recon_state()?, shell)?;
        self.attached.store(true, Ordering::Release);
        Ok(())
    }

    /// Resolves one temporal band; `false` when the parse has not reached
    /// every unit the band owes, so the row is claimed again later.
    fn resolve(&self, band: usize) -> Result<bool> {
        let recon = self.recon_state()?;
        let (batches, awaiting) = recon.resolve(band)?;
        self.resolved_batches
            .fetch_max(batches.end, Ordering::AcqRel);
        self.awaiting_units
            .store(awaiting.unwrap_or(0), Ordering::Release);
        Ok(awaiting.is_none())
    }

    fn precompute(&self, batch: usize) -> Result<()> {
        self.recon_state()?.precompute(batch)
    }

    fn commit(&self, batch: usize) -> Result<()> {
        let recon = self.recon_state()?;
        let progress = recon.commit(batch)?;
        if progress.recon_complete {
            *self.scratch.lock() = Some(recon.take_scheduled_scratch()?);
        }
        self.closed_rows
            .fetch_max(progress.frontier_rows.end, Ordering::AcqRel);
        Ok(())
    }

    fn frontier(&self, row: usize) -> Result<()> {
        let recon = self.recon_state()?;
        let mut progress = recon.frontier(row)?;
        let mut outcome = Ok(());
        {
            let mut jobs = self.filter_jobs.lock();
            for filter in progress.filters.drain(..) {
                let stripe = filter.stripe();
                let Some(slot) = jobs.get_mut(stripe) else {
                    outcome = Err(super::unsupported(
                        "inter_admission_filter_index",
                        None,
                        "scheduled filter stripe index is out of range",
                    ));
                    break;
                };
                *slot = Some(filter);
                self.filter.ready(stripe);
            }
        }
        recon.recycle_filter_jobs(core::mem::take(&mut progress.filters));
        outcome?;
        if let Some(filter) = progress.output {
            *self.output_filter.lock() = Some(filter);
            self.output_ready.store(true, Ordering::Release);
        }
        Ok(())
    }

    /// Runs one filter stripe; the frame's first stripe error fails its output.
    fn filter_stripe(&self, stripe: usize) -> Result<()> {
        let job = self
            .filter_jobs
            .lock()
            .get_mut(stripe)
            .and_then(Option::take)
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        if let Err(error) = job.run() {
            let mut owed = self.filter_error.lock();
            if owed.is_none() {
                *owed = Some(error);
            }
        }
        Ok(())
    }

    fn output(&self) -> Result<()> {
        if let Some(error) = self.filter_error.lock().take() {
            return Err(error);
        }
        let finish = self
            .finish
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let filter = self
            .output_filter
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        *self.filter_shell.lock() = Some(finish.run_owned_finish(filter));
        Ok(())
    }

    /// Settles every product of a failed frame so its dependents fail closed.
    fn fail(&self, error: DecodeError) {
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(recon) = self.recon.get() {
            recon.fail_temporal();
        }
        if let Some(motion) = self.motion.lock().as_ref() {
            motion.fail();
        }
        if let Some(finish) = self.finish.lock().take() {
            finish.fail(error);
        }
        self.notify();
    }
}

impl<T: SplitSample> FrameWork for SplitFrame<T> {
    fn claim(&self) -> Option<Claim> {
        if self.failed.load(Ordering::Acquire) {
            return None;
        }
        let claim = |stage: usize, row: usize| Claim { stage, row };
        if let Some(row) = self.stages[PARSE].claim(|_| {
            self.entropy
                .get()
                .is_none_or(inter::EntropyDependencies::is_settled)
        }) {
            return Some(claim(PARSE, row));
        }
        if let Some(row) = self.stages[PREPARE].claim(|_| {
            self.early_ready.load(Ordering::Acquire)
                && self.motion_deps.get().is_none_or(|deps| {
                    deps.iter()
                        .flatten()
                        .all(inter::MotionFieldHandle::metadata_settled)
                })
        }) {
            return Some(claim(PREPARE, row));
        }
        if let Some(row) = self.stages[ATTACH].claim(|_| {
            self.parse_done.load(Ordering::Acquire) && self.prepared.load(Ordering::Acquire)
        }) {
            return Some(claim(ATTACH, row));
        }
        let recon = self.recon.get()?;
        if let Some(row) = self.stages[RESOLVE].claim(|band| {
            let awaiting = self.awaiting_units.load(Ordering::Acquire);
            (awaiting == 0
                || self
                    .parse_progress
                    .get()
                    .is_some_and(|parse| parse.cell().current() >= awaiting))
                && recon.resolve_ready(band)
        }) {
            return Some(claim(RESOLVE, row));
        }
        if let Some(row) = self.stages[COMMIT].claim(|batch| self.precompute.is_done(batch)) {
            return Some(claim(COMMIT, row));
        }
        if let Some(row) = self.stages[FRONTIER].claim(|row| {
            self.attached.load(Ordering::Acquire) && row < self.closed_rows.load(Ordering::Acquire)
        }) {
            return Some(claim(FRONTIER, row));
        }
        if let Some(stripe) = self.filter.claim_ready() {
            return Some(claim(FILTER, stripe));
        }
        if let Some(row) = self.precompute.claim(|batch| {
            batch < self.resolved_batches.load(Ordering::Acquire) && recon.batch_ready(batch)
        }) {
            return Some(claim(PRECOMPUTE, row));
        }
        if let Some(row) = self.stages[OUTPUT].claim(|_| {
            self.output_ready.load(Ordering::Acquire)
                && self.stages[FRONTIER].is_finished()
                && self.filter.is_finished()
        }) {
            return Some(claim(OUTPUT, row));
        }
        None
    }

    fn run(&self, claim: Claim) {
        let outcome = match claim.stage {
            PARSE => self.parse().map(|()| true),
            PREPARE => self.prepare().map(|()| true),
            ATTACH => self.attach().map(|()| true),
            RESOLVE => self.resolve(claim.row),
            COMMIT => self.commit(claim.row).map(|()| true),
            FRONTIER => self.frontier(claim.row).map(|()| true),
            PRECOMPUTE => self.precompute(claim.row).map(|()| true),
            FILTER => self.filter_stripe(claim.row).map(|()| true),
            _ => self.output().map(|()| true),
        };
        match claim.stage {
            PRECOMPUTE => self.precompute.finish(claim.row),
            FILTER => self.filter.finish(claim.row),
            stage => match outcome {
                Ok(false) => self.stages[stage].retry(claim.row),
                Ok(true) | Err(_) => self.stages[stage].finish(claim.row),
            },
        }
        if let Err(error) = outcome {
            self.fail(error);
        }
        self.notify();
    }

    fn done(&self) -> bool {
        self.failed.load(Ordering::Acquire) || self.stages[OUTPUT].is_finished()
    }
}

/// The `D` reusable slots one decode runs its split inter frames through.
pub(crate) struct SplitFrames<T: SplitSample> {
    slots: Vec<Arc<SplitFrame<T>>>,
    next: usize,
    depth: usize,
    workers: Arc<inter::InterReconScratchPool<T>>,
}

impl<T: SplitSample> SplitFrames<T> {
    pub(crate) fn new(depth: usize, threads: usize) -> Self {
        let workers: Arc<inter::InterReconScratchPool<T>> = Arc::default();
        workers.ensure_workers(threads.max(1));
        Self {
            slots: Vec::new(),
            next: 0,
            depth: depth.max(1),
            workers,
        }
    }

    /// Frees the next slot, waiting for its previous frame to finish and
    /// release the storage other frames borrowed from it.
    pub(crate) fn reserve(&mut self, rows: &RowScheduler, options: DecodeOptions) -> usize {
        let index = self.next;
        self.next = (index + 1) % self.depth;
        if index == self.slots.len() {
            self.slots.push(Arc::new(SplitFrame::new(
                rows.wake(),
                options,
                Arc::clone(&self.workers),
            )));
        }
        let slot = &mut self.slots[index];
        rows.wait(|| Arc::strong_count(slot) == 1);
        loop {
            if let Some(frame) = Arc::get_mut(slot)
                && frame.reset()
            {
                frame.options = options;
                return index;
            }
            std::thread::yield_now();
        }
    }

    /// Loads one frame into the slot [`Self::reserve`] freed and hands it to
    /// the scheduler.
    pub(crate) fn push(&mut self, rows: &RowScheduler, index: usize, inputs: SplitFrameInputs<T>) {
        let slot = &mut self.slots[index];
        if let Some(frame) = Arc::get_mut(slot) {
            frame.arm(inputs);
        }
        rows.push(Arc::clone(slot) as Arc<dyn FrameWork>);
    }

    /// Waits for every slot's frame to finish.
    pub(crate) fn drain(&mut self, rows: &RowScheduler) {
        for slot in &mut self.slots {
            rows.wait(|| Arc::strong_count(slot) == 1);
        }
    }
}

/// What the driver settles for one `TIP_FRAME_AS_OUTPUT` frame.
pub(crate) struct TipFrameInputs<T: SplitSample> {
    pub(crate) candidate: DecodePlannedObu,
    pub(crate) core: Arc<FrameHeaderCore>,
    pub(crate) sequence: Arc<SequenceHeader>,
    pub(crate) reference: inter::InterReferenceState<T>,
    pub(crate) geometry: inter::FrameDecodeGeometry,
    pub(crate) products: inter::FrameProductWriters,
    pub(crate) motion: inter::MotionFieldHandle,
    pub(crate) finish: PendingFinish<T>,
    pub(crate) entropy: inter::EntropyDependencies,
    pub(crate) motion_deps: RefSlots<Option<inter::MotionFieldHandle>>,
    pub(crate) offset: ByteOffset,
}

const TIP_INIT: usize = 0;
const TIP_FINISH: usize = 1;
const TIP_BAND: usize = 2;

/// One TIP output frame as row-scheduled stages: the projection, the bands
/// (any order, concurrently, each admitted once its references published the
/// rows it reads, each committing the finished prefix in band order), and the
/// deblock plus freeze.
pub(crate) struct TipFrame<T: SplitSample> {
    wake: RowWake,
    options: DecodeOptions,
    /// Set when the stages run on the driver, which predicts bands in place.
    inline: bool,
    spares: Arc<inter::TipSpares<T>>,
    stages: [Stage; 2],
    bands: ParallelStage,
    failed: AtomicBool,
    inputs: Mutex<Option<TipFrameInputs<T>>>,
    core: OnceLock<Arc<FrameHeaderCore>>,
    sequence: OnceLock<Arc<SequenceHeader>>,
    /// Taken by the stage that last reads them, so the handles they share
    /// retire before the slot is reused.
    reference: RwLock<Option<inter::InterReferenceState<T>>>,
    entropy: Mutex<Option<inter::EntropyDependencies>>,
    motion_deps: Mutex<Option<RefSlots<Option<inter::MotionFieldHandle>>>>,
    band_bounds: OnceLock<Vec<inter::RowReferenceBounds>>,
    work: RwLock<Option<inter::TipOutputStages<T>>>,
    motion: Mutex<Option<inter::MotionFieldHandle>>,
    finish: Mutex<Option<PendingFinish<T>>>,
}

impl<T: SplitSample> TipFrame<T> {
    fn new(wake: RowWake, options: DecodeOptions, spares: Arc<inter::TipSpares<T>>) -> Self {
        Self {
            wake,
            options,
            inline: false,
            spares,
            stages: core::array::from_fn(|_| Stage::new(0)),
            bands: ParallelStage::new(),
            failed: AtomicBool::new(false),
            inputs: Mutex::new(None),
            core: OnceLock::new(),
            sequence: OnceLock::new(),
            reference: RwLock::new(None),
            entropy: Mutex::new(None),
            motion_deps: Mutex::new(None),
            band_bounds: OnceLock::new(),
            work: RwLock::new(None),
            motion: Mutex::new(None),
            finish: Mutex::new(None),
        }
    }

    fn reset(&mut self) -> bool {
        for stage in &mut self.stages {
            stage.reset(0);
        }
        self.bands.set_rows(0);
        *self.failed.get_mut() = false;
        self.inputs.get_mut().take();
        self.core.take();
        self.sequence.take();
        self.reference.get_mut().take();
        self.entropy.get_mut().take();
        self.motion_deps.get_mut().take();
        self.band_bounds.take();
        self.work.get_mut().take();
        self.motion.get_mut().take();
        self.finish.get_mut().take();
        true
    }

    fn arm(&mut self, mut inputs: TipFrameInputs<T>) {
        let _ = self.core.set(Arc::clone(&inputs.core));
        let _ = self.sequence.set(Arc::clone(&inputs.sequence));
        *self.entropy.get_mut() = Some(core::mem::take(&mut inputs.entropy));
        *self.motion_deps.get_mut() = Some(core::mem::take(&mut inputs.motion_deps));
        *self.motion.get_mut() = Some(inputs.motion.clone());
        self.stages[TIP_INIT].reset(1);
        self.stages[TIP_FINISH].reset(1);
        *self.inputs.get_mut() = Some(inputs);
    }

    fn init(&self) -> Result<()> {
        self.entropy.lock().take();
        self.motion_deps.lock().take();
        let TipFrameInputs {
            candidate,
            core,
            sequence,
            reference,
            geometry,
            mut products,
            motion: _,
            finish,
            entropy: _,
            motion_deps: _,
            offset,
        } = self
            .inputs
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let progress = finish.progress_handle();
        *self.finish.lock() = Some(finish);
        let planes = progress.take_unfiltered_planes().unwrap_or_else(|| {
            splot_recon::FramePlaneSamples::default()
                .with_pool(progress.buffers().map(|buffers| buffers.planes()))
        });
        let (cdfs, segments) = inter::tip_output_products(
            &core,
            &candidate,
            offset,
            &sequence,
            &self.options,
            &reference,
            &geometry,
            &mut products,
        )?;
        let _ = products.settle(cdfs, None, segments);
        let work = inter::TipOutputStages::init(
            Arc::clone(&self.spares),
            planes,
            &sequence,
            &core,
            &reference,
            geometry,
            offset,
            !self.inline,
        )?;
        self.bands.set_rows(work.band_count());
        let _ = self.band_bounds.set(work.band_bounds().to_vec());
        *self.work.write() = Some(work);
        *self.reference.write() = Some(reference);
        Ok(())
    }

    fn band(&self, index: usize) -> Result<()> {
        let (Some(core), Some(sequence)) = (self.core.get(), self.sequence.get()) else {
            return Err(inter::invalid_inter_tile_scheduling_state());
        };
        let reference = self.reference.read();
        let reference = reference
            .as_ref()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let work = self.work.read();
        let work = work
            .as_ref()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let field = if self.inline {
            work.band_in_place(index, sequence, core, reference)?
        } else {
            work.band(index, sequence, core, reference)?;
            work.commit_ready()?
        };
        if let Some(field) = field
            && let Some(motion) = self.motion.lock().take()
        {
            motion.publish(field);
        }
        Ok(())
    }

    fn finish_frame(&self) -> Result<()> {
        let (Some(core), Some(sequence)) = (self.core.get(), self.sequence.get()) else {
            return Err(inter::invalid_inter_tile_scheduling_state());
        };
        let work = self
            .work
            .write()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let finish = self
            .finish
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let (frame, output, temporal) = work.finish(sequence, core)?;
        self.reference.write().take();
        if let Some(motion) = self.motion.lock().take() {
            motion.fail();
        }
        finish.complete_frame(frame);
        self.spares.give(temporal, output);
        Ok(())
    }

    fn fail(&self, error: DecodeError) {
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.inputs.lock().take();
        self.reference.write().take();
        if let Some(motion) = self.motion.lock().take() {
            motion.fail();
        }
        if let Some(finish) = self.finish.lock().take() {
            finish.fail(error);
        }
        self.wake.notify();
    }
}

impl<T: SplitSample> FrameWork for TipFrame<T> {
    fn claim(&self) -> Option<Claim> {
        if self.failed.load(Ordering::Acquire) {
            return None;
        }
        let (entropy, motion_deps) = (self.entropy.try_lock()?, self.motion_deps.try_lock()?);
        if let Some(row) = self.stages[TIP_INIT].claim(|_| {
            entropy
                .as_ref()
                .is_none_or(inter::EntropyDependencies::is_settled)
                && motion_deps.as_ref().is_none_or(|deps| {
                    deps.iter()
                        .flatten()
                        .all(inter::MotionFieldHandle::field_settled)
                })
        }) {
            return Some(Claim {
                stage: TIP_INIT,
                row,
            });
        }
        let (Some(core), Some(bounds)) = (self.core.get(), self.band_bounds.get()) else {
            return None;
        };
        let reference = self.reference.try_read()?;
        let reference = reference.as_ref()?;
        let ref_frame_idx = core
            .inter
            .as_ref()
            .map_or(&[][..], |inter| inter.ref_frame_idx.as_slice());
        if let Some(row) = self.bands.claim(|band| {
            bounds
                .get(band)
                .is_none_or(|bound| inter::bounds_satisfied(reference, core, ref_frame_idx, bound))
        }) {
            return Some(Claim {
                stage: TIP_BAND,
                row,
            });
        }
        if let Some(row) = self.stages[TIP_FINISH].claim(|_| self.bands.is_finished()) {
            return Some(Claim {
                stage: TIP_FINISH,
                row,
            });
        }
        None
    }

    fn run(&self, claim: Claim) {
        let outcome = match claim.stage {
            TIP_INIT => self.init(),
            TIP_BAND => self.band(claim.row),
            _ => self.finish_frame(),
        };
        if claim.stage == TIP_BAND {
            self.bands.finish(claim.row);
        } else {
            self.stages[claim.stage].finish(claim.row);
        }
        if let Err(error) = outcome {
            self.fail(error);
        }
        self.wake.notify();
    }

    fn done(&self) -> bool {
        self.failed.load(Ordering::Acquire) || self.stages[TIP_FINISH].is_finished()
    }
}

/// Reserves the pending frame and product handles published by one TIP job.
pub(crate) fn reserve_tip_output<T: SpareFramePlanes>(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    bit_depth: BitDepth,
    erase: fn(RefFrameSlot<T>) -> PipelineFrameSlot,
    frames: &mut super::FrameStore,
    ring: &mut super::inflight::InflightRing,
    frame_index: usize,
) -> Result<(
    PipelineFrameSlot,
    PendingFinish<T>,
    inter::FrameDecodeGeometry,
    inter::FrameProductWriters,
    inter::MotionFieldHandle,
)> {
    let geometry = inter::FrameDecodeGeometry::new(core, sequence, bit_depth, false)?;
    let motion = frames.reserve_motion(geometry.motion_layout())?;
    let (slot, finish) =
        super::inflight::reserve_pending_slot(geometry.info(), erase, ring, frames, frame_index)?;
    let products = frames.reserve_products()?;
    Ok((slot, finish, geometry, products, motion))
}

const FINISH_INIT: usize = 0;
const FINISH_ADVANCE: usize = 1;
const FINISH_OUTPUT: usize = 2;
const FINISH_FILTER: usize = 3;

/// The filter phase of a frame the driver walked itself: the setup, the
/// deblock advancing one stripe at a time and releasing that stripe's filter
/// job, the filter stripes (any order, concurrently), and the freeze.
pub(crate) struct FinishFrame<T: SplitSample> {
    wake: RowWake,
    stages: [Stage; 3],
    filter: ParallelStage,
    started: AtomicBool,
    failed: AtomicBool,
    job: Mutex<
        Option<(
            PendingFinish<T>,
            super::frame_engine::finish::WalkedFrame<T>,
        )>,
    >,
    finish: Mutex<Option<PendingFinish<T>>>,
    frontier: Mutex<Option<crate::filters::wienerns_lr::recon::OwnedFilterFrontier<T>>>,
    filter_jobs: Mutex<Vec<Option<crate::filters::wienerns_lr::recon::OwnedFilterJob<T>>>>,
    filter_error: Mutex<Option<DecodeError>>,
    /// The emptied setup shell of the last frame, reused by the next.
    filter_shell: Mutex<Option<crate::filters::wienerns_lr::recon::OwnedFilterShell<T>>>,
}

impl<T: SplitSample> FinishFrame<T> {
    fn new(wake: RowWake) -> Self {
        Self {
            wake,
            stages: core::array::from_fn(|_| Stage::new(0)),
            filter: ParallelStage::new(),
            started: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            job: Mutex::new(None),
            finish: Mutex::new(None),
            frontier: Mutex::new(None),
            filter_jobs: Mutex::new(Vec::new()),
            filter_error: Mutex::new(None),
            filter_shell: Mutex::new(None),
        }
    }

    fn reset(&mut self) -> bool {
        for stage in &mut self.stages {
            stage.reset(0);
        }
        self.filter.set_rows(0);
        *self.started.get_mut() = false;
        *self.failed.get_mut() = false;
        self.job.get_mut().take();
        self.finish.get_mut().take();
        self.frontier.get_mut().take();
        self.filter_jobs.get_mut().clear();
        self.filter_error.get_mut().take();
        true
    }

    fn arm(
        &mut self,
        finish: PendingFinish<T>,
        walked: super::frame_engine::finish::WalkedFrame<T>,
    ) {
        *self.job.get_mut() = Some((finish, walked));
        self.stages[FINISH_INIT].reset(1);
        self.stages[FINISH_OUTPUT].reset(1);
    }

    fn init(&self) -> Result<()> {
        let (finish, walked) = self
            .job
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let progress = finish.progress_handle();
        *self.finish.lock() = Some(finish);
        let frontier = walked.into_frontier(progress, self.filter_shell.lock().take())?;
        let stripes = frontier.stripes();
        self.filter_jobs.lock().resize_with(stripes, || None);
        self.filter.set_rows(stripes);
        self.stages[FINISH_ADVANCE].set_rows(stripes);
        *self.frontier.lock() = Some(frontier);
        self.started.store(true, Ordering::Release);
        Ok(())
    }

    fn advance(&self, stripe: usize) -> Result<()> {
        let job = self
            .frontier
            .lock()
            .as_mut()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?
            .advance()?
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        *self
            .filter_jobs
            .lock()
            .get_mut(stripe)
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)? = Some(job);
        self.filter.ready(stripe);
        Ok(())
    }

    fn filter_stripe(&self, stripe: usize) -> Result<()> {
        let job = self
            .filter_jobs
            .lock()
            .get_mut(stripe)
            .and_then(Option::take)
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        if let Err(error) = job.run() {
            let mut owed = self.filter_error.lock();
            if owed.is_none() {
                *owed = Some(error);
            }
        }
        Ok(())
    }

    fn output(&self) -> Result<()> {
        if let Some(error) = self.filter_error.lock().take() {
            return Err(error);
        }
        let finish = self
            .finish
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let frontier = self
            .frontier
            .lock()
            .take()
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        let filter = frontier.finish()?;
        *self.filter_shell.lock() = Some(finish.run_owned_finish(filter));
        Ok(())
    }

    fn fail(&self, error: DecodeError) {
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.job.lock().take();
        self.frontier.lock().take();
        if let Some(finish) = self.finish.lock().take() {
            finish.fail(error);
        }
        self.wake.notify();
    }
}

impl<T: SplitSample> FrameWork for FinishFrame<T> {
    fn claim(&self) -> Option<Claim> {
        if self.failed.load(Ordering::Acquire) {
            return None;
        }
        let claim = |stage: usize, row: usize| Claim { stage, row };
        if let Some(row) = self.stages[FINISH_INIT].claim(|_| true) {
            return Some(claim(FINISH_INIT, row));
        }
        if let Some(row) = self.stages[FINISH_ADVANCE].claim(|_| true) {
            return Some(claim(FINISH_ADVANCE, row));
        }
        if let Some(stripe) = self.filter.claim_ready() {
            return Some(claim(FINISH_FILTER, stripe));
        }
        if let Some(row) = self.stages[FINISH_OUTPUT].claim(|_| {
            self.started.load(Ordering::Acquire)
                && self.stages[FINISH_ADVANCE].is_finished()
                && self.filter.is_finished()
        }) {
            return Some(claim(FINISH_OUTPUT, row));
        }
        None
    }

    fn run(&self, claim: Claim) {
        let outcome = match claim.stage {
            FINISH_INIT => self.init(),
            FINISH_ADVANCE => self.advance(claim.row),
            FINISH_FILTER => self.filter_stripe(claim.row),
            _ => self.output(),
        };
        if claim.stage == FINISH_FILTER {
            self.filter.finish(claim.row);
        } else {
            self.stages[claim.stage].finish(claim.row);
        }
        if let Err(error) = outcome {
            self.fail(error);
        }
        self.wake.notify();
    }

    fn done(&self) -> bool {
        self.failed.load(Ordering::Acquire) || self.stages[FINISH_OUTPUT].is_finished()
    }
}

/// A growable pool of reusable frame works, for the frame kinds the in-flight
/// ring already bounds. Without a row scheduler the work runs inline.
pub(crate) struct WorkPool<W> {
    slots: Vec<Arc<W>>,
}

impl<W> Default for WorkPool<W> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

impl<W: FrameWork + 'static> WorkPool<W> {
    /// The index of an idle slot reset for reuse, or of a new one.
    fn reserve(&mut self, make: impl FnOnce() -> W, reset: impl Fn(&mut W) -> bool) -> usize {
        let idle = self
            .slots
            .iter_mut()
            .position(|slot| Arc::get_mut(slot).is_some_and(&reset));
        idle.unwrap_or_else(|| {
            self.slots.push(Arc::new(make()));
            self.slots.len() - 1
        })
    }

    fn push_slot(
        &mut self,
        rows: Option<&RowScheduler>,
        index: usize,
        arm: impl FnOnce(&mut W),
    ) -> Result<()> {
        let slot = self
            .slots
            .get_mut(index)
            .ok_or_else(inter::invalid_inter_tile_scheduling_state)?;
        arm(Arc::get_mut(slot).ok_or_else(inter::invalid_inter_tile_scheduling_state)?);
        if let Some(rows) = rows {
            rows.push(Arc::clone(slot) as Arc<dyn FrameWork>);
            return Ok(());
        }
        while let Some(claim) = slot.claim() {
            slot.run(claim);
        }
        if slot.done() {
            Ok(())
        } else {
            Err(inter::invalid_inter_tile_scheduling_state())
        }
    }

    fn drain(&mut self, rows: &RowScheduler) {
        for slot in &mut self.slots {
            rows.wait(|| Arc::strong_count(slot) == 1);
        }
    }
}

/// The reusable TIP output and finish works of one decode.
#[derive(Default)]
pub(crate) struct RowPools {
    tip_eight: WorkPool<TipFrame<u8>>,
    tip_ten: WorkPool<TipFrame<u16>>,
    tip_spares_eight: Arc<inter::TipSpares<u8>>,
    tip_spares_ten: Arc<inter::TipSpares<u16>>,
    finish_eight: WorkPool<FinishFrame<u8>>,
    finish_ten: WorkPool<FinishFrame<u16>>,
}

impl RowPools {
    /// Hands one TIP output frame to the scheduler, or decodes it inline.
    pub(crate) fn push_tip<T: SplitSample>(
        &mut self,
        rows: Option<&RowScheduler>,
        options: DecodeOptions,
        inputs: TipFrameInputs<T>,
    ) -> Result<()> {
        let spares = Arc::clone(T::tip_spares(self));
        let pool = T::tip_pool(self);
        let index = pool.reserve(
            || {
                TipFrame::new(
                    rows.map_or_else(RowWake::detached, RowScheduler::wake),
                    options,
                    spares,
                )
            },
            TipFrame::reset,
        );
        pool.push_slot(rows, index, |work| {
            work.options = options;
            work.inline = rows.is_none();
            work.arm(inputs);
        })
    }

    /// Hands one walked frame's filter phase to the scheduler, or runs it inline.
    pub(crate) fn push_finish<T: SplitSample>(
        &mut self,
        rows: Option<&RowScheduler>,
        finish: PendingFinish<T>,
        walked: super::frame_engine::finish::WalkedFrame<T>,
    ) -> Result<()> {
        let pool = T::finish_pool(self);
        let index = pool.reserve(
            || FinishFrame::new(rows.map_or_else(RowWake::detached, RowScheduler::wake)),
            FinishFrame::reset,
        );
        pool.push_slot(rows, index, |work| work.arm(finish, walked))
    }

    /// Waits for every work in every pool to finish.
    pub(crate) fn drain(&mut self, rows: &RowScheduler) {
        self.tip_eight.drain(rows);
        self.tip_ten.drain(rows);
        self.finish_eight.drain(rows);
        self.finish_ten.drain(rows);
    }
}
