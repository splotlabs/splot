// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

mod common;

use common::{conformance_vector, splot, temp_input, temp_output};
use sha2::{Digest as _, Sha256};
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::stream_reader::TemporalUnitReader;

#[test]
fn atlas_presentation_matches_independent_raw_planes() {
    for (fixture, bytes, digest) in [
        (
            "syn-atlas-multistream-side-by-side-8bit.obu",
            55296,
            "85b9b854163552e0c00ee6d0acc5b5ce36356da01f4e3b4d0357a303626cc249",
        ),
        (
            "syn-atlas-alpha-paired-8bit.obu",
            55296,
            "20a8e51779232b000421884c4b7e4b8f8fa74780d7c1fa93f9f1259dfe159d2d",
        ),
        (
            "syn-atlas-multistream-side-by-side-10bit.obu",
            276480,
            "748bcce6e28d05c5bd9348e5dc37a159dd324eeeaca674c45f2d9d88a599c313",
        ),
        (
            "syn-atlas-alpha-paired-10bit.obu",
            276480,
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
            assert_eq!(format!("{:x}", Sha256::digest(result.stdout)), digest);
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
        format!("{:x}", Sha256::digest(result.stdout)),
        "2286b9dc5c613cf1da66d598a6a67bc2da6ad37095006fd3a19ce4124f49adb4"
    );
}

#[test]
fn a_failed_composition_preserves_existing_output() {
    let output = temp_output("yuv");
    std::fs::write(&output, b"existing").unwrap();
    let input = conformance_vector("syn-msdo-two-streams-8bit.obu");
    let result = splot(&[
        "compose",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no multistream atlas"));
    assert_eq!(std::fs::read(output).unwrap(), b"existing");
}
