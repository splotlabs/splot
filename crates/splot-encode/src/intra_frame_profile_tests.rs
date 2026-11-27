// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streams with a requested `seq_profile_idc` (§ 5.4.1, Annex A.2).

use super::*;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::sequence::{ProfileIdc, parse_sequence_header};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

/// The legal pairs below today's lowest profile and the Configurable profile.
const CASES: [(PixelFormat, BitDepth, u8); 5] = [
    (PixelFormat::Yuv420, BitDepth::Eight, 1),
    (PixelFormat::Yuv420, BitDepth::Eight, 2),
    (PixelFormat::Yuv420, BitDepth::Eight, 3),
    (PixelFormat::Yuv420, BitDepth::Eight, 31),
    (PixelFormat::Yuv444, BitDepth::Ten, 31),
];

/// A 72x40 panning stream at preset 6 (mini-GOPs, filters on).
fn profile_stream(
    (chroma, bit_depth): (PixelFormat, BitDepth),
    profile: Option<u8>,
    threads: usize,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(72, 40, bit_depth, 6, 150);
    config.chroma = chroma;
    config.profile = profile.map(ProfileIdc::from_bits);
    config.threads = splot_parallel::ThreadCount::from(threads);
    encode_frames(&config, (0..7).map(|index| panning_frame(&config, index)))
}

/// The `seq_profile_idc` of every sequence header in `units`, and the
/// payloads of the other OBUs.
fn split_profile(units: &[Vec<u8>]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let stream = units.concat();
    let parsed = parse_annex_b_obus_partial(&stream);
    assert!(parsed.error.is_none());
    let (mut profiles, mut rest) = (Vec::new(), Vec::new());
    for obu in &parsed.obus {
        if obu.header.obu_type == ObuType::SequenceHeader {
            let mut reader = BitReader::new(obu.payload, ByteOffset::new(0));
            let header = parse_sequence_header(&mut reader).unwrap();
            profiles.push(header.general.seq_profile_idc.get());
        } else {
            rest.push(obu.payload.to_vec());
        }
    }
    (profiles, rest)
}

/// Profiles 1, 2, 3 and 31 on 4:2:0 8-bit and 31 on 4:4:4 10-bit reach
/// every sequence header, decode to the encoder recon with the same bytes
/// on 1, 2 and 8 threads, and change no other OBU and no sample.
#[test]
fn requested_profiles_decode_to_the_encoder_recon() {
    for (chroma, bit_depth, profile) in CASES {
        let case = format!("{chroma:?} {bit_depth:?} profile {profile}");
        let (units, recon) = profile_stream((chroma, bit_depth), Some(profile), 1);
        let (profiles, rest) = split_profile(&units);
        assert!(
            !profiles.is_empty() && profiles.iter().all(|&p| p == profile),
            "{case}"
        );
        assert_same_samples(&decode(&units.concat()), &recon, &case);
        for threads in [2, 8] {
            let other = profile_stream((chroma, bit_depth), Some(profile), threads).0;
            assert!(other == units, "{case}: {threads} threads");
        }
        let (plain, plain_recon) = profile_stream((chroma, bit_depth), None, 1);
        let (lowest, plain_rest) = split_profile(&plain);
        let expected = if chroma == PixelFormat::Yuv444 { 4 } else { 0 };
        assert!(lowest.iter().all(|&p| p == expected), "{case}: {lowest:?}");
        assert_eq!(plain_recon, recon, "{case}: the profile changes the recon");
        assert!(
            plain_rest == rest,
            "{case}: the profile changes another OBU"
        );
    }
}

/// A reserved profile, or one whose Table A.1 set does not hold the chroma
/// format, is refused at configuration.
#[test]
fn profiles_that_do_not_allow_the_chroma_format_are_refused() {
    let mut config = config(72, 40, BitDepth::Eight, 6, 150);
    for (chroma, refused, accepted) in [
        (
            PixelFormat::Yuv420,
            &[5, 17, 30][..],
            &[0, 1, 2, 3, 4, 31][..],
        ),
        (PixelFormat::Yuv422, &[0, 1, 2, 4, 5], &[3, 31]),
        (PixelFormat::Yuv444, &[0, 1, 2, 3, 5], &[4, 31]),
    ] {
        config.chroma = chroma;
        for &value in refused {
            config.profile = Some(ProfileIdc::from_bits(value));
            assert!(
                matches!(config.validate(), Err(Error::InvalidProfile { profile, .. }) if profile.get() == value),
                "{chroma:?} {value}"
            );
        }
        for &value in accepted {
            config.profile = Some(ProfileIdc::from_bits(value));
            assert!(config.validate().is_ok(), "{chroma:?} {value}");
        }
    }
}

/// AVM decodes the requested-profile streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_requested_profiles_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth, profile) in CASES {
        let (units, recon) = profile_stream((chroma, bit_depth), Some(profile), 1);
        let name = format!("profile{chroma:?}{bit_depth:?}{profile}");
        let avm = avm_decode(&avmdec, &units, (72, 40), bit_depth, &name);
        assert!(avm == recon, "{name}: AVM output differs");
    }
}
