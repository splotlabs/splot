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
// The AV2 § 9 4-point DCT kernel (`Dct4` basis), from the generated single-source
// `splot-tables` § 9 tables (the same kernel the decoder inverse uses), so a
// generator/spec correction updates both directions at once and the forward kernel
// cannot drift from the decoder's. `splot-tables` is the dependency-free § 9 tables
// crate AGENTS.md § 2 permits any crate to depend on.
use splot_tables::tables::transform_1d::DCT_KERNEL4;

use crate::error::{Error, Result};

// Re-export the sibling 16x16 forward transform (`ENC-FORWARD-TRANSFORM-DCT-16X16`),
// split into `forward_transform_16x16` only to keep each source file under the
// project's 1000-line budget, so callers reach both block sizes through this module.
// The re-export is not yet consumed inside the crate (the 16x16 path has no production
// caller wired in this brick), so it is allowed-unused like the modules' `dead_code`.
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
// The column (second) pass must carry the single rounding; see FORWARD_ROW_SHIFT.
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
        validate_4x4_shape(plane, block)?;
        if residual.len() != DCT_DCT_4X4_COEFF_COUNT {
            return Err(Error::ForwardTransformInputLengthMismatch {
                plane,
                block,
                expected: DCT_DCT_4X4_COEFF_COUNT,
                actual: residual.len(),
            });
        }

        // Row pass (FORWARD_ROW_SHIFT): transform each row of 4 horizontally
        // adjacent residual samples into 4 frequencies. The intermediate is kept
        // in `i64` so the column pass sees full precision (no narrowing between
        // passes), keeping the transform total over the full `i32` residual domain.
        let mut intermediate = [0i64; DCT_DCT_4X4_COEFF_COUNT];
        for r in 0..DCT_DCT_4X4_HEIGHT {
            let mut row = [0i64; DCT_DCT_4X4_WIDTH];
            for (c, slot) in row.iter_mut().enumerate() {
                *slot = i64::from(residual[r * DCT_DCT_4X4_WIDTH + c]);
            }
            let transformed = forward_dct4_1d(&row, FORWARD_ROW_SHIFT);
            for (c, &value) in transformed.iter().enumerate() {
                intermediate[r * DCT_DCT_4X4_WIDTH + c] = value;
            }
        }

        // Column pass (FORWARD_COL_SHIFT): transform each column of the
        // intermediate, then narrow to the `i32` coefficient. The narrowing is
        // checked: an out-of-`i32` coefficient is unreachable for valid 8-bit
        // residuals but yields a typed error rather than a wrap for any input.
        let mut coefficients = [0; DCT_DCT_4X4_COEFF_COUNT];
        for c in 0..DCT_DCT_4X4_WIDTH {
            let mut column = [0i64; DCT_DCT_4X4_HEIGHT];
            for (r, slot) in column.iter_mut().enumerate() {
                *slot = intermediate[r * DCT_DCT_4X4_WIDTH + c];
            }
            let transformed = forward_dct4_1d(&column, FORWARD_COL_SHIFT);
            for (r, &value) in transformed.iter().enumerate() {
                let index = r * DCT_DCT_4X4_WIDTH + c;
                coefficients[index] = i32::try_from(value).map_err(|_| {
                    Error::ForwardTransformCoefficientRangeExceeded {
                        plane,
                        block,
                        index,
                        value,
                    }
                })?;
            }
        }

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

/// AV2 § 4.8 `Round2(value, n)`, identical to the `splot-recon` inverse transform's
/// `round2`: `n == 0` returns `value`, otherwise `(value + (1 << (n - 1))) >> n`
/// with an arithmetic (sign-extending) shift, the rounding add done in `i128` so it
/// is total for every `i64` value. Matching the decoder's rounding exactly is what
/// keeps the forward transform the precise numerical inverse of the inverse pass.
fn forward_round2(value: i64, shift: u32) -> i64 {
    if shift == 0 {
        return value;
    }
    ((i128::from(value) + (1i128 << (shift - 1))) >> shift) as i64
}

/// One forward 4-point DCT pass:
/// `out[r] = Round2(sum over i of DCT_KERNEL4[r][i] * input[i], shift)`.
///
/// The kernel ROW index `r` is the output frequency and the COLUMN index `i` is the
/// input sample — the transpose of the decoder inverse's `DCT_KERNEL4[j][i]`
/// indexing (`splot-recon` `inverse_transform.rs` `kernel_sum`), so this pass is the
/// analytic inverse of the 1D inverse DCT. The kernel matrix is asymmetric, so the
/// `[r][i]` orientation is load-bearing. The 4-tap sum accumulates in `i64`
/// (`|kernel| <= 83` times the column-pass intermediate stays well within `i64`).
fn forward_dct4_1d(input: &[i64; DCT_DCT_4X4_WIDTH], shift: u32) -> [i64; DCT_DCT_4X4_WIDTH] {
    let mut out = [0i64; DCT_DCT_4X4_WIDTH];
    for (r, slot) in out.iter_mut().enumerate() {
        let mut sum = 0i64;
        for (i, &sample) in input.iter().enumerate() {
            sum += i64::from(DCT_KERNEL4[r][i]) * sample;
        }
        *slot = forward_round2(sum, shift);
    }
    out
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

    // --- Full 4x4 DCT_DCT (`ENC-FORWARD-TRANSFORM-DCT-4X4`) ---

    #[test]
    fn full_dct_flat_residual_matches_dc_only_stub_and_reconstructs_exactly() {
        for v in [-50, -8, -1, 0, 1, 7, 40, 127] {
            let full =
                ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), &uniform(v)).unwrap();
            // The full DCT reproduces the DC-only stub on uniform input (DC = v*32,
            // every AC coefficient 0) — proving zero regression for the flat subset.
            assert_eq!(full.coefficients(), transform(v).coefficients(), "v {v}");
            let mut expected = [0; DCT_DCT_4X4_COEFF_COUNT];
            expected[0] = v * DCT_DCT_4X4_DC_SCALE;
            assert_eq!(full.coefficients(), &expected, "v {v}");
            // ...and that DC-only block reconstructs the uniform residual bit-exactly.
            assert_eq!(
                inverse_4x4_dct_dct(full.coefficients()),
                uniform(v),
                "v {v}"
            );
        }
    }

    #[test]
    fn full_dct_horizontal_ramp_pins_kernel_orientation() {
        // A horizontally-varying, vertically-constant residual ([0,1,2,3] in every
        // row) puts all its energy in the horizontal frequencies — coefficient row 0
        // — and zero in every vertical frequency. A transposed kernel would put the
        // energy in column 0 instead, so this pins the [r][i] orientation and the
        // exact Round2 / shift arithmetic with hand-computed values.
        let residual = [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3];
        let block = ForwardTransformBlock::dct_dct_4x4(PlaneId::Y, rect(4, 4), &residual).unwrap();
        assert_eq!(
            block.coefficients(),
            &[48, -35, 0, -3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn full_dct_nonuniform_roundtrips_within_residue_bound() {
        // Genuinely non-uniform residuals reconstruct through the decoder inverse
        // only within the non-orthogonality bound (NOT bit-exactly): the AV2 integer
        // DCT4 odd basis rows {83, 35} are not orthonormal.
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
        // Byte-identical to splot-recon inverse_transform.rs round2: arithmetic
        // shift, the half rounds toward +inf, and n == 0 is the identity.
        assert_eq!(forward_round2(6, 2), 2); // (6 + 2) >> 2
        assert_eq!(forward_round2(-6, 2), -1); // (-6 + 2) >> 2 = -4 >> 2
        assert_eq!(forward_round2(1023, 11), 0); // rounds down
        assert_eq!(forward_round2(1024, 11), 1); // the half rounds up
        assert_eq!(forward_round2(-12345, 0), -12345); // identity
    }

    // The shift budget (sum == 11) and the non-zero column shift are pinned at
    // compile time by the module-level `const _: () = assert!(...)` blocks, which
    // are stronger than a runtime test (a wrong split fails the build).

    #[test]
    fn full_dct_random_residuals_roundtrip_within_bound_and_never_panic() {
        // A deterministic LCG sweeps the valid 8-bit residual domain [-255, 255];
        // every block round-trips within the bound and the transform never panics.
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
        // A residual far outside the 8-bit domain yields a coefficient beyond i32;
        // the checked narrowing returns a typed error rather than panicking/wrapping.
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
