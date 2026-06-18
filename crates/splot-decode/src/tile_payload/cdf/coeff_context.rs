// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 coefficient-symbol CDF context derivation.
//!
//! This module derives the per-symbol `ctx` index that selects a coefficient
//! CDF row in the § 8.3.2 Cdf selection process
//! (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). It is the coefficient
//! counterpart of [`super::block_context`] (which derives the block-mode
//! contexts).
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.
//!
//! Scope: the two **position-only** coefficient base contexts — `coeff_base_eob`
//! (the end-of-block base context, keyed on the scan position relative to the
//! transform block size) and `coeff_base_bob` (the begin-of-block base context,
//! keyed on the begin position relative to the segment end-of-block). Both are
//! pure functions of caller-supplied scan/segment scalars and caller-resolved
//! transform-block geometry, so they need no `Level[]` magnitude buffer. The
//! `Level[]`-dependent coefficient contexts (`coeff_base`, `coeff_br`, the IDTX
//! variants) and the sign contexts (`dc_sign`, `idtx_sign`) are derived by
//! future increments once the per-transform-block level/sign buffers and the
//! § 5.20 coefficient decode loop exist. Nothing here is wired into a decode
//! path yet, so it is no-output-change.

/// AV2 § 3 `SIG_COEF_CONTEXTS_EOB`: the number of `coeff_base_eob` contexts
/// (`03-symbols.md`); the four contexts are `SIG_COEF_CONTEXTS_EOB - 4 ..=
/// SIG_COEF_CONTEXTS_EOB - 1`, i.e. `0..=3`.
const SIG_COEF_CONTEXTS_EOB: usize = 4;

/// Returns the AV2 § 8.3.2 `coeff_base_eob` CDF context for the scan position
/// `c` in a transform block of caller-resolved adjusted geometry
/// (`08-parsing-process.md#s-8-3-2`).
///
/// The context partitions the scan position by the adjusted transform block's
/// coefficient count `numCoeffs = height << bwl` (the spec's
/// `Tx_Height[adjTxSz] << Tx_Width_Log2[adjTxSz]`):
///
/// - `c == 0` → `SIG_COEF_CONTEXTS_EOB - 4` (`0`)
/// - `c <= numCoeffs / 8` → `SIG_COEF_CONTEXTS_EOB - 3` (`1`)
/// - `c <= numCoeffs / 4` → `SIG_COEF_CONTEXTS_EOB - 2` (`2`)
/// - otherwise → `SIG_COEF_CONTEXTS_EOB - 1` (`3`)
///
/// `bwl` is `Tx_Width_Log2[adjTxSz]` and `height` is `Tx_Height[adjTxSz]`, both
/// caller-resolved from the adjusted transform size (this module does not model
/// the § 9.2 conversion tables). The result is in `0..=3`. The `numCoeffs`
/// shift is computed total: an out-of-range `bwl` saturates to `usize::MAX`
/// rather than overflowing, so the function never panics.
pub(crate) const fn coeff_base_eob_ctx(c: usize, bwl: u32, height: usize) -> usize {
    // numCoeffs = height << bwl, computed without a panic on a bad shift width.
    let num_coeffs = match height.checked_shl(bwl) {
        Some(v) => v,
        None => usize::MAX,
    };
    if c == 0 {
        SIG_COEF_CONTEXTS_EOB - 4
    } else if c <= num_coeffs / 8 {
        SIG_COEF_CONTEXTS_EOB - 3
    } else if c <= num_coeffs / 4 {
        SIG_COEF_CONTEXTS_EOB - 2
    } else {
        SIG_COEF_CONTEXTS_EOB - 1
    }
}

/// Returns the AV2 § 8.3.2 `coeff_base_bob` CDF context for the begin-of-block
/// position `bob` relative to the segment end-of-block `seg_eob`
/// (`08-parsing-process.md#s-8-3-2`).
///
/// - `bob <= seg_eob >> 3` → `0`
/// - `bob <= seg_eob >> 2` → `1`
/// - otherwise → `2`
///
/// The result is in `0..=2`. Pure function of the two caller-supplied scalars;
/// total and panic-free.
pub(crate) const fn coeff_base_bob_ctx(bob: usize, seg_eob: usize) -> usize {
    if bob <= seg_eob >> 3 {
        0
    } else if bob <= seg_eob >> 2 {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A fixed PlaneTxType/position resolves at compile time (const fn).
    const _CONST_EOB_CHECK: () = assert!(coeff_base_eob_ctx(0, 5, 32) == 0);
    const _CONST_BOB_CHECK: () = assert!(coeff_base_bob_ctx(0, 64) == 0);

    #[test]
    fn coeff_base_eob_partitions_the_scan_position() {
        // TX_32X32 adjusted: bwl = Tx_Width_Log2 = 5, height = 32, so
        // numCoeffs = 32 << 5 = 1024; thresholds 1024/8 = 128 and 1024/4 = 256.
        let (bwl, height) = (5u32, 32usize);
        assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0, "c == 0");
        // 1 ..= 128 -> ctx 1.
        assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
        assert_eq!(
            coeff_base_eob_ctx(128, bwl, height),
            1,
            "boundary numCoeffs/8"
        );
        // 129 ..= 256 -> ctx 2.
        assert_eq!(coeff_base_eob_ctx(129, bwl, height), 2);
        assert_eq!(
            coeff_base_eob_ctx(256, bwl, height),
            2,
            "boundary numCoeffs/4"
        );
        // > 256 -> ctx 3.
        assert_eq!(coeff_base_eob_ctx(257, bwl, height), 3);
        assert_eq!(coeff_base_eob_ctx(1023, bwl, height), 3, "last position");
    }

    #[test]
    fn coeff_base_eob_smallest_block() {
        // TX_4X4 adjusted: bwl = 2, height = 4, numCoeffs = 4 << 2 = 16;
        // thresholds 16/8 = 2 and 16/4 = 4.
        let (bwl, height) = (2u32, 4usize);
        assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0);
        assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
        assert_eq!(coeff_base_eob_ctx(2, bwl, height), 1, "boundary 16/8");
        assert_eq!(coeff_base_eob_ctx(3, bwl, height), 2);
        assert_eq!(coeff_base_eob_ctx(4, bwl, height), 2, "boundary 16/4");
        assert_eq!(coeff_base_eob_ctx(5, bwl, height), 3);
    }

    #[test]
    fn coeff_base_eob_is_total_for_out_of_range_shift() {
        // A pathological bwl must not panic; numCoeffs saturates so any non-zero
        // c lands in ctx 1 (c <= usize::MAX / 8).
        assert_eq!(coeff_base_eob_ctx(0, u32::MAX, 32), 0);
        assert_eq!(coeff_base_eob_ctx(1, u32::MAX, 32), 1);
    }

    #[test]
    fn coeff_base_bob_partitions_the_begin_position() {
        // segEob = 64: thresholds 64 >> 3 = 8 and 64 >> 2 = 16.
        let seg_eob = 64usize;
        assert_eq!(coeff_base_bob_ctx(0, seg_eob), 0);
        assert_eq!(coeff_base_bob_ctx(8, seg_eob), 0, "boundary segEob>>3");
        assert_eq!(coeff_base_bob_ctx(9, seg_eob), 1);
        assert_eq!(coeff_base_bob_ctx(16, seg_eob), 1, "boundary segEob>>2");
        assert_eq!(coeff_base_bob_ctx(17, seg_eob), 2);
        assert_eq!(coeff_base_bob_ctx(64, seg_eob), 2, "bob == segEob");
    }

    #[test]
    fn coeff_base_bob_zero_segment_eob() {
        // segEob = 0: both thresholds are 0, so only bob == 0 takes ctx 0.
        assert_eq!(coeff_base_bob_ctx(0, 0), 0);
        assert_eq!(coeff_base_bob_ctx(1, 0), 2);
    }
}
