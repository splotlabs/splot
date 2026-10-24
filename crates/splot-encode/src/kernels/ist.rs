// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Forward § 7.15.3 secondary transform: the transpose of the decoder's
//! inverse. Each of the `n` outputs is `Round2Signed(row · x, 7)` of one kernel
//! row and the primary coefficients `x` at the layout's output positions; the
//! outputs go to the input positions and every other coefficient is zero.

use std::simd::Simd;
use std::simd::num::SimdInt as _;

use splot_recon::SecondaryTransformLayout;
use splot_recon::math::round2_signed_i32;

use super::KernelError;

const LANES: usize = 8;

/// The clipped primary coefficients the kernel columns weigh, after checking
/// that `coeffs` holds every layout position.
fn gather(layout: &SecondaryTransformLayout, coeffs: &[i32]) -> Result<[i32; 48], KernelError> {
    let needed = layout.inputs[..layout.n]
        .iter()
        .chain(&layout.outputs[..layout.width])
        .map(|&pos| usize::from(pos) + 1)
        .max()
        .unwrap_or(0);
    if coeffs.len() < needed || layout.kernel.len() != layout.n * layout.width {
        return Err(KernelError::BufferLength {
            buffer: "secondary transform coefficients",
            expected: needed,
            actual: coeffs.len(),
        });
    }
    let bound = layout.bound;
    let mut x = [0; 48];
    for (slot, &pos) in x.iter_mut().zip(&layout.outputs[..layout.width]) {
        *slot = coeffs[usize::from(pos)].clamp(-bound, bound - 1);
    }
    Ok(x)
}

/// Writes the `n` outputs to the input positions of a cleared `coeffs`.
fn scatter(layout: &SecondaryTransformLayout, outputs: &[i32], coeffs: &mut [i32]) {
    coeffs.fill(0);
    for (&value, &pos) in outputs.iter().zip(&layout.inputs[..layout.n]) {
        coeffs[usize::from(pos)] = round2_signed_i32(value, 7);
    }
}

/// The scalar forward secondary transform of `coeffs`, in place.
///
/// # Errors
/// Returns [`KernelError::BufferLength`] if `coeffs` misses a layout position.
#[cfg(test)]
pub(crate) fn forward_scalar(
    layout: &SecondaryTransformLayout,
    coeffs: &mut [i32],
) -> Result<(), KernelError> {
    let x = gather(layout, coeffs)?;
    let mut sums = [0; 32];
    for (sum, row) in sums
        .iter_mut()
        .zip(layout.kernel.chunks_exact(layout.width))
    {
        *sum = row.iter().zip(&x).map(|(&k, &v)| k * v).sum();
    }
    scatter(layout, &sums[..layout.n], coeffs);
    Ok(())
}

/// [`forward_scalar`] with eight-lane dot products.
///
/// # Errors
/// Returns [`KernelError::BufferLength`] if `coeffs` misses a layout position.
pub(crate) fn forward_simd(
    layout: &SecondaryTransformLayout,
    coeffs: &mut [i32],
) -> Result<(), KernelError> {
    let x = gather(layout, coeffs)?;
    let x = &x[..layout.width];
    let mut sums = [0; 32];
    for (sum, row) in sums
        .iter_mut()
        .zip(layout.kernel.chunks_exact(layout.width))
    {
        let mut acc = Simd::<i32, LANES>::splat(0);
        for (k, v) in row.chunks_exact(LANES).zip(x.chunks_exact(LANES)) {
            acc += Simd::from_slice(k) * Simd::from_slice(v);
        }
        *sum = acc.reduce_sum();
    }
    scatter(layout, &sums[..layout.n], coeffs);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use proptest::prelude::*;
    use splot_recon::{
        BitDepth, SecondaryInverseTransform, TransformClass, secondary_inverse_transform,
        secondary_transform_layout,
    };

    use super::*;

    fn params() -> impl Strategy<Value = SecondaryInverseTransform> {
        let sides = || prop_oneof![Just(4usize), Just(8), Just(16), Just(32)];
        (
            sides(),
            sides(),
            0..14usize,
            1..4usize,
            any::<bool>(),
            0..3usize,
            any::<bool>(),
        )
            .prop_flat_map(|(w, h, kernel, sec_tx_type, transpose, class, ten)| {
                let large = w >= 8 && h >= 8;
                let (height, sets): (usize, usize) = if large { (32, 11) } else { (8, 14) };
                (1..=height).prop_map(move |n| SecondaryInverseTransform {
                    w,
                    h,
                    n,
                    kernel: kernel % sets,
                    sec_tx_type,
                    primary_scan_class: [
                        TransformClass::TwoD,
                        TransformClass::Horizontal,
                        TransformClass::Vertical,
                    ][class],
                    transpose,
                    bit_depth: if ten { BitDepth::Ten } else { BitDepth::Eight },
                })
            })
    }

    proptest! {
        /// Coefficients of asymmetric range and sign, past the clip bound too.
        #[test]
        fn simd_matches_scalar(
            params in params(),
            seed in prop::collection::vec(-300_000i32..170_000, 1024),
        ) {
            let layout = secondary_transform_layout(&params).unwrap();
            let len = params.w * params.h;
            let mut scalar = seed[..len].to_vec();
            let mut simd = scalar.clone();
            forward_scalar(&layout, &mut scalar).unwrap();
            forward_simd(&layout, &mut simd).unwrap();
            prop_assert_eq!(scalar, simd);
        }

        /// The forward transform of the decoder's inverse returns the inputs
        /// within a few units plus the leak of a Gram matrix within 1.5% of
        /// `128^2` times identity.
        #[test]
        fn forward_undoes_inverse(
            params in params().prop_filter("full height", |p| p.n == if p.w >= 8 && p.h >= 8 { 32 } else { 8 }),
            values in prop::collection::vec(-900i32..2_500, 48),
        ) {
            let layout = secondary_transform_layout(&params).unwrap();
            let mut coeffs = vec![0; params.w * params.h];
            let mut forward = vec![0; 32];
            for (r, slot) in forward[..layout.n].iter_mut().enumerate() {
                *slot = values[r];
            }
            for (&value, &pos) in forward[..layout.n].iter().zip(&layout.inputs) {
                coeffs[usize::from(pos)] = value;
            }
            secondary_inverse_transform(&mut coeffs, &params).unwrap();
            forward_simd(&layout, &mut coeffs).unwrap();
            let leak = forward.iter().map(|v| v.abs()).sum::<i32>() / 64;
            for (r, &pos) in layout.inputs[..layout.n].iter().enumerate() {
                let error = (coeffs[usize::from(pos)] - forward[r]).abs();
                prop_assert!(error <= 4 + leak, "row {r}: {error} above {leak}");
            }
        }
    }

    #[test]
    fn rejects_a_short_block() {
        let layout = secondary_transform_layout(&SecondaryInverseTransform {
            w: 8,
            h: 8,
            n: 20,
            kernel: 1,
            sec_tx_type: 2,
            primary_scan_class: TransformClass::TwoD,
            transpose: false,
            bit_depth: BitDepth::Eight,
        })
        .unwrap();
        assert!(matches!(
            forward_simd(&layout, &mut [0; 16]),
            Err(KernelError::BufferLength { .. })
        ));
    }
}
