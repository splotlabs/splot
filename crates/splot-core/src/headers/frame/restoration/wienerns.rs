// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.10.6 frame-level Wiener non-separable filter syntax.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::frame::decode_signed_subexp_with_ref;
use crate::tables::loop_restoration::PC_WIENER_FILTERS;

use super::CoreSeqRestorationView;

const WIENER_NS_LUMA_COEFFS: usize = 16;
const WIENER_NS_CHROMA_COEFFS: usize = 18;
const WIENER_NS_SHORT_COEFFS: usize = 6;
const LR_BANK_SIZE: usize = 4;

const WIENER_NS_TAPS_MIN: [[i16; WIENER_NS_CHROMA_COEFFS]; 2] = [
    [
        -24, -24, -14, -14, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
    ],
    [
        -24, -24, -14, -14, -16, -16, -16, -16, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8,
    ],
];

const WIENER_NS_TAPS_K: [[u8; WIENER_NS_CHROMA_COEFFS]; 2] = [
    [6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [6, 6, 5, 5, 5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4],
];

const WIENER_NS_TAPS_PRESENT: [[[bool; WIENER_NS_CHROMA_COEFFS]; 4]; 2] = [
    [
        [
            true, true, true, true, true, true, false, false, false, false, false, false, false,
            false, false, false, false, false,
        ],
        [
            true, true, false, false, false, false, true, true, true, true, true, true, false,
            false, false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, false, false,
            false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, false, false,
        ],
    ],
    [
        [
            true, true, true, true, true, true, false, false, false, false, false, false, false,
            false, false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, false, false, false, false,
            false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, true, true,
        ],
        [false; WIENER_NS_CHROMA_COEFFS],
    ],
];

const SHUFFLED_INDEX: [usize; 64] = [
    16, 7, 58, 21, 12, 61, 26, 38, 18, 30, 50, 45, 23, 49, 43, 62, 42, 54, 27, 36, 17, 44, 32, 34,
    4, 24, 52, 31, 37, 11, 33, 19, 35, 6, 22, 53, 63, 25, 41, 47, 1, 59, 0, 28, 40, 55, 48, 8, 5,
    51, 9, 46, 56, 60, 15, 2, 13, 14, 57, 29, 3, 20, 39, 10,
];

/// The resolved frame-level `FrameLrWienerNs[plane]` bank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WienerNsFrameFilterBank {
    /// One resolved class entry per `numClasses`.
    pub classes: Vec<WienerNsFrameFilterClass>,
}

/// One class from a resolved frame-level Wiener NS filter bank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WienerNsFrameFilterClass {
    /// The frame-filter match index selected before the merge flags, or the class ordinal
    /// for a bank copied by the frame-header temporal-prediction arm.
    pub match_index: u8,
    /// `merged[c]`: whether the class reuses its selected reference bank entry. Temporal
    /// copies set this because their coefficients are inherited rather than coded locally.
    pub merged: bool,
    /// `refBank[c]`; the frame-level fixed-coded path always selects bank slot `0`.
    pub ref_bank: u8,
    /// The decoded subset for an explicitly coded class, or `None` when `merged`.
    pub subset: Option<u8>,
    /// `wiener_ns_uv_sym`, only meaningful for chroma classes with `subset > 0`.
    pub wiener_ns_uv_sym: bool,
    /// The parsed `FrameLrWienerNs[plane][c]` coefficients.
    pub coeffs: Vec<i16>,
}

/// Parses a § 5.18 frame-level Wiener-NS filter bank. `ref_taps` are the
/// retained taps of the § 5.18 `search_frame_filters` reference entries, in
/// order; a `Some(None)` entry is a stored reference whose bank was LR
/// temporal-copied rather than locally parsed, so a match that selects it
/// fails closed. An absent entry (a counts-only caller supplies no taps)
/// keeps the bit-exact index parse with the unresolved value's zero seed.
pub(super) fn parse_frame_wiener_ns_filter(
    reader: &mut BitReader<'_>,
    plane: usize,
    num_filter_classes: u8,
    num_ref_filters: usize,
    ref_taps: &[Option<&[i16]>],
    view: CoreSeqRestorationView,
) -> Result<WienerNsFrameFilterBank> {
    let plane_is_chroma = plane > 0;
    let plane_index = usize::from(plane_is_chroma);
    let num_classes = if plane_is_chroma {
        1
    } else {
        usize::from(num_filter_classes.max(1))
    };
    let n_coeffs = if plane_is_chroma {
        WIENER_NS_CHROMA_COEFFS
    } else {
        WIENER_NS_LUMA_COEFFS
    };
    let nopcw = view.lr_pc_wiener_disabled;

    let match_indices = read_match_indices(reader, plane, num_classes, num_ref_filters, nopcw)?;
    let capped_ref = capped_reference_filter_count(plane, num_classes, num_ref_filters, nopcw);
    let ref_taps = ref_taps
        .get(..capped_ref.min(ref_taps.len()))
        .unwrap_or(&[]);
    for &match_index in &match_indices {
        if (num_classes..num_classes + capped_ref).contains(&match_index)
            && matches!(ref_taps.get(match_index - num_classes), Some(None))
        {
            return Err(crate::error::Error::Unimplemented {
                feature: "lr_temporal_reference_filter_match",
            });
        }
    }
    let merged = read_merged_flags(reader, num_classes)?;
    let mut ref_bank = vec![[[0i16; WIENER_NS_CHROMA_COEFFS]; LR_BANK_SIZE]; num_classes];
    let mut bank_size = vec![0usize; num_classes];
    let mut bank_ptr = vec![0usize; num_classes];
    for class_bank in &mut ref_bank {
        for slot in class_bank {
            for (j, coeff) in slot.iter_mut().enumerate() {
                *coeff = initial_tap_value(plane_index, j);
            }
        }
    }

    let mut frame_coeffs = vec![vec![0i16; n_coeffs]; num_classes];
    let mut classes = Vec::with_capacity(num_classes);
    for c in 0..num_classes {
        fill_first_slot_of_bank_with_filter_match(
            c,
            plane_is_chroma,
            match_indices[c],
            num_classes,
            capped_ref,
            ref_taps,
            &mut bank_ptr,
            &mut bank_size,
            &mut ref_bank,
            &frame_coeffs,
            n_coeffs,
        );

        let mut subset = None;
        let mut wiener_ns_uv_sym = false;
        if merged[c] {
            if bank_size[c] == 0 {
                bank_size[c] = 1;
            }
        } else {
            if bank_size[c] < LR_BANK_SIZE {
                bank_ptr[c] = bank_size[c];
                bank_size[c] += 1;
            } else {
                bank_ptr[c] = (bank_ptr[c] + 1) % LR_BANK_SIZE;
            }
            let read_subset = read_wiener_ns_subset(reader, plane_is_chroma)?;
            if plane_is_chroma && read_subset > 0 {
                wiener_ns_uv_sym = reader.read_flag()?;
            }
            subset = Some(read_subset);
        }

        let mut coeffs = vec![0i16; n_coeffs];
        let mut j = 0usize;
        while j < n_coeffs {
            let mut value = ref_bank[c][0][j];
            if !merged[c] {
                let tap_subset = usize::from(subset.unwrap_or(0));
                if WIENER_NS_TAPS_PRESENT[plane_index][tap_subset][j] {
                    let min = i64::from(WIENER_NS_TAPS_MIN[plane_index][j]);
                    let k = u32::from(WIENER_NS_TAPS_K[plane_index][j]);
                    value = decode_signed_subexp_with_ref(
                        reader,
                        min,
                        min + (1i64 << k),
                        i64::from(value),
                        k.saturating_sub(3),
                    )? as i16;
                } else {
                    value = 0;
                }
            }
            coeffs[j] = value;
            if !merged[c]
                && plane_is_chroma
                && j >= WIENER_NS_SHORT_COEFFS
                && wiener_ns_uv_sym
                && j + 1 < n_coeffs
            {
                coeffs[j + 1] = value;
                j += 2;
            } else {
                j += 1;
            }
        }
        frame_coeffs[c].clone_from(&coeffs);
        classes.push(WienerNsFrameFilterClass {
            match_index: match_indices[c] as u8,
            merged: merged[c],
            ref_bank: 0,
            subset,
            wiener_ns_uv_sym,
            coeffs,
        });
    }

    Ok(WienerNsFrameFilterBank { classes })
}

fn read_match_indices(
    reader: &mut BitReader<'_>,
    plane: usize,
    num_classes: usize,
    num_ref_filters: usize,
    nopcw: bool,
) -> Result<Vec<usize>> {
    let group_counts = [
        num_classes,
        capped_reference_filter_count(plane, num_classes, num_ref_filters, nopcw),
        sampled_pc_wiener_filter_count(plane, num_classes, num_ref_filters, nopcw),
    ];
    let mut match_indices = Vec::with_capacity(num_classes);

    for c in 0..num_classes {
        let pred_group = if c == 0 {
            most_probable_group(c, group_counts)
        } else {
            predict_group_from_prior_matches(&match_indices, group_counts)
        };

        let group = if only_group_available(group_counts, pred_group) || reader.read_bit()? == 0 {
            pred_group
        } else {
            let zero_group = first_zero_group(group_counts, pred_group);
            if let Some(zero_group) = zero_group {
                3usize.saturating_sub(pred_group + zero_group)
            } else {
                let group_bit = reader.read_bit()?;
                if group_bit != 0 {
                    [2usize, 2, 1][pred_group]
                } else {
                    [1usize, 0, 0][pred_group]
                }
            }
        };

        let n = if group == 0 {
            c + 1
        } else {
            group_counts[group]
        };
        let base = group_base(group, group_counts);
        let match_index = if n == 1 {
            base
        } else {
            let ref_index = base + (n >> 1);
            let decoded = decode_signed_subexp_with_ref(
                reader,
                base as i64,
                (base + n) as i64,
                ref_index as i64,
                4,
            )?;
            usize::try_from(decoded).unwrap_or(base)
        };
        match_indices.push(match_index);
    }
    Ok(match_indices)
}

const fn num_dictionary_slots(num_classes: usize, nopcw: bool) -> usize {
    let _ = num_classes;
    if nopcw { 16 } else { 64 }
}

const fn max_num_base_filters(num_classes: usize, nopcw: bool) -> usize {
    num_dictionary_slots(num_classes, nopcw).saturating_sub(num_classes)
}

const fn sampled_pc_wiener_filter_count(
    plane: usize,
    num_classes: usize,
    num_ref_filters: usize,
    nopcw: bool,
) -> usize {
    if plane != 0 || nopcw {
        0
    } else {
        let available = max_num_base_filters(num_classes, false).saturating_sub(num_ref_filters);
        if available > 64 { 64 } else { available }
    }
}

const fn capped_reference_filter_count(
    plane: usize,
    num_classes: usize,
    num_ref_filters: usize,
    nopcw: bool,
) -> usize {
    let min_pc_wiener = if plane == 0 && !nopcw { 16 } else { 0 };
    let allowed = max_num_base_filters(num_classes, nopcw).saturating_sub(min_pc_wiener);
    if num_ref_filters > allowed {
        allowed
    } else {
        num_ref_filters
    }
}

fn read_merged_flags(reader: &mut BitReader<'_>, num_classes: usize) -> Result<Vec<bool>> {
    let mut merged = Vec::with_capacity(num_classes);
    for _ in 0..num_classes {
        merged.push(reader.read_flag()?);
    }
    Ok(merged)
}

fn read_wiener_ns_subset(reader: &mut BitReader<'_>, plane_is_chroma: bool) -> Result<u8> {
    let num_subsets = if plane_is_chroma { 3 } else { 4 };
    let mut subset = 0u8;
    while usize::from(subset) < num_subsets - 1 {
        if reader.read_bit()? == 0 {
            break;
        }
        subset = subset.saturating_add(1);
    }
    Ok(subset)
}

fn most_probable_group(c: usize, counts: [usize; 3]) -> usize {
    let group_count_0 = c + 1;
    if group_count_0 > 2 || counts[1] > 2 {
        return usize::from(group_count_0 <= counts[1]);
    }
    if group_count_0 >= counts[1] && group_count_0 >= counts[2] {
        0
    } else if counts[1] >= counts[2] {
        1
    } else {
        2
    }
}

fn predict_group_from_prior_matches(match_indices: &[usize], group_counts: [usize; 3]) -> usize {
    let mut counts = [0usize; 3];
    for &match_index in match_indices {
        counts[index_to_group(match_index, group_counts)] += 1;
    }
    let mut pred = 0usize;
    for i in 1..=2 {
        if counts[i] > counts[pred] {
            pred = i;
        }
    }
    pred
}

fn index_to_group(match_index: usize, group_counts: [usize; 3]) -> usize {
    if match_index < group_counts[0] {
        0
    } else if match_index < group_counts[0] + group_counts[1] {
        1
    } else {
        2
    }
}

fn only_group_available(group_counts: [usize; 3], pred_group: usize) -> bool {
    group_counts
        .iter()
        .enumerate()
        .all(|(group, count)| group == pred_group || *count == 0)
}

fn first_zero_group(group_counts: [usize; 3], pred_group: usize) -> Option<usize> {
    group_counts
        .iter()
        .enumerate()
        .find_map(|(group, count)| (group != pred_group && *count == 0).then_some(group))
}

const fn group_base(group: usize, group_counts: [usize; 3]) -> usize {
    match group {
        0 => 0,
        1 => group_counts[0],
        _ => group_counts[0] + group_counts[1],
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_first_slot_of_bank_with_filter_match(
    c: usize,
    plane_is_chroma: bool,
    match_index: usize,
    num_classes: usize,
    capped_ref: usize,
    ref_taps: &[Option<&[i16]>],
    bank_ptr: &mut [usize],
    bank_size: &mut [usize],
    ref_bank: &mut [[[i16; WIENER_NS_CHROMA_COEFFS]; LR_BANK_SIZE]],
    frame_coeffs: &[Vec<i16>],
    n_coeffs: usize,
) {
    bank_ptr[c] = 0;
    bank_size[c] = 1;
    for (j, coeff) in ref_bank[c][0].iter_mut().enumerate().take(n_coeffs) {
        *coeff = filter_match_coeff(
            plane_is_chroma,
            match_index,
            num_classes,
            capped_ref,
            ref_taps,
            frame_coeffs,
            j,
        );
    }
}

/// § 5.18 `fill_first_slot_of_bank_with_filter_match` value resolution: match
/// `m == 0` seeds zero, `m < numClasses` copies an earlier class of the frame
/// bank, `m < numClasses + numRefFilters` copies the matched REFERENCE frame's
/// retained taps (`RefFrameLrWienerNs`, 05:17763-17780), and the remainder
/// translates a sampled PC-Wiener filter offset by BOTH group sizes.
fn filter_match_coeff(
    plane_is_chroma: bool,
    match_index: usize,
    num_classes: usize,
    capped_ref: usize,
    ref_taps: &[Option<&[i16]>],
    frame_coeffs: &[Vec<i16>],
    j: usize,
) -> i16 {
    if match_index == 0 {
        0
    } else if match_index < num_classes {
        let old_class = match_index - 1;
        frame_coeffs
            .get(old_class)
            .and_then(|coeffs| coeffs.get(j))
            .copied()
            .unwrap_or(0)
    } else if match_index < num_classes + capped_ref {
        ref_taps
            .get(match_index - num_classes)
            .copied()
            .flatten()
            .and_then(|taps| taps.get(j))
            .copied()
            .unwrap_or(0)
    } else if plane_is_chroma {
        0
    } else {
        translated_pc_wiener(match_index.saturating_sub(num_classes + capped_ref), j)
    }
}

fn initial_tap_value(plane_index: usize, j: usize) -> i16 {
    let min = WIENER_NS_TAPS_MIN[plane_index][j];
    let k = WIENER_NS_TAPS_K[plane_index][j];
    min + ((1i16 << k) >> 1)
}

fn translated_pc_wiener(match_index: usize, j: usize) -> i16 {
    if j >= 12 {
        return 0;
    }
    let Some(&filter_index) = SHUFFLED_INDEX.get(match_index) else {
        return 0;
    };
    let coeff = PC_WIENER_FILTERS
        .first()
        .and_then(|filters| filters.get(filter_index))
        .and_then(|filter| filter.get(j))
        .copied()
        .unwrap_or(0);
    let min = i32::from(WIENER_NS_TAPS_MIN[0][j]);
    let max = min + (1i32 << WIENER_NS_TAPS_K[0][j]) - 1;
    coeff.clamp(min, max) as i16
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::span::ByteOffset;

    use super::*;

    use crate::test_bits::Bits;

    fn reader(data: &[u8]) -> BitReader<'_> {
        BitReader::new(data, ByteOffset::new(0))
    }

    fn restoration_without_pc_wiener() -> CoreSeqRestorationView {
        CoreSeqRestorationView {
            enable_restoration: true,
            lr_pc_wiener_disabled: true,
            lr_wiener_nonsep_disabled: false,
            lr_uv_pc_wiener_disabled: true,
            lr_uv_wiener_nonsep_disabled: false,
        }
    }

    #[test]
    fn luma_two_class_merged_bank_shape() {
        let mut bits = Bits::default();
        bits.bit(0); // c=1 match index decodes to prior class 0.
        bits.bit(1); // merged[0]
        bits.bit(1); // merged[1]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let bank =
            parse_frame_wiener_ns_filter(&mut r, 0, 2, 0, &[], restoration_without_pc_wiener())
                .unwrap();

        assert_eq!(r.consumed_bits(), 3);
        assert_eq!(bank.classes.len(), 2);
        assert_eq!(bank.classes[0].match_index, 0);
        assert_eq!(bank.classes[1].match_index, 1);
        assert!(bank.classes.iter().all(|class| class.merged));
        assert!(bank.classes.iter().all(|class| class.ref_bank == 0));
        assert!(bank.classes.iter().all(|class| class.coeffs.len() == 16));
        assert!(bank.classes.iter().all(|class| class.coeffs == vec![0; 16]));
    }

    #[test]
    fn luma_reference_match_resolves_retained_taps() {
        let taps: Vec<i16> = (0..WIENER_NS_LUMA_COEFFS as i16).map(|v| v - 3).collect();
        let ref_taps: [Option<&[i16]>; 1] = [Some(taps.as_slice())];
        let mut bits = Bits::default();
        bits.bit(1); // c=0 selects the reference group (match index 1).
        bits.bit(1); // merged[0] -> class copies the reference bank slot verbatim.
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let bank = parse_frame_wiener_ns_filter(
            &mut r,
            0,
            1,
            1,
            &ref_taps,
            restoration_without_pc_wiener(),
        )
        .unwrap();

        assert_eq!(bank.classes.len(), 1);
        assert_eq!(bank.classes[0].match_index, 1);
        assert_eq!(bank.classes[0].coeffs, taps);
    }

    #[test]
    fn luma_reference_match_counts_only_parses_without_taps() {
        let mut bits = Bits::default();
        bits.bit(1); // c=0 selects the reference group (match index 1).
        bits.bit(1); // merged[0]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let bank =
            parse_frame_wiener_ns_filter(&mut r, 0, 1, 1, &[], restoration_without_pc_wiener())
                .unwrap();

        assert_eq!(bank.classes.len(), 1);
        assert_eq!(bank.classes[0].match_index, 1);
        assert_eq!(bank.classes[0].coeffs, vec![0; WIENER_NS_LUMA_COEFFS]);
    }

    #[test]
    fn luma_reference_match_without_retained_taps_fails_closed() {
        let ref_taps: [Option<&[i16]>; 1] = [None];
        let mut bits = Bits::default();
        bits.bit(1); // c=0 selects the reference group (match index 1).
        bits.bit(1); // merged[0]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_frame_wiener_ns_filter(
            &mut r,
            0,
            1,
            1,
            &ref_taps,
            restoration_without_pc_wiener(),
        );
        assert!(matches!(
            outcome,
            Err(crate::error::Error::Unimplemented {
                feature: "lr_temporal_reference_filter_match"
            })
        ));
    }

    #[test]
    fn luma_reference_filter_group_reads_selection_bit() {
        let mut bits = Bits::default();
        bits.bit(0); // keep c=0 in the predicted previous-class group, not the ref group.
        bits.bit(1); // merged[0]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let bank =
            parse_frame_wiener_ns_filter(&mut r, 0, 1, 1, &[], restoration_without_pc_wiener())
                .unwrap();

        assert_eq!(r.consumed_bits(), 2);
        assert_eq!(bank.classes.len(), 1);
        assert_eq!(bank.classes[0].match_index, 0);
        assert!(bank.classes[0].merged);
    }

    #[test]
    fn luma_nonmerged_subset_zero_reads_present_taps() {
        let mut bits = Bits::default();
        bits.bit(0); // merged[0] == 0.
        bits.bit(0); // subset 0.
        bits.raw("01010010"); // tap 0: decode -23 from midpoint 8.
        bits.raw("01010010"); // tap 1.
        bits.raw("01010"); // tap 2.
        bits.raw("01010"); // tap 3.
        bits.raw("01010"); // tap 4.
        bits.raw("01010"); // tap 5.
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let bank =
            parse_frame_wiener_ns_filter(&mut r, 0, 1, 0, &[], restoration_without_pc_wiener())
                .unwrap();

        assert_eq!(bank.classes.len(), 1);
        let class = &bank.classes[0];
        assert!(!class.merged);
        assert_eq!(class.subset, Some(0));
        assert_eq!(&class.coeffs[..6], &[-3, 1, 1, -3, -1, 1]);
        assert!(class.coeffs[6..].iter().all(|&coeff| coeff == 0));
    }

    #[test]
    fn eof_inside_frame_bank_is_structured_error() {
        let mut r = reader(&[]);
        assert!(
            parse_frame_wiener_ns_filter(&mut r, 0, 1, 0, &[], restoration_without_pc_wiener())
                .is_err()
        );
    }
}
