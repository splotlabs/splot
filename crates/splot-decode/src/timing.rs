// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Env-gated decode phase timing trace.
//!
//! When `SPLOT_DECODE_TIMING` is set, decode entry points emit compact
//! `splot.decode_timing <phase>_ms=<value>` lines on stderr. Disabled by
//! default; normal CLI output is unchanged.

use std::sync::OnceLock;
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
