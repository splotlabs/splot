// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-filter and restoration orchestration.

/// Runs `$body` with `$simd` bound to the baseline SIMD token: the AVX2
/// token directly where the baseline has it, since the closure `dispatch!`
/// wraps its body in is not inlined, else through `dispatch!`.
#[cfg(target_arch = "x86_64")]
macro_rules! baseline_simd {
    ($simd:ident => $body:expr) => {
        match fearless_simd::Level::baseline().as_avx2() {
            Some($simd) => $body,
            None => fearless_simd::dispatch!(fearless_simd::Level::baseline(), $simd => $body),
        }
    };
}

/// [`baseline_simd`] for targets without an AVX2 baseline.
#[cfg(not(target_arch = "x86_64"))]
macro_rules! baseline_simd {
    ($simd:ident => $body:expr) => {
        fearless_simd::dispatch!(fearless_simd::Level::baseline(), $simd => $body)
    };
}

pub(crate) mod ccso;
pub(crate) mod cdef;
pub(crate) mod deblock;
pub(crate) mod gdf;
pub(crate) mod lossless;
pub(crate) mod source;
pub(crate) mod wienerns_lr;
