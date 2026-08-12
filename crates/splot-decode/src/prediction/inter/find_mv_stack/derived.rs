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

pub(super) struct DerivedMvState<'a> {
    temporal: Option<&'a TemporalMvContext>,
    order_hints: Option<OrderHintMvContext<'a>>,
    entries: FixedStack<Mv, MAX_DR_STACK_SIZE>,
    prune_count: usize,
    global_mv: Mv,
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
        }
    }

    pub(super) fn add_spatial(
        &mut self,
        block: &MvBlockContext,
        candidate_ref: i8,
        candidate_mv: Mv,
        cell: NeighbourCell,
    ) {
        if block.ref_frame0 == TIP_REF_FRAME {
            if candidate_ref != cell.flags.ref_frame0 {
                return;
            }
            if let Some(ref_frame1) = cell.flags.ref_frame1
                && let Some(candidate) = self.temporal.and_then(|temporal| {
                    temporal.derive_tip_base_mv(
                        [cell.flags.ref_frame0, ref_frame1],
                        [cell.motion.sub_mv, cell.motion.sub_mv1],
                    )
                })
            {
                self.push(candidate);
            }
            return;
        }
        let candidate = self
            .temporal
            .and_then(|temporal| {
                temporal.derive_spatial_mv(
                    block.ref_frame0,
                    candidate_ref,
                    candidate_mv,
                    block.mi_row >> 1,
                    block.mi_col >> 1,
                )
            })
            .or_else(|| {
                self.order_hints.and_then(|order_hints| {
                    order_hints.derive_spatial_mv(block.ref_frame0, candidate_ref, candidate_mv)
                })
            });
        let Some(candidate) = candidate else {
            return;
        };
        self.push(candidate);
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
        &self,
        entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
        max_ref_mv_count: usize,
        prune_count: &mut usize,
    ) {
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

        derived.fill(&mut entries, 4, &mut prune_count);

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
}
