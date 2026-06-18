// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient `maxLevel` derivation.
//!
//! Feature tracking: `DECODE-COEFF-MAX-LEVEL-DERIVE`.

use std::collections::TryReserveError;

use super::quant_pass::CoeffQuantPassInput;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

const NUM_BASE_LEVELS: u32 = 2;
const COEFF_BASE_RANGE: u32 = 3;
const LF_NUM_BASE_LEVELS: u32 = 4;

/// Caller-resolved transform class for ordinary coefficient syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffTransformClass {
    /// `TX_CLASS_2D`.
    TwoD,
    /// `TX_CLASS_HORIZ`.
    Horizontal,
    /// `TX_CLASS_VERT`.
    Vertical,
}

/// Block-level facts needed to derive §5.20.7.27 `maxLevel`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffMaxLevelConfig {
    /// Plane index, 0 for luma and greater than 0 for chroma.
    pub(crate) plane: usize,
    /// Caller-resolved `get_tx_class(PlaneTxType)` result.
    pub(crate) tx_class: CoeffTransformClass,
    /// Whether hidden parity is active for this transform block.
    pub(crate) is_hidden: bool,
}

/// Result of deriving `maxLevel` for one checked scan entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffMaxLevel {
    /// Checked scan entry this derivation belongs to.
    pub(crate) entry: CoeffScanEntry,
    /// Whether `get_lf_limits(row, col, txClass, plane)` is true.
    pub(crate) is_low_frequency: bool,
    /// Derived `maxLevel` consumed by §5.20.7.28 `read_quant`.
    pub(crate) max_level: u32,
}

impl CoeffMaxLevel {
    /// Converts this derivation into the input consumed by the quant pass.
    #[must_use]
    pub(crate) const fn quant_pass_input(self) -> CoeffQuantPassInput {
        CoeffQuantPassInput {
            entry: self.entry,
            max_level: self.max_level,
        }
    }
}

/// Error returned while deriving coefficient `maxLevel` records.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffMaxLevelError {
    /// Allocation for derived max-level records failed.
    #[error("coefficient maxLevel allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
}

/// Derives §5.20.7.27 `maxLevel` records for a checked scan walk.
///
/// The caller supplies transform class, plane, and hidden-parity facts. This
/// helper implements the spec's `get_lf_limits(row, col, txClass, plane)` and
/// `maxLevel` selection from
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`. It does not
/// derive transform class or hidden parity from real block syntax, read symbols,
/// mutate coefficient state, or invoke reconstruction.
pub(crate) fn derive_nonzero_coeff_max_levels(
    walk: &NonZeroCoeffScanWalk,
    config: CoeffMaxLevelConfig,
) -> Result<Vec<CoeffMaxLevel>, CoeffMaxLevelError> {
    let entries = walk.entries();
    let mut levels = Vec::new();
    levels.try_reserve(entries.len())?;
    for entry in entries.iter().copied() {
        levels.push(derive_coeff_max_level(entry, config));
    }
    Ok(levels)
}

/// Converts derived `maxLevel` records into quant-pass inputs.
pub(crate) fn max_levels_to_quant_pass_inputs(
    levels: &[CoeffMaxLevel],
) -> Result<Vec<CoeffQuantPassInput>, CoeffMaxLevelError> {
    let mut inputs = Vec::new();
    inputs.try_reserve(levels.len())?;
    inputs.extend(levels.iter().copied().map(CoeffMaxLevel::quant_pass_input));
    Ok(inputs)
}

fn derive_coeff_max_level(entry: CoeffScanEntry, config: CoeffMaxLevelConfig) -> CoeffMaxLevel {
    let is_low_frequency = get_lf_limits(entry, config);
    let mut max_level = if is_low_frequency {
        if config.plane == 0 {
            LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1
        } else {
            LF_NUM_BASE_LEVELS + 1
        }
    } else {
        NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1
    };
    if config.is_hidden && entry.scan_index() == 0 {
        max_level = NUM_BASE_LEVELS + 1;
    }

    CoeffMaxLevel {
        entry,
        is_low_frequency,
        max_level,
    }
}

fn get_lf_limits(entry: CoeffScanEntry, config: CoeffMaxLevelConfig) -> bool {
    let is_luma = config.plane == 0;
    match config.tx_class {
        CoeffTransformClass::TwoD => {
            let diagonal = entry.row().saturating_add(entry.col());
            if is_luma { diagonal < 4 } else { diagonal < 1 }
        }
        CoeffTransformClass::Horizontal => {
            if is_luma {
                entry.col() < 2
            } else {
                entry.col() < 1
            }
        }
        CoeffTransformClass::Vertical => {
            if is_luma {
                entry.row() < 2
            } else {
                entry.row() < 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(scan_index: usize, row: usize, col: usize) -> CoeffScanEntry {
        let pos = row.saturating_mul(8).saturating_add(col);
        CoeffScanEntry::for_test(scan_index, pos, row, col)
    }

    fn config(plane: usize, tx_class: CoeffTransformClass, is_hidden: bool) -> CoeffMaxLevelConfig {
        CoeffMaxLevelConfig {
            plane,
            tx_class,
            is_hidden,
        }
    }

    #[test]
    fn coefficient_max_level_derives_low_frequency_limits() {
        let cases = [
            (0, CoeffTransformClass::TwoD, entry(3, 0, 3), true, 8),
            (0, CoeffTransformClass::TwoD, entry(3, 2, 2), false, 6),
            (1, CoeffTransformClass::TwoD, entry(3, 0, 0), true, 5),
            (1, CoeffTransformClass::TwoD, entry(3, 0, 1), false, 6),
            (0, CoeffTransformClass::Horizontal, entry(3, 7, 1), true, 8),
            (0, CoeffTransformClass::Horizontal, entry(3, 0, 2), false, 6),
            (2, CoeffTransformClass::Horizontal, entry(3, 7, 0), true, 5),
            (2, CoeffTransformClass::Horizontal, entry(3, 0, 1), false, 6),
            (0, CoeffTransformClass::Vertical, entry(3, 1, 7), true, 8),
            (0, CoeffTransformClass::Vertical, entry(3, 2, 0), false, 6),
            (2, CoeffTransformClass::Vertical, entry(3, 0, 7), true, 5),
            (2, CoeffTransformClass::Vertical, entry(3, 1, 0), false, 6),
        ];

        for (plane, tx_class, entry, is_low_frequency, max_level) in cases {
            let derived = derive_coeff_max_level(entry, config(plane, tx_class, false));
            assert_eq!(derived.entry, entry);
            assert_eq!(derived.is_low_frequency, is_low_frequency);
            assert_eq!(derived.max_level, max_level);
        }
    }

    #[test]
    fn coefficient_max_level_hidden_final_scan_entry_overrides_limit() {
        let hidden =
            derive_coeff_max_level(entry(0, 31, 31), config(0, CoeffTransformClass::TwoD, true));
        assert!(!hidden.is_low_frequency);
        assert_eq!(hidden.max_level, 3);

        let not_final =
            derive_coeff_max_level(entry(1, 31, 31), config(0, CoeffTransformClass::TwoD, true));
        assert_eq!(not_final.max_level, 6);
    }

    #[test]
    fn coefficient_max_level_builds_quant_pass_inputs_in_walk_order()
    -> Result<(), CoeffMaxLevelError> {
        let entries = vec![entry(2, 0, 3), entry(1, 4, 4), entry(0, 0, 0)];
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(entries.clone());

        let levels =
            derive_nonzero_coeff_max_levels(&walk, config(0, CoeffTransformClass::TwoD, true))?;
        let inputs = max_levels_to_quant_pass_inputs(&levels)?;

        assert_eq!(levels.len(), 3);
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[0].entry, entries[0]);
        assert_eq!(inputs[0].max_level, 8);
        assert_eq!(inputs[1].entry, entries[1]);
        assert_eq!(inputs[1].max_level, 6);
        assert_eq!(inputs[2].entry, entries[2]);
        assert_eq!(inputs[2].max_level, 3);

        Ok(())
    }

    #[test]
    fn coefficient_max_level_pathological_coordinates_are_total() {
        let derived = derive_coeff_max_level(
            CoeffScanEntry::for_test(7, usize::MAX, usize::MAX, usize::MAX),
            config(0, CoeffTransformClass::TwoD, false),
        );

        assert!(!derived.is_low_frequency);
        assert_eq!(derived.max_level, 6);
    }
}
