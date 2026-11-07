// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.1 `decode_partition` child-call expansion.

use super::super::partition::PartitionType;
use super::super::partition_size::{BlockSize, h_partition_midsize};
use super::{
    BLOCK_8X32, BLOCK_32X8, ChromaRefGeometry, TilePartitionCall, TilePartitionFrameFacts,
    TilePartitionTraversalError, valid_subsize,
};

/// The most child calls one § 5.20.3.1 partition makes.
pub(crate) const MAX_CHILD_CALLS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TilePartitionChildCalls {
    calls: [TilePartitionCall; MAX_CHILD_CALLS],
    len: usize,
}

impl TilePartitionChildCalls {
    fn new(fill: TilePartitionCall) -> Self {
        Self {
            calls: [fill; MAX_CHILD_CALLS],
            len: 0,
        }
    }

    fn push(&mut self, call: TilePartitionCall) -> Result<(), TilePartitionTraversalError> {
        let Some(slot) = self.calls.get_mut(self.len) else {
            return Err(TilePartitionTraversalError::TooManyChildCalls);
        };
        *slot = call;
        self.len += 1;
        Ok(())
    }

    pub(crate) fn as_slice(&self) -> &[TilePartitionCall] {
        &self.calls[..self.len]
    }
}

/// Where a partition's child calls go, in syntax order.
pub(crate) trait ChildCallSink {
    fn push_call(&mut self, call: TilePartitionCall) -> Result<(), TilePartitionTraversalError>;
}

impl ChildCallSink for TilePartitionChildCalls {
    fn push_call(&mut self, call: TilePartitionCall) -> Result<(), TilePartitionTraversalError> {
        self.push(call)
    }
}

struct ChildCallBuilder<'s, S: ChildCallSink> {
    calls: &'s mut S,
    /// The state every child inherits; each push sets its position, size
    /// and chroma fields.
    template: TilePartitionCall,
    inherited_chroma_ref: ChromaRefGeometry,
    chroma_offset: bool,
}

impl<'s, S: ChildCallSink> ChildCallBuilder<'s, S> {
    #[inline]
    fn new(
        calls: &'s mut S,
        call: TilePartitionCall,
        partition: PartitionType,
        sub_size: BlockSize,
        frame: &TilePartitionFrameFacts,
        chroma_offset: bool,
    ) -> Self {
        let inherited_chroma_ref = call.chroma_ref_geometry();
        let template = TilePartitionCall::child(
            call.r(),
            call.c(),
            call.b_size,
            Some(call.b_size),
            chroma_offset,
            call.has_chroma(),
            call.tree_type(),
            Some(inherited_chroma_ref),
            super::extended_sdp_allowed_for_child(frame, call, partition, sub_size),
            call.intra_region(),
        )
        .with_cfl_allowed_in_sdp(call.cfl_allowed_in_sdp());
        Self {
            calls,
            template,
            inherited_chroma_ref,
            chroma_offset,
        }
    }

    #[inline]
    fn push(
        &mut self,
        r: usize,
        c: usize,
        b_size: BlockSize,
        has_chroma: bool,
    ) -> Result<(), TilePartitionTraversalError> {
        self.push_with_chroma_offset(r, c, b_size, self.chroma_offset, has_chroma)
    }

    #[inline]
    fn push_with_chroma_offset(
        &mut self,
        r: usize,
        c: usize,
        b_size: BlockSize,
        chroma_offset: bool,
        has_chroma: bool,
    ) -> Result<(), TilePartitionTraversalError> {
        self.push_with_chroma_ref(
            r,
            c,
            b_size,
            chroma_offset,
            has_chroma,
            self.inherited_chroma_ref,
        )
    }

    #[inline]
    fn push_with_chroma_ref(
        &mut self,
        r: usize,
        c: usize,
        b_size: BlockSize,
        chroma_offset: bool,
        has_chroma: bool,
        chroma_ref: ChromaRefGeometry,
    ) -> Result<(), TilePartitionTraversalError> {
        let call = self
            .template
            .at_child(r, c, b_size, chroma_offset, has_chroma, chroma_ref);
        self.calls.push_call(call)
    }
}

#[derive(Clone, Copy)]
enum ChildAxis {
    Row,
    Col,
}

impl ChildAxis {
    const fn facts(self) -> (&'static str, PartitionType) {
        match self {
            Self::Row => ("r", PartitionType::Horz),
            Self::Col => ("c", PartitionType::Vert),
        }
    }

    const fn offset(self, call: TilePartitionCall, scale: usize, step: usize) -> (usize, usize) {
        match self {
            Self::Row => (call.r() + scale * step, call.c()),
            Self::Col => (call.r(), call.c() + scale * step),
        }
    }
}

fn middle_partition_size(
    partition: PartitionType,
    b_size: BlockSize,
) -> Result<BlockSize, TilePartitionTraversalError> {
    let Some(midsize) = h_partition_midsize(b_size)? else {
        return Err(TilePartitionTraversalError::InvalidPartitionSubsize {
            partition,
            b_size: b_size.index(),
        });
    };
    Ok(midsize)
}

fn push_two_way_children<S: ChildCallSink>(
    children: &mut ChildCallBuilder<'_, S>,
    call: TilePartitionCall,
    sub_size: BlockSize,
    axis: ChildAxis,
    step: usize,
    has_chroma_before_offset: bool,
) -> Result<(), TilePartitionTraversalError> {
    children.push(call.r(), call.c(), sub_size, has_chroma_before_offset)?;
    let (r, c) = axis.offset(call, 1, step);
    children.push(r, c, sub_size, call.has_chroma())
}

fn push_split_children<S: ChildCallSink>(
    children: &mut ChildCallBuilder<'_, S>,
    call: TilePartitionCall,
    sub_size: BlockSize,
    half_w: usize,
    half_h: usize,
) -> Result<(), TilePartitionTraversalError> {
    children.push_with_chroma_offset(call.r(), call.c(), sub_size, false, call.has_chroma())?;
    children.push_with_chroma_offset(
        call.r(),
        call.c() + half_w,
        sub_size,
        false,
        call.has_chroma(),
    )?;
    children.push_with_chroma_offset(
        call.r() + half_h,
        call.c(),
        sub_size,
        false,
        call.has_chroma(),
    )?;
    children.push_with_chroma_offset(
        call.r() + half_h,
        call.c() + half_w,
        sub_size,
        false,
        call.has_chroma(),
    )?;
    Ok(())
}

fn push_four_way_children<S: ChildCallSink>(
    children: &mut ChildCallBuilder<'_, S>,
    call: TilePartitionCall,
    sub_size: BlockSize,
    axis: ChildAxis,
    a_layout: bool,
    step: usize,
    has_chroma_before_offset: bool,
) -> Result<(), TilePartitionTraversalError> {
    let (_, partition) = axis.facts();
    let b_size_big = valid_subsize(partition, call.b_size)?;
    let b_size_med = valid_subsize(partition, b_size_big)?;
    let (second, third, third_scale) = if a_layout {
        (b_size_med, b_size_big, 3)
    } else {
        (b_size_big, b_size_med, 5)
    };
    children.push(call.r(), call.c(), sub_size, has_chroma_before_offset)?;

    let (r, c) = axis.offset(call, 1, step);
    children.push(r, c, second, has_chroma_before_offset)?;

    let (r, c) = axis.offset(call, third_scale, step);
    children.push(r, c, third, has_chroma_before_offset)?;

    let (r, c) = axis.offset(call, 7, step);
    children.push(r, c, sub_size, call.has_chroma())
}

pub(crate) fn child_calls(
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: &TilePartitionFrameFacts,
    chroma_offset: bool,
) -> Result<TilePartitionChildCalls, TilePartitionTraversalError> {
    let mut calls = TilePartitionChildCalls::new(call);
    child_calls_into(&mut calls, call, partition, sub_size, frame, chroma_offset)?;
    Ok(calls)
}

/// Pushes `call`'s § 5.20.3.1 child calls onto `sink` in syntax order.
#[inline]
pub(crate) fn child_calls_into<S: ChildCallSink>(
    sink: &mut S,
    call: TilePartitionCall,
    partition: PartitionType,
    sub_size: BlockSize,
    frame: &TilePartitionFrameFacts,
    chroma_offset: bool,
) -> Result<(), TilePartitionTraversalError> {
    let num4x4wide = call.b_size.num_4x4_wide()?;
    let num4x4high = call.b_size.num_4x4_high()?;
    let half_w = num4x4wide >> 1;
    let half_h = num4x4high >> 1;
    let has_chroma_before_offset = call.has_chroma() && !chroma_offset;
    let mut children = ChildCallBuilder::new(sink, call, partition, sub_size, frame, chroma_offset);
    match partition {
        PartitionType::None => {}
        PartitionType::Horz => {
            push_two_way_children(
                &mut children,
                call,
                sub_size,
                ChildAxis::Row,
                half_h,
                has_chroma_before_offset,
            )?;
        }
        PartitionType::Vert => {
            push_two_way_children(
                &mut children,
                call,
                sub_size,
                ChildAxis::Col,
                half_w,
                has_chroma_before_offset,
            )?;
        }
        PartitionType::Split => {
            push_split_children(&mut children, call, sub_size, half_w, half_h)?;
        }
        PartitionType::Horz3 => {
            let middle = middle_partition_size(partition, call.b_size)?;
            let middle_chroma =
                call.b_size.index() == BLOCK_8X32 && call.has_chroma() && frame.subsampling_x;
            let middle_chroma_ref = if middle_chroma {
                ChromaRefGeometry::new(
                    call.r() + (half_h >> 1),
                    call.c(),
                    valid_subsize(PartitionType::Horz, call.b_size)?,
                )
            } else {
                children.inherited_chroma_ref
            };
            children.push(call.r(), call.c(), sub_size, has_chroma_before_offset)?;
            children.push_with_chroma_ref(
                call.r() + (half_h >> 1),
                call.c(),
                middle,
                chroma_offset || middle_chroma,
                has_chroma_before_offset && !middle_chroma,
                middle_chroma_ref,
            )?;
            children.push_with_chroma_ref(
                call.r() + (half_h >> 1),
                call.c() + half_w,
                middle,
                chroma_offset || middle_chroma,
                has_chroma_before_offset,
                middle_chroma_ref,
            )?;
            children.push(
                call.r() + 3 * (half_h >> 1),
                call.c(),
                sub_size,
                call.has_chroma(),
            )?;
        }
        PartitionType::Vert3 => {
            let middle = middle_partition_size(partition, call.b_size)?;
            let middle_chroma =
                call.b_size.index() == BLOCK_32X8 && call.has_chroma() && frame.subsampling_y;
            // AV2 § 5.20.4.1: VERT_3 middleChroma uses the half-width VERT sub-block.
            let middle_chroma_ref = if middle_chroma {
                ChromaRefGeometry::new(
                    call.r(),
                    call.c() + (half_w >> 1),
                    valid_subsize(PartitionType::Vert, call.b_size)?,
                )
            } else {
                children.inherited_chroma_ref
            };
            children.push(call.r(), call.c(), sub_size, has_chroma_before_offset)?;
            children.push_with_chroma_ref(
                call.r(),
                call.c() + (half_w >> 1),
                middle,
                chroma_offset || middle_chroma,
                has_chroma_before_offset && !middle_chroma,
                middle_chroma_ref,
            )?;
            children.push_with_chroma_ref(
                call.r() + half_h,
                call.c() + (half_w >> 1),
                middle,
                chroma_offset || middle_chroma,
                has_chroma_before_offset,
                middle_chroma_ref,
            )?;
            children.push(
                call.r(),
                call.c() + 3 * (half_w >> 1),
                sub_size,
                call.has_chroma(),
            )?;
        }
        PartitionType::Horz4A | PartitionType::Horz4B => {
            push_four_way_children(
                &mut children,
                call,
                sub_size,
                ChildAxis::Row,
                partition == PartitionType::Horz4A,
                num4x4high >> 3,
                has_chroma_before_offset,
            )?;
        }
        PartitionType::Vert4A | PartitionType::Vert4B => {
            push_four_way_children(
                &mut children,
                call,
                sub_size,
                ChildAxis::Col,
                partition == PartitionType::Vert4A,
                num4x4wide >> 3,
                has_chroma_before_offset,
            )?;
        }
    }
    Ok(())
}
