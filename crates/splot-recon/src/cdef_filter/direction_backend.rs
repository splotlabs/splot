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

mod sealed_backend {
    pub trait Sealed {}

    #[cfg(target_arch = "x86_64")]
    impl Sealed for splot_core::simd::Sse2Backend {}

    #[cfg(target_arch = "x86_64")]
    impl Sealed for splot_core::simd::Avx2Backend {}

    #[cfg(target_arch = "x86_64")]
    impl Sealed for splot_core::simd::Avx512Backend {}

    #[cfg(target_arch = "aarch64")]
    impl Sealed for splot_core::simd::NeonBackend {}

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    impl Sealed for splot_core::simd::FallbackBackend {}
}

/// Sealed backend dispatch hook for padded AV2 § 7.18.2 CDEF direction search.
pub trait CdefDirectionBackend: SimdBackend + sealed_backend::Sealed {
    /// Computes `(yDir, var)` on `pad` with pre-shift `coeff_shift`.
    fn cdef_direction_padded_impl(
        self,
        pad: &[u16; CDEF_PADDED_AREA],
        coeff_shift: u32,
    ) -> (usize, i32);
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
impl CdefDirectionBackend for splot_core::simd::Avx512Backend {
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

#[cfg(target_arch = "aarch64")]
impl CdefDirectionBackend for splot_core::simd::NeonBackend {
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

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
impl CdefDirectionBackend for splot_core::simd::FallbackBackend {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdef_filter::{DIV_TABLE, cdef_direction, cdef_direction_padded};

    #[test]
    fn flat_block_has_zero_variance() {
        let block = [[0i32; 8]; 8];
        let (y_dir, var) = cdef_direction(&block);
        assert_eq!((y_dir, var), (0, 0), "flat block: yDir 0, var 0");
    }

    #[test]
    fn horizontal_gradient_picks_a_horizontal_direction() {
        let mut block = [[0i32; 8]; 8];
        for (i, row) in block.iter_mut().enumerate() {
            for cell in row.iter_mut() {
                *cell = i as i32 * 16 - 56; // distinct per-row value, zero mean-ish
            }
        }
        let (y_dir, var) = cdef_direction(&block);
        assert_eq!(y_dir, 2, "row-varying block selects direction 2");
        assert!(var > 0, "a non-flat block has positive variance: var={var}");
    }

    /// § 7.18.2 as written, one sample at a time.
    fn spec_direction(block: &[[i32; 8]; 8]) -> (usize, i32) {
        let mut partial = [[0i32; 15]; 8];
        for i in 0..8 {
            for j in 0..8 {
                let x = block[i][j];
                partial[0][i + j] += x;
                partial[1][i + j / 2] += x;
                partial[2][i] += x;
                partial[3][3 + i - j / 2] += x;
                partial[4][7 + i - j] += x;
                partial[5][3 - i / 2 + j] += x;
                partial[6][j] += x;
                partial[7][i / 2 + j] += x;
            }
        }
        let square = |v: i32| v * v;
        let mut cost = [0i32; 8];
        for i in 0..8 {
            cost[2] += square(partial[2][i]);
            cost[6] += square(partial[6][i]);
        }
        cost[2] *= DIV_TABLE[8];
        cost[6] *= DIV_TABLE[8];
        for i in 0..7 {
            for d in [0, 4] {
                cost[d] += (square(partial[d][i]) + square(partial[d][14 - i])) * DIV_TABLE[i + 1];
            }
        }
        cost[0] += square(partial[0][7]) * DIV_TABLE[8];
        cost[4] += square(partial[4][7]) * DIV_TABLE[8];
        for i in (1..8).step_by(2) {
            for j in 0..5 {
                cost[i] += square(partial[i][3 + j]);
            }
            cost[i] *= DIV_TABLE[8];
            for j in 0..3 {
                cost[i] +=
                    (square(partial[i][j]) + square(partial[i][10 - j])) * DIV_TABLE[2 * j + 2];
            }
        }
        let (mut best_cost, mut y_dir) = (0, 0);
        for (i, &c) in cost.iter().enumerate() {
            if c > best_cost {
                best_cost = c;
                y_dir = i;
            }
        }
        (y_dir, (best_cost - cost[(y_dir + 4) & 7]) >> 10)
    }

    #[test]
    fn direction_matches_spec_partials_and_costs() {
        let mut state = 0x2545_f491u32;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 8
        };
        let mut directions = [0usize; 8];
        for case in 0..4000usize {
            let mut block = [[0i32; 8]; 8];
            let slope = (case % 9) as i32 - 4;
            for (i, row) in block.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    let ramp =
                        slope * (i as i32 * (case % 3) as i32 + j as i32 * (case % 5) as i32);
                    let noise = (next() % 64) as i32 - 32;
                    *cell = (ramp * 6 + noise).clamp(-128, 127);
                }
            }
            if case % 7 == 0 {
                block = [[if case % 2 == 0 { -128 } else { 127 }; 8]; 8];
            }
            let expected = spec_direction(&block);
            directions[expected.0] += 1;
            assert_eq!(cdef_direction(&block), expected, "case {case}");
            let mut pad = [0u16; CDEF_PADDED_AREA];
            for (i, row) in block.iter().enumerate() {
                for (j, &cell) in row.iter().enumerate() {
                    pad[(i + 2) * CDEF_PADDED_SIDE + j + 2] = (cell + 128) as u16;
                }
            }
            assert_eq!(
                cdef_direction_padded(&pad, 0),
                expected,
                "padded case {case}"
            );
        }
        assert!(directions.iter().all(|&count| count > 0), "{directions:?}");
    }

    #[test]
    fn padded_direction_matches_materialized_block() {
        for coeff_shift in [0u32, 2, 4] {
            let mut state = 0x9e37_79b9u32 ^ coeff_shift;
            let mut pad = [0u16; CDEF_PADDED_AREA];
            let mut block = [[0i32; 8]; 8];
            for (i, row) in block.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let sample = ((state >> 16) & ((256 << coeff_shift) - 1)) as u16;
                    pad[(i + 2) * CDEF_PADDED_SIDE + j + 2] = sample;
                    *cell = (i32::from(sample) >> coeff_shift) - 128;
                }
            }
            assert_eq!(
                cdef_direction_padded(&pad, coeff_shift),
                cdef_direction(&block),
                "coeff_shift={coeff_shift}"
            );
        }
    }

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
