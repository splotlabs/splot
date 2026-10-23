// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-frame CDF carry (§ 5.18.2 `set_primary_ref_frame_and_ctx`, § 7.23).

use super::*;
use crate::gop::{GopPlanner, GopStep};
use crate::preset::PresetParams;
use splot_core::headers::frame::FrameHeaderCore;

/// `PRIMARY_REF_NONE` (§ 3).
const PRIMARY_REF_NONE: u8 = 7;

/// A panning mini-GOP stream coded with `preset`: the units in coding order,
/// the recon in display order, and the header of every frame.
fn planned_stream(
    config: &Config,
    preset: &'static PresetParams,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    let mut coder = IntraFrameCoder::with_preset(config, preset).unwrap();
    let mut planner = GopPlanner::new(config.keyint, preset.mini_gop_size);
    let (mut units, mut recons, mut cores) = (Vec::new(), BTreeMap::new(), Vec::new());
    let mut queued = 9;
    while let Some(len) = planner.next_len(queued, true) {
        queued -= len;
        for step in planner.plan(len).unwrap() {
            match step {
                GopStep::Code(plan) => {
                    let source = panning_frame(config, plan.display_index as u32);
                    let mut coded = coder.code_frame(&source, &plan, 110).unwrap();
                    cores.push(coded.reference.as_ref().unwrap().0.clone());
                    coder.commit(&mut coded).unwrap();
                    recons.insert(plan.display_index, recon_bytes(&coded, config.bit_depth));
                    units.extend(coded.temporal_unit);
                }
                GopStep::ShowExisting { slot, .. } => {
                    units.extend(coder.show_existing_frame(slot).unwrap());
                }
            }
        }
    }
    (
        units,
        recons.into_values().flatten().collect(),
        cores,
        coder,
    )
}

/// With carry, every inter frame codes a primary reference and the sequence
/// averages CDFs, so the decoder loads the saved CDFs the encoder coded with;
/// without carry, frames code `PRIMARY_REF_NONE` and start from the defaults.
#[test]
fn inter_frames_start_from_the_primary_reference_cdfs() {
    let config = config(72, 40, BitDepth::Ten, 7, 110);
    let on = Preset::new(7).unwrap().params();
    let off: &'static PresetParams = Box::leak(Box::new(PresetParams {
        cdf_carry: false,
        ..*on
    }));
    let mut streams = Vec::new();
    for (preset, carry) in [(on, true), (off, false)] {
        let (units, expected, cores, coder) = planned_stream(&config, preset);
        let tq = coder.sequence.transform_quant_entropy.unwrap();
        assert_eq!(tq.enable_avg_cdf, carry);
        let inter: Vec<_> = cores
            .iter()
            .filter_map(|core| core.inter.as_ref())
            .collect();
        assert!(
            inter.len() >= 7,
            "carry {carry}: {} inter frames",
            inter.len()
        );
        for header in inter {
            assert_eq!(header.disable_cross_frame_cdf_init, Some(!carry));
            assert_eq!(
                header.primary_ref_frame != Some(PRIMARY_REF_NONE),
                carry,
                "carry {carry}: primary_ref_frame {:?}",
                header.primary_ref_frame
            );
        }
        assert_same_samples(&decode(&units), &expected, &format!("carry {carry}"));
        streams.push(units);
    }
    assert!(
        streams[0].len() < streams[1].len(),
        "carried CDFs cost {} bytes, default CDFs {}",
        streams[0].len(),
        streams[1].len()
    );
}

/// A mini-GOP stream in 2x2 tiles, coded through the [`Encoder`] on `threads`.
fn tiled_mini_gop(threads: usize) -> (Config, Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(136, 72, BitDepth::Eight, 7, 100);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 1,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let (units, expected) = encode_stream(&config, 9);
    (config, units, expected)
}

#[test]
fn tiled_mini_gops_decode_alike_on_any_thread_count() {
    let (_, one, expected) = tiled_mini_gop(1);
    assert_same_samples(&decode(&one.concat()), &expected, "2x2 tiles");
    for threads in [2, 8] {
        assert!(tiled_mini_gop(threads).1 == one, "{threads} threads");
    }
}

/// AVM decodes a carried 2x2-tile mini-GOP stream to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_tiled_mini_gops_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (config, units, expected) = tiled_mini_gop(4);
    let size = (config.width, config.height);
    let avm = avm_decode(&avmdec, &units, size, config.bit_depth, "carry");
    assert!(avm == expected, "AVM output differs");
}
