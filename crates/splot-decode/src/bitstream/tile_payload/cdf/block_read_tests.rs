// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use crate::bitstream::tile_payload::MvCdfSelector;

use super::super::FrameCdfSubset;
use super::*;

const PAYLOAD: [u8; 2] = [0x00, 0x80];
const SUPPORTED_BLOCK_SYMBOL_SELECTORS: &[TileCdfSelector] = &[
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
    TileCdfSelector::PaletteYMode,
    TileCdfSelector::IntrabcMode,
    TileCdfSelector::IntrabcPrecision,
    TileCdfSelector::MorphPred { ctx: 0 },
    TileCdfSelector::FscMode {
        ctx: 0,
        bsize_group: 0,
    },
    TileCdfSelector::DeltaQ,
    TileCdfSelector::IsWarp { ctx: 0 },
    TileCdfSelector::WarpMv,
    TileCdfSelector::WarpIdx { ctx: 0 },
    TileCdfSelector::WarpWithMvd,
    TileCdfSelector::WarpPrecision { block_size: 5 },
    TileCdfSelector::WarpDeltaParamLow { index_type: 0 },
    TileCdfSelector::WarpDeltaParamHigh { index_type: 1 },
    TileCdfSelector::WarpDeltaParamSign,
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
    TileCdfSelector::UsePcWiener,
    TileCdfSelector::FlexRestorationType { tool: 0, plane: 0 },
    TileCdfSelector::UseAmvd { index: 4, ctx: 0 },
    TileCdfSelector::ReadMv(MvCdfSelector::AmvdJoint),
    TileCdfSelector::ReadMv(MvCdfSelector::AmvdIndex { comp: 0 }),
    TileCdfSelector::UseBawp,
    TileCdfSelector::UseBawpChroma,
    TileCdfSelector::ExplicitBawp { ctx: 0 },
    TileCdfSelector::ExplicitBawpScale,
];

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
    let frame = FrameCdfSubset::from_defaults();

    for &selector in SUPPORTED_BLOCK_SYMBOL_SELECTORS {
        let mut direct_tile = frame.tile_copy();
        let mut helper_tile = frame.tile_copy();
        let mut direct = decoder(CdfUpdateMode::Enabled);
        let mut helper = decoder(CdfUpdateMode::Enabled);

        let expected = direct_tile
            .with_row_mut(selector, |row| direct.read_symbol_u16(row))
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
    let untouched = TileCdfSelector::DoSplit {
        plane_start: 0,
        ctx: 0,
    };

    for &selector in SUPPORTED_BLOCK_SYMBOL_SELECTORS {
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
