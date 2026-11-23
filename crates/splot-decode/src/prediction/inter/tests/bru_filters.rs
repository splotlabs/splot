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
