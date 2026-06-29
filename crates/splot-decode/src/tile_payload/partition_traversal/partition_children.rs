// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 `decode_partition` child-call expansion.
//!
//! Builds the ordered child [`TilePartitionCall`]s for one decoded partition,
//! threading the § 5.20.4.1 chroma-reference geometry to chroma-offset
//! descendants.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-TRAVERSAL-FRONTIER`.

use super::super::partition::PartitionType;
use super::super::partition_allowed::PartitionTreeType;
use super::super::partition_size::{BlockSize, h_partition_midsize};
use super::{
    BLOCK_8X32, BLOCK_32X8, ChromaRefGeometry, TilePartitionCall, TilePartitionFrameFacts,
    TilePartitionTraversalError, checked_add, checked_scaled_add, valid_subsize,
};

/// Up-to-four ordered child calls produced by one `decode_partition` step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TilePartitionChildCalls {
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

    /// The produced child calls in spec decode order.
    pub(super) fn as_slice(&self) -> &[TilePartitionCall] {
        &self.calls[..self.len]
    }
}

/// Expands one decoded partition into its ordered child `decode_partition` calls.
pub(super) fn child_calls(
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
    let inherited_chroma_ref = Some(call.chroma_ref_geometry());
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
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
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
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
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
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                false,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                call.c,
                sub_size,
                parent,
                false,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                checked_add("c", call.c, half_w)?,
                sub_size,
                parent,
                false,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
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
            let middle_chroma_ref = if middle_chroma {
                Some(ChromaRefGeometry::new(
                    checked_add("r", call.r, half_h >> 1)?,
                    call.c,
                    valid_subsize(PartitionType::Horz, call.b_size)?,
                ))
            } else {
                inherited_chroma_ref
            };
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h >> 1)?,
                call.c,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset && !middle_chroma,
                call.tree_type(),
                middle_chroma_ref,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h >> 1)?,
                checked_add("c", call.c, half_w)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset,
                call.tree_type(),
                middle_chroma_ref,
            ))?;
            children.push(child(
                checked_scaled_add("r", call.r, 3, half_h >> 1)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
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
            // AV2 § 5.20.4.1 VERT_3 middleChroma override (spec lines 9325-9329):
            // the two middle children reference the half-width VERT sub-block.
            let middle_chroma_ref = if middle_chroma {
                Some(ChromaRefGeometry::new(
                    call.r,
                    checked_add("c", call.c, half_w >> 1)?,
                    valid_subsize(PartitionType::Vert, call.b_size)?,
                ))
            } else {
                inherited_chroma_ref
            };
            children.push(child(
                call.r,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, half_w >> 1)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset && !middle_chroma,
                call.tree_type(),
                middle_chroma_ref,
            ))?;
            children.push(child(
                checked_add("r", call.r, half_h)?,
                checked_add("c", call.c, half_w >> 1)?,
                middle,
                parent,
                chroma_offset || middle_chroma,
                call.has_chroma && !chroma_offset,
                call.tree_type(),
                middle_chroma_ref,
            ))?;
            children.push(child(
                call.r,
                checked_scaled_add("c", call.c, 3, half_w >> 1)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
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
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                checked_add("r", call.r, num4x4high >> 3)?,
                call.c,
                second,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type(),
                inherited_chroma_ref,
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
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                checked_scaled_add("r", call.r, 7, num4x4high >> 3)?,
                call.c,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
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
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                call.r,
                checked_add("c", call.c, num4x4wide >> 3)?,
                second,
                parent,
                chroma_offset,
                call.has_chroma && !chroma_offset,
                call.tree_type(),
                inherited_chroma_ref,
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
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
            children.push(child(
                call.r,
                checked_scaled_add("c", call.c, 7, num4x4wide >> 3)?,
                sub_size,
                parent,
                chroma_offset,
                call.has_chroma,
                call.tree_type(),
                inherited_chroma_ref,
            ))?;
        }
    }
    for child in &mut children.calls[..children.len] {
        child.set_cfl_allowed_in_sdp(call.cfl_allowed_in_sdp());
    }
    Ok(children)
}

#[allow(clippy::too_many_arguments)]
fn child(
    r: usize,
    c: usize,
    b_size: BlockSize,
    parent_size: Option<BlockSize>,
    chroma_offset: bool,
    has_chroma: bool,
    tree_type: PartitionTreeType,
    chroma_ref: Option<ChromaRefGeometry>,
) -> TilePartitionCall {
    TilePartitionCall::child(
        r,
        c,
        b_size,
        parent_size,
        chroma_offset,
        has_chroma,
        tree_type,
        chroma_ref,
    )
}
