// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The kernels a decoder selects once, at its start, beyond those its backend
//! runs. On x86-64 that is the AVX-512 set, bound to safe function pointers.
//!
//! Adding one: write the kernel generic over `S: Simd` and
//! `#[inline(always)]`, add its entry to [`wide_kernels!`], branch on
//! [`KernelSelection`] at the entry of the kernel it widens, and compare the
//! two in a test.

#[cfg(target_arch = "x86_64")]
use std::sync::OnceLock;

#[cfg(target_arch = "x86_64")]
use fearless_simd::Avx512;

#[cfg(target_arch = "x86_64")]
use crate::IntraRectBlockSize;

/// PAETH prediction of one block: its size, the left and above edges with the
/// corner sample, and the output rows with their stride.
#[cfg(target_arch = "x86_64")]
pub(crate) type PaethFn<O> = fn(IntraRectBlockSize, (&[O], &[O], u16), &mut [O], usize);

/// The palette lookup of one block: the low and high bytes of its colors, its
/// color map and the output samples; returns the largest index of the map.
#[cfg(target_arch = "x86_64")]
pub(crate) type PaletteFn<O> = fn([u8; 16], Option<[u8; 16]>, &[u8], &mut [O]) -> u8;

/// Scaled film-grain noise added to one row: the row, its scaling values and
/// its noise.
#[cfg(target_arch = "x86_64")]
pub(crate) type GrainFn<O> =
    fn(&mut [O], &[i16], &[i16], crate::film_grain::NoiseParams) -> crate::Result<()>;

/// Luma film-grain noise of one 8-bit row through its packed scaling table.
#[cfg(target_arch = "x86_64")]
pub(crate) type LumaGrainFn = fn(&mut [u8], &[u8; 256], &[i16], crate::film_grain::NoiseParams);

/// The 8x8 secondary-transform sums of one kernel: its paired weight rows,
/// the input coefficients and whether one exceeds `i16`.
#[cfg(target_arch = "x86_64")]
pub(crate) type IstSumsFn = fn(
    &[[i16; 2 * crate::secondary_transform::IST_8X8_WIDTH];
         crate::secondary_transform::IST_8X8_HEIGHT / 2],
    &[i32],
    bool,
) -> [i32; crate::secondary_transform::IST_8X8_WIDTH];

/// Declares [`WideKernels`]: each entry names a field, the arguments and
/// pointer type of its kernel, and the generic kernel it runs. Each pointer
/// is made by a `fearless_simd::kernel!` factory, so the kernel is compiled
/// where the AVX-512 features are enabled.
macro_rules! wide_kernels {
    ($( $field:ident($($arg:ident),*): $pointer:ty => $kernel:expr; )*) => {
        /// One entry per kernel and sample type.
        #[cfg(target_arch = "x86_64")]
        pub(crate) struct WideKernels {
            $(pub(crate) $field: $pointer,)*
        }

        #[cfg(target_arch = "x86_64")]
        impl WideKernels {
            /// The set for `token`, bound on first use.
            pub(crate) fn of(token: Avx512) -> &'static Self {
                static SET: OnceLock<WideKernels> = OnceLock::new();
                SET.get_or_init(|| {
                    $(
                        fearless_simd::kernel!(
                            fn $field(_proof: Avx512) -> $pointer {
                                |$($arg),*| $kernel(Avx512::assume_supported(), $($arg),*)
                            }
                        );
                    )*
                    Self { $($field: $field(token),)* }
                })
            }
        }
    };
}

wide_kernels! {
    paeth_u8(size, edges, output, stride): PaethFn<u8> => crate::intra_basic::paeth_rows_wide;
    paeth_u16(size, edges, output, stride): PaethFn<u16> => crate::intra_basic::paeth_rows_wide;
    palette_u8(low, high, map, output): PaletteFn<u8> => crate::intra::palette_lanes_wide;
    palette_u16(low, high, map, output): PaletteFn<u16> => crate::intra::palette_lanes_wide;
    grain_u8(row, scale, noise, params): GrainFn<u8> =>
        crate::film_grain::add_noise_row::<_, _, true>;
    grain_u16(row, scale, noise, params): GrainFn<u16> =>
        crate::film_grain::add_noise_row::<_, _, true>;
    luma_grain(row, table, noise, params): LumaGrainFn => crate::film_grain::add_luma_noise;
    ist_8x8_sums(pairs, coefs, wide): IstSumsFn => crate::secondary_transform::ist_8x8_sums_wide;
}

/// What a decoder selected beyond its backend: the AVX-512 kernels when it
/// selected AVX-512, nothing otherwise. The frame workspace carries it to the
/// kernels that have a wider form.
#[derive(Clone, Copy, Default)]
pub struct KernelSelection {
    #[cfg(target_arch = "x86_64")]
    wide: Option<&'static WideKernels>,
}

impl PartialEq for KernelSelection {
    fn eq(&self, other: &Self) -> bool {
        self.holds_avx512() == other.holds_avx512()
    }
}

impl Eq for KernelSelection {}

impl core::fmt::Debug for KernelSelection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("KernelSelection")
            .field("avx512", &self.holds_avx512())
            .finish()
    }
}

impl KernelSelection {
    /// The selection of a decoder that resolved `backend`.
    #[must_use]
    pub fn of(backend: splot_core::simd::ResolvedBackend) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self {
                wide: backend.avx512_token().map(WideKernels::of),
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = backend;
            Self {}
        }
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn wide(self) -> Option<&'static WideKernels> {
        self.wide
    }

    /// Whether the selection holds the AVX-512 kernels; there is one set.
    const fn holds_avx512(self) -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            self.wide.is_some()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = self;
            false
        }
    }
}

/// What the comparison and timing tests of the AVX-512 kernels share.
#[cfg(all(test, target_arch = "x86_64"))]
pub(crate) mod test_support {
    use fearless_simd::{Avx512, Level};

    /// The host's AVX-512 token; a run with `SPLOT_REQUIRE_AVX512` set fails
    /// on a host without one.
    pub(crate) fn native_avx512() -> Option<Avx512> {
        let native = Level::new().as_avx512();
        assert!(
            native.is_some() || std::env::var_os("SPLOT_REQUIRE_AVX512").is_none(),
            "SPLOT_REQUIRE_AVX512 is set on a host without AVX-512"
        );
        native
    }

    /// Prints twelve alternating rounds of the two timings after one warm-up
    /// of each, as `label,round,narrow,wide`.
    pub(crate) fn alternate(label: &str, narrow: impl Fn() -> f64, wide: impl Fn() -> f64) {
        let _ = (narrow(), wide());
        for round in 0..12 {
            let (narrow, wide) = if round % 2 == 0 {
                (narrow(), wide())
            } else {
                let wide = wide();
                (narrow(), wide)
            };
            println!("{label},{},{narrow:.2},{wide:.2}", round + 1);
        }
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;
    use splot_core::simd::{SelectedBackend, SimdPolicy, resolve_backend};

    /// A decoder holds the AVX-512 kernels exactly when it selected AVX-512.
    #[test]
    fn selection_follows_the_resolved_policy() {
        let forced = resolve_backend(SimdPolicy::Avx2).expect("x86-64-v3 host");
        assert_eq!(KernelSelection::of(forced), KernelSelection::default());
        let auto = resolve_backend(SimdPolicy::Auto).expect("supported host");
        assert_eq!(
            KernelSelection::of(auto).wide().is_some(),
            auto.kind() == SelectedBackend::Avx512
        );
        if let Ok(wide) = resolve_backend(SimdPolicy::Avx512) {
            assert_ne!(KernelSelection::of(wide), KernelSelection::default());
        }
        assert_eq!(
            resolve_backend(SimdPolicy::Avx512).is_ok(),
            test_support::native_avx512().is_some()
        );
    }
}
