// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Reference-frame buffer state model (AV2 v1.0.0 § 7.23).
//!
//! The validator tracks the § 7.23 *Reference frame update process* state per
//! extended layer from each completed frame's parsed `refresh_frame_flags`,
//! `OrderHint`, and dimensions, so reference-state-gated frame-header decisions and
//! § 6/§ 7 checks become reachable. This is **validator-derived** state — the
//! bitstream does not declare reference-frame buffers out of band (they are written
//! only by the § 7.23 process at the end of each decoded frame), so the tracker is
//! the single in-band source of truth and never consults external HLS.
//!
//! # § 7.23 update semantics (mirror `07-decoding-process.md#s-7-23`, :14095-14124)
//!
//! "For each value of i from 0 to NUM_REF_FRAMES - 1, the following applies if bit i
//! of refresh_frame_flags is equal to 1": slot `i` is overwritten with the current
//! frame's facts. `RefValid[ i ]` is set to `(FrameType == KEY_FRAME || FrameType ==
//! SWITCH_FRAME) ? first : 1`, where `first` is `1` only for the *lowest* refreshed
//! slot and `0` thereafter (so a key/switch frame leaves only its lowest refreshed
//! slot valid, invalidating the rest it touches). `RefOrderHint[ i ] = OrderHint`,
//! `RefFrameWidth[ i ] = FrameWidth`, `RefFrameHeight[ i ] = FrameHeight`. This phase
//! models the subset the parsed intra header decides: `RefValid`, `RefOrderHint`, and
//! the frame dimensions; the remaining § 7.23 arrays (motion vectors, CDFs, grain,
//! CCSO, …) are out of scope for the validator's reference-state checks.
//!
//! # Resets (grounded, never guessed)
//!
//! - **New CVS / CLK reset** (`OBU_CLOSED_LOOP_KEY && FirstPictureInTU`): § 5.18.2
//!   (mirror `05-syntax-structures.md` :4449-4455) sets `RefValid[ i ] = 0` for every
//!   `i` in `0..NumRefFrames` *before* `refresh_frame_flags` is applied. The tracker
//!   models this as the slots becoming [`SlotState::ProvenInvalid`]; the CLK's own
//!   refresh mask (`refresh_frame_flags = allFrames` for `max_mlayer_id == 0`, mirror
//!   :4431-4433) then re-validates per § 7.23 with the key-frame `first` rule.
//! - **A mid-stream join** (the validator starts inside the bitstream at a random
//!   access point) begins with every slot [`SlotState::Unknown`], so an early
//!   reference against an unestablished slot stays Unknown (no false positive) until a
//!   grounded reset establishes the buffer. This is the random-access-replay soundness
//!   property: the tracker is all-poisoned until the first grounded reset it observes.
//!
//! # Honest poisoning
//!
//! When a frame's refresh mask is **not parsed** — an inter / TIP / bridge frame, a
//! truncation, an `Unknown`-classed frame, or an [`FrameBoundary::Ambiguous`] coded-frame
//! boundary — the mask could refresh *any* slot, so **all** slots are poisoned to
//! [`SlotState::Unknown`]. Per-slot `Unknown` is the resting state; the tracker never
//! guesses which slots an unparsed mask touched.

use splot_core::headers::frame::FrameType;
use splot_core::types::ExtendedLayerId;

use std::collections::BTreeMap;

/// `NUM_REF_FRAMES` — the number of reference-frame slots (AV2 v1.0.0 § 3, symbol
/// table mirror `03-symbols.md` :607: "NUM_REF_FRAMES 16"). The § 7.23 loop runs `i`
/// over `0..NUM_REF_FRAMES`; the active sequence header's `NumRefFrames` (≤ 16) bounds
/// how many of those slots the CLK reset and `allFrames` mask touch.
pub(crate) const NUM_REF_FRAMES: usize = 16;

/// One reference-frame slot's modeled § 7.23 state.
///
/// Only the reference-state-relevant subset is modeled (the parsed-intra-header-decidable
/// fields): validity plus, when valid, `RefOrderHint` and the stored dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotState {
    /// The slot's state is not known to the validator: it was never established in this
    /// modeled run, or a frame whose refresh mask could not be parsed may have written
    /// it. A reference against an `Unknown` slot is undecidable and drops to silence.
    Unknown,
    /// `RefValid[ i ] == 0` is **proven**: a CLK reset cleared the slot and no later
    /// parsed frame re-validated it (§ 5.18.2 :4449-4455 / § 7.23). A reference against a
    /// proven-invalid slot is a decidable conformance defect.
    ProvenInvalid,
    /// `RefValid[ i ] == 1` is **proven**, with the § 7.23 stored facts this phase models.
    Valid(SlotFacts),
}

/// The § 7.23 stored facts for a proven-valid slot that this phase models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlotFacts {
    /// `RefOrderHint[ i ]` — the modeled `OrderHint` LSB proxy stored at refresh time
    /// (§ 7.23 :14123). The intra path parses `order_hint` / `sef_order_hint` as the LSB,
    /// so this is the LSB value the parser decided (see [`crate::celu`]).
    pub(crate) order_hint: u32,
    /// `RefFrameWidth[ i ]` (§ 7.23 :14102).
    pub(crate) width: u32,
    /// `RefFrameHeight[ i ]` (§ 7.23 :14103).
    pub(crate) height: u32,
    /// `RefLongTermId[ i ]` (§ 7.23 :14113), the slot's modeled long-term id. `None` models
    /// the spec's `-1` sentinel ("not a long-term reference frame"); `Some(id)` is a KEY
    /// frame's `LongTermId = long_term_id_plus_1 - 1` (§ 5.18.2 mirror :4231-4239), the only
    /// frame type that establishes a non-`-1` `LongTermId`. The § 6.17.2 RAS
    /// `long_term_id_in_use(RefLongTermId[ ref_frame_idx[i] ])` check (mirror :4615-4616)
    /// reads this for the slots a RAS frame selects.
    pub(crate) long_term_id: Option<u32>,
}

/// One extended layer's `NUM_REF_FRAMES`-slot reference-frame buffer state.
///
/// Scoping decision (per § 7.23 / § 7.3.6): the § 7.23 process runs once per decoded
/// frame in that frame's decoding context, and the validator's coded-video-sequence /
/// CELU model is keyed per **extended layer** (`obu_xlayer_id`) — the granularity at
/// which a CLK starts a new CVS and resets the reference buffers (§ 7.3.6). So the
/// reference buffer is modeled per extended layer, matching the existing CVS-epoch and
/// `frame_header_copy_record` keying. (Embedded/temporal layers select *which* slots a
/// frame references within that buffer via the inter syntax — not modeled this phase —
/// but the buffer itself is one per extended layer.)
#[derive(Debug, Clone)]
struct LayerReferenceState {
    /// The `NUM_REF_FRAMES` slots. Initialized all-[`SlotState::Unknown`] (a fresh /
    /// mid-stream-join layer has an unestablished buffer).
    slots: [SlotState; NUM_REF_FRAMES],
}

impl Default for LayerReferenceState {
    fn default() -> Self {
        Self {
            slots: [SlotState::Unknown; NUM_REF_FRAMES],
        }
    }
}

impl LayerReferenceState {
    /// Applies the § 5.18.2 CLK reset: `RefValid[ i ] = 0` for `i` in
    /// `0..numRefFrames` (mirror :4449-4455). Slots at or beyond `numRefFrames` are not
    /// touched by the reset (the loop bound is `NumRefFrames`), but a conformant stream
    /// never references them; they keep their prior state.
    fn clk_reset(&mut self, num_ref_frames: usize) {
        let bound = num_ref_frames.min(NUM_REF_FRAMES);
        for slot in &mut self.slots[..bound] {
            *slot = SlotState::ProvenInvalid;
        }
    }

    /// Applies the § 7.23 update for a parsed frame: for each set bit of
    /// `refresh_frame_flags`, store the frame's facts into that slot and set its
    /// `RefValid` per the key/switch `first` rule (mirror :14095-14124).
    fn apply_refresh(
        &mut self,
        refresh_frame_flags: u32,
        is_key_or_switch: bool,
        facts: SlotFacts,
    ) {
        let mut first = true;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if (refresh_frame_flags >> i) & 1 != 1 {
                continue;
            }
            let ref_valid = if is_key_or_switch { first } else { true };
            *slot = if ref_valid {
                SlotState::Valid(facts)
            } else {
                SlotState::ProvenInvalid
            };
            first = false;
        }
    }

    /// Poisons every slot to [`SlotState::Unknown`]: the resting state when a frame's
    /// refresh mask could not be parsed (it may have refreshed any slot).
    fn poison_all(&mut self) {
        self.slots = [SlotState::Unknown; NUM_REF_FRAMES];
    }
}

/// The § 7.23 reference-frame buffer state, tracked per extended layer.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReferenceStateTracker {
    /// Per extended layer, its `NUM_REF_FRAMES`-slot buffer. A layer with no entry has
    /// an all-[`SlotState::Unknown`] buffer (lazily inserted on first observation).
    layers: BTreeMap<ExtendedLayerId, LayerReferenceState>,
}

/// The grounded outcome of a completed frame for the § 7.23 update, derived by the
/// validator from the parsed frame-header core and the segmenter boundary.
///
/// The validator decides the kind from the parsed core (`refresh_frame_flags`,
/// `frame_type`, `frame_size`, `order_hint_lsb`, the CLK-reset condition) and the
/// segmenter's coded-frame boundary; this module only *applies* it. Anything the
/// validator cannot ground becomes [`FrameRefUpdate::PoisonAll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameRefUpdate {
    /// A CLK that starts a new CVS (`OBU_CLOSED_LOOP_KEY && FirstPictureInTU`): reset
    /// `RefValid[ i ] = 0` over `0..numRefFrames` (§ 5.18.2 :4449-4455), then apply this
    /// frame's own refresh mask per § 7.23 with the key-frame `first` rule.
    ClkReset {
        /// `NumRefFrames` from the active sequence header, bounding the reset loop.
        num_ref_frames: usize,
        /// This CLK's parsed `refresh_frame_flags` (the post-reset § 7.23 update).
        refresh_frame_flags: u32,
        /// The slot facts to store for the refreshed slots.
        facts: SlotFacts,
    },
    /// A non-CLK frame whose refresh mask, frame type, dims, and order hint were all
    /// parsed: apply the § 7.23 update with the key/switch `first` rule.
    Refresh {
        /// The parsed `refresh_frame_flags`.
        refresh_frame_flags: u32,
        /// `FrameType == KEY_FRAME || FrameType == SWITCH_FRAME` (the `first` rule).
        is_key_or_switch: bool,
        /// The slot facts to store for the refreshed slots.
        facts: SlotFacts,
    },
    /// A show-existing-frame: `refresh_frame_flags = 0` (§ 5.18.2 :4180), so § 7.23
    /// updates **no** slot. The buffer is unchanged.
    SefNoUpdate,
    /// The frame's refresh mask could not be parsed (inter / TIP / bridge / truncated /
    /// Unknown-classed / ambiguous-boundary frame): poison all slots, since the unknown
    /// mask could refresh any of them.
    PoisonAll,
}

impl ReferenceStateTracker {
    /// Applies a completed frame's § 7.23 update to its extended layer's buffer.
    ///
    /// Called at the **grounded decode point** of a frame (the segmenter's coded-frame
    /// boundary, with the no-trailing-delimiter end-of-bitstream flush), AFTER the
    /// frame's facts are known and BEFORE any later frame's reference checks consult the
    /// buffer — matching § 7.23's "final step in decoding a frame" ordering.
    pub(crate) fn apply(&mut self, xlayer: ExtendedLayerId, update: FrameRefUpdate) {
        let layer = self.layers.entry(xlayer).or_default();
        match update {
            FrameRefUpdate::ClkReset {
                num_ref_frames,
                refresh_frame_flags,
                facts,
            } => {
                layer.clk_reset(num_ref_frames);
                layer.apply_refresh(refresh_frame_flags, true, facts);
            }
            FrameRefUpdate::Refresh {
                refresh_frame_flags,
                is_key_or_switch,
                facts,
            } => layer.apply_refresh(refresh_frame_flags, is_key_or_switch, facts),
            FrameRefUpdate::SefNoUpdate => {}
            FrameRefUpdate::PoisonAll => layer.poison_all(),
        }
    }

    /// Returns the modeled state of slot `idx` in `xlayer`'s buffer. A layer with no
    /// observed frames, or a slot index at or beyond `NUM_REF_FRAMES`, is
    /// [`SlotState::Unknown`] (an unestablished / out-of-range slot is never *proven*
    /// invalid, so a reference against it drops to silence rather than firing).
    pub(crate) fn slot(&self, xlayer: ExtendedLayerId, idx: usize) -> SlotState {
        if idx >= NUM_REF_FRAMES {
            return SlotState::Unknown;
        }
        self.layers
            .get(&xlayer)
            .map_or(SlotState::Unknown, |layer| layer.slots[idx])
    }

    /// The modeled `RefLongTermId[ idx ]` of slot `idx` in `xlayer`'s buffer, when the slot
    /// is **proven valid** (§ 7.23): `Some(Some(id))` for a long-term slot, `Some(None)` for
    /// a proven-valid non-long-term slot (the spec's `-1`). Returns `None` when the slot is
    /// `Unknown` or `ProvenInvalid` — the long-term id is then undecidable, so a dependent
    /// judgment must drop to silence (the Unknown invariant).
    #[allow(clippy::option_option)]
    pub(crate) fn slot_long_term_id(
        &self,
        xlayer: ExtendedLayerId,
        idx: usize,
    ) -> Option<Option<u32>> {
        match self.slot(xlayer, idx) {
            SlotState::Valid(facts) => Some(facts.long_term_id),
            SlotState::Unknown | SlotState::ProvenInvalid => None,
        }
    }

    /// Borrows `xlayer`'s `RefValid[]` / `RefOrderHint[]` / dims as the parallel slices a
    /// [`splot_core::headers::frame::FrameReferenceStateView`] threads into the parser,
    /// writing them into the caller-provided scratch buffers. Returns `None` when the
    /// layer has no buffer yet (every slot Unknown — there is nothing to thread).
    ///
    /// An `Unknown` slot contributes `RefValid[i] = false` (the parser must not treat an
    /// unestablished slot as available); a `ProvenInvalid` slot likewise contributes
    /// `false`; a `Valid` slot contributes `true` plus its stored facts. Width/height/
    /// order-hint for non-valid slots are filler `0` — the parser reads them only where
    /// `RefValid[i]` is `true` (the § 5.18 reference paths gate on `RefValid`).
    pub(crate) fn view_into<'a>(
        &self,
        xlayer: ExtendedLayerId,
        ref_valid: &'a mut [bool; NUM_REF_FRAMES],
        ref_order_hint: &'a mut [u32; NUM_REF_FRAMES],
        ref_width: &'a mut [u32; NUM_REF_FRAMES],
        ref_height: &'a mut [u32; NUM_REF_FRAMES],
    ) -> Option<()> {
        let layer = self.layers.get(&xlayer)?;
        for (i, slot) in layer.slots.iter().enumerate() {
            match slot {
                SlotState::Valid(facts) => {
                    ref_valid[i] = true;
                    ref_order_hint[i] = facts.order_hint;
                    ref_width[i] = facts.width;
                    ref_height[i] = facts.height;
                }
                SlotState::Unknown | SlotState::ProvenInvalid => {
                    ref_valid[i] = false;
                    ref_order_hint[i] = 0;
                    ref_width[i] = 0;
                    ref_height[i] = 0;
                }
            }
        }
        Some(())
    }

    /// Drops all per-layer state — used only at the end of the bitstream is not needed
    /// (the tracker is consumed with the context); exposed for symmetry/tests.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

/// Builds [`SlotFacts`] from a parsed frame's `OrderHint` LSB, dimensions, and
/// `LongTermId`, or `None` when any *required* fact is missing (the validator then
/// poisons rather than storing partial facts).
///
/// `long_term_id` is the parsed `LongTermId` (`< 0` is the spec's `-1` "not a long-term
/// frame" sentinel, mapped to `None`; `>= 0` is a KEY frame's `long_term_id_plus_1 - 1`).
/// A `None` for `long_term_id` (the frame's parse never reached the long-term field) is
/// treated as the `-1` sentinel — the slot is then modeled as not a long-term frame — so a
/// missing `LongTermId` does NOT poison: a frame's reference-state facts (order hint, dims)
/// can be fully grounded while its `LongTermId` defaults to "not long-term", which is the
/// only conformant value when `long_term_frame_id_bits == 0`.
pub(crate) fn slot_facts(
    order_hint_lsb: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    long_term_id: Option<i64>,
) -> Option<SlotFacts> {
    Some(SlotFacts {
        order_hint: order_hint_lsb?,
        width: width?,
        height: height?,
        long_term_id: long_term_id.and_then(|id| u32::try_from(id).ok()),
    })
}

/// Whether a `FrameType` is `KEY_FRAME` or `SWITCH_FRAME` for the § 7.23 `first`
/// RefValid rule (mirror :14100).
pub(crate) fn is_key_or_switch(frame_type: FrameType) -> bool {
    matches!(frame_type, FrameType::Key | FrameType::Switch)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const XL: ExtendedLayerId = ExtendedLayerId::from_bits(0);

    fn facts(order_hint: u32) -> SlotFacts {
        SlotFacts {
            order_hint,
            width: 320,
            height: 240,
            long_term_id: None,
        }
    }

    /// A long-term-bearing slot fact with the given `RefLongTermId`.
    fn lt_facts(order_hint: u32, long_term_id: u32) -> SlotFacts {
        SlotFacts {
            order_hint,
            width: 320,
            height: 240,
            long_term_id: Some(long_term_id),
        }
    }

    #[test]
    fn fresh_layer_is_all_unknown() {
        let tracker = ReferenceStateTracker::default();
        assert!(tracker.is_empty());
        for i in 0..NUM_REF_FRAMES {
            assert_eq!(tracker.slot(XL, i), SlotState::Unknown);
        }
        assert_eq!(tracker.slot(XL, NUM_REF_FRAMES), SlotState::Unknown);
        assert_eq!(tracker.slot(XL, 999), SlotState::Unknown);
    }

    #[test]
    fn refresh_stores_facts_into_set_slots_only() {
        let mut tracker = ReferenceStateTracker::default();
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0b101,
                is_key_or_switch: false,
                facts: facts(7),
            },
        );
        assert_eq!(tracker.slot(XL, 0), SlotState::Valid(facts(7)));
        assert_eq!(tracker.slot(XL, 1), SlotState::Unknown);
        assert_eq!(tracker.slot(XL, 2), SlotState::Valid(facts(7)));
        assert_eq!(tracker.slot(XL, 3), SlotState::Unknown);
    }

    #[test]
    fn key_switch_first_rule_validates_only_lowest_refreshed_slot() {
        let mut tracker = ReferenceStateTracker::default();
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0b10_1010,
                is_key_or_switch: true,
                facts: facts(3),
            },
        );
        assert_eq!(tracker.slot(XL, 1), SlotState::Valid(facts(3)));
        assert_eq!(tracker.slot(XL, 3), SlotState::ProvenInvalid);
        assert_eq!(tracker.slot(XL, 5), SlotState::ProvenInvalid);
        assert_eq!(tracker.slot(XL, 0), SlotState::Unknown);
    }

    #[test]
    fn clk_reset_invalidates_then_refreshes() {
        let mut tracker = ReferenceStateTracker::default();
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0b1111,
                is_key_or_switch: false,
                facts: facts(1),
            },
        );
        tracker.apply(
            XL,
            FrameRefUpdate::ClkReset {
                num_ref_frames: 8,
                refresh_frame_flags: 0b1,
                facts: facts(9),
            },
        );
        assert_eq!(tracker.slot(XL, 0), SlotState::Valid(facts(9)));
        assert_eq!(tracker.slot(XL, 1), SlotState::ProvenInvalid);
        assert_eq!(tracker.slot(XL, 3), SlotState::ProvenInvalid);
    }

    #[test]
    fn clk_reset_with_all_frames_mask_validates_only_lowest() {
        let mut tracker = ReferenceStateTracker::default();
        tracker.apply(
            XL,
            FrameRefUpdate::ClkReset {
                num_ref_frames: 8,
                refresh_frame_flags: 0xFFFF,
                facts: facts(0),
            },
        );
        assert_eq!(tracker.slot(XL, 0), SlotState::Valid(facts(0)));
        for i in 1..NUM_REF_FRAMES {
            assert_eq!(tracker.slot(XL, i), SlotState::ProvenInvalid);
        }
    }

    #[test]
    fn sef_does_not_update() {
        let mut tracker = ReferenceStateTracker::default();
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0b1,
                is_key_or_switch: false,
                facts: facts(5),
            },
        );
        tracker.apply(XL, FrameRefUpdate::SefNoUpdate);
        assert_eq!(tracker.slot(XL, 0), SlotState::Valid(facts(5)));
    }

    #[test]
    fn poison_all_makes_every_slot_unknown() {
        let mut tracker = ReferenceStateTracker::default();
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0xFFFF,
                is_key_or_switch: false,
                facts: facts(2),
            },
        );
        tracker.apply(XL, FrameRefUpdate::PoisonAll);
        for i in 0..NUM_REF_FRAMES {
            assert_eq!(tracker.slot(XL, i), SlotState::Unknown);
        }
    }

    #[test]
    fn layers_are_independent() {
        let mut tracker = ReferenceStateTracker::default();
        let xl1 = ExtendedLayerId::from_bits(1);
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0b1,
                is_key_or_switch: false,
                facts: facts(4),
            },
        );
        assert_eq!(tracker.slot(xl1, 0), SlotState::Unknown);
        assert_eq!(tracker.slot(XL, 0), SlotState::Valid(facts(4)));
    }

    #[test]
    fn view_into_reflects_slot_validity() {
        let mut tracker = ReferenceStateTracker::default();
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0b1,
                is_key_or_switch: false,
                facts: SlotFacts {
                    order_hint: 11,
                    width: 64,
                    height: 48,
                    long_term_id: None,
                },
            },
        );
        let mut ref_valid = [false; NUM_REF_FRAMES];
        let mut ref_order_hint = [0u32; NUM_REF_FRAMES];
        let mut ref_width = [0u32; NUM_REF_FRAMES];
        let mut ref_height = [0u32; NUM_REF_FRAMES];
        let got = tracker.view_into(
            XL,
            &mut ref_valid,
            &mut ref_order_hint,
            &mut ref_width,
            &mut ref_height,
        );
        assert!(got.is_some());
        assert!(ref_valid[0]);
        assert_eq!(ref_order_hint[0], 11);
        assert_eq!(ref_width[0], 64);
        assert_eq!(ref_height[0], 48);
        assert!(!ref_valid[1]);
        let absent = tracker.view_into(
            ExtendedLayerId::from_bits(2),
            &mut ref_valid,
            &mut ref_order_hint,
            &mut ref_width,
            &mut ref_height,
        );
        assert!(absent.is_none());
    }

    #[test]
    fn slot_facts_requires_order_hint_and_dims_but_not_long_term_id() {
        assert!(slot_facts(Some(1), Some(2), Some(3), Some(-1)).is_some());
        assert!(slot_facts(None, Some(2), Some(3), Some(-1)).is_none());
        assert!(slot_facts(Some(1), None, Some(3), Some(-1)).is_none());
        assert!(slot_facts(Some(1), Some(2), None, Some(-1)).is_none());
        assert_eq!(
            slot_facts(Some(1), Some(2), Some(3), Some(-1))
                .unwrap()
                .long_term_id,
            None
        );
        assert_eq!(
            slot_facts(Some(1), Some(2), Some(3), None)
                .unwrap()
                .long_term_id,
            None
        );
        assert_eq!(
            slot_facts(Some(1), Some(2), Some(3), Some(4))
                .unwrap()
                .long_term_id,
            Some(4)
        );
    }

    #[test]
    fn slot_long_term_id_reads_only_valid_slots() {
        let mut tracker = ReferenceStateTracker::default();
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0b1,
                is_key_or_switch: false,
                facts: lt_facts(7, 5),
            },
        );
        tracker.apply(
            XL,
            FrameRefUpdate::Refresh {
                refresh_frame_flags: 0b100,
                is_key_or_switch: false,
                facts: facts(8),
            },
        );
        assert_eq!(tracker.slot_long_term_id(XL, 0), Some(Some(5)));
        assert_eq!(tracker.slot_long_term_id(XL, 2), Some(None));
        assert_eq!(tracker.slot_long_term_id(XL, 1), None);
    }

    #[test]
    fn is_key_or_switch_classifies_frame_types() {
        assert!(is_key_or_switch(FrameType::Key));
        assert!(is_key_or_switch(FrameType::Switch));
        assert!(!is_key_or_switch(FrameType::Inter));
        assert!(!is_key_or_switch(FrameType::IntraOnly));
    }

    proptest! {
        /// `apply` never panics on any update kind, and `slot` is always queryable.
        #[test]
        fn apply_never_panics(
            mask in any::<u32>(),
            is_kos in any::<bool>(),
            order_hint in any::<u32>(),
            width in any::<u32>(),
            height in any::<u32>(),
            num_ref in 0usize..=64,
            kind in 0u8..=3,
            xlayer_raw in 0u8..=31,
        ) {
            let mut tracker = ReferenceStateTracker::default();
            let xl = ExtendedLayerId::from_bits(xlayer_raw);
            let f = SlotFacts { order_hint, width, height, long_term_id: None };
            let update = match kind {
                0 => FrameRefUpdate::ClkReset { num_ref_frames: num_ref, refresh_frame_flags: mask, facts: f },
                1 => FrameRefUpdate::Refresh { refresh_frame_flags: mask, is_key_or_switch: is_kos, facts: f },
                2 => FrameRefUpdate::SefNoUpdate,
                _ => FrameRefUpdate::PoisonAll,
            };
            tracker.apply(xl, update);
            for i in 0..(NUM_REF_FRAMES + 4) {
                let _ = tracker.slot(xl, i);
            }
        }

        /// Refresh invariant: a non-key/switch refresh leaves exactly the set-bit slots
        /// valid (with the stored facts) and the others unchanged from Unknown.
        #[test]
        fn refresh_validates_exactly_set_bits(mask in any::<u16>()) {
            let mut tracker = ReferenceStateTracker::default();
            let f = SlotFacts { order_hint: 1, width: 2, height: 3, long_term_id: None };
            tracker.apply(
                XL,
                FrameRefUpdate::Refresh {
                    refresh_frame_flags: u32::from(mask),
                    is_key_or_switch: false,
                    facts: f,
                },
            );
            for i in 0..NUM_REF_FRAMES {
                let expected = if (mask >> i) & 1 == 1 {
                    SlotState::Valid(f)
                } else {
                    SlotState::Unknown
                };
                prop_assert_eq!(tracker.slot(XL, i), expected);
            }
        }

        /// PoisonAll always wins: after it, every slot is Unknown regardless of prior state.
        #[test]
        fn poison_all_dominates(mask in any::<u32>(), is_kos in any::<bool>()) {
            let mut tracker = ReferenceStateTracker::default();
            tracker.apply(
                XL,
                FrameRefUpdate::Refresh {
                    refresh_frame_flags: mask,
                    is_key_or_switch: is_kos,
                    facts: SlotFacts { order_hint: 9, width: 9, height: 9, long_term_id: None },
                },
            );
            tracker.apply(XL, FrameRefUpdate::PoisonAll);
            for i in 0..NUM_REF_FRAMES {
                prop_assert_eq!(tracker.slot(XL, i), SlotState::Unknown);
            }
        }
    }
}
