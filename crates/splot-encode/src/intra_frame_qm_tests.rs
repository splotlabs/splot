// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Quantizer matrices (§ 5.18.6.2 `setup_qm_params`, § 7.14.4 `useQm`).

use super::*;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::types::ObuType;
use splot_recon::QmFrameLevels;

/// Preset 6 with `quant_matrices`.
fn with_qm(quant_matrices: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        quant_matrices,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// A mini-GOP stream of `chroma` at `bit_depth` and `qp` with
/// `quant_matrices` on `threads` threads: its units, the encoder recon and
/// the frame headers.
fn qm_stream(
    (chroma, bit_depth, qp): (PixelFormat, BitDepth, u16),
    quant_matrices: u8,
    threads: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>) {
    let mut config = config(136, 72, bit_depth, 6, qp);
    config.chroma = chroma;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, with_qm(quant_matrices)).unwrap();
    let (units, recon, cores, _) =
        super::carry::planned_stream_of(&config, coder, panning_frame, qp);
    (units, recon, cores)
}

const CASES: [((PixelFormat, BitDepth, u16), u8, u8); 3] = [
    ((PixelFormat::Yuv420, BitDepth::Eight, 160), 2, 8),
    ((PixelFormat::Yuv444, BitDepth::Ten, 240), 3, 11),
    ((PixelFormat::Yuv422, BitDepth::Eight, 200), 1, 13),
];

/// Each frame codes `using_qmatrix` with the level that rises with
/// `base_q_idx`, segment 0 takes it as `SegQMLevel` (§ 5.18.2), every loop
/// filter codes in some frame, and the stream differs from the one without
/// matrices, decodes to the encoder recon, and has the same bytes on 1, 2
/// and 8 threads.
#[test]
fn qm_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    each_case(CASES, |(case, quant_matrices, level)| {
        let name = format!("{case:?} quant_matrices {quant_matrices}");
        let (units, recon, cores) = qm_stream(case, quant_matrices, 1);
        let mut filters = [false; 5];
        for core in cores
            .iter()
            .filter(|core| core.obu_type != ObuType::RegularTip)
        {
            let qm = core.setup_qm_params.unwrap();
            assert!(qm.using_qmatrix, "{name}");
            let levels = qm.levels[0];
            assert_eq!(
                [levels.qm_y, levels.qm_u, levels.qm_v],
                [level; 3],
                "{name}"
            );
            let seg = core.lossless_info.as_ref().unwrap().seg_qm_levels[0];
            assert_eq!(seg, [level; 3], "{name}");
            let used = [
                core.deblocking_filter_params
                    .is_some_and(|df| df.apply_deblocking_filter.contains(&true)),
                core.cdef_params.is_some_and(|cdef| cdef.cdef_frame_enable),
                core.lr_params.as_ref().is_some_and(|lr| lr.uses_lr),
                core.ccso_params
                    .as_ref()
                    .is_some_and(|ccso| ccso.ccso_frame_flag == Some(true)),
                core.gdf_params.is_some_and(|gdf| gdf.gdf_frame_enable),
            ];
            filters = core::array::from_fn(|i| filters[i] || used[i]);
        }
        assert!(cores.len() > 2, "{name}");
        assert_eq!(filters, [true; 5], "{name}: deblock, CDEF, LR, CCSO, GDF");
        assert_same_samples(&decode(&units), &recon, &name);
        assert!(qm_stream(case, 0, 1).0 != units, "{name}");
        for threads in [2, 8] {
            assert!(
                qm_stream(case, quant_matrices, threads).0 == units,
                "{name}: {threads} threads"
            );
        }
    });
}

/// The facade resolves `segLvl` as the decoder does (§ 7.14.4): blocks of at
/// most 8x8 take `SegQMLevel` of segment 0, larger blocks `qm_*[0]`, and
/// types from IDTX on and level 15 take no matrix.
#[test]
fn facade_resolves_the_matrix_level_of_each_block() {
    let qm = QmFrameLevels {
        levels_gt8: [2, 3, 4],
        levels_le8: [[9, 10, 11]; 16],
    };
    let level = |plane, log2: (u32, u32), tx_type, qm| {
        let (params, _) = recon_params(
            90,
            plane,
            log2,
            (tx_type, false, false),
            (BitDepth::Eight, Some(qm)),
        )
        .unwrap();
        params.qm.map(|qm| qm.seg_level)
    };
    assert_eq!(level(PlaneId::Y, (3, 3), TxType::DctDct, qm), Some(9));
    assert_eq!(level(PlaneId::U, (2, 3), TxType::DctDct, qm), Some(10));
    assert_eq!(level(PlaneId::V, (3, 2), TxType::DctDct, qm), Some(11));
    assert_eq!(level(PlaneId::Y, (2, 4), TxType::DctDct, qm), Some(2));
    assert_eq!(level(PlaneId::U, (4, 4), TxType::DctDct, qm), Some(3));
    assert_eq!(level(PlaneId::V, (5, 5), TxType::DctDct, qm), Some(4));
    assert_eq!(level(PlaneId::Y, (3, 3), TxType::Idtx, qm), None);
    let flat = QmFrameLevels {
        levels_gt8: [15; 3],
        levels_le8: [[15; 3]; 16],
    };
    assert_eq!(level(PlaneId::Y, (3, 3), TxType::DctDct, flat), None);
    assert_eq!(level(PlaneId::Y, (4, 4), TxType::DctDct, flat), None);
}

/// The level map rises with `base_q_idx` over each band and stays in it.
#[test]
fn qm_level_rises_with_qindex_inside_its_band() {
    for (value, band) in [(1, 10..=14), (2, 5..=9), (3, 0..=14)] {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let max = u32::from(crate::rate_control::max_qindex(bit_depth));
            let levels: Vec<u8> = (1..=max)
                .map(|q| crate::tx_choice::qm_level(with_qm(value), q, bit_depth).unwrap())
                .collect();
            assert!(levels.windows(2).all(|w| w[0] <= w[1]));
            assert_eq!(levels.first(), Some(band.start()));
            assert_eq!(levels.last(), Some(band.end()));
        }
    }
    assert_eq!(
        crate::tx_choice::qm_level(with_qm(0), 100, BitDepth::Eight),
        None
    );
}

/// AVM decodes quantizer-matrix streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_qm_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (case, quant_matrices, _) in CASES {
        let (units, recon, _) = qm_stream(case, quant_matrices, 1);
        let avm = avm_decode(&avmdec, &[units], (136, 72), case.1, "qm");
        assert!(avm == recon, "{case:?}: AVM output differs");
    }
}
