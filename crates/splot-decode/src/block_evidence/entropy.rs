// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::sync::Arc;

use splot_core::symbol::{EntropyCaptureStop, EntropyObserver};
pub use splot_core::symbol::{EntropyCdf, EntropyCoding, EntropySymbol, EntropySyntax};
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};
use splot_recon::{PlaneId, PlaneRect};

use super::{BlockEvidenceId, BlockEvidenceStatus, BlockEvidenceTile, BlockTransformId};
use crate::bitstream::tile_payload::DecodeBlockFrontier;

/// The AV2 revision for captured entropy operations.
pub const ENTROPY_SPEC_VERSION: &str = "1.0.0";
/// The version of the symbol observation schema and its syntax keys.
pub const ENTROPY_SCHEMA_VERSION: u16 = 1;

/// An entropy observation scope within the selected coded frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockEntropyScope {
    /// Arithmetic reads during one coding block's parse. Excludes its parent partition.
    Block(BlockEvidenceId),
    /// Arithmetic reads during one transform's coefficient parse, from `all_zero`
    /// through the last coefficient read. Excludes shared transform partition syntax.
    Transform(BlockTransformId),
}

impl BlockEntropyScope {
    fn block(self) -> BlockEvidenceId {
        match self {
            Self::Block(block) => block,
            Self::Transform(transform) => transform.block,
        }
    }
}

/// Entropy observations for one scope. The report identifies the coded frame.
#[derive(Debug)]
#[non_exhaustive]
pub struct BlockEntropyEvidence {
    /// The selected block or transform.
    pub scope: BlockEntropyScope,
    /// The exact byte count of the enclosing tile payload.
    pub tile_payload_bytes: u64,
    /// Captured reads in decode order. Pair the ordinal with the scope and frame.
    pub symbols: Vec<EntropySymbol>,
    /// The exclusive end ordinal, including units past the recording limit.
    /// A decode error can stop the parse before it reaches the end of the scope.
    pub end_ordinal: u64,
    stopped: Option<EntropyCaptureStop>,
}

impl BlockEntropyEvidence {
    /// Returns the transform that owns the read at the zero-based record index.
    /// Shared or block-only reads have no transform owner.
    #[must_use]
    pub fn transform_for(&self, symbol_index: usize) -> Option<BlockTransformId> {
        let [plane, x, y, size] = self.symbols.get(symbol_index)?.scope_tag()?;
        let plane = match plane {
            0 => PlaneId::Y,
            1 => PlaneId::U,
            2 => PlaneId::V,
            _ => return None,
        };
        let width = *TX_WIDTH.get(size as usize)? as usize;
        let height = *TX_HEIGHT.get(size as usize)? as usize;
        let coded = PlaneRect::new(x as usize, y as usize, width, height).ok()?;
        Some(BlockTransformId::new(self.scope.block(), plane, coded))
    }

    pub(super) fn status(&self) -> Option<BlockEvidenceStatus> {
        match self.stopped? {
            EntropyCaptureStop::Limit => Some(BlockEvidenceStatus::Truncated),
            EntropyCaptureStop::Cancelled => Some(BlockEvidenceStatus::Cancelled),
            EntropyCaptureStop::AllocationFailed => Some(BlockEvidenceStatus::AllocationFailed),
            _ => Some(BlockEvidenceStatus::Unavailable),
        }
    }
}

impl BlockEvidenceTile<'_> {
    pub(crate) fn start_entropy(
        self,
        frontier: &DecodeBlockFrontier,
    ) -> Option<(BlockEntropyScope, EntropyObserver)> {
        let state = &self.recorder.state;
        let scope = state.request.entropy?;
        if scope.block().tile != self.tile || self.block_id(frontier)? != scope.block() {
            return None;
        }
        let capture = state.state.lock();
        let owner = Arc::clone(state);
        let observer = EntropyObserver::new(
            capture.remaining_events,
            capture.remaining_bytes,
            move || owner.cancelled(),
        );
        Some((scope, observer))
    }

    pub(crate) fn finish_entropy(
        self,
        scope: BlockEntropyScope,
        tile_payload_bytes: u64,
        observer: &EntropyObserver,
    ) {
        let capture = observer.take();
        if matches!(scope, BlockEntropyScope::Transform(_)) && capture.end_ordinal == 0 {
            return;
        }
        let state = &mut *self.recorder.state.state.lock();
        if state.finished || state.frames_selected != self.recorder.frame_number {
            return;
        }
        state.entropy = Some(BlockEntropyEvidence {
            scope,
            tile_payload_bytes,
            symbols: capture.symbols,
            end_ordinal: capture.end_ordinal,
            stopped: capture.stopped,
        });
    }
}
