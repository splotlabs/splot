// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const TWO_STREAMS_8BIT: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-msdo-reproducible-two-streams-8bit.obu"
);
const TWO_STREAMS_10BIT: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-msdo-reproducible-two-streams-10bit.obu"
);
const FOUR_STREAMS: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-msdo-reproducible-four-streams-obu-order.obu"
);
const OPS_RAP: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-msdo-reproducible-ops-random-access.obu"
);

#[test]
fn independently_encoded_msdo_components_match_avm() {
    for (fixture, depth, timeline, digest) in [
        (
            TWO_STREAMS_8BIT,
            BitDepth::Eight,
            &[3, 7, 3, 7, 3, 7, 3, 7][..],
            "eeb71b784ebb1a91727c489bd894065c85b1ff1c5fcfd6660c2deb3b14c4856e",
        ),
        (
            TWO_STREAMS_10BIT,
            BitDepth::Ten,
            &[2, 9, 2, 9, 2, 2, 9, 9, 2, 9][..],
            "d8599f59de7943c3e3574a955a066e7cd20dbbb06f84102c66cb898aff0f7489",
        ),
        (
            FOUR_STREAMS,
            BitDepth::Eight,
            &[1, 5, 12, 20, 1, 5, 12, 20, 1, 5, 12, 20][..],
            "dd776ef10bc346a53bcf66e595ae426989c3fa0a9cfbeac6b53e9597ccd60d38",
        ),
    ] {
        for threads in [1usize, 3, 12] {
            let pictures = stream_decode(
                fixture,
                &DecoderSettings::default().with_threads(ThreadCount::from(threads)),
            )
            .unwrap();
            let observed: Vec<_> = pictures
                .iter()
                .map(|picture| picture.extended_layer_id().get())
                .collect();
            assert_eq!(observed, timeline);
            assert!(pictures.iter().all(|picture| picture.bit_depth() == depth));
            assert_eq!(sha256_hex(&raw_all_and_by_xlayer(&pictures).0), digest);
        }
    }
}

#[test]
fn independently_encoded_ops_components_match_avm_after_random_access() {
    for (op, count, digest) in [
        (
            OperatingPointSelection::None,
            12,
            "2e831b82d6636d4e6cbdcb333f1575785c7b12816d38309d827a056bbd15140a",
        ),
        (
            OperatingPointSelection::Global {
                ops_id: 0,
                op_index: 0,
            },
            6,
            "ab2c9fcbb2a58f759bb898529d832eb0121c41ec8c793535f44119495f2f3c29",
        ),
        (
            OperatingPointSelection::Global {
                ops_id: 0,
                op_index: 1,
            },
            6,
            "b637d3cfb5c6729d922c4879431eb14f79df6cebe2a1b27de1e352e9badbf419",
        ),
        (
            OperatingPointSelection::Global {
                ops_id: 1,
                op_index: 0,
            },
            9,
            "387d9cc42c4f5019d0da760e945f38ce48450adb2fed16d1f3c72560ac8fa1e4",
        ),
        (
            OperatingPointSelection::Local {
                xlayer_id: ExtendedLayerId::from_bits(3),
                ops_id: 2,
                op_index: 0,
            },
            9,
            "23b839d8523c966235aad5c50ef9a35a0a128a124d6c8937b999175d2f2a76fd",
        ),
    ] {
        for threads in [1usize, 3, 12] {
            let pictures = stream_decode(
                OPS_RAP,
                &DecoderSettings::default()
                    .with_threads(ThreadCount::from(threads))
                    .with_operating_point_selection(op),
            )
            .unwrap();
            assert_eq!(pictures.len(), count);
            assert_eq!(sha256_hex(&raw_all_and_by_xlayer(&pictures).0), digest);
        }
    }
}
