// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Staged mode decision: the inter candidates of a leaf are grouped by
//! class, and a stage keeps a bounded number per class, cheapest first,
//! before the next, more exact stage scores the survivors.

/// The class of an inter candidate; each class has its own budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CandidateClass {
    /// A single-reference or skip-mode prediction from the reference MV
    /// stack without a coded difference (`NEARMV`, `GLOBALMV`, skip mode).
    Mvp,
    /// A single-reference prediction that codes a difference or a model
    /// (`NEWMV`, its BAWP and inter-intra variants, warp).
    Newmv,
    /// A two-reference prediction.
    Compound,
}

/// The limits of one pruning stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StageLimits {
    /// The candidates each class keeps, in [`CandidateClass`] order, lowest
    /// cost first; `u8::MAX` keeps every one.
    pub(crate) budgets: [u8; 3],
    /// Over the classes, the lowest-cost survivor is kept, and the second
    /// only when its cost is within this margin, in 1/16, of the first; the
    /// others are dropped. `u8::MAX` drops none.
    pub(crate) light_margin: u8,
}

impl StageLimits {
    /// Whether the stage keeps every candidate.
    pub(crate) fn keeps_all(self) -> bool {
        self.budgets == [u8::MAX; 3] && self.light_margin == u8::MAX
    }

    /// Whether each entry of `scored` survives the stage: an unscored
    /// (`None`) candidate always survives; ties keep the earlier candidate.
    pub(crate) fn survivors(self, scored: &[Option<(CandidateClass, u64)>]) -> Vec<bool> {
        let mut order: Vec<usize> = (0..scored.len()).collect();
        order.sort_by_key(|&i| scored[i].map(|(_, cost)| cost));
        let mut kept = [0u8; 3];
        let mut out = vec![false; scored.len()];
        for &i in &order {
            out[i] = match scored[i] {
                None => true,
                Some((class, _)) if kept[class as usize] < self.budgets[class as usize] => {
                    kept[class as usize] += 1;
                    true
                }
                Some(_) => false,
            };
        }
        if self.light_margin != u8::MAX {
            let kept: Vec<(usize, u64)> = order
                .iter()
                .filter_map(|&i| scored[i].filter(|_| out[i]).map(|(_, cost)| (i, cost)))
                .collect();
            if let Some(&(_, first)) = kept.first() {
                let bound = u128::from(first) * (16 + u128::from(self.light_margin));
                for (rank, &(i, cost)) in kept.iter().enumerate().skip(1) {
                    out[i] = rank == 1 && u128::from(cost) * 16 <= bound;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::CandidateClass::{Compound, Mvp, Newmv};
    use super::*;

    #[test]
    fn open_limits_keep_everything() {
        let open = StageLimits {
            budgets: [u8::MAX; 3],
            light_margin: u8::MAX,
        };
        let scored = [Some((Mvp, 9)), None, Some((Compound, 1)), Some((Newmv, 5))];
        assert!(open.keeps_all());
        assert_eq!(open.survivors(&scored), [true; 4]);
    }

    #[test]
    fn budgets_keep_the_cheapest_of_each_class_and_ties_keep_the_earlier() {
        let limits = StageLimits {
            budgets: [2, 1, 0],
            light_margin: u8::MAX,
        };
        assert!(!limits.keeps_all());
        let scored = [
            Some((Mvp, 30)),
            Some((Mvp, 10)),
            Some((Newmv, 7)),
            Some((Mvp, 10)),
            Some((Newmv, 3)),
            Some((Compound, 1)),
            None,
            Some((Mvp, 10)),
        ];
        assert_eq!(
            limits.survivors(&scored),
            [false, true, false, true, true, false, true, false]
        );
    }

    #[test]
    fn light_margin_keeps_the_best_and_a_close_second_over_the_classes() {
        let limits = |light_margin| StageLimits {
            budgets: [2, 1, 1],
            light_margin,
        };
        let scored = [
            Some((Mvp, 40)),
            Some((Newmv, 33)),
            None,
            Some((Compound, 32)),
            Some((Mvp, 50)),
        ];
        assert!(!limits(0).keeps_all());
        assert_eq!(
            limits(0).survivors(&scored),
            [false, false, true, true, false]
        );
        assert_eq!(
            limits(1).survivors(&scored),
            [false, true, true, true, false]
        );
        assert_eq!(
            limits(u8::MAX).survivors(&scored),
            [true, true, true, true, true]
        );
    }
}
