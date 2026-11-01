// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end encoder oracle: `splot encode` codes a 4:2:0, 4:2:2 or 4:4:4
//! Y4M clip as key frames and a mini-GOP with a hidden frame, `splot validate`
//! finds the stream conformant, and `splot decode` returns the frames in input
//! order with the PSNR against the source that the encoder reported. The IVF
//! bytes do not depend on `--threads`. Coverage builds skip it: other tests
//! reach the gated lines, and instrumented encoding takes minutes.

#![cfg(not(coverage))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::{splot, temp_input, temp_output};

const WIDTH: usize = 72;
const HEIGHT: usize = 24;
const FRAMES: usize = 6;

/// Chroma subsampling `(x, y)` of 4:2:0, 4:2:2 and 4:4:4.
const YUV420: (usize, usize) = (1, 1);
const YUV422: (usize, usize) = (1, 0);
const YUV444: (usize, usize) = (0, 0);

/// The planes of a frame with chroma subsampling `(sub_x, sub_y)`.
fn plane_sizes((sub_x, sub_y): (usize, usize)) -> [(usize, usize); 3] {
    let chroma = (WIDTH >> sub_x, HEIGHT >> sub_y);
    [(WIDTH, HEIGHT), chroma, chroma]
}

/// A Y4M clip of `count` frames with moving gradients and texture at `bits`
/// per sample and chroma subsampling `subsampling`.
fn synthetic_y4m(bits: u32, subsampling: (usize, usize), count: usize) -> (Vec<u8>, Vec<Vec<u16>>) {
    let layout = ["444", "422", "420"][subsampling.0 + subsampling.1];
    let tag = match (bits, layout) {
        (8, "420") => "C420jpeg".to_owned(),
        (8, _) => format!("C{layout}"),
        _ => format!("C{layout}p10"),
    };
    let mut y4m = format!("YUV4MPEG2 W{WIDTH} H{HEIGHT} F30:1 Ip A1:1 {tag}\n").into_bytes();
    let max = (1u32 << bits) - 1;
    let mut frames = Vec::new();
    for frame in 0..count {
        y4m.extend_from_slice(b"FRAME\n");
        let mut samples = Vec::new();
        for (plane, (w, h)) in plane_sizes(subsampling).into_iter().enumerate() {
            for y in 0..h {
                for x in 0..w {
                    let texture = ((x * 7 + y * 13 + frame * 5 + plane * 3) % 23) as u32;
                    let ramp = ((x + y * 2 + frame * 4) as u32 * max) / 200;
                    let sample = ((ramp + texture * max / 64) % (max + 1)) as u16;
                    samples.push(sample);
                    if bits == 8 {
                        y4m.push(sample as u8);
                    } else {
                        y4m.extend_from_slice(&sample.to_le_bytes());
                    }
                }
            }
        }
        frames.push(samples);
    }
    (y4m, frames)
}

fn psnr(source: &[u16], decoded: &[u16], bits: u32) -> f64 {
    let sse: f64 = source
        .iter()
        .zip(decoded)
        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum();
    if sse == 0.0 {
        return f64::INFINITY;
    }
    let peak = f64::from((1u32 << bits) - 1);
    10.0 * (peak * peak * source.len() as f64 / sse).log10()
}

fn round_trip(bits: u32, subsampling: (usize, usize), rate_control: &[&str]) {
    let (y4m, source) = synthetic_y4m(bits, subsampling, FRAMES);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let raw = temp_output("yuv");
    let (input_s, ivf_s, raw_s) = (
        input.to_str().unwrap(),
        ivf.to_str().unwrap(),
        raw.to_str().unwrap(),
    );

    let mut args = vec!["encode", input_s, "-o", ivf_s, "--keyint", "4", "--stats"];
    args.extend(rate_control);
    let encode = splot(&args);
    assert!(encode.status.success(), "{encode:?}");
    let stats = String::from_utf8(encode.stdout).unwrap();
    let mut reported = [[f64::NAN; 3]; FRAMES];
    for line in stats.lines().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        reported[fields[1].parse::<usize>().unwrap()] =
            core::array::from_fn(|plane| fields[4 + plane].parse().unwrap());
    }
    assert_eq!(stats.lines().count(), FRAMES + 1, "{stats}");

    let validate = splot(&["validate", "--strict", ivf_s]);
    assert!(validate.status.success(), "{validate:?}");

    let decode = splot(&["decode", ivf_s, "--output", raw_s, "--output-format", "raw"]);
    assert!(decode.status.success(), "{decode:?}");
    let bytes = std::fs::read(&raw).unwrap();
    let decoded: Vec<u16> = if bits == 8 {
        bytes.iter().map(|&b| u16::from(b)).collect()
    } else {
        bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    };
    let sizes = plane_sizes(subsampling);
    let frame_len: usize = sizes.iter().map(|(w, h)| w * h).sum();
    assert_eq!(decoded.len(), FRAMES * frame_len);
    for (index, (frame, coded)) in source.iter().zip(decoded.chunks(frame_len)).enumerate() {
        let mut start = 0;
        for (plane, (w, h)) in sizes.into_iter().enumerate() {
            let span = start..start + w * h;
            start = span.end;
            let measured = psnr(&frame[span.clone()], &coded[span], bits);
            let expected = reported[index][plane];
            assert!(
                (measured - expected).abs() < 1e-3,
                "frame {index} plane {plane}: decoded psnr {measured} != encoder psnr {expected}"
            );
            assert!(
                measured > 15.0,
                "frame {index} plane {plane}: psnr {measured}"
            );
        }
    }
    for path in [input, ivf, raw] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn eight_bit_clip_encodes_validates_and_decodes_to_the_encoder_stats() {
    round_trip(8, YUV420, &["--qp", "90"]);
}

#[test]
fn ten_bit_clip_encodes_validates_and_decodes_to_the_encoder_stats() {
    round_trip(10, YUV420, &["--qp", "90"]);
}

#[test]
fn chroma_422_and_444_clips_encode_validate_and_decode_to_the_encoder_stats() {
    for bits in [8, 10] {
        round_trip(bits, YUV422, &["--qp", "90"]);
        round_trip(bits, YUV444, &["--qp", "90"]);
    }
}

/// `--low-delay` is the `mini_gop_size=1`, `bitrate_mini_gop_size=1` and
/// `lookahead_frames=1` overrides: every frame is coded and shown in input order.
#[test]
fn low_delay_clips_code_every_frame_in_input_order() {
    round_trip(8, YUV420, &["--qp", "90", "--low-delay"]);
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let encode = |flags: &[&str]| {
        let ivf = temp_output("ivf");
        let mut args = vec!["encode", input.to_str().unwrap(), "-o"];
        args.push(ivf.to_str().unwrap());
        args.extend(["--keyint", "16", "--stats"]);
        args.extend(flags);
        let out = splot(&args);
        assert!(out.status.success(), "{out:?}");
        let bytes = std::fs::read(&ivf).unwrap();
        let _ = std::fs::remove_file(ivf);
        (String::from_utf8(out.stdout).unwrap(), bytes)
    };
    let (stats, low_delay) = encode(&["--low-delay"]);
    let pts: Vec<&str> = stats
        .lines()
        .skip(1)
        .map(|line| line.split(',').nth(1).unwrap())
        .collect();
    assert_eq!(pts, ["0", "1", "2", "3", "4", "5"]);
    let overrides = [
        "--preset-set",
        "mini_gop_size=1",
        "--preset-set",
        "bitrate_mini_gop_size=1",
        "--preset-set",
        "lookahead_frames=1",
    ];
    assert_eq!(encode(&overrides).1, low_delay);
    assert_ne!(encode(&[]).1, low_delay);
    let _ = std::fs::remove_file(input);
}

/// `--resize-denominator 16` codes the 72x24 clip at 36x12: the IVF header,
/// the validated stream and its decode have the coded size.
#[test]
fn resized_clips_code_and_decode_at_the_scaled_size() {
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let raw = temp_output("yuv");
    let (input_s, ivf_s, raw_s) = (
        input.to_str().unwrap(),
        ivf.to_str().unwrap(),
        raw.to_str().unwrap(),
    );
    let encode = splot(&["encode", input_s, "-o", ivf_s, "--resize-denominator", "16"]);
    assert!(encode.status.success(), "{encode:?}");
    let header = std::fs::read(&ivf).unwrap();
    assert_eq!(header[12..16], [36, 0, 12, 0]);
    let validate = splot(&["validate", "--strict", ivf_s]);
    assert!(validate.status.success(), "{validate:?}");
    let decode = splot(&["decode", ivf_s, "--output", raw_s, "--output-format", "raw"]);
    assert!(decode.status.success(), "{decode:?}");
    assert_eq!(
        std::fs::read(&raw).unwrap().len(),
        FRAMES * (36 * 12 + 2 * 18 * 6)
    );
    let bad = splot(&["encode", input_s, "-o", ivf_s, "--resize-denominator", "8"]);
    assert_eq!(bad.status.code(), Some(2), "{bad:?}");
    for path in [input, ivf, raw] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn crf_and_bitrate_clips_encode_validate_and_decode_to_the_encoder_stats() {
    round_trip(8, YUV420, &["--crf", "25"]);
    round_trip(10, YUV420, &["--bitrate", "200", "--rc", "cbr"]);
    round_trip(8, YUV420, &["--bitrate", "200", "--two-pass"]);
}

/// Ten frames at presets 6 and 7 code a key frame, two mini-GOPs of 4 with
/// waves of leaf frames, and a short one; one tile and two tiles (preset 7
/// codes 64x64 superblocks) give the same IVF on one thread and on more
/// threads than tiles or leaf frames per wave.
#[test]
fn encode_420_output_is_byte_identical_across_thread_counts() {
    byte_identical_across_thread_counts(YUV420);
}

#[test]
fn encode_422_output_is_byte_identical_across_thread_counts() {
    byte_identical_across_thread_counts(YUV422);
}

#[test]
fn encode_444_output_is_byte_identical_across_thread_counts() {
    byte_identical_across_thread_counts(YUV444);
}

fn byte_identical_across_thread_counts(subsampling: (usize, usize)) {
    let (y4m, _) = synthetic_y4m(8, subsampling, 10);
    let input = temp_input("y4m", &y4m);
    let input_s = input.to_str().unwrap();
    for (tiles, preset) in [("1x1", "6"), ("2x1", "7")] {
        let mut streams = Vec::new();
        for threads in ["1", "3"] {
            let ivf = temp_output("ivf");
            let ivf_s = ivf.to_str().unwrap();
            let encode = splot(&[
                "encode",
                input_s,
                "-o",
                ivf_s,
                "--keyint",
                "16",
                "--preset",
                preset,
                "--qp",
                "90",
                "--tiles",
                tiles,
                "--threads",
                threads,
            ]);
            assert!(encode.status.success(), "{encode:?}");
            streams.push((threads, std::fs::read(&ivf).unwrap()));
            let _ = std::fs::remove_file(ivf);
        }
        for (threads, bytes) in &streams[1..] {
            assert!(
                *bytes == streams[0].1,
                "{subsampling:?} --tiles {tiles} --threads {threads} changed the IVF"
            );
        }
    }
    let _ = std::fs::remove_file(input);
}

#[test]
fn two_tile_clip_encodes_validates_and_decodes_to_the_encoder_stats() {
    round_trip(
        8,
        YUV420,
        &[
            "--qp",
            "90",
            "--preset",
            "10",
            "--tiles",
            "2x1",
            "--threads",
            "2",
        ],
    );
}
