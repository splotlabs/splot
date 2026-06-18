# OpenSpec changes

Each subdirectory is one proposed change. Start a new change by copying the
template:

```bash
mkdir -p openspec/changes/<change-id>/specs/<area>
cp openspec/templates/change/proposal.md openspec/changes/<change-id>/proposal.md
cp openspec/templates/change/tasks.md openspec/changes/<change-id>/tasks.md
cp openspec/templates/change/design.md openspec/changes/<change-id>/design.md
```

Then:

1. Fill in `proposal.md` (why, scope, non-goals, acceptance criteria) and list the
   Feature IDs from `docs/IMPLEMENTATION-MATRIX.toml`.
2. Record the `<change-id>` in each affected matrix row's `openspec_change` field.
3. Work through `tasks.md`.
4. Keep `design.md` for non-trivial design decisions; small changes may drop it.

A `<change-id>` is lowercase-kebab (for example, `parse-frame-header`). Use the
same id in the matrix, the OpenSpec folder, and the GitHub issue/PR.

## Active changes

| Change | Feature IDs | State |
|---|---|---|
| `avm-differential-harness` | `CONF-AVM-DIFF-HARNESS` | proposed |
| `coeff-all-zero-context-state` | `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE` | blocked (invalid OpenSpec metadata; `openspec status` fails) |
| `recon-dequant-process` | `RECON-DEQUANT-PROCESS` | in-progress (unchecked completion/review/gate tasks) |
| `recon-dequant-qm-weight` | `INFRA-SHARED-SPEC-TABLES`, `RECON-DEQUANT-PROCESS`, `RECON-DEQUANT-QM-WEIGHT` | in-progress (unchecked completion/review/gate tasks) |
| `recon-dequant-quantizer-index-resolution` | `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION`, `RECON-DEQUANT-QUANTIZER-LOOKUP` | in-progress (unchecked completion/review/gate tasks) |
| `recon-dequant-quantizer-lookup` | `RECON-DEQUANT-QUANTIZER-LOOKUP` | in-progress (unchecked completion/review/gate tasks) |
| `recon-inverse-transform-1d` | `INFRA-SHARED-SPEC-TABLES`, `RECON-INVERSE-TRANSFORM-1D` | in-progress (unchecked completion/review/gate tasks) |
| `recon-inverse-transform-2d` | `RECON-INVERSE-TRANSFORM-2D` | in-progress (unchecked completion/review/gate tasks) |
| `recon-inverse-transform-2d-outer` | `RECON-INVERSE-TRANSFORM-2D-OUTER` | in-progress (unchecked completion/review/gate tasks) |
| `recon-reference-frame-store-refresh-flags` | `AV2-7.23-REFERENCE-FRAME-UPDATE`, `CONF-RECON-REFERENCE-FRAME-STORE-FUZZ`, `RECON-REFERENCE-FRAME-STORE` | in-progress (unchecked completion/review/gate tasks) |
| `recon-residual-addition` | `RECON-RESIDUAL-ADDITION` | in-progress (unchecked completion/review/gate tasks) |
| `recon-transform-matrix-free` | `RECON-INVERSE-TRANSFORM-1D`, `RECON-INVERSE-TRANSFORM-MATRIX-FREE` | in-progress (unchecked completion/review/gate tasks) |
| `shared-spec-tables-crate` | `INFRA-SHARED-SPEC-TABLES` | in-progress (unchecked completion/review/gate tasks) |
| `tile-txb-skip-context-derivation` | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | in-progress (unchecked completion/review/gate tasks) |
| `tile-uv-mode-context-derivation` | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | in-progress (unchecked completion/review/gate tasks) |
| `tile-y-mode-index-context-derivation` | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | in-progress (unchecked completion/review/gate tasks) |
| `toy-intra-encoder-v0` | `ENC-INTRA-TOY-V0` (deps: `ENC-BITSTREAM-WRITER`, `AV2-5.4-SEQUENCE-HEADER`, `AV2-5.18-FRAME-HEADER`, `AV2-5.19-TILE-GROUP`, `CONF-AVM-DIFF-HARNESS`) | parked (encoder track, behind the [VALIDATOR-ROADMAP](../../docs/VALIDATOR-ROADMAP.md) fence) |
