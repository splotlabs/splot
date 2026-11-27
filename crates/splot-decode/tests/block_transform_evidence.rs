// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Transform observations from independently encoded AV2 fixtures.
//! The expected geometry and transform types come from the AVM residual parser.
//! AVM revision a1ec4594d97028ebc34083ec8346616d697c2fc9 supplies these values.

#![expect(clippy::unwrap_used, clippy::panic)]

use std::collections::HashSet;

use splot_core::stream_reader::TemporalUnitReader;
use splot_decode::block_evidence::{
    BlockCoefficientDomain, BlockEvidenceData, BlockEvidenceId, BlockEvidenceKind,
    BlockEvidenceRecord, BlockEvidenceReport, BlockEvidenceRequest, BlockEvidenceStatus,
    BlockEvidenceTree, BlockTransformCoefficients, BlockTransformEvidence, BlockTransformId,
};
use splot_decode::encoder_syntax::TxType;
use splot_decode::{
    BitDepth, DataProps, Decoder, DecoderSettings, FrameDelay, Picture, PictureSamples, PlaneId,
    PlaneRect, SendStatus, ThreadCount,
};

const TWO_TILES: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-2tile-zone1-edge-128x64-q192.ivf");
const INTER_EDGE_422: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-3frame-padded-filters-422-134x69-10bit-q150.ivf"
);
const SEPARATE_TREES_422: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-resid-lossless-offframe-unit-422-10bit-66x50.ivf"
);

fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
    PlaneRect::new(x, y, width, height).unwrap()
}

fn request(token: u64, region: Option<PlaneRect>, limit: usize) -> BlockEvidenceRequest {
    BlockEvidenceRequest::new(move |obu| obu.user_data == token, region, limit, usize::MAX)
        .with_kind(BlockEvidenceKind::Transforms)
}

fn raw_samples(picture: &Picture, bytes: &mut Vec<u8>) {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        match picture.plane(plane) {
            Some(PictureSamples::Eight(plane)) => {
                for row in plane.visible_rows() {
                    bytes.extend_from_slice(row);
                }
            }
            Some(PictureSamples::Ten(plane)) => {
                for sample in plane.visible_rows().flatten() {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
            }
            None => {}
        }
    }
}

fn decode(
    input: &[u8],
    threads: usize,
    request: Option<BlockEvidenceRequest>,
) -> (Vec<u8>, Option<BlockEvidenceReport>) {
    let settings = DecoderSettings::default()
        .with_threads(ThreadCount::from(threads))
        .with_frame_delay(FrameDelay::from(2usize));
    let (mut decoder, capture) = if let Some(request) = request {
        let (decoder, capture) = Decoder::new_with_block_evidence(settings, request).unwrap();
        (decoder, Some(capture))
    } else {
        (Decoder::new(settings).unwrap(), None)
    };
    let mut bytes = Vec::new();
    let mut reader = TemporalUnitReader::new(input);
    let mut token = 0u64;
    while let Some(unit) = reader.next_temporal_unit().unwrap() {
        if unit.bytes.is_empty() {
            continue;
        }
        let props = DataProps::default().with_user_data(token);
        while decoder.send_data(unit.bytes, props).unwrap() == SendStatus::Full {
            while let Some(picture) = decoder.get_picture().unwrap() {
                raw_samples(&picture, &mut bytes);
            }
        }
        token += 1;
    }
    decoder.drain();
    while let Some(picture) = decoder.get_picture().unwrap() {
        raw_samples(&picture, &mut bytes);
    }
    (bytes, capture.map(|capture| capture.take().unwrap()))
}

fn transforms(
    report: &BlockEvidenceReport,
) -> impl Iterator<Item = (&BlockEvidenceRecord, BlockTransformEvidence)> {
    report.records.iter().map(|record| {
        let BlockEvidenceData::Transform(transform) = record.data else {
            panic!("expected a transform record");
        };
        assert_eq!(record.id, transform.id.block);
        (record, transform)
    })
}

#[test]
fn two_tiles_8bit_report_rectangular_native_units_and_exact_details() {
    let (samples, report) = decode(TWO_TILES, 2, Some(request(0, None, 128)));
    assert_eq!(samples, decode(TWO_TILES, 2, None).0);
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    let all: HashSet<_> = transforms(&report).map(|(_, unit)| unit.id).collect();
    assert_eq!(all.len(), report.records.len());
    assert!(all.iter().any(|id| id.block.tile == 0));

    let block = rect(64, 0, 64, 32);
    let mut top_right = transforms(&report)
        .filter(|(_, unit)| unit.id.block.tile == 1 && unit.id.block.coded == block);
    let (luma_record, luma) = top_right.next().unwrap();
    assert_eq!(luma.id.plane, PlaneId::Y);
    assert_eq!(luma.id.coded, block);
    assert_eq!(luma_record.visible, Some(block));
    assert_eq!(luma.details, None);
    let (_, u) = top_right.next().unwrap();
    let (_, v) = top_right.next().unwrap();
    assert!(top_right.next().is_none());
    assert_eq!([u.id.plane, v.id.plane], [PlaneId::U, PlaneId::V]);
    assert_eq!(u.id.coded, rect(32, 0, 32, 16));
    assert_eq!(u.id.coded, v.id.coded);
    assert_eq!([u.luma_coded, v.luma_coded], [block, block]);

    for (id, expected_type) in [(luma.id, Some(TxType::DctDct)), (u.id, None)] {
        let selected = request(0, None, 1).with_transform_details(id);
        let (_, report) = decode(TWO_TILES, 1, Some(selected));
        let report = report.unwrap();
        assert_eq!(report.status, BlockEvidenceStatus::Complete);
        assert_eq!(report.records.len(), 1);
        let (_, unit) = transforms(&report).next().unwrap();
        assert_eq!(unit.id, id);
        let details = unit.details.unwrap();
        assert_eq!(details.has_coefficients, expected_type.is_some());
        assert_eq!(details.plane_tx_type, expected_type);
        assert!(!details.lossless);
    }
}

#[test]
fn inter_10bit_region_preserves_coded_edges_and_bounds_records() {
    let roi = rect(133, 0, 1, 1);
    let (samples, report) = decode(INTER_EDGE_422, 2, Some(request(1, Some(roi), 3)));
    assert_eq!(samples, decode(INTER_EDGE_422, 2, None).0);
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(report.records.len(), 3);
    for (record, unit) in transforms(&report) {
        let (native, coded, visible) = if unit.id.plane == PlaneId::Y {
            (rect(132, 0, 4, 8), rect(132, 0, 4, 8), rect(132, 0, 2, 8))
        } else {
            (rect(64, 0, 4, 16), rect(128, 0, 8, 16), rect(128, 0, 6, 16))
        };
        assert_eq!(unit.id.coded, native);
        assert_eq!(unit.luma_coded, coded);
        assert_eq!(record.visible, Some(visible));
    }

    let (_, report) = decode(INTER_EDGE_422, 2, Some(request(1, Some(roi), 2)));
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Truncated);
    assert_eq!(report.records.len(), 2);

    let padding = rect(134, 0, 2, 1);
    let (_, report) = decode(INTER_EDGE_422, 2, Some(request(1, Some(padding), 3)));
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(report.records.len(), 3);
    assert!(report.records.iter().all(|record| record.visible.is_some()));

    let outside = rect(1024, 0, 1, 1);
    let (_, report) = decode(INTER_EDGE_422, 2, Some(request(1, Some(outside), 0)));
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert!(report.records.is_empty());
}

#[test]
fn separate_chroma_tree_and_luma_tree_keep_distinct_lossless_units() {
    let roi = rect(0, 0, 1, 8);
    let (_, report) = decode(SEPARATE_TREES_422, 2, Some(request(0, Some(roi), 6)));
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(report.records.len(), 6);
    let mut selected_u = None;
    for (_, unit) in transforms(&report) {
        let (tree, block) = if unit.id.plane == PlaneId::Y {
            (BlockEvidenceTree::Luma, rect(0, 0, 4, 8))
        } else {
            (BlockEvidenceTree::Chroma, rect(0, 0, 8, 8))
        };
        assert_eq!(unit.id.block.tree, tree);
        assert_eq!(unit.id.block.coded, block);
        assert_eq!(unit.id.coded.x(), 0);
        assert_eq!(unit.id.coded.width(), 4);
        assert_eq!(unit.id.coded.height(), 4);
        assert!([0, 4].contains(&unit.id.coded.y()));
        if unit.id.plane == PlaneId::U && unit.id.coded.y() == 0 {
            selected_u = Some(unit.id);
        }
    }
    let selected = request(0, Some(roi), 1).with_transform_details(selected_u.unwrap());
    let (_, report) = decode(SEPARATE_TREES_422, 1, Some(selected));
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(report.records.len(), 1);
    let (_, unit) = transforms(&report).next().unwrap();
    assert_eq!(unit.id.plane, PlaneId::U);
    let details = unit.details.unwrap();
    assert!(details.has_coefficients);
    assert!(details.lossless);
    assert_eq!(details.plane_tx_type.map(TxType::av2_name), Some("IDTX"));
}

fn nonzero_grid(values: &[i32]) -> Vec<(usize, i32)> {
    values
        .iter()
        .copied()
        .enumerate()
        .filter(|&(_, value)| value != 0)
        .collect()
}

fn assert_quantization(
    coefficients: &BlockTransformCoefficients,
    bit_depth: BitDepth,
    qindex: u32,
    quantizer: u32,
    denominator: u32,
) {
    let quantization = coefficients.quantization.unwrap();
    assert_eq!(quantization.bit_depth, bit_depth);
    assert_eq!(quantization.qindex, qindex);
    assert_eq!(quantization.dc_quant, quantizer);
    assert_eq!(quantization.ac_quant, quantizer);
    assert_eq!(quantization.denominator, denominator);
    assert_eq!(quantization.matrix, None);
    assert!(!coefficients.use_tcq);
    assert!(!coefficients.forward_scan);
    assert_eq!(coefficients.secondary, None);
    assert_eq!(coefficients.domain, BlockCoefficientDomain::Plane);
}

#[test]
fn selected_8bit_coefficients_match_avm_and_fit_the_exact_byte_budget() {
    let coded = rect(64, 0, 64, 32);
    let block = BlockEvidenceId::new(1, BlockEvidenceTree::Shared, coded);
    let id = BlockTransformId::new(block, PlaneId::Y, coded);
    let budget = core::mem::size_of::<BlockEvidenceRecord>() + 8192;
    let select = |max_bytes| {
        BlockEvidenceRequest::new(|obu| obu.user_data == 0, None, 1, max_bytes)
            .with_transform_coefficients(id)
    };
    let (samples, report) = decode(TWO_TILES, 2, Some(select(budget)));
    assert_eq!(samples, decode(TWO_TILES, 1, None).0);
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(transforms(&report).next().unwrap().1.id, id);
    let coefficients = report.coefficients.unwrap();
    assert_eq!(coefficients.id, id);
    assert_eq!((coefficients.width, coefficients.height), (32, 32));
    assert_eq!(coefficients.quantized.len(), 1024);
    assert_eq!(coefficients.dequantized.len(), 1024);
    assert_eq!(coefficients.eob, 10);
    assert_eq!(
        &coefficients.scan.unwrap()[..10],
        &[0, 32, 1, 64, 33, 2, 96, 65, 34, 3]
    );
    assert_eq!(
        nonzero_grid(&coefficients.quantized),
        [(0, -1), (1, -11), (3, -1)]
    );
    assert_eq!(
        nonzero_grid(&coefficients.dequantized),
        [(0, -312), (1, -3432), (3, -312)]
    );
    assert_quantization(&coefficients, BitDepth::Eight, 192, 9984, 4);

    let (_, report) = decode(TWO_TILES, 2, Some(select(budget - 1)));
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Truncated);
    assert!(report.records.is_empty());
    assert_eq!(report.coefficients, None);

    let (_, report) = decode(
        TWO_TILES,
        2,
        Some(select(budget).with_cancellation(|| true)),
    );
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Cancelled);
    assert!(report.records.is_empty());
    assert_eq!(report.coefficients, None);
}

#[test]
fn selected_inter_edge_chroma_coefficients_match_avm_10bit() {
    let block = BlockEvidenceId::new(0, BlockEvidenceTree::Shared, rect(128, 0, 8, 16));
    let id = BlockTransformId::new(block, PlaneId::U, rect(64, 0, 4, 16));
    let selected = request(1, Some(rect(133, 0, 1, 1)), 1).with_transform_coefficients(id);
    let (samples, report) = decode(INTER_EDGE_422, 2, Some(selected));
    assert_eq!(samples, decode(INTER_EDGE_422, 2, None).0);
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(transforms(&report).next().unwrap().1.id, id);
    let coefficients = report.coefficients.unwrap();
    assert_eq!(coefficients.id, id);
    assert_eq!((coefficients.width, coefficients.height), (4, 16));
    assert_eq!(coefficients.quantized.len(), 64);
    assert_eq!(coefficients.dequantized.len(), 64);
    assert_eq!(coefficients.eob, 17);
    assert_eq!(
        &coefficients.scan.unwrap()[..17],
        &[0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 16, 13, 10, 7, 20, 17, 14]
    );
    assert_eq!(
        nonzero_grid(&coefficients.quantized),
        [(0, -18), (4, 1), (5, 1), (10, 1), (13, -1), (14, -1)]
    );
    assert_eq!(
        nonzero_grid(&coefficients.dequantized),
        [
            (0, -27072),
            (4, 1504),
            (5, 1504),
            (10, 1504),
            (13, -1504),
            (14, -1504)
        ]
    );
    assert_quantization(&coefficients, BitDepth::Ten, 198, 12032, 1);
}

#[test]
fn all_zero_payload_differs_from_no_payload() {
    let block_coded = rect(64, 0, 64, 32);
    let block = BlockEvidenceId::new(1, BlockEvidenceTree::Shared, block_coded);
    let id = BlockTransformId::new(block, PlaneId::U, rect(32, 0, 32, 16));
    let (_, report) = decode(
        TWO_TILES,
        1,
        Some(request(0, None, 1).with_transform_coefficients(id)),
    );
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    let coefficients = report.coefficients.unwrap();
    assert_eq!(coefficients.id, id);
    assert_eq!(coefficients.quantized, vec![0; 512]);
    assert_eq!(coefficients.dequantized, vec![0; 512]);
    assert_eq!(coefficients.eob, 0);
    assert_eq!(coefficients.scan, None);
    assert_eq!(coefficients.quantization, None);

    let (_, report) = decode(
        TWO_TILES,
        1,
        Some(request(0, None, 1).with_transform_details(id)),
    );
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(report.records.len(), 1);
    assert_eq!(report.coefficients, None);

    let excluded = request(0, Some(rect(0, 0, 1, 1)), 1).with_transform_coefficients(id);
    let (_, report) = decode(TWO_TILES, 1, Some(excluded));
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert!(report.records.is_empty());
    assert_eq!(report.coefficients, None);
}

#[test]
fn dc_only_matches_avm() {
    let coded = rect(0, 0, 64, 64);
    let id = BlockTransformId::new(
        BlockEvidenceId::new(0, BlockEvidenceTree::Shared, coded),
        PlaneId::Y,
        coded,
    );
    let (_, report) = decode(
        TWO_TILES,
        1,
        Some(request(0, None, 1).with_transform_coefficients(id)),
    );
    let coefficients = report.unwrap().coefficients.unwrap();
    assert_eq!(coefficients.eob, 1);
    assert_eq!(nonzero_grid(&coefficients.quantized), [(0, 38)]);
    assert_eq!(nonzero_grid(&coefficients.dequantized), [(0, 11856)]);
    assert_quantization(&coefficients, BitDepth::Eight, 192, 9984, 4);
}

fn capture_fixture_transform(
    input: &[u8],
    plane: PlaneId,
    coded: PlaneRect,
) -> BlockEvidenceReport {
    let roi = rect(coded.x(), coded.y(), 1, 1);
    let (_, report) = decode(input, 2, Some(request(0, Some(roi), 128)));
    let id = transforms(&report.unwrap())
        .find(|(_, unit)| unit.id.plane == plane && unit.id.coded == coded)
        .unwrap()
        .1
        .id;
    let (_, report) = decode(
        input,
        2,
        Some(request(0, Some(roi), 1).with_transform_coefficients(id)),
    );
    let report = report.unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(report.records.len(), 1);
    assert_eq!(report.coefficients.as_ref().unwrap().id, id);
    report
}

#[test]
fn secondary_transform_keeps_the_pre_ist_values_in_raster_order() {
    let input =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-ist-intra-128x128.ivf");
    let report = capture_fixture_transform(input, PlaneId::Y, rect(32, 32, 8, 8));
    let details = transforms(&report).next().unwrap().1.details.unwrap();
    assert_eq!(details.plane_tx_type, Some(TxType::DctDct));
    let coefficients = report.coefficients.unwrap();
    assert_eq!(coefficients.eob, 6);
    assert_eq!(&coefficients.scan.unwrap()[..6], &[0, 8, 1, 16, 9, 2]);
    assert_eq!(
        nonzero_grid(&coefficients.quantized),
        [(1, -2), (2, -1), (8, 1)]
    );
    assert_eq!(
        nonzero_grid(&coefficients.dequantized),
        [(1, -132), (2, -66), (8, 66)]
    );
    let secondary = coefficients.secondary.unwrap();
    assert_eq!(secondary.sec_tx_type, 3);
}

#[test]
fn cross_component_pair_labels_both_slots_even_when_v_is_zero() {
    let input =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-cctx-444-intra-128x128.ivf");
    let coded = rect(12, 4, 4, 4);
    let report = capture_fixture_transform(input, PlaneId::U, coded);
    let details = transforms(&report).next().unwrap().1.details.unwrap();
    assert_eq!(details.plane_tx_type, Some(TxType::AdstDct));
    let u = report.coefficients.unwrap();
    assert_eq!(u.domain, BlockCoefficientDomain::CrossComponent { mode: 5 });
    assert_eq!(u.eob, 6);
    assert_eq!(&u.scan.unwrap()[..6], &[0, 4, 1, 8, 5, 2]);
    assert_eq!(
        nonzero_grid(&u.quantized),
        [(0, 2), (1, -2), (2, 1), (4, -1), (5, 1)]
    );
    assert_eq!(
        nonzero_grid(&u.dequantized),
        [(0, 55), (1, -55), (2, 28), (4, -28), (5, 28)]
    );

    let report = capture_fixture_transform(input, PlaneId::V, coded);
    let v = report.coefficients.unwrap();
    assert_eq!(v.id.block, u.id.block);
    assert_eq!(v.domain, u.domain);
    assert_eq!(v.eob, 0);
    assert_eq!(v.quantized, vec![0; 16]);
    assert_eq!(v.dequantized, vec![0; 16]);
    assert_eq!(v.quantization, None);
}

#[test]
fn forward_skip_reports_the_coded_suffix() {
    let input =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-fsc-intra-128x128.ivf");
    let coded = rect(24, 0, 8, 8);
    let report = capture_fixture_transform(input, PlaneId::Y, coded);
    let details = transforms(&report).next().unwrap().1.details.unwrap();
    assert_eq!(details.plane_tx_type.map(TxType::av2_name), Some("IDTX"));
    let coefficients = report.coefficients.unwrap();
    assert!(coefficients.forward_scan);
    assert_eq!(coefficients.eob, 6);
    assert_eq!(
        &coefficients.scan.unwrap()[58..64],
        &[61, 54, 47, 62, 55, 63]
    );
    assert_eq!(
        nonzero_grid(&coefficients.quantized),
        [(54, 12), (55, 12), (61, 12), (62, 12)]
    );
    assert_eq!(
        nonzero_grid(&coefficients.dequantized),
        [(54, 792), (55, 792), (61, 792), (62, 792)]
    );
}

#[test]
fn reports_derived_dc_parity_and_tcq_levels() {
    let parity =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-parity-intra-128x128.ivf");
    let parity = capture_fixture_transform(parity, PlaneId::Y, rect(84, 80, 4, 16))
        .coefficients
        .unwrap();
    assert!(parity.dc_parity_hidden);
    assert!(!parity.use_tcq);
    assert_eq!(parity.quantized[0], 3);
    assert_eq!(parity.dequantized[0], 198);

    let tcq = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-2sb-deblock-intra-128x64-q100.ivf"
    );
    let tcq = capture_fixture_transform(tcq, PlaneId::Y, rect(0, 0, 32, 32))
        .coefficients
        .unwrap();
    assert!(tcq.use_tcq);
    assert!(!tcq.dc_parity_hidden);
    assert_eq!(tcq.eob, 5);
    assert_eq!(tcq.quantization.unwrap().denominator, 4);
    assert_eq!(
        nonzero_grid(&tcq.quantized),
        [(0, -223), (1, 2), (32, 2), (33, 52)]
    );
    assert_eq!(
        nonzero_grid(&tcq.dequantized),
        [(0, -4906), (1, 44), (32, 44), (33, 1144)]
    );
}

#[test]
fn reports_effective_matrix_even_when_a_user_matrix_is_present() {
    let built_in =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-qm-intra-64x64.ivf");
    let user =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-output-user-qm-64x64.ivf");
    for (input, coded, qindex, quantizer, level, quant, dequant) in [
        (
            built_in.as_slice(),
            rect(0, 0, 64, 64),
            200,
            12544,
            8,
            -1,
            -392,
        ),
        (user.as_slice(), rect(0, 0, 32, 64), 80, 392, 6, -59, -722),
    ] {
        let coefficients = capture_fixture_transform(input, PlaneId::Y, coded)
            .coefficients
            .unwrap();
        assert_eq!(nonzero_grid(&coefficients.quantized), [(0, quant)]);
        assert_eq!(nonzero_grid(&coefficients.dequantized), [(0, dequant)]);
        let quantization = coefficients.quantization.unwrap();
        assert_eq!(quantization.qindex, qindex);
        assert_eq!(quantization.dc_quant, quantizer);
        assert_eq!(quantization.ac_quant, quantizer);
        assert_eq!(quantization.denominator, 4);
        let matrix = quantization.matrix.unwrap();
        assert_eq!(matrix.level, level);
        assert!(!matrix.user_defined);
    }
}

/// AVM a1ec4594 encodes one synthetic 8x8 4:2:0 frame with one thread and no lag.
/// Start with `AVM_USAGE_GOOD_QUALITY`. Select `AVM_Q` with both quantizer limits at 20.
/// Set both QM limits to 2. Use `AV2E_SET_USER_DEFINED_QMATRIX` at level 2.
/// The samples are `64 + (31*x + 17*y + 3*plane) % 139`.
/// The matrix weights are `32 + (13*i + 3*shape + 5*plane) % 32`.
/// Index shapes as 8x8, 8x4, 4x8 and planes as Y, U, V. Use an IVF time base of 1/1.
/// IVF SHA-256 `795f2b3840aef32ba87eeaad22ebda257b91fd4cd0b3cefb86fb68dba8ea8b14`.
#[test]
fn selected_small_transform_uses_the_user_matrix() {
    let input = include_bytes!("data/block-evidence-user-qm.ivf");
    let report = capture_fixture_transform(input, PlaneId::U, rect(0, 0, 4, 4));
    let details = transforms(&report).next().unwrap().1.details.unwrap();
    assert_eq!(details.plane_tx_type, Some(TxType::DctAdst));
    let coefficients = report.coefficients.unwrap();
    assert_eq!(coefficients.eob, 16);
    assert_eq!(
        coefficients.scan.unwrap(),
        [0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15]
    );
    assert_eq!(
        coefficients.quantized,
        [
            57, -33, -41, -24, -4, -63, 18, -30, -51, 25, -18, 7, 15, -20, 13, -7
        ]
    );
    assert_eq!(
        coefficients.dequantized,
        [
            577, -569, -641, -336, -58, -811, 203, -289, -516, 431, -281, 98, 218, -258, 146, -67
        ]
    );
    let quantization = coefficients.quantization.unwrap();
    assert_eq!(quantization.qindex, 20);
    assert_eq!(quantization.dc_quant, 70);
    assert_eq!(quantization.ac_quant, 70);
    assert_eq!(quantization.denominator, 1);
    let matrix = quantization.matrix.unwrap();
    assert_eq!(matrix.level, 2);
    assert!(matrix.user_defined);
}
