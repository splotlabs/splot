// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Logical frame identities over reusable streaming metadata slots.
//!
//! References and pending displays each hold at most `MAX_REF_FRAMES`. A full
//! table stops admission until outstanding work and queued emission drain.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.

use super::{PipelineFrame, unsupported};
use crate::Result;
use splot_core::headers::sequence::MAX_REF_FRAMES;

pub(crate) struct FrameEntry {
    pub(super) index: usize,
    pub(super) frame: Option<PipelineFrame>,
}

pub(crate) struct FrameStore {
    pub(super) entries: Vec<FrameEntry>,
    count: usize,
    retain: bool,
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
                })
                .collect(),
            count: 0,
            retain,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }

    pub(super) fn has_space(&self) -> bool {
        self.retain || self.entries.iter().any(|entry| entry.frame.is_none())
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
        let entry = FrameEntry {
            index: self.count,
            frame: Some(frame),
        };
        if self.retain {
            self.entries.push(entry);
        } else {
            let free = self
                .entries
                .iter_mut()
                .find(|entry| entry.frame.is_none())
                .ok_or_else(|| {
                    unsupported(
                        "frame_slot_unavailable",
                        None,
                        "decode pipeline admitted a frame without a free metadata slot",
                    )
                })?;
            *free = entry;
        }
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
                .map(|(index, frame)| FrameEntry { index, frame })
                .collect(),
            retain: true,
        }
    }
}
