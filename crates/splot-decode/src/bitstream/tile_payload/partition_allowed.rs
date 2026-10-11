// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.2 partition implied and allowed-set derivation.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-ALLOWED-BOUNDARY`.

use super::cdf::context::{PartitionContextInput, RectPartitionType, SquareSplitContextInput};
use super::partition::{AllowedPartitions, PartitionType, ReadPartitionDecisionInput};
use super::partition_size::{
    BlockSize, PartitionSizeError, h_partition_midsize, partition_subsize,
};

const BLOCK_4X4: usize = 0;
const BLOCK_4X8: usize = 1;
const BLOCK_8X4: usize = 2;
const BLOCK_8X8: usize = 3;
const BLOCK_8X16: usize = 4;
const BLOCK_16X8: usize = 5;
const BLOCK_64X64: usize = 12;
const BLOCK_64X128: usize = 13;
const BLOCK_128X64: usize = 14;
const BLOCK_128X256: usize = 16;
const BLOCK_256X128: usize = 17;
const BLOCK_4X16: usize = 19;
const BLOCK_16X4: usize = 20;
const BLOCK_8X32: usize = 21;
const BLOCK_32X8: usize = 22;
const BLOCK_4X32: usize = 25;

#[derive(Clone, Copy)]
struct BlockGeometry {
    wide_4x4: usize,
    high_4x4: usize,
}

impl BlockGeometry {
    fn new(block_size: BlockSize) -> Result<Self, PartitionAllowedError> {
        Ok(Self {
            wide_4x4: block_size.num_4x4_wide()?,
            high_4x4: block_size.num_4x4_high()?,
        })
    }

    const fn half_wide_4x4(self) -> usize {
        self.wide_4x4 >> 1
    }

    const fn half_high_4x4(self) -> usize {
        self.high_4x4 >> 1
    }

    const fn width_samples(self) -> usize {
        self.wide_4x4 * 4
    }

    const fn height_samples(self) -> usize {
        self.high_4x4 * 4
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PartitionTreeType {
    Shared,
    LumaPart,
    ChromaPart,
}

impl PartitionTreeType {
    const fn is_luma_part(self) -> bool {
        matches!(self, Self::LumaPart)
    }

    const fn is_chroma_part(self) -> bool {
        matches!(self, Self::ChromaPart)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionFeatureFlags {
    enable_ext_partitions: bool,
    enable_uneven_4way_partitions: bool,
}

impl PartitionFeatureFlags {
    #[must_use]
    pub(crate) const fn new(
        enable_ext_partitions: bool,
        enable_uneven_4way_partitions: bool,
    ) -> Self {
        Self {
            enable_ext_partitions,
            enable_uneven_4way_partitions,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionAllowedInput {
    r: usize,
    c: usize,
    mi_rows: usize,
    mi_cols: usize,
    b_size: BlockSize,
    tree_type: PartitionTreeType,
    subsampling_x: bool,
    subsampling_y: bool,
    features: PartitionFeatureFlags,
    frame_is_intra: bool,
    mixed_region: bool,
    max_pb_aspect_ratio: usize,
    has_chroma: bool,
    chroma_offset: bool,
    num_planes: usize,
    known_chroma_luma_partition: Option<PartitionType>,
}

impl PartitionAllowedInput {
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    pub(crate) fn new(
        r: usize,
        c: usize,
        mi_rows: usize,
        mi_cols: usize,
        b_size: usize,
        tree_type: PartitionTreeType,
        subsampling_x: bool,
        subsampling_y: bool,
        features: PartitionFeatureFlags,
        frame_is_intra: bool,
        mixed_region: bool,
        max_pb_aspect_ratio: usize,
        has_chroma: bool,
        chroma_offset: bool,
        num_planes: usize,
        known_chroma_luma_partition: Option<PartitionType>,
    ) -> Result<Self, PartitionAllowedError> {
        Ok(Self {
            r,
            c,
            mi_rows,
            mi_cols,
            b_size: BlockSize::new(b_size)?,
            tree_type,
            subsampling_x,
            subsampling_y,
            features,
            frame_is_intra,
            mixed_region,
            max_pb_aspect_ratio,
            has_chroma,
            chroma_offset,
            num_planes,
            known_chroma_luma_partition,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionDecisionFacts {
    implied_partition: Option<PartitionType>,
    allowed: AllowedPartitions,
    rect_type: Option<RectPartitionType>,
}

impl PartitionDecisionFacts {
    pub(crate) fn read_partition_decision_input<'a>(
        self,
        bru_active: bool,
        partition_context: PartitionContextInput<'a>,
        square_split_context: SquareSplitContextInput<'a>,
    ) -> ReadPartitionDecisionInput<'a> {
        ReadPartitionDecisionInput::new(
            self.allowed,
            self.implied_partition,
            bru_active,
            self.rect_type,
            partition_context,
            square_split_context,
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PartitionAllowedError {
    #[error("partition allowance size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        coordinate: &'static str,
        base: usize,
        offset: usize,
    },
    #[error("{operation} overflow: {left} * {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
}

pub(crate) fn partition_decision_facts(
    input: PartitionAllowedInput,
) -> Result<PartitionDecisionFacts, PartitionAllowedError> {
    Ok(PartitionDecisionFacts {
        implied_partition: partition_implied(input)?,
        allowed: init_allowed_partitions(input)?,
        rect_type: rect_type_implied_by_bsize(input.b_size, input.tree_type),
    })
}

const MEMO_SLOTS: usize = 29 * 3 * 8;

/// [`partition_decision_facts`] of blocks wholly inside the frame, keyed by
/// size, tree type and the three chroma/region flags. Inside the frame the
/// facts do not depend on the position, so one memo is valid while the other
/// frame-level inputs stay fixed, which holds for one tile walk. A slot
/// packs the facts into 16 bits; 0 marks an empty slot, because at least one
/// partition is always allowed.
pub(crate) struct PartitionDecisionMemo {
    slots: [u16; MEMO_SLOTS],
}

impl Default for PartitionDecisionMemo {
    fn default() -> Self {
        Self {
            slots: [0; MEMO_SLOTS],
        }
    }
}

impl PartitionDecisionMemo {
    pub(crate) fn facts(
        &mut self,
        input: PartitionAllowedInput,
    ) -> Result<PartitionDecisionFacts, PartitionAllowedError> {
        let Some(slot) = interior_memo_slot(input).and_then(|slot| self.slots.get_mut(slot)) else {
            return partition_decision_facts(input);
        };
        if *slot != 0 {
            return Ok(PartitionDecisionFacts::unpacked(*slot));
        }
        let facts = partition_decision_facts(input)?;
        *slot = facts.packed();
        Ok(facts)
    }
}

impl PartitionDecisionFacts {
    fn packed(self) -> u16 {
        let allowed = PartitionType::ALL.into_iter().fold(0, |bits, partition| {
            bits | u16::from(self.allowed.contains(partition)) << partition.index()
        });
        let implied = self
            .implied_partition
            .map_or(PartitionType::ALL.len(), PartitionType::index) as u16;
        let rect = match self.rect_type {
            None => 0,
            Some(RectPartitionType::Horz) => 1,
            Some(RectPartitionType::Vert) => 2,
        };
        allowed | implied << 10 | rect << 14
    }

    fn unpacked(bits: u16) -> Self {
        Self {
            implied_partition: PartitionType::ALL
                .get(usize::from(bits >> 10 & 15))
                .copied(),
            allowed: AllowedPartitions::new(core::array::from_fn(|index| bits >> index & 1 != 0)),
            rect_type: match bits >> 14 {
                1 => Some(RectPartitionType::Horz),
                2 => Some(RectPartitionType::Vert),
                _ => None,
            },
        }
    }
}

fn interior_memo_slot(input: PartitionAllowedInput) -> Option<usize> {
    if input.known_chroma_luma_partition.is_some() {
        return None;
    }
    let geometry = BlockGeometry::new(input.b_size).ok()?;
    if input.r.checked_add(geometry.high_4x4)? > input.mi_rows
        || input.c.checked_add(geometry.wide_4x4)? > input.mi_cols
    {
        return None;
    }
    let tree = input.tree_type as usize;
    let flags = usize::from(input.has_chroma) << 2
        | usize::from(input.chroma_offset) << 1
        | usize::from(input.mixed_region);
    Some((input.b_size.index() * 3 + tree) * 8 + flags)
}

pub(crate) fn get_plane_residual_size(
    sub_size: BlockSize,
    plane: usize,
    subsampling_x: bool,
    subsampling_y: bool,
) -> Result<Option<BlockSize>, PartitionAllowedError> {
    let subx = plane > 0 && subsampling_x;
    let suby = plane > 0 && subsampling_y;
    let raw_width_4x4 = subsampled_4x4(sub_size.num_4x4_wide()?, subx);
    let raw_height_4x4 = subsampled_4x4(sub_size.num_4x4_high()?, suby);

    if (subx && raw_width_4x4 == 0 && !suby && raw_height_4x4 > 1)
        || (suby && raw_height_4x4 == 0 && !subx && raw_width_4x4 > 1)
    {
        return Ok(None);
    }

    let width_4x4 = raw_width_4x4.max(1);
    let height_4x4 = raw_height_4x4.max(1);
    Ok(BlockSize::from_4x4_dimensions(width_4x4, height_4x4))
}

pub(crate) fn rect_type_implied_by_bsize(
    b_size: BlockSize,
    tree_type: PartitionTreeType,
) -> Option<RectPartitionType> {
    match b_size.index() {
        BLOCK_4X8 | BLOCK_64X128 | BLOCK_128X256 | BLOCK_4X16 => Some(RectPartitionType::Horz),
        BLOCK_8X4 | BLOCK_128X64 | BLOCK_256X128 | BLOCK_16X4 => Some(RectPartitionType::Vert),
        BLOCK_8X16 | BLOCK_8X32 if tree_type.is_chroma_part() => Some(RectPartitionType::Horz),
        BLOCK_16X8 | BLOCK_32X8 if tree_type.is_chroma_part() => Some(RectPartitionType::Vert),
        _ => None,
    }
}

pub(crate) fn partition_implied_at_boundary(
    input: PartitionAllowedInput,
) -> Result<Option<PartitionType>, PartitionAllowedError> {
    let geometry = BlockGeometry::new(input.b_size)?;
    let has_rows = checked_less_than("r", input.r, geometry.half_high_4x4(), input.mi_rows)?;
    let has_cols = checked_less_than("c", input.c, geometry.half_wide_4x4(), input.mi_cols)?;
    if has_rows && has_cols {
        return Ok(None);
    }

    if geometry.wide_4x4 == geometry.high_4x4 {
        return Ok(Some(if has_rows {
            PartitionType::Vert
        } else {
            PartitionType::Horz
        }));
    }
    if geometry.high_4x4 > geometry.wide_4x4 {
        if !has_rows {
            return Ok(Some(PartitionType::Horz));
        }
        let sub_has_cols = checked_less_than("c", input.c, geometry.wide_4x4 >> 2, input.mi_cols)?;
        if geometry.wide_4x4 >= 4 && !sub_has_cols {
            return Ok(Some(PartitionType::Horz));
        }
    } else {
        if !has_cols {
            return Ok(Some(PartitionType::Vert));
        }
        let sub_has_rows = checked_less_than("r", input.r, geometry.high_4x4 >> 2, input.mi_rows)?;
        if geometry.high_4x4 >= 4 && !sub_has_rows {
            return Ok(Some(PartitionType::Vert));
        }
    }
    Ok(None)
}

pub(crate) fn partition_implied(
    input: PartitionAllowedInput,
) -> Result<Option<PartitionType>, PartitionAllowedError> {
    if input.b_size.index() == BLOCK_4X4 || input.b_size.index() >= BLOCK_4X32 {
        return Ok(Some(PartitionType::None));
    }
    if input.tree_type.is_chroma_part() && input.b_size.index() == BLOCK_8X8 {
        return Ok(Some(PartitionType::None));
    }
    if input.tree_type.is_chroma_part()
        && input.b_size.index() == BLOCK_64X64
        && let Some(partition) = input.known_chroma_luma_partition
    {
        return Ok(Some(partition));
    }
    partition_implied_at_boundary(input)
}

pub(crate) fn is_partition_allowed(
    input: PartitionAllowedInput,
    partition: PartitionType,
) -> Result<bool, PartitionAllowedError> {
    let Some(sub_size) = partition_subsize(partition, input.b_size)? else {
        return Ok(false);
    };
    if !input.frame_is_intra && input.mixed_region && sub_size.index() == BLOCK_4X4 {
        return Ok(false);
    }
    let partition_rect_type = partition_rect_type(partition);
    if let Some(rect_type) = rect_type_implied_by_bsize(input.b_size, input.tree_type)
        && partition_rect_type.is_some_and(|partition_rect_type| partition_rect_type != rect_type)
    {
        return Ok(false);
    }

    let sub_geometry = BlockGeometry::new(sub_size)?;
    let bw = sub_geometry.width_samples();
    let bh = sub_geometry.height_samples();
    if bw > checked_mul("bh * MaxPbAspectRatio", bh, input.max_pb_aspect_ratio)?
        || bh > checked_mul("bw * MaxPbAspectRatio", bw, input.max_pb_aspect_ratio)?
    {
        if partition == PartitionType::None {
            return Ok(false);
        }
        if bw >= checked_mul("bh * 8", bh, 8)? || bh >= checked_mul("bw * 8", bw, 8)? {
            return Ok(false);
        }
    }

    let block_geometry = BlockGeometry::new(input.b_size)?;
    let mut chroma_offset = input.chroma_offset;
    if input.has_chroma && !input.tree_type.is_chroma_part() && !chroma_offset {
        chroma_offset = is_chroma_offset_for_partition(input, partition, sub_size)?;
    }
    if ((input.has_chroma && !chroma_offset && !input.tree_type.is_luma_part())
        || check_chroma(input, block_geometry)?)
        && get_plane_residual_size(sub_size, 1, input.subsampling_x, input.subsampling_y)?.is_none()
    {
        return Ok(false);
    }

    if !partition_feature_allowed(input, partition, block_geometry) {
        return Ok(false);
    }
    if partition == PartitionType::None {
        let has_rows =
            checked_less_than("r", input.r, block_geometry.half_high_4x4(), input.mi_rows)?;
        let has_cols =
            checked_less_than("c", input.c, block_geometry.half_wide_4x4(), input.mi_cols)?;
        if (!input.tree_type.is_chroma_part() || input.b_size.index() != BLOCK_8X8)
            && (!has_rows || !has_cols)
        {
            return Ok(false);
        }
    }
    if input.has_chroma && !input.tree_type.is_luma_part() && input.num_planes > 1 && chroma_offset
    {
        return chroma_offset_block_coded(input, partition, sub_size);
    }
    Ok(true)
}

pub(crate) fn init_allowed_partitions(
    input: PartitionAllowedInput,
) -> Result<AllowedPartitions, PartitionAllowedError> {
    let mut flags = [false; PartitionType::ALL.len()];
    let mut any_allowed = false;
    for partition in PartitionType::ALL {
        let good = is_partition_allowed(input, partition)?;
        flags[partition.index()] = good;
        any_allowed |= good;
    }
    if !any_allowed {
        flags[PartitionType::None.index()] = true;
    }
    Ok(AllowedPartitions::new(flags))
}

fn partition_feature_allowed(
    input: PartitionAllowedInput,
    partition: PartitionType,
    block_geometry: BlockGeometry,
) -> bool {
    let Some(rect_type) = partition_rect_type(partition) else {
        return true;
    };
    if matches!(
        partition,
        PartitionType::Horz3
            | PartitionType::Vert3
            | PartitionType::Horz4A
            | PartitionType::Horz4B
            | PartitionType::Vert4A
            | PartitionType::Vert4B
    ) && !is_ext_partition_allowed(input, rect_type, block_geometry)
    {
        return false;
    }
    if matches!(
        partition,
        PartitionType::Horz4A
            | PartitionType::Horz4B
            | PartitionType::Vert4A
            | PartitionType::Vert4B
    ) {
        return is_uneven_4way_partition_allowed(input, rect_type, block_geometry);
    }
    true
}

fn is_ext_partition_allowed(
    input: PartitionAllowedInput,
    rect_type: RectPartitionType,
    block_geometry: BlockGeometry,
) -> bool {
    if !input.features.enable_ext_partitions {
        return false;
    }
    let width = block_geometry.width_samples();
    let height = block_geometry.height_samples();
    !input.tree_type.is_chroma_part()
        || (rect_type == RectPartitionType::Horz && height > 16 && width > 8)
        || (rect_type == RectPartitionType::Vert && width > 16 && height > 8)
}

fn is_uneven_4way_partition_allowed(
    input: PartitionAllowedInput,
    rect_type: RectPartitionType,
    block_geometry: BlockGeometry,
) -> bool {
    if !input.features.enable_uneven_4way_partitions {
        return false;
    }
    let width = block_geometry.width_samples();
    let height = block_geometry.height_samples();
    !input.tree_type.is_chroma_part()
        || (rect_type == RectPartitionType::Horz && height == 64)
        || (rect_type == RectPartitionType::Vert && width == 64)
}
fn check_chroma(
    input: PartitionAllowedInput,
    block_geometry: BlockGeometry,
) -> Result<bool, PartitionAllowedError> {
    if get_plane_residual_size(input.b_size, 1, input.subsampling_x, input.subsampling_y)?.is_none()
    {
        return Ok(false);
    }
    Ok(input.tree_type.is_luma_part()
        && block_geometry.width_samples() >= 64
        && block_geometry.height_samples() >= 64)
}

fn is_chroma_offset_for_partition(
    input: PartitionAllowedInput,
    partition: PartitionType,
    sub_size: BlockSize,
) -> Result<bool, PartitionAllowedError> {
    if is_chroma_offset_for_subsize(input, sub_size)? {
        return Ok(true);
    }
    if partition == PartitionType::Horz3 {
        let middle_chroma = input.b_size.index() == BLOCK_8X32 && input.subsampling_x;
        if !middle_chroma
            && let Some(midsize) = h_partition_midsize(input.b_size)?
            && is_chroma_offset_for_subsize(input, midsize)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_chroma_offset_for_subsize(
    input: PartitionAllowedInput,
    sub_size: BlockSize,
) -> Result<bool, PartitionAllowedError> {
    Ok((input.subsampling_y && sub_size.mi_height_log2()? == 0)
        || (input.subsampling_x && sub_size.mi_width_log2()? == 0))
}

fn chroma_offset_block_coded(
    input: PartitionAllowedInput,
    partition: PartitionType,
    sub_size: BlockSize,
) -> Result<bool, PartitionAllowedError> {
    match partition {
        PartitionType::Horz => Ok(block_coded(
            input,
            checked_add("r", input.r, input.b_size.num_4x4_high()? >> 1)?,
            input.c,
        )),
        PartitionType::Vert => Ok(block_coded(
            input,
            input.r,
            checked_add("c", input.c, input.b_size.num_4x4_wide()? >> 1)?,
        )),
        PartitionType::Horz3 => Ok(block_coded(
            input,
            checked_scaled_add("r", input.r, 3, (input.b_size.num_4x4_high()? >> 1) >> 1)?,
            input.c,
        )),
        PartitionType::Vert3 => Ok(block_coded(
            input,
            input.r,
            checked_scaled_add("c", input.c, 3, (input.b_size.num_4x4_wide()? >> 1) >> 1)?,
        )),
        PartitionType::Horz4A | PartitionType::Horz4B => Ok(block_coded(
            input,
            checked_scaled_add("r", input.r, 7, sub_size.num_4x4_high()?)?,
            input.c,
        )),
        PartitionType::Vert4A | PartitionType::Vert4B => Ok(block_coded(
            input,
            input.r,
            checked_scaled_add("c", input.c, 7, sub_size.num_4x4_wide()?)?,
        )),
        _ => Ok(true),
    }
}

fn checked_less_than(
    coordinate: &'static str,
    base: usize,
    offset: usize,
    limit: usize,
) -> Result<bool, PartitionAllowedError> {
    Ok(checked_add(coordinate, base, offset)? < limit)
}

fn checked_scaled_add(
    coordinate: &'static str,
    base: usize,
    scale: usize,
    value: usize,
) -> Result<usize, PartitionAllowedError> {
    checked_add(
        coordinate,
        base,
        checked_mul("partition coordinate offset", scale, value)?,
    )
}

fn checked_add(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, PartitionAllowedError> {
    base.checked_add(offset)
        .ok_or(PartitionAllowedError::CoordinateOverflow {
            coordinate,
            base,
            offset,
        })
}

fn checked_mul(
    operation: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, PartitionAllowedError> {
    left.checked_mul(right)
        .ok_or(PartitionAllowedError::ArithmeticOverflow {
            operation,
            left,
            right,
        })
}

const fn subsampled_4x4(value: usize, subsampled: bool) -> usize {
    if subsampled { value >> 1 } else { value }
}

const fn block_coded(input: PartitionAllowedInput, r: usize, c: usize) -> bool {
    r < input.mi_rows && c < input.mi_cols
}

const fn partition_rect_type(partition: PartitionType) -> Option<RectPartitionType> {
    match partition {
        PartitionType::Horz
        | PartitionType::Horz3
        | PartitionType::Horz4A
        | PartitionType::Horz4B => Some(RectPartitionType::Horz),
        PartitionType::Vert
        | PartitionType::Vert3
        | PartitionType::Vert4A
        | PartitionType::Vert4B => Some(RectPartitionType::Vert),
        PartitionType::None | PartitionType::Split => None,
    }
}

#[cfg(test)]
mod memo_tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const FAR: usize = 1 << 12;

    fn expand(
        inputs: Vec<PartitionAllowedInput>,
        count: usize,
        vary: impl Fn(PartitionAllowedInput, usize) -> PartitionAllowedInput,
    ) -> Vec<PartitionAllowedInput> {
        inputs
            .into_iter()
            .flat_map(|input| (0..count).map(move |index| (input, index)))
            .map(|(input, index)| vary(input, index))
            .collect()
    }

    fn far_input() -> PartitionAllowedInput {
        PartitionAllowedInput::new(
            FAR,
            FAR,
            2 * FAR,
            2 * FAR,
            0,
            PartitionTreeType::Shared,
            false,
            false,
            PartitionFeatureFlags::new(false, false),
            false,
            false,
            8,
            false,
            false,
            1,
            None,
        )
        .unwrap()
    }

    fn frame_keys() -> Vec<PartitionAllowedInput> {
        let keys = expand(vec![far_input()], 4, |input, i| PartitionAllowedInput {
            subsampling_x: i & 2 != 0,
            subsampling_y: i & 1 != 0,
            ..input
        });
        let keys = expand(keys, 4, |input, i| PartitionAllowedInput {
            features: PartitionFeatureFlags::new(i & 2 != 0, i & 1 != 0),
            ..input
        });
        let keys = expand(keys, 2, |input, i| PartitionAllowedInput {
            frame_is_intra: i != 0,
            ..input
        });
        let keys = expand(keys, 3, |input, i| PartitionAllowedInput {
            max_pb_aspect_ratio: 2 << i,
            ..input
        });
        expand(keys, 2, |input, i| PartitionAllowedInput {
            num_planes: 1 + 2 * i,
            ..input
        })
    }

    fn slots(frame: PartitionAllowedInput) -> Vec<PartitionAllowedInput> {
        let slots = expand(vec![frame], 29, |input, i| PartitionAllowedInput {
            b_size: BlockSize::new(i).unwrap(),
            ..input
        });
        let trees = [
            PartitionTreeType::Shared,
            PartitionTreeType::LumaPart,
            PartitionTreeType::ChromaPart,
        ];
        let slots = expand(slots, 3, |input, i| PartitionAllowedInput {
            tree_type: trees[i],
            ..input
        });
        expand(slots, 8, |input, i| PartitionAllowedInput {
            has_chroma: i & 4 != 0,
            chroma_offset: i & 2 != 0,
            mixed_region: i & 1 != 0,
            ..input
        })
    }

    #[test]
    fn memo_matches_direct_facts_for_every_slot_and_frame_key() {
        for frame in frame_keys() {
            let mut memo = PartitionDecisionMemo::default();
            let mut seen = [false; MEMO_SLOTS];
            for far in slots(frame) {
                let slot = interior_memo_slot(far).unwrap();
                assert!(!seen[slot], "slot {slot} is shared");
                seen[slot] = true;
                let geometry = BlockGeometry::new(far.b_size).unwrap();
                let tight = PartitionAllowedInput {
                    r: far.mi_rows - geometry.high_4x4,
                    c: far.mi_cols - geometry.wide_4x4,
                    ..far
                };
                let outside = PartitionAllowedInput {
                    r: tight.r + 1,
                    ..tight
                };
                let overflow = PartitionAllowedInput {
                    c: usize::MAX - 1,
                    ..far
                };
                for input in [far, tight, outside, overflow] {
                    let direct = partition_decision_facts(input).ok();
                    assert_eq!(memo.facts(input).ok(), direct, "{input:?}");
                }
            }
            assert!(seen.iter().all(|seen| *seen));
        }
    }

    #[test]
    fn memo_takes_the_direct_path_at_the_frame_edge_and_for_forced_chroma() {
        let interior = PartitionAllowedInput {
            b_size: BlockSize::new(BLOCK_64X64).unwrap(),
            ..far_input()
        };
        let edge = PartitionAllowedInput {
            r: interior.mi_rows - 4,
            ..interior
        };
        let mut memo = PartitionDecisionMemo::default();
        let inside = memo.facts(interior).unwrap();
        let at_edge = memo.facts(edge).unwrap();
        assert_eq!(at_edge, partition_decision_facts(edge).unwrap());
        assert_eq!(at_edge.implied_partition, Some(PartitionType::Horz));
        assert_ne!(at_edge, inside);

        let chroma = PartitionAllowedInput {
            tree_type: PartitionTreeType::ChromaPart,
            ..interior
        };
        let forced = PartitionAllowedInput {
            known_chroma_luma_partition: Some(PartitionType::Vert),
            ..chroma
        };
        assert_eq!(memo.facts(chroma).unwrap().implied_partition, None);
        assert_eq!(
            memo.facts(forced).unwrap().implied_partition,
            Some(PartitionType::Vert)
        );
    }
}

#[cfg(test)]
mod spec_table_tests;

#[cfg(test)]
#[path = "partition_allowed_tests.rs"]
mod tests;
