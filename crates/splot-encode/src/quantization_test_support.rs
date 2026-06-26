// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared quantization test fixtures used by both the 4x4 (`quantization`) and 16x16
//! (`quantization_16x16`) quantizer test suites. The independent per-coefficient level
//! reference lives here once instead of being duplicated byte-for-byte in each suite.

/// Independent round-to-nearest level for one coefficient at quantizer `q`
/// (`dq_denom == 1`): `round(|c| * 8 / q)` with sign, mirroring `quantize_coefficient`
/// so the per-coefficient quantizer selection is cross-checked rather than re-derived
/// through the production path.
pub(crate) fn expected_level(coeff: i32, q: u32) -> i32 {
    if coeff == 0 {
        return 0;
    }
    let numerator = u64::from(coeff.unsigned_abs()) * 8;
    let level = ((numerator + u64::from(q) / 2) / u64::from(q)) as i32;
    if coeff < 0 { -level } else { level }
}
