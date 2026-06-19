// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 coefficient-symbol context constants shared across crates.
//!
//! These are AV2 § 3 symbol constants (`03-symbols.md`) consumed by the § 8.3.2
//! `coeff_base` CDF-context derivation (`08-parsing-process.md#s-8-3-2`). They live
//! here, next to the bitstream model, so the `splot-decode` `coeff_base` context
//! derivation and the `splot-encode` `coeff_base` tokenizer share a single
//! definition instead of each re-declaring a local copy. The associated § 9.2
//! `Sig_Ref_Diff_Offset` *table* already lives in
//! [`crate::tables::conversion`](crate::tables::conversion::SIG_REF_DIFF_OFFSET);
//! these two scalars complete that table's vocabulary.

/// AV2 § 3 `SIG_REF_DIFF_OFFSET_NUM` (`03-symbols.md`): the number of luma
/// `coeff_base` significant-neighbour samples summed in the § 8.3.2 context
/// derivation (chroma instead uses 3 for the 2D transform class, 2 otherwise).
///
/// It equals the middle dimension of the generated § 9.2
/// [`SIG_REF_DIFF_OFFSET`](crate::tables::conversion::SIG_REF_DIFF_OFFSET) table
/// (`Sig_Ref_Diff_Offset[ 3 ][ SIG_REF_DIFF_OFFSET_NUM ][ 2 ]`); the
/// `sig_ref_diff_offset_num_matches_table_dimension` test guards against drift.
pub const SIG_REF_DIFF_OFFSET_NUM: usize = 5;

/// AV2 § 3 `LF_SIG_COEF_CONTEXTS_2D` (`03-symbols.md`): the low-frequency luma 2D
/// `coeff_base` context-band base, used by the § 8.3.2 non-2D (horizontal/vertical
/// transform class) low-frequency branch.
pub const LF_SIG_COEF_CONTEXTS_2D: usize = 21;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::conversion::SIG_REF_DIFF_OFFSET;

    /// `SIG_REF_DIFF_OFFSET_NUM` is, by definition, the middle dimension of the
    /// generated § 9.2 `SIG_REF_DIFF_OFFSET` table; assert the scalar and the table
    /// cannot drift apart (a regenerated table with a different neighbour count
    /// would fail here rather than silently desync the context derivation).
    #[test]
    fn sig_ref_diff_offset_num_matches_table_dimension() {
        assert_eq!(SIG_REF_DIFF_OFFSET_NUM, SIG_REF_DIFF_OFFSET[0].len());
        for class in &SIG_REF_DIFF_OFFSET {
            assert_eq!(class.len(), SIG_REF_DIFF_OFFSET_NUM);
        }
    }
}
