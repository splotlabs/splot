// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end encoder oracle: `splot encode` codes a 4:2:0, 4:2:2 or 4:4:4
//! Y4M clip as key frames and a mini-GOP with a hidden frame, `splot validate`
//! finds the stream conformant, and `splot decode` returns the frames in input
//! order with the PSNR against the source that the encoder reported. The IVF
//! bytes do not depend on `--threads`. Coverage builds skip it: other tests
//! reach the gated lines, and instrumented encoding takes minutes.

#![cfg(not(coverage))]
#![expect(clippy::unwrap_used, clippy::expect_used)]

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
            let measured = psnr(&frame[span.clone()], &coded[span.clone()], bits);
            let expected = reported[index][plane];
            if rate_control.contains(&"--lossless") {
                assert_eq!(
                    frame[span.clone()],
                    coded[span.clone()],
                    "frame {index} plane {plane}"
                );
            }
            assert!(
                (measured.is_infinite() && expected.is_infinite())
                    || (measured - expected).abs() < 1e-3,
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

/// `--profile 2` on 4:2:0 and `--profile 31` on 4:4:4 give streams that
/// validate with `--strict` and decode to the encoder stats; a reserved
/// profile, or one that does not allow the chroma format, is refused.
#[test]
fn requested_profiles_validate_and_decode() {
    round_trip(8, YUV420, &["--qp", "90", "--profile", "2"]);
    round_trip(10, YUV444, &["--qp", "90", "--profile", "31"]);
    let (y4m, _) = synthetic_y4m(8, YUV444, 1);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let (input_s, ivf_s) = (input.to_str().unwrap(), ivf.to_str().unwrap());
    for bad in ["3", "5", "32"] {
        let bad = splot(&["encode", input_s, "-o", ivf_s, "--profile", bad]);
        assert_eq!(bad.status.code(), Some(2), "{bad:?}");
    }
    for path in [input, ivf] {
        let _ = std::fs::remove_file(path);
    }
}

/// `--low-delay` is the `mini_gop_size=1`, `bitrate_mini_gop_size=1` and
/// `lookahead_frames=1` overrides: every frame is coded and shown in input order.
/// With `implicit_output` the IVF holds one frame per input frame, in coding
/// order with non-monotonic pts, and `splot decode` returns every frame in
/// input order.
#[test]
fn implicit_output_clips_decode_in_input_order() {
    let flags = ["--qp", "90", "--preset-set", "implicit_output=true"];
    round_trip(8, YUV420, &flags);
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let mut args = vec![
        "encode",
        input.to_str().unwrap(),
        "-o",
        ivf.to_str().unwrap(),
    ];
    args.extend(["--keyint", "4"]);
    args.extend(flags);
    assert!(splot(&args).status.success());
    let bytes = std::fs::read(&ivf).unwrap();
    let (mut at, mut pts) = (32, Vec::new());
    while at + 12 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        pts.push(u64::from_le_bytes(
            bytes[at + 4..at + 12].try_into().unwrap(),
        ));
        at += 12 + size;
    }
    assert!(!pts.is_sorted(), "{pts:?}");
    pts.sort_unstable();
    assert_eq!(pts, (0..FRAMES as u64).collect::<Vec<_>>());
    for path in [input, ivf] {
        let _ = std::fs::remove_file(path);
    }
}

/// `--multi-frame-header` streams validate with `--strict` and decode to
/// the reported frames.
#[test]
fn multi_frame_header_clips_validate_and_decode() {
    round_trip(8, YUV420, &["--qp", "90", "--multi-frame-header"]);
}

/// `--output-format annexb` writes the IVF frame payloads back to back with
/// no container header; the raw stream validates with `--strict` and decodes
/// to the frames of the IVF stream.
/// AVM reads the raw Annex B file (it detects the format; no option) and
/// decodes it to the frames of the IVF file.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_annexb_output_like_the_ivf_output() {
    let avmdec = std::env::var_os("SPLOT_AVM_DECODER").expect("set SPLOT_AVM_DECODER");
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let streams = [temp_output("ivf"), temp_output("obu")];
    let raws = [temp_output("yuv"), temp_output("yuv")];
    for (stream, format) in streams.iter().zip(["ivf", "annexb"]) {
        let (input, stream) = (input.to_str().unwrap(), stream.to_str().unwrap());
        let args = [
            "encode",
            input,
            "-o",
            stream,
            "--qp",
            "90",
            "--output-format",
            format,
        ];
        let encode = splot(&args);
        assert!(encode.status.success(), "{encode:?}");
    }
    for (stream, raw) in streams.iter().zip(&raws) {
        let status = std::process::Command::new(&avmdec)
            .arg("--rawvideo")
            .arg("-o")
            .arg(raw)
            .arg(stream)
            .status()
            .unwrap();
        assert!(status.success(), "avmdec {}", stream.display());
    }
    let frames = std::fs::read(&raws[0]).unwrap();
    assert_eq!(frames.len(), FRAMES * (72 * 24 + 2 * 36 * 12));
    assert_eq!(std::fs::read(&raws[1]).unwrap(), frames);
    for path in [&input].into_iter().chain(&streams).chain(&raws) {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn annexb_output_is_the_ivf_payloads_without_the_container() {
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let (ivf, obu, raw_ivf, raw_obu) = (
        temp_output("ivf"),
        temp_output("obu"),
        temp_output("yuv"),
        temp_output("yuv"),
    );
    let paths = [&input, &ivf, &obu, &raw_ivf, &raw_obu].map(|path| path.to_str().unwrap());
    let [input_s, ivf_s, obu_s, raw_ivf_s, raw_obu_s] = paths;
    let encode = splot(&["encode", input_s, "-o", ivf_s, "--qp", "90"]);
    assert!(encode.status.success(), "{encode:?}");
    let annexb = ["--qp", "90", "--output-format", "annexb"];
    let encode = splot(&[&["encode", input_s, "-o", obu_s][..], &annexb].concat());
    assert!(encode.status.success(), "{encode:?}");
    let (ivf_bytes, obu_bytes) = (std::fs::read(&ivf).unwrap(), std::fs::read(&obu).unwrap());
    let mut payloads = Vec::new();
    let mut at = 32;
    while at < ivf_bytes.len() {
        let size = u32::from_le_bytes(ivf_bytes[at..at + 4].try_into().unwrap()) as usize;
        payloads.extend_from_slice(&ivf_bytes[at + 12..at + 12 + size]);
        at += 12 + size;
    }
    assert_eq!(obu_bytes, payloads);
    let validate = splot(&["validate", "--strict", obu_s]);
    assert!(validate.status.success(), "{validate:?}");
    for (stream, raw) in [(ivf_s, raw_ivf_s), (obu_s, raw_obu_s)] {
        let decode = splot(&["decode", stream, "--output", raw, "--output-format", "raw"]);
        assert!(decode.status.success(), "{decode:?}");
    }
    assert_eq!(
        std::fs::read(&raw_obu).unwrap(),
        std::fs::read(&raw_ivf).unwrap()
    );
    for path in [input, ivf, obu, raw_ivf, raw_obu] {
        let _ = std::fs::remove_file(path);
    }
}

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

/// Without a rate-control flag `splot encode` codes `--crf 20`, not the flat
/// `--qp 80`, and `--max-bitrate` alone caps that CRF.
#[test]
fn the_default_rate_control_is_crf_20() {
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let encode = |flags: &[&str]| {
        let ivf = temp_output("ivf");
        let mut args = vec!["encode", input.to_str().unwrap(), "-o"];
        args.push(ivf.to_str().unwrap());
        args.extend(flags);
        let out = splot(&args);
        assert!(out.status.success(), "{out:?}");
        let bytes = std::fs::read(&ivf).unwrap();
        let _ = std::fs::remove_file(ivf);
        bytes
    };
    let default = encode(&[]);
    assert_eq!(default, encode(&["--crf", "20"]));
    assert_ne!(default, encode(&["--qp", "80"]));
    let capped = encode(&["--max-bitrate", "50", "--buffer", "500"]);
    assert_eq!(
        capped,
        encode(&["--crf", "20", "--max-bitrate", "50", "--buffer", "500"])
    );
    assert_ne!(capped, default, "the cap does not bind");
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

/// `--crop 2,5,2,4` codes the 72x24 clip with a cropping window: the IVF
/// header keeps the coded size, the validated stream decodes to the 65x18
/// window, and a window larger than the frame or an odd 4:2:0 left offset
/// is a usage error.
#[test]
fn cropped_clips_decode_to_the_window() {
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let raw = temp_output("yuv");
    let (input_s, ivf_s, raw_s) = (
        input.to_str().unwrap(),
        ivf.to_str().unwrap(),
        raw.to_str().unwrap(),
    );
    let encode = splot(&["encode", input_s, "-o", ivf_s, "--crop", "2,5,2,4"]);
    assert!(encode.status.success(), "{encode:?}");
    let header = std::fs::read(&ivf).unwrap();
    assert_eq!(header[12..16], [72, 0, 24, 0]);
    let validate = splot(&["validate", "--strict", ivf_s]);
    assert!(validate.status.success(), "{validate:?}");
    let decode = splot(&["decode", ivf_s, "--output", raw_s, "--output-format", "raw"]);
    assert!(decode.status.success(), "{decode:?}");
    assert_eq!(
        std::fs::read(&raw).unwrap().len(),
        FRAMES * (65 * 18 + 2 * 33 * 9)
    );
    for bad in ["40,40,0,0", "1,2,2,4", "1,2,3"] {
        let bad = splot(&["encode", input_s, "-o", ivf_s, "--crop", bad]);
        assert_eq!(bad.status.code(), Some(2), "{bad:?}");
    }
    for path in [input, ivf, raw] {
        let _ = std::fs::remove_file(path);
    }
}

/// `--cbr-padding` with `--rc cbr` gives a stream that validates with
/// `--strict` and decodes to every frame; without CBR it is a usage error.
#[test]
fn cbr_padding_clips_validate_and_decode() {
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let raw = temp_output("yuv");
    let (input_s, ivf_s, raw_s) = (
        input.to_str().unwrap(),
        ivf.to_str().unwrap(),
        raw.to_str().unwrap(),
    );
    let cbr = ["--bitrate", "500", "--rc", "cbr", "--cbr-padding"];
    let encode = splot(&[&["encode", input_s, "-o", ivf_s][..], &cbr].concat());
    assert!(encode.status.success(), "{encode:?}");
    let validate = splot(&["validate", "--strict", ivf_s]);
    assert!(validate.status.success(), "{validate:?}");
    let decode = splot(&["decode", ivf_s, "--output", raw_s, "--output-format", "raw"]);
    assert!(decode.status.success(), "{decode:?}");
    assert_eq!(
        std::fs::read(&raw).unwrap().len(),
        FRAMES * (72 * 24 + 2 * 36 * 12)
    );
    let bad = splot(&["encode", input_s, "-o", ivf_s, "--cbr-padding"]);
    assert_eq!(bad.status.code(), Some(2), "{bad:?}");
    for path in [input, ivf, raw] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn crf_and_bitrate_clips_encode_validate_and_decode_to_the_encoder_stats() {
    round_trip(8, YUV420, &["--crf", "25"]);
    round_trip(10, YUV420, &["--bitrate", "200", "--rc", "cbr"]);
    let small = ["--buffer", "250", "--initial-buffer", "50"];
    round_trip(
        8,
        YUV420,
        &[&["--bitrate", "200", "--rc", "cbr"][..], &small].concat(),
    );
    round_trip(8, YUV420, &["--bitrate", "200", "--two-pass"]);
    round_trip(
        8,
        YUV420,
        &["--crf", "20", "--max-bitrate", "100", "--buffer", "500"],
    );
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

/// `--tile-groups 2` codes each frame of a 2-tile clip in two tile group
/// OBUs that validate with `--strict`; `--tile-groups 0` is a usage error.
#[test]
fn tile_group_clips_validate_and_decode() {
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
            "--tile-groups",
            "2",
        ],
    );
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let (input_s, ivf_s) = (input.to_str().unwrap(), ivf.to_str().unwrap());
    let bad = splot(&["encode", input_s, "-o", ivf_s, "--tile-groups", "0"]);
    assert_eq!(bad.status.code(), Some(2), "{bad:?}");
    let _ = std::fs::remove_file(input);
}

/// `--tile-widths 1` codes a non-uniform 2-tile layout in 2 tile groups
/// that validates with `--strict` and decodes; with `--tiles 2x1` it is
/// refused.
#[test]
fn explicit_tile_clips_validate_and_decode() {
    round_trip(
        8,
        YUV420,
        &[
            "--qp",
            "90",
            "--preset",
            "10",
            "--tile-widths",
            "1",
            "--tile-groups",
            "2",
        ],
    );
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let (input_s, ivf_s) = (input.to_str().unwrap(), ivf.to_str().unwrap());
    let args = ["encode", input_s, "-o", ivf_s, "--tiles", "2x1"];
    let bad = splot(&[&args[..], &["--tile-widths", "1"]].concat());
    assert!(!bad.status.success(), "{bad:?}");
    let _ = std::fs::remove_file(input);
}

/// The content flags and the Y4M `A` tag reach the stream: every key frame
/// has a content interpretation OBU and the two HDR metadata OBUs, and the
/// clip still validates and decodes. `splot inspect` may name a metadata
/// type more than once per OBU.
#[test]
fn content_flags_write_a_content_interpretation_obu() {
    let content = [
        ["--color-primaries", "9"],
        ["--transfer-characteristics", "16"],
        ["--matrix-coefficients", "9"],
        ["--color-range", "full"],
        ["--chroma-sample-position", "left"],
        ["--sar", "4:3"],
        [
            "--mastering-display",
            "G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(10000000,1)",
        ],
        ["--content-light", "1000,400"],
    ];
    let mut flags = vec!["--qp", "90", "--timing-info"];
    flags.extend(content.as_flattened());
    round_trip(8, YUV420, &flags);

    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let ivf = temp_output("ivf");
    let mut args = vec!["encode", input.to_str().unwrap(), "-o"];
    args.extend([ivf.to_str().unwrap(), "--keyint", "4"]);
    args.extend(flags);
    assert!(splot(&args).status.success());
    let inspect = splot(&["inspect", "--json", ivf.to_str().unwrap()]);
    assert!(inspect.status.success(), "{inspect:?}");
    let json = String::from_utf8(inspect.stdout).unwrap();
    let described = json.matches("\"color_description_present\": true").count();
    assert_eq!(described, FRAMES.div_ceil(4), "{json}");
    for field in ["chroma_sample_position", "aspect_ratio_info", "timing_info"] {
        let present = format!("\"{field}_present\": true");
        assert_eq!(json.matches(&present).count(), described, "{field}");
    }
    for name in ["METADATA_TYPE_HDR_MDCV", "METADATA_TYPE_HDR_CLL"] {
        let named = json.matches(name).count();
        assert!(named > 0 && named % described == 0, "{name}: {named}");
    }

    for bad in [
        ["--sar", "4:0"],
        ["--mastering-display", "G(1,2)B(3,4)R(5,6)WP(7,8)"],
        ["--content-light", "1000"],
    ] {
        let mut args = vec!["encode", input.to_str().unwrap(), "-o"];
        args.push(ivf.to_str().unwrap());
        args.extend(bad);
        assert!(!splot(&args).status.success(), "{bad:?}");
    }
    for path in [input, ivf] {
        let _ = std::fs::remove_file(path);
    }
}

/// `--lossless` decodes to the source samples at presets 0, 6 and 10 without
/// `--preset-set`, and in 4:4:4 at 10 bits.
#[test]
fn lossless_clips_decode_to_the_source_samples() {
    for preset in ["0", "6", "10"] {
        round_trip(8, YUV420, &["--lossless", "--preset", preset]);
    }
    round_trip(10, YUV444, &["--lossless", "--preset", "6"]);
}

/// `--icc`, `--itut-t35` and `--user-data-unregistered` put one metadata OBU
/// of each file in front of every key frame, the stream does not depend on
/// the thread count, and a file that its syntax cannot carry is refused.
#[test]
fn metadata_files_write_metadata_obus() {
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let icc = temp_input("icc", &[0x49; 130]);
    let t35 = temp_input("t35", &[0xFF, 0x01, 0x00, 0x3C]);
    let user = temp_input("uuid", b"0123456789abcdefuser data");
    let files = [
        ["--icc", icc.to_str().unwrap()],
        ["--itut-t35", t35.to_str().unwrap()],
        ["--user-data-unregistered", user.to_str().unwrap()],
    ];
    let encode = |threads: &str, extra: &[&str]| {
        let ivf = temp_output("ivf");
        let mut args = vec!["encode", input.to_str().unwrap(), "-o"];
        args.extend([ivf.to_str().unwrap(), "--keyint", "4", "--threads", threads]);
        args.extend(extra);
        let status = splot(&args).status.success();
        let bytes = std::fs::read(&ivf).unwrap_or_default();
        let _ = std::fs::remove_file(&ivf);
        (status, bytes)
    };
    let (ok, stream) = encode("1", files.as_flattened());
    assert!(ok);
    assert_eq!(encode("8", files.as_flattened()), (true, stream.clone()));
    let ivf = temp_output("ivf");
    std::fs::write(&ivf, &stream).unwrap();
    for args in [vec!["inspect", "--json"], vec!["validate", "--strict"]] {
        let mut args = args;
        args.push(ivf.to_str().unwrap());
        let output = splot(&args);
        assert!(output.status.success(), "{output:?}");
        if args[0] == "inspect" {
            let json = String::from_utf8(output.stdout).unwrap();
            for name in [
                "METADATA_TYPE_ICC_PROFILE",
                "METADATA_TYPE_ITUT_T35",
                "METADATA_TYPE_USER_DATA_UNREGISTERED",
            ] {
                let named = json.matches(name).count();
                assert!(
                    named > 0 && named % FRAMES.div_ceil(4) == 0,
                    "{name}: {named}"
                );
            }
        }
    }

    let bad_icc = temp_input("icc", &[0x49; 127]);
    let bad_t35 = temp_input("t35", &[0xFF]);
    let bad_user = temp_input("uuid", &[0; 15]);
    for bad in [
        ["--icc", bad_icc.to_str().unwrap()],
        ["--itut-t35", bad_t35.to_str().unwrap()],
        ["--user-data-unregistered", bad_user.to_str().unwrap()],
        ["--icc", "/nonexistent/splot.icc"],
    ] {
        assert!(!encode("1", &bad).0, "{bad:?}");
    }
    for path in [input, icc, t35, user, bad_icc, bad_t35, bad_user, ivf] {
        let _ = std::fs::remove_file(path);
    }
}

/// `--timecode`, `--temporal-point-info`, `--scan-type-info` and
/// `--banding-hints` put one metadata OBU of each kind in front of every
/// output frame, `splot validate --strict` accepts the stream, and a
/// stream without timing info or a bad banding hints file is refused.
#[test]
fn frame_metadata_flags_write_metadata_obus() {
    let (y4m, _) = synthetic_y4m(8, YUV420, FRAMES);
    let input = temp_input("y4m", &y4m);
    let banding = temp_input("banding", &[0xA8, 0xA6]);
    let bad_banding = temp_input("banding", &[0xA8]);
    let ivf = temp_output("ivf");
    let encode = |extra: &[&str]| {
        let mut args = vec!["encode", input.to_str().unwrap(), "-o"];
        args.extend([ivf.to_str().unwrap(), "--keyint", "4"]);
        args.extend(extra);
        splot(&args).status.success()
    };
    let flags = [
        "--timing-info",
        "--timecode",
        "--temporal-point-info",
        "--scan-type-info",
        "--frame-hash",
        "per-plane",
        "--banding-hints",
        banding.to_str().unwrap(),
    ];
    assert!(encode(&flags));
    for args in [vec!["inspect", "--json"], vec!["validate", "--strict"]] {
        let mut args = args;
        args.push(ivf.to_str().unwrap());
        let output = splot(&args);
        assert!(output.status.success(), "{output:?}");
        if args[0] == "inspect" {
            let json = String::from_utf8(output.stdout).unwrap();
            for name in [
                "METADATA_TYPE_TIMECODE",
                "METADATA_TYPE_TEMPORAL_POINT_INFO",
                "METADATA_TYPE_SCAN_TYPE",
                "METADATA_TYPE_BANDING_HINTS",
                "METADATA_TYPE_DECODED_FRAME_HASH",
            ] {
                let named = json.matches(name).count();
                assert!(named >= FRAMES && named % FRAMES == 0, "{name}: {named}");
            }
        }
    }
    assert!(!encode(&flags[1..]));
    assert!(!encode(&["--banding-hints", bad_banding.to_str().unwrap()]));
    for path in [input, banding, bad_banding, ivf] {
        let _ = std::fs::remove_file(path);
    }
}
