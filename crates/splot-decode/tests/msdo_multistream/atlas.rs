// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use splot_core::headers::atlas_segment::AtlasSegmentMode;

const COLLISION_8: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/needs-external-hls/syn-atlas-global-local-id-scope-8bit.obu"
);
const COLLISION_10: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/needs-external-hls/syn-atlas-global-local-id-scope-10bit.obu"
);

#[test]
fn atlas_snapshots_follow_the_key_activation_and_keep_the_native_samples() {
    let timeline8 = include_str!(
        "../../../../tests/conformance/vectors/needs-external-hls/syn-atlas-global-local-id-scope-8bit.tsv"
    );
    let timeline10 = include_str!(
        "../../../../tests/conformance/vectors/needs-external-hls/syn-atlas-global-local-id-scope-10bit.tsv"
    );
    for (source, timeline, hash, per_stream) in [
        (
            COLLISION_8,
            timeline8,
            SHA_MSDO_8BIT_ALL,
            [(3, SHA_MSDO_8BIT_X3), (7, SHA_MSDO_8BIT_X7)],
        ),
        (
            COLLISION_10,
            timeline10,
            "e694723a6dac2ca92ee9177905f00afa3d4656a5b6bd550f459696e42eb4d441",
            [
                (
                    2,
                    "a19c49839344102ea1dc266b86c2943e47ae2746145326bfa01f14f0819f507b",
                ),
                (
                    9,
                    "366d1de38bc4faa23e6b65e81e9cd07ecfefd5f0c3ed163d459c4c561a2baac5",
                ),
            ],
        ),
    ] {
        for threads in [1usize, 3, 12] {
            let pics = stream_decode(
                source,
                &DecoderSettings::default().with_threads(ThreadCount::from(threads)),
            )
            .unwrap();
            let report = context(threads, 3)
                .decode_hash_report_reader(ChunkedReader::new(source, 7), DecodeOptions::DEFAULT)
                .unwrap();
            let (native, layers) = raw_all_and_by_xlayer(&pics);
            for (xlayer, expected) in per_stream {
                assert_eq!(sha256_hex(&layers[&xlayer]), expected);
            }
            assert_eq!(sha256_hex(&native), hash);
            assert_eq!(pics.len(), report.frames.len());
            let mut actual = String::from(
                "picture\txlayer\tglobal_offset\tglobal_mode\tglobal_segment\tglobal_index\tlocal_offset\n",
            );
            for (pic, frame) in pics.iter().zip(&report.frames) {
                let associations = pic.atlas_associations();
                assert_eq!(associations, frame.atlas_associations);
                assert_eq!(pic.extended_layer_id(), frame.extended_layer_id);
                let global = &associations[0];
                assert_eq!(global.scope, GLOBAL_XLAYER_ID);
                assert_eq!(global.atlas_id, 5);
                let layout = global.layout.as_ref().unwrap();
                assert!(!layout.external);
                let id = global.segment_id.unwrap();
                let index = global.segment_index().unwrap();
                let local_offset = match &associations[1..] {
                    [] => "-".to_owned(),
                    [local] => {
                        assert_eq!(local.scope, pic.extended_layer_id());
                        assert_eq!(local.atlas_id, 5);
                        assert_eq!(local.segment_id, Some(41));
                        let layout = local.layout.as_ref().unwrap();
                        assert_eq!(layout.atlas.mode, AtlasSegmentMode::Single);
                        assert_eq!(layout.atlas.label.segment_ids, [41]);
                        layout.offset.get().to_string()
                    }
                    rest => panic!("unexpected associations {rest:?}"),
                };
                writeln!(
                    &mut actual,
                    "{}\t{}\t{}\t{}\t{id}\t{index}\t{local_offset}",
                    frame.output_index,
                    pic.extended_layer_id().get(),
                    layout.offset.get(),
                    layout.atlas.mode.idc()
                )
                .unwrap();
            }
            assert_eq!(actual, timeline);
        }
    }
}

#[test]
fn optional_global_and_required_local_have_distinct_outcomes() {
    let mut no_global = Vec::new();
    let mut no_local = Vec::new();
    let mut first_local_lcr = None;
    for obu in parse_annex_b_obus(COLLISION_8).unwrap() {
        let global = obu.header.extended_layer_id.is_global();
        if obu.header.obu_type != ObuType::AtlasSegment || !global {
            write_obu(&mut no_global, obu.header, obu.payload);
        }
        if obu.header.obu_type != ObuType::AtlasSegment || global {
            if obu.header.obu_type == ObuType::LayerConfigurationRecord && !global {
                first_local_lcr.get_or_insert(no_local.len() as u64 + 1);
            }
            write_obu(&mut no_local, obu.header, obu.payload);
        }
    }
    let pics = stream_decode(&no_global, &DecoderSettings::default()).unwrap();
    assert!(!pics.is_empty());
    assert!(pics.iter().all(|p| {
        let global = &p.atlas_associations()[0];
        global.scope.is_global() && global.atlas_id == 5 && global.layout.is_none()
    }));
    let error = context(1, 1)
        .decode_hash_report_bytes(&no_local, DecodeOptions::DEFAULT)
        .unwrap_err();
    let DecodeError::MalformedSource { issue } = error else {
        panic!("{error:?}");
    };
    assert_eq!(
        issue.offset().map(splot_core::span::ByteOffset::get),
        first_local_lcr
    );
    assert_eq!(issue.spec_section(), Some("7.3.8.4"));
}

#[test]
fn all_atlas_modes_reach_decoded_pictures() {
    let modes: [&[u8]; 5] = [
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-atlas-mode-0.obu"),
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-atlas-mode-1.obu"),
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-atlas-mode-2.obu"),
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-atlas-mode-3.obu"),
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-atlas-mode-4.obu"),
    ];
    for (mode, stream) in modes.iter().enumerate() {
        let pictures = stream_decode(stream, &DecoderSettings::default()).unwrap();
        assert_eq!(pictures.len(), 2);
        assert!(pictures.iter().all(|p| {
            p.atlas_associations()[0]
                .layout
                .as_ref()
                .unwrap()
                .atlas
                .mode
                .idc()
                == mode as u32
        }));
    }
}

#[test]
fn external_atlas_supports_random_access_and_flush() {
    use splot_core::span::ByteOffset;
    use splot_decode::ExternalAtlases;

    let side8 = include_bytes!(
        "../../../../tests/conformance/vectors/needs-external-hls/syn-atlas-local-8bit.annexb"
    );
    let side10 = include_bytes!(
        "../../../../tests/conformance/vectors/needs-external-hls/syn-atlas-local-10bit.annexb"
    );
    for (bytes, offset, sidecar, xlayer) in [
        (COLLISION_8, 1862, side8.as_slice(), 3),
        (COLLISION_10, 5247, side10.as_slice(), 2),
    ] {
        let rap = &bytes[offset..];
        let ctx = context(1, 1);
        let err = ctx
            .decode_hash_report_bytes(rap, DecodeOptions::DEFAULT)
            .unwrap_err();
        let DecodeError::MalformedSource { issue } = err else {
            panic!("{err:?}");
        };
        assert_eq!(issue.spec_section(), Some("7.3.8.4"));
        let mut external = ExternalAtlases::default();
        external.insert(sidecar, ByteOffset::new(900)).unwrap();
        external.insert(sidecar, ByteOffset::new(1000)).unwrap();
        assert!(external.insert(sidecar, ByteOffset::new(u64::MAX)).is_err());
        assert!(
            external
                .insert(&[sidecar, sidecar].concat(), ByteOffset::new(2000))
                .is_err()
        );
        let external_ctx = ctx.with_external_atlases(external.clone());
        let expected = external_ctx
            .decode_hash_report_bytes(rap, DecodeOptions::DEFAULT)
            .unwrap();
        let mut decoder =
            Decoder::new_with_external_atlases(DecoderSettings::default(), external).unwrap();
        let previous = stream_pictures(&mut decoder, rap).unwrap();
        for (picture, frame) in previous.iter().zip(&expected.frames) {
            assert_eq!(picture.atlas_associations(), frame.atlas_associations);
        }
        let mut no_replacement = Vec::new();
        for obu in parse_annex_b_obus(rap).unwrap() {
            if obu.header.obu_type != ObuType::AtlasSegment
                || obu.header.extended_layer_id.is_global()
            {
                write_obu(&mut no_replacement, obu.header, obu.payload);
            }
        }
        decoder.flush();
        let subsequent = stream_pictures(&mut decoder, &no_replacement).unwrap();
        let local = |p: &Picture| {
            p.atlas_associations()
                .iter()
                .find(|a| a.scope.get() == xlayer)
                .unwrap()
                .layout
                .clone()
                .unwrap()
        };
        let first = previous
            .iter()
            .find(|p| p.extended_layer_id().get() == xlayer)
            .unwrap();
        let second = subsequent
            .iter()
            .find(|p| p.extended_layer_id().get() == xlayer)
            .unwrap();
        assert!(!local(first).external);
        assert!(local(second).external);
        assert_eq!(local(second).offset.get(), 1001);
        assert_eq!(previous.len(), subsequent.len());
        assert_eq!(
            raw_all_and_by_xlayer(&previous),
            raw_all_and_by_xlayer(&subsequent)
        );
    }
}

#[test]
fn atlas_label_lookup_uses_the_inverse_map_and_reports_missing_labels() {
    let mut changed = Vec::new();
    for obu in parse_annex_b_obus(COLLISION_8).unwrap() {
        if obu.offset.get() == 37 {
            let PayloadStatus::Parsed(ParsedObu::AtlasSegment(mut layout)) =
                obu.payload_status().unwrap()
            else {
                panic!("expected the first global atlas");
            };
            layout.label.segment_ids[1] = 41;
            write_parsed(&mut changed, obu.header, &ParsedObu::AtlasSegment(layout));
        } else {
            write_obu(&mut changed, obu.header, obu.payload);
        }
    }
    let ctx = context(1, 1);
    let first = ctx
        .decode_hash_report_bytes(
            &changed,
            DecodeOptions::DEFAULT
                .with_extended_layer_selection(ExtendedLayerSelection::OutputMask(1 << 3)),
        )
        .unwrap();
    assert_eq!(
        first.frames[0].atlas_associations[0].segment_index(),
        Some(1)
    );
    let error = ctx
        .decode_hash_report_bytes(&changed, DecodeOptions::DEFAULT)
        .unwrap_err();
    let DecodeError::MalformedSource { issue } = error else {
        panic!("{error:?}");
    };
    assert_eq!(issue.spec_section(), Some("6.9.2"));
    assert_eq!(
        issue.offset().map(splot_core::span::ByteOffset::get),
        Some(37)
    );
    assert!(issue.message().contains("77"));
}

#[test]
fn reading_atlas_geometry_respects_native_frame_limits_without_allocating_a_canvas() {
    use splot_core::headers::atlas_segment::AtlasModeInfo;
    let mode3 = include_bytes!("../../../../tests/conformance/vectors/valid/syn-atlas-mode-3.obu");
    let mut bytes = Vec::new();
    let virtual_size = 1 << 28;
    for obu in parse_annex_b_obus(mode3).unwrap() {
        if obu.header.obu_type == ObuType::AtlasSegment {
            let PayloadStatus::Parsed(ParsedObu::AtlasSegment(mut layout)) =
                obu.payload_status().unwrap()
            else {
                panic!("expected atlas payload");
            };
            let AtlasModeInfo::Multistream(info) = &mut layout.mode_info else {
                panic!("expected mode 3");
            };
            info.width = virtual_size;
            info.height = virtual_size;
            write_parsed(&mut bytes, obu.header, &ParsedObu::AtlasSegment(layout));
        } else {
            write_obu(&mut bytes, obu.header, obu.payload);
        }
    }
    let limits = DecodeLimits::DEFAULT
        .with_max_frame_width(DecodeLimitThreshold::Max(80))
        .with_max_frame_height(DecodeLimitThreshold::Max(64));
    let report = context(1, 1)
        .decode_hash_report_bytes(&bytes, DecodeOptions::new(limits))
        .unwrap();
    assert_eq!(report.frames.len(), 2);
    let layout = report.frames[0].atlas_associations[0]
        .layout
        .as_ref()
        .unwrap();
    let AtlasModeInfo::Multistream(info) = &layout.atlas.mode_info else {
        panic!("expected mode 3");
    };
    assert_eq!((info.width, info.height), (virtual_size, virtual_size));
}

const LOCAL_ATLAS_8: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/needs-external-hls/syn-atlas-local-8bit.annexb"
);

#[expect(
    clippy::panic,
    reason = "the fixture setup requires a parsed local LCR"
)]
fn local_atlas_lcr_without_global_parent() -> (LcrId, Vec<u8>) {
    use splot_core::headers::layer_config_record::LayerConfigurationRecord;

    let obu = parse_annex_b_obus(COLLISION_8)
        .unwrap()
        .into_iter()
        .find(|obu| {
            obu.header.obu_type == ObuType::LayerConfigurationRecord
                && !obu.header.extended_layer_id.is_global()
        })
        .unwrap();
    let PayloadStatus::Parsed(ParsedObu::LayerConfigurationRecord(mut record)) =
        obu.payload_status().unwrap()
    else {
        panic!("expected local LCR");
    };
    let LayerConfigurationRecord::Local(local) = &mut *record else {
        panic!("expected local LCR");
    };
    local.global_id = 0;
    let id = LcrId::from_bits(local.local_id);
    let mut bytes = Vec::new();
    write_parsed(
        &mut bytes,
        obu.header,
        &ParsedObu::LayerConfigurationRecord(record),
    );
    (id, bytes)
}

#[test]
fn local_atlas_precedes_the_first_decodable_frame_of_its_stream() {
    let (id, local_lcr) = local_atlas_lcr_without_global_parent();
    let units = temporal_units(MSDO_OPS_RA);
    for (retire_at_unaligned_rap, use_external_atlas) in
        [(false, false), (true, false), (true, true)]
    {
        let mut bytes = if retire_at_unaligned_rap {
            units[..2].concat()
        } else {
            Vec::new()
        };
        let mut external = splot_decode::ExternalAtlases::default();
        let mut atlas_offset = None;
        if use_external_atlas {
            external
                .insert(LOCAL_ATLAS_8, splot_core::span::ByteOffset::new(900))
                .unwrap();
            atlas_offset = Some(901);
        }
        for (tu, unit) in units.iter().enumerate().skip(2) {
            for obu in parse_annex_b_obus(unit).unwrap() {
                if retire_at_unaligned_rap && obu.header.obu_type == ObuType::Msdo {
                    let PayloadStatus::Parsed(ParsedObu::Msdo(mut msdo)) =
                        obu.payload_status().unwrap()
                    else {
                        panic!("expected MSDO");
                    };
                    msdo.multistream_level_idx += 1;
                    write_parsed(&mut bytes, obu.header, &ParsedObu::Msdo(msdo));
                    continue;
                }
                if obu.header.extended_layer_id.get() == 3 {
                    if tu == 2
                        && !use_external_atlas
                        && obu.header.obu_type == ObuType::SequenceHeader
                    {
                        atlas_offset = Some(bytes.len() as u64 + 1);
                        bytes.extend_from_slice(LOCAL_ATLAS_8);
                    }
                    if tu == 4 && obu.header.obu_type == ObuType::OperatingPointSet {
                        bytes.extend_from_slice(&local_lcr);
                    }
                    if tu == 4 && obu.header.obu_type == ObuType::SequenceHeader {
                        let PayloadStatus::Parsed(ParsedObu::SequenceHeader(mut seq)) =
                            obu.payload_status().unwrap()
                        else {
                            panic!("expected a sequence header");
                        };
                        seq.general.seq_lcr_id = id;
                        write_parsed(&mut bytes, obu.header, &ParsedObu::SequenceHeader(seq));
                        continue;
                    }
                }
                write_obu(&mut bytes, obu.header, obu.payload);
            }
        }
        let mut decoder =
            Decoder::new_with_external_atlases(DecoderSettings::default(), external.clone())
                .unwrap();
        let pictures = stream_pictures(&mut decoder, &bytes).unwrap();
        // AVM emits donor pictures 5, 7, 8, 9, 10 and 11 after access, plus 0–3 before transition.
        let native = if retire_at_unaligned_rap {
            "0b301c2b9c4a946cd3d2516c703433b07d33da0d51345d4f0fa1306935fc990d"
        } else {
            "41f6c331f13e350167753d725765da86e88b6ac093db9038fc44a5864b82c3a1"
        };
        assert_eq!(sha256_hex(&raw_all_and_by_xlayer(&pictures).0), native);
        let active: Vec<_> = pictures
            .iter()
            .filter(|p| p.extended_layer_id().get() == 3)
            .collect();
        let prior_pictures = if retire_at_unaligned_rap { 2 } else { 0 };
        assert_eq!(active.len(), prior_pictures + 2);
        assert!(
            active[..prior_pictures]
                .iter()
                .all(|picture| picture.atlas_associations().is_empty())
        );
        let report = context(3, 2)
            .with_external_atlases(external)
            .decode_hash_report_bytes(&bytes, DecodeOptions::DEFAULT)
            .unwrap();
        assert_eq!(report.frames.len(), pictures.len());
        for (frame, picture) in report.frames.iter().zip(&pictures) {
            assert_eq!(frame.atlas_associations, picture.atlas_associations());
        }
        for picture in &active[prior_pictures..] {
            let [local] = picture.atlas_associations() else {
                panic!("expected only the local atlas");
            };
            assert_eq!(local.segment_id, Some(41));
            assert_eq!(local.layout.as_ref().unwrap().external, use_external_atlas);
            assert_eq!(
                local.layout.as_ref().map(|layout| layout.offset.get()),
                atlas_offset
            );
        }
    }
}

#[test]
fn pending_local_lcr_shadows_global_at_the_next_cmvs_boundary() {
    let collision = parse_annex_b_obus(COLLISION_8).unwrap();
    let global = collision
        .iter()
        .find(|obu| {
            obu.header.extended_layer_id.is_global()
                && obu.header.obu_type == ObuType::LayerConfigurationRecord
        })
        .unwrap();
    let (id, local_lcr) = local_atlas_lcr_without_global_parent();
    let units = temporal_units(MSDO_OPS_RA);
    for include_local in [true, false] {
        let mut stream = Vec::new();
        let mut sequence_offset = None;
        for (tu, unit) in units.iter().enumerate().take(5).skip(2) {
            for obu in parse_annex_b_obus(unit).unwrap() {
                let kind = obu.header.obu_type;
                let x3 = obu.header.extended_layer_id.get() == 3;
                if tu == 4 {
                    if kind == ObuType::TemporalDelimiter {
                        write_obu(&mut stream, obu.header, obu.payload);
                        write_obu(&mut stream, global.header, global.payload);
                    } else if x3 && kind == ObuType::SequenceHeader {
                        let PayloadStatus::Parsed(ParsedObu::SequenceHeader(mut seq)) =
                            obu.payload_status().unwrap()
                        else {
                            panic!("expected a sequence header");
                        };
                        seq.general.seq_lcr_id = id;
                        sequence_offset = Some(stream.len() as u64 + 1);
                        write_parsed(&mut stream, obu.header, &ParsedObu::SequenceHeader(seq));
                    } else if x3 && kind == ObuType::ClosedLoopKey {
                        write_obu(&mut stream, obu.header, obu.payload);
                    }
                    continue;
                }
                if tu == 2 && x3 && kind == ObuType::SequenceHeader {
                    stream.extend_from_slice(LOCAL_ATLAS_8);
                    if include_local {
                        stream.extend_from_slice(&local_lcr);
                    }
                }
                write_obu(&mut stream, obu.header, obu.payload);
            }
        }
        let streaming = stream_decode(&stream, &DecoderSettings::default());
        let batch = context(3, 2).decode_hash_report_bytes(&stream, DecodeOptions::DEFAULT);
        if include_local {
            let pictures = streaming.unwrap();
            assert_eq!(
                sha256_hex(&raw_all_and_by_xlayer(&pictures).0),
                "51e56b0ab0ec25c38846b7d054b2cf9de179f96b6d56555cfe4e36c544913d9b",
                "AVM donor output indices 5, 7 and 8"
            );
            let report = batch.unwrap();
            let last = pictures.last().unwrap();
            assert_eq!(last.extended_layer_id().get(), 3);
            let [association] = last.atlas_associations() else {
                panic!("expected a local atlas")
            };
            assert_eq!(association.scope.get(), 3);
            assert_eq!(association.segment_id, Some(41));
            assert_eq!(
                last.atlas_associations(),
                report.frames.last().unwrap().atlas_associations
            );
        } else {
            for err in [streaming.unwrap_err(), batch.unwrap_err()] {
                let DecodeError::MalformedSource { issue } = err else {
                    panic!("{err:?}")
                };
                assert_eq!(issue.spec_section(), Some("7.3.2"));
                assert_eq!(
                    issue.offset().map(splot_core::span::ByteOffset::get),
                    sequence_offset
                );
            }
        }
    }
}

#[test]
fn sequence_header_binds_its_lcr_before_a_later_local_record() {
    let obus = parse_annex_b_obus(COLLISION_8).unwrap();
    for resend_header in [false, true] {
        let mut bytes = Vec::new();
        let mut tu = 0;
        let mut global_lcr = Vec::new();
        let mut global_atlas = Vec::new();
        let mut local_lcr = 0;
        let mut local_atlas = 0;
        for obu in &obus {
            let kind = obu.header.obu_type;
            let layer = obu.header.extended_layer_id;
            if kind == ObuType::TemporalDelimiter {
                tu += 1;
            }
            if tu == 3 && layer.get() == 3 && kind == ObuType::SequenceHeader && !resend_header {
                continue;
            }
            let offset = bytes.len() as u64 + 1;
            match (kind, layer.is_global()) {
                (ObuType::LayerConfigurationRecord, true) => global_lcr.push(offset),
                (ObuType::AtlasSegment, true) => global_atlas.push(offset),
                (ObuType::LayerConfigurationRecord, false) => local_lcr = offset,
                (ObuType::AtlasSegment, false) => local_atlas = offset,
                _ => {}
            }
            write_obu(&mut bytes, obu.header, obu.payload);
        }
        let pictures = stream_decode(&bytes, &DecoderSettings::default()).unwrap();
        let report = context(2, 2)
            .decode_hash_report_reader(ChunkedReader::new(&bytes, 3), DecodeOptions::DEFAULT)
            .unwrap();
        assert_eq!(
            sha256_hex(&raw_all_and_by_xlayer(&pictures).0),
            SHA_MSDO_8BIT_ALL
        );
        assert_eq!(pictures.len(), 8);
        assert_eq!(report.frames.len(), pictures.len());
        for (index, (pic, frame)) in pictures.iter().zip(&report.frames).enumerate() {
            let second = index >= 4;
            let x3 = pic.extended_layer_id().get() == 3;
            let associations = pic.atlas_associations();
            assert_eq!(frame.atlas_associations, associations);
            let expect_local = x3 && second && resend_header;
            assert_eq!(associations.len(), 1 + usize::from(expect_local));
            let global = &associations[0];
            assert!(global.scope.is_global());
            let rebind = second && (!x3 || resend_header);
            assert_eq!(
                global.lcr_offset.unwrap().get(),
                global_lcr[usize::from(rebind)]
            );
            assert_eq!(
                global.layout.as_ref().unwrap().offset.get(),
                global_atlas[usize::from(second)]
            );
            if expect_local {
                let local = &associations[1];
                assert_eq!(local.scope.get(), 3);
                assert_eq!(local.lcr_offset.unwrap().get(), local_lcr);
                assert_eq!(local.layout.as_ref().unwrap().offset.get(), local_atlas);
            }
        }
    }
}

#[test]
fn local_record_with_id_zero_fails_at_its_obu() {
    use splot_core::headers::layer_config_record::LayerConfigurationRecord;

    let mut bytes = Vec::new();
    let mut invalid_offset = None;
    for obu in parse_annex_b_obus(COLLISION_8).unwrap() {
        if invalid_offset.is_none()
            && obu.header.obu_type == ObuType::LayerConfigurationRecord
            && !obu.header.extended_layer_id.is_global()
        {
            let PayloadStatus::Parsed(ParsedObu::LayerConfigurationRecord(mut record)) =
                obu.payload_status().unwrap()
            else {
                panic!("expected the local LCR");
            };
            let LayerConfigurationRecord::Local(local) = &mut *record else {
                panic!("expected the local LCR");
            };
            local.local_id = 0;
            invalid_offset = Some(obu.offset);
            write_parsed(
                &mut bytes,
                obu.header,
                &ParsedObu::LayerConfigurationRecord(record),
            );
        } else {
            write_obu(&mut bytes, obu.header, obu.payload);
        }
    }
    for error in [
        stream_decode(&bytes, &DecoderSettings::default()).unwrap_err(),
        context(2, 2)
            .decode_hash_report_reader(ChunkedReader::new(&bytes, 3), DecodeOptions::DEFAULT)
            .unwrap_err(),
    ] {
        let DecodeError::MalformedSource { issue } = error else {
            panic!("{error:?}");
        };
        assert_eq!(
            issue.kind(),
            splot_decode::DecodeSourceIssueKind::ObuPayloadParseError
        );
        assert_eq!(issue.spec_section(), Some("6.8.3"));
        assert_eq!(issue.offset(), invalid_offset);
    }
}

#[test]
fn derived_sef_selects_the_atlas_segment_of_the_reference_layer() {
    use splot_core::headers::layer_config_record::LayerConfigurationRecord;

    let fixture = include_bytes!(
        "../../../../tests/conformance/vectors/valid/syn-mlayer2-derived-sef-8bit.obu"
    );
    let atlas = parse_annex_b_obus(include_bytes!(
        "../../../../tests/conformance/vectors/valid/syn-atlas-mode-0.obu"
    ))
    .unwrap()
    .into_iter()
    .find(|obu| obu.header.obu_type == ObuType::AtlasSegment)
    .unwrap();
    let donor = parse_annex_b_obus(COLLISION_8)
        .unwrap()
        .into_iter()
        .find(|obu| {
            obu.header.obu_type == ObuType::LayerConfigurationRecord
                && obu.header.extended_layer_id.is_global()
        })
        .unwrap();
    let PayloadStatus::Parsed(ParsedObu::LayerConfigurationRecord(mut record)) =
        donor.payload_status().unwrap()
    else {
        panic!("expected a global LCR");
    };
    let LayerConfigurationRecord::Global(global) = &mut *record else {
        panic!("expected a global LCR");
    };
    global.xlayer_map = 1;
    global.payloads.truncate(1);
    let payload = &mut global.payloads[0];
    payload.xlayer_id = 0;
    // The second embedded-layer entry occupies six bytes (§ 5.8.8).
    payload.data_size += 6;
    let embedded = payload.xlayer_info.embedded_layer_info.as_mut().unwrap();
    embedded.mlayer_map = 3;
    embedded.layers[0].tlayer_map = 3;
    let mut enhancement = embedded.layers[0];
    enhancement.mlayer_index = 1;
    enhancement.atlas_segment_id = Some(77);
    enhancement.dependent_layer_map = Some(1);
    embedded.layers.push(enhancement);

    let mut prefix = Vec::new();
    write_parsed(
        &mut prefix,
        donor.header,
        &ParsedObu::LayerConfigurationRecord(record),
    );
    write_obu(&mut prefix, atlas.header, atlas.payload);
    let mut stream = Vec::new();
    for obu in parse_annex_b_obus(fixture).unwrap() {
        if obu.header.obu_type == ObuType::LayerConfigurationRecord {
            stream.extend_from_slice(&prefix);
        } else {
            write_obu(&mut stream, obu.header, obu.payload);
        }
    }
    let pictures = stream_decode(&stream, &DecoderSettings::default()).unwrap();
    assert_eq!(
        sha256_hex(&raw_all_and_by_xlayer(&pictures).0),
        "bca7d45ad940d920c77414eea6516e243bd0ea98d58441a6de270fa1fb231033"
    );
    assert_eq!(pictures.len(), 4);
    for (pic, (mlayer, label)) in pictures.iter().zip([(0, 41), (1, 77), (0, 41), (0, 41)]) {
        let [association] = pic.atlas_associations() else {
            panic!("expected one global atlas")
        };
        assert_eq!(pic.embedded_layer_id().get(), mlayer);
        assert_eq!(pic.temporal_layer_id().get(), 0);
        assert_eq!(association.segment_id, Some(label));
        assert_eq!(association.segment_index(), Some(usize::from(mlayer)));
    }
}
