// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic)]

use splot_core::headers::frame::{
    FrameHeaderCore, TxMode, build_minimal_intra_clk_core, build_minimal_intra_sequence_header,
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
