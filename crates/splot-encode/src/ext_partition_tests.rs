// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Three-way and uneven four-way partitions in coded streams.

use super::*;

/// Preset 9 with every extended shape on and open margins.
static EXT: PresetParams = PresetParams {
    ext_partitions: 2,
    ext_three_way_margin: u8::MAX,
    ext_four_way_margin: u8::MAX,
    ..PRESETS[9]
};

/// Key and inter frames with extended partitions decode to the encoder recon,
/// and 1, 2 and 8 threads write the same bytes.
#[test]
fn ext_partitions_decode_to_the_encoder_recon_on_any_thread_count() {
    for case in [
        (136, 72, BitDepth::Eight, 9, 120, (1, 0)),
        (97, 70, BitDepth::Ten, 9, 60, (0, 0)),
    ] {
        let (one, expected) = tiled_stream(case, 1, Some(&EXT));
        assert_same_samples(&decode(&one.concat()), &expected, &format!("{case:?}"));
        for threads in [2, 8] {
            assert!(
                tiled_stream(case, threads, Some(&EXT)).0 == one,
                "{threads} threads"
            );
        }
    }
}

/// Luma bands that each extended shape of a 64x64 quadrant fits: 1:2:1 rows
/// with a two-level middle, the same across columns, then 1:2:4:1 rows and
/// columns.
fn banded_frame(config: &Config) -> SourceFrame {
    let sizes = config.plane_sizes().unwrap();
    let band = |at: usize, cross: usize, uneven: bool| -> u16 {
        let eighth = at % 64 / 8;
        if uneven {
            [30, 110, 110, 200, 200, 200, 200, 60][eighth]
        } else if (2..6).contains(&eighth) {
            if cross % 64 < 32 { 180 } else { 120 }
        } else {
            40
        }
    };
    let planes = core::array::from_fn(|plane| {
        let size = sizes[plane];
        (0..size.height())
            .flat_map(|y| (0..size.width()).map(move |x| (x, y)))
            .map(|(x, y)| match (plane, x / 64, y / 64) {
                (1.., ..) => 128,
                (0, 0, 0) => band(y, x, false),
                (0, 1, 0) => band(x, y, false),
                (0, 0, _) => band(y, x, true),
                _ => band(x, y, true),
            })
            .collect()
    });
    SourceFrame::new(config, 0, planes).unwrap()
}

fn kinds(tree: &PartitionTree, out: &mut Vec<Partition>) {
    if let PartitionTree::Split { kind, children } = tree {
        out.push(*kind);
        for child in children {
            kinds(child, out);
        }
    }
}

const EXTENDED: [Partition; 6] = [
    Partition::Horz3,
    Partition::Vert3,
    Partition::Horz4A,
    Partition::Horz4B,
    Partition::Vert4A,
    Partition::Vert4B,
];

/// The partition kinds the search picks for the banded frame under `preset`.
fn banded_kinds(preset: &'static PresetParams) -> Vec<Partition> {
    let config = config(128, 128, BitDepth::Eight, 9, 90);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let key = crate::gop::plan(0, config.keyint);
    let mut found = Vec::new();
    for tree in search_frame(&coder, &key, &banded_frame(&config), config.bit_depth) {
        kinds(&tree, &mut found);
    }
    found
}

/// The search codes three-way and uneven four-way shapes where the source
/// has their bands, and a level-0 preset codes none.
#[test]
fn search_codes_three_way_and_uneven_four_way_partitions() {
    let found = banded_kinds(&EXT);
    let three = [Partition::Horz3, Partition::Vert3];
    assert!(found.iter().any(|kind| three.contains(kind)), "{found:?}");
    assert!(
        found.iter().any(|kind| EXTENDED[2..].contains(kind)),
        "{found:?}"
    );
    assert_eq!(PRESETS[9].ext_partitions, 0);
    let off = banded_kinds(&PRESETS[9]);
    assert!(!off.iter().any(|kind| EXTENDED.contains(kind)), "{off:?}");
}

/// The sequence header enables the shapes of the preset's level only.
#[test]
fn sequence_flags_follow_the_preset_level() {
    let config = config(64, 64, BitDepth::Eight, 9, 90);
    for (level, flags) in [(0, (false, false)), (1, (true, false)), (2, (true, true))] {
        let preset = Box::leak(Box::new(PresetParams {
            ext_partitions: level,
            ..EXT
        }));
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        let partition = coder.sequence.partition.unwrap();
        assert_eq!(
            (
                partition.enable_ext_partitions,
                partition.enable_uneven_4way_partitions
            ),
            flags,
            "level {level}"
        );
    }
}

/// AVM decodes key and inter frames with extended partitions to the encoder
/// reconstructions.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_ext_partitions_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let case = (136, 72, BitDepth::Eight, 9, 120, (1, 0));
    let (units, expected) = tiled_stream(case, 2, Some(&EXT));
    let avm = avm_decode(&avmdec, &units, (136, 72), BitDepth::Eight, "ext");
    assert!(avm == expected, "AVM output differs");
}
