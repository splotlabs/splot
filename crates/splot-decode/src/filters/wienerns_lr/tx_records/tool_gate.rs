// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{
    FrameHeaderCore, FrameHeaderParseStatus, FrameRestorationType, TxMode,
};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_core::span::ByteOffset;
use splot_recon::{PlaneId, WIENER_NS_CHROMA_COEFFS, WIENER_NS_LUMA_COEFFS};

use crate::error::Result;

use super::selectable_decode_error;

enum SelectableToolGate {
    ChromaFormat,
    FrameType,
    TileGrid,
    IntraTail,
    MissingIntraConfig,
    UnsupportedIntraTool,
    MissingPartitionConfig,
    MissingTransformQuantEntropyConfig,
    ScreenContentTools,
    Segmentation,
    QuantMatrix,
    Lossless,
    Gdf,
    Cdef,
    LoopRestoration,
}

impl SelectableToolGate {
    const fn reason(self) -> &'static str {
        match self {
            Self::ChromaFormat => selectable_reason!("chroma_format"),
            Self::FrameType => selectable_reason!("frame_type"),
            Self::TileGrid => selectable_reason!("tile_grid"),
            Self::IntraTail => selectable_reason!("intra_tail"),
            Self::MissingIntraConfig => selectable_reason!("missing_intra_config"),
            Self::UnsupportedIntraTool => selectable_reason!("unsupported_intra_tool"),
            Self::MissingPartitionConfig => selectable_reason!("missing_partition_config"),
            Self::MissingTransformQuantEntropyConfig => {
                selectable_reason!("missing_transform_quant_entropy_config")
            }
            Self::ScreenContentTools => selectable_reason!("screen_content_tools"),
            Self::Segmentation => selectable_reason!("segmentation"),
            Self::QuantMatrix => selectable_reason!("quant_matrix"),
            Self::Lossless => selectable_reason!("lossless"),
            Self::Gdf => selectable_reason!("gdf"),
            Self::Cdef => selectable_reason!("cdef"),
            Self::LoopRestoration => selectable_reason!("loop_restoration"),
        }
    }
}

pub(crate) fn ensure_selectable_transform_record_tool_gates(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<()> {
    if let Some(gate) = selectable_tool_gate_failure(sequence, core) {
        return selectable_tool_gate_error(offset, gate);
    }
    Ok(())
}

fn selectable_tool_gate_failure(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Option<SelectableToolGate> {
    use SelectableToolGate::{
        Cdef, ChromaFormat, FrameType, Gdf, IntraTail, LoopRestoration, Lossless,
        MissingIntraConfig, MissingPartitionConfig, MissingTransformQuantEntropyConfig,
        QuantMatrix, ScreenContentTools, Segmentation, TileGrid, UnsupportedIntraTool,
    };

    if sequence.general.chroma_format_idc != ChromaFormatIdc::Yuv420 {
        return Some(ChromaFormat);
    }
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete
        || core.frame_is_intra != Some(true)
        || !core.is_key_frame
    {
        return Some(FrameType);
    }
    if core
        .tile_info
        .as_ref()
        .is_none_or(|tile_info| tile_info.tile_cols != 1 || tile_info.tile_rows != 1)
    {
        return Some(TileGrid);
    }
    if core
        .intra_tail
        .is_none_or(|tail| tail.tx_mode != TxMode::Select || tail.film_grain.apply_grain)
    {
        return Some(IntraTail);
    }
    let Some(intra) = sequence.intra.as_ref() else {
        return Some(MissingIntraConfig);
    };
    if intra.enable_dip {
        return Some(UnsupportedIntraTool);
    }
    if sequence.partition.is_none() {
        return Some(MissingPartitionConfig);
    }
    if sequence.transform_quant_entropy.is_none() {
        return Some(MissingTransformQuantEntropyConfig);
    }
    if core.allow_screen_content_tools != Some(false) || core.allow_intrabc.is_none() {
        return Some(ScreenContentTools);
    }
    if core
        .segmentation_params
        .as_ref()
        .is_none_or(|seg| seg.segmentation_enabled)
    {
        return Some(Segmentation);
    }
    if core.setup_qm_params.is_none_or(|qm| qm.using_qmatrix) {
        return Some(QuantMatrix);
    }
    if core
        .lossless_info
        .as_ref()
        .is_none_or(|lossless| lossless.coded_lossless)
    {
        return Some(Lossless);
    }
    if core.gdf_params.is_none_or(|gdf| gdf.gdf_frame_enable) {
        return Some(Gdf);
    }
    if core.cdef_params.as_ref().is_none_or(|cdef| {
        cdef.cdef_frame_enable
            && (cdef.cdef_strengths.is_none() || cdef.cdef_on_skip_txfm_frame_enable != Some(true))
    }) {
        return Some(Cdef);
    }
    if selectable_lr_tool_gate_failure(core) {
        return Some(LoopRestoration);
    }
    None
}

fn selectable_lr_tool_gate_failure(core: &FrameHeaderCore) -> bool {
    let Some(lr) = core.lr_params.as_ref() else {
        return false;
    };
    lr.planes.iter().enumerate().any(|(plane, params)| {
        if params.restoration_type != FrameRestorationType::WienerNonsep {
            return false;
        }
        let is_luma = plane == PlaneId::Y.index();
        (is_luma && !params.frame_filters_on)
            || (params.frame_filters_on && !frame_wiener_ns_bank_is_supported(params, is_luma))
    })
}

fn frame_wiener_ns_bank_is_supported(
    params: &splot_core::headers::frame::LrPlaneParams,
    is_luma: bool,
) -> bool {
    let expected_classes = if is_luma {
        usize::from(params.num_filter_classes.unwrap_or(1))
    } else {
        1
    };
    let expected_coeffs = if is_luma {
        WIENER_NS_LUMA_COEFFS
    } else {
        WIENER_NS_CHROMA_COEFFS
    };
    params.frame_filter_bank.as_ref().is_some_and(|bank| {
        bank.classes.len() == expected_classes
            && bank
                .classes
                .iter()
                .all(|class| class.coeffs.len() == expected_coeffs)
    })
}

fn selectable_tool_gate_error(offset: ByteOffset, gate: SelectableToolGate) -> Result<()> {
    Err(selectable_decode_error(offset, gate.reason()))
}
