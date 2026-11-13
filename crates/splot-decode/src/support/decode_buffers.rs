// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared frame-plane storage and filter-record capacity hints for one decode.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::Mutex;

use crate::filters::wienerns_lr::FrameFilterRecordCapacities;
use crate::prediction::inter::{
    ReconRowBuffers, TemporalMotionBlock, TemporalMvContext, TileParseState,
};

/// A frame walk's temporal context and motion records.
pub(crate) type WalkTemporal = (TemporalMvContext, Vec<TemporalMotionBlock>);

/// The storage one decode's finished work leaves for the work behind it.
#[derive(Default)]
pub(crate) struct DecodeBuffers {
    planes: Arc<splot_recon::PlanePool>,
    tile_records: Mutex<FrameFilterRecordCapacities>,
    /// Sixteen times the running mean of each large row list's length.
    unit_lengths: Mutex<UnitLengths>,
    /// Spent superblock units' row buffers, lent to the next unit parsed.
    units: Mutex<Vec<ReconRowBuffers>>,
    pub(crate) parse_states: Mutex<Vec<TileParseState>>,
    /// The first split frame, in decode order, whose reconstruction is open.
    oldest_open: AtomicUsize,
    /// Parsed superblock units that reconstruction has not committed.
    parsed_units: AtomicUsize,
    /// The superblock units of the last frame laid out.
    frame_units: AtomicUsize,
    /// The next split frame's number, and the numbers of frames after the
    /// oldest open one whose reconstruction has ended.
    frame_order: Mutex<(usize, Vec<usize>)>,
    /// Spent filter stripe windows of every frame: a frame's frontier fills
    /// only the few stripes its filters have not run yet.
    pub(crate) filter_windows: Mutex<Vec<splot_recon::RetiredFramePlanes>>,
    /// One walk's temporal state for the whole decode: the inter and TIP output
    /// walks take turns, so a copy per walk or lane is memory nobody reads.
    walk_temporal: Mutex<Option<WalkTemporal>>,
}

/// The lengths of a superblock unit's large row lists.
pub(crate) type UnitLengths = [usize; 11];

impl DecodeBuffers {
    /// Opens the storage for one decode.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The frame-sized plane storage, which reconstruction workspaces name so
    /// their buffers come home whichever holder releases them last.
    pub(crate) fn planes(&self) -> &Arc<splot_recon::PlanePool> {
        &self.planes
    }

    /// The record capacities a spent tile reached, for the next tile's set.
    pub(crate) fn tile_record_capacities(&self) -> FrameFilterRecordCapacities {
        *self.tile_records.lock()
    }

    /// Notes the record capacities one spent tile reached.
    pub(crate) fn note_tile_record_capacities(&self, reached: FrameFilterRecordCapacities) {
        self.tile_records.lock().cover(reached);
    }

    /// A spent unit's row buffers, if any is free.
    pub(crate) fn take_unit_buffers(&self) -> Option<ReconRowBuffers> {
        self.units.lock().pop()
    }

    /// Takes back a committed unit's row buffers.
    pub(crate) fn give_unit_buffers(&self, buffers: ReconRowBuffers) {
        self.units.lock().push(buffers);
    }

    /// Numbers the next split frame in decode order.
    pub(crate) fn open_frame(&self) -> usize {
        let mut order = self.frame_order.lock();
        order.0 += 1;
        order.0 - 1
    }

    /// Notes that frame `frame` has committed or failed its reconstruction.
    pub(crate) fn end_frame(&self, frame: usize) {
        let mut order = self.frame_order.lock();
        let mut oldest = self.oldest_open.load(Ordering::Relaxed);
        if frame != oldest {
            order.1.push(frame);
            return;
        }
        oldest += 1;
        while let Some(at) = order.1.iter().position(|&ended| ended == oldest) {
            order.1.swap_remove(at);
            oldest += 1;
        }
        self.oldest_open.store(oldest, Ordering::Release);
    }

    /// Whether split frame `frame` may start its parse: once the parsed units
    /// nobody has committed fill `window` frames, a further parse would only
    /// hold units that reconstruction cannot reach yet. The oldest open frame
    /// always may, so the frames a waiting parse depends on never wait.
    pub(crate) fn may_parse(&self, frame: usize, window: usize) -> bool {
        frame == self.oldest_open.load(Ordering::Acquire)
            || self.parsed_units.load(Ordering::Acquire)
                < window.saturating_mul(self.frame_units.load(Ordering::Relaxed))
    }

    /// Notes the unit count of the frame being laid out.
    pub(crate) fn note_frame_units(&self, units: usize) {
        self.frame_units.store(units, Ordering::Relaxed);
    }

    /// Adds `units` parsed units, or takes off committed ones.
    pub(crate) fn note_parsed_units(&self, units: usize, committed: bool) {
        if committed {
            let _ = self
                .parsed_units
                .try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                    Some(held.saturating_sub(units))
                });
        } else {
            self.parsed_units.fetch_add(units, Ordering::AcqRel);
        }
    }

    /// The running mean of each large row list's length.
    pub(crate) fn unit_lengths(&self) -> UnitLengths {
        self.unit_lengths.lock().map(|mean| mean / 16)
    }

    /// Folds one spent unit's list lengths into the decode's running mean,
    /// over about the last sixteen units, and returns that mean.
    pub(crate) fn note_unit_lengths(&self, used: UnitLengths) -> UnitLengths {
        let mut means = self.unit_lengths.lock();
        for (mean, used) in means.iter_mut().zip(used) {
            *mean = (*mean - *mean / 16).saturating_add(used);
        }
        means.map(|mean| mean / 16)
    }

    /// Lends the decode's walk temporal state to one walk, or a new one when
    /// another walk holds it or there are no decode buffers.
    pub(crate) fn lend_temporal(buffers: Option<&Self>) -> TemporalLease<'_> {
        let (temporal, records) = buffers
            .and_then(|buffers| buffers.walk_temporal.lock().take())
            .unwrap_or_else(|| (TemporalMvContext::empty(), Vec::new()));
        TemporalLease {
            buffers,
            temporal,
            records,
        }
    }
}

/// The temporal state lent to one walk, given back however the walk ends.
pub(crate) struct TemporalLease<'a> {
    buffers: Option<&'a DecodeBuffers>,
    pub(crate) temporal: TemporalMvContext,
    pub(crate) records: Vec<TemporalMotionBlock>,
}

impl Drop for TemporalLease<'_> {
    fn drop(&mut self) {
        if let Some(buffers) = self.buffers {
            let temporal = core::mem::replace(&mut self.temporal, TemporalMvContext::empty());
            *buffers.walk_temporal.lock() = Some((temporal, core::mem::take(&mut self.records)));
        }
    }
}

/// Sizes an emptied unit list for the next unit: to twice the decode's
/// typical length for it when its capacity is under the typical length, or
/// over three times it and 8 KiB. Pooled buffers otherwise each climb to the
/// largest unit any of them has held. The list is empty, so a fresh
/// allocation replaces it without a copy.
pub(crate) fn fit_list<E>(list: &mut Vec<E>, typical: usize) {
    debug_assert!(list.is_empty());
    let keep = typical.saturating_mul(2);
    let floor = (8 << 10) / core::mem::size_of::<E>().max(1);
    if list.capacity() < typical || list.capacity() > keep.saturating_add(keep / 2).max(floor) {
        *list = Vec::with_capacity(keep);
    }
}

#[cfg(test)]
mod tests {
    use super::DecodeBuffers;

    #[test]
    fn the_oldest_open_frame_always_parses_and_ended_frames_move_it_on() {
        let buffers = DecodeBuffers::default();
        let [first, second, third] = [(); 3].map(|()| buffers.open_frame());
        buffers.note_frame_units(2);
        buffers.note_parsed_units(5, false);
        assert!(buffers.may_parse(first, 2));
        assert!(!buffers.may_parse(second, 2));
        buffers.end_frame(second);
        assert!(!buffers.may_parse(third, 2));
        buffers.end_frame(first);
        assert!(buffers.may_parse(third, 2));
        buffers.note_parsed_units(2, true);
        assert!(buffers.may_parse(third + 1, 2));
        buffers.note_parsed_units(9, true);
        assert!(buffers.may_parse(third + 1, 1));
    }
}
