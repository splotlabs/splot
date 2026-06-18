// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient sign reads.
//!
//! Feature tracking: `DECODE-COEFF-SIGN-SYMBOL-READ`,
//! `DECODE-COEFF-SIGN-SOURCE-DERIVE`.

use std::collections::TryReserveError;

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::dc_sign_ctx;
use super::super::cdf::{TileCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::max_level::CoeffTransformClass;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

/// Caller-selected sign CDF syntax name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffSignCdfSyntax {
    /// `dc_sign` syntax element.
    DcSign,
    /// `dc_sign_horz_vert` syntax element.
    DcSignHorzVert,
}

/// Caller-resolved `TileDcSignCdf` selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffDcSignSelector {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Plane type context.
    pub(crate) plane_type: usize,
    /// `isHidden` group.
    pub(crate) group: usize,
    /// DC-sign context.
    pub(crate) ctx: usize,
}

impl CoeffDcSignSelector {
    fn tile_selector(self) -> TileCdfSelector {
        TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: self.coeff_cdf_q_ctx,
            plane_type: self.plane_type,
            group: self.group,
            ctx: self.ctx,
        }
    }
}

/// Caller-selected sign source for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffSignReadSource {
    /// Do not read sign syntax; the sign is false.
    None,
    /// Read a caller-selected `dc_sign` or `dc_sign_horz_vert` CDF row.
    Cdf {
        /// Syntax element name.
        syntax: CoeffSignCdfSyntax,
        /// Caller-resolved CDF selector.
        selector: CoeffDcSignSelector,
    },
    /// Read a raw `sign_bit` literal.
    SignBit,
}

/// Caller-resolved sign read facts for one checked scan-walk entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffSignReadInput {
    /// Checked scan entry this input belongs to.
    pub(crate) entry: CoeffScanEntry,
    /// Caller-selected sign source.
    pub(crate) source: CoeffSignReadSource,
}

/// Caller-resolved facts needed to derive ordinary non-FSC sign sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffSignSourceDeriveConfig<'a> {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Plane index, 0 for luma and greater than 0 for chroma.
    pub(crate) plane: usize,
    /// Plane type context used by `TileDcSignCdf`.
    pub(crate) plane_type: usize,
    /// Caller-resolved `get_tx_class(PlaneTxType)` result.
    pub(crate) tx_class: CoeffTransformClass,
    /// Whether hidden parity is active for this transform block.
    pub(crate) is_hidden: bool,
    /// First-pass `sumAbs1` parity summary.
    pub(crate) sum_abs1: u32,
    /// `AboveDcContext[plane]`.
    pub(crate) above_dc: &'a [u8],
    /// `LeftDcContext[plane]`.
    pub(crate) left_dc: &'a [u8],
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
}

impl CoeffSignSourceDeriveConfig<'_> {
    fn dc_selector(self, ctx: usize) -> CoeffDcSignSelector {
        CoeffDcSignSelector {
            coeff_cdf_q_ctx: self.coeff_cdf_q_ctx,
            plane_type: self.plane_type,
            group: usize::from(self.is_hidden),
            ctx,
        }
    }
}

/// Raw sign syntax consumed for one entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffSignReadSymbol {
    /// No sign syntax was read.
    None,
    /// CDF-backed sign syntax was read.
    Cdf {
        /// Syntax element name.
        syntax: CoeffSignCdfSyntax,
        /// Raw decoded symbol.
        symbol: u8,
    },
    /// One-bit `sign_bit` literal was read.
    SignBit {
        /// Raw decoded bit.
        bit: bool,
    },
}

/// Decoded sign summary for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffSignRead {
    entry: CoeffScanEntry,
    level: u32,
    symbol: CoeffSignReadSymbol,
    sign: bool,
}

impl CoeffSignRead {
    #[cfg(test)]
    pub(crate) const fn for_test(
        entry: CoeffScanEntry,
        level: u32,
        symbol: CoeffSignReadSymbol,
        sign: bool,
    ) -> Self {
        Self {
            entry,
            level,
            symbol,
            sign,
        }
    }

    /// Checked scan entry associated with this read.
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    /// Local `Level[row][col]` read before sign syntax.
    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }

    /// Raw sign syntax consumed for this entry.
    #[must_use]
    pub(crate) const fn symbol(self) -> CoeffSignReadSymbol {
        self.symbol
    }

    /// Boolean sign value used by later quantization state.
    #[must_use]
    pub(crate) const fn sign(self) -> bool {
        self.sign
    }
}

/// Error returned by the coefficient sign-read boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffSignReadError {
    /// The number of sign inputs did not match the checked scan walk.
    #[error("coefficient sign input count {inputs} does not match scan entries {entries}")]
    InputCountMismatch {
        /// Caller-provided input count.
        inputs: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// One sign input was not paired with the matching checked scan entry.
    #[error(
        "coefficient sign input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    ScanEntryMismatch {
        /// Input index.
        index: usize,
        /// Expected checked scan entry.
        expected: CoeffScanEntry,
        /// Caller-provided scan entry.
        actual: CoeffScanEntry,
    },
    /// A nonzero level was paired with a disabled sign source.
    #[error("coefficient sign input {index} disabled sign read for nonzero level {level}")]
    MissingRequiredSign {
        /// Input index.
        index: usize,
        /// Checked scan entry.
        entry: CoeffScanEntry,
        /// Local level value.
        level: u32,
    },
    /// The local transform-block state rejected a checked coordinate.
    #[error("coefficient sign read state error: {0}")]
    State(#[from] TileCoeffStateError),
    /// Allocation for decoded coefficient sign records failed.
    #[error("coefficient sign read allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// CDF row selection or AV2 §8.2 symbol decoding failed.
    #[error("coefficient sign symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
    /// Reading a raw `sign_bit` literal failed.
    #[error("coefficient sign literal read failed: {source}")]
    LiteralRead {
        /// Source symbol-decoder error.
        #[source]
        source: CoreError,
    },
}

/// Error returned by coefficient sign-source derivation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffSignSourceDeriveError {
    /// The local transform-block state rejected a checked coordinate.
    #[error("coefficient sign-source derivation state error: {0}")]
    State(#[from] TileCoeffStateError),
    /// Allocation for derived coefficient sign inputs failed.
    #[error("coefficient sign-source derivation allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
}

/// Derives ordinary non-FSC §5.20.7.27 sign sources over checked scan entries.
///
/// The caller supplies post-first-pass local `Level[]`, hidden-parity summary,
/// plane, transform-class, and DC context facts. This helper derives the source
/// for each existing [`CoeffSignReadInput`] according to the sign branch in
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`, using the
/// §8.3.2 `dc_sign` context rule from
/// `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`. It does not consume
/// symbols, mutate CDF rows, write coefficient state, commit tile context
/// lines, or invoke reconstruction.
pub(crate) fn derive_nonzero_coeff_sign_inputs(
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    config: CoeffSignSourceDeriveConfig<'_>,
) -> Result<Vec<CoeffSignReadInput>, CoeffSignSourceDeriveError> {
    let entries = walk.entries();
    let mut inputs = Vec::new();
    inputs.try_reserve(entries.len())?;
    for entry in entries.iter().copied() {
        let level = block.level_at(entry.row(), entry.col())?;
        inputs.push(CoeffSignReadInput {
            entry,
            source: derive_coeff_sign_source(entry, level, config),
        });
    }
    Ok(inputs)
}

fn derive_coeff_sign_source(
    entry: CoeffScanEntry,
    level: u32,
    config: CoeffSignSourceDeriveConfig<'_>,
) -> CoeffSignReadSource {
    if !coeff_requires_sign_source(entry, level, config) {
        return CoeffSignReadSource::None;
    }

    if entry.row() == 0 && entry.col() == 0 && config.plane == 0 {
        return CoeffSignReadSource::Cdf {
            syntax: CoeffSignCdfSyntax::DcSign,
            selector: config.dc_selector(dc_sign_ctx(
                config.above_dc,
                config.left_dc,
                config.x4,
                config.y4,
                config.w4,
                config.h4,
            )),
        };
    }
    if config.tx_class == CoeffTransformClass::Horizontal && entry.col() == 0 && config.plane == 0 {
        return CoeffSignReadSource::Cdf {
            syntax: CoeffSignCdfSyntax::DcSignHorzVert,
            selector: config.dc_selector(0),
        };
    }
    if config.tx_class == CoeffTransformClass::Vertical && entry.row() == 0 && config.plane == 0 {
        return CoeffSignReadSource::Cdf {
            syntax: CoeffSignCdfSyntax::DcSignHorzVert,
            selector: config.dc_selector(0),
        };
    }

    CoeffSignReadSource::SignBit
}

fn coeff_requires_sign_source(
    entry: CoeffScanEntry,
    level: u32,
    config: CoeffSignSourceDeriveConfig<'_>,
) -> bool {
    level != 0 || (config.is_hidden && entry.scan_index() == 0 && config.sum_abs1 > 0)
}

/// Reads ordinary non-FSC §5.20.7.27 coefficient signs over checked scan entries.
///
/// The caller owns the branch that selects `dc_sign`, `dc_sign_horz_vert`,
/// `sign_bit`, or no read. This helper validates the already checked scan
/// entries against local `Level[]` state, enforces that nonzero levels have a
/// sign source, and then consumes the requested sign syntax
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27` and
/// `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). It does not write
/// `QuantSign[]`, `Quant[]`, tile context lines, or reconstruction state.
pub(crate) fn read_nonzero_coeff_signs(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    inputs: &[CoeffSignReadInput],
) -> Result<Vec<CoeffSignRead>, CoeffSignReadError> {
    let entries = walk.entries();
    if inputs.len() != entries.len() {
        return Err(CoeffSignReadError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }

    let levels = preflight_nonzero_coeff_signs(block, walk, inputs)?;
    let mut reads = Vec::new();
    reads.try_reserve(entries.len())?;
    for (input, level) in inputs.iter().copied().zip(levels) {
        reads.push(read_preflighted_nonzero_coeff_sign(
            cdfs, symbols, input, level,
        )?);
    }
    Ok(reads)
}

/// Validates sign inputs before any sign or quant syntax is consumed.
pub(crate) fn preflight_nonzero_coeff_signs(
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    inputs: &[CoeffSignReadInput],
) -> Result<Vec<u32>, CoeffSignReadError> {
    let entries = walk.entries();
    if inputs.len() != entries.len() {
        return Err(CoeffSignReadError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }

    let mut levels = Vec::new();
    levels.try_reserve(entries.len())?;
    for (index, (entry, input)) in entries
        .iter()
        .copied()
        .zip(inputs.iter().copied())
        .enumerate()
    {
        if input.entry != entry {
            return Err(CoeffSignReadError::ScanEntryMismatch {
                index,
                expected: entry,
                actual: input.entry,
            });
        }
        let level = block.level_at(entry.row(), entry.col())?;
        if level != 0 && input.source == CoeffSignReadSource::None {
            return Err(CoeffSignReadError::MissingRequiredSign {
                index,
                entry,
                level,
            });
        }
        levels.push(level);
    }
    Ok(levels)
}

/// Reads one sign whose scan entry and local level were already preflighted.
pub(crate) fn read_preflighted_nonzero_coeff_sign(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffSignReadInput,
    level: u32,
) -> Result<CoeffSignRead, CoeffSignReadError> {
    let (symbol, sign) = match input.source {
        CoeffSignReadSource::None => (CoeffSignReadSymbol::None, false),
        CoeffSignReadSource::Cdf { syntax, selector } => {
            let symbol = cdfs
                .read_block_symbol_trace(selector.tile_selector(), symbols)?
                .get();
            (CoeffSignReadSymbol::Cdf { syntax, symbol }, symbol != 0)
        }
        CoeffSignReadSource::SignBit => {
            let bit = symbols
                .read_literal(1)
                .map_err(|source| CoeffSignReadError::LiteralRead { source })?
                != 0;
            (CoeffSignReadSymbol::SignBit { bit }, bit)
        }
    };
    Ok(CoeffSignRead {
        entry: input.entry,
        level,
        symbol,
        sign,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

    use super::super::super::cdf::block_read::BlockSymbolTraceReadError;
    use super::super::super::cdf::{FrameCdfSubset, TileCdfArray, TileCdfError};
    use super::super::super::coeff_state::TransformCoeffBlockState;
    use super::super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStartInput};
    use super::super::max_level::CoeffTransformClass;
    use super::super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
    use super::super::*;
    use super::*;

    const EOB_SCAN: [u16; 4] = [0, 8, 1, 9];
    const ALT_SCAN: [u16; 4] = [0, 8, 9, 1];
    const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
        [0x00, 0x00, 0x80],
        [0xff, 0x00, 0x80],
        [0x55, 0xaa, 0x80],
        [0xff, 0xff, 0x80],
    ];
    static ABOVE_DC_POSITIVE: [u8; 4] = [2, 2, 0, 0];
    static LEFT_DC_ZERO: [u8; 4] = [0, 0, 0, 0];

    fn symbol_decoder(payload: &[u8], mode: CdfUpdateMode) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(mode),
        )
        .unwrap()
    }

    fn branch_nonzero(
        branch: CoeffBlockEobBranch,
    ) -> Option<super::super::branch::NonZeroCoeffBlockStart> {
        match branch {
            CoeffBlockEobBranch::AllZero(_) => None,
            CoeffBlockEobBranch::NonZero(start) => Some(start),
        }
    }

    fn setup_walk(payload: &[u8], scan: &[u16]) -> Option<NonZeroCoeffScanWalk> {
        let frame = FrameCdfSubset::from_defaults();
        let mut tile = frame.tile_copy();
        let mut symbols = symbol_decoder(payload, CdfUpdateMode::Enabled);
        let mut state = super::super::super::coeff_state::TileCoeffContextState::new(4, 4).ok()?;
        let branch = read_coeff_block_eob_branch(
            &mut state,
            &mut tile,
            &mut symbols,
            CoeffBlockEobBranchInput::NonZero(NonZeroCoeffBlockStartInput {
                block: AllZeroCoeffBlockInput {
                    plane: 0,
                    x4: 0,
                    y4: 0,
                    w4: 2,
                    h4: 2,
                },
                eob: NonZeroCoeffEobContextInput {
                    plane: 0,
                    is_inter: false,
                    tx_width_log2: 3,
                    tx_height_log2: 3,
                    coeff_cdf_q_ctx: 0,
                },
            }),
        )
        .ok()?;
        let start = branch_nonzero(branch)?;
        if start.eob_read().eob().eob() != scan.len() {
            return None;
        }
        walk_nonzero_coeff_scan(&start, scan).ok()
    }

    fn find_eob_payload() -> [u8; 5] {
        for first in u8::MIN..=u8::MAX {
            for second in u8::MIN..=u8::MAX {
                for suffix in PAYLOAD_SUFFIXES {
                    let payload = [first, second, suffix[0], suffix[1], suffix[2]];
                    if setup_walk(&payload, &EOB_SCAN).is_some() {
                        return payload;
                    }
                }
            }
        }
        panic!("no coefficient sign EOB payload found");
    }

    fn block_for(walk: &NonZeroCoeffScanWalk) -> TransformCoeffBlockState {
        let mut block = TransformCoeffBlockState::new(8, 8).unwrap();
        for (index, entry) in walk.entries().iter().copied().enumerate() {
            let level = match index {
                0 => 3,
                1 => 2,
                2 => 0,
                _ => 1,
            };
            block.set_level(entry.row(), entry.col(), level).unwrap();
        }
        block
    }

    fn dc_sign_selector() -> CoeffDcSignSelector {
        CoeffDcSignSelector {
            coeff_cdf_q_ctx: 0,
            plane_type: 0,
            group: 0,
            ctx: 0,
        }
    }

    fn invalid_dc_sign_selector() -> CoeffDcSignSelector {
        CoeffDcSignSelector {
            coeff_cdf_q_ctx: 4,
            plane_type: 0,
            group: 0,
            ctx: 0,
        }
    }

    fn scan_entry(scan_index: usize, row: usize, col: usize) -> CoeffScanEntry {
        CoeffScanEntry::for_test(
            scan_index,
            row.saturating_mul(8).saturating_add(col),
            row,
            col,
        )
    }

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
            above_dc: &ABOVE_DC_POSITIVE,
            left_dc: &LEFT_DC_ZERO,
            x4: 0,
            y4: 0,
            w4: 2,
            h4: 2,
        }
    }

    fn inputs_for(walk: &NonZeroCoeffScanWalk) -> Vec<CoeffSignReadInput> {
        walk.entries()
            .iter()
            .copied()
            .enumerate()
            .map(|(index, entry)| CoeffSignReadInput {
                entry,
                source: match index {
                    0 => CoeffSignReadSource::Cdf {
                        syntax: CoeffSignCdfSyntax::DcSign,
                        selector: dc_sign_selector(),
                    },
                    1 => CoeffSignReadSource::Cdf {
                        syntax: CoeffSignCdfSyntax::DcSignHorzVert,
                        selector: dc_sign_selector(),
                    },
                    2 => CoeffSignReadSource::None,
                    _ => CoeffSignReadSource::SignBit,
                },
            })
            .collect()
    }

    #[test]
    fn coefficient_sign_source_derives_luma_dc_context_and_hidden_group() {
        let entry = scan_entry(0, 0, 0);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
        let block = TransformCoeffBlockState::new(8, 8).unwrap();

        let inputs = derive_nonzero_coeff_sign_inputs(
            &block,
            &walk,
            source_config(0, CoeffTransformClass::TwoD, true, 3),
        )
        .unwrap();

        assert_eq!(
            inputs,
            vec![CoeffSignReadInput {
                entry,
                source: CoeffSignReadSource::Cdf {
                    syntax: CoeffSignCdfSyntax::DcSign,
                    selector: CoeffDcSignSelector {
                        coeff_cdf_q_ctx: 3,
                        plane_type: 0,
                        group: 1,
                        ctx: 2,
                    },
                },
            }]
        );
    }

    #[test]
    fn coefficient_sign_source_derives_horizontal_and_vertical_axis_contexts() {
        let horizontal_entry = scan_entry(2, 5, 0);
        let horizontal_walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![horizontal_entry]);
        let vertical_entry = scan_entry(2, 0, 5);
        let vertical_walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![vertical_entry]);
        let mut block = TransformCoeffBlockState::new(8, 8).unwrap();
        block
            .set_level(horizontal_entry.row(), horizontal_entry.col(), 1)
            .unwrap();
        block
            .set_level(vertical_entry.row(), vertical_entry.col(), 1)
            .unwrap();

        let horizontal = derive_nonzero_coeff_sign_inputs(
            &block,
            &horizontal_walk,
            source_config(0, CoeffTransformClass::Horizontal, false, 0),
        )
        .unwrap();
        let vertical = derive_nonzero_coeff_sign_inputs(
            &block,
            &vertical_walk,
            source_config(0, CoeffTransformClass::Vertical, false, 0),
        )
        .unwrap();

        for (entry, inputs) in [(horizontal_entry, horizontal), (vertical_entry, vertical)] {
            assert_eq!(
                inputs,
                vec![CoeffSignReadInput {
                    entry,
                    source: CoeffSignReadSource::Cdf {
                        syntax: CoeffSignCdfSyntax::DcSignHorzVert,
                        selector: CoeffDcSignSelector {
                            coeff_cdf_q_ctx: 3,
                            plane_type: 0,
                            group: 0,
                            ctx: 0,
                        },
                    },
                }]
            );
        }
    }

    #[test]
    fn coefficient_sign_source_derives_sign_bit_and_skip_sources() {
        let sign_bit_entry = scan_entry(3, 1, 1);
        let zero_entry = scan_entry(2, 2, 2);
        let chroma_dc_entry = scan_entry(1, 0, 0);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![sign_bit_entry, zero_entry]);
        let chroma_walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![chroma_dc_entry]);
        let mut block = TransformCoeffBlockState::new(8, 8).unwrap();
        block
            .set_level(sign_bit_entry.row(), sign_bit_entry.col(), 4)
            .unwrap();
        block
            .set_level(chroma_dc_entry.row(), chroma_dc_entry.col(), 2)
            .unwrap();

        let inputs = derive_nonzero_coeff_sign_inputs(
            &block,
            &walk,
            source_config(0, CoeffTransformClass::TwoD, false, 0),
        )
        .unwrap();
        let chroma_inputs = derive_nonzero_coeff_sign_inputs(
            &block,
            &chroma_walk,
            source_config(1, CoeffTransformClass::TwoD, false, 0),
        )
        .unwrap();

        assert_eq!(inputs[0].source, CoeffSignReadSource::SignBit);
        assert_eq!(inputs[1].source, CoeffSignReadSource::None);
        assert_eq!(chroma_inputs[0].source, CoeffSignReadSource::SignBit);
    }

    #[test]
    fn coefficient_sign_source_derivation_reports_state_errors() {
        let entry = CoeffScanEntry::for_test(0, 99, 4, 0);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
        let block = TransformCoeffBlockState::new(4, 4).unwrap();

        let err = derive_nonzero_coeff_sign_inputs(
            &block,
            &walk,
            source_config(0, CoeffTransformClass::TwoD, false, 0),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffSignSourceDeriveError::State(
                super::super::super::coeff_state::TileCoeffStateError::TransformCoordinateOutOfBounds {
                    row: 4,
                    col: 0,
                    height: 4,
                    width: 4,
                }
            )
        ));
    }

    #[test]
    fn coefficient_sign_read_consumes_mixed_sources() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let block = block_for(&walk);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&[0xff, 0xff, 0x80], CdfUpdateMode::Enabled);
        let consumed_before = symbols.consumed_bits();
        let inputs = inputs_for(&walk);

        let reads =
            read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &walk, &inputs).unwrap();

        assert_eq!(reads.len(), walk.entries().len());
        assert!(matches!(
            reads[0].symbol(),
            CoeffSignReadSymbol::Cdf {
                syntax: CoeffSignCdfSyntax::DcSign,
                ..
            }
        ));
        assert!(matches!(
            reads[1].symbol(),
            CoeffSignReadSymbol::Cdf {
                syntax: CoeffSignCdfSyntax::DcSignHorzVert,
                ..
            }
        ));
        assert_eq!(reads[2].level(), 0);
        assert_eq!(reads[2].symbol(), CoeffSignReadSymbol::None);
        assert!(!reads[2].sign());
        assert!(matches!(
            reads[3].symbol(),
            CoeffSignReadSymbol::SignBit { .. }
        ));
        for (read, input) in reads.iter().zip(&inputs) {
            assert_eq!(read.entry(), input.entry);
        }
        assert!(symbols.consumed_bits() > consumed_before);
        assert!(symbols.symbol_count() >= 2);
    }

    #[test]
    fn coefficient_sign_read_rejects_missing_required_sign_before_consumption() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let block = block_for(&walk);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let tile_before = tile.clone();
        let mut symbols = symbol_decoder(&[0xff, 0x80], CdfUpdateMode::Enabled);
        let consumed_before = symbols.consumed_bits();
        let symbol_count_before = symbols.symbol_count();
        let mut inputs = inputs_for(&walk);
        inputs[0].source = CoeffSignReadSource::None;

        let err =
            read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &walk, &inputs).unwrap_err();

        assert!(matches!(
            err,
            CoeffSignReadError::MissingRequiredSign {
                index: 0,
                level: 3,
                ..
            }
        ));
        assert_eq!(tile, tile_before);
        assert_eq!(symbols.consumed_bits(), consumed_before);
        assert_eq!(symbols.symbol_count(), symbol_count_before);
    }

    #[test]
    fn coefficient_sign_read_rejects_scan_entry_mismatch_before_consumption() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let alt_walk = setup_walk(&payload, &ALT_SCAN).unwrap();
        let block = block_for(&walk);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let tile_before = tile.clone();
        let mut symbols = symbol_decoder(&[0xff, 0x80], CdfUpdateMode::Enabled);
        let consumed_before = symbols.consumed_bits();
        let inputs = inputs_for(&walk);

        let err = read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &alt_walk, &inputs)
            .unwrap_err();

        assert!(matches!(
            err,
            CoeffSignReadError::ScanEntryMismatch { index: 0, .. }
        ));
        assert_eq!(tile, tile_before);
        assert_eq!(symbols.consumed_bits(), consumed_before);
    }

    #[test]
    fn coefficient_sign_read_rejects_invalid_cdf_selector_before_symbol_read() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let block = block_for(&walk);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let tile_before = tile.clone();
        let mut symbols = symbol_decoder(&[0xff, 0x80], CdfUpdateMode::Enabled);
        let consumed_before = symbols.consumed_bits();
        let symbol_count_before = symbols.symbol_count();
        let mut inputs = inputs_for(&walk);
        inputs[0].source = CoeffSignReadSource::Cdf {
            syntax: CoeffSignCdfSyntax::DcSign,
            selector: invalid_dc_sign_selector(),
        };

        let err =
            read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &walk, &inputs).unwrap_err();

        assert!(matches!(
            err,
            CoeffSignReadError::SymbolRead(BlockSymbolTraceReadError::Cdf(
                TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::DcSign,
                    index_name: "coeff_cdf_q_ctx",
                    actual: 4,
                    max_exclusive: 4,
                }
            ))
        ));
        assert_eq!(tile, tile_before);
        assert_eq!(symbols.consumed_bits(), consumed_before);
        assert_eq!(symbols.symbol_count(), symbol_count_before);
    }

    #[test]
    fn coefficient_sign_read_rejects_input_count_mismatch_before_consumption() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let block = block_for(&walk);
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let tile_before = tile.clone();
        let mut symbols = symbol_decoder(&[0xff, 0x80], CdfUpdateMode::Enabled);
        let consumed_before = symbols.consumed_bits();
        let symbol_count_before = symbols.symbol_count();
        let mut inputs = inputs_for(&walk);
        inputs.pop();

        let err =
            read_nonzero_coeff_signs(&mut tile, &mut symbols, &block, &walk, &inputs).unwrap_err();

        assert!(matches!(
            err,
            CoeffSignReadError::InputCountMismatch {
                inputs: 3,
                entries: 4
            }
        ));
        assert_eq!(tile, tile_before);
        assert_eq!(symbols.consumed_bits(), consumed_before);
        assert_eq!(symbols.symbol_count(), symbol_count_before);
    }
}
