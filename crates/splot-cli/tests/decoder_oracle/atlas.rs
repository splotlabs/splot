// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::sync::Arc;

use splot_core::stream_reader::TemporalUnitReader;
use splot_core::types::ExtendedLayerId;
use splot_decode::{
    AtlasCompositionError, AtlasCompositor, AtlasLayout, BitDepth, DataProps, DecodeContext,
    DecodeLimitThreshold, DecodeLimits, DecodeOptions, DecodeRuntimeConfig, Decoder,
    DecoderSettings, ExtendedLayerSelection, Picture, SendStatus, ThreadCount,
};

use super::{append_raw, sha256_hex};

const NATIVE_8: [(u8, &str); 2] = [
    (
        3,
        "a76f8a3d2bd55c7af06aa183f96977f144498c31587aa62a8bdcbe4534cb3fbe",
    ),
    (
        7,
        "24f5d1b318ed0085b36d6371c3ff331fb4ec329b1ca252e93b40232b1e9f1d4a",
    ),
];
const NATIVE_10: [(u8, &str); 2] = [
    (
        2,
        "a19c49839344102ea1dc266b86c2943e47ae2746145326bfa01f14f0819f507b",
    ),
    (
        9,
        "366d1de38bc4faa23e6b65e81e9cd07ecfefd5f0c3ed163d459c4c561a2baac5",
    ),
];

struct ShortReads<'a>(&'a [u8]);

impl Read for ShortReads<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(1 + self.0.len() % 11);
        self.0.read(&mut buf[..len])
    }
}

fn decode_timed(bytes: &[u8], pts: &[i64], threads: ThreadCount) -> Vec<Picture> {
    let mut decoder =
        Decoder::new(DecoderSettings::default().with_threads(threads)).expect("decoder");
    let mut units = TemporalUnitReader::new(ShortReads(bytes));
    let mut pictures = Vec::new();
    for &time in pts {
        let unit = units
            .next_temporal_unit()
            .expect("framing")
            .expect("temporal unit");
        let props = DataProps::default().with_pts(time);
        while decoder.send_data(unit.bytes, props).expect("accepted TU") == SendStatus::Full {
            while let Some(picture) = decoder.get_picture().expect("picture") {
                pictures.push(picture);
            }
        }
        while let Some(picture) = decoder.get_picture().expect("picture") {
            pictures.push(picture);
        }
    }
    assert!(units.next_temporal_unit().expect("EOF").is_none());
    decoder.drain();
    while let Some(picture) = decoder.get_picture().expect("drain") {
        pictures.push(picture);
    }
    decoder.flush();
    assert!(decoder.get_picture().expect("flushed output").is_none());
    pictures
}

fn global_layout(picture: &Picture) -> &Arc<AtlasLayout> {
    picture
        .atlas_associations()
        .iter()
        .find(|association| association.scope.is_global())
        .and_then(|association| association.layout.as_ref())
        .expect("global layout")
}

#[test]
fn native_and_composite_output_match_independent_goldens() {
    let vectors = [
        (
            "multistream-side-by-side",
            BitDepth::Eight,
            "85b9b854163552e0c00ee6d0acc5b5ce36356da01f4e3b4d0357a303626cc249",
        ),
        (
            "alpha-paired",
            BitDepth::Eight,
            "20a8e51779232b000421884c4b7e4b8f8fa74780d7c1fa93f9f1259dfe159d2d",
        ),
        (
            "multistream-side-by-side",
            BitDepth::Ten,
            "748bcce6e28d05c5bd9348e5dc37a159dd324eeeaca674c45f2d9d88a599c313",
        ),
        (
            "alpha-paired",
            BitDepth::Ten,
            "2286b9dc5c613cf1da66d598a6a67bc2da6ad37095006fd3a19ce4124f49adb4",
        ),
    ];
    for (name, depth, composite_sha256) in vectors {
        // In the nonmonotonic 10-bit stream, presentation timestamps differ from TU indices.
        let (bits, pts, native) = match depth {
            BitDepth::Eight => (8, &[0, 1, 2, 3][..], &NATIVE_8),
            BitDepth::Ten => (10, &[0, 3, 1, 2, 4, 5, 8, 6, 7, 9][..], &NATIVE_10),
        };
        let filename = format!("syn-atlas-{name}-{bits}bit.obu");
        let path = super::repo_root()
            .join("tests/conformance/vectors/valid")
            .join(&filename);
        let bytes = std::fs::read(path).expect("atlas stream");
        for width in [1, 3, 12] {
            let threads = ThreadCount::from(width);
            let context =
                DecodeContext::new(DecodeRuntimeConfig::new(threads)).expect("decode context");
            let pictures = decode_timed(&bytes, pts, threads);
            let mut at_time = BTreeMap::<i64, Vec<&Picture>>::new();
            for picture in &pictures {
                at_time
                    .entry(picture.props().pts)
                    .or_default()
                    .push(picture);
            }
            assert_eq!(at_time.len(), pts.len());
            let mut output = Vec::new();
            let mut compositor = AtlasCompositor::new(DecodeLimits::default());
            for (time, mut group) in at_time {
                assert_eq!(group.len(), 2);
                group.reverse();
                let canvas = compositor
                    .compose(global_layout(group[0]), time, &group)
                    .expect("compose");
                assert_eq!((canvas.width(), canvas.height()), (144, 64));
                assert_eq!(canvas.pts(), time);
                assert_eq!(canvas.bit_depth(), depth);
                canvas.write_raw(&mut output).expect("write composite");
            }
            assert_eq!(
                sha256_hex(&output),
                composite_sha256,
                "{filename}, {width} threads"
            );
            for &(layer, native_sha256) in native {
                let mut after_compose = Vec::new();
                for picture in pictures
                    .iter()
                    .filter(|p| p.extended_layer_id().get() == layer)
                {
                    append_raw(picture, &mut after_compose);
                }
                assert_eq!(sha256_hex(&after_compose), native_sha256);
                let options = DecodeOptions::default().with_extended_layer_selection(
                    ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(layer)),
                );
                let mut batch = Vec::new();
                context
                    .decode_raw_bytes(&bytes, options, &mut batch)
                    .expect("batch decode");
                assert_eq!(batch, after_compose);
            }
        }
    }
}

#[test]
fn missing_duplicate_stale_and_oversized_groups_return_typed_errors() {
    let bytes = include_bytes!(
        "../../../../tests/conformance/vectors/valid/syn-atlas-alpha-paired-8bit.obu"
    );
    let pics = decode_timed(bytes, &[0, 1, 2, 3], ThreadCount::from(1usize));
    let first = global_layout(&pics[0]);
    let mut compositor = AtlasCompositor::new(DecodeLimits::default());
    assert!(matches!(
        compositor.compose(first, 0, &[&pics[0]]),
        Err(AtlasCompositionError::MissingPicture(7))
    ));
    assert!(matches!(
        compositor.compose(first, 0, &[&pics[0], &pics[0]]),
        Err(AtlasCompositionError::UnexpectedPicture(3))
    ));
    assert!(matches!(
        compositor.compose(first, 0, &[&pics[0], &pics[3]]),
        Err(AtlasCompositionError::TimestampMismatch(7))
    ));
    assert!(matches!(
        compositor.compose(first, 2, &[&pics[4], &pics[5]]),
        Err(AtlasCompositionError::StaleLayout(3))
    ));
    let mut limited = AtlasCompositor::new(
        DecodeLimits::default().with_max_decoded_frame_bytes(DecodeLimitThreshold::Max(100)),
    );
    assert!(matches!(
        limited.compose(first, 0, &[&pics[0], &pics[1]]),
        Err(AtlasCompositionError::Limit(_))
    ));
    assert!(compositor.compose(first, 0, &[&pics[0], &pics[1]]).is_ok());
}
