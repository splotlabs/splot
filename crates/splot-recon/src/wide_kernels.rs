// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The kernels a decoder selects once, at its start, beyond those its backend
//! runs. On x86-64 that is the AVX-512 set, bound to safe function pointers.

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

/// One entry per kernel and sample type. Each is made by a factory beside
/// its kernel, which converts the kernel to a pointer where the AVX-512
/// features are enabled.
#[cfg(target_arch = "x86_64")]
pub(crate) struct WideKernels {
    pub(crate) paeth_u8: PaethFn<u8>,
    pub(crate) paeth_u16: PaethFn<u16>,
    pub(crate) palette_u8: PaletteFn<u8>,
    pub(crate) palette_u16: PaletteFn<u16>,
}

#[cfg(target_arch = "x86_64")]
impl WideKernels {
    /// The set for `token`, bound on first use.
    pub(crate) fn of(token: Avx512) -> &'static Self {
        static SET: OnceLock<WideKernels> = OnceLock::new();
        SET.get_or_init(|| Self {
            paeth_u8: crate::intra_basic::bind_paeth_u8(token),
            paeth_u16: crate::intra_basic::bind_paeth_u16(token),
            palette_u8: crate::intra::bind_palette_u8(token),
            palette_u16: crate::intra::bind_palette_u16(token),
        })
    }
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
        #[cfg(target_arch = "x86_64")]
        {
            match (self.wide, other.wide) {
                (Some(a), Some(b)) => core::ptr::eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = other;
            true
        }
    }
}

impl Eq for KernelSelection {}

impl core::fmt::Debug for KernelSelection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("KernelSelection")
            .finish_non_exhaustive()
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
        } else {
            assert!(std::env::var_os("SPLOT_REQUIRE_AVX512").is_none());
        }
    }
}
