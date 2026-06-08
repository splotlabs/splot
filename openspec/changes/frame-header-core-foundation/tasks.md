# Tasks: frame-header core foundation

## OpenSpec and matrix

- [x] Create/validate `openspec/changes/frame-header-core-foundation`.
- [x] Add/refine matrix rows for:
  - [x] `AV2-5.18.2-FRAME-HEADER-INFO`
  - [x] `AV2-5.18.3-FRAME-CONFIGURATION`
  - [x] `AV2-5.18.4-FRAME-SIZE`
  - [x] `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS`
- [x] Keep umbrella `AV2-5.18-FRAME-HEADER` partial.
- [x] Add valid `TODO(spec: FEATURE-ID)` markers for deferred branches.

## Core parser

- [x] Add `FrameHeaderParseMode`.
- [x] Add `FrameHeaderParseStatus`.
- [x] Add typed frame core result struct.
- [x] Add explicit parser input/context type.
- [x] Preserve activation-prefix parser behavior.
- [x] Implement core parser fields that are state-supported.
- [x] Add frame-size helper type.
- [x] Stop with structured status before unimplemented deep §5.18 sections.

## Validator

- [x] Wire core parser in validator where active sequence state exists.
- [x] Preserve existing HLS unavailable sequence/MFH diagnostics.
- [x] Add local diagnostics:
  - [x] `frame-header/bridge-ref-index-out-of-range`
  - [x] `frame-header/frame-to-show-map-index-out-of-range`
  - [x] `frame-header/ref-frame-index-out-of-range`
  - [x] `frame-header/primary-ref-frame-out-of-range`
  - [x] `frame-header/frame-size-exceeds-sequence-max`
  - [x] `frame-header/frame-size-zero`
  - [x] `frame-header/ras-requires-long-term-frame-id-bits`
- [x] Do not emit false positives for unimplemented reference-state paths.

## Inspector

- [x] Add JSON frame-core summary.
- [x] Include parse status.
- [x] Avoid raw/unbounded payload dumps.

## Tests

- [x] Existing frame/tile-group prefix tests still pass.
- [x] Positive core parser tests.
- [x] EOF tests at variable-width fields.
- [x] Validator diagnostics tests.
- [x] Inspector JSON test or field assertions.
- [x] Regenerate `docs/FEATURE-STATUS.md`.

## Acceptance

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test -p splot-core frame_header`
- [x] `cargo test -p splot-core tile_group`
- [x] `cargo test -p splot-validate frame_header`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `openspec validate frame-header-core-foundation --strict`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
