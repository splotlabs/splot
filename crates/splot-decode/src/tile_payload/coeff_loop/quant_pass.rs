// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient quant pass composition.
//!
//! Feature tracking: `DECODE-COEFF-QUANT-PASS-COMPOSE`.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::max_level::{
    CoeffMaxLevelConfig, CoeffMaxLevelError, CoeffTransformClass, derive_nonzero_coeff_max_levels,
    max_levels_to_quant_pass_inputs,
};
use super::quant_state::{
    CoeffQuantReadInput, CoeffQuantStateConfig, CoeffQuantStateWriteError, NonZeroCoeffQuantState,
    apply_nonzero_coeff_quant_state,
};
use super::read_quant::{
    CoeffReadQuant, CoeffReadQuantConfig, CoeffReadQuantError, CoeffReadQuantInput,
    read_nonzero_coeff_quants,
};
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};
use super::sign_symbol::{CoeffSignRead, CoeffSignReadSymbol};

/// Block-level facts for composing ordinary non-FSC quant reads and writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantPassConfig {
    /// Whether parity hiding is active for this transform block.
    pub(crate) is_hidden: bool,
    /// Caller-maintained `sumAbs1` parity accumulator.
    pub(crate) sum_abs1: u32,
    /// Whether TCQ is active for this transform block.
    pub(crate) use_tcq: bool,
    /// Whether the block is lossless.
    pub(crate) lossless: bool,
    /// Initial `hrLevelAvg` entering the `read_quant` pass.
    pub(crate) hr_level_avg: u32,
}

/// Block-level facts needed to derive quant-pass `maxLevel` inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantPassMaxLevelConfig {
    /// Plane index, 0 for luma and greater than 0 for chroma.
    pub(crate) plane: usize,
    /// Caller-resolved `get_tx_class(PlaneTxType)` result.
    pub(crate) tx_class: CoeffTransformClass,
}

/// Per-coefficient caller facts for ordinary non-FSC quant pass composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantPassInput {
    /// Checked scan entry this input belongs to.
    pub(crate) entry: CoeffScanEntry,
    /// Caller-derived `maxLevel` for this scan entry.
    pub(crate) max_level: u32,
}

/// Result of the composed ordinary non-FSC quant pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffQuantPass {
    read_quants: Vec<CoeffReadQuant>,
    quant_state: NonZeroCoeffQuantState,
}

impl NonZeroCoeffQuantPass {
    /// Builds a quant-pass summary from interleaved per-entry steps.
    pub(crate) fn from_interleaved_parts(
        read_quants: Vec<CoeffReadQuant>,
        quant_state: NonZeroCoeffQuantState,
    ) -> Self {
        Self {
            read_quants,
            quant_state,
        }
    }

    /// Raw `read_quant` results in scan-walk order.
    #[must_use]
    pub(crate) fn read_quants(&self) -> &[CoeffReadQuant] {
        &self.read_quants
    }

    /// Final quant-state summary after signed `Quant[]` writes.
    #[must_use]
    pub(crate) const fn quant_state(&self) -> &NonZeroCoeffQuantState {
        &self.quant_state
    }
}

/// Error returned by the composed ordinary non-FSC quant pass.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffQuantPassError {
    /// Hidden parity was enabled with TCQ or lossless facts, which §5.4.8 and
    /// §5.20.7.27 do not derive for a valid block.
    #[error("coefficient quant pass enabled hidden parity with TCQ or lossless facts")]
    InconsistentHiddenParityConfig {
        /// Caller-provided TCQ active flag.
        use_tcq: bool,
        /// Caller-provided lossless flag.
        lossless: bool,
    },
    /// TCQ was enabled for a lossless block, which cannot occur in §5.20.7.27.
    #[error("coefficient quant pass enabled TCQ for a lossless block")]
    InconsistentTcqConfig,
    /// The number of sign records did not match the checked scan walk.
    #[error("coefficient quant pass sign count {signs} does not match scan entries {entries}")]
    SignCountMismatch {
        /// Decoded sign record count.
        signs: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// The number of max-level inputs did not match the checked scan walk.
    #[error(
        "coefficient quant pass max-level input count {inputs} does not match scan entries {entries}"
    )]
    InputCountMismatch {
        /// Caller-provided max-level input count.
        inputs: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// One sign record was not paired with the matching checked scan entry.
    #[error(
        "coefficient quant pass sign {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    SignEntryMismatch {
        /// Input index.
        index: usize,
        /// Expected checked scan entry.
        expected: CoeffScanEntry,
        /// Actual sign-read entry.
        actual: CoeffScanEntry,
    },
    /// One max-level input was not paired with the matching checked scan entry.
    #[error(
        "coefficient quant pass input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    InputEntryMismatch {
        /// Input index.
        index: usize,
        /// Expected checked scan entry.
        expected: CoeffScanEntry,
        /// Caller-provided scan entry.
        actual: CoeffScanEntry,
    },
    /// A sign record was decoded against a different local level.
    #[error(
        "coefficient quant pass sign {index} carried level {actual}, expected local level {expected}"
    )]
    SignLevelMismatch {
        /// Input index.
        index: usize,
        /// Level read from local state.
        expected: u32,
        /// Level carried by the sign record.
        actual: u32,
    },
    /// Hidden parity required sign syntax for the final scan entry.
    #[error("coefficient quant pass input {index} skipped required hidden-parity sign")]
    HiddenParityMissingSign {
        /// Input index.
        index: usize,
        /// Checked scan entry.
        entry: CoeffScanEntry,
    },
    /// `maxLevel - useTcq` underflowed for caller-provided facts.
    #[error("coefficient quant pass input {index} has invalid maxLevel {max_level}")]
    InvalidMaxLevel {
        /// Input index.
        index: usize,
        /// Caller-provided max level.
        max_level: u32,
        /// Caller-provided TCQ active flag.
        use_tcq: bool,
    },
    /// The local transform-block state rejected a checked coordinate or position.
    #[error("coefficient quant pass state error: {0}")]
    State(#[from] TileCoeffStateError),
    /// Allocation for composed quant inputs failed.
    #[error("coefficient quant pass allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// Deriving `maxLevel` inputs failed.
    #[error("coefficient quant pass maxLevel derivation failed: {0}")]
    MaxLevel(#[from] CoeffMaxLevelError),
    /// The `read_quant` parser failed.
    #[error("coefficient quant pass read_quant failed: {0}")]
    ReadQuant(#[from] CoeffReadQuantError),
    /// The quant-state writer failed.
    #[error("coefficient quant pass write failed: {0}")]
    QuantState(#[from] CoeffQuantStateWriteError),
}

/// Runs the ordinary non-FSC `read_quant` and quant-state write pass.
///
/// This composes AV2 §5.20.7.28 `read_quant` literal parsing with the
/// §5.20.7.27 signed `Quant[]` write step. The caller still owns scan-table,
/// sign-source, `maxLevel`, hidden-parity, `sumAbs1`, TCQ, and lossless fact
/// derivation from real block syntax. Runtime `coeffs()` integration, tile
/// context writes, dequantization, and reconstruction remain out of scope.
pub(crate) fn apply_nonzero_coeff_quant_pass(
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    signs: &[CoeffSignRead],
    inputs: &[CoeffQuantPassInput],
    config: CoeffQuantPassConfig,
) -> Result<NonZeroCoeffQuantPass, CoeffQuantPassError> {
    validate_coeff_quant_pass_config(config)?;

    let read_inputs = preflight_quant_pass(block, walk.entries(), signs, inputs, config)?;
    let read_quants = read_nonzero_coeff_quants(
        symbols,
        walk,
        &read_inputs,
        CoeffReadQuantConfig {
            is_hidden: config.is_hidden,
            allow_tcq: config.use_tcq,
            hr_level_avg: config.hr_level_avg,
        },
    )?;
    let quant_inputs = quant_inputs_from_reads(&read_quants)?;
    let quant_state = apply_nonzero_coeff_quant_state(
        block,
        walk,
        signs,
        &quant_inputs,
        CoeffQuantStateConfig {
            is_hidden: config.is_hidden,
            sum_abs1: config.sum_abs1,
            use_tcq: config.use_tcq,
            lossless: config.lossless,
        },
    )?;

    Ok(NonZeroCoeffQuantPass {
        read_quants,
        quant_state,
    })
}

/// Validates block-level facts shared by batch and interleaved quant passes.
pub(crate) fn validate_coeff_quant_pass_config(
    config: CoeffQuantPassConfig,
) -> Result<(), CoeffQuantPassError> {
    if config.is_hidden && (config.use_tcq || config.lossless) {
        return Err(CoeffQuantPassError::InconsistentHiddenParityConfig {
            use_tcq: config.use_tcq,
            lossless: config.lossless,
        });
    }
    if config.lossless && config.use_tcq {
        return Err(CoeffQuantPassError::InconsistentTcqConfig);
    }
    Ok(())
}

/// Runs the ordinary non-FSC quant pass with derived `maxLevel` inputs.
///
/// This is the same loaded-but-unwired second-pass boundary as
/// [`apply_nonzero_coeff_quant_pass`], but it removes the per-coefficient
/// `maxLevel` caller fact by applying the AV2 §5.20.7.27 derivation over the
/// checked scan walk before invoking the quant pass. The caller still owns
/// transform-class derivation, sign-source selection, hidden-parity and TCQ
/// facts, runtime `coeffs()` integration, tile context writes, dequantization,
/// and reconstruction.
pub(crate) fn apply_nonzero_coeff_quant_pass_with_derived_max_levels(
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    signs: &[CoeffSignRead],
    max_level_config: CoeffQuantPassMaxLevelConfig,
    config: CoeffQuantPassConfig,
) -> Result<NonZeroCoeffQuantPass, CoeffQuantPassError> {
    let levels = derive_nonzero_coeff_max_levels(
        walk,
        CoeffMaxLevelConfig {
            plane: max_level_config.plane,
            tx_class: max_level_config.tx_class,
            is_hidden: config.is_hidden,
        },
    )?;
    let inputs = max_levels_to_quant_pass_inputs(&levels)?;
    apply_nonzero_coeff_quant_pass(symbols, block, walk, signs, &inputs, config)
}

fn preflight_quant_pass(
    block: &TransformCoeffBlockState,
    entries: &[CoeffScanEntry],
    signs: &[CoeffSignRead],
    inputs: &[CoeffQuantPassInput],
    config: CoeffQuantPassConfig,
) -> Result<Vec<CoeffReadQuantInput>, CoeffQuantPassError> {
    if signs.len() != entries.len() {
        return Err(CoeffQuantPassError::SignCountMismatch {
            signs: signs.len(),
            entries: entries.len(),
        });
    }
    if inputs.len() != entries.len() {
        return Err(CoeffQuantPassError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }

    let mut read_inputs = Vec::new();
    read_inputs.try_reserve(entries.len())?;
    for (index, ((entry, sign), input)) in entries
        .iter()
        .copied()
        .zip(signs.iter().copied())
        .zip(inputs.iter().copied())
        .enumerate()
    {
        if sign.entry() != entry {
            return Err(CoeffQuantPassError::SignEntryMismatch {
                index,
                expected: entry,
                actual: sign.entry(),
            });
        }
        if input.entry != entry {
            return Err(CoeffQuantPassError::InputEntryMismatch {
                index,
                expected: entry,
                actual: input.entry,
            });
        }

        let level = block.level_at(entry.row(), entry.col())?;
        if sign.level() != level {
            return Err(CoeffQuantPassError::SignLevelMismatch {
                index,
                expected: level,
                actual: sign.level(),
            });
        }
        if config.is_hidden
            && config.sum_abs1 > 0
            && entry.scan_index() == 0
            && sign.symbol() == CoeffSignReadSymbol::None
        {
            return Err(CoeffQuantPassError::HiddenParityMissingSign { index, entry });
        }
        input
            .max_level
            .checked_sub(u32::from(config.use_tcq))
            .ok_or(CoeffQuantPassError::InvalidMaxLevel {
                index,
                max_level: input.max_level,
                use_tcq: config.use_tcq,
            })?;
        block.quant_at(entry.pos())?;

        read_inputs.push(CoeffReadQuantInput {
            entry,
            level,
            max_level: input.max_level,
        });
    }
    Ok(read_inputs)
}

fn quant_inputs_from_reads(
    reads: &[CoeffReadQuant],
) -> Result<Vec<CoeffQuantReadInput>, CoeffQuantPassError> {
    let mut inputs = Vec::new();
    inputs.try_reserve(reads.len())?;
    inputs.extend(reads.iter().map(|read| read.quant_input()));
    Ok(inputs)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

    use super::super::read_quant::CoeffReadQuantPath;
    use super::*;

    fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap()
    }

    fn walk() -> NonZeroCoeffScanWalk {
        NonZeroCoeffScanWalk::from_entries_for_test(vec![
            CoeffScanEntry::for_test(1, 1, 0, 1),
            CoeffScanEntry::for_test(0, 0, 0, 0),
        ])
    }

    fn block_for(walk: &NonZeroCoeffScanWalk, levels: &[u32]) -> TransformCoeffBlockState {
        block_for_extent(2, 2, walk, levels)
    }

    fn block_for_extent(
        width: usize,
        height: usize,
        walk: &NonZeroCoeffScanWalk,
        levels: &[u32],
    ) -> TransformCoeffBlockState {
        let mut block = TransformCoeffBlockState::new(width, height).unwrap();
        for (entry, level) in walk.entries().iter().copied().zip(levels.iter().copied()) {
            block.set_level(entry.row(), entry.col(), level).unwrap();
            block.set_quant_sign(entry.row(), entry.col(), 11).unwrap();
        }
        block
    }

    fn signs_for(
        walk: &NonZeroCoeffScanWalk,
        levels: &[u32],
        signs: &[bool],
    ) -> Vec<CoeffSignRead> {
        walk.entries()
            .iter()
            .copied()
            .zip(levels.iter().copied())
            .zip(signs.iter().copied())
            .map(|((entry, level), sign)| {
                CoeffSignRead::for_test(
                    entry,
                    level,
                    CoeffSignReadSymbol::SignBit { bit: sign },
                    sign,
                )
            })
            .collect()
    }

    fn inputs_for(walk: &NonZeroCoeffScanWalk, max_levels: &[u32]) -> Vec<CoeffQuantPassInput> {
        walk.entries()
            .iter()
            .copied()
            .zip(max_levels.iter().copied())
            .map(|(entry, max_level)| CoeffQuantPassInput { entry, max_level })
            .collect()
    }

    fn config() -> CoeffQuantPassConfig {
        CoeffQuantPassConfig {
            is_hidden: false,
            sum_abs1: 0,
            use_tcq: false,
            lossless: false,
            hr_level_avg: 16,
        }
    }

    fn max_level_config() -> CoeffQuantPassMaxLevelConfig {
        CoeffQuantPassMaxLevelConfig {
            plane: 0,
            tx_class: CoeffTransformClass::TwoD,
        }
    }

    #[test]
    fn coefficient_quant_pass_reads_quant_and_writes_signed_quant() {
        let walk = walk();
        let levels = [3, 2];
        let signs = signs_for(&walk, &levels, &[false, true]);
        let inputs = inputs_for(&walk, &[3, 5]);
        let mut block = block_for(&walk, &levels);
        let quant_sign_before = block.quant_sign().to_vec();
        let mut symbols = symbol_decoder(&[0b0011_0100, 0x80]);

        let pass = apply_nonzero_coeff_quant_pass(
            &mut symbols,
            &mut block,
            &walk,
            &signs,
            &inputs,
            config(),
        )
        .unwrap();

        assert_eq!(pass.read_quants().len(), 2);
        assert_eq!(
            pass.read_quants()[0].path(),
            CoeffReadQuantPath::Extended {
                m: 4,
                k: 5,
                c_max: 6,
                q: 2,
                length: 4,
                x_base: 32,
                coeff_rem: 10,
                x: 42,
            }
        );
        assert_eq!(pass.read_quants()[0].quant_input().quant, 45);
        assert_eq!(
            pass.read_quants()[1].path(),
            CoeffReadQuantPath::BelowThreshold
        );
        assert_eq!(pass.read_quants()[1].quant_input().quant, 2);
        assert_eq!(pass.quant_state().hr_level_avg(), 29);
        assert_eq!(block.quant_at(walk.entries()[0].pos()).unwrap(), 45);
        assert_eq!(block.quant_at(walk.entries()[1].pos()).unwrap(), -2);
        assert_eq!(pass.quant_state().dc_category(), 1);
        assert_eq!(block.quant_sign(), quant_sign_before);
        assert_eq!(symbols.symbol_count(), 7);
    }

    #[test]
    fn coefficient_quant_pass_derives_low_frequency_max_levels() {
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![
            CoeffScanEntry::for_test(1, 1, 0, 1),
            CoeffScanEntry::for_test(0, 15, 3, 3),
        ]);
        let levels = [7, 5];
        let signs = signs_for(&walk, &levels, &[false, false]);
        let mut block = block_for_extent(4, 4, &walk, &levels);
        let mut symbols = symbol_decoder(&[0xff, 0x80]);
        let consumed_before = symbols.consumed_bits();

        let pass = apply_nonzero_coeff_quant_pass_with_derived_max_levels(
            &mut symbols,
            &mut block,
            &walk,
            &signs,
            max_level_config(),
            config(),
        )
        .unwrap();

        assert_eq!(pass.read_quants().len(), 2);
        assert_eq!(
            pass.read_quants()[0].path(),
            CoeffReadQuantPath::BelowThreshold
        );
        assert_eq!(
            pass.read_quants()[1].path(),
            CoeffReadQuantPath::BelowThreshold
        );
        assert_eq!(block.quant_at(1).unwrap(), 7);
        assert_eq!(block.quant_at(15).unwrap(), 5);
        assert_eq!(symbols.consumed_bits(), consumed_before);
    }

    #[test]
    fn coefficient_quant_pass_derives_hidden_final_max_level() {
        let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
        let levels = [3];
        let signs = signs_for(&walk, &levels, &[false]);
        let mut block = block_for(&walk, &levels);
        let mut symbols = symbol_decoder(&[0b1000_0000]);
        let config = CoeffQuantPassConfig {
            is_hidden: true,
            sum_abs1: 1,
            ..config()
        };

        let pass = apply_nonzero_coeff_quant_pass_with_derived_max_levels(
            &mut symbols,
            &mut block,
            &walk,
            &signs,
            max_level_config(),
            config,
        )
        .unwrap();

        assert_eq!(
            pass.read_quants()[0].path(),
            CoeffReadQuantPath::Extended {
                m: 3,
                k: 4,
                c_max: 6,
                q: 0,
                length: 3,
                x_base: 0,
                coeff_rem: 0,
                x: 0,
            }
        );
        assert_eq!(pass.read_quants()[0].quant_input().quant, 3);
        assert_eq!(block.quant_at(entry.pos()).unwrap(), 7);
        assert_eq!(symbols.symbol_count(), 4);
    }

    #[test]
    fn coefficient_quant_pass_applies_hidden_parity_consistently() {
        let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
        let levels = [2];
        let signs = signs_for(&walk, &levels, &[false]);
        let inputs = inputs_for(&walk, &[3]);
        let mut block = block_for(&walk, &levels);
        let mut symbols = symbol_decoder(&[0b1000_0100, 0x80]);
        let consumed_before = symbols.consumed_bits();
        let config = CoeffQuantPassConfig {
            is_hidden: true,
            sum_abs1: 1,
            use_tcq: false,
            lossless: false,
            hr_level_avg: 64,
        };

        let pass = apply_nonzero_coeff_quant_pass(
            &mut symbols,
            &mut block,
            &walk,
            &signs,
            &inputs,
            config,
        )
        .unwrap();

        assert_eq!(
            pass.read_quants()[0].path(),
            CoeffReadQuantPath::BelowThreshold
        );
        assert_eq!(pass.read_quants()[0].quant_input().quant, 2);
        assert_eq!(pass.read_quants()[0].quant_input().hr_level_avg, 64);
        assert_eq!(pass.quant_state().tcq_state(), 0);
        assert_eq!(pass.quant_state().cul_level(), 4);
        assert_eq!(pass.quant_state().dc_category(), 2);
        assert_eq!(block.quant_at(entry.pos()).unwrap(), 5);
        assert_eq!(symbols.consumed_bits(), consumed_before);
    }

    #[test]
    fn coefficient_quant_pass_applies_tcq_consistently() {
        let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
        let levels = [1];
        let signs = signs_for(&walk, &levels, &[false]);
        let inputs = inputs_for(&walk, &[3]);
        let mut block = block_for(&walk, &levels);
        let mut symbols = symbol_decoder(&[0xff, 0x80]);
        let consumed_before = symbols.consumed_bits();
        let config = CoeffQuantPassConfig {
            use_tcq: true,
            ..config()
        };

        let pass = apply_nonzero_coeff_quant_pass(
            &mut symbols,
            &mut block,
            &walk,
            &signs,
            &inputs,
            config,
        )
        .unwrap();

        assert_eq!(
            pass.read_quants()[0].path(),
            CoeffReadQuantPath::BelowThreshold
        );
        assert_eq!(pass.read_quants()[0].quant_input().quant, 1);
        assert_eq!(pass.quant_state().tcq_state(), 4);
        assert_eq!(pass.quant_state().cul_level(), 1);
        assert_eq!(pass.quant_state().dc_category(), 2);
        assert_eq!(block.quant_at(entry.pos()).unwrap(), 2);
        assert_eq!(symbols.consumed_bits(), consumed_before);
    }

    #[test]
    fn coefficient_quant_pass_allows_hidden_dc_without_parity_sign_when_sum_abs1_zero() {
        let entry = CoeffScanEntry::for_test(0, 0, 0, 0);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
        let levels = [0];
        let signs = vec![CoeffSignRead::for_test(
            entry,
            0,
            CoeffSignReadSymbol::None,
            false,
        )];
        let inputs = inputs_for(&walk, &[5]);
        let mut block = block_for(&walk, &levels);
        let mut symbols = symbol_decoder(&[0xff, 0x80]);
        let consumed_before = symbols.consumed_bits();
        let config = CoeffQuantPassConfig {
            is_hidden: true,
            sum_abs1: 0,
            ..config()
        };

        let pass = apply_nonzero_coeff_quant_pass(
            &mut symbols,
            &mut block,
            &walk,
            &signs,
            &inputs,
            config,
        )
        .unwrap();

        assert_eq!(
            pass.read_quants()[0].path(),
            CoeffReadQuantPath::BelowThreshold
        );
        assert_eq!(pass.read_quants()[0].quant_input().quant, 0);
        assert_eq!(pass.quant_state().cul_level(), 0);
        assert_eq!(pass.quant_state().dc_category(), 0);
        assert_eq!(block.quant_at(entry.pos()).unwrap(), 0);
        assert_eq!(symbols.consumed_bits(), consumed_before);
    }

    #[test]
    fn coefficient_quant_pass_derived_max_levels_rejects_bad_facts_before_consumption() {
        let walk = walk();
        let levels = [3, 2];
        let signs = signs_for(&walk, &levels, &[false, true]);
        let mut block = block_for(&walk, &levels);
        let before = block.clone();
        let mut symbols = symbol_decoder(&[0xff, 0x80]);
        let consumed_before = symbols.consumed_bits();
        let config = CoeffQuantPassConfig {
            is_hidden: true,
            use_tcq: true,
            ..config()
        };

        let err = apply_nonzero_coeff_quant_pass_with_derived_max_levels(
            &mut symbols,
            &mut block,
            &walk,
            &signs,
            max_level_config(),
            config,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffQuantPassError::InconsistentHiddenParityConfig {
                use_tcq: true,
                lossless: false,
            }
        ));
        assert_eq!(symbols.consumed_bits(), consumed_before);
        assert_eq!(block, before);
    }

    #[test]
    fn coefficient_quant_pass_rejects_bad_facts_before_consumption() {
        let walk = walk();
        let levels = [3, 2];
        let signs = signs_for(&walk, &levels, &[false, true]);
        let inputs = inputs_for(&walk, &[3, 5]);
        let block = block_for(&walk, &levels);

        let mut mismatch_block = block.clone();
        let mismatch_before = mismatch_block.clone();
        let mut mismatch_signs = signs.clone();
        mismatch_signs[0] = CoeffSignRead::for_test(
            walk.entries()[1],
            levels[0],
            CoeffSignReadSymbol::SignBit { bit: false },
            false,
        );
        let mut mismatch_symbols = symbol_decoder(&[0xff, 0x80]);
        let consumed_before = mismatch_symbols.consumed_bits();
        let err = apply_nonzero_coeff_quant_pass(
            &mut mismatch_symbols,
            &mut mismatch_block,
            &walk,
            &mismatch_signs,
            &inputs,
            config(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CoeffQuantPassError::SignEntryMismatch { index: 0, .. }
        ));
        assert_eq!(mismatch_symbols.consumed_bits(), consumed_before);
        assert_eq!(mismatch_block, mismatch_before);

        let mut max_block = block.clone();
        let max_before = max_block.clone();
        let mut max_symbols = symbol_decoder(&[0xff, 0x80]);
        let invalid_inputs = inputs_for(&walk, &[0, 5]);
        let invalid_config = CoeffQuantPassConfig {
            use_tcq: true,
            ..config()
        };
        let consumed_before = max_symbols.consumed_bits();
        let err = apply_nonzero_coeff_quant_pass(
            &mut max_symbols,
            &mut max_block,
            &walk,
            &signs,
            &invalid_inputs,
            invalid_config,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CoeffQuantPassError::InvalidMaxLevel {
                index: 0,
                max_level: 0,
                use_tcq: true,
            }
        ));
        assert_eq!(max_symbols.consumed_bits(), consumed_before);
        assert_eq!(max_block, max_before);

        let mut hidden_tcq_block = block.clone();
        let hidden_tcq_before = hidden_tcq_block.clone();
        let mut hidden_tcq_symbols = symbol_decoder(&[0xff, 0x80]);
        let hidden_tcq_config = CoeffQuantPassConfig {
            is_hidden: true,
            use_tcq: true,
            ..config()
        };
        let consumed_before = hidden_tcq_symbols.consumed_bits();
        let err = apply_nonzero_coeff_quant_pass(
            &mut hidden_tcq_symbols,
            &mut hidden_tcq_block,
            &walk,
            &signs,
            &inputs,
            hidden_tcq_config,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CoeffQuantPassError::InconsistentHiddenParityConfig {
                use_tcq: true,
                lossless: false,
            }
        ));
        assert_eq!(hidden_tcq_symbols.consumed_bits(), consumed_before);
        assert_eq!(hidden_tcq_block, hidden_tcq_before);

        let mut hidden_lossless_block = block.clone();
        let hidden_lossless_before = hidden_lossless_block.clone();
        let mut hidden_lossless_symbols = symbol_decoder(&[0xff, 0x80]);
        let hidden_lossless_config = CoeffQuantPassConfig {
            is_hidden: true,
            lossless: true,
            ..config()
        };
        let consumed_before = hidden_lossless_symbols.consumed_bits();
        let err = apply_nonzero_coeff_quant_pass(
            &mut hidden_lossless_symbols,
            &mut hidden_lossless_block,
            &walk,
            &signs,
            &inputs,
            hidden_lossless_config,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CoeffQuantPassError::InconsistentHiddenParityConfig {
                use_tcq: false,
                lossless: true,
            }
        ));
        assert_eq!(hidden_lossless_symbols.consumed_bits(), consumed_before);
        assert_eq!(hidden_lossless_block, hidden_lossless_before);
    }
}
