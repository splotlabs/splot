// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Logical frame identities over reusable streaming metadata slots.
//!
//! References and pending displays each hold at most `MAX_REF_FRAMES`. A full
//! table stops admission until outstanding work and queued emission drain.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.

use super::PipelineFrame;
use crate::Result;
use crate::prediction::inter::{
    FrameProductWriters, FrameProducts, MotionFieldHandle, MotionFieldLayout,
};
use splot_core::headers::sequence::MAX_REF_FRAMES;

pub(crate) struct FrameEntry {
    pub(super) index: usize,
    pub(super) frame: Option<PipelineFrame>,
    motion: Option<MotionFieldHandle>,
    products: Option<FrameProducts>,
    pub(super) retired: Option<super::inflight::PipelineFrameSlot>,
}

pub(crate) struct FrameStore {
    pub(super) entries: Vec<FrameEntry>,
    count: usize,
    retain: bool,
    reserved: Option<usize>,
}

impl FrameStore {
    pub(super) fn new(retain: bool, depth: usize) -> Self {
        let slots = if retain {
            0
        } else {
            2 * MAX_REF_FRAMES + depth + 1
        };
        Self {
            entries: (0..slots)
                .map(|_| FrameEntry {
                    index: 0,
                    frame: None,
                    motion: None,
                    products: None,
                    retired: None,
                })
                .collect(),
            count: 0,
            retain,
            reserved: None,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }

    pub(super) fn has_space(&mut self) -> bool {
        self.reserved.is_some() || self.retain || self.entries.iter_mut().any(FrameEntry::available)
    }

    pub(super) fn reserve(&mut self) -> Result<usize> {
        if let Some(index) = self.reserved {
            return Ok(index);
        }
        let index = if self.retain {
            self.entries.push(FrameEntry {
                index: self.count,
                frame: None,
                motion: None,
                products: None,
                retired: None,
            });
            self.entries.len() - 1
        } else {
            self.entries
                .iter_mut()
                .position(FrameEntry::available)
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?
        };
        self.reserved = Some(index);
        Ok(index)
    }

    pub(super) fn take_retired(&mut self) -> Result<Option<super::inflight::PipelineFrameSlot>> {
        let index = self.reserve()?;
        Ok(self.entries[index].retired.take())
    }

    pub(super) fn reserve_motion(
        &mut self,
        layout: MotionFieldLayout,
    ) -> Result<MotionFieldHandle> {
        let index = self.reserve()?;
        let motion = self.entries[index]
            .motion
            .get_or_insert_with(|| MotionFieldHandle::pending_with_layout(layout));
        motion.reset_layout(layout)?;
        Ok(motion.clone())
    }

    pub(super) fn reserve_products(&mut self) -> Result<FrameProductWriters> {
        let index = self.reserve()?;
        self.entries[index]
            .products
            .get_or_insert_default()
            .claim()
            .ok_or_else(|| crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
    }

    pub(crate) fn get(&self, index: usize) -> Option<&PipelineFrame> {
        if self.retain {
            return self
                .entries
                .get(index)
                .and_then(|entry| entry.frame.as_ref());
        }
        self.entries
            .iter()
            .find(|entry| entry.index == index && entry.frame.is_some())
            .and_then(|entry| entry.frame.as_ref())
    }

    pub(super) fn take(&mut self, index: usize) -> Option<PipelineFrame> {
        if self.retain {
            return self
                .entries
                .get_mut(index)
                .and_then(|entry| entry.frame.take());
        }
        self.entries
            .iter_mut()
            .find(|entry| entry.index == index && entry.frame.is_some())
            .and_then(|entry| entry.frame.take())
    }

    pub(super) fn push(&mut self, frame: PipelineFrame) -> Result<()> {
        let index = self.reserve()?;
        let entry = &mut self.entries[index];
        entry.index = self.count;
        entry.frame = Some(frame);
        self.reserved = None;
        self.count += 1;
        Ok(())
    }
}

#[cfg(test)]
impl From<Vec<Option<PipelineFrame>>> for FrameStore {
    fn from(frames: Vec<Option<PipelineFrame>>) -> Self {
        Self {
            count: frames.len(),
            entries: frames
                .into_iter()
                .enumerate()
                .map(|(index, frame)| FrameEntry {
                    index,
                    frame,
                    motion: None,
                    products: None,
                    retired: None,
                })
                .collect(),
            retain: true,
            reserved: None,
        }
    }
}

impl FrameEntry {
    fn available(&mut self) -> bool {
        self.frame.is_none()
            && self
                .retired
                .as_ref()
                .is_none_or(super::inflight::PipelineFrameSlot::can_reuse)
            && self
                .motion
                .as_mut()
                .is_none_or(MotionFieldHandle::try_retire)
            && self.products.as_ref().is_none_or(FrameProducts::can_reuse)
    }
}
