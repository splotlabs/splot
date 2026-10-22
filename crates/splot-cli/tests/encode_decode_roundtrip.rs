// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end encoder oracle: `splot encode` codes a Y4M clip as key frames
//! and a mini-GOP with a hidden frame, `splot validate` finds the stream
//! conformant, and `splot decode` returns the frames in input order with the
//! PSNR against the source that the encoder reported. The IVF bytes do not
//! depend on `--threads`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::{splot, temp_input, temp_output};

const WIDTH: usize = 72;
const HEIGHT: usize = 40;
const FRAMES: usize = 6;

/// A Y4M clip with moving gradients and texture at `bits` per sample.
fn synthetic_y4m(bits: u32) -> (Vec<u8>, Vec<Vec<u16>>) {
    let tag = if bits == 8 { "C420jpeg" } else { "C420p10" };
    let mut y4m = format!("YUV4MPEG2 W{WIDTH} H{HEIGHT} F30:1 Ip A1:1 {tag}\n").into_bytes();
    let max = (1u32 << bits) - 1;
    let mut frames = Vec::new();
    for frame in 0..FRAMES {
        y4m.extend_from_slice(b"FRAME\n");
        let mut samples = Vec::new();
        for (plane, (w, h)) in [
            (WIDTH, HEIGHT),
            (WIDTH / 2, HEIGHT / 2),
            (WIDTH / 2, HEIGHT / 2),
        ]
        .into_iter()
        .enumerate()
        {
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

fn round_trip(bits: u32, rate_control: &[&str]) {
    let (y4m, source) = synthetic_y4m(bits);
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
    let mut psnr_y = [f64::NAN; FRAMES];
    for line in stats.lines().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        psnr_y[fields[1].parse::<usize>().unwrap()] = fields[4].parse().unwrap();
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
    let frame_len = WIDTH * HEIGHT * 3 / 2;
    assert_eq!(decoded.len(), FRAMES * frame_len);
    for (index, (frame, coded)) in source.iter().zip(decoded.chunks(frame_len)).enumerate() {
        let luma = WIDTH * HEIGHT;
        let measured = psnr(&frame[..luma], &coded[..luma], bits);
        assert!(
            (measured - psnr_y[index]).abs() < 1e-3,
            "frame {index}: decoded psnr {measured} != encoder psnr {}",
            psnr_y[index]
        );
        assert!(measured > 15.0, "frame {index}: psnr {measured}");
    }
    for path in [input, ivf, raw] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn eight_bit_clip_encodes_validates_and_decodes_to_the_encoder_stats() {
    round_trip(8, &["--qp", "90"]);
}

#[test]
fn ten_bit_clip_encodes_validates_and_decodes_to_the_encoder_stats() {
    round_trip(10, &["--qp", "90"]);
}

#[test]
fn crf_and_bitrate_clips_encode_validate_and_decode_to_the_encoder_stats() {
    round_trip(8, &["--crf", "25"]);
    round_trip(10, &["--bitrate", "200", "--rc", "cbr"]);
}

#[test]
fn encode_output_is_byte_identical_across_thread_counts() {
    let (y4m, _) = synthetic_y4m(8);
    let input = temp_input("y4m", &y4m);
    let input_s = input.to_str().unwrap();
    let mut streams = Vec::new();
    for threads in ["1", "2", "8"] {
        let ivf = temp_output("ivf");
        let ivf_s = ivf.to_str().unwrap();
        let encode = splot(&[
            "encode",
            input_s,
            "-o",
            ivf_s,
            "--keyint",
            "4",
            "--qp",
            "90",
            "--threads",
            threads,
        ]);
        assert!(encode.status.success(), "{encode:?}");
        streams.push((threads, std::fs::read(&ivf).unwrap()));
        let _ = std::fs::remove_file(ivf);
    }
    let _ = std::fs::remove_file(input);
    for (threads, bytes) in &streams[1..] {
        assert!(
            *bytes == streams[0].1,
            "--threads {threads} changed the IVF"
        );
    }
}
