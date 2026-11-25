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

fn atlas_units(fixture: &str) -> anyhow::Result<Vec<Vec<u8>>> {
    let source = std::fs::read(conformance_vector(fixture))?;
    let mut reader = TemporalUnitReader::new(source.as_slice());
    let mut units = Vec::new();
    while let Some(unit) = reader.next_temporal_unit()? {
        units.push(unit.bytes.to_vec());
    }
    Ok(units)
}

fn compose_at_one_timestamp(units: &[Vec<u8>]) -> anyhow::Result<std::process::Output> {
    let count = u32::try_from(units.len())?;
    let mut ivf = Vec::new();
    write_ivf_header(&mut ivf, &IvfHeader::new(*b"AV02", 144, 64, 1, 1, count))?;
    for unit in units {
        write_ivf_frame(&mut ivf, 9, unit)?;
    }
    let path = temp_input("ivf", &ivf);
    Ok(splot(&[
        "compose",
        &path.to_string_lossy(),
        "-o",
        "-",
        "--threads",
        "1",
    ]))
}

#[test]
fn repeated_ivf_timestamps_keep_each_temporal_unit_distinct() -> anyhow::Result<()> {
    let units = atlas_units("syn-atlas-alpha-paired-10bit.obu")?;
    let result = compose_at_one_timestamp(&units)?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        sha256_hex(&result.stdout),
        "2286b9dc5c613cf1da66d598a6a67bc2da6ad37095006fd3a19ce4124f49adb4"
    );
    Ok(())
}

#[test]
fn late_atlas_pictures_obey_the_reorder_window_boundary() -> anyhow::Result<()> {
    let units = atlas_units("syn-atlas-alpha-paired-10bit.obu")?;
    // TU 1 reaches the compositor after TU 3. Empty IVF records increase that distance.
    for (padding, accepted) in [(29, true), (30, false)] {
        let mut records = units.clone();
        records.splice(2..2, std::iter::repeat_n(Vec::new(), padding));
        let result = compose_at_one_timestamp(&records)?;
        assert_eq!(
            result.status.success(),
            accepted,
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if accepted {
            assert_eq!(
                sha256_hex(&result.stdout),
                "2286b9dc5c613cf1da66d598a6a67bc2da6ad37095006fd3a19ce4124f49adb4"
            );
        } else {
            assert!(
                String::from_utf8_lossy(&result.stderr)
                    .contains("Pictures exceed the reorder window of 32 temporal units")
            );
        }
    }
    Ok(())
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

#[test]
fn external_atlas_restores_composition_at_random_access() {
    use splot_core::types::ObuType;

    let stream = std::fs::read(conformance_vector("syn-atlas-alpha-paired-8bit.obu")).unwrap();
    let mut units = TemporalUnitReader::new(stream.as_slice());
    let mut suffix = Vec::new();
    let mut sidecar = Vec::new();
    while let Some(unit) = units.next_temporal_unit().unwrap() {
        if unit.index < 2 {
            continue;
        }
        let mut start = 0;
        for obu in parse_annex_b_obus(unit.bytes).unwrap() {
            let end = (obu.offset.get() + u64::from(obu.size)) as usize;
            let record = &unit.bytes[start..end];
            if obu.header.obu_type == ObuType::AtlasSegment {
                sidecar.extend_from_slice(record);
            } else {
                suffix.extend_from_slice(record);
            }
            start = end;
        }
    }
    let input = temp_input("obu", &suffix);
    let atlas = temp_input("annexb", &sidecar);
    let output = temp_output("yuv");
    std::fs::write(&output, b"existing").unwrap();
    let args = [
        "compose",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ];
    let missing = splot(&args);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("Global atlas 5 is unavailable"));
    assert_eq!(std::fs::read(&output).unwrap(), b"existing");
    let mut args = args.to_vec();
    args.extend(["--external-atlas", atlas.to_str().unwrap()]);
    let result = splot(&args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let samples = std::fs::read(output).unwrap();
    assert_eq!(samples.len(), 27_648);
    assert_eq!(
        sha256_hex(&samples),
        "3be335cfc4560e3c6b1675729501fd20e288f23c09a537cc37c434b427a6390c"
    );
}
