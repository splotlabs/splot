// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target for CONF-RECON-REFERENCE-FRAME-STORE-FUZZ: the source-backed
// reference-frame store must preserve slot, occupancy, replacement, removal,
// and iteration invariants for bounded public-API operation sequences derived
// from arbitrary bytes. This target intentionally does not parse AV2 bitstreams,
// invoke splot-decode, model AV2 reference refresh semantics, write filesystem
// paths, or invoke AVM/dav2d/ffmpeg. Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run recon_reference_frame_store_bytes
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_recon::{ReconError, ReferenceFrameStore, ReferenceSlot};

const MAX_RAW_CAPACITY: usize = 32;
const MAX_RAW_SLOT: usize = 20;
const MAX_OPERATIONS: usize = 64;
const PAYLOAD_BYTES: usize = 4;

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    assert_eq!(ReferenceSlot::MAX_SLOTS, 16);
    assert_slot_construction();
    assert_capacity_construction(input.byte());

    let initial_capacity = capacity_from_seed(input.byte());
    let operation_count = 1 + usize::from(input.byte()) % MAX_OPERATIONS;
    let mut case = StoreCase::new(initial_capacity);

    for _ in 0..operation_count {
        match input.byte() % 9 {
            0 => {
                let capacity = capacity_from_seed(input.byte());
                case.reset(capacity);
            }
            1 => {
                let raw_slot = raw_slot_from_seed(input.byte());
                assert_slot(raw_slot);
            }
            2 => {
                let raw_slot = raw_slot_from_seed(input.byte());
                case.assert_contains(raw_slot);
            }
            3 => {
                let raw_slot = raw_slot_from_seed(input.byte());
                case.assert_get(raw_slot);
            }
            4 => {
                let raw_slot = raw_slot_from_seed(input.byte());
                let payload = payload_from_input(&mut input);
                case.assert_put(raw_slot, payload);
            }
            5 => {
                let raw_slot = raw_slot_from_seed(input.byte());
                case.assert_take(raw_slot);
            }
            6 => {
                case.clear();
            }
            7 => {
                case.assert_entries();
            }
            _ => {
                case.assert_invariants();
            }
        }
        case.assert_invariants();
    }
});

fn assert_slot_construction() {
    for raw_slot in 0..=MAX_RAW_SLOT {
        assert_slot(raw_slot);
    }
}

fn assert_slot(raw_slot: usize) {
    match ReferenceSlot::new(raw_slot) {
        Ok(slot) => {
            assert!(raw_slot < ReferenceSlot::MAX_SLOTS);
            assert_eq!(slot.index(), raw_slot);
        }
        Err(ReconError::InvalidReferenceSlotIndex { index, max_slots }) => {
            assert!(raw_slot >= ReferenceSlot::MAX_SLOTS);
            assert_eq!(index, raw_slot);
            assert_eq!(max_slots, ReferenceSlot::MAX_SLOTS);
        }
        Err(other) => panic!("unexpected reference slot error: {other:?}"),
    }
}

fn assert_capacity_construction(seed: u8) {
    let capacity = capacity_from_seed(seed);
    match ReferenceFrameStore::<Payload>::with_capacity(capacity) {
        Ok(store) => {
            assert!((1..=ReferenceSlot::MAX_SLOTS).contains(&capacity));
            assert_eq!(store.capacity(), capacity);
            assert_eq!(store.occupied(), 0);
            assert!(store.is_empty());
            assert_eq!(store.entries().count(), 0);
        }
        Err(ReconError::InvalidReferenceStoreCapacity {
            capacity: observed,
            max_slots,
        }) => {
            assert!(capacity == 0 || capacity > ReferenceSlot::MAX_SLOTS);
            assert_eq!(observed, capacity);
            assert_eq!(max_slots, ReferenceSlot::MAX_SLOTS);
        }
        Err(other) => panic!("unexpected reference store capacity error: {other:?}"),
    }
}

fn capacity_from_seed(seed: u8) -> usize {
    usize::from(seed) % (MAX_RAW_CAPACITY + 1)
}

fn raw_slot_from_seed(seed: u8) -> usize {
    usize::from(seed) % (MAX_RAW_SLOT + 1)
}

fn payload_from_input(input: &mut FuzzInput<'_>) -> Payload {
    let id = u16::from(input.byte()) | (u16::from(input.byte()) << 8);
    let tag = input.byte();
    let bytes = [input.byte(), input.byte(), input.byte(), input.byte()];
    Payload::new(PayloadMeta { id, tag, bytes })
}

#[derive(Debug)]
struct StoreCase {
    store: Option<ReferenceFrameStore<Payload>>,
    oracle: Vec<Option<PayloadMeta>>,
}

impl StoreCase {
    fn new(capacity: usize) -> Self {
        let store = ReferenceFrameStore::with_capacity(capacity).ok();
        let oracle = match store.as_ref() {
            Some(store) => vec![None; store.capacity()],
            None => Vec::new(),
        };
        Self { store, oracle }
    }

    fn reset(&mut self, capacity: usize) {
        *self = Self::new(capacity);
    }

    fn clear(&mut self) {
        if let Some(store) = self.store.as_mut() {
            store.clear();
            self.oracle.fill(None);
        }
    }

    fn assert_contains(&self, raw_slot: usize) {
        let Some(slot) = ReferenceSlot::new(raw_slot).ok() else {
            return;
        };
        if let Some(store) = self.store.as_ref() {
            assert_eq!(store.contains_slot(slot), raw_slot < self.oracle.len());
        }
    }

    fn assert_get(&self, raw_slot: usize) {
        let Some(slot) = ReferenceSlot::new(raw_slot).ok() else {
            return;
        };
        let Some(store) = self.store.as_ref() else {
            return;
        };

        match store.get(slot) {
            Ok(frame) => {
                assert!(raw_slot < self.oracle.len());
                assert_eq!(frame.map(Payload::meta), self.oracle[raw_slot].as_ref());
            }
            Err(ReconError::ReferenceSlotOutOfBounds {
                slot: observed,
                capacity,
            }) => {
                assert!(raw_slot >= self.oracle.len());
                assert_eq!(observed, slot);
                assert_eq!(capacity, self.oracle.len());
            }
            Err(other) => panic!("unexpected get error: {other:?}"),
        }
    }

    fn assert_put(&mut self, raw_slot: usize, payload: Payload) {
        let Some(slot) = ReferenceSlot::new(raw_slot).ok() else {
            return;
        };
        let Some(store) = self.store.as_mut() else {
            return;
        };

        let meta = payload.meta;
        let before = self.oracle.clone();
        match store.put(slot, payload) {
            Ok(previous) => {
                assert!(raw_slot < self.oracle.len());
                let expected_previous = self.oracle[raw_slot];
                assert_eq!(
                    previous.map(|payload| payload.into_meta()),
                    expected_previous
                );
                self.oracle[raw_slot] = Some(meta);
            }
            Err(ReconError::ReferenceSlotOutOfBounds {
                slot: observed,
                capacity,
            }) => {
                assert!(raw_slot >= self.oracle.len());
                assert_eq!(observed, slot);
                assert_eq!(capacity, self.oracle.len());
                assert_eq!(self.oracle, before);
            }
            Err(other) => panic!("unexpected put error: {other:?}"),
        }
    }

    fn assert_take(&mut self, raw_slot: usize) {
        let Some(slot) = ReferenceSlot::new(raw_slot).ok() else {
            return;
        };
        let Some(store) = self.store.as_mut() else {
            return;
        };

        let before = self.oracle.clone();
        match store.take(slot) {
            Ok(previous) => {
                assert!(raw_slot < self.oracle.len());
                let expected_previous = self.oracle[raw_slot].take();
                assert_eq!(
                    previous.map(|payload| payload.into_meta()),
                    expected_previous
                );
            }
            Err(ReconError::ReferenceSlotOutOfBounds {
                slot: observed,
                capacity,
            }) => {
                assert!(raw_slot >= self.oracle.len());
                assert_eq!(observed, slot);
                assert_eq!(capacity, self.oracle.len());
                assert_eq!(self.oracle, before);
            }
            Err(other) => panic!("unexpected take error: {other:?}"),
        }
    }

    fn assert_entries(&self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };

        let entries: Vec<(usize, PayloadMeta)> = store
            .entries()
            .map(|entry| (entry.slot().index(), *entry.frame().meta()))
            .collect();
        let expected: Vec<(usize, PayloadMeta)> = self
            .oracle
            .iter()
            .enumerate()
            .filter_map(|(slot, payload)| payload.map(|payload| (slot, payload)))
            .collect();

        assert_eq!(entries, expected);
        assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    fn assert_invariants(&self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };

        let expected_occupied = self.oracle.iter().filter(|slot| slot.is_some()).count();
        assert_eq!(store.capacity(), self.oracle.len());
        assert_eq!(store.occupied(), expected_occupied);
        assert_eq!(store.is_empty(), expected_occupied == 0);
        assert!(store.occupied() <= store.capacity());
        self.assert_entries();

        for raw_slot in 0..=ReferenceSlot::MAX_SLOTS {
            self.assert_contains(raw_slot);
            self.assert_get(raw_slot);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PayloadMeta {
    id: u16,
    tag: u8,
    bytes: [u8; PAYLOAD_BYTES],
}

#[derive(Debug)]
struct Payload {
    meta: PayloadMeta,
}

impl Payload {
    const fn new(meta: PayloadMeta) -> Self {
        Self { meta }
    }

    const fn meta(&self) -> &PayloadMeta {
        &self.meta
    }

    fn into_meta(self) -> PayloadMeta {
        self.meta
    }
}

struct FuzzInput<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> FuzzInput<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn byte(&mut self) -> u8 {
        let byte = self.bytes.get(self.offset).copied().unwrap_or(0);
        self.offset = self.offset.saturating_add(1);
        byte
    }
}
