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

/// The frame-level `FrameLrWienerNs[plane]` bank parsed by AV2 § 5.20.10.6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WienerNsFrameFilterBank {
    /// One parsed class entry per `numClasses`.
    pub classes: Vec<WienerNsFrameFilterClass>,
}

/// One class from a parsed frame-level Wiener NS filter bank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WienerNsFrameFilterClass {
    /// The frame-filter match index selected before the merge flags.
    pub match_index: u8,
    /// `merged[c]`: whether the class reuses its selected reference bank entry.
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

pub(super) fn parse_frame_wiener_ns_filter(
    reader: &mut BitReader<'_>,
    plane: usize,
    num_filter_classes: u8,
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

    let match_indices = read_match_indices(reader, plane_is_chroma, num_classes, nopcw)?;
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
    plane_is_chroma: bool,
    num_classes: usize,
    nopcw: bool,
) -> Result<Vec<usize>> {
    let mut group_counts = [
        num_classes,
        0usize,
        if plane_is_chroma || nopcw {
            0
        } else {
            64usize.saturating_sub(num_classes)
        },
    ];
    let group_base = [0usize, group_counts[0], group_counts[0] + group_counts[1]];
    let mut group_hits = [0usize; 3];
    let mut match_indices = Vec::with_capacity(num_classes);

    for c in 0..num_classes {
        group_counts[0] = c + 1;
        let pred_group = if c == 0 {
            if group_counts[1] > 2 {
                1
            } else {
                predict_group(group_counts)
            }
        } else {
            predict_group(group_hits)
        };

        let (num_zeros, alt_group) = alternate_group(group_counts, pred_group);
        let use_alt_group = if num_zeros == 2 {
            false
        } else {
            reader.read_flag()?
        };
        let group = if use_alt_group {
            if num_zeros == 1 {
                alt_group
            } else {
                let group_bit = usize::from(reader.read_bit()?);
                if pred_group <= group_bit {
                    group_bit + 1
                } else {
                    group_bit
                }
            }
        } else {
            pred_group
        };

        let n = group_counts[group];
        let base = group_base[group];
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
        group_hits[group] += 1;
        match_indices.push(match_index);
    }
    Ok(match_indices)
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

fn alternate_group(group_counts: [usize; 3], pred_group: usize) -> (u8, usize) {
    let mut num_zeros = 0u8;
    let mut alt_group = 0usize;
    for (i, count) in group_counts.iter().enumerate() {
        if i != pred_group {
            if *count == 0 {
                num_zeros += 1;
            } else {
                alt_group = i;
            }
        }
    }
    (num_zeros, alt_group)
}

fn predict_group(counts: [usize; 3]) -> usize {
    let mut pred = 0usize;
    for i in 1..=2 {
        if counts[i] > counts[pred] {
            pred = i;
        }
    }
    pred
}

#[allow(clippy::too_many_arguments)]
fn fill_first_slot_of_bank_with_filter_match(
    c: usize,
    plane_is_chroma: bool,
    match_index: usize,
    num_classes: usize,
    bank_ptr: &mut [usize],
    bank_size: &mut [usize],
    ref_bank: &mut [[[i16; WIENER_NS_CHROMA_COEFFS]; LR_BANK_SIZE]],
    frame_coeffs: &[Vec<i16>],
    n_coeffs: usize,
) {
    bank_ptr[c] = 0;
    bank_size[c] = 1;
    for (j, coeff) in ref_bank[c][0].iter_mut().enumerate().take(n_coeffs) {
        *coeff = filter_match_coeff(plane_is_chroma, match_index, num_classes, frame_coeffs, j);
    }
}

fn filter_match_coeff(
    plane_is_chroma: bool,
    match_index: usize,
    num_classes: usize,
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
    } else if plane_is_chroma {
        0
    } else {
        translated_pc_wiener(match_index.saturating_sub(num_classes), j)
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
            parse_frame_wiener_ns_filter(&mut r, 0, 2, restoration_without_pc_wiener()).unwrap();

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
            parse_frame_wiener_ns_filter(&mut r, 0, 1, restoration_without_pc_wiener()).unwrap();

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
            parse_frame_wiener_ns_filter(&mut r, 0, 1, restoration_without_pc_wiener()).is_err()
        );
    }
}
