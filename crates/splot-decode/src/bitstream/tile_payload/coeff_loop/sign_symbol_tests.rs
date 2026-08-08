// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::super::super::cdf::FrameCdfSubset;
use super::*;

static ABOVE_DC: [u8; 4] = [2, 2, 0, 0];
static LEFT_DC: [u8; 4] = [0; 4];

fn source_config(
    plane: usize,
    tx_class: CoeffTransformClass,
    is_hidden: bool,
    sum_abs1: u32,
) -> CoeffSignSourceDeriveConfig<'static> {
    CoeffSignSourceDeriveConfig {
        coeff_cdf_q_ctx: 3,
        plane,
        plane_type: usize::from(plane > 0),
        tx_class,
        is_hidden,
        sum_abs1,
        above_dc: &ABOVE_DC,
        left_dc: &LEFT_DC,
        x4: 0,
        y4: 0,
        w4: 2,
        h4: 2,
    }
}

fn entry(scan_index: usize, row: usize, col: usize) -> CoeffScanEntry {
    CoeffScanEntry::new(scan_index, row * 8 + col, row, col)
}

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

#[test]
fn derives_dc_axis_hidden_and_zero_sources() {
    let dc = derive_nonzero_coeff_sign_input(
        entry(0, 0, 0),
        0,
        source_config(0, CoeffTransformClass::TwoD, true, 3),
    );
    assert_eq!(
        dc.source,
        CoeffSignReadSource::Cdf {
            syntax: CoeffSignCdfSyntax::DcSign,
            selector: CoeffDcSignSelector {
                coeff_cdf_q_ctx: 3,
                plane_type: 0,
                group: 1,
                ctx: 2,
            },
        }
    );

    let axis = derive_nonzero_coeff_sign_input(
        entry(2, 5, 0),
        1,
        source_config(0, CoeffTransformClass::Horizontal, false, 0),
    );
    assert!(matches!(
        axis.source,
        CoeffSignReadSource::Cdf {
            syntax: CoeffSignCdfSyntax::DcSignHorzVert,
            ..
        }
    ));

    let zero = derive_nonzero_coeff_sign_input(
        entry(3, 1, 1),
        0,
        source_config(0, CoeffTransformClass::TwoD, false, 0),
    );
    assert_eq!(zero.source, CoeffSignReadSource::None);
}

#[test]
fn reads_none_and_literal_sources_in_syntax_order() {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&[0xff, 0x80]);
    let before = symbols.consumed_bits();
    let none = read_preflighted_nonzero_coeff_sign(
        &mut tile,
        &mut symbols,
        CoeffSignReadInput {
            source: CoeffSignReadSource::None,
        },
    )
    .unwrap();
    assert_eq!(none.symbol(), CoeffSignReadSymbol::None);
    assert!(!none.sign());
    assert_eq!(symbols.consumed_bits(), before);

    let literal = read_preflighted_nonzero_coeff_sign(
        &mut tile,
        &mut symbols,
        CoeffSignReadInput {
            source: CoeffSignReadSource::SignBit,
        },
    )
    .unwrap();
    assert!(matches!(
        literal.symbol(),
        CoeffSignReadSymbol::SignBit { .. }
    ));
    assert!(symbols.consumed_bits() > before);
}
