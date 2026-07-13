// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient `maxLevel` derivation.
//!
//! Feature tracking: `DECODE-COEFF-MAX-LEVEL-DERIVE`.

use std::collections::TryReserveError;

pub(crate) use splot_core::coefficient::{COEFF_BASE_RANGE, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};

use super::quant_pass::CoeffQuantPassInput;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffMaxLevelPlaneTxTypeConfig {
    pub(crate) plane: usize,
    pub(crate) plane_tx_type: usize,
    pub(crate) is_hidden: bool,
}

impl CoeffMaxLevelPlaneTxTypeConfig {
    const fn max_level_config(self) -> CoeffMaxLevelConfig {
        CoeffMaxLevelConfig {
            plane: self.plane,
            tx_class: CoeffTransformClass::from_plane_tx_type(self.plane_tx_type),
            is_hidden: self.is_hidden,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffMaxLevel {
    pub(crate) entry: CoeffScanEntry,
    pub(crate) is_low_frequency: bool,
    pub(crate) max_level: u32,
}

impl CoeffMaxLevel {
    #[must_use]
    pub(crate) const fn quant_pass_input(self) -> CoeffQuantPassInput {
        CoeffQuantPassInput {
            entry: self.entry,
            max_level: self.max_level,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffMaxLevelError {
    #[error("coefficient maxLevel allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
}

pub(crate) fn derive_nonzero_coeff_max_levels(
    walk: &NonZeroCoeffScanWalk,
    config: CoeffMaxLevelConfig,
) -> Result<Vec<CoeffMaxLevel>, CoeffMaxLevelError> {
    let entries = walk.entries();
    let mut levels = Vec::new();
    levels.try_reserve(entries.len())?;
    levels.extend(
        entries
            .iter()
            .copied()
            .map(|entry| derive_coeff_max_level(entry, config)),
    );
    Ok(levels)
}

pub(crate) fn derive_nonzero_coeff_max_levels_from_plane_tx_type(
    walk: &NonZeroCoeffScanWalk,
    config: CoeffMaxLevelPlaneTxTypeConfig,
) -> Result<Vec<CoeffMaxLevel>, CoeffMaxLevelError> {
    derive_nonzero_coeff_max_levels(walk, config.max_level_config())
}

pub(crate) fn max_levels_to_quant_pass_inputs(
    levels: &[CoeffMaxLevel],
) -> Result<Vec<CoeffQuantPassInput>, CoeffMaxLevelError> {
    let mut inputs = Vec::new();
    inputs.try_reserve(levels.len())?;
    inputs.extend(levels.iter().copied().map(CoeffMaxLevel::quant_pass_input));
    Ok(inputs)
}

pub(crate) fn derive_coeff_max_level(
    entry: CoeffScanEntry,
    config: CoeffMaxLevelConfig,
) -> CoeffMaxLevel {
    let is_low_frequency = get_lf_limits(entry, config);
    let max_level = if config.is_hidden && entry.scan_index() == 0 {
        NUM_BASE_LEVELS + 1
    } else {
        match (is_low_frequency, config.plane == 0) {
            (true, true) => LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
            (true, false) => LF_NUM_BASE_LEVELS + 1,
            (false, _) => NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
        }
    };

    CoeffMaxLevel {
        entry,
        is_low_frequency,
        max_level,
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
