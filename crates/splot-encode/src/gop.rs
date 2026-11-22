// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Reference structure: the coding order, frame types, order hints,
//! refreshed reference slots and show-existing frames of the stream, and the
//! frame header and OBUs each coded frame gives.
//!
//! The first frame, every frame `keyint` frames after the last key frame, and
//! every scene cut the lookahead reports are closed-loop key frames; with the
//! preset's `open_loop_key` every Nth periodic one is an open-loop key frame
//! (§ 7.4.4) instead: the hidden layer-0 frame of the mini-GOP that ends at it,
//! whose other frames are leading frames. The frames
//! between key frames form mini-GOPs of the preset's `mini_gop_size` frames;
//! the last mini-GOP before a key frame or the end of the input can be
//! shorter. A mini-GOP codes its last frame first, then the middle frame of
//! each open span, recursively. A frame coded before frames that precede it in
//! display order is hidden, and a show-existing frame outputs it once those
//! frames are out; every other frame is shown when coded. So size 1 is IPPP,
//! and size 4 codes `4 2 1 [2] 3 [4]` (brackets are show-existing frames).
//!
//! A temporal unit holds the hidden frames coded since the last output and
//! one output: a shown frame or a show-existing frame (§ 7.3.3, § 7.3.4,
//! § 7.3.6). A key frame refreshes every slot and keeps only slot 0 valid
//! (§ 7.23). A frame shown when coded above layer 0 is a non-reference frame
//! (`refresh_frame_flags = 0`), except the last one of its mini-GOP: the
//! § 5.18.2 `OrderHint` derivation needs a shown reference within half the
//! order-hint window, and the other references of a mini-GOP are hidden. The
//! frames coded after a non-reference frame up to the next reference frame
//! read the same references: they form one wave and can code at the same
//! time. Any other frame refreshes one slot: an empty one, else the one
//! with the oldest display index that no show-existing frame waits for,
//! other than the newest frame shown when coded, which the § 5.18.2
//! derivation reads (mini-GOPs of 16 and more fill every slot). The
//! § 7.7 implicit reference map ranks the valid slots by display distance and
//! `base_q_idx`, so at one quantizer the closest frames rank first.
//!
//! An inter frame starts from the saved CDFs of its primary reference
//! (§ 5.18.2), picked among the references whose layer is not above the
//! frame's own: the closest layer, then an inter frame over the key frame,
//! then the closest in display order, then the lower slot.
//!
//! Encoder policy: a hidden frame is shown by a show-existing frame with
//! `derive_sef_order_hint = 1`, or with the preset's `implicit_output` codes
//! `implicit_output_frame` in a temporal unit of its own; with the preset's
//! `sef_repeat` an input frame equal to the frame before it is a temporal
//! unit with a show-existing frame of that frame's slot and an explicit
//! `sef_order_hint`, between mini-GOPs; motion vectors are
//! eighth-pel when the preset refines to eighth samples at the frame's
//! quantizer, else quarter-pel.

use core::num::NonZeroU32;

use crate::error::{Error, Result};
use crate::rate_control::FrameType;

/// `NUM_REF_FRAMES` (§ 3): the most slots a sequence can code.
const MAX_REF_SLOTS: usize = 16;
/// `NumRefFrames` of a sequence with `explicit_num_ref_frames = 0`.
const DEFAULT_REF_SLOTS: usize = 8;
/// Half the order-hint window of the `OrderHintBits = 8` that every encoder
/// sequence codes (§ 5.18.2 `get_disp_order_hint`).
const HALF_ORDER_HINT_WINDOW: u64 = 128;

/// The plan of one coded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GopFrame {
    /// Zero-based coding-order index.
    pub(crate) frame_index: u64,
    /// Zero-based input (display) index.
    pub(crate) display_index: u64,
    /// Frame type.
    pub(crate) frame_type: FrameType,
    /// `OrderHint`: the distance from the last key frame.
    pub(crate) order_hint: u32,
    /// `immediate_output_frame`; a hidden frame waits for a show-existing frame.
    pub(crate) show_frame: bool,
    /// Depth in the mini-GOP hierarchy: `0` for key frames and the last frame
    /// of each mini-GOP, one more per bisection.
    pub(crate) layer: u8,
    /// `refresh_frame_flags`.
    pub(crate) refresh_frame_flags: u32,
    /// The slots that hold a frame when this frame is coded: the frames it may
    /// reference.
    pub(crate) ref_slots: u32,
    /// The slot whose saved CDFs the frame starts from; `None` for key frames.
    pub(crate) primary_slot: Option<u32>,
    /// The frame starts its temporal unit, after the temporal delimiter.
    pub(crate) opens_temporal_unit: bool,
    /// An inter-type frame coded as `INTRA_ONLY_FRAME` (§ 5.18.2): intra
    /// blocks, `PRIMARY_REF_NONE`, and the slots it does not refresh keep
    /// their frames.
    pub(crate) intra_only: bool,
    /// `implicit_output_frame`: a hidden frame the decoder outputs by order
    /// hint, in a temporal unit of its own (§ 7.3.3, § 7.3.6).
    pub(crate) implicit_output: bool,
    /// A key frame coded as `OBU_OPEN_LOOP_KEY` (§ 7.4.4): hidden, it
    /// refreshes one slot and the other slots keep their frames.
    pub(crate) open_loop: bool,
    /// A frame of the mini-GOP of an open-loop key frame that precedes it in
    /// display order, coded with a leading OBU type (§ 7.4.4).
    pub(crate) leading: bool,
    /// A layer-0 inter frame coded as a `SWITCH_FRAME` (§ 5.18.2).
    pub(crate) switch: Option<SwitchKind>,
}

/// The OBU type of a switch frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitchKind {
    /// `OBU_SWITCH` over the default references; it refreshes one slot.
    Switch,
    /// `OBU_RAS_FRAME` (§ 7.4.5) that references only the key frame, by its
    /// long-term id; after it only its own slot and the key frame's stay
    /// valid.
    Ras,
}

/// One step of the coding order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GopStep {
    /// Code a frame.
    Code(GopFrame),
    /// Output the frame in `slot` as `display_index`: a hidden frame with
    /// `derive_sef_order_hint = 1`, or, with `order_hint`, a repeat of a
    /// shown frame with that `sef_order_hint`. It always starts its temporal
    /// unit.
    ShowExisting {
        slot: u32,
        display_index: u64,
        order_hint: Option<u32>,
        /// `OBU_LEADING_SEF`: it outputs a leading frame.
        leading: bool,
    },
}

impl GopStep {
    /// The display index of the frame this step outputs; the output closes
    /// the temporal unit.
    pub(crate) fn output(&self) -> Option<u64> {
        match *self {
            Self::Code(frame) => {
                (frame.show_frame || frame.implicit_output).then_some(frame.display_index)
            }
            Self::ShowExisting { display_index, .. } => Some(display_index),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    display_index: u64,
    layer: u8,
    awaiting_show: bool,
    shown: bool,
}

/// Plans key frames and mini-GOPs over the input frames.
#[derive(Debug, Clone)]
pub(crate) struct GopPlanner {
    keyint: u64,
    mini_gop_size: u64,
    slots: [Option<Slot>; MAX_REF_SLOTS],
    /// `NumRefFrames`: the first slots the planner uses.
    ref_slots: usize,
    next_display: u64,
    next_frame: u64,
    key_display: u64,
    /// Display index of the next key frame `keyint` frames after the last one.
    scheduled_key: u64,
    /// Scene cuts not planned yet, ascending.
    cuts: Vec<u64>,
    after_output: bool,
    /// Display index of the last frame of the mini-GOP being planned.
    last_display: u64,
    /// Every this many mini-GOPs code their first frame intra-only; `0` never.
    intra_only: u64,
    /// Mini-GOPs planned since the start.
    mini_gops: u64,
    /// The next layer-0 inter frame is coded intra-only.
    intra_only_next: bool,
    /// Hidden frames code `implicit_output_frame` in place of a show-existing
    /// frame.
    implicit_output: bool,
    /// Input frames equal to the frame before them are shown again from its
    /// slot in place of being coded.
    sef_repeat: bool,
    /// Repeated input frames not planned yet, ascending.
    repeats: Vec<u64>,
    /// The last repeat [`Self::repeat`] took and the coded frame it repeats.
    last_repeat: Option<(u64, u64)>,
    /// Every this many periodic key frames are open-loop key frames; `0` never.
    open_loop_key: u64,
    /// Periodic key frames planned since the start.
    periodic_keys: u64,
    /// The display index of the open-loop key frame of the mini-GOP being
    /// planned.
    open_loop_display: Option<u64>,
    /// Every this many mini-GOPs code their layer-0 frame as a switch frame;
    /// `0` never.
    switch_frames: u64,
    /// The switch frames are RAS frames, and no frame refreshes the key
    /// frame's slot.
    switch_ras: bool,
    /// The next layer-0 inter frame is a switch frame.
    switch_next: bool,
}

impl GopPlanner {
    /// A planner with key frames every `keyint` frames and mini-GOPs of
    /// `mini_gop_size` frames (`0` counts as `1`).
    pub(crate) fn new(keyint: NonZeroU32, mini_gop_size: u8) -> Self {
        Self {
            keyint: u64::from(keyint.get()),
            mini_gop_size: u64::from(mini_gop_size.max(1)),
            slots: [None; MAX_REF_SLOTS],
            ref_slots: DEFAULT_REF_SLOTS,
            next_display: 0,
            next_frame: 0,
            key_display: 0,
            scheduled_key: 0,
            cuts: Vec::new(),
            after_output: true,
            last_display: 0,
            intra_only: 0,
            mini_gops: 0,
            intra_only_next: false,
            implicit_output: false,
            sef_repeat: false,
            repeats: Vec::new(),
            last_repeat: None,
            open_loop_key: 0,
            periodic_keys: 0,
            open_loop_display: None,
            switch_frames: 0,
            switch_ras: false,
            switch_next: false,
        }
    }

    /// The planner that codes the layer-0 frame of every `every`th mini-GOP,
    /// other than one of an open-loop key frame, as a switch frame: with
    /// `switch_type = 0` an `OBU_SWITCH`, else an `OBU_RAS_FRAME`; `0` never
    /// codes one. Such a mini-GOP codes no intra-only frame.
    pub(crate) fn with_switch_frames(self, every: u8, switch_type: u8) -> Self {
        Self {
            switch_frames: u64::from(every),
            switch_ras: switch_type > 0,
            ..self
        }
    }

    /// The planner that codes every `every`th periodic key frame, `keyint`
    /// frames after the key frame before it, as an open-loop key frame: the
    /// hidden layer-0 frame of the mini-GOP that ends at it, whose other
    /// frames are its leading frames. A scene cut and a key frame with no
    /// frame before it in its mini-GOP stay closed-loop; `0` never codes one.
    pub(crate) fn with_open_loop_key(self, every: u8) -> Self {
        Self {
            open_loop_key: u64::from(every),
            ..self
        }
    }

    /// Whether the next scheduled key frame is an open-loop key frame: a
    /// periodic one that no scene cut comes before or at.
    fn open_loop_next(&self) -> bool {
        self.open_loop_key > 0
            && self.scheduled_key > 0
            && self
                .cuts
                .first()
                .is_none_or(|&cut| cut > self.scheduled_key)
            && (self.periodic_keys + 1).is_multiple_of(self.open_loop_key)
    }

    /// The planner that shows input frames equal to the frame before them
    /// again (see [`Self::repeat`]).
    pub(crate) fn with_sef_repeat(self, sef_repeat: bool) -> Self {
        Self { sef_repeat, ..self }
    }

    /// Whether the planner takes repeats.
    pub(crate) fn sef_repeat(&self) -> bool {
        self.sef_repeat
    }

    /// Shows input frame `display_index`, equal to the frame before it, with
    /// a show-existing frame of the slot that holds that frame, unless it is
    /// a key frame. A run of repeats stops so that the next coded frame stays
    /// within half the order-hint window of the frame it repeats (§ 5.18.2
    /// `get_disp_order_hint`); the frame after the run is coded. Repeats come
    /// in display order.
    pub(crate) fn repeat(&mut self, display_index: u64) {
        let Some(previous) = display_index.checked_sub(1) else {
            return;
        };
        let source = match self.last_repeat {
            Some((repeat, source)) if repeat == previous => source,
            _ => previous,
        };
        if self.sef_repeat
            && display_index >= self.next_display
            && display_index - source + self.mini_gop_size <= HALF_ORDER_HINT_WINDOW
        {
            self.repeats.push(display_index);
            self.last_repeat = Some((display_index, source));
        }
    }

    /// The planner that codes the first frame of every `every`th mini-GOP as
    /// an intra-only frame; `0` never does.
    pub(crate) fn with_intra_only(self, every: u8) -> Self {
        Self {
            intra_only: u64::from(every),
            ..self
        }
    }

    /// The planner whose hidden frames code `implicit_output_frame`.
    pub(crate) fn with_implicit_output(self, implicit_output: bool) -> Self {
        Self {
            implicit_output,
            ..self
        }
    }

    /// The planner over `NumRefFrames = slots`, `1..=16`.
    pub(crate) fn with_ref_slots(self, slots: u8) -> Self {
        Self {
            ref_slots: usize::from(slots).clamp(1, MAX_REF_SLOTS),
            ..self
        }
    }

    /// The display index of the first frame the next [`Self::plan`] takes.
    pub(crate) fn next_display(&self) -> u64 {
        self.next_display
    }

    /// Makes the scene cut `display_index` a key frame, unless it is already
    /// planned or it is less than `mini_gop_size` frames after the key frame
    /// before it. Cuts come in display order.
    pub(crate) fn force_key(&mut self, display_index: u64) {
        let last = self
            .cuts
            .last()
            .map_or(self.key_display, |&cut| cut.max(self.key_display));
        let last = display_index.saturating_sub(last) / self.keyint * self.keyint + last;
        if display_index >= self.next_display
            && display_index.saturating_sub(last) >= self.mini_gop_size
            && !self.cuts.contains(&display_index)
        {
            self.cuts.push(display_index);
            self.cuts.sort_unstable();
        }
    }

    /// The display index of the next key frame.
    pub(crate) fn next_key(&self) -> u64 {
        self.cuts
            .first()
            .map_or(self.scheduled_key, |&cut| cut.min(self.scheduled_key))
    }

    /// The number of frames the next [`Self::plan`] takes from `queued` input
    /// frames: a key frame alone, else a mini-GOP that stops before the next
    /// key frame. `None` while too few frames are queued, unless the input has
    /// `ended`.
    pub(crate) fn next_len(&self, queued: usize, ended: bool) -> Option<usize> {
        let to_key = self.next_key().saturating_sub(self.next_display);
        let to_repeat = self
            .repeats
            .iter()
            .find(|&&repeat| repeat >= self.next_display)
            .map_or(u64::MAX, |&repeat| repeat - self.next_display);
        let to_end = if self.open_loop_next() && to_key > 0 {
            to_key + 1
        } else {
            to_key
        };
        let wanted = if to_key == 0 || to_repeat == 0 {
            1
        } else {
            to_end.min(to_repeat).min(self.mini_gop_size)
        };
        let wanted = usize::try_from(wanted).unwrap_or(usize::MAX);
        (queued > 0 && (queued >= wanted || ended)).then_some(queued.min(wanted))
    }

    /// Plans the next `len` input frames, as [`Self::next_len`] returned.
    /// Display index 0 is a key frame, so a mini-GOP always follows a coded frame.
    ///
    /// # Errors
    /// Returns [`Error::Unsupported`] when a frame finds no slot to refresh:
    /// every slot holds a hidden frame that is not shown yet.
    pub(crate) fn plan(&mut self, len: usize) -> Result<Vec<GopStep>> {
        let mut steps = Vec::new();
        if len == 0 {
            return Ok(steps);
        }
        let first = self.next_display;
        let last = first + len as u64 - 1;
        let mut anchor = first.saturating_sub(1);
        self.repeats.retain(|&repeat| repeat >= first);
        if first != self.next_key() && self.repeats.first() == Some(&first) {
            self.repeats.remove(0);
            let slot = (0..self.ref_slots)
                .filter_map(|slot| self.slots[slot].map(|held| (held.display_index, slot as u32)))
                .max()
                .map(|(_, slot)| slot)
                .ok_or(Error::Unsupported {
                    what: "a repeat without a coded frame",
                })?;
            steps.push(GopStep::ShowExisting {
                slot,
                display_index: first,
                order_hint: Some((first - self.key_display) as u32),
                leading: false,
            });
            self.after_output = true;
            self.next_display = first + 1;
            return Ok(steps);
        }
        if first == self.next_key() {
            if first == self.scheduled_key && self.scheduled_key > 0 {
                self.periodic_keys += 1;
            }
            self.cuts.retain(|&cut| cut > first);
            self.scheduled_key = first + self.keyint;
            steps.push(self.code(first, FrameType::Key, true, 0)?);
            anchor = first;
        } else if last == self.scheduled_key && self.open_loop_next() {
            self.periodic_keys += 1;
            self.cuts.retain(|&cut| cut > last);
            self.scheduled_key = last + self.keyint;
            self.open_loop_display = Some(last);
        }
        if last > anchor {
            self.mini_gops += 1;
            self.switch_next = self.open_loop_display.is_none()
                && self.switch_frames > 0
                && self.mini_gops.is_multiple_of(self.switch_frames);
            self.intra_only_next = self.open_loop_display.is_none()
                && !self.switch_next
                && self.intra_only > 0
                && self.mini_gops.is_multiple_of(self.intra_only);
            self.last_display = last;
            self.place(anchor, last, 0, &mut steps)?;
        }
        if self.open_loop_display.take().is_some() {
            for held in &mut self.slots {
                if held.is_some_and(|held| held.display_index != last) {
                    *held = None;
                }
            }
        }
        self.next_display = last + 1;
        Ok(steps)
    }

    /// Codes `hi` at `layer` and the frames between the coded frame `lo` and `hi`.
    fn place(&mut self, lo: u64, hi: u64, layer: u8, steps: &mut Vec<GopStep>) -> Result<()> {
        if hi - lo == 1 {
            steps.push(self.code(hi, FrameType::Inter, true, layer)?);
            return Ok(());
        }
        let frame_type = if self.open_loop_display == Some(hi) {
            FrameType::Key
        } else {
            FrameType::Inter
        };
        steps.push(self.code(hi, frame_type, false, layer)?);
        self.fill(lo, hi, layer + 1, steps)?;
        let (slot, held) = self
            .slots
            .iter_mut()
            .enumerate()
            .find_map(|(slot, held)| {
                held.as_mut()
                    .filter(|held| held.display_index == hi && held.awaiting_show)
                    .map(|held| (slot as u32, held))
            })
            .ok_or(Error::Unsupported {
                what: "a show-existing frame of an evicted frame",
            })?;
        held.awaiting_show = false;
        if !self.implicit_output {
            steps.push(GopStep::ShowExisting {
                slot,
                display_index: hi,
                order_hint: None,
                leading: self.leading(hi),
            });
        }
        self.after_output = true;
        Ok(())
    }

    /// Codes the frames between the coded frames `lo` and `hi` from `layer` on.
    fn fill(&mut self, lo: u64, hi: u64, layer: u8, steps: &mut Vec<GopStep>) -> Result<()> {
        if hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            self.place(lo, mid, layer, steps)?;
            self.fill(mid, hi, layer + 1, steps)?;
        }
        Ok(())
    }

    fn code(
        &mut self,
        display_index: u64,
        frame_type: FrameType,
        show_frame: bool,
        layer: u8,
    ) -> Result<GopStep> {
        let open_loop = frame_type == FrameType::Key && !show_frame;
        let switch = (frame_type == FrameType::Inter && layer == 0)
            .then(|| core::mem::take(&mut self.switch_next))
            .filter(|&next| next)
            .map(|_| {
                if self.switch_ras {
                    SwitchKind::Ras
                } else {
                    SwitchKind::Switch
                }
            });
        if switch == Some(SwitchKind::Ras) {
            return self.code_ras(display_index, show_frame);
        }
        let (slot, refresh_frame_flags) = if open_loop {
            let slot = self.refresh_slot(false)?;
            (Some(slot), 1 << slot)
        } else if frame_type == FrameType::Key {
            self.slots = [None; MAX_REF_SLOTS];
            self.key_display = display_index;
            (Some(0), (1 << self.ref_slots) - 1)
        } else if show_frame && layer > 0 && display_index + 1 != self.last_display {
            (None, 0)
        } else {
            let slot = self.refresh_slot(show_frame)?;
            (Some(slot), 1 << slot)
        };
        let ref_slots = (0..self.ref_slots)
            .filter(|&slot| self.slots[slot].is_some())
            .fold(0, |mask, slot| mask | 1 << slot);
        let primary_slot = (0..self.ref_slots)
            .filter_map(|slot| {
                self.slots[slot]
                    .filter(|held| held.layer <= layer)
                    .map(|held| {
                        let distance = held.display_index.abs_diff(display_index);
                        let key = held.display_index == self.key_display;
                        ((layer - held.layer, key, distance), slot as u32)
                    })
            })
            .min()
            .map(|(_, slot)| slot);
        let intra_only = frame_type == FrameType::Inter
            && layer == 0
            && core::mem::take(&mut self.intra_only_next);
        let primary_slot = primary_slot.filter(|_| !intra_only && !open_loop && switch.is_none());
        if let Some(slot) = slot {
            self.slots[slot] = Some(Slot {
                display_index,
                layer,
                awaiting_show: !show_frame,
                shown: show_frame,
            });
        }
        let frame = GopFrame {
            frame_index: self.next_frame,
            display_index,
            frame_type,
            order_hint: (display_index - self.key_display) as u32,
            show_frame,
            layer,
            refresh_frame_flags,
            ref_slots,
            primary_slot,
            opens_temporal_unit: self.after_output,
            intra_only,
            implicit_output: self.implicit_output && !show_frame,
            open_loop,
            leading: self.leading(display_index),
            switch,
        };
        self.next_frame += 1;
        self.after_output = show_frame || self.implicit_output;
        Ok(GopStep::Code(frame))
    }

    /// Codes `display_index` as a RAS frame (§ 7.4.5) that references the
    /// key frame alone: it refreshes every other slot, and only the first one
    /// it refreshes stays valid (§ 7.23).
    fn code_ras(&mut self, display_index: u64, show_frame: bool) -> Result<GopStep> {
        let key_slot = self.key_slot().ok_or(Error::Unsupported {
            what: "a RAS frame without its key frame",
        })?;
        let own = (0..self.ref_slots)
            .find(|&slot| slot != key_slot)
            .ok_or(Error::Unsupported {
                what: "a RAS frame without a slot of its own",
            })?;
        let all = (1u32 << self.ref_slots) - 1;
        for (slot, held) in self.slots.iter_mut().enumerate() {
            if slot != key_slot {
                *held = None;
            }
        }
        self.slots[own] = Some(Slot {
            display_index,
            layer: 0,
            awaiting_show: !show_frame,
            shown: show_frame,
        });
        let frame = GopFrame {
            frame_index: self.next_frame,
            display_index,
            frame_type: FrameType::Inter,
            order_hint: (display_index - self.key_display) as u32,
            show_frame,
            layer: 0,
            refresh_frame_flags: all & !(1 << key_slot),
            ref_slots: 1 << key_slot,
            primary_slot: None,
            opens_temporal_unit: self.after_output,
            intra_only: false,
            implicit_output: self.implicit_output && !show_frame,
            open_loop: false,
            leading: false,
            switch: Some(SwitchKind::Ras),
        };
        self.next_frame += 1;
        self.after_output = show_frame || self.implicit_output;
        Ok(GopStep::Code(frame))
    }

    /// The slot of the last key frame, while it holds it.
    fn key_slot(&self) -> Option<usize> {
        (0..self.ref_slots).find(|&slot| {
            self.slots[slot].is_some_and(|held| held.display_index == self.key_display)
        })
    }

    /// Whether `display_index` precedes the open-loop key frame being planned.
    fn leading(&self, display_index: u64) -> bool {
        self.open_loop_display
            .is_some_and(|key| display_index < key)
    }

    /// An empty slot, else the slot of the oldest frame no show-existing
    /// frame waits for, other than the newest frame shown when coded; a shown
    /// frame, which becomes the newest shown frame, may take that one last.
    fn refresh_slot(&self, show_frame: bool) -> Result<usize> {
        if let Some(empty) = self.slots[..self.ref_slots]
            .iter()
            .position(Option::is_none)
        {
            return Ok(empty);
        }
        let newest_shown = self
            .slots
            .iter()
            .flatten()
            .filter(|held| held.shown)
            .map(|held| held.display_index)
            .max();
        let pinned = self
            .key_slot()
            .filter(|_| self.switch_ras && self.switch_frames > 0);
        (0..self.ref_slots)
            .filter(|&slot| Some(slot) != pinned)
            .filter_map(|slot| {
                self.slots[slot]
                    .filter(|held| !held.awaiting_show)
                    .map(|held| {
                        (
                            Some(held.display_index) == newest_shown,
                            held.display_index,
                            slot,
                        )
                    })
            })
            .filter(|&(newest, ..)| show_frame || !newest)
            .min()
            .map(|(.., slot)| slot)
            .ok_or(Error::Unsupported {
                what: "more hidden frames than reference slots",
            })
    }
}

/// Splits `steps` into waves: a wave ends after a frame that refreshes a slot.
/// A show-existing frame does not end a wave: it changes only the output flag
/// of its slot and the frame counter of the § 7.20 buffer, and coding a frame
/// reads neither.
pub(crate) fn waves(steps: &[GopStep]) -> impl Iterator<Item = &[GopStep]> {
    steps.split_inclusive(
        |step| matches!(step, GopStep::Code(frame) if frame.refresh_frame_flags != 0),
    )
}

/// Frame `frame_index` of an IPPP stream with key frames every `keyint` frames.
#[cfg(test)]
pub(crate) fn plan(frame_index: u64, keyint: NonZeroU32) -> GopFrame {
    let mut planner = GopPlanner::new(keyint, 1);
    let mut frame = None;
    for _ in 0..=frame_index {
        if let Ok([GopStep::Code(coded)]) = planner.plan(1).as_deref() {
            frame = Some(*coded);
        }
    }
    frame.unwrap_or_else(|| unreachable!("IPPP codes every input frame"))
}

mod header;

pub(crate) use header::{
    FrameTools, TipOutputChoice, coded_grain_seed, frame_tools, show_existing_unit,
    shown_grain_seed,
};

#[cfg(test)]
#[path = "gop_tests.rs"]
mod tests;
