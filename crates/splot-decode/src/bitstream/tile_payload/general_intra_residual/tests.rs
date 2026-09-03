// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for general intra transform-block coefficient decode helpers.

#![allow(clippy::unwrap_used)]

use splot_core::segment::MAX_SEGMENTS;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::tables::conversion::MAX_TX_SIZE_RECT;

use super::*;
use crate::bitstream::tile_payload::TileCoeffFrameFactsInput;

const PAYLOAD: [u8; 2] = [0x00, 0x80];

fn symbol_decoder_for_payload(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap()
}

fn tile_cdfs() -> TileCdfSubset {
    crate::bitstream::tile_payload::FrameCdfSubset::from_defaults().tile_copy()
}

fn encode_transform_symbols(sequence: &[(TileCdfSelector, u8)]) -> Vec<u8> {
    let mut cdfs = tile_cdfs();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    );
    for &(selector, value) in sequence {
        cdfs.with_row_mut(selector, |row| {
            encoder.write_symbol_u16(row, Symbol::new(value))
        })
        .unwrap()
        .unwrap();
    }
    encoder.finish().unwrap().into_bytes()
}

fn sec_tx_type_payload(
    tx_size: usize,
    sec_tx_type: u8,
    most_probable_stx_set: Option<u8>,
) -> Vec<u8> {
    let tx_size_sqr = TX_SIZE_SQR[tx_size] as usize;
    let mut sequence = vec![(
        TileCdfSelector::SecTxType {
            is_inter: 0,
            tx_size_sqr,
        },
        sec_tx_type,
    )];
    if let Some(stx_set) = most_probable_stx_set {
        sequence.push((TileCdfSelector::MostProbableStxSet, stx_set));
    }
    encode_transform_symbols(&sequence)
}

fn intra_tx_type_set1_payload(tx_size: usize, intra_tx_type: u8) -> Vec<u8> {
    encode_transform_symbols(&[(
        TileCdfSelector::IntraTxTypeSet1 {
            tx_size_sqr: TX_SIZE_SQR[tx_size] as usize,
        },
        intra_tx_type,
    )])
}

fn intra_tx_type_set1_with_sec_tx_payload(
    tx_size: usize,
    intra_tx_type: u8,
    sec_tx_type: u8,
    stx_selector: TileCdfSelector,
    most_probable_stx_set: u8,
) -> Vec<u8> {
    encode_transform_symbols(&[
        (
            TileCdfSelector::IntraTxTypeSet1 {
                tx_size_sqr: TX_SIZE_SQR[tx_size] as usize,
            },
            intra_tx_type,
        ),
        (
            TileCdfSelector::SecTxType {
                is_inter: 0,
                tx_size_sqr: TX_SIZE_SQR[tx_size] as usize,
            },
            sec_tx_type,
        ),
        (stx_selector, most_probable_stx_set),
    ])
}

fn dc_luma_context() -> LumaTransformTypeContext {
    LumaTransformTypeContext::new(IntraYMode::Dc, 0)
}

#[test]
fn reconstruct_with_prediction_rejects_wrong_prediction_length() {
    let mut quant = vec![0i32; 16];
    quant[0] = 1;
    let block = LumaCoeffBlock {
        eob: 1,
        coeffs: {
            let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
            c
        },
        quant: {
            let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
            r
        },
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: DCT_DCT,
        use_tcq: false,
        lossless: false,
    };
    let prediction = vec![128u8; 8];
    let mut output = Vec::new();
    let result = reconstruct_general_intra_coeff_block_rect_with_prediction_into(
        &block,
        &prediction,
        &mut output,
        64,
        PlaneId::Y,
        2,
        2,
        false,
        None,
        None,
        BitDepth::Eight,
    );
    assert!(matches!(
        result,
        Err(GeneralIntraResidualError::PredictionLength {
            expected: 16,
            actual: 8
        })
    ));
}

#[test]
fn reconstruct_into_reuses_rectangular_u16_output_storage() {
    let mut quant = vec![0; 32];
    quant[0] = 1;
    let block = LumaCoeffBlock {
        eob: 1,
        coeffs: {
            let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
            c
        },
        quant: {
            let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
            r
        },
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: DCT_DCT,
        use_tcq: false,
        lossless: false,
    };
    let prediction = vec![301u16; 32];
    let mut out = Vec::with_capacity(32);
    out.push(7);
    let allocation = out.as_ptr();

    reconstruct_general_intra_coeff_block_rect_with_prediction_into(
        &block,
        &prediction,
        &mut out,
        80,
        PlaneId::Y,
        2,
        3,
        false,
        None,
        None,
        BitDepth::Ten,
    )
    .unwrap();

    assert_eq!(out, vec![302; 32]);
    assert_eq!(out.as_ptr(), allocation);
}

#[test]
fn reconstruct_into_supports_maximum_u8_transform_geometry() {
    let mut quant = vec![0; 32 * 32];
    quant[0] = 1;
    let block = LumaCoeffBlock {
        eob: 1,
        coeffs: {
            let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
            c
        },
        quant: {
            let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
            r
        },
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: DCT_DCT,
        use_tcq: false,
        lossless: false,
    };
    let prediction = vec![128u8; 64 * 64];
    let mut out = Vec::with_capacity(64 * 64);
    let allocation = out.as_ptr();

    reconstruct_general_intra_coeff_block_rect_with_prediction_into(
        &block,
        &prediction,
        &mut out,
        64,
        PlaneId::Y,
        6,
        6,
        false,
        None,
        None,
        BitDepth::Eight,
    )
    .unwrap();

    assert_eq!(out, prediction);
    assert_eq!(out.as_ptr(), allocation);
}

#[test]
fn reconstruct_into_keeps_output_on_truncated_inputs() {
    let prediction = vec![128u8; 16];
    let mut quant = vec![0; 15];
    quant[0] = 1;
    let invalid_quant = LumaCoeffBlock {
        eob: 1,
        coeffs: {
            let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
            c
        },
        quant: {
            let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
            r
        },
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: DCT_DCT,
        use_tcq: false,
        lossless: false,
    };
    let mut out = vec![91u8; 7];

    let quant_result = reconstruct_general_intra_coeff_block_rect_with_prediction_into(
        &invalid_quant,
        &prediction,
        &mut out,
        64,
        PlaneId::Y,
        2,
        2,
        false,
        None,
        None,
        BitDepth::Eight,
    );
    assert!(matches!(
        quant_result,
        Err(GeneralIntraResidualError::QuantLength {
            expected: 16,
            actual: 15,
        })
    ));
    assert_eq!(out, vec![91; 7]);

    let mut valid_quant = invalid_quant;
    let (coeffs, quant) = super::super::coeff_arena::sealed(vec![0i32; 16]);
    valid_quant.coeffs = coeffs;
    valid_quant.quant = quant;
    let prediction_result = reconstruct_general_intra_coeff_block_rect_with_prediction_into(
        &valid_quant,
        &prediction[..15],
        &mut out,
        64,
        PlaneId::Y,
        2,
        2,
        false,
        None,
        None,
        BitDepth::Eight,
    );
    assert!(matches!(
        prediction_result,
        Err(GeneralIntraResidualError::PredictionLength {
            expected: 16,
            actual: 15,
        })
    ));
    assert_eq!(out, vec![91; 7]);
}

#[test]
fn txb_skip_tx_size_ctx_matches_spec_formula_for_square_sizes() {
    for (tx_size, expected) in [(0, 0), (1, 1), (2, 2), (3, 3), (TX_64X64, 4)] {
        assert_eq!(txb_skip_tx_size_ctx(tx_size), expected);
    }
}

#[test]
fn txb_skip_tx_size_ctx_is_total_for_out_of_range_tx_size() {
    assert_eq!(txb_skip_tx_size_ctx(usize::MAX), 0);
    assert_eq!(txb_skip_tx_size_ctx(TX_SIZE_SQR.len()), 0);
}

#[test]
fn partition_type_symbols_map_to_every_partition_except_none() {
    let mapped: Vec<LumaTxPartition> = (0..=6)
        .map(LumaTxPartition::from_partition_type_symbol)
        .collect();

    assert_eq!(
        mapped,
        vec![
            LumaTxPartition::Split,
            LumaTxPartition::Horz,
            LumaTxPartition::Vert,
            LumaTxPartition::Horz4,
            LumaTxPartition::Vert4,
            LumaTxPartition::Horz5,
            LumaTxPartition::Vert5,
        ]
    );
}

#[test]
fn split_luma_transform_partition_records_follow_raster_order() {
    let records =
        luma_transform_records_for_partition(64, 32, TX_16X16, LumaTxPartition::Split).unwrap();
    let coords: Vec<(usize, usize, usize)> = records
        .iter()
        .map(|record| (record.x, record.y, record.tx_size))
        .collect();

    assert_eq!(
        coords,
        vec![
            (64, 32, TX_8X8),
            (72, 32, TX_8X8),
            (64, 40, TX_8X8),
            (72, 40, TX_8X8),
        ]
    );
}

#[test]
fn five_way_luma_transform_partition_fills_bounded_storage() {
    let records =
        luma_transform_records_for_partition(64, 32, TX_16X16, LumaTxPartition::Horz5).unwrap();
    let geometry: Vec<_> = records
        .iter()
        .map(|record| (record.x, record.y, record.middle))
        .collect();

    assert_eq!(records.len(), MAX_LUMA_TRANSFORM_PARTITION_UNITS);
    assert_eq!(
        geometry,
        [
            (64, 32, false),
            (72, 32, true),
            (64, 36, true),
            (64, 44, true),
            (72, 44, true),
        ]
    );
}

#[test]
fn every_transform_partition_has_spec_geometry_order_and_middle_flags() {
    type RecordGeometry = (usize, usize, usize, usize, bool);
    let cases: &[(LumaTxPartition, &[RecordGeometry])] = &[
        (LumaTxPartition::None, &[(64, 32, 16, 16, false)]),
        (
            LumaTxPartition::Split,
            &[
                (64, 32, 8, 8, false),
                (72, 32, 8, 8, false),
                (64, 40, 8, 8, false),
                (72, 40, 8, 8, false),
            ],
        ),
        (
            LumaTxPartition::Horz,
            &[(64, 32, 16, 8, false), (64, 40, 16, 8, false)],
        ),
        (
            LumaTxPartition::Vert,
            &[(64, 32, 8, 16, false), (72, 32, 8, 16, false)],
        ),
        (
            LumaTxPartition::Horz4,
            &[
                (64, 32, 16, 4, false),
                (64, 36, 16, 4, false),
                (64, 40, 16, 4, false),
                (64, 44, 16, 4, false),
            ],
        ),
        (
            LumaTxPartition::Vert4,
            &[
                (64, 32, 4, 16, false),
                (68, 32, 4, 16, false),
                (72, 32, 4, 16, false),
                (76, 32, 4, 16, false),
            ],
        ),
        (
            LumaTxPartition::Horz5,
            &[
                (64, 32, 8, 4, false),
                (72, 32, 8, 4, true),
                (64, 36, 16, 8, true),
                (64, 44, 8, 4, true),
                (72, 44, 8, 4, true),
            ],
        ),
        (
            LumaTxPartition::Vert5,
            &[
                (64, 32, 4, 8, false),
                (64, 40, 4, 8, true),
                (68, 32, 8, 16, true),
                (76, 32, 4, 8, true),
                (76, 40, 4, 8, true),
            ],
        ),
    ];

    for &(partition, expected) in cases {
        let records = luma_transform_records_for_partition(64, 32, TX_16X16, partition).unwrap();
        let actual: Vec<_> = records
            .iter()
            .map(|record| {
                let (width, height) = tx_size_dimensions(record.tx_size).unwrap();
                (record.x, record.y, width, height, record.middle)
            })
            .collect();
        assert_eq!(actual, expected, "partition {partition:?}");
    }
}

#[test]
fn luma_transform_partition_storage_has_only_syntax_cardinalities() {
    for (units, expected) in [
        (LumaTransformPartitionUnits::one(0), vec![0]),
        (LumaTransformPartitionUnits::two([0, 1]), vec![0, 1]),
        (
            LumaTransformPartitionUnits::four([0, 1, 2, 3]),
            vec![0, 1, 2, 3],
        ),
        (
            LumaTransformPartitionUnits::five([0, 1, 2, 3, 4]),
            vec![0, 1, 2, 3, 4],
        ),
    ] {
        assert_eq!(units.len(), expected.len());
        assert!(units.iter().copied().eq(expected));
    }
}

#[test]
fn every_block_size_context_derives_its_table_max_tx_size() {
    for (block_size_index, &max_tx_size) in MAX_TX_SIZE_RECT.iter().enumerate() {
        let block_size = BlockSize::new(block_size_index).unwrap();
        let context = LumaTransformPartitionContext::new(block_size);
        assert_eq!(context.block_size(), block_size);
        assert_eq!(context.max_tx_size(), max_tx_size as usize);
    }
}

#[test]
fn writer_uses_sequence_selected_transform_partition_cdf() {
    for reduced_tx_part_set in [false, true] {
        let payload = encode_transform_symbols(&[
            (
                TileCdfSelector::TxDoPartition {
                    fsc_mode: 0,
                    is_inter: 0,
                    txfm_split_group: 1,
                },
                1,
            ),
            (
                TileCdfSelector::TxPartitionType {
                    fsc_mode: 0,
                    is_inter: 0,
                    ctx: 0,
                    reduced: reduced_tx_part_set,
                },
                3,
            ),
        ]);
        let mut symbols = symbol_decoder_for_payload(&payload);
        let partition = read_luma_tx_partition_type(
            &mut tile_cdfs(),
            &mut symbols,
            3,
            false,
            false,
            true,
            true,
            reduced_tx_part_set,
        )
        .unwrap();

        assert_eq!(partition, LumaTxPartition::Horz4);
        symbols.exit_symbol().unwrap();
    }
}

#[test]
fn sequence_partition_reduction_suppresses_one_axis_four_way_symbol() {
    let full_payload = encode_transform_symbols(&[
        (
            TileCdfSelector::TxDoPartition {
                fsc_mode: 0,
                is_inter: 0,
                txfm_split_group: 8,
            },
            1,
        ),
        (
            TileCdfSelector::Tx2Or3PartitionType {
                fsc_mode: 0,
                is_inter: 0,
                ctx: 0,
            },
            1,
        ),
    ]);
    let mut full_symbols = symbol_decoder_for_payload(&full_payload);
    assert_eq!(
        read_luma_tx_partition_type(
            &mut tile_cdfs(),
            &mut full_symbols,
            19,
            false,
            false,
            true,
            false,
            false,
        )
        .unwrap(),
        LumaTxPartition::Horz4
    );
    full_symbols.exit_symbol().unwrap();

    let reduced_payload = encode_transform_symbols(&[(
        TileCdfSelector::TxDoPartition {
            fsc_mode: 0,
            is_inter: 0,
            txfm_split_group: 8,
        },
        1,
    )]);
    let mut reduced_symbols = symbol_decoder_for_payload(&reduced_payload);
    assert_eq!(
        read_luma_tx_partition_type(
            &mut tile_cdfs(),
            &mut reduced_symbols,
            19,
            false,
            false,
            true,
            false,
            true,
        )
        .unwrap(),
        LumaTxPartition::Horz
    );
    reduced_symbols.exit_symbol().unwrap();
}

#[test]
fn sequence_partition_and_frame_transform_type_reductions_are_independent() {
    let mut input = frame_facts_input();
    input.reduced_tx_set = 3;
    let frame_type_reduced = TileCoeffFrameFacts::new(input);
    assert_eq!(frame_type_reduced.reduced_tx_set(), 3);
    assert!(!frame_type_reduced.reduced_tx_part_set());

    input.reduced_tx_set = 0;
    input.reduced_tx_part_set = true;
    let sequence_partition_reduced = TileCoeffFrameFacts::new(input);
    assert_eq!(sequence_partition_reduced.reduced_tx_set(), 0);
    assert!(sequence_partition_reduced.reduced_tx_part_set());
}

#[test]
fn writer_produced_narrow_four_and_five_way_partitions_are_nonconforming() {
    let cases = [
        (
            3,
            vec![
                (
                    TileCdfSelector::TxDoPartition {
                        fsc_mode: 0,
                        is_inter: 0,
                        txfm_split_group: 1,
                    },
                    1,
                ),
                (
                    TileCdfSelector::TxPartitionType {
                        fsc_mode: 0,
                        is_inter: 0,
                        ctx: 0,
                        reduced: false,
                    },
                    3,
                ),
            ],
            LumaTxPartition::Horz4,
            (8, 0),
        ),
        (
            3,
            vec![
                (
                    TileCdfSelector::TxDoPartition {
                        fsc_mode: 0,
                        is_inter: 0,
                        txfm_split_group: 1,
                    },
                    1,
                ),
                (
                    TileCdfSelector::TxPartitionType {
                        fsc_mode: 0,
                        is_inter: 0,
                        ctx: 0,
                        reduced: false,
                    },
                    4,
                ),
            ],
            LumaTxPartition::Vert4,
            (0, 8),
        ),
        (
            3,
            vec![
                (
                    TileCdfSelector::TxDoPartition {
                        fsc_mode: 0,
                        is_inter: 0,
                        txfm_split_group: 1,
                    },
                    1,
                ),
                (
                    TileCdfSelector::TxPartitionType {
                        fsc_mode: 0,
                        is_inter: 0,
                        ctx: 0,
                        reduced: false,
                    },
                    5,
                ),
            ],
            LumaTxPartition::Horz5,
            (4, 0),
        ),
        (
            3,
            vec![
                (
                    TileCdfSelector::TxDoPartition {
                        fsc_mode: 0,
                        is_inter: 0,
                        txfm_split_group: 1,
                    },
                    1,
                ),
                (
                    TileCdfSelector::TxPartitionType {
                        fsc_mode: 0,
                        is_inter: 0,
                        ctx: 0,
                        reduced: false,
                    },
                    6,
                ),
            ],
            LumaTxPartition::Vert5,
            (0, 4),
        ),
    ];

    for (block_size_index, sequence, expected_partition, (width, height)) in cases {
        let payload = encode_transform_symbols(&sequence);
        let mut symbols = symbol_decoder_for_payload(&payload);
        let mut cdfs = tile_cdfs();
        let block_size = BlockSize::new(block_size_index).unwrap();
        let max_tx_size = LumaTransformPartitionContext::new(block_size).max_tx_size();
        let (tx_width, tx_height) = tx_size_dimensions(max_tx_size).unwrap();
        let partition = read_luma_tx_partition_type(
            &mut cdfs,
            &mut symbols,
            block_size_index,
            false,
            false,
            tx_size_from_dimensions(tx_width, tx_height >> 1).is_some(),
            tx_size_from_dimensions(tx_width >> 1, tx_height).is_some(),
            false,
        )
        .unwrap();
        assert_eq!(partition, expected_partition);
        symbols.exit_symbol().unwrap();
        let result = luma_transform_records_for_partition(0, 0, max_tx_size, partition);
        assert!(
            matches!(
                result,
                Err(GeneralIntraResidualError::InvalidTransformPartitionDimensions {
                    width: actual_width,
                    height: actual_height,
                }) if actual_width == width && actual_height == height
            ),
            "block {block_size_index} partition {partition:?} produced {result:?}"
        );
    }
}

#[test]
fn sparse_partition_filter_keeps_record_order_and_original_count() {
    let records =
        luma_transform_records_for_partition(320, 64, TX_64X64, LumaTxPartition::Split).unwrap();
    let record_count = records.len();
    let visible = records
        .try_filter_map::<_, ()>(|record| {
            Ok(luma_transform_record_starts_in_frame(&record, 352, 288).then_some(record))
        })
        .unwrap();

    assert_eq!(record_count, 4);
    assert_eq!(visible.len(), 2);
    assert!(
        visible
            .iter()
            .map(|record| (record.x, record.y))
            .eq([(320, 64), (320, 96),])
    );
}

#[test]
fn partitioned_luma_transform_record_does_not_fill_block_for_txb_skip_ctx() {
    let records =
        luma_transform_records_for_partition(64, 32, TX_16X16, LumaTxPartition::Split).unwrap();
    assert!(!luma_partition_record_fills_block(
        true,
        records.len(),
        *records.iter().next().unwrap(),
        64,
        32
    ));
    assert_eq!(txb_skip_ctx_luma(0, 0, false, false), 1);
    assert_eq!(txb_skip_ctx_luma(0, 0, true, false), 0);

    let unpartitioned = [LumaTransformPartitionRecord {
        middle: false,
        x: 64,
        y: 32,
        tx_size: TX_16X16,
    }];
    assert!(luma_partition_record_fills_block(
        true,
        unpartitioned.len(),
        unpartitioned[0],
        64,
        32
    ));
}

#[test]
fn partitioned_luma_transform_records_skip_units_starting_outside_frame() {
    let records =
        luma_transform_records_for_partition(320, 64, TX_64X64, LumaTxPartition::Split).unwrap();
    let visible: Vec<(usize, usize)> = records
        .iter()
        .filter(|record| luma_transform_record_starts_in_frame(record, 352, 288))
        .map(|record| (record.x, record.y))
        .collect();

    assert_eq!(visible, vec![(320, 64), (320, 96)]);
}

#[allow(clippy::fn_params_excessive_bools)]
fn frame_facts(
    enable_intra_ist: bool,
    enable_chroma_dctonly: bool,
    enable_cctx: bool,
) -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_intra_ist,
        enable_chroma_dctonly,
        enable_cctx,
        ..frame_facts_input()
    })
}

fn frame_facts_with_coeff_tools(allow_tcq: bool, allow_parity_hiding: bool) -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        allow_tcq,
        allow_parity_hiding,
        ..frame_facts_input()
    })
}

fn frame_facts_with_fsc() -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_fsc: true,
        ..frame_facts_input()
    })
}

fn lossless_frame_facts() -> TileCoeffFrameFacts {
    let mut input = frame_facts_input();
    input.enable_intra_ist = true;
    input.lossless_array[0] = true;
    TileCoeffFrameFacts::new(input)
}

fn frame_facts_input() -> TileCoeffFrameFactsInput {
    TileCoeffFrameFactsInput {
        enable_fsc: false,
        enable_intra_ist: false,
        enable_inter_ist: false,
        enable_chroma_dctonly: false,
        enable_cctx: false,
        reduced_tx_part_set: false,
        reduced_tx_set: 0,
        lossless_array: [false; MAX_SEGMENTS],
        allow_tcq: false,
        allow_parity_hiding: false,
        base_q_idx: 128,
    }
}

fn invalid_state_context<T>(result: &Result<T, GeneralIntraResidualError>) -> Option<&'static str> {
    match result {
        Err(GeneralIntraResidualError::InvalidReconstructionState { context }) => Some(*context),
        _ => None,
    }
}

fn assert_missing_luma_context_is_atomic(
    facts: TileCoeffFrameFacts,
    tx_size: usize,
    expected_context: &'static str,
) {
    let mut cdfs = tile_cdfs();
    let cdfs_before = cdfs.clone();
    let mut symbols = symbol_decoder_for_payload(&PAYLOAD);
    let symbol_count_before = symbols.symbol_count();
    let consumed_bits_before = symbols.consumed_bits();

    let result = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: facts,
            plane: 0,
            tx_size,
            is_inter: false,
            lossless: false,
            fsc_mode: false,
            eob: 2,
            cctx_allowed: true,
            luma_transform_type_context: None,
        },
    );

    assert_eq!(invalid_state_context(&result), Some(expected_context));
    assert_eq!(symbols.symbol_count(), symbol_count_before);
    assert_eq!(symbols.consumed_bits(), consumed_bits_before);
    assert_eq!(cdfs, cdfs_before);
}

fn ensure_with_test_state(
    facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
    eob: usize,
    luma: Option<LumaTransformTypeContext>,
) -> Result<TransformToolResidualMetadata, GeneralIntraResidualError> {
    ensure_with_test_payload_and_policy(facts, plane, tx_size, is_inter, eob, luma, &PAYLOAD)
}

fn ensure_with_test_payload_and_policy(
    facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
    eob: usize,
    luma: Option<LumaTransformTypeContext>,
    payload: &[u8],
) -> Result<TransformToolResidualMetadata, GeneralIntraResidualError> {
    ensure_with_test_payload_fsc_and_policy(
        facts, plane, tx_size, is_inter, false, eob, luma, payload,
    )
}

#[allow(clippy::too_many_arguments)]
fn ensure_with_test_payload_fsc_and_policy(
    facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
    fsc_mode: bool,
    eob: usize,
    luma: Option<LumaTransformTypeContext>,
    payload: &[u8],
) -> Result<TransformToolResidualMetadata, GeneralIntraResidualError> {
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(payload);
    ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: facts,
            plane,
            tx_size,
            is_inter,
            lossless: facts
                .lossless_for_segment(current_frame_qm_segment_id())
                .unwrap_or(false),
            fsc_mode,
            eob,
            cctx_allowed: true,
            luma_transform_type_context: luma,
        },
    )
}

#[test]
fn lossless_intra_transform_handoff_forces_dct_without_tx_type_or_ist_reads() {
    let payload = intra_tx_type_set1_with_sec_tx_payload(
        TX_8X8,
        1,
        1,
        TileCdfSelector::MostProbableStxSet,
        2,
    );
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&payload);

    let metadata = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: lossless_frame_facts(),
            plane: 0,
            tx_size: TX_8X8,
            is_inter: false,
            lossless: true,
            fsc_mode: false,
            eob: 2,
            cctx_allowed: true,
            luma_transform_type_context: Some(dc_luma_context()),
        },
    )
    .unwrap();

    assert_eq!(metadata.luma_tx_type, DCT_DCT);
    assert_eq!(metadata.intra_ist, None);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn lossless_fsc_luma_transform_handoff_retains_idtx_without_tx_type_read() {
    let payload = intra_tx_type_set1_payload(TX_8X8, 1);
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&payload);

    let metadata = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: lossless_frame_facts(),
            plane: 0,
            tx_size: TX_8X8,
            is_inter: false,
            lossless: true,
            fsc_mode: true,
            eob: 2,
            cctx_allowed: true,
            luma_transform_type_context: None,
        },
    )
    .unwrap();

    assert_eq!(metadata.luma_tx_type, IDTX);
    assert_eq!(metadata.intra_ist, None);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn lossless_fsc_chroma_transform_handoff_follows_luma_idtx() {
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&PAYLOAD);

    let metadata = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: lossless_frame_facts(),
            plane: 1,
            tx_size: TX_4X4,
            is_inter: false,
            lossless: true,
            fsc_mode: true,
            eob: 2,
            cctx_allowed: true,
            luma_transform_type_context: None,
        },
    )
    .unwrap();

    assert_eq!(metadata.luma_tx_type, IDTX);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn fsc_mode_luma_transform_handoff_derives_idtx_without_luma_context() {
    let metadata = ensure_with_test_payload_fsc_and_policy(
        frame_facts_with_fsc(),
        0,
        TX_8X8,
        false,
        true,
        2,
        None,
        &PAYLOAD,
    )
    .unwrap();

    assert_eq!(metadata.luma_tx_type, IDTX);
    assert_eq!(metadata.intra_ist, None);
}

#[test]
fn non_fsc_luma_transform_handoff_still_requires_luma_context() {
    assert_missing_luma_context_is_atomic(
        frame_facts_with_fsc(),
        TX_8X8,
        "active luma transform context",
    );
}

#[test]
fn dctonly_residual_admits_luma_when_ist_cannot_read_after_eob_limit() {
    let result = ensure_with_test_state(
        frame_facts(true, false, false),
        0,
        TX_32X32,
        false,
        IST_8X8_HEIGHT + 1,
        Some(dc_luma_context()),
    );

    assert!(result.is_ok());
}

#[test]
fn dctonly_residual_admits_luma_when_intra_ist_reads_zero_sec_tx_type() {
    let result = ensure_with_test_state(
        frame_facts(true, false, false),
        0,
        TX_32X32,
        false,
        2,
        Some(dc_luma_context()),
    );

    assert!(result.is_ok());
}

#[test]
fn dctonly_residual_rejects_intra_ist_without_luma_context() {
    assert_missing_luma_context_is_atomic(
        frame_facts(true, false, false),
        TX_32X32,
        "intra IST luma context",
    );
}

#[test]
fn typed_transform_and_ist_domains_do_not_reach_invalid_state() {
    let modes = [
        IntraYMode::Dc,
        IntraYMode::Vertical,
        IntraYMode::Horizontal,
        IntraYMode::D45,
        IntraYMode::D135,
        IntraYMode::D113,
        IntraYMode::D157,
        IntraYMode::D203,
        IntraYMode::D67,
        IntraYMode::Smooth,
        IntraYMode::SmoothVertical,
        IntraYMode::SmoothHorizontal,
        IntraYMode::Paeth,
    ];
    for tx_size in 0..TX_WIDTH.len() {
        let (width, height) = tx_size_dimensions(tx_size).unwrap();
        for mode in modes {
            for angle_delta in -3..=3 {
                for mrl_index in 0..=3 {
                    let context = LumaTransformTypeContext::with_mrl_indices(
                        mode,
                        angle_delta,
                        mrl_index,
                        None,
                    );
                    assert!(luma_transform_intra_dir(tx_size, context).is_ok());
                    assert!(intra_secondary_transform_mode(context, width, height).is_ok());
                }
            }
        }
    }
    for (mode, row) in INV_MOST_PROBABLE_STX_MAPPING.iter().enumerate() {
        for set in 0..row.len() {
            assert!(intra_secondary_transform_kernel(mode, DCT_DCT, set, 8, 8).is_ok());
        }
    }
    for (mode, row) in INV_MOST_PROBABLE_STX_MAPPING_ADST.iter().enumerate() {
        for set in 0..row.len() {
            assert!(intra_secondary_transform_kernel(mode, ADST_ADST, set, 8, 8).is_ok());
        }
    }
}

#[test]
fn invalid_ist_shape_is_reconstruction_state() {
    let block = LumaCoeffBlock {
        eob: 2,
        coeffs: {
            let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(vec![0; 16]);
            c
        },
        quant: {
            let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(vec![0; 16]);
            r
        },
        intra_ist: Some(IntraIstSyntax {
            sec_tx_type: 1,
            most_probable_stx_set: Some(0),
        }),
        cctx_type: None,
        plane_tx_type: DCT_DCT,
        use_tcq: false,
        lossless: false,
    };

    let result = reconstruct::resolve_secondary_inverse_transform(
        &block,
        1,
        2,
        BitDepth::Eight,
        Some(dc_luma_context()),
    );

    assert_eq!(
        invalid_state_context(&result),
        Some("intra IST transform shape")
    );
}

#[test]
fn dctonly_residual_lr_handoff_admits_active_intra_ist_metadata() {
    let payload = sec_tx_type_payload(TX_32X32, 1, Some(2));

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(true, false, false),
        0,
        TX_32X32,
        false,
        2,
        Some(dc_luma_context()),
        &payload,
    )
    .unwrap();

    assert_eq!(
        metadata.intra_ist,
        Some(IntraIstSyntax {
            sec_tx_type: 1,
            most_probable_stx_set: Some(2),
        })
    );
}

#[test]
fn dctonly_residual_maps_nonzero_intra_tx_type_to_non_dct() {
    let tx_type = md_idx_luma_tx_type(TX_8X8, dc_luma_context(), 1).unwrap();

    assert_ne!(tx_type, DCT_DCT);
}

const INTER_SET_TX_SIZE: usize = TX_8X16;
const INTER_SET_EOB: usize = 10;

fn read_inter_tx_type_from_symbols(tx_set: usize, sequence: &[(TileCdfSelector, u8)]) -> usize {
    let payload = encode_transform_symbols(sequence);
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&payload);
    read_active_inter_transform_type(
        &mut cdfs,
        &mut symbols,
        INTER_SET_TX_SIZE,
        tx_set,
        INTER_SET_EOB,
    )
    .unwrap()
}

#[test]
fn inter_tx_set_ctx_for_frontier_block_is_zero() {
    assert_eq!(
        inter_tx_type_long_ctx(INTER_SET_TX_SIZE, INTER_SET_EOB).unwrap(),
        0
    );
}

#[test]
fn inter_set1_index_branch_inverts_via_tx_type_inter_inv_set1() {
    let tx_size_sqr = TX_SIZE_SQR[INTER_SET_TX_SIZE] as usize;
    let tx_type = read_inter_tx_type_from_symbols(
        TX_SET_INTER_1,
        &[
            (
                TileCdfSelector::InterTxTypeSet1 {
                    ctx: 0,
                    tx_size_sqr,
                },
                0,
            ),
            (TileCdfSelector::InterTxTypeIndexSet1 { ctx: 0 }, 7),
        ],
    );
    assert_eq!(tx_type, TX_TYPE_INTER_INV_SET1[7]);
    assert_eq!(tx_type, DCT_DCT);
}

#[test]
fn inter_set1_offset_branch_inverts_via_tx_type_inter_inv_set1() {
    let tx_size_sqr = TX_SIZE_SQR[INTER_SET_TX_SIZE] as usize;
    let tx_type = read_inter_tx_type_from_symbols(
        TX_SET_INTER_1,
        &[
            (
                TileCdfSelector::InterTxTypeSet1 {
                    ctx: 0,
                    tx_size_sqr,
                },
                1,
            ),
            (TileCdfSelector::InterTxTypeOffsetSet1 { ctx: 0 }, 0),
        ],
    );
    assert_eq!(tx_type, TX_TYPE_INTER_INV_SET1[8]);
    assert_eq!(tx_type, ADST_DCT);
}

#[test]
fn inter_set2_index_branch_inverts_via_tx_type_inter_inv_set2() {
    let tx_type = read_inter_tx_type_from_symbols(
        TX_SET_INTER_2,
        &[
            (TileCdfSelector::InterTxTypeSet2 { ctx: 0 }, 0),
            (TileCdfSelector::InterTxTypeIndexSet2 { ctx: 0 }, 3),
        ],
    );
    assert_eq!(tx_type, TX_TYPE_INTER_INV_SET2[3]);
    assert_eq!(tx_type, DCT_DCT);
}

#[test]
fn inter_set2_offset_branch_inverts_via_tx_type_inter_inv_set2() {
    let tx_type = read_inter_tx_type_from_symbols(
        TX_SET_INTER_2,
        &[
            (TileCdfSelector::InterTxTypeSet2 { ctx: 0 }, 1),
            (TileCdfSelector::InterTxTypeOffsetSet2 { ctx: 0 }, 0),
        ],
    );
    assert_eq!(tx_type, TX_TYPE_INTER_INV_SET2[8]);
    assert_eq!(tx_type, ADST_ADST);
}

#[test]
fn inter_dct_idtx_set3_inverts_idtx_and_dct_dct() {
    let tx_size_sqr = TX_SIZE_SQR[INTER_SET_TX_SIZE] as usize;
    let idtx = read_inter_tx_type_from_symbols(
        TX_SET_DCT_IDTX,
        &[(
            TileCdfSelector::InterTxTypeSet3 {
                ctx: 0,
                tx_size_sqr,
            },
            0,
        )],
    );
    let dct = read_inter_tx_type_from_symbols(
        TX_SET_DCT_IDTX,
        &[(
            TileCdfSelector::InterTxTypeSet3 {
                ctx: 0,
                tx_size_sqr,
            },
            1,
        )],
    );
    assert_eq!(idtx, IDTX);
    assert_eq!(dct, DCT_DCT);
}

#[test]
fn inter_dct_idtx_iddct_set4_inverts_per_spec_table() {
    let tx_size_sqr = TX_SIZE_SQR[INTER_SET_TX_SIZE] as usize;
    for (symbol, expected) in [(0u8, DCT_DCT), (1, V_DCT), (2, H_DCT), (3, IDTX)] {
        let tx_type = read_inter_tx_type_from_symbols(
            TX_SET_DCT_IDTX_IDDCT,
            &[(
                TileCdfSelector::InterTxTypeSet4 {
                    ctx: 0,
                    tx_size_sqr,
                },
                symbol,
            )],
        );
        assert_eq!(tx_type, expected);
    }
}

#[test]
fn luma_transform_context_applies_mrl_delta_before_wide_angle_mapping() {
    let mode_index =
        crate::bitstream::tile_payload::cdf::block_context::ModeIndex::try_new(19).unwrap();
    let luma =
        crate::bitstream::tile_payload::cdf::block_context::reconstruct_y_mode_top_left(mode_index)
            .unwrap();
    assert_eq!(luma.y_mode.value(), 8);
    assert_eq!(luma.angle_delta_y, -2);

    let no_mrl =
        md_idx_luma_tx_type(TX_8X16, LumaTransformTypeContext::new(luma.y_mode, -2), 4).unwrap();
    let active_mrl = md_idx_luma_tx_type(
        TX_8X16,
        LumaTransformTypeContext::with_mrl_indices(luma.y_mode, -2, 2, None),
        4,
    )
    .unwrap();

    assert_ne!(no_mrl, active_mrl);
    assert_eq!(no_mrl, DCT_FLIPADST);
    assert_eq!(active_mrl, FLIPADST_DCT);
}

#[test]
fn luma_txtype_residual_lr_handoff_retains_non_dct_luma_tx_type() {
    let payload = intra_tx_type_set1_payload(TX_8X8, 1);
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 1).unwrap();

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false),
        0,
        TX_8X8,
        false,
        2,
        Some(luma),
        &payload,
    )
    .unwrap();

    assert_ne!(expected, DCT_DCT);
    assert_eq!(metadata.luma_tx_type, expected);
}

#[test]
fn luma_txtype_residual_lr_handoff_skips_intra_ist_for_non_sec_tx_type() {
    let payload = intra_tx_type_set1_payload(TX_8X8, 2);
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 2).unwrap();

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(true, false, false),
        0,
        TX_8X8,
        false,
        2,
        Some(luma),
        &payload,
    )
    .unwrap();

    assert_ne!(expected, DCT_DCT);
    assert_ne!(expected, ADST_ADST);
    assert_eq!(metadata.luma_tx_type, expected);
    assert_eq!(metadata.intra_ist, None);
}

#[test]
fn luma_txtype_residual_adst_adst_uses_reduced_ist_eob_limit() {
    let payload = intra_tx_type_set1_payload(TX_16X16, 1);
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_16X16, luma, 1).unwrap();

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(true, false, false),
        0,
        TX_16X16,
        false,
        IST_8X8_HEIGHT_RED + 1,
        Some(luma),
        &payload,
    )
    .unwrap();

    assert_eq!(expected, ADST_ADST);
    assert_eq!(metadata.luma_tx_type, ADST_ADST);
    assert_eq!(metadata.intra_ist, None);
}

#[test]
fn luma_txtype_residual_adst_adst_uses_reduced_ist_stx_cdf() {
    let payload = intra_tx_type_set1_with_sec_tx_payload(
        TX_8X8,
        1,
        1,
        TileCdfSelector::MostProbableStxSetAdst,
        2,
    );
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 1).unwrap();

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(true, false, false),
        0,
        TX_8X8,
        false,
        5,
        Some(luma),
        &payload,
    )
    .unwrap();

    assert_eq!(expected, ADST_ADST);
    assert_eq!(metadata.luma_tx_type, ADST_ADST);
    assert_eq!(
        metadata.intra_ist,
        Some(IntraIstSyntax {
            sec_tx_type: 1,
            most_probable_stx_set: Some(2),
        })
    );
}

#[test]
fn intra_ist_stx_selector_uses_reduced_adst_cdf_for_every_large_size() {
    for tx_size in 0..TX_WIDTH.len() {
        let (width, height) = tx_size_dimensions(tx_size).unwrap();
        let expected = if width >= 8 && height >= 8 {
            TileCdfSelector::MostProbableStxSetAdst
        } else {
            TileCdfSelector::MostProbableStxSet
        };
        assert_eq!(
            intra_ist_most_probable_stx_selector(tx_size, ADST_ADST).unwrap(),
            expected,
            "tx_size={tx_size} width={width} height={height}"
        );
        assert_eq!(
            intra_ist_most_probable_stx_selector(tx_size, DCT_DCT).unwrap(),
            TileCdfSelector::MostProbableStxSet,
            "tx_size={tx_size} width={width} height={height}"
        );
    }
}

#[test]
fn luma_txtype_residual_staged_base_config_uses_retained_luma_tx_type() {
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 1).unwrap();

    let config = staged_transform_tool_lossless_base_config(
        frame_facts(false, false, false),
        0,
        0,
        0,
        DCT_DCT,
        false,
        TransformToolResidualMetadata {
            luma_tx_type: expected,
            ..TransformToolResidualMetadata::default()
        },
    );

    assert_ne!(expected, DCT_DCT);
    assert_eq!(config.luma_tx_type, expected);
}

#[test]
fn lossless_fsc_staged_plane_tx_type_remains_idtx() {
    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane: 0,
        start_x: 0,
        start_y: 0,
        tx_size: TX_8X4,
    };
    let base_config = staged_transform_tool_lossless_base_config(
        lossless_frame_facts(),
        0,
        0,
        0,
        DCT_DCT,
        true,
        TransformToolResidualMetadata {
            luma_tx_type: IDTX,
            ..TransformToolResidualMetadata::default()
        },
    );

    assert_eq!(
        staged_transform_tool_plane_tx_type(geometry, false, true, base_config).unwrap(),
        IDTX
    );
}

#[test]
fn lossless_inter_staged_plane_tx_type_remains_idtx() {
    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane: 0,
        start_x: 0,
        start_y: 0,
        tx_size: TX_4X4,
    };
    let base_config = staged_transform_tool_lossless_base_config(
        lossless_frame_facts(),
        0,
        0,
        0,
        DCT_DCT,
        true,
        TransformToolResidualMetadata {
            luma_tx_type: IDTX,
            ..TransformToolResidualMetadata::default()
        },
    );

    assert_eq!(
        staged_transform_tool_plane_tx_type(geometry, true, true, base_config).unwrap(),
        IDTX
    );
}

#[test]
fn lossless_inter_chroma_staged_plane_tx_type_uses_colocated_luma_tx_type() {
    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane: 1,
        start_x: 0,
        start_y: 0,
        tx_size: TX_4X4,
    };
    let base_config = staged_transform_tool_lossless_base_config(
        lossless_frame_facts(),
        1,
        0,
        0,
        IDTX,
        true,
        TransformToolResidualMetadata::default(),
    );

    assert_eq!(
        staged_transform_tool_plane_tx_type(geometry, true, true, base_config).unwrap(),
        IDTX
    );
}

#[test]
fn luma_txtype_residual_staged_base_config_derives_coeff_tool_flags() {
    for (luma_tx_type, parity_hiding, use_tcq) in [
        (ADST_DCT, true, true),
        (IDTX, false, true),
        (V_DCT, true, false),
    ] {
        let config = staged_transform_tool_lossless_base_config(
            frame_facts_with_coeff_tools(true, true),
            0,
            0,
            0,
            DCT_DCT,
            false,
            TransformToolResidualMetadata {
                luma_tx_type,
                ..TransformToolResidualMetadata::default()
            },
        );

        assert_eq!(config.parity_hiding, parity_hiding);
        assert_eq!(config.use_tcq, use_tcq);
    }
}

#[test]
fn lossless_chroma_transform_handoff_skips_cctx_read() {
    let mut input = frame_facts_input();
    input.enable_cctx = true;
    input.lossless_array[0] = true;
    let facts = TileCoeffFrameFacts::new(input);
    let payload = encode_transform_symbols(&[(TileCdfSelector::CctxType, 1)]);
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&payload);

    let metadata = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: facts,
            plane: 1,
            tx_size: TX_8X8,
            is_inter: false,
            lossless: true,
            fsc_mode: false,
            eob: 2,
            cctx_allowed: true,
            luma_transform_type_context: None,
        },
    )
    .unwrap();

    assert_eq!(metadata.cctx_type, None);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn chroma_transform_handoff_skips_cctx_read_when_geometry_disallows() {
    let facts = frame_facts(false, false, true);
    let payload = encode_transform_symbols(&[(TileCdfSelector::CctxType, 1)]);
    for (is_inter, eob) in [(false, 2), (true, 1)] {
        for (cctx_allowed, expected_cctx_type, expected_symbols) in
            [(false, None, 0), (true, Some(1), 1)]
        {
            let mut cdfs = tile_cdfs();
            let mut symbols = symbol_decoder_for_payload(&payload);

            let metadata = ensure_transform_tool_residual_handoff(
                &mut cdfs,
                &mut symbols,
                TransformToolResidualInput {
                    frame_facts: facts,
                    plane: 1,
                    tx_size: TX_32X32,
                    is_inter,
                    lossless: false,
                    fsc_mode: false,
                    eob,
                    cctx_allowed,
                    luma_transform_type_context: None,
                },
            )
            .unwrap();

            assert_eq!(metadata.cctx_type, expected_cctx_type);
            assert_eq!(symbols.symbol_count(), expected_symbols);
        }
    }
}

#[test]
fn cctx_geometry_allowance_matches_spec_clause() {
    assert!(is_cctx_geometry_allowed(true, 64, 64));
    assert!(is_cctx_geometry_allowed(false, 16, 64));
    assert!(is_cctx_geometry_allowed(false, 64, 16));
    assert!(is_cctx_geometry_allowed(false, 16, 16));
    assert!(!is_cctx_geometry_allowed(false, 32, 32));
    assert!(!is_cctx_geometry_allowed(false, 32, 64));
    assert!(!is_cctx_geometry_allowed(false, 64, 64));
}

#[test]
fn lossless_inter_luma_transform_handoff_reads_tx_type_metadata() {
    for (symbol, expected_tx_type) in [(0, DCT_DCT), (1, IDTX)] {
        let facts = lossless_frame_facts();
        let payload = encode_transform_symbols(&[(TileCdfSelector::LosslessInterTxType, symbol)]);
        let mut cdfs = tile_cdfs();
        let mut symbols = symbol_decoder_for_payload(&payload);

        let metadata = ensure_transform_tool_residual_handoff(
            &mut cdfs,
            &mut symbols,
            TransformToolResidualInput {
                frame_facts: facts,
                plane: 0,
                tx_size: TX_4X4,
                is_inter: true,
                lossless: true,
                fsc_mode: false,
                eob: 16,
                cctx_allowed: true,
                luma_transform_type_context: None,
            },
        )
        .unwrap();

        assert_eq!(metadata.luma_tx_type, expected_tx_type);
        assert_eq!(symbols.symbol_count(), 1);
    }
}

#[test]
fn lossless_inter_luma_transform_handoff_large_tx_implies_idtx_without_symbol() {
    let facts = lossless_frame_facts();
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&[]);

    let metadata = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: facts,
            plane: 0,
            tx_size: TX_8X8,
            is_inter: true,
            lossless: true,
            fsc_mode: false,
            eob: 16,
            cctx_allowed: true,
            luma_transform_type_context: None,
        },
    )
    .unwrap();

    assert_eq!(metadata.luma_tx_type, IDTX);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn lossless_inter_chroma_transform_handoff_skips_tx_type_metadata() {
    let facts = lossless_frame_facts();
    let payload = encode_transform_symbols(&[(TileCdfSelector::LosslessInterTxType, 1)]);
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&payload);

    let metadata = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: facts,
            plane: 1,
            tx_size: TX_4X4,
            is_inter: true,
            lossless: true,
            fsc_mode: false,
            eob: 16,
            cctx_allowed: true,
            luma_transform_type_context: None,
        },
    )
    .unwrap();

    assert_eq!(metadata.luma_tx_type, DCT_DCT);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn dctonly_residual_lr_handoff_admits_chroma_non_dct_tx_set() {
    let result = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false),
        1,
        TX_8X8,
        false,
        1,
        None,
        &PAYLOAD,
    );

    assert!(result.is_ok());
}

#[test]
fn dctonly_residual_lr_handoff_admits_inter_chroma_non_dct_tx_set() {
    let result = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false),
        2,
        TX_8X8,
        true,
        1,
        None,
        &PAYLOAD,
    );

    assert!(result.is_ok());
}

#[test]
fn dctonly_residual_lr_handoff_reads_cctx_metadata() {
    for cctx_type in [0u8, 1] {
        let payload = encode_transform_symbols(&[(TileCdfSelector::CctxType, cctx_type)]);

        let metadata = ensure_with_test_payload_and_policy(
            frame_facts(false, false, true),
            1,
            TX_8X8,
            false,
            2,
            None,
            &payload,
        )
        .unwrap();

        assert_eq!(metadata.cctx_type, Some(usize::from(cctx_type)));
    }
}

#[test]
fn dctonly_residual_lr_handoff_reads_inter_cctx_metadata() {
    for cctx_type in [0u8, 1] {
        let payload = encode_transform_symbols(&[(TileCdfSelector::CctxType, cctx_type)]);

        let metadata = ensure_with_test_payload_and_policy(
            frame_facts(false, false, true),
            1,
            TX_8X8,
            true,
            1,
            None,
            &payload,
        )
        .unwrap();

        assert_eq!(metadata.cctx_type, Some(usize::from(cctx_type)));
    }
}

#[test]
fn dctonly_residual_maps_intra_tx_type_zero_to_dct_dct() {
    let tx_type = md_idx_luma_tx_type(TX_8X8, dc_luma_context(), 0).unwrap();

    assert_eq!(tx_type, DCT_DCT);
}

#[test]
fn dctonly_residual_long_set_maps_dct_symbol_only_for_long_side_dct() {
    assert_eq!(TX_TYPE_INV_LONG[1][0][0], DCT_DCT);
    assert_eq!(TX_TYPE_INV_LONG[1][1][0], DCT_DCT);
    assert_ne!(TX_TYPE_INV_LONG[0][0][0], DCT_DCT);
    assert_ne!(TX_TYPE_INV_LONG[0][1][0], DCT_DCT);
}

#[test]
fn fsc_idtx_block_reconstructs_without_tcq_dequant_shift() {
    let block = LumaCoeffBlock {
        eob: 16,
        coeffs: {
            let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(vec![
                0, 0, 0, 3, 0, 0, 2, 9, 0, 0, 0, 6, 0, 0, 0, 6,
            ]);
            c
        },
        quant: {
            let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(vec![
                0, 0, 0, 3, 0, 0, 2, 9, 0, 0, 0, 6, 0, 0, 0, 6,
            ]);
            r
        },
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: IDTX,
        use_tcq: false,
        lossless: false,
    };
    let prediction = vec![
        38u8, 40, 42, 43, 40, 41, 42, 43, 41, 41, 42, 42, 42, 42, 42, 42,
    ];

    let mut fsc = Vec::new();
    reconstruct_general_intra_coeff_block_rect_with_prediction_into(
        &block,
        &prediction,
        &mut fsc,
        78,
        PlaneId::Y,
        2,
        2,
        true,
        None,
        None,
        BitDepth::Eight,
    )
    .unwrap();

    assert_eq!(
        fsc,
        vec![
            38, 40, 42, 61, 40, 41, 54, 96, 41, 41, 42, 77, 42, 42, 42, 77
        ]
    );

    let mut ordinary_tcq = block;
    ordinary_tcq.use_tcq = true;
    let mut ordinary = Vec::new();
    reconstruct_general_intra_coeff_block_rect_with_prediction_into(
        &ordinary_tcq,
        &prediction,
        &mut ordinary,
        78,
        PlaneId::Y,
        2,
        2,
        true,
        None,
        None,
        BitDepth::Eight,
    )
    .unwrap();

    assert_eq!(
        ordinary,
        vec![
            38, 40, 42, 52, 40, 41, 48, 69, 41, 41, 42, 60, 42, 42, 42, 60
        ]
    );
}

#[test]
fn cctx_minus30_rotates_saved_chroma_dequant_pair() {
    let mut u = [-108, -54, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, 54, 0, 0, 0];
    let mut v = [0i32; 16];

    apply_cross_chroma_transform(5, BitDepth::Eight, &mut u, &mut v).unwrap();

    assert_eq!(u, [-94, -47, 0, 0, 0, 0, 0, 0, 47, 0, 0, 0, 47, 0, 0, 0]);
    assert_eq!(v, [54, 27, 0, 0, 0, 0, 0, 0, -27, 0, 0, 0, -27, 0, 0, 0]);
}

#[test]
fn invalid_cctx_state_preserves_coefficient_pairs() {
    let mut u = [11, 12];
    let mut v = [21, 22];
    for cctx_type in [0, 7] {
        let before = (u, v);
        let result = apply_cross_chroma_transform(cctx_type, BitDepth::Eight, &mut u, &mut v);

        assert!(matches!(
            result,
            Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CCTX type"
            })
        ));
        assert_eq!((u, v), before);
    }

    let mut short_u = [31];
    let mut long_v = [41, 42];
    let before = (short_u, long_v);
    let result = apply_cross_chroma_transform(1, BitDepth::Eight, &mut short_u, &mut long_v);

    assert!(matches!(
        result,
        Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX coefficient lengths"
        })
    ));
    assert_eq!((short_u, long_v), before);
}

#[test]
fn cctx_pair_uses_u_transform_type_for_all_zero_v_block() {
    let u_block = LumaCoeffBlock {
        eob: 7,
        coeffs: {
            let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(vec![
                -2, -1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0,
            ]);
            c
        },
        quant: {
            let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(vec![
                -2, -1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0,
            ]);
            r
        },
        intra_ist: None,
        cctx_type: Some(5),
        plane_tx_type: DCT_ADST,
        use_tcq: false,
        lossless: false,
    };
    let v_block = LumaCoeffBlock {
        eob: 0,
        coeffs: crate::bitstream::tile_payload::coeff_arena::batch(),
        quant: 0..0,
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: DCT_DCT,
        use_tcq: false,
        lossless: false,
    };
    let u_prediction = [
        124u8, 125, 125, 126, 126, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127,
    ];
    let v_prediction = [
        126u8, 126, 126, 127, 126, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127,
    ];

    let (u, v) = reconstruct_general_intra_chroma_cctx_pair_with_predictions(
        &u_block,
        &u_prediction,
        &v_block,
        &v_prediction,
        83,
        2,
        2,
        5,
        false,
        BitDepth::Eight,
    )
    .unwrap();

    assert_eq!(
        u,
        vec![
            123, 122, 124, 127, 123, 120, 119, 120, 125, 123, 124, 125, 125, 123, 124, 126
        ]
    );
    assert_eq!(
        v,
        vec![
            127, 127, 127, 127, 128, 131, 131, 131, 128, 129, 129, 128, 128, 129, 129, 128
        ]
    );
}

#[test]
fn residual_scratch_reuse_leaks_nothing_between_consecutive_blocks() {
    let reconstruct_a = || {
        let mut quant = vec![0i32; 16];
        quant[0] = 37;
        quant[1] = -21;
        quant[5] = 9;
        let block = LumaCoeffBlock {
            eob: 6,
            coeffs: {
                let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
                c
            },
            quant: {
                let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
                r
            },
            intra_ist: None,
            cctx_type: None,
            plane_tx_type: DCT_DCT,
            use_tcq: false,
            lossless: false,
        };
        let prediction = vec![301u16; 16];
        let mut output = Vec::new();
        reconstruct_general_intra_coeff_block_rect_with_prediction_into(
            &block,
            &prediction,
            &mut output,
            80,
            PlaneId::Y,
            2,
            2,
            false,
            None,
            None,
            BitDepth::Ten,
        )
        .unwrap();
        output
    };
    let reconstruct_b = || {
        let mut quant = vec![0i32; 64];
        quant[0] = -5;
        quant[9] = 61;
        let block = LumaCoeffBlock {
            eob: 10,
            coeffs: {
                let (c, _) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
                c
            },
            quant: {
                let (_, r) = crate::bitstream::tile_payload::coeff_arena::sealed(quant.clone());
                r
            },
            intra_ist: None,
            cctx_type: None,
            plane_tx_type: DCT_DCT,
            use_tcq: false,
            lossless: false,
        };
        let prediction = vec![144u16; 64];
        let mut output = Vec::new();
        reconstruct_general_intra_coeff_block_rect_with_prediction_into(
            &block,
            &prediction,
            &mut output,
            96,
            PlaneId::U,
            3,
            3,
            false,
            None,
            None,
            BitDepth::Ten,
        )
        .unwrap();
        output
    };

    let first_a = reconstruct_a();
    let b_after_a = reconstruct_b();
    let a_after_b = reconstruct_a();
    let (fresh_a, fresh_b) = std::thread::spawn(move || (reconstruct_a(), reconstruct_b()))
        .join()
        .unwrap();

    assert_eq!(first_a, fresh_a, "4x4 result depends on scratch history");
    assert_eq!(b_after_a, fresh_b, "8x8 result depends on scratch history");
    assert_eq!(a_after_b, fresh_a, "4x4 rerun depends on scratch history");
}

/// § 7.14.4 `segLvl` / `useQm` / `Qm_Offset` resolution: `tw > 8` selects the
/// `levels_gt8` set, `tw <= 8 && th <= 8` selects `levels_le8`, and the chroma
/// plane row is selected for U/V.
#[test]
fn resolve_block_qm_selects_level_plane_and_offset() {
    let _scope = FrameQmScope::install(Some(QmFrameLevels {
        levels_gt8: [8, 3, 4],
        levels_le8: {
            let mut levels = [[0u8; 3]; 16];
            levels[0] = [5, 2, 6];
            levels[3] = [9, 10, 11];
            levels
        },
    }));
    let luma = resolve_block_qm(PlaneId::Y, DCT_DCT, 32, 32, 5, 5).unwrap();
    assert_eq!(luma.seg_level, 8);
    assert!(!luma.plane_is_chroma);
    assert_eq!(
        luma.qm_offset,
        usize::try_from(QM_OFFSET[tx_size_index(5, 5).unwrap()]).unwrap()
    );
    let chroma_v = resolve_block_qm(PlaneId::V, DCT_DCT, 8, 8, 3, 3).unwrap();
    assert_eq!(chroma_v.seg_level, 6);
    assert!(chroma_v.plane_is_chroma);
    let _segment = FrameQmSegmentScope::install(3);
    let luma_segment = resolve_block_qm(PlaneId::Y, DCT_DCT, 8, 8, 3, 3).unwrap();
    assert_eq!(luma_segment.seg_level, 9);
}

#[test]
fn resolve_block_qm_limits_user_matrix_to_eight_by_eight() {
    let _qm = FrameQmScope::install(Some(QmFrameLevels {
        levels_gt8: [6, 6, 6],
        levels_le8: [[6, 6, 6]; 16],
    }));
    let mut levels: [Option<FrameUserQmLevel>; NUM_CUSTOM_QMS] = std::array::from_fn(|_| None);
    let mut transforms = std::array::from_fn(|_| std::array::from_fn(|_| None));
    transforms[0][0] = Some(QmUserPlane {
        width: 8,
        height: 8,
        values: Arc::from([16; 64]),
    });
    levels[6] = Some(FrameUserQmLevel { transforms });
    let _user = FrameUserQmScope::install(Some(Arc::new(levels)));

    assert!(
        resolve_block_qm(PlaneId::Y, DCT_DCT, 4, 4, 2, 2)
            .unwrap()
            .user
            .is_some()
    );
    assert!(
        resolve_block_qm(PlaneId::Y, DCT_DCT, 32, 32, 5, 5)
            .unwrap()
            .user
            .is_none()
    );
}

/// `resolve_block_qm` is the flat path (`None`) with no installed scope, for
/// `PlaneTxType >= IDTX`, and for `segLvl >= NUM_CUSTOM_QMS`.
#[test]
fn resolve_block_qm_none_for_flat_paths() {
    assert!(resolve_block_qm(PlaneId::Y, DCT_DCT, 32, 32, 5, 5).is_none());
    let _scope = FrameQmScope::install(Some(QmFrameLevels {
        levels_gt8: [8, 8, 8],
        levels_le8: [[8, 8, 8]; 16],
    }));
    assert!(resolve_block_qm(PlaneId::Y, IDTX, 32, 32, 5, 5).is_none());
    let _flat = FrameQmScope::install(Some(QmFrameLevels {
        levels_gt8: [NUM_CUSTOM_QMS as u8, 0, 0],
        levels_le8: [[0, 0, 0]; 16],
    }));
    assert!(resolve_block_qm(PlaneId::Y, DCT_DCT, 32, 32, 5, 5).is_none());
}
