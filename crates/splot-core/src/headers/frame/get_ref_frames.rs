// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The implicit reference-map ranking — `get_ref_frames()` (AV2 v1.0.0 § 7.7,
//! `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-7`).
//!
//! When `frame_header_info()` (§ 5.18.2) is NOT using the explicit reference map
//! (`explicitRefFrameMap == 0`), it calls `get_ref_frames( checkRes )` (mirror :4607 with
//! `checkRes == 0`, then mirror :4647 with `checkRes == 1`) to DERIVE `NumTotalRefs` and the
//! `ref_frame_idx[]` array from the saved per-slot quantizer and display-order-hint state —
//! no bits are read, but the result determines every later inter-header bit position
//! (`CeilLog2(NumTotalRefs)` for `bru_ref`, the `use_ref_frame_mvs` / TIP gates that test
//! `NumTotalRefs`, …). This module models that derivation as a typed, total, panic-free
//! function so the inter parser can advance past it for the minimal single-reference frame.
//!
//! ## What is modeled
//!
//! [`get_ref_frames`] implements the full § 7.7 ranking on EXPLICIT per-slot inputs
//! ([`GetRefFramesInput`]): the distinct-reference detection (`first_slot_with_ref`), the
//! resolution gate (`valid_ref_frame_size`), the per-reference scoring (`Dist_Score_Lookup`,
//! the `tDist` / `maxDisp` arms, the `refRatio` penalty), the duplicate-score rejection
//! (`new_score_or_dist`), the over-selection drop (`get_unmapped_ref`), the score sort
//! (`bubble_sort_ref_scores`), the `NumTotalRefs = Min(NRanked, ActiveNumRefFrames)` cut, and
//! the trailing restricted-frame append (the `checkRes && !IsBridge` loop). It is derived
//! from the § 7.7 spec text, never from AVM source.
//!
//! ## What the caller must supply vs default
//!
//! § 7.7 reads saved reference state the validator's § 7.23 buffer models in part
//! ([`super::FrameReferenceStateView`] carries `RefValid` / `RefOrderHint` / dims, and — via
//! [`super::FrameReferenceStateView::from_slots_with_base_q_idx`] — `RefBaseQIdx`) and in
//! part does NOT yet model (`RefCounter`, `RefMLayerId`, `RefTLayerId`, the per-frame
//! `AllowedFrames`, the `TLayerDependencyMap` / `MLayerDependencyMap`). The caller passes
//! whatever it can prove; for the single-spatial-layer minimal inter frame the unmodeled
//! facts are deterministic (one TU layer → all dependency maps `1`, `AllowedFrames == -1`,
//! and the `new_score_or_dist` dedup makes a shared `RefCounter` harmless), so the result is
//! exact. With `RefBaseQIdx` supplied (the `from_slots_with_base_q_idx` caller) the inter
//! parser admits the multi-reference (>= 2 valid slot) case; the
//! [`super::FrameReferenceStateView::from_slots`] caller (no `RefBaseQIdx`) still stops at an
//! honest `UnmodeledDerivation` for > 1 valid slot.

/// `NUM_REF_FRAMES` (AV2 v1.0.0 § 3): the number of reference-frame buffer slots.
pub(crate) const NUM_REF_FRAMES: usize = 16;

/// `REFS_PER_FRAME` (AV2 v1.0.0 § 3, `docs/spec/av2/1.0.0/03-symbols.md`): the maximum number
/// of reference frames that can be used by a frame, bounding `ActiveNumRefFrames`.
pub(crate) const REFS_PER_FRAME: u32 = 7;

/// `RESTRICTED_OH` (AV2 v1.0.0 § 3): the sentinel order hint marking a restricted reference
/// frame. § 7.7 excludes a slot whose `RefOrderHint` equals this from the scored set, then
/// (when `checkRes && !IsBridge`) appends any remaining restricted slots at the end.
pub const RESTRICTED_OH: i32 = -1;

/// `DIST_WEIGHT_BITS` (AV2 v1.0.0 § 3): the scaling applied to `tDist` in the
/// `maxDisp > OrderHint` scoring arm (§ 7.7).
const DIST_WEIGHT_BITS: u32 = 6;

/// `DECAY_DIST_CAP` (AV2 v1.0.0 § 3): the maximum distance that can index
/// `Dist_Score_Lookup` (§ 7.7).
const DECAY_DIST_CAP: i32 = 6;

/// `Dist_Score_Lookup[ DECAY_DIST_CAP + 1 ]` (AV2 v1.0.0 § 7.7,
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-7`): the decay score table indexed by
/// `Min(tDist, DECAY_DIST_CAP)` in the `maxDisp <= OrderHint` scoring arm.
const DIST_SCORE_LOOKUP: [i32; (DECAY_DIST_CAP as usize) + 1] = [0, 64, 96, 112, 120, 124, 126];

/// One reference-frame buffer slot's saved state, as § 7.7 reads it (AV2 v1.0.0 § 7.23 sets
/// each field on the reference-frame update; § 7.7 consumes them).
///
/// Every field is named for the spec variable it mirrors. `order_hint` carries `RefOrderHint`
/// as a signed value so the `RESTRICTED_OH` (`-1`) sentinel is representable distinctly from a
/// real order hint of `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RefSlot {
    /// `RefValid[ i ]` (§ 7.23): whether the slot holds a usable reference frame.
    pub valid: bool,
    /// `RefOrderHint[ i ]` (§ 7.23), or [`RESTRICTED_OH`] for a restricted reference.
    pub order_hint: i32,
    /// `RefBaseQIdx[ i ]` (§ 7.23): the slot's base quantizer index, used in the score.
    pub base_q_idx: u32,
    /// `RefCounter[ i ]` (§ 7.23): equal across slots that store the same decoded frame, so
    /// `first_slot_with_ref` keeps only the lowest-indexed slot of each distinct frame.
    pub counter: u32,
    /// `RefMLayerId[ i ]` (§ 7.23): the slot's modeling-layer id, used in `tDist` and the
    /// dependency-map gate.
    pub mlayer_id: u8,
    /// `RefTLayerId[ i ]` (§ 7.23): the slot's temporal-layer id, used in the dependency-map
    /// gate.
    pub tlayer_id: u8,
    /// `RefFrameWidth[ i ]` (§ 7.23): the slot's stored frame width, used in `refRatio` and
    /// `valid_ref_frame_size`.
    pub width: u32,
    /// `RefFrameHeight[ i ]` (§ 7.23): the slot's stored frame height.
    pub height: u32,
}

/// The per-frame and per-slot inputs to `get_ref_frames()` (AV2 v1.0.0 § 7.7).
///
/// The caller fills `slots[0..num_ref_frames]`; entries at or beyond `num_ref_frames` are
/// ignored (the § 7.7 loops run `i = 0 .. NumRefFrames - 1`).
#[derive(Debug, Clone)]
pub(crate) struct GetRefFramesInput {
    /// `NumRefFrames` (§ 5.4.6): the active reference-slot count; bounds the § 7.7 loops.
    pub num_ref_frames: u32,
    /// The per-slot saved reference state, one entry per `0..NUM_REF_FRAMES`.
    pub slots: [RefSlot; NUM_REF_FRAMES],
    /// `OrderHint` (§ 5.18.2): the current frame's display order hint.
    pub order_hint: i32,
    /// `obu_mlayer_id` (§ 5.2.2): the current frame's modeling-layer id, used in `tDist` and
    /// the dependency-map gates.
    pub obu_mlayer_id: u8,
    /// `obu_tlayer_id` (§ 5.2.2): the current frame's temporal-layer id, used in the
    /// dependency-map gate.
    pub obu_tlayer_id: u8,
    /// `AllowedFrames` (§ 5.18.2 :4539): the bitmask of slots eligible for selection (`-1`
    /// when every slot is allowed). Modeled as a signed value so the spec's `-1` all-ones
    /// default is representable; only the low `NUM_REF_FRAMES` bits are tested.
    pub allowed_frames: i32,
    /// `IsBridge` (§ 5.18.2): a bridge frame restricts the distinct-reference loop to its
    /// `bridge_frame_ref_idx` slot and skips the trailing restricted-frame append.
    pub is_bridge: bool,
    /// `bridge_frame_ref_idx` (§ 5.18.2), only meaningful when `is_bridge`.
    pub bridge_frame_ref_idx: u32,
    /// `FrameWidth` (§ 5.18.4): the current frame width, used by `valid_ref_frame_size` when
    /// `checkRes`.
    pub frame_width: u32,
    /// `FrameHeight` (§ 5.18.4): the current frame height, used by `valid_ref_frame_size`.
    pub frame_height: u32,
    /// `TLayerDependencyMap[ obu_mlayer_id ][ obu_tlayer_id ][ RefTLayerId[i] ]` and
    /// `MLayerDependencyMap[ obu_mlayer_id ][ RefMLayerId[i] ]` collapsed to a single
    /// predicate the caller evaluates per slot. For the single-spatial-layer minimal frame
    /// this is always `1` (a layer depends on itself); the caller is responsible for the
    /// general dependency-map lookup. The closure receives the frame's
    /// `(obu_mlayer_id, obu_tlayer_id)` and the slot's `(ref_mlayer_id, ref_tlayer_id)`, so a
    /// real implementation can index both dependency maps.
    pub layer_dependency: fn(u8, u8, u8, u8) -> bool,
}

/// A stack-resident list of at most `REFS_PER_FRAME` `u32` values, replacing the
/// per-inter-frame `Vec<u32>` that backed the reference map. It holds both
/// `ref_frame_idx[0..NumTotalRefs]` (slot indices, § 7.7) and the parallel
/// `OrderHints[0..NumTotalRefs]` (§ 5.18.2 mirror :4711); each is indexed `0..NumTotalRefs`.
/// `NumTotalRefs <= ActiveNumRefFrames <= REFS_PER_FRAME` always (the explicit path reads
/// `num_total_refs` as `f(3)`, `<= 7`; the implicit § 7.7 path caps at `ActiveNumRefFrames`),
/// so the fixed backing never overflows.
///
/// Derefs to `[u32]`, so every read (`len`, `get`, `first`, iteration, slicing) matches the
/// former `Vec<u32>` surface. `push` and its [`FromIterator`] stop silently at capacity;
/// callers are bounded by construction, so the cap is a total-function guard, not a
/// truncation path.
#[derive(Clone, Default)]
pub struct RefIdxBuf {
    entries: [u32; REFS_PER_FRAME as usize],
    len: u8,
}

impl RefIdxBuf {
    /// Returns the active entries.
    #[must_use]
    pub fn as_slice(&self) -> &[u32] {
        &self.entries[..self.len as usize]
    }

    /// Appends `value` unless the buffer already holds `REFS_PER_FRAME` entries.
    pub(crate) fn push(&mut self, value: u32) {
        if let Some(slot) = self.entries.get_mut(self.len as usize) {
            *slot = value;
            self.len += 1;
        }
    }
}

impl core::ops::Deref for RefIdxBuf {
    type Target = [u32];

    fn deref(&self) -> &[u32] {
        self.as_slice()
    }
}

impl core::fmt::Debug for RefIdxBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for RefIdxBuf {
    fn eq(&self, other: &Self) -> bool {
        self[..] == other[..]
    }
}

impl Eq for RefIdxBuf {}

impl PartialEq<Vec<u32>> for RefIdxBuf {
    fn eq(&self, other: &Vec<u32>) -> bool {
        self[..] == other[..]
    }
}

impl<'a> IntoIterator for &'a RefIdxBuf {
    type Item = &'a u32;
    type IntoIter = core::slice::Iter<'a, u32>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl FromIterator<u32> for RefIdxBuf {
    fn from_iter<I: IntoIterator<Item = u32>>(iter: I) -> Self {
        let mut buf = Self::default();
        for value in iter {
            buf.push(value);
        }
        buf
    }
}

/// The output of `get_ref_frames()` (AV2 v1.0.0 § 7.7): `NumTotalRefs` and the derived
/// `ref_frame_idx[0..NumTotalRefs]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GetRefFrames {
    /// `NumTotalRefs` (§ 7.7 :1636 / :1684): `Min(NRanked, ActiveNumRefFrames)` plus any
    /// appended restricted frames.
    pub num_total_refs: u32,
    /// `ref_frame_idx[i]` for `i in 0..num_total_refs` (§ 7.7 :1637 / :1685): the ranked slot
    /// indices.
    pub ref_frame_idx: RefIdxBuf,
}

/// One ranked reference's score row (`Scores*[]` arrays in § 7.7), kept together so the sort
/// and drop operate on a single vector rather than parallel arrays.
#[derive(Debug, Clone, Copy, Default)]
struct Ranked {
    /// `ScoresIndex[]`: the reference slot index.
    index: u32,
    /// `ScoresScore[]`: the score (lower is better after the sort).
    score: i32,
    /// `ScoresOrderHint[]`: the mapped display order hint `d`.
    order_hint: i32,
    /// `ScoresDistance[]`: `get_relative_dist(OrderHint, d)`.
    distance: i32,
    /// `ScoresBaseQIdx[]`: the mapped base quantizer index `q`.
    base_q_idx: u32,
    /// `ScoresLayer[]`: `RefMLayerId[i]`.
    layer: u8,
}

/// `get_relative_dist( a, b )` (AV2 v1.0.0 § 5.18.3.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3-1`): the signed distance between two
/// order hints, with the `RESTRICTED_OH` sentinel arms.
pub fn get_relative_dist(a: i32, b: i32) -> i32 {
    if a == RESTRICTED_OH && b == RESTRICTED_OH {
        0
    } else if a == RESTRICTED_OH {
        127
    } else if b == RESTRICTED_OH {
        -127
    } else {
        (a - b).clamp(-127, 127)
    }
}

/// `first_slot_with_ref( i )` (AV2 v1.0.0 § 7.7): `true` when slot `i` is the lowest-indexed
/// valid slot holding its distinct decoded frame (deduplicated by `RefCounter`).
fn first_slot_with_ref(slots: &[RefSlot], i: usize) -> bool {
    slots[i].valid
        && !slots[..i]
            .iter()
            .any(|slot| slot.valid && slot.counter == slots[i].counter)
}

/// `valid_ref_frame_size( checkRes, slot )` (AV2 v1.0.0 § 7.7): the resolution-compatibility
/// gate. When `checkRes` is false this is always `true`; otherwise the current frame must be
/// within the spec's scale window of the reference's stored size.
fn valid_ref_frame_size(check_res: bool, frame_w: u32, frame_h: u32, slot: &RefSlot) -> bool {
    if !check_res {
        return true;
    }
    frame_w >= slot.width.div_ceil(2)
        && frame_h >= slot.height.div_ceil(2)
        && frame_w <= slot.width.saturating_mul(16)
        && frame_h <= slot.height.saturating_mul(16)
}

/// `get_ref_frames( checkRes )` (AV2 v1.0.0 § 7.7,
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-7`).
///
/// Derives `NumTotalRefs` and `ref_frame_idx[]` from the saved reference-frame state. The
/// implementation is total and panic-free: every index is bounded by `NUM_REF_FRAMES`, every
/// arithmetic step uses widened integer types, and the result is the ranked slot list the
/// inter header would otherwise need bits to encode.
///
/// `check_res` is the § 7.7 `checkRes` input: the first inter-header call (mirror :4607)
/// passes `false` (resolution unknown — `frame_size()` runs after), the second (mirror :4647)
/// passes `true` once `FrameWidth` / `FrameHeight` are resolved.
pub(crate) fn get_ref_frames(input: &GetRefFramesInput, check_res: bool) -> GetRefFrames {
    let num_ref_frames = (input.num_ref_frames as usize).min(NUM_REF_FRAMES);
    let active_num_ref_frames = REFS_PER_FRAME.min(input.num_ref_frames);

    let mut map_order_hint: [Option<i32>; NUM_REF_FRAMES] = [None; NUM_REF_FRAMES];
    let mut max_disp: i32 = 0;

    for (i, mapped_order_hint) in map_order_hint.iter_mut().enumerate().take(num_ref_frames) {
        let slot = &input.slots[i];
        let allowed = (input.allowed_frames & (1i32 << i)) != 0;
        let bridge_ok = !input.is_bridge || i as u32 == input.bridge_frame_ref_idx;
        let layer_ok = (input.layer_dependency)(
            input.obu_mlayer_id,
            input.obu_tlayer_id,
            slot.mlayer_id,
            slot.tlayer_id,
        );
        if first_slot_with_ref(&input.slots, i)
            && slot.order_hint != RESTRICTED_OH
            && bridge_ok
            && allowed
            && layer_ok
        {
            if valid_ref_frame_size(check_res, input.frame_width, input.frame_height, slot) {
                *mapped_order_hint = Some(slot.order_hint);
            }
            max_disp = max_disp.max(slot.order_hint);
        }
    }

    let mut ranked = [Ranked::default(); NUM_REF_FRAMES];
    let mut ranked_len = 0usize;
    let mut min_q: u32 = 0;
    let mut max_q: u32 = 0;
    for (i, mapped_order_hint) in map_order_hint.iter().enumerate().take(num_ref_frames) {
        let Some(d) = *mapped_order_hint else {
            continue;
        };
        let slot = &input.slots[i];
        let q = slot.base_q_idx;
        let disp_diff = get_relative_dist(input.order_hint, d);
        let t_dist = disp_diff.abs() + i32::from(input.obu_mlayer_id) - i32::from(slot.mlayer_id);
        let mut score = if max_disp > input.order_hint {
            (t_dist << DIST_WEIGHT_BITS) + q as i32
        } else {
            // TODO(spec: AV2-7.7-GET-REF-FRAMES): re-validate the tDist < 0 score against a
            let cap = DECAY_DIST_CAP;
            let lookup_idx = t_dist.clamp(0, cap) as usize;
            DIST_SCORE_LOOKUP[lookup_idx] + (t_dist - cap).max(0) + q as i32
        };
        let area = u64::from(slot.width).saturating_mul(u64::from(slot.height));
        let ref_ratio = floor_log2_u32_from_u64(area) as i32;
        score -= ref_ratio << 5;

        if new_score_or_dist(&ranked[..ranked_len], d, score, slot.mlayer_id) {
            if ranked_len == 0 {
                min_q = q;
                max_q = q;
            } else {
                min_q = min_q.min(q);
                max_q = max_q.max(q);
            }
            ranked[ranked_len] = Ranked {
                index: i as u32,
                score,
                order_hint: d,
                distance: disp_diff,
                base_q_idx: q,
                layer: slot.mlayer_id,
            };
            ranked_len += 1;
        }
    }

    if ranked_len as u32 > REFS_PER_FRAME {
        let q_thresh = (max_q + min_q).div_ceil(2);
        if let Some(unmapped) = get_unmapped_ref(&ranked[..ranked_len], q_thresh) {
            ranked[unmapped].score = 0x7fff_ffff;
        }
    }

    bubble_sort_ref_scores(&mut ranked[..ranked_len]);

    let n_ranked = ranked_len as u32;
    let mut num_total_refs = n_ranked.min(active_num_ref_frames);
    let mut ref_frame_idx: RefIdxBuf = ranked[..ranked_len]
        .iter()
        .take(num_total_refs as usize)
        .map(|r| r.index)
        .collect();

    if check_res && !input.is_bridge {
        for i in 0..num_ref_frames {
            let slot = &input.slots[i];
            let allowed = (input.allowed_frames & (1i32 << i)) != 0;
            let layer_ok = (input.layer_dependency)(
                input.obu_mlayer_id,
                input.obu_tlayer_id,
                slot.mlayer_id,
                slot.tlayer_id,
            );
            if slot.valid
                && slot.order_hint == RESTRICTED_OH
                && layer_ok
                && allowed
                && num_total_refs < active_num_ref_frames
            {
                ref_frame_idx.push(i as u32);
                num_total_refs += 1;
            }
        }
    }

    GetRefFrames {
        num_total_refs,
        ref_frame_idx,
    }
}

/// `new_score_or_dist( d, score, mLayer )` (AV2 v1.0.0 § 7.7): `true` when no already-ranked
/// reference shares the same order hint, score, and modeling layer.
fn new_score_or_dist(ranked: &[Ranked], d: i32, score: i32, m_layer: u8) -> bool {
    !ranked
        .iter()
        .any(|r| r.order_hint == d && r.score == score && r.layer == m_layer)
}

/// `get_unmapped_ref( qThresh )` (AV2 v1.0.0 § 7.7): chooses the index of the reference to
/// drop when more than `REFS_PER_FRAME` were selected, or `None` (`-1`) when none qualifies.
fn get_unmapped_ref(ranked: &[Ranked], q_thresh: u32) -> Option<usize> {
    let mut n_past = 0u32;
    let mut n_future = 0u32;
    let mut max_past_distance = 0i32;
    let mut max_future_distance = 0i32;
    let mut past_idx = 0usize;
    let mut future_idx = 0usize;
    for (i, r) in ranked.iter().enumerate() {
        if r.base_q_idx >= q_thresh {
            let d = r.distance;
            if d > 0 {
                if d > max_past_distance {
                    max_past_distance = d;
                    past_idx = i;
                }
                n_past += 1;
            } else if d < 0 {
                if -d > max_future_distance {
                    max_future_distance = -d;
                    future_idx = i;
                }
                n_future += 1;
            }
        }
    }
    if n_past > n_future {
        return Some(past_idx);
    }
    if n_past < n_future {
        return Some(future_idx);
    }
    if n_past > 0 {
        return Some(if max_past_distance >= max_future_distance {
            past_idx
        } else {
            future_idx
        });
    }
    None
}

/// `bubble_sort_ref_scores()` (AV2 v1.0.0 § 7.7): a stable ascending bubble sort by score,
/// matching the spec's exact comparison (`ScoresScore[j] > ScoresScore[j + 1]`) so the
/// equal-score ordering is the spec's, not a library sort's.
fn bubble_sort_ref_scores(ranked: &mut [Ranked]) {
    for i in (1..ranked.len()).rev() {
        for j in 0..i {
            if ranked[j].score > ranked[j + 1].score {
                ranked.swap(j, j + 1);
            }
        }
    }
}

/// `FloorLog2( value )` over a `u64` area product (§ 7.7 `refRatio`). `FloorLog2(0)` is `0` by
/// the spec convention (the same convention [`super::size::ceil_log2`] uses for its degenerate
/// inputs).
fn floor_log2_u32_from_u64(value: u64) -> u32 {
    if value < 2 { 0 } else { value.ilog2() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial all-layers-depend layer-dependency predicate for the single-spatial-layer
    /// case (a layer always depends on itself; for the minimal frame every ref is layer 0).
    fn all_depend(_frame_mlayer: u8, _frame_tlayer: u8, _ref_mlayer: u8, _ref_tlayer: u8) -> bool {
        true
    }

    fn empty_slot() -> RefSlot {
        RefSlot {
            valid: false,
            order_hint: 0,
            base_q_idx: 0,
            counter: 0,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 0,
            height: 0,
        }
    }

    fn base_input() -> GetRefFramesInput {
        GetRefFramesInput {
            num_ref_frames: 8,
            slots: [empty_slot(); NUM_REF_FRAMES],
            order_hint: 0,
            obu_mlayer_id: 0,
            obu_tlayer_id: 0,
            allowed_frames: -1,
            is_bridge: false,
            bridge_frame_ref_idx: 0,
            frame_width: 64,
            frame_height: 64,
            layer_dependency: all_depend,
        }
    }

    /// AV2 § 7.7 worked example — the minimal `syn-key-inter-64x64` inter frame.
    ///
    /// After the CLK key frame (`refresh_frame_flags == 255`, § 7.23 :14132 `first` rule),
    /// ONLY slot 0 is `RefValid` (slots 1..15 take `(KEY) ? first(0) : 1 == 0`). The key
    /// stored `RefOrderHint[0] == 0`, `RefBaseQIdx[0] == 70` (the fixture's `base_q_idx`),
    /// `RefFrameWidth/Height[0] == 64`. The inter frame has `OrderHint == 1`, single spatial
    /// layer (`obu_mlayer_id == obu_tlayer_id == 0`), `AllowedFrames == -1`.
    ///
    /// EXPECTED (derived by hand from § 7.7): `first_slot_with_ref(0) == 1` (only valid
    /// slot), all others `0` (`!RefValid`). One distinct reference → `mapOrderHint[0] = 0`,
    /// scored once → `NRanked == 1`. `ActiveNumRefFrames == Min(7, 8) == 7`, so
    /// `NumTotalRefs == Min(1, 7) == 1`, `ref_frame_idx == [0]`. checkRes is irrelevant here
    /// (no restricted frame; `valid_ref_frame_size` holds: `2*64 >= 64`, `64 <= 16*64`).
    #[test]
    fn minimal_single_valid_slot_after_key_yields_one_ref() {
        let mut input = base_input();
        input.slots[0] = RefSlot {
            valid: true,
            order_hint: 0,
            base_q_idx: 70,
            counter: 0,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 64,
            height: 64,
        };
        input.order_hint = 1;

        let res0 = get_ref_frames(&input, false);
        assert_eq!(res0.num_total_refs, 1);
        assert_eq!(res0.ref_frame_idx, vec![0]);

        let res1 = get_ref_frames(&input, true);
        assert_eq!(res1.num_total_refs, 1);
        assert_eq!(res1.ref_frame_idx, vec![0]);
    }

    /// AV2 § 7.7 — no valid reference yields an empty map (`NumTotalRefs == 0`).
    #[test]
    fn no_valid_slot_yields_zero_refs() {
        let input = base_input();
        let res = get_ref_frames(&input, false);
        assert_eq!(res.num_total_refs, 0);
        assert!(res.ref_frame_idx.is_empty());
    }

    /// AV2 § 7.7 `first_slot_with_ref` — two valid slots that store the SAME decoded frame
    /// (equal `RefCounter`) collapse to one distinct reference (the lowest-indexed slot).
    ///
    /// Slots 0 and 3 both valid with `RefCounter == 5`; slot 3 is suppressed because slot 0
    /// (valid, same counter) precedes it. Only slot 0 is scored → `NumTotalRefs == 1`,
    /// `ref_frame_idx == [0]`.
    #[test]
    fn equal_ref_counter_collapses_to_first_slot() {
        let mut input = base_input();
        let s = RefSlot {
            valid: true,
            order_hint: 4,
            base_q_idx: 100,
            counter: 5,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 64,
            height: 64,
        };
        input.slots[0] = s;
        input.slots[3] = s;
        input.order_hint = 8;
        let res = get_ref_frames(&input, false);
        assert_eq!(res.num_total_refs, 1);
        assert_eq!(res.ref_frame_idx, vec![0]);
    }

    /// AV2 § 7.7 scoring + `bubble_sort_ref_scores` — two distinct references rank by score.
    ///
    /// Worked by hand with `OrderHint == 10`, both refs in the past (`maxDisp <= OrderHint`,
    /// the `Dist_Score_Lookup` arm), identical dims `64x64` (`refRatio == FloorLog2(4096) ==
    /// 12`, `score -= 12 << 5 == 384` for both), single layer (`tDist == Abs(dispDiff)`):
    ///   slot 0: d = 8, dispDiff = get_relative_dist(10, 8) = 2, tDist = 2,
    ///           score = Dist_Score_Lookup[2] + 0 + q(40) - 384 = 96 + 40 - 384 = -248.
    ///   slot 1: d = 5, dispDiff = 5, tDist = 5,
    ///           score = Dist_Score_Lookup[5] + 0 + q(40) - 384 = 124 + 40 - 384 = -220.
    /// Ascending sort → slot 0 (-248) before slot 1 (-220): ref_frame_idx == [0, 1].
    #[test]
    fn two_distinct_refs_rank_by_score() {
        let mut input = base_input();
        input.order_hint = 10;
        input.slots[0] = RefSlot {
            valid: true,
            order_hint: 8,
            base_q_idx: 40,
            counter: 1,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 64,
            height: 64,
        };
        input.slots[1] = RefSlot {
            valid: true,
            order_hint: 5,
            base_q_idx: 40,
            counter: 2,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 64,
            height: 64,
        };
        let res = get_ref_frames(&input, false);
        assert_eq!(res.num_total_refs, 2);
        assert_eq!(res.ref_frame_idx, vec![0, 1]);
    }

    /// AV2 § 7.7 — `ActiveNumRefFrames == Min(REFS_PER_FRAME, NumRefFrames)` caps the result.
    ///
    /// Eight distinct valid references (NumRefFrames == 8) but `ActiveNumRefFrames == Min(7,
    /// 8) == 7`, so `NumTotalRefs == 7` and only seven slots are kept. (None is dropped by
    /// `get_unmapped_ref`: `NRanked == 8 > 7` triggers the drop, marking one score
    /// `0x7fffffff`, which the sort pushes to the end and the `Min(8, 7)` cut removes.)
    #[test]
    fn active_num_ref_frames_caps_the_result() {
        let mut input = base_input();
        input.order_hint = 100;
        for i in 0..8u32 {
            input.slots[i as usize] = RefSlot {
                valid: true,
                order_hint: (90 - i * 3) as i32,
                base_q_idx: 50 + i,
                counter: i,
                mlayer_id: 0,
                tlayer_id: 0,
                width: 64,
                height: 64,
            };
        }
        let res = get_ref_frames(&input, false);
        assert_eq!(res.num_total_refs, 7);
        assert_eq!(res.ref_frame_idx.len(), 7);
        for idx in &res.ref_frame_idx {
            assert!(*idx < 8);
        }
    }

    /// AV2 § 7.7 — a restricted reference (`RefOrderHint == RESTRICTED_OH`) is excluded from
    /// scoring and, only when `checkRes && !IsBridge`, appended at the end.
    #[test]
    fn restricted_frame_appended_only_with_check_res() {
        let mut input = base_input();
        input.order_hint = 10;
        input.slots[0] = RefSlot {
            valid: true,
            order_hint: 8,
            base_q_idx: 40,
            counter: 1,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 64,
            height: 64,
        };
        input.slots[1] = RefSlot {
            valid: true,
            order_hint: RESTRICTED_OH,
            base_q_idx: 40,
            counter: 2,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 64,
            height: 64,
        };
        let res0 = get_ref_frames(&input, false);
        assert_eq!(res0.num_total_refs, 1);
        assert_eq!(res0.ref_frame_idx, vec![0]);
        let res1 = get_ref_frames(&input, true);
        assert_eq!(res1.num_total_refs, 2);
        assert_eq!(res1.ref_frame_idx, vec![0, 1]);
    }

    /// AV2 § 7.7 `valid_ref_frame_size` — on the `checkRes == 1` call a reference whose
    /// stored size is outside the current frame's scale window is dropped from the scored set.
    ///
    /// A 1280x720 frame against a 64x64 reference: `FrameWidth(1280) <= 16 * 64 == 1024` is
    /// FALSE, so `valid_ref_frame_size` returns 0 and the ref's `mapOrderHint` stays `-1`. On
    /// `checkRes == 0` (size unknown) the same ref is admitted. This is the gate that, for a
    /// resolution-incompatible reference, makes the second `get_ref_frames()` call yield fewer
    /// references than the first.
    #[test]
    fn check_res_drops_resolution_incompatible_reference() {
        let mut input = base_input();
        input.frame_width = 1280;
        input.frame_height = 720;
        input.order_hint = 5;
        input.slots[0] = RefSlot {
            valid: true,
            order_hint: 4,
            base_q_idx: 40,
            counter: 1,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 64,
            height: 64,
        };
        let res0 = get_ref_frames(&input, false);
        assert_eq!(res0.num_total_refs, 1);
        assert_eq!(res0.ref_frame_idx, vec![0]);
        let res1 = get_ref_frames(&input, true);
        assert_eq!(res1.num_total_refs, 0);
        assert!(res1.ref_frame_idx.is_empty());
    }

    /// AV2 § 7.7 — `AllowedFrames` masks out a slot whose bit is clear.
    #[test]
    fn allowed_frames_mask_excludes_slot() {
        let mut input = base_input();
        input.order_hint = 10;
        input.slots[0] = RefSlot {
            valid: true,
            order_hint: 8,
            base_q_idx: 40,
            counter: 1,
            mlayer_id: 0,
            tlayer_id: 0,
            width: 64,
            height: 64,
        };
        input.allowed_frames = !1;
        let res = get_ref_frames(&input, false);
        assert_eq!(res.num_total_refs, 0);
        assert!(res.ref_frame_idx.is_empty());
    }

    /// AV2 § 7.7 / § 5.18.3.1 — `get_relative_dist` clamps and handles the RESTRICTED_OH arms.
    #[test]
    fn get_relative_dist_clamps_and_handles_sentinels() {
        assert_eq!(get_relative_dist(10, 8), 2);
        assert_eq!(get_relative_dist(8, 10), -2);
        assert_eq!(get_relative_dist(200, 0), 127);
        assert_eq!(get_relative_dist(0, 200), -127);
        assert_eq!(get_relative_dist(RESTRICTED_OH, RESTRICTED_OH), 0);
        assert_eq!(get_relative_dist(RESTRICTED_OH, 5), 127);
        assert_eq!(get_relative_dist(5, RESTRICTED_OH), -127);
    }

    /// FloorLog2 over the area product matches the spec convention (FloorLog2(0) == 0).
    #[test]
    fn floor_log2_u64_matches_spec() {
        assert_eq!(floor_log2_u32_from_u64(0), 0);
        assert_eq!(floor_log2_u32_from_u64(1), 0);
        assert_eq!(floor_log2_u32_from_u64(2), 1);
        assert_eq!(floor_log2_u32_from_u64(4096), 12);
        assert_eq!(floor_log2_u32_from_u64(4095), 11);
    }
}
