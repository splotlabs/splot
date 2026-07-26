// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Env-gated decode phase timing trace.
//!
//! When `SPLOT_DECODE_TIMING` is set, decode entry points emit compact
//! `splot.decode_timing <phase>_ms=<value>` lines on stderr, and parallel
//! decode stages append work-unit and worker-utilization attribution so the
//! thread-scaling behavior of each stage is visible. Disabled by default;
//! normal CLI output is unchanged.

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SPLOT_DECODE_TIMING").is_some())
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

/// Parse phases of the fused block walk.
///
/// `Row` nests `Block`, and `Block` nests every remaining phase, so the pure
/// symbol-decode share is `Block` less the phases nested inside it.
#[derive(Clone, Copy)]
pub(crate) enum WalkPhase {
    /// One parser step: partition walk, leaf decode, recon-entry building.
    Row,
    /// One leaf's mode-info and residual parse.
    Block,
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
}

const WALK_PHASE_NAMES: [&str; 9] = [
    "row",
    "block",
    "mv_stack",
    "mv_bank",
    "mode_ctx",
    "mode_record",
    "coeff",
    "records",
    "intra_leaf",
];

static WALK_PHASE_NS: [AtomicU64; WALK_PHASE_NAMES.len()] =
    [const { AtomicU64::new(0) }; WALK_PHASE_NAMES.len()];
static WALK_BLOCKS: AtomicU64 = AtomicU64::new(0);

/// Accumulates one [`WalkPhase`] interval for as long as the value is held.
pub(crate) struct WalkPhaseScope {
    index: usize,
    started: Option<Instant>,
}

impl WalkPhaseScope {
    pub(crate) fn new(phase: WalkPhase) -> Self {
        Self {
            index: phase as usize,
            started: start(),
        }
    }
}

impl Drop for WalkPhaseScope {
    fn drop(&mut self) {
        if let Some(started) = self.started
            && let Some(slot) = WALK_PHASE_NS.get(self.index)
        {
            let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            slot.fetch_add(nanos, Ordering::Relaxed);
        }
    }
}

pub(crate) fn note_walk_block() {
    if enabled() {
        WALK_BLOCKS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Emits and clears the accumulated walk phase totals for the whole stream.
pub(crate) fn report_walk_phases() {
    if !enabled() {
        return;
    }
    let mut detail = format!("blocks={}", WALK_BLOCKS.swap(0, Ordering::Relaxed));
    for (name, slot) in WALK_PHASE_NAMES.iter().zip(&WALK_PHASE_NS) {
        let ms = slot.swap(0, Ordering::Relaxed) as f64 / 1.0e6;
        let _ = write!(detail, " {name}_ms={ms:.3}");
    }
    eprintln!("splot.decode_timing walk_phases {detail}");
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
