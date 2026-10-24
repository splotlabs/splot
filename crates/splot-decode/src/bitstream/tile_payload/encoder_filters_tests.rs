// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use splot_core::headers::frame::{
    CoreSeqView, FrameFilterParams, FrameWienerNs, KeyFrameTxParams, UniformTiles, WienerNsCoeffs,
    WienerNsFrameFilterBank, WienerNsFrameFilterClass, build_encoder_sequence_header,
    build_key_frame_header,
};
use splot_core::headers::frame::{EncoderStreamParams, EncoderToolFlags};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SuperblockSize};
use splot_recon::{BitDepth, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneSize};

use super::*;
use crate::filters::deblock::ChromaDeblockRecords;
use crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord;

const QINDEX: u32 = 120;

/// A luma texture of regions with different activity: flat, row and column
/// stripes, and noise of several strengths.
fn sample(x: usize, y: usize) -> u16 {
    let hash = (x * 7919 + y * 104_729 + (x * y) % 97) % 251;
    let value = match (x / 16 + y / 16 * 3) % 6 {
        0 => 120,
        1 => 60 + (y % 2) * 120,
        2 => 70 + (x % 3) * 50,
        3 => 100 + hash % 16,
        4 => 40 + hash % 160,
        _ => 30 + (x * 9 + y * 4) % 190,
    };
    value as u16
}

/// The publication check of the class map: with a 16-class bank whose class
/// `s` weighs only the vertical tap pair with `s - 8`, every luma sample of
/// the decoder's restoration is the one `SubclassLookup` of its
/// [`pc_wiener_cell_classes`] class predicts.
#[test]
fn cell_classes_are_the_classes_the_restoration_applies() {
    let (width, height) = (128usize, 64usize);
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: width as u32,
        height: height as u32,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            loop_restoration: true,
            ..EncoderToolFlags::default()
        },
    })
    .unwrap();
    let class = |s: i16| {
        let mut coeffs = [0i16; 16];
        coeffs[0] = s - 8;
        WienerNsFrameFilterClass {
            match_index: 0,
            merged: false,
            ref_bank: 0,
            subset: Some(0),
            wiener_ns_uv_sym: false,
            coeffs: WienerNsCoeffs::from_iter_checked(coeffs).unwrap(),
        }
    };
    let bank = WienerNsFrameFilterBank {
        classes: splot_core::tile::InlineVec::from_iter_checked((0..16).map(class)).unwrap(),
    };
    let filters = FrameFilterParams {
        lr_luma_unit_size: Some(64),
        lr_luma_wiener_ns: Some(FrameWienerNs {
            switchable: false,
            rst_ref_pic_idx: None,
            bank,
        }),
        ..FrameFilterParams::default()
    };
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let core = build_key_frame_header(
        &view,
        QINDEX,
        0,
        &filters,
        KeyFrameTxParams::default(),
        UniformTiles::default(),
    )
    .unwrap();
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        PlaneSize::new(width, height).unwrap(),
        PlaneRect::new(0, 0, width, height).unwrap(),
    )
    .unwrap();
    let mut workspace = CurrentFrameWorkspace::new(info, 0).unwrap();
    let luma: Vec<u16> = (0..height)
        .flat_map(|y| (0..width).map(move |x| sample(x, y)))
        .collect();
    workspace
        .write_rect(
            PlaneId::Y,
            PlaneRect::new(0, 0, width, height).unwrap(),
            &luma,
            width,
        )
        .unwrap();
    let chroma = vec![128u16; width * height / 4];
    for id in [PlaneId::U, PlaneId::V] {
        let rect = PlaneRect::new(0, 0, width / 2, height / 2).unwrap();
        workspace.write_rect(id, rect, &chroma, width / 2).unwrap();
    }
    let frame = workspace.freeze().unwrap();
    let mut tx_skip = Vec::new();
    for row in 0..height / 4 {
        for col in 0..width / 4 {
            tx_skip.push(WienerNsLrTxSkipTransformRecord {
                row,
                col,
                rows: 1,
                cols: 1,
                skip_flag: (row * 3 + col) % 5 == 0,
                eob: (row + col) % 3,
            });
        }
    }
    let recorder = FilterRecorder {
        qindex: QINDEX,
        subsampling: (1, 1),
        deblock_blocks: Vec::new(),
        chroma_blocks: ChromaDeblockRecords::new(),
        tx_skip,
    };
    let mut lr_on = Vec::new();
    for layout in LrLayout::tiles(&seq, &core).unwrap() {
        lr_on.extend(
            layout
                .all_units()
                .unwrap()
                .into_iter()
                .map(|unit| (unit, LrUnitType::WienerNs)),
        );
    }
    let out = apply_loop_filters(&seq, &core, &recorder, &frame, (&[], &[]), &lr_on).unwrap();
    let classes = pc_wiener_cell_classes(&seq, &core, &recorder, &frame).unwrap();
    let table =
        splot_recon::pc_wiener_subclass_table(16, splot_recon::pc_wiener_filter_set_index(QINDEX))
            .unwrap();
    let restored = out.plane(PlaneId::Y).unwrap();
    let at = |x: usize, y: isize| i32::from(sample(x, y.clamp(0, height as isize - 1) as usize));
    let mut used = [false; 16];
    for y in 0..height {
        for x in 0..width {
            let subclass = table[usize::from(classes[(y / 4) * (width / 4) + x / 4])];
            used[usize::from(subclass)] = true;
            let m = at(x, y as isize);
            let coeff = i32::from(subclass) - 8;
            let s =
                m * 128 + coeff * (at(x, y as isize + 1) - m) + coeff * (at(x, y as isize - 1) - m);
            let expected = ((s + 64) >> 7).clamp(0, 255);
            let actual = i32::from(restored.samples()[y * restored.stride_samples() + x]);
            assert_eq!(actual, expected, "sample ({x}, {y}) subclass {subclass}");
        }
    }
    assert!(used.iter().filter(|&&u| u).count() >= 4, "{used:?}");
}
