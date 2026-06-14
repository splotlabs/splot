// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const MAX: fn(u64) -> DecodeLimitThreshold = DecodeLimitThreshold::Max;

#[test]
fn default_options_and_limits_are_finite_and_pinned() {
    let limits = DecodeLimits::default();
    let cases = [
        (
            DecodeLimitName::MaxInputBytes,
            limits.max_input_bytes(),
            16 * 1024 * 1024,
        ),
        (DecodeLimitName::MaxObus, limits.max_obus(), 4_096),
        (
            DecodeLimitName::MaxIvfFrameRecords,
            limits.max_ivf_frame_records(),
            4_096,
        ),
        (
            DecodeLimitName::MaxFramesToDecode,
            limits.max_frames_to_decode(),
            128,
        ),
        (
            DecodeLimitName::MaxOutputFrames,
            limits.max_output_frames(),
            128,
        ),
        (
            DecodeLimitName::MaxFrameWidth,
            limits.max_frame_width(),
            4_096,
        ),
        (
            DecodeLimitName::MaxFrameHeight,
            limits.max_frame_height(),
            4_096,
        ),
        (
            DecodeLimitName::MaxLumaSamplesPerFrame,
            limits.max_luma_samples_per_frame(),
            4_096 * 4_096,
        ),
        (
            DecodeLimitName::MaxDecodedFrameBytes,
            limits.max_decoded_frame_bytes(),
            64 * 1024 * 1024,
        ),
        (
            DecodeLimitName::MaxReferenceSlots,
            limits.max_reference_slots(),
            16,
        ),
        (
            DecodeLimitName::MaxReferenceStoreBytes,
            limits.max_reference_store_bytes(),
            256 * 1024 * 1024,
        ),
        (
            DecodeLimitName::MaxTileCount,
            limits.max_tile_count(),
            4_096,
        ),
        (
            DecodeLimitName::MaxTilePayloadBytes,
            limits.max_tile_payload_bytes(),
            16 * 1024 * 1024,
        ),
        (
            DecodeLimitName::MaxOutputBytes,
            limits.max_output_bytes(),
            256 * 1024 * 1024,
        ),
    ];

    assert_eq!(DecodeOptions::default(), DecodeOptions::DEFAULT);
    assert_eq!(DecodeOptions::default().limits(), DecodeLimits::DEFAULT);
    assert_eq!(DecodeOptions::new(limits).limits(), limits);
    assert_eq!(
        DecodeOptions::default()
            .with_limits(DecodeLimits::zero())
            .limits(),
        DecodeLimits::zero()
    );
    for (name, getter, expected) in cases {
        assert_eq!(getter, MAX(expected));
        assert_eq!(limits.threshold(name), MAX(expected));
        assert!(expected > 0);
    }
}

#[test]
fn zero_and_unlimited_policies_are_explicit() {
    for name in DecodeLimitName::ALL {
        assert_eq!(DecodeLimits::zero().threshold(name), MAX(0));
        assert_eq!(
            DecodeLimits::unlimited().threshold(name),
            DecodeLimitThreshold::Unlimited
        );
        assert!(DecodeLimits::unlimited().threshold(name).is_unlimited());
    }
}

#[test]
fn limit_names_and_units_are_stable() {
    assert_eq!(
        DecodeLimitName::ALL.map(DecodeLimitName::as_str),
        [
            "max_input_bytes",
            "max_obus",
            "max_ivf_frame_records",
            "max_frames_to_decode",
            "max_output_frames",
            "max_frame_width",
            "max_frame_height",
            "max_luma_samples_per_frame",
            "max_decoded_frame_bytes",
            "max_reference_slots",
            "max_reference_store_bytes",
            "max_tile_count",
            "max_tile_payload_bytes",
            "max_output_bytes",
        ]
    );
    assert_eq!(
        DecodeLimitName::ALL.map(DecodeLimitName::unit),
        [
            DecodeLimitUnit::Bytes,
            DecodeLimitUnit::Count,
            DecodeLimitUnit::Count,
            DecodeLimitUnit::Count,
            DecodeLimitUnit::Count,
            DecodeLimitUnit::LumaSamples,
            DecodeLimitUnit::LumaSamples,
            DecodeLimitUnit::LumaSamples,
            DecodeLimitUnit::Bytes,
            DecodeLimitUnit::Count,
            DecodeLimitUnit::Bytes,
            DecodeLimitUnit::Count,
            DecodeLimitUnit::Bytes,
            DecodeLimitUnit::Bytes,
        ]
    );
    assert_eq!(DecodeLimitUnit::Bytes.as_str(), "bytes");
    assert_eq!(DecodeLimitUnit::Count.as_str(), "count");
    assert_eq!(DecodeLimitUnit::LumaSamples.as_str(), "luma_samples");
    assert_eq!(
        DecodeLimitName::MaxTilePayloadBytes.to_string(),
        "max_tile_payload_bytes"
    );
    assert_eq!(DecodeLimitOp::Mul.to_string(), "mul");
}

#[test]
fn threshold_lookup_and_field_helpers_route_to_typed_names() {
    let limits = DecodeLimits::unlimited()
        .with_max_input_bytes(MAX(1))
        .with_max_obus(MAX(2))
        .with_max_ivf_frame_records(MAX(3))
        .with_max_frames_to_decode(MAX(4))
        .with_max_output_frames(MAX(5))
        .with_max_frame_width(MAX(6))
        .with_max_frame_height(MAX(7))
        .with_max_luma_samples_per_frame(MAX(8))
        .with_max_decoded_frame_bytes(MAX(9))
        .with_max_reference_slots(MAX(10))
        .with_max_reference_store_bytes(MAX(11))
        .with_max_tile_count(MAX(12))
        .with_max_tile_payload_bytes(MAX(13))
        .with_max_output_bytes(MAX(14));
    let cases = [
        (DecodeLimitName::MaxInputBytes, limits.max_input_bytes(), 1),
        (DecodeLimitName::MaxObus, limits.max_obus(), 2),
        (
            DecodeLimitName::MaxIvfFrameRecords,
            limits.max_ivf_frame_records(),
            3,
        ),
        (
            DecodeLimitName::MaxFramesToDecode,
            limits.max_frames_to_decode(),
            4,
        ),
        (
            DecodeLimitName::MaxOutputFrames,
            limits.max_output_frames(),
            5,
        ),
        (DecodeLimitName::MaxFrameWidth, limits.max_frame_width(), 6),
        (
            DecodeLimitName::MaxFrameHeight,
            limits.max_frame_height(),
            7,
        ),
        (
            DecodeLimitName::MaxLumaSamplesPerFrame,
            limits.max_luma_samples_per_frame(),
            8,
        ),
        (
            DecodeLimitName::MaxDecodedFrameBytes,
            limits.max_decoded_frame_bytes(),
            9,
        ),
        (
            DecodeLimitName::MaxReferenceSlots,
            limits.max_reference_slots(),
            10,
        ),
        (
            DecodeLimitName::MaxReferenceStoreBytes,
            limits.max_reference_store_bytes(),
            11,
        ),
        (DecodeLimitName::MaxTileCount, limits.max_tile_count(), 12),
        (
            DecodeLimitName::MaxTilePayloadBytes,
            limits.max_tile_payload_bytes(),
            13,
        ),
        (
            DecodeLimitName::MaxOutputBytes,
            limits.max_output_bytes(),
            14,
        ),
    ];

    for (name, getter, expected) in cases {
        assert_eq!(limits.threshold(name), getter);
        assert_eq!(getter, MAX(expected));
    }
}

#[test]
fn limit_checks_are_inclusive() {
    let width_limited =
        DecodeLimits::unlimited().with_max_frame_width(DecodeLimitThreshold::Max(10));
    let zero_limited = DecodeLimits::unlimited().with_max_obus(DecodeLimitThreshold::Max(0));

    assert!(
        width_limited
            .check(DecodeLimitName::MaxFrameWidth, 9)
            .is_allowed()
    );
    assert_eq!(
        width_limited.ensure(DecodeLimitName::MaxFrameWidth, 10),
        Ok(DecodeLimitCheck::new(
            DecodeLimitName::MaxFrameWidth,
            MAX(10),
            10,
        ))
    );
    assert!(
        width_limited
            .check(DecodeLimitName::MaxFrameWidth, 11)
            .is_exceeded()
    );
    assert!(zero_limited.check(DecodeLimitName::MaxObus, 0).is_allowed());
    assert!(
        zero_limited
            .check(DecodeLimitName::MaxObus, 1)
            .is_exceeded()
    );
    assert!(
        DecodeLimits::unlimited()
            .check(DecodeLimitName::MaxOutputBytes, u64::MAX)
            .is_allowed()
    );
    assert_eq!(
        width_limited.ensure(DecodeLimitName::MaxFrameWidth, 11),
        Err(DecodeLimitError::LimitExceeded {
            check: DecodeLimitCheck::new(DecodeLimitName::MaxFrameWidth, MAX(10), 11,),
        })
    );
}

#[test]
fn checked_arithmetic_helpers_preserve_metadata() {
    let limits = DecodeLimits::unlimited().with_max_output_bytes(MAX(100));

    assert_eq!(
        limits.ensure_add(DecodeLimitName::MaxOutputBytes, 40, 2),
        Ok(DecodeLimitCheck::new(
            DecodeLimitName::MaxOutputBytes,
            MAX(100),
            42,
        ))
    );
    assert_eq!(
        limits.ensure_mul(DecodeLimitName::MaxOutputBytes, 6, 7),
        Ok(DecodeLimitCheck::new(
            DecodeLimitName::MaxOutputBytes,
            MAX(100),
            42,
        ))
    );
    assert_eq!(
        limits.ensure_add(DecodeLimitName::MaxOutputBytes, u64::MAX, 1),
        Err(DecodeLimitError::ArithmeticOverflow {
            name: DecodeLimitName::MaxOutputBytes,
            op: DecodeLimitOp::Add,
            left: u64::MAX,
            right: 1,
        })
    );
    assert_eq!(
        limits.ensure_mul(DecodeLimitName::MaxOutputBytes, u64::MAX, 2),
        Err(DecodeLimitError::ArithmeticOverflow {
            name: DecodeLimitName::MaxOutputBytes,
            op: DecodeLimitOp::Mul,
            left: u64::MAX,
            right: 2,
        })
    );
    assert_eq!(
        limits.ensure_mul(DecodeLimitName::MaxOutputBytes, 11, 10),
        Err(DecodeLimitError::LimitExceeded {
            check: DecodeLimitCheck::new(DecodeLimitName::MaxOutputBytes, MAX(100), 110,),
        })
    );
}

#[test]
fn allocation_handoff_checks_limit_then_host_size() {
    let limits = DecodeLimits::unlimited().with_max_output_bytes(MAX(42));

    assert_eq!(
        limits.ensure_allocation_len(DecodeLimitName::MaxOutputBytes, 42),
        Ok(42usize)
    );
    assert_eq!(
        limits.ensure_allocation_len(DecodeLimitName::MaxOutputBytes, 43),
        Err(DecodeLimitError::LimitExceeded {
            check: DecodeLimitCheck::new(DecodeLimitName::MaxOutputBytes, MAX(42), 43,),
        })
    );
}

#[test]
fn allocation_handoff_reports_platform_size_error() {
    let too_large = isize::MAX as u64 + 1;
    let err =
        DecodeLimits::unlimited().ensure_allocation_len(DecodeLimitName::MaxOutputBytes, too_large);

    assert_eq!(
        err,
        Err(DecodeLimitError::HostAllocationTooLarge {
            name: DecodeLimitName::MaxOutputBytes,
            actual: too_large,
        })
    );
}

#[test]
fn limit_errors_are_local_and_not_decoder_diagnostics() {
    let error = DecodeLimits::unlimited()
        .with_max_input_bytes(MAX(3))
        .ensure(DecodeLimitName::MaxInputBytes, 4);
    let expected = DecodeLimitError::LimitExceeded {
        check: DecodeLimitCheck::new(DecodeLimitName::MaxInputBytes, MAX(3), 4),
    };
    let local_errors = [
        expected,
        DecodeLimitError::ArithmeticOverflow {
            name: DecodeLimitName::MaxOutputBytes,
            op: DecodeLimitOp::Mul,
            left: u64::MAX,
            right: 2,
        },
        DecodeLimitError::HostAllocationTooLarge {
            name: DecodeLimitName::MaxOutputBytes,
            actual: isize::MAX as u64 + 1,
        },
    ];

    assert_eq!(error, Err(expected));
    assert_eq!(expected.name(), DecodeLimitName::MaxInputBytes);
    assert_eq!(expected.check().map(DecodeLimitCheck::actual), Some(4));
    assert_eq!(expected.actual(), Some(4));
    assert_eq!(expected.op(), None);
    assert_eq!(expected.left(), None);
    assert_eq!(expected.right(), None);
    for local_error in local_errors {
        assert!(
            !local_error
                .to_string()
                .contains(concat!("decode/", "resource-limit"))
        );
    }
    assert_ne!(
        core::any::type_name::<DecodeLimitError>(),
        core::any::type_name::<crate::DecodeDiagnostic>()
    );
    assert_eq!(
        crate::unsupported_feature_diagnostic(),
        crate::UNSUPPORTED_FEATURE_DIAGNOSTIC
    );
}
