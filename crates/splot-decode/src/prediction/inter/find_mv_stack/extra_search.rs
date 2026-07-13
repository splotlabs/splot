// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{
    CWP_EQUAL, CompoundMvCandidate, CompoundMvStackEntry, FixedStack, MAX_PR_NUM,
    MAX_REF_MV_STACK_SIZE, Mv, MvBlockContext, MvStackEntry, clamp_mv,
    insert_compound_mv_stack_entry,
};

pub(super) fn extra_search(
    block: &MvBlockContext,
    global_mv: Mv,
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
) {
    for entry in entries.iter_mut() {
        entry.mv = clamp_mv(block, entry.mv);
    }

    if entries.len() < MAX_REF_MV_STACK_SIZE {
        let mut already_present = false;
        if *prune_count < MAX_PR_NUM {
            for entry in entries.iter() {
                *prune_count += 1;
                if entry.mv == global_mv {
                    already_present = true;
                    break;
                }
            }
        }
        if !already_present {
            let _ = entries.try_push(MvStackEntry {
                mv: global_mv,
                weight: 0,
                offsets: (0, 0),
            });
        }
    }

    if block.bw4 > 8 && block.bh4 > 8 {
        let num = entries.len();
        if num > 1 {
            insert_mixture_candidate(entries, prune_count, 0, 1);
            insert_mixture_candidate(entries, prune_count, 1, 0);
        }
        if num > 2 {
            insert_mixture_candidate(entries, prune_count, 0, 2);
            insert_mixture_candidate(entries, prune_count, 2, 0);
            insert_mixture_candidate(entries, prune_count, 1, 2);
            insert_mixture_candidate(entries, prune_count, 2, 1);
        }
    }
}

pub(super) fn compound_extra_search(
    block: &MvBlockContext,
    global_mvs: [Mv; 2],
    entries: &mut FixedStack<CompoundMvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
) {
    for entry in entries.iter_mut() {
        entry.candidate.mvs = entry.candidate.mvs.map(|mv| clamp_mv(block, mv));
    }
    insert_compound_mv_stack_entry(
        entries,
        prune_count,
        CompoundMvCandidate {
            mvs: global_mvs,
            cwp_weight: CWP_EQUAL,
        },
        0,
    );

    if block.bw4 > 8 && block.bh4 > 8 {
        let num = entries.len();
        if num > 1 {
            insert_compound_mixture_candidate(entries, prune_count, 0, 1);
            insert_compound_mixture_candidate(entries, prune_count, 1, 0);
        }
        if num > 2 {
            insert_compound_mixture_candidate(entries, prune_count, 0, 2);
            insert_compound_mixture_candidate(entries, prune_count, 2, 0);
            insert_compound_mixture_candidate(entries, prune_count, 1, 2);
            insert_compound_mixture_candidate(entries, prune_count, 2, 1);
        }
    }
}

fn insert_compound_mixture_candidate(
    entries: &mut FixedStack<CompoundMvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
    y_cand: usize,
    x_cand: usize,
) {
    let (Some(y_entry), Some(x_entry)) =
        (entries.get(y_cand).copied(), entries.get(x_cand).copied())
    else {
        return;
    };
    insert_compound_mv_stack_entry(
        entries,
        prune_count,
        CompoundMvCandidate {
            mvs: [
                Mv {
                    row: y_entry.candidate.mvs[0].row,
                    col: x_entry.candidate.mvs[0].col,
                },
                Mv {
                    row: y_entry.candidate.mvs[1].row,
                    col: x_entry.candidate.mvs[1].col,
                },
            ],
            cwp_weight: CWP_EQUAL,
        },
        0,
    );
}

fn insert_mixture_candidate(
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
    y_cand: usize,
    x_cand: usize,
) {
    let (Some(y_entry), Some(x_entry)) = (entries.get(y_cand), entries.get(x_cand)) else {
        return;
    };
    let candidate = Mv {
        row: y_entry.mv.row,
        col: x_entry.mv.col,
    };
    if entries.len() >= MAX_REF_MV_STACK_SIZE {
        return;
    }
    if *prune_count < MAX_PR_NUM {
        for entry in entries.iter() {
            *prune_count += 1;
            if entry.mv == candidate {
                return;
            }
        }
    }
    let _ = entries.try_push(MvStackEntry {
        mv: candidate,
        weight: 0,
        offsets: (0, 0),
    });
}
