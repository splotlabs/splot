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
//! It is split into a sibling module (re-exported from [`crate::forward_transform`])
//! purely to keep each source file under the project's 1000-line budget; the 4x4 and
//! 16x16 paths are independent.
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
// The AV2 § 9 16-point DCT kernel (`Dct16` basis), from the generated single-source
// `splot-tables` § 9 tables (the same kernel the decoder inverse uses), so a
// generator/spec correction updates both directions at once and the forward kernel
// cannot drift from the decoder's. `splot-tables` is the dependency-free § 9 tables
// crate AGENTS.md § 2 permits any crate to depend on.
use splot_tables::tables::transform_1d::DCT_KERNEL16;

use crate::error::{Error, Result};

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
// The column (second) pass must carry the single rounding; see FORWARD_ROW_SHIFT_16X16.
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
        validate_16x16_shape(plane, block)?;
        if residual.len() != DCT_DCT_16X16_COEFF_COUNT {
            return Err(Error::ForwardTransformInputLengthMismatch {
                plane,
                block,
                expected: DCT_DCT_16X16_COEFF_COUNT,
                actual: residual.len(),
            });
        }

        // Row pass (FORWARD_ROW_SHIFT_16X16): transform each row of 16 horizontally
        // adjacent residual samples into 16 frequencies. The intermediate is kept in
        // `i64` so the column pass sees full precision (no narrowing between passes).
        // The worst row-pass magnitude over the 8-bit domain is `1024 * 255 = 261120`,
        // far within `i64`.
        let mut intermediate = [0i64; DCT_DCT_16X16_COEFF_COUNT];
        for r in 0..DCT_DCT_16X16_HEIGHT {
            let mut row = [0i64; DCT_DCT_16X16_WIDTH];
            for (c, slot) in row.iter_mut().enumerate() {
                let Some(&sample) = residual.get(r * DCT_DCT_16X16_WIDTH + c) else {
                    return Err(Error::ForwardTransformInputLengthMismatch {
                        plane,
                        block,
                        expected: DCT_DCT_16X16_COEFF_COUNT,
                        actual: residual.len(),
                    });
                };
                *slot = i64::from(sample);
            }
            let transformed = forward_dct16_1d(&row, FORWARD_ROW_SHIFT_16X16);
            for (c, &value) in transformed.iter().enumerate() {
                intermediate[r * DCT_DCT_16X16_WIDTH + c] = value;
            }
        }

        // Column pass (FORWARD_COL_SHIFT_16X16): transform each column of the
        // intermediate, then narrow to the `i32` coefficient. The narrowing is
        // checked: an out-of-`i32` coefficient is unreachable for valid 8-bit
        // residuals but yields a typed error rather than a wrap for any input.
        let mut coefficients = [0; DCT_DCT_16X16_COEFF_COUNT];
        for c in 0..DCT_DCT_16X16_WIDTH {
            let mut column = [0i64; DCT_DCT_16X16_HEIGHT];
            for (r, slot) in column.iter_mut().enumerate() {
                *slot = intermediate[r * DCT_DCT_16X16_WIDTH + c];
            }
            let transformed = forward_dct16_1d(&column, FORWARD_COL_SHIFT_16X16);
            for (r, &value) in transformed.iter().enumerate() {
                let index = r * DCT_DCT_16X16_WIDTH + c;
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
    pub(crate) const fn coefficients(&self) -> &[i32; DCT_DCT_16X16_COEFF_COUNT] {
        &self.coefficients
    }
}

fn validate_16x16_shape(plane: PlaneId, block: PlaneRect) -> Result<()> {
    if block.width() == DCT_DCT_16X16_WIDTH && block.height() == DCT_DCT_16X16_HEIGHT {
        Ok(())
    } else {
        Err(Error::ForwardTransformUnsupportedShape {
            plane,
            block,
            expected_width: DCT_DCT_16X16_WIDTH,
            expected_height: DCT_DCT_16X16_HEIGHT,
        })
    }
}

/// AV2 § 4.8 `Round2(value, n)`, identical to the `splot-recon` inverse transform's
/// `round2` and the 4x4 forward `forward_round2`: `n == 0` returns `value`, otherwise
/// `(value + (1 << (n - 1))) >> n` with an arithmetic (sign-extending) shift, the
/// rounding add done in `i128` so it is total for every `i64` value. Matching the
/// decoder's rounding exactly keeps the forward transform the precise numerical
/// inverse of the inverse pass.
fn forward_round2(value: i64, shift: u32) -> i64 {
    if shift == 0 {
        return value;
    }
    ((i128::from(value) + (1i128 << (shift - 1))) >> shift) as i64
}

/// One forward 16-point DCT pass:
/// `out[r] = Round2(sum over i of DCT_KERNEL16[r][i] * input[i], shift)`.
///
/// The kernel ROW index `r` is the output frequency and the COLUMN index `i` is the
/// input sample — the transpose of the decoder inverse's `DCT_KERNEL16[j][i]`
/// indexing (`splot-recon` `inverse_transform.rs` `kernel_sum`), so this pass is the
/// analytic inverse of the 1D inverse DCT. The kernel matrix is asymmetric, so the
/// `[r][i]` orientation is load-bearing. The 16-tap sum accumulates in `i64`
/// (`|kernel| <= 90`; the largest pass product stays well within `i64`).
fn forward_dct16_1d(
    input: &[i64; DCT_DCT_16X16_WIDTH],
    shift: u32,
) -> [i64; DCT_DCT_16X16_WIDTH] {
    let mut out = [0i64; DCT_DCT_16X16_WIDTH];
    for (r, slot) in out.iter_mut().enumerate() {
        let mut sum = 0i64;
        for (i, &sample) in input.iter().enumerate() {
            sum += i64::from(DCT_KERNEL16[r][i]) * sample;
        }
        *slot = forward_round2(sum, shift);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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
        // A uniform residual `v` maps to a DC-only block (`coefficients[0] = v * 128`,
        // every AC coefficient 0) and reconstructs bit-exactly through the inverse.
        for v in [-127, -50, -8, -1, 0, 1, 7, 40, 127] {
            let block =
                ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::Y, rect(16, 16), &uniform(v))
                    .unwrap();
            let mut expected = [0; DCT_DCT_16X16_COEFF_COUNT];
            expected[0] = v * 128;
            assert_eq!(block.coefficients(), &expected, "v {v}");
            assert_eq!(inverse_16x16_dct_dct(block.coefficients()), uniform(v), "v {v}");
        }
    }

    #[test]
    fn horizontal_ramp_pins_kernel_orientation() {
        // A horizontally-varying, vertically-constant residual (0..16 repeated in
        // every row) puts all its energy in the horizontal frequencies — coefficient
        // row 0 — and zero in every vertical frequency. A transposed kernel would put
        // the energy in column 0 instead, so this pins the [r][i] orientation.
        let mut residual = [0i32; DCT_DCT_16X16_COEFF_COUNT];
        for r in 0..DCT_DCT_16X16_HEIGHT {
            for c in 0..DCT_DCT_16X16_WIDTH {
                residual[r * DCT_DCT_16X16_WIDTH + c] = c as i32;
            }
        }
        let block =
            ForwardTransformBlock16x16::dct_dct_16x16(PlaneId::Y, rect(16, 16), &residual).unwrap();
        let coeffs = block.coefficients();
        // All energy is in coefficient row 0 (the horizontal frequencies); every other
        // row is zero. A transposed kernel orientation would fail this.
        for r in 1..DCT_DCT_16X16_HEIGHT {
            for c in 0..DCT_DCT_16X16_WIDTH {
                assert_eq!(coeffs[r * DCT_DCT_16X16_WIDTH + c], 0, "row {r} col {c}");
            }
        }
        // Row 0, hand-computed with the exact Round2 / shift arithmetic. The DC is the
        // ramp mean energy: each row 0..=15 sums to 120; the row pass DC basis (`64`)
        // gives 7680 at shift 0, then the column pass over the vertically-constant
        // intermediate scales by the DC basis `1024` and rounds down 13 bits:
        // `7680 * 1024 >> 13 = 960`. The odd horizontal frequencies pick up the rest.
        assert_eq!(
            &coeffs[..DCT_DCT_16X16_WIDTH],
            &[960, -586, 0, -64, 0, -22, 0, -10, 0, -5, 0, -3, 0, -1, 0, -2]
        );
    }

    #[test]
    fn forward_round2_matches_recon_round2() {
        // Byte-identical to splot-recon inverse_transform.rs round2: arithmetic shift,
        // the half rounds toward +inf, and n == 0 is the identity.
        assert_eq!(forward_round2(6, 2), 2); // (6 + 2) >> 2
        assert_eq!(forward_round2(-6, 2), -1); // (-6 + 2) >> 2 = -4 >> 2
        assert_eq!(forward_round2(8191, 13), 1); // rounds up at the half
        assert_eq!(forward_round2(4095, 13), 0); // rounds down
        assert_eq!(forward_round2(-12345, 0), -12345); // identity
    }

    // The shift budget (sum == 13) and the non-zero column shift are pinned at compile
    // time by the module-level `const _: () = assert!(...)` blocks, which are stronger
    // than a runtime test (a wrong split fails the build).

    #[test]
    fn random_residuals_roundtrip_within_bound_and_never_panic() {
        // CLOSED-LOOP PROOF. A deterministic LCG sweeps the valid 8-bit residual
        // domain [-255, 255]; every 16x16 block forward-transforms then reconstructs
        // through the decoder § 7.15.4 inverse within |err| <= 5 and never panics.
        //
        // The bound 5 is the AV2 integer DCT16 non-orthogonality residue at the chosen
        // forward total shift 13 (= round-trip gain 2^32 minus the inverse total 19).
        // Neighboring totals fail this: 12 -> max error ~261, 14 -> ~129 (measured by
        // the same sweep). 13 collapses it to <= 5.
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
            let block = ForwardTransformBlock16x16::dct_dct_16x16(
                PlaneId::Y,
                rect(16, 16),
                &residual,
            )
            .unwrap();
            let reconstructed = inverse_16x16_dct_dct(block.coefficients());
            for (&got, &want) in reconstructed.iter().zip(residual.iter()) {
                let err = (got - want).abs();
                worst = worst.max(err);
                assert!(err <= BOUND, "residual {residual:?}: err {err} exceeds bound {BOUND}");
            }
        }
        // The sweep actually exercises AC content up to the bound (not a degenerate
        // all-zero pass): the worst observed error is non-trivial yet within bound.
        assert!(worst >= 1, "expected non-trivial AC residue, got {worst}");
    }

    #[test]
    fn out_of_range_residual_errors_without_panicking() {
        // A residual far outside the 8-bit domain yields a coefficient beyond i32; the
        // checked narrowing returns a typed error rather than panicking/wrapping.
        let err = ForwardTransformBlock16x16::dct_dct_16x16(
            PlaneId::Y,
            rect(16, 16),
            &uniform(i32::MAX),
        )
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
