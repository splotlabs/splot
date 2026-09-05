// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 coefficient-symbol context constants shared across crates.
//!
//! AV2 § 3 (`docs/spec/av2/1.0.0/03-symbols.md`), used by § 8.3.2
//! context derivation and § 5.20.7.27 coefficient-level syntax.

/// Number of luma significant-neighbour samples in § 8.3.2 context derivation.
/// Matches the middle dimension of the § 9.2
/// [`SIG_REF_DIFF_OFFSET`](crate::tables::conversion::SIG_REF_DIFF_OFFSET) table.
pub const SIG_REF_DIFF_OFFSET_NUM: usize = 5;

/// Low-frequency luma 2D context-band base, used by the § 8.3.2 non-2D branch.
pub const LF_SIG_COEF_CONTEXTS_2D: usize = 21;

/// Number of `coeff_br` symbols; each adds `0..=2` to the base level (§ 5.20.7.27).
pub const COEFF_BASE_RANGE: u32 = 3;

/// Low-frequency base-level threshold for reading `coeff_br` (§ 5.20.7.27).
pub const LF_NUM_BASE_LEVELS: u32 = 4;

/// High-frequency base-level threshold for reading `coeff_br` (§ 5.20.7.27),
/// distinct from [`LF_NUM_BASE_LEVELS`].
pub const NUM_BASE_LEVELS: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::conversion::SIG_REF_DIFF_OFFSET;

    #[test]
    fn sig_ref_diff_offset_num_matches_table_dimension() {
        assert_eq!(SIG_REF_DIFF_OFFSET_NUM, SIG_REF_DIFF_OFFSET[0].len());
        for class in &SIG_REF_DIFF_OFFSET {
            assert_eq!(class.len(), SIG_REF_DIFF_OFFSET_NUM);
        }
    }
}
