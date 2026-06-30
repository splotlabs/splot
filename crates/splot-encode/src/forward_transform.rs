// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder forward-transform stage.
//!
//! This module advances `ENC-FORWARD-TRANSFORM-FOUNDATION` (the DC-only flat
//! foundation) and `ENC-FORWARD-TRANSFORM-DCT-4X4` (the real 4x4 DCT_DCT). It is
//! an encoder-policy arithmetic stage, not a normative AV2 decoding process: AV2
//! specifies only the *inverse* transform, so the forward transform is derived from
//! it (a true inverse for the flat subset, bounded for general AC — see below).
//!
//! [`ForwardTransformBlock::dct_dct_4x4`] is the real 4x4 DCT_DCT: it maps any
//! signed residual block to all 16 row-major coefficients via the transposed § 9
//! [`DCT_KERNEL4`] (a row pass then a column pass with the [`FORWARD_ROW_SHIFT`] /
//! [`FORWARD_COL_SHIFT`] down-shifts that pair with the decoder's § 7.15.4 4x4
//! inverse shifts). [`ForwardTransformBlock::dct_dct_4x4_dc_only`] is the flat-only
//! fast path the closed loop still uses; a uniform residual `v` maps identically
//! through both (`coefficients[0] = v * 32`, every AC coefficient `0`).
//!
//! Reconstruction through the `splot-recon` inverse is bit-exact for a uniform
//! (DC-only) residual but only *bounded* for general AC content: the AV2 integer
//! DCT4 odd basis rows are not orthonormal, so a no-quant forward then inverse
//! round trip differs from the input by a small residue (observed `<= 5` over the
//! tested 8-bit residual domain). Quantization absorbs this residue; the bound is
//! the proof tier for AC content, not equality.
//!
//! The module does not select transforms, emit syntax, own quantization, or
//! produce [`crate::Packet`] values.

#![allow(dead_code)]

use splot_recon::{PlaneId, PlaneRect};
use splot_tables::tables::transform_1d::DCT_KERNEL4;

use crate::error::{Error, Result};
#[cfg(test)]
use crate::forward_transform_shared::forward_round2;
use crate::forward_transform_shared::{
    forward_dct_dct_square, validate_forward_input_length, validate_forward_shape,
};

#[allow(unused_imports)]
pub(crate) use crate::forward_transform_16x16::ForwardTransformBlock16x16;

const DCT_DCT_4X4_WIDTH: usize = 4;
const DCT_DCT_4X4_HEIGHT: usize = 4;
const DCT_DCT_4X4_COEFF_COUNT: usize = DCT_DCT_4X4_WIDTH * DCT_DCT_4X4_HEIGHT;
const DCT_DCT_4X4_DC_SCALE: i32 = 32;

/// Forward 4x4 DCT row-pass (first pass) down-shift.
///
/// The two passes' shifts MUST sum to 11: the § 7.15.4 4x4 DCT_DCT inverse removes
/// `row_shift 7 + col_shift 10 = 17` bits, and the separable forward 2D DCT gains
/// `2^28` on a unit DC, so the forward must remove `28 - 17 = 11` bits. A total
/// other than 11 explodes the round-trip error (verified by simulation: 10 or 12
/// give error ~130, only 11 collapses to the small non-orthogonality residue
/// `<= 5`). The split between the two passes is free as long as the column (second)
/// pass keeps a non-zero shift; concentrating the whole shift in the row pass (a
/// `(11, 0)` split) loses the rounding the asymmetric inverse expects and breaks
/// both the bound and the flat `dc = v * 32` property.
const FORWARD_ROW_SHIFT: u32 = 0;

/// Forward 4x4 DCT column-pass (second pass) down-shift. See [`FORWARD_ROW_SHIFT`].
const FORWARD_COL_SHIFT: u32 = 11;

const _: () = assert!(FORWARD_ROW_SHIFT + FORWARD_COL_SHIFT == 11);
const _: () = assert!(FORWARD_COL_SHIFT >= 1);

/// Row-major transform coefficients for one private encoder block.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ForwardTransformBlock {
    plane: PlaneId,
    block: PlaneRect,
    coefficients: [i32; DCT_DCT_4X4_COEFF_COUNT],
}

impl ForwardTransformBlock {
    /// Computes the current 4x4 DCT_DCT DC-only transform subset.
    ///
    /// `residual` is row-major signed residual input for `block`. The current
    /// subset accepts only a 4x4 block where every residual sample is identical.
    pub(crate) fn dct_dct_4x4_dc_only(
        plane: PlaneId,
        block: PlaneRect,
        residual: &[i32],
    ) -> Result<Self> {
        validate_forward_shape(plane, block, DCT_DCT_4X4_WIDTH, DCT_DCT_4X4_HEIGHT)?;
        validate_forward_input_length(plane, block, DCT_DCT_4X4_COEFF_COUNT, residual.len())?;

        let first = residual[0];
        for (index, &sample) in residual.iter().enumerate().skip(1) {
            if sample != first {
                return Err(Error::ForwardTransformNonUniformResidual {
                    plane,
                    block,
                    first,
                    mismatch_index: index,
                    value: sample,
                });
            }
        }

        let dc = first.checked_mul(DCT_DCT_4X4_DC_SCALE).ok_or(
            Error::ForwardTransformCoefficientOverflow {
                plane,
                block,
                sample: first,
                scale: DCT_DCT_4X4_DC_SCALE,
            },
        )?;
        let mut coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
        coefficients[0] = dc;
        Ok(Self {
            plane,
            block,
            coefficients,
        })
    }

    /// Computes the real 4x4 DCT_DCT forward transform: all 16 row-major
    /// coefficients of `residual` (any signed 4x4 block, uniform or not).
    ///
    /// The forward transform is **paired with** (derived from) `splot-recon`'s
    /// § 7.15.4 4x4 DCT_DCT inverse — the transposed § 9 [`DCT_KERNEL4`] applied as a
    /// row pass then a column pass with the [`FORWARD_ROW_SHIFT`] /
    /// [`FORWARD_COL_SHIFT`] down-shifts. It is a true inverse **only for the flat
    /// (DC-only) subset**: a uniform residual `v` maps to `coefficients[0] = v * 32`,
    /// every AC coefficient `0`, exactly matching [`Self::dct_dct_4x4_dc_only`], and
    /// reconstructs bit-exactly. For general AC content the reconstruction is
    /// **bounded, not bit-exact** (the AV2 integer DCT4 odd basis rows are not
    /// orthonormal; observed `<= 5` over the tested 8-bit domain) — callers MUST NOT
    /// assume bit-exact AC recovery; later quantization absorbs the residue.
    ///
    /// # Errors
    /// Returns [`Error::ForwardTransformUnsupportedShape`] if `block` is not 4x4,
    /// [`Error::ForwardTransformInputLengthMismatch`] if `residual` is not 16
    /// samples, and [`Error::ForwardTransformCoefficientRangeExceeded`] if a
    /// coefficient falls outside `i32` (unreachable for valid 8-bit residuals; the
    /// passes accumulate in `i64`).
    pub(crate) fn dct_dct_4x4(plane: PlaneId, block: PlaneRect, residual: &[i32]) -> Result<Self> {
        let coefficients = forward_dct_dct_square::<DCT_DCT_4X4_WIDTH, DCT_DCT_4X4_COEFF_COUNT>(
            plane,
            block,
            residual,
            &DCT_KERNEL4,
            FORWARD_ROW_SHIFT,
            FORWARD_COL_SHIFT,
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
    pub(crate) const fn coefficients(&self) -> &[i32; DCT_DCT_4X4_COEFF_COUNT] {
        &self.coefficients
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use splot_recon::{
        BitDepth as ReconBitDepth, InverseTransform1dType, InverseTransform2dDim,
        InverseTransform2dOuter, inverse_transform_2d_outer,
    };

    fn rect(width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(0, 0, width, height).unwrap()
    }

    fn uniform(sample: i32) -> [i32; DCT_DCT_4X4_COEFF_COUNT] {
        [sample; DCT_DCT_4X4_COEFF_COUNT]
    }

    fn transform(sample: i32) -> ForwardTransformBlock {
        ForwardTransformBlock::dct_dct_4x4_dc_only(PlaneId::Y, rect(4, 4), &uniform(sample))
            .unwrap()
    }

    fn dct() -> InverseTransform2dDim {
        InverseTransform2dDim::Kernel(InverseTransform1dType::Dct)
    }

    fn inverse_4x4_dct_dct(coefficients: &[i32; DCT_DCT_4X4_COEFF_COUNT]) -> Vec<i32> {
        let params = InverseTransform2dOuter {
            log2_width: 2,
            log2_height: 2,
            lossless: false,
            plane_tx_type_is_idtx: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 7,
            col_shift: 10,
            bit_depth: ReconBitDepth::Eight,
            dpcm: None,
        };
        let mut residual = vec![0; DCT_DCT_4X4_COEFF_COUNT];
        inverse_transform_2d_outer(&params, coefficients, &mut residual).unwrap();
        residual
    }

    #[test]
    fn zero_uniform_residual_maps_to_all_zero_coefficients() {
        let transformed = transform(0);

        assert_eq!(transformed.plane(), PlaneId::Y);
        assert_eq!(transformed.block(), rect(4, 4));
        assert_eq!(transformed.coefficients(), &[0; DCT_DCT_4X4_COEFF_COUNT]);
    }

    #[test]
    fn positive_uniform_residual_maps_to_dc_only_coefficients() {
        let transformed = transform(7);
        let mut expected = [0; DCT_DCT_4X4_COEFF_COUNT];
        expected[0] = 224;

        assert_eq!(transformed.coefficients(), &expected);
    }

    #[test]
    fn negative_uniform_residual_maps_to_signed_dc_coefficient() {
        let transformed = transform(-8);
        let mut expected = [0; DCT_DCT_4X4_COEFF_COUNT];
        expected[0] = -256;

        assert_eq!(transformed.coefficients(), &expected);
    }

    #[test]
    fn no_op_quant_dequant_inverse_reconstructs_uniform_residual() {
        for sample in [-16, -1, 0, 1, 15] {
            let transformed = transform(sample);

            assert_eq!(
                inverse_4x4_dct_dct(transformed.coefficients()),
                uniform(sample),
                "sample {sample}"
            );
        }
    }

    #[test]
    fn rejects_non_4x4_block_shape() {
        let err = ForwardTransformBlock::dct_dct_4x4_dc_only(PlaneId::U, rect(2, 4), &[0; 16])
            .unwrap_err();

        assert!(matches!(
            err,
            Error::ForwardTransformUnsupportedShape {
                plane: PlaneId::U,
                block,
                expected_width: 4,
                expected_height: 4,
            } if block == rect(2, 4)
        ));
    }

    #[test]
    fn rejects_wrong_residual_input_length() {
        let err = ForwardTransformBlock::dct_dct_4x4_dc_only(PlaneId::Y, rect(4, 4), &[0; 15])
            .unwrap_err();

        assert!(matches!(
            err,
            Error::ForwardTransformInputLengthMismatch {
                plane: PlaneId::Y,
                expected: 16,
                actual: 15,
                ..
            }
        ));
    }

    #[test]
    fn rejects_non_uniform_residual_input() {
        let mut residual = uniform(4);
        residual[9] = 5;
        let err = ForwardTransformBlock::dct_dct_4x4_dc_only(PlaneId::V, rect(4, 4), &residual)
            .unwrap_err();

        assert!(matches!(
            err,
            Error::ForwardTransformNonUniformResidual {
                plane: PlaneId::V,
                first: 4,
                mismatch_index: 9,
                value: 5,
                ..
            }
        ));
    }

    #[test]
    fn rejects_dc_coefficient_overflow() {
        let err =
            ForwardTransformBlock::dct_dct_4x4_dc_only(PlaneId::Y, rect(4, 4), &uniform(i32::MAX))
                .unwrap_err();

        assert!(matches!(
            err,
            Error::ForwardTransformCoefficientOverflow {
                plane: PlaneId::Y,
                sample: i32::MAX,
                scale: 32,
                ..
            }
        ));
    }

    #[test]
    fn full_dct_flat_residual_matches_dc_only_stub_and_reconstructs_exactly() {
        for v in [-50, -8, -1, 0, 1, 7, 40, 127] {
            let full =
                ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), &uniform(v)).unwrap();
            assert_eq!(full.coefficients(), transform(v).coefficients(), "v {v}");
            let mut expected = [0; DCT_DCT_4X4_COEFF_COUNT];
            expected[0] = v * DCT_DCT_4X4_DC_SCALE;
            assert_eq!(full.coefficients(), &expected, "v {v}");
            assert_eq!(
                inverse_4x4_dct_dct(full.coefficients()),
                uniform(v),
                "v {v}"
            );
        }
    }

    #[test]
    fn full_dct_horizontal_ramp_pins_kernel_orientation() {
        let residual = [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3];
        let block = ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), &residual).unwrap();
        assert_eq!(
            block.coefficients(),
            &[48, -35, 0, -3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn full_dct_nonuniform_roundtrips_within_residue_bound() {
        let cases: [[i32; DCT_DCT_4X4_COEFF_COUNT]; 3] = [
            [-7, -6, -5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8],
            [
                10, 10, 10, 10, 10, 10, 10, 10, -20, -20, -20, -20, -20, -20, -20, -20,
            ],
            [3, -9, 14, -2, 0, 5, -11, 8, -6, 1, 12, -4, 7, -13, 2, -1],
        ];
        for residual in cases {
            let block =
                ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), &residual).unwrap();
            let reconstructed = inverse_4x4_dct_dct(block.coefficients());
            for (k, (&got, &want)) in reconstructed.iter().zip(residual.iter()).enumerate() {
                assert!(
                    (got - want).abs() <= 5,
                    "residual {residual:?} index {k}: reconstructed {got} vs original {want} exceeds bound 5"
                );
            }
        }
    }

    #[test]
    fn forward_round2_matches_recon_round2() {
        assert_eq!(forward_round2(6, 2), 2); // (6 + 2) >> 2
        assert_eq!(forward_round2(-6, 2), -1); // (-6 + 2) >> 2 = -4 >> 2
        assert_eq!(forward_round2(1023, 11), 0); // rounds down
        assert_eq!(forward_round2(1024, 11), 1); // the half rounds up
        assert_eq!(forward_round2(-12345, 0), -12345); // identity
    }

    #[test]
    fn full_dct_random_residuals_roundtrip_within_bound_and_never_panic() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) % 511) as i32 - 255
        };
        for _ in 0..2000 {
            let mut residual = [0i32; DCT_DCT_4X4_COEFF_COUNT];
            for sample in &mut residual {
                *sample = next();
            }
            let block =
                ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), &residual).unwrap();
            let reconstructed = inverse_4x4_dct_dct(block.coefficients());
            for (&got, &want) in reconstructed.iter().zip(residual.iter()) {
                assert!((got - want).abs() <= 5, "residual {residual:?}");
            }
        }
    }

    #[test]
    fn full_dct_out_of_range_residual_errors_without_panicking() {
        let err = ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), &uniform(i32::MAX))
            .unwrap_err();
        assert!(matches!(
            err,
            Error::ForwardTransformCoefficientRangeExceeded {
                plane: PlaneId::Y,
                index: 0,
                ..
            }
        ));
    }

    #[test]
    fn full_dct_rejects_non_4x4_and_wrong_length() {
        assert!(matches!(
            ForwardTransformBlock::dct_dct_4x4(PlaneId::U, rect(2, 4), &[0; 16]).unwrap_err(),
            Error::ForwardTransformUnsupportedShape {
                plane: PlaneId::U,
                ..
            }
        ));
        assert!(matches!(
            ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), &[0; 15]).unwrap_err(),
            Error::ForwardTransformInputLengthMismatch {
                expected: 16,
                actual: 15,
                ..
            }
        ));
    }
}
