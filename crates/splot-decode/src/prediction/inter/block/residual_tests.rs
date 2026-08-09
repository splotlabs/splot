// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;

use crate::bitstream::tile_payload::{encode_symbol_sequence, make_test_work_unit};

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::super::block_chroma_subsampling;

const BLOCK_16X8: usize = 5;

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
fn residual_table_bounds_failure_is_internal() {
    let offset = ByteOffset::new(17);
    let error = tx_size_dimension(&[], 0, offset).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::InternalState {
            reason: "inter_block_residual_geometry",
            byte_offset,
        } if byte_offset == offset
    ));
}

#[test]
fn residual_parse_failures_keep_typed_boundary() {
    let offset = ByteOffset::new(19);
    let parse_error = crate::bitstream::tile_payload::GeneralIntraResidualError::AllZeroRead {
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
    let cdf_error = crate::bitstream::tile_payload::GeneralIntraResidualError::AllZeroRead {
        source: BlockSymbolTraceReadError::Symbol(source),
    };
    let decoder = SymbolDecoder::new(&[]).unwrap();
    let decoder_state = decoder.finish().unwrap_err();
    let fallback =
        crate::bitstream::tile_payload::GeneralIntraResidualError::TransformPartitionGeometry {
            table: "test",
            index: usize::MAX,
        };
    let internal_errors: [&(dyn std::error::Error + 'static); 3] =
        [&cdf_error, &decoder_state, &fallback];
    for internal_error in internal_errors {
        let error = residual_read_error(internal_error, SPEC_RESIDUAL, offset);
        assert!(matches!(
            error,
            crate::DecodeError::InternalState {
                reason: "inter_block_residual_parse",
                byte_offset,
            } if byte_offset == offset
        ));
    }

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
fn cctx_type_eof_uses_residual_spec_section() {
    let offset = ByteOffset::new(31);
    let parse_error = crate::bitstream::tile_payload::GeneralIntraResidualError::CctxTypeRead {
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
            if issue.offset() == Some(offset) && issue.spec_section() == Some(SPEC_RESIDUAL)
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
            if issue.offset() == Some(offset) && issue.spec_section() == Some(SPEC_READ_QUANT)
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
            if issue.offset() == Some(offset) && issue.spec_section() == Some(SPEC_READ_QUANT)
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
    let offset = ByteOffset::new(0);
    let mut map = InterLumaTxTypeMap::new(9, 4, 8, 8, offset).unwrap();
    map.update(9, 4, tx_size_for(8, 4), V_DCT, offset).unwrap();

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
    let offset = ByteOffset::new(0);
    let mut map = InterLumaTxTypeMap::new(9, 4, 8, 8, offset).unwrap();
    map.update(9, 4, tx_size_for(8, 4), V_DCT, offset).unwrap();

    assert_eq!(
        map.chroma_inter_tx_type(9, 4, 5, 3, (true, true), true),
        V_DCT
    );
}

#[test]
fn luma_tx_type_map_updates_on_16x16_units() {
    let offset = ByteOffset::new(0);
    let mut map = InterLumaTxTypeMap::new(0, 0, 8, 8, offset).unwrap();
    map.update(0, 0, tx_size_for(32, 16), V_DCT, offset)
        .unwrap();

    assert_eq!(map.values[map.index(0, 0).unwrap()], V_DCT);
    assert_eq!(map.values[map.index(0, 4).unwrap()], V_DCT);
    assert_eq!(map.values[map.index(0, 7).unwrap()], DCT_DCT);
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
fn inter_residual_parse_scratch_reuses_tile_local_storage() {
    let offset = ByteOffset::new(0);
    let mut scratch = InterResidualParseScratch::default();
    scratch.luma_tx_types.reset(0, 0, 8, 8, offset).unwrap();
    scratch.luma_tx_types.values[0] = V_DCT;
    scratch.chroma_reads.reserve(4);
    scratch.chroma_reads.push(InterChromaURead {
        unit: InterChromaUnit {
            x4: 1,
            y4: 2,
            tx_fills_block: false,
            chroma_inter_tx_type: DCT_DCT,
        },
        block_index: 0,
        uses_cctx: false,
        u_nonzero: true,
    });
    let luma_capacity = scratch.luma_tx_types.values.capacity();
    let luma_pointer = scratch.luma_tx_types.values.as_ptr();
    let chroma_capacity = scratch.chroma_reads.capacity();
    let chroma_pointer = scratch.chroma_reads.as_ptr();

    scratch.luma_tx_types.reset(4, 8, 4, 4, offset).unwrap();
    scratch.chroma_reads.clear();

    assert_eq!(scratch.luma_tx_types.values, vec![DCT_DCT; 16]);
    assert_eq!(scratch.luma_tx_types.values.capacity(), luma_capacity);
    assert!(core::ptr::eq(
        scratch.luma_tx_types.values.as_ptr(),
        luma_pointer
    ));
    assert!(scratch.chroma_reads.is_empty());
    assert_eq!(scratch.chroma_reads.capacity(), chroma_capacity);
    assert!(core::ptr::eq(scratch.chroma_reads.as_ptr(), chroma_pointer));
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
            InterResidualLumaTxSizeMode::Inter,
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
