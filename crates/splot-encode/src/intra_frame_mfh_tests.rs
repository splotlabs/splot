// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streams whose frame headers refer to a multi-frame header (§ 5.7,
//! § 5.18.2) under [`Config::multi_frame_header`].

use super::*;
use crate::preset::PresetParams;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{FrameHeaderCore, FrameType as CoreFrameType};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

/// The OBU types of `stream`, each frame OBU with its `cur_mfh_id`
/// (§ 5.18.2), which follows `is_first_tile_group` in a tile group (§ 5.19).
pub(super) fn mfh_ids(stream: &[u8]) -> Vec<(ObuType, Option<u32>)> {
    let parsed = parse_annex_b_obus_partial(stream);
    assert!(parsed.error.is_none());
    let id = |payload: &[u8], tile_group: bool| {
        let mut reader = BitReader::new(payload, ByteOffset::new(0));
        if tile_group {
            reader.read_bit().unwrap();
        }
        reader.read_uvlc().unwrap()
    };
    parsed
        .obus
        .iter()
        .map(|obu| {
            let kind = obu.header.obu_type;
            let cur_mfh_id = match kind {
                ObuType::ClosedLoopKey | ObuType::RegularTileGroup => Some(id(obu.payload, true)),
                ObuType::RegularTip | ObuType::RegularSef => Some(id(obu.payload, false)),
                _ => None,
            };
            (kind, cur_mfh_id)
        })
        .collect()
}

/// Asserts that `ids` codes one multi-frame header OBU right after each
/// sequence header and none elsewhere, and `cur_mfh_id = 1` in every frame;
/// returns the number of key frames.
pub(super) fn assert_mfh_references(ids: &[(ObuType, Option<u32>)], case: &str) -> usize {
    let mfh_at: Vec<usize> = (0..ids.len())
        .filter(|&i| ids[i].0 == ObuType::MultiFrameHeader)
        .collect();
    let sequence_at: Vec<usize> = (0..ids.len())
        .filter(|&i| ids[i].0 == ObuType::SequenceHeader)
        .map(|i| i + 1)
        .collect();
    assert_eq!(mfh_at, sequence_at, "{case}");
    assert!(
        ids.iter().all(|&(_, id)| id.is_none_or(|id| id == 1)),
        "{case}: {ids:?}"
    );
    mfh_at.len()
}

/// Preset 6 with TIP output frames, and intra-only frames with
/// `intra_only`.
fn mfh_preset(intra_only: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        tip: 1,
        tip_output: 200,
        intra_only,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// A panning mini-GOP stream with key frames every `keyint` frames: its
/// units, the encoder recon and the frame headers.
fn mfh_stream(
    (chroma, bit_depth): (PixelFormat, BitDepth),
    (keyint, intra_only): (u32, u8),
    multi_frame_header: bool,
    threads: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>) {
    let mut config = config(72, 40, bit_depth, 6, 150);
    config.chroma = chroma;
    config.keyint = core::num::NonZeroU32::new(keyint).unwrap();
    config.multi_frame_header = multi_frame_header;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, mfh_preset(intra_only)).unwrap();
    let (units, expected, cores, _) =
        super::carry::planned_stream_of(&config, coder, panning_frame, 150);
    (units, expected, cores)
}

/// 4:2:0 8-bit and 4:4:4 10-bit streams with a second key frame, TIP output
/// frames, show-existing frames and (with `intra_only`) intra-only frames
/// refer to the multi-frame header of every key frame unit, decode to the
/// encoder recon, and code the same bytes on 1, 2 and 8 threads. Without
/// the option no frame refers to one, and the recon does not change.
#[test]
fn mfh_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    let mut tip_outputs = 0;
    for format in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        for (keyint, intra_only) in [(8, 0), (30, 1)] {
            let case = format!("{format:?} intra_only {intra_only}");
            let (units, expected, cores) = mfh_stream(format, (keyint, intra_only), true, 1);
            let ids = mfh_ids(&units);
            let keys = assert_mfh_references(&ids, &case);
            assert_eq!(keys, if intra_only == 0 { 2 } else { 1 }, "{case}");
            assert!(
                ids.iter().any(|&(kind, _)| kind == ObuType::RegularSef),
                "{case}"
            );
            tip_outputs += ids
                .iter()
                .filter(|&&(kind, _)| kind == ObuType::RegularTip)
                .count();
            let intra = cores
                .iter()
                .filter(|core| core.frame_type == Some(CoreFrameType::IntraOnly))
                .count();
            assert_eq!(
                intra > 0,
                intra_only > 0,
                "{case}: {intra} intra-only frames"
            );
            assert_same_samples(&decode(&units), &expected, &case);
            for threads in [2, 8] {
                let other = mfh_stream(format, (keyint, intra_only), true, threads).0;
                assert!(other == units, "{case}: {threads} threads");
            }
            let (plain, plain_recon, _) = mfh_stream(format, (keyint, intra_only), false, 1);
            assert!(
                mfh_ids(&plain)
                    .iter()
                    .all(|&(kind, id)| kind != ObuType::MultiFrameHeader && id != Some(1)),
                "{case}: off"
            );
            assert_eq!(plain_recon, expected, "{case}: the recon changes");
        }
    }
    assert!(tip_outputs > 0, "no TIP output frame");
}

/// AVM decodes multi-frame header streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_mfh_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for format in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        for setup in [(8, 0), (30, 1)] {
            let (units, expected, _) = mfh_stream(format, setup, true, 1);
            let name = format!("mfh{:?}{:?}{}", format.0, format.1, setup.1);
            let avm = avm_decode(&avmdec, &[units], (72, 40), format.1, &name);
            assert!(avm == expected, "{name}: AVM output differs");
        }
    }
}
