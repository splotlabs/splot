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
/// flag parsing, output scheduling, or other AV2 reference semantics.
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

/// Caller-derived reference refresh slot mask.
///
/// AV2 § 5.18.2 and § 6.17.2 derive `refresh_frame_flags`, and § 7.23 uses
/// each set bit to select a reference-frame storage slot. This type only
/// validates the AV2 § 3 `NUM_REF_FRAMES` slot ceiling and exposes selected
/// [`ReferenceSlot`] values; it does not parse, infer, or semantically validate
/// `refresh_frame_flags`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ReferenceRefreshMask(u32);

impl ReferenceRefreshMask {
    /// Maximum number of refresh bits accepted by this source-backed model.
    pub const MAX_BITS: usize = ReferenceSlot::MAX_SLOTS;

    const VALID_BITS: u32 = (1u32 << Self::MAX_BITS) - 1;

    /// Creates a refresh mask after validating it against [`Self::MAX_BITS`].
    ///
    /// # Errors
    /// Returns [`ReconError::InvalidReferenceRefreshMask`] when `bits` contains
    /// any bit at or above [`Self::MAX_BITS`].
    pub const fn new(bits: u32) -> Result<Self> {
        if (bits & !Self::VALID_BITS) == 0 {
            Ok(Self(bits))
        } else {
            Err(ReconError::InvalidReferenceRefreshMask {
                mask: bits,
                max_slots: Self::MAX_BITS,
            })
        }
    }

    /// Returns the validated raw mask bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns whether this mask selects no reference slots.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns whether `slot` is selected by this mask.
    pub const fn contains(self, slot: ReferenceSlot) -> bool {
        (self.0 & (1u32 << slot.index())) != 0
    }

    /// Iterates over selected slots in ascending slot order.
    pub const fn slots(self) -> ReferenceRefreshSlots {
        ReferenceRefreshSlots {
            mask: self.0,
            next_slot: 0,
        }
    }

    const fn valid_bits_for_capacity(capacity: usize) -> u32 {
        if capacity >= ReferenceSlot::MAX_SLOTS {
            Self::VALID_BITS
        } else {
            (1u32 << capacity) - 1
        }
    }
}

/// Iterator over refresh-mask-selected slots in ascending slot order.
#[derive(Clone, Debug)]
pub struct ReferenceRefreshSlots {
    mask: u32,
    next_slot: usize,
}

impl Iterator for ReferenceRefreshSlots {
    type Item = ReferenceSlot;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next_slot < ReferenceSlot::MAX_SLOTS {
            let index = self.next_slot;
            self.next_slot += 1;

            if (self.mask & (1u32 << index)) != 0 {
                return Some(ReferenceSlot(index));
            }
        }

        None
    }
}

/// Fixed-capacity store of immutable frame payloads in reference slots.
///
/// This is a dependency-free runtime storage model for future callers that have
/// already derived AV2 § 7.23 reference update decisions. It stores caller-owned
/// payloads by slot and can apply already-derived refresh masks, but it does
/// not implement byte-consuming decode, frame reconstruction,
/// `refresh_frame_flags` parsing or inference, `RefValid`, film grain, output
/// scheduling, motion vectors, CDFs, segment IDs, global motion state, or
/// reference metadata.
///
/// The store moves or shares payload handles and never duplicates them: it does
/// not implement `Clone` and never requires `F: Clone` (see
/// [`docs/ZERO_COPY.md`](../../../docs/ZERO_COPY.md)).
#[derive(Debug, Eq, PartialEq)]
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

    /// Applies a caller-derived refresh mask to this store.
    ///
    /// The helper validates all selected slots against this store's fixed
    /// capacity before invoking `produce`. For each selected slot, in ascending
    /// slot order, `produce` supplies the new payload handle to store. Previous
    /// payloads are returned in the same order. A zero mask is a no-op.
    ///
    /// # Errors
    /// Returns [`ReconError::ReferenceRefreshMaskOutOfBounds`] when `mask`
    /// selects any slot outside this store's fixed capacity. In that case
    /// `produce` is not called and the store is not mutated.
    pub fn refresh_slots_with(
        &mut self,
        mask: ReferenceRefreshMask,
        mut produce: impl FnMut(ReferenceSlot) -> F,
    ) -> Result<ReferenceRefreshOutcome<F>> {
        self.ensure_refresh_mask(mask)?;

        let mut replacements = Vec::new();
        for slot in mask.slots() {
            let previous = self.slots[slot.index()].replace(produce(slot));
            if previous.is_none() {
                self.occupied += 1;
            }
            replacements.push(ReferenceFrameReplacement { slot, previous });
        }

        Ok(ReferenceRefreshOutcome { replacements })
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

    fn ensure_refresh_mask(&self, mask: ReferenceRefreshMask) -> Result<()> {
        let valid_bits = ReferenceRefreshMask::valid_bits_for_capacity(self.capacity());
        if (mask.bits() & !valid_bits) == 0 {
            Ok(())
        } else {
            Err(ReconError::ReferenceRefreshMaskOutOfBounds {
                mask: mask.bits(),
                capacity: self.capacity(),
            })
        }
    }
}

/// Replaced payload information for one refreshed reference slot.
#[derive(Debug, Eq, PartialEq)]
pub struct ReferenceFrameReplacement<F> {
    slot: ReferenceSlot,
    previous: Option<F>,
}

impl<F> ReferenceFrameReplacement<F> {
    /// Returns the refreshed reference slot.
    pub const fn slot(&self) -> ReferenceSlot {
        self.slot
    }

    /// Returns the previous payload in the refreshed slot, if any.
    pub fn previous(&self) -> Option<&F> {
        self.previous.as_ref()
    }

    /// Consumes this replacement and returns the previous payload, if any.
    pub fn into_previous(self) -> Option<F> {
        self.previous
    }
}

/// Result of applying a refresh mask to a reference-frame store.
#[derive(Debug, Eq, PartialEq)]
pub struct ReferenceRefreshOutcome<F> {
    replacements: Vec<ReferenceFrameReplacement<F>>,
}

impl<F> ReferenceRefreshOutcome<F> {
    /// Returns the number of selected slots that were refreshed.
    pub fn len(&self) -> usize {
        self.replacements.len()
    }

    /// Returns whether the refresh mask selected no slots.
    pub fn is_empty(&self) -> bool {
        self.replacements.is_empty()
    }

    /// Iterates over slot replacement results in ascending slot order.
    pub fn iter(&self) -> core::slice::Iter<'_, ReferenceFrameReplacement<F>> {
        self.replacements.iter()
    }

    /// Consumes this outcome and returns the replacement records.
    pub fn into_replacements(self) -> Vec<ReferenceFrameReplacement<F>> {
        self.replacements
    }
}

impl<'a, F> IntoIterator for &'a ReferenceRefreshOutcome<F> {
    type Item = &'a ReferenceFrameReplacement<F>;
    type IntoIter = core::slice::Iter<'a, ReferenceFrameReplacement<F>>;

    fn into_iter(self) -> Self::IntoIter {
        self.replacements.iter()
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
    fn reference_refresh_mask_validates_bits_and_iterates_selected_slots() {
        let mask = ReferenceRefreshMask::new((1 << 0) | (1 << 3) | (1 << 15)).unwrap();

        assert_eq!(ReferenceRefreshMask::MAX_BITS, 16);
        assert_eq!(ReferenceRefreshMask::new(0).unwrap().bits(), 0);
        assert!(ReferenceRefreshMask::new(0).unwrap().is_empty());
        assert!(mask.contains(ReferenceSlot::new(3).unwrap()));
        assert!(!mask.contains(ReferenceSlot::new(4).unwrap()));
        assert_eq!(
            mask.slots().map(ReferenceSlot::index).collect::<Vec<_>>(),
            vec![0, 3, 15]
        );
        assert!(matches!(
            ReferenceRefreshMask::new(1 << 16),
            Err(ReconError::InvalidReferenceRefreshMask {
                mask,
                max_slots: 16
            }) if mask == 1 << 16
        ));
        assert!(matches!(
            ReferenceRefreshMask::new(u32::MAX),
            Err(ReconError::InvalidReferenceRefreshMask {
                mask: u32::MAX,
                max_slots: 16
            })
        ));
    }

    #[test]
    fn refresh_slots_with_zero_mask_is_noop() {
        let mut store = ReferenceFrameStore::with_capacity(2).unwrap();
        let slot0 = ReferenceSlot::new(0).unwrap();
        store.put(slot0, frame(7, 7)).unwrap();
        let mut calls = 0;

        let outcome = store
            .refresh_slots_with(ReferenceRefreshMask::new(0).unwrap(), |_| {
                calls += 1;
                frame(99, 99)
            })
            .unwrap();

        assert!(outcome.is_empty());
        assert_eq!(outcome.len(), 0);
        assert_eq!(calls, 0);
        assert_eq!(store.occupied(), 1);
        assert_eq!(store.get(slot0).unwrap().unwrap().output_index().get(), 7);
    }

    #[test]
    fn refresh_slots_with_fills_empty_slot() {
        let mut store = ReferenceFrameStore::with_capacity(3).unwrap();
        let slot1 = ReferenceSlot::new(1).unwrap();

        let outcome = store
            .refresh_slots_with(ReferenceRefreshMask::new(1 << 1).unwrap(), |slot| {
                frame(slot.index() as u64, 11)
            })
            .unwrap();

        assert_eq!(outcome.len(), 1);
        let replacement = outcome.iter().next().unwrap();
        assert_eq!(replacement.slot(), slot1);
        assert!(replacement.previous().is_none());
        assert_eq!(store.occupied(), 1);
        assert_eq!(store.get(slot1).unwrap().unwrap().output_index().get(), 1);
    }

    #[test]
    fn refresh_slots_with_visits_slots_in_order_and_returns_replacements() {
        let mut store = ReferenceFrameStore::with_capacity(4).unwrap();
        store.put(ReferenceSlot::new(1).unwrap(), 10).unwrap();
        store.put(ReferenceSlot::new(3).unwrap(), 30).unwrap();
        let mut calls = Vec::new();

        let outcome = store
            .refresh_slots_with(ReferenceRefreshMask::new(0b1011).unwrap(), |slot| {
                calls.push(slot.index());
                100 + slot.index() as u8
            })
            .unwrap();

        assert_eq!(calls, vec![0, 1, 3]);
        let replacements: Vec<(usize, Option<u8>)> = outcome
            .into_replacements()
            .into_iter()
            .map(|replacement| {
                let slot = replacement.slot().index();
                (slot, replacement.into_previous())
            })
            .collect();
        assert_eq!(replacements, vec![(0, None), (1, Some(10)), (3, Some(30))]);
        assert_eq!(store.occupied(), 3);
        assert_eq!(
            store.get(ReferenceSlot::new(0).unwrap()).unwrap(),
            Some(&100)
        );
        assert_eq!(
            store.get(ReferenceSlot::new(1).unwrap()).unwrap(),
            Some(&101)
        );
        assert_eq!(store.get(ReferenceSlot::new(2).unwrap()).unwrap(), None);
        assert_eq!(
            store.get(ReferenceSlot::new(3).unwrap()).unwrap(),
            Some(&103)
        );
    }

    #[test]
    fn refresh_slots_with_rejects_out_of_capacity_mask_before_mutation() {
        let mut store = ReferenceFrameStore::with_capacity(2).unwrap();
        let slot0 = ReferenceSlot::new(0).unwrap();
        store.put(slot0, 7).unwrap();
        let mask = ReferenceRefreshMask::new(1 << 2).unwrap();
        let mut calls = 0;

        assert!(matches!(
            store.refresh_slots_with(mask, |_| {
                calls += 1;
                99
            }),
            Err(ReconError::ReferenceRefreshMaskOutOfBounds {
                mask: observed,
                capacity: 2
            }) if observed == mask.bits()
        ));
        assert_eq!(calls, 0);
        assert_eq!(store.occupied(), 1);
        assert_eq!(store.get(slot0).unwrap(), Some(&7));
    }

    #[test]
    fn refresh_slots_with_accepts_max_capacity_edge_slot() {
        let mut store = ReferenceFrameStore::with_capacity(ReferenceSlot::MAX_SLOTS).unwrap();
        let edge_slot = ReferenceSlot::new(15).unwrap();

        store
            .refresh_slots_with(ReferenceRefreshMask::new(1 << 15).unwrap(), |slot| {
                frame(slot.index() as u64, 15)
            })
            .unwrap();

        assert_eq!(store.occupied(), 1);
        assert_eq!(
            store.get(edge_slot).unwrap().unwrap().output_index().get(),
            15
        );
    }

    #[test]
    fn refresh_slots_with_accepts_non_clone_payloads() {
        #[derive(Debug, Eq, PartialEq)]
        struct NonClonePayload {
            id: u8,
        }

        let mut store = ReferenceFrameStore::with_capacity(3).unwrap();

        let outcome = store
            .refresh_slots_with(ReferenceRefreshMask::new(0b101).unwrap(), |slot| {
                NonClonePayload {
                    id: slot.index() as u8,
                }
            })
            .unwrap();

        assert_eq!(outcome.len(), 2);
        assert_eq!(
            store.get(ReferenceSlot::new(0).unwrap()).unwrap(),
            Some(&NonClonePayload { id: 0 })
        );
        assert_eq!(
            store.get(ReferenceSlot::new(2).unwrap()).unwrap(),
            Some(&NonClonePayload { id: 2 })
        );
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

    #[test]
    fn reference_store_holds_shared_frames_without_clone() {
        use crate::SharedFrame;

        let mut store = ReferenceFrameStore::with_capacity(2).unwrap();
        let shared = SharedFrame::new(frame(4, 41));
        let slot0 = ReferenceSlot::new(0).unwrap();
        let slot1 = ReferenceSlot::new(1).unwrap();

        assert!(store.put(slot0, shared.share()).unwrap().is_none());
        assert!(store.put(slot1, shared.share()).unwrap().is_none());
        assert_eq!(shared.handle_count(), 3); // original + two stored handles

        let ptr0 = store
            .get(slot0)
            .unwrap()
            .unwrap()
            .get()
            .y()
            .samples()
            .as_ptr();
        let ptr1 = store
            .get(slot1)
            .unwrap()
            .unwrap()
            .get()
            .y()
            .samples()
            .as_ptr();
        assert_eq!(ptr0, ptr1);
    }

    #[test]
    fn refresh_slots_with_explicitly_shares_frame_handles() {
        use crate::SharedFrame;

        let mut store = ReferenceFrameStore::with_capacity(2).unwrap();
        let shared = SharedFrame::new(frame(5, 50));
        let slot0 = ReferenceSlot::new(0).unwrap();
        let slot1 = ReferenceSlot::new(1).unwrap();

        store
            .refresh_slots_with(ReferenceRefreshMask::new(0b11).unwrap(), |_| shared.share())
            .unwrap();

        assert_eq!(shared.handle_count(), 3);
        let ptr0 = store
            .get(slot0)
            .unwrap()
            .unwrap()
            .get()
            .y()
            .samples()
            .as_ptr();
        let ptr1 = store
            .get(slot1)
            .unwrap()
            .unwrap()
            .get()
            .y()
            .samples()
            .as_ptr();
        assert_eq!(ptr0, ptr1);
    }
}
