// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.4.1 sequence-header layer-dependency maps: the embedded-layer
//! (`MLayerDependencyMap` + its derived `MLayerPresenceMap` closure) and temporal-layer
//! (`TLayerDependencyMap`) dependency models, plus the `dependency_maps()` parser region.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::types::{EmbeddedLayerId, TemporalLayerId};

/// `MAX_NUM_TLAYERS` used by sequence-header dependency maps (AV2 § 5.4.1).
pub const MAX_NUM_TLAYERS: usize = 4;
/// `MAX_NUM_MLAYERS` used by sequence-header dependency maps (AV2 § 5.4.1).
pub const MAX_NUM_MLAYERS: usize = 8;

/// Derived `MLayerDependencyMap[currLayer][refLayer]` (AV2 § 5.4.1): `true` when
/// embedded layer `currLayer` may depend on embedded layer `refLayer`
/// (`mlayer_dependency_map` "specifies the embedded layer dependencies", § 6.4.1).
///
/// The § 5.4.1 default fill is the lower-triangular pattern clipped to
/// `max_mlayer_id` (`refLayer <= currLayer && currLayer <= max_mlayer_id`). When
/// `mlayer_dependency_present_flag` is set, signaled bits replace rows
/// `1..=max_mlayer_id` (the diagonal is itself signaled and may be 0); row 0 keeps
/// the default. The mirror also spells this variable `MlayerDependencyMap` in
/// § 6.8.9 / § 6.10.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MLayerDependencyMap([[bool; MAX_NUM_MLAYERS]; MAX_NUM_MLAYERS]);

impl MLayerDependencyMap {
    /// Builds the § 5.4.1 default fill:
    /// `MLayerDependencyMap[currLayer][refLayer] = refLayer <= currLayer &&
    /// currLayer <= max_mlayer_id` over all `MAX_NUM_MLAYERS x MAX_NUM_MLAYERS`
    /// entries.
    #[must_use]
    pub fn default_for(max_mlayer_id: EmbeddedLayerId) -> Self {
        let max_mlayer = usize::from(max_mlayer_id.get());
        let mut map = [[false; MAX_NUM_MLAYERS]; MAX_NUM_MLAYERS];
        for (curr_layer, row) in map.iter_mut().enumerate() {
            for (ref_layer, entry) in row.iter_mut().enumerate() {
                *entry = ref_layer <= curr_layer && curr_layer <= max_mlayer;
            }
        }
        Self(map)
    }

    /// Returns `MLayerDependencyMap[curr][reference]`. Ids outside the 3-bit
    /// `obu_mlayer_id` range read as `false`.
    #[must_use]
    pub fn depends_on(&self, curr: EmbeddedLayerId, reference: EmbeddedLayerId) -> bool {
        self.0
            .get(usize::from(curr.get()))
            .and_then(|row| row.get(usize::from(reference.get())))
            .copied()
            .unwrap_or(false)
    }

    /// Stores a signaled `mlayer_dependency_map` bit; out-of-range indices (never
    /// produced by the parser, whose loop bounds come from the 3-bit
    /// `max_mlayer_id`) are ignored rather than panicking.
    ///
    /// `pub(super)` so the relocated `sequence::tests` module can drive presence-map
    /// derivations directly, as it did when this type lived in `sequence` itself.
    pub(super) fn set(&mut self, curr_layer: u8, ref_layer: u8, value: bool) {
        if let Some(entry) = self
            .0
            .get_mut(usize::from(curr_layer))
            .and_then(|row| row.get_mut(usize::from(ref_layer)))
        {
            *entry = value;
        }
    }

    /// Derives `MLayerPresenceMap` (AV2 § 5.4.1, mirror
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md` :583-601): the reflexive-transitive
    /// closure of this `MLayerDependencyMap`, computed verbatim per the spec double loop.
    /// `MLayerPresenceMap[mlayerId][refMlayer]` is `1` when embedded layer `refMlayer` is
    /// (transitively) present whenever `mlayerId` is decoded — i.e. `mlayerId == refMlayer`,
    /// or `mlayerId` depends on `refMlayer`, or `mlayerId` depends on some layer that
    /// (transitively) requires `refMlayer`.
    #[must_use]
    pub fn presence_map(&self) -> MLayerPresenceMap {
        let mut presence = [[false; MAX_NUM_MLAYERS]; MAX_NUM_MLAYERS];
        for mlayer_id in 0..MAX_NUM_MLAYERS {
            for ref_mlayer in 0..MAX_NUM_MLAYERS {
                // presence[mlayer_id][ref_mlayer] starts `false` (zero-init above).
                if mlayer_id == ref_mlayer || self.0[mlayer_id][ref_mlayer] {
                    presence[mlayer_id][ref_mlayer] = true;
                    // Fold in everything refMlayer (transitively) requires over `dep < refMlayer`.
                    // Rows with refMlayer < mlayer_id are already fully computed; snapshotting
                    // the refMlayer row (a `Copy` `[bool; N]`) avoids aliasing the array on the
                    // read and the write (and makes the refMlayer == mlayer_id case a no-op `x|=x`).
                    let inherited_row = presence[ref_mlayer];
                    for (curr, inherited) in presence[mlayer_id]
                        .iter_mut()
                        .zip(inherited_row.iter())
                        .take(ref_mlayer)
                    {
                        *curr |= *inherited;
                    }
                }
            }
        }
        MLayerPresenceMap(presence)
    }
}

/// Derived `MLayerPresenceMap[mlayerId][refMlayer]` (AV2 § 5.4.1): `true` when embedded
/// layer `refMlayer` is present (transitively required) whenever embedded layer `mlayerId`
/// is decoded. It is the reflexive-transitive closure of [`MLayerDependencyMap`] and is
/// consumed by the § 5.18.2 `reset_qm()` SWITCH/RAS arm
/// (`MLayerPresenceMap[QmMLayerId[level]][obu_mlayer_id]`, mirror :5352).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MLayerPresenceMap([[bool; MAX_NUM_MLAYERS]; MAX_NUM_MLAYERS]);

impl MLayerPresenceMap {
    /// Returns `MLayerPresenceMap[mlayer][reference]`. Ids outside the 3-bit `obu_mlayer_id`
    /// range read as `false` (no panic).
    #[must_use]
    pub fn is_present(&self, mlayer: EmbeddedLayerId, reference: EmbeddedLayerId) -> bool {
        self.0
            .get(usize::from(mlayer.get()))
            .and_then(|row| row.get(usize::from(reference.get())))
            .copied()
            .unwrap_or(false)
    }
}

/// Derived `TLayerDependencyMap[mLayer][currTLayer][refTLayer]` (AV2 § 5.4.1):
/// `true` when, within embedded layer `mLayer`, temporal layer `currTLayer` may
/// depend on temporal layer `refTLayer` (`tlayer_dependency_map` "specifies the
/// temporal layer dependencies", § 6.4.1).
///
/// The § 5.4.1 default fill is `refTLayer <= currTLayer && currTLayer <=
/// max_tlayer_id && mLayer <= max_mlayer_id`. When
/// `tlayer_dependency_present_flag` is set, signaled bits replace rows
/// `currTLayer 1..=max_tlayer_id`; with `multi_tlayer_dependency_map_present_flag
/// == 0` the bits are signaled only for embedded layer 0 and replicated to
/// embedded layers `1..=max_mlayer_id` (§ 6.4.1: "the same values are used for
/// all embedded layers"). The mirror also spells this variable
/// `TlayerDependencyMap` in § 6.8.9 / § 6.10.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TLayerDependencyMap([[[bool; MAX_NUM_TLAYERS]; MAX_NUM_TLAYERS]; MAX_NUM_MLAYERS]);

impl TLayerDependencyMap {
    /// Builds the § 5.4.1 default fill:
    /// `TLayerDependencyMap[mLayer][currTLayer][refTLayer] = refTLayer <=
    /// currTLayer && currTLayer <= max_tlayer_id && mLayer <= max_mlayer_id` over
    /// all `MAX_NUM_MLAYERS x MAX_NUM_TLAYERS x MAX_NUM_TLAYERS` entries.
    #[must_use]
    pub fn default_for(max_tlayer_id: TemporalLayerId, max_mlayer_id: EmbeddedLayerId) -> Self {
        let max_tlayer = usize::from(max_tlayer_id.get());
        let max_mlayer = usize::from(max_mlayer_id.get());
        let mut map = [[[false; MAX_NUM_TLAYERS]; MAX_NUM_TLAYERS]; MAX_NUM_MLAYERS];
        for (m_layer, plane) in map.iter_mut().enumerate() {
            for (curr_tlayer, row) in plane.iter_mut().enumerate() {
                for (ref_tlayer, entry) in row.iter_mut().enumerate() {
                    *entry = ref_tlayer <= curr_tlayer
                        && curr_tlayer <= max_tlayer
                        && m_layer <= max_mlayer;
                }
            }
        }
        Self(map)
    }

    /// Returns `TLayerDependencyMap[mlayer][curr][reference]`. Ids outside the
    /// 3-bit `obu_mlayer_id` / 2-bit `obu_tlayer_id` ranges read as `false`.
    #[must_use]
    pub fn depends_on(
        &self,
        mlayer: EmbeddedLayerId,
        curr: TemporalLayerId,
        reference: TemporalLayerId,
    ) -> bool {
        self.0
            .get(usize::from(mlayer.get()))
            .and_then(|plane| plane.get(usize::from(curr.get())))
            .and_then(|row| row.get(usize::from(reference.get())))
            .copied()
            .unwrap_or(false)
    }

    /// Stores a signaled (or row-0-replicated) `tlayer_dependency_map` bit;
    /// out-of-range indices (never produced by the parser, whose loop bounds come
    /// from the 2-bit `max_tlayer_id` and 3-bit `max_mlayer_id`) are ignored
    /// rather than panicking.
    fn set(&mut self, m_layer: u8, curr_tlayer: u8, ref_tlayer: u8, value: bool) {
        if let Some(entry) = self
            .0
            .get_mut(usize::from(m_layer))
            .and_then(|plane| plane.get_mut(usize::from(curr_tlayer)))
            .and_then(|row| row.get_mut(usize::from(ref_tlayer)))
        {
            *entry = value;
        }
    }
}

/// Parsed § 5.4.1 dependency-map flags plus the two derived maps.
pub(super) struct DependencyMaps {
    /// `mlayer_dependency_present_flag`, inferred `0` when `max_mlayer_id == 0`.
    pub(super) mlayer_dependency_present_flag: bool,
    /// `tlayer_dependency_present_flag`, inferred `0` when `max_tlayer_id == 0`.
    pub(super) tlayer_dependency_present_flag: bool,
    /// `multi_tlayer_dependency_map_present_flag`, inferred `0` when not read.
    pub(super) multi_tlayer_dependency_map_present_flag: bool,
    /// Derived `MLayerDependencyMap`.
    pub(super) mlayer_dependency_map: MLayerDependencyMap,
    /// Derived `TLayerDependencyMap`.
    pub(super) tlayer_dependency_map: TLayerDependencyMap,
}

/// Parses the § 5.4.1 dependency-map region: both maps receive their
/// unconditional default fill first, then the signaled override bits (when
/// present) replace entries following the spec's loop structure exactly.
pub(super) fn parse_dependency_maps(
    reader: &mut BitReader<'_>,
    max_tlayer_id: TemporalLayerId,
    max_mlayer_id: EmbeddedLayerId,
) -> Result<DependencyMaps> {
    let mut mlayer_dependency_map = MLayerDependencyMap::default_for(max_mlayer_id);
    let mut tlayer_dependency_map = TLayerDependencyMap::default_for(max_tlayer_id, max_mlayer_id);

    let mut mlayer_dependency_present_flag = false;
    if max_mlayer_id.get() > 0 {
        mlayer_dependency_present_flag = reader.read_flag()?;
        if mlayer_dependency_present_flag {
            for curr_layer in 1..=max_mlayer_id.get() {
                // AV2 § 5.4.1: refLayer iterates from currLayer down to 0, so the
                // diagonal bit is signaled first (and may be 0).
                for ref_layer in (0..=curr_layer).rev() {
                    let bit = reader.read_flag()?;
                    mlayer_dependency_map.set(curr_layer, ref_layer, bit);
                }
            }
        }
    }

    let mut tlayer_dependency_present_flag = false;
    let mut multi_tlayer_dependency_map_present_flag = false;
    if max_tlayer_id.get() > 0 {
        tlayer_dependency_present_flag = reader.read_flag()?;
        if tlayer_dependency_present_flag {
            multi_tlayer_dependency_map_present_flag = if max_mlayer_id.get() > 0 {
                reader.read_flag()?
            } else {
                false
            };
            for m_layer in 0..=max_mlayer_id.get() {
                for curr_tlayer in 1..=max_tlayer_id.get() {
                    for ref_tlayer in (0..=curr_tlayer).rev() {
                        if multi_tlayer_dependency_map_present_flag || m_layer == 0 {
                            let bit = reader.read_flag()?;
                            tlayer_dependency_map.set(m_layer, curr_tlayer, ref_tlayer, bit);
                        } else {
                            // AV2 § 5.4.1: with the multi flag clear, embedded
                            // layers 1..=max_mlayer_id copy embedded layer 0's
                            // signaled values (not the default fill).
                            let bit = tlayer_dependency_map.depends_on(
                                EmbeddedLayerId::from_bits(0),
                                TemporalLayerId::from_bits(curr_tlayer),
                                TemporalLayerId::from_bits(ref_tlayer),
                            );
                            tlayer_dependency_map.set(m_layer, curr_tlayer, ref_tlayer, bit);
                        }
                    }
                }
            }
        }
    }

    Ok(DependencyMaps {
        mlayer_dependency_present_flag,
        tlayer_dependency_present_flag,
        multi_tlayer_dependency_map_present_flag,
        mlayer_dependency_map,
        tlayer_dependency_map,
    })
}
