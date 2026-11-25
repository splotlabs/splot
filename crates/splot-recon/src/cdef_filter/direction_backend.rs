// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Sealed backend specialization for AV2 § 7.18.2 CDEF direction search.

use splot_core::simd::SimdBackend;

#[cfg(target_arch = "x86_64")]
use super::cdef_direction_avx2;
use super::{CDEF_PADDED_AREA, CDEF_PADDED_SIDE, cdef_direction_input, cdef_direction_rows};

/// Variant of [`super::cdef_direction_padded`] using the caller's
/// startup-selected static SIMD backend `backend`.
pub fn cdef_direction_padded_with<B: SimdBackend>(
    backend: B,
    pad: &[u16; CDEF_PADDED_AREA],
    coeff_shift: u32,
) -> (usize, i32) {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = backend.avx2_token() {
        let rows = cdef_direction_input::<_, CDEF_PADDED_SIDE, _>(avx2, pad, coeff_shift);
        return cdef_direction_avx2(avx2, rows);
    }
    let narrow = backend.narrow();
    splot_core::with_simd!(narrow, simd => {
        cdef_direction_rows(
            simd,
            cdef_direction_input::<_, CDEF_PADDED_SIDE, _>(simd, pad, coeff_shift),
        )
    })
}

/// [`super::cdef_direction_padded`] using an explicit startup-selected
/// [`SimdBackend`] without consulting global baseline detection in the hot path.
pub fn cdef_direction_padded_with_backend<B: CdefDirectionBackend>(
    backend: B,
    pad: &[u16; CDEF_PADDED_AREA],
    coeff_shift: u32,
) -> (usize, i32) {
    backend.cdef_direction_padded_impl(pad, coeff_shift)
}

/// Sealed backend dispatch hook for padded AV2 § 7.18.2 CDEF direction search.
pub trait CdefDirectionBackend: SimdBackend {
    /// Computes `(yDir, var)` on `pad` with pre-shift `coeff_shift`.
    #[inline]
    fn cdef_direction_padded_impl(
        self,
        pad: &[u16; CDEF_PADDED_AREA],
        coeff_shift: u32,
    ) -> (usize, i32) {
        self.vectorize(|| {
            let simd = self.token();
            cdef_direction_rows(
                simd,
                cdef_direction_input::<_, CDEF_PADDED_SIDE, _>(simd, pad, coeff_shift),
            )
        })
    }
}

#[cfg(target_arch = "x86_64")]
impl CdefDirectionBackend for splot_core::simd::Sse2Backend {
    fn cdef_direction_padded_impl(
        self,
        pad: &[u16; CDEF_PADDED_AREA],
        coeff_shift: u32,
    ) -> (usize, i32) {
        cdef_direction_padded_with(self, pad, coeff_shift)
    }
}

#[cfg(target_arch = "x86_64")]
impl CdefDirectionBackend for splot_core::simd::Avx2Backend {
    #[inline]
    fn cdef_direction_padded_impl(
        self,
        pad: &[u16; CDEF_PADDED_AREA],
        coeff_shift: u32,
    ) -> (usize, i32) {
        self.vectorize(|| {
            let avx2 = self.token();
            let rows = cdef_direction_input::<_, CDEF_PADDED_SIDE, _>(avx2, pad, coeff_shift);
            cdef_direction_avx2(avx2, rows)
        })
    }
}

#[cfg(target_arch = "x86_64")]
impl CdefDirectionBackend for splot_core::simd::Avx512Backend {}

#[cfg(target_arch = "aarch64")]
impl CdefDirectionBackend for splot_core::simd::NeonBackend {}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
impl CdefDirectionBackend for splot_core::simd::FallbackBackend {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdef_filter::cdef_direction_padded;

    #[inline(never)]
    fn cdef_direction_backend_audit_entry(
        backend: impl CdefDirectionBackend,
        pad: &[u16; CDEF_PADDED_AREA],
        coeff_shift: u32,
    ) -> (usize, i32) {
        cdef_direction_padded_with_backend(backend, pad, coeff_shift)
    }

    #[test]
    fn backend_cdef_direction_matches_baseline_on_random_and_extreme_inputs() {
        use splot_core::simd::{ResolvedBackend, SimdPolicy, resolve_backend};

        let resolved = resolve_backend(SimdPolicy::Auto).expect("supported host baseline");
        let run = |pad: &[u16; CDEF_PADDED_AREA], coeff_shift: u32| -> (usize, i32) {
            match resolved {
                #[cfg(target_arch = "x86_64")]
                ResolvedBackend::Sse2(b) => cdef_direction_backend_audit_entry(b, pad, coeff_shift),
                #[cfg(target_arch = "x86_64")]
                ResolvedBackend::Avx2(b) => cdef_direction_backend_audit_entry(b, pad, coeff_shift),
                #[cfg(target_arch = "x86_64")]
                ResolvedBackend::Avx512(b) => {
                    cdef_direction_backend_audit_entry(b, pad, coeff_shift)
                }
                #[cfg(target_arch = "aarch64")]
                ResolvedBackend::Neon(b) => cdef_direction_backend_audit_entry(b, pad, coeff_shift),
                #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
                ResolvedBackend::Fallback(b) => {
                    cdef_direction_backend_audit_entry(b, pad, coeff_shift)
                }
                other => panic!("unexpected backend variant: {other:?}"),
            }
        };

        for (coeff_shift, max_sample) in [(0u32, 255u16), (2u32, 1023u16)] {
            for fill in [0u16, max_sample] {
                let pad = [fill; CDEF_PADDED_AREA];
                assert_eq!(
                    run(&pad, coeff_shift),
                    cdef_direction_padded(&pad, coeff_shift)
                );
            }

            let mut checker = [0u16; CDEF_PADDED_AREA];
            for (idx, sample) in checker.iter_mut().enumerate() {
                let (r, c) = (idx / CDEF_PADDED_SIDE, idx % CDEF_PADDED_SIDE);
                *sample = if (r + c) % 2 == 0 { 0 } else { max_sample };
            }
            assert_eq!(
                run(&checker, coeff_shift),
                cdef_direction_padded(&checker, coeff_shift)
            );

            let mut state = 0x9e37_79b9u32;
            for _ in 0..32 {
                let mut pad = [0u16; CDEF_PADDED_AREA];
                for sample in &mut pad {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    *sample = ((state >> 16) as u16) % (max_sample + 1);
                }
                assert_eq!(
                    run(&pad, coeff_shift),
                    cdef_direction_padded(&pad, coeff_shift)
                );
            }
        }
    }
}
