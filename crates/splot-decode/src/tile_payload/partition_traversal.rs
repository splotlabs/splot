// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 partition traversal frontier.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`.

use splot_core::symbol::{SymbolDecoder, SymbolDecoderCheckpoint, SymbolDecoderConfig};

use super::DecodeTileWorkUnit;
use super::block_decoded_state::TileBlockDecodedState;
use super::cdf::TileCdfError;
use super::cdf::block_context::IntraYMode;
use super::cdf::context::{PartitionContextInput, SquareSplitContextInput};
use super::intra_joint_modes::{
    TileFscModeState, TileFscModeStateError, TileIntraJointModeState, TileIntraYModeState,
    TileIntraYModeStateError, TileUsesMrlsState, TileUsesMrlsStateError,
};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition::{PartitionDecisionError, PartitionType, ReadPartitionDecision};
use super::partition_allowed::{
    PartitionAllowedError, PartitionAllowedInput, PartitionFeatureFlags, PartitionTreeType,
    partition_decision_facts,
};
use super::partition_size::{
    BlockSize, PartitionSizeError, PartitionSubsize, h_partition_midsize, partition_subsize,
};
use crate::{DecodeLimitError, DecodeLimitName, DecodeLimits};

const BLOCK_8X32: usize = 21;
const BLOCK_32X8: usize = 22;
const BLOCK_64X64: usize = 12;
const MI_SIZE: usize = 4;
const LR_BANK_SIZE: usize = 4;
const WIENER_NS_LUMA_COEFFS: usize = 16;
const WIENER_NS_CHROMA_COEFFS: usize = 18;
const WIENER_NS_SHORT_COEFFS: usize = 6;
const WIENER_NS_LUMA_SUBSETS: usize = 4;
const WIENER_NS_CHROMA_SUBSETS: usize = 3;
const WIENER_NS_TAPS_K: [[u8; WIENER_NS_CHROMA_COEFFS]; 2] = [
    [6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [6, 6, 5, 5, 5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4],
];
const WIENER_NS_TAPS_PRESENT: [[[bool; WIENER_NS_CHROMA_COEFFS]; WIENER_NS_LUMA_SUBSETS]; 2] = [
    [
        [
            true, true, true, true, true, true, false, false, false, false, false, false, false,
            false, false, false, false, false,
        ],
        [
            true, true, false, false, false, false, true, true, true, true, true, true, false,
            false, false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, false, false,
            false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, false, false,
        ],
    ],
    [
        [
            true, true, true, true, true, true, false, false, false, false, false, false, false,
            false, false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, false, false, false, false,
            false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, true, true,
        ],
        [false; WIENER_NS_CHROMA_COEFFS],
    ],
];

/// Decoder support matrix row for this traversal frontier.
pub(crate) const TILE_PARTITION_TRAVERSAL_MATRIX_ROW: &str = "tile-partition-traversal-boundary";
/// Implementation-matrix feature id for this traversal frontier.
pub(crate) const TILE_PARTITION_TRAVERSAL_FEATURE_ID: &str =
    "DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY";

/// AV2 partition-context state read by the traversal frontier.
// `*mi_sizes` names mirror the AV2 MiSizes/LeftMiSizes/AboveMiSizes context arrays; the suffix preserves that mapping.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionContextState<'a> {
    mi_sizes: [&'a [&'a [usize]]; 2],
    left_mi_sizes: [&'a [usize]; 2],
    above_mi_sizes: [&'a [usize]; 2],
}

impl<'a> TilePartitionContextState<'a> {
    /// Creates a read-only context-state view for § 8.3.2 partition selectors.
    #[must_use]
    pub(crate) const fn new(
        mi_sizes: [&'a [&'a [usize]]; 2],
        left_mi_sizes: [&'a [usize]; 2],
        above_mi_sizes: [&'a [usize]; 2],
    ) -> Self {
        Self {
            mi_sizes,
            left_mi_sizes,
            above_mi_sizes,
        }
    }
}

/// BRU state supported by the traversal frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionBruState {
    /// Normal non-BRU path: `bru_mode == BRU_ACTIVE`.
    Active,
    /// BRU, bridge, or inactive paths remain outside this frontier.
    Unsupported,
}

/// Loop-restoration syntax state supported by the traversal frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionLoopRestorationState {
    /// No §5.20.10.4 `read_lr()` syntax is needed before partition reads.
    NoSyntax,
    /// Narrow frame-level Wiener NS LR unit syntax is supported before partition reads.
    FrameWienerNs(TilePartitionWienerNsLoopRestorationState),
    /// Root `read_lr()` syntax remains outside this frontier.
    UnsupportedReadLrSyntax,
}

/// Narrow frame-level Wiener NS LR state supported by the traversal frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionWienerNsLoopRestorationState {
    plane_enabled: [bool; 3],
    frame_filters_on: [bool; 3],
    unit_size: [usize; 3],
}

impl TilePartitionWienerNsLoopRestorationState {
    /// Creates checked-copyable Wiener NS LR unit facts for the active frame.
    #[must_use]
    pub(crate) const fn new(
        plane_enabled: [bool; 3],
        frame_filters_on: [bool; 3],
        unit_size: [usize; 3],
    ) -> Self {
        Self {
            plane_enabled,
            frame_filters_on,
            unit_size,
        }
    }
}

/// Frame and sequence facts required by the traversal frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionFrameFacts {
    mi_rows: usize,
    mi_cols: usize,
    sb_size: BlockSize,
    num_planes: usize,
    subsampling_x: bool,
    subsampling_y: bool,
    frame_is_intra: bool,
    enable_sdp: bool,
    enable_extended_sdp: bool,
    disable_loopfilters_across_tiles: bool,
    loop_restoration: TilePartitionLoopRestorationState,
    features: PartitionFeatureFlags,
    max_pb_aspect_ratio: usize,
    has_chroma: bool,
    bru_state: TilePartitionBruState,
}

impl TilePartitionFrameFacts {
    /// Creates checked frame facts for the traversal frontier.
    // Each bool is a distinct AV2 frame-level syntax flag; bundling them would obscure the spec mapping.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size: usize,
        num_planes: usize,
        subsampling_x: bool,
        subsampling_y: bool,
        frame_is_intra: bool,
        enable_sdp: bool,
        enable_extended_sdp: bool,
        disable_loopfilters_across_tiles: bool,
        loop_restoration: TilePartitionLoopRestorationState,
        features: PartitionFeatureFlags,
        max_pb_aspect_ratio: usize,
        has_chroma: bool,
        bru_state: TilePartitionBruState,
    ) -> Result<Self, TilePartitionTraversalError> {
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size: BlockSize::new(sb_size)?,
            num_planes,
            subsampling_x,
            subsampling_y,
            frame_is_intra,
            enable_sdp,
            enable_extended_sdp,
            disable_loopfilters_across_tiles,
            loop_restoration,
            features,
            max_pb_aspect_ratio,
            has_chroma,
            bru_state,
        })
    }

    /// Superblock size used by this frame's tile partition traversal.
    #[must_use]
    pub(crate) const fn sb_size(&self) -> BlockSize {
        self.sb_size
    }
}

/// Input to the crate-private traversal frontier.
#[derive(Debug)]
pub(crate) struct TilePartitionTraversalInput<'work, 'payload, 'ctx> {
    work_unit: &'work mut DecodeTileWorkUnit<'payload>,
    frame: TilePartitionFrameFacts,
    context: TilePartitionContextState<'ctx>,
    limits: DecodeLimits,
}

impl<'work, 'payload, 'ctx> TilePartitionTraversalInput<'work, 'payload, 'ctx> {
    /// Creates traversal-frontier input from explicit caller facts.
    #[must_use]
    pub(crate) const fn new(
        work_unit: &'work mut DecodeTileWorkUnit<'payload>,
        frame: TilePartitionFrameFacts,
        context: TilePartitionContextState<'ctx>,
        limits: DecodeLimits,
    ) -> Self {
        Self {
            work_unit,
            frame,
            context,
            limits,
        }
    }
}

/// One `decode_partition()` call on the frontier path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionCall {
    pub(crate) r: usize,
    pub(crate) c: usize,
    pub(crate) b_size: BlockSize,
    pub(crate) parent_size: Option<BlockSize>,
    pub(crate) chroma_offset: bool,
    pub(crate) has_chroma: bool,
    tree_type: PartitionTreeType,
    cfl_allowed_in_sdp: bool,
}

impl TilePartitionCall {
    const fn root(r: usize, c: usize, b_size: BlockSize, has_chroma: bool) -> Self {
        Self {
            r,
            c,
            b_size,
            parent_size: None,
            chroma_offset: false,
            has_chroma,
            tree_type: PartitionTreeType::Shared,
            cfl_allowed_in_sdp: true,
        }
    }

    const fn with_tree_type(self, tree_type: PartitionTreeType) -> Self {
        Self { tree_type, ..self }
    }

    const fn with_cfl_allowed_in_sdp(self, cfl_allowed_in_sdp: bool) -> Self {
        Self {
            cfl_allowed_in_sdp,
            ..self
        }
    }
}

/// AV2 tile-local MI bounds used by § 5.20.9.1 `is_inside`.
// `mi_*` fields mirror the AV2 MiRowStart/MiRowEnd/MiColStart/MiColEnd tile bounds; the prefix preserves that mapping.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TilePartitionBounds {
    mi_row_start: usize,
    mi_row_end: usize,
    mi_col_start: usize,
    mi_col_end: usize,
}

impl TilePartitionBounds {
    fn from_work_unit(work_unit: &DecodeTileWorkUnit<'_>) -> Self {
        let row_range = work_unit.mi_row_range();
        let col_range = work_unit.mi_col_range();
        Self {
            mi_row_start: row_range.start as usize,
            mi_row_end: row_range.end as usize,
            mi_col_start: col_range.start as usize,
            mi_col_end: col_range.end as usize,
        }
    }

    const fn is_inside(self, r: usize, c: usize) -> bool {
        self.mi_col_start <= c
            && c < self.mi_col_end
            && self.mi_row_start <= r
            && r < self.mi_row_end
    }

    fn avail_u(self, call: TilePartitionCall) -> bool {
        match call.r.checked_sub(1) {
            Some(candidate_r) => self.is_inside(candidate_r, call.c),
            None => false,
        }
    }

    fn avail_l(self, call: TilePartitionCall) -> bool {
        match call.c.checked_sub(1) {
            Some(candidate_c) => self.is_inside(call.r, candidate_c),
            None => false,
        }
    }
}

/// One consumed partition decision on the frontier path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionFrontierStep {
    pub(crate) call: TilePartitionCall,
    pub(crate) decision: ReadPartitionDecision,
    pub(crate) symbol_count_before: u64,
    pub(crate) symbol_count_after: u64,
}

/// The first § 5.20.4.1 `decode_block()` boundary reached by traversal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodeBlockFrontier {
    pub(crate) r: usize,
    pub(crate) c: usize,
    pub(crate) b_size: BlockSize,
    pub(crate) has_chroma: bool,
    pub(crate) chroma_offset: bool,
    tree_type: PartitionTreeType,
    stored_luma_y_mode: Option<IntraYMode>,
    cfl_allowed_in_sdp: bool,
    pub(crate) symbol_count_before_block: u64,
    pub(crate) symbol_checkpoint_before_block: SymbolDecoderCheckpoint,
}

impl DecodeBlockFrontier {
    /// True when this leaf is in the SDP luma-only tree.
    pub(crate) const fn is_luma_part(&self) -> bool {
        matches!(self.tree_type, PartitionTreeType::LumaPart)
    }

    /// True when this leaf is in the SDP chroma-only tree.
    pub(crate) const fn is_chroma_part(&self) -> bool {
        matches!(self.tree_type, PartitionTreeType::ChromaPart)
    }

    /// Stored luma `YMode` inherited by an SDP chroma-only leaf.
    pub(crate) const fn stored_luma_y_mode(&self) -> Option<IntraYMode> {
        self.stored_luma_y_mode
    }

    /// AV2 §5.20.3.1 `CflAllowedInSdp` for SDP chroma leaves.
    pub(crate) const fn cfl_allowed_in_sdp(&self) -> bool {
        self.cfl_allowed_in_sdp
    }
}

/// Mode-state update returned by a decoded partition leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraLeafMode {
    intra_joint_mode: Option<u8>,
    y_mode: Option<IntraYMode>,
    fsc_mode: Option<u8>,
    uses_mrls: Option<u8>,
}

impl GeneralIntraLeafMode {
    /// Leaf with a decoded luma mode (`TreeType != CHROMA_PART`).
    #[must_use]
    pub(crate) const fn luma(
        intra_joint_mode: u8,
        y_mode: IntraYMode,
        fsc_mode: u8,
        uses_mrls: u8,
    ) -> Self {
        Self {
            intra_joint_mode: Some(intra_joint_mode),
            y_mode: Some(y_mode),
            fsc_mode: Some(fsc_mode),
            uses_mrls: Some(uses_mrls),
        }
    }

    /// Leaf without intra luma mode state: SDP chroma-only or inter block.
    #[must_use]
    pub(crate) const fn no_luma_mode() -> Self {
        Self {
            intra_joint_mode: None,
            y_mode: None,
            fsc_mode: None,
            uses_mrls: None,
        }
    }

    /// The decoded §5.20.5.3 luma intra `y_mode` for this leaf, when the leaf
    /// carried a luma mode (`None` for an SDP chroma-only or inter leaf). The
    /// ac0ej3 reconstruction bridge reads it to gate the verified DC_PRED subset.
    #[must_use]
    pub(crate) const fn luma_y_mode(self) -> Option<IntraYMode> {
        self.y_mode
    }
}

/// Successful partition frontier plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionTraversalPlan {
    pub(crate) tile_num: u32,
    steps: Vec<TilePartitionFrontierStep>,
    skipped_out_of_frame: Vec<TilePartitionCall>,
    pending_children: Vec<TilePartitionCall>,
    frontier: DecodeBlockFrontier,
    pub(crate) consumed_bits_before: u64,
    pub(crate) consumed_bits_after: u64,
    symbol_count_after: u64,
}

/// Successful LR-unit root syntax frontier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileLoopRestorationRootFrontier {
    symbol_count_after: u64,
    consumed_bits_after: u64,
    lr_units_consumed: usize,
    active_wiener_ns_units: usize,
    selections: Vec<WienerNsLrUnitSelection>,
    active_source_blocks: Vec<WienerNsLrSourceBlock>,
}

/// Caller-visible selection state for one supported Wiener NS LR unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrUnitSelection {
    /// Plane whose LR unit syntax was consumed.
    pub(crate) plane: usize,
    /// Absolute LR unit row after tile-origin offset adjustment.
    pub(crate) unit_row: usize,
    /// Absolute LR unit column after tile-origin offset adjustment.
    pub(crate) unit_col: usize,
    /// Whether AV2 §5.20.10.5 selected `RESTORE_WIENER_NONSEP`.
    pub(crate) active: bool,
}

/// Caller-visible §7.20.1 source-bound facts for one active Wiener NS LR block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrSourceBlock {
    /// Plane whose active LR unit covers the block.
    pub(crate) plane: usize,
    /// Loop-restore block row in luma 4x4 units.
    pub(crate) row: usize,
    /// Loop-restore block column in luma 4x4 units.
    pub(crate) col: usize,
    /// Absolute LR unit row selected for this block.
    pub(crate) unit_row: usize,
    /// Absolute LR unit column selected for this block.
    pub(crate) unit_col: usize,
    /// Tile `MiRowStart` bound used by AV2 §7.20.4 `get_tx_skip`.
    pub(crate) tile_mi_row_start: usize,
    /// Tile `MiRowEnd` bound used by AV2 §7.20.4 `get_tx_skip`.
    pub(crate) tile_mi_row_end: usize,
    /// Tile `MiColStart` bound retained with the LR block for symmetry.
    pub(crate) tile_mi_col_start: usize,
    /// Tile `MiColEnd` bound used by AV2 §7.20.4 `BlockEndX`.
    pub(crate) tile_mi_col_end: usize,
    /// Block x coordinate in current-plane samples.
    pub(crate) x: usize,
    /// Block y coordinate in current-plane samples.
    pub(crate) y: usize,
    /// Block width in current-plane samples.
    pub(crate) width: usize,
    /// Block height in current-plane samples.
    pub(crate) height: usize,
    /// Inclusive `LumaStartX` bound from AV2 §7.20.1.
    pub(crate) luma_start_x: usize,
    /// Inclusive `LumaEndX` bound from AV2 §7.20.1.
    pub(crate) luma_end_x: usize,
    /// Inclusive `LumaStartY` bound from AV2 §7.20.1.
    pub(crate) luma_start_y: usize,
    /// Inclusive `LumaEndY` bound from AV2 §7.20.1.
    pub(crate) luma_end_y: usize,
    /// Inclusive full-frame luma Y bound (`MiRows * MI_SIZE - 1`) used by
    /// AV2 §7.20.3 `get_luma_sample`.
    pub(crate) frame_luma_end_y: usize,
    /// Inclusive `LumaStripeStartY` bound from AV2 §7.20.1.
    pub(crate) luma_stripe_start_y: usize,
    /// Inclusive `LumaStripeEndY` bound from AV2 §7.20.1.
    pub(crate) luma_stripe_end_y: usize,
}

impl TileLoopRestorationRootFrontier {
    /// Symbol count after consuming supported root LR syntax.
    #[must_use]
    pub(crate) const fn symbol_count_after(&self) -> u64 {
        self.symbol_count_after
    }

    /// Consumed tile-payload bits after supported root LR syntax.
    #[must_use]
    pub(crate) const fn consumed_bits_after(&self) -> u64 {
        self.consumed_bits_after
    }

    /// Number of supported frame-level Wiener NS LR units consumed.
    #[must_use]
    pub(crate) const fn lr_units_consumed(&self) -> usize {
        self.lr_units_consumed
    }

    /// Number of consumed LR units that selected `RESTORE_WIENER_NONSEP`.
    #[must_use]
    pub(crate) const fn active_wiener_ns_units(&self) -> usize {
        self.active_wiener_ns_units
    }

    /// Supported frame-level Wiener NS LR unit selections in syntax order.
    #[must_use]
    pub(crate) fn selections(&self) -> &[WienerNsLrUnitSelection] {
        &self.selections
    }

    /// Active Wiener NS blocks with caller-resolved §7.20.1 source bounds.
    #[must_use]
    pub(crate) fn active_source_blocks(&self) -> &[WienerNsLrSourceBlock] {
        &self.active_source_blocks
    }

    /// Whether every consumed LR unit selected `RESTORE_NONE`.
    #[must_use]
    pub(crate) const fn all_lr_units_inactive(&self) -> bool {
        self.active_wiener_ns_units == 0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct WienerNsLrUnitActivity {
    units_consumed: usize,
    active_units: usize,
    selections: Vec<WienerNsLrUnitSelection>,
    active_source_blocks: Vec<WienerNsLrSourceBlock>,
    unit_filter_state: WienerNsUnitFilterState,
    retain_source_blocks: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct WienerNsUnitFilterState {
    bank_size: [usize; 3],
    bank_ptr: [usize; 3],
}

impl WienerNsLrUnitActivity {
    fn retaining_source_blocks() -> Self {
        Self {
            retain_source_blocks: true,
            ..Self::default()
        }
    }

    fn record(
        &mut self,
        plane: usize,
        unit_row: usize,
        unit_col: usize,
        active: bool,
    ) -> Result<(), TilePartitionTraversalError> {
        self.units_consumed = checked_add("lr_units_consumed", self.units_consumed, 1)?;
        if active {
            self.active_units = checked_add("lr_active_wiener_ns_units", self.active_units, 1)?;
        }
        self.selections.push(WienerNsLrUnitSelection {
            plane,
            unit_row,
            unit_col,
            active,
        });
        Ok(())
    }

    fn record_source_block(
        &mut self,
        block: WienerNsLrSourceBlock,
        limits: DecodeLimits,
    ) -> Result<(), TilePartitionTraversalError> {
        if !self.retain_source_blocks {
            return Ok(());
        }
        let next_len = checked_add(
            "lr_active_source_blocks",
            self.active_source_blocks.len(),
            1,
        )?;
        limits.ensure_allocation_len(DecodeLimitName::MaxLumaSamplesPerFrame, next_len as u64)?;
        self.active_source_blocks.push(block);
        Ok(())
    }
}

impl TilePartitionTraversalPlan {
    /// Ordered partition decisions consumed before the block frontier.
    #[must_use]
    pub(crate) fn steps(&self) -> &[TilePartitionFrontierStep] {
        &self.steps
    }

    /// Pending sibling child calls that cannot be processed before block syntax.
    #[must_use]
    pub(crate) fn pending_children(&self) -> &[TilePartitionCall] {
        &self.pending_children
    }

    /// First block frontier.
    #[must_use]
    pub(crate) const fn frontier(&self) -> DecodeBlockFrontier {
        self.frontier
    }

    /// Symbol count after planning the frontier.
    #[must_use]
    pub(crate) const fn symbol_count_after(&self) -> u64 {
        self.symbol_count_after
    }
}

/// Planned frontier plus the live symbol cursor positioned before block syntax.
pub(crate) struct TilePartitionTraversalCursor<'payload> {
    plan: TilePartitionTraversalPlan,
    symbols: SymbolDecoder<'payload>,
}

impl<'payload> TilePartitionTraversalCursor<'payload> {
    /// Splits the cursor into the deterministic plan and live symbol decoder.
    #[must_use]
    pub(crate) fn into_parts(self) -> (TilePartitionTraversalPlan, SymbolDecoder<'payload>) {
        (self.plan, self.symbols)
    }
}

/// Error returned by the traversal frontier.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TilePartitionTraversalError {
    /// Resource limit rejected traversal.
    #[error("partition traversal rejected by resource limit: {0}")]
    Limit(#[from] DecodeLimitError),
    /// The § 5.20.2.3 `BlockDecoded` state allocation/sizing failed.
    #[error("partition traversal block-decoded state failed: {0}")]
    BlockDecoded(#[from] super::block_decoded_state::TileBlockDecodedStateError),
    /// The § 5.20.5.3 `YModes` state allocation/sizing failed.
    #[error("partition traversal intra YMode state failed: {0}")]
    IntraYModeState(#[from] TileIntraYModeStateError),
    /// The § 5.20.5.3 `UsesMrls` state allocation/sizing failed.
    #[error("partition traversal intra UsesMrls state failed: {0}")]
    UsesMrlsState(#[from] TileUsesMrlsStateError),
    /// The § 5.20.5.3 `FscModes` state allocation/sizing failed.
    #[error("partition traversal intra FscModes state failed: {0}")]
    FscModeState(#[from] TileFscModeStateError),
    /// A partition-size lookup failed.
    #[error("partition traversal size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    /// Allowed-partition derivation failed.
    #[error("partition traversal allowed-set derivation failed: {0}")]
    Allowed(#[from] PartitionAllowedError),
    /// Partition decision failed.
    #[error("partition traversal decision failed: {0}")]
    Decision(#[from] PartitionDecisionError),
    /// Symbol decoder initialization failed.
    #[error("partition traversal symbol initialization failed: {0}")]
    Symbol(#[from] splot_core::Error),
    /// CDF context access failed.
    #[error("partition traversal CDF context failed: {0}")]
    Cdf(#[from] TileCdfError),
    /// Unsupported traversal path.
    #[error("partition traversal unsupported path: {0:?}")]
    Unsupported(TilePartitionTraversalUnsupported),
    /// A coordinate subtraction underflowed.
    #[error("{coordinate} coordinate underflow: {base} - {offset}")]
    CoordinateUnderflow {
        /// Coordinate name.
        coordinate: &'static str,
        /// Base coordinate.
        base: usize,
        /// Derived offset.
        offset: usize,
    },
    /// A coordinate addition overflowed.
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        /// Coordinate name.
        coordinate: &'static str,
        /// Base coordinate.
        base: usize,
        /// Derived offset.
        offset: usize,
    },
    /// A coordinate multiplication overflowed.
    #[error("{coordinate} coordinate offset overflow: {left} * {right}")]
    CoordinateOffsetOverflow {
        /// Coordinate name.
        coordinate: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// Loop-restoration unit size was invalid for a supported LR plane.
    #[error("loop restoration plane {plane} has invalid unit size {unit_size}")]
    InvalidLoopRestorationUnitSize {
        /// Plane index.
        plane: usize,
        /// Invalid `LoopRestorationSize[plane]`.
        unit_size: usize,
    },
    /// A selected partition had no valid child size.
    #[error("partition traversal selected invalid child size for {partition:?} at bSize {b_size}")]
    InvalidPartitionSubsize {
        /// Selected partition.
        partition: PartitionType,
        /// Source block size.
        b_size: usize,
    },
    /// Internal child-call arity invariant failed.
    #[error("partition traversal produced more than four child calls")]
    TooManyChildCalls,
    /// Traversal found no in-frame block frontier.
    #[error("partition traversal reached no in-frame decode_block frontier")]
    NoBlockFrontier,
    /// A luma/shared intra leaf did not provide mode state for later neighbours.
    #[error("partition traversal missing intra luma mode state at ({r}, {c})")]
    MissingIntraLumaModeState {
        /// MI row.
        r: usize,
        /// MI column.
        c: usize,
    },
    /// A luma/shared intra leaf did not provide MRL state for later neighbours.
    #[error("partition traversal missing intra UsesMrls state at ({r}, {c})")]
    MissingIntraUsesMrlsState {
        /// MI row.
        r: usize,
        /// MI column.
        c: usize,
    },
    /// A luma/shared intra leaf did not provide FSC state for later neighbours.
    #[error("partition traversal missing intra FscModes state at ({r}, {c})")]
    MissingIntraFscModeState {
        /// MI row.
        r: usize,
        /// MI column.
        c: usize,
    },
}

/// Unsupported traversal-frontier path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TilePartitionTraversalUnsupported {
    /// SDP luma/chroma partition side effects remain outside this frontier.
    #[allow(
        dead_code,
        reason = "retained for future SDP subcases outside the currently admitted intra 64x64 split"
    )]
    Sdp,
    /// Extended SDP region signaling remains outside this frontier.
    ExtendedSdp,
    /// Root `read_lr()` syntax remains outside this frontier.
    ReadLoopRestoration,
    /// BRU/bridge/inactive behavior remains outside this frontier.
    BruOrBridge,
}

/// Plans the AV2 § 5.20.3.1 partition frontier to the first block boundary.
pub(crate) fn plan_tile_partition_traversal_frontier(
    input: TilePartitionTraversalInput<'_, '_, '_>,
) -> Result<TilePartitionTraversalPlan, TilePartitionTraversalError> {
    Ok(plan_tile_partition_traversal_cursor(input)?.plan)
}

/// Consumes only the AV2 §5.20.10.4 superblock-root `read_lr()` syntax supported
/// by this frontier and stops before §5.20.3.1 `read_partition`.
pub(crate) fn consume_tile_loop_restoration_root_frontier(
    input: TilePartitionTraversalInput<'_, '_, '_>,
) -> Result<TileLoopRestorationRootFrontier, TilePartitionTraversalError> {
    let TilePartitionTraversalInput {
        work_unit,
        frame,
        context: _,
        limits,
    } = input;
    let extended_sdp_allowed = frame.enable_extended_sdp && !frame.frame_is_intra;
    if extended_sdp_allowed {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ExtendedSdp,
        ));
    }
    if frame.loop_restoration == TilePartitionLoopRestorationState::UnsupportedReadLrSyntax {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration,
        ));
    }
    if frame.bru_state != TilePartitionBruState::Active {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::BruOrBridge,
        ));
    }

    let mut cdfs = work_unit.cdf().tile_cdfs().clone();
    let mut lr_activity = WienerNsLrUnitActivity::retaining_source_blocks();
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(work_unit.cdf().update_mode());
    let mut symbols = SymbolDecoder::with_base_and_config(
        work_unit.tile_bytes(),
        work_unit.tile_byte_span().start,
        config,
    )?;
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let root = TilePartitionCall::root(
        work_unit.mi_row_range().start as usize,
        work_unit.mi_col_range().start as usize,
        frame.sb_size,
        frame.has_chroma,
    );
    limits.ensure(DecodeLimitName::MaxTilePartitionSteps, 1)?;
    if root.r < frame.mi_rows && root.c < frame.mi_cols {
        ensure_supported_call(frame, root)?;
        read_loop_restoration_for_call(
            frame,
            root,
            tile_bounds,
            &mut cdfs,
            &mut symbols,
            &mut lr_activity,
            limits,
        )?;
    }
    *work_unit.cdf_mut().tile_cdfs_mut() = cdfs;
    Ok(TileLoopRestorationRootFrontier {
        symbol_count_after: symbols.symbol_count(),
        consumed_bits_after: symbols.consumed_bits().get(),
        lr_units_consumed: lr_activity.units_consumed,
        active_wiener_ns_units: lr_activity.active_units,
        selections: lr_activity.selections,
        active_source_blocks: lr_activity.active_source_blocks,
    })
}

/// Plans the partition frontier and returns the live symbol cursor at it.
pub(crate) fn plan_tile_partition_traversal_cursor<'payload>(
    input: TilePartitionTraversalInput<'_, 'payload, '_>,
) -> Result<TilePartitionTraversalCursor<'payload>, TilePartitionTraversalError> {
    let TilePartitionTraversalInput {
        work_unit,
        frame,
        context,
        limits,
    } = input;
    let extended_sdp_allowed = frame.enable_extended_sdp && !frame.frame_is_intra;
    if extended_sdp_allowed {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ExtendedSdp,
        ));
    }
    // The §5.20.3.1 partition syntax (`read_partition` -> `do_split` / `rect_type` /
    // …) is frame-type agnostic: the CDF contexts carry `frame_is_intra` via the
    // partition-structure `PlaneStart`, so the same traversal walks an intra
    // `OBU_CLOSED_LOOP_KEY` tile and an inter `OBU_REGULAR_TILE_GROUP` tile. The
    // inter path is admitted here (DECODE-FIRST-INTER-FRAME-FRONTIER); the inter leaf
    // callback reads the §5.20.7.6 inter `mode_info` instead of intra modes, and the
    // §8.2.4 `exit_symbol()` check the caller runs guards bit-exactness so a wrong
    // partition read for an unverified inter shape is rejected, never confident-wrong.
    if frame.loop_restoration == TilePartitionLoopRestorationState::UnsupportedReadLrSyntax {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration,
        ));
    }
    if frame.bru_state != TilePartitionBruState::Active {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::BruOrBridge,
        ));
    }

    let mut cdfs = work_unit.cdf().tile_cdfs().clone();
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(work_unit.cdf().update_mode());
    let mut symbols = SymbolDecoder::with_base_and_config(
        work_unit.tile_bytes(),
        work_unit.tile_byte_span().start,
        config,
    )?;
    let mut lr_activity = WienerNsLrUnitActivity::default();
    let mut sdp_state = SdpPartitionState::default();
    let consumed_bits_before = symbols.consumed_bits().get();
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let root = TilePartitionCall::root(
        work_unit.mi_row_range().start as usize,
        work_unit.mi_col_range().start as usize,
        frame.sb_size,
        frame.has_chroma,
    );
    let mut stack = vec![root];
    let mut steps = Vec::new();
    let mut skipped_out_of_frame = Vec::new();

    while let Some(call) = stack.pop() {
        limits.ensure(
            DecodeLimitName::MaxTilePartitionSteps,
            (steps.len() + 1) as u64,
        )?;
        if call.r >= frame.mi_rows || call.c >= frame.mi_cols {
            skipped_out_of_frame.push(call);
            continue;
        }
        if is_intra_sdp_shared_root(frame, call) {
            stack.push(call.with_tree_type(PartitionTreeType::ChromaPart));
            stack.push(call.with_tree_type(PartitionTreeType::LumaPart));
            continue;
        }
        ensure_supported_call(frame, call)?;
        read_loop_restoration_for_call(
            frame,
            call,
            tile_bounds,
            &mut cdfs,
            &mut symbols,
            &mut lr_activity,
            limits,
        )?;

        let symbol_count_before = symbols.symbol_count();
        let forced_chroma_partition = sdp_state.forced_chroma_partition(frame, call);
        let decision = read_frontier_partition_decision(
            call,
            frame,
            tile_bounds,
            context,
            forced_chroma_partition,
            &mut cdfs,
            &mut symbols,
        )?;
        let symbol_count_after = symbols.symbol_count();
        let partition = decision.partition;
        let call = call.with_cfl_allowed_in_sdp(sdp_state.record_partition(frame, call, partition));
        steps.push(TilePartitionFrontierStep {
            call,
            decision,
            symbol_count_before,
            symbol_count_after,
        });

        let sub_size = valid_subsize(partition, call.b_size)?;
        let chroma_offset = updated_chroma_offset(call, partition, sub_size, frame)?;
        if partition == PartitionType::None {
            stack.reverse();
            let tree_type = partition_tree_type(call);
            let plan = TilePartitionTraversalPlan {
                tile_num: work_unit.tile_num(),
                steps,
                skipped_out_of_frame,
                pending_children: stack,
                frontier: DecodeBlockFrontier {
                    r: call.r,
                    c: call.c,
                    b_size: sub_size,
                    // AV2 §5.20.3.1 decode_block() excludes LUMA_PART from HasChroma.
                    has_chroma: call.has_chroma
                        && frame.num_planes > 1
                        && tree_type != PartitionTreeType::LumaPart,
                    chroma_offset,
                    tree_type,
                    stored_luma_y_mode: None,
                    cfl_allowed_in_sdp: call.cfl_allowed_in_sdp,
                    symbol_count_before_block: symbols.symbol_count(),
                    symbol_checkpoint_before_block: symbols.checkpoint(),
                },
                consumed_bits_before,
                consumed_bits_after: symbols.consumed_bits().get(),
                symbol_count_after: symbols.symbol_count(),
            };
            *work_unit.cdf_mut().tile_cdfs_mut() = cdfs;
            return Ok(TilePartitionTraversalCursor { plan, symbols });
        }

        let children = child_calls(call, partition, sub_size, frame, chroma_offset)?;
        for child in children.as_slice().iter().rev().copied() {
            stack.push(child);
        }
    }

    Err(TilePartitionTraversalError::NoBlockFrontier)
}

/// Error from the general intra full partition-tree walk, distinguishing
/// traversal/MI-state failures from a caller leaf-decode failure `E`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraTreeWalkError<E> {
    /// A partition traversal step failed.
    #[error("partition tree walk traversal failed: {0}")]
    Traversal(#[from] TilePartitionTraversalError),
    /// An MI-size state update failed.
    #[error("partition tree walk MI-size update failed: {0}")]
    MiSize(TileMiSizeStateError),
    /// The caller's per-leaf-block decode failed.
    #[error("partition tree walk leaf-block decode failed")]
    Leaf(E),
}

/// Drives the complete AV2 § 5.20.3.1 partition tree for a general intra tile,
/// invoking `on_leaf` at each `PARTITION_NONE` leaf block in decode (DFS) order.
///
/// Unlike [`plan_tile_partition_traversal_cursor`], which stops at the first
/// leaf, this walks the whole tree: partition-split symbols and per-block syntax
/// are read interleaved on one live symbol decoder and the tile CDFs, exactly as
/// the spec orders them. The MI-size partition context is maintained across
/// blocks via `mi_size_state` (read for each partition decision, updated after
/// each leaf), and the AV2 § 5.20.5.3 `IntraJointModes` neighbour-mode grid via
/// `joint_modes` (read by `on_leaf` for the § 8.3.2 `y_mode_index` context,
/// updated after luma/shared leaves with the leaf's returned `IntraJointMode`),
/// the sibling `UsesMrls` grid for § 8.3.2 MRL contexts, plus a stored `YModes`
/// grid used by SDP chroma leaves. The live symbol decoder is returned for the
/// caller's § 8.2.4 `exit_symbol()` check.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_partition_tree<'payload, E, F>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    frame: TilePartitionFrameFacts,
    mi_size_state: &mut TileMiSizeState,
    joint_modes: &mut TileIntraJointModeState,
    uses_mrls: &mut TileUsesMrlsState,
    fsc_modes: &mut TileFscModeState,
    limits: DecodeLimits,
    mut on_leaf: F,
) -> Result<SymbolDecoder<'payload>, GeneralIntraTreeWalkError<E>>
where
    F: FnMut(
        &mut DecodeTileWorkUnit<'payload>,
        &mut SymbolDecoder<'payload>,
        &DecodeBlockFrontier,
        &TileIntraJointModeState,
        &TileUsesMrlsState,
        &TileFscModeState,
        &TileBlockDecodedState,
    ) -> Result<GeneralIntraLeafMode, E>,
{
    // The §5.20.3.1 partition tree walk is frame-type agnostic (see the cursor
    // planner's note): it walks an intra `OBU_CLOSED_LOOP_KEY` tile and an inter
    // `OBU_REGULAR_TILE_GROUP` tile (DECODE-FIRST-INTER-FRAME-FRONTIER). The inter
    // leaf callback reads the §5.20.7.6 inter `mode_info`, and the caller's §8.2.4
    // `exit_symbol()` check guards bit-exactness.
    if frame.loop_restoration == TilePartitionLoopRestorationState::UnsupportedReadLrSyntax {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration,
        )
        .into());
    }
    if frame.bru_state != TilePartitionBruState::Active {
        return Err(TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::BruOrBridge,
        )
        .into());
    }

    let config = SymbolDecoderConfig::new().with_cdf_update_mode(work_unit.cdf().update_mode());
    let mut symbols = SymbolDecoder::with_base_and_config(
        work_unit.tile_bytes(),
        work_unit.tile_byte_span().start,
        config,
    )
    .map_err(TilePartitionTraversalError::from)?;
    let mut lr_activity = WienerNsLrUnitActivity::default();
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let mut y_modes = TileIntraYModeState::new(frame.mi_rows, frame.mi_cols)
        .map_err(TilePartitionTraversalError::from)?;
    // AV2 § 5.20.2.1 decode_tile(): iterate the tile's MI range as a raster grid
    // of superblocks, `sbSize4 = Num_4x4_Blocks_Wide[SbSize]` MI units apart.
    // Each superblock is one `decode_partition(r, c, SbSize, ...)` root; the
    // shared symbol decoder, tile CDFs, and MI-size context carry across them so
    // later superblocks read the already-decoded left/above neighbours.
    // `frame.sb_size` is a validated BlockSize, so `num_4x4_wide()` is >= 1; the
    // `max(1)` is a belt-and-braces guard that keeps the loop progressing.
    let sb_size4 = frame
        .sb_size
        .num_4x4_wide()
        .map_err(TilePartitionTraversalError::from)?
        .max(1);
    let mi_row_start = work_unit.mi_row_range().start as usize;
    let mi_row_end = (work_unit.mi_row_range().end as usize).min(frame.mi_rows);
    let mi_col_start = work_unit.mi_col_range().start as usize;
    let mi_col_end = (work_unit.mi_col_range().end as usize).min(frame.mi_cols);
    // AV2 § 5.20.2.3 BlockDecoded state: a superblock-relative per-plane decoded
    // flag grid, re-initialized by clear_block_decoded_flags at each superblock
    // and updated after each transform block, so a later sub-block reads the
    // §7.13.2.1 above-right / below-left sentinel availability correctly.
    let mut block_decoded = TileBlockDecodedState::new(
        frame.num_planes,
        usize::from(frame.subsampling_x),
        usize::from(frame.subsampling_y),
        sb_size4,
        mi_col_end,
        mi_row_end,
    )
    .map_err(TilePartitionTraversalError::from)?;
    let sb_mask = sb_size4.saturating_sub(1);
    let mut step_count: u64 = 0;
    let mut sdp_state = SdpPartitionState::default();

    let mut sb_row = mi_row_start;
    while sb_row < mi_row_end {
        // § 5.20.2.1 clear_left_context() runs at the start of every superblock
        // row; the above context persists across rows.
        mi_size_state.clear_left_context();
        let mut sb_col = mi_col_start;
        while sb_col < mi_col_end {
            // § 5.20.2.1 / § 5.20.2.3: clear_block_decoded_flags(r, c, sbSize4)
            // re-initializes the superblock-relative BlockDecoded grid before the
            // superblock's partition DFS.
            block_decoded.clear_superblock(sb_row, sb_col);
            // One superblock-rooted § 5.20.3.1 partition DFS.
            let root = TilePartitionCall::root(sb_row, sb_col, frame.sb_size, frame.has_chroma);
            let mut stack = vec![root];
            while let Some(call) = stack.pop() {
                step_count += 1;
                limits
                    .ensure(DecodeLimitName::MaxTilePartitionSteps, step_count)
                    .map_err(TilePartitionTraversalError::from)?;
                if call.r >= frame.mi_rows || call.c >= frame.mi_cols {
                    continue;
                }
                if is_intra_sdp_shared_root(frame, call) {
                    stack.push(call.with_tree_type(PartitionTreeType::ChromaPart));
                    stack.push(call.with_tree_type(PartitionTreeType::LumaPart));
                    continue;
                }
                ensure_supported_call(frame, call)?;
                read_loop_restoration_for_call(
                    frame,
                    call,
                    tile_bounds,
                    work_unit.cdf_mut().tile_cdfs_mut(),
                    &mut symbols,
                    &mut lr_activity,
                    limits,
                )?;

                let forced_chroma_partition = sdp_state.forced_chroma_partition(frame, call);
                let decision = mi_size_state
                    .with_context_state(|context| {
                        read_frontier_partition_decision(
                            call,
                            frame,
                            tile_bounds,
                            context,
                            forced_chroma_partition,
                            work_unit.cdf_mut().tile_cdfs_mut(),
                            &mut symbols,
                        )
                    })
                    .map_err(GeneralIntraTreeWalkError::MiSize)??;
                let partition = decision.partition;
                let call = call
                    .with_cfl_allowed_in_sdp(sdp_state.record_partition(frame, call, partition));

                let sub_size = valid_subsize(partition, call.b_size)?;
                let chroma_offset = updated_chroma_offset(call, partition, sub_size, frame)?;
                if partition == PartitionType::None {
                    let tree_type = partition_tree_type(call);
                    let frontier = DecodeBlockFrontier {
                        r: call.r,
                        c: call.c,
                        b_size: sub_size,
                        has_chroma: call.has_chroma
                            && frame.num_planes > 1
                            && tree_type != PartitionTreeType::LumaPart,
                        chroma_offset,
                        tree_type,
                        stored_luma_y_mode: if tree_type == PartitionTreeType::ChromaPart {
                            y_modes.y_mode_at(call.r, call.c)
                        } else {
                            None
                        },
                        cfl_allowed_in_sdp: call.cfl_allowed_in_sdp,
                        symbol_count_before_block: symbols.symbol_count(),
                        symbol_checkpoint_before_block: symbols.checkpoint(),
                    };
                    let leaf_mode = on_leaf(
                        work_unit,
                        &mut symbols,
                        &frontier,
                        joint_modes,
                        uses_mrls,
                        fsc_modes,
                        &block_decoded,
                    )
                    .map_err(GeneralIntraTreeWalkError::Leaf)?;
                    // AV2 § 5.20.5.3: store the block's IntraJointMode and
                    // UsesMrls into every MI cell it covers, so later blocks'
                    // § 8.3.2 mode/MRL contexts see them as left/above neighbours.
                    let block_n4w = sub_size
                        .num_4x4_wide()
                        .map_err(TilePartitionTraversalError::from)?;
                    let block_n4h = sub_size
                        .num_4x4_high()
                        .map_err(TilePartitionTraversalError::from)?;
                    if frame.frame_is_intra && tree_type != PartitionTreeType::ChromaPart {
                        let joint_mode = leaf_mode.intra_joint_mode.ok_or(
                            TilePartitionTraversalError::MissingIntraLumaModeState {
                                r: call.r,
                                c: call.c,
                            },
                        )?;
                        let y_mode = leaf_mode.y_mode.ok_or(
                            TilePartitionTraversalError::MissingIntraLumaModeState {
                                r: call.r,
                                c: call.c,
                            },
                        )?;
                        let uses_mrls_value = leaf_mode.uses_mrls.ok_or(
                            TilePartitionTraversalError::MissingIntraUsesMrlsState {
                                r: call.r,
                                c: call.c,
                            },
                        )?;
                        let fsc_mode = leaf_mode.fsc_mode.ok_or(
                            TilePartitionTraversalError::MissingIntraFscModeState {
                                r: call.r,
                                c: call.c,
                            },
                        )?;
                        joint_modes.record_block(call.r, call.c, block_n4w, block_n4h, joint_mode);
                        fsc_modes.record_block(call.r, call.c, block_n4w, block_n4h, fsc_mode);
                        uses_mrls.record_block(
                            call.r,
                            call.c,
                            block_n4w,
                            block_n4h,
                            uses_mrls_value,
                        );
                        y_modes.record_block(call.r, call.c, block_n4w, block_n4h, y_mode);
                    }
                    // AV2 § 5.20.4: mark every plane 4x4 unit of the decoded block
                    // (BlockDecoded[plane][(subBlockMiRow >> subY) + i]
                    // [(subBlockMiCol >> subX) + j] = 1). The superblock-relative MI
                    // position is `row & sbMask` / `col & sbMask`. The minimal-tier
                    // subset uses a single full-block transform (TX_MODE_LARGEST),
                    // so each plane's transform-block 4x4 extent is the block's
                    // plane 4x4 width / height. Luma (plane 0) is never subsampled;
                    // chroma uses the frame subsampling.
                    let sub_block_mi_row = call.r & sb_mask;
                    let sub_block_mi_col = call.c & sb_mask;
                    let (plane_start, plane_end) =
                        plane_range_for_tree_type(tree_type, frame.num_planes);
                    for plane in plane_start..plane_end {
                        let (sub_x, sub_y) = if plane == 0 {
                            (0, 0)
                        } else {
                            (
                                usize::from(frame.subsampling_x),
                                usize::from(frame.subsampling_y),
                            )
                        };
                        block_decoded.set_block(
                            plane,
                            sub_block_mi_row,
                            sub_block_mi_col,
                            block_n4w >> sub_x,
                            block_n4h >> sub_y,
                        );
                    }
                    if tree_type == PartitionTreeType::ChromaPart {
                        mi_size_state
                            .update_chroma_block(call.r, call.c, sub_size)
                            .map_err(GeneralIntraTreeWalkError::MiSize)?;
                    } else {
                        mi_size_state
                            .update_luma_block(call.r, call.c, sub_size)
                            .map_err(GeneralIntraTreeWalkError::MiSize)?;
                    }
                } else {
                    let children = child_calls(call, partition, sub_size, frame, chroma_offset)?;
                    for child in children.as_slice().iter().rev().copied() {
                        stack.push(child);
                    }
                }
            }
            sb_col += sb_size4;
        }
        sb_row += sb_size4;
    }

    Ok(symbols)
}

// Fail-closed partition-call gate invoked with `?` at every traversal step: the `Result`
// is the designated rejection point for unsupported calls (the `frame`/`call` args feed the
// future check), so the wrapper is intentional even though the body cannot error yet.
#[allow(clippy::unnecessary_wraps)]
fn ensure_supported_call(
    _frame: TilePartitionFrameFacts,
    _call: TilePartitionCall,
) -> Result<(), TilePartitionTraversalError> {
    Ok(())
}

fn read_loop_restoration_for_call(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    tile_bounds: TilePartitionBounds,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
    limits: DecodeLimits,
) -> Result<(), TilePartitionTraversalError> {
    // AV2 §5.20.3.1 invokes §5.20.10.4 `read_lr()` only for superblock-root
    // partition calls (`SbSize == bSize`), before `read_partition`.
    if call.b_size != frame.sb_size {
        return Ok(());
    }
    let TilePartitionLoopRestorationState::FrameWienerNs(lr) = frame.loop_restoration else {
        return Ok(());
    };
    let w = call.b_size.num_4x4_wide()?;
    let h = call.b_size.num_4x4_high()?;
    let (plane_start, plane_end) = plane_range_for_tree_type(call.tree_type, frame.num_planes);
    for plane in plane_start..plane_end.min(3) {
        if !lr.plane_enabled[plane] {
            continue;
        }
        read_wiener_ns_lr_units_for_plane(
            plane,
            lr.unit_size[plane],
            lr.frame_filters_on[plane],
            frame,
            call,
            tile_bounds,
            w,
            h,
            cdfs,
            symbols,
            lr_activity,
            limits,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_wiener_ns_lr_units_for_plane(
    plane: usize,
    unit_size: usize,
    frame_filters_on: bool,
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    tile_bounds: TilePartitionBounds,
    w: usize,
    h: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
    limits: DecodeLimits,
) -> Result<(), TilePartitionTraversalError> {
    if unit_size == 0 {
        return Err(
            TilePartitionTraversalError::InvalidLoopRestorationUnitSize { plane, unit_size },
        );
    }
    let sub_x = if plane == 0 {
        0
    } else {
        usize::from(frame.subsampling_x)
    };
    let sub_y = if plane == 0 {
        0
    } else {
        usize::from(frame.subsampling_y)
    };
    let sample_step_x = MI_SIZE >> sub_x;
    let sample_step_y = MI_SIZE >> sub_y;

    let mi_cols = checked_sub(
        "lr_mi_cols",
        tile_bounds.mi_col_end,
        tile_bounds.mi_col_start,
    )?;
    let mi_rows = checked_sub(
        "lr_mi_rows",
        tile_bounds.mi_row_end,
        tile_bounds.mi_row_start,
    )?;
    let frame_cols = checked_mul_shifted("lr_frame_cols", mi_cols, MI_SIZE, sub_x)?;
    let frame_rows = checked_mul_shifted("lr_frame_rows", mi_rows, MI_SIZE, sub_y)?;
    let lr_row_offset =
        checked_mul_shifted("lr_row_offset", tile_bounds.mi_row_start, MI_SIZE, sub_y)? / unit_size;
    let lr_col_offset =
        checked_mul_shifted("lr_col_offset", tile_bounds.mi_col_start, MI_SIZE, sub_x)? / unit_size;
    let c = checked_sub("lr_c", call.c, tile_bounds.mi_col_start)?;
    let r = checked_sub("lr_r", call.r, tile_bounds.mi_row_start)?;

    let unit_rows = count_units_in_frame(unit_size, frame_rows)?;
    let unit_cols = count_units_in_frame(unit_size, frame_cols)?;
    let unit_row_start = ceil_unit_index(
        checked_mul("lr_unit_row_start", r, sample_step_y)?,
        unit_size,
    )?;
    let unit_col_start = ceil_unit_index(
        checked_mul("lr_unit_col_start", c, sample_step_x)?,
        unit_size,
    )?;
    let unit_row_end = unit_rows.min(ceil_unit_index(
        checked_mul(
            "lr_unit_row_end",
            checked_add("lr_r_end", r, h)?,
            sample_step_y,
        )?,
        unit_size,
    )?);
    let unit_col_end = unit_cols.min(ceil_unit_index(
        checked_mul(
            "lr_unit_col_end",
            checked_add("lr_c_end", c, w)?,
            sample_step_x,
        )?,
        unit_size,
    )?);

    for unit_row in unit_row_start..unit_row_end {
        for unit_col in unit_col_start..unit_col_end {
            let unit_row = checked_add("lr_unit_row", unit_row, lr_row_offset)?;
            let unit_col = checked_add("lr_unit_col", unit_col, lr_col_offset)?;
            let active = read_wiener_ns_lr_unit(
                plane,
                frame_filters_on,
                unit_row,
                unit_col,
                cdfs,
                symbols,
                lr_activity,
            )?;
            if active {
                record_active_wiener_ns_source_blocks_for_unit(
                    LrSourceBlockDerivation {
                        plane,
                        unit_size,
                        unit_row,
                        unit_col,
                        frame,
                        tile_bounds,
                        sub_x,
                        sub_y,
                    },
                    limits,
                    lr_activity,
                )?;
            }
        }
    }
    Ok(())
}

fn read_wiener_ns_lr_unit(
    plane: usize,
    frame_filters_on: bool,
    unit_row: usize,
    unit_col: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
) -> Result<bool, TilePartitionTraversalError> {
    let use_wiener_ns = cdfs
        .with_row_mut(super::cdf::TileCdfSelector::UseWienerNs, |row| {
            symbols.read_symbol(row)
        })??
        .get()
        != 0;
    lr_activity.record(plane, unit_row, unit_col, use_wiener_ns)?;
    // AV2 §5.20.10.5 maps `use_wiener_ns == 0` to `RESTORE_NONE`; active
    // units immediately invoke §5.20.10.6 with `readFrameFilters == 0`.
    if use_wiener_ns && !frame_filters_on {
        read_wiener_ns_unit_filter(plane, cdfs, symbols, &mut lr_activity.unit_filter_state)?;
    }
    Ok(use_wiener_ns)
}

fn read_wiener_ns_unit_filter(
    plane: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &mut WienerNsUnitFilterState,
) -> Result<(), TilePartitionTraversalError> {
    // This is the entropy-coded `readFrameFilters == 0` branch of AV2
    // §5.20.10.6. The current frontier consumes syntax and CDF updates only; the
    // decoded coefficients stay outside runtime reconstruction until §7.20.3 is
    // implemented.
    let merged = symbols.read_literal(1)? != 0;
    let previous_bank_size = state.bank_size[plane];
    for _ in 0..previous_bank_size.saturating_sub(1) {
        let use_bank = symbols.read_literal(1)? != 0;
        if use_bank {
            break;
        }
    }
    if merged {
        if state.bank_size[plane] == 0 {
            state.bank_size[plane] = 1;
        }
        return Ok(());
    }

    if state.bank_size[plane] < LR_BANK_SIZE {
        state.bank_ptr[plane] = state.bank_size[plane];
        state.bank_size[plane] = checked_add("wiener_ns_bank_size", state.bank_size[plane], 1)?;
    } else {
        state.bank_ptr[plane] =
            checked_add("wiener_ns_bank_ptr", state.bank_ptr[plane], 1)? % LR_BANK_SIZE;
    }

    let subset = read_wiener_ns_subset_symbol(plane, cdfs, symbols)?;
    let wiener_ns_uv_sym = if plane > 0 && subset > 0 {
        cdfs.with_row_mut(super::cdf::TileCdfSelector::WienerNsUvSym, |row| {
            symbols.read_symbol(row)
        })??
        .get()
            != 0
    } else {
        false
    };

    let plane_index = usize::from(plane > 0);
    let n_coeffs = if plane > 0 {
        WIENER_NS_CHROMA_COEFFS
    } else {
        WIENER_NS_LUMA_COEFFS
    };
    let mut j = 0usize;
    while j < n_coeffs {
        if WIENER_NS_TAPS_PRESENT[plane_index][subset][j] {
            read_wiener_ns_4part(WIENER_NS_TAPS_K[plane_index][j], cdfs, symbols)?;
        }
        if plane > 0 && j >= WIENER_NS_SHORT_COEFFS && wiener_ns_uv_sym {
            j = checked_add("wiener_ns_coeff_index", j, 2)?;
        } else {
            j = checked_add("wiener_ns_coeff_index", j, 1)?;
        }
    }
    Ok(())
}

fn read_wiener_ns_subset_symbol(
    plane: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<usize, TilePartitionTraversalError> {
    let num_subsets = if plane > 0 {
        WIENER_NS_CHROMA_SUBSETS
    } else {
        WIENER_NS_LUMA_SUBSETS
    };
    let mut subset = 0usize;
    while subset < num_subsets.saturating_sub(1) {
        let wiener_ns_length = cdfs.with_row_mut(
            super::cdf::TileCdfSelector::WienerNsLength {
                plane_ctx: plane.min(1),
            },
            |row| symbols.read_symbol(row),
        )??;
        if wiener_ns_length.get() == 0 {
            break;
        }
        subset = checked_add("wiener_ns_subset", subset, 1)?;
    }
    Ok(subset)
}

fn read_wiener_ns_4part(
    k: u8,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<(), TilePartitionTraversalError> {
    let num = checked_sub("wiener_ns_4part_num", 6, usize::from(k))?;
    let wiener_ns_base = cdfs
        .with_row_mut(super::cdf::TileCdfSelector::WienerNsBase, |row| {
            symbols.read_symbol(row)
        })??
        .get() as usize;
    let bits_base = checked_sub("wiener_ns_4part_bits", 2, num)?;
    let bits = checked_add("wiener_ns_4part_bits", bits_base, wiener_ns_base.max(1))?;
    let bits =
        u32::try_from(bits).map_err(|_| TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_4part_bits",
            base: bits,
            offset: 0,
        })?;
    let _ = symbols.read_literal(bits)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct LrSourceBlockDerivation {
    plane: usize,
    unit_size: usize,
    unit_row: usize,
    unit_col: usize,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    sub_x: usize,
    sub_y: usize,
}

fn record_active_wiener_ns_source_blocks_for_unit(
    input: LrSourceBlockDerivation,
    limits: DecodeLimits,
    lr_activity: &mut WienerNsLrUnitActivity,
) -> Result<(), TilePartitionTraversalError> {
    if !lr_activity.retain_source_blocks {
        return Ok(());
    }
    for row in input.tile_bounds.mi_row_start..input.tile_bounds.mi_row_end {
        for col in input.tile_bounds.mi_col_start..input.tile_bounds.mi_col_end {
            let (unit_row, unit_col) = lr_unit_for_block(input, row, col)?;
            if unit_row == input.unit_row && unit_col == input.unit_col {
                let block = lr_source_block_for(input, row, col)?;
                lr_activity.record_source_block(block, limits)?;
            }
        }
    }
    Ok(())
}

fn lr_unit_for_block(
    input: LrSourceBlockDerivation,
    row: usize,
    col: usize,
) -> Result<(usize, usize), TilePartitionTraversalError> {
    let mi_cols = checked_sub(
        "lr_source_mi_cols",
        input.tile_bounds.mi_col_end,
        input.tile_bounds.mi_col_start,
    )?;
    let mi_rows = checked_sub(
        "lr_source_mi_rows",
        input.tile_bounds.mi_row_end,
        input.tile_bounds.mi_row_start,
    )?;
    let frame_cols = checked_mul_shifted("lr_source_frame_cols", mi_cols, MI_SIZE, input.sub_x)?;
    let frame_rows = checked_mul_shifted("lr_source_frame_rows", mi_rows, MI_SIZE, input.sub_y)?;
    let unit_rows = count_units_in_frame(input.unit_size, frame_rows)?;
    let unit_cols = count_units_in_frame(input.unit_size, frame_cols)?;
    let lr_row_offset = checked_mul_shifted(
        "lr_source_row_offset",
        input.tile_bounds.mi_row_start,
        MI_SIZE,
        input.sub_y,
    )? / input.unit_size;
    let lr_col_offset = checked_mul_shifted(
        "lr_source_col_offset",
        input.tile_bounds.mi_col_start,
        MI_SIZE,
        input.sub_x,
    )? / input.unit_size;
    let local_row = checked_sub("lr_source_row", row, input.tile_bounds.mi_row_start)?;
    let local_col = checked_sub("lr_source_col", col, input.tile_bounds.mi_col_start)?;
    let row_sample = checked_mul("lr_source_unit_row_sample", local_row, MI_SIZE)?;
    let row_sample = checked_add("lr_source_unit_row_sample", row_sample, 8)?;
    let row_sample = row_sample >> input.sub_y;
    let col_sample =
        checked_mul_shifted("lr_source_unit_col_sample", local_col, MI_SIZE, input.sub_x)?;
    let unit_row = checked_add(
        "lr_source_unit_row",
        lr_row_offset,
        (row_sample / input.unit_size).min(unit_rows.saturating_sub(1)),
    )?;
    let unit_col = checked_add(
        "lr_source_unit_col",
        lr_col_offset,
        (col_sample / input.unit_size).min(unit_cols.saturating_sub(1)),
    )?;
    Ok((unit_row, unit_col))
}

fn lr_source_block_for(
    input: LrSourceBlockDerivation,
    row: usize,
    col: usize,
) -> Result<WienerNsLrSourceBlock, TilePartitionTraversalError> {
    let x = checked_mul_shifted("lr_source_x", col, MI_SIZE, input.sub_x)?;
    let y = checked_mul_shifted("lr_source_y", row, MI_SIZE, input.sub_y)?;
    let width = MI_SIZE >> input.sub_x;
    let height = MI_SIZE >> input.sub_y;
    let (luma_start_x_mi, luma_end_x_mi, luma_start_y_mi, luma_end_y_mi) =
        if input.frame.disable_loopfilters_across_tiles {
            (
                input.tile_bounds.mi_col_start,
                input.tile_bounds.mi_col_end,
                input.tile_bounds.mi_row_start,
                input.tile_bounds.mi_row_end,
            )
        } else {
            (0, input.frame.mi_cols, 0, input.frame.mi_rows)
        };
    let luma_start_x = checked_mul("lr_luma_start_x", luma_start_x_mi, MI_SIZE)?;
    let luma_start_y = checked_mul("lr_luma_start_y", luma_start_y_mi, MI_SIZE)?;
    let luma_end_x = checked_sub(
        "lr_luma_end_x",
        checked_mul("lr_luma_end_x", luma_end_x_mi, MI_SIZE)?,
        1,
    )?;
    let luma_end_y = checked_sub(
        "lr_luma_end_y",
        checked_mul("lr_luma_end_y", luma_end_y_mi, MI_SIZE)?,
        1,
    )?;
    let frame_luma_end_y = checked_sub(
        "lr_frame_luma_end_y",
        checked_mul("lr_frame_luma_end_y", input.frame.mi_rows, MI_SIZE)?,
        1,
    )?;
    let local_row = checked_sub("lr_source_local_row", row, input.tile_bounds.mi_row_start)?;
    let luma_y = checked_mul("lr_source_luma_y", local_row, MI_SIZE)?;
    let stripe_num = checked_add("lr_source_stripe_num", luma_y, 8)? / 64;
    let stripe_base = checked_add(
        "lr_source_stripe_base",
        checked_mul(
            "lr_source_stripe_base",
            input.tile_bounds.mi_row_start,
            MI_SIZE,
        )?,
        checked_mul("lr_source_stripe_base", stripe_num, 64)?,
    )?;
    let luma_stripe_start_y = stripe_base
        .checked_sub(8)
        .map_or(luma_start_y, |start| luma_start_y.max(start));
    let luma_stripe_end_y = luma_end_y.min(checked_add("lr_source_stripe_end_y", stripe_base, 55)?);

    Ok(WienerNsLrSourceBlock {
        plane: input.plane,
        row,
        col,
        unit_row: input.unit_row,
        unit_col: input.unit_col,
        tile_mi_row_start: input.tile_bounds.mi_row_start,
        tile_mi_row_end: input.tile_bounds.mi_row_end,
        tile_mi_col_start: input.tile_bounds.mi_col_start,
        tile_mi_col_end: input.tile_bounds.mi_col_end,
        x,
        y,
        width,
        height,
        luma_start_x,
        luma_end_x,
        luma_start_y,
        luma_end_y,
        frame_luma_end_y,
        luma_stripe_start_y,
        luma_stripe_end_y,
    })
}

fn count_units_in_frame(
    unit_size: usize,
    frame_size: usize,
) -> Result<usize, TilePartitionTraversalError> {
    Ok(checked_add("lr_count_units", frame_size, unit_size >> 1)? / unit_size)
        .map(|count| count.max(1))
}

fn ceil_unit_index(value: usize, unit_size: usize) -> Result<usize, TilePartitionTraversalError> {
    let adjusted = checked_add("lr_unit_ceil", value, unit_size.saturating_sub(1))?;
    Ok(adjusted / unit_size)
}

fn checked_mul_shifted(
    coordinate: &'static str,
    value: usize,
    scale: usize,
    shift: usize,
) -> Result<usize, TilePartitionTraversalError> {
    Ok(checked_mul(coordinate, value, scale)? >> shift)
}

fn read_frontier_partition_decision(
    call: TilePartitionCall,
    frame: TilePartitionFrameFacts,
    tile_bounds: TilePartitionBounds,
    context: TilePartitionContextState<'_>,
    forced_chroma_partition: Option<PartitionType>,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<ReadPartitionDecision, TilePartitionTraversalError> {
    let allowed = PartitionAllowedInput::new(
        call.r,
        call.c,
        frame.mi_rows,
        frame.mi_cols,
        call.b_size.index(),
        partition_tree_type(call),
        frame.subsampling_x,
        frame.subsampling_y,
        frame.features,
        frame.frame_is_intra,
        false,
        frame.max_pb_aspect_ratio,
        call.has_chroma,
        call.chroma_offset,
        frame.num_planes,
        forced_chroma_partition,
    )?;
    let facts = partition_decision_facts(allowed)?;
    // §5.20.3.1 / §8.3.2: the SDP chroma partition tree reads its partition-entry
    // CDFs and neighbor block-size context from plane 1; the luma / shared tree
    // uses plane 0 (AVM `plane = tree_type == CHROMA_PART`).
    let partition_plane = partition_cdf_plane(call.tree_type);
    let partition_context = PartitionContextInput::new(
        call.b_size.index(),
        partition_plane,
        call.r,
        call.c,
        context.left_mi_sizes,
        context.above_mi_sizes,
    )?;
    let avail_u = tile_bounds.avail_u(call);
    let avail_l = tile_bounds.avail_l(call);
    // §8.3.2 NOTE: `PlaneStart` is fixed at 0 for `do_square_split` (the chroma
    // partition is forced for large block sizes), so the square-split context and
    // its `MiSizes[PlaneStart]` always use plane 0 — never the chroma SDP plane.
    let square_context = SquareSplitContextInput::new(
        call.b_size.index(),
        0,
        call.r,
        call.c,
        avail_u,
        avail_l,
        context.mi_sizes,
    )?;
    let decision_input =
        facts.read_partition_decision_input(true, partition_context, square_context);
    Ok(super::partition::read_partition_decision(
        decision_input,
        cdfs,
        symbols,
    )?)
}

fn partition_tree_type(call: TilePartitionCall) -> PartitionTreeType {
    call.tree_type
}

fn plane_range_for_tree_type(tree_type: PartitionTreeType, num_planes: usize) -> (usize, usize) {
    match tree_type {
        PartitionTreeType::Shared => (0, num_planes.min(3)),
        PartitionTreeType::LumaPart => (0, num_planes.min(1)),
        PartitionTreeType::ChromaPart => (1, num_planes.min(3)),
    }
}

/// AV2 § 5.20.3.1 partition-CDF plane index (`plane = TreeType == CHROMA_PART`).
///
/// The § 8.3.2 partition-entry CDFs (`do_split`, `do_square_split`, `rect_type`,
/// `do_ext_partition`, `do_uneven_4way_partition`) and their neighbor block-size
/// context arrays are keyed by this plane: the luma / shared partition tree reads
/// plane `0`, and the SDP chroma partition tree reads plane `1`. This mirrors
/// AVM `decodeframe.c` `const int plane = xd->tree_type == CHROMA_PART;`.
const fn partition_cdf_plane(tree_type: PartitionTreeType) -> usize {
    matches!(tree_type, PartitionTreeType::ChromaPart) as usize
}

fn is_intra_sdp_shared_root(frame: TilePartitionFrameFacts, call: TilePartitionCall) -> bool {
    frame.enable_sdp
        && frame.frame_is_intra
        && call.tree_type == PartitionTreeType::Shared
        && call.b_size.index() == BLOCK_64X64
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SdpPartitionState {
    top_luma_horz: bool,
    top_luma_vert: bool,
    top_luma_uneven_horz: bool,
    top_luma_uneven_vert: bool,
    chroma_follows_luma: bool,
    /// AV2 §5.20.3.1 `LumaPartitions[r][c]`: the LUMA_PART 64x64 partition
    /// inherited by the collocated CHROMA_PART 64x64 when `ChromaFollowsLuma`.
    luma_64x64_partition: Option<PartitionType>,
}

impl SdpPartitionState {
    fn record_partition(
        &mut self,
        frame: TilePartitionFrameFacts,
        call: TilePartitionCall,
        partition: PartitionType,
    ) -> bool {
        if !frame.frame_is_intra {
            return call.cfl_allowed_in_sdp;
        }

        if call.tree_type == PartitionTreeType::LumaPart && call.b_size.index() == BLOCK_64X64 {
            self.top_luma_horz = matches!(partition, PartitionType::Horz | PartitionType::Horz3);
            self.top_luma_vert = matches!(partition, PartitionType::Vert | PartitionType::Vert3);
            self.top_luma_uneven_horz =
                matches!(partition, PartitionType::Horz4A | PartitionType::Horz4B);
            self.top_luma_uneven_vert =
                matches!(partition, PartitionType::Vert4A | PartitionType::Vert4B);
            self.chroma_follows_luma =
                partition == PartitionType::None || self.top_luma_horz || self.top_luma_vert;
            // AV2 §5.20.3.1: `LumaPartitions[r][c] = partition` at the LUMA_PART
            // 64x64 root; the collocated CHROMA_PART 64x64 inherits it when
            // `ChromaPartitionKnown` (`chroma_follows_luma`) holds.
            self.luma_64x64_partition = Some(partition);
        }

        let this_horz = matches!(
            partition,
            PartitionType::Horz
                | PartitionType::Horz3
                | PartitionType::Horz4A
                | PartitionType::Horz4B
        );
        let this_vert = matches!(
            partition,
            PartitionType::Vert
                | PartitionType::Vert3
                | PartitionType::Vert4A
                | PartitionType::Vert4B
        );

        let mut cfl_allowed_in_sdp = call.cfl_allowed_in_sdp;
        if call.tree_type == PartitionTreeType::ChromaPart && call.b_size.index() == BLOCK_64X64 {
            cfl_allowed_in_sdp = self.chroma_follows_luma
                || partition == PartitionType::None
                || ((self.top_luma_horz || self.top_luma_uneven_horz) && this_horz)
                || ((self.top_luma_vert || self.top_luma_uneven_vert) && this_vert);
        }

        if call.tree_type == PartitionTreeType::LumaPart
            && call
                .parent_size
                .is_some_and(|parent| parent.index() == BLOCK_64X64)
            && (partition == PartitionType::None
                || (self.top_luma_horz && this_horz)
                || (self.top_luma_vert && this_vert))
        {
            self.chroma_follows_luma = false;
        }

        cfl_allowed_in_sdp
    }

    /// AV2 §5.20.3.2 `partition_implied`: when the CHROMA_PART 64x64 collocated
    /// `ChromaPartitionKnown[r][c]` holds, the chroma partition is forced to
    /// `LumaPartitions[r][c]` and no `read_partition` symbol is consumed.
    ///
    /// Returns `None` when the chroma block reads its own partition tree (the
    /// luma/chroma trees diverged) or when the call is not a CHROMA_PART 64x64.
    fn forced_chroma_partition(
        &self,
        frame: TilePartitionFrameFacts,
        call: TilePartitionCall,
    ) -> Option<PartitionType> {
        if !frame.frame_is_intra
            || call.tree_type != PartitionTreeType::ChromaPart
            || call.b_size.index() != BLOCK_64X64
            || !self.chroma_follows_luma
        {
            return None;
        }
        self.luma_64x64_partition
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TilePartitionChildCalls {
    calls: [TilePartitionCall; 4],
    len: usize,
}

impl TilePartitionChildCalls {
    fn new(fill: TilePartitionCall) -> Self {
        Self {
            calls: [fill; 4],
            len: 0,
        }
    }

    fn push(&mut self, call: TilePartitionCall) -> Result<(), TilePartitionTraversalError> {
        let slot = self
            .calls
            .get_mut(self.len)
            .ok_or(TilePartitionTraversalError::TooManyChildCalls)?;
        *slot = call;
        self.len += 1;
        Ok(())
    }

    fn as_slice(&self) -> &[TilePartitionCall] {
        &self.calls[..self.len]
    }
}

fn child_calls(
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: TilePartitionFrameFacts,
    chroma_offset: bool,
) -> Result<TilePartitionChildCalls, TilePartitionTraversalError> {
    let num4x4wide = call.b_size.num_4x4_wide()?;
    let num4x4high = call.b_size.num_4x4_high()?;
    let half_w = num4x4wide >> 1;
    let half_h = num4x4high >> 1;
    let parent = Some(call.b_size);
    let mut children = TilePartitionChildCalls::new(call);
    match partition {
        PartitionType::None => {}
        PartitionType::Horz => {
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type,
            ))?;
        }
        PartitionType::Vert => {
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type,
            ))?;
        }
        PartitionType::Split => {
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                false,
                call.has_chroma,
                call.tree_type,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                false,
                call.has_chroma,
                call.tree_type,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                call.c,
                sub_size,
                parent,
                false,
                call.has_chroma,
                call.tree_type,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                false,
                call.has_chroma,
                call.tree_type,
            ))?;
        }
        PartitionType::Horz3 => {
            let middle = h_partition_midsize(call.b_size)?.valid().ok_or(
                TilePartitionTraversalError::InvalidPartitionSubsize {
                    partition,
                    b_size: call.b_size.index(),
                },
            )?;
            let middle_chroma =
                call.b_size.index() == BLOCK_8X32 && call.has_chroma && frame.subsampling_x;
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h >> 1)?,
                call.c,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset && !middle_chroma,
                call.tree_type,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h >> 1)?,
                checked_add("c", call.c, half_w)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                checked_scaled_add("r", call.r, 3, half_h >> 1)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type,
            ))?;
        }
        PartitionType::Vert3 => {
            let middle = h_partition_midsize(call.b_size)?.valid().ok_or(
                TilePartitionTraversalError::InvalidPartitionSubsize {
                    partition,
                    b_size: call.b_size.index(),
                },
            )?;
            let middle_chroma =
                call.b_size.index() == BLOCK_32X8 && call.has_chroma && frame.subsampling_y;
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w >> 1)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset && !middle_chroma,
                call.tree_type,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                checked_add("c", call.c, half_w >> 1)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                call.r,
                checked_scaled_add("c", call.c, 3, half_w >> 1)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type,
            ))?;
        }
        PartitionType::Horz4A | PartitionType::Horz4B => {
            let b_size_big = valid_subsize(PartitionType::Horz, call.b_size)?;
            let b_size_med = valid_subsize(PartitionType::Horz, b_size_big)?;
            let third = if partition == PartitionType::Horz4A {
                b_size_big
            } else {
                b_size_med
            };
            let second = if partition == PartitionType::Horz4A {
                b_size_med
            } else {
                b_size_big
            };
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                checked_add("r", call.r, num4x4high >> 3)?,
                call.c,
                second,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                checked_scaled_add(
                    "r",
                    call.r,
                    if partition == PartitionType::Horz4A {
                        3
                    } else {
                        5
                    },
                    num4x4high >> 3,
                )?,
                call.c,
                third,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                checked_scaled_add("r", call.r, 7, num4x4high >> 3)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type,
            ))?;
        }
        PartitionType::Vert4A | PartitionType::Vert4B => {
            let b_size_big = valid_subsize(PartitionType::Vert, call.b_size)?;
            let b_size_med = valid_subsize(PartitionType::Vert, b_size_big)?;
            let third = if partition == PartitionType::Vert4A {
                b_size_big
            } else {
                b_size_med
            };
            let second = if partition == PartitionType::Vert4A {
                b_size_med
            } else {
                b_size_big
            };
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, num4x4wide >> 3)?,
                second,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                call.r,
                checked_scaled_add(
                    "c",
                    call.c,
                    if partition == PartitionType::Vert4A {
                        3
                    } else {
                        5
                    },
                    num4x4wide >> 3,
                )?,
                third,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type,
            ))?;
            children.push(child(
                call.r,
                checked_scaled_add("c", call.c, 7, num4x4wide >> 3)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type,
            ))?;
        }
    }
    for child in &mut children.calls[..children.len] {
        child.cfl_allowed_in_sdp = call.cfl_allowed_in_sdp;
    }
    Ok(children)
}

fn child(
    r: usize,
    c: usize,
    b_size: BlockSize,
    parent_size: Option<BlockSize>,
    chroma_offset: bool,
    has_chroma: bool,
    tree_type: PartitionTreeType,
) -> TilePartitionCall {
    TilePartitionCall {
        r,
        c,
        b_size,
        parent_size,
        chroma_offset,
        has_chroma,
        tree_type,
        cfl_allowed_in_sdp: true,
    }
}

fn updated_chroma_offset(
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    if call.chroma_offset || !call.has_chroma {
        return Ok(call.chroma_offset);
    }
    if is_chroma_offset_for_subsize(sub_size, frame)? {
        return Ok(true);
    }
    if partition == PartitionType::Horz3 {
        let middle_chroma = call.b_size.index() == BLOCK_8X32 && frame.subsampling_x;
        if !middle_chroma
            && let Some(midsize) = h_partition_midsize(call.b_size)?.valid()
            && is_chroma_offset_for_subsize(midsize, frame)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_chroma_offset_for_subsize(
    sub_size: BlockSize,
    frame: TilePartitionFrameFacts,
) -> Result<bool, TilePartitionTraversalError> {
    Ok((frame.subsampling_y && sub_size.mi_height_log2()? == 0)
        || (frame.subsampling_x && sub_size.mi_width_log2()? == 0))
}

fn valid_subsize(
    partition: PartitionType,
    b_size: BlockSize,
) -> Result<BlockSize, TilePartitionTraversalError> {
    match partition_subsize(partition, b_size)? {
        PartitionSubsize::Valid(sub_size) => Ok(sub_size),
        PartitionSubsize::Invalid => Err(TilePartitionTraversalError::InvalidPartitionSubsize {
            partition,
            b_size: b_size.index(),
        }),
    }
}

fn checked_add(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, TilePartitionTraversalError> {
    base.checked_add(offset)
        .ok_or(TilePartitionTraversalError::CoordinateOverflow {
            coordinate,
            base,
            offset,
        })
}

fn checked_sub(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, TilePartitionTraversalError> {
    base.checked_sub(offset)
        .ok_or(TilePartitionTraversalError::CoordinateUnderflow {
            coordinate,
            base,
            offset,
        })
}

fn checked_mul(
    coordinate: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, TilePartitionTraversalError> {
    left.checked_mul(right)
        .ok_or(TilePartitionTraversalError::CoordinateOffsetOverflow {
            coordinate,
            left,
            right,
        })
}

fn checked_scaled_add(
    coordinate: &'static str,
    base: usize,
    scale: usize,
    value: usize,
) -> Result<usize, TilePartitionTraversalError> {
    checked_add(coordinate, base, checked_mul(coordinate, scale, value)?)
}

#[cfg(test)]
#[path = "partition_traversal_tests.rs"]
mod tests;
