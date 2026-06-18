// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder forward-transform foundation.
//!
//! This module advances `ENC-FORWARD-TRANSFORM-FOUNDATION`. It is an
//! encoder-policy arithmetic stage, not a normative AV2 decoding process. The
//! current subset handles only 4x4 DCT_DCT DC-only blocks: a uniform residual
//! block maps to a coefficient block whose DC coefficient reconstructs exactly
//! through the existing `splot-recon` 4x4 DCT_DCT inverse path with a no-op
//! quant/dequant handoff.
//!
//! The module does not select transforms, emit syntax, own quantization, or
//! produce [`crate::Packet`] values.

#![allow(dead_code)]

use splot_recon::{PlaneId, PlaneRect};

use crate::error::{Error, Result};

const DCT_DCT_4X4_WIDTH: usize = 4;
const DCT_DCT_4X4_HEIGHT: usize = 4;
const DCT_DCT_4X4_COEFF_COUNT: usize = DCT_DCT_4X4_WIDTH * DCT_DCT_4X4_HEIGHT;
const DCT_DCT_4X4_DC_SCALE: i32 = 32;

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
        validate_4x4_shape(plane, block)?;
        if residual.len() != DCT_DCT_4X4_COEFF_COUNT {
            return Err(Error::ForwardTransformInputLengthMismatch {
                plane,
                block,
                expected: DCT_DCT_4X4_COEFF_COUNT,
                actual: residual.len(),
            });
        }

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

fn validate_4x4_shape(plane: PlaneId, block: PlaneRect) -> Result<()> {
    if block.width() == DCT_DCT_4X4_WIDTH && block.height() == DCT_DCT_4X4_HEIGHT {
        Ok(())
    } else {
        Err(Error::ForwardTransformUnsupportedShape {
            plane,
            block,
            expected_width: DCT_DCT_4X4_WIDTH,
            expected_height: DCT_DCT_4X4_HEIGHT,
        })
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
}
