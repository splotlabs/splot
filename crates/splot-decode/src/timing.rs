// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Env-gated decode phase timing trace.
//!
//! When `SPLOT_DECODE_TIMING` is set, decode entry points emit compact
//! `splot.decode_timing <phase>_ms=<value>` lines on stderr. Disabled by
//! default; normal CLI output is unchanged.

use core::num::NonZeroUsize;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

pub(crate) fn enabled() -> bool {
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

/// Tracks which worker threads execute an env-gated parallel stage.
pub(crate) struct WorkerSet {
    bits: AtomicUsize,
    overflow: AtomicUsize,
}

impl WorkerSet {
    /// Creates a worker set when decode timing is enabled.
    #[must_use]
    pub(crate) fn maybe_new() -> Option<Self> {
        enabled().then(|| Self {
            bits: AtomicUsize::new(0),
            overflow: AtomicUsize::new(0),
        })
    }

    fn mark_current(&self) {
        let Some(index) = splot_parallel::current_worker_index() else {
            return;
        };
        if index < usize::BITS as usize {
            self.bits.fetch_or(1usize << index, Ordering::Relaxed);
        } else {
            self.overflow.fetch_or(1, Ordering::Relaxed);
        }
    }

    fn active_workers(&self) -> usize {
        self.bits.load(Ordering::Relaxed).count_ones() as usize
            + usize::from(self.overflow.load(Ordering::Relaxed) != 0)
    }
}

/// Marks the current worker in a timing-only [`WorkerSet`], if one is active.
pub(crate) fn mark_worker(worker_set: Option<&WorkerSet>) {
    if let Some(worker_set) = worker_set {
        worker_set.mark_current();
    }
}

/// Emits one `splot.decode_timing` line with parallel-stage attribution.
pub(crate) fn report_parallel(
    phase: &str,
    started: Option<Instant>,
    resolved_threads: Option<NonZeroUsize>,
    work_units: usize,
    grain_size: usize,
    worker_set: Option<&WorkerSet>,
    fallback_reason: Option<&str>,
) {
    if let Some(started) = started {
        eprintln!(
            "splot.decode_timing {phase}_ms={:.3} threads={} work_units={} grain_size={} active_workers={} fallback_reason={}",
            started.elapsed().as_secs_f64() * 1000.0,
            resolved_threads.map_or(0, NonZeroUsize::get),
            work_units,
            grain_size,
            worker_set.map_or(0, WorkerSet::active_workers),
            fallback_reason.unwrap_or("none")
        );
    }
}
