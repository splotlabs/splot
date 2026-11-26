// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Validation and installation of an initial external HLS snapshot.

use splot_core::annexb::AnnexBObuCursor;
use splot_core::obu::{ParsedObu, PayloadStatus};

use super::*;
use crate::checks::default_checks;
use crate::options::{ExternalHlsError, ExternalHlsObjects, ExternalSequenceHeader};

pub(crate) fn parse_external_hls(bytes: &[u8]) -> Result<ExternalHlsObjects, ExternalHlsError> {
    if bytes.is_empty() {
        return Err(ExternalHlsError::Empty);
    }
    let mut sequences = BTreeMap::new();
    let mut operating_points = BTreeMap::new();
    let mut cursor = AnnexBObuCursor::new(bytes, ByteOffset::new(0));
    let context = ValidatorContext::default();
    while let Some(obu) = cursor.next_obu()? {
        let offset = obu.offset;
        if !matches!(
            obu.header.obu_type,
            ObuType::SequenceHeader | ObuType::OperatingPointSet
        ) {
            return Err(ExternalHlsError::Unsupported {
                offset,
                reason: "Supply only sequence headers and operating point sets.",
            });
        }
        let mut report = ValidationReport::new();
        for check in default_checks() {
            check.run(&obu, &mut report);
        }
        let xlayer = obu.header.extended_layer_id;
        match obu.payload_status()? {
            PayloadStatus::Parsed(ParsedObu::SequenceHeader(sequence)) => {
                if sequence.general.seq_lcr_id.get() != 0 {
                    return Err(ExternalHlsError::Unsupported {
                        offset,
                        reason: "The sequence header requires a layer configuration record.",
                    });
                }
                check_sequence_value_space(&sequence.general, offset, &mut report);
                check_snapshot_report(report)?;
                let key = (xlayer, sequence.general.seq_header_id);
                let definition = ExternalSequenceHeader {
                    header: sequence,
                    fingerprint: payload_fingerprint(obu.payload),
                };
                if sequences.insert(key, definition).is_some() {
                    return Err(ExternalHlsError::Duplicate { offset });
                }
            }
            PayloadStatus::Parsed(ParsedObu::OperatingPointSet(ops)) => {
                if ops.reset_flag || ops.ops_cnt == 0 {
                    return Err(ExternalHlsError::Unsupported {
                        offset,
                        reason: "Supply a definition without an operating point set reset.",
                    });
                }
                if ops.payloads.iter().any(|payload| {
                    payload
                        .xlayer_entries
                        .iter()
                        .any(|entry| matches!(entry.mlayer, OpsMlayerSource::Inherited { .. }))
                }) {
                    return Err(ExternalHlsError::Unsupported {
                        offset,
                        reason: "Supply operating points without inherited layer configuration.",
                    });
                }
                context.check_operating_point_set_semantics(&obu, &ops, &mut report);
                check_ops_level_tier_value_space(&obu, &ops, &mut report);
                check_snapshot_report(report)?;
                if operating_points.insert((xlayer, ops.ops_id), ops).is_some() {
                    return Err(ExternalHlsError::Duplicate { offset });
                }
            }
            _ => {
                return Err(ExternalHlsError::Unsupported {
                    offset,
                    reason: "Splot cannot parse the full OBU payload.",
                });
            }
        }
    }
    Ok(ExternalHlsObjects {
        sequences,
        operating_points,
    })
}

fn check_snapshot_report(report: ValidationReport) -> Result<(), ExternalHlsError> {
    if let Some(error) = report
        .diagnostics
        .into_iter()
        .find(|d| d.severity == crate::Severity::Error)
    {
        return Err(ExternalHlsError::InvalidObject(Box::new(error)));
    }
    Ok(())
}

impl ValidatorContext {
    pub(crate) fn with_options(options: &ValidationOptions) -> Self {
        let mut context = Self::default();
        let ExternalHlsMode::Objects(objects) = &options.external_hls else {
            return context;
        };

        for (&(xlayer, id), header) in &objects.sequences {
            context.hls.layer_mut(xlayer).sequences[usize::from(id.get())] =
                Some(Box::new(StoredSequenceHeader {
                    header: header.header.as_ref().clone(),
                    offset: None,
                    fingerprint: header.fingerprint,
                }));
        }
        for ops in objects.operating_points.values() {
            context.ops.apply(
                OperatingPointSetRecord {
                    xlayer_id: ops.xlayer_id,
                    ops_id: ops.ops_id,
                    ops_cnt: ops.ops_cnt,
                    offset: None,
                    explicit_entries: ops_explicit_entries(ops),
                },
                false,
            );
        }
        context
    }
}
