// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Run separation for the process-global decode phase counters.

use std::sync::atomic::Ordering;

use super::{PHASE_COUNTERS, Phase, phase_delta, phase_totals};

/// Adds one interval to a phase, as a traced worker would.
fn add(phase: Phase, nanos: u64) {
    let counter = &PHASE_COUNTERS[phase as usize];
    counter.nanos.fetch_add(nanos, Ordering::Relaxed);
    counter.hits.fetch_add(1, Ordering::Relaxed);
}

#[test]
fn two_overlapping_runs_each_report_their_whole_window() {
    let index = Phase::Row as usize;
    let outer = phase_totals();
    add(Phase::Row, 1_000);

    let inner = phase_totals();
    add(Phase::Row, 2_000);

    assert_eq!(
        phase_delta(index, &inner),
        (2_000, 1),
        "the run that finishes first reports only what it added"
    );
    assert_eq!(
        phase_delta(index, &outer),
        (3_000, 2),
        "and it consumes nothing, so the run still going keeps its own samples"
    );
}
