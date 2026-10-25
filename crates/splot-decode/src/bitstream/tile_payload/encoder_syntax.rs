// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder-facing syntax facade; not a stable API.
//!
//! [`IntraTileSyntax`] owns one tile's CDF rows, neighbour state, and
//! reconstruction, and hands an encoder the same partition rules, § 8.3.2
//! contexts, CDF rows, and prediction kernels the tile decoder uses. The encoder
//! writes each symbol into the returned row, reconstructs each transform block
//! through the facade, then publishes the coded facts back, so both sides stay
//! in step. An inter frame codes intra and single-reference inter leaves;
//! [`EncoderReferences`] keeps the decoder's reference buffer they predict from.

use core::ops::Range;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;
use splot_recon::{
    BitDepth, DequantBlockParams, InverseTransform2dOuter, PlaneId, coefficient_scan_slice,
    tx_class,
};

use super::cdf::block_context::{IntraYMode, uv_mode_ctx};
use super::cdf::coeff_context::{
    CoeffBaseContext, CoeffBaseSelection, CoeffBrContext, coeff_base_eob_ctx,
};
use super::cdf::{CoeffBaseRow, CoeffCdfSelector, FrameCdfSubset, TileCdfSelector, TileCdfSubset};
use super::coeff_loop::base_level_pass::tx_class_index;
use super::coeff_loop::max_level::{
    CoeffMaxLevelConfig, CoeffTransformClass, coeff_is_low_frequency, derive_coeff_max_level,
    visible_max_level,
};
use super::coeff_loop::scan_walk::CoeffScanEntry;
use super::coeff_loop::sign_symbol::{
    CoeffSignReadSource, CoeffSignSourceDeriveConfig, derive_nonzero_coeff_sign_input,
};
use super::coeff_loop::{NonZeroCoeffEobContextInput, nonzero_coeff_eob_symbol_input};
use super::coeff_state::{CoeffContextUpdate, TileCoeffContextState, TransformCoeffBlockState};
use super::general_intra_residual::{block_recon_params, txb_skip_selector, txb_skip_tx_size_ctx};
use super::intra_joint_modes::{
    TileFscModeState, TileIntraJointModeState, TileIntraYModeState, TileLumaPaletteState,
    TileUseDipState, TileUsesMrlsState, TileUvCflState,
};
use super::mi_size_state::TileMiSizeState;
pub use super::partition::PartitionType as Partition;
use super::partition::{PartitionDecisionError, PartitionSymbols, read_partition_decision};
use super::partition_allowed::{PartitionDecisionFacts, partition_decision_facts};
use super::partition_size::BlockSize;
use super::partition_traversal::{
    DecodeBlockFrontier, DecodeBlockPart, ROOT_HAS_CHROMA, TilePartitionBounds, TilePartitionCall,
    TilePartitionFrameFacts, call_in_frame, child_calls, decode_block_frontier,
    partition_allowed_input, partition_context_inputs, updated_chroma_offset, valid_subsize,
};
use super::tile_frontier::minimal_partition_frame_facts;

mod cctx;
mod compound;
mod fsc;
mod inter;
mod intra;
mod mv;
mod parity;
mod rollback;
mod tcq;
mod transform;

pub use super::cdf::block_context::IntraYMode as IntraMode;
pub use crate::prediction::chroma::cfl::{CFL_FILTERS_420, CFL_FILTERS_422};
pub use cctx::CCTX_ROTATIONS;
pub use compound::{
    COMPOUND_MODES, CompoundMode, CompoundModes, InterPrediction, JMVD_SCALE_MODES,
};
pub use fsc::FSC_MAX_LEVEL;
pub use inter::{
    AMVD_STEPS, EncoderReferences, INTERP_FILTERS, InterMode, InterModes, MV_PRECISION_EIGHTH_PEL,
    MotionVector, SingleMvs,
};
pub use intra::{ChromaMode, Dip, IntraModes, MAX_CFL_ALPHA, Mrl, TxType};
pub use parity::ParityHiding;
pub use rollback::SyntaxSnapshot;
use rollback::{CdfJournal, CdfRow};
pub use tcq::{TCQ_STATES, tcq_index, tcq_next_state, tcq_quantizer};
pub use transform::{IST_SETS, Ist, TX_PARTITIONS, TxPartition};

#[path = "encoder_filters.rs"]
mod filters;
mod gdf;
pub use filters::{
    CcsoBlockMap, CcsoGrid, CdefIndexMap, CdefSlot, FilterRecorder, LrLayout, LrSymbol, LrUnit,
    LrUnitType, PcWienerClassMap, apply_loop_filters, ccso_offset, ccso_taps, cdef_unit_grid,
    merge_tiles, pc_wiener_cell_classes,
};
pub use gdf::{GdfBlockMap, GdfFlags, GdfGrid, apply_gdf};
/// The first tap `(dy, dx, coeff_index)` of each § 7.20.3 luma pair; the partner is `(-dy, -dx)`.
pub use splot_recon::WIENER_NS_CONFIG_Y_PAIRS;
/// The § 7.20.3 chroma taps `(dy, dx, coeff_index)` and their luma samples.
pub use splot_recon::{
    WIENER_NS_CONFIG_UV, pc_wiener_filter_set_index, pc_wiener_subclass_table,
    wiener_ns_chroma_luma_plane,
};

type BoxedSource = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Error from the encoder syntax facade.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EncoderSyntaxError {
    /// The frame uses a tool the facade does not model.
    #[error("unsupported encoder syntax: {0}")]
    Unsupported(&'static str),
    /// A decoder rule rejected the request.
    #[error("{stage} failed: {source}")]
    Rule {
        /// The failing step.
        stage: &'static str,
        /// The decoder error.
        source: BoxedSource,
    },
}

fn rule<E: std::error::Error + Send + Sync + 'static>(
    stage: &'static str,
) -> impl FnOnce(E) -> EncoderSyntaxError {
    move |source| EncoderSyntaxError::Rule {
        stage,
        source: Box::new(source),
    }
}

/// Result alias of the facade.
pub type Result<T> = core::result::Result<T, EncoderSyntaxError>;

/// One `decode_partition` call (§ 5.20.3.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionNode(TilePartitionCall);

impl PartitionNode {
    /// Luma row in 4x4 units.
    #[must_use]
    pub const fn row4(self) -> usize {
        self.0.r
    }

    /// Luma column in 4x4 units.
    #[must_use]
    pub const fn col4(self) -> usize {
        self.0.c
    }

    /// § 9.3 block-size index.
    #[must_use]
    pub const fn block_size(self) -> usize {
        self.0.b_size.index()
    }

    /// Luma `(width, height)` in samples.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid block size.
    pub fn dimensions(self) -> Result<(usize, usize)> {
        let size = self.0.b_size;
        Ok((
            size.width_samples().map_err(rule("block width"))?,
            size.height_samples().map_err(rule("block height"))?,
        ))
    }
}

/// The § 5.20.3.2 partitions a node allows and the one its position implies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionOptions(PartitionDecisionFacts);

impl PartitionOptions {
    /// Whether `partition` is in the allowed set.
    #[must_use]
    pub fn allows(self, partition: Partition) -> bool {
        self.0.allowed().contains(partition)
    }

    /// The `partition_implied` result, before the allowed-set check.
    #[must_use]
    pub const fn implied(self) -> Option<Partition> {
        self.0.implied()
    }
}

/// Answers each § 5.20.3.2 partition symbol for `target` and writes it, so
/// the decoder's `read_partition` decides which symbols the target needs.
struct PartitionWriter<'a, S> {
    target: Partition,
    sink: &'a mut S,
    journal: &'a mut CdfJournal,
}

impl<S: SymbolSink> PartitionSymbols for PartitionWriter<'_, S> {
    fn flag(
        &mut self,
        _syntax: &'static str,
        selector: TileCdfSelector,
        cdfs: &mut TileCdfSubset,
    ) -> core::result::Result<bool, PartitionDecisionError> {
        use Partition as P;
        let target = self.target;
        let value = match selector {
            TileCdfSelector::DoSplit { .. } => target != P::None,
            TileCdfSelector::DoSquareSplit { .. } => target == P::Split,
            TileCdfSelector::RectType { .. } => {
                matches!(target, P::Vert | P::Vert3 | P::Vert4A | P::Vert4B)
            }
            TileCdfSelector::DoExtPartition { .. } => !matches!(target, P::Horz | P::Vert),
            _ => matches!(target, P::Horz4A | P::Horz4B | P::Vert4A | P::Vert4B),
        };
        let (base, _) = cdfs.rows_span();
        let row = cdfs.row_mut(selector)?;
        self.journal.log(base, CdfRow::Tile(selector), row);
        self.sink
            .symbol(row, Symbol::new(u8::from(value)))
            .map_err(PartitionDecisionError::Write)?;
        Ok(value)
    }

    fn bit(&mut self) -> core::result::Result<bool, PartitionDecisionError> {
        let value = matches!(self.target, Partition::Horz4B | Partition::Vert4B);
        self.sink
            .literal(u32::from(value), 1)
            .map_err(PartitionDecisionError::Write)?;
        Ok(value)
    }
}

/// A `PARTITION_NONE` leaf (§ 5.20.4.1 `decode_block`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeafBlock {
    call: TilePartitionCall,
    frontier: DecodeBlockFrontier,
}

impl LeafBlock {
    /// Luma row in 4x4 units.
    #[must_use]
    pub const fn row4(&self) -> usize {
        self.frontier.r
    }

    /// Luma column in 4x4 units.
    #[must_use]
    pub const fn col4(&self) -> usize {
        self.frontier.c
    }

    /// § 9.3 block-size index.
    #[must_use]
    pub const fn block_size(&self) -> usize {
        self.frontier.b_size.index()
    }

    /// `HasChroma`: this leaf codes the chroma planes.
    #[must_use]
    pub const fn has_chroma(&self) -> bool {
        self.frontier.has_chroma
    }

    /// Luma `(width, height)` in samples.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid block size.
    pub fn dimensions(&self) -> Result<(usize, usize)> {
        PartitionNode(self.call).dimensions()
    }
}

/// One § 5.20.7.24 transform block, in plane samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransformBlock {
    /// A planned block holds the type the decoder infers: `Mode_To_Txfm` for
    /// chroma (§ 5.20.7.29), and `DCT_DCT` for luma, which may code another of
    /// [`IntraTileSyntax::allowed_luma_tx_types`]. The coefficient syntax reads
    /// the scan and the contexts of this type.
    pub tx_type: TxType,
    /// Plane index (`0` luma).
    pub plane: usize,
    /// Left sample in the plane.
    pub x: usize,
    /// Top sample in the plane.
    pub y: usize,
    /// § 9.3 transform-size index.
    pub tx_size: usize,
    /// The transform covers the plane residual block.
    pub fills_block: bool,
    /// The secondary transform of an intra luma block; a planned block has none.
    pub ist: Ist,
    /// `cctx_type` of a U block, `0..=6`; a planned block codes none.
    pub cctx: u8,
}

/// The `Level` grid of one transform block (§ 5.20.7.27), in adjusted
/// coordinates, with the scan and the transform class its contexts read.
#[derive(Debug)]
pub struct CoeffLevels {
    state: TransformCoeffBlockState,
    class: CoeffTransformClass,
    scan: &'static [u16],
    hidden: bool,
    /// `tcqState` before the next level of a TCQ block.
    tcq: Option<usize>,
}

impl CoeffLevels {
    /// A zero grid of `width x height` adjusted coefficients (each `4..=32`)
    /// coded with `tx_type`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid extent.
    pub fn new(width: usize, height: usize, tx_type: TxType) -> Result<Self> {
        let scan = type_scan(width, height, tx_type)?;
        TransformCoeffBlockState::new(width, height)
            .map(|state| Self {
                state,
                class: CoeffTransformClass::from_plane_tx_type(tx_type as usize),
                scan,
                hidden: false,
                tcq: None,
            })
            .map_err(rule("coefficient grid"))
    }

    /// A zero grid of the adjusted size and the type of `tx`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid transform size.
    pub fn for_block(tx: &TransformBlock) -> Result<Self> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        Self::new(1 << log2_w.min(5), 1 << log2_h.min(5), tx.tx_type)
    }

    /// The § 5.20.7.30 coefficient scan, as row-major adjusted positions.
    #[must_use]
    #[inline]
    pub const fn scan(&self) -> &'static [u16] {
        self.scan
    }

    /// Codes the levels as a block that hides the parity of its scan index 0
    /// (`isHidden` of § 5.20.7.27), from [`ParityHiding::sum_abs1`].
    pub const fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    /// Whether scan index `c` codes `quant >> 1`: the level whose parity the block hides.
    #[must_use]
    #[inline]
    pub const fn hides(&self, c: usize) -> bool {
        self.hidden && c == 0
    }

    /// Codes the levels as TCQ levels ([`IntraTileSyntax::uses_tcq`]) whose
    /// next level is coded in `state`; its quantizer picks the `coeff_base` rows.
    pub const fn set_tcq_state(&mut self, state: usize) {
        self.tcq = Some(state % tcq::TCQ_STATES);
    }

    /// The `tcqState` of [`Self::set_tcq_state`]; `None` for levels without TCQ.
    #[must_use]
    pub const fn tcq_state(&self) -> Option<usize> {
        self.tcq
    }

    /// The `(Level, remainder)` that code the magnitude `coded` at scan index
    /// `c` (`quant >> 1` when [`Self::hides`] `c`): `read_quant` (§ 5.20.7.28)
    /// reads the remainder once `Level` reaches `maxLevel - allowTcq`, and adds
    /// it shifted left by `allowTcq`.
    #[must_use]
    #[inline]
    pub fn split(
        &self,
        c: usize,
        coded: u32,
        is_low_frequency: bool,
        plane: usize,
    ) -> (u32, Option<u32>) {
        let tcq = u32::from(self.tcq.is_some());
        let threshold = self.max_level(c, is_low_frequency, plane) - tcq;
        if coded < threshold {
            return (coded, None);
        }
        let level = threshold + ((coded - threshold) & tcq);
        (level, Some((coded - level) >> tcq))
    }

    /// Copies the levels at scan indices `scan` from `from`, or zeroes them
    /// without `from`. The grids must have one shape.
    pub fn copy_cells(&mut self, from: Option<&Self>, scan: Range<usize>) {
        let (stride, log2_w) = (
            self.state.level_stride(),
            self.state.width().trailing_zeros(),
        );
        let level = self.state.level_mut();
        for &pos in self.scan.get(scan).unwrap_or_default() {
            let pos = usize::from(pos);
            let at = (pos >> log2_w) * stride + (pos & ((1 << log2_w) - 1));
            if let Some(slot) = level.get_mut(at) {
                *slot = from
                    .and_then(|from| from.state.level().get(at).copied())
                    .unwrap_or(0);
            }
        }
    }

    /// The largest scan distance from a coefficient to a level its `coeff_base`
    /// or `coeff_br` context reads (§ 8.3.2): the contexts at scan index `c`
    /// read only scan indices `c + 1..=c + context_reach()`.
    #[must_use]
    pub fn context_reach(&self) -> usize {
        static REACH: std::sync::OnceLock<[[[usize; 3]; 4]; 4]> = std::sync::OnceLock::new();
        let (log2_w, log2_h) = (
            self.state.width().trailing_zeros(),
            self.state.height().trailing_zeros(),
        );
        let class = tx_class_index(self.class);
        let table = REACH.get_or_init(|| {
            core::array::from_fn(|w| {
                core::array::from_fn(|h| {
                    [TxType::DctDct, TxType::HDct, TxType::VDct]
                        .map(|tx_type| scan_reach(4 << w, 4 << h, tx_type))
                })
            })
        });
        table[(log2_w as usize).saturating_sub(2).min(3)]
            [(log2_h as usize).saturating_sub(2).min(3)][class]
    }

    /// Stores `Level[row][col]`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] outside the grid.
    pub fn set(&mut self, row: usize, col: usize, level: u32) -> Result<()> {
        self.state
            .set_level(row, col, level.min(u32::from(u8::MAX)))
            .map_err(rule("coefficient grid"))
    }

    /// Whether the coefficient at `(row, col)` is low frequency for this grid's
    /// class (`get_lf_limits`, § 5.20.7.27).
    #[must_use]
    #[inline]
    pub fn is_low_frequency(&self, row: usize, col: usize, plane: usize) -> bool {
        coeff_is_low_frequency(CoeffScanEntry::new(0, 0, row, col), plane, self.class)
    }

    /// Whether the sign of a nonzero level at `(row, col)` is a CDF symbol
    /// (`dc_sign` or `dc_sign_horz_vert`) rather than an `L(1)` literal.
    #[must_use]
    #[inline]
    pub fn sign_has_cdf(&self, row: usize, col: usize, plane: usize) -> bool {
        if row > 0 && col > 0 {
            return false;
        }
        let entry = CoeffScanEntry::new(0, 0, row, col);
        matches!(
            derive_nonzero_coeff_sign_input(entry, 1, self.sign_config(plane)).source,
            CoeffSignReadSource::Cdf(_)
        )
    }

    /// The sign source of this grid, without the neighbour DC contexts.
    pub(super) const fn sign_config(&self, plane: usize) -> CoeffSignSourceDeriveConfig<'static> {
        CoeffSignSourceDeriveConfig {
            plane,
            plane_type: if plane > 0 { 1 } else { 0 },
            tx_class: self.class,
            is_hidden: self.hidden,
            sum_abs1: 0,
            above_dc: &[],
            left_dc: &[],
            x4: 0,
            y4: 0,
            w4: 0,
            h4: 0,
        }
    }

    /// `maxLevel` of § 5.20.7.27 at scan index `c` of a coefficient that
    /// [`Self::is_low_frequency`] puts in the low-frequency region or not.
    #[must_use]
    #[inline]
    pub fn max_level(&self, c: usize, is_low_frequency: bool, plane: usize) -> u32 {
        if self.hides(c) {
            let entry = CoeffScanEntry::new(c, 0, 0, 0);
            let config = CoeffMaxLevelConfig {
                plane,
                tx_class: self.class,
                is_hidden: true,
            };
            derive_coeff_max_level(entry, config)
        } else {
            visible_max_level(is_low_frequency, plane)
        }
    }
}

/// [`CoeffLevels::context_reach`] of a `width x height` grid coded with `tx_type`.
fn scan_reach(width: usize, height: usize, tx_type: TxType) -> usize {
    let Ok(scan) = type_scan(width, height, tx_type) else {
        return 0;
    };
    let class = tx_class_index(CoeffTransformClass::from_plane_tx_type(tx_type as usize));
    let mut index = vec![0usize; width * height];
    for (c, &pos) in scan.iter().enumerate() {
        if let Some(slot) = index.get_mut(usize::from(pos)) {
            *slot = c;
        }
    }
    let mut reach = 0;
    for (pos, &c) in index.iter().enumerate() {
        let (row, col) = (pos / width, pos % width);
        for [dr, dc] in super::cdf::coeff_context::context_offsets(class) {
            let (r, k) = (row + dr, col + dc);
            if r < height && k < width {
                reach = reach.max(index[r * width + k].saturating_sub(c));
            }
        }
    }
    reach
}

/// The § 6.19.7.23 bound: every coded `Quant` magnitude is below it.
pub const MAX_QUANT_MAGNITUDE: u32 = super::coeff_loop::quant_state::MAX_CONFORMING_QUANT_MAGNITUDE;

/// The longest § 5.20.7.28 `golomb_length_bit` prefix the decoder reads.
pub const MAX_GOLOMB_PREFIX_BITS: u32 = super::coeff_loop::read_quant::MAX_EXP_GOLOMB_PREFIX_BITS;

/// The § 5.20.7.30 coefficient scan of `tx` for its type, as row-major adjusted positions.
///
/// # Errors
/// Returns [`EncoderSyntaxError`] for an invalid transform size.
pub fn coefficient_scan(tx: &TransformBlock) -> Result<&'static [u16]> {
    let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
    type_scan(1 << log2_w.min(5), 1 << log2_h.min(5), tx.tx_type)
}

/// The § 5.20.7.30 coefficient scan of a `width x height` adjusted block
/// (each `4..=32`) coded with `tx_type`, as row-major positions.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] for an invalid extent.
pub fn type_scan(width: usize, height: usize, tx_type: TxType) -> Result<&'static [u16]> {
    coefficient_scan_slice(width, height, tx_class(tx_type as usize))
        .map_err(rule("coefficient scan"))
}

/// The decoder's dequantization and inverse-transform parameters of a DCT block.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] for an invalid shape.
pub fn dct_recon_params(
    qindex: u32,
    plane: PlaneId,
    log2_width: u32,
    log2_height: u32,
    bit_depth: BitDepth,
) -> Result<(DequantBlockParams, InverseTransform2dOuter)> {
    recon_params(
        qindex,
        plane,
        (log2_width, log2_height),
        (TxType::DctDct, false, false),
        bit_depth,
    )
}

/// The decoder's dequantization and inverse-transform parameters of a
/// `(log2 width, log2 height)` block of type `tx_type`, whose levels are TCQ
/// levels when `use_tcq` ([`IntraTileSyntax::uses_tcq`]) and whose kernels are
/// DDT kernels when `use_ddt` ([`IntraTileSyntax::uses_ddt`]).
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] for an invalid shape.
pub fn recon_params(
    qindex: u32,
    plane: PlaneId,
    (log2_width, log2_height): (u32, u32),
    (tx_type, use_tcq, use_ddt): (TxType, bool, bool),
    bit_depth: BitDepth,
) -> Result<(DequantBlockParams, InverseTransform2dOuter)> {
    block_recon_params(
        qindex,
        plane,
        (log2_width, log2_height),
        (tx_type as usize, use_tcq, use_ddt),
        bit_depth,
    )
    .map_err(rule("recon parameters"))
}

/// The number of tiles of the frame `core` (§ 5.18.7.2).
#[must_use]
pub fn tile_count(core: &FrameHeaderCore) -> usize {
    core.tile_info
        .as_ref()
        .map_or(1, |info| (info.tile_cols * info.tile_rows) as usize)
}

/// The luma 4x4 rows and columns of tile `tile`, in raster order, of the frame
/// `core`: `MiRowStarts` and `MiColStarts` of § 5.18.7.2.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Unsupported`] without tile info or for a tile
/// the frame does not have.
pub fn tile_ranges(core: &FrameHeaderCore, tile: usize) -> Result<(Range<usize>, Range<usize>)> {
    const NO_TILE: EncoderSyntaxError = EncoderSyntaxError::Unsupported("a tile of the frame");
    let info = core.tile_info.as_ref().ok_or(NO_TILE)?;
    let cols = (info.tile_cols as usize).max(1);
    let span = |starts: &[u32], index: usize| match (starts.get(index), starts.get(index + 1)) {
        (Some(&start), Some(&end)) => Ok(start as usize..end as usize),
        _ => Err(NO_TILE),
    };
    Ok((
        span(&info.mi_row_starts, tile / cols)?,
        span(&info.mi_col_starts, tile % cols)?,
    ))
}

/// The state a frame starts from, shared by its tiles: the CDFs (§ 5.18.2)
/// and, with `use_ref_frame_mvs`, the projected motion field (§ 7.9).
#[derive(Clone)]
pub struct FrameStart {
    cdfs: std::sync::Arc<FrameCdfSubset>,
    temporal: Option<std::sync::Arc<crate::prediction::inter::TemporalMvContext>>,
}

/// The CDFs and § 7.19 motion records of a coded tile after its last leaf,
/// which [`EncoderReferences::update`] folds into the saved state of the frame.
pub struct TileEnd {
    cdfs: TileCdfSubset,
    motion: Vec<crate::prediction::inter::TemporalMotionBlock>,
}

/// One tile's CDF rows, neighbour state, and reconstruction.
pub struct IntraTileSyntax {
    frame: TilePartitionFrameFacts,
    bounds: TilePartitionBounds,
    chroma: ChromaFormatIdc,
    sb_size4: usize,
    tools: intra::IntraTools,
    recon: intra::Recon,
    cdfs: TileCdfSubset,
    coeff: TileCoeffContextState,
    mi_sizes: TileMiSizeState,
    joint_modes: TileIntraJointModeState,
    uses_mrls: TileUsesMrlsState,
    use_dip: TileUseDipState,
    fsc_modes: TileFscModeState,
    palette_y: TileLumaPaletteState,
    uv_cfls: TileUvCflState,
    y_modes: TileIntraYModeState,
    y_mode_table: intra::YModeTable,
    inter: Option<inter::InterLeafState>,
    journal: CdfJournal,
    tile: (Range<usize>, Range<usize>),
    /// The planned leaf may code CCTX.
    cctx_leaf: bool,
    /// The planned intra leaf codes `fsc_mode`.
    fsc_leaf: bool,
}

impl IntraTileSyntax {
    /// State for tile `tile`, in raster order, of a key or inter frame coded
    /// with `seq` and `core`; an inter frame predicts from the frames in `refs`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for SDP, skip mode, missing
    /// header facts, a tile the frame does not have, or another tool the facade
    /// does not model, and [`EncoderSyntaxError::Rule`] for state setup failures.
    pub fn new(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        refs: &EncoderReferences,
        tile: usize,
    ) -> Result<Self> {
        Self::with_start(seq, core, refs, &refs.frame_start(core)?, tile)
    }

    /// [`Self::new`] for a frame that starts from the `start`
    /// [`EncoderReferences::frame_start`] returned for `core`, so the tiles
    /// of one frame share it.
    ///
    /// # Errors
    /// As [`Self::new`].
    pub fn with_start(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        refs: &EncoderReferences,
        start: &FrameStart,
        tile: usize,
    ) -> Result<Self> {
        if seq.partition.is_some_and(|partition| partition.enable_sdp) {
            return Err(EncoderSyntaxError::Unsupported(
                "semi-decoupled partitioning",
            ));
        }
        let base_q_idx = core
            .quantization_params
            .ok_or(EncoderSyntaxError::Unsupported(
                "missing quantization params",
            ))?
            .base_q_idx;
        let frame = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
        let (rows, cols) = tile_ranges(core, tile)?;
        let sb_size4 = frame
            .sb_size()
            .num_4x4_wide()
            .map_err(rule("superblock size"))?;
        let chroma = seq.general.chroma_format_idc;
        let tools = intra::IntraTools::new(seq, core, base_q_idx)?;
        let cdfs = start.cdfs.tile_copy();
        let journal = CdfJournal::new(&cdfs);
        let inter = inter::InterLeafState::new(
            seq,
            core,
            refs,
            start.temporal.clone(),
            (rows.clone(), cols.clone()),
            (base_q_idx, tools.bit_depth),
        )?;
        let mut coeff = TileCoeffContextState::default();
        coeff
            .reset_for_tile_chroma(rows.clone(), cols.clone(), chroma)
            .map_err(rule("coefficient contexts"))?;
        Ok(Self {
            frame,
            bounds: TilePartitionBounds::new(rows.clone(), cols.clone()),
            chroma,
            sb_size4,
            recon: intra::Recon::new(seq, core, &tools, (rows.clone(), cols.clone()), sb_size4)?,
            tools,
            cdfs,
            coeff,
            mi_sizes: TileMiSizeState::new_for_tile(rows.clone(), cols.clone(), frame.sb_size())
                .map_err(rule("MI-size state"))?,
            joint_modes: neighbour(TileIntraJointModeState::new_for_tile(
                rows.clone(),
                cols.clone(),
                sb_size4,
            ))?,
            uses_mrls: neighbour(TileUsesMrlsState::new_for_tile(
                rows.clone(),
                cols.clone(),
                sb_size4,
            ))?,
            use_dip: neighbour(TileUseDipState::new_for_tile(
                rows.clone(),
                cols.clone(),
                sb_size4,
            ))?,
            fsc_modes: neighbour(TileFscModeState::new_for_tile(
                rows.clone(),
                cols.clone(),
                sb_size4,
            ))?,
            palette_y: neighbour(TileLumaPaletteState::new_for_tile(
                rows.clone(),
                cols.clone(),
                sb_size4,
            ))?,
            uv_cfls: neighbour(
                TileUvCflState::new(rows.len(), cols.len(), sb_size4)
                    .map(|state| state.with_origin(rows.start, cols.start)),
            )?,
            y_modes: neighbour(
                TileIntraYModeState::new(rows.len(), cols.len(), sb_size4)
                    .map(|state| state.with_origin(rows.start, cols.start)),
            )?,
            y_mode_table: intra::YModeTable::default(),
            inter,
            journal,
            tile: (rows, cols),
            cctx_leaf: false,
            fsc_leaf: false,
        })
    }

    /// The luma 4x4 rows and columns of this tile.
    #[must_use]
    pub fn tile_ranges(&self) -> (Range<usize>, Range<usize>) {
        self.tile.clone()
    }

    /// Superblock side in 4x4 units.
    #[must_use]
    pub const fn superblock_size4(&self) -> usize {
        self.sb_size4
    }

    /// `MiRows` and `MiCols`.
    #[must_use]
    pub const fn mi_dimensions(&self) -> (usize, usize) {
        (self.frame.mi_rows(), self.frame.mi_cols())
    }

    /// The root node of the superblock at luma 4x4 position `(row4, col4)`.
    #[must_use]
    pub const fn superblock_root(&self, row4: usize, col4: usize) -> PartitionNode {
        PartitionNode(TilePartitionCall::root(
            row4,
            col4,
            self.frame.sb_size(),
            ROOT_HAS_CHROMA,
        ))
    }

    /// Starts coding the superblock of `root`; call once per superblock in raster order.
    pub fn begin_superblock(&mut self, root: PartitionNode) {
        self.journal.clear();
        if root.0.c == self.tile.1.start {
            self.mi_sizes.enter_sb_row(root.0.r);
        }
        self.recon
            .block_decoded
            .clear_superblock(root.0.r, root.0.c);
        self.enter_inter_superblock(root.0.r);
    }

    /// Whether `node` starts inside the frame; nodes outside code nothing.
    #[must_use]
    pub const fn in_frame(&self, node: PartitionNode) -> bool {
        call_in_frame(self.frame, node.0)
    }

    /// The partitions `node` allows.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid node.
    pub fn partition_options(&self, node: PartitionNode) -> Result<PartitionOptions> {
        let input = partition_allowed_input(node.0, self.frame, None).map_err(rule("partition"))?;
        partition_decision_facts(input)
            .map(PartitionOptions)
            .map_err(rule("partition"))
    }

    /// Writes the § 5.20.3.2 `read_partition` symbols that select `partition`
    /// for `node`: the decoder's own read runs with each symbol answered for
    /// `partition`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] when `node` cannot code
    /// `partition`, after any symbols the read took, and
    /// [`EncoderSyntaxError::Rule`] for a context or write failure.
    pub fn write_partition(
        &mut self,
        node: PartitionNode,
        partition: Partition,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let options = self.partition_options(node)?;
        if !options.allows(partition) {
            return Err(EncoderSyntaxError::Unsupported("partition not allowed"));
        }
        let (partition_context, square_context) =
            partition_context_inputs(node.0, self.bounds, self.mi_sizes.context_state())
                .map_err(rule("partition context"))?;
        let input =
            options
                .0
                .read_partition_decision_input(true, partition_context, square_context);
        let mut writer = PartitionWriter {
            target: partition,
            sink: symbols,
            journal: &mut self.journal,
        };
        let coded = read_partition_decision(input, &mut self.cdfs, &mut writer)
            .map_err(rule("partition"))?;
        if coded == partition {
            Ok(())
        } else {
            Err(EncoderSyntaxError::Unsupported(
                "the node codes another partition",
            ))
        }
    }

    /// The child nodes of a non-`NONE` partition of `node`, in syntax order.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid partition of `node`.
    pub fn children(
        &self,
        node: PartitionNode,
        partition: Partition,
    ) -> Result<Vec<PartitionNode>> {
        let sub_size = valid_subsize(partition, node.0.b_size).map_err(rule("partition"))?;
        let chroma_offset = updated_chroma_offset(node.0, partition, sub_size, self.frame)
            .map_err(rule("partition"))?;
        let calls = child_calls(node.0, partition, sub_size, self.frame, chroma_offset)
            .map_err(rule("partition"))?;
        Ok(calls
            .as_slice()
            .iter()
            .copied()
            .map(PartitionNode)
            .collect())
    }

    /// The leaf block of `node` coded with `PARTITION_NONE`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid node.
    pub fn leaf(&self, node: PartitionNode) -> Result<LeafBlock> {
        let call = node.0;
        let chroma_offset = updated_chroma_offset(call, Partition::None, call.b_size, self.frame)
            .map_err(rule("partition"))?;
        let frontier = decode_block_frontier(
            call,
            self.frame,
            call.b_size,
            chroma_offset,
            DecodeBlockPart::Shared,
        );
        Ok(LeafBlock { call, frontier })
    }

    /// The `y_mode_set` CDF row.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a CDF table defect.
    pub fn y_mode_set_cdf(&mut self) -> Result<&mut [u16]> {
        self.tile_row(TileCdfSelector::YModeSet)
    }

    /// The `y_mode_index` CDF row of `leaf`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid geometry.
    pub fn y_mode_index_cdf(&mut self, leaf: &LeafBlock) -> Result<&mut [u16]> {
        let ctx = self.y_mode_ctx(leaf)?;
        self.tile_row(TileCdfSelector::YModeIndex { ctx })
    }

    /// The `y_mode_offset` CDF row of `leaf`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid geometry.
    pub fn y_mode_offset_cdf(&mut self, leaf: &LeafBlock) -> Result<&mut [u16]> {
        let ctx = self.y_mode_ctx(leaf)?;
        self.tile_row(TileCdfSelector::YModeOffset { ctx })
    }

    /// The `uv_mode` CDF row (CFL not allowed) for a luma mode that is or is not directional.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a CDF table defect.
    pub fn uv_mode_cdf(&mut self, y_mode_directional: bool) -> Result<&mut [u16]> {
        let y_mode = if y_mode_directional {
            IntraYMode::Vertical
        } else {
            IntraYMode::Dc
        };
        self.tile_row(TileCdfSelector::UvModeCflNotAllowed {
            ctx: uv_mode_ctx(y_mode),
        })
    }

    /// The `all_zero` CDF row of `tx`; `eob_u_nonzero` is `EobU != 0` for the V block.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] outside the tile.
    pub fn txb_skip_cdf(&mut self, tx: &TransformBlock, eob_u_nonzero: bool) -> Result<&mut [u16]> {
        let selector = txb_skip_selector(
            &self.coeff,
            tx.plane,
            tx.tx_size,
            tx_area4(tx)?,
            tx.fills_block,
            eob_u_nonzero,
            usize::from(self.leaf_is_inter() || self.fsc_leaf),
            self.fsc_leaf && self.tools.enable_fsc,
        )
        .map_err(rule("all_zero context"))?;
        self.tile_row(selector)
    }

    /// The `eob_pt_*` CDF row of a transform block of the planned leaf.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid transform size.
    pub fn eob_pt_cdf(&mut self, plane: usize, tx_size: usize) -> Result<&mut [u16]> {
        let (log2_w, log2_h) = tx_log2(tx_size)?;
        let input = nonzero_coeff_eob_symbol_input(NonZeroCoeffEobContextInput {
            plane,
            is_inter: self.leaf_is_inter(),
            tx_width_log2: log2_w as usize,
            tx_height_log2: log2_h as usize,
        })
        .map_err(rule("eob context"))?;
        self.tile_row(TileCdfSelector::EobPt {
            size: input.size,
            eob_ctx: input.eob_ctx,
        })
    }

    /// The `eob_extra` CDF row.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a CDF table defect.
    pub fn eob_extra_cdf(&mut self) -> Result<&mut [u16]> {
        self.tile_row(TileCdfSelector::EobExtra {})
    }

    /// The `coeff_base_eob` CDF row for scan index `c` of a block with adjusted width
    /// `1 << adjusted_log2_w` and adjusted height `adjusted_h`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a CDF table defect.
    pub fn coeff_base_eob_cdf(
        &mut self,
        plane: usize,
        tx_size: usize,
        c: usize,
        (adjusted_log2_w, adjusted_h): (u32, usize),
        is_lf: bool,
    ) -> Result<&mut [u16]> {
        let ctx = coeff_base_eob_ctx(c, adjusted_log2_w, adjusted_h);
        let tx_size = txb_skip_tx_size_ctx(tx_size);
        let selector = match (plane > 0, is_lf) {
            (true, true) => CoeffCdfSelector::BaseLfEobUv { ctx },
            (true, false) => CoeffCdfSelector::BaseEobUv { ctx },
            (false, true) => CoeffCdfSelector::BaseLfEob { tx_size, ctx },
            (false, false) => CoeffCdfSelector::BaseEob { tx_size, ctx },
        };
        let (base, _) = self.cdfs.rows_span();
        let row = self
            .cdfs
            .coeff_rows_mut()
            .row_mut(selector)
            .map_err(rule("coeff_base_eob row"))?;
        self.journal.log(base, CdfRow::Coeff(selector), row);
        Ok(row)
    }

    /// The `coeff_base` CDF row for the coefficient at adjusted `(row, col)`, scan index `c`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a CDF table defect.
    pub fn coeff_base_cdf(
        &mut self,
        levels: &CoeffLevels,
        plane: usize,
        tx_size: usize,
        (row, col, c): (usize, usize, usize),
        is_lf: bool,
    ) -> Result<&mut [u16]> {
        let context = CoeffBaseContext {
            row,
            col,
            stride: levels.state.level_stride(),
            plane,
            is_lf,
            is_hidden: false,
            c,
            tx_class: tx_class_index(levels.class),
        };
        let selection = if levels.hides(c) {
            hidden_base_selection(context, levels.state.level())
        } else {
            context.select(levels.state.level())
        };
        let (base, _) = self.cdfs.rows_span();
        let tx_size = txb_skip_tx_size_ctx(tx_size);
        let quantizer = levels.tcq.map_or(0, tcq_quantizer);
        let row = match self
            .cdfs
            .coeff_rows_mut()
            .base_row(selection, tx_size, quantizer)
            .map_err(rule("coeff_base row"))?
        {
            CoeffBaseRow::Base(row) => row.as_mut_slice(),
            CoeffBaseRow::Lf(row) => row.as_mut_slice(),
        };
        self.journal
            .log(base, CdfRow::Base(selection, tx_size, quantizer), row);
        Ok(row)
    }

    /// The `coeff_br` CDF row for the coefficient at adjusted `(row, col)`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a CDF table defect.
    pub fn coeff_br_cdf(
        &mut self,
        levels: &CoeffLevels,
        plane: usize,
        (row, col): (usize, usize),
        is_lf: bool,
    ) -> Result<&mut [u16]> {
        let ctx = CoeffBrContext {
            row,
            col,
            stride: levels.state.level_stride(),
            plane,
            is_lf,
            tx_class: tx_class_index(levels.class),
        }
        .ctx(levels.state.level());
        let (base, _) = self.cdfs.rows_span();
        let row = self
            .cdfs
            .coeff_rows_mut()
            .br_row(plane, is_lf, ctx)
            .map_err(rule("coeff_br row"))?;
        self.journal.log(base, CdfRow::Br(plane, is_lf, ctx), row);
        Ok(row)
    }

    /// The CDF row of the sign of the nonzero coefficient at adjusted `(row, col)`
    /// of `tx` coded as `levels`, or `None` when the sign is an `L(1)` literal.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] outside the tile.
    pub fn sign_cdf(
        &mut self,
        tx: &TransformBlock,
        levels: &CoeffLevels,
        (row, col): (usize, usize),
    ) -> Result<Option<&mut [u16]>> {
        let plane = tx.plane;
        if !levels.sign_has_cdf(row, col, plane) {
            return Ok(None);
        }
        let entry = CoeffScanEntry::new(0, 0, row, col);
        let mut config = levels.sign_config(plane);
        let [x4, y4, w4, h4] = tx_area4(tx)?;
        let context = |error| rule("sign context")(error);
        config.above_dc = self.coeff.above_dc(plane).map_err(context)?;
        config.left_dc = self.coeff.left_dc(plane).map_err(context)?;
        config.x4 = self.coeff.local_x4(plane, x4).map_err(context)?;
        config.y4 = self.coeff.local_y4(plane, y4).map_err(context)?;
        (config.w4, config.h4) = (w4, h4);
        match derive_nonzero_coeff_sign_input(entry, 1, config).source {
            CoeffSignReadSource::Cdf(selector) => self.tile_row(selector.tile_selector()).map(Some),
            CoeffSignReadSource::None | CoeffSignReadSource::SignBit => Ok(None),
        }
    }

    /// Records `culLevel` and `dcCategory` of `tx` in the coefficient contexts.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] outside the tile.
    pub fn record_coefficients(
        &mut self,
        tx: &TransformBlock,
        cul_level: u8,
        dc_category: u8,
    ) -> Result<()> {
        let [x4, y4, w4, h4] = tx_area4(tx)?;
        self.coeff
            .update_after_coeffs(CoeffContextUpdate {
                plane: tx.plane,
                x4,
                y4,
                w4,
                h4,
                cul_level,
                dc_category,
            })
            .map_err(rule("coefficient contexts"))
    }

    fn y_mode_ctx(&self, leaf: &LeafBlock) -> Result<usize> {
        let b_size: BlockSize = leaf.frontier.b_size;
        let n4w = b_size.num_4x4_wide().map_err(rule("block size"))?;
        let n4h = b_size.num_4x4_high().map_err(rule("block size"))?;
        Ok(self
            .joint_modes
            .y_mode_index_ctx(leaf.frontier.r, leaf.frontier.c, n4w, n4h))
    }

    fn tile_row(&mut self, selector: TileCdfSelector) -> Result<&mut [u16]> {
        let (base, _) = self.cdfs.rows_span();
        let row = self.cdfs.row_mut(selector).map_err(rule("CDF row"))?;
        self.journal.log(base, CdfRow::Tile(selector), row);
        Ok(row)
    }
}

/// The `coeff_base` row selection of a level whose parity the block hides.
#[inline(never)]
fn hidden_base_selection(context: CoeffBaseContext, level: &[u8]) -> CoeffBaseSelection {
    CoeffBaseContext {
        is_hidden: true,
        ..context
    }
    .select(level)
}

fn neighbour<T, E: std::error::Error + Send + Sync + 'static>(
    state: core::result::Result<T, E>,
) -> Result<T> {
    state.map_err(rule("neighbour state"))
}

/// The `(Tx_Width_Log2, Tx_Height_Log2)` of a § 9.3 transform size.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Unsupported`] for an unknown size.
pub fn tx_log2(tx_size: usize) -> Result<(u32, u32)> {
    use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
    match (TX_WIDTH_LOG2.get(tx_size), TX_HEIGHT_LOG2.get(tx_size)) {
        (Some(&w), Some(&h)) => Ok((w as u32, h as u32)),
        _ => Err(EncoderSyntaxError::Unsupported("transform size")),
    }
}

fn tx_area4(tx: &TransformBlock) -> Result<[usize; 4]> {
    let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
    Ok([
        tx.x >> 2,
        tx.y >> 2,
        (1usize << log2_w) >> 2,
        (1usize << log2_h) >> 2,
    ])
}
