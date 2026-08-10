// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Runtime residual transform dispatch.

use splot_recon::{DpcmDirection, IntraCardinalDirection, PlaneId};

use core::fmt;
use core::ops::{Deref, DerefMut};
use std::cell::Cell;
use std::thread::LocalKey;

use crate::bitstream::tile_payload::{
    CflParams, LumaPalette, SupportedChromaMode, SupportedNonDcLumaMode,
};
use crate::tile::block_context::{BlockCtx, TxShape};

mod chroma_pair;
mod deblock_recorder;
mod palette;
mod plan;
mod plane_execution;
mod reconstruct_dispatch;
mod transform_units;

#[cfg(test)]
use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaTransformTypeContext, PositionedLumaCoeffBlock,
};
pub(crate) use deblock_recorder::DeblockRecorder;
#[cfg(test)]
use plan::{MAX_RESIDUAL_PLANES, coeff_plane, tx_size_from_log2};
pub(crate) use plane_execution::ParsedGeneralIntraResidual;
#[cfg(test)]
use plane_execution::{
    CctxRole, ParsedResidualPlane, ParsedResidualPlaneKind, ParsedTransformUnit,
    chroma_angle_delta_uv,
};
#[cfg(test)]
use splot_core::tables::conversion::TX_WIDTH_LOG2;
#[cfg(test)]
use transform_units::tx_size_log2;

const CHROMA_PLANES: [PlaneId; 2] = [PlaneId::U, PlaneId::V];
const CHUNK_64_N4: usize = 16;
const TX_4X4: usize = 0;
const DCT_DCT: usize = 0;
const IDTX: usize = 9;

struct RecycledVec<T: 'static> {
    entries: Vec<T>,
    recycler: &'static LocalKey<Cell<Option<Vec<T>>>>,
}

impl<T> RecycledVec<T> {
    fn take(
        recycler: &'static LocalKey<Cell<Option<Vec<T>>>>,
        capacity: usize,
    ) -> core::result::Result<Self, std::collections::TryReserveError> {
        let mut entries = recycler.with(Cell::take).unwrap_or_default();
        entries.clear();
        entries.try_reserve(capacity)?;
        Ok(Self { entries, recycler })
    }
}

impl<T> fmt::Debug for RecycledVec<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.entries.fmt(formatter)
    }
}

impl<T> PartialEq for RecycledVec<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl<T> Eq for RecycledVec<T> where T: Eq {}

impl<T> Deref for RecycledVec<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<T> DerefMut for RecycledVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

impl<T> Drop for RecycledVec<T> {
    fn drop(&mut self) {
        let mut entries = core::mem::take(&mut self.entries);
        entries.clear();
        self.recycler.with(|slot| {
            let entries = match slot.take() {
                Some(cached) if cached.capacity() > entries.capacity() => cached,
                _ => entries,
            };
            slot.set(Some(entries));
        });
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraResidualPlan {
    planes: RecycledVec<ResidualPlanePlan>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RectLumaPlan {
    Palette {
        palette: LumaPalette,
        use_tcq: bool,
    },
    Dc {
        use_tcq: bool,
    },
    Dip {
        mode: u8,
        transpose: bool,
        use_tcq: bool,
    },
    Middle {
        p_angle: u16,
        use_tcq: bool,
    },
    MiddleMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        is_sb_boundary: bool,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    OneSidedAboveMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    OneSidedLeftMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        is_sb_boundary: bool,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    CardinalMrl {
        direction: IntraCardinalDirection,
        mrl_index: usize,
        above_mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    OneSidedAbove {
        p_angle: u16,
        use_tcq: bool,
    },
    OneSidedLeft {
        p_angle: u16,
        use_tcq: bool,
    },
    Cardinal {
        direction: IntraCardinalDirection,
        use_tcq: bool,
    },
    Paeth {
        use_tcq: bool,
    },
    Smooth {
        mode: SupportedNonDcLumaMode,
        use_tcq: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RectChromaPlan {
    Mode(SupportedChromaMode, Option<DpcmDirection>),
    Directional {
        mode: SupportedChromaMode,
        angle_delta_uv: i8,
        dpcm: Option<DpcmDirection>,
    },
    Cfl {
        params: CflParams,
        cfl_ds_filter_index: u8,
        sb_mib: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ResidualPlanError {
    #[error("residual plan geometry is inconsistent with the AV2 block-size domain")]
    InvalidGeometry,
    #[error("failed to allocate general-intra residual plan storage for the {plane:?} plane")]
    Allocation { plane: PlaneId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResidualPlanePlan {
    plane_id: PlaneId,
    block_ctx: BlockCtx,
    coeff_plane: usize,
    tx_size: usize,
    x: usize,
    y: usize,
    tx: TxShape,
    residual_width4: usize,
    residual_height4: usize,
    fsc_mode: bool,
    txb_skip_fsc_mode: bool,
    zero_corners: bool,
    defer_reconstruction: bool,
    reconstruction_tx_type: Option<usize>,
    reconstruction: ResidualReconstructionPlan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResidualReconstructionPlan {
    LumaPalette {
        palette: LumaPalette,
        use_tcq: bool,
    },
    LumaRectSmooth {
        mode: SupportedNonDcLumaMode,
        use_tcq: bool,
    },
    LumaRectDip {
        mode: u8,
        transpose: bool,
        use_tcq: bool,
    },
    LumaRectMiddle {
        p_angle: u16,
        use_tcq: bool,
    },
    LumaRectMiddleMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        is_sb_boundary: bool,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    LumaRectOneSidedAboveMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    LumaRectOneSidedLeftMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        is_sb_boundary: bool,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    LumaRectCardinalMrl {
        direction: IntraCardinalDirection,
        mrl_index: usize,
        above_mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    LumaRectOneSidedAbove {
        p_angle: u16,
        use_tcq: bool,
    },
    LumaRectOneSidedLeft {
        p_angle: u16,
        use_tcq: bool,
    },
    LumaRectCardinal {
        direction: IntraCardinalDirection,
        use_tcq: bool,
    },
    LumaRectPaeth {
        use_tcq: bool,
    },
    Chroma {
        mode: SupportedChromaMode,
        dpcm: Option<DpcmDirection>,
    },
    ChromaDirectional {
        mode: SupportedChromaMode,
        angle_delta_uv: i8,
        dpcm: Option<DpcmDirection>,
    },
    ChromaOneSided(u16, Option<DpcmDirection>),
    ChromaMiddle(u16, Option<DpcmDirection>),
    ChromaCfl {
        params: CflParams,
        cfl_ds_filter_index: u8,
        sb_mib: usize,
    },
    Rect {
        use_tcq: bool,
    },
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
