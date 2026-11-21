// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter secondary transforms and DDT kernels (§ 5.20.8.2, § 7.15.3,
//! § 7.15.4 `get_transform_1d_type`).

use super::large_inter::{in_first_inter_frame, preset6};
use super::mvpred::top_left;
use super::*;
use crate::preset::PresetParams;
use splot_decode::encoder_syntax::{IntraModes, Ist};

/// Preset 6 with `inter_ist` and `inter_ddt`.
fn with_tools(inter_ist: bool, inter_ddt: bool) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        inter_ddt,
        inter_ist,
        ..*preset6(true)
    }))
}

/// The facade codes `sec_tx_type` for a `DCT_DCT` inter luma block of at
/// least 16x16 with `eob > 3` and no set, and resolves its secondary
/// transform with the inter kernel; an 8x8 inter block codes none.
#[test]
fn inter_luma_blocks_code_ist_as_the_decoder_reads_it() {
    let config = config(136, 136, BitDepth::Eight, 6, 110);
    in_first_inter_frame(&config, with_tools(true, false), |syntax, root, _| {
        let prediction = InterPrediction::Single(InterModes {
            ref_frame: 0,
            mode: InterMode::Global,
            drl: 0,
            mv: MotionVector::default(),
            filter: splot_recon::InterpolationFilter::EightTap,
            precision: None,
            amvd: false,
            interintra: None,
            bawp: None,
        });
        for (size, coded) in [(32, true), (16, true), (8, false)] {
            let leaf = syntax.leaf(top_left(syntax, root, size))?;
            let luma = syntax.plan_inter_leaf(&leaf, prediction)?[0];
            assert!(syntax.leaf_is_inter());
            assert_eq!(syntax.ist_coded(&luma, 4)?, coded, "{size}");
            assert!(!syntax.ist_coded(&luma, 3)?, "{size}");
            if coded {
                assert_eq!(syntax.ist_sets(&luma)?, 1);
                let ist = Ist {
                    sec_tx_type: 2,
                    set: 0,
                };
                let tx = TransformBlock { ist, ..luma };
                assert!(syntax.secondary_transform(&tx)?.is_some());
            }
        }
        Ok(())
    });
}

/// With inter IST, and with DDT, panning streams of 4:2:0 10-bit and 4:4:4
/// 8-bit decode to the encoder recon; IST changes the stream.
#[test]
fn inter_ist_and_ddt_streams_decode_to_the_encoder_recon() {
    each_case(
        [
            (PixelFormat::Yuv420, BitDepth::Ten),
            (PixelFormat::Yuv444, BitDepth::Eight),
        ],
        |(chroma, bit_depth)| {
            let mut config = config(72, 40, bit_depth, 6, 90);
            config.chroma = chroma;
            let mut streams = Vec::new();
            for (ist, ddt) in [(true, false), (false, false), (false, true)] {
                let (units, expected, cores, coder) =
                    super::carry::planned_stream(&config, with_tools(ist, ddt));
                let tq = coder.sequence.transform_quant_entropy.unwrap();
                assert_eq!((tq.enable_inter_ist, tq.enable_inter_ddt), (ist, ddt));
                assert!(cores.iter().any(|core| core.inter.is_some()));
                let name = format!("{chroma:?} ist {ist} ddt {ddt}");
                assert_same_samples(&decode(&units), &expected, &name);
                streams.push(units);
            }
            assert_ne!(streams[0], streams[1], "{chroma:?}");
        },
    );
}

/// RD over the inter IST kernels changes the stream against the
/// most-energy kernel and chooses another kernel; the streams decode to the
/// encoder recon at both bit depths with the same bytes on 1, 2 and 8
/// threads.
#[test]
fn inter_ist_rd_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    let (mut chosen, mut differs) = (0, false);
    for (width, height, bit_depth, qp) in [
        (136, 72, BitDepth::Eight, 150),
        (72, 40, BitDepth::Ten, 190),
    ] {
        let stream = |rd: bool, threads: usize| {
            let mut config = config(width, height, bit_depth, 6, 90);
            config.threads = splot_parallel::ThreadCount::from(threads);
            let params = Box::leak(Box::new(PresetParams {
                inter_ist_rd: rd,
                ..*with_tools(true, false)
            }));
            let coder = IntraFrameCoder::with_preset(&config, params).unwrap();
            super::carry::planned_stream_of(&config, coder, super::panning_frame, qp)
        };
        IST_RD_OTHER.set(0);
        let (units, expected, _, _) = stream(true, 1);
        chosen += IST_RD_OTHER.get();
        assert_same_samples(&decode(&units), &expected, &format!("{bit_depth:?}"));
        differs |= stream(false, 1).0 != units;
        for threads in [2, 8] {
            assert!(stream(true, threads).0 == units, "{bit_depth:?} {threads}");
        }
    }
    assert!(differs, "RD over the kernels coded the same streams");
    assert!(
        chosen > 0,
        "RD never chose another kernel than the energy one"
    );
}

/// A tiled stream with inter IST codes the same bytes on 1, 2 and 8 threads.
#[test]
fn inter_ist_streams_match_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 90);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        super::carry::planned_stream(&config, with_tools(true, false)).0
    };
    let one = stream(1);
    each_case([2, 8], |threads| {
        assert!(stream(threads) == one, "{threads} threads");
    });
}

/// DDT follows the planned leaf like the decoder's `useDdt`: an inter leaf of
/// a DDT sequence transforms with DDT, an intra leaf does not, and an IntraBC
/// block never does (the planned inter leaf carries `use_intrabc` into both
/// `uses_ddt` and the residual reconstruction).
#[test]
fn ddt_follows_the_planned_leaf() {
    let config = config(136, 136, BitDepth::Eight, 6, 110);
    in_first_inter_frame(&config, with_tools(false, true), |syntax, root, _| {
        let leaf = syntax.leaf(top_left(syntax, root, 32))?;
        let prediction = InterPrediction::Single(InterModes {
            ref_frame: 0,
            mode: InterMode::Global,
            drl: 0,
            mv: MotionVector::default(),
            filter: splot_recon::InterpolationFilter::EightTap,
            precision: None,
            amvd: false,
            interintra: None,
            bawp: None,
        });
        syntax.plan_inter_leaf(&leaf, prediction)?;
        assert!(syntax.uses_ddt());
        syntax.plan_leaf(&leaf, IntraModes::DC)?;
        assert!(!syntax.uses_ddt());
        Ok(())
    });
}
