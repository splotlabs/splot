// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 coefficient-symbol context constants shared across crates.
//!
//! These are AV2 § 3 symbol constants (`03-symbols.md`) consumed by the § 8.3.2
//! `coeff_base` CDF-context derivation (`08-parsing-process.md#s-8-3-2`) and the
//! § 5.20.7.27 coefficient level / `coeff_br` syntax. They live
//! here, next to the bitstream model, so the `splot-decode` coefficient decode and
//! the `splot-encode` `coeff_base` tokenizer share a single
//! definition instead of each re-declaring a local copy. (The related § 9.2
//! `Sig_Ref_Diff_Offset` *table* itself lives in
//! [`crate::tables::conversion`](crate::tables::conversion::SIG_REF_DIFF_OFFSET).)

/// AV2 § 3 `SIG_REF_DIFF_OFFSET_NUM` (`03-symbols.md:753`, "Maximum number of
/// context samples to be used"): the number of luma `coeff_base`
/// significant-neighbour samples summed in the § 8.3.2 context derivation (chroma
/// instead uses 3 for the 2D transform class, 2 otherwise).
///
/// It equals the middle dimension of the generated § 9.2
/// [`SIG_REF_DIFF_OFFSET`](crate::tables::conversion::SIG_REF_DIFF_OFFSET) table
/// (`Sig_Ref_Diff_Offset[ 3 ][ SIG_REF_DIFF_OFFSET_NUM ][ 2 ]`); the
/// `sig_ref_diff_offset_num_matches_table_dimension` test guards against drift.
pub const SIG_REF_DIFF_OFFSET_NUM: usize = 5;

/// AV2 § 3 `LF_SIG_COEF_CONTEXTS_2D` (`03-symbols.md:430`, "Number of contexts for
/// 2d luma transform class"): the low-frequency luma 2D `coeff_base` context-band
/// base, used by the § 8.3.2 non-2D (horizontal/vertical transform class)
/// low-frequency branch.
pub const LF_SIG_COEF_CONTEXTS_2D: usize = 21;

/// AV2 § 3 `COEFF_BASE_RANGE` (`03-symbols.md:93`, "Number of values for
/// `coeff_br`"): the count of distinct `coeff_br` symbol values. A `coeff_br`
/// symbol is read once and adds `0..COEFF_BASE_RANGE` (i.e. `0..=2`) to a
/// coefficient's base level (§ 5.20.7.27, `level += coeff_br`); the § 8.3.2
/// `idtx_sign` context is raised when a current `Level` exceeds it.
pub const COEFF_BASE_RANGE: u32 = 3;

/// AV2 § 3 `LF_NUM_BASE_LEVELS` (`03-symbols.md:419`, "Base level threshold for low
/// frequency region"; `= LF_BASE_SYMBOLS - 2 = 4`): the low-frequency base-level
/// threshold. In § 5.20.7.27 a low-frequency coefficient reads `coeff_br` when its
/// level exceeds `LF_NUM_BASE_LEVELS` (the non-low-frequency threshold is the
/// distinct, decode-local `NUM_BASE_LEVELS = 2`, `03-symbols.md:585`).
pub const LF_NUM_BASE_LEVELS: u32 = 4;

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
