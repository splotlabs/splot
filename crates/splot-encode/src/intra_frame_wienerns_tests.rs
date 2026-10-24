// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma and chroma Wiener-NS frame filters (§ 5.20.10.6, § 7.20.3) in key
//! and inter frames.

use splot_core::headers::frame::FrameRestorationType;

use super::*;
use crate::preset::PresetParams;

/// `preset` with `(luma, chroma)` bank fits.
fn with_wiener_ns(
    preset: &PresetParams,
    (wiener_ns, wiener_ns_chroma): (u8, u8),
) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        wiener_ns,
        wiener_ns_chroma,
        ..*preset
    }))
}

/// What each plane's restoration of a frame codes: `(type, temporal_pred_flag)`.
type FrameLr = [(FrameRestorationType, bool); 3];

/// A key frame and three inter frames of a two-tile `chroma` pan on `threads`
/// threads: the units, the encoder recon and the luma restoration of each frame.
fn wiener_stream(
    (threads, chroma): (usize, PixelFormat),
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<FrameLr>) {
    pan_stream((threads, chroma), preset, 1)
}

/// [`wiener_stream`] whose frame `i` shows pan position `i * speed`.
fn pan_stream(
    (threads, chroma): (usize, PixelFormat),
    preset: &'static PresetParams,
    speed: u32,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<FrameLr>) {
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
        let params = core.lr_params.as_ref().unwrap();
        if params
            .planes
            .iter()
            .skip(1)
            .any(|p| p.restoration_type != FrameRestorationType::None)
        {
            let sub_y = u32::from(chroma == PixelFormat::Yuv420);
            assert!(params.loop_restoration_size[1] >= 64 >> sub_y, "{params:?}");
        }
        let planes = &params.planes;
        lr.push(core::array::from_fn(|plane| {
            planes
                .get(plane)
                .map_or((FrameRestorationType::None, false), |p| {
                    (p.restoration_type, p.rst_ref_pic_idx.is_some())
                })
        }));
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon, lr)
}

/// Whether a frame codes a Wiener-NS bank on one of `planes`.
fn codes_wiener_ns(lr: &[FrameLr], planes: core::ops::Range<usize>) -> bool {
    lr.iter().any(|frame| {
        frame[planes.clone()].iter().any(|(tool, _)| {
            matches!(
                tool,
                FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
            )
        })
    })
}

/// Key and inter frames code luma and chroma Wiener-NS banks, the stream
/// decodes to the encoder recon, and 1 and 3 threads write the same bytes.
/// Without the tool no plane codes it; without the chroma fits only luma does.
#[test]
fn wiener_ns_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let on = Preset::new(6).unwrap().params();
    let (one, recon, lr) = wiener_stream((1, PixelFormat::Yuv420), on);
    assert!(codes_wiener_ns(&lr[..1], 0..1), "{lr:?}");
    assert!(codes_wiener_ns(&lr[1..], 0..1), "{lr:?}");
    assert!(codes_wiener_ns(&lr, 1..3), "{lr:?}");
    assert_same_samples(&decode(&one.concat()), &recon, "wiener-ns");
    assert!(
        wiener_stream((3, PixelFormat::Yuv420), on).0 == one,
        "3 threads"
    );
    let (off, off_recon, off_lr) =
        wiener_stream((1, PixelFormat::Yuv420), with_wiener_ns(on, (0, 0)));
    assert!(!codes_wiener_ns(&off_lr, 0..3), "{off_lr:?}");
    assert_same_samples(&decode(&off.concat()), &off_recon, "no wiener-ns");
    assert_ne!(off, one);
    let (luma, luma_recon, luma_lr) =
        wiener_stream((1, PixelFormat::Yuv420), with_wiener_ns(on, (2, 0)));
    assert!(
        codes_wiener_ns(&luma_lr, 0..1) && !codes_wiener_ns(&luma_lr, 1..3),
        "{luma_lr:?}"
    );
    assert_same_samples(&decode(&luma.concat()), &luma_recon, "luma wiener-ns");
    assert_ne!(luma, one);
}

/// A still scene reuses the bank of the primary reference, which decodes to
/// the encoder recon.
#[test]
fn temporal_wiener_ns_bank_decodes_to_the_encoder_recon() {
    let on = Preset::new(6).unwrap().params();
    let (units, recon, lr) = pan_stream((2, PixelFormat::Yuv420), with_wiener_ns(on, (1, 1)), 0);
    assert!(lr.iter().flatten().any(|&(_, temporal)| temporal), "{lr:?}");
    assert_same_samples(&decode(&units.concat()), &recon, "temporal");
}

/// Two-tile 4:2:2 and 4:4:4 frames code luma and chroma Wiener-NS and
/// decode to the encoder recon.
#[test]
fn wiener_ns_422_and_444_tiles_decode_to_the_encoder_recon() {
    let on = Preset::new(6).unwrap().params();
    for chroma in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let (units, recon, lr) = wiener_stream((2, chroma), on);
        assert!(
            codes_wiener_ns(&lr, 0..1) && codes_wiener_ns(&lr, 1..3),
            "{chroma:?}: {lr:?}"
        );
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
    for (speed, preset) in [(1, on), (0, with_wiener_ns(on, (1, 1)))] {
        let (units, recon, lr) = pan_stream((2, PixelFormat::Yuv420), preset, speed);
        assert!(
            codes_wiener_ns(&lr, 0..1) && codes_wiener_ns(&lr, 1..3),
            "{lr:?}"
        );
        let avm = avm_decode(&avmdec, &units, (200, 96), BitDepth::Ten, "wienerns");
        assert!(avm == recon, "speed {speed}: AVM output differs");
    }
}
