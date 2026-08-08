// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder 16x16 forward-transform stage.
//!
//! This module advances `ENC-FORWARD-TRANSFORM-DCT-16X16` (the real 16x16
//! DCT_DCT), the 16x16 analogue of the 4x4 forward transform in
//! [`crate::forward_transform`]. Like the 4x4, it is an encoder-policy arithmetic
//! stage, not a normative AV2 decoding process: AV2 specifies only the *inverse*
//! transform, so the forward transform is derived from it (a true inverse for the
//! flat DC-only subset, bounded for general AC).
//!
//! [`ForwardTransformBlock16x16::dct_dct_16x16`] maps any signed 16x16 residual block
//! to its 256 row-major coefficients via the transposed § 9 [`DCT_KERNEL16`] (a row
//! pass then a column pass with the [`FORWARD_ROW_SHIFT_16X16`] /
//! [`FORWARD_COL_SHIFT_16X16`] down-shifts that pair with the decoder's § 7.15.4
//! 16x16 inverse shifts `(6, 13)`). It is a true inverse **only for the flat
//! (DC-only) subset**: a uniform residual `v` maps to `coefficients[0] = v * 128`,
//! every AC coefficient `0`, and reconstructs bit-exactly. For general AC content the
//! reconstruction is **bounded, not bit-exact** (the AV2 integer DCT16 odd basis rows
//! are not orthonormal; observed `<= 5` over the tested 8-bit residual domain).
//! Quantization absorbs the residue.

#![allow(dead_code)]

use splot_recon::{PlaneId, PlaneRect};
use splot_tables::tables::transform_1d::DCT_KERNEL16;

use crate::error::Result;
use crate::forward_transform_shared::forward_dct_dct_square;
#[cfg(test)]
use crate::forward_transform_shared::forward_round2;

pub(crate) const DCT_DCT_16X16_WIDTH: usize = 16;
pub(crate) const DCT_DCT_16X16_HEIGHT: usize = 16;
pub(crate) const DCT_DCT_16X16_COEFF_COUNT: usize = DCT_DCT_16X16_WIDTH * DCT_DCT_16X16_HEIGHT;

/// Forward 16x16 DCT row-pass (first pass) down-shift.
///
/// The two passes' shifts MUST sum to 13: the § 7.15.4 16x16 DCT_DCT inverse
/// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-15-4`) removes
/// `row_shift 6 + col_shift 13 = 19` bits, and the round-trip separable 2D integer
/// DCT16 gains `2^32`, so the forward must remove `32 - 19 = 13` bits. A total other
/// than 13 explodes the round-trip error (verified by simulation over 3000 random
/// 8-bit blocks: total 12 gives error ~261, total 14 gives ~129; only 13 collapses to
/// the small non-orthogonality residue `<= 5`). The split between the two passes is
/// free as long as the column (second) pass keeps a non-zero shift; concentrating the
/// whole shift in the row pass would lose the rounding the asymmetric inverse expects.
/// (On a flat input the forward DC gain alone is `64 * 16 = 2^10` per pass = `2^20`,
/// so the DC coefficient is `v * 2^(20 - 13) = v * 128`.)
pub(crate) const FORWARD_ROW_SHIFT_16X16: u32 = 0;

/// Forward 16x16 DCT column-pass (second pass) down-shift. See
/// [`FORWARD_ROW_SHIFT_16X16`].
pub(crate) const FORWARD_COL_SHIFT_16X16: u32 = 13;

const _: () = assert!(FORWARD_ROW_SHIFT_16X16 + FORWARD_COL_SHIFT_16X16 == 13);
const _: () = assert!(FORWARD_COL_SHIFT_16X16 >= 1);

/// Row-major transform coefficients for one private encoder 16x16 block.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ForwardTransformBlock16x16 {
    plane: PlaneId,
    block: PlaneRect,
    coefficients: [i32; DCT_DCT_16X16_COEFF_COUNT],
}

impl ForwardTransformBlock16x16 {
    /// Computes the real 16x16 DCT_DCT forward transform: all 256 row-major
    /// coefficients of `residual` (any signed 16x16 block, uniform or not).
    ///
    /// The forward transform is **paired with** (derived from) `splot-recon`'s
    /// § 7.15.4 16x16 DCT_DCT inverse — the transposed § 9 [`DCT_KERNEL16`] applied as
    /// a row pass then a column pass with the [`FORWARD_ROW_SHIFT_16X16`] /
    /// [`FORWARD_COL_SHIFT_16X16`] down-shifts. It is a true inverse **only for the
    /// flat (DC-only) subset**: a uniform residual `v` maps to
    /// `coefficients[0] = v * 128`, every AC coefficient `0`, and reconstructs
    /// bit-exactly. For general AC content the reconstruction is **bounded, not
    /// bit-exact** (the AV2 integer DCT16 odd basis rows are not orthonormal; observed
    /// `<= 5` over the tested 8-bit domain) — callers MUST NOT assume bit-exact AC
    /// recovery; later quantization absorbs the residue.
    ///
    /// # Errors
    /// Returns [`Error::ForwardTransformUnsupportedShape`] if `block` is not 16x16,
    /// [`Error::ForwardTransformInputLengthMismatch`] if `residual` is not 256
    /// samples, and [`Error::ForwardTransformCoefficientRangeExceeded`] if a
    /// coefficient falls outside `i32` (unreachable for valid 8-bit residuals; the
    /// passes accumulate in `i64`).
    pub(crate) fn dct_dct_16x16(
        plane: PlaneId,
        block: PlaneRect,
        residual: &[i32],
    ) -> Result<Self> {
        let coefficients = forward_dct_dct_square::<DCT_DCT_16X16_WIDTH, DCT_DCT_16X16_COEFF_COUNT>(
            plane,
            block,
            residual,
            &DCT_KERNEL16,
            FORWARD_ROW_SHIFT_16X16,
            FORWARD_COL_SHIFT_16X16,
        )?;
        Ok(Self {
            plane,
            block,
            coefficients,
        })
    }

    /// Returns the source plane identity.
    pub(crate) const fn plane(&self) -> PlaneId {
        self.plane
    }

    /// Returns the visible-plane-relative transform block rectangle.
    pub(crate) const fn block(&self) -> PlaneRect {
        self.block
    }

    /// Returns row-major transform coefficients.
    pub(crate) const fn coefficients(&self) -> &[i32; DCT_DCT_16X16_COEFF_COUNT] {
        &self.coefficients
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use splot_recon::{
        BitDepth as ReconBitDepth, InverseTransform1dType, InverseTransform2dDim,
        InverseTransform2dOuter, inverse_transform_2d_outer,
    };

    fn rect(width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(0, 0, width, height).unwrap()
    }

    fn uniform(sample: i32) -> [i32; DCT_DCT_16X16_COEFF_COUNT] {
        [sample; DCT_DCT_16X16_COEFF_COUNT]
    }

    fn dct() -> InverseTransform2dDim {
        InverseTransform2dDim::Kernel(InverseTransform1dType::Dct)
    }

    /// The decoder § 7.15.4 16x16 DCT_DCT inverse (`splot-recon`) with the spec
    /// `(row_shift 6, col_shift 13)` shifts (`transform_shift(4, 4)`).
    fn inverse_16x16_dct_dct(coefficients: &[i32; DCT_DCT_16X16_COEFF_COUNT]) -> Vec<i32> {
        let params = InverseTransform2dOuter {
            log2_width: 4,
            log2_height: 4,
            lossless: false,
            plane_tx_type_is_idtx: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 6,
            col_shift: 13,
            bit_depth: ReconBitDepth::Eight,
            dpcm: None,
        };
        let mut residual = vec![0; DCT_DCT_16X16_COEFF_COUNT];
        inverse_transform_2d_outer(&params, coefficients, &mut residual).unwrap();
        residual
    }

    #[test]
    fn flat_residual_maps_to_dc_only_and_reconstructs_exactly() {
        for v in [-127, -50, -8, -1, 0, 1, 7, 40, 127] {
            let block =
                ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::Y, rect(16, 16), &uniform(v))
                    .unwrap();
            let mut expected = [0; DCT_DCT_16X16_COEFF_COUNT];
            expected[0] = v * 128;
            assert_eq!(block.coefficients(), &expected, "v {v}");
            assert_eq!(
                inverse_16x16_dct_dct(block.coefficients()),
                uniform(v),
                "v {v}"
            );
        }
    }

    #[test]
    fn horizontal_ramp_pins_kernel_orientation() {
        let mut residual = [0i32; DCT_DCT_16X16_COEFF_COUNT];
        for r in 0..DCT_DCT_16X16_HEIGHT {
            for c in 0..DCT_DCT_16X16_WIDTH {
                residual[r * DCT_DCT_16X16_WIDTH + c] = c as i32;
            }
        }
        let block =
            ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::Y, rect(16, 16), &residual).unwrap();
        let coeffs = block.coefficients();
        for r in 1..DCT_DCT_16X16_HEIGHT {
            for c in 0..DCT_DCT_16X16_WIDTH {
                assert_eq!(coeffs[r * DCT_DCT_16X16_WIDTH + c], 0, "row {r} col {c}");
            }
        }
        assert_eq!(
            &coeffs[..DCT_DCT_16X16_WIDTH],
            &[
                960, -586, 0, -64, 0, -22, 0, -10, 0, -5, 0, -3, 0, -1, 0, -2
            ]
        );
    }

    #[test]
    fn forward_round2_matches_recon_round2() {
        assert_eq!(forward_round2(6, 2), 2); // (6 + 2) >> 2
        assert_eq!(forward_round2(-6, 2), -1); // (-6 + 2) >> 2 = -4 >> 2
        assert_eq!(forward_round2(8191, 13), 1); // rounds up at the half
        assert_eq!(forward_round2(4095, 13), 0); // rounds down
        assert_eq!(forward_round2(-12345, 0), -12345); // identity
    }

    #[test]
    fn random_residuals_roundtrip_within_bound_and_never_panic() {
        const BOUND: i32 = 5;
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) % 511) as i32 - 255
        };
        let mut worst = 0i32;
        for _ in 0..2000 {
            let mut residual = [0i32; DCT_DCT_16X16_COEFF_COUNT];
            for sample in &mut residual {
                *sample = next();
            }
            let block =
                ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::Y, rect(16, 16), &residual)
                    .unwrap();
            let reconstructed = inverse_16x16_dct_dct(block.coefficients());
            for (&got, &want) in reconstructed.iter().zip(residual.iter()) {
                let err = (got - want).abs();
                worst = worst.max(err);
                assert!(
                    err <= BOUND,
                    "residual {residual:?}: err {err} exceeds bound {BOUND}"
                );
            }
        }
        assert!(worst >= 1, "expected non-trivial AC residue, got {worst}");
    }

    #[test]
    fn out_of_range_residual_errors_without_panicking() {
        let err =
            ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::Y, rect(16, 16), &uniform(i32::MAX))
                .unwrap_err();
        assert!(matches!(
            err,
            Error::ForwardTransformCoefficientRangeExceeded {
                plane: PlaneId::Y,
                ..
            }
        ));
    }

    #[test]
    fn rejects_non_16x16_and_wrong_length() {
        assert!(matches!(
            ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::U, rect(4, 16), &[0; 256])
                .unwrap_err(),
            Error::ForwardTransformUnsupportedShape {
                plane: PlaneId::U,
                expected_width: 16,
                expected_height: 16,
                ..
            }
        ));
        assert!(matches!(
            ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::Y, rect(16, 16), &[0; 255])
                .unwrap_err(),
            Error::ForwardTransformInputLengthMismatch {
                expected: 256,
                actual: 255,
                ..
            }
        ));
    }
}
