// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal AV2 block-symbol `S()` reads for the traced runtime frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER`.

use splot_core::Error as CoreError;
use splot_core::symbol::{Symbol, SymbolDecoder};

use super::{TileCdfError, TileCdfSelector, TileCdfSubset};

/// Error returned by the crate-private block-symbol trace read boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BlockSymbolTraceReadError {
    /// The CDF selector was invalid for the supported tile CDF subset.
    #[error("block-symbol trace CDF selection failed: {0}")]
    Cdf(#[from] TileCdfError),
    /// AV2 § 8.2 symbol decoding rejected the selected row or tile payload state.
    #[error("block-symbol trace read failed: {0}")]
    Symbol(#[from] CoreError),
}

impl TileCdfSubset {
    /// Reads one traced AV2 § 5.20 block-symbol `S()` value.
    ///
    /// This helper is deliberately no broader than the current minimal flat
    /// intra trace. It validates a [`TileCdfSelector`] from the crate-private
    /// CDF subset and hands the selected row to the caller-owned symbol cursor.
    pub(crate) fn read_block_symbol_trace(
        &mut self,
        selector: TileCdfSelector,
        symbol_decoder: &mut SymbolDecoder<'_>,
    ) -> Result<Symbol, BlockSymbolTraceReadError> {
        self.with_row_mut(selector, |row| symbol_decoder.read_symbol(row))
            .map_err(BlockSymbolTraceReadError::Cdf)?
            .map_err(BlockSymbolTraceReadError::Symbol)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

    use super::super::FrameCdfSubset;
    use super::*;

    const PAYLOAD: [u8; 2] = [0x00, 0x80];

    fn decoder(mode: CdfUpdateMode) -> SymbolDecoder<'static> {
        SymbolDecoder::with_base_and_config(
            &PAYLOAD,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(mode),
        )
        .unwrap()
    }

    #[test]
    fn reads_supported_block_symbol_rows() {
        let selectors = [
            TileCdfSelector::YModeSet,
            TileCdfSelector::YModeIndex { ctx: 0 },
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx: 2,
                plane_type: 0,
                tx_size: 0,
                ctx: 0,
            },
            TileCdfSelector::UvModeCflNotAllowed { ctx: 0 },
            TileCdfSelector::IsCfl { ctx: 0 },
            TileCdfSelector::CflIndex,
            TileCdfSelector::CflSign,
            TileCdfSelector::CflAlpha { ctx: 0 },
            TileCdfSelector::CflMhccp,
            TileCdfSelector::CflMhDir { size_group: 0 },
            TileCdfSelector::IntrabcMode,
            TileCdfSelector::IntrabcPrecision,
            TileCdfSelector::FscMode {
                ctx: 0,
                bsize_group: 0,
            },
            TileCdfSelector::DeltaQ,
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx: 1,
                ctx: 3,
            },
            TileCdfSelector::IsLongSideDct { is_inter: 0 },
            TileCdfSelector::IntraTxTypeLong { tx_size_sqr: 2 },
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr: 0 },
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr: 1 },
            TileCdfSelector::SecTxType {
                is_inter: 0,
                tx_size_sqr: 2,
            },
            TileCdfSelector::MostProbableStxSet,
            TileCdfSelector::MostProbableStxSetAdst,
            TileCdfSelector::CctxType,
            TileCdfSelector::UseWienerNs,
        ];
        let frame = FrameCdfSubset::from_defaults();

        for selector in selectors {
            let mut direct_tile = frame.tile_copy();
            let mut helper_tile = frame.tile_copy();
            let mut direct = decoder(CdfUpdateMode::Enabled);
            let mut helper = decoder(CdfUpdateMode::Enabled);

            let expected = direct_tile
                .with_row_mut(selector, |row| direct.read_symbol(row))
                .unwrap()
                .unwrap();
            let actual = helper_tile
                .read_block_symbol_trace(selector, &mut helper)
                .unwrap();

            assert_eq!(actual, expected);
            assert_eq!(helper.consumed_bits(), direct.consumed_bits());
            assert_eq!(
                helper_tile.row(selector).unwrap(),
                direct_tile.row(selector).unwrap()
            );
        }
    }

    #[test]
    fn invalid_block_symbol_selector_fails_before_symbol_read() {
        let frame = FrameCdfSubset::from_defaults();
        let valid = TileCdfSelector::YModeSet;
        let invalid = TileCdfSelector::YModeIndex { ctx: 3 };
        let mut tile = frame.tile_copy();
        let before = tile.row(valid).unwrap().to_vec();
        let mut symbol = decoder(CdfUpdateMode::Enabled);
        let consumed_before = symbol.consumed_bits();

        let err = tile
            .read_block_symbol_trace(invalid, &mut symbol)
            .unwrap_err();

        assert!(matches!(
            err,
            BlockSymbolTraceReadError::Cdf(TileCdfError::SelectorOutOfRange {
                array: super::super::TileCdfArray::YModeIndex,
                index_name: "ctx",
                actual: 3,
                max_exclusive: 3,
            })
        ));
        assert_eq!(symbol.consumed_bits(), consumed_before);
        assert_eq!(tile.row(valid).unwrap(), before.as_slice());
    }

    #[test]
    fn update_mode_controls_only_selected_block_symbol_rows() {
        let frame = FrameCdfSubset::from_defaults();
        let selectors = [
            TileCdfSelector::YModeSet,
            TileCdfSelector::YModeIndex { ctx: 0 },
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx: 2,
                plane_type: 0,
                tx_size: 0,
                ctx: 0,
            },
            TileCdfSelector::UvModeCflNotAllowed { ctx: 0 },
            TileCdfSelector::IsCfl { ctx: 0 },
            TileCdfSelector::CflIndex,
            TileCdfSelector::CflSign,
            TileCdfSelector::CflAlpha { ctx: 0 },
            TileCdfSelector::CflMhccp,
            TileCdfSelector::CflMhDir { size_group: 0 },
            TileCdfSelector::IntrabcMode,
            TileCdfSelector::IntrabcPrecision,
            TileCdfSelector::FscMode {
                ctx: 0,
                bsize_group: 0,
            },
            TileCdfSelector::DeltaQ,
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx: 1,
                ctx: 3,
            },
            TileCdfSelector::IsLongSideDct { is_inter: 0 },
            TileCdfSelector::IntraTxTypeLong { tx_size_sqr: 2 },
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr: 0 },
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr: 1 },
            TileCdfSelector::SecTxType {
                is_inter: 0,
                tx_size_sqr: 2,
            },
            TileCdfSelector::MostProbableStxSet,
            TileCdfSelector::MostProbableStxSetAdst,
            TileCdfSelector::CctxType,
            TileCdfSelector::UseWienerNs,
        ];
        let untouched = TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        };

        for selector in selectors {
            let mut enabled = frame.tile_copy();
            let selected_before = enabled.row(selector).unwrap().to_vec();
            let untouched_before = enabled.row(untouched).unwrap().to_vec();
            let mut symbol = decoder(CdfUpdateMode::Enabled);
            let _ = enabled
                .read_block_symbol_trace(selector, &mut symbol)
                .unwrap();
            assert_ne!(enabled.row(selector).unwrap(), selected_before.as_slice());
            assert_eq!(enabled.row(untouched).unwrap(), untouched_before.as_slice());

            let mut disabled = frame.tile_copy();
            let selected_before = disabled.row(selector).unwrap().to_vec();
            let mut symbol = decoder(CdfUpdateMode::Disabled);
            let _ = disabled
                .read_block_symbol_trace(selector, &mut symbol)
                .unwrap();
            assert_eq!(disabled.row(selector).unwrap(), selected_before.as_slice());
        }
    }
}
