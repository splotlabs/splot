// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Restriction switches (§ 5.4.3, § 5.4.8, § 5.4.10) and the preset
//! `reduced_tx_set` (§ 5.18.2) in coded streams. The decoder's own parse
//! shows that no block uses a restricted tool.

use core::num::NonZeroUsize;

use super::*;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::sequence::{SequenceHeader, parse_sequence_header};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_decode::block_evidence::{
    BlockEvidenceData, BlockEvidenceId, BlockEvidenceKind, BlockEvidenceRecord,
    BlockEvidenceRequest, BlockEvidenceStatus, BlockTransformId,
};
use splot_decode::{DataProps, Decoder, DecoderSettings, ObuLocation, PlaneId, SendStatus};

/// A 64x64 panning stream of 3 frames at `preset` with `overrides`.
fn stream(
    (chroma, bit_depth): (PixelFormat, BitDepth),
    preset: u8,
    overrides: &[(&str, &str)],
    threads: usize,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(64, 64, bit_depth, preset, 120);
    config.chroma = chroma;
    config.threads = splot_parallel::ThreadCount::from(threads);
    config.preset_overrides = overrides
        .iter()
        .map(|&(field, value)| (field.to_owned(), value.to_owned()))
        .collect();
    encode_frames(&config, (0..3).map(|index| panning_frame(&config, index)))
}

/// The first sequence header of `units`.
fn sequence(units: &[Vec<u8>]) -> SequenceHeader {
    let stream = units.concat();
    let parsed = parse_annex_b_obus_partial(&stream);
    let obu = parsed
        .obus
        .iter()
        .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
        .unwrap();
    parse_sequence_header(&mut BitReader::new(obu.payload, ByteOffset::new(0))).unwrap()
}

/// The decoder's records of every coded frame of `units` under `request`.
fn evidence(units: &[Vec<u8>], request: BlockEvidenceRequest) -> Vec<BlockEvidenceRecord> {
    let (mut decoder, capture) =
        Decoder::new_with_block_evidence(DecoderSettings::default(), request).unwrap();
    for (token, unit) in units.iter().enumerate() {
        let props = DataProps::default().with_user_data(token as u64);
        while decoder.send_data(unit, props).unwrap() == SendStatus::Full {
            while decoder.get_picture().unwrap().is_some() {}
        }
    }
    decoder.drain();
    while decoder.get_picture().unwrap().is_some() {}
    let report = capture.take().unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    report.records
}

/// A request for the records of `kind` of the first `frames` coded frames.
fn request(frames: usize, kind: BlockEvidenceKind) -> BlockEvidenceRequest {
    BlockEvidenceRequest::new(|_: ObuLocation| true, None, 1 << 20, 1 << 28)
        .with_frame_limit(NonZeroUsize::new(frames).unwrap())
        .with_kind(kind)
}

/// The luma `(width, height)` of every coding block of `units`.
fn block_sizes(units: &[Vec<u8>]) -> Vec<(usize, usize)> {
    evidence(units, request(units.len(), BlockEvidenceKind::Blocks))
        .iter()
        .filter(|record| matches!(record.data, BlockEvidenceData::Block(_)))
        .map(|record| (record.id.coded.width(), record.id.coded.height()))
        .collect()
}

/// The transform units of every coding block of `units`.
fn transforms(units: &[Vec<u8>]) -> Vec<BlockTransformId> {
    evidence(units, request(units.len(), BlockEvidenceKind::Transforms))
        .into_iter()
        .filter_map(|record| match record.data {
            BlockEvidenceData::Transform(transform) => Some(transform.id),
            _ => None,
        })
        .collect()
}

/// The `PlaneTxType` of each transform of the key frame of `units` in
/// `planes` that codes coefficients.
fn key_frame_types(units: &[Vec<u8>], planes: &[PlaneId]) -> Vec<TxType> {
    let key = &units[..1];
    transforms(key)
        .into_iter()
        .filter(|id| planes.contains(&id.plane))
        .filter_map(|id| {
            let selected = request(1, BlockEvidenceKind::Transforms).with_transform_details(id);
            evidence(key, selected)
                .iter()
                .find_map(|record| match record.data {
                    BlockEvidenceData::Transform(transform) if transform.id == id => {
                        transform.details.and_then(|details| details.plane_tx_type)
                    }
                    _ => None,
                })
        })
        .collect()
}

/// Luma blocks 4 samples wide or high with a long side of at least 16, and
/// the number of luma transforms each codes: § 5.20.6.3 splits them on one
/// axis only, into two or (`tx_2or3_partition_type`) four units.
fn one_axis_luma_splits(units: &[Vec<u8>]) -> Vec<usize> {
    let mut counts = BTreeMap::<(usize, usize, usize, usize), usize>::new();
    for id in transforms(units) {
        let BlockEvidenceId { coded, .. } = id.block;
        let (w, h) = (coded.width(), coded.height());
        if id.plane == PlaneId::Y && w.min(h) == 4 && w.max(h) >= 16 {
            *counts.entry((coded.x(), coded.y(), w, h)).or_default() += 1;
        }
    }
    counts.into_values().collect()
}

const YUV420: (PixelFormat, BitDepth) = (PixelFormat::Yuv420, BitDepth::Eight);
const YUV444: (PixelFormat, BitDepth) = (PixelFormat::Yuv444, BitDepth::Ten);

/// Asserts that `overrides` decode to the encoder recon with the same bytes on
/// 1, 2 and 8 threads, and returns the stream.
fn checked_stream(
    format: (PixelFormat, BitDepth),
    preset: u8,
    overrides: &[(&str, &str)],
) -> Vec<Vec<u8>> {
    let case = format!("{format:?} p{preset} {overrides:?}");
    let (units, recon) = stream(format, preset, overrides, 1);
    assert_same_samples(&decode(&units.concat()), &recon, &case);
    for threads in [2, 8] {
        assert!(
            stream(format, preset, overrides, threads).0 == units,
            "{case}: {threads} threads"
        );
    }
    units
}

/// `max_pb_aspect_ratio` codes `MaxPbAspectRatio` and the decoder parses no
/// block beyond it; the full set codes a longer block.
#[test]
fn max_pb_aspect_ratio_limits_every_coded_block() {
    let off = block_sizes(&stream(YUV420, 3, &[], 1).0);
    assert!(off.iter().any(|&(w, h)| w > 2 * h || h > 2 * w), "{off:?}");
    for (format, ratio) in [(YUV420, 2), (YUV444, 4)] {
        let value = ratio.to_string();
        let units = checked_stream(format, 3, &[("max_pb_aspect_ratio", &value)]);
        let partition = sequence(&units).partition.unwrap();
        assert!(partition.reduce_pb_aspect_ratio);
        assert_eq!(partition.max_pb_aspect_ratio, ratio as u32);
        let sizes = block_sizes(&units);
        assert!(
            sizes.iter().all(|&(w, h)| w <= ratio * h && h <= ratio * w),
            "{format:?}: {sizes:?}"
        );
    }
}

/// `chroma_dct_only` codes `enable_chroma_dctonly` and every chroma transform
/// the decoder parses is `DCT_DCT`; the full set codes another type.
#[test]
fn chroma_dct_only_codes_dct_chroma_transforms() {
    let chroma = [PlaneId::U, PlaneId::V];
    let off = key_frame_types(&stream(YUV420, 6, &[], 1).0, &chroma);
    assert!(off.iter().any(|&t| t != TxType::DctDct), "{off:?}");
    for format in [YUV420, YUV444] {
        let units = checked_stream(format, 6, &[("chroma_dct_only", "1")]);
        let tq = sequence(&units).transform_quant_entropy.unwrap();
        assert!(tq.enable_chroma_dctonly);
        let types = key_frame_types(&units, &chroma);
        assert!(!types.is_empty(), "{format:?}");
        assert!(
            types.iter().all(|&t| t == TxType::DctDct),
            "{format:?}: {types:?}"
        );
    }
}

/// `reduced_tx_part_set` codes the flag and the decoder parses no four-way
/// split of a one-axis luma block; the full set codes one.
#[test]
fn reduced_tx_part_set_codes_no_four_way_one_axis_split() {
    let off = one_axis_luma_splits(&stream(YUV420, 0, &[], 1).0);
    assert!(off.iter().any(|&count| count == 4), "{off:?}");
    let units = checked_stream(YUV420, 0, &[("reduced_tx_part_set", "1")]);
    assert!(
        sequence(&units)
            .transform_quant_entropy
            .unwrap()
            .reduced_tx_part_set
    );
    let counts = one_axis_luma_splits(&units);
    assert!(counts.iter().all(|&count| count <= 2), "{counts:?}");
}

/// An explicit `reduced_tx_set` 2 codes every intra luma block of the key
/// frame with `DCT_DCT` at a preset that searches types; the derived 0 codes
/// another type. 1 and 3 decode to the encoder recon.
#[test]
fn reduced_tx_set_limits_the_luma_types() {
    let luma = [PlaneId::Y];
    let off = key_frame_types(&stream(YUV420, 6, &[], 1).0, &luma);
    assert!(off.iter().any(|&t| t != TxType::DctDct), "{off:?}");
    let units = checked_stream(YUV420, 6, &[("reduced_tx_set", "2")]);
    let types = key_frame_types(&units, &luma);
    assert!(types.iter().all(|&t| t == TxType::DctDct), "{types:?}");
    for set in ["1", "3"] {
        checked_stream(YUV420, 6, &[("reduced_tx_set", set)]);
    }
}

/// `ccso_unit_sb` and `gdf_unit_sb` code the unit flags, and their streams
/// decode to the encoder recon: the decoder takes the units from the parsed
/// flags.
#[test]
fn unit_size_switches_reach_the_sequence_header() {
    for format in [YUV420, YUV444] {
        let units = checked_stream(format, 8, &[("ccso_unit_sb", "1"), ("gdf_unit_sb", "1")]);
        let filter = sequence(&units).filter.unwrap();
        assert!(filter.enable_ccso && filter.ccso_unit_matches_sb_size);
        assert!(filter.enable_gdf && filter.gdf_unit_matches_sb_size);
    }
}

/// Values the syntax cannot carry are refused with their section.
#[test]
fn restriction_switches_refuse_forms_the_syntax_cannot_carry() {
    for (overrides, field) in [
        (&[("max_pb_aspect_ratio", "3")][..], "max_pb_aspect_ratio"),
        (
            &[("gdf_unit_sb", "1"), ("max_block_log2", "7")],
            "gdf_unit_sb",
        ),
        (
            &[("gdf_unit_sb", "1"), ("gdf_candidates", "0")],
            "gdf_unit_sb",
        ),
        (
            &[("ccso_unit_sb", "1"), ("ccso_candidates", "0")],
            "ccso_unit_sb",
        ),
    ] {
        let mut preset = PRESETS[8];
        for &(name, value) in overrides {
            preset.set(name, value).unwrap();
        }
        let error = preset.validate().unwrap_err();
        assert!(
            matches!(&error, crate::preset::PresetError::OutOfRange { field: f, allowed, .. }
                if *f == field && allowed.contains("§ 5.4.")),
            "{overrides:?}: {error}"
        );
    }
}

/// AVM decodes the restricted streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_restricted_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (format, preset, overrides) in [
        (YUV420, 3, &[("max_pb_aspect_ratio", "2")][..]),
        (YUV444, 3, &[("max_pb_aspect_ratio", "4")]),
        (YUV420, 6, &[("chroma_dct_only", "1")]),
        (YUV444, 6, &[("chroma_dct_only", "1")]),
        (YUV420, 0, &[("reduced_tx_part_set", "1")]),
        (YUV420, 6, &[("reduced_tx_set", "1")]),
        (YUV420, 6, &[("reduced_tx_set", "3")]),
        (YUV444, 8, &[("ccso_unit_sb", "1"), ("gdf_unit_sb", "1")]),
    ] {
        let (units, recon) = stream(format, preset, overrides, 1);
        let name = format!("restrict{preset}{:?}{}", format.0, overrides[0].0);
        let avm = avm_decode(&avmdec, &units, (64, 64), format.1, &name);
        assert!(avm == recon, "{name}: AVM output differs");
    }
}
