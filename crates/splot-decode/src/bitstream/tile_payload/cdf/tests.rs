// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use splot_core::span::ByteOffset;
use splot_core::symbol::{SymbolDecoder, SymbolDecoderConfig};
use splot_core::tables::cdf::{
    DEFAULT_CCTX_TYPE_CDF, DEFAULT_CFL_ALPHA_CDF, DEFAULT_CFL_INDEX_CDF, DEFAULT_CFL_MH_DIR_CDF,
    DEFAULT_CFL_MHCCP_CDF, DEFAULT_CFL_SIGN_CDF, DEFAULT_COEFF_BASE_BOB_CDF,
    DEFAULT_COEFF_BASE_CDF, DEFAULT_COEFF_BASE_EOB_CDF, DEFAULT_COEFF_BASE_EOB_UV_CDF,
    DEFAULT_COEFF_BASE_IDTX_CDF, DEFAULT_COEFF_BASE_LF_CDF, DEFAULT_COEFF_BASE_LF_EOB_CDF,
    DEFAULT_COEFF_BASE_LF_EOB_UV_CDF, DEFAULT_COEFF_BASE_LF_UV_CDF, DEFAULT_COEFF_BASE_PH_CDF,
    DEFAULT_COEFF_BASE_UV_CDF, DEFAULT_COEFF_BR_CDF, DEFAULT_COEFF_BR_IDTX_CDF,
    DEFAULT_COEFF_BR_LF_CDF, DEFAULT_COEFF_BR_UV_CDF, DEFAULT_COMP_GROUP_IDX_CDF,
    DEFAULT_COMP_MODE_CDF, DEFAULT_COMP_REF0_CDF, DEFAULT_COMP_REF1_CDF,
    DEFAULT_COMPOUND_MODE_NON_JOINT_CDF, DEFAULT_COMPOUND_TYPE_CDF, DEFAULT_CWP_IDX_CDF,
    DEFAULT_DC_SIGN_CDF, DEFAULT_DELTA_Q_CDF, DEFAULT_DIP_MODE_CDF, DEFAULT_EOB_EXTRA_CDF,
    DEFAULT_EOB_PT_16_CDF, DEFAULT_EOB_PT_32_CDF, DEFAULT_EOB_PT_64_CDF, DEFAULT_EOB_PT_128_CDF,
    DEFAULT_EOB_PT_256_CDF, DEFAULT_EOB_PT_512_CDF, DEFAULT_EOB_PT_1024_CDF, DEFAULT_FSC_MODE_CDF,
    DEFAULT_IDTX_SIGN_CDF, DEFAULT_INTRA_TX_TYPE_LONG_CDF, DEFAULT_INTRA_TX_TYPE_SET1_CDF,
    DEFAULT_INTRA_TX_TYPE_SET2_CDF, DEFAULT_IS_CFL_CDF, DEFAULT_IS_JOINT_CDF,
    DEFAULT_IS_LONG_SIDE_DCT_CDF, DEFAULT_JMVD_ADAPTIVE_SCALE_MODE_CDF,
    DEFAULT_JMVD_SCALE_MODE_CDF, DEFAULT_LOSSLESS_INTER_TX_TYPE_CDF, DEFAULT_MORPH_PRED_CDF,
    DEFAULT_MOST_PROBABLE_STX_SET_ADST_CDF, DEFAULT_MOST_PROBABLE_STX_SET_CDF,
    DEFAULT_MRL_INDEX_CDF, DEFAULT_MRL_SEC_INDEX_CDF, DEFAULT_PALETTE_Y_MODE_CDF,
    DEFAULT_SEC_TX_TYPE_CDF, DEFAULT_SKIP_MODE_CDF, DEFAULT_TIP_MODE_CDF,
    DEFAULT_TX_2OR3_PARTITION_TYPE_CDF, DEFAULT_TX_DO_PARTITION_CDF, DEFAULT_TX_PARTITION_TYPE_CDF,
    DEFAULT_TX_PARTITION_TYPE_REDUCED_CDF, DEFAULT_TXB_SKIP_CDF, DEFAULT_USE_DIP_CDF,
    DEFAULT_USE_OPTFLOW_CDF, DEFAULT_USE_WIENER_NS_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
    DEFAULT_V_TXB_SKIP_CDF, DEFAULT_WIENER_NS_BASE_CDF, DEFAULT_WIENER_NS_LENGTH_CDF,
    DEFAULT_WIENER_NS_UV_SYM_CDF, DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
};

use super::block_rows::*;

impl FrameCdfSubset {
    pub(crate) const fn rows(&self) -> &TileCdfRows {
        &self.rows
    }
}

impl TileCdfSubset {
    pub(crate) const fn rows(&self) -> &TileCdfRows {
        &self.rows
    }

    pub(crate) fn rows_mut(&mut self) -> &mut TileCdfRows {
        &mut self.rows
    }
}

impl TileCdfWorkUnitBoundary {
    pub(crate) const fn saved_cdfs(&self) -> &SavedCdfSubset {
        &self.saved_cdfs
    }
}

impl TileCdfRows {
    pub(crate) const fn do_square_split(&self) -> &DoSquareSplitCdfRows {
        &self.do_square_split
    }

    pub(crate) const fn rect_type(&self) -> &RectTypeCdfRows {
        &self.rect_type
    }

    pub(crate) const fn do_uneven_4way_partition(&self) -> &DoUneven4WayPartitionCdfRows {
        &self.do_uneven_4way_partition
    }

    pub(crate) const fn tx_do_partition(&self) -> &TxDoPartitionCdfRows {
        &self.tx_do_partition
    }

    pub(crate) const fn tx_2or3_partition_type(&self) -> &Tx2Or3PartitionTypeCdfRows {
        &self.tx_2or3_partition_type
    }

    pub(crate) const fn tx_partition_type(&self) -> &TxPartitionTypeCdfRows {
        &self.tx_partition_type
    }

    pub(crate) const fn tx_partition_type_reduced(&self) -> &TxPartitionTypeCdfRows {
        &self.tx_partition_type_reduced
    }

    pub(crate) const fn lossless_inter_tx_type(&self) -> &LosslessInterTxTypeCdfRow {
        &self.lossless_inter_tx_type
    }

    pub(crate) const fn delta_q(&self) -> &DeltaQCdfRow {
        &self.delta_q
    }

    pub(crate) const fn intrabc_mode(&self) -> &IntrabcModeCdfRow {
        &self.intrabc_mode
    }

    pub(crate) const fn intrabc_precision(&self) -> &IntrabcPrecisionCdfRow {
        &self.intrabc_precision
    }

    pub(crate) const fn morph_pred(&self) -> &MorphPredCdfRows {
        &self.morph_pred
    }

    pub(crate) const fn fsc_mode(&self) -> &FscModeCdfRows {
        &self.fsc_mode
    }

    pub(crate) const fn mrl_index(&self) -> &MrlIndexCdfRows {
        &self.mrl_index
    }

    pub(crate) const fn mrl_sec_index(&self) -> &MrlSecIndexCdfRows {
        &self.mrl_sec_index
    }

    #[allow(dead_code)]
    pub(crate) const fn region_type(&self) -> &RegionTypeCdfRows {
        &self.region_type
    }

    pub(crate) const fn y_mode_set(&self) -> &block_rows::YModeSetCdfRow {
        self.block.y_mode_set()
    }

    pub(crate) const fn y_mode_index(&self) -> &block_rows::YModeIndexCdfRows {
        self.block.y_mode_index()
    }

    pub(crate) const fn txb_skip(&self) -> &block_rows::TxbSkipCdfRows {
        self.block.txb_skip()
    }

    pub(crate) const fn is_long_side_dct(&self) -> &block_rows::IsLongSideDctCdfRows {
        self.block.is_long_side_dct()
    }

    pub(crate) const fn intra_tx_type_long(&self) -> &block_rows::IntraTxTypeLongCdfRows {
        self.block.intra_tx_type_long()
    }

    pub(crate) const fn intra_tx_type_set1(&self) -> &block_rows::IntraTxTypeSet1CdfRows {
        self.block.intra_tx_type_set1()
    }

    pub(crate) const fn intra_tx_type_set2(&self) -> &block_rows::IntraTxTypeSet2CdfRows {
        self.block.intra_tx_type_set2()
    }

    pub(crate) const fn sec_tx_type(&self) -> &block_rows::SecTxTypeCdfRows {
        self.block.sec_tx_type()
    }

    pub(crate) const fn most_probable_stx_set(&self) -> &block_rows::MostProbableStxSetCdfRow {
        self.block.most_probable_stx_set()
    }

    pub(crate) const fn most_probable_stx_set_adst(
        &self,
    ) -> &block_rows::MostProbableStxSetAdstCdfRow {
        self.block.most_probable_stx_set_adst()
    }

    pub(crate) const fn cctx_type(&self) -> &block_rows::CctxTypeCdfRow {
        self.block.cctx_type()
    }

    pub(crate) const fn palette_y_mode(&self) -> &block_rows::PaletteYModeCdfRow {
        self.block.palette_y_mode()
    }

    pub(crate) const fn uv_mode_cfl_not_allowed(&self) -> &block_rows::UvModeCflNotAllowedCdfRows {
        self.block.uv_mode_cfl_not_allowed()
    }

    pub(crate) const fn is_cfl(&self) -> &block_rows::IsCflCdfRows {
        self.block.is_cfl()
    }

    pub(crate) const fn cfl_index(&self) -> &block_rows::CflIndexCdfRow {
        self.block.cfl_index()
    }

    pub(crate) const fn cfl_sign(&self) -> &block_rows::CflSignCdfRow {
        self.block.cfl_sign()
    }

    pub(crate) const fn cfl_alpha(&self) -> &block_rows::CflAlphaCdfRows {
        self.block.cfl_alpha()
    }

    pub(crate) const fn cfl_mhccp(&self) -> &block_rows::CflMhccpCdfRow {
        self.block.cfl_mhccp()
    }

    pub(crate) const fn cfl_mh_dir(&self) -> &block_rows::CflMhDirCdfRows {
        self.block.cfl_mh_dir()
    }

    pub(crate) const fn use_dip(&self) -> &block_rows::UseDipCdfRows {
        self.block.use_dip()
    }

    pub(crate) const fn dip_mode(&self) -> &block_rows::DipModeCdfRow {
        self.block.dip_mode()
    }

    pub(crate) const fn v_txb_skip(&self) -> &block_rows::VTxbSkipCdfRows {
        self.block.v_txb_skip()
    }

    pub(crate) const fn eob_extra(&self) -> &block_rows::EobExtraCdfRows {
        self.block.eob_extra()
    }

    pub(crate) const fn comp_mode(&self) -> &block_rows::CompModeCdfRows {
        self.block.comp_mode()
    }

    pub(crate) const fn is_joint(&self) -> &block_rows::IsJointCdfRows {
        self.block.is_joint()
    }

    pub(crate) const fn compound_mode_non_joint(&self) -> &block_rows::CompoundModeNonJointCdfRows {
        self.block.compound_mode_non_joint()
    }

    pub(crate) const fn compound_type(&self) -> &block_rows::CompoundTypeCdfRow {
        self.block.compound_type()
    }

    pub(crate) const fn comp_group_idx(&self) -> &block_rows::CompGroupIdxCdfRows {
        self.block.comp_group_idx()
    }

    pub(crate) const fn cwp_idx(&self) -> &block_rows::CwpIdxCdfRows {
        self.block.cwp_idx()
    }

    pub(crate) const fn comp_ref0(&self) -> &block_rows::CompRef0CdfRows {
        self.block.comp_ref0()
    }

    pub(crate) const fn comp_ref1(&self) -> &block_rows::CompRef1CdfRows {
        self.block.comp_ref1()
    }

    pub(crate) const fn tip_mode(&self) -> &block_rows::TipModeCdfRows {
        self.block.tip_mode()
    }

    pub(crate) const fn use_wiener_ns(&self) -> &block_rows::UseWienerNsCdfRow {
        self.block.use_wiener_ns()
    }

    pub(crate) const fn wiener_ns_length(&self) -> &block_rows::WienerNsLengthCdfRows {
        self.block.wiener_ns_length()
    }

    pub(crate) const fn wiener_ns_uv_sym(&self) -> &block_rows::WienerNsUvSymCdfRow {
        self.block.wiener_ns_uv_sym()
    }

    pub(crate) const fn wiener_ns_base(&self) -> &block_rows::WienerNsBaseCdfRow {
        self.block.wiener_ns_base()
    }
}

impl BlockCdfRows {
    pub(crate) const fn y_mode_set(&self) -> &YModeSetCdfRow {
        &self.y_mode_set
    }

    pub(crate) const fn y_mode_index(&self) -> &YModeIndexCdfRows {
        &self.y_mode_index
    }

    pub(crate) const fn txb_skip(&self) -> &TxbSkipCdfRows {
        &self.txb_skip
    }

    pub(crate) const fn uv_mode_cfl_not_allowed(&self) -> &UvModeCflNotAllowedCdfRows {
        &self.uv_mode_cfl_not_allowed
    }

    pub(crate) const fn is_cfl(&self) -> &IsCflCdfRows {
        &self.is_cfl
    }

    pub(crate) const fn cfl_index(&self) -> &CflIndexCdfRow {
        &self.cfl_index
    }

    pub(crate) const fn cfl_sign(&self) -> &CflSignCdfRow {
        &self.cfl_sign
    }

    pub(crate) const fn cfl_alpha(&self) -> &CflAlphaCdfRows {
        &self.cfl_alpha
    }

    pub(crate) const fn cfl_mhccp(&self) -> &CflMhccpCdfRow {
        &self.cfl_mhccp
    }

    pub(crate) const fn cfl_mh_dir(&self) -> &CflMhDirCdfRows {
        &self.cfl_mh_dir
    }

    pub(crate) const fn use_dip(&self) -> &UseDipCdfRows {
        &self.use_dip
    }

    pub(crate) const fn dip_mode(&self) -> &DipModeCdfRow {
        &self.dip_mode
    }

    pub(crate) const fn v_txb_skip(&self) -> &VTxbSkipCdfRows {
        &self.v_txb_skip
    }

    pub(crate) const fn eob_extra(&self) -> &EobExtraCdfRows {
        &self.eob_extra
    }

    pub(crate) const fn comp_mode(&self) -> &CompModeCdfRows {
        &self.comp_mode
    }

    pub(crate) const fn is_joint(&self) -> &IsJointCdfRows {
        &self.is_joint
    }

    pub(crate) const fn compound_mode_non_joint(&self) -> &CompoundModeNonJointCdfRows {
        &self.compound_mode_non_joint
    }

    pub(crate) const fn compound_type(&self) -> &CompoundTypeCdfRow {
        &self.compound_type
    }

    pub(crate) const fn comp_group_idx(&self) -> &CompGroupIdxCdfRows {
        &self.comp_group_idx
    }

    pub(crate) const fn cwp_idx(&self) -> &CwpIdxCdfRows {
        &self.cwp_idx
    }

    pub(crate) const fn comp_ref0(&self) -> &CompRef0CdfRows {
        &self.comp_ref0
    }

    pub(crate) const fn comp_ref1(&self) -> &CompRef1CdfRows {
        &self.comp_ref1
    }

    pub(crate) const fn tip_mode(&self) -> &TipModeCdfRows {
        &self.tip_mode
    }

    pub(crate) const fn use_wiener_ns(&self) -> &UseWienerNsCdfRow {
        &self.use_wiener_ns
    }

    pub(crate) const fn wiener_ns_length(&self) -> &WienerNsLengthCdfRows {
        &self.wiener_ns_length
    }

    pub(crate) const fn wiener_ns_uv_sym(&self) -> &WienerNsUvSymCdfRow {
        &self.wiener_ns_uv_sym
    }

    pub(crate) const fn wiener_ns_base(&self) -> &WienerNsBaseCdfRow {
        &self.wiener_ns_base
    }

    pub(crate) const fn is_long_side_dct(&self) -> &IsLongSideDctCdfRows {
        &self.is_long_side_dct
    }

    pub(crate) const fn intra_tx_type_long(&self) -> &IntraTxTypeLongCdfRows {
        &self.intra_tx_type_long
    }

    pub(crate) const fn intra_tx_type_set1(&self) -> &IntraTxTypeSet1CdfRows {
        &self.intra_tx_type_set1
    }

    pub(crate) const fn intra_tx_type_set2(&self) -> &IntraTxTypeSet2CdfRows {
        &self.intra_tx_type_set2
    }

    pub(crate) const fn sec_tx_type(&self) -> &SecTxTypeCdfRows {
        &self.sec_tx_type
    }

    pub(crate) const fn most_probable_stx_set(&self) -> &MostProbableStxSetCdfRow {
        &self.most_probable_stx_set
    }

    pub(crate) const fn most_probable_stx_set_adst(&self) -> &MostProbableStxSetAdstCdfRow {
        &self.most_probable_stx_set_adst
    }

    pub(crate) const fn cctx_type(&self) -> &CctxTypeCdfRow {
        &self.cctx_type
    }

    pub(crate) const fn palette_y_mode(&self) -> &PaletteYModeCdfRow {
        &self.palette_y_mode
    }
}

impl SavedCdfSubset {
    pub(crate) const fn rows(&self) -> &TileCdfRows {
        &self.rows
    }
}

impl TileCdfWorkUnitBoundary {
    pub(crate) const fn frame_cdfs(&self) -> &FrameCdfSubset {
        &self.frame_cdfs
    }
}

impl TileCdfRows {
    pub(crate) const fn do_split(&self) -> &DoSplitCdfRows {
        &self.do_split
    }

    pub(crate) const fn do_ext_partition(&self) -> &DoExtPartitionCdfRows {
        &self.do_ext_partition
    }
}

fn coeff(selector: CoeffCdfSelector) -> TileCdfSelector {
    TileCdfSelector::Coeff(selector)
}

fn assert_selector_out_of_range(
    tile: &TileCdfSubset,
    selector: TileCdfSelector,
    array: TileCdfArray,
    index_name: &'static str,
    actual: usize,
    max_exclusive: usize,
) {
    assert_eq!(
        tile.row(selector).unwrap_err(),
        TileCdfError::SelectorOutOfRange {
            array,
            index_name,
            actual,
            max_exclusive,
        },
        "{selector:?}"
    );
}

fn expected_blend_prob(current: i32, saved: i32) -> i32 {
    CDF_PROB_SCALE - (((CDF_PROB_SCALE - saved) + 7 * (CDF_PROB_SCALE - current) + 4) >> 3)
}

fn expected_blend_count(current: i32, saved: i32) -> i32 {
    (saved + 7 * current + 4) >> 3
}

#[test]
fn frame_cdf_subset_blends_saved_rows() {
    let mut frame = FrameCdfSubset::from_defaults();
    let mut saved = FrameCdfSubset::from_defaults();
    frame.rows.delta_q = [
        9000, 12_000, 15_000, 18_000, 21_000, 24_000, 27_000, 30_000, 40,
    ];
    saved.rows.delta_q = [
        11_000, 13_000, 17_000, 19_000, 22_000, 26_000, 28_000, 31_000, 80,
    ];
    frame.rows.block.tip_mode[1] = [10_000, 28_000, 20];
    saved.rows.block.tip_mode[1] = [14_000, 30_000, 100];

    let original_delta_q = frame.rows.delta_q;
    let saved_delta_q = saved.rows.delta_q;
    let original_tip = frame.rows.block.tip_mode[1];
    let saved_tip = saved.rows.block.tip_mode[1];
    frame.blend_from_saved(&saved);

    for i in 0..DELTA_Q_CDF_ROW_LEN - 2 {
        assert_eq!(
            frame.rows.delta_q[i],
            expected_blend_prob(original_delta_q[i], saved_delta_q[i]),
            "delta_q probability index {i}"
        );
    }
    assert_eq!(
        frame.rows.delta_q[DELTA_Q_CDF_ROW_LEN - 2],
        original_delta_q[DELTA_Q_CDF_ROW_LEN - 2]
    );
    assert_eq!(
        frame.rows.delta_q[DELTA_Q_CDF_ROW_LEN - 1],
        expected_blend_count(
            original_delta_q[DELTA_Q_CDF_ROW_LEN - 1],
            saved_delta_q[DELTA_Q_CDF_ROW_LEN - 1]
        )
    );
    assert_eq!(
        frame.rows.block.tip_mode[1][0],
        expected_blend_prob(original_tip[0], saved_tip[0])
    );
    assert_eq!(frame.rows.block.tip_mode[1][1], original_tip[1]);
    assert_eq!(
        frame.rows.block.tip_mode[1][2],
        expected_blend_count(original_tip[2], saved_tip[2])
    );
}

#[test]
fn frame_cdf_subset_copies_generated_defaults_without_aliasing() {
    let frame = FrameCdfSubset::from_defaults();
    assert_eq!(frame.rows().do_split(), &DEFAULT_DO_SPLIT_CDF);
    assert_eq!(
        frame.rows().do_ext_partition(),
        &DEFAULT_DO_EXT_PARTITION_CDF
    );
    assert_eq!(frame.rows().do_square_split(), &DEFAULT_DO_SQUARE_SPLIT_CDF);
    assert_eq!(frame.rows().rect_type(), &DEFAULT_RECT_TYPE_CDF);
    assert_eq!(
        frame.rows().do_uneven_4way_partition(),
        &DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF
    );
    assert_eq!(frame.rows().tx_do_partition(), &DEFAULT_TX_DO_PARTITION_CDF);
    assert_eq!(
        frame.rows().tx_2or3_partition_type(),
        &DEFAULT_TX_2OR3_PARTITION_TYPE_CDF
    );
    assert_eq!(
        frame.rows().tx_partition_type(),
        &DEFAULT_TX_PARTITION_TYPE_CDF
    );
    assert_eq!(
        frame.rows().tx_partition_type_reduced(),
        &DEFAULT_TX_PARTITION_TYPE_REDUCED_CDF
    );
    assert_eq!(
        frame.rows().lossless_inter_tx_type(),
        &DEFAULT_LOSSLESS_INTER_TX_TYPE_CDF
    );
    assert_eq!(frame.rows().delta_q(), &DEFAULT_DELTA_Q_CDF);
    assert_eq!(frame.rows().morph_pred(), &DEFAULT_MORPH_PRED_CDF);
    assert_eq!(frame.rows().fsc_mode(), &DEFAULT_FSC_MODE_CDF);
    assert_eq!(frame.rows().mrl_index(), &DEFAULT_MRL_INDEX_CDF);
    assert_eq!(frame.rows().mrl_sec_index(), &DEFAULT_MRL_SEC_INDEX_CDF);
    assert_eq!(frame.rows().y_mode_set(), &DEFAULT_Y_MODE_SET_CDF);
    assert_eq!(frame.rows().y_mode_index(), &DEFAULT_Y_MODE_INDEX_CDF);
    assert_eq!(frame.rows().txb_skip(), &DEFAULT_TXB_SKIP_CDF);
    assert_eq!(
        frame.rows().intra_tx_type_set1(),
        &DEFAULT_INTRA_TX_TYPE_SET1_CDF
    );
    assert_eq!(
        frame.rows().intra_tx_type_set2(),
        &DEFAULT_INTRA_TX_TYPE_SET2_CDF
    );
    assert_eq!(
        frame.rows().intra_tx_type_long(),
        &DEFAULT_INTRA_TX_TYPE_LONG_CDF
    );
    assert_eq!(
        frame.rows().is_long_side_dct(),
        &DEFAULT_IS_LONG_SIDE_DCT_CDF
    );
    assert_eq!(frame.rows().sec_tx_type(), &DEFAULT_SEC_TX_TYPE_CDF);
    assert_eq!(
        frame.rows().most_probable_stx_set(),
        &DEFAULT_MOST_PROBABLE_STX_SET_CDF
    );
    assert_eq!(
        frame.rows().most_probable_stx_set_adst(),
        &DEFAULT_MOST_PROBABLE_STX_SET_ADST_CDF
    );
    assert_eq!(frame.rows().cctx_type(), &DEFAULT_CCTX_TYPE_CDF);
    assert_eq!(
        frame.rows().uv_mode_cfl_not_allowed(),
        &DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF
    );
    assert_eq!(frame.rows().is_cfl(), &DEFAULT_IS_CFL_CDF);
    assert_eq!(frame.rows().cfl_index(), &DEFAULT_CFL_INDEX_CDF);
    assert_eq!(frame.rows().cfl_sign(), &DEFAULT_CFL_SIGN_CDF);
    assert_eq!(frame.rows().cfl_alpha(), &DEFAULT_CFL_ALPHA_CDF);
    assert_eq!(frame.rows().cfl_mhccp(), &DEFAULT_CFL_MHCCP_CDF);
    assert_eq!(frame.rows().cfl_mh_dir(), &DEFAULT_CFL_MH_DIR_CDF);
    assert_eq!(frame.rows().use_dip(), &DEFAULT_USE_DIP_CDF);
    assert_eq!(frame.rows().dip_mode(), &DEFAULT_DIP_MODE_CDF);
    assert_eq!(frame.rows().palette_y_mode(), &DEFAULT_PALETTE_Y_MODE_CDF);
    assert_eq!(frame.rows().v_txb_skip(), &DEFAULT_V_TXB_SKIP_CDF);
    assert_eq!(frame.rows().eob_extra(), &DEFAULT_EOB_EXTRA_CDF);
    assert_eq!(frame.rows().comp_mode(), &DEFAULT_COMP_MODE_CDF);
    assert_eq!(frame.rows().is_joint(), &DEFAULT_IS_JOINT_CDF);
    assert_eq!(
        frame.rows().compound_mode_non_joint(),
        &DEFAULT_COMPOUND_MODE_NON_JOINT_CDF
    );
    assert_eq!(frame.rows().compound_type(), &DEFAULT_COMPOUND_TYPE_CDF);
    assert_eq!(frame.rows().comp_group_idx(), &DEFAULT_COMP_GROUP_IDX_CDF);
    assert_eq!(frame.rows().cwp_idx(), &DEFAULT_CWP_IDX_CDF);
    assert_eq!(frame.rows().comp_ref0(), &DEFAULT_COMP_REF0_CDF);
    assert_eq!(frame.rows().comp_ref1(), &DEFAULT_COMP_REF1_CDF);
    assert_eq!(frame.rows().tip_mode(), &DEFAULT_TIP_MODE_CDF);
    assert_eq!(frame.rows().use_wiener_ns(), &DEFAULT_USE_WIENER_NS_CDF);
    assert_eq!(
        frame.rows().wiener_ns_length(),
        &DEFAULT_WIENER_NS_LENGTH_CDF
    );
    assert_eq!(
        frame.rows().wiener_ns_uv_sym(),
        &DEFAULT_WIENER_NS_UV_SYM_CDF
    );
    assert_eq!(frame.rows().wiener_ns_base(), &DEFAULT_WIENER_NS_BASE_CDF);

    let mut tile = frame.tile_copy();
    tile.rows_mut().do_split[0][0][0] = 1234;
    tile.rows_mut().do_ext_partition[0][4][0] = 5678;
    tile.rows_mut().rect_type[1][63][0] = 3456;
    tile.rows_mut().do_uneven_4way_partition[0][8][0] = 9012;

    assert_eq!(frame.rows().do_split()[0][0], DEFAULT_DO_SPLIT_CDF[0][0]);
    assert_eq!(
        frame.rows().do_ext_partition()[0][4],
        DEFAULT_DO_EXT_PARTITION_CDF[0][4]
    );
    assert_eq!(
        frame.rows().do_uneven_4way_partition()[0][8],
        DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF[0][8]
    );
    assert_eq!(
        frame.rows().rect_type()[1][63],
        DEFAULT_RECT_TYPE_CDF[1][63]
    );
    assert_ne!(
        tile.row(TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0
        })
        .unwrap(),
        DEFAULT_DO_SPLIT_CDF[0][0].as_slice()
    );
    assert_ne!(
        tile.row(TileCdfSelector::DoExtPartition {
            plane_start: 0,
            ctx: 4
        })
        .unwrap(),
        DEFAULT_DO_EXT_PARTITION_CDF[0][4].as_slice()
    );
    assert_ne!(
        tile.row(TileCdfSelector::RectType {
            plane_start: 1,
            ctx: 63
        })
        .unwrap(),
        DEFAULT_RECT_TYPE_CDF[1][63].as_slice()
    );
    assert_ne!(
        tile.row(TileCdfSelector::DoUneven4WayPartition {
            plane_start: 0,
            ctx: 8
        })
        .unwrap(),
        DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF[0][8].as_slice()
    );
}

#[test]
fn cfl_cdf_selectors_match_generated_defaults_and_check_bounds() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();

    assert_eq!(
        tile.row(TileCdfSelector::CflIndex).unwrap(),
        DEFAULT_CFL_INDEX_CDF.as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::CflSign).unwrap(),
        DEFAULT_CFL_SIGN_CDF.as_slice()
    );
    for (ctx, expected) in DEFAULT_CFL_ALPHA_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::CflAlpha { ctx }).unwrap(),
            expected.as_slice(),
            "cfl_alpha ctx {ctx}"
        );
    }
    assert_eq!(
        tile.row(TileCdfSelector::CflMhccp).unwrap(),
        DEFAULT_CFL_MHCCP_CDF.as_slice()
    );
    for (size_group, expected) in DEFAULT_CFL_MH_DIR_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::CflMhDir { size_group }).unwrap(),
            expected.as_slice(),
            "cfl_mh_dir size_group {size_group}"
        );
    }
    assert_eq!(
        tile.row(TileCdfSelector::PaletteYMode).unwrap(),
        DEFAULT_PALETTE_Y_MODE_CDF.as_slice()
    );

    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::CflAlpha { ctx: 6 },
        TileCdfArray::CflAlpha,
        "ctx",
        6,
        6,
    );
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::CflMhDir { size_group: 4 },
        TileCdfArray::CflMhDir,
        "size_group",
        4,
        4,
    );
}

#[test]
fn dip_cdf_selectors_match_generated_defaults_and_check_bounds() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();

    for (ctx, expected) in DEFAULT_USE_DIP_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::UseDip { ctx }).unwrap(),
            expected.as_slice(),
            "use_dip ctx {ctx}"
        );
    }
    assert_eq!(
        tile.row(TileCdfSelector::DipMode).unwrap(),
        DEFAULT_DIP_MODE_CDF.as_slice()
    );
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::UseDip { ctx: 3 },
        TileCdfArray::UseDip,
        "ctx",
        3,
        3,
    );
}

#[test]
fn compound_inter_cdf_selectors_load_defaults_and_bound_contexts() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();

    for (ctx, expected) in DEFAULT_COMP_MODE_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::CompMode { ctx }).unwrap(),
            expected.as_slice(),
            "comp_mode ctx {ctx}"
        );
    }
    for (ctx, expected) in DEFAULT_IS_JOINT_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::IsJoint { ctx }).unwrap(),
            expected.as_slice(),
            "is_joint ctx {ctx}"
        );
    }
    assert_eq!(
        tile.row(TileCdfSelector::JmvdScaleMode).unwrap(),
        DEFAULT_JMVD_SCALE_MODE_CDF.as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::JmvdAdaptiveScaleMode).unwrap(),
        DEFAULT_JMVD_ADAPTIVE_SCALE_MODE_CDF.as_slice()
    );
    for (ctx, expected) in DEFAULT_COMPOUND_MODE_NON_JOINT_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::CompoundModeNonJoint { ctx })
                .unwrap(),
            expected.as_slice(),
            "compound_mode_non_joint ctx {ctx}"
        );
    }
    assert_eq!(
        tile.row(TileCdfSelector::CompoundType).unwrap(),
        DEFAULT_COMPOUND_TYPE_CDF.as_slice()
    );
    for (ctx, expected) in DEFAULT_COMP_GROUP_IDX_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::CompGroupIdx { ctx }).unwrap(),
            expected.as_slice(),
            "comp_group_idx ctx {ctx}"
        );
    }
    for (idx, expected) in DEFAULT_CWP_IDX_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::CwpIdx { idx }).unwrap(),
            expected.as_slice(),
            "cwp_idx idx {idx}"
        );
    }
    for (ctx, expected) in DEFAULT_USE_OPTFLOW_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::UseOptflow { ctx }).unwrap(),
            expected.as_slice(),
            "use_optflow ctx {ctx}"
        );
    }
    for (ctx, ref_rows) in DEFAULT_COMP_REF0_CDF.iter().enumerate() {
        for (ref_idx, expected) in ref_rows.iter().enumerate() {
            assert_eq!(
                tile.row(TileCdfSelector::CompRef0 { ctx, ref_idx })
                    .unwrap(),
                expected.as_slice(),
                "comp_ref0 ctx {ctx} ref {ref_idx}"
            );
        }
    }
    for (ctx, bit_banks) in DEFAULT_COMP_REF1_CDF.iter().enumerate() {
        for (bit_type, ref_rows) in bit_banks.iter().enumerate() {
            for (ref_idx, expected) in ref_rows.iter().enumerate() {
                assert_eq!(
                    tile.row(TileCdfSelector::CompRef1 {
                        ctx,
                        bit_type,
                        ref_idx
                    })
                    .unwrap(),
                    expected.as_slice(),
                    "comp_ref1 ctx {ctx} bit_type {bit_type} ref {ref_idx}"
                );
            }
        }
    }
    for (ctx, expected) in DEFAULT_TIP_MODE_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::TipMode { ctx }).unwrap(),
            expected.as_slice(),
            "tip_mode ctx {ctx}"
        );
    }
    for (ctx, expected) in DEFAULT_SKIP_MODE_CDF.iter().enumerate() {
        assert_eq!(
            tile.row(TileCdfSelector::SkipMode { ctx }).unwrap(),
            expected.as_slice(),
            "skip_mode ctx {ctx}"
        );
    }

    tile.with_row_mut(TileCdfSelector::CompMode { ctx: 0 }, |row| row[0] = 12_345)
        .unwrap();
    assert_eq!(
        frame.rows().comp_mode()[0],
        DEFAULT_COMP_MODE_CDF[0],
        "tile_copy mutation must not alias frame defaults"
    );

    let error_cases = [
        (
            TileCdfSelector::CompMode { ctx: 5 },
            TileCdfArray::CompMode,
            "ctx",
            5,
            5,
        ),
        (
            TileCdfSelector::IsJoint { ctx: 2 },
            TileCdfArray::IsJoint,
            "ctx",
            2,
            2,
        ),
        (
            TileCdfSelector::CompoundModeNonJoint { ctx: 5 },
            TileCdfArray::CompoundModeNonJoint,
            "ctx",
            5,
            5,
        ),
        (
            TileCdfSelector::CompGroupIdx { ctx: 12 },
            TileCdfArray::CompGroupIdx,
            "ctx",
            12,
            12,
        ),
        (
            TileCdfSelector::CwpIdx { idx: 4 },
            TileCdfArray::CwpIdx,
            "idx",
            4,
            4,
        ),
        (
            TileCdfSelector::CompRef0 { ctx: 3, ref_idx: 0 },
            TileCdfArray::CompRef0,
            "ctx",
            3,
            3,
        ),
        (
            TileCdfSelector::CompRef1 {
                ctx: 3,
                bit_type: 0,
                ref_idx: 0,
            },
            TileCdfArray::CompRef1,
            "ctx",
            3,
            3,
        ),
        (
            TileCdfSelector::TipMode { ctx: 3 },
            TileCdfArray::TipMode,
            "ctx",
            3,
            3,
        ),
        (
            TileCdfSelector::SkipMode { ctx: 3 },
            TileCdfArray::SkipMode,
            "ctx",
            3,
            3,
        ),
    ];
    for (selector, array, index_name, actual, max_exclusive) in error_cases {
        assert_selector_out_of_range(&tile, selector, array, index_name, actual, max_exclusive);
    }
}

#[test]
fn selector_returns_rows_and_bounds_errors() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let row = tile
        .row(TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_DO_SPLIT_CDF[0][0].as_slice());
    assert_eq!(row.len(), CDF_ROW_LEN);

    let row = tile
        .row(TileCdfSelector::DoExtPartition {
            plane_start: 1,
            ctx: 63,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_DO_EXT_PARTITION_CDF[1][63].as_slice());

    let row = tile
        .row(TileCdfSelector::DoUneven4WayPartition {
            plane_start: 1,
            ctx: 63,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF[1][63].as_slice());

    let row = tile
        .row(TileCdfSelector::RectType {
            plane_start: 1,
            ctx: 63,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_RECT_TYPE_CDF[1][63].as_slice());

    let row = tile
        .row(TileCdfSelector::DoSquareSplit {
            plane_start: 0,
            ctx: 0,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_DO_SQUARE_SPLIT_CDF[0][0].as_slice());

    let row = tile.row(TileCdfSelector::LosslessInterTxType).unwrap();
    assert_eq!(row, DEFAULT_LOSSLESS_INTER_TX_TYPE_CDF.as_slice());

    let err = tile
        .with_row_mut(
            TileCdfSelector::DoSquareSplit {
                plane_start: 1,
                ctx: 0,
            },
            |_| (),
        )
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoSquareSplit,
            index_name: "plane_start",
            actual: 1,
            max_exclusive: 1,
        }
    );

    let err = tile
        .row(TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 64,
        })
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoSplit,
            index_name: "ctx",
            actual: 64,
            max_exclusive: 64,
        }
    );

    let err = tile
        .row(TileCdfSelector::DoExtPartition {
            plane_start: 2,
            ctx: 0,
        })
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoExtPartition,
            index_name: "plane_start",
            actual: 2,
            max_exclusive: 2,
        }
    );

    let err = tile
        .with_row_mut(
            TileCdfSelector::RectType {
                plane_start: 2,
                ctx: 0,
            },
            |_| (),
        )
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::RectType,
            index_name: "plane_start",
            actual: 2,
            max_exclusive: 2,
        }
    );

    let err = tile
        .row(TileCdfSelector::RectType {
            plane_start: 0,
            ctx: 64,
        })
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::RectType,
            index_name: "ctx",
            actual: 64,
            max_exclusive: 64,
        }
    );

    let err = tile
        .with_row_mut(
            TileCdfSelector::DoUneven4WayPartition {
                plane_start: 0,
                ctx: 64,
            },
            |_| (),
        )
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoUneven4WayPartition,
            index_name: "ctx",
            actual: 64,
            max_exclusive: 64,
        }
    );
}

#[test]
fn selected_row_hands_off_to_symbol_decoder_update_modes() {
    let frame = FrameCdfSubset::from_defaults();
    let selectors = [
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        },
        TileCdfSelector::DoExtPartition {
            plane_start: 0,
            ctx: 4,
        },
        TileCdfSelector::DoSquareSplit {
            plane_start: 0,
            ctx: 0,
        },
        TileCdfSelector::RectType {
            plane_start: 0,
            ctx: 4,
        },
        TileCdfSelector::DoUneven4WayPartition {
            plane_start: 0,
            ctx: 8,
        },
    ];
    let payload = [0x80, 0x00];

    for selector in selectors {
        let mut enabled = frame.tile_copy();
        let before = enabled.row(selector).unwrap().to_vec();
        let mut symbol = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap();
        enabled
            .read_partition_entry_symbol(selector, &mut symbol)
            .unwrap();
        assert_ne!(enabled.row(selector).unwrap(), before.as_slice());

        let mut disabled = frame.tile_copy();
        let before = disabled.row(selector).unwrap().to_vec();
        let mut symbol = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        )
        .unwrap();
        disabled
            .read_partition_entry_symbol(selector, &mut symbol)
            .unwrap();
        assert_eq!(disabled.row(selector).unwrap(), before.as_slice());
    }
}

#[test]
fn cdf_save_policy_matches_spec() {
    let single = tile_cdf_save_policy(TileCdfPolicyInput::new(1, 1, false, false, 0), 0).unwrap();
    assert_eq!(single.num_log2(), 0);
    assert!(single.copy_cdf());
    assert!(!single.avg_cdf());

    let avg = tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, true, true, 0), 2).unwrap();
    assert_eq!(avg.num_log2(), 2);
    assert!(avg.avg_cdf());
    assert!(!avg.copy_cdf());

    let not_averaged =
        tile_cdf_save_policy(TileCdfPolicyInput::new(16, 1, true, true, 0), 8).unwrap();
    assert_eq!(not_averaged.num_log2(), 3);
    assert!(!not_averaged.avg_cdf());

    let context = tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, false, false, 3), 3).unwrap();
    assert!(context.copy_cdf());

    assert!(matches!(
        tile_cdf_save_policy(TileCdfPolicyInput::new(u32::MAX, 2, false, false, 0), 0),
        Err(TileCdfError::TileCountOverflow { .. })
    ));
    assert!(matches!(
        tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, false, false, 4), 0),
        Err(TileCdfError::ContextUpdateTileOutOfRange { .. })
    ));
}

#[test]
fn saved_copy_and_average_are_exact_for_supported_subset() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().do_split[0][0] = [20_000, 7, 4];
    tile.rows_mut().do_ext_partition[0][4] = [22_000, 5, 8];
    tile.rows_mut().do_square_split[0][0] = [21_000, 6, 2];
    tile.rows_mut().rect_type[1][63] = [24_000, 3, 16];
    tile.rows_mut().do_uneven_4way_partition[0][8] = [23_000, 4, 12];
    tile.rows_mut().block.y_mode_set = [20_000, 21_000, 22_000, 9, 8];
    tile.rows_mut().block.y_mode_index[0] = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12,
    ];
    tile.rows_mut().block.txb_skip[2][0][0][0] = [25_000, 13, 20];
    tile.rows_mut().block.uv_mode_cfl_not_allowed[0] = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12,
    ];
    tile.rows_mut().block.cfl_index = [25_500, 13, 20];
    tile.rows_mut().block.cfl_sign = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12,
    ];
    tile.rows_mut().block.cfl_alpha[4] = [
        20_100, 21_100, 22_100, 23_100, 24_100, 25_100, 26_100, 11, 12,
    ];
    tile.rows_mut().block.cfl_mhccp = [26_000, 14, 24];
    tile.rows_mut().block.cfl_mh_dir[2] = [20_000, 21_000, 22_000, 9];
    tile.rows_mut().block.v_txb_skip[1][3] = [26_000, 14, 24];
    tile.rows_mut().block.coeff.coeff_base[1][2][3][1] = [20_000, 21_000, 22_000, 9, 8];
    tile.rows_mut().block.coeff.coeff_base_idtx[1][2][3] = [20_000, 21_000, 22_000, 9, 8];
    tile.rows_mut().block.coeff.idtx_sign[1][2][3] = [20_000, 9, 8];
    tile.rows_mut().intrabc_mode = [25_500, 13, 20];
    tile.rows_mut().intrabc_precision = [26_000, 14, 24];
    tile.rows_mut().mrl_index[1] = [20_000, 21_000, 22_000, 23_000, 20];
    tile.rows_mut().mrl_sec_index[2] = [20_000, 21_000, 20];
    tile.rows_mut().block.cctx_type = [20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 20];

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_completed_tile(
        0,
        &tile,
        TileCdfSavePolicy {
            num_log2: 0,
            copy_cdf: true,
            avg_cdf: false,
        },
    );
    assert_eq!(saved.rows(), tile.rows());

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_completed_tile(
        0,
        &tile,
        TileCdfSavePolicy {
            num_log2: 2,
            copy_cdf: false,
            avg_cdf: true,
        },
    );
    assert_eq!(saved.rows().do_split()[0][0], [29_576, 7, 1]);
    assert_eq!(saved.rows().do_ext_partition()[0][4], [30_076, 5, 2]);
    assert_eq!(saved.rows().do_square_split()[0][0], [29_826, 6, 0]);
    assert_eq!(saved.rows().rect_type()[1][63], [30_576, 3, 4]);
    assert_eq!(
        saved.rows().do_uneven_4way_partition()[0][8],
        [30_326, 4, 3]
    );
    assert_eq!(saved.rows().y_mode_set(), &[29_576, 29_826, 30_076, 9, 2]);
    assert_eq!(
        saved.rows().y_mode_index()[0],
        [
            29_576, 29_826, 30_076, 30_326, 30_576, 30_826, 31_076, 11, 3
        ]
    );
    assert_eq!(saved.rows().txb_skip()[2][0][0][0], [30_826, 13, 5]);
    assert_eq!(
        saved.rows().uv_mode_cfl_not_allowed()[0],
        [
            29_576, 29_826, 30_076, 30_326, 30_576, 30_826, 31_076, 11, 3
        ]
    );
    assert_eq!(saved.rows().cfl_index(), &[30_951, 13, 5]);
    assert_eq!(
        saved.rows().cfl_sign(),
        &[
            29_576, 29_826, 30_076, 30_326, 30_576, 30_826, 31_076, 11, 3
        ]
    );
    assert_eq!(
        saved.rows().cfl_alpha()[4],
        [
            29_601, 29_851, 30_101, 30_351, 30_601, 30_851, 31_101, 11, 3
        ]
    );
    assert_eq!(saved.rows().cfl_mhccp(), &[31_076, 14, 6]);
    assert_eq!(saved.rows().cfl_mh_dir()[2], [29_576, 29_826, 22_000, 2]);
    assert_eq!(saved.rows().v_txb_skip()[1][3], [31_076, 14, 6]);
    assert_eq!(
        saved.rows().block.coeff.coeff_base[1][2][3][1],
        [29_576, 29_826, 30_076, 9, 2]
    );
    assert_eq!(
        saved.rows().block.coeff.coeff_base_idtx[1][2][3],
        [29_576, 29_826, 30_076, 9, 2]
    );
    assert_eq!(saved.rows().block.coeff.idtx_sign[1][2][3], [29_576, 9, 2]);
    assert_eq!(saved.rows().intrabc_mode(), &[30_951, 13, 5]);
    assert_eq!(saved.rows().intrabc_precision(), &[31_076, 14, 6]);
    assert_eq!(
        saved.rows().mrl_index()[1],
        [29_576, 29_826, 30_076, 23_000, 5]
    );
    assert_eq!(saved.rows().mrl_sec_index()[2], [29_576, 21_000, 5]);
    assert_eq!(
        saved.rows().cctx_type(),
        &[29_576, 29_826, 30_076, 30_326, 30_576, 30_826, 26_000, 5]
    );
}

#[test]
fn disabled_cdf_update_keeps_saved_subset_at_initial_rows() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbol = SymbolDecoder::with_base_and_config(
        &[0x80, 0x00],
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();

    tile.read_partition_entry_symbol(
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        },
        &mut symbol,
    )
    .unwrap();

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_completed_tile(
        0,
        &tile,
        TileCdfSavePolicy {
            num_log2: 0,
            copy_cdf: true,
            avg_cdf: false,
        },
    );

    assert_eq!(tile.rows(), frame.rows());
    assert_eq!(saved.rows(), frame.rows());
}

#[test]
fn frame_end_update_copies_saved_rows_and_scales_counts() {
    let mut frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().do_split[0][0] = [20_000, 7, 20];
    tile.rows_mut().do_ext_partition[0][4] = [22_000, 5, 8];
    tile.rows_mut().do_square_split[0][0] = [21_000, 6, 2];
    tile.rows_mut().rect_type[1][63] = [24_000, 3, 16];
    tile.rows_mut().do_uneven_4way_partition[0][8] = [23_000, 4, 12];
    tile.rows_mut().block.y_mode_set = [20_000, 21_000, 22_000, 9, 20];
    tile.rows_mut().block.y_mode_index[0] = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12,
    ];
    tile.rows_mut().block.txb_skip[2][0][0][0] = [25_000, 13, 20];
    tile.rows_mut().block.uv_mode_cfl_not_allowed[0] = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 16,
    ];
    tile.rows_mut().block.cfl_index = [25_500, 13, 20];
    tile.rows_mut().block.cfl_sign = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 16,
    ];
    tile.rows_mut().block.cfl_alpha[4] = [
        20_100, 21_100, 22_100, 23_100, 24_100, 25_100, 26_100, 11, 16,
    ];
    tile.rows_mut().block.cfl_mhccp = [26_000, 14, 24];
    tile.rows_mut().block.cfl_mh_dir[2] = [20_000, 21_000, 22_000, 20];
    tile.rows_mut().block.v_txb_skip[1][3] = [26_000, 14, 24];
    tile.rows_mut().block.coeff.coeff_base[1][2][3][1] = [20_000, 21_000, 22_000, 9, 20];
    tile.rows_mut().block.coeff.coeff_base_bob[1][2][1] = [20_000, 21_000, 9, 20];
    tile.rows_mut().block.coeff.coeff_br_idtx[1][2][3] = [20_000, 21_000, 22_000, 9, 20];
    tile.rows_mut().block.coeff.idtx_sign[1][2][3] = [20_000, 9, 20];
    tile.rows_mut().mrl_index[1] = [20_000, 21_000, 22_000, 23_000, 20];
    tile.rows_mut().mrl_sec_index[2] = [20_000, 21_000, 20];
    tile.rows_mut().block.cctx_type = [20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 20];

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_completed_tile(
        0,
        &tile,
        TileCdfSavePolicy {
            num_log2: 0,
            copy_cdf: true,
            avg_cdf: false,
        },
    );
    frame.frame_end_update_from_saved(&saved);

    assert_eq!(frame.rows().do_split()[0][0], [20_000, 7, 15]);
    assert_eq!(frame.rows().do_ext_partition()[0][4], [22_000, 5, 6]);
    assert_eq!(frame.rows().do_square_split()[0][0], [21_000, 6, 1]);
    assert_eq!(frame.rows().rect_type()[1][63], [24_000, 3, 12]);
    assert_eq!(
        frame.rows().do_uneven_4way_partition()[0][8],
        [23_000, 4, 9]
    );
    assert_eq!(frame.rows().y_mode_set(), &[20_000, 21_000, 22_000, 9, 15]);
    assert_eq!(
        frame.rows().y_mode_index()[0],
        [
            20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 9
        ]
    );
    assert_eq!(frame.rows().txb_skip()[2][0][0][0], [25_000, 13, 15]);
    assert_eq!(
        frame.rows().uv_mode_cfl_not_allowed()[0],
        [
            20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12
        ]
    );
    assert_eq!(frame.rows().cfl_index(), &[25_500, 13, 15]);
    assert_eq!(
        frame.rows().cfl_sign(),
        &[
            20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12
        ]
    );
    assert_eq!(
        frame.rows().cfl_alpha()[4],
        [
            20_100, 21_100, 22_100, 23_100, 24_100, 25_100, 26_100, 11, 12
        ]
    );
    assert_eq!(frame.rows().cfl_mhccp(), &[26_000, 14, 18]);
    assert_eq!(frame.rows().cfl_mh_dir()[2], [20_000, 21_000, 22_000, 15]);
    assert_eq!(frame.rows().v_txb_skip()[1][3], [26_000, 14, 18]);
    assert_eq!(
        frame.rows().block.coeff.coeff_base[1][2][3][1],
        [20_000, 21_000, 22_000, 9, 15]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_base_bob[1][2][1],
        [20_000, 21_000, 9, 15]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_br_idtx[1][2][3],
        [20_000, 21_000, 22_000, 9, 15]
    );
    assert_eq!(frame.rows().block.coeff.idtx_sign[1][2][3], [20_000, 9, 15]);
    assert_eq!(
        frame.rows().mrl_index()[1],
        [20_000, 21_000, 22_000, 23_000, 15]
    );
    assert_eq!(frame.rows().mrl_sec_index()[2], [20_000, 21_000, 15]);
    assert_eq!(
        frame.rows().cctx_type(),
        &[20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 15]
    );
}

#[test]
fn work_unit_boundary_applies_saved_and_frame_updates_transactionally() {
    let expected_frame = FrameCdfSubset::from_defaults();
    let mut boundary = TileCdfWorkUnitBoundary::new(
        CdfUpdateMode::Enabled,
        TileCdfSavePolicy {
            num_log2: 0,
            copy_cdf: true,
            avg_cdf: false,
        },
        FrameCdfSubset::from_defaults(),
    );
    boundary.tile_cdfs_mut().rows_mut().do_split[0][0] = [20_000, 7, 20];
    boundary.tile_cdfs_mut().rows_mut().block.y_mode_set = [20_000, 21_000, 22_000, 9, 20];

    assert_eq!(boundary.saved_cdfs().rows(), expected_frame.rows());
    assert_eq!(boundary.frame_cdfs().rows(), expected_frame.rows());

    boundary.apply_completed_tile_to_saved(0);
    assert_eq!(
        boundary.saved_cdfs().rows().do_split()[0][0],
        [20_000, 7, 20]
    );
    assert_eq!(
        boundary.saved_cdfs().rows().y_mode_set(),
        &[20_000, 21_000, 22_000, 9, 20]
    );
    assert_eq!(boundary.frame_cdfs().rows(), expected_frame.rows());

    boundary.frame_end_update_cdf_subset();
    assert_eq!(
        boundary.frame_cdfs().rows().do_split()[0][0],
        [20_000, 7, 15]
    );
    assert_eq!(
        boundary.frame_cdfs().rows().y_mode_set(),
        &[20_000, 21_000, 22_000, 9, 15]
    );
}

#[test]
fn eob_extra_selector_returns_rows_and_bounds_error() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    for (q, expected) in DEFAULT_EOB_EXTRA_CDF.iter().enumerate() {
        let row = tile
            .row(TileCdfSelector::EobExtra { coeff_cdf_q_ctx: q })
            .unwrap();
        assert_eq!(row, expected.as_slice(), "eob_extra q-ctx {q}");
    }
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::EobExtra { coeff_cdf_q_ctx: 4 },
        TileCdfArray::EobExtra,
        "coeff_cdf_q_ctx",
        4,
        4,
    );
}

#[test]
fn eob_extra_tile_copy_does_not_alias_the_frame() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().block.eob_extra[2] = [12_345, 0, 7];
    assert_eq!(tile.rows().eob_extra()[2], [12_345, 0, 7]);
    assert_eq!(frame.rows().eob_extra()[2], DEFAULT_EOB_EXTRA_CDF[2]);
}

fn assert_eob_pt_bank<const N: usize>(
    tile: &TileCdfSubset,
    size: EobPtSize,
    expected: &[[[i32; N]; 3]; 4],
) {
    for (q, expected_q) in expected.iter().enumerate() {
        for (c, expected_qc) in expected_q.iter().enumerate() {
            let row = tile
                .row(TileCdfSelector::EobPt {
                    size,
                    coeff_cdf_q_ctx: q,
                    eob_ctx: c,
                })
                .unwrap();
            assert_eq!(row, expected_qc.as_slice(), "eob_pt {size:?} q {q} ctx {c}");
        }
    }
}

#[test]
fn eob_pt_family_loads_defaults_and_selects_by_size_and_context() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    assert_eob_pt_bank(&tile, EobPtSize::Pt16, &DEFAULT_EOB_PT_16_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt32, &DEFAULT_EOB_PT_32_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt64, &DEFAULT_EOB_PT_64_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt128, &DEFAULT_EOB_PT_128_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt256, &DEFAULT_EOB_PT_256_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt512, &DEFAULT_EOB_PT_512_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt1024, &DEFAULT_EOB_PT_1024_CDF);
}

#[test]
fn eob_pt_selector_rejects_out_of_range_contexts() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::EobPt {
            size: EobPtSize::Pt16,
            coeff_cdf_q_ctx: 4,
            eob_ctx: 0,
        },
        TileCdfArray::EobPt,
        "coeff_cdf_q_ctx",
        4,
        4,
    );
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::EobPt {
            size: EobPtSize::Pt1024,
            coeff_cdf_q_ctx: 0,
            eob_ctx: 3,
        },
        TileCdfArray::EobPt,
        "eob_ctx",
        3,
        3,
    );
}

#[test]
fn eob_pt_tile_copy_does_not_alias_the_frame() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().block.eob_pt_16[1][2] = [10, 20, 30, 40, 50, 7];
    assert_eq!(tile.rows().block.eob_pt_16[1][2], [10, 20, 30, 40, 50, 7]);
    assert_eq!(
        frame.rows().block.eob_pt_16[1][2],
        DEFAULT_EOB_PT_16_CDF[1][2]
    );
}

#[test]
fn dc_sign_loads_defaults_and_selects_by_all_indices() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    for (q, q_rows) in DEFAULT_DC_SIGN_CDF.iter().enumerate() {
        for (p, p_rows) in q_rows.iter().enumerate() {
            for (g, g_rows) in p_rows.iter().enumerate() {
                for (c, expected) in g_rows.iter().enumerate() {
                    let row = tile
                        .row(TileCdfSelector::DcSign {
                            coeff_cdf_q_ctx: q,
                            plane_type: p,
                            group: g,
                            ctx: c,
                        })
                        .unwrap();
                    assert_eq!(row, expected.as_slice(), "dc_sign q{q} p{p} g{g} c{c}");
                }
            }
        }
    }
}

#[test]
fn dc_sign_selector_rejects_out_of_range_indices() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 4,
            plane_type: 0,
            group: 0,
            ctx: 0,
        },
        TileCdfArray::DcSign,
        "coeff_cdf_q_ctx",
        4,
        4,
    );
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 0,
            plane_type: 2,
            group: 0,
            ctx: 0,
        },
        TileCdfArray::DcSign,
        "plane_type",
        2,
        2,
    );
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 0,
            plane_type: 0,
            group: 2,
            ctx: 0,
        },
        TileCdfArray::DcSign,
        "group",
        2,
        2,
    );
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 0,
            plane_type: 0,
            group: 0,
            ctx: 3,
        },
        TileCdfArray::DcSign,
        "ctx",
        3,
        3,
    );
}

#[test]
fn dc_sign_tile_copy_does_not_alias_the_frame() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().block.dc_sign[2][1][0][1] = [111, 5, 9];
    assert_eq!(tile.rows().block.dc_sign[2][1][0][1], [111, 5, 9]);
    assert_eq!(
        frame.rows().block.dc_sign[2][1][0][1],
        DEFAULT_DC_SIGN_CDF[2][1][0][1]
    );
}

#[test]
fn txb_skip_plane_type_error_still_names_txb_skip() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    assert_selector_out_of_range(
        &tile,
        TileCdfSelector::TxbSkip {
            coeff_cdf_q_ctx: 0,
            plane_type: 2,
            tx_size: 0,
            ctx: 0,
        },
        TileCdfArray::TxbSkip,
        "plane_type",
        2,
        2,
    );
}

#[test]
fn tx_partition_rows_load_defaults_and_report_selector_errors() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    assert_eq!(
        tile.row(TileCdfSelector::TxDoPartition {
            fsc_mode: 0,
            is_inter: 0,
            txfm_split_group: 2,
        })
        .unwrap(),
        DEFAULT_TX_DO_PARTITION_CDF[0][0][2].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::Tx2Or3PartitionType {
            fsc_mode: 1,
            is_inter: 0,
            ctx: 1,
        })
        .unwrap(),
        DEFAULT_TX_2OR3_PARTITION_TYPE_CDF[1][0][1].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::TxPartitionType {
            fsc_mode: 0,
            is_inter: 1,
            ctx: 3,
            reduced: false,
        })
        .unwrap(),
        DEFAULT_TX_PARTITION_TYPE_CDF[0][1][3].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::TxPartitionType {
            fsc_mode: 1,
            is_inter: 0,
            ctx: 4,
            reduced: true,
        })
        .unwrap(),
        DEFAULT_TX_PARTITION_TYPE_REDUCED_CDF[1][0][4].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::FscMode {
            ctx: 2,
            bsize_group: 4,
        })
        .unwrap(),
        DEFAULT_FSC_MODE_CDF[2][4].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::MrlIndex { ctx: 2 }).unwrap(),
        DEFAULT_MRL_INDEX_CDF[2].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::MrlSecIndex { ctx: 1 }).unwrap(),
        DEFAULT_MRL_SEC_INDEX_CDF[1].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::MorphPred { ctx: 2 }).unwrap(),
        DEFAULT_MORPH_PRED_CDF[2].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr: 2 })
            .unwrap(),
        DEFAULT_INTRA_TX_TYPE_SET1_CDF[2].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr: 1 })
            .unwrap(),
        DEFAULT_INTRA_TX_TYPE_SET2_CDF[1].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::IntraTxTypeLong { tx_size_sqr: 3 })
            .unwrap(),
        DEFAULT_INTRA_TX_TYPE_LONG_CDF[3].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::IsLongSideDct { is_inter: 0 })
            .unwrap(),
        DEFAULT_IS_LONG_SIDE_DCT_CDF[0].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::SecTxType {
            is_inter: 0,
            tx_size_sqr: 3,
        })
        .unwrap(),
        DEFAULT_SEC_TX_TYPE_CDF[0][3].as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::MostProbableStxSet).unwrap(),
        DEFAULT_MOST_PROBABLE_STX_SET_CDF.as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::MostProbableStxSetAdst).unwrap(),
        DEFAULT_MOST_PROBABLE_STX_SET_ADST_CDF.as_slice()
    );
    assert_eq!(
        tile.row(TileCdfSelector::CctxType).unwrap(),
        DEFAULT_CCTX_TYPE_CDF.as_slice()
    );

    let error_cases = [
        (
            TileCdfSelector::TxDoPartition {
                fsc_mode: 2,
                is_inter: 0,
                txfm_split_group: 0,
            },
            TileCdfArray::TxDoPartition,
            "fsc_mode",
            2,
            2,
        ),
        (
            TileCdfSelector::TxPartitionType {
                fsc_mode: 0,
                is_inter: 0,
                ctx: 14,
                reduced: true,
            },
            TileCdfArray::TxPartitionTypeReduced,
            "ctx",
            14,
            14,
        ),
        (
            TileCdfSelector::FscMode {
                ctx: 4,
                bsize_group: 0,
            },
            TileCdfArray::FscMode,
            "ctx",
            4,
            4,
        ),
        (
            TileCdfSelector::MrlIndex { ctx: 3 },
            TileCdfArray::MrlIndex,
            "ctx",
            3,
            3,
        ),
        (
            TileCdfSelector::MrlSecIndex { ctx: 3 },
            TileCdfArray::MrlSecIndex,
            "ctx",
            3,
            3,
        ),
        (
            TileCdfSelector::MorphPred { ctx: 3 },
            TileCdfArray::MorphPred,
            "ctx",
            3,
            3,
        ),
        (
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr: 3 },
            TileCdfArray::IntraTxTypeSet1,
            "tx_size_sqr",
            3,
            3,
        ),
        (
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr: 3 },
            TileCdfArray::IntraTxTypeSet2,
            "tx_size_sqr",
            3,
            3,
        ),
        (
            TileCdfSelector::IntraTxTypeLong { tx_size_sqr: 4 },
            TileCdfArray::IntraTxTypeLong,
            "tx_size_sqr",
            4,
            4,
        ),
        (
            TileCdfSelector::IsLongSideDct { is_inter: 2 },
            TileCdfArray::IsLongSideDct,
            "is_inter",
            2,
            2,
        ),
        (
            TileCdfSelector::SecTxType {
                is_inter: 2,
                tx_size_sqr: 0,
            },
            TileCdfArray::SecTxType,
            "is_inter",
            2,
            2,
        ),
        (
            TileCdfSelector::SecTxType {
                is_inter: 0,
                tx_size_sqr: 5,
            },
            TileCdfArray::SecTxType,
            "tx_size_sqr",
            5,
            5,
        ),
        (
            TileCdfSelector::FscMode {
                ctx: 0,
                bsize_group: 6,
            },
            TileCdfArray::FscMode,
            "bsize_group",
            6,
            6,
        ),
    ];
    for (selector, array, index_name, actual, max_exclusive) in error_cases {
        assert_selector_out_of_range(&tile, selector, array, index_name, actual, max_exclusive);
    }
}

#[test]
fn coeff_base_rows_load_defaults_and_select_by_family() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    let cases: &[(TileCdfSelector, &[i32])] = &[
        (
            coeff(CoeffCdfSelector::Base {
                coeff_cdf_q_ctx: 1,
                tx_size: 2,
                ctx: 3,
                tcq_ctx: 1,
            }),
            DEFAULT_COEFF_BASE_CDF[1][2][3][1].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BasePh {
                coeff_cdf_q_ctx: 2,
                ctx: 4,
            }),
            DEFAULT_COEFF_BASE_PH_CDF[2][4].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseUv {
                coeff_cdf_q_ctx: 2,
                ctx: 11,
            }),
            DEFAULT_COEFF_BASE_UV_CDF[2][11].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseLf {
                coeff_cdf_q_ctx: 3,
                tx_size: 4,
                ctx: 32,
                tcq_ctx: 0,
            }),
            DEFAULT_COEFF_BASE_LF_CDF[3][4][32][0].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseLfUv {
                coeff_cdf_q_ctx: 0,
                ctx: 11,
            }),
            DEFAULT_COEFF_BASE_LF_UV_CDF[0][11].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseEob {
                coeff_cdf_q_ctx: 1,
                tx_size: 2,
                ctx: 3,
            }),
            DEFAULT_COEFF_BASE_EOB_CDF[1][2][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseEobUv {
                coeff_cdf_q_ctx: 2,
                ctx: 3,
            }),
            DEFAULT_COEFF_BASE_EOB_UV_CDF[2][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseBob {
                coeff_cdf_q_ctx: 1,
                tx_size_ctx: 2,
                ctx: 1,
            }),
            DEFAULT_COEFF_BASE_BOB_CDF[1][2][1].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseIdtx {
                coeff_cdf_q_ctx: 3,
                tx_size_ctx: 2,
                ctx: 6,
            }),
            DEFAULT_COEFF_BASE_IDTX_CDF[3][2][6].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseLfEob {
                coeff_cdf_q_ctx: 3,
                tx_size: 4,
                ctx: 3,
            }),
            DEFAULT_COEFF_BASE_LF_EOB_CDF[3][4][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseLfEobUv {
                coeff_cdf_q_ctx: 1,
                ctx: 3,
            }),
            DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[1][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::Br {
                coeff_cdf_q_ctx: 2,
                ctx: 6,
            }),
            DEFAULT_COEFF_BR_CDF[2][6].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BrUv {
                coeff_cdf_q_ctx: 3,
                ctx: 3,
            }),
            DEFAULT_COEFF_BR_UV_CDF[3][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BrLf {
                coeff_cdf_q_ctx: 1,
                ctx: 13,
            }),
            DEFAULT_COEFF_BR_LF_CDF[1][13].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BrIdtx {
                coeff_cdf_q_ctx: 2,
                tx_size_ctx: 1,
                ctx: 6,
            }),
            DEFAULT_COEFF_BR_IDTX_CDF[2][1][6].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::IdtxSign {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 2,
                ctx: 8,
            }),
            DEFAULT_IDTX_SIGN_CDF[0][2][8].as_slice(),
        ),
    ];

    for (selector, expected) in cases {
        assert_eq!(tile.row(*selector).unwrap(), *expected, "{selector:?}");
    }
}

#[test]
fn coeff_base_selectors_reject_out_of_range_axes() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();

    let error_cases = [
        (
            coeff(CoeffCdfSelector::Base {
                coeff_cdf_q_ctx: 4,
                tx_size: 0,
                ctx: 0,
                tcq_ctx: 0,
            }),
            TileCdfArray::CoeffBase,
            "coeff_cdf_q_ctx",
            4,
            4,
        ),
        (
            coeff(CoeffCdfSelector::BaseLfEob {
                coeff_cdf_q_ctx: 0,
                tx_size: 5,
                ctx: 0,
            }),
            TileCdfArray::CoeffBaseLfEob,
            "tx_size",
            5,
            5,
        ),
        (
            coeff(CoeffCdfSelector::BasePh {
                coeff_cdf_q_ctx: 4,
                ctx: 0,
            }),
            TileCdfArray::CoeffBasePh,
            "coeff_cdf_q_ctx",
            4,
            4,
        ),
        (
            coeff(CoeffCdfSelector::BasePh {
                coeff_cdf_q_ctx: 0,
                ctx: 5,
            }),
            TileCdfArray::CoeffBasePh,
            "ctx",
            5,
            5,
        ),
        (
            coeff(CoeffCdfSelector::BaseBob {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 3,
                ctx: 0,
            }),
            TileCdfArray::CoeffBaseBob,
            "tx_size_ctx",
            3,
            3,
        ),
        (
            coeff(CoeffCdfSelector::BaseBob {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 0,
                ctx: 3,
            }),
            TileCdfArray::CoeffBaseBob,
            "ctx",
            3,
            3,
        ),
        (
            coeff(CoeffCdfSelector::BaseIdtx {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 0,
                ctx: 7,
            }),
            TileCdfArray::CoeffBaseIdtx,
            "ctx",
            7,
            7,
        ),
        (
            coeff(CoeffCdfSelector::BaseUv {
                coeff_cdf_q_ctx: 0,
                ctx: 12,
            }),
            TileCdfArray::CoeffBaseUv,
            "ctx",
            12,
            12,
        ),
        (
            coeff(CoeffCdfSelector::Base {
                coeff_cdf_q_ctx: 0,
                tx_size: 0,
                ctx: 0,
                tcq_ctx: 2,
            }),
            TileCdfArray::CoeffBase,
            "tcq_ctx",
            2,
            2,
        ),
        (
            coeff(CoeffCdfSelector::BrLf {
                coeff_cdf_q_ctx: 0,
                ctx: 14,
            }),
            TileCdfArray::CoeffBrLf,
            "ctx",
            14,
            14,
        ),
        (
            coeff(CoeffCdfSelector::BrIdtx {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 0,
                ctx: 7,
            }),
            TileCdfArray::CoeffBrIdtx,
            "ctx",
            7,
            7,
        ),
        (
            coeff(CoeffCdfSelector::IdtxSign {
                coeff_cdf_q_ctx: 0,
                tx_size_ctx: 0,
                ctx: 9,
            }),
            TileCdfArray::IdtxSign,
            "ctx",
            9,
            9,
        ),
    ];
    for (selector, array, index_name, actual, max_exclusive) in error_cases {
        assert_selector_out_of_range(&tile, selector, array, index_name, actual, max_exclusive);
    }
}

#[test]
fn coeff_base_tile_copy_does_not_alias_the_frame() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();

    tile.rows_mut().block.coeff.coeff_base[1][2][3][1] = [12_000, 13_000, 14_000, 7, 9];
    tile.rows_mut().block.coeff.coeff_base_ph[2][4] = [12_000, 13_000, 14_000, 8, 10];
    tile.rows_mut().block.coeff.coeff_base_lf_uv[0][11] =
        [11_000, 12_000, 13_000, 14_000, 15_000, 5, 8];
    tile.rows_mut().block.coeff.coeff_br_lf[1][13] = [15_000, 16_000, 17_000, 6, 12];
    tile.rows_mut().block.coeff.coeff_base_bob[1][2][1] = [12_000, 13_000, 8, 10];
    tile.rows_mut().block.coeff.coeff_base_idtx[3][2][6] = [12_000, 13_000, 14_000, 8, 10];
    tile.rows_mut().block.coeff.coeff_br_idtx[2][1][6] = [12_000, 13_000, 14_000, 8, 10];
    tile.rows_mut().block.coeff.idtx_sign[0][2][8] = [12_000, 8, 10];

    assert_eq!(
        frame.rows().block.coeff.coeff_base[1][2][3][1],
        DEFAULT_COEFF_BASE_CDF[1][2][3][1]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_base_ph[2][4],
        DEFAULT_COEFF_BASE_PH_CDF[2][4]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_base_lf_uv[0][11],
        DEFAULT_COEFF_BASE_LF_UV_CDF[0][11]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_br_lf[1][13],
        DEFAULT_COEFF_BR_LF_CDF[1][13]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_base_bob[1][2][1],
        DEFAULT_COEFF_BASE_BOB_CDF[1][2][1]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_base_idtx[3][2][6],
        DEFAULT_COEFF_BASE_IDTX_CDF[3][2][6]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_br_idtx[2][1][6],
        DEFAULT_COEFF_BR_IDTX_CDF[2][1][6]
    );
    assert_eq!(
        frame.rows().block.coeff.idtx_sign[0][2][8],
        DEFAULT_IDTX_SIGN_CDF[0][2][8]
    );
}

#[test]
fn coeff_base_row_hands_off_to_symbol_decoder_update_mode() {
    let frame = FrameCdfSubset::from_defaults();
    let selectors = [
        coeff(CoeffCdfSelector::BasePh {
            coeff_cdf_q_ctx: 1,
            ctx: 3,
        }),
        coeff(CoeffCdfSelector::BaseIdtx {
            coeff_cdf_q_ctx: 1,
            tx_size_ctx: 2,
            ctx: 3,
        }),
        coeff(CoeffCdfSelector::IdtxSign {
            coeff_cdf_q_ctx: 1,
            tx_size_ctx: 2,
            ctx: 3,
        }),
    ];
    let payload = [0x80, 0x00];

    for selector in selectors {
        let mut tile = frame.tile_copy();
        let before = tile.row(selector).unwrap().to_vec();
        let mut symbol = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap();
        let consumed_before = symbol.consumed_bits();

        tile.read_block_symbol_trace(selector, &mut symbol).unwrap();

        assert_ne!(
            tile.row(selector).unwrap(),
            before.as_slice(),
            "{selector:?}"
        );
        assert_ne!(symbol.consumed_bits(), consumed_before, "{selector:?}");
    }
}
