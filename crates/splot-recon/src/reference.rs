// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Immutable reference-frame store model for future decode and encoder reuse.

use core::{iter::Enumerate, slice::Iter};

use crate::{ReconError, Result};

/// Zero-based reference-frame slot index.
///
/// `MAX_SLOTS` is the AV2 § 3 `NUM_REF_FRAMES` slot ceiling that motivates
/// reference-frame storage in § 7.23. This type is only a safe runtime slot
/// identifier; it does not model active `NumRefFrames`, `RefValid`, refresh
/// masks, output scheduling, or other AV2 reference semantics.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ReferenceSlot(usize);

impl ReferenceSlot {
    /// Maximum reference slot count supported by this source-backed model.
    pub const MAX_SLOTS: usize = 16;

    /// Creates a reference slot after validating it against [`Self::MAX_SLOTS`].
    ///
    /// # Errors
    /// Returns [`ReconError::InvalidReferenceSlotIndex`] when `index` is not in
    /// `0..Self::MAX_SLOTS`.
    pub const fn new(index: usize) -> Result<Self> {
        if index < Self::MAX_SLOTS {
            Ok(Self(index))
        } else {
            Err(ReconError::InvalidReferenceSlotIndex {
                index,
                max_slots: Self::MAX_SLOTS,
            })
        }
    }

    /// Returns the zero-based slot index.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Fixed-capacity store of immutable frame payloads in reference slots.
///
/// This is a dependency-free runtime storage model for future callers that have
/// already derived AV2 § 7.23 reference update decisions. It stores caller-owned
/// payloads by slot, but it does not implement byte-consuming decode, frame
/// reconstruction, `refresh_frame_flags`, `RefValid`, film grain, output
/// scheduling, motion vectors, CDFs, segment IDs, global motion state, or
/// reference metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceFrameStore<F> {
    slots: Vec<Option<F>>,
    occupied: usize,
}

impl<F> ReferenceFrameStore<F> {
    /// Creates an empty reference-frame store with `capacity` slots.
    ///
    /// # Errors
    /// Returns [`ReconError::InvalidReferenceStoreCapacity`] when `capacity` is
    /// zero or exceeds [`ReferenceSlot::MAX_SLOTS`].
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        if capacity == 0 || capacity > ReferenceSlot::MAX_SLOTS {
            return Err(ReconError::InvalidReferenceStoreCapacity {
                capacity,
                max_slots: ReferenceSlot::MAX_SLOTS,
            });
        }

        let mut slots = Vec::with_capacity(capacity);
        slots.resize_with(capacity, || None);
        Ok(Self { slots, occupied: 0 })
    }

    /// Returns the fixed slot capacity.
    pub const fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Returns the number of occupied slots.
    pub const fn occupied(&self) -> usize {
        self.occupied
    }

    /// Returns whether no slots are occupied.
    pub const fn is_empty(&self) -> bool {
        self.occupied == 0
    }

    /// Returns whether `slot` is inside this store's fixed capacity.
    pub fn contains_slot(&self, slot: ReferenceSlot) -> bool {
        slot.index() < self.capacity()
    }

    /// Returns the immutable frame stored in `slot`, if occupied.
    ///
    /// # Errors
    /// Returns [`ReconError::ReferenceSlotOutOfBounds`] when `slot` is outside
    /// this store's fixed capacity.
    pub fn get(&self, slot: ReferenceSlot) -> Result<Option<&F>> {
        self.ensure_slot(slot)?;
        Ok(self.slots[slot.index()].as_ref())
    }

    /// Stores `frame` in `slot`, returning the previous frame if the slot was
    /// occupied.
    ///
    /// # Errors
    /// Returns [`ReconError::ReferenceSlotOutOfBounds`] when `slot` is outside
    /// this store's fixed capacity.
    pub fn put(&mut self, slot: ReferenceSlot, frame: F) -> Result<Option<F>> {
        self.ensure_slot(slot)?;
        let previous = self.slots[slot.index()].replace(frame);
        if previous.is_none() {
            self.occupied += 1;
        }
        Ok(previous)
    }

    /// Clears `slot`, returning the stored frame if the slot was occupied.
    ///
    /// # Errors
    /// Returns [`ReconError::ReferenceSlotOutOfBounds`] when `slot` is outside
    /// this store's fixed capacity.
    pub fn take(&mut self, slot: ReferenceSlot) -> Result<Option<F>> {
        self.ensure_slot(slot)?;
        let previous = self.slots[slot.index()].take();
        if previous.is_some() {
            self.occupied -= 1;
        }
        Ok(previous)
    }

    /// Clears every occupied slot without changing capacity.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.occupied = 0;
    }

    /// Iterates over occupied entries in ascending slot order.
    pub fn entries(&self) -> ReferenceFrameEntries<'_, F> {
        ReferenceFrameEntries {
            inner: self.slots.iter().enumerate(),
        }
    }

    fn ensure_slot(&self, slot: ReferenceSlot) -> Result<()> {
        if self.contains_slot(slot) {
            Ok(())
        } else {
            Err(ReconError::ReferenceSlotOutOfBounds {
                slot,
                capacity: self.capacity(),
            })
        }
    }
}

/// Immutable occupied reference-frame store entry.
#[derive(Clone, Copy, Debug)]
pub struct ReferenceFrameEntry<'a, F> {
    slot: ReferenceSlot,
    frame: &'a F,
}

impl<'a, F> ReferenceFrameEntry<'a, F> {
    /// Returns the occupied reference slot.
    pub const fn slot(&self) -> ReferenceSlot {
        self.slot
    }

    /// Returns the immutable frame payload in the slot.
    pub const fn frame(&self) -> &'a F {
        self.frame
    }
}

/// Iterator over occupied reference-frame entries in ascending slot order.
#[derive(Clone, Debug)]
pub struct ReferenceFrameEntries<'a, F> {
    inner: Enumerate<Iter<'a, Option<F>>>,
}

impl<'a, F> Iterator for ReferenceFrameEntries<'a, F> {
    type Item = ReferenceFrameEntry<'a, F>;

    fn next(&mut self) -> Option<Self::Item> {
        for (index, frame) in self.inner.by_ref() {
            if let Some(frame) = frame.as_ref() {
                return Some(ReferenceFrameEntry {
                    slot: ReferenceSlot(index),
                    frame,
                });
            }
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{
        BitDepth, DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane,
        PlaneRect, PlaneSize,
    };

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
    }

    fn frame(output_index: u64, sample: u8) -> DecodedFrame<u8> {
        let luma_size = size(2, 2);
        let luma_rect = rect(0, 0, 2, 2);
        let chroma_size = size(1, 1);
        let chroma_rect = rect(0, 0, 1, 1);
        let info = DecodedFrameInfo::new(
            OutputIndex::new(output_index),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            luma_size,
            luma_rect,
        )
        .unwrap();
        let y = Plane::from_vec(luma_size, 2, luma_rect, vec![sample; 4]).unwrap();
        let u = Plane::from_vec(chroma_size, 1, chroma_rect, vec![sample.wrapping_add(1)]).unwrap();
        let v = Plane::from_vec(chroma_size, 1, chroma_rect, vec![sample.wrapping_add(2)]).unwrap();

        DecodedFrame::try_new(info, FramePlanes::new(y, Some(u), Some(v))).unwrap()
    }

    #[test]
    fn reference_slot_validates_index() {
        assert_eq!(ReferenceSlot::new(0).unwrap().index(), 0);
        assert_eq!(ReferenceSlot::new(15).unwrap().index(), 15);
        assert!(matches!(
            ReferenceSlot::new(16),
            Err(ReconError::InvalidReferenceSlotIndex {
                index: 16,
                max_slots: 16
            })
        ));
    }

    #[test]
    fn reference_store_rejects_invalid_capacity() {
        assert!(matches!(
            ReferenceFrameStore::<u8>::with_capacity(0),
            Err(ReconError::InvalidReferenceStoreCapacity {
                capacity: 0,
                max_slots: 16
            })
        ));
        assert!(matches!(
            ReferenceFrameStore::<u8>::with_capacity(17),
            Err(ReconError::InvalidReferenceStoreCapacity {
                capacity: 17,
                max_slots: 16
            })
        ));
    }

    #[test]
    fn reference_store_starts_empty_with_fixed_capacity() {
        let store = ReferenceFrameStore::<u8>::with_capacity(3).unwrap();
        assert_eq!(store.capacity(), 3);
        assert_eq!(store.occupied(), 0);
        assert!(store.is_empty());
        assert!(store.contains_slot(ReferenceSlot::new(2).unwrap()));
        assert!(!store.contains_slot(ReferenceSlot::new(3).unwrap()));
    }

    #[test]
    fn reference_store_accepts_max_capacity_edge_slot() {
        let mut store = ReferenceFrameStore::with_capacity(ReferenceSlot::MAX_SLOTS).unwrap();
        let edge_slot = ReferenceSlot::new(15).unwrap();

        assert_eq!(store.capacity(), ReferenceSlot::MAX_SLOTS);
        assert!(store.contains_slot(edge_slot));
        assert!(store.put(edge_slot, frame(15, 15)).unwrap().is_none());
        assert_eq!(store.occupied(), 1);
        assert_eq!(
            store.get(edge_slot).unwrap().unwrap().output_index().get(),
            15
        );

        let removed = store.take(edge_slot).unwrap().unwrap();
        assert_eq!(removed.output_index().get(), 15);
        assert!(store.is_empty());
    }

    #[test]
    fn reference_store_accepts_payload_without_output_metadata() {
        let mut store = ReferenceFrameStore::<u8>::with_capacity(1).unwrap();
        let slot = ReferenceSlot::new(0).unwrap();

        assert!(store.put(slot, 42).unwrap().is_none());
        assert_eq!(store.get(slot).unwrap().copied(), Some(42));
        assert_eq!(store.entries().next().unwrap().frame(), &42);
    }

    #[test]
    fn reference_store_rejects_out_of_bounds_access_without_mutation() {
        let mut store = ReferenceFrameStore::with_capacity(2).unwrap();
        let valid_slot = ReferenceSlot::new(0).unwrap();
        let slot = ReferenceSlot::new(2).unwrap();

        store.put(valid_slot, frame(9, 9)).unwrap();
        let expected_entries = vec![(0, 9)];

        assert!(matches!(
            store.get(slot),
            Err(ReconError::ReferenceSlotOutOfBounds {
                slot: observed,
                capacity: 2
            }) if observed == slot
        ));
        assert!(matches!(
            store.put(slot, frame(0, 3)),
            Err(ReconError::ReferenceSlotOutOfBounds {
                slot: observed,
                capacity: 2
            }) if observed == slot
        ));
        assert!(matches!(
            store.take(slot),
            Err(ReconError::ReferenceSlotOutOfBounds {
                slot: observed,
                capacity: 2
            }) if observed == slot
        ));
        assert_eq!(store.occupied(), 1);
        assert_eq!(
            store.get(valid_slot).unwrap().unwrap().output_index().get(),
            9
        );
        assert_eq!(
            store
                .entries()
                .map(|entry| (entry.slot().index(), entry.frame().output_index().get()))
                .collect::<Vec<_>>(),
            expected_entries
        );
    }

    #[test]
    fn put_replaces_frame_and_tracks_occupancy() {
        let mut store = ReferenceFrameStore::with_capacity(2).unwrap();
        let slot = ReferenceSlot::new(1).unwrap();

        assert!(store.put(slot, frame(0, 7)).unwrap().is_none());
        assert_eq!(store.occupied(), 1);
        assert_eq!(store.get(slot).unwrap().unwrap().output_index().get(), 0);

        let previous = store.put(slot, frame(1, 11)).unwrap().unwrap();
        assert_eq!(previous.output_index().get(), 0);
        assert_eq!(store.occupied(), 1);
        assert_eq!(store.get(slot).unwrap().unwrap().output_index().get(), 1);
    }

    #[test]
    fn take_clears_slot_and_updates_occupancy() {
        let mut store = ReferenceFrameStore::with_capacity(2).unwrap();
        let slot = ReferenceSlot::new(0).unwrap();

        assert!(store.take(slot).unwrap().is_none());
        store.put(slot, frame(3, 19)).unwrap();
        let removed = store.take(slot).unwrap().unwrap();
        assert_eq!(removed.output_index().get(), 3);
        assert_eq!(store.occupied(), 0);
        assert!(store.is_empty());
        assert!(store.get(slot).unwrap().is_none());
    }

    #[test]
    fn clear_removes_all_frames_without_changing_capacity() {
        let mut store = ReferenceFrameStore::with_capacity(3).unwrap();
        store
            .put(ReferenceSlot::new(0).unwrap(), frame(0, 1))
            .unwrap();
        store
            .put(ReferenceSlot::new(2).unwrap(), frame(2, 2))
            .unwrap();

        store.clear();

        assert_eq!(store.capacity(), 3);
        assert_eq!(store.occupied(), 0);
        assert_eq!(store.entries().count(), 0);
    }

    #[test]
    fn entries_yield_occupied_frames_in_slot_order() {
        let mut store = ReferenceFrameStore::with_capacity(4).unwrap();
        store
            .put(ReferenceSlot::new(3).unwrap(), frame(30, 3))
            .unwrap();
        store
            .put(ReferenceSlot::new(1).unwrap(), frame(10, 1))
            .unwrap();

        let entries: Vec<(usize, u64)> = store
            .entries()
            .map(|entry| (entry.slot().index(), entry.frame().output_index().get()))
            .collect();

        assert_eq!(entries, vec![(1, 10), (3, 30)]);
    }
}
