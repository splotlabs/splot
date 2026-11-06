// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! SIMD token binding shared by the § 7.20 and § 7.21.7 kernels.

/// Runs `$body` with the baseline SIMD token bound to `$simd`; on x86 the
/// AVX2 token binds directly, since a `dispatch!` closure does not inline
/// into per-row callers.
macro_rules! lr_simd {
    ($simd:ident => $body:expr) => {{
        #[cfg(target_arch = "x86_64")]
        let value = match fearless_simd::Level::baseline().as_avx2() {
            Some($simd) => $body,
            None => fearless_simd::dispatch!(fearless_simd::Level::baseline(), $simd => $body),
        };
        #[cfg(not(target_arch = "x86_64"))]
        let value = fearless_simd::dispatch!(fearless_simd::Level::baseline(), $simd => $body);
        value
    }};
}
pub(crate) use lr_simd;
