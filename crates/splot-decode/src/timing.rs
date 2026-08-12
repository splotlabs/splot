// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Env-gated decode phase timing trace.
//!
//! When `SPLOT_DECODE_TIMING` is set, decode entry points emit compact
//! `splot.decode_timing <phase>_ms=<value>` lines on stderr, and parallel
//! decode stages append work-unit and worker-utilization attribution so the
//! thread-scaling behavior of each stage is visible. Phases that fire per
//! block, per prediction unit, or per filter stripe accumulate into atomic
//! totals and emit one summed line each at the end of the stream instead of
//! printing as they run. Those totals are process-global, so a run reports the
//! delta it added over the totals it started from rather than clearing them:
//! two decodes traced at once in one process then each report a whole window,
//! though the windows overlap. Disabled by default; normal CLI output is
//! unchanged.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SPLOT_DECODE_TIMING").is_some())
}

/// Emits one decode's dependency-admission counters.
pub(crate) fn report_admission(metrics: splot_parallel::AdmissionMetrics) {
    if !enabled() {
        return;
    }
    eprintln!(
        "splot.decode_timing admission total_jobs={} conditions_registered={} \
         conditionless_jobs={} immediately_satisfied_jobs={} scheduler_slots={} \
         peak_scheduler_slots={} ready_heap_pushes={} ready_heap_pops={} direct_jobs={} \
         parallel_batches={}",
        metrics.total_jobs,
        metrics.conditions_registered,
        metrics.conditionless_jobs,
        metrics.immediately_satisfied_jobs,
        metrics.scheduler_slots,
        metrics.peak_scheduler_slots,
        metrics.ready_heap_pushes,
        metrics.ready_heap_pops,
        metrics.direct_jobs,
        metrics.parallel_batches,
    );
}

pub(crate) fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

pub(crate) fn report(phase: &str, started: Option<Instant>) {
    if let Some(started) = started {
        eprintln!(
            "splot.decode_timing {phase}_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

pub(crate) fn report_detail(phase: &str, started: Option<Instant>, detail: &str) {
    if let Some(started) = started {
        eprintln!(
            "splot.decode_timing {phase}_ms={:.3} {detail}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Emits one decode's pool-assisted wait counters.
pub(crate) fn report_pool_wait(metrics: splot_parallel::PoolWaitMetrics) {
    if !enabled() {
        return;
    }
    let average_park_us = if metrics.idle_parks == 0 {
        0.0
    } else {
        metrics.park_nanos as f64 / metrics.idle_parks as f64 / 1_000.0
    };
    let average_wake_us = if metrics.progress_wakes == 0 {
        0.0
    } else {
        metrics.wake_to_progress_nanos as f64 / metrics.progress_wakes as f64 / 1_000.0
    };
    eprintln!(
        "splot.decode_timing pool_wait assist_calls={} assisted_jobs={} idle_parks={} \
         park_ms={:.3} park_avg_us={average_park_us:.3} \
         timeout_wakes={} notifications={} progress_wakes={} \
         wake_to_progress_avg_us={average_wake_us:.3}",
        metrics.assist_calls,
        metrics.assisted_jobs,
        metrics.idle_parks,
        metrics.park_nanos as f64 / 1.0e6,
        metrics.timeout_wakes,
        metrics.notifications,
        metrics.progress_wakes,
    );
}

/// Phases whose intervals are summed in memory instead of printed as they run.
///
/// A phase that fires per block, per prediction unit, or per filter stripe
/// cannot print: the stderr lock serializes the workers, and the print lands
/// inside the interval of every phase bracketing it, so the enclosing counters
/// read high in proportion to how many prints they contain. These sum across
/// workers and report once.
///
/// `Row` nests both `Block` (the parse pass) and `ResolveRow` (the § 7.12 pass
/// that follows it), so the pure symbol-decode share is `Block` less the phases
/// nested inside it, and the resolve pass's share is `ResolveRow`.
#[derive(Clone, Copy)]
pub(crate) enum Phase {
    /// One parser step: partition walk, leaf decode, recon-entry building.
    Row,
    /// One leaf's mode-info and residual parse.
    Block,
    /// One parse unit's AV2 § 7.12 resolution pass over its parsed leaves.
    ResolveRow,
    /// AV2 § 7.12 reference MV stack and warp sample construction.
    MvStack,
    /// Reference MV bank and warp parameter bank maintenance.
    MvBank,
    /// Neighbour mode-info context derivation for symbol decode.
    ModeCtx,
    /// Neighbour mode-info grid block records.
    ModeRecord,
    /// Inter residual coefficient parse.
    Coeff,
    /// Deblock, transform-skip, and reconstruction command records.
    Records,
    /// General-intra leaf parse, coefficients and records included.
    IntraLeaf,
    /// One unit's whole ordered commit, publication included.
    Commit,
    /// Publication of one unit's precomputed surface into the frame.
    CommitPublish,
    /// General-intra reconstruction on the ordered commit.
    CommitIntra,
    /// IntraBC reconstruction on the ordered commit.
    CommitIntrabc,
    /// Inter reconstruction the prepass left to the ordered commit.
    CommitInter,
    /// Motion-record replay and block-decoded maintenance on the commit.
    CommitReplay,
    /// AV2 § 7.13.2.15 chroma-from-luma AC sample gathering for one block.
    CflLumaAc,
    /// TIP prediction-unit planning for one block.
    TipUnits,
    /// TIP prediction for one block's units.
    TipPrediction,
    /// TIP sample publication and temporal records for one block.
    TipPublish,
    /// TIP batched optical-flow motion grid for one unit row.
    TipMotionGrid,
    /// TIP batched compound prediction for one unit row.
    TipBatchPredict,
    /// AV2 § 7.18 CDEF over one filter stripe.
    FilterCdefStripe,
    /// AV2 § 7.19 CCSO over one filter stripe.
    FilterCcsoStripe,
    /// CCSO offsets over one plane's units within a stripe.
    CcsoUnits,
    /// AV2 § 7.17 loop restoration over one filter stripe.
    FilterLrStripe,
    /// AV2 § 7.21 guided detail filter over one filter stripe.
    FilterGdfStripe,
    /// AV2 § 7.14 deblocking advanced far enough for one stripe's window.
    FilterDeblock,
    /// One deblock plane pass over its mode-info row range.
    DeblockPlanePass,
    /// One stripe's deblocked source window copied out of the workspace.
    FilterStripeWindow,
    /// One drain of the finished stripes into the frame being published.
    FilterStripePublish,
    /// The filtered frame's freeze, once the last stripe has landed.
    FilterFreeze,
    /// Wiener NS luma restoration of one block.
    WienerNsLuma,
    /// PC-Wiener classification of one restoration block's cells.
    PcWienerClassify,
    /// PC-Wiener filtering of one restoration block.
    PcWienerFilter,
}

const PHASE_NAMES: [&str; 35] = [
    "row",
    "block",
    "resolve_row",
    "mv_stack",
    "mv_bank",
    "mode_ctx",
    "mode_record",
    "coeff",
    "records",
    "intra_leaf",
    "commit",
    "commit_publish",
    "commit_intra",
    "commit_intrabc",
    "commit_inter",
    "commit_replay",
    "cfl_luma_ac",
    "inter_tip_units",
    "inter_tip_prediction",
    "inter_tip_publish",
    "inter_tip_motion_grid",
    "inter_tip_batch_predict",
    "filter_cdef_stripe",
    "filter_ccso_stripe",
    "ccso_units",
    "filter_lr_stripe",
    "filter_gdf_stripe",
    "filter_deblock",
    "deblock_plane_pass",
    "filter_stripe_window",
    "filter_stripe_publish",
    "filter_freeze",
    "wiener_ns_luma",
    "pc_wiener_classify",
    "pc_wiener_filter",
];

/// One phase's running total, aligned so that no two phases share a cache line.
///
/// Every worker adds into the same counter, so the line it lives on moves
/// between cores for the whole decode. Padding keeps that traffic to the one
/// phase that earned it instead of the eight that happened to sit beside it.
#[repr(align(128))]
struct PhaseCounter {
    nanos: AtomicU64,
    hits: AtomicU64,
}

impl PhaseCounter {
    const fn new() -> Self {
        Self {
            nanos: AtomicU64::new(0),
            hits: AtomicU64::new(0),
        }
    }
}

static PHASE_COUNTERS: [PhaseCounter; PHASE_NAMES.len()] =
    [const { PhaseCounter::new() }; PHASE_NAMES.len()];

/// Adds one interval to a [`Phase`] total, in place of printing it.
///
/// This is the drop-in replacement for [`report`] on a hot path: same call
/// shape, one relaxed add per counter instead of a locked write to stderr.
pub(crate) fn accumulate(phase: Phase, started: Option<Instant>) {
    if let Some(started) = started
        && let Some(counter) = PHASE_COUNTERS.get(phase as usize)
    {
        let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        counter.nanos.fetch_add(nanos, Ordering::Relaxed);
        counter.hits.fetch_add(1, Ordering::Relaxed);
    }
}

/// Accumulates one [`Phase`] interval for as long as the value is held.
pub(crate) struct PhaseScope {
    phase: Phase,
    started: Option<Instant>,
}

impl PhaseScope {
    pub(crate) fn new(phase: Phase) -> Self {
        Self {
            phase,
            started: start(),
        }
    }
}

impl Drop for PhaseScope {
    fn drop(&mut self) {
        accumulate(self.phase, self.started);
    }
}

/// One reading of every phase total, taken at the start of a decode run.
#[derive(Clone, Copy)]
pub(crate) struct PhaseTotals([(u64, u64); PHASE_NAMES.len()]);

/// Reads every phase total, so a run can later report only what it added.
///
/// The counters are process-global: a second decode running beside this one
/// adds into the same totals, and no plumbing separates the two without putting
/// a handle on every hot path. Taking a delta at least keeps the two runs
/// non-destructive — see [`report_phases`].
pub(crate) fn phase_totals() -> PhaseTotals {
    PhaseTotals(core::array::from_fn(|index| {
        PHASE_COUNTERS.get(index).map_or((0, 0), |counter| {
            (
                counter.nanos.load(Ordering::Relaxed),
                counter.hits.load(Ordering::Relaxed),
            )
        })
    }))
}

/// The nanoseconds and intervals one phase has added since `since`.
fn phase_delta(index: usize, since: &PhaseTotals) -> (u64, u64) {
    let (nanos, hits) = since.0.get(index).copied().unwrap_or_default();
    PHASE_COUNTERS.get(index).map_or((0, 0), |counter| {
        (
            counter.nanos.load(Ordering::Relaxed).saturating_sub(nanos),
            counter.hits.load(Ordering::Relaxed).saturating_sub(hits),
        )
    })
}

/// Emits the phase totals one decode run added over `since`.
///
/// Each total is the sum over every worker that ran the phase, so a phase
/// reads above the wall time it occupied whenever it ran in parallel; `n` is
/// how many intervals the total covers.
///
/// Reporting a delta rather than clearing the counters is what makes a second
/// concurrent decode's report whole: neither run consumes the other's samples.
/// A phase another traced decode ran inside this run's window is still counted
/// in both reports, so attribution across concurrent decodes in one process is
/// approximate — trace one decode at a time when it has to be exact.
pub(crate) fn report_phases(since: &PhaseTotals) {
    if !enabled() {
        return;
    }
    for (index, name) in PHASE_NAMES.iter().enumerate() {
        let (nanos, n) = phase_delta(index, since);
        let ms = nanos as f64 / 1.0e6;
        eprintln!("splot.decode_timing {name}_ms={ms:.3} n={n}");
    }
}

pub(crate) struct WorkerTally {
    mask: Option<AtomicU32>,
}

impl WorkerTally {
    pub(crate) fn new() -> Self {
        Self {
            mask: enabled().then(|| AtomicU32::new(0)),
        }
    }

    pub(crate) fn note_worker(&self) {
        if let Some(mask) = &self.mask {
            let index = splot_parallel::current_worker_index().unwrap_or(0);
            mask.fetch_or(1 << index.min(31), Ordering::Relaxed);
        }
    }

    pub(crate) fn workers_used(&self) -> u32 {
        self.mask
            .as_ref()
            .map_or(0, |mask| mask.load(Ordering::Relaxed).count_ones())
    }
}

#[cfg(test)]
#[path = "timing_tests.rs"]
mod tests;
