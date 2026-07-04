// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic)]

use splot_core::headers::frame::{
    FrameHeaderCore, FrameRestorationType, LrPlaneParams, TxMode, build_minimal_intra_clk_core,
    build_minimal_intra_sequence_header,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;

use crate::error::DecodeError;

use super::ensure_selectable_transform_record_tool_gates;

fn selectable_tool_gate_fixture() -> (SequenceHeader, FrameHeaderCore) {
    let mut sequence = build_minimal_intra_sequence_header().unwrap();
    let (mut core, _) = build_minimal_intra_clk_core().unwrap();
    if let Some(intra) = sequence.intra.as_mut() {
        intra.enable_dip = false;
        intra.enable_ibp = false;
        intra.enable_mrls = false;
        intra.enable_intra_edge_filter = false;
    }
    if let Some(tq) = sequence.transform_quant_entropy.as_mut() {
        tq.enable_fsc = false;
        tq.enable_cctx = false;
        tq.enable_idtx_intra = false;
        tq.enable_intra_ist = false;
        tq.enable_chroma_dctonly = false;
    }
    core.intra_tail.as_mut().unwrap().tx_mode = TxMode::Select;
    (sequence, core)
}

fn unsupported_reason(error: DecodeError) -> &'static str {
    match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
        other => panic!("unexpected decode error: {other:?}"),
    }
}

fn assert_unsupported_reason(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    expected: &'static str,
) {
    let error = ensure_selectable_transform_record_tool_gates(sequence, core, ByteOffset::new(74))
        .unwrap_err();

    assert_eq!(unsupported_reason(error), expected);
}

fn replace_lr_params(core: &mut FrameHeaderCore, planes: Vec<LrPlaneParams>, size: [u32; 3]) {
    let lr = core.lr_params.as_mut().unwrap();
    lr.uses_lr = true;
    lr.planes = planes;
    lr.loop_restoration_size = size;
}

#[test]
fn selectable_tool_gate_admits_minimal_inert_selectable_intra_header() {
    let (sequence, core) = selectable_tool_gate_fixture();

    ensure_selectable_transform_record_tool_gates(&sequence, &core, ByteOffset::new(0)).unwrap();
}

#[test]
fn selectable_tool_gate_rejects_unsupported_intra_tools_before_tile_decode() {
    let (mut sequence, core) = selectable_tool_gate_fixture();
    sequence.intra.as_mut().unwrap().enable_dip = true;

    let error =
        ensure_selectable_transform_record_tool_gates(&sequence, &core, ByteOffset::new(74))
            .unwrap_err();

    assert_eq!(
        unsupported_reason(error),
        "unsupported_wienerns_lr_selectable_transform_records_unsupported_intra_tool"
    );
}

#[test]
fn selectable_tool_gate_admits_enabled_inactive_transform_and_mrl_tools() {
    let (mut sequence, mut core) = selectable_tool_gate_fixture();
    sequence.intra.as_mut().unwrap().enable_mrls = true;
    sequence.intra.as_mut().unwrap().enable_intra_edge_filter = true;
    sequence.intra.as_mut().unwrap().enable_ibp = true;
    let tq = sequence.transform_quant_entropy.as_mut().unwrap();
    tq.enable_fsc = true;
    tq.enable_cctx = true;
    tq.enable_idtx_intra = true;
    tq.enable_intra_ist = true;
    tq.enable_chroma_dctonly = true;
    core.ccso_params.as_mut().unwrap().ccso_frame_flag = Some(true);

    ensure_selectable_transform_record_tool_gates(&sequence, &core, ByteOffset::new(0)).unwrap();
}

#[test]
fn selectable_tool_gate_rejects_cdef_skip_txfm_disabled_before_tile_decode() {
    let (sequence, mut core) = selectable_tool_gate_fixture();
    let cdef = core.cdef_params.as_mut().unwrap();
    cdef.cdef_frame_enable = true;
    cdef.cdef_damping = Some(4);
    cdef.cdef_strengths = Some(1);
    cdef.cdef_on_skip_txfm_frame_enable = Some(false);

    assert_unsupported_reason(
        &sequence,
        &core,
        "unsupported_wienerns_lr_selectable_transform_records_cdef",
    );
}

#[test]
fn selectable_tool_gate_handles_wiener_ns_lr_shapes_before_tile_decode() {
    struct Case {
        planes: [(FrameRestorationType, bool); 3],
        size: [u32; 3],
        unsupported_reason: Option<&'static str>,
    }

    let cases = [
        Case {
            planes: [
                (FrameRestorationType::WienerNonsep, false),
                (FrameRestorationType::None, false),
                (FrameRestorationType::None, false),
            ],
            size: [128, 128, 128],
            unsupported_reason: Some(
                "unsupported_wienerns_lr_selectable_transform_records_loop_restoration",
            ),
        },
        Case {
            planes: [
                (FrameRestorationType::None, false),
                (FrameRestorationType::WienerNonsep, false),
                (FrameRestorationType::WienerNonsep, false),
            ],
            size: [128, 256, 256],
            unsupported_reason: None,
        },
        Case {
            planes: [
                (FrameRestorationType::None, false),
                (FrameRestorationType::WienerNonsep, true),
                (FrameRestorationType::None, false),
            ],
            size: [64, 128, 128],
            unsupported_reason: Some(
                "unsupported_wienerns_lr_selectable_transform_records_loop_restoration",
            ),
        },
    ];

    for case in cases {
        let (sequence, mut core) = selectable_tool_gate_fixture();
        let planes = case
            .planes
            .into_iter()
            .map(|(restoration_type, frame_filters_on)| {
                lr_plane(restoration_type, frame_filters_on)
            })
            .collect();
        replace_lr_params(&mut core, planes, case.size);

        if let Some(reason) = case.unsupported_reason {
            assert_unsupported_reason(&sequence, &core, reason);
        } else {
            ensure_selectable_transform_record_tool_gates(&sequence, &core, ByteOffset::new(0))
                .unwrap();
        }
    }
}

const fn lr_plane(restoration_type: FrameRestorationType, frame_filters_on: bool) -> LrPlaneParams {
    LrPlaneParams {
        restoration_type,
        frame_filters_on,
        num_filter_classes: None,
        frame_filter_bank: None,
    }
}
