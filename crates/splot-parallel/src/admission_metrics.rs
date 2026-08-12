// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Optional counters for dependency-admission diagnostics.

use std::sync::atomic::{AtomicUsize, Ordering};

/// One dependency-admission scheduler's diagnostic counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdmissionMetrics {
    /// Jobs submitted with zero or more dynamic conditions.
    pub total_jobs: usize,
    /// Conditions registered across all submitted jobs.
    pub conditions_registered: usize,
    /// Submitted jobs whose condition list was empty.
    pub conditionless_jobs: usize,
    /// Jobs with conditions that all held before submission finished.
    pub immediately_satisfied_jobs: usize,
    /// Scheduler-slot occupancies across the run.
    pub scheduler_slots: usize,
    /// Peak simultaneously occupied scheduler slots.
    pub peak_scheduler_slots: usize,
    /// Ready-heap pushes.
    pub ready_heap_pushes: usize,
    /// Ready-heap pops.
    pub ready_heap_pops: usize,
    /// Proven-ready jobs spawned without a scheduler slot.
    pub direct_jobs: usize,
    /// Proven-ready job batches submitted as one ordered entry.
    pub parallel_batches: usize,
}

#[derive(Debug, Default)]
pub(crate) struct AdmissionDiagnosticCounters {
    values: [AtomicUsize; 10],
}

impl AdmissionDiagnosticCounters {
    pub(crate) fn note_submission(&self, conditions: usize) {
        self.values[0].fetch_add(1, Ordering::Relaxed);
        self.values[1].fetch_add(conditions, Ordering::Relaxed);
        if conditions == 0 {
            self.values[2].fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn note_immediately_satisfied(&self) {
        self.values[3].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_slot(&self, live: usize) {
        self.values[4].fetch_add(1, Ordering::Relaxed);
        self.values[5].fetch_max(live, Ordering::Relaxed);
    }

    pub(crate) fn note_heap_push(&self) {
        self.values[6].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_heap_pop(&self) {
        self.values[7].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_direct(&self) {
        self.values[8].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> AdmissionMetrics {
        let [
            total_jobs,
            conditions_registered,
            conditionless_jobs,
            immediately_satisfied_jobs,
            scheduler_slots,
            peak_scheduler_slots,
            ready_heap_pushes,
            ready_heap_pops,
            direct_jobs,
            parallel_batches,
        ] = core::array::from_fn(|index| self.values[index].load(Ordering::Relaxed));
        AdmissionMetrics {
            total_jobs,
            conditions_registered,
            conditionless_jobs,
            immediately_satisfied_jobs,
            scheduler_slots,
            peak_scheduler_slots,
            ready_heap_pushes,
            ready_heap_pops,
            direct_jobs,
            parallel_batches,
        }
    }
}
