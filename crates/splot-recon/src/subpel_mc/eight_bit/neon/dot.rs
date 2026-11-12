// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The choice between the `usdot`, the `sdot` and the multiply-accumulate
//! kernels, and the `usdot` instruction, which has no intrinsic yet.

#![allow(
    unsafe_code,
    reason = "enters the dot-product kernels on a CPU detected to have them"
)]

use super::{ByteOut, Compound, Programs, Strip, Strips, Windows};
use core::arch::aarch64::{int8x16_t, int32x4_t, uint8x16_t};
use fearless_simd::Neon;

/// One call of the kernels, compiled with `usdot`, with `sdot` and with
/// multiply-accumulate horizontal sums.
pub(super) trait Job {
    /// What the call returns.
    type Output;

    /// Whether the `usdot` kernels take fewer instructions than the `sdot`
    /// ones for this call's kernel class, once their call is paid.
    fn usdot_wins(&self) -> bool;

    /// Runs the call in the multiply-accumulate kernels.
    fn mla(self, t: Neon) -> Self::Output;

    /// Runs the call in the `sdot` kernels.
    ///
    /// # Safety
    ///
    /// Safety: the CPU has `FEAT_DotProd`.
    unsafe fn sdot(self, t: Neon) -> Self::Output;

    /// Runs the call in the `usdot` kernels.
    ///
    /// # Safety
    ///
    /// Safety: the CPU has `FEAT_DotProd` and `FEAT_I8MM`.
    unsafe fn usdot(self, t: Neon) -> Self::Output;
}

/// Runs `job` in the `usdot` kernels when its kernel class wins with them
/// ([`Job::usdot_wins`]) and the CPU has `FEAT_DotProd` and `FEAT_I8MM`,
/// which the standard library detects once and caches, else in
/// the `sdot` kernels when it has `FEAT_DotProd`, else in the
/// multiply-accumulate kernels.
#[allow(clippy::inline_always, reason = "one dispatch per kernel call")]
#[inline(always)]
pub(super) fn run<J: Job>(t: Neon, job: J) -> J::Output {
    if !std::arch::is_aarch64_feature_detected!("dotprod") || families() == 0 {
        return job.mla(t);
    }
    let i8mm = job.usdot_wins() && std::arch::is_aarch64_feature_detected!("i8mm");
    // SAFETY: the CPU has `FEAT_DotProd`, and with `i8mm` `FEAT_I8MM`, detected above.
    unsafe {
        if i8mm && families() > 1 {
            job.usdot(t)
        } else {
            job.sdot(t)
        }
    }
}

/// `usdot` of the unsigned bytes `samples` by the signed bytes of word `L`
/// of `taps`, added to `acc`.
#[inline]
#[target_feature(enable = "neon,i8mm")]
pub(super) fn usdot_lane<const L: usize>(
    mut acc: int32x4_t,
    samples: uint8x16_t,
    taps: int8x16_t,
) -> int32x4_t {
    // SAFETY: one `usdot` on the named registers; `i8mm` is enabled here.
    unsafe {
        core::arch::asm!(
            "usdot {acc:v}.4s, {a:v}.16b, {b:v}.4b[{l}]",
            acc = inout(vreg) acc,
            a = in(vreg) samples,
            b = in(vreg) taps,
            l = const L,
            options(pure, nomem, nostack, preserves_flags),
        );
    }
    acc
}

impl<O: ByteOut> Job for Strip<'_, '_, O> {
    type Output = ();

    #[inline]
    fn usdot_wins(&self) -> bool {
        self.0.horizontal.is_some() && self.0.w >= 8
    }

    #[inline]
    fn mla(self, t: Neon) {
        super::mla_strips(t, self.0, self.1);
    }

    /// Safety: as [`Job::sdot`].
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn sdot(self, t: Neon) {
        super::sdot::strips(t, self.0, self.1);
    }

    /// Safety: as [`Job::usdot`].
    #[target_feature(enable = "neon,dotprod,i8mm")]
    unsafe fn usdot(self, t: Neon) {
        super::usdot::strips(t, self.0, self.1);
    }
}

impl<O: ByteOut> Job for Strips<'_, '_, O> {
    type Output = ();

    #[inline]
    fn usdot_wins(&self) -> bool {
        self.0[0].usdot_wins()
    }

    #[inline]
    fn mla(self, t: Neon) {
        for Strip(block, out) in self.0 {
            super::mla_strips(t, block, out);
        }
    }

    /// Safety: as [`Job::sdot`].
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn sdot(self, t: Neon) {
        for Strip(block, out) in self.0 {
            super::sdot::strips(t, block, out);
        }
    }

    /// Safety: as [`Job::usdot`].
    #[target_feature(enable = "neon,dotprod,i8mm")]
    unsafe fn usdot(self, t: Neon) {
        for Strip(block, out) in self.0 {
            super::usdot::strips(t, block, out);
        }
    }
}

impl<const CELL: bool, const P: usize> Job for Compound<'_, '_, CELL, P> {
    type Output = ();

    #[inline]
    fn usdot_wins(&self) -> bool {
        self.0[0][0].w >= 16 && self.0[0][0].horizontal.is_some()
    }

    #[inline]
    fn mla(self, t: Neon) {
        let Compound(pairs, forward, outputs) = self;
        for (blocks, (output, stride)) in pairs.into_iter().zip(outputs) {
            super::mla_compound(t, CELL, blocks, forward, output, stride);
        }
    }

    /// Safety: as [`Job::sdot`].
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn sdot(self, t: Neon) {
        let Compound(pairs, forward, outputs) = self;
        for (blocks, (output, stride)) in pairs.into_iter().zip(outputs) {
            if CELL {
                super::sdot::compound_cell(t, blocks, forward, output, stride);
            } else {
                super::sdot::compound_strips(t, blocks, forward, output, stride);
            }
        }
    }

    /// Safety: as [`Job::usdot`].
    #[target_feature(enable = "neon,dotprod,i8mm")]
    unsafe fn usdot(self, t: Neon) {
        let Compound(pairs, forward, outputs) = self;
        for (blocks, (output, stride)) in pairs.into_iter().zip(outputs) {
            if CELL {
                super::usdot::compound_cell(t, blocks, forward, output, stride);
            } else {
                super::usdot::compound_strips(t, blocks, forward, output, stride);
            }
        }
    }
}

impl Job for Windows<'_, '_> {
    type Output = bool;

    #[inline]
    fn usdot_wins(&self) -> bool {
        self.1
            .iter()
            .all(|window| window.w == 8 && window.horizontal.is_some() && window.vertical.is_some())
    }

    #[inline]
    fn mla(self, t: Neon) -> bool {
        super::mla_windows(t, self)
    }

    /// Safety: as [`Job::sdot`].
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn sdot(self, t: Neon) -> bool {
        if self.1[0].w == 4 {
            super::sdot::windows_cell::<4>(t, self)
        } else {
            super::sdot::windows_cell::<8>(t, self)
        }
    }

    /// Safety: as [`Job::usdot`].
    #[target_feature(enable = "neon,dotprod,i8mm")]
    unsafe fn usdot(self, t: Neon) -> bool {
        if self.1[0].w == 4 {
            super::usdot::windows_cell::<4>(t, self)
        } else {
            super::usdot::windows_cell::<8>(t, self)
        }
    }
}

impl Job for Programs<'_, '_> {
    type Output = ();

    #[inline]
    fn usdot_wins(&self) -> bool {
        self.1 == 8
    }

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

    /// Safety: as [`Job::usdot`].
    #[target_feature(enable = "neon,dotprod,i8mm")]
    unsafe fn usdot(self, t: Neon) {
        *self.2 = if self.1 == 4 {
            super::usdot::run_program::<4>(t, self.0)
        } else {
            super::usdot::run_program::<8>(t, self.0)
        };
    }
}

#[cfg(test)]
std::thread_local! {
    /// The kernel families this test thread may run past multiply-accumulates:
    /// 0, 1 (`sdot`) or 2 (`sdot` and `usdot`).
    pub(in crate::subpel_mc) static FAMILIES: core::cell::Cell<u8> =
        const { core::cell::Cell::new(2) };
}

#[cfg(test)]
fn families() -> u8 {
    FAMILIES.with(core::cell::Cell::get)
}

#[cfg(not(test))]
const fn families() -> u8 {
    2
}
