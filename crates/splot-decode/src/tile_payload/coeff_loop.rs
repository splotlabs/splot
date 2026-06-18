// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop foundation helpers.
//!
//! Feature tracking: `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE`, `DECODE-COEFF-ALL-ZERO-BLOCK-STATE`,
//! `DECODE-COEFF-EOB-VALUE-STATE`, `DECODE-COEFF-EOB-SYMBOL-READ`, `DECODE-COEFF-EOB-SIZE-CONTEXT`,
//! `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ`, `DECODE-COEFF-EOB-BRANCH-HANDOFF`,
//! `DECODE-COEFF-NONZERO-BLOCK-STATE`, `DECODE-COEFF-SCAN-WALK`, `DECODE-COEFF-LEVEL-STATE-WRITE`.

use std::collections::TryReserveError;

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

use super::cdf::block_context::{txb_skip_ctx_luma, v_txb_skip_ctx};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{EobPtSize, TileCdfSelector, TileCdfSubset};
use super::coeff_state::{
    CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError, TransformCoeffBlockState,
};

const LUMA_PLANE: usize = 0;
const V_PLANE: usize = 2;
const COEFFS_PER_4X4: usize = 4;
const MAX_ADJUSTED_COEFF_EXTENT: usize = 32;
const MIN_EOB_TX_LOG2: usize = 2;
const EOB_MULTISIZE_LOG2_CAP: usize = 5;
const EOB_MULTISIZE_OFFSET: usize = 4;
const MIN_NONZERO_EOB_PT: usize = 1;
const MAX_NONZERO_EOB_PT: usize = 11;
pub(crate) mod base_symbol;
mod branch;
pub(crate) use branch::{CoeffBlockEobBranchInput, read_coeff_block_eob_branch};
pub(crate) mod level_state;
pub(crate) mod quant_state;
pub(crate) mod read_quant;
mod scan_walk;
pub(crate) mod sign_symbol;
/// Caller-resolved facts for luma § 8.3.2 `all_zero` context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaAllZeroContextInput {
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
    /// Whether the transform fills its plane residual block (`bw == w && bh == h`).
    pub(crate) tx_fills_block: bool,
    /// Whether `fsc_mode && enable_fsc` selects the final luma context.
    pub(crate) fsc_active: bool,
}

/// Caller-resolved facts for V-plane § 8.3.2 `all_zero` context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VAllZeroContextInput {
    /// Transform-block x coordinate in chroma 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in chroma 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in chroma 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in chroma 4x4 units.
    pub(crate) h4: usize,
    /// Whether the chroma residual block is larger than the transform.
    pub(crate) chroma_block_larger_than_tx: bool,
    /// Whether the previously decoded U-plane EOB is nonzero.
    pub(crate) eob_u_nonzero: bool,
}

/// Caller-resolved facts for applying the § 5.20.7.27 all-zero coefficient path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AllZeroCoeffBlockInput {
    /// Plane index, 0 for luma and 1/2 for chroma.
    pub(crate) plane: usize,
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
}

/// Caller-resolved facts for computing the nonzero § 5.20.7.27 EOB value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobInput {
    /// Decoded `eobPt` after the active `eob_pt_*` and any size-specific
    /// `eob_pt_*_extra` syntax have been resolved by the caller.
    pub(crate) eob_pt: usize,
    /// Decoded `eob_extra` flag. This flag is only present for `eobPt >= 3`.
    pub(crate) eob_extra: bool,
    /// Packed `eob_extra_bit` refinements. Bit `i` corresponds to the spec loop
    /// contribution `1 << i`.
    pub(crate) eob_extra_bits: usize,
}

/// Caller-resolved facts for reading the nonzero § 5.20.7.27 EOB syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobSymbolInput {
    /// Transform-size EOB CDF family selected by the caller.
    pub(crate) size: EobPtSize,
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// `eobCtx = (plane > 0) ? 2 : is_inter`, resolved by the caller.
    pub(crate) eob_ctx: usize,
}

/// Caller-resolved facts for deriving nonzero § 5.20.7.27 EOB CDF selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobContextInput {
    /// Plane index, 0 for luma and 1/2 for chroma.
    pub(crate) plane: usize,
    /// Whether the current block is inter-predicted.
    pub(crate) is_inter: bool,
    /// `Tx_Width_Log2[txSz]`, resolved by the caller from transform syntax.
    pub(crate) tx_width_log2: usize,
    /// `Tx_Height_Log2[txSz]`, resolved by the caller from transform syntax.
    pub(crate) tx_height_log2: usize,
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
}

/// Summary of a § 5.20.7.27 all-zero coefficient block state application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AllZeroCoeffBlock {
    eob: usize,
    cul_level: u32,
    dc_category: u8,
    block: TransformCoeffBlockState,
}

impl AllZeroCoeffBlock {
    /// End-of-block value returned by `coeffs()`.
    #[must_use]
    pub(crate) const fn eob(&self) -> usize {
        self.eob
    }

    /// `culLevel` written to level context lines.
    #[must_use]
    pub(crate) const fn cul_level(&self) -> u32 {
        self.cul_level
    }

    /// `dcCategory` written to DC context lines.
    #[must_use]
    pub(crate) const fn dc_category(&self) -> u8 {
        self.dc_category
    }

    /// Zero-initialized local transform coefficient state.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

/// Checked nonzero § 5.20.7.27 EOB value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEob {
    eob_pt: usize,
    eob: usize,
}

impl NonZeroCoeffEob {
    /// Decoded `eobPt` used to derive this EOB value.
    #[must_use]
    pub(crate) const fn eob_pt(self) -> usize {
        self.eob_pt
    }

    /// End-of-block value returned by the nonzero `coeffs()` branch.
    #[must_use]
    pub(crate) const fn eob(self) -> usize {
        self.eob
    }
}

/// Result of the crate-private nonzero EOB symbol-read sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobSymbolRead {
    eob: NonZeroCoeffEob,
    eob_pt_symbol: u8,
    eob_pt_extra: u32,
    eob_extra: bool,
    eob_extra_bits: u32,
}

impl NonZeroCoeffEobSymbolRead {
    /// Checked EOB value derived from the decoded syntax elements.
    #[must_use]
    pub(crate) const fn eob(self) -> NonZeroCoeffEob {
        self.eob
    }

    /// Raw symbol decoded from the selected `eob_pt_*` CDF row.
    #[must_use]
    pub(crate) const fn eob_pt_symbol(self) -> u8 {
        self.eob_pt_symbol
    }

    /// Size-specific `eob_pt_*_extra` literal value, or zero when absent.
    #[must_use]
    pub(crate) const fn eob_pt_extra(self) -> u32 {
        self.eob_pt_extra
    }

    /// Decoded `eob_extra` flag, or false when absent.
    #[must_use]
    pub(crate) const fn eob_extra(self) -> bool {
        self.eob_extra
    }

    /// Packed `eob_extra_bit` refinement value, or zero when absent.
    #[must_use]
    pub(crate) const fn eob_extra_bits(self) -> u32 {
        self.eob_extra_bits
    }
}

/// Error returned by coefficient-loop context handoff helpers.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffLoopContextError {
    /// The underlying coefficient context state rejected a plane or allocation fact.
    #[error("coefficient context state error: {0}")]
    State(#[from] TileCoeffStateError),
    /// Reading an EOB CDF symbol failed.
    #[error("coefficient EOB symbol read failed: {0}")]
    EobSymbolRead(#[from] BlockSymbolTraceReadError),
    /// Reading EOB literal refinement bits failed.
    #[error("coefficient EOB literal read failed for {syntax}: {source}")]
    EobLiteralRead {
        /// Syntax element being read.
        syntax: &'static str,
        /// Source symbol-decoder error.
        #[source]
        source: CoreError,
    },
    /// Caller supplied an `eobPt` outside the nonzero § 5.20.7.27 range.
    #[error("coefficient EOB point {eob_pt} is outside the supported AV2 range 1..=11")]
    InvalidEobPoint {
        /// Caller-provided `eobPt`.
        eob_pt: usize,
    },
    /// Caller supplied refinement syntax for an `eobPt` that has no refinements.
    #[error(
        "coefficient EOB point {eob_pt} cannot carry eob_extra={eob_extra} or eob_extra_bits={eob_extra_bits}"
    )]
    UnexpectedEobRefinement {
        /// Caller-provided `eobPt`.
        eob_pt: usize,
        /// Caller-provided `eob_extra`.
        eob_extra: bool,
        /// Caller-provided packed `eob_extra_bit` refinements.
        eob_extra_bits: usize,
    },
    /// Caller supplied packed `eob_extra_bit` refinements outside the implied width.
    #[error(
        "coefficient EOB point {eob_pt} allows eob_extra_bits <= {max_eob_extra_bits}, got {eob_extra_bits}"
    )]
    EobExtraBitsOutOfRange {
        /// Caller-provided `eobPt`.
        eob_pt: usize,
        /// Caller-provided packed `eob_extra_bit` refinements.
        eob_extra_bits: usize,
        /// Largest packed refinement value allowed by `eobPt`.
        max_eob_extra_bits: usize,
    },
    /// Caller supplied a transform log2 dimension outside the AV2 EOB-size range.
    #[error(
        "coefficient EOB transform {axis} log2 value {value} is below the AV2 minimum {minimum}"
    )]
    InvalidEobTransformLog2 {
        /// Transform axis whose log2 dimension is invalid.
        axis: &'static str,
        /// Caller-provided log2 dimension.
        value: usize,
        /// Minimum accepted log2 dimension.
        minimum: usize,
    },
    /// Caller reached the ordinary non-FSC scan walk without a positive EOB.
    #[error("coefficient scan walk requires nonzero EOB, got {eob}")]
    InvalidScanWalkEob {
        /// Caller-provided EOB.
        eob: usize,
    },
    /// Caller supplied fewer scan entries than the decoded EOB requires.
    #[error("coefficient scan walk EOB {eob} exceeds scan length {scan_len}")]
    ScanWalkEobOutOfRange {
        /// Decoded EOB.
        eob: usize,
        /// Caller-supplied scan table length.
        scan_len: usize,
    },
    /// Caller supplied a scan position outside the initialized coefficient block.
    #[error(
        "coefficient scan index {scan_index} points to position {pos}, outside coefficient count {coeff_count}"
    )]
    ScanWalkPositionOutOfRange {
        /// Scan index `c` from § 5.20.7.27.
        scan_index: usize,
        /// Caller-supplied raster coefficient position.
        pos: usize,
        /// Local adjusted block coefficient count.
        coeff_count: usize,
    },
    /// Allocation for checked scan-walk entries failed.
    #[error("coefficient scan walk allocation failed: {0}")]
    ScanWalkAllocation(#[from] TryReserveError),
}

/// Derives the luma § 8.3.2 `all_zero` (`txb_skip`) context from tile state.
///
/// The context formula is defined in § 8.3.2
/// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). This helper only
/// resolves the `AboveLevelContext[0]` / `LeftLevelContext[0]` OR reductions
/// from owned tile state; transform geometry and FSC facts stay caller-resolved
/// until broader § 5.20 transform-block syntax is wired.
pub(crate) fn luma_all_zero_context(
    state: &TileCoeffContextState,
    input: LumaAllZeroContextInput,
) -> Result<usize, CoeffLoopContextError> {
    let above = bounded_or_u32(state.above_level(LUMA_PLANE)?, input.x4, input.w4);
    let left = bounded_or_u32(state.left_level(LUMA_PLANE)?, input.y4, input.h4);
    Ok(txb_skip_ctx_luma(
        above,
        left,
        input.tx_fills_block,
        input.fsc_active,
    ))
}

/// Derives the V-plane § 8.3.2 `all_zero` (`v_txb_skip`) context from tile state.
///
/// The context formula is defined in § 8.3.2
/// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). This helper resolves
/// the V-plane level/DC above and left nonzero facts from owned tile state; the
/// chroma geometry and `EobU` facts remain caller-resolved until broader
/// § 5.20 transform-block syntax is wired.
pub(crate) fn v_all_zero_context(
    state: &TileCoeffContextState,
    input: VAllZeroContextInput,
) -> Result<usize, CoeffLoopContextError> {
    let above = bounded_or_level_dc(
        state.above_level(V_PLANE)?,
        state.above_dc(V_PLANE)?,
        input.x4,
        input.w4,
    );
    let left = bounded_or_level_dc(
        state.left_level(V_PLANE)?,
        state.left_dc(V_PLANE)?,
        input.y4,
        input.h4,
    );
    Ok(v_txb_skip_ctx(
        above != 0,
        left != 0,
        input.chroma_block_larger_than_tx,
        input.eob_u_nonzero,
    ))
}

/// Applies the AV2 § 5.20.7.27 `all_zero == 1` coefficient-block state effects.
///
/// The syntax initializes `Quant[]`, `QuantSign[]`, `Level[]`, sets `eob`,
/// `culLevel`, and `dcCategory` to zero, then writes those zero context values to
/// `AboveLevelContext` / `LeftLevelContext` and `AboveDcContext` /
/// `LeftDcContext` at the end of `coeffs()`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). This helper
/// models only that all-zero branch. Transform size, transform type, scan order,
/// nonzero EOB, `read_quant`, dequantization, and reconstruction stay deferred.
pub(crate) fn apply_all_zero_coeff_block(
    state: &mut TileCoeffContextState,
    input: AllZeroCoeffBlockInput,
) -> Result<AllZeroCoeffBlock, CoeffLoopContextError> {
    // TODO(spec: DECODE-COEFF-ALL-ZERO-BLOCK-STATE): Model the plane-0
    // `TxTypes[y4+j][x4+i] = DCT_DCT` writes and plane-1 `EobU` / `cctx_type`
    // reset when broader transform-block state is wired.
    let width = adjusted_coeff_extent(input.w4);
    let height = adjusted_coeff_extent(input.h4);
    let block = TransformCoeffBlockState::new(width, height)?;
    let cul_level = 0;
    let dc_category = 0;
    state.update_after_coeffs(CoeffContextUpdate {
        plane: input.plane,
        x4: input.x4,
        y4: input.y4,
        w4: input.w4,
        h4: input.h4,
        cul_level,
        dc_category,
    })?;
    Ok(AllZeroCoeffBlock {
        eob: 0,
        cul_level,
        dc_category,
        block,
    })
}

/// Computes the AV2 § 5.20.7.27 nonzero-branch EOB value.
///
/// This helper starts after the caller has decoded the size-specific
/// `eob_pt_*` symbol and any `eob_pt_*_extra` literal bits into `eobPt`; it
/// models only the following `eob`, `eob_extra`, and `eob_extra_bit` arithmetic
/// in `coeffs()` (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
/// CDF reads, transform-size dispatch, scan walking, coefficient reads,
/// `Quant[]` writes, dequantization, and reconstruction remain caller-owned.
pub(crate) fn nonzero_coeff_eob(
    input: NonZeroCoeffEobInput,
) -> Result<NonZeroCoeffEob, CoeffLoopContextError> {
    let eob_pt = input.eob_pt;
    if !(MIN_NONZERO_EOB_PT..=MAX_NONZERO_EOB_PT).contains(&eob_pt) {
        return Err(CoeffLoopContextError::InvalidEobPoint { eob_pt });
    }
    if eob_pt < 3 {
        if input.eob_extra || input.eob_extra_bits != 0 {
            return Err(CoeffLoopContextError::UnexpectedEobRefinement {
                eob_pt,
                eob_extra: input.eob_extra,
                eob_extra_bits: input.eob_extra_bits,
            });
        }
        return Ok(NonZeroCoeffEob {
            eob_pt,
            eob: eob_pt,
        });
    }

    let extra_bits_width = eob_pt - 3;
    let max_eob_extra_bits = (1usize << extra_bits_width) - 1;
    if input.eob_extra_bits > max_eob_extra_bits {
        return Err(CoeffLoopContextError::EobExtraBitsOutOfRange {
            eob_pt,
            eob_extra_bits: input.eob_extra_bits,
            max_eob_extra_bits,
        });
    }

    let base = (1usize << (eob_pt - 2)) + 1;
    let extra = if input.eob_extra {
        1usize << (eob_pt - 3)
    } else {
        0
    };
    Ok(NonZeroCoeffEob {
        eob_pt,
        eob: base + extra + input.eob_extra_bits,
    })
}

/// Reads the AV2 § 5.20.7.27 nonzero-branch EOB syntax and returns its value.
///
/// The caller resolves the transform-size class, coefficient-CDF q context, and
/// `eobCtx`; this helper performs only the `eob_pt_*`, optional
/// `eob_pt_*_extra`, `eob_extra`, and `eob_extra_bit` read sequence before
/// delegating the arithmetic to [`nonzero_coeff_eob`]. Scan walking,
/// coefficient symbols, `Quant[]` writes, dequantization, and reconstruction
/// remain deferred.
pub(crate) fn read_nonzero_coeff_eob(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffEobSymbolInput,
) -> Result<NonZeroCoeffEobSymbolRead, CoeffLoopContextError> {
    let eob_pt_symbol = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::EobPt {
                size: input.size,
                coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                eob_ctx: input.eob_ctx,
            },
            symbols,
        )?
        .get();
    let eob_pt_extra_width = eob_pt_extra_width(input.size, eob_pt_symbol);
    let eob_pt_extra = read_eob_literal(symbols, eob_pt_extra_width, "eob_pt_extra")?;
    let eob_pt = resolved_eob_pt(eob_pt_symbol, eob_pt_extra_width, eob_pt_extra);

    let (eob_extra, eob_extra_bits) = if eob_pt >= 3 {
        let eob_extra = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::EobExtra {
                    coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                },
                symbols,
            )?
            .get()
            != 0;
        let width = (eob_pt - 3) as u32;
        (
            eob_extra,
            read_eob_literal(symbols, width, "eob_extra_bit")?,
        )
    } else {
        (false, 0)
    };

    let eob = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt,
        eob_extra,
        eob_extra_bits: eob_extra_bits as usize,
    })?;
    Ok(NonZeroCoeffEobSymbolRead {
        eob,
        eob_pt_symbol,
        eob_pt_extra,
        eob_extra,
        eob_extra_bits,
    })
}

/// Derives EOB selector facts and reads the AV2 § 5.20.7.27 nonzero EOB syntax.
///
/// Invalid selector facts fail before CDF or symbol-decoder consumption. Scan
/// walking, coefficient symbols, `Quant[]`, dequantization, and reconstruction
/// remain deferred.
pub(crate) fn read_nonzero_coeff_eob_from_context(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffEobContextInput,
) -> Result<NonZeroCoeffEobSymbolRead, CoeffLoopContextError> {
    let input = nonzero_coeff_eob_symbol_input(input)?;
    read_nonzero_coeff_eob(cdfs, symbols, input)
}

/// Derives the AV2 § 5.20.7.27 nonzero EOB symbol-reader input.
///
/// This helper maps caller-resolved `Tx_Width_Log2[txSz]` and
/// `Tx_Height_Log2[txSz]` to the active `eob_pt_*` CDF family and derives
/// `eobCtx = (plane > 0) ? 2 : is_inter` before handing the facts to
/// [`read_nonzero_coeff_eob`]. It does not read symbols, walk scan order, update
/// coefficient state, dequantize, or reconstruct.
pub(crate) fn nonzero_coeff_eob_symbol_input(
    input: NonZeroCoeffEobContextInput,
) -> Result<NonZeroCoeffEobSymbolInput, CoeffLoopContextError> {
    Ok(NonZeroCoeffEobSymbolInput {
        size: eob_pt_size_from_tx_log2(input.tx_width_log2, input.tx_height_log2)?,
        coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
        eob_ctx: eob_context(input.plane, input.is_inter),
    })
}

fn eob_pt_size_from_tx_log2(
    tx_width_log2: usize,
    tx_height_log2: usize,
) -> Result<EobPtSize, CoeffLoopContextError> {
    checked_eob_tx_log2("width", tx_width_log2)?;
    checked_eob_tx_log2("height", tx_height_log2)?;

    let eob_multisize = tx_width_log2.min(EOB_MULTISIZE_LOG2_CAP)
        + tx_height_log2.min(EOB_MULTISIZE_LOG2_CAP)
        - EOB_MULTISIZE_OFFSET;
    Ok(match eob_multisize {
        0 => EobPtSize::Pt16,
        1 => EobPtSize::Pt32,
        2 => EobPtSize::Pt64,
        3 => EobPtSize::Pt128,
        4 => EobPtSize::Pt256,
        5 => EobPtSize::Pt512,
        _ => EobPtSize::Pt1024,
    })
}

fn checked_eob_tx_log2(axis: &'static str, value: usize) -> Result<(), CoeffLoopContextError> {
    if value < MIN_EOB_TX_LOG2 {
        return Err(CoeffLoopContextError::InvalidEobTransformLog2 {
            axis,
            value,
            minimum: MIN_EOB_TX_LOG2,
        });
    }
    Ok(())
}

fn eob_context(plane: usize, is_inter: bool) -> usize {
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

fn read_eob_literal(
    symbols: &mut SymbolDecoder<'_>,
    width: u32,
    syntax: &'static str,
) -> Result<u32, CoeffLoopContextError> {
    if width == 0 {
        return Ok(0);
    }
    symbols
        .read_literal(width)
        .map_err(|source| CoeffLoopContextError::EobLiteralRead { syntax, source })
}

fn bounded_or_u32(values: &[u32], start: usize, count: usize) -> u32 {
    let mut value = 0;
    if let Some(tail) = values.get(start..) {
        for entry in tail.iter().take(count) {
            value |= *entry;
        }
    }
    value
}

fn bounded_or_u8(values: &[u8], start: usize, count: usize) -> u32 {
    let mut value = 0;
    if let Some(tail) = values.get(start..) {
        for entry in tail.iter().take(count) {
            value |= u32::from(*entry);
        }
    }
    value
}

fn bounded_or_level_dc(level: &[u32], dc: &[u8], start: usize, count: usize) -> u32 {
    bounded_or_u32(level, start, count) | bounded_or_u8(dc, start, count)
}

fn adjusted_coeff_extent(size4: usize) -> usize {
    size4
        .saturating_mul(COEFFS_PER_4X4)
        .min(MAX_ADJUSTED_COEFF_EXTENT)
}
#[cfg(test)]
mod base_symbol_tests;
#[cfg(test)]
mod eob_symbol_tests;
#[cfg(test)]
mod level_state_tests;
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
    use super::*;

    fn update(plane: usize, x4: usize, y4: usize, w4: usize, h4: usize) -> CoeffContextUpdate {
        CoeffContextUpdate {
            plane,
            x4,
            y4,
            w4,
            h4,
            cul_level: 4,
            dc_category: 2,
        }
    }

    #[test]
    fn luma_all_zero_context_reads_zero_state_for_first_block() {
        let state = TileCoeffContextState::new(16, 16).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 0,
                y4: 0,
                w4: 16,
                h4: 16,
                tx_fills_block: true,
                fsc_active: false,
            },
        )
        .unwrap();

        assert_eq!(ctx, 0);
    }

    #[test]
    fn luma_all_zero_context_reduces_state_lines_when_not_filling() {
        let mut state = TileCoeffContextState::new(8, 8).unwrap();
        state.update_after_coeffs(update(0, 2, 3, 2, 2)).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 1,
                y4: 2,
                w4: 4,
                h4: 4,
                tx_fills_block: false,
                fsc_active: false,
            },
        )
        .unwrap();

        assert_eq!(ctx, 5);
    }

    #[test]
    fn luma_all_zero_context_fsc_overrides_state() {
        let mut state = TileCoeffContextState::new(4, 4).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 4, 4)).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 0,
                y4: 0,
                w4: 4,
                h4: 4,
                tx_fills_block: true,
                fsc_active: true,
            },
        )
        .unwrap();

        assert_eq!(ctx, 9);
    }

    #[test]
    fn v_all_zero_context_combines_level_dc_state_and_geometry() {
        let mut state = TileCoeffContextState::new(8, 8).unwrap();
        state.update_after_coeffs(update(2, 2, 5, 2, 1)).unwrap();
        let ctx = v_all_zero_context(
            &state,
            VAllZeroContextInput {
                x4: 1,
                y4: 4,
                w4: 4,
                h4: 3,
                chroma_block_larger_than_tx: true,
                eob_u_nonzero: true,
            },
        )
        .unwrap();

        assert_eq!(ctx, 11);
    }

    #[test]
    fn all_zero_context_reductions_bound_out_of_range_and_pathological_counts() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(2, 0, 0, 1, 1)).unwrap();

        let luma = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: usize::MAX,
                y4: usize::MAX,
                w4: usize::MAX,
                h4: usize::MAX,
                tx_fills_block: false,
                fsc_active: false,
            },
        )
        .unwrap();
        let v = v_all_zero_context(
            &state,
            VAllZeroContextInput {
                x4: usize::MAX,
                y4: usize::MAX,
                w4: usize::MAX,
                h4: usize::MAX,
                chroma_block_larger_than_tx: false,
                eob_u_nonzero: false,
            },
        )
        .unwrap();

        assert_eq!(luma, 1);
        assert_eq!(v, 0);
    }

    #[test]
    fn all_zero_coeff_block_applies_zero_state_and_context_writes() {
        let mut state = TileCoeffContextState::new(6, 6).unwrap();
        state.update_after_coeffs(update(0, 1, 2, 3, 2)).unwrap();

        let applied = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 1,
                y4: 2,
                w4: 3,
                h4: 2,
            },
        )
        .unwrap();

        assert_eq!(applied.eob(), 0);
        assert_eq!(applied.cul_level(), 0);
        assert_eq!(applied.dc_category(), 0);
        assert_eq!(applied.block().width(), 12);
        assert_eq!(applied.block().height(), 8);
        assert!(applied.block().level().iter().all(|level| *level == 0));
        assert!(applied.block().quant_sign().iter().all(|sign| *sign == 0));
        assert!(applied.block().quant().iter().all(|quant| *quant == 0));
        assert_eq!(state.above_level(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.above_dc(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.left_level(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.left_dc(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn all_zero_coeff_block_rejects_bad_ranges_without_mutation() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 1, 1)).unwrap();
        let before = state.clone();

        let err = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 1,
                y4: 0,
                w4: 2,
                h4: 1,
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::State(TileCoeffStateError::ContextRangeOutOfBounds {
                context: "above",
                start: 1,
                end: 3,
                len: 2
            })
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn all_zero_coeff_block_rejects_zero_transform_extent_before_mutation() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 1, 1)).unwrap();
        let before = state.clone();

        let err = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 0,
                y4: 0,
                w4: 0,
                h4: 1,
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::State(TileCoeffStateError::InvalidAdjustedTransformExtent {
                axis: "width",
                value: 0
            })
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn all_zero_coeff_block_saturates_adjusted_extent_to_spec_cap() {
        let mut state = TileCoeffContextState::new(16, 16).unwrap();

        let applied = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 2,
                x4: 0,
                y4: 0,
                w4: 16,
                h4: 16,
            },
        )
        .unwrap();

        assert_eq!(applied.block().width(), 32);
        assert_eq!(applied.block().height(), 32);
        assert_eq!(applied.block().quant().len(), 1024);
    }

    #[test]
    fn nonzero_coeff_eob_maps_small_points_without_refinements() {
        let eob_one = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 1,
            eob_extra: false,
            eob_extra_bits: 0,
        })
        .unwrap();
        let eob_two = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 2,
            eob_extra: false,
            eob_extra_bits: 0,
        })
        .unwrap();

        assert_eq!(eob_one.eob_pt(), 1);
        assert_eq!(eob_one.eob(), 1);
        assert_eq!(eob_two.eob_pt(), 2);
        assert_eq!(eob_two.eob(), 2);
    }

    #[test]
    fn nonzero_coeff_eob_applies_eob_extra_and_refinement_bits() {
        let eob = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 5,
            eob_extra: true,
            eob_extra_bits: 0b10,
        })
        .unwrap();

        // eobPt 5 starts from (1 << 3) + 1, then `eob_extra` adds bit 2 and
        // packed refinement bits add bits 1..=0.
        assert_eq!(eob.eob(), 15);
    }

    #[test]
    fn nonzero_coeff_eob_reaches_max_av2_eob() {
        let eob = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 11,
            eob_extra: true,
            eob_extra_bits: 0xFF,
        })
        .unwrap();

        assert_eq!(eob.eob(), 1024);
    }

    #[test]
    fn nonzero_coeff_eob_rejects_invalid_eob_points() {
        let zero = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 0,
            eob_extra: false,
            eob_extra_bits: 0,
        })
        .unwrap_err();
        let oversized = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 12,
            eob_extra: false,
            eob_extra_bits: 0,
        })
        .unwrap_err();

        assert!(matches!(
            zero,
            CoeffLoopContextError::InvalidEobPoint { eob_pt: 0 }
        ));
        assert!(matches!(
            oversized,
            CoeffLoopContextError::InvalidEobPoint { eob_pt: 12 }
        ));
    }

    #[test]
    fn nonzero_coeff_eob_rejects_refinements_for_small_points() {
        let err = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 1,
            eob_extra: true,
            eob_extra_bits: 0,
        })
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::UnexpectedEobRefinement {
                eob_pt: 1,
                eob_extra: true,
                eob_extra_bits: 0
            }
        ));
    }

    #[test]
    fn nonzero_coeff_eob_rejects_out_of_range_refinement_bits() {
        let err = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 5,
            eob_extra: false,
            eob_extra_bits: 0b100,
        })
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::EobExtraBitsOutOfRange {
                eob_pt: 5,
                eob_extra_bits: 4,
                max_eob_extra_bits: 3
            }
        ));
    }
}
