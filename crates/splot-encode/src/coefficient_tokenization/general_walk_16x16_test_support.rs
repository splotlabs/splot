// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared § 8.2 test fixtures for the general 16x16 DCT_DCT luma walk test files
//! (`general_walk_16x16_tests.rs` and `general_walk_16x16_refine_tests.rs`). Both the
//! base-pass and the full-range refinement suites build `Quant[256]` blocks the same
//! way, so the scan-order and block-builder helpers live here once instead of being
//! duplicated byte-for-byte in each suite.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use splot_recon::{TransformClass, coefficient_scan_order};

/// The coefficient CDF q-context the general walk uses in these tests (q-ctx 0).
pub(super) const Q_CTX: usize = 0;
/// 16x16 DCT_DCT coefficient count (`Quant[256]`).
pub(super) const COEFF_COUNT: usize = 256;

/// Builds the 16x16 2D scan order (`scan[c]` = raster position of scan index `c`),
/// using the SAME `coefficient_scan_order` the tokenizer uses — never a hard-coded
/// raster table.
pub(super) fn scan_16x16() -> Vec<u16> {
    let mut scan = vec![0u16; COEFF_COUNT];
    coefficient_scan_order(16, 16, TransformClass::TwoD, &mut scan).unwrap();
    scan
}

/// Builds a signed raster `[i32; 256]` from a list of `(scan_index, magnitude)` pairs,
/// with an ASYMMETRIC, mixed-sign pattern: an even scan index is negative, an odd one
/// positive (so a swapped sign order cannot masquerade as a match — the
/// decode-verify-asymmetric-values lesson). A magnitude of 0 leaves the position zero.
pub(super) fn block_from(pairs: &[(usize, u32)]) -> [i32; COEFF_COUNT] {
    let scan = scan_16x16();
    let mut quant = [0i32; COEFF_COUNT];
    for &(c, mag) in pairs {
        if mag == 0 {
            continue;
        }
        let raster = scan[c] as usize;
        let value = if c % 2 == 0 {
            -(mag as i32)
        } else {
            mag as i32
        };
        quant[raster] = value;
    }
    quant
}
