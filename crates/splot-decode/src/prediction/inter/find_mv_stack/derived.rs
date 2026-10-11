// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{
    CWP_EQUAL, CompoundMvCandidate, CompoundMvStackEntry, FixedStack, MAX_PR_NUM,
    MAX_REF_MV_STACK_SIZE, Mv, MvBlockContext, MvStackEntry, NeighbourCell, OrderHintMvContext,
    TIP_REF_FRAME, TemporalMvContext, insert_compound_mv_stack_entry,
};

const MAX_DR_STACK_SIZE: usize = 4;
const MAX_DR_PR_NUM: usize = 2;

fn push_bounded_unique<T: Eq>(
    entries: &mut FixedStack<T, MAX_DR_STACK_SIZE>,
    prune_count: &mut usize,
    candidate: T,
) {
    if *prune_count < MAX_DR_PR_NUM {
        for entry in entries.iter() {
            *prune_count += 1;
            if entry == &candidate {
                return;
            }
        }
    }
    if entries.len() < MAX_DR_STACK_SIZE {
        let _ = entries.try_push(candidate);
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
struct SingleMvCandidate {
    ref_frame: i8,
    mv: Mv,
}

pub(super) struct CompoundDerivedMvState {
    entries: FixedStack<[Mv; 2], MAX_DR_STACK_SIZE>,
    singles: FixedStack<SingleMvCandidate, MAX_DR_STACK_SIZE>,
    prune_count: usize,
    single_prune_count: usize,
}

pub(super) struct CompoundScanState {
    pub(super) entries: FixedStack<CompoundMvStackEntry, MAX_REF_MV_STACK_SIZE>,
    pub(super) prune_count: usize,
    pub(super) derived: CompoundDerivedMvState,
}

impl CompoundScanState {
    pub(super) fn new() -> Self {
        Self {
            entries: FixedStack::new(),
            prune_count: 0,
            derived: CompoundDerivedMvState::new(),
        }
    }
}

impl CompoundDerivedMvState {
    pub(super) fn new() -> Self {
        Self {
            entries: FixedStack::new(),
            singles: FixedStack::new(),
            prune_count: 0,
            single_prune_count: 0,
        }
    }

    pub(super) fn add_spatial(
        &mut self,
        block: &MvBlockContext,
        candidates: [Option<(i8, Mv)>; 2],
        temporal: Option<&TemporalMvContext>,
    ) {
        let Some(ref_frame1) = block.ref_frame1 else {
            return;
        };
        let target_refs = [block.ref_frame0, ref_frame1];
        if target_refs[0] != target_refs[1] {
            for &(candidate_ref, candidate_mv) in candidates.iter().flatten() {
                if candidate_ref < 0 || candidate_ref == TIP_REF_FRAME {
                    continue;
                }
                let Some(derived) = temporal.and_then(|temporal| {
                    temporal.derive_compound_spatial_mvs(
                        target_refs,
                        candidate_ref,
                        candidate_mv,
                        block.mi_row >> 1,
                        block.mi_col >> 1,
                    )
                }) else {
                    continue;
                };
                push_bounded_unique(&mut self.entries, &mut self.prune_count, derived);
            }
        }
        let target = if candidates
            .iter()
            .flatten()
            .any(|(r, _)| *r == target_refs[0])
        {
            0
        } else if candidates
            .iter()
            .flatten()
            .any(|(r, _)| *r == target_refs[1])
        {
            1
        } else {
            return;
        };
        let Some((_, candidate_mv)) = candidates
            .iter()
            .flatten()
            .find(|(r, _)| *r == target_refs[target])
        else {
            return;
        };
        let other = 1 - target;
        if let Some(single) = self
            .singles
            .iter()
            .find(|single| single.ref_frame == target_refs[other])
        {
            let mut pair = [single.mv; 2];
            pair[target] = *candidate_mv;
            push_bounded_unique(&mut self.entries, &mut self.prune_count, pair);
        }
        push_bounded_unique(
            &mut self.singles,
            &mut self.single_prune_count,
            SingleMvCandidate {
                ref_frame: target_refs[target],
                mv: *candidate_mv,
            },
        );
    }

    pub(super) fn fill(
        &self,
        entries: &mut FixedStack<CompoundMvStackEntry, MAX_REF_MV_STACK_SIZE>,
        max_ref_mv_count: usize,
        prune_count: &mut usize,
    ) {
        for &mvs in self.entries.iter() {
            if entries.len() >= max_ref_mv_count {
                return;
            }
            insert_compound_mv_stack_entry(
                entries,
                prune_count,
                CompoundMvCandidate {
                    mvs,
                    cwp_weight: CWP_EQUAL,
                },
                0,
            );
        }
    }
}

/// Inputs of one derived candidate, kept until [`DerivedMvState::fill`]
/// shows that the stack still has room for it.
#[derive(Clone, Copy)]
struct DerivedSource {
    refs: [i8; 2],
    mvs: [Mv; 2],
}

const PENDING_SOURCES: usize = 8;

pub(super) struct DerivedMvState<'a> {
    temporal: Option<&'a TemporalMvContext>,
    order_hints: Option<OrderHintMvContext<'a>>,
    entries: FixedStack<Mv, MAX_DR_STACK_SIZE>,
    prune_count: usize,
    global_mv: Mv,
    pending: [DerivedSource; PENDING_SOURCES],
    pending_len: usize,
}

impl<'a> DerivedMvState<'a> {
    pub(super) fn new(
        temporal: Option<&'a TemporalMvContext>,
        order_hints: Option<OrderHintMvContext<'a>>,
        global_mv: Mv,
    ) -> Self {
        Self {
            temporal,
            order_hints,
            entries: FixedStack::new(),
            prune_count: 0,
            global_mv,
            pending: [DerivedSource {
                refs: [0; 2],
                mvs: [Mv::ZERO; 2],
            }; PENDING_SOURCES],
            pending_len: 0,
        }
    }

    /// Records a candidate on another reference; [`Self::fill`] derives it
    /// only when the stack still has room, which most blocks never reach.
    #[inline]
    pub(super) fn add_spatial(
        &mut self,
        block: &MvBlockContext,
        candidate_ref: i8,
        candidate_mv: Mv,
        cell: &NeighbourCell,
    ) {
        let source = if block.ref_frame0 == TIP_REF_FRAME {
            let Some(ref_frame1) = cell.flags.ref_frame1 else {
                return;
            };
            if candidate_ref != cell.flags.ref_frame0 || self.temporal.is_none() {
                return;
            }
            DerivedSource {
                refs: [cell.flags.ref_frame0, ref_frame1],
                mvs: [cell.motion.sub_mv, cell.motion.sub_mv1],
            }
        } else {
            if self.temporal.is_none() && self.order_hints.is_none() {
                return;
            }
            DerivedSource {
                refs: [candidate_ref, 0],
                mvs: [candidate_mv, Mv::ZERO],
            }
        };
        if self.pending_len == PENDING_SOURCES {
            self.derive_pending(block);
        }
        self.pending[self.pending_len] = source;
        self.pending_len += 1;
    }

    /// Derives the recorded candidates in order, as an eager derivation
    /// would have; a full derived stack ignores the rest.
    fn derive_pending(&mut self, block: &MvBlockContext) {
        let pending = core::mem::take(&mut self.pending_len).min(PENDING_SOURCES);
        for index in 0..pending {
            let source = self.pending[index];
            if self.entries.len() == MAX_DR_STACK_SIZE {
                return;
            }
            let candidate = if block.ref_frame0 == TIP_REF_FRAME {
                self.temporal
                    .and_then(|temporal| temporal.derive_tip_base_mv(source.refs, source.mvs))
            } else {
                self.temporal
                    .and_then(|temporal| {
                        temporal.derive_spatial_mv(
                            block.ref_frame0,
                            source.refs[0],
                            source.mvs[0],
                            block.mi_row >> 1,
                            block.mi_col >> 1,
                        )
                    })
                    .or_else(|| {
                        self.order_hints.and_then(|order_hints| {
                            order_hints.derive_spatial_mv(
                                block.ref_frame0,
                                source.refs[0],
                                source.mvs[0],
                            )
                        })
                    })
            };
            if let Some(candidate) = candidate {
                self.push(candidate);
            }
        }
    }

    pub(super) const fn temporal(&self) -> Option<&'a TemporalMvContext> {
        self.temporal
    }

    pub(super) const fn global_mv(&self) -> Mv {
        self.global_mv
    }

    fn push(&mut self, candidate: Mv) {
        push_bounded_unique(&mut self.entries, &mut self.prune_count, candidate);
    }

    pub(super) fn fill(
        &mut self,
        block: &MvBlockContext,
        entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
        max_ref_mv_count: usize,
        prune_count: &mut usize,
    ) {
        if entries.len() >= max_ref_mv_count {
            return;
        }
        self.derive_pending(block);
        for &candidate in self.entries.iter() {
            if entries.len() >= max_ref_mv_count {
                return;
            }
            let mut duplicate = false;
            if *prune_count < MAX_PR_NUM {
                for entry in entries.iter() {
                    *prune_count += 1;
                    if entry.mv == candidate {
                        duplicate = true;
                        break;
                    }
                }
            }
            if !duplicate
                && !entries.try_push(MvStackEntry {
                    mv: candidate,
                    weight: 0,
                    offsets: (0, 0),
                })
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_preserves_order_and_honors_the_drl_limit() {
        let first = Mv { row: 10, col: 365 };
        let second = Mv { row: -6, col: 233 };
        let mut derived = DerivedMvState::new(None, None, Mv::ZERO);
        derived.push(first);
        derived.push(second);
        let mut entries = FixedStack::from_entries([
            MvStackEntry {
                mv: Mv { row: 17, col: -10 },
                weight: 1,
                offsets: (0, 0),
            },
            MvStackEntry {
                mv: Mv { row: 17, col: -8 },
                weight: 1,
                offsets: (0, 0),
            },
            MvStackEntry {
                mv: Mv { row: 7, col: 315 },
                weight: 1,
                offsets: (0, 0),
            },
        ]);
        let mut prune_count = MAX_PR_NUM;

        let block = MvBlockContext {
            mi_row: 0,
            mi_col: 0,
            bw4: 4,
            bh4: 4,
            sb_h4: 16,
            ref_frame0: 0,
            ref_frame1: None,
            mi_rows: 16,
            mi_cols: 16,
        };
        derived.fill(&block, &mut entries, 4, &mut prune_count);

        assert_eq!(entries.len(), 4);
        assert_eq!(entries[3].mv, first);
        assert_eq!(entries[3].offsets, (0, 0));
    }

    #[test]
    fn collection_prunes_an_early_duplicate() {
        let candidate = Mv { row: 10, col: 365 };
        let mut derived = DerivedMvState::new(None, None, Mv::ZERO);

        derived.push(candidate);
        derived.push(candidate);

        assert_eq!(&derived.entries[..], [candidate]);
    }

    #[test]
    fn derived_mv_storage_keeps_the_first_four_candidates() {
        let mut derived = DerivedMvState::new(None, None, Mv::ZERO);

        for col in 0..5 {
            derived.push(Mv { row: 0, col });
        }

        assert_eq!(derived.entries.len(), MAX_DR_STACK_SIZE);
        for (index, entry) in derived.entries.iter().enumerate() {
            assert_eq!(entry.col, index as i32);
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn deferred_derivation_matches_eager_past_the_pending_capacity() {
        use super::super::temporal::TipReferencePair;
        use super::super::{BlockPrecisionRecord, NeighbourMvGrid};

        let references = TipReferencePair {
            past_ref: 0,
            future_ref: 1,
            past_offset: -1,
            future_offset: 1,
            ref_offset: 1,
        };
        let mut temporal =
            TemporalMvContext::with_tip_sample(16, 16, references, 0, 0, Mv::ZERO).unwrap();
        temporal
            .set_trajectory_sample(0, 0, 0, Mv { row: 5, col: -3 })
            .unwrap();
        temporal
            .set_trajectory_sample(1, 0, 0, Mv { row: -2, col: 7 })
            .unwrap();
        let mut grid = NeighbourMvGrid::new(16, 16).unwrap();
        grid.record_block(
            0,
            0,
            4,
            4,
            true,
            1,
            None,
            false,
            Mv::ZERO,
            false,
            0,
            false,
            BlockPrecisionRecord::default(),
        );
        let cell = grid.get(0, 0).unwrap();
        let block = MvBlockContext {
            mi_row: 0,
            mi_col: 0,
            bw4: 4,
            bh4: 4,
            sb_h4: 16,
            ref_frame0: 0,
            ref_frame1: None,
            mi_rows: 16,
            mi_cols: 16,
        };
        let mut deferred = DerivedMvState::new(Some(&temporal), None, Mv::ZERO);
        let mut eager = DerivedMvState::new(Some(&temporal), None, Mv::ZERO);
        let mvs = (0..PENDING_SOURCES as i32 + 3).map(|i| Mv {
            row: 1,
            col: (i - 1).max(0),
        });
        for mv in mvs {
            deferred.add_spatial(&block, 1, mv, &cell);
            eager.add_spatial(&block, 1, mv, &cell);
            eager.derive_pending(&block);
        }
        deferred.derive_pending(&block);

        assert_eq!(eager.entries.len(), MAX_DR_STACK_SIZE);
        assert_eq!(&deferred.entries[..], &eager.entries[..]);
    }
}
