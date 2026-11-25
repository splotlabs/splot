// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end checks for atlas presentation and raw canvas output.

#[expect(
    clippy::expect_used,
    reason = "The shared test helpers require their fixture paths and child processes to exist."
)]
mod common;

use common::{conformance_vector, sha256_hex, splot, temp_input, temp_output};
use splot_core::annexb::parse_annex_b_obus;
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::stream_reader::TemporalUnitReader;

#[test]
fn atlas_presentation_matches_independent_raw_planes() {
    for (fixture, bytes, digest) in [
        (
            "syn-atlas-multistream-side-by-side-8bit.obu",
            55_296,
            "85b9b854163552e0c00ee6d0acc5b5ce36356da01f4e3b4d0357a303626cc249",
        ),
        (
            "syn-atlas-alpha-paired-8bit.obu",
            55_296,
            "20a8e51779232b000421884c4b7e4b8f8fa74780d7c1fa93f9f1259dfe159d2d",
        ),
        (
            "syn-atlas-multistream-side-by-side-10bit.obu",
            276_480,
            "748bcce6e28d05c5bd9348e5dc37a159dd324eeeaca674c45f2d9d88a599c313",
        ),
        (
            "syn-atlas-alpha-paired-10bit.obu",
            276_480,
            "2286b9dc5c613cf1da66d598a6a67bc2da6ad37095006fd3a19ce4124f49adb4",
        ),
    ] {
        let input = conformance_vector(fixture);
        for threads in ["1", "3", "12"] {
            let result = splot(&[
                "compose",
                input.to_str().unwrap(),
                "--threads",
                threads,
                "-o",
                "-",
            ]);
            assert!(
                result.status.success(),
                "{fixture}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(result.stdout.len(), bytes);
            assert_eq!(sha256_hex(&result.stdout), digest);
        }
    }
}

#[test]
fn repeated_ivf_timestamps_keep_each_temporal_unit_distinct() {
    let input = std::fs::read(conformance_vector("syn-atlas-alpha-paired-10bit.obu")).unwrap();
    let mut units = TemporalUnitReader::new(input.as_slice());
    let mut ivf = Vec::new();
    write_ivf_header(&mut ivf, &IvfHeader::new(*b"AV02", 144, 64, 1, 1, 10)).unwrap();
    while let Some(unit) = units.next_temporal_unit().unwrap() {
        write_ivf_frame(&mut ivf, 9, unit.bytes).unwrap();
    }
    let path = temp_input("ivf", &ivf);
    let result = splot(&["compose", path.to_str().unwrap(), "-o", "-"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        sha256_hex(&result.stdout),
        "2286b9dc5c613cf1da66d598a6a67bc2da6ad37095006fd3a19ce4124f49adb4"
    );
}

#[test]
fn a_failed_composition_preserves_existing_output() {
    let output = temp_output("yuv");
    std::fs::write(&output, b"existing").unwrap();
    let mut bytes = std::fs::read(conformance_vector("syn-atlas-alpha-paired-8bit.obu")).unwrap();
    let last_left = parse_annex_b_obus(&bytes)
        .unwrap()
        .into_iter()
        .rfind(|obu| obu.header.extended_layer_id.get() == 3)
        .unwrap();
    bytes.truncate((last_left.offset.get() + u64::from(last_left.size)) as usize);
    let input = temp_input("obu", &bytes);
    let result = splot(&[
        "compose",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("lacks one or more atlas pictures"));
    assert_eq!(std::fs::read(output).unwrap(), b"existing");
}

#[test]
fn two_global_layouts_require_an_explicit_atlas_id() {
    use splot_core::types::ObuType;

    let mut stream = std::fs::read(conformance_vector("syn-msdo-two-streams-8bit.obu")).unwrap();
    let source = std::fs::read(conformance_vector(
        "syn-atlas-multistream-side-by-side-8bit.obu",
    ))
    .unwrap();
    let atlas = parse_annex_b_obus(&source)
        .unwrap()
        .into_iter()
        .find(|obu| obu.header.obu_type == ObuType::AtlasSegment)
        .unwrap();
    let header = atlas.offset.get() as usize;
    let size = atlas.size as usize;
    assert!(size < 128);
    let mut records = Vec::new();
    for id in [2, 5] {
        let mut copy = source[header..header + size].to_vec();
        let payload = usize::from(atlas.header.header_size_bytes);
        copy[payload] = (copy[payload] & 0x1f) | (id << 5);
        records.push(size as u8);
        records.extend(copy);
    }
    let offsets: Vec<_> = parse_annex_b_obus(&stream)
        .unwrap()
        .into_iter()
        .filter(|obu| obu.header.obu_type == ObuType::Msdo)
        .map(|obu| (obu.offset.get() + u64::from(obu.size)) as usize)
        .collect();
    for at in offsets.into_iter().rev() {
        stream.splice(at..at, records.iter().copied());
    }
    let input = temp_input("obu", &stream);
    let path = input.to_str().unwrap();
    let ambiguous = splot(&["compose", path, "-o", "-"]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("Select one with --atlas-id"));
    let selected = splot(&["compose", path, "-o", "-", "--atlas-id", "5"]);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(selected.stdout.len(), 55_296);
}

#[test]
fn pending_atlas_pictures_respect_the_byte_limit() {
    let input = conformance_vector("syn-atlas-alpha-paired-8bit.obu");
    let result = splot(&[
        "compose",
        input.to_str().unwrap(),
        "-o",
        "-",
        "--max-bytes",
        "6144",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Pending atlas pictures exceed"));
    assert!(result.stdout.is_empty());
}
