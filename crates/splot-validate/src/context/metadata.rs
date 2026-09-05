// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Metadata dispatch plus HDR, scan-type, and timecode metadata checks.

use super::*;

const LAYER_GLOBAL: u8 = 1;
const LAYER_CURRENT: u8 = 2;
const LAYER_VALUES: u8 = 3;

/// Bitstream-derived HDR targeting (AV2 § 6.16.3, § 6.16.5–6.16.6;
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md`). Repeated content
/// must agree when association sets intersect, regardless of targeting encoding.
/// Unspecified/reserved targeting and LAYER_CURRENT on a global OBU have no
/// concrete association; skipping them conservatively avoids false positives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HdrAssociation {
    /// `LAYER_GLOBAL` on a `GLOBAL_XLAYER_ID` OBU: "The metadata applies to all
    /// layers" (§ 6.16.3) — every embedded layer of every extended layer.
    Universal,
    /// `LAYER_GLOBAL` on a concrete OBU: "layers with matching obu_xlayer_id
    /// only" (§ 6.16.3) — every embedded layer of that extended layer.
    XLayerWide(ExtendedLayerId),
    /// An explicit `(obu_xlayer_id, obu_mlayer_id)` pair set: `LAYER_CURRENT`
    /// (the carrying OBU's own pair) or `LAYER_VALUES` ("The metadata unit is
    /// intended for an extended layer x if bit x of muh_xlayer_map is equal to
    /// 1" and "... for an embedded layer m if bit m of muh_mlayer_map is equal
    /// to 1", § 6.16.3; map layout per § 5.17.3). Never empty.
    Pairs(Vec<(ExtendedLayerId, EmbeddedLayerId)>),
}

impl HdrAssociation {
    /// Returns `true` when the two association sets share at least one embedded
    /// layer — the condition under which § 6.16.5 / § 6.16.6 require the two
    /// units to "have the same content".
    pub(super) fn intersects(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Universal, _) | (_, Self::Universal) => true,
            (Self::XLayerWide(a), Self::XLayerWide(b)) => a == b,
            (Self::XLayerWide(x), Self::Pairs(pairs))
            | (Self::Pairs(pairs), Self::XLayerWide(x)) => {
                pairs.iter().any(|(pair_x, _)| pair_x == x)
            }
            (Self::Pairs(a), Self::Pairs(b)) => a.iter().any(|pair| b.contains(pair)),
        }
    }

    /// Returns `true` when the association includes any embedded layer of
    /// `xlayer`, i.e. the record belongs to that extended layer's
    /// coded-video-sequence scope (drives the § 7.3.6 CLK pruning; a `Universal`
    /// record touches every layer, mirroring the global-record pruning of the
    /// other CVS-scoped stores).
    pub(super) fn touches_xlayer(&self, xlayer: ExtendedLayerId) -> bool {
        match self {
            Self::Universal => true,
            Self::XLayerWide(x) => *x == xlayer,
            Self::Pairs(pairs) => pairs.iter().any(|(pair_x, _)| *pair_x == xlayer),
        }
    }

    /// The concrete extended layers this association enumerates (`Universal`
    /// applies to all layers and enumerates none).
    pub(super) fn concrete_xlayers(&self) -> Vec<ExtendedLayerId> {
        match self {
            Self::Universal => Vec::new(),
            Self::XLayerWide(x) => vec![*x],
            Self::Pairs(pairs) => pairs.iter().map(|(pair_x, _)| *pair_x).collect(),
        }
    }

    /// The concrete `(obu_xlayer_id, obu_mlayer_id)` embedded-layer pairs this
    /// association names exactly, or `None` when the association covers every
    /// embedded layer of an extended layer (`XLayerWide`) or all layers
    /// (`Universal`) and so names no single concrete first coded picture. Used by
    /// the § 6.16.5 / § 6.16.6 first-coded-picture check, which evaluates lateness
    /// independently for each explicitly named pair.
    pub(super) fn explicit_embedded_pairs(&self) -> Option<&[(ExtendedLayerId, EmbeddedLayerId)]> {
        match self {
            Self::Universal | Self::XLayerWide(_) => None,
            Self::Pairs(pairs) => Some(pairs),
        }
    }

    /// Whether this association contains an embedded-layer pair. Used for both
    /// CI pairing (§ 6.16.7 / Annex E.4.2) and per-pair HDR establishment
    /// (§ 6.16.5–6.16.6); an already-established different pair does not suffice.
    pub(super) fn includes_embedded_pair(
        &self,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
    ) -> bool {
        match self {
            Self::Universal => true,
            Self::XLayerWide(x) => *x == xlayer,
            Self::Pairs(pairs) => pairs.contains(&(xlayer, mlayer)),
        }
    }
}

/// The single concrete extended layer scoping a comparison between two
/// intersecting [`HdrAssociation`]s, for [`CvsTracker::defer_or_emit`] tagging.
/// When the intersection spans several extended layers — or only the all-layers
/// `Universal` pair, which enumerates none — `GLOBAL_XLAYER_ID` tags it instead,
/// reusing the documented any-CLK-drops approximation of
/// [`CvsTracker::flush_completed_tu`] (sound: it only drops comparisons).
pub(super) fn hdr_intersection_scope(a: &HdrAssociation, b: &HdrAssociation) -> ExtendedLayerId {
    let xlayers: Vec<ExtendedLayerId> = match (a, b) {
        (HdrAssociation::Universal, HdrAssociation::Universal) => Vec::new(),
        (HdrAssociation::Universal, other) | (other, HdrAssociation::Universal) => {
            other.concrete_xlayers()
        }
        (HdrAssociation::XLayerWide(x), _) | (_, HdrAssociation::XLayerWide(x)) => vec![*x],
        (HdrAssociation::Pairs(a_pairs), HdrAssociation::Pairs(b_pairs)) => a_pairs
            .iter()
            .filter(|pair| b_pairs.contains(pair))
            .map(|(pair_x, _)| *pair_x)
            .collect(),
    };
    match xlayers.as_slice() {
        [first, rest @ ..] if rest.iter().all(|x| x == first) => *first,
        _ => GLOBAL_XLAYER_ID,
    }
}

/// Describes an embedded-layer association the two intersecting
/// [`HdrAssociation`]s share, naming a concrete `(obu_xlayer_id, obu_mlayer_id)`
/// pair whenever one is enumerable so a cross-mode § 6.16.5 / § 6.16.6 finding
/// is intelligible.
pub(super) fn describe_hdr_intersection(a: &HdrAssociation, b: &HdrAssociation) -> String {
    let common_pair = match (a, b) {
        (HdrAssociation::Pairs(a_pairs), HdrAssociation::Pairs(b_pairs)) => {
            a_pairs.iter().find(|pair| b_pairs.contains(pair)).copied()
        }
        (HdrAssociation::Pairs(pairs), HdrAssociation::XLayerWide(x))
        | (HdrAssociation::XLayerWide(x), HdrAssociation::Pairs(pairs)) => {
            pairs.iter().find(|(pair_x, _)| pair_x == x).copied()
        }
        (HdrAssociation::Pairs(pairs), HdrAssociation::Universal)
        | (HdrAssociation::Universal, HdrAssociation::Pairs(pairs)) => pairs.first().copied(),
        _ => None,
    };
    if let Some((xlayer, mlayer)) = common_pair {
        return format!(
            "embedded layer obu_xlayer_id {} / obu_mlayer_id {}",
            xlayer.get(),
            mlayer.get()
        );
    }
    match (a, b) {
        (HdrAssociation::XLayerWide(x), _) | (_, HdrAssociation::XLayerWide(x)) => {
            format!("every embedded layer of obu_xlayer_id {}", x.get())
        }
        _ => "all layers".to_owned(),
    }
}

/// Names a set of concrete `(obu_xlayer_id, obu_mlayer_id)` embedded-layer pairs for
/// a § 6.16.5 / § 6.16.6 first-coded-picture finding, so a unit late for a *subset*
/// of its targeted layers reports exactly which layers.
pub(super) fn describe_embedded_pairs(pairs: &[(ExtendedLayerId, EmbeddedLayerId)]) -> String {
    let names: Vec<String> = pairs
        .iter()
        .map(|(xlayer, mlayer)| {
            format!(
                "obu_xlayer_id {} / obu_mlayer_id {}",
                xlayer.get(),
                mlayer.get()
            )
        })
        .collect();
    format!("embedded layer(s) {}", names.join(", "))
}

/// One observed HDR CLL / MDCV unit's content within its coded-video-sequence
/// scope (AV2 § 6.16.5 / § 6.16.6), compared against every later unit of the
/// same metadata type whose [`HdrAssociation`] intersects this one.
#[derive(Debug)]
pub(super) struct HdrBaselineRecord {
    /// The unit's bitstream-derived embedded-layer association.
    pub(super) association: HdrAssociation,
    /// `true` for `metadata_hdr_mdcv()`, `false` for `metadata_hdr_cll()` —
    /// § 6.16.5 and § 6.16.6 state the rule identically but each binds only its
    /// own metadata type.
    pub(super) is_mdcv: bool,
    /// The parsed `metadata_hdr_cll()` / `metadata_hdr_mdcv()` payload, compared
    /// field-for-field against every later intersecting unit.
    pub(super) payload: MetadataPayload,
    /// Source byte offset of the OBU that produced this record.
    pub(super) offset: ByteOffset,
    /// Temporal unit ([`CvsTracker::tu_index`]) of this record's latest appearance,
    /// used by the exact § 7.3.6 CVS scoping (CLK pruning and deferral decisions).
    pub(super) tu_index: u64,
}

/// The parsed `muh_*` unit-header fields the stateful metadata observers consume
/// for one non-cancel metadata unit (AV2 § 5.17.2 / § 5.17.3 / § 6.16.3).
pub(super) struct MetadataUnitHeader<'a> {
    /// `muh_layer_idc` (short form: parsed from the 1-byte header; group form:
    /// per-unit).
    pub(super) layer_idc: u8,
    /// `muh_xlayer_map` (global group-form `LAYER_VALUES` only, § 5.17.3).
    pub(super) xlayer_map: Option<u32>,
    /// Every parsed `muh_mlayer_map` byte: one per set `muh_xlayer_map` bit when
    /// global, a single byte when local, empty for the short form (§ 5.17.3).
    pub(super) mlayer_maps: &'a [u8],
}

/// Derives the [`HdrAssociation`] of one non-cancel metadata unit from its
/// carrying OBU's layer ids and its `muh_*` layer targeting (AV2 § 6.16.3
/// per-`muh_layer_idc` modes; § 5.17.3 map layout). Returns `None` when the
/// association is not derivable from the bitstream (see [`HdrAssociation`]) or
/// when explicit `LAYER_VALUES` maps select no layer.
pub(super) fn derive_hdr_association(
    obu: &ObuEnvelope<'_>,
    header: &MetadataUnitHeader<'_>,
) -> Option<HdrAssociation> {
    let xlayer = obu.header.extended_layer_id;
    match header.layer_idc {
        LAYER_GLOBAL if xlayer.is_global() => Some(HdrAssociation::Universal),
        LAYER_GLOBAL => Some(HdrAssociation::XLayerWide(xlayer)),
        LAYER_CURRENT if !xlayer.is_global() => Some(HdrAssociation::Pairs(vec![(
            xlayer,
            obu.header.embedded_layer_id,
        )])),
        LAYER_VALUES => {
            let mut pairs = Vec::new();
            if xlayer.is_global() {
                let xlayer_map = header.xlayer_map?;
                let mut maps = header.mlayer_maps.iter();
                for x in 0..31u8 {
                    if xlayer_map & (1 << x) == 0 {
                        continue;
                    }
                    let &mlayer_map = maps.next()?;
                    push_mlayer_pairs(&mut pairs, ExtendedLayerId::from_bits(x), mlayer_map);
                }
            } else if let [single] = header.mlayer_maps {
                push_mlayer_pairs(&mut pairs, xlayer, *single);
            }
            (!pairs.is_empty()).then_some(HdrAssociation::Pairs(pairs))
        }
        _ => None,
    }
}

/// Appends `(xlayer, m)` for each set bit `m` of one 8-bit `muh_mlayer_map`
/// (AV2 § 6.16.3: "The metadata unit is intended for an embedded layer m if bit
/// m of muh_mlayer_map is equal to 1").
pub(super) fn push_mlayer_pairs(
    pairs: &mut Vec<(ExtendedLayerId, EmbeddedLayerId)>,
    xlayer: ExtendedLayerId,
    mlayer_map: u8,
) {
    for m in 0..8u8 {
        if mlayer_map >> m & 1 == 1 {
            pairs.push((xlayer, EmbeddedLayerId::from_bits(m)));
        }
    }
}

impl ValidatorContext {
    /// Observes a metadata OBU (short or group form), feeding the § 6.16.5 / § 6.16.6
    /// HDR repeat-content baselines plus the scan-type and timecode consistency
    /// checks. A parse failure is silent, matching
    /// `observe_content_interpretation` — the stateless `MetadataSyntax` check owns
    /// failure reporting.
    pub(super) fn observe_metadata(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        match obu.header.obu_type {
            ObuType::MetadataShort => {
                let Ok(short) = parse_metadata_short(&mut reader, obu.payload.len()) else {
                    return;
                };
                let Some(unit) = short.unit else {
                    return;
                };
                self.observe_metadata_unit(
                    obu,
                    &MetadataUnitHeader {
                        layer_idc: short.muh_layer_idc,
                        xlayer_map: None,
                        mlayer_maps: &[],
                    },
                    &unit,
                    report,
                );
            }
            ObuType::MetadataGroup => {
                let Ok(group) = parse_metadata_group(&mut reader, obu.header.extended_layer_id)
                else {
                    return;
                };
                for group_unit in group.units {
                    let (Some(layer_idc), Some(unit)) = (group_unit.muh_layer_idc, group_unit.unit)
                    else {
                        continue;
                    };
                    self.observe_metadata_unit(
                        obu,
                        &MetadataUnitHeader {
                            layer_idc,
                            xlayer_map: group_unit.muh_xlayer_map,
                            mlayer_maps: &group_unit.muh_mlayer_maps,
                        },
                        &unit,
                        report,
                    );
                }
            }
            _ => {}
        }
    }

    /// Runs the stateful checks for one parsed non-cancel metadata unit.
    pub(super) fn observe_metadata_unit(
        &mut self,
        obu: &ObuEnvelope<'_>,
        header: &MetadataUnitHeader<'_>,
        unit: &MetadataUnit,
        report: &mut ValidationReport,
    ) {
        self.check_hdr_repeat_content(obu, header, unit, report);
        if let MetadataPayload::ScanType(scan) = &unit.payload {
            self.check_scan_type_consistency(obu, *scan, report);
        }
        if let MetadataPayload::Timecode(timecode) = &unit.payload {
            let targeting = derive_hdr_association(obu, header);
            self.check_timecode_consistency(obu, timecode, targeting, report);
        }
    }

    /// Checks § 6.16.5–6.16.6 repeat content across intersecting layer associations.
    /// Cancellation does not erase the baseline; disjoint or underivable associations
    /// are not compared. Earlier-TU comparisons defer through CvsTracker so a later
    /// CLK can place them in different CVSs. First-picture placement is checked by
    /// Self::check_hdr_first_coded_picture.
    pub(super) fn check_hdr_repeat_content(
        &mut self,
        obu: &ObuEnvelope<'_>,
        header: &MetadataUnitHeader<'_>,
        unit: &MetadataUnit,
        report: &mut ValidationReport,
    ) {
        let is_mdcv = match unit.payload {
            MetadataPayload::HdrCll(_) => false,
            MetadataPayload::HdrMdcv(_) => true,
            _ => return,
        };
        let Some(association) = derive_hdr_association(obu, header) else {
            return;
        };
        let tu_index = self.cvs.tu_index;
        self.check_hdr_first_coded_picture(obu, &association, is_mdcv, report);
        for record in &self.hdr_baselines {
            if record.is_mdcv != is_mdcv
                || record.payload == unit.payload
                || !record.association.intersects(&association)
            {
                continue;
            }
            let (rule_id, spec_section, unit_name) = if is_mdcv {
                (
                    "metadata/hdr-mdcv-repeat-content-differs",
                    "6.16.6",
                    "metadata_hdr_mdcv",
                )
            } else {
                (
                    "metadata/hdr-cll-repeat-content-differs",
                    "6.16.5",
                    "metadata_hdr_cll",
                )
            };
            let diagnostic = Diagnostic::error(
                rule_id,
                format!(
                    "{unit_name} metadata associated with {} is repeated within the coded \
                     video sequence with different content (previous copy at byte {})",
                    describe_hdr_intersection(&record.association, &association),
                    record.offset
                ),
            )
            .with_spec_section(spec_section)
            .with_byte_offset(obu.offset);
            self.cvs.defer_or_emit(
                hdr_intersection_scope(&record.association, &association),
                record.tu_index,
                diagnostic,
                report,
            );
        }
        if let Some(record) = self
            .hdr_baselines
            .iter_mut()
            .find(|record| record.is_mdcv == is_mdcv && record.association == association)
        {
            record.payload = unit.payload.clone();
            record.offset = obu.offset;
            record.tu_index = tu_index;
        } else {
            self.hdr_baselines.push(HdrBaselineRecord {
                association,
                is_mdcv,
                payload: unit.payload.clone(),
                offset: obu.offset,
                tu_index,
            });
        }
    }

    /// Checks first-coded-picture placement independently for each explicitly
    /// named pair (§ 6.16.5–6.16.6, mirror 06-syntax-structures-semantics.md).
    /// A pair is late only if no prior same-type baseline established it and its
    /// first coded frame unit has ended. Suffix metadata inside that first unit
    /// is still on time. Universal/XLayerWide targeting and color inheritance
    /// remain conservatively unchecked because no exact first-picture pair is known.
    /// Same-TU first pictures emit eagerly; earlier-TU ones defer through CvsTracker
    /// because a later CLK can start a new CVS at the current temporal unit.
    pub(super) fn check_hdr_first_coded_picture(
        &mut self,
        obu: &ObuEnvelope<'_>,
        association: &HdrAssociation,
        is_mdcv: bool,
        report: &mut ValidationReport,
    ) {
        let Some(pairs) = association.explicit_embedded_pairs() else {
            return;
        };
        let current_tu = self.cvs.tu_index;
        let is_suffix_metadata = metadata_is_suffix(obu) == Some(true);
        let mut eager_late: Vec<(ExtendedLayerId, EmbeddedLayerId)> = Vec::new();
        let mut deferred_late: Vec<((ExtendedLayerId, EmbeddedLayerId), u64)> = Vec::new();
        for &pair in pairs {
            let already_established = self.hdr_baselines.iter().any(|record| {
                record.is_mdcv == is_mdcv
                    && record.association.includes_embedded_pair(pair.0, pair.1)
            });
            if already_established {
                continue;
            }
            let Some(&seen_tu) = self.embedded_layer_first_picture_seen.get(&pair) else {
                continue;
            };
            if seen_tu == current_tu {
                if is_suffix_metadata
                    && self
                        .frame_unit
                        .completed_units_for_embedded_layer(pair.0, pair.1)
                        == 0
                {
                    continue;
                }
                eager_late.push(pair);
            } else {
                deferred_late.push((pair, seen_tu));
            }
        }
        if eager_late.is_empty() && deferred_late.is_empty() {
            return;
        }
        let (rule_id, spec_section, unit_name) = if is_mdcv {
            (
                "metadata/hdr-mdcv-first-coded-picture",
                "6.16.6",
                "metadata_hdr_mdcv",
            )
        } else {
            (
                "metadata/hdr-cll-first-coded-picture",
                "6.16.5",
                "metadata_hdr_cll",
            )
        };
        let build = |late: &[(ExtendedLayerId, EmbeddedLayerId)]| {
            Diagnostic::error(
                rule_id,
                format!(
                    "{unit_name} metadata first establishes content for {} after that embedded \
                     layer's first coded picture of the coded video sequence; it shall be \
                     indicated at the first coded picture",
                    describe_embedded_pairs(late)
                ),
            )
            .with_spec_section(spec_section)
            .with_byte_offset(obu.offset)
        };
        if !eager_late.is_empty() {
            report.push(build(&eager_late));
        }
        for (pair, seen_tu) in deferred_late {
            self.cvs
                .defer_or_emit(pair.0, seen_tu, build(std::slice::from_ref(&pair)), report);
        }
    }
}
