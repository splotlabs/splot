// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop foundation helpers.

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    ADJUSTED_TX_SIZE, TX_HEIGHT, TX_HEIGHT_LOG2, TX_SIZE_SQR, TX_SIZE_SQR_UP, TX_WIDTH,
    TX_WIDTH_LOG2,
};

use super::cdf::block_read::{BlockSymbolTraceReadError, read_row, row_at};
use super::cdf::{EobPtSize, TileCdfSubset};
use super::coeff_state::TileCoeffStateError;

const EOB_MULTISIZE_OFFSET: usize = 4;
const MAX_NONZERO_EOB_PT: usize = 11;
const TX_SIZES: usize = 25;

/// The coefficient geometry of one `TxSize`: its 4x4 extent, the
/// `Adjusted_Tx_Size` coefficient extent (§ 7.13.3), the `all_zero` and
/// level transform-size context `(Tx_Size_Sqr + Tx_Size_Sqr_Up + 1) >> 1`
/// and the EOB symbol size (§ 5.20.7.27).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TxGeometry {
    pub(crate) scan_offset: u16,
    pub(crate) w4: u8,
    pub(crate) h4: u8,
    pub(crate) width: u8,
    pub(crate) height: u8,
    pub(crate) width_log2: u8,
    pub(crate) height_log2: u8,
    pub(crate) size_ctx: u8,
    pub(crate) eob_size: EobPtSize,
}

/// [`TxGeometry`] by `TxSize`.
pub(crate) static TX_GEOMETRY: [TxGeometry; TX_SIZES] = {
    let mut out = [TxGeometry {
        scan_offset: 0,
        w4: 0,
        h4: 0,
        width: 0,
        height: 0,
        width_log2: 0,
        height_log2: 0,
        size_ctx: 0,
        eob_size: EobPtSize::Pt16,
    }; TX_SIZES];
    let mut tx_size = 0;
    while tx_size < TX_SIZES {
        let adjusted = ADJUSTED_TX_SIZE[tx_size] as usize;
        let (width_log2, height_log2) = (TX_WIDTH_LOG2[adjusted], TX_HEIGHT_LOG2[adjusted]);
        out[tx_size] = TxGeometry {
            scan_offset: transform_block::scan_shape_offset(
                width_log2 as usize - 2,
                height_log2 as usize - 2,
            ) as u16,
            w4: (TX_WIDTH[tx_size] >> 2) as u8,
            h4: (TX_HEIGHT[tx_size] >> 2) as u8,
            width: TX_WIDTH[adjusted] as u8,
            height: TX_HEIGHT[adjusted] as u8,
            width_log2: width_log2 as u8,
            height_log2: height_log2 as u8,
            size_ctx: ((TX_SIZE_SQR[tx_size] + TX_SIZE_SQR_UP[tx_size] + 1) >> 1) as u8,
            eob_size: eob_pt_size(
                width_log2 as usize + height_log2 as usize - EOB_MULTISIZE_OFFSET,
            ),
        };
        tx_size += 1;
    }
    out
};
pub(crate) mod base_level_pass;
pub(crate) mod base_symbol;
mod branch;
pub(crate) use branch::read_nonzero_coeff_block_start;
pub(crate) mod fsc_block;
pub(crate) mod max_level;
pub(crate) mod ordinary_pass;
pub(crate) mod quant_pass;
pub(crate) mod quant_state;
pub(crate) mod read_quant;
pub(crate) mod scan_walk;
pub(crate) mod sign_symbol;
pub(crate) mod transform_block;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffParseErrorClass {
    Allocation,
    Malformed { spec_section: &'static str },
    EntropyState,
    CoefficientState,
}

pub(crate) fn classify_coeff_parse_error(
    error: &(dyn std::error::Error + 'static),
    syntax_spec_section: &'static str,
) -> CoeffParseErrorClass {
    let mut source = Some(error);
    let mut source_spec_section = syntax_spec_section;
    while let Some(current) = source {
        if current
            .downcast_ref::<std::collections::TryReserveError>()
            .is_some()
        {
            return CoeffParseErrorClass::Allocation;
        }
        if matches!(
            current.downcast_ref::<CoeffLoopContextError>(),
            Some(CoeffLoopContextError::InvalidPt512EobExtra { .. })
        ) {
            return CoeffParseErrorClass::Malformed {
                spec_section: "6.19.7.23",
            };
        }
        if matches!(
            current.downcast_ref::<quant_state::CoeffQuantStateWriteError>(),
            Some(quant_state::CoeffQuantStateWriteError::QuantMagnitudeOutOfRange { .. })
        ) {
            return CoeffParseErrorClass::Malformed {
                spec_section: "6.19.7.23",
            };
        }
        if let Some(read_quant_error) = current.downcast_ref::<read_quant::CoeffReadQuantError>() {
            source_spec_section = "5.20.7.28";
            if matches!(
                read_quant_error,
                read_quant::CoeffReadQuantError::OverlongGolombPrefix { .. }
            ) {
                return CoeffParseErrorClass::Malformed {
                    spec_section: "6.19.7.24",
                };
            }
        }
        if matches!(
            current.downcast_ref::<BlockSymbolTraceReadError>(),
            Some(BlockSymbolTraceReadError::Cdf(_))
        ) {
            return CoeffParseErrorClass::EntropyState;
        }
        if let Some(core_error) = current.downcast_ref::<CoreError>() {
            return if matches!(core_error, CoreError::UnexpectedEof { .. }) {
                CoeffParseErrorClass::Malformed {
                    spec_section: source_spec_section,
                }
            } else {
                CoeffParseErrorClass::EntropyState
            };
        }
        source = current.source();
    }
    CoeffParseErrorClass::CoefficientState
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobSymbolInput {
    pub(crate) size: EobPtSize,
    pub(crate) eob_ctx: usize,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffLoopContextError {
    #[error("coefficient context state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error("coefficient EOB literal read failed for {syntax}: {source}")]
    EobLiteralRead {
        syntax: &'static str,
        #[source]
        source: CoreError,
    },
    #[error("coefficient EOB point {eob_pt} is outside the supported AV2 range 1..=11")]
    InvalidEobPoint { eob_pt: usize },
    #[error("coefficient EOB Pt512 extra value {eob_pt_extra} is reserved by AV2")]
    InvalidPt512EobExtra { eob_pt_extra: u32 },
    #[error("coefficient scan walk requires nonzero EOB, got {eob}")]
    InvalidScanWalkEob { eob: usize },
    #[error("coefficient scan walk EOB {eob} exceeds scan length {scan_len}")]
    ScanWalkEobOutOfRange { eob: usize, scan_len: usize },
    #[error("coefficient FSC scan walk EOB {eob} exceeds segment EOB {seg_eob}")]
    FscScanWalkEobOutOfRange { eob: usize, seg_eob: usize },
}

pub(crate) fn read_nonzero_coeff_eob(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffEobSymbolInput,
) -> Result<usize, CoeffLoopContextError> {
    let rows = cdfs.block_rows_mut();
    let ctx = input.eob_ctx;
    #[cfg(feature = "block-evidence")]
    if symbols.observes_entropy() {
        symbols.annotate_entropy(splot_core::symbol::EntropySyntax::new(
            "eob_pt",
            "eob_multi_size,eob_ctx",
            [input.size as u16, ctx as u16, 0],
        ));
    }
    let eob_pt_symbol = match input.size {
        EobPtSize::Pt16 => read_row(symbols, row_at(rows.eob_pt_16(), ctx)),
        EobPtSize::Pt32 => read_row(symbols, row_at(rows.eob_pt_32(), ctx)),
        EobPtSize::Pt64 => read_row(symbols, row_at(rows.eob_pt_64(), ctx)),
        EobPtSize::Pt128 => read_row(symbols, row_at(rows.eob_pt_128(), ctx)),
        EobPtSize::Pt256 => read_row(symbols, row_at(rows.eob_pt_256(), ctx)),
        EobPtSize::Pt512 => read_row(symbols, row_at(rows.eob_pt_512(), ctx)),
        EobPtSize::Pt1024 => read_row(symbols, row_at(rows.eob_pt_1024(), ctx)),
    } as u8;
    let eob_pt_extra_width = eob_pt_extra_width(input.size, eob_pt_symbol);
    let eob_pt_extra = read_eob_literal(symbols, eob_pt_extra_width, "eob_pt_extra")?;
    let eob_pt =
        checked_resolved_eob_pt(input.size, eob_pt_symbol, eob_pt_extra_width, eob_pt_extra)?;

    if eob_pt < 3 {
        return Ok(eob_pt);
    }
    #[cfg(feature = "block-evidence")]
    if symbols.observes_entropy() {
        symbols.annotate_entropy(splot_core::symbol::EntropySyntax::new(
            "eob_extra",
            "",
            [0; 3],
        ));
    }
    let eob_extra = read_row(symbols, cdfs.block_rows_mut().eob_extra()) != 0;
    if eob_pt > MAX_NONZERO_EOB_PT {
        return Err(CoeffLoopContextError::InvalidEobPoint { eob_pt });
    }
    let width = eob_pt - 3;
    let eob_extra_bits = read_eob_literal(symbols, width as u32, "eob_extra_bit")? as usize;
    Ok(1 + ((2 + usize::from(eob_extra)) << width) + eob_extra_bits)
}

const fn eob_pt_size(eob_multisize: usize) -> EobPtSize {
    match eob_multisize {
        0 => EobPtSize::Pt16,
        1 => EobPtSize::Pt32,
        2 => EobPtSize::Pt64,
        3 => EobPtSize::Pt128,
        4 => EobPtSize::Pt256,
        5 => EobPtSize::Pt512,
        _ => EobPtSize::Pt1024,
    }
}

pub(crate) fn eob_context(plane: usize, is_inter: bool) -> usize {
    if plane > 0 { 2 } else { usize::from(is_inter) }
}

fn eob_pt_extra_width(size: EobPtSize, eob_pt_symbol: u8) -> u32 {
    match (size, eob_pt_symbol) {
        (EobPtSize::Pt256, 7) => 1,
        (EobPtSize::Pt512 | EobPtSize::Pt1024, 7) => 2,
        _ => 0,
    }
}

fn resolved_eob_pt(eob_pt_symbol: u8, eob_pt_extra_width: u32, eob_pt_extra: u32) -> usize {
    if eob_pt_extra_width == 0 {
        usize::from(eob_pt_symbol) + 1
    } else {
        8 + eob_pt_extra as usize
    }
}

fn checked_resolved_eob_pt(
    size: EobPtSize,
    eob_pt_symbol: u8,
    eob_pt_extra_width: u32,
    eob_pt_extra: u32,
) -> Result<usize, CoeffLoopContextError> {
    if matches!(size, EobPtSize::Pt512) && eob_pt_extra_width != 0 && eob_pt_extra == 3 {
        return Err(CoeffLoopContextError::InvalidPt512EobExtra { eob_pt_extra });
    }
    Ok(resolved_eob_pt(
        eob_pt_symbol,
        eob_pt_extra_width,
        eob_pt_extra,
    ))
}

#[expect(
    clippy::inline_always,
    reason = "measured: once or twice per nonzero block"
)]
#[inline(always)]
fn read_eob_literal(
    symbols: &mut SymbolDecoder<'_>,
    width: u32,
    syntax: &'static str,
) -> Result<u32, CoeffLoopContextError> {
    if width == 0 {
        return Ok(0);
    }
    #[cfg(feature = "block-evidence")]
    if symbols.observes_entropy() {
        symbols.annotate_entropy(splot_core::symbol::EntropySyntax::new(syntax, "", [0; 3]));
    }
    let value = symbols
        .read_literal(width)
        .map_err(|source| CoeffLoopContextError::EobLiteralRead { syntax, source })?;
    Ok(value)
}

#[cfg(test)]
#[path = "coeff_loop_tests.rs"]
mod tests;
