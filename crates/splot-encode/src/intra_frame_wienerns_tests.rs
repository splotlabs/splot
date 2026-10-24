// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma Wiener-NS frame filters (§ 5.20.10.6, § 7.20.3) in key and inter frames.

use splot_core::headers::frame::FrameRestorationType;

use super::*;
use crate::preset::PresetParams;

/// `preset` with `wiener_ns` bank fits.
fn with_wiener_ns(preset: &PresetParams, wiener_ns: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        wiener_ns,
        ..*preset
    }))
}

/// What a frame's luma restoration codes: `(type, temporal_pred_flag)`.
type LumaLr = (FrameRestorationType, bool);

/// A key frame and three inter frames of a two-tile `chroma` pan on `threads`
/// threads: the units, the encoder recon and the luma restoration of each frame.
fn wiener_stream(
    (threads, chroma): (usize, PixelFormat),
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<LumaLr>) {
    pan_stream((threads, chroma), preset, 1)
}

/// [`wiener_stream`] whose frame `i` shows pan position `i * speed`.
fn pan_stream(
    (threads, chroma): (usize, PixelFormat),
    preset: &'static PresetParams,
    speed: u32,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<LumaLr>) {
    let mut config = config(200, 96, BitDepth::Ten, 6, 190);
    config.chroma = chroma;
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (mut units, mut recon, mut lr) = (Vec::new(), Vec::new(), Vec::new());
    for index in 0..4 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let mut coded = coder
            .code_frame(&panning_frame(&config, index * speed), &plan, 190)
            .unwrap();
        let core = &coded.reference.as_ref().unwrap().0;
        let luma = core.lr_params.as_ref().unwrap().planes[0].clone();
        lr.push((luma.restoration_type, luma.rst_ref_pic_idx.is_some()));
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon, lr)
}

fn codes_wiener_ns((tool, _): LumaLr) -> bool {
    matches!(
        tool,
        FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
    )
}

/// Key and inter frames code a Wiener-NS bank, an inter frame reuses the bank
/// of its reference, the stream decodes to the encoder recon, and 1, 2 and 8
/// threads write the same bytes. Without the tool no frame codes it.
#[test]
fn wiener_ns_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let on = Preset::new(6).unwrap().params();
    let (one, recon, lr) = wiener_stream((1, PixelFormat::Yuv420), on);
    assert!(codes_wiener_ns(lr[0]), "{lr:?}");
    assert!(lr[1..].iter().copied().any(codes_wiener_ns), "{lr:?}");
    assert_same_samples(&decode(&one.concat()), &recon, "wiener-ns");
    for threads in [2, 8] {
        assert!(
            wiener_stream((threads, PixelFormat::Yuv420), on).0 == one,
            "{threads} threads"
        );
    }
    let (off, off_recon, off_lr) = wiener_stream((1, PixelFormat::Yuv420), with_wiener_ns(on, 0));
    assert!(!off_lr.iter().copied().any(codes_wiener_ns), "{off_lr:?}");
    assert_same_samples(&decode(&off.concat()), &off_recon, "no wiener-ns");
    assert_ne!(off, one);
}

/// A still scene reuses the bank of the primary reference, which decodes to
/// the encoder recon.
#[test]
fn temporal_wiener_ns_bank_decodes_to_the_encoder_recon() {
    let on = Preset::new(6).unwrap().params();
    let (units, recon, lr) = pan_stream((2, PixelFormat::Yuv420), with_wiener_ns(on, 1), 0);
    assert!(lr.iter().any(|&(_, temporal)| temporal), "{lr:?}");
    assert_same_samples(&decode(&units.concat()), &recon, "temporal");
}

/// Two-tile 4:2:2 and 4:4:4 frames code Wiener-NS and decode to the encoder recon.
#[test]
fn wiener_ns_422_and_444_tiles_decode_to_the_encoder_recon() {
    let on = Preset::new(6).unwrap().params();
    for chroma in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let (units, recon, lr) = wiener_stream((2, chroma), on);
        assert!(lr.iter().copied().any(codes_wiener_ns), "{chroma:?}: {lr:?}");
        assert_same_samples(&decode(&units.concat()), &recon, &format!("{chroma:?}"));
    }
}

/// AVM decodes the Wiener-NS key and inter frames, coded and temporal banks,
/// to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_wiener_ns_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let on = Preset::new(6).unwrap().params();
    for (speed, preset) in [(1, on), (0, with_wiener_ns(on, 1))] {
        let (units, recon, lr) = pan_stream((2, PixelFormat::Yuv420), preset, speed);
        assert!(lr.iter().copied().any(codes_wiener_ns), "{lr:?}");
        let avm = avm_decode(&avmdec, &units, (200, 96), BitDepth::Ten, "wienerns");
        assert!(avm == recon, "speed {speed}: AVM output differs");
    }
}
