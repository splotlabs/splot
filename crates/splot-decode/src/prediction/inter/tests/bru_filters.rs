// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::header_state::repack_frame_obu;
use super::*;
use splot_core::write::{BitWriter, write_annexb_obu};

#[test]
fn bru_gdf_vectors_exercise_partial_bru_per_block_gdf_lr_and_reference_consumers() {
    use crate::bitstream::tile_payload::BRU_ACTIVE;

    let fixtures: [(&str, &[u8], bool); 4] = [
        (
            "syn-bru-gdf-block-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-gdf-block-8bit.ivf"
            ),
            false,
        ),
        (
            "syn-bru-gdf-block-10bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-gdf-block-10bit.ivf"
            ),
            false,
        ),
        (
            "syn-bru-gdf-lr-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-gdf-lr-8bit.ivf"
            ),
            true,
        ),
        (
            "syn-bru-gdf-multitile-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-gdf-multitile-8bit.ivf"
            ),
            false,
        ),
    ];

    for (name, bytes, expect_lr) in fixtures {
        let (headers, audits) =
            crate::pipeline::collect_bru_lr_audit(bytes, &DecodeOptions::default())
                .unwrap_or_else(|err| panic!("{name}: BRU+GDF audit decode failed: {err}"));
        assert_eq!(headers.len(), audits.len(), "{name}: header/audit mismatch");

        let matching: Vec<_> = audits
            .iter()
            .enumerate()
            .filter(|(_, audit)| {
                let active_sbs = audit
                    .superblock_modes
                    .iter()
                    .filter(|(_, _, mode)| *mode == Some(BRU_ACTIVE))
                    .count();
                let non_active_sbs = audit
                    .superblock_modes
                    .iter()
                    .filter(|(_, _, mode)| mode.is_some() && *mode != Some(BRU_ACTIVE))
                    .count();
                let lr_matches = if expect_lr {
                    audit.lr_uses_lr && !audit.post_bru_blocks.is_empty()
                } else {
                    !audit.lr_uses_lr
                };
                audit.use_bru
                    && !audit.bru_inactive
                    && active_sbs > 0
                    && non_active_sbs > 0
                    && audit.gdf_frame_enable
                    && audit.gdf_per_block
                    && audit.gdf_enabled_units > 0
                    && audit.gdf_enabled_units < audit.gdf_total_units
                    && lr_matches
            })
            .map(|(index, _)| index)
            .collect();
        assert!(
            !matching.is_empty(),
            "{name}: expected at least one partial-BRU frame with active mixed per-block GDF (lr={expect_lr}); audits={audits:#?}"
        );

        let first_idx = matching[0];
        assert!(
            first_idx + 1 < headers.len(),
            "{name}: missing post-BRU consumer frame"
        );
        let refreshed_mask = headers[first_idx].refresh_frame_flags.unwrap_or(0);
        assert_ne!(
            refreshed_mask, 0,
            "{name} f{first_idx}: expected reference refresh"
        );
        let consumer_refs = &headers[first_idx + 1]
            .inter
            .as_ref()
            .expect("consumer inter control")
            .ref_frame_idx;
        assert!(
            consumer_refs
                .iter()
                .any(|&slot| (refreshed_mask & (1u32 << slot)) != 0),
            "{name} f{}: expected ref consumer to retain refreshed slot from f{first_idx}",
            first_idx + 1
        );
    }
}

#[test]
fn bru_gdf_corrupt_truncated_and_missing_ref_fail_closed() {
    for fixture in [
        include_bytes!(
            "../../../../../../tests/conformance/vectors/valid/syn-bru-gdf-block-8bit.ivf"
        )
        .as_slice(),
        include_bytes!(
            "../../../../../../tests/conformance/vectors/valid/syn-bru-gdf-block-10bit.ivf"
        )
        .as_slice(),
        include_bytes!("../../../../../../tests/conformance/vectors/valid/syn-bru-gdf-lr-8bit.ivf")
            .as_slice(),
    ] {
        let truncated =
            repack_frame_obu(fixture, 3, |payload| payload[..payload.len() / 2].to_vec());
        let err = crate::pipeline::decode_frames_from_plan(&truncated, &DecodeOptions::default())
            .err()
            .expect("truncated BRU+GDF tile payload must fail");
        assert!(
            matches!(err, DecodeError::MalformedSource { .. }),
            "expected MalformedSource for truncated BRU+GDF tile payload, got {err}"
        );

        let flipped = repack_frame_obu(fixture, 3, |payload| {
            let mut copy = payload.to_vec();
            let last = copy.len() - 1;
            copy[last] ^= 0xFF;
            if last > 2 {
                copy[last - 2] ^= 0x5A;
            }
            copy
        });
        assert!(
            crate::pipeline::decode_frames_from_plan(&flipped, &DecodeOptions::default()).is_err(),
            "corrupted BRU+GDF tile payload must fail closed without panicking"
        );

        let parsed = parse_ivf_fixture(fixture, "bru gdf missing ref");
        let mut header = parsed.header.expect("fixture has IVF header");
        header.frame_count = u32::try_from(parsed.frames.len() - 1).unwrap();
        let mut missing_ref = Vec::new();
        write_repacked_ivf_header(&mut missing_ref, &header);
        let mut first_seq_obus = Vec::new();
        for envelope in &parsed.frames[0].obus {
            if matches!(
                envelope.header.obu_type,
                splot_core::types::ObuType::TemporalDelimiter
                    | splot_core::types::ObuType::SequenceHeader
            ) {
                first_seq_obus.push(envelope);
            }
        }
        for (idx, frame) in parsed.frames.iter().enumerate().skip(1) {
            let mut writer = BitWriter::new();
            if idx == 1 {
                for seq in &first_seq_obus {
                    write_annexb_obu(&mut writer, &seq.header, seq.payload).unwrap();
                }
            }
            for envelope in &frame.obus {
                if idx == 1
                    && envelope.header.obu_type == splot_core::types::ObuType::TemporalDelimiter
                {
                    continue;
                }
                write_annexb_obu(&mut writer, &envelope.header, envelope.payload).unwrap();
            }
            write_repacked_ivf_frame(&mut missing_ref, frame.frame.pts, &writer.into_bytes());
        }
        assert!(
            crate::pipeline::decode_frames_from_plan(&missing_ref, &DecodeOptions::default())
                .is_err(),
            "missing reference key frame for BRU+GDF stream must fail closed"
        );
    }
}

#[test]
fn bru_local_intrabc_vectors_exercise_mixed_bru_modes_same_frame_intrabc_lr_gdf_and_refs() {
    use crate::bitstream::tile_payload::{BRU_ACTIVE, BRU_INACTIVE};
    use crate::{DecodeContext, DecodeRuntimeConfig, ThreadCount};

    const BRU_SUPPORT: u8 = 1;
    let fixtures: [(&str, &[u8], bool, bool); 5] = [
        (
            "syn-bru-local-intrabc-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-8bit.ivf"
            ),
            false,
            false,
        ),
        (
            "syn-bru-local-intrabc-10bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-10bit.ivf"
            ),
            false,
            false,
        ),
        (
            "syn-bru-local-intrabc-multitile-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-multitile-8bit.ivf"
            ),
            false,
            false,
        ),
        (
            "syn-bru-local-intrabc-lr-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-lr-8bit.ivf"
            ),
            true,
            false,
        ),
        (
            "syn-bru-local-intrabc-gdf-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-gdf-8bit.ivf"
            ),
            false,
            true,
        ),
    ];

    for (name, bytes, expect_lr, expect_gdf) in fixtures {
        let (headers, audits) =
            crate::pipeline::collect_bru_lr_audit(bytes, &DecodeOptions::default()).unwrap_or_else(
                |err| panic!("{name}: BRU+local IntraBC audit decode failed: {err}"),
            );
        assert_eq!(headers.len(), audits.len(), "{name}: header/audit mismatch");

        let matching: Vec<_> = audits
            .iter()
            .enumerate()
            .filter(|(_, audit)| {
                let inactive_sbs = audit
                    .superblock_modes
                    .iter()
                    .filter(|(_, _, mode)| *mode == Some(BRU_INACTIVE))
                    .count();
                let support_sbs = audit
                    .superblock_modes
                    .iter()
                    .filter(|(_, _, mode)| *mode == Some(BRU_SUPPORT))
                    .count();
                let active_sbs = audit
                    .superblock_modes
                    .iter()
                    .filter(|(_, _, mode)| *mode == Some(BRU_ACTIVE))
                    .count();
                let lr_matches = !expect_lr || audit.lr_uses_lr;
                let gdf_matches = !expect_gdf
                    || (audit.gdf_frame_enable
                        && audit.gdf_per_block
                        && audit.gdf_enabled_units > 0
                        && audit.gdf_enabled_units < audit.gdf_total_units);
                audit.use_bru
                    && !audit.bru_inactive
                    && audit.allow_intrabc
                    && audit.allow_local_intrabc
                    && !audit.allow_global_intrabc
                    && audit.intrabc_block_count > 0
                    && inactive_sbs > 0
                    && support_sbs > 0
                    && active_sbs > 0
                    && lr_matches
                    && gdf_matches
            })
            .map(|(index, _)| index)
            .collect();
        assert!(
            !matching.is_empty(),
            "{name}: expected partial-BRU frame with mixed BRU modes and local IntraBC (lr={expect_lr}, gdf={expect_gdf}); audits={audits:#?}"
        );

        let first_idx = matching[0];
        assert!(
            first_idx + 1 < headers.len(),
            "{name}: missing post-BRU consumer frame"
        );
        let refreshed_mask = headers[first_idx].refresh_frame_flags.unwrap_or(0);
        assert_ne!(
            refreshed_mask, 0,
            "{name} f{first_idx}: expected reference refresh"
        );
        let consumer_refs = &headers[first_idx + 1]
            .inter
            .as_ref()
            .expect("consumer inter control")
            .ref_frame_idx;
        assert!(
            consumer_refs
                .iter()
                .any(|&slot| (refreshed_mask & (1u32 << slot)) != 0),
            "{name} f{}: expected ref consumer to retain refreshed slot from f{first_idx}",
            first_idx + 1
        );

        let mut expected_raw = Vec::new();
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .unwrap()
            .decode_raw_bytes(bytes, DecodeOptions::default(), &mut expected_raw)
            .unwrap_or_else(|err| panic!("{name}: single-thread raw decode failed: {err}"));
        for threads in [3usize, 12] {
            let mut raw = Vec::new();
            DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(threads)))
                .unwrap()
                .decode_raw_bytes(bytes, DecodeOptions::default(), &mut raw)
                .unwrap_or_else(|err| panic!("{name}: {threads}-thread raw decode failed: {err}"));
            assert_eq!(
                raw, expected_raw,
                "{name}: {threads}-thread parity mismatch"
            );
        }
    }
}

#[test]
fn bru_local_intrabc_invalid_vectors_and_truncated_streams_fail_closed() {
    use crate::bitstream::tile_payload::{BlockSize, DecodeBlockFrontier};
    use crate::filters::wienerns_lr::intrabc_mv_valid::{
        TileLocalIbcState, check_intrabc_block_vector,
    };
    use crate::filters::wienerns_lr::intrabc_records::IntrabcInfo;
    use crate::prediction::inter::MotionVector;

    const MULTITILE_FIXTURE: &[u8] = include_bytes!(
        "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-multitile-8bit.ivf"
    );

    for (name, fixture, is_multitile) in [
        (
            "syn-bru-local-intrabc-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-8bit.ivf"
            )
            .as_slice(),
            false,
        ),
        (
            "syn-bru-local-intrabc-10bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-10bit.ivf"
            )
            .as_slice(),
            false,
        ),
        (
            "syn-bru-local-intrabc-multitile-8bit",
            MULTITILE_FIXTURE,
            true,
        ),
        (
            "syn-bru-local-intrabc-lr-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-lr-8bit.ivf"
            )
            .as_slice(),
            false,
        ),
        (
            "syn-bru-local-intrabc-gdf-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-local-intrabc-gdf-8bit.ivf"
            )
            .as_slice(),
            false,
        ),
    ] {
        let truncated = repack_frame_obu(fixture, 3, |payload| {
            payload[..payload.len().saturating_sub(8)].to_vec()
        });
        let err = crate::pipeline::decode_frames_from_plan(&truncated, &DecodeOptions::default())
            .err()
            .unwrap_or_else(|| panic!("{name}: truncated BRU+local IntraBC tile payload must fail"));
        if is_multitile {
            assert!(
                matches!(
                    err,
                    DecodeError::MalformedSource { .. } | DecodeError::UnsupportedFeature { .. }
                ),
                "{name}: expected fail-closed truncation error for multi-tile BRU+local IntraBC payload, got {err}"
            );
        } else {
            assert!(
                matches!(err, DecodeError::MalformedSource { .. }),
                "{name}: expected MalformedSource for truncated BRU+local IntraBC payload, got {err}"
            );
        }

        let flipped = repack_frame_obu(fixture, 3, |payload| {
            let mut copy = payload.to_vec();
            let last = copy.len() - 1;
            copy[last] ^= 0xFF;
            if last > 2 {
                copy[last - 2] ^= 0x5A;
            }
            copy
        });
        assert!(
            crate::pipeline::decode_frames_from_plan(&flipped, &DecodeOptions::default()).is_err(),
            "corrupted BRU+local IntraBC tile payload must fail closed without panicking"
        );

        let parsed = parse_ivf_fixture(fixture, "bru local intrabc missing ref");
        let mut header = parsed.header.expect("fixture has IVF header");
        header.frame_count = u32::try_from(parsed.frames.len() - 1).unwrap();
        let mut missing_ref = Vec::new();
        write_repacked_ivf_header(&mut missing_ref, &header);
        let mut first_seq_obus = Vec::new();
        for envelope in &parsed.frames[0].obus {
            if matches!(
                envelope.header.obu_type,
                splot_core::types::ObuType::TemporalDelimiter
                    | splot_core::types::ObuType::SequenceHeader
            ) {
                first_seq_obus.push(envelope);
            }
        }
        for (idx, frame) in parsed.frames.iter().enumerate().skip(1) {
            let mut writer = BitWriter::new();
            if idx == 1 {
                for seq in &first_seq_obus {
                    write_annexb_obu(&mut writer, &seq.header, seq.payload).unwrap();
                }
            }
            for envelope in &frame.obus {
                if idx == 1
                    && envelope.header.obu_type == splot_core::types::ObuType::TemporalDelimiter
                {
                    continue;
                }
                write_annexb_obu(&mut writer, &envelope.header, envelope.payload).unwrap();
            }
            write_repacked_ivf_frame(&mut missing_ref, frame.frame.pts, &writer.into_bytes());
        }
        assert!(
            crate::pipeline::decode_frames_from_plan(&missing_ref, &DecodeOptions::default())
                .is_err(),
            "missing reference key frame for BRU+local IntraBC stream must fail closed"
        );
    }

    let (_, mut core) = fixture_sequence_and_key_core(MULTITILE_FIXTURE);
    core.frame_is_intra = Some(false);
    core.allow_intrabc = Some(true);
    core.intrabc = None;
    let tile_info = core.tile_info.as_ref().expect("multitile tile_info");
    assert!(tile_info.mi_col_starts.len() >= 3);
    let tile0_end = usize::try_from(tile_info.mi_col_starts[1]).unwrap();
    let tile1_end = usize::try_from(tile_info.mi_col_starts[2]).unwrap();

    let mut tile0 = TileLocalIbcState::new(0..tile0_end, 16, false).unwrap();
    tile0.begin_superblock(0, true);
    tile0.enter_leaf(0, 0, 16, 16, true);
    tile0.begin_superblock(0, false);
    tile0.enter_leaf(0, 16, 2, 2, false);

    let b8x8 = BlockSize::new(BLOCK_8X8).unwrap();
    let offset = ByteOffset::new(0x42);
    let check_vector = |local: Option<&TileLocalIbcState>, mi: (usize, usize), mv: (i32, i32)| {
        check_intrabc_block_vector(
            &core,
            &DecodeBlockFrontier::shared_leaf(mi.0, mi.1, b8x8),
            (2, 2),
            (16, false),
            IntrabcInfo {
                morph_pred: false,
                block_mv: MotionVector {
                    row: mv.0,
                    col: mv.1,
                },
            },
            local,
            offset,
        )
    };
    let assert_conformance_fail = |label: &str, result: Result<()>| match result {
        Err(DecodeError::MalformedSource { issue }) => {
            assert_eq!(issue.spec_section(), Some("6.19.7.12"), "{label}");
            assert_eq!(issue.offset(), Some(offset), "{label}");
        }
        other => panic!("{label}: expected 6.19.7.12 MalformedSource, got {other:?}"),
    };

    assert!(
        check_vector(Some(&tile0), (0, 18), (0, -2 * 4 * 8)).is_ok(),
        "coded intra-SB source must succeed"
    );
    assert_conformance_fail(
        "SB64 BRU_INACTIVE reset source must fail closed",
        check_vector(Some(&tile0), (0, 18), (0, -18 * 4 * 8)),
    );
    assert_conformance_fail(
        "uncoded future block in current superblock must fail closed",
        check_vector(Some(&tile0), (0, 18), (0, 2 * 4 * 8)),
    );
    assert_conformance_fail(
        "missing local state with allow_local_intrabc must fail closed",
        check_vector(None, (0, 18), (0, -2 * 4 * 8)),
    );

    let mut tile1 = TileLocalIbcState::new(tile0_end..tile1_end, 16, false).unwrap();
    tile1.begin_superblock(0, false);
    tile1.enter_leaf(0, tile0_end, 2, 2, false);
    assert_conformance_fail(
        "cross-tile source across column boundary must fail closed",
        check_vector(Some(&tile1), (0, tile0_end), (0, -4 * 8)),
    );
}
