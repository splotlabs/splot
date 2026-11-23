// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Source-activity segmentation maps (§ 5.18.7.1, § 5.20.5.8) in key and
//! inter frames.

use super::delta_q::{half_flat_frame, tpl_stream_of};
use super::*;

/// [`tpl_stream_of`] of half-flat frames with `segmentation = 1`.
fn segment_stream(
    mut config: Config,
    strength: u8,
    threads: usize,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<[bool; 7]>) {
    config
        .preset_overrides
        .push(("segmentation".into(), "1".into()));
    tpl_stream_of(config, strength, threads, half_flat_frame)
}

/// `segmentation = 1` codes a map in every key and inter frame, alone and
/// over the temporal model's delta-q with quantizer matrices, at 4:2:0 and
/// 4:4:4 and 8 and 10 bits:
/// the stream decodes to the encoder recon with every loop filter coding in
/// some frame, has the same bytes on 1, 2 and 8 threads, and differs from
/// the stream without the map.
#[test]
fn segment_maps_decode_to_the_encoder_recon_on_any_thread_count() {
    each_case(
        [
            (BitDepth::Eight, PixelFormat::Yuv420, 0, "0"),
            (BitDepth::Ten, PixelFormat::Yuv444, 100, "1"),
        ],
        |(bit_depth, chroma, strength, quant_matrices)| {
            let mut config = config(136, 136, bit_depth, 9, 0);
            config.chroma = chroma;
            config
                .preset_overrides
                .push(("quant_matrices".into(), quant_matrices.into()));
            let name = format!("{bit_depth:?} {chroma:?} {strength}");
            let (units, recon, tools) = segment_stream(config.clone(), strength, 1);
            assert_same_samples(&decode(&units.concat()), &recon, &name);
            assert!(tools.iter().all(|t| t[6]), "{name}: {tools:?}");
            assert_eq!(tools.iter().any(|t| t[0]), strength > 0, "{name}");
            let used: [bool; 5] = core::array::from_fn(|i| tools.iter().any(|t| t[i + 1]));
            assert_eq!(used, [true; 5], "{name}: {tools:?}");
            let (off, _, off_tools) = tpl_stream_of(config.clone(), strength, 1, half_flat_frame);
            assert!(off != units && off_tools.iter().all(|t| !t[6]), "{name}");
            if strength > 0 {
                assert!(segment_stream(config, strength, 2).0 == units, "{name}: 2");
            }
        },
    );
}

#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_segment_maps_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (bit_depth, strength) in [(BitDepth::Eight, 0), (BitDepth::Ten, 100)] {
        let (units, recon, _) = segment_stream(config(200, 136, bit_depth, 6, 0), strength, 2);
        let avm = avm_decode(&avmdec, &units, (200, 136), bit_depth, "segmentation");
        assert!(avm == recon, "{bit_depth:?} {strength}: AVM output differs");
    }
}
