// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.18.2 CDEF direction search on the startup-selected backend.

use splot_core::simd::SimdBackend;

#[cfg(target_arch = "x86_64")]
use super::cdef_direction_avx2;
use super::{CDEF_PADDED_AREA, CDEF_PADDED_SIDE, cdef_direction_input, cdef_direction_rows};

/// Variant of [`super::cdef_direction_padded`] using the caller's
/// startup-selected SIMD backend `backend`.
pub fn cdef_direction_padded_with(
    backend: SimdBackend,
    pad: &[u16; CDEF_PADDED_AREA],
    coeff_shift: u32,
) -> (usize, i32) {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = backend.avx2_token() {
        let rows = cdef_direction_input::<_, CDEF_PADDED_SIDE, _>(avx2, pad, coeff_shift);
        return cdef_direction_avx2(avx2, rows);
    }
    splot_core::with_simd!(backend, simd => {
        cdef_direction_rows(
            simd,
            cdef_direction_input::<_, CDEF_PADDED_SIDE, _>(simd, pad, coeff_shift),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdef_filter::cdef_direction_padded;

    #[test]
    fn backend_cdef_direction_matches_baseline_on_random_and_extreme_inputs() {
        use splot_core::simd::{SimdPolicy, resolve_backend};

        let backend = resolve_backend(SimdPolicy::Auto)
            .expect("supported host baseline")
            .backend();
        let run = |pad: &[u16; CDEF_PADDED_AREA], coeff_shift: u32| -> (usize, i32) {
            cdef_direction_padded_with(backend, pad, coeff_shift)
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
