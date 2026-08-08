// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient `maxLevel` derivation.
//!
//! Feature tracking: `DECODE-COEFF-MAX-LEVEL-DERIVE`.

pub(crate) use splot_core::coefficient::{COEFF_BASE_RANGE, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};

use super::scan_walk::CoeffScanEntry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffTransformClass {
    TwoD,
    Horizontal,
    Vertical,
}

impl CoeffTransformClass {
    #[must_use]
    pub(crate) const fn from_plane_tx_type(plane_tx_type: usize) -> Self {
        match plane_tx_type {
            10 | 12 | 14 => Self::Vertical,
            11 | 13 | 15 => Self::Horizontal,
            _ => Self::TwoD,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffMaxLevelConfig {
    pub(crate) plane: usize,
    pub(crate) tx_class: CoeffTransformClass,
    pub(crate) is_hidden: bool,
}

pub(crate) fn derive_coeff_max_level(entry: CoeffScanEntry, config: CoeffMaxLevelConfig) -> u32 {
    let is_low_frequency = get_lf_limits(entry, config);
    if config.is_hidden && entry.scan_index() == 0 {
        NUM_BASE_LEVELS + 1
    } else {
        match (is_low_frequency, config.plane == 0) {
            (true, true) => LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
            (true, false) => LF_NUM_BASE_LEVELS + 1,
            (false, _) => NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
        }
    }
}

pub(crate) fn coeff_is_low_frequency(
    entry: CoeffScanEntry,
    plane: usize,
    tx_class: CoeffTransformClass,
) -> bool {
    get_lf_limits(
        entry,
        CoeffMaxLevelConfig {
            plane,
            tx_class,
            is_hidden: false,
        },
    )
}

fn get_lf_limits(entry: CoeffScanEntry, config: CoeffMaxLevelConfig) -> bool {
    let is_luma = config.plane == 0;
    let coordinate = match config.tx_class {
        CoeffTransformClass::TwoD => entry.row().saturating_add(entry.col()),
        CoeffTransformClass::Horizontal => entry.col(),
        CoeffTransformClass::Vertical => entry.row(),
    };
    let limit = match (config.tx_class, is_luma) {
        (CoeffTransformClass::TwoD, true) => 4,
        (CoeffTransformClass::Horizontal | CoeffTransformClass::Vertical, true) => 2,
        (_, false) => 1,
    };
    coordinate < limit
}

#[cfg(test)]
#[path = "max_level_tests.rs"]
mod tests;
