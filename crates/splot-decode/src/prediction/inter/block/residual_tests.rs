// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

use crate::bitstream::tile_payload::{encode_symbol_sequence, make_test_work_unit};

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::super::block_chroma_subsampling;

const BLOCK_16X8: usize = 5;

#[test]
fn transform_origin_reaches_the_last_mi_of_a_65536_sample_plane() {
    let last_mi = (1 << 14) - 1;
    let mut blocks = Vec::new();
    push_inter_residual_block(
        &mut blocks,
        ReconPlaneId::Y,
        last_mi,
        last_mi,
        TX_4X4,
        LumaCoeffBlock::empty(DCT_DCT, false),
    )
    .unwrap();
    assert_eq!((blocks[0].x, blocks[0].y), (65532, 65532));
}

#[test]
fn residual_block_allocation_failure_is_operational() {
    let error = residual_allocation_error();

    assert!(matches!(
        error,
        crate::DecodeError::Reconstruction {
            source: splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: ReconPlaneId::Y,
                context: "inter residual transform-block list",
            }
        }
    ));
}

#[test]
fn residual_table_bounds_failure_is_typed_coefficient_state() {
    let error = tx_size_dimension(&[], 0).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterResidualCoefficientState,
        }
    ));
}

#[test]
fn residual_parse_failures_keep_typed_boundary() {
    let offset = ByteOffset::new(19);
    let parse_error =
        crate::bitstream::tile_payload::GeneralIntraResidualError::PaletteSymbolRead {
            source: BlockSymbolTraceReadError::Symbol(splot_core::Error::UnexpectedEof {
                offset: ByteOffset::new(0),
                needed: 1,
            }),
        };
    let error = residual_plane_read_error(&parse_error, offset);
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.offset() == Some(offset) && issue.spec_section() == Some(SPEC_RESIDUAL)
    ));

    let mut decoder = SymbolDecoder::new(&[0x80]).unwrap();
    let source = decoder.read_symbol_u16(&mut [0, 0, 0]).unwrap_err();
    let cdf_error = crate::bitstream::tile_payload::GeneralIntraResidualError::PaletteSymbolRead {
        source: BlockSymbolTraceReadError::Symbol(source),
    };
    let decoder = SymbolDecoder::new(&[]).unwrap();
    let decoder_state = decoder.finish().unwrap_err();
    let fallback =
        crate::bitstream::tile_payload::GeneralIntraResidualError::TransformPartitionGeometry {
            table: "test",
            index: usize::MAX,
        };
    for entropy_error in [
        &cdf_error as &(dyn std::error::Error + 'static),
        &decoder_state,
    ] {
        let error = residual_read_error(entropy_error, SPEC_RESIDUAL, offset);
        assert!(matches!(
            error,
            crate::DecodeError::HeaderState {
                source: crate::DecodeHeaderStateError::InvalidInterResidualCoefficientEntropyState,
            }
        ));
    }

    let error = residual_read_error(&fallback, SPEC_RESIDUAL, offset);
    assert!(matches!(
        error,
        crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterResidualCoefficientState,
        }
    ));

    let mut allocation = Vec::<u8>::new();
    let allocation_error = allocation.try_reserve(usize::MAX).unwrap_err();
    let error = residual_read_error(&allocation_error, SPEC_RESIDUAL, offset);
    assert!(matches!(
        error,
        crate::DecodeError::Reconstruction {
            source: splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: ReconPlaneId::Y,
                context: "inter residual coefficient parse state",
            }
        }
    ));
}

#[test]
fn lossless_tx_size_eof_uses_tx_size_spec_section() {
    let offset = ByteOffset::new(23);
    let parse_error = crate::bitstream::tile_payload::GeneralIntraBlockModeError::SymbolRead {
        reason: "lossless_tx_size",
        source: BlockSymbolTraceReadError::Symbol(splot_core::Error::UnexpectedEof {
            offset: ByteOffset::new(0),
            needed: 1,
        }),
    };

    let error = residual_read_error(&parse_error, SPEC_TX_SIZE, offset);
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.offset() == Some(offset) && issue.spec_section() == Some(SPEC_TX_SIZE)
    ));
}

#[test]
fn transform_type_eof_uses_transform_type_spec_section() {
    let offset = ByteOffset::new(29);
    let parse_error =
        crate::bitstream::tile_payload::GeneralIntraResidualError::TransformTypeRead {
            source: BlockSymbolTraceReadError::Symbol(splot_core::Error::UnexpectedEof {
                offset: ByteOffset::new(0),
                needed: 1,
            }),
        };

    let error = residual_plane_read_error(&parse_error, offset);
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.offset() == Some(offset)
                && issue.spec_section() == Some(SPEC_TRANSFORM_TYPE)
    ));
}

#[test]
fn reserved_pt512_eob_literal_is_malformed_residual_syntax() {
    let offset = ByteOffset::new(37);
    let parse_error = crate::bitstream::tile_payload::GeneralIntraResidualError::NonZeroStart {
        source: crate::bitstream::tile_payload::CoeffLoopContextError::InvalidPt512EobExtra {
            eob_pt_extra: 3,
        },
    };

    let error = residual_plane_read_error(&parse_error, offset);
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.offset() == Some(offset) && issue.spec_section() == Some("6.19.7.23")
    ));
}

#[test]
fn overlong_golomb_prefix_is_malformed_read_quant_syntax() {
    let offset = ByteOffset::new(41);
    let parse_error =
        crate::bitstream::tile_payload::CoeffReadQuantError::OverlongGolombPrefix { index: 0 };

    let error = residual_read_error(&parse_error, SPEC_RESIDUAL, offset);
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.offset() == Some(offset) && issue.spec_section() == Some("6.19.7.24")
    ));
}

#[test]
fn quant_magnitude_escape_is_malformed_coefficient_semantics() {
    let offset = ByteOffset::new(42);
    let parse_error =
        crate::bitstream::tile_payload::CoeffQuantStateWriteError::QuantMagnitudeOutOfRange {
            index: 0,
            magnitude: 1 << 20,
        };

    let error = residual_read_error(&parse_error, SPEC_RESIDUAL, offset);
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.offset() == Some(offset) && issue.spec_section() == Some("6.19.7.23")
    ));
}

#[test]
fn read_quant_eof_uses_read_quant_spec_section() {
    let offset = ByteOffset::new(43);
    let parse_error = crate::bitstream::tile_payload::CoeffReadQuantError::LiteralRead {
        index: 0,
        syntax: "golomb_length",
        source: splot_core::Error::UnexpectedEof {
            offset: ByteOffset::new(0),
            needed: 1,
        },
    };

    let error = residual_read_error(&parse_error, SPEC_RESIDUAL, offset);
    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource { issue }
            if issue.offset() == Some(offset) && issue.spec_section() == Some("5.20.7.28")
    ));
}

fn tx_size_for(width: usize, height: usize) -> usize {
    TX_WIDTH
        .iter()
        .zip(TX_HEIGHT.iter())
        .position(|(&w, &h)| w == width as i32 && h == height as i32)
        .expect("tx size")
}

#[test]
fn luma_tx_type_map_scales_chroma_coordinates_with_mi_floor() {
    let mut map = InterLumaTxTypeMap::new(9, 4, 8, 8).unwrap();
    map.update(9, 4, tx_size_for(8, 4), V_DCT).unwrap();

    assert_eq!(
        map.chroma_inter_tx_type(9, 4, 4, 2, (true, true), false),
        V_DCT
    );
    assert_eq!(
        map.chroma_inter_tx_type(9, 4, 5, 3, (true, true), false),
        DCT_DCT
    );
}

#[test]
fn lossless_non_base_chroma_uses_current_luma_tx_type() {
    let mut map = InterLumaTxTypeMap::new(9, 4, 8, 8).unwrap();
    map.update(9, 4, tx_size_for(8, 4), V_DCT).unwrap();

    assert_eq!(
        map.chroma_inter_tx_type(9, 4, 5, 3, (true, true), true),
        V_DCT
    );
}

#[test]
fn luma_tx_type_map_updates_on_16x16_units() {
    let mut map = InterLumaTxTypeMap::new(0, 0, 8, 8).unwrap();
    map.update(0, 0, tx_size_for(32, 16), V_DCT).unwrap();

    assert_eq!(usize::from(map.values[map.index(0, 0).unwrap()]), V_DCT);
    assert_eq!(usize::from(map.values[map.index(0, 4).unwrap()]), V_DCT);
    assert_eq!(usize::from(map.values[map.index(0, 7).unwrap()]), DCT_DCT);
}

/// AV2 § 5.20.7.23 parses each chroma group once at the `atStart` (top-left)
/// collocated luma chunk, interleaved before the remaining luma chunks. The
/// 64x128 4:2:0 case (widthChunks=1, heightChunks=2) is the regression: chroma
/// must parse at the first luma chunk `(0, 0)`, never the last `(0, 1)`.
#[test]
fn chroma_group_parses_at_group_start_chunk() {
    assert_eq!(
        chroma_parse_group_start(0, 0, 1, 2, true, true, false),
        Some((0, 0))
    );
    assert_eq!(
        chroma_parse_group_start(0, 1, 1, 2, true, true, false),
        None
    );

    assert_eq!(
        chroma_parse_group_start(0, 0, 2, 2, true, true, false),
        Some((0, 0))
    );
    assert_eq!(
        chroma_parse_group_start(1, 0, 2, 2, true, true, false),
        None
    );
    assert_eq!(
        chroma_parse_group_start(0, 1, 2, 2, true, true, false),
        None
    );
    assert_eq!(
        chroma_parse_group_start(1, 1, 2, 2, true, true, false),
        None
    );

    assert_eq!(
        chroma_parse_group_start(0, 0, 1, 1, true, true, false),
        Some((0, 0))
    );

    assert_eq!(
        chroma_parse_group_start(1, 1, 2, 2, false, false, false),
        Some((1, 1))
    );

    assert_eq!(
        chroma_parse_group_start(0, 1, 1, 2, true, true, true),
        Some((0, 1))
    );
}

#[test]
fn selectable_inter_luma_tx_records_skip_lossless_blocks() {
    assert!(inter_luma_tx_records_are_selectable(true, false));
    assert!(!inter_luma_tx_records_are_selectable(true, true));
    assert!(!inter_luma_tx_records_are_selectable(false, false));
}

#[test]
fn luma_tx_type_map_reuses_tile_local_storage() {
    let mut map = InterLumaTxTypeMap::new(0, 0, 8, 8).unwrap();
    map.update(0, 0, TX_4X4, V_DCT).unwrap();
    let capacity = map.values.capacity();
    let storage = map.values.as_ptr();
    map.reset(4, 8, 4, 4).unwrap();
    assert_eq!(
        map.chroma_inter_tx_type(4, 8, 4, 8, (false, false), false),
        DCT_DCT
    );
    map.update(4, 8, TX_4X4, V_DCT).unwrap();
    assert_eq!(
        map.chroma_inter_tx_type(4, 8, 4, 8, (false, false), false),
        V_DCT
    );
    assert_eq!(map.values.capacity(), capacity);
    assert_eq!(map.values.as_ptr(), storage);
}

#[test]
fn lossless_inter_residual_tx_size_reads_selector() {
    let offset = ByteOffset::new(0);
    let size_group =
        usize::try_from(splot_core::tables::conversion::SIZE_GROUP[BLOCK_16X8]).unwrap();
    let payload = encode_symbol_sequence(&[(
        TileCdfSelector::LosslessTxSize {
            size_group,
            is_inter: 1,
        },
        1,
    )]);
    let mut work_unit = make_test_work_unit(&payload, CdfUpdateMode::Disabled);
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        offset,
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();

    assert_eq!(
        inter_residual_tx_size(
            &mut work_unit,
            &mut symbols,
            BlockSize::new(BLOCK_16X8).unwrap(),
            true,
            offset,
        )
        .unwrap(),
        tx_size_for(16, 8)
    );
    assert_eq!(symbols.symbol_count(), 1);
}

/// A skipped block resets its chroma coefficient context over the plane extent,
/// so only 4:2:0 shifts both axes.
#[test]
fn block_chroma_subsampling_follows_the_chroma_format() {
    for (format, expected) in [
        (ChromaFormatIdc::Yuv420, (1, 1)),
        (ChromaFormatIdc::Yuv422, (1, 0)),
        (ChromaFormatIdc::Yuv444, (0, 0)),
        (ChromaFormatIdc::Monochrome, (1, 1)),
    ] {
        assert_eq!(block_chroma_subsampling(format), expected, "{format:?}");
    }
}

/// § 5.20.7.23 stops each luma chunk loop at `MiCols` / `MiRows`: a 66x50
/// frame has `MiCols` 18 and `MiRows` 14.
#[test]
fn luma_chunk_loop_stops_at_the_mode_info_grid() {
    assert_eq!(luma_chunk_end(8, 0, 18), 8);
    assert_eq!(luma_chunk_end(32, 0, 64), 16);
    assert_eq!(luma_chunk_end(16, 0, 14), 14);
    assert_eq!(luma_chunk_end(32, 16, 18), 18);
    assert_eq!(luma_chunk_end(32, 16, 16), 16);
}

/// § 5.20.7.24 `maxX = (MiCols * MI_SIZE) >> subX`: with `MiCols` 18 and
/// `MiRows` 14 a plane ends at 4x4 column 18 (luma, 4:4:4) or 9 (4:2:0,
/// 4:2:2), and at row 14 or 7 (4:2:0). An odd `MiCols` 17 leaves a 2-sample
/// column at 4:2:0 that still starts a transform block.
#[test]
fn transform_units_at_or_past_the_plane_edge_are_not_coded() {
    assert_eq!(plane_end4(18, false), 18);
    assert_eq!(plane_end4(18, true), 9);
    assert_eq!(plane_end4(14, false), 14);
    assert_eq!(plane_end4(14, true), 7);
    assert_eq!(plane_end4(17, true), 9);
}

fn read_edge_chroma_group(payload: &[u8]) -> Result<Vec<InterResidualBlock>> {
    const BLOCK_16X16: usize = 6;
    let frontier = DecodeBlockFrontier::shared_leaf(12, 16, BlockSize::new(BLOCK_16X16).unwrap());
    let mut work_unit = make_test_work_unit(payload, CdfUpdateMode::Disabled);
    let mut symbols = SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();
    let mut coeff_ctx =
        TileCoeffContextState::new_for_tile_chroma(0..14, 0..18, ChromaFormatIdc::Yuv420).unwrap();
    let mut blocks = Vec::new();
    crate::test_support::with_auto_backend!(backend => read_inter_residual_chroma_group(
        backend,
        &mut work_unit,
        &mut symbols,
        &mut coeff_ctx,
        &mut blocks,
        &mut Vec::new(),
        &InterLumaTxTypeMap::new(12, 16, 4, 4).unwrap(),
        &frontier,
        frontier.b_size,
        true,
        true,
        0,
        0,
        Some((14, 18)),
        Some(TX_4X4),
        false,
        TransformToolResidualPolicy::default(),
        ByteOffset::new(0),
    ))?;
    Ok(blocks)
}

/// A lossless 16x16 4:2:0 block at mi (12, 16) of a 66x50 frame has four
/// 4x4 chroma units per plane; only the one at chroma (8, 6) starts before
/// `maxX` 36 and `maxY` 28, so U and V read one `all_zero` each.
#[test]
fn lossless_chroma_group_reads_only_units_inside_the_plane() {
    let blocks = read_edge_chroma_group(&[0xff; 16]).unwrap();
    let units: Vec<_> = blocks
        .iter()
        .map(|block| (block.plane, block.x, block.y, block.coeffs.eob))
        .collect();
    assert_eq!(
        units,
        [(ReconPlaneId::U, 32, 24, 0), (ReconPlaneId::V, 32, 24, 0)]
    );
}

/// Past the payload end the § 8.2 decoder reads padding, and the group
/// still reads only the units inside the plane.
#[test]
fn lossless_chroma_group_at_payload_end_reads_only_units_inside_the_plane() {
    let blocks = read_edge_chroma_group(&[]).unwrap();
    let units: Vec<_> = blocks
        .iter()
        .map(|block| (block.plane, block.x, block.y))
        .collect();
    assert_eq!(
        units,
        [(ReconPlaneId::U, 32, 24), (ReconPlaneId::V, 32, 24)]
    );
}

/// A 16x16 luma read that fails on an overlong Golomb prefix leaves the
/// block list and the coefficient arena as they were before the block.
#[test]
fn failed_inter_residual_drops_its_blocks_and_coefficients() {
    const BLOCK_16X16: usize = 6;
    const FIXTURE: &[u8] = include_bytes!(
        "../../../../../../tests/conformance/vectors/valid/syn-2frame-global-rotzoom-64x64-q80.ivf"
    );
    let payload = [0x12, 0x02, 0x00, 0x00, 0x52, 0xb1, 0xc4, 0x0c];
    let (sequence, core, _) =
        super::super::super::tests::parse_inter_core_for_validation(FIXTURE).unwrap();
    let frontier = DecodeBlockFrontier::shared_leaf(0, 0, BlockSize::new(BLOCK_16X16).unwrap());
    let mut work_unit = make_test_work_unit(&payload, CdfUpdateMode::Disabled);
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();
    let mut coeff_ctx =
        TileCoeffContextState::new_for_tile_chroma(0..16, 0..16, ChromaFormatIdc::Yuv420).unwrap();
    let mut blocks = Vec::new();
    let mut arena = vec![7; 3];

    let result = crate::test_support::with_auto_backend!(backend => read_inter_residual(
        backend,
        &mut work_unit,
        &mut symbols,
        &mut coeff_ctx,
        &mut InterLumaTxTypeMap::default(),
        &mut blocks,
        &mut arena,
        &sequence,
        &core,
        &frontier,
        4,
        4,
        16,
        16,
        false,
        TransformToolResidualPolicy::default(),
        ByteOffset::new(0),
    ));

    assert!(result.is_err());
    assert!(blocks.is_empty());
    assert_eq!(arena, [7; 3]);
}
