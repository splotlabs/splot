# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` row `AV2-5.18.10-FILM-GRAIN-STRUCTURES`
      (notes: residual (b) closed; openspec_change pointer) and `AV2-5.14-FILM-GRAIN`
      reference-check note re-stated.
- [x] Register the three new diagnostics in `docs/VALIDATOR-DIAGNOSTICS.md` (`frame-header/`
      namespace).
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md` (unchanged — only notes moved).

## Implementation

- [x] Type `FgmSlotRecord`'s `mlayer_id` / `tlayer_id` as `EmbeddedLayerId` / `TemporalLayerId`
      so `depends_on` can be called without reconstruction.
- [x] Thread `active_sequence` into `frame_film_grain_reference_checks` (mirror
      `frame_qm_reference_checks`).
- [x] Emit `frame-header/film-grain-mlayer-dependency-missing` when
      `MLayerDependencyMap[obu_mlayer_id][FgmMLayerId[fgm_id]] != 1`.
- [x] Emit `frame-header/film-grain-tlayer-dependency-missing` when
      `TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][FgmTLayerId[fgm_id]] != 1`.
- [x] Emit `frame-header/film-grain-chroma-idc-mismatch` when
      `FgmChromaIdc[fgm_id] != chroma_format_idc`.
- [x] Keep the constraints gated on `apply_grain == 1`, `ExternalHlsMode::Disabled`, and an
      in-band recorded model (`record.is_some()`); an unavailable slot is owned by the
      existing availability diagnostic, not layer-checked.

## Tests and proof

- [x] Positive: satisfied m-layer/t-layer dependencies and matching chroma stay silent.
- [x] Negative: each of the three constraints fires on a crafted violation.
- [x] Negative: an unavailable model (no recorded slot) fires only the availability
      diagnostic, not the layer-dependency ones.
- [x] Suppression: a Provided external-HLS mode does not fire the layer-dependency checks.
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
