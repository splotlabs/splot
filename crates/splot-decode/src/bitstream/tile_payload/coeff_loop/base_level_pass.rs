// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Configuration, errors and parity summary of the ordinary non-FSC
//! coefficient base/level pass (see [`super::transform_block`]).
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use super::base_symbol::CoeffBaseSymbolReadError;
use super::max_level::{COEFF_BASE_RANGE, CoeffTransformClass, NUM_BASE_LEVELS};
use super::quant_state::next_tcq_state;

const PHTHRESH: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseDerivedLevelPassConfig {
    pub(crate) tx_size_ctx: usize,
    pub(crate) tx_width_log2: u32,
    pub(crate) tx_width: usize,
    pub(crate) tx_height: usize,
    pub(crate) plane: usize,
    pub(crate) tx_class: CoeffTransformClass,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
}

/// The base pass's cross-coefficient state packed into one register-sized
/// value: bits 0-2 `tcqState`, bit 3 the `sumAbs1` parity, bits 4-7 the
/// nonzero count capped below `is_hidden`'s threshold decision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CoeffBaseFirstPassSummary {
    packed: u32,
}

const SUMMARY_TCQ_MASK: u32 = 0b111;
const SUMMARY_PARITY_SHIFT: u32 = 3;
const SUMMARY_COUNT_SHIFT: u32 = 4;
const SUMMARY_COUNT_MASK: u32 = 0b1111;

impl CoeffBaseFirstPassSummary {
    #[must_use]
    pub(crate) const fn sum_abs1(self) -> u32 {
        (self.packed >> SUMMARY_PARITY_SHIFT) & 1
    }

    #[must_use]
    pub(crate) const fn is_hidden(self) -> bool {
        ((self.packed >> SUMMARY_COUNT_SHIFT) & SUMMARY_COUNT_MASK) >= PHTHRESH as u32
    }

    const fn tcq_state(self) -> usize {
        (self.packed & SUMMARY_TCQ_MASK) as usize
    }

    #[allow(clippy::inline_always, reason = "measured: per-coefficient update")]
    #[inline(always)]
    pub(crate) fn update_after_level<const TCQ: bool, const PH: bool>(
        &mut self,
        scan_index: usize,
        level: u32,
    ) {
        if TCQ {
            let tcq_state = next_tcq_state(self.tcq_state(), level);
            self.packed = (self.packed & !SUMMARY_TCQ_MASK) | tcq_state as u32;
        }
        if PH && level != 0 && scan_index > 0 {
            let clipped = level.min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
            let parity = ((self.packed >> SUMMARY_PARITY_SHIFT) & 1) ^ (clipped & 1);
            let count = ((self.packed >> SUMMARY_COUNT_SHIFT) & SUMMARY_COUNT_MASK)
                .min(SUMMARY_COUNT_MASK - 1);
            self.packed = (self.packed
                & !(SUMMARY_COUNT_MASK << SUMMARY_COUNT_SHIFT)
                & !(1 << SUMMARY_PARITY_SHIFT))
                | (parity << SUMMARY_PARITY_SHIFT)
                | ((count + 1) << SUMMARY_COUNT_SHIFT);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffBaseDerivedLevelPassError {
    #[error(
        "coefficient base/level config geometry {config_width}x{config_height} does not match block {block_width}x{block_height}"
    )]
    BlockGeometryMismatch {
        block_width: usize,
        block_height: usize,
        config_width: usize,
        config_height: usize,
    },
    #[error("coefficient base/level config cannot enable parity hiding and TCQ together")]
    InconsistentParityAndTcq,
    #[error(
        "coefficient base/level config enables parity hiding or TCQ outside their planes and classes"
    )]
    UnsupportedToolClass,
    #[error("coefficient base/level base symbol read failed: {0}")]
    Base(#[from] CoeffBaseSymbolReadError),
}

/// The `tx_class` index of the context tables (2D, horizontal, vertical).
pub(crate) const fn tx_class_index(tx_class: CoeffTransformClass) -> usize {
    match tx_class {
        CoeffTransformClass::TwoD => 0,
        CoeffTransformClass::Horizontal => 1,
        CoeffTransformClass::Vertical => 2,
    }
}
