// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient `maxLevel` derivation.
//!
//! Feature tracking: `DECODE-COEFF-MAX-LEVEL-DERIVE`.

pub(crate) use splot_core::coefficient::{COEFF_BASE_RANGE, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};

use splot_recon::TransformClass;

use super::scan_walk::CoeffScanEntry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffMaxLevelConfig {
    pub(crate) plane: usize,
    pub(crate) tx_class: TransformClass,
    pub(crate) is_hidden: bool,
}

#[inline]
pub(crate) fn derive_coeff_max_level(entry: CoeffScanEntry, config: CoeffMaxLevelConfig) -> u32 {
    let is_low_frequency = coeff_is_low_frequency(entry, config.plane, config.tx_class);
    if config.is_hidden && entry.scan_index() == 0 {
        NUM_BASE_LEVELS + 1
    } else {
        visible_max_level(is_low_frequency, config.plane)
    }
}

/// `maxLevel` of a coefficient whose level parity is not hidden.
#[inline]
pub(crate) const fn visible_max_level(is_low_frequency: bool, plane: usize) -> u32 {
    match (is_low_frequency, plane == 0) {
        (true, true) => LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
        (true, false) => LF_NUM_BASE_LEVELS + 1,
        (false, _) => NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
    }
}

#[inline]
pub(crate) fn coeff_is_low_frequency(
    entry: CoeffScanEntry,
    plane: usize,
    tx_class: TransformClass,
) -> bool {
    let is_luma = plane == 0;
    let coordinate = match tx_class {
        TransformClass::TwoD => entry.row().saturating_add(entry.col()),
        TransformClass::Horizontal => entry.col(),
        TransformClass::Vertical => entry.row(),
    };
    let limit = match (tx_class, is_luma) {
        (TransformClass::TwoD, true) => 4,
        (TransformClass::Horizontal | TransformClass::Vertical, true) => 2,
        (_, false) => 1,
    };
    coordinate < limit
}

#[cfg(test)]
#[path = "max_level_tests.rs"]
mod tests;
