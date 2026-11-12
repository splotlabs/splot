// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The choice between the `sdot` and the multiply-accumulate kernels.

#![allow(
    unsafe_code,
    reason = "enters the dot-product kernels on a CPU detected to have them"
)]

use super::{Compound, Out, Programs, Strips};
use fearless_simd::Neon;

/// One call of the kernels, compiled once with `sdot` horizontal sums and
/// once with multiply-accumulates.
pub(super) trait Job {
    /// Runs the call in the multiply-accumulate kernels.
    fn mla(self, t: Neon);

    /// Runs the call in the `sdot` kernels.
    ///
    /// # Safety
    ///
    /// Safety: the CPU has `FEAT_DotProd`.
    unsafe fn sdot(self, t: Neon);
}

/// Runs `job` in the `sdot` kernels when the CPU has `FEAT_DotProd`, which
/// the standard library detects once and caches, else in the
/// multiply-accumulate kernels.
#[allow(clippy::inline_always, reason = "one dispatch per kernel call")]
#[inline(always)]
pub(super) fn run(t: Neon, job: impl Job) {
    if std::arch::is_aarch64_feature_detected!("dotprod") && !mla_only() {
        // SAFETY: the CPU has `FEAT_DotProd`, detected above.
        unsafe { job.sdot(t) }
    } else {
        job.mla(t);
    }
}

impl<O: Out> Job for Strips<'_, '_, O> {
    #[inline]
    fn mla(self, t: Neon) {
        super::mla_strips(t, self.0, self.1);
    }

    /// Safety: as [`Job::sdot`].
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn sdot(self, t: Neon) {
        super::sdot::strips(t, self.0, self.1);
    }
}

impl Job for Compound<'_, '_> {
    #[inline]
    fn mla(self, t: Neon) {
        super::mla_compound(t, self);
    }

    /// Safety: as [`Job::sdot`].
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn sdot(self, t: Neon) {
        super::sdot::compound(t, self.0, self.1, self.2, self.3);
    }
}

impl Job for Programs<'_, '_> {
    #[inline]
    fn mla(self, t: Neon) {
        super::mla_program(t, self);
    }

    /// Safety: as [`Job::sdot`].
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn sdot(self, t: Neon) {
        *self.2 = if self.1 == 4 {
            super::sdot::run_program::<4>(t, self.0)
        } else {
            super::sdot::run_program::<8>(t, self.0)
        };
    }
}

#[cfg(test)]
std::thread_local! {
    /// Whether this test thread runs the multiply-accumulate kernels.
    pub(in crate::subpel_mc) static MLA_ONLY: core::cell::Cell<bool> =
        const { core::cell::Cell::new(false) };
}

#[cfg(test)]
fn mla_only() -> bool {
    MLA_ONLY.with(core::cell::Cell::get)
}

#[cfg(not(test))]
const fn mla_only() -> bool {
    false
}
