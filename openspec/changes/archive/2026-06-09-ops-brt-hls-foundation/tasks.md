# Tasks: Operating point set + buffer removal timing HLS foundation

## 1. Planning and tracking

- [x] Read `AGENTS.md`, `docs/FEATURE-TRACKING.md`, and `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Create this OpenSpec change.
- [x] Update matrix rows to use `openspec_change = "ops-brt-hls-foundation"`.
- [x] Keep statuses honest: parser stages become `done`; validation stays `partial`.

## 2. Core OPS parser (`splot-core`)

- [x] Add OPS data structures (`OperatingPointSet`, `OperatingPointPayload`,
      `OpsXlayerEntry`, `OpsMlayerSource`, and the § 5.11.1-§ 5.11.5 child structs).
- [x] Parse § 5.10 `operating_point_set_obu()` (reset/update fields + payload loop).
- [x] Parse § 5.11 `operating_point_payload()` with exact `opsBytes` accounting.
- [x] Parse § 5.11.1 aggregate, § 5.11.2 PTL, § 5.11.3 decoder model, § 5.11.4 color,
      and § 5.11.5 mlayer info, including global xlayer-map and inheritance branches.
- [x] Preserve reserved bits, declared/computed sizes, and inherited references.
- [x] Unit-test reset-only, global/local payloads, reserved bits, reserved idc, PTL
      reserved bits, payload-size accounting, inheritance, and the never-panic property.

## 3. Core BRT parser (`splot-core`)

- [x] Add `BufferRemovalTiming` data structures and accessor methods.
- [x] Parse the extended-layer `br_time` path and the OPS-dependent path.
- [x] Unit-test both forms and the never-panic property.

## 4. Dispatch and inspector

- [x] Dispatch `OBU_OPERATING_POINT_SET` (extensible tail) and
      `OBU_BUFFER_REMOVAL_TIMING` (non-extensible) through `dispatch_obu_payload`.
- [x] Add `ParsedObu` variants with `feature_id()` / `syntax_name()`.
- [x] Surface `operating_point_set` and `buffer_removal_timing` in `inspect --json`.
- [x] Add committed fixtures and CLI inspector tests.

## 5. Validator state and diagnostics (`splot-validate`)

- [x] Add the non-monotonic `OpsAvailabilityStore` and § 6.10.1 reset/update semantics.
- [x] Emit the locally-decidable `ops/*` diagnostics (reserved bits, reserved idc, PTL
      reserved bits, payload-size mismatch, inherited op-index bounds).
- [x] Validate `brt/*` OPS references under external-HLS-disabled mode and keep external
      HLS from producing false hard errors.
- [x] Replace the temporal-unit BRT ordering TODO with a spec-backed, tested rule.
- [x] Add validator tests for reset/remove, update, missing OPS, count mismatch, and
      ordering classification.

## 6. Proof

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test -p splot-core ops` / `brt`
- [x] `cargo test -p splot-validate ops` / `brt`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask ci`
- [x] `cargo xtask feature-status` / `check-feature-status` / `spec-coverage`
- [x] `openspec validate ops-brt-hls-foundation --strict`
- [x] Record exact command results in the implementation summary.

## 7. Deferred (tracked, not done)

- [ ] OPS dependency-map agreement with the activated sequence header (§ 6.10.7).
- [ ] Hard `brt/global-ordering-position` ordering diagnostic (§ 7.3.7).
- [ ] Annex A.4 level and Annex E schedule/resource conformance.
