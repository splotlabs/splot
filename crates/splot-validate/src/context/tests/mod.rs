// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for validator-context helper state.

use splot_core::bitio::BitReader;
use splot_core::headers::sequence::{MLayerDependencyMap, TLayerDependencyMap};
use splot_core::hls::parse_msdo;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, TemporalLayerId};
use splot_core::write::BitWriter;

use super::{
    CmvsState, CmvsTracker, MsdoObservation, MsdoObserver, ValidationReport,
    mlayer_closure_violation, tlayer_closure_violation,
};

/// Builds the synthetic `multistream_decoder_operation_obu()` payload bytes with the
/// given § 7.3.2 condition-2 key fields. `num_streams_minus_2` drives the
/// per-substream loop; the substream entries are filled with zeros (they are not
/// § 7.3.2 key fields). The bytes are parsed by [`observe_test_msdo`].
fn msdo_bytes(profile_idc: u8, level_idx: u8, tier: u8, num_streams_minus_2: u8) -> Vec<u8> {
    msdo_bytes_uneven(profile_idc, level_idx, tier, num_streams_minus_2, None)
}

/// Like [`msdo_bytes`] but lets the caller set `multistream_even_allocation_flag`
/// and the `multistream_large_picture_idc` carried when allocation is not even.
fn msdo_bytes_uneven(
    profile_idc: u8,
    level_idx: u8,
    tier: u8,
    num_streams_minus_2: u8,
    large_picture_idc: Option<u8>,
) -> Vec<u8> {
    let mut bits = BitWriter::new();
    assert!(bits.write_bits(u32::from(num_streams_minus_2), 3).is_ok());
    assert!(bits.write_bits(u32::from(profile_idc), 5).is_ok());
    assert!(bits.write_bits(u32::from(level_idx), 5).is_ok());
    assert!(bits.write_bits(u32::from(tier), 1).is_ok());
    match large_picture_idc {
        None => assert!(bits.write_bits(1, 1).is_ok()), // multistream_even_allocation_flag = 1
        Some(idc) => {
            assert!(bits.write_bits(0, 1).is_ok()); // multistream_even_allocation_flag = 0
            assert!(bits.write_bits(u32::from(idc), 3).is_ok()); // multistream_large_picture_idc
        }
    }
    for _ in 0..(u32::from(num_streams_minus_2) + 2) {
        assert!(bits.write_bits(0, 5).is_ok()); // sub_xlayer_id
        assert!(bits.write_bits(0, 5).is_ok()); // sub_stream_max_profile
        assert!(bits.write_bits(0, 5).is_ok()); // sub_stream_max_level
        assert!(bits.write_bits(0, 1).is_ok()); // sub_stream_max_tier
    }
    assert!(bits.write_bits(0, 1).is_ok()); // multistream_doh_constraint_flag
    bits.into_bytes()
}

/// Parses synthetic MSDO bytes and observes them, asserting fixture validity.
fn observe_test_msdo(observer: &mut MsdoObserver, bytes: &[u8]) -> MsdoObservation {
    let mut reader = BitReader::new(bytes, ByteOffset::new(0));
    let parsed = parse_msdo(&mut reader).ok();
    assert!(parsed.is_some(), "synthetic MSDO must parse");
    match parsed {
        Some(msdo) => observer.observe(&msdo),
        None => MsdoObservation::Unchanged,
    }
}

#[test]
fn msdo_observer_reports_first_then_unchanged() {
    let mut observer = MsdoObserver::default();
    assert_eq!(
        observe_test_msdo(&mut observer, &msdo_bytes(1, 2, 0, 1)),
        MsdoObservation::First
    );
    assert_eq!(
        observe_test_msdo(&mut observer, &msdo_bytes(1, 2, 0, 1)),
        MsdoObservation::Unchanged
    );
}

#[test]
fn msdo_observer_detects_each_condition_two_key_field_change() {
    let baseline = msdo_bytes(1, 2, 0, 1);
    let changes = [
        msdo_bytes(2, 2, 0, 1),                 // multistream_profile_idc
        msdo_bytes(1, 3, 0, 1),                 // multistream_level_idx
        msdo_bytes(1, 2, 1, 1),                 // multistream_tier
        msdo_bytes(1, 2, 0, 2),                 // num_streams_minus_2
        msdo_bytes_uneven(1, 2, 0, 1, Some(0)), // multistream_even_allocation_flag
    ];
    for changed in &changes {
        let mut observer = MsdoObserver::default();
        assert_eq!(
            observe_test_msdo(&mut observer, &baseline),
            MsdoObservation::First
        );
        assert_eq!(
            observe_test_msdo(&mut observer, changed),
            MsdoObservation::Changed,
            "expected a key-field change to be detected"
        );
    }
    let mut observer = MsdoObserver::default();
    assert_eq!(
        observe_test_msdo(&mut observer, &msdo_bytes_uneven(1, 2, 0, 1, Some(1))),
        MsdoObservation::First
    );
    assert_eq!(
        observe_test_msdo(&mut observer, &msdo_bytes_uneven(1, 2, 0, 1, Some(2))),
        MsdoObservation::Changed
    );
}

#[test]
fn msdo_observer_ignores_non_key_field_changes() {
    let mut observer = MsdoObserver::default();
    assert_eq!(
        observe_test_msdo(&mut observer, &msdo_bytes(0, 0, 0, 0)),
        MsdoObservation::First
    );
    assert_eq!(
        observe_test_msdo(&mut observer, &msdo_bytes(0, 0, 0, 0)),
        MsdoObservation::Unchanged
    );
}

/// Drives a [`CmvsTracker`] through one temporal unit with the given facts and
/// returns the resulting state. `clk` toggles the CLK-present fact; `msdo` records
/// an MSDO observation; `global_lcr` toggles a global-LCR-present fact.
fn cmvs_after_tu(
    tracker: &mut CmvsTracker,
    clk: bool,
    msdo_obs: Option<MsdoObservation>,
    global_lcr: bool,
) -> CmvsState {
    if clk {
        tracker.note_clk();
    }
    if let Some(observation) = msdo_obs {
        tracker.note_msdo(observation);
    }
    if global_lcr {
        tracker.note_global_lcr_present();
    }
    let mut report = ValidationReport::default();
    tracker.complete_temporal_unit(0, &mut report);
    tracker.state()
}

#[test]
fn cmvs_starts_outside() {
    let tracker = CmvsTracker::default();
    assert_eq!(tracker.state(), CmvsState::Outside);
}

#[test]
fn cmvs_begin_condition_1_clk_plus_msdo_enters_inside() {
    let mut tracker = CmvsTracker::default();
    let state = cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false);
    assert_eq!(state, CmvsState::Inside);
}

#[test]
fn cmvs_begin_condition_2_changed_msdo_keeps_inside() {
    let mut tracker = CmvsTracker::default();
    assert_eq!(
        cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
        CmvsState::Inside
    );
    assert_eq!(
        cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::Changed), false),
        CmvsState::Inside
    );
    assert_eq!(
        cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::Unchanged), false),
        CmvsState::Inside
    );
}

#[test]
fn cmvs_begin_condition_3_global_lcr_only_is_unknown() {
    let mut tracker = CmvsTracker::default();
    let state = cmvs_after_tu(&mut tracker, true, None, true);
    assert_eq!(state, CmvsState::Unknown);
}

#[test]
fn cmvs_end_condition_2_clk_without_msdo_exits_inside() {
    let mut tracker = CmvsTracker::default();
    assert_eq!(
        cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
        CmvsState::Inside
    );
    let state = cmvs_after_tu(&mut tracker, true, None, false);
    assert_eq!(state, CmvsState::Outside);
}

#[test]
fn cmvs_end_condition_2_with_global_lcr_is_unknown() {
    let mut tracker = CmvsTracker::default();
    assert_eq!(
        cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
        CmvsState::Inside
    );
    let state = cmvs_after_tu(&mut tracker, true, None, true);
    assert_eq!(state, CmvsState::Unknown);
}

#[test]
fn cmvs_inside_continues_across_non_boundary_tu() {
    let mut tracker = CmvsTracker::default();
    assert_eq!(
        cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
        CmvsState::Inside
    );
    let state = cmvs_after_tu(&mut tracker, false, None, false);
    assert_eq!(state, CmvsState::Inside);
}

#[test]
fn cmvs_no_clk_temporal_unit_does_not_begin() {
    let mut tracker = CmvsTracker::default();
    let state = cmvs_after_tu(&mut tracker, false, Some(MsdoObservation::First), false);
    assert_eq!(state, CmvsState::Outside);
}

#[test]
fn cmvs_unknown_is_conservative_and_persists() {
    let mut tracker = CmvsTracker::default();
    assert_eq!(
        cmvs_after_tu(&mut tracker, true, None, true),
        CmvsState::Unknown
    );
    assert_eq!(
        cmvs_after_tu(&mut tracker, false, None, false),
        CmvsState::Unknown
    );
    assert_eq!(
        cmvs_after_tu(&mut tracker, true, Some(MsdoObservation::First), false),
        CmvsState::Inside
    );
}

/// § 5.4.1 default fill for `max_mlayer_id == 1`: `MLayerDependencyMap[1][0]`
/// is 1, so a mask including layer 1 without layer 0 violates the closure.
#[test]
fn mlayer_closure_violation_reports_missing_required_dependency() {
    let m_map = MLayerDependencyMap::default_for(EmbeddedLayerId::from_bits(1));
    assert_eq!(mlayer_closure_violation(0b10, &m_map), Some((1, 0)));
}

#[test]
fn mlayer_closure_violation_accepts_closed_and_independent_masks() {
    let m_map = MLayerDependencyMap::default_for(EmbeddedLayerId::from_bits(1));
    assert_eq!(mlayer_closure_violation(0b11, &m_map), None);
    assert_eq!(mlayer_closure_violation(0b01, &m_map), None);
    assert_eq!(mlayer_closure_violation(0b1000_0000, &m_map), None);
}

/// § 5.4.1 default fill for `max_tlayer_id == 1`: within embedded layer 0,
/// `TLayerDependencyMap[0][1][0]` is 1.
#[test]
fn tlayer_closure_violation_reports_missing_required_dependency() {
    let t_map = TLayerDependencyMap::default_for(
        TemporalLayerId::from_bits(1),
        EmbeddedLayerId::from_bits(1),
    );
    assert_eq!(tlayer_closure_violation(0, 0b10, &t_map), Some((1, 0)));
}

#[test]
fn tlayer_closure_violation_accepts_closed_masks_and_out_of_range_layers() {
    let t_map = TLayerDependencyMap::default_for(
        TemporalLayerId::from_bits(1),
        EmbeddedLayerId::from_bits(1),
    );
    assert_eq!(tlayer_closure_violation(0, 0b11, &t_map), None);
    assert_eq!(tlayer_closure_violation(0, 0b01, &t_map), None);
    assert_eq!(tlayer_closure_violation(5, 0b10, &t_map), None);
}
