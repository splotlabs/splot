// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mini-GOPs of 16 and 32 frames through the [`Encoder`].

use super::*;

/// Mini-GOPs of 16 frames.
const LONG: [(&str, &str); 2] = [("mini_gop_size", "16"), ("lookahead_frames", "16")];

/// Frame `index` of a texture panning one sample a frame.
fn pan_frame(config: &Config, index: u32) -> SourceFrame {
    crate::lookahead::tests::frame(config, u64::from(index), index as usize, 0)
}

fn stream(config: &Config, threads: usize, bulk: bool) -> Stream {
    tf_stream_of(config.clone(), &LONG, (threads, 34), bulk, &pan_frame)
}

/// Two 16-frame mini-GOPs (five layers) code every loop filter in some
/// frame, the frames of the second read the first's base frame, and the
/// stream decodes to the encoder recon with the same bytes on 1, 2 and 8
/// threads and for bulk and interleaved callers. VBR codes the preset's
/// `bitrate_mini_gop_size` instead.
#[test]
fn long_mini_gops_decode_to_the_encoder_recon_on_any_thread_count() {
    let mut config = config(160, 64, BitDepth::Eight, 9, 160);
    let long = stream(&config, 1, false);
    assert_eq!(long.bases, [0, 16, 32, 33]);
    assert_eq!(long.tools, [true; 5]);
    assert_same_samples(&decode(&long.units.concat()), &long.recon, "long");
    for threads in [2, 8] {
        assert!(
            stream(&config, threads, false).units == long.units,
            "{threads} threads"
        );
    }
    assert!(stream(&config, 1, true).units == long.units, "bulk");
    config.rate_control = RateControlMode::Vbr { bitrate_kbps: 200 };
    let vbr = stream(&config, 1, false);
    let pairs: Vec<u64> = (0..34).step_by(2).chain([33]).collect();
    assert_eq!(vbr.bases, pairs);
}

/// AVM decodes a stream with a 32-frame mini-GOP to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_32_frame_mini_gops_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let config = config(160, 64, BitDepth::Eight, 6, 160);
    let sets = [("mini_gop_size", "32"), ("lookahead_frames", "32")];
    let stream = tf_stream_of(config, &sets, (2, 40), false, &pan_frame);
    assert_eq!(stream.bases, [0, 32, 39]);
    let avm = avm_decode(
        &avmdec,
        &stream.units,
        (160, 64),
        BitDepth::Eight,
        "minigop",
    );
    assert!(avm == stream.recon, "AVM output differs");
}
