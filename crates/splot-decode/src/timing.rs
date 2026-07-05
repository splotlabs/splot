// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Env-gated decode phase timing trace.
//!
//! When `SPLOT_DECODE_TIMING` is set, decode entry points emit compact
//! `splot.decode_timing <phase>_ms=<value>` lines on stderr, and parallel
//! decode stages append work-unit and worker-utilization attribution so the
//! thread-scaling behavior of each stage is visible. Disabled by default;
//! normal CLI output is unchanged.

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SPLOT_DECODE_TIMING").is_some())
}

/// Starts a phase timer, or returns `None` when the trace is disabled.
pub(crate) fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

/// Emits one `splot.decode_timing` stderr line for a phase started via [`start`].
pub(crate) fn report(phase: &str, started: Option<Instant>) {
    if let Some(started) = started {
        eprintln!(
            "splot.decode_timing {phase}_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Emits one `splot.decode_timing` line with extra `key=value` attribution
/// (work-unit counts, chosen grain, serial-fallback reasons).
pub(crate) fn report_detail(phase: &str, started: Option<Instant>, detail: &str) {
    if let Some(started) = started {
        eprintln!(
            "splot.decode_timing {phase}_ms={:.3} {detail}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

thread_local! {
    /// Nanoseconds spent in the reconstruction sink (dequant, inverse
    /// transform, intra prediction, residual add, workspace write) since the
    /// last reset, split `[luma, chroma]`. Only maintained when the trace is
    /// enabled; the intra tile decodes on one thread, so a thread-local read
    /// at the tile boundary sees the whole tile.
    static SINK_NANOS: Cell<[u128; 2]> = const { Cell::new([0, 0]) };
}

/// Tracks which pool workers actually executed work inside one parallel stage.
///
/// The tally is a bitmask over worker indexes (workers past 63 saturate into
/// the last bit, far beyond the supported pool widths). All operations are
/// no-ops when the timing trace is disabled, so hot loops only pay an
/// already-branch-predicted `None` check.
pub(crate) struct WorkerTally {
    mask: Option<AtomicU64>,
}

impl WorkerTally {
    pub(crate) fn new() -> Self {
        Self {
            mask: enabled().then(|| AtomicU64::new(0)),
        }
    }

    /// Records the calling pool worker as having executed stage work.
    pub(crate) fn note_worker(&self) {
        if let Some(mask) = &self.mask {
            let index = splot_parallel::current_worker_index().unwrap_or(0);
            mask.fetch_or(1 << index.min(63), Ordering::Relaxed);
        }
    }

    /// The number of distinct workers that executed stage work.
    pub(crate) fn workers_used(&self) -> u32 {
        self.mask
            .as_ref()
            .map_or(0, |mask| mask.load(Ordering::Relaxed).count_ones())
    }
}
