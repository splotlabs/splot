// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame overlap across the mini-GOPs of a batch.
//!
//! The frames of a wave read the reference frame `P` the previous waves
//! refreshed last. They search on row facades whose references hold `P` as
//! its writer left it after superblock row `r + MARGIN`: the reconstruction
//! before loop filtering, the rows below repeating the last finished one, the
//! motion records and intra cells of those rows, and `P`'s start CDFs. That
//! snapshot depends on `P`'s finished rows alone, so the search does not
//! depend on timing; `P` taken from the previous batch is cut to the same
//! rows. A frame's writer starts once every earlier wave is committed, codes
//! the decisions with the exact references and CDFs, and quantizes again.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use splot_core::headers::frame::FrameHeaderCore;
use splot_decode::encoder_syntax::{EncoderReferences, FrameStart, ProvisionalFrame};

use super::{
    Committed, Error, FramePrep, IntraCells, IntraFrameCoder, KeyFrameTileWriter, MotionSearch,
    Result, SourceResidualCoder, coding,
};
use crate::encoder::{BatchOutput, BatchStep, CodedFrame};
use crate::frame::SourceFrame;
use crate::gop::GopFrame;
use crate::loop_filter::{CodedTile, FrameDecisions};
use crate::wavefront::{TileSearch, run_graph};

/// Superblock rows of the read frame finished past the row a frame searches.
const MARGIN: usize = 1;

/// Whether `coder` codes batches with frame overlap: the preset asks for it
/// and the frame reads no tool that needs its references finished as a
/// whole (warped motion, the TIP weight probe), nor anything the overlap
/// does not model (tiles, film grain, screen content).
pub(super) fn eligible(coder: &IntraFrameCoder) -> bool {
    let preset = &coder.preset;
    preset.frame_overlap
        && preset.warp == 0
        && preset.tip < 2
        && !preset.palette
        && !preset.intrabc
        && !coder.params.tools.film_grain
        && coder.tiles.cols_log2 == 0
        && coder.tiles.rows_log2 == 0
        && coder.view.frame_sb_size(true) == coder.view.frame_sb_size(false)
}

/// The frame a later batch reads first: its snapshot of every row and the
/// references it was stored into.
pub(super) struct Trail {
    read: Arc<Read>,
    full: Arc<Snapshot>,
}

/// A reference frame still being written, as the frames that read it see it.
struct Read {
    core: FrameHeaderCore,
    initial: FrameStart,
    /// The references after every step before it.
    before: EncoderReferences,
    slot_intra: [Option<Arc<IntraCells>>; 8],
    /// The show-existing slots between it and the reading wave.
    after: Vec<u32>,
}

/// One row snapshot of a frame being written.
struct Snapshot {
    frame: ProvisionalFrame,
    intra: Option<Arc<IntraCells>>,
}

/// The committed coder state after a wave.
struct Waved {
    refs: EncoderReferences,
    slot_intra: [Option<Arc<IntraCells>>; 8],
}

/// The state the commits of a batch change, moved out of the coder.
struct State {
    references: EncoderReferences,
    slot_intra: [Option<Arc<IntraCells>>; 8],
    warp_gate: crate::warp::WarpGate,
    grain_active: bool,
    shown_existing: u64,
}

/// Where a frame's read frame comes from.
enum Source {
    None,
    Batch(usize),
    Trail,
}

/// One frame of a batch.
struct Job<'a> {
    source: &'a SourceFrame,
    plan: GopFrame,
    qp: u16,
    wave: usize,
    read: Source,
    /// The search's frame decisions over the provisional references.
    prep: OnceLock<FramePrep<'a>>,
    /// The writer's frame decisions over the exact references.
    exact: OnceLock<(FramePrep<'a>, Waved)>,
    writer: Mutex<Option<(KeyFrameTileWriter, FrameDecisions)>>,
    intra: Mutex<Vec<(usize, usize, usize, usize)>>,
    rows: Vec<OnceLock<Arc<Snapshot>>>,
    coded: Mutex<Option<CodedFrame>>,
}

/// Codes `batch` in coding order with frame overlap, committing each frame
/// and show-existing step in turn.
pub(super) fn encode_batch(
    coder: &mut IntraFrameCoder,
    batch: &[Vec<Vec<BatchStep<'_>>>],
) -> Result<Vec<BatchOutput>> {
    if !eligible(coder) {
        return encode_in_turn(coder, batch);
    }
    let placeholder = EncoderReferences::new(&coder.sequence).map_err(coding("references"))?;
    let state = State {
        references: core::mem::replace(&mut coder.references, placeholder),
        slot_intra: core::mem::take(&mut coder.slot_intra),
        warp_gate: core::mem::take(&mut coder.warp_gate),
        grain_active: coder.grain_active,
        shown_existing: coder.shown_existing,
    };
    let trail = coder.trail.take();
    let result = Pipeline::run(coder, batch, state, trail);
    let (state, trail, outputs) = result?;
    coder.references = state.references;
    coder.slot_intra = state.slot_intra;
    coder.warp_gate = state.warp_gate;
    coder.grain_active = state.grain_active;
    coder.shown_existing = state.shown_existing;
    coder.trail = trail;
    Ok(outputs)
}

/// Codes `batch` one wave at a time, as without the overlap.
fn encode_in_turn(
    coder: &mut IntraFrameCoder,
    batch: &[Vec<Vec<BatchStep<'_>>>],
) -> Result<Vec<BatchOutput>> {
    let mut outputs = Vec::new();
    for wave in batch.iter().flatten() {
        let jobs: Vec<_> = wave
            .iter()
            .filter_map(|step| match *step {
                BatchStep::Code(source, plan, qp) => Some((source, plan, qp)),
                BatchStep::ShowExisting(_) => None,
            })
            .collect();
        let mut coded = crate::encoder::FrameCoder::encode_wave(coder, &jobs)?.into_iter();
        for step in wave {
            outputs.push(match *step {
                BatchStep::Code(..) => {
                    let mut frame = coded.next().ok_or(missing("a wave frame"))?;
                    crate::encoder::FrameCoder::commit(coder, &mut frame)?;
                    BatchOutput::Coded(Box::new(frame))
                }
                BatchStep::ShowExisting(slot) => BatchOutput::Unit(
                    crate::encoder::FrameCoder::show_existing_frame(coder, slot)?,
                ),
            });
        }
    }
    Ok(outputs)
}

struct Pipeline<'c, 'a> {
    coder: &'c IntraFrameCoder,
    jobs: Vec<Job<'a>>,
    /// Per wave: its steps as job indices or show-existing slots.
    waves: Vec<Vec<core::result::Result<usize, u32>>>,
    /// The read frame of each wave that has one.
    reads: Vec<OnceLock<Arc<Read>>>,
    waved: Vec<OnceLock<Arc<Waved>>>,
    start: Arc<Waved>,
    state: Mutex<(State, Vec<BatchOutput>)>,
    trail: Option<Trail>,
    next_trail: Mutex<Option<Trail>>,
    grid: (usize, usize),
    sb: usize,
}

impl<'c, 'a> Pipeline<'c, 'a> {
    fn run(
        coder: &'c IntraFrameCoder,
        batch: &[Vec<Vec<BatchStep<'a>>>],
        state: State,
        trail: Option<Trail>,
    ) -> Result<(State, Option<Trail>, Vec<BatchOutput>)> {
        let start = Arc::new(Waved {
            refs: state.references.fork().map_err(coding("references"))?,
            slot_intra: state.slot_intra.clone(),
        });
        let sb = match coder.view.frame_sb_size(false) {
            splot_core::headers::sequence::SuperblockSize::Block64x64 => 64,
            splot_core::headers::sequence::SuperblockSize::Block128x128 => 128,
            splot_core::headers::sequence::SuperblockSize::Block256x256 => 256,
        };
        let mi = |side: u32| (side as usize).div_ceil(8) * 2;
        let grid = (
            mi(coder.params.height).div_ceil(sb / 4),
            mi(coder.params.width).div_ceil(sb / 4),
        );
        let mut jobs = Vec::new();
        let mut waves = Vec::new();
        let mut last_refresh: Option<usize> = None;
        for wave in batch.iter().flatten() {
            let index = waves.len();
            let mut steps = Vec::new();
            let read = match last_refresh {
                Some(job) => Source::Batch(job),
                None if trail.is_some() => Source::Trail,
                None => Source::None,
            };
            let mut refreshed = None;
            for step in wave {
                steps.push(match *step {
                    BatchStep::Code(source, plan, qp) => {
                        if plan.refresh_frame_flags != 0 {
                            refreshed = Some(jobs.len());
                        }
                        jobs.push(Job {
                            source,
                            plan,
                            qp,
                            wave: index,
                            read: match read {
                                Source::Batch(job) => Source::Batch(job),
                                Source::Trail => Source::Trail,
                                Source::None => Source::None,
                            },
                            prep: OnceLock::new(),
                            exact: OnceLock::new(),
                            writer: Mutex::new(None),
                            intra: Mutex::new(Vec::new()),
                            rows: (0..grid.0).map(|_| OnceLock::new()).collect(),
                            coded: Mutex::new(None),
                        });
                        Ok(jobs.len() - 1)
                    }
                    BatchStep::ShowExisting(slot) => Err(slot),
                });
            }
            if refreshed.is_some() {
                last_refresh = refreshed;
            }
            waves.push(steps);
        }
        let pipeline = Pipeline {
            coder,
            reads: (0..waves.len()).map(|_| OnceLock::new()).collect(),
            waved: (0..waves.len()).map(|_| OnceLock::new()).collect(),
            jobs,
            waves,
            start,
            state: Mutex::new((state, Vec::new())),
            trail,
            next_trail: Mutex::new(None),
            grid,
            sb,
        };
        pipeline.code()?;
        let Pipeline {
            state,
            next_trail,
            trail,
            ..
        } = pipeline;
        let (state, outputs) = state.into_inner().map_err(|_| poisoned())?;
        let next = next_trail.into_inner().map_err(|_| poisoned())?.or(trail);
        Ok((state, next, outputs))
    }

    /// Builds and runs the graph of the batch.
    fn code(&self) -> Result<()> {
        let count = self.grid.0 * self.grid.1;
        let per_job = 2 * count + 3;
        let jobs = self.jobs.len();
        let reads_base = jobs * per_job;
        let commits_base = reads_base + self.waves.len();
        let prep = |job: usize| job * per_job;
        let search = |job: usize, index: usize| job * per_job + 1 + index;
        let writer_start = |job: usize| job * per_job + 1 + count;
        let write = |job: usize, index: usize| job * per_job + 2 + count + index;
        let finish = |job: usize| job * per_job + 2 + 2 * count;
        let mut preds = vec![Vec::new(); commits_base + self.waves.len()];
        let row_end = |job: usize, row: usize| write(job, (row + 1) * self.grid.1 - 1);
        for (j, job) in self.jobs.iter().enumerate() {
            let read_rows = |r: usize| (r + MARGIN).min(self.grid.0 - 1);
            match job.read {
                Source::Batch(p) => {
                    preds[prep(j)].push(reads_base + job.wave);
                    preds[prep(j)].push(row_end(p, read_rows(0)));
                }
                Source::Trail => {}
                Source::None => {
                    if let Some(w) = job.wave.checked_sub(1) {
                        preds[prep(j)].push(commits_base + w);
                    }
                }
            }
            for index in 0..count {
                let node = search(j, index);
                preds[node].push(prep(j));
                preds[node].extend(self.tile_order(index).into_iter().map(|i| search(j, i)));
                if let (Source::Batch(p), 0) = (&job.read, index % self.grid.1) {
                    preds[node].push(row_end(*p, read_rows(index / self.grid.1)));
                }
                let node = write(j, index);
                preds[node].push(search(j, index));
                preds[node].push(match index {
                    0 => writer_start(j),
                    _ => write(j, index - 1),
                });
            }
            preds[writer_start(j)].push(prep(j));
            if let Some(w) = job.wave.checked_sub(1) {
                preds[writer_start(j)].push(commits_base + w);
            }
            preds[finish(j)].push(write(j, count - 1));
        }
        for (w, steps) in self.waves.iter().enumerate() {
            if let Some(p) = self.read_job(w) {
                let p_wave = self.jobs[p].wave;
                preds[reads_base + w].push(writer_start(p));
                for &step in &self.waves[p_wave] {
                    if let Ok(j) = step
                        && j != p
                    {
                        preds[reads_base + w].push(prep(j));
                    }
                }
            }
            for &step in steps {
                if let Ok(j) = step {
                    preds[commits_base + w].push(finish(j));
                }
            }
            if let Some(before) = w.checked_sub(1) {
                preds[commits_base + w].push(commits_base + before);
            }
        }
        let tiles: Vec<Box<dyn SearchTile + '_>> = (0..jobs).map(|j| self.tile(j)).collect();
        run_graph(self.coder.pool.as_ref(), &preds, |node| {
            if node >= commits_base {
                return self.commit(node - commits_base);
            }
            if node >= reads_base {
                return self.read(node - reads_base);
            }
            let (j, local) = (node / per_job, node % per_job);
            match local {
                0 => self.prepare(j),
                l if l <= count => tiles[j].superblock(l - 1),
                l if l == count + 1 => self.writer_start(j),
                l if l < 2 * count + 2 => self.write(j, l - count - 2, &*tiles[j]),
                _ => self.finish(j),
            }
        })
    }

    /// The search predecessors of superblock `index` of a job, local: the
    /// left and top-right neighbours in a wavefront, else the previous one.
    fn tile_order(&self, index: usize) -> Vec<usize> {
        let cols = self.grid.1;
        let (r, c) = (index / cols, index % cols);
        let left = (c > 0).then(|| index - 1);
        let above = (r > 0).then(|| {
            if self.coder.preset.wavefront {
                (r - 1) * cols + (c + 1).min(cols - 1)
            } else {
                r * cols - 1
            }
        });
        left.into_iter().chain(above).collect()
    }

    /// The batch job of the read frame of wave `w`.
    fn read_job(&self, w: usize) -> Option<usize> {
        self.waves[w]
            .iter()
            .find_map(|step| step.ok())
            .and_then(|j| match self.jobs[j].read {
                Source::Batch(p) => Some(p),
                _ => None,
            })
    }

    /// The committed state after wave `w`, or before the batch.
    fn waved(&self, w: Option<usize>) -> Result<Arc<Waved>> {
        match w {
            None => Ok(Arc::clone(&self.start)),
            Some(w) => self.waved[w]
                .get()
                .cloned()
                .ok_or(missing("a committed wave")),
        }
    }

    /// The read frame of wave `w` from the batch.
    fn read(&self, w: usize) -> Result<()> {
        let Some(p) = self.read_job(w) else {
            return Ok(());
        };
        let read = self.read_frame(p, w)?;
        self.reads[w]
            .set(read)
            .map_err(|_| missing("a read frame set twice"))
    }

    /// Job `p` as the waves before wave `until` leave it for a reader: the
    /// references after every step before it, and the show-existing slots
    /// between it and `until`.
    fn read_frame(&self, p: usize, until: usize) -> Result<Arc<Read>> {
        let p_wave = self.jobs[p].wave;
        let base = self.waved(p_wave.checked_sub(1))?;
        let mut before = base.refs.fork().map_err(coding("references"))?;
        let mut after = Vec::new();
        let mut seen = false;
        for &step in &self.waves[p_wave] {
            match step {
                Ok(j) if j == p => seen = true,
                Ok(_) if seen => {}
                Ok(j) => {
                    let prep = self.jobs[j].prep.get().ok_or(missing("a prepared frame"))?;
                    before
                        .update_tip_output(&prep.core)
                        .map_err(coding("reference update"))?;
                }
                Err(slot) if seen => after.push(slot),
                Err(slot) => before
                    .show_existing(slot)
                    .map_err(coding("show-existing frame"))?,
            }
        }
        for steps in &self.waves[p_wave + 1..until] {
            after.extend(steps.iter().filter_map(|step| step.err()));
        }
        let (exact, _) = self.jobs[p]
            .exact
            .get()
            .ok_or(missing("a started writer"))?;
        Ok(Arc::new(Read {
            core: exact.core.clone(),
            initial: exact.initial.clone(),
            before,
            slot_intra: base.slot_intra.clone(),
            after,
        }))
    }

    /// The read frame of job `j` and its snapshot as of read row `row`.
    fn snapshot(&self, j: usize, row: usize) -> Result<Option<(Arc<Read>, Arc<Snapshot>)>> {
        let job = &self.jobs[j];
        let row = (row + MARGIN).min(self.grid.0 - 1);
        match job.read {
            Source::None => Ok(None),
            Source::Batch(p) => {
                let read = self.reads[job.wave]
                    .get()
                    .cloned()
                    .ok_or(missing("a read frame"))?;
                let snapshot = self.jobs[p].rows[row]
                    .get()
                    .cloned()
                    .ok_or(missing("a row"))?;
                Ok(Some((read, snapshot)))
            }
            Source::Trail => {
                let trail = self.trail.as_ref().ok_or(missing("a trail"))?;
                let rows = (row + 1) * self.sb;
                let frame = trail
                    .full
                    .frame
                    .rows(rows)
                    .map_err(coding("provisional frame"))?;
                let intra = trail
                    .full
                    .intra
                    .as_ref()
                    .map(|cells| Arc::new(cells.above(rows)));
                Ok(Some((
                    Arc::clone(&trail.read),
                    Arc::new(Snapshot { frame, intra }),
                )))
            }
        }
    }

    /// The references job `j` searches row `row` with, and the intra cells of
    /// its slots.
    fn search_refs(
        &self,
        j: usize,
        row: usize,
    ) -> Result<(EncoderReferences, [Option<Arc<IntraCells>>; 8])> {
        let Some((read, snapshot)) = self.snapshot(j, row)? else {
            let waved = self.waved(self.jobs[j].wave.checked_sub(1))?;
            return Ok((
                waved.refs.fork().map_err(coding("references"))?,
                waved.slot_intra.clone(),
            ));
        };
        let mut refs = read.before.fork().map_err(coding("references"))?;
        let frame = snapshot
            .frame
            .rows(usize::MAX)
            .map_err(coding("provisional frame"))?;
        refs.store_provisional(&read.core, frame, &read.initial)
            .map_err(coding("provisional reference"))?;
        for &slot in &read.after {
            refs.show_existing(slot)
                .map_err(coding("show-existing frame"))?;
        }
        let mut slot_intra = read.slot_intra.clone();
        let refreshed = read.core.refresh_frame_flags.unwrap_or(u32::MAX);
        for (slot, cells) in slot_intra.iter_mut().enumerate() {
            if refreshed >> slot & 1 == 1 {
                cells.clone_from(&snapshot.intra);
            }
        }
        Ok((refs, slot_intra))
    }

    fn prepare(&self, j: usize) -> Result<()> {
        let job = &self.jobs[j];
        let (refs, slot_intra) = self.search_refs(j, 0)?;
        let prep = self.coder.prepare(
            &refs,
            &slot_intra,
            (job.source, &job.plan, job.qp),
            self.coder.superblock_quant(&job.plan),
        )?;
        job.prep
            .set(prep)
            .map_err(|_| missing("a frame prepared twice"))
    }

    /// The row search of job `j`: each row facade searches over the
    /// references of [`Self::search_refs`] for its row.
    fn tile<'p>(&'p self, j: usize) -> Box<dyn SearchTile + 'p> {
        let row_writer = move |row: usize| {
            let prep = self.jobs[j].prep.get().ok_or(missing("a prepared frame"))?;
            let (refs, slot_intra) = self.search_refs(j, row)?;
            let start = refs
                .frame_start(&prep.core)
                .map_err(coding("initial CDFs"))?;
            let writer =
                KeyFrameTileWriter::with_start(&self.coder.sequence, &prep.core, &refs, &start, 0)
                    .map_err(coding("tile setup"))?;
            let motion = MotionSearch::new(
                writer.syntax(),
                &prep.padded,
                &self.coder.preset,
                prep.lambda,
                prep.open_loop.clone(),
            )
            .map_err(coding("motion search"))?;
            let reference_intra = self.coder.reference_intra(prep, &slot_intra);
            Ok((
                writer,
                self.coder.residual_coder(prep, None),
                (motion, reference_intra),
            ))
        };
        let search = move |writer: &mut KeyFrameTileWriter,
                           coder: &mut SourceResidualCoder<'p>,
                           (motion, reference_intra): &(
            Option<MotionSearch>,
            Option<Arc<IntraCells>>,
        )| {
            let prep = self.jobs[j].prep.get().ok_or(missing("a prepared frame"))?;
            self.coder.search_next(
                prep,
                (writer, coder),
                (motion.as_ref(), reference_intra.as_deref()),
                &self.jobs[j].intra,
            )
        };
        Box::new(TileSearch::new(
            self.grid,
            !self.coder.preset.wavefront,
            row_writer,
            search,
        ))
    }

    fn writer_start(&self, j: usize) -> Result<()> {
        let job = &self.jobs[j];
        let waved = self.waved(job.wave.checked_sub(1))?;
        let exact = self.coder.prepare(
            &waved.refs,
            &waved.slot_intra,
            (job.source, &job.plan, job.qp),
            self.coder.superblock_quant(&job.plan),
        )?;
        let prep = job.prep.get().ok_or(missing("a prepared frame"))?;
        if exact.core != prep.core {
            return Err(Error::Unsupported {
                what: "a provisional frame header that differs from the exact one",
            });
        }
        let writer = KeyFrameTileWriter::with_start(
            &self.coder.sequence,
            &exact.core,
            &waved.refs,
            &exact.initial,
            0,
        )
        .map_err(coding("tile setup"))?;
        *lock(&job.writer)? = Some((writer, FrameDecisions::default()));
        let waved = Waved {
            refs: waved.refs.fork().map_err(coding("references"))?,
            slot_intra: waved.slot_intra.clone(),
        };
        job.exact
            .set((exact, waved))
            .map_err(|_| missing("a writer started twice"))
    }

    fn write(&self, j: usize, index: usize, tile: &dyn SearchTile) -> Result<()> {
        let job = &self.jobs[j];
        let (prep, _) = job.exact.get().ok_or(missing("a started writer"))?;
        let (decision, target) = tile
            .take(index)?
            .into_decision()
            .ok_or(missing("a decision"))?;
        let mut guard = lock(&job.writer)?;
        let (writer, kept) = guard.as_mut().ok_or(missing("a started writer"))?;
        let mut coder: SourceResidualCoder = self.coder.residual_coder(prep, None);
        let qindex = match (target, prep.sb_quant.get(index)) {
            (Some(target), Some(&(_, lambda_log2))) => {
                let (lambda, qindex) =
                    self.coder
                        .superblock_quantizer(prep, writer, target, lambda_log2)?;
                coder.set_quantizer(qindex, lambda);
                Some(qindex)
            }
            _ => None,
        };
        kept.write_superblock(writer, (decision, qindex), &mut coder)
            .map_err(coding("superblock"))?;
        let cols = self.grid.1;
        if index % cols == cols - 1 {
            let row = index / cols;
            let rows = (row + 1) * self.sb;
            let frame = writer
                .syntax()
                .provisional_frame(rows)
                .map_err(coding("provisional frame"))?;
            let intra = prep.publishes.then(|| {
                let leaves = lock(&job.intra)
                    .map(|leaves| leaves.clone())
                    .unwrap_or_default();
                let age = prep
                    .reference_intra
                    .as_ref()
                    .map_or(0, |cells| cells.age.saturating_add(1));
                Arc::new(IntraCells::new(prep.frame_size, leaves, age).above(rows))
            });
            let _ = job.rows[row].set(Arc::new(Snapshot { frame, intra }));
        }
        Ok(())
    }

    /// Keeps job `j`, the batch's last reference frame, as the trail the
    /// next batch reads.
    fn keep_trail(&self, j: usize) -> Result<()> {
        let full = self.jobs[j]
            .rows
            .last()
            .and_then(OnceLock::get)
            .ok_or(missing("a row"))?;
        let read = self.read_frame(j, self.waves.len())?;
        *lock(&self.next_trail)? = Some(Trail {
            read,
            full: Arc::clone(full),
        });
        Ok(())
    }

    fn finish(&self, j: usize) -> Result<()> {
        let job = &self.jobs[j];
        let (writer, kept) = lock(&job.writer)?
            .take()
            .ok_or(missing("a written frame"))?;
        let tile = CodedTile::finish(writer, kept).map_err(coding("tile finish"))?;
        let intra = core::mem::take(&mut *lock(&job.intra)?);
        let (prep, waved) = job.exact.get().ok_or(missing("a started writer"))?;
        let prep = self.coder.prepare(
            &waved.refs,
            &waved.slot_intra,
            (job.source, &job.plan, job.qp),
            prep.sb_quant.clone(),
        )?;
        let coded = self.coder.finish(&waved.refs, prep, vec![tile], intra)?;
        *lock(&job.coded)? = Some(coded);
        Ok(())
    }

    fn commit(&self, w: usize) -> Result<()> {
        let mut guard = lock(&self.state)?;
        let (state, outputs) = &mut *guard;
        for &step in &self.waves[w] {
            let mut committed = Committed {
                references: &mut state.references,
                slot_intra: &mut state.slot_intra,
                warp_gate: &mut state.warp_gate,
                grain_active: &mut state.grain_active,
                shown_existing: &mut state.shown_existing,
            };
            outputs.push(match step {
                Ok(j) => {
                    let mut coded = lock(&self.jobs[j].coded)?
                        .take()
                        .ok_or(missing("a finished frame"))?;
                    committed.commit(&mut coded)?;
                    BatchOutput::Coded(Box::new(coded))
                }
                Err(slot) => BatchOutput::Unit(committed.show_existing(&self.coder.view, slot)?),
            });
        }
        let waved = Arc::new(Waved {
            refs: state.references.fork().map_err(coding("references"))?,
            slot_intra: state.slot_intra.clone(),
        });
        drop(guard);
        let last = self
            .jobs
            .iter()
            .rposition(|job| job.plan.refresh_frame_flags != 0);
        if let Some(j) = last.filter(|&j| self.jobs[j].wave == w) {
            self.keep_trail(j)?;
        }
        self.waved[w]
            .set(waved)
            .map_err(|_| missing("a wave committed twice"))
    }
}

/// The superblock steps of one job's row search.
trait SearchTile: Sync {
    fn superblock(&self, index: usize) -> Result<()>;
    fn take(&self, index: usize) -> Result<FrameDecisions>;
}

impl<C, X, W, S> SearchTile for TileSearch<C, X, W, S>
where
    C: crate::tile_writer::ResidualCoder + Send,
    X: Send,
    W: Fn(usize) -> Result<(KeyFrameTileWriter, C, X)> + Sync,
    S: Fn(
            &mut KeyFrameTileWriter,
            &mut C,
            &X,
        ) -> Result<(crate::tile_writer::SuperblockDecision, Option<u32>)>
        + Sync,
{
    fn superblock(&self, index: usize) -> Result<()> {
        Self::superblock(self, index)
    }

    fn take(&self, index: usize) -> Result<FrameDecisions> {
        Self::take(self, index)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| poisoned())
}

const fn poisoned() -> Error {
    Error::Unsupported {
        what: "a frame pipeline task that panicked",
    }
}

const fn missing(what: &'static str) -> Error {
    Error::Unsupported { what }
}
