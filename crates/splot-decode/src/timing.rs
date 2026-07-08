// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Env-gated decode phase timing trace.
//!
//! When `SPLOT_DECODE_TIMING` is set, decode entry points emit compact
//! `splot.decode_timing <phase>_ms=<value>` lines on stderr, and parallel
//! decode stages append work-unit and worker-utilization attribution so the
//! thread-scaling behavior of each stage is visible. Disabled by default;
//! normal CLI output is unchanged.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
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

pub(crate) struct WorkerTally {
    mask: Option<AtomicU64>,
}

impl WorkerTally {
    pub(crate) fn new() -> Self {
        Self {
            mask: enabled().then(|| AtomicU64::new(0)),
        }
    }

    pub(crate) fn note_worker(&self) {
        if let Some(mask) = &self.mask {
            let index = splot_parallel::current_worker_index().unwrap_or(0);
            mask.fetch_or(1 << index.min(63), Ordering::Relaxed);
        }
    }

    pub(crate) fn workers_used(&self) -> u32 {
        self.mask
            .as_ref()
            .map_or(0, |mask| mask.load(Ordering::Relaxed).count_ones())
    }
}
