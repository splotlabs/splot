// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder-facing syntax facade; not a stable API.
//!
//! [`IntraTileSyntax`] owns one key-frame tile's CDF rows, neighbour state, and
//! reconstruction, and hands an encoder the same partition rules, § 8.3.2
//! contexts, CDF rows, and prediction kernels the tile decoder uses. The encoder
//! writes each symbol into the returned row, reconstructs each transform block
//! through the facade, then publishes the coded facts back, so both sides stay
//! in step.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_recon::{
    BitDepth, DequantBlockParams, InverseTransform2dOuter, PlaneId, coefficient_scan_slice,
    tx_class,
};

use super::cdf::block_context::{IntraYMode, uv_mode_ctx};
use super::cdf::coeff_context::{
    CoeffBaseContext, CoeffBrContext, coeff_base_eob_ctx, dc_sign_ctx,
};
pub use super::cdf::context::RectPartitionType as RectKind;
use super::cdf::{CoeffBaseRow, CoeffCdfSelector, FrameCdfSubset, TileCdfSelector, TileCdfSubset};
use super::coeff_loop::base_level_pass::tx_class_index;
use super::coeff_loop::max_level::{CoeffTransformClass, coeff_is_low_frequency};
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
use super::partition_allowed::{PartitionDecisionFacts, partition_decision_facts};
use super::partition_size::BlockSize;
use super::partition_traversal::{
    DecodeBlockFrontier, DecodeBlockPart, ROOT_HAS_CHROMA, TilePartitionBounds, TilePartitionCall,
    TilePartitionFrameFacts, call_in_frame, child_calls, decode_block_frontier,
    partition_allowed_input, partition_context_inputs, updated_chroma_offset, valid_subsize,
};
use super::tile_frontier::{frame_mi_dimensions, minimal_partition_frame_facts};

mod intra;
mod transform;

pub use super::cdf::block_context::IntraYMode as IntraMode;
pub use intra::{ChromaMode, IntraModes, MAX_CFL_ALPHA, TxType};
pub use transform::{TX_PARTITIONS, TxPartition};

#[path = "encoder_filters.rs"]
mod filters;
pub use filters::{
    CdefIndexMap, CdefSlot, FilterRecorder, LrLayout, LrUnit, apply_loop_filters, cdef_unit_grid,
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

    /// The rectangle direction the block size implies.
    #[must_use]
    pub const fn rect_type(self) -> Option<RectKind> {
        self.0.rect_type()
    }
}

/// A partition CDF of § 5.20.3.2.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionCdf {
    /// `do_split`.
    DoSplit,
    /// `do_square_split`.
    DoSquareSplit,
    /// `rect_type`.
    RectType,
    /// `do_ext_partition` for a direction.
    DoExtPartition(RectKind),
    /// `do_uneven_4way_partition` for a direction.
    DoUneven4Way(RectKind),
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
}

/// The `Level` grid of one transform block (§ 5.20.7.27), in adjusted
/// coordinates, with the transform class its contexts read.
#[derive(Debug)]
pub struct CoeffLevels {
    state: TransformCoeffBlockState,
    class: CoeffTransformClass,
}

impl CoeffLevels {
    /// A zero 2D-class grid of `width x height` adjusted coefficients (each `4..=32`).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid extent.
    pub fn new(width: usize, height: usize) -> Result<Self> {
        TransformCoeffBlockState::new(width, height)
            .map(|state| Self {
                state,
                class: CoeffTransformClass::TwoD,
            })
            .map_err(rule("coefficient grid"))
    }

    /// A zero grid of the adjusted size and the transform class of `tx`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid transform size.
    pub fn for_block(tx: &TransformBlock) -> Result<Self> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let mut grid = Self::new(1 << log2_w.min(5), 1 << log2_h.min(5))?;
        grid.class = CoeffTransformClass::from_plane_tx_type(tx.tx_type as usize);
        Ok(grid)
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
    pub fn is_low_frequency(&self, row: usize, col: usize, plane: usize) -> bool {
        coeff_is_low_frequency(CoeffScanEntry::new(0, 0, row, col), plane, self.class)
    }
}

/// Whether a 2D-class coefficient is low frequency (`get_lf_limits`, § 5.20.7.27).
#[must_use]
pub fn is_low_frequency(row: usize, col: usize, plane: usize) -> bool {
    coeff_is_low_frequency(
        CoeffScanEntry::new(0, 0, row, col),
        plane,
        CoeffTransformClass::TwoD,
    )
}

/// The § 5.20.7.30 coefficient scan of `tx` for its type, as row-major adjusted positions.
///
/// # Errors
/// Returns [`EncoderSyntaxError`] for an invalid transform size.
pub fn coefficient_scan(tx: &TransformBlock) -> Result<&'static [u16]> {
    let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
    coefficient_scan_slice(
        1 << log2_w.min(5),
        1 << log2_h.min(5),
        tx_class(tx.tx_type as usize),
    )
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
        TxType::DctDct,
        bit_depth,
    )
}

/// The decoder's dequantization and inverse-transform parameters of a
/// `(log2 width, log2 height)` block of type `tx_type`.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] for an invalid shape.
pub fn recon_params(
    qindex: u32,
    plane: PlaneId,
    (log2_width, log2_height): (u32, u32),
    tx_type: TxType,
    bit_depth: BitDepth,
) -> Result<(DequantBlockParams, InverseTransform2dOuter)> {
    block_recon_params(
        qindex,
        plane,
        log2_width,
        log2_height,
        tx_type as usize,
        bit_depth,
    )
    .map_err(rule("recon parameters"))
}

/// A saved [`IntraTileSyntax`] context plus the reconstruction of one region, so
/// a search can code a trial and roll it back without copying the frame.
pub struct SyntaxSnapshot {
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
    recon: intra::ReconSnapshot,
}

/// One key-frame tile's CDF rows, neighbour state, and reconstruction.
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
}

impl IntraTileSyntax {
    /// State for the single tile of a key frame coded with `seq` and `core`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for SDP, a non-intra frame, or
    /// missing header facts, and [`EncoderSyntaxError::Rule`] for state setup failures.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Self> {
        if core.frame_is_intra != Some(true) {
            return Err(EncoderSyntaxError::Unsupported("non-intra frame"));
        }
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
        let (mi_rows, mi_cols) = frame_mi_dimensions(core).map_err(rule("frame size"))?;
        let rows = 0..mi_rows;
        let cols = 0..mi_cols;
        let sb_size4 = frame
            .sb_size()
            .num_4x4_wide()
            .map_err(rule("superblock size"))?;
        let chroma = seq.general.chroma_format_idc;
        let tools = intra::IntraTools::new(seq, core, base_q_idx)?;
        let mut coeff = TileCoeffContextState::default();
        coeff
            .reset_for_tile_chroma(rows.clone(), cols.clone(), chroma)
            .map_err(rule("coefficient contexts"))?;
        Ok(Self {
            frame,
            bounds: TilePartitionBounds::new(rows.clone(), cols.clone()),
            chroma,
            sb_size4,
            recon: intra::Recon::new(seq, core, &tools, (mi_rows, mi_cols), sb_size4)?,
            tools,
            cdfs: FrameCdfSubset::default_for_base_q(base_q_idx).tile_copy(),
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
            uv_cfls: neighbour(TileUvCflState::new(mi_rows, mi_cols, sb_size4))?,
            y_modes: neighbour(TileIntraYModeState::new(mi_rows, mi_cols, sb_size4))?,
        })
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
        if root.0.c == 0 {
            self.mi_sizes.enter_sb_row(root.0.r);
        }
        self.recon
            .block_decoded
            .clear_superblock(root.0.r, root.0.c);
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

    /// The CDF row of one partition symbol of `node`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] when the context is invalid.
    pub fn partition_cdf(&mut self, node: PartitionNode, cdf: PartitionCdf) -> Result<&mut [u16]> {
        let (partition, square) =
            partition_context_inputs(node.0, self.bounds, self.mi_sizes.context_state())
                .map_err(rule("partition context"))?;
        let selector = match cdf {
            PartitionCdf::DoSplit => partition.do_split_selector(),
            PartitionCdf::DoSquareSplit => square.do_square_split_selector(),
            PartitionCdf::RectType => partition.rect_type_selector(),
            PartitionCdf::DoExtPartition(kind) => partition.do_ext_partition_selector(kind),
            PartitionCdf::DoUneven4Way(kind) => partition.do_uneven_4way_partition_selector(kind),
        }
        .map_err(rule("partition context"))?;
        self.tile_row(selector)
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
            0,
            false,
        )
        .map_err(rule("all_zero context"))?;
        self.tile_row(selector)
    }

    /// The `eob_pt_*` CDF row of an intra transform block.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid transform size.
    pub fn eob_pt_cdf(&mut self, plane: usize, tx_size: usize) -> Result<&mut [u16]> {
        let (log2_w, log2_h) = tx_log2(tx_size)?;
        let input = nonzero_coeff_eob_symbol_input(NonZeroCoeffEobContextInput {
            plane,
            is_inter: false,
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
        self.cdfs
            .coeff_rows_mut()
            .row_mut(selector)
            .map_err(rule("coeff_base_eob row"))
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
        let selection = CoeffBaseContext {
            row,
            col,
            stride: levels.state.level_stride(),
            plane,
            is_lf,
            is_hidden: false,
            c,
            tx_class: tx_class_index(levels.class),
        }
        .select(levels.state.level());
        let row = self
            .cdfs
            .coeff_rows_mut()
            .base_row(selection, txb_skip_tx_size_ctx(tx_size), 0)
            .map_err(rule("coeff_base row"))?;
        Ok(match row {
            CoeffBaseRow::Base(row) => row.as_mut_slice(),
            CoeffBaseRow::Lf(row) => row.as_mut_slice(),
        })
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
        self.cdfs
            .coeff_rows_mut()
            .br_row(plane, is_lf, ctx)
            .map(<[u16; _]>::as_mut_slice)
            .map_err(rule("coeff_br row"))
    }

    /// The luma `dc_sign` CDF row of `tx`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] outside the tile.
    pub fn dc_sign_cdf(&mut self, tx: &TransformBlock) -> Result<&mut [u16]> {
        let [x4, y4, w4, h4] = tx_area4(tx)?;
        let plane = tx.plane;
        let local_x4 = self
            .coeff
            .local_x4(plane, x4)
            .map_err(rule("dc_sign context"))?;
        let local_y4 = self
            .coeff
            .local_y4(plane, y4)
            .map_err(rule("dc_sign context"))?;
        let above = self
            .coeff
            .above_dc(plane)
            .map_err(rule("dc_sign context"))?;
        let left = self.coeff.left_dc(plane).map_err(rule("dc_sign context"))?;
        let ctx = dc_sign_ctx(above, left, local_x4, local_y4, w4, h4);
        self.tile_row(TileCdfSelector::DcSign {
            plane_type: usize::from(plane > 0),
            group: 0,
            ctx,
        })
    }

    /// The CDF row of the sign of the nonzero coefficient at adjusted `(row, col)`
    /// of `tx`, or `None` when the sign is an `L(1)` literal.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] outside the tile.
    pub fn sign_cdf(
        &mut self,
        tx: &TransformBlock,
        row: usize,
        col: usize,
    ) -> Result<Option<&mut [u16]>> {
        let [x4, y4, w4, h4] = tx_area4(tx)?;
        let plane = tx.plane;
        let input = derive_nonzero_coeff_sign_input(
            CoeffScanEntry::new(0, 0, row, col),
            1,
            CoeffSignSourceDeriveConfig {
                plane,
                plane_type: usize::from(plane > 0),
                tx_class: CoeffTransformClass::from_plane_tx_type(tx.tx_type as usize),
                is_hidden: false,
                sum_abs1: 0,
                above_dc: self.coeff.above_dc(plane).map_err(rule("sign context"))?,
                left_dc: self.coeff.left_dc(plane).map_err(rule("sign context"))?,
                x4: self
                    .coeff
                    .local_x4(plane, x4)
                    .map_err(rule("sign context"))?,
                y4: self
                    .coeff
                    .local_y4(plane, y4)
                    .map_err(rule("sign context"))?,
                w4,
                h4,
            },
        );
        match input.source {
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
        self.cdfs.row_mut(selector).map_err(rule("CDF row"))
    }
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
