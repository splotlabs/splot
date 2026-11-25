// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Multi-extended-layer decode isolation, selection, streaming, resource-budget, and conformance tests.

#![expect(clippy::unwrap_used)]

use std::fmt::Write as _;
use std::io::{self, Read};

use splot_core::annexb::parse_annex_b_obus;
use splot_core::headers::layer_config_record::LayerConfigurationRecord;
use splot_core::headers::metadata::MetadataType;
use splot_core::headers::sequence::{LcrId, SequenceHeaderId};
use splot_core::obu::{ObuHeader, ParsedObu, PayloadStatus, read_obu_header_from_slice};
use splot_core::span::ByteOffset;
use splot_core::stream_reader::TemporalUnitReader;
use splot_core::types::{
    EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId,
};
use splot_core::write::{BitWriter, write_annexb_obu, write_complete_obu};
use splot_decode::{
    DataProps, DecodeContext, DecodeError, DecodeLimitThreshold, DecodeLimits, DecodeOptions,
    DecodeRuntimeConfig, Decoder, DecoderSettings, EmbeddedLayerSelection, ExtendedLayerSelection,
    FrameDelay, Picture, PictureSamples, PlaneId, SendStatus, TemporalLayerSelection, ThreadCount,
};
use splot_recon::Y4mFrameRate;

const XLAYER3_7_ISOLATION: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-xlayer3-7-global-lcr-isolation.obu"
);
const XLAYER3_7_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-xlayer3-7-global-lcr-10bit.obu");
const XLAYER3_7_LAYERED_MIXED: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-xlayer3-7-layered-mixed.obu");
const XLAYER1_ONLY_176X144: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-5frame-xlayer1-only-176x144.ivf");

const SHA_ISO_INPUT: &str = "87e62f3da20ce7690dfe0d7a69cb97e15b1ad279e651650d86b4722e410456b2";
const SHA_ISO_X3: &str = "8bdd46a3f2c2af0d989621bd90253a101ee29a5e2274062301dcce60f9398c46";
const SHA_ISO_X7: &str = "0abae5b2ef2bf8a1b8d2000e927da1f55ccd1083ea92d755c45a2893cd057b8e";
const SHA_10B_X3: &str = "49bb757a8539e871f291d96bc62632159f6eea670431a1e04808c3700d52ef94";
const SHA_10B_X7: &str = "32ea1a187f3332e64a900fea920db9e32c82ac0fb68e86af36c91709955dd40e";
const SHA_LAY_X3: &str = "80f2b107ff4512837e25b724f380259077ad451f73e7cb2c046e64feee8062cc";
const SHA_LAY_X7: &str = "8762b65ff05fd6aa3b144f2b0675544f17c61ae8fd8431dbe9f2c7aad9c1598b";
const SHA_X1_ONLY: &str = "027375b43be1226e72df68a996edbc937ae747db39e4ab5da964e50811fa0b4d";

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
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while !(msg.len() + 8).is_multiple_of(64) {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut sched = [0u32; 64];
        for i in 0..16 {
            sched[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
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
            let temp1 = vh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(ROUND_K[i])
                .wrapping_add(sched[i]);
            let s0 = va.rotate_right(2) ^ va.rotate_right(13) ^ va.rotate_right(22);
            let maj = (va & vb) ^ (va & vc) ^ (vb & vc);
            let temp2 = s0.wrapping_add(maj);
            vh = vg;
            vg = vf;
            vf = ve;
            ve = vd.wrapping_add(temp1);
            vd = vc;
            vc = vb;
            vb = va;
            va = temp1.wrapping_add(temp2);
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

fn write_obu(writer: &mut BitWriter, header: ObuHeader, payload: &[u8]) {
    write_annexb_obu(writer, &header, payload).unwrap();
}

fn stream_decode_with_settings(
    bytes: &[u8],
    settings: &DecoderSettings,
) -> Result<Vec<Picture>, DecodeError> {
    let mut decoder = Decoder::new(*settings)?;
    let mut pictures = Vec::new();
    for (idx, unit) in temporal_units(bytes).iter().enumerate() {
        let props = DataProps::default().with_pts(i64::try_from(idx).unwrap());
        loop {
            match decoder.send_data(unit, props)? {
                SendStatus::Accepted => break,
                SendStatus::Full => {
                    while let Some(picture) = decoder.get_picture()? {
                        pictures.push(picture);
                    }
                }
            }
        }
        while let Some(picture) = decoder.get_picture()? {
            pictures.push(picture);
        }
    }
    decoder.drain();
    while let Some(picture) = decoder.get_picture()? {
        pictures.push(picture);
    }
    Ok(pictures)
}

fn pictures_to_raw(pictures: &[&Picture]) -> Vec<u8> {
    let mut out = Vec::new();
    for picture in pictures {
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            match picture.plane(plane) {
                Some(PictureSamples::Eight(plane)) => {
                    for row in plane.visible_rows() {
                        out.extend_from_slice(row);
                    }
                }
                Some(PictureSamples::Ten(plane)) => {
                    for sample in plane.visible_rows().flatten() {
                        out.extend_from_slice(&sample.to_le_bytes());
                    }
                }
                None => {}
            }
        }
    }
    out
}

fn raw_for_xlayer(pictures: &[Picture], xlayer: u8) -> Vec<u8> {
    let filtered: Vec<&Picture> = pictures
        .iter()
        .filter(|pic| pic.extended_layer_id().get() == xlayer)
        .collect();
    pictures_to_raw(&filtered)
}

fn raw_via_context(bytes: &[u8], options: &DecodeOptions) -> Vec<u8> {
    let mut out = Vec::new();
    context(1, 2)
        .decode_raw_bytes(bytes, *options, &mut out)
        .unwrap();
    out
}

fn write_global_lcr_annexb(global_id: u8, xlayer_map: u32, doh: bool) -> Vec<u8> {
    let mut body = BitWriter::new();
    body.write_bits_u8(global_id, 3).unwrap();
    body.write_bits(xlayer_map, 31).unwrap();
    body.write_flag(false).unwrap();
    body.write_flag(false).unwrap();
    body.write_flag(false).unwrap();
    body.write_flag(false).unwrap();
    body.write_flag(false).unwrap();
    body.write_bits_u8(0, 7).unwrap();
    body.write_flag(doh).unwrap();
    body.write_flag(false).unwrap();
    body.write_bits_u8(0, 3).unwrap();
    body.write_bits_u8(0, 5).unwrap();
    body.write_bit(0).unwrap();
    let rem = body.bit_len() % 8;
    let nb = if rem == 0 { 8 } else { 8 - rem };
    body.write_trailing_bits(nb).unwrap();
    let header = ObuHeader {
        has_header_extension: true,
        obu_type: ObuType::LayerConfigurationRecord,
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: GLOBAL_XLAYER_ID,
        header_size_bytes: 2,
    };
    let mut out = BitWriter::new();
    write_obu(&mut out, header, &body.into_bytes());
    out.into_bytes()
}

fn rewrite_complete_annexb(header: ObuHeader, payload: &ParsedObu) -> Vec<u8> {
    let mut full = BitWriter::new();
    write_complete_obu(&mut full, &header, payload, &[]).unwrap();
    let full_bytes = full.into_bytes();
    let mut framed = BitWriter::new();
    framed.write_leb128(full_bytes.len() as u32).unwrap();
    framed.write_le(&full_bytes).unwrap();
    framed.into_bytes()
}

fn append_obu_with_seq_id(
    out: &mut Vec<u8>,
    env: &splot_core::annexb::ObuEnvelope<'_>,
    header: ObuHeader,
    seq_id: u32,
) {
    if header.obu_type == ObuType::SequenceHeader || header.obu_type == ObuType::MultiFrameHeader {
        let rewritten = match env.payload_status().unwrap() {
            PayloadStatus::Parsed(ParsedObu::SequenceHeader(mut sh)) => {
                sh.general.seq_header_id = SequenceHeaderId::try_new(seq_id).unwrap();
                ParsedObu::SequenceHeader(sh)
            }
            PayloadStatus::Parsed(ParsedObu::MultiFrameHeader(mut mfh)) => {
                mfh.mfh_seq_header_id = seq_id;
                ParsedObu::MultiFrameHeader(mfh)
            }
            _ => unreachable!("header already filtered to SequenceHeader or MultiFrameHeader"),
        };
        out.extend_from_slice(&rewrite_complete_annexb(header, &rewritten));
    } else {
        let mut bw = BitWriter::new();
        write_obu(&mut bw, header, env.payload);
        out.extend_from_slice(&bw.into_bytes());
    }
}

#[test]
fn routing_header_unit_vectors() {
    assert_eq!(sha256_hex(XLAYER3_7_ISOLATION), SHA_ISO_INPUT);

    let seq_x3 = read_obu_header_from_slice(&[0x84, 0x03], ByteOffset::new(0)).unwrap();
    assert_eq!(seq_x3.obu_type, ObuType::SequenceHeader);
    assert!(seq_x3.has_header_extension);
    assert_eq!(seq_x3.extended_layer_id, ExtendedLayerId::from_bits(3));
    assert_eq!(seq_x3.embedded_layer_id, EmbeddedLayerId::from_bits(0));

    let seq_x7 = read_obu_header_from_slice(&[0x84, 0x07], ByteOffset::new(0)).unwrap();
    assert_eq!(seq_x7.obu_type, ObuType::SequenceHeader);
    assert!(seq_x7.has_header_extension);
    assert_eq!(seq_x7.extended_layer_id, ExtendedLayerId::from_bits(7));

    let td = read_obu_header_from_slice(&[0x08], ByteOffset::new(0)).unwrap();
    assert_eq!(td.obu_type, ObuType::TemporalDelimiter);
    assert!(!td.has_header_extension);
    assert_eq!(td.extended_layer_id, GLOBAL_XLAYER_ID);

    let annexb = [0x01, 0x08, 0x02, 0x84, 0x03, 0x02, 0x84, 0x07];
    let plan = DecodeContext::plan_bytes(&annexb, DecodeOptions::DEFAULT).unwrap();
    let planned: Vec<_> = plan.obus().collect();
    assert_eq!(planned.len(), 3);
    assert_eq!(planned[0].header().extended_layer_id, GLOBAL_XLAYER_ID);
    assert_eq!(
        planned[1].header().extended_layer_id,
        ExtendedLayerId::from_bits(3)
    );
    assert_eq!(
        planned[2].header().extended_layer_id,
        ExtendedLayerId::from_bits(7)
    );
}

#[test]
fn avm_digest_parity_across_full_target_mask_and_3d_selections() {
    for selection in [
        ExtendedLayerSelection::All,
        ExtendedLayerSelection::OutputMask((1 << 3) | (1 << 7)),
    ] {
        let pics = stream_decode_with_settings(
            XLAYER3_7_ISOLATION,
            &DecoderSettings::default()
                .with_threads(ThreadCount::from(1usize))
                .with_frame_delay(FrameDelay::from(2usize))
                .with_extended_layer_selection(selection),
        )
        .unwrap();
        let meta: Vec<(u8, usize, usize)> = pics
            .iter()
            .map(|p| (p.extended_layer_id().get(), p.width(), p.height()))
            .collect();
        assert_eq!(
            meta,
            vec![
                (3, 64, 64),
                (7, 96, 64),
                (3, 64, 64),
                (7, 96, 64),
                (3, 80, 64),
                (7, 96, 64),
                (3, 80, 64),
                (7, 96, 64),
            ]
        );
        assert_eq!(sha256_hex(&raw_for_xlayer(&pics, 3)), SHA_ISO_X3);
        assert_eq!(sha256_hex(&raw_for_xlayer(&pics, 7)), SHA_ISO_X7);
    }

    for (selection, xlayer, expected_sha, expected_dims) in [
        (
            ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(3)),
            3u8,
            SHA_ISO_X3,
            vec![(64usize, 64usize), (64, 64), (80, 64), (80, 64)],
        ),
        (
            ExtendedLayerSelection::OutputMask(1 << 3),
            3u8,
            SHA_ISO_X3,
            vec![(64, 64), (64, 64), (80, 64), (80, 64)],
        ),
        (
            ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(7)),
            7u8,
            SHA_ISO_X7,
            vec![(96, 64); 4],
        ),
        (
            ExtendedLayerSelection::OutputMask(1 << 7),
            7u8,
            SHA_ISO_X7,
            vec![(96, 64); 4],
        ),
    ] {
        let opts = DecodeOptions::default().with_extended_layer_selection(selection);
        let raw = raw_via_context(XLAYER3_7_ISOLATION, &opts);
        assert_eq!(sha256_hex(&raw), expected_sha);

        let pics = stream_decode_with_settings(
            XLAYER3_7_ISOLATION,
            &DecoderSettings::default().with_extended_layer_selection(selection),
        )
        .unwrap();
        let dims: Vec<(usize, usize)> = pics.iter().map(|p| (p.width(), p.height())).collect();
        assert_eq!(dims, expected_dims);
        for p in &pics {
            assert_eq!(p.extended_layer_id().get(), xlayer);
        }
        assert_eq!(raw_for_xlayer(&pics, xlayer), raw);
    }

    let pics_10b = stream_decode_with_settings(
        XLAYER3_7_10BIT,
        &DecoderSettings::default().with_extended_layer_selection(ExtendedLayerSelection::All),
    )
    .unwrap();
    assert_eq!(pics_10b.len(), 6);
    assert_eq!(sha256_hex(&raw_for_xlayer(&pics_10b, 3)), SHA_10B_X3);
    assert_eq!(sha256_hex(&raw_for_xlayer(&pics_10b, 7)), SHA_10B_X7);
    assert_eq!(
        sha256_hex(&raw_via_context(
            XLAYER3_7_10BIT,
            &DecodeOptions::default().with_extended_layer_selection(
                ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(3)),
            ),
        )),
        SHA_10B_X3
    );
    assert_eq!(
        sha256_hex(&raw_via_context(
            XLAYER3_7_10BIT,
            &DecodeOptions::default().with_extended_layer_selection(
                ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(7)),
            ),
        )),
        SHA_10B_X7
    );

    let pics_mixed =
        stream_decode_with_settings(XLAYER3_7_LAYERED_MIXED, &DecoderSettings::default()).unwrap();
    assert_eq!(pics_mixed.len(), 12);
    assert_eq!(sha256_hex(&raw_for_xlayer(&pics_mixed, 3)), SHA_LAY_X3);
    assert_eq!(sha256_hex(&raw_for_xlayer(&pics_mixed, 7)), SHA_LAY_X7);

    let pics_3d_base = stream_decode_with_settings(
        XLAYER3_7_LAYERED_MIXED,
        &DecoderSettings::default()
            .with_extended_layer_selection(ExtendedLayerSelection::OutputMask((1 << 3) | (1 << 7)))
            .with_embedded_layer_selection(EmbeddedLayerSelection::BASE_ONLY)
            .with_temporal_layer_selection(TemporalLayerSelection::BASE_ONLY),
    )
    .unwrap();
    let summary_3d: Vec<(u8, u8, u8)> = pics_3d_base
        .iter()
        .map(|p| {
            (
                p.extended_layer_id().get(),
                p.embedded_layer_id().get(),
                p.temporal_layer_id().get(),
            )
        })
        .collect();
    assert_eq!(
        summary_3d,
        vec![(3, 0, 0), (7, 0, 0), (7, 0, 0), (7, 0, 0), (7, 0, 0)]
    );

    let x3_ml1_tl1 = stream_decode_with_settings(
        XLAYER3_7_LAYERED_MIXED,
        &DecoderSettings::default()
            .with_extended_layer_selection(ExtendedLayerSelection::Target(
                ExtendedLayerId::from_bits(3),
            ))
            .with_embedded_layer_selection(EmbeddedLayerSelection::OutputMask(0b10))
            .with_temporal_layer_selection(TemporalLayerSelection::OutputMask(0b10)),
    )
    .unwrap();
    assert_eq!(x3_ml1_tl1.len(), 3);
    for p in &x3_ml1_tl1 {
        assert_eq!(p.extended_layer_id().get(), 3);
        assert_eq!(p.embedded_layer_id().get(), 1);
        assert_eq!(p.temporal_layer_id().get(), 1);
    }

    for sel in [
        ExtendedLayerSelection::All,
        ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(1)),
    ] {
        let pics_x1 = stream_decode_with_settings(
            XLAYER1_ONLY_176X144,
            &DecoderSettings::default().with_extended_layer_selection(sel),
        )
        .unwrap();
        assert_eq!(pics_x1.len(), 5);
        for p in &pics_x1 {
            assert_eq!(p.extended_layer_id().get(), 1);
            assert_eq!((p.width(), p.height()), (176, 144));
        }
        assert_eq!(sha256_hex(&raw_for_xlayer(&pics_x1, 1)), SHA_X1_ONLY);
    }
    let absent = stream_decode_with_settings(
        XLAYER1_ONLY_176X144,
        &DecoderSettings::default().with_extended_layer_selection(ExtendedLayerSelection::Target(
            ExtendedLayerId::from_bits(0),
        )),
    );
    assert!(matches!(
        absent,
        Err(DecodeError::UnsupportedFeature { unsupported })
            if unsupported.reason() == "missing_frame_candidate"
    ));
}

#[test]
fn shared_numeric_hls_ids_isolate_sequence_mfh_and_film_grain() {
    let mut collided_a = Vec::new();
    for env in parse_annex_b_obus(XLAYER3_7_ISOLATION).unwrap() {
        let hdr = env.header;
        if hdr.extended_layer_id.get() == 7 {
            append_obu_with_seq_id(&mut collided_a, &env, hdr, 0);
        } else {
            let mut bw = BitWriter::new();
            write_obu(&mut bw, hdr, env.payload);
            collided_a.extend_from_slice(&bw.into_bytes());
        }
    }
    let pics_a = stream_decode_with_settings(
        &collided_a,
        &DecoderSettings::default().with_threads(ThreadCount::from(3usize)),
    )
    .unwrap();
    assert_eq!(pics_a.len(), 8);
    assert_eq!(sha256_hex(&raw_for_xlayer(&pics_a, 3)), SHA_ISO_X3);
    assert_eq!(sha256_hex(&raw_for_xlayer(&pics_a, 7)), SHA_ISO_X7);

    let tus_8b = temporal_units(XLAYER3_7_LAYERED_MIXED);
    let tus_10b = temporal_units(XLAYER3_7_10BIT);
    let mut collided_b = Vec::new();
    for i in 0..3 {
        for (tu, src_xlayer, dst_xlayer) in [(&tus_10b[i], 7u8, 3u8), (&tus_8b[i], 7u8, 7u8)] {
            for env in parse_annex_b_obus(tu).unwrap() {
                let mut hdr = env.header;
                if hdr.obu_type == ObuType::TemporalDelimiter {
                    if src_xlayer == 7 && dst_xlayer == 3 {
                        let mut bw = BitWriter::new();
                        write_obu(&mut bw, hdr, env.payload);
                        collided_b.extend_from_slice(&bw.into_bytes());
                    }
                    continue;
                }
                if hdr.obu_type == ObuType::LayerConfigurationRecord
                    && hdr.extended_layer_id == GLOBAL_XLAYER_ID
                {
                    if dst_xlayer == 3 {
                        let mut bw = BitWriter::new();
                        write_obu(&mut bw, hdr, env.payload);
                        collided_b.extend_from_slice(&bw.into_bytes());
                    }
                    continue;
                }
                if hdr.extended_layer_id.get() != src_xlayer {
                    continue;
                }
                hdr.extended_layer_id = ExtendedLayerId::from_bits(dst_xlayer);
                append_obu_with_seq_id(&mut collided_b, &env, hdr, 0);
            }
        }
    }
    let pics_b = stream_decode_with_settings(&collided_b, &DecoderSettings::default()).unwrap();
    assert_eq!(pics_b.len(), 6);
    assert_eq!(sha256_hex(&raw_for_xlayer(&pics_b, 3)), SHA_10B_X7);
    let x7_3frames = raw_via_context(
        &collided_b,
        &DecodeOptions::default().with_extended_layer_selection(ExtendedLayerSelection::Target(
            ExtendedLayerId::from_bits(7),
        )),
    );
    assert_eq!(x7_3frames, raw_for_xlayer(&pics_b, 7));
}

#[test]
fn streaming_chunks_thread_counts_and_flush_recovery() {
    let expected_report = context(1, 2)
        .decode_hash_report_bytes(XLAYER3_7_ISOLATION, DecodeOptions::DEFAULT)
        .unwrap();
    for threads in [1usize, 3, 12] {
        for chunk_size in [1usize, 7] {
            let report = context(threads, 2)
                .decode_hash_report_reader(
                    ChunkedReader::new(XLAYER3_7_ISOLATION, chunk_size),
                    DecodeOptions::DEFAULT,
                )
                .unwrap();
            assert_eq!(report.frames.len(), expected_report.frames.len());
            for (got, exp) in report.frames.iter().zip(&expected_report.frames) {
                assert_eq!(got.hashes[0].digest_hex, exp.hashes[0].digest_hex);
            }

            for (xlayer, sha) in [(3u8, SHA_ISO_X3), (7u8, SHA_ISO_X7)] {
                let mut raw = Vec::new();
                context(threads, 2)
                    .decode_raw_reader(
                        ChunkedReader::new(XLAYER3_7_ISOLATION, chunk_size),
                        DecodeOptions::default().with_extended_layer_selection(
                            ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(xlayer)),
                        ),
                        &mut raw,
                    )
                    .unwrap();
                assert_eq!(sha256_hex(&raw), sha);
            }
        }
    }

    let tus = temporal_units(XLAYER3_7_ISOLATION);
    let mut decoder = Decoder::new(DecoderSettings::default()).unwrap();
    for tu in &tus[..2] {
        let _ = decoder.send_data(tu, DataProps::default()).unwrap();
        while decoder.get_picture().unwrap().is_some() {}
    }
    decoder.flush();
    while decoder.get_picture().unwrap().is_some() {}
    let mut post_flush = Vec::new();
    for tu in &tus {
        loop {
            match decoder.send_data(tu, DataProps::default()).unwrap() {
                SendStatus::Accepted => break,
                SendStatus::Full => {
                    while let Some(p) = decoder.get_picture().unwrap() {
                        post_flush.push(p);
                    }
                }
            }
        }
        while let Some(p) = decoder.get_picture().unwrap() {
            post_flush.push(p);
        }
    }
    decoder.drain();
    while let Some(p) = decoder.get_picture().unwrap() {
        post_flush.push(p);
    }
    assert_eq!(post_flush.len(), 8);
    assert_eq!(sha256_hex(&raw_for_xlayer(&post_flush, 3)), SHA_ISO_X3);
    assert_eq!(sha256_hex(&raw_for_xlayer(&post_flush, 7)), SHA_ISO_X7);
}

#[test]
fn global_metadata_group_targets_single_xlayer_only() {
    let mut mg_payload = vec![
        0x00, // is_suffix=0, necessity=0, app_id=0
        0x00, // metadata_unit_cnt_minus_1 = 0
        0x01, // metadata_type = 1 (HDR_CLL)
        0x10, // muh_header_size = 8, cancel = 0
        0x04, // payload_size = 4
        0x68, 0x00, // layer_idc=LAYER_VALUES(3), persistence=2 (no), priority=0
    ];
    mg_payload.extend_from_slice(&(1u32 << 3).to_be_bytes());
    mg_payload.push(0x01); // mlayer_map = bit 0
    mg_payload.extend_from_slice(&[0x00, 0x64, 0x00, 0x32]); // max_cll=100, max_fall=50
    mg_payload.push(0x80); // obu trailing byte
    let mut mg_bw = BitWriter::new();
    let mg_hdr = ObuHeader {
        has_header_extension: true,
        obu_type: ObuType::MetadataGroup,
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: GLOBAL_XLAYER_ID,
        header_size_bytes: 2,
    };
    write_obu(&mut mg_bw, mg_hdr, &mg_payload);
    let mg_obu = mg_bw.into_bytes();
    assert!(matches!(
        parse_annex_b_obus(&mg_obu).unwrap()[0]
            .payload_status()
            .unwrap(),
        PayloadStatus::Parsed(ParsedObu::MetadataGroup(_))
    ));

    let tus = temporal_units(XLAYER3_7_ISOLATION);
    let mut with_mg = Vec::new();
    let tu0_obus = parse_annex_b_obus(&tus[0]).unwrap();
    let mut inserted = false;
    for env in &tu0_obus {
        let mut bw = BitWriter::new();
        write_obu(&mut bw, env.header, env.payload);
        with_mg.extend_from_slice(&bw.into_bytes());
        if env.header.obu_type == ObuType::LayerConfigurationRecord && !inserted {
            with_mg.extend_from_slice(&mg_obu);
            inserted = true;
        }
    }
    assert!(inserted);
    for tu in &tus[1..] {
        with_mg.extend_from_slice(tu);
    }

    let pics = stream_decode_with_settings(&with_mg, &DecoderSettings::default()).unwrap();
    assert_eq!(pics.len(), 8);
    assert_eq!(pics[0].extended_layer_id().get(), 3);
    assert_eq!(pics[0].metadata().len(), 1);
    assert_eq!(pics[0].metadata()[0].metadata_type, MetadataType::HdrCll);
    assert_eq!(pics[1].extended_layer_id().get(), 7);
    assert!(pics[1].metadata().is_empty());
    assert!(pics[2].metadata().is_empty());
}

#[test]
fn flat_raw_and_y4m_reject_multiple_emitting_xlayers() {
    let fps = Y4mFrameRate::new(25, 1).unwrap();
    for selection in [
        ExtendedLayerSelection::All,
        ExtendedLayerSelection::OutputMask((1 << 3) | (1 << 7)),
    ] {
        let opts = DecodeOptions::default().with_extended_layer_selection(selection);
        let err_raw = context(1, 2)
            .decode_raw_bytes(XLAYER3_7_ISOLATION, opts, io::sink())
            .unwrap_err();
        assert!(matches!(
            err_raw,
            DecodeError::UnsupportedFeature { ref unsupported }
                if unsupported.reason() == "multiple_extended_layers_flat_output"
        ));

        let err_y4m = context(1, 2)
            .decode_y4m_bytes(
                XLAYER3_7_10BIT,
                opts.with_y4m_frame_rate_override(Some(fps)),
                io::sink(),
            )
            .unwrap_err();
        assert!(matches!(
            err_y4m,
            DecodeError::UnsupportedFeature { ref unsupported }
                if unsupported.reason() == "multiple_extended_layers_flat_output"
        ));
    }

    let mut y4m_out = Vec::new();
    context(1, 2)
        .decode_y4m_bytes(
            XLAYER3_7_ISOLATION,
            DecodeOptions::default()
                .with_y4m_frame_rate_override(Some(fps))
                .with_extended_layer_selection(ExtendedLayerSelection::Target(
                    ExtendedLayerId::from_bits(7),
                )),
            &mut y4m_out,
        )
        .unwrap();
    assert!(y4m_out.starts_with(b"YUV4MPEG2 W96 H64 "));
}

#[test]
fn negative_lcr_scope_corruption_isolation_and_aggregate_bounds() {
    for invalid in [
        ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(31)),
        ExtendedLayerSelection::OutputMask(0),
        ExtendedLayerSelection::OutputMask(1u32 << 31),
    ] {
        assert!(matches!(
            DecodeContext::plan_bytes(
                XLAYER3_7_ISOLATION,
                DecodeOptions::default().with_extended_layer_selection(invalid),
            ),
            Err(DecodeError::InvalidExtendedSelection { .. })
        ));
        assert!(matches!(
            Decoder::new(DecoderSettings::default().with_extended_layer_selection(invalid)),
            Err(DecodeError::InvalidExtendedSelection { .. })
        ));
    }

    let mut missing_lcr = BitWriter::new();
    for env in parse_annex_b_obus(XLAYER3_7_ISOLATION).unwrap() {
        if env.header.obu_type == ObuType::LayerConfigurationRecord {
            continue;
        }
        write_obu(&mut missing_lcr, env.header, env.payload);
    }
    assert!(matches!(
        context(1, 2).decode_hash_report_bytes(&missing_lcr.into_bytes(), DecodeOptions::DEFAULT),
        Err(DecodeError::MalformedSource { .. })
    ));

    let partial_lcr = write_global_lcr_annexb(1, 1 << 3, true);
    let mut mismatched_lcr = BitWriter::new();
    for env in parse_annex_b_obus(XLAYER3_7_ISOLATION).unwrap() {
        if env.header.obu_type == ObuType::LayerConfigurationRecord {
            mismatched_lcr.write_le(&partial_lcr).unwrap();
        } else {
            write_obu(&mut mismatched_lcr, env.header, env.payload);
        }
    }
    assert!(matches!(
        context(1, 2)
            .decode_hash_report_bytes(&mismatched_lcr.into_bytes(), DecodeOptions::DEFAULT),
        Err(DecodeError::MalformedSource { .. })
    ));

    let mut global_seq = BitWriter::new();
    let mut bad_td = BitWriter::new();
    for env in parse_annex_b_obus(XLAYER3_7_ISOLATION).unwrap() {
        let mut h_seq = env.header;
        if h_seq.obu_type == ObuType::SequenceHeader {
            h_seq.extended_layer_id = GLOBAL_XLAYER_ID;
        }
        write_obu(&mut global_seq, h_seq, env.payload);

        let mut h_td = env.header;
        if h_td.obu_type == ObuType::TemporalDelimiter {
            h_td.has_header_extension = true;
            h_td.extended_layer_id = ExtendedLayerId::from_bits(3);
            h_td.header_size_bytes = 2;
        }
        write_obu(&mut bad_td, h_td, env.payload);
    }
    assert!(matches!(
        DecodeContext::plan_bytes(&global_seq.into_bytes(), DecodeOptions::DEFAULT),
        Err(DecodeError::UnsupportedStructure { .. })
    ));
    assert!(matches!(
        DecodeContext::plan_bytes(&bad_td.into_bytes(), DecodeOptions::DEFAULT),
        Err(DecodeError::UnsupportedStructure { .. })
    ));

    let mut corrupted_x7 = BitWriter::new();
    for env in parse_annex_b_obus(XLAYER3_7_ISOLATION).unwrap() {
        if env.header.obu_type == ObuType::RegularTileGroup
            && env.header.extended_layer_id.get() == 7
        {
            let mut bad_payload = env.payload.to_vec();
            let last = bad_payload.len() - 1;
            let mid = bad_payload.len() / 2;
            bad_payload[last] ^= 0xff;
            if bad_payload.len() > 8 {
                bad_payload[mid] ^= 0xa5;
            }
            write_obu(&mut corrupted_x7, env.header, &bad_payload);
        } else {
            write_obu(&mut corrupted_x7, env.header, env.payload);
        }
    }
    let corrupted_x7_bytes = corrupted_x7.into_bytes();
    let isolated_x3 = raw_via_context(
        &corrupted_x7_bytes,
        &DecodeOptions::default().with_extended_layer_selection(ExtendedLayerSelection::Target(
            ExtendedLayerId::from_bits(3),
        )),
    );
    assert_eq!(sha256_hex(&isolated_x3), SHA_ISO_X3);
    assert!(
        context(1, 2)
            .decode_hash_report_bytes(&corrupted_x7_bytes, DecodeOptions::DEFAULT)
            .is_err()
    );

    let budget_frames =
        DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::Max(4));
    assert!(
        context(1, 2)
            .decode_raw_bytes(
                XLAYER3_7_ISOLATION,
                DecodeOptions::new(budget_frames).with_extended_layer_selection(
                    ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(3)),
                ),
                io::sink(),
            )
            .is_ok()
    );
    assert!(matches!(
        context(1, 2)
            .decode_hash_report_bytes(XLAYER3_7_ISOLATION, DecodeOptions::new(budget_frames),),
        Err(DecodeError::Limit { .. })
    ));

    let tight_ref_budget =
        DecodeLimits::default().with_max_reference_store_bytes(DecodeLimitThreshold::Max(80_000));
    assert!(
        context(1, 1)
            .decode_raw_bytes(
                XLAYER3_7_10BIT,
                DecodeOptions::new(tight_ref_budget).with_extended_layer_selection(
                    ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(3)),
                ),
                io::sink(),
            )
            .is_ok()
    );
    assert!(matches!(
        context(1, 1)
            .decode_hash_report_bytes(XLAYER3_7_10BIT, DecodeOptions::new(tight_ref_budget),),
        Err(DecodeError::Limit { .. })
    ));
}

#[test]
fn global_lcr_ids_must_agree_after_all_xlayer_activations() {
    for split_references in [false, true] {
        let mut bytes = Vec::new();
        for (tu, unit) in temporal_units(XLAYER3_7_ISOLATION).iter().enumerate() {
            for envelope in parse_annex_b_obus(unit).unwrap() {
                let header = envelope.header;
                if let PayloadStatus::Parsed(ParsedObu::LayerConfigurationRecord(mut record)) =
                    envelope.payload_status().unwrap()
                    && let LayerConfigurationRecord::Global(global) = record.as_mut()
                {
                    if split_references || tu < 2 {
                        let mut obu = BitWriter::new();
                        write_obu(&mut obu, header, envelope.payload);
                        bytes.extend(obu.into_bytes());
                    }
                    if split_references || tu >= 2 {
                        global.global_config_record_id = 2;
                        bytes.extend(rewrite_complete_annexb(
                            header,
                            &ParsedObu::LayerConfigurationRecord(record),
                        ));
                    }
                    continue;
                }
                if (split_references && header.extended_layer_id.get() == 7
                    || !split_references && tu >= 2)
                    && let PayloadStatus::Parsed(ParsedObu::SequenceHeader(mut sequence)) =
                        envelope.payload_status().unwrap()
                {
                    sequence.general.seq_lcr_id = LcrId::from_bits(2);
                    bytes.extend(rewrite_complete_annexb(
                        header,
                        &ParsedObu::SequenceHeader(sequence),
                    ));
                } else {
                    let mut obu = BitWriter::new();
                    write_obu(&mut obu, header, envelope.payload);
                    bytes.extend(obu.into_bytes());
                }
            }
        }
        let batch = context(3, 2).decode_hash_report_bytes(&bytes, DecodeOptions::default());
        let streaming = stream_decode_with_settings(&bytes, &DecoderSettings::default());
        if split_references {
            for error in [batch.unwrap_err(), streaming.unwrap_err()] {
                assert!(
                    matches!(
                        error,
                        DecodeError::MalformedSource { ref issue }
                            if issue.spec_section() == Some("6.8.2")
                    ),
                    "got {error:?}"
                );
            }
        } else {
            assert_eq!(batch.unwrap().frames.len(), 8);
            let pictures = streaming.unwrap();
            assert_eq!(sha256_hex(&raw_for_xlayer(&pictures, 3)), SHA_ISO_X3);
            assert_eq!(sha256_hex(&raw_for_xlayer(&pictures, 7)), SHA_ISO_X7);
        }
    }
}

#[test]
fn global_lcr_ids_must_include_the_parent_of_each_local_lcr() {
    let mut bytes = BitWriter::new();
    for envelope in parse_annex_b_obus(XLAYER3_7_LAYERED_MIXED).unwrap() {
        let mut payload = envelope.payload.to_vec();
        if envelope.header.obu_type == ObuType::LayerConfigurationRecord
            && envelope.header.extended_layer_id.get() == 3
        {
            assert_eq!(payload[0], 0x24);
            payload[0] = 0x04;
        }
        write_obu(&mut bytes, envelope.header, &payload);
    }
    let bytes = bytes.into_bytes();
    let error = context(1, 2)
        .decode_hash_report_bytes(&bytes, DecodeOptions::default())
        .unwrap_err();
    assert!(
        matches!(
            error,
            DecodeError::MalformedSource { ref issue }
                if issue.spec_section() == Some("6.8.2")
        ),
        "got {error:?}"
    );
}

#[test]
fn filtered_local_lcr_peers_keep_the_common_parent_without_direct_activation() {
    let bytes = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-xlayer3-7-9-shared-global-parent.obu"
    );
    for (selection, expected_count) in [
        (ExtendedLayerSelection::All, 12),
        (ExtendedLayerSelection::OutputMask((1 << 7) | (1 << 9)), 8),
    ] {
        let batch = context(3, 2)
            .decode_hash_report_bytes(
                bytes,
                DecodeOptions::default().with_extended_layer_selection(selection),
            )
            .unwrap();
        let streaming = stream_decode_with_settings(
            bytes,
            &DecoderSettings::default().with_extended_layer_selection(selection),
        )
        .unwrap();
        assert_eq!(batch.frames.len(), expected_count);
        assert_eq!(streaming.len(), expected_count);
        assert_eq!(sha256_hex(&raw_for_xlayer(&streaming, 7)), SHA_ISO_X7);
        assert_eq!(sha256_hex(&raw_for_xlayer(&streaming, 9)), SHA_ISO_X7);
        if selection == ExtendedLayerSelection::All {
            assert_eq!(sha256_hex(&raw_for_xlayer(&streaming, 3)), SHA_ISO_X3);
        } else {
            assert!(raw_for_xlayer(&streaming, 3).is_empty());
        }
    }
}
