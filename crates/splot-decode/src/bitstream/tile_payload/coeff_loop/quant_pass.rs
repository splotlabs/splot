// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient quant pass composition.
//!
//! Feature tracking: `DECODE-COEFF-QUANT-PASS-COMPOSE`.

use super::quant_state::CoeffQuantStateWriteError;
use super::read_quant::CoeffReadQuantError;
use super::scan_walk::CoeffScanEntry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantPassConfig {
    pub(crate) is_hidden: bool,
    pub(crate) sum_abs1: u32,
    pub(crate) use_tcq: bool,
    pub(crate) lossless: bool,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffQuantPassError {
    #[error("coefficient quant pass enabled hidden parity with TCQ or lossless facts")]
    InconsistentHiddenParityConfig { use_tcq: bool, lossless: bool },
    #[error("coefficient quant pass enabled TCQ for a lossless block")]
    InconsistentTcqConfig,
    #[error("coefficient quant pass input {index} skipped required hidden-parity sign")]
    HiddenParityMissingSign { index: usize, entry: CoeffScanEntry },
    #[error("coefficient quant pass read_quant failed: {0}")]
    ReadQuant(#[from] CoeffReadQuantError),
    #[error("coefficient quant pass write failed: {0}")]
    QuantState(#[from] CoeffQuantStateWriteError),
}

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
