// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Chroma prediction evidence from independently encoded fixtures.

#![expect(clippy::unwrap_used)]

use splot_core::stream_reader::TemporalUnitReader;
use splot_decode::block_evidence::{
    BlockChromaAngleDelta, BlockChromaPrediction, BlockEvidenceData, BlockEvidenceKind,
    BlockEvidenceRecord, BlockEvidenceRequest, BlockEvidenceStatus, BlockEvidenceTree,
    BlockLumaPrediction, BlockPrediction, ChromaIntraMode, IntraMode,
};
use splot_decode::{DataProps, Decoder, DecoderSettings, PlaneRect, SendStatus};

const D67_FOLLOW: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-luma-d67-chroma-follow-intra-64x64.ivf"
);

const D67_EXPLICIT: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d67-intra-64x64.ivf"
);

fn capture_blocks(input: &[u8]) -> Vec<BlockEvidenceRecord> {
    let settings = DecoderSettings::default();
    let request =
        BlockEvidenceRequest::new(|_| true, None, 8, 4096).with_kind(BlockEvidenceKind::Blocks);
    let (mut decoder, capture) = Decoder::new_with_block_evidence(settings, request).unwrap();
    let mut reader = TemporalUnitReader::new(input);
    let unit = reader.next_temporal_unit().unwrap().unwrap();
    assert_eq!(
        decoder.send_data(unit.bytes, DataProps::default()).unwrap(),
        SendStatus::Accepted
    );
    decoder.drain();
    assert!(decoder.get_picture().unwrap().is_some());

    let report = capture.take().unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    report.records
}

#[test]
fn chroma_tree_preserves_d67_follow_and_delta_source() {
    // AVM a1ec4594 reports a 64x64 D67 leaf in each tree and zero angle deltas.
    let rect = PlaneRect::new(0, 0, 64, 64).unwrap();
    let mut saw_luma = false;
    let mut saw_chroma = false;
    for record in capture_blocks(D67_FOLLOW) {
        let BlockEvidenceData::Block(facts) = record.data else {
            continue;
        };
        let BlockPrediction::Intra(intra) = facts.prediction else {
            continue;
        };
        assert_eq!(record.id.coded, rect);
        match record.id.tree {
            BlockEvidenceTree::Luma => {
                assert!(intra.chroma.is_none());
                assert!(matches!(
                    intra.luma,
                    Some(BlockLumaPrediction::Mode {
                        mode: IntraMode::D67,
                        direction: Some(direction),
                        dpcm: None,
                        ..
                    }) if direction.angle_delta == 0 && direction.angle_degrees == 67
                ));
                saw_luma = true;
            }
            BlockEvidenceTree::Chroma => {
                assert!(intra.luma.is_none());
                assert!(matches!(
                    intra.chroma,
                    Some(BlockChromaPrediction::Mode {
                        mode: ChromaIntraMode::D67Follow,
                        angle_delta: Some(BlockChromaAngleDelta::FromLuma(0)),
                        dpcm: None,
                        ..
                    })
                ));
                saw_chroma = true;
            }
            _ => {}
        }
    }
    assert!(saw_luma && saw_chroma);
}

#[test]
fn independent_chroma_prediction_reports_a_derived_delta() {
    let records = capture_blocks(D67_EXPLICIT);
    assert!(records.iter().any(|record| {
        let BlockEvidenceData::Block(facts) = record.data else {
            return false;
        };
        let BlockPrediction::Intra(intra) = facts.prediction else {
            return false;
        };
        matches!(
            intra.chroma,
            Some(BlockChromaPrediction::Mode {
                mode: ChromaIntraMode::D67,
                angle_delta: Some(BlockChromaAngleDelta::Derived(0)),
                dpcm: None,
                ..
            })
        )
    }));
}
