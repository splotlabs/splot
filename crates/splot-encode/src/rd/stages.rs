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
}

impl StageLimits {
    /// Whether the stage keeps every candidate.
    pub(crate) fn keeps_all(self) -> bool {
        self.budgets == [u8::MAX; 3]
    }

    /// Whether each entry of `scored` survives the stage: an unscored
    /// (`None`) candidate always survives; ties keep the earlier candidate.
    pub(crate) fn survivors(self, scored: &[Option<(CandidateClass, u64)>]) -> Vec<bool> {
        let mut order: Vec<usize> = (0..scored.len()).collect();
        order.sort_by_key(|&i| scored[i].map(|(_, cost)| cost));
        let mut kept = [0u8; 3];
        let mut out = vec![false; scored.len()];
        for i in order {
            out[i] = match scored[i] {
                None => true,
                Some((class, _)) if kept[class as usize] < self.budgets[class as usize] => {
                    kept[class as usize] += 1;
                    true
                }
                Some(_) => false,
            };
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
        };
        let scored = [Some((Mvp, 9)), None, Some((Compound, 1)), Some((Newmv, 5))];
        assert!(open.keeps_all());
        assert_eq!(open.survivors(&scored), [true; 4]);
    }

    #[test]
    fn budgets_keep_the_cheapest_of_each_class_and_ties_keep_the_earlier() {
        let limits = StageLimits { budgets: [2, 1, 0] };
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
}
