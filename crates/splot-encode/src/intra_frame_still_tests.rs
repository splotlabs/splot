// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Single-picture streams (`Config::single_picture`, § 5.4.1, § 5.18.2).

use super::*;
use crate::{Encoder, Error, PixelFormat};

/// A `width x height` single-picture config at preset 9.
fn still_config(
    (width, height): (u32, u32),
    chroma: PixelFormat,
    bit_depth: BitDepth,
    threads: usize,
) -> Config {
    let mut config = config(width, height, bit_depth, 9, 150);
    config.chroma = chroma;
    config.single_picture = true;
    config.threads = splot_parallel::ThreadCount::from(threads);
    config
}

/// The temporal unit, recon and parsed frame header of the still `source`
/// of `config`, or a flat frame when `flat`.
fn still_unit(config: &Config, flat: bool) -> (Vec<u8>, Vec<u8>, FrameHeaderCore) {
    let mut coder = IntraFrameCoder::new(config).unwrap();
    assert!(coder.sequence.general.single_picture_header_flag);
    let source = if flat {
        let sizes = config.plane_sizes().unwrap();
        let value = config.bit_depth.max_sample() / 3;
        let planes = core::array::from_fn(|plane| {
            let count = sizes
                .get(plane)
                .map_or(0, |size| size.width() * size.height());
            vec![value; count]
        });
        SourceFrame::new(config, 0, planes).unwrap()
    } else {
        panning_frame(config, 0)
    };
    let plan = crate::gop::plan(0, config.keyint);
    let mut coded = coder.code_frame(&source, &plan, 150, &[]).unwrap();
    let core = coded.reference.as_ref().unwrap().0.clone();
    coder.commit(&mut coded).unwrap();
    (
        coded.temporal_unit.clone(),
        recon_bytes(&coded, config.bit_depth),
        core,
    )
}

const CASES: [(PixelFormat, BitDepth); 4] = [
    (PixelFormat::Yuv420, BitDepth::Eight),
    (PixelFormat::Yuv420, BitDepth::Ten),
    (PixelFormat::Yuv444, BitDepth::Eight),
    (PixelFormat::Yuv444, BitDepth::Ten),
];

/// Still pictures of 4:2:0 and 4:4:4 at 8 and 10 bit decode to the encoder
/// recon, a flat one codes the forced CDEF and CCSO enables with no-op
/// values, a 64x64 one (one GDF block) codes no GDF, and 1, 2 and 8 threads
/// write the same bytes.
#[test]
fn still_pictures_decode_to_the_encoder_recon() {
    for (chroma, depth) in CASES {
        for (size, flat) in [((200, 136), false), ((200, 136), true), ((64, 64), false)] {
            let case = format!("{chroma:?} {depth:?} {size:?} flat {flat}");
            let config = still_config(size, chroma, depth, 1);
            let (unit, recon, core) = still_unit(&config, flat);
            assert_eq!(
                core.frame_type,
                Some(splot_core::headers::frame::FrameType::Key)
            );
            assert_same_samples(&decode(&unit), &recon, &case);
            if flat {
                let cdef = core.cdef_params.unwrap();
                assert!(cdef.cdef_frame_enable, "{case}");
                assert!(
                    cdef.strengths.iter().all(|set| *set == Default::default()),
                    "{case}"
                );
                let ccso = core.ccso_params.unwrap();
                assert!(ccso.planes.iter().all(|plane| !plane.ccso_planes), "{case}");
            }
            if size == (64, 64) {
                assert!(
                    !core.gdf_params.is_some_and(|gdf| gdf.gdf_frame_enable),
                    "{case}"
                );
            }
            if (chroma, depth, flat) == (PixelFormat::Yuv420, BitDepth::Ten, false) {
                for threads in [2, 8] {
                    let config = still_config(size, chroma, depth, threads);
                    assert!(
                        still_unit(&config, flat).0 == unit,
                        "{case}: {threads} threads"
                    );
                }
            }
        }
    }
}

/// A single-picture encoder codes the first frame and refuses a second.
#[test]
fn single_picture_encoder_refuses_a_second_frame() {
    let config = still_config((64, 64), PixelFormat::Yuv420, BitDepth::Eight, 1);
    let mut encoder = Encoder::new(config.clone()).unwrap();
    encoder.send_frame(panning_frame(&config, 0)).unwrap();
    assert!(matches!(
        encoder.send_frame(panning_frame(&config, 1)),
        Err(Error::SecondSinglePicture)
    ));
    encoder.flush().unwrap();
    let mut units = Vec::new();
    while let Some(packet) = encoder.receive_packet().unwrap() {
        units.push(packet.data);
    }
    assert_eq!(decode(&units.concat()).len(), 64 * 64 * 3 / 2);
}

/// AVM decodes every still picture like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_still_pictures_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, depth) in CASES {
        for (size, flat) in [((200, 136), false), ((200, 136), true), ((64, 64), false)] {
            let name = format!("still-{chroma:?}-{depth:?}-{}x{}-{flat}", size.0, size.1);
            let (unit, recon, _) = still_unit(&still_config(size, chroma, depth, 1), flat);
            assert!(
                avm_decode(&avmdec, &[unit], size, depth, &name) == recon,
                "{name}"
            );
        }
    }
}
