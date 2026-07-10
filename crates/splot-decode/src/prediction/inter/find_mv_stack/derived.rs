// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{
    CWP_EQUAL, CompoundMvCandidate, CompoundMvStackEntry, MAX_PR_NUM, MAX_REF_MV_STACK_SIZE, Mv,
    MvBlockContext, MvStackEntry, TemporalMvContext, insert_compound_mv_stack_entry,
};

const MAX_DR_STACK_SIZE: usize = 4;
const MAX_DR_PR_NUM: usize = 2;

fn push_bounded_unique<T: Copy + Eq>(entries: &mut Vec<T>, prune_count: &mut usize, candidate: T) {
    if entries.len() >= MAX_DR_STACK_SIZE {
        return;
    }
    if *prune_count < MAX_DR_PR_NUM {
        for entry in entries.iter() {
            *prune_count += 1;
            if *entry == candidate {
                return;
            }
        }
    }
    entries.push(candidate);
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct SingleMvCandidate {
    ref_frame: i8,
    mv: Mv,
}

pub(super) struct CompoundDerivedMvState {
    entries: Vec<[Mv; 2]>,
    singles: Vec<SingleMvCandidate>,
    prune_count: usize,
    single_prune_count: usize,
}

pub(super) struct CompoundScanState {
    pub(super) entries: Vec<CompoundMvStackEntry>,
    pub(super) prune_count: usize,
    pub(super) derived: CompoundDerivedMvState,
}

impl CompoundScanState {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::with_capacity(MAX_REF_MV_STACK_SIZE),
            prune_count: 0,
            derived: CompoundDerivedMvState::new(),
        }
    }
}

impl CompoundDerivedMvState {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::with_capacity(MAX_DR_STACK_SIZE),
            singles: Vec::with_capacity(MAX_DR_STACK_SIZE),
            prune_count: 0,
            single_prune_count: 0,
        }
    }

    pub(super) fn add_spatial(
        &mut self,
        block: &MvBlockContext,
        candidates: [Option<(i8, Mv)>; 2],
    ) {
        let Some(ref_frame1) = block.ref_frame1 else {
            return;
        };
        let target_refs = [block.ref_frame0, ref_frame1];
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
        entries: &mut Vec<CompoundMvStackEntry>,
        max_ref_mv_count: usize,
        prune_count: &mut usize,
    ) {
        for &mvs in &self.entries {
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
    entries: Vec<Mv>,
    prune_count: usize,
}

impl<'a> DerivedMvState<'a> {
    pub(super) fn new(temporal: Option<&'a TemporalMvContext>) -> Self {
        Self {
            temporal,
            entries: Vec::with_capacity(MAX_DR_STACK_SIZE),
            prune_count: 0,
        }
    }

    pub(super) fn add_spatial(
        &mut self,
        block: &MvBlockContext,
        candidate_ref: i8,
        candidate_mv: Mv,
    ) {
        let Some(candidate) = self.temporal.and_then(|temporal| {
            temporal.derive_spatial_mv(
                block.ref_frame0,
                candidate_ref,
                candidate_mv,
                block.mi_row >> 1,
                block.mi_col >> 1,
            )
        }) else {
            return;
        };
        self.push(candidate);
    }

    fn push(&mut self, candidate: Mv) {
        push_bounded_unique(&mut self.entries, &mut self.prune_count, candidate);
    }

    pub(super) fn fill(
        &self,
        entries: &mut Vec<MvStackEntry>,
        max_ref_mv_count: usize,
        prune_count: &mut usize,
    ) {
        for &candidate in &self.entries {
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
            if !duplicate {
                entries.push(MvStackEntry {
                    mv: candidate,
                    weight: 0,
                    offsets: (0, 0),
                });
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
        let mut derived = DerivedMvState::new(None);
        derived.push(first);
        derived.push(second);
        let mut entries = vec![
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
        ];
        let mut prune_count = MAX_PR_NUM;

        derived.fill(&mut entries, 4, &mut prune_count);

        assert_eq!(entries.len(), 4);
        assert_eq!(entries[3].mv, first);
        assert_eq!(entries[3].offsets, (0, 0));
    }

    #[test]
    fn collection_prunes_an_early_duplicate() {
        let candidate = Mv { row: 10, col: 365 };
        let mut derived = DerivedMvState::new(None);

        derived.push(candidate);
        derived.push(candidate);

        assert_eq!(derived.entries, [candidate]);
    }
}
