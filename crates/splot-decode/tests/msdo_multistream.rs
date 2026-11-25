// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Multistream MSDO lifecycle, OPS selection, unaligned random access, and CMVS reset tests.

#![expect(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, Read};

use splot_core::annexb::parse_annex_b_obus;
use splot_core::headers::sequence::{LcrId, SequenceHeaderId};
use splot_core::obu::{ObuHeader, ParsedObu, PayloadStatus};
use splot_core::stream_reader::TemporalUnitReader;
use splot_core::types::{
    EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId,
};
use splot_core::write::{BitWriter, write_annexb_obu, write_complete_obu};
use splot_decode::{
    BitDepth, DataProps, DecodeContext, DecodeError, DecodeLimitThreshold, DecodeLimits,
    DecodeOptions, DecodeRuntimeConfig, Decoder, DecoderSettings, EmbeddedLayerSelection,
    ExtendedLayerSelection, FrameDelay, OperatingPointSelection, Picture, PictureSamples, PlaneId,
    SendStatus, TemporalLayerSelection, ThreadCount,
};

const MSDO_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-msdo-two-streams-8bit.obu");
const MSDO_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-msdo-two-streams-10bit.obu");
const MSDO_4STREAM: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-msdo-four-streams-obu-order.obu");
const MSDO_OPS_RA: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-msdo-ops-random-access.obu");

const SHA_MSDO_8BIT_ALL: &str = "6cdd94998ec0e812fb8c8b029221f95527872abee3c1a9a30dbbaf3e94e5d97b";
const SHA_MSDO_8BIT_X3: &str = "a76f8a3d2bd55c7af06aa183f96977f144498c31587aa62a8bdcbe4534cb3fbe";
const SHA_MSDO_8BIT_X7: &str = "24f5d1b318ed0085b36d6371c3ff331fb4ec329b1ca252e93b40232b1e9f1d4a";
const SHA_MSDO_10BIT_ALL: &str = "754993eb0ce0034850dbe99d1ffa1e7da358b3ce7fa34b32319f53a927e1b62b";
const SHA_MSDO_4STREAM_ALL: &str =
    "45460122ee439e322b620ed69612d173aaf5522658461245edbd9e7d0b4f76be";
const SHA_OPS_ALL: &str = "abcb36ae31ba11246e8445e87f90fd6ffddab5c0eeaf15fdffbe2d53d83b6830";
const SHA_OPS_0_0: &str = "2ec69bc4694d071cad578706823dee7cf1ab9a50c9feee6478bd3f9ca2a1d2c3";
const SHA_OPS_0_1: &str = "1f7bb640c50012cfbbdebbcaf3fce4c2c0442a31899bcb850ef5221153a26c49";
const SHA_OPS_1_0: &str = "5dd12c346465a01afad48df7d8e54aa24f66954c57351064e4144c12607df789";
const SHA_OPS_LOCAL_3_2_0: &str =
    "7eafb4d548549b91d3b30300b0cd32f3ec708a0ea2d3489e9cc9950588625542";

fn sha256_hex(data: &[u8]) -> String {
    const ROUND_K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let mut padded = data.to_vec();
    let bit_len = (padded.len() as u64).wrapping_mul(8);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for block in padded.chunks_exact(64) {
        let mut sched = [0u32; 64];
        for i in 0..16 {
            sched[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = sched[i - 15].rotate_right(7)
                ^ sched[i - 15].rotate_right(18)
                ^ (sched[i - 15] >> 3);
            let s1 = sched[i - 2].rotate_right(17)
                ^ sched[i - 2].rotate_right(19)
                ^ (sched[i - 2] >> 10);
            sched[i] = sched[i - 16]
                .wrapping_add(s0)
                .wrapping_add(sched[i - 7])
                .wrapping_add(s1);
        }
        let [
            mut va,
            mut vb,
            mut vc,
            mut vd,
            mut ve,
            mut vf,
            mut vg,
            mut vh,
        ] = state;
        for i in 0..64 {
            let s1 = ve.rotate_right(6) ^ ve.rotate_right(11) ^ ve.rotate_right(25);
            let ch = (ve & vf) ^ ((!ve) & vg);
            let t1 = vh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(ROUND_K[i])
                .wrapping_add(sched[i]);
            let s0 = va.rotate_right(2) ^ va.rotate_right(13) ^ va.rotate_right(22);
            let maj = (va & vb) ^ (va & vc) ^ (vb & vc);
            let t2 = s0.wrapping_add(maj);
            vh = vg;
            vg = vf;
            vf = ve;
            ve = vd.wrapping_add(t1);
            vd = vc;
            vc = vb;
            vb = va;
            va = t1.wrapping_add(t2);
        }
        for (slot, delta) in state.iter_mut().zip([va, vb, vc, vd, ve, vf, vg, vh]) {
            *slot = slot.wrapping_add(delta);
        }
    }
    let mut out = String::with_capacity(64);
    for word in state {
        write!(out, "{word:08x}").unwrap();
    }
    out
}

struct ChunkedReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    chunk_size: usize,
}

impl<'a> ChunkedReader<'a> {
    const fn new(bytes: &'a [u8], chunk_size: usize) -> Self {
        Self {
            bytes,
            offset: 0,
            chunk_size,
        }
    }
}

impl Read for ChunkedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.bytes[self.offset..];
        if remaining.is_empty() || buf.is_empty() {
            return Ok(0);
        }
        let n = remaining.len().min(buf.len()).min(self.chunk_size);
        buf[..n].copy_from_slice(&remaining[..n]);
        self.offset += n;
        Ok(n)
    }
}

fn context(threads: usize, frame_delay: usize) -> DecodeContext {
    DecodeContext::new(
        DecodeRuntimeConfig::new(ThreadCount::from(threads))
            .with_frame_delay(FrameDelay::from(frame_delay)),
    )
    .unwrap()
}

fn temporal_units(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut reader = TemporalUnitReader::new(bytes);
    let mut out = Vec::new();
    while let Some(unit) = reader.next_temporal_unit().unwrap() {
        out.push(unit.bytes.to_vec());
    }
    out
}

fn stream_decode(bytes: &[u8], settings: &DecoderSettings) -> Result<Vec<Picture>, DecodeError> {
    let mut decoder = Decoder::new(*settings)?;
    let mut pictures = Vec::new();
    for (idx, unit) in temporal_units(bytes).iter().enumerate() {
        let props = DataProps::default().with_pts(i64::try_from(idx).unwrap());
        loop {
            match decoder.send_data(unit, props)? {
                SendStatus::Accepted => break,
                SendStatus::Full => {
                    while let Some(pic) = decoder.get_picture()? {
                        pictures.push(pic);
                    }
                }
            }
        }
        while let Some(pic) = decoder.get_picture()? {
            pictures.push(pic);
        }
    }
    decoder.drain();
    while let Some(pic) = decoder.get_picture()? {
        pictures.push(pic);
    }
    Ok(pictures)
}

fn append_picture_raw(pic: &Picture, out: &mut Vec<u8>) {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        match pic.plane(plane) {
            Some(PictureSamples::Eight(p)) => {
                for row in p.visible_rows() {
                    out.extend_from_slice(row);
                }
            }
            Some(PictureSamples::Ten(p)) => {
                for sample in p.visible_rows().flatten() {
                    out.extend_from_slice(&sample.to_le_bytes());
                }
            }
            None => {}
        }
    }
}

fn raw_all_and_by_xlayer(pics: &[Picture]) -> (Vec<u8>, BTreeMap<u8, Vec<u8>>) {
    let mut all = Vec::new();
    let mut by_x: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
    for pic in pics {
        append_picture_raw(pic, &mut all);
        append_picture_raw(pic, by_x.entry(pic.extended_layer_id().get()).or_default());
    }
    (all, by_x)
}

fn write_obu(out: &mut Vec<u8>, header: ObuHeader, payload: &[u8]) {
    let mut bw = BitWriter::new();
    write_annexb_obu(&mut bw, &header, payload).unwrap();
    out.extend_from_slice(&bw.into_bytes());
}

fn write_parsed(out: &mut Vec<u8>, header: ObuHeader, parsed: &ParsedObu) {
    let mut full = BitWriter::new();
    write_complete_obu(&mut full, &header, parsed, &[]).unwrap();
    let full_bytes = full.into_bytes();
    let mut framed = BitWriter::new();
    framed.write_leb128(full_bytes.len() as u32).unwrap();
    framed.write_le(&full_bytes).unwrap();
    out.extend_from_slice(&framed.into_bytes());
}

fn make_msdo_obu(ext_header: bool, lvl: u8, doh_flag: bool, subs: &[(u8, u8, u8, u8)]) -> Vec<u8> {
    let mut body = BitWriter::new();
    body.write_bits_u8((subs.len() - 2) as u8, 3).unwrap();
    body.write_bits_u8(2, 5).unwrap(); // multistream_profile_idc = 2
    body.write_bits_u8(lvl, 5).unwrap();
    body.write_bits_u8(0, 1).unwrap();
    body.write_flag(true).unwrap();
    for &(x, p, l, t) in subs {
        body.write_bits_u8(x, 5).unwrap();
        body.write_bits_u8(p, 5).unwrap();
        body.write_bits_u8(l, 5).unwrap();
        body.write_bits_u8(t, 1).unwrap();
    }
    body.write_flag(doh_flag).unwrap();
    let rem = body.bit_len() % 8;
    let nb = if rem == 0 { 8 } else { 8 - rem };
    body.write_trailing_bits(nb).unwrap();
    let payload = body.into_bytes();
    let header = ObuHeader {
        has_header_extension: ext_header,
        obu_type: ObuType::Msdo,
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: GLOBAL_XLAYER_ID,
        header_size_bytes: if ext_header { 2 } else { 1 },
    };
    let mut out = Vec::new();
    write_obu(&mut out, header, &payload);
    out
}

#[test]
fn msdo_routing_header_and_layer_scope() {
    for prefix in [&[0x01u8, 0x50][..], &[0x02u8, 0xD0, 0x1F][..]] {
        let mut stream = prefix.to_vec();
        stream.extend_from_slice(&[0x01, 0x0A]);
        let err = context(1, 1)
            .decode_hash_report_bytes(&stream, DecodeOptions::DEFAULT)
            .unwrap_err();
        assert!(
            !matches!(
                &err,
                DecodeError::UnsupportedStructure { unsupported }
                    if unsupported.reason().as_str() == "invalid_layer_scope"
            ),
            "valid MSDO header {prefix:02x?} unexpectedly hit InvalidLayerScope: {err:?}"
        );
    }

    for bad_msdo in [
        &[0x02u8, 0xD0, 0x03][..],
        &[0x02u8, 0xD0, 0x9F][..],
        &[0x02u8, 0xD0, 0x5F][..],
        &[0x02u8, 0xD0, 0x00][..],
    ] {
        let err = context(1, 1)
            .decode_hash_report_bytes(bad_msdo, DecodeOptions::DEFAULT)
            .unwrap_err();
        assert!(
            matches!(
                &err,
                DecodeError::UnsupportedStructure { unsupported }
                    if unsupported.reason().as_str() == "invalid_layer_scope"
                        && matches!(unsupported.spec_section(), "6.2.2" | "6.6")
            ),
            "expected InvalidLayerScope for {bad_msdo:02x?}, got {err:?}"
        );
    }
}

#[test]
fn avm_digest_parity_and_timelines_across_all_four_msdo_vectors() {
    for threads in [1usize, 3, 12] {
        let pics1 = stream_decode(
            MSDO_8BIT,
            &DecoderSettings::default().with_threads(ThreadCount::from(threads)),
        )
        .unwrap();
        let timeline1: Vec<(u8, usize, usize, BitDepth)> = pics1
            .iter()
            .map(|p| {
                (
                    p.extended_layer_id().get(),
                    p.width(),
                    p.height(),
                    p.bit_depth(),
                )
            })
            .collect();
        assert_eq!(
            timeline1,
            vec![
                (3, 64, 64, BitDepth::Eight),
                (7, 80, 64, BitDepth::Eight),
                (3, 64, 64, BitDepth::Eight),
                (7, 80, 64, BitDepth::Eight),
                (3, 64, 64, BitDepth::Eight),
                (7, 80, 64, BitDepth::Eight),
                (3, 64, 64, BitDepth::Eight),
                (7, 80, 64, BitDepth::Eight),
            ]
        );
        let (all1, by_x1) = raw_all_and_by_xlayer(&pics1);
        assert_eq!(sha256_hex(&all1), SHA_MSDO_8BIT_ALL);
        assert_eq!(sha256_hex(&by_x1[&3]), SHA_MSDO_8BIT_X3);
        assert_eq!(sha256_hex(&by_x1[&7]), SHA_MSDO_8BIT_X7);

        let pics2 = stream_decode(
            MSDO_10BIT,
            &DecoderSettings::default().with_threads(ThreadCount::from(threads)),
        )
        .unwrap();
        assert_eq!(pics2.len(), 10);
        assert!(pics2.iter().all(|p| p.bit_depth() == BitDepth::Ten));
        let (all2, _) = raw_all_and_by_xlayer(&pics2);
        assert_eq!(sha256_hex(&all2), SHA_MSDO_10BIT_ALL);

        let pics3 = stream_decode(
            MSDO_4STREAM,
            &DecoderSettings::default().with_threads(ThreadCount::from(threads)),
        )
        .unwrap();
        let timeline3: Vec<(u8, usize, usize)> = pics3
            .iter()
            .map(|p| (p.extended_layer_id().get(), p.width(), p.height()))
            .collect();
        assert_eq!(
            timeline3,
            vec![
                (1, 64, 64),
                (5, 80, 64),
                (12, 64, 80),
                (20, 96, 64),
                (1, 64, 64),
                (5, 80, 64),
                (12, 64, 80),
                (20, 96, 64),
                (1, 64, 64),
                (5, 80, 64),
                (12, 64, 80),
                (20, 96, 64),
            ]
        );
        let (all3, _) = raw_all_and_by_xlayer(&pics3);
        assert_eq!(sha256_hex(&all3), SHA_MSDO_4STREAM_ALL);

        for (op, expected_count, expected_sha) in [
            (OperatingPointSelection::None, 12usize, SHA_OPS_ALL),
            (
                OperatingPointSelection::Global {
                    ops_id: 0,
                    op_index: 0,
                },
                6,
                SHA_OPS_0_0,
            ),
            (
                OperatingPointSelection::Global {
                    ops_id: 0,
                    op_index: 1,
                },
                6,
                SHA_OPS_0_1,
            ),
            (
                OperatingPointSelection::Global {
                    ops_id: 1,
                    op_index: 0,
                },
                9,
                SHA_OPS_1_0,
            ),
            (
                OperatingPointSelection::Local {
                    xlayer_id: ExtendedLayerId::from_bits(3),
                    ops_id: 2,
                    op_index: 0,
                },
                9,
                SHA_OPS_LOCAL_3_2_0,
            ),
        ] {
            let pics4 = stream_decode(
                MSDO_OPS_RA,
                &DecoderSettings::default()
                    .with_threads(ThreadCount::from(threads))
                    .with_operating_point_selection(op),
            )
            .unwrap();
            assert_eq!(pics4.len(), expected_count, "op={op:?}");
            let (raw4, _) = raw_all_and_by_xlayer(&pics4);
            assert_eq!(sha256_hex(&raw4), expected_sha, "op={op:?}");
        }
    }

    let mut raw = Vec::new();
    context(1, 2)
        .decode_raw_bytes(
            MSDO_OPS_RA,
            DecodeOptions::default().with_operating_point_selection(
                OperatingPointSelection::Global {
                    ops_id: 0,
                    op_index: 1,
                },
            ),
            &mut raw,
        )
        .unwrap();
    assert_eq!(sha256_hex(&raw), SHA_OPS_0_1);

    for multi_op in [
        OperatingPointSelection::Global {
            ops_id: 1,
            op_index: 0,
        },
        OperatingPointSelection::Local {
            xlayer_id: ExtendedLayerId::from_bits(3),
            ops_id: 2,
            op_index: 0,
        },
    ] {
        let err = context(1, 2)
            .decode_raw_bytes(
                MSDO_OPS_RA,
                DecodeOptions::default().with_operating_point_selection(multi_op),
                io::sink(),
            )
            .unwrap_err();
        assert!(matches!(
            err,
            DecodeError::UnsupportedFeature { ref unsupported }
                if unsupported.reason() == "multiple_extended_layers_flat_output"
        ));
    }
}

fn drain_until_error(decoder: &mut Decoder) -> Option<DecodeError> {
    decoder.drain();
    loop {
        match decoder.get_picture() {
            Ok(Some(_)) => {}
            Ok(None) => return None,
            Err(err) => return Some(err),
        }
    }
}

#[test]
fn streaming_chunks_1b_7b_and_flush_recovery() {
    for (bytes, op) in [
        (MSDO_8BIT, OperatingPointSelection::None),
        (
            MSDO_OPS_RA,
            OperatingPointSelection::Global {
                ops_id: 1,
                op_index: 0,
            },
        ),
        (
            MSDO_OPS_RA,
            OperatingPointSelection::Local {
                xlayer_id: ExtendedLayerId::from_bits(3),
                ops_id: 2,
                op_index: 0,
            },
        ),
    ] {
        let opts = DecodeOptions::default().with_operating_point_selection(op);
        let expected = context(1, 2).decode_hash_report_bytes(bytes, opts).unwrap();
        for threads in [1usize, 3, 12] {
            for chunk_size in [1usize, 7] {
                let actual = context(threads, 2)
                    .decode_hash_report_reader(ChunkedReader::new(bytes, chunk_size), opts)
                    .unwrap();
                assert_eq!(
                    actual.frames, expected.frames,
                    "threads={threads}, chunk={chunk_size}"
                );
            }
        }
    }

    let mut decoder = Decoder::new(DecoderSettings::default()).unwrap();
    let mut bad = temporal_units(MSDO_8BIT)[0].clone();
    bad.truncate(4);
    assert_eq!(
        decoder.send_data(&bad, DataProps::default()).unwrap(),
        SendStatus::Accepted
    );
    let err = drain_until_error(&mut decoder).unwrap();
    assert!(
        matches!(err, DecodeError::MalformedSource { .. }),
        "got {err:?}"
    );

    decoder.flush();
    let mut recovered = Vec::new();
    for (idx, unit) in temporal_units(MSDO_8BIT).iter().enumerate() {
        assert_eq!(
            decoder
                .send_data(unit, DataProps::default().with_pts(idx as i64))
                .unwrap(),
            SendStatus::Accepted
        );
        while let Some(pic) = decoder.get_picture().unwrap() {
            recovered.push(pic);
        }
    }
    decoder.drain();
    while let Some(pic) = decoder.get_picture().unwrap() {
        recovered.push(pic);
    }
    let (raw, _) = raw_all_and_by_xlayer(&recovered);
    assert_eq!(sha256_hex(&raw), SHA_MSDO_8BIT_ALL);
}

#[test]
fn unaligned_rap_pre_skip_and_cmvs_transition() {
    let tus_ra = temporal_units(MSDO_OPS_RA);
    let sliced: Vec<u8> = tus_ra[2..].concat();
    let pics = stream_decode(&sliced, &DecoderSettings::default()).unwrap();
    let x3_cnt = pics
        .iter()
        .filter(|p| p.extended_layer_id().get() == 3)
        .count();
    let x7_cnt = pics
        .iter()
        .filter(|p| p.extended_layer_id().get() == 7)
        .count();
    assert_eq!((x3_cnt, x7_cnt), (2, 4));

    let pics_inherited = stream_decode(
        &sliced,
        &DecoderSettings::default().with_operating_point_selection(
            OperatingPointSelection::Global {
                ops_id: 1,
                op_index: 0,
            },
        ),
    )
    .unwrap();
    let x3_inherited = pics_inherited
        .iter()
        .filter(|p| p.extended_layer_id().get() == 3)
        .count();
    let x7_inherited = pics_inherited
        .iter()
        .filter(|p| p.extended_layer_id().get() == 7)
        .count();
    assert_eq!((x3_inherited, x7_inherited), (2, 2));

    let tus_8b = temporal_units(MSDO_8BIT);
    let mut cmvs = Vec::new();
    for (ti, tu) in tus_8b.iter().enumerate() {
        let new_msdo = make_msdo_obu(
            false,
            u8::from(ti >= 2),
            true,
            &[(3, 2, 1, 0), (7, 2, 1, 0)],
        );
        for env in parse_annex_b_obus(tu).unwrap() {
            if env.header.obu_type == ObuType::Msdo {
                let updated = parse_annex_b_obus(&new_msdo).unwrap();
                write_obu(&mut cmvs, updated[0].header, updated[0].payload);
            } else {
                write_obu(&mut cmvs, env.header, env.payload);
            }
        }
    }
    let pics_cmvs = stream_decode(&cmvs, &DecoderSettings::default()).unwrap();
    let (raw_cmvs, _) = raw_all_and_by_xlayer(&pics_cmvs);
    assert_eq!(pics_cmvs.len(), 8);
    assert_eq!(sha256_hex(&raw_cmvs), SHA_MSDO_8BIT_ALL);
}

#[test]
fn multistream_sequence_ends_before_single_stream_cvs() {
    let units = temporal_units(MSDO_8BIT);
    let mut single_stream = Vec::new();
    let new_xlayer = ExtendedLayerId::from_bits(9);
    for envelope in parse_annex_b_obus(&units[0]).unwrap() {
        let mut header = envelope.header;
        if header.extended_layer_id.get() == 3 {
            header.extended_layer_id = new_xlayer;
        } else if header.obu_type != ObuType::TemporalDelimiter {
            continue;
        }
        write_obu(&mut single_stream, header, envelope.payload);
    }
    let msdo_10bit_units = temporal_units(MSDO_10BIT);
    let unused_lcr = parse_annex_b_obus(&msdo_10bit_units[0])
        .unwrap()
        .into_iter()
        .find(|obu| {
            obu.header.obu_type == ObuType::LayerConfigurationRecord
                && obu.header.extended_layer_id.is_global()
        })
        .unwrap();
    let global_lcr_only: &[u8] = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-xlayer3-7-global-lcr-isolation.obu"
    );
    let msdo_then_single = "64fdd83bafcdf7ceaf9b87d848b460f1f4ec7aa52d51dd10eaf0a2e6d4bfc6b4";
    for (prior, available_lcr, avm_sha) in [
        (MSDO_8BIT, None, msdo_then_single),
        (MSDO_8BIT, Some(unused_lcr), msdo_then_single),
        (
            global_lcr_only,
            None,
            "ee5ee2e55a7d69117ab67ea6df4c5360de5ef1d23db63dbe788ea9e5ef4ff0c2",
        ),
    ] {
        let mut combined = prior.to_vec();
        for envelope in parse_annex_b_obus(&single_stream).unwrap() {
            write_obu(&mut combined, envelope.header, envelope.payload);
            if envelope.header.obu_type == ObuType::TemporalDelimiter
                && let Some(lcr) = available_lcr
            {
                write_obu(&mut combined, lcr.header, lcr.payload);
            }
        }
        let pictures = stream_decode(&combined, &DecoderSettings::default()).unwrap();
        let (raw, _) = raw_all_and_by_xlayer(&pictures);
        assert_eq!(pictures.len(), 9);
        assert_eq!(sha256_hex(&raw), avm_sha);
        assert_eq!(pictures.last().unwrap().extended_layer_id(), new_xlayer);
        let report = context(3, 2)
            .decode_hash_report_bytes(&combined, DecodeOptions::default())
            .unwrap();
        assert_eq!(report.frames.len(), 9);
    }
}

#[test]
fn standalone_cvs_reuses_available_sequence_after_cmvs_end() {
    let units = temporal_units(MSDO_8BIT);
    for stage_unused_sequence in [false, true] {
        let mut combined = MSDO_8BIT.to_vec();
        for envelope in parse_annex_b_obus(&units[0]).unwrap() {
            if envelope.header.obu_type == ObuType::TemporalDelimiter
                || (envelope.header.obu_type == ObuType::ClosedLoopKey
                    && envelope.header.extended_layer_id.get() == 3)
            {
                write_obu(&mut combined, envelope.header, envelope.payload);
            } else if stage_unused_sequence
                && envelope.header.extended_layer_id.get() == 3
                && let PayloadStatus::Parsed(ParsedObu::SequenceHeader(mut header)) =
                    envelope.payload_status().unwrap()
            {
                header.general.seq_header_id = SequenceHeaderId::try_new(1).unwrap();
                write_parsed(
                    &mut combined,
                    envelope.header,
                    &ParsedObu::SequenceHeader(header),
                );
            }
        }
        let pictures = stream_decode(&combined, &DecoderSettings::default()).unwrap();
        let (raw, _) = raw_all_and_by_xlayer(&pictures);
        assert_eq!(pictures.len(), 9);
        assert_eq!(
            sha256_hex(&raw),
            "64fdd83bafcdf7ceaf9b87d848b460f1f4ec7aa52d51dd10eaf0a2e6d4bfc6b4"
        );
        assert_eq!(pictures.last().unwrap().extended_layer_id().get(), 3);
        let report = context(3, 2)
            .decode_hash_report_bytes(&combined, DecodeOptions::default())
            .unwrap();
        assert_eq!(report.frames.len(), 9);
    }
}

#[test]
fn standalone_cvs_reuses_available_mfh_after_cmvs_end() {
    let prior: &[u8] = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-xlayer3-7-global-lcr-isolation.obu"
    );
    let units = temporal_units(prior);
    let mut combined = prior.to_vec();
    let xlayer = ExtendedLayerId::from_bits(7);
    for envelope in parse_annex_b_obus(&units[0]).unwrap() {
        if envelope.header.extended_layer_id == xlayer
            && let PayloadStatus::Parsed(ParsedObu::SequenceHeader(mut sequence)) =
                envelope.payload_status().unwrap()
        {
            sequence.general.seq_lcr_id = LcrId::from_bits(0);
            write_parsed(
                &mut combined,
                envelope.header,
                &ParsedObu::SequenceHeader(sequence.clone()),
            );
            sequence.general.seq_header_id = SequenceHeaderId::try_new(0).unwrap();
            write_parsed(
                &mut combined,
                envelope.header,
                &ParsedObu::SequenceHeader(sequence),
            );
        } else if envelope.header.obu_type == ObuType::TemporalDelimiter
            || (envelope.header.extended_layer_id == xlayer
                && envelope.header.obu_type == ObuType::ClosedLoopKey)
        {
            write_obu(&mut combined, envelope.header, envelope.payload);
        }
    }
    let pictures = stream_decode(&combined, &DecoderSettings::default()).unwrap();
    let (raw, _) = raw_all_and_by_xlayer(&pictures);
    assert_eq!(
        sha256_hex(&raw),
        "b5d63982ae20876398411909697cd8299bea23ab6677b699ebc617b0b825e9c5"
    );
    assert_eq!(pictures.last().unwrap().extended_layer_id(), xlayer);
    let report = context(3, 2)
        .decode_hash_report_bytes(&combined, DecodeOptions::default())
        .unwrap();
    assert_eq!(report.frames.len(), 9);
}

#[test]
fn local_lcr_shadows_the_available_global_lcr_at_cmvs_end() {
    let units = temporal_units(MSDO_10BIT);
    let global_lcr = parse_annex_b_obus(&units[0])
        .unwrap()
        .into_iter()
        .find(|obu| {
            obu.header.obu_type == ObuType::LayerConfigurationRecord
                && obu.header.extended_layer_id.is_global()
        })
        .unwrap();
    let embedded: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-mlayer2-crossref-10bit.obu");
    let xlayer = ExtendedLayerId::from_bits(2);
    for keep_local in [true, false] {
        let mut combined = MSDO_8BIT.to_vec();
        for (index, envelope) in parse_annex_b_obus(embedded)
            .unwrap()
            .into_iter()
            .enumerate()
        {
            let mut header = envelope.header;
            if !header.extended_layer_id.is_global() {
                if !keep_local && header.obu_type == ObuType::LayerConfigurationRecord {
                    continue;
                }
                header.extended_layer_id = xlayer;
                header.has_header_extension = true;
                header.header_size_bytes = 2;
            }
            write_obu(&mut combined, header, envelope.payload);
            if index == 0 {
                write_obu(&mut combined, global_lcr.header, global_lcr.payload);
            }
        }
        let pictures = stream_decode(&combined, &DecoderSettings::default());
        let report = context(3, 2).decode_hash_report_bytes(&combined, DecodeOptions::default());
        if keep_local {
            let pictures = pictures.unwrap();
            let (raw, _) = raw_all_and_by_xlayer(&pictures);
            assert_eq!(pictures.len(), 14);
            assert_eq!(pictures.last().unwrap().extended_layer_id(), xlayer);
            assert_eq!(
                sha256_hex(&raw),
                "217f4167763d3c7e4b572ff6992ad7bcae87042aeac00b34e02cb7dac07804ac"
            );
            assert_eq!(report.unwrap().frames.len(), 14);
        } else {
            for error in [pictures.unwrap_err(), report.unwrap_err()] {
                assert!(
                    matches!(
                        error,
                        DecodeError::MalformedSource { ref issue }
                            if issue.spec_section() == Some("7.3.2")
                    ),
                    "got {error:?}"
                );
            }
        }
    }
}

#[test]
fn first_unit_order_uses_frames_outside_the_temporal_layer_selection() {
    let units = temporal_units(MSDO_OPS_RA);
    let rap = parse_annex_b_obus(&units[2]).unwrap();
    let following = parse_annex_b_obus(&units[3]).unwrap();
    let xlayer = ExtendedLayerId::from_bits(3);
    let other_xlayer = ExtendedLayerId::from_bits(7);
    let mut bytes = Vec::new();
    for obu in &rap[..2] {
        write_obu(&mut bytes, obu.header, obu.payload);
    }
    for obu in rap
        .iter()
        .filter(|obu| obu.header.extended_layer_id == other_xlayer)
    {
        let header = ObuHeader {
            extended_layer_id: xlayer,
            ..obu.header
        };
        write_obu(&mut bytes, header, obu.payload);
    }
    let ops = rap
        .iter()
        .find(|obu| {
            obu.header.extended_layer_id == xlayer
                && obu.header.obu_type == ObuType::OperatingPointSet
        })
        .unwrap();
    let other_frame = following
        .iter()
        .find(|obu| {
            obu.header.extended_layer_id == xlayer
                && obu.header.temporal_layer_id == TemporalLayerId::from_bits(1)
                && obu.header.obu_type == ObuType::RegularTileGroup
        })
        .unwrap();
    for obu in [ops, other_frame] {
        let header = ObuHeader {
            extended_layer_id: other_xlayer,
            ..obu.header
        };
        write_obu(&mut bytes, header, obu.payload);
    }

    let all = stream_decode(&bytes, &DecoderSettings::default()).unwrap();
    assert_eq!(all.len(), 1);
    let selected = stream_decode(
        &bytes,
        &DecoderSettings::default()
            .with_temporal_layer_selection(TemporalLayerSelection::BASE_ONLY),
    )
    .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].extended_layer_id(), xlayer);
    assert_eq!(
        raw_all_and_by_xlayer(&selected).0,
        raw_all_and_by_xlayer(&all).0
    );
}

#[test]
fn first_unit_order_rejects_misplaced_metadata_at_an_unaligned_rap() {
    let units = temporal_units(MSDO_OPS_RA);
    let mut bytes = units[2].clone();
    let obus = parse_annex_b_obus(&bytes).unwrap();
    let header = ObuHeader {
        obu_type: ObuType::MetadataShort,
        ..obus.last().unwrap().header
    };
    write_obu(&mut bytes, header, &[0x22, 9, 0, 0x80]);

    let result = context(1, 1).decode_hash_report_bytes(&bytes, DecodeOptions::default());
    assert!(matches!(
        result,
        Err(DecodeError::UnsupportedFeature { unsupported })
            if unsupported.reason() == "unexpected_leading_obu_after_key"
    ));
}

#[test]
fn negative_msdo_ops_lifecycle_ordering_doh_and_aggregate_limits() {
    for opts in [
        DecodeOptions::default()
            .with_operating_point_selection(OperatingPointSelection::Global {
                ops_id: 0,
                op_index: 0,
            })
            .with_extended_layer_selection(ExtendedLayerSelection::Target(
                ExtendedLayerId::from_bits(3),
            )),
        DecodeOptions::default()
            .with_operating_point_selection(OperatingPointSelection::Global {
                ops_id: 0,
                op_index: 0,
            })
            .with_temporal_layer_selection(TemporalLayerSelection::BASE_ONLY),
        DecodeOptions::default()
            .with_operating_point_selection(OperatingPointSelection::Local {
                xlayer_id: ExtendedLayerId::from_bits(3),
                ops_id: 2,
                op_index: 0,
            })
            .with_embedded_layer_selection(EmbeddedLayerSelection::BASE_ONLY),
    ] {
        let err = context(1, 1)
            .decode_hash_report_bytes(MSDO_OPS_RA, opts)
            .unwrap_err();
        assert!(matches!(
            err,
            DecodeError::InvalidOperatingPointSelection { .. }
        ));
    }
    assert!(matches!(
        Decoder::new(
            DecoderSettings::default()
                .with_operating_point_selection(OperatingPointSelection::Global {
                    ops_id: 0,
                    op_index: 0,
                })
                .with_extended_layer_selection(ExtendedLayerSelection::OutputMask(1 << 3)),
        ),
        Err(DecodeError::InvalidOperatingPointSelection { .. })
    ));

    assert!(matches!(
        stream_decode(
            MSDO_OPS_RA,
            &DecoderSettings::default().with_operating_point_selection(
                OperatingPointSelection::Global {
                    ops_id: 0,
                    op_index: 7,
                },
            ),
        ),
        Err(DecodeError::InvalidOperatingPointSelection { .. })
    ));
    for unavailable in [
        OperatingPointSelection::Global {
            ops_id: 9,
            op_index: 0,
        },
        OperatingPointSelection::Global {
            ops_id: 0,
            op_index: 6,
        },
        OperatingPointSelection::Local {
            xlayer_id: ExtendedLayerId::from_bits(7),
            ops_id: 2,
            op_index: 0,
        },
    ] {
        let err = stream_decode(
            MSDO_OPS_RA,
            &DecoderSettings::default().with_operating_point_selection(unavailable),
        )
        .unwrap_err();
        assert!(
            matches!(err, DecodeError::MalformedSource { .. }),
            "{unavailable:?} -> {err:?}"
        );
    }

    let tus_8b = temporal_units(MSDO_8BIT);

    {
        let diff_msdo = make_msdo_obu(false, 1, true, &[(3, 0, 0, 0), (7, 0, 0, 0)]);
        let parsed = parse_annex_b_obus(&diff_msdo).unwrap();
        let mut bad_non_rap = Vec::new();
        for (ti, tu) in tus_8b.iter().enumerate() {
            for env in parse_annex_b_obus(tu).unwrap() {
                write_obu(&mut bad_non_rap, env.header, env.payload);
                if ti == 1 && env.header.obu_type == ObuType::TemporalDelimiter {
                    write_obu(&mut bad_non_rap, parsed[0].header, parsed[0].payload);
                }
            }
        }
        let err = stream_decode(&bad_non_rap, &DecoderSettings::default()).unwrap_err();
        assert!(
            matches!(
                &err,
                DecodeError::MalformedSource { issue }
                    if issue.spec_section() == Some("7.3.8.2")
            ),
            "got {err:?}"
        );

        let mut bad_non_clk = Vec::new();
        for (ti, tu) in tus_8b.iter().enumerate() {
            for env in parse_annex_b_obus(tu).unwrap() {
                if ti == 2 && env.header.obu_type == ObuType::Msdo {
                    write_obu(&mut bad_non_clk, parsed[0].header, parsed[0].payload);
                } else if ti == 2 && env.header.obu_type == ObuType::ClosedLoopKey {
                    let mut hdr = env.header;
                    hdr.obu_type = ObuType::OpenLoopKey;
                    write_obu(&mut bad_non_clk, hdr, env.payload);
                } else {
                    write_obu(&mut bad_non_clk, env.header, env.payload);
                }
            }
        }
        let err = stream_decode(&bad_non_clk, &DecoderSettings::default()).unwrap_err();
        assert!(
            matches!(
                &err,
                DecodeError::MalformedSource { issue }
                    if issue.spec_section() == Some("7.3.2")
            ),
            "got {err:?}"
        );
    }

    for after_frame in [false, true] {
        let dup_msdo = make_msdo_obu(false, 0, true, &[(3, 0, 0, 0), (7, 0, 0, 0)]);
        let parsed = parse_annex_b_obus(&dup_msdo).unwrap();
        let mut bad = Vec::new();
        for env in parse_annex_b_obus(&tus_8b[0]).unwrap() {
            write_obu(&mut bad, env.header, env.payload);
            if (!after_frame && env.header.obu_type == ObuType::Msdo)
                || (after_frame
                    && env.header.obu_type == ObuType::ClosedLoopKey
                    && env.header.extended_layer_id.get() == 7)
            {
                write_obu(&mut bad, parsed[0].header, parsed[0].payload);
            }
        }
        let err = stream_decode(&bad, &DecoderSettings::default()).unwrap_err();
        assert!(
            matches!(
                err,
                DecodeError::MalformedSource { .. } | DecodeError::UnsupportedFeature { .. }
            ),
            "after_frame={after_frame} -> {err:?}"
        );
    }

    {
        let mut bad = Vec::new();
        let envs = parse_annex_b_obus(&tus_8b[0]).unwrap();
        for env in envs
            .iter()
            .filter(|e| e.header.extended_layer_id == GLOBAL_XLAYER_ID)
        {
            write_obu(&mut bad, env.header, env.payload);
        }
        for env in envs
            .iter()
            .filter(|e| e.header.extended_layer_id.get() == 7)
        {
            write_obu(&mut bad, env.header, env.payload);
        }
        for env in envs
            .iter()
            .filter(|e| e.header.extended_layer_id.get() == 3)
        {
            write_obu(&mut bad, env.header, env.payload);
        }
        let err = stream_decode(&bad, &DecoderSettings::default()).unwrap_err();
        assert!(
            matches!(err, DecodeError::UnsupportedFeature { .. }),
            "got {err:?}"
        );
    }

    {
        let mut bad = Vec::new();
        for env in parse_annex_b_obus(MSDO_10BIT).unwrap() {
            if let PayloadStatus::Parsed(ParsedObu::LayerConfigurationRecord(mut lcr)) =
                env.payload_status().unwrap()
            {
                if let splot_core::headers::layer_config_record::LayerConfigurationRecord::Global(
                    ref mut info,
                ) = *lcr
                {
                    info.aggregate_info.as_mut().unwrap().max_interop = 1;
                }
                write_parsed(
                    &mut bad,
                    env.header,
                    &ParsedObu::LayerConfigurationRecord(lcr),
                );
            } else {
                write_obu(&mut bad, env.header, env.payload);
            }
        }
        let err = stream_decode(&bad, &DecoderSettings::default()).unwrap_err();
        assert!(
            matches!(
                &err,
                DecodeError::MalformedSource { issue }
                    if issue.spec_section() == Some("6.8.2")
            ),
            "got {err:?}"
        );
    }

    for keep_msdo in [false, true] {
        let mut bad = Vec::new();
        bad.extend_from_slice(&tus_8b[0]);
        let resume = parse_annex_b_obus(&tus_8b[2]).unwrap();
        let msdo = resume
            .iter()
            .find(|env| env.header.obu_type == ObuType::Msdo)
            .unwrap();
        for env in parse_annex_b_obus(&tus_8b[1]).unwrap() {
            if env.header.extended_layer_id == GLOBAL_XLAYER_ID
                || env.header.extended_layer_id.get() == 3
            {
                write_obu(&mut bad, env.header, env.payload);
            }
            if keep_msdo && env.header.obu_type == ObuType::TemporalDelimiter {
                write_obu(&mut bad, msdo.header, msdo.payload);
            }
        }
        for env in resume {
            if env.header.extended_layer_id.get() == 7 {
                let mut hdr = env.header;
                if let PayloadStatus::Parsed(ParsedObu::SequenceHeader(mut sh)) =
                    env.payload_status().unwrap()
                {
                    sh.general.seq_header_id = SequenceHeaderId::try_new(0).unwrap();
                    write_parsed(&mut bad, hdr, &ParsedObu::SequenceHeader(sh));
                } else {
                    hdr.extended_layer_id = ExtendedLayerId::from_bits(7);
                    write_obu(&mut bad, hdr, env.payload);
                }
            }
        }
        let err = stream_decode(&bad, &DecoderSettings::default()).unwrap_err();
        if keep_msdo {
            assert!(
                matches!(
                    &err,
                    DecodeError::MalformedSource { issue }
                        if issue.spec_section() == Some("7.3.7")
                ),
                "got {err:?}"
            );
        } else {
            assert!(
                matches!(
                    &err,
                    DecodeError::UnsupportedFeature { unsupported }
                        if unsupported.reason() == "missing_random_access_frame"
                ),
                "got {err:?}"
            );
        }
    }

    {
        let mut bad = Vec::new();
        for env in parse_annex_b_obus(&temporal_units(MSDO_OPS_RA)[0]).unwrap() {
            if env.header.obu_type == ObuType::OperatingPointSet
                && env.header.extended_layer_id == GLOBAL_XLAYER_ID
                && env.payload.len() > 2
            {
                write_obu(&mut bad, env.header, &env.payload[..2]);
                break;
            }
            write_obu(&mut bad, env.header, env.payload);
        }
        let mut decoder = Decoder::new(DecoderSettings::default().with_operating_point_selection(
            OperatingPointSelection::Global {
                ops_id: 0,
                op_index: 0,
            },
        ))
        .unwrap();
        assert_eq!(
            decoder.send_data(&bad, DataProps::default()).unwrap(),
            SendStatus::Accepted
        );
        let err = drain_until_error(&mut decoder).unwrap();
        assert!(
            matches!(err, DecodeError::MalformedSource { .. }),
            "got {err:?}"
        );
        decoder.flush();
        for (idx, unit) in temporal_units(MSDO_OPS_RA).iter().enumerate() {
            assert_eq!(
                decoder
                    .send_data(unit, DataProps::default().with_pts(idx as i64))
                    .unwrap(),
                SendStatus::Accepted
            );
            while decoder.get_picture().unwrap().is_some() {}
        }
        decoder.drain();
        while decoder.get_picture().unwrap().is_some() {}
    }

    let limited_outputs = context(1, 2)
        .decode_hash_report_bytes(
            MSDO_4STREAM,
            DecodeOptions::new(
                DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::Max(11)),
            ),
        )
        .unwrap_err();
    assert!(matches!(limited_outputs, DecodeError::Limit { .. }));

    let limited_bytes = context(1, 2)
        .decode_hash_report_bytes(
            MSDO_4STREAM,
            DecodeOptions::new(
                DecodeLimits::default()
                    .with_max_reference_store_bytes(DecodeLimitThreshold::Max(16_384)),
            ),
        )
        .unwrap_err();
    assert!(matches!(limited_bytes, DecodeError::Limit { .. }));
}
