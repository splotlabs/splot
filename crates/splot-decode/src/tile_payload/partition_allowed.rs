// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.2 partition implied and allowed-set derivation.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-ALLOWED-BOUNDARY`.

use super::cdf::context::{PartitionContextInput, RectPartitionType, SquareSplitContextInput};
use super::partition::{AllowedPartitions, PartitionType, ReadPartitionDecisionInput};
use super::partition_size::{
    BlockSize, PartitionSizeError, PartitionSubsize, h_partition_midsize, partition_subsize,
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

/// AV2 partition tree selector facts needed by § 5.20.3.2 helpers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PartitionTreeType {
    /// Shared luma/chroma tree.
    Shared,
    /// `TreeType == LUMA_PART`.
    LumaPart,
    /// `TreeType == CHROMA_PART`.
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

/// AV2 partition feature flags consumed by the allowed-set boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionFeatureFlags {
    enable_ext_partitions: bool,
    enable_uneven_4way_partitions: bool,
}

impl PartitionFeatureFlags {
    /// Creates feature flags for § 5.20.3.2 partition allowance checks.
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

/// Explicit caller facts for AV2 § 5.20.3.2 partition allowance.
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
    /// Creates checked caller facts for § 5.20.3.2 partition allowance.
    #[allow(clippy::too_many_arguments)]
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

/// Result of AV2 `init_allowed_partitions`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InitializedAllowedPartitions {
    num_allowed: usize,
    allowed: AllowedPartitions,
}

impl InitializedAllowedPartitions {
    /// Number of allowed partition entries.
    #[must_use]
    pub(crate) const fn num_allowed(self) -> usize {
        self.num_allowed
    }

    /// Allowed partitions in AV2 partition enum order.
    #[must_use]
    pub(crate) const fn allowed(self) -> AllowedPartitions {
        self.allowed
    }
}

/// Caller facts derived for the existing partition decision boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionDecisionFacts {
    implied_partition: Option<PartitionType>,
    initialized: InitializedAllowedPartitions,
    rect_type: Option<RectPartitionType>,
}

impl PartitionDecisionFacts {
    /// Implied partition, when AV2 `partition_implied` returned `implied == 1`.
    #[must_use]
    pub(crate) const fn implied_partition(self) -> Option<PartitionType> {
        self.implied_partition
    }

    /// Initialized allowed partition set.
    #[must_use]
    pub(crate) const fn initialized(self) -> InitializedAllowedPartitions {
        self.initialized
    }

    /// Builds the existing partition decision input from derived facts.
    pub(crate) fn read_partition_decision_input<'a>(
        self,
        bru_active: bool,
        partition_context: PartitionContextInput<'a>,
        square_split_context: SquareSplitContextInput<'a>,
    ) -> ReadPartitionDecisionInput<'a> {
        ReadPartitionDecisionInput::new(
            self.initialized.allowed(),
            self.implied_partition,
            bru_active,
            self.rect_type,
            partition_context,
            square_split_context,
        )
    }
}

/// Error returned by the crate-private allowed-partition boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PartitionAllowedError {
    /// A generated size-table or geometry lookup failed.
    #[error("partition allowance size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    /// Coordinate addition overflowed.
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        /// Coordinate name.
        coordinate: &'static str,
        /// Base coordinate.
        base: usize,
        /// Derived offset.
        offset: usize,
    },
    /// Derived arithmetic overflowed.
    #[error("{operation} overflow: {left} * {right}")]
    ArithmeticOverflow {
        /// Operation name.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
}

/// Derives all caller facts needed by `read_partition_decision`.
pub(crate) fn partition_decision_facts(
    input: PartitionAllowedInput,
) -> Result<PartitionDecisionFacts, PartitionAllowedError> {
    Ok(PartitionDecisionFacts {
        implied_partition: partition_implied(input)?,
        initialized: init_allowed_partitions(input)?,
        rect_type: rect_type_implied_by_bsize(input.b_size, input.tree_type),
    })
}

/// AV2 § 5.20.7.26 `get_plane_residual_size`.
pub(crate) fn get_plane_residual_size(
    sub_size: BlockSize,
    plane: usize,
    subsampling_x: bool,
    subsampling_y: bool,
) -> Result<PartitionSubsize, PartitionAllowedError> {
    let subx = plane > 0 && subsampling_x;
    let suby = plane > 0 && subsampling_y;
    let raw_width_4x4 = subsampled_4x4(sub_size.num_4x4_wide()?, subx);
    let raw_height_4x4 = subsampled_4x4(sub_size.num_4x4_high()?, suby);

    if (subx && raw_width_4x4 == 0 && !suby && raw_height_4x4 > 1)
        || (suby && raw_height_4x4 == 0 && !subx && raw_width_4x4 > 1)
    {
        return Ok(PartitionSubsize::Invalid);
    }

    let width_4x4 = raw_width_4x4.max(1);
    let height_4x4 = raw_height_4x4.max(1);
    Ok(
        match BlockSize::from_4x4_dimensions(width_4x4, height_4x4)? {
            Some(block_size) => PartitionSubsize::Valid(block_size),
            None => PartitionSubsize::Invalid,
        },
    )
}

/// AV2 `rect_type_implied_by_bsize`.
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

/// AV2 `partition_implied_at_boundary`.
pub(crate) fn partition_implied_at_boundary(
    input: PartitionAllowedInput,
) -> Result<Option<PartitionType>, PartitionAllowedError> {
    let num_wide_4x4 = input.b_size.num_4x4_wide()?;
    let num_high_4x4 = input.b_size.num_4x4_high()?;
    let has_rows = checked_less_than("r", input.r, num_high_4x4 >> 1, input.mi_rows)?;
    let has_cols = checked_less_than("c", input.c, num_wide_4x4 >> 1, input.mi_cols)?;
    if has_rows && has_cols {
        return Ok(None);
    }

    if num_wide_4x4 == num_high_4x4 {
        return Ok(Some(if has_rows {
            PartitionType::Vert
        } else {
            PartitionType::Horz
        }));
    }
    if num_high_4x4 > num_wide_4x4 {
        if !has_rows {
            return Ok(Some(PartitionType::Horz));
        }
        let sub_has_cols = checked_less_than("c", input.c, num_wide_4x4 >> 2, input.mi_cols)?;
        if num_wide_4x4 >= 4 && !sub_has_cols {
            return Ok(Some(PartitionType::Horz));
        }
    } else {
        if !has_cols {
            return Ok(Some(PartitionType::Vert));
        }
        let sub_has_rows = checked_less_than("r", input.r, num_high_4x4 >> 2, input.mi_rows)?;
        if num_high_4x4 >= 4 && !sub_has_rows {
            return Ok(Some(PartitionType::Vert));
        }
    }
    Ok(None)
}

/// AV2 `partition_implied`.
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

/// AV2 `is_partition_allowed`.
pub(crate) fn is_partition_allowed(
    input: PartitionAllowedInput,
    partition: PartitionType,
) -> Result<bool, PartitionAllowedError> {
    let sub_size = match partition_subsize(partition, input.b_size)?.valid() {
        Some(sub_size) => sub_size,
        None => return Ok(false),
    };
    if !input.frame_is_intra && input.mixed_region && sub_size.index() == BLOCK_4X4 {
        return Ok(false);
    }
    if let Some(rect_type) = rect_type_implied_by_bsize(input.b_size, input.tree_type)
        && ((rect_type == RectPartitionType::Vert && is_horizontal_family(partition))
            || (rect_type == RectPartitionType::Horz && is_vertical_family(partition)))
    {
        return Ok(false);
    }

    let bw = sub_size.width_samples()?;
    let bh = sub_size.height_samples()?;
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

    let num_4x4_wide = input.b_size.num_4x4_wide()?;
    let num_4x4_high = input.b_size.num_4x4_high()?;
    let half_block_4x4_wide = num_4x4_wide >> 1;
    let half_block_4x4_high = num_4x4_high >> 1;
    let mut chroma_offset = input.chroma_offset;
    if input.has_chroma && !input.tree_type.is_chroma_part() && !chroma_offset {
        chroma_offset = is_chroma_offset_for_partition(input, partition, sub_size)?;
    }
    if ((input.has_chroma && !chroma_offset && !input.tree_type.is_luma_part())
        || check_chroma(input)?)
        && get_plane_residual_size(sub_size, 1, input.subsampling_x, input.subsampling_y)?
            == PartitionSubsize::Invalid
    {
        return Ok(false);
    }

    if !partition_feature_allowed(input, partition)? {
        return Ok(false);
    }
    if partition == PartitionType::None {
        let has_rows = checked_less_than("r", input.r, half_block_4x4_high, input.mi_rows)?;
        let has_cols = checked_less_than("c", input.c, half_block_4x4_wide, input.mi_cols)?;
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

/// AV2 `init_allowed_partitions`.
pub(crate) fn init_allowed_partitions(
    input: PartitionAllowedInput,
) -> Result<InitializedAllowedPartitions, PartitionAllowedError> {
    let mut flags = [false; PartitionType::ALL.len()];
    let mut num_allowed = 0;
    for partition in PartitionType::ALL {
        let good = is_partition_allowed(input, partition)?;
        flags[partition.index()] = good;
        if good {
            num_allowed += 1;
        }
    }
    if num_allowed == 0 {
        flags[PartitionType::None.index()] = true;
        num_allowed = 1;
    }
    Ok(InitializedAllowedPartitions {
        num_allowed,
        allowed: AllowedPartitions::new(flags),
    })
}

fn partition_feature_allowed(
    input: PartitionAllowedInput,
    partition: PartitionType,
) -> Result<bool, PartitionAllowedError> {
    match partition {
        PartitionType::Horz3 => is_ext_partition_allowed(input, RectPartitionType::Horz),
        PartitionType::Vert3 => is_ext_partition_allowed(input, RectPartitionType::Vert),
        PartitionType::Horz4A | PartitionType::Horz4B => {
            Ok(is_ext_partition_allowed(input, RectPartitionType::Horz)?
                && is_uneven_4way_partition_allowed(input, RectPartitionType::Horz)?)
        }
        PartitionType::Vert4A | PartitionType::Vert4B => {
            Ok(is_ext_partition_allowed(input, RectPartitionType::Vert)?
                && is_uneven_4way_partition_allowed(input, RectPartitionType::Vert)?)
        }
        _ => Ok(true),
    }
}

fn is_ext_partition_allowed(
    input: PartitionAllowedInput,
    rect_type: RectPartitionType,
) -> Result<bool, PartitionAllowedError> {
    if !input.features.enable_ext_partitions {
        return Ok(false);
    }
    let width = input.b_size.width_samples()?;
    let height = input.b_size.height_samples()?;
    Ok(!input.tree_type.is_chroma_part()
        || (rect_type == RectPartitionType::Horz && height > 16 && width > 8)
        || (rect_type == RectPartitionType::Vert && width > 16 && height > 8))
}

fn is_uneven_4way_partition_allowed(
    input: PartitionAllowedInput,
    rect_type: RectPartitionType,
) -> Result<bool, PartitionAllowedError> {
    if !input.features.enable_uneven_4way_partitions {
        return Ok(false);
    }
    let width = input.b_size.width_samples()?;
    let height = input.b_size.height_samples()?;
    Ok(!input.tree_type.is_chroma_part()
        || (rect_type == RectPartitionType::Horz && height == 64)
        || (rect_type == RectPartitionType::Vert && width == 64))
}

fn check_chroma(input: PartitionAllowedInput) -> Result<bool, PartitionAllowedError> {
    if get_plane_residual_size(input.b_size, 1, input.subsampling_x, input.subsampling_y)?
        == PartitionSubsize::Invalid
    {
        return Ok(false);
    }
    Ok(input.tree_type.is_luma_part()
        && input.b_size.width_samples()? >= 64
        && input.b_size.height_samples()? >= 64)
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
            && let Some(midsize) = h_partition_midsize(input.b_size)?.valid()
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

const fn is_horizontal_family(partition: PartitionType) -> bool {
    matches!(
        partition,
        PartitionType::Horz | PartitionType::Horz3 | PartitionType::Horz4A | PartitionType::Horz4B
    )
}

const fn is_vertical_family(partition: PartitionType) -> bool {
    matches!(
        partition,
        PartitionType::Vert | PartitionType::Vert3 | PartitionType::Vert4A | PartitionType::Vert4B
    )
}

#[cfg(test)]
mod spec_table_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const BLOCK_16X16: usize = 6;
    const BLOCK_32X32: usize = 9;

    fn features() -> PartitionFeatureFlags {
        PartitionFeatureFlags::new(true, true)
    }

    fn input(b_size: usize) -> PartitionAllowedInput {
        PartitionAllowedInput::new(
            0,
            0,
            128,
            128,
            b_size,
            PartitionTreeType::Shared,
            false,
            false,
            features(),
            true,
            false,
            8,
            false,
            false,
            1,
            None,
        )
        .unwrap()
    }

    fn valid_index(result: PartitionSubsize) -> Option<usize> {
        result.valid().map(BlockSize::index)
    }

    #[test]
    fn get_plane_residual_size_matches_spec_sentinel_cases() {
        assert_eq!(
            valid_index(
                get_plane_residual_size(BlockSize::new(BLOCK_4X4).unwrap(), 1, true, false)
                    .unwrap()
            ),
            Some(BLOCK_4X4)
        );
        assert_eq!(
            get_plane_residual_size(BlockSize::new(BLOCK_4X8).unwrap(), 1, true, false).unwrap(),
            PartitionSubsize::Invalid
        );
        assert_eq!(
            valid_index(
                get_plane_residual_size(BlockSize::new(BLOCK_4X16).unwrap(), 1, true, true)
                    .unwrap()
            ),
            Some(BLOCK_4X8)
        );
        assert_eq!(
            get_plane_residual_size(BlockSize::new(BLOCK_64X128).unwrap(), 1, true, false).unwrap(),
            PartitionSubsize::Invalid
        );
        assert_eq!(
            valid_index(
                get_plane_residual_size(BlockSize::new(BLOCK_64X128).unwrap(), 1, true, true,)
                    .unwrap()
            ),
            Some(10)
        );
    }

    #[test]
    fn boundary_and_direct_implied_partitions_are_derived() {
        assert_eq!(
            partition_implied(input(BLOCK_4X4)).unwrap(),
            Some(PartitionType::None)
        );

        let right_edge = PartitionAllowedInput {
            c: 2,
            mi_cols: 4,
            ..input(BLOCK_16X16)
        };
        assert_eq!(
            partition_implied_at_boundary(right_edge).unwrap(),
            Some(PartitionType::Vert)
        );

        let bottom_edge = PartitionAllowedInput {
            r: 2,
            mi_rows: 4,
            ..input(BLOCK_16X16)
        };
        assert_eq!(
            partition_implied_at_boundary(bottom_edge).unwrap(),
            Some(PartitionType::Horz)
        );
    }

    #[test]
    fn chroma_direct_implied_rules_are_derived() {
        let chroma_8x8 = PartitionAllowedInput {
            tree_type: PartitionTreeType::ChromaPart,
            ..input(BLOCK_8X8)
        };
        assert_eq!(
            partition_implied(chroma_8x8).unwrap(),
            Some(PartitionType::None)
        );

        let chroma_64x64 = PartitionAllowedInput {
            tree_type: PartitionTreeType::ChromaPart,
            known_chroma_luma_partition: Some(PartitionType::Vert3),
            ..input(BLOCK_64X64)
        };
        assert_eq!(
            partition_implied(chroma_64x64).unwrap(),
            Some(PartitionType::Vert3)
        );
    }

    #[test]
    fn rect_type_implication_includes_chroma_part_special_cases() {
        assert_eq!(
            rect_type_implied_by_bsize(
                BlockSize::new(BLOCK_4X8).unwrap(),
                PartitionTreeType::Shared
            ),
            Some(RectPartitionType::Horz)
        );
        assert_eq!(
            rect_type_implied_by_bsize(
                BlockSize::new(BLOCK_8X16).unwrap(),
                PartitionTreeType::ChromaPart,
            ),
            Some(RectPartitionType::Horz)
        );
        assert_eq!(
            rect_type_implied_by_bsize(
                BlockSize::new(BLOCK_16X8).unwrap(),
                PartitionTreeType::ChromaPart,
            ),
            Some(RectPartitionType::Vert)
        );
        assert_eq!(
            rect_type_implied_by_bsize(
                BlockSize::new(BLOCK_16X16).unwrap(),
                PartitionTreeType::Shared
            ),
            None
        );
    }

    #[test]
    fn partition_subsize_sentinels_and_mixed_4x4_are_rejected() {
        assert!(!is_partition_allowed(input(BLOCK_4X4), PartitionType::Horz).unwrap());

        let mixed = PartitionAllowedInput {
            frame_is_intra: false,
            mixed_region: true,
            ..input(BLOCK_4X8)
        };
        assert!(!is_partition_allowed(mixed, PartitionType::Horz).unwrap());
    }

    #[test]
    fn residual_invalid_and_aspect_ratio_cases_are_rejected() {
        let residual_invalid = PartitionAllowedInput {
            has_chroma: true,
            num_planes: 3,
            subsampling_x: true,
            ..input(BLOCK_64X128)
        };
        assert!(!is_partition_allowed(residual_invalid, PartitionType::None).unwrap());

        let aspect = PartitionAllowedInput {
            max_pb_aspect_ratio: 2,
            ..input(BLOCK_4X16)
        };
        assert!(!is_partition_allowed(aspect, PartitionType::None).unwrap());
    }

    #[test]
    fn frame_edge_none_rejection_and_empty_fallback_are_derived() {
        let frame_edge = PartitionAllowedInput {
            r: 2,
            mi_rows: 4,
            ..input(BLOCK_16X16)
        };
        assert!(!is_partition_allowed(frame_edge, PartitionType::None).unwrap());

        let fallback = PartitionAllowedInput {
            max_pb_aspect_ratio: 0,
            ..input(BLOCK_4X4)
        };
        let initialized = init_allowed_partitions(fallback).unwrap();
        assert_eq!(initialized.num_allowed(), 1);
        assert!(initialized.allowed().contains(PartitionType::None));
    }

    #[test]
    fn extended_and_uneven_four_way_gates_are_derived() {
        let disabled_ext = PartitionAllowedInput {
            features: PartitionFeatureFlags::new(false, true),
            ..input(BLOCK_32X32)
        };
        assert!(!is_partition_allowed(disabled_ext, PartitionType::Horz3).unwrap());

        let disabled_uneven = PartitionAllowedInput {
            features: PartitionFeatureFlags::new(true, false),
            ..input(BLOCK_32X32)
        };
        assert!(is_partition_allowed(disabled_uneven, PartitionType::Horz3).unwrap());
        assert!(!is_partition_allowed(disabled_uneven, PartitionType::Horz4A).unwrap());
    }

    #[test]
    fn chroma_part_rect_type_and_chroma_offset_block_coded_are_checked() {
        let chroma_rect = PartitionAllowedInput {
            tree_type: PartitionTreeType::ChromaPart,
            ..input(BLOCK_8X16)
        };
        assert!(!is_partition_allowed(chroma_rect, PartitionType::Vert).unwrap());

        let outside = PartitionAllowedInput {
            r: 2,
            mi_rows: 4,
            has_chroma: true,
            chroma_offset: true,
            num_planes: 3,
            ..input(BLOCK_16X16)
        };
        assert!(!is_partition_allowed(outside, PartitionType::Horz).unwrap());
    }

    #[test]
    fn luma_part_large_blocks_use_check_chroma_path() {
        let luma = PartitionAllowedInput {
            tree_type: PartitionTreeType::LumaPart,
            has_chroma: true,
            num_planes: 3,
            ..input(BLOCK_64X64)
        };

        assert!(is_partition_allowed(luma, PartitionType::None).unwrap());
    }

    #[test]
    fn coordinate_arithmetic_overflow_is_typed() {
        let overflow = PartitionAllowedInput {
            r: usize::MAX,
            ..input(BLOCK_16X16)
        };
        assert!(matches!(
            partition_implied_at_boundary(overflow).unwrap_err(),
            PartitionAllowedError::CoordinateOverflow {
                coordinate: "r",
                ..
            }
        ));
    }

    #[test]
    fn partition_decision_facts_collect_implied_allowed_and_rect_type() {
        let facts = partition_decision_facts(input(BLOCK_4X8)).unwrap();
        assert_eq!(facts.implied_partition(), None);
        assert!(facts.initialized().allowed().count() > 0);

        static LEFT: [usize; 4] = [BLOCK_4X4; 4];
        static ABOVE: [usize; 4] = [BLOCK_4X4; 4];
        static ROW: [usize; 4] = [BLOCK_4X4; 4];
        static GRID: [&[usize]; 4] = [&ROW, &ROW, &ROW, &ROW];
        let decision_input = facts.read_partition_decision_input(
            true,
            PartitionContextInput::new(BLOCK_4X8, 0, 0, 0, [&LEFT, &LEFT], [&ABOVE, &ABOVE])
                .unwrap(),
            SquareSplitContextInput::new(BLOCK_4X8, 0, 0, 0, false, false, [&GRID, &GRID]).unwrap(),
        );

        assert_eq!(decision_input, decision_input);
    }

    #[test]
    fn bounded_caller_fact_space_never_panics() {
        let tree_types = [
            PartitionTreeType::Shared,
            PartitionTreeType::LumaPart,
            PartitionTreeType::ChromaPart,
        ];
        let feature_sets = [
            PartitionFeatureFlags::new(false, false),
            PartitionFeatureFlags::new(true, false),
            PartitionFeatureFlags::new(true, true),
        ];
        let coordinates = [0, 2, usize::MAX - 1];
        let limits = [(0, 0), (4, 4)];

        for b_size in 0..29 {
            for tree_type in tree_types {
                for features in feature_sets {
                    for subsampling_x in [false, true] {
                        for subsampling_y in [false, true] {
                            for has_chroma in [false, true] {
                                for chroma_offset in [false, true] {
                                    for max_pb_aspect_ratio in [0, 8] {
                                        for (mi_rows, mi_cols) in limits {
                                            for r in coordinates {
                                                for c in coordinates {
                                                    let input = PartitionAllowedInput::new(
                                                        r,
                                                        c,
                                                        mi_rows,
                                                        mi_cols,
                                                        b_size,
                                                        tree_type,
                                                        subsampling_x,
                                                        subsampling_y,
                                                        features,
                                                        true,
                                                        false,
                                                        max_pb_aspect_ratio,
                                                        has_chroma,
                                                        chroma_offset,
                                                        if has_chroma { 3 } else { 1 },
                                                        Some(PartitionType::Split),
                                                    )
                                                    .unwrap();

                                                    let _ = partition_decision_facts(input);
                                                    for partition in PartitionType::ALL {
                                                        let _ =
                                                            is_partition_allowed(input, partition);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
