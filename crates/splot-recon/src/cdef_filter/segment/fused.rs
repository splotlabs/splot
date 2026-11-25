// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Interior CDEF segment gather and filtering in one SIMD scope.

use super::*;

#[derive(Clone, Copy)]
struct InScopeSegment<S, R>(S, R);

impl<S: SimdToken, R: LaneBounds<S>> SegmentSimd for InScopeSegment<S, R> {
    #[expect(clippy::inline_always, reason = "reuses the segment's SIMD scope")]
    #[inline(always)]
    fn gather<O>(self, op: impl SegmentGather<O>) -> Option<O> {
        op.run(self.0, self.1)
    }
}

/// Borrowed source samples for one interior CDEF segment plane or chroma pair.
pub struct CdefSegmentSource<S> {
    /// One plane or two chroma planes with common dimensions and stride.
    pub samples: S,
    /// Visible plane width in samples.
    pub width: usize,
    /// Distance between input rows in samples.
    pub stride: usize,
    /// Column and row of the first output sample, relative to the source slice.
    pub origin: (usize, usize),
}

/// The deblocked luma and chroma inputs used by a four-block CDEF segment.
pub struct CdefSegmentSources<'a, T> {
    /// Luma input when luma filtering or chroma direction uses it.
    pub luma: Option<CdefSegmentSource<&'a [T]>>,
    /// The U and V inputs of a 4:2:0 segment when chroma filtering uses them.
    pub chroma: Option<CdefSegmentSource<[&'a [T]; 2]>>,
}

/// A 32×8 luma destination and, when present, the two 16×4 chroma destinations.
/// Each destination is a slice starting at the first output sample and a stride.
pub type CdefSegmentDestination<'a, T> = ((&'a mut [T], usize), Option<[(&'a mut [T], usize); 2]>);

/// Scratch reused across interior CDEF segments in a stripe.
#[derive(Default)]
pub struct CdefSegmentScratch {
    luma: CdefLumaSegment,
    pair: CdefPairSegment,
    #[cfg(target_arch = "x86_64")]
    block_call: Option<table::BlockCall>,
}

/// Fixes the experimental CDEF call mode for this process at decoder startup.
/// Only sessions carrying a validated AVX2 capability can use the table.
#[doc(hidden)]
pub fn initialize_cdef_block_experiment(backend: splot_core::simd::ResolvedBackend) {
    #[cfg(target_arch = "x86_64")]
    if matches!(
        backend,
        splot_core::simd::ResolvedBackend::Avx2(_) | splot_core::simd::ResolvedBackend::Avx512(_)
    ) {
        table::configure();
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = backend;
}

impl CdefSegmentScratch {
    /// Binds the startup-selected block call before iterating a decode stripe.
    #[doc(hidden)]
    pub fn for_decode<B: SimdBackend>(backend: B) -> Self {
        #[cfg(not(target_arch = "x86_64"))]
        let _ = backend;
        Self {
            #[cfg(target_arch = "x86_64")]
            block_call: backend.avx2_token().and_then(|_| table::selected()),
            ..Self::default()
        }
    }

    /// Gathers a four-block interior segment, filters it and writes destination rows.
    /// The caller validates the tile halo and initializes skipped output samples.
    /// Returns `false` when a gather or filter cannot access the supplied rows.
    /// A failed gather leaves the destination unchanged. The filter may have
    /// written earlier blocks before reporting a failure.
    pub fn filter_with<B: SimdBackend, T: crate::ReconSample>(
        &mut self,
        backend: B,
        sources: CdefSegmentSources<'_, T>,
        unit: &CdefUnitFilters,
        blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
        fill_flat: [bool; 3],
        output: CdefSegmentDestination<'_, T>,
    ) -> bool {
        #[cfg(target_arch = "x86_64")]
        if let Some(avx2) = backend.avx2_token() {
            let block_call = self.block_call;
            return avx2.vectorize(
                #[inline(always)]
                || {
                    self.filter(
                        InScopeSegment(avx2, MinPos),
                        sources,
                        unit,
                        output,
                        #[inline(always)]
                        |segment, output| match block_call {
                            Some(call) => table::filter_segment(
                                call, avx2, segment, unit, blocks, fill_flat, output,
                            ),
                            None => {
                                segment_with(avx2, InLane, segment, unit, blocks, fill_flat, output)
                            }
                        },
                    )
                },
            );
        }
        let narrow = backend.narrow();
        splot_core::with_simd!(narrow, simd => self.filter(InScopeSegment(simd, Reduced), sources, unit, output,
            #[inline(always)]
            |segment, output| segment_with(simd, Ordered, segment, unit, blocks, fill_flat, output),
        ))
    }

    #[expect(
        clippy::inline_always,
        reason = "gathers and filters in one SIMD scope"
    )]
    #[inline(always)]
    fn filter<S: SimdToken, R: LaneBounds<S>, T: crate::ReconSample>(
        &mut self,
        simd: InScopeSegment<S, R>,
        sources: CdefSegmentSources<'_, T>,
        unit: &CdefUnitFilters,
        (luma_out, chroma_out): CdefSegmentDestination<'_, T>,
        run: impl FnOnce(&CdefLumaSegment, CdefSegmentOutput<'_, T>) -> bool,
    ) -> bool {
        if let Some(CdefSegmentSource {
            samples,
            width,
            stride,
            origin,
        }) = sources.luma
        {
            let gathered =
                if let Some(bytes) = T::u8_slice(samples).filter(|_| unit.luma_bytes_suffice()) {
                    self.luma
                        .gather_bytes_with_simd(simd, bytes, width, stride, origin)
                } else {
                    self.luma
                        .gather_with_simd(simd, samples, width, stride, origin)
                };
            if !gathered {
                return false;
            }
        }
        let chroma = match (sources.chroma, chroma_out) {
            (
                Some(CdefSegmentSource {
                    samples: [u, v],
                    width,
                    stride,
                    origin,
                }),
                Some(planes),
            ) => {
                let gathered = match (T::u8_slice(u), T::u8_slice(v)) {
                    (Some(u), Some(v)) if unit.chroma_bytes_suffice() => self
                        .pair
                        .gather_with_mode::<_, _, true>(simd, u, v, width, stride, origin),
                    _ => self
                        .pair
                        .gather_with_mode::<_, _, false>(simd, u, v, width, stride, origin),
                };
                if !gathered {
                    return false;
                }
                Some((&self.pair, planes))
            }
            (None, None) => None,
            _ => return false,
        };
        run(&self.luma, (luma_out, chroma))
    }
}
