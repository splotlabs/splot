// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The choice between the `sdot` and the multiply-accumulate kernels.

#![allow(
    unsafe_code,
    reason = "enters the dot-product kernels on a CPU detected to have them"
)]

use super::Job;
use fearless_simd::Neon;

/// Runs `job` in the `sdot` kernels when the CPU has `FEAT_DotProd`, which
/// the standard library detects once and caches, else in the
/// multiply-accumulate kernels.
#[inline]
pub(super) fn run(t: Neon, job: Job<'_, '_, '_>) {
    if std::arch::is_aarch64_feature_detected!("dotprod") && !mla_only() {
        // SAFETY: NEON (the token `t`) and dotprod (detected above) are every feature of the callee.
        unsafe { super::sdot::entry(t, job) }
    } else {
        super::mla_entry(t, job);
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
