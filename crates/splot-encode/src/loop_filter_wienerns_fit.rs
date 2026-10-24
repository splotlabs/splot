// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Wiener-NS bank fits: least-squares solutions of the normal equations per
//! coefficient subset, rounded, clipped and refined on the predicted squared
//! error, priced with the exact bank bits.

use splot_core::headers::frame::{
    CoreSeqRestorationView, CoreSeqView, WienerNsCoeffs, WienerNsFrameFilterBank,
    WienerNsFrameFilterClass, wiener_ns_coeff_range, wiener_ns_tap_present,
    write_frame_wiener_ns_filter,
};
use splot_core::write::BitWriter;
use splot_decode::encoder_syntax::{pc_wiener_filter_set_index, pc_wiener_subclass_table};

use super::super::ccso::rd;
use super::super::{Search, coding};
use super::{CHROMA_SHORT, PREC};
use crate::error::{Error, Result};
use crate::kernels::wiener::WienerStats;
use crate::rd::Bits;

impl Search<'_> {
    /// The one-class bank of `plane` that minimizes the predicted squared
    /// error of `stats` plus its bits, and that cost, or `None` when no filter
    /// lowers the cost.
    pub(super) fn fit(
        &self,
        stats: &WienerStats,
        plane: usize,
    ) -> Result<Option<(i128, WienerNsFrameFilterBank)>> {
        let view = self.restoration_view()?;
        let Some((_, class)) = self.best_class(stats, plane, view)? else {
            return Ok(None);
        };
        let bank = bank_of(&[class])?;
        let cost = rd(
            predicted_delta(stats, &class.coeffs),
            Bits::whole(bank_bits(&bank, plane, view)?),
            self.lambda,
        );
        Ok((cost < 0).then_some((cost, bank)))
    }

    /// The luma bank of the class count of `counts` whose classes, each fitted
    /// on the § 7.20.4 PC-Wiener classes `SubclassLookup` merges into it,
    /// minimize the predicted squared error of `stats` (one per PC-Wiener
    /// class) plus the bank bits, and that cost.
    pub(super) fn fit_classes(
        &self,
        stats: &[WienerStats],
        counts: &[u8],
    ) -> Result<Option<(i128, WienerNsFrameFilterBank)>> {
        let view = self.restoration_view()?;
        let set = pc_wiener_filter_set_index(self.qindex);
        let mut best: Option<(i128, WienerNsFrameFilterBank)> = None;
        for &count in counts {
            let table = pc_wiener_subclass_table(usize::from(count), set)
                .map_err(coding("PC-Wiener subclasses"))?;
            let mut merged = vec![
                WienerStats::new(stats.first().map_or(16, WienerStats::len));
                usize::from(count)
            ];
            for (class, unit) in stats.iter().enumerate() {
                if let Some(target) = table
                    .get(class)
                    .and_then(|&t| merged.get_mut(usize::from(t)))
                {
                    target.add(unit);
                }
            }
            let mut classes = Vec::with_capacity(merged.len());
            let mut distortion = 0;
            for class_stats in &merged {
                let class = match self.best_class(class_stats, 0, view)? {
                    Some((_, class)) => class,
                    None => zero_class(class_stats.len())?,
                };
                distortion += predicted_delta(class_stats, &class.coeffs);
                classes.push(class);
            }
            let bank = bank_of(&classes)?;
            let cost = rd(
                distortion,
                Bits::whole(bank_bits(&bank, 0, view)?),
                self.lambda,
            );
            if cost < 0 && best.as_ref().is_none_or(|(best, _)| cost < *best) {
                best = Some((cost, bank));
            }
        }
        Ok(best)
    }

    fn restoration_view(&self) -> Result<CoreSeqRestorationView> {
        Ok(CoreSeqView::from_sequence(self.seq)
            .ok_or(Error::Unsupported {
                what: "a sequence without a complete view",
            })?
            .restoration())
    }

    /// The class of `plane` over every coefficient shape that minimizes the
    /// predicted squared error of `stats` plus its bits as a one-class bank,
    /// with that cost, or `None` when no class lowers the cost.
    fn best_class(
        &self,
        stats: &WienerStats,
        plane: usize,
        view: CoreSeqRestorationView,
    ) -> Result<Option<(i128, WienerNsFrameFilterClass)>> {
        let chroma = plane > 0;
        let shapes: &[(usize, bool)] = if chroma {
            &[(0, false), (1, false), (1, true), (2, false), (2, true)]
        } else {
            &[(0, false), (1, false), (2, false), (3, false)]
        };
        let mut best: Option<(i128, WienerNsFrameFilterClass)> = None;
        for &(subset, sym) in shapes {
            let groups = tap_groups(stats.len(), chroma, subset, sym);
            let Some(weights) = solve(stats, &groups) else {
                continue;
            };
            let mut coeffs = vec![0i16; stats.len()];
            for (group, &w) in groups.iter().zip(&weights) {
                let (min, max) = wiener_ns_coeff_range(chroma, group[0]).unwrap_or((0, 0));
                let value = ((w * PREC).round() as i64).clamp(i64::from(min), i64::from(max));
                for &j in group {
                    coeffs[j] = value as i16;
                }
            }
            refine(stats, chroma, &groups, &mut coeffs);
            let class = class_of(subset, sym, &coeffs)?;
            let cost = rd(
                predicted_delta(stats, &coeffs),
                Bits::whole(bank_bits(&bank_of(&[class])?, plane, view)?),
                self.lambda,
            );
            if cost < 0 && best.as_ref().is_none_or(|(best, _)| cost < *best) {
                best = Some((cost, class));
            }
        }
        Ok(best)
    }
}

/// The coefficients `wiener_ns_length` subset `subset` codes, grouped so that
/// a group shares one value: pairs of luma taps of a chroma filter with
/// `wiener_ns_uv_sym`, single coefficients otherwise.
fn tap_groups(n: usize, chroma: bool, subset: usize, sym: bool) -> Vec<Vec<usize>> {
    let mut groups = Vec::new();
    let mut j = 0;
    while j < n {
        if !wiener_ns_tap_present(chroma, subset, j) {
            j += 1;
        } else if sym && j >= CHROMA_SHORT && j + 1 < n {
            groups.push(vec![j, j + 1]);
            j += 2;
        } else {
            groups.push(vec![j]);
            j += 1;
        }
    }
    groups
}

fn a_at(stats: &WienerStats, i: usize, j: usize) -> f64 {
    stats.a(i, j) as f64
}

/// `A` and `b` of the group features: each group feature is the sum of the
/// features of its coefficients.
fn group_a(stats: &WienerStats, g: &[usize], h: &[usize]) -> f64 {
    g.iter()
        .flat_map(|&i| h.iter().map(move |&j| a_at(stats, i, j)))
        .sum()
}

fn group_b(stats: &WienerStats, g: &[usize]) -> f64 {
    g.iter().map(|&i| stats.b[i] as f64).sum()
}

/// The least-squares weights of `groups`, by a Cholesky factorization with
/// a small ridge, or `None` for a singular system.
fn solve(stats: &WienerStats, groups: &[Vec<usize>]) -> Option<Vec<f64>> {
    let n = groups.len();
    let trace: f64 = groups.iter().map(|g| group_a(stats, g, g)).sum();
    if n == 0 || trace <= 0.0 {
        return None;
    }
    let ridge = trace / n as f64 * 1e-6;
    let mut l = vec![0.0; n * n];
    for r in 0..n {
        for c in 0..=r {
            let mut v = group_a(stats, &groups[r], &groups[c]) + if r == c { ridge } else { 0.0 };
            for k in 0..c {
                v -= l[r * n + k] * l[c * n + k];
            }
            if r == c {
                if v <= 0.0 {
                    return None;
                }
                l[r * n + r] = v.sqrt();
            } else {
                l[r * n + c] = v / l[c * n + c];
            }
        }
    }
    let mut y = vec![0.0; n];
    for r in 0..n {
        let s: f64 = (0..r).map(|k| l[r * n + k] * y[k]).sum();
        y[r] = (group_b(stats, &groups[r]) - s) / l[r * n + r];
    }
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| l[k * n + r] * y[k]).sum();
        y[r] = (y[r] - s) / l[r * n + r];
    }
    Some(y)
}

/// Moves the value of one group by one step while that lowers the predicted
/// squared error, within the coded ranges.
fn refine(stats: &WienerStats, chroma: bool, groups: &[Vec<usize>], coeffs: &mut [i16]) {
    for _ in 0..8 * coeffs.len() {
        let mut best = (0.0, 0, 0i16);
        for (index, group) in groups.iter().enumerate() {
            let (min, max) = wiener_ns_coeff_range(chroma, group[0]).unwrap_or((0, 0));
            let row: f64 = group
                .iter()
                .map(|&k| {
                    coeffs
                        .iter()
                        .enumerate()
                        .map(|(j, &c)| f64::from(c) * a_at(stats, k, j))
                        .sum::<f64>()
                })
                .sum();
            for step in [-1i16, 1] {
                let next = coeffs[group[0]] + step;
                if next < min || next > max {
                    continue;
                }
                let d = f64::from(step);
                let change = (2.0 * d * row + d * d * group_a(stats, group, group)) / (PREC * PREC)
                    - 2.0 * d * group_b(stats, group) / PREC;
                if change < best.0 {
                    best = (change, index, step);
                }
            }
        }
        if best.2 == 0 {
            return;
        }
        for &j in &groups[best.1] {
            coeffs[j] += best.2;
        }
    }
}

/// The predicted squared-error change of filtering the samples of `stats`
/// with `coeffs`: `c^T A c / 2^14 - 2 c.b / 2^7`, plus the `1/12` per sample
/// that rounding the output to integers adds.
fn predicted_delta(stats: &WienerStats, coeffs: &[i16]) -> i64 {
    if coeffs.iter().all(|&c| c == 0) {
        return 0;
    }
    let mut quadratic = 0.0;
    let mut linear = 0.0;
    for (i, &ci) in coeffs.iter().enumerate() {
        let ci = f64::from(ci);
        linear += ci * stats.b[i] as f64;
        for (j, &cj) in coeffs.iter().enumerate() {
            quadratic += ci * f64::from(cj) * a_at(stats, i, j);
        }
    }
    (quadratic / (PREC * PREC) - 2.0 * linear / PREC + stats.samples as f64 / 12.0).round() as i64
}

pub(super) fn one_class_bank(
    subset: usize,
    sym: bool,
    coeffs: &[i16],
) -> Result<WienerNsFrameFilterBank> {
    bank_of(&[class_of(subset, sym, coeffs)?])
}

/// A coded class with `coeffs`, matched to the zero filter.
fn class_of(subset: usize, sym: bool, coeffs: &[i16]) -> Result<WienerNsFrameFilterClass> {
    Ok(WienerNsFrameFilterClass {
        match_index: 0,
        merged: false,
        ref_bank: 0,
        subset: Some(subset as u8),
        wiener_ns_uv_sym: sym,
        coeffs: WienerNsCoeffs::from_iter_checked(coeffs.iter().copied()).ok_or_else(bad_bank)?,
    })
}

/// A coded class whose coefficients are all `0`.
fn zero_class(n: usize) -> Result<WienerNsFrameFilterClass> {
    class_of(0, false, &vec![0; n])
}

fn bank_of(classes: &[WienerNsFrameFilterClass]) -> Result<WienerNsFrameFilterBank> {
    Ok(WienerNsFrameFilterBank {
        classes: splot_core::tile::InlineVec::from_iter_checked(classes.iter().copied())
            .ok_or_else(bad_bank)?,
    })
}

fn bad_bank() -> Error {
    Error::Unsupported {
        what: "a Wiener-NS bank",
    }
}

/// The bits of `bank` of `plane` coded without reference filters.
fn bank_bits(
    bank: &WienerNsFrameFilterBank,
    plane: usize,
    view: CoreSeqRestorationView,
) -> Result<u32> {
    let mut writer = BitWriter::new();
    write_frame_wiener_ns_filter(&mut writer, plane, (0, &[]), view, bank)
        .map_err(coding("Wiener-NS bank"))?;
    Ok(writer.bit_len() as u32)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::{CHROMA_COEFFS, LUMA_COEFFS};
    use super::*;
    use crate::kernels::wiener::WienerAccumulator;

    /// The solve recovers the weights of a target built from the features,
    /// and the predicted change of those coefficients matches a direct sum.
    #[test]
    fn solve_recovers_a_known_filter() {
        let mut state = 0x9e37_79b9u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            i32::from((state % 200) as u16) - 100
        };
        let truth: [i16; LUMA_COEFFS] = [9, -3, 4, 0, -7, 2, 1, 0, -1, 2, 0, 0, 3, -2, 0, 1];
        let mut features = vec![Vec::new(); LUMA_COEFFS];
        let mut target = Vec::new();
        let mut direct = 0i64;
        for _ in 0..4000 {
            let f: [i32; LUMA_COEFFS] = core::array::from_fn(|_| next());
            let fitted: i64 = f
                .iter()
                .zip(&truth)
                .map(|(&f, &c)| i64::from(f) * i64::from(c))
                .sum();
            let t = (fitted + 64).div_euclid(128);
            for (row, &f) in features.iter_mut().zip(&f) {
                row.push(f);
            }
            target.push(t as i32);
            direct += (t * 128 - fitted).pow(2) - (t * 128).pow(2);
        }
        let mut stats = WienerStats::new(LUMA_COEFFS);
        let rows: [&[i32]; LUMA_COEFFS] = core::array::from_fn(|k| features[k].as_slice());
        let mut lanes = WienerAccumulator::<LUMA_COEFFS>::new(1 << 20);
        lanes.add(&rows, &target, &mut stats);
        lanes.flush(&mut stats);
        stats.samples = 0;
        let groups = tap_groups(LUMA_COEFFS, false, 3, false);
        let weights = solve(&stats, &groups).unwrap();
        for (w, &c) in weights.iter().zip(&truth) {
            assert!((w * PREC - f64::from(c)).abs() < 0.3, "{} vs {c}", w * PREC);
        }
        let predicted = predicted_delta(&stats, &truth) as f64;
        assert!((predicted - direct as f64 / 16384.0).abs() <= 1.0 + predicted.abs() * 1e-9);
    }

    /// Symmetric chroma groups pair the luma taps of each subset.
    #[test]
    fn chroma_groups_pair_the_luma_taps() {
        assert_eq!(tap_groups(CHROMA_COEFFS, true, 0, true).len(), 6);
        let sym = tap_groups(CHROMA_COEFFS, true, 1, true);
        assert_eq!(sym[6..], [vec![6, 7], vec![8, 9]]);
        assert_eq!(tap_groups(CHROMA_COEFFS, true, 2, true).len(), 12);
        assert_eq!(tap_groups(CHROMA_COEFFS, true, 2, false).len(), 18);
    }
}
