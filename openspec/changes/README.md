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
| `ci-pipeline-speedups` | `INFRA-CI-PIPELINE-SPEEDUPS` | in-progress (implemented; pending review) |
| `doc-budget-gate` | `XTASK-DOC-BUDGET` | in-progress (implemented; pending verification) |
| `closed-loop-nonuniform-4x4` | `ENC-CLOSED-LOOP-NONUNIFORM-4X4` | blocked (artifact incomplete: design=ready) |
| `coeff-all-zero-context-state` | `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE` | blocked (invalid OpenSpec metadata; ✖ Error: Invalid YAML in metadata file: Source contains multiple documents; please use YAML.parseAllDocuments() at line 3, column 1:) |
| `coeff-general-walk-coeff-br` | `ENC-COEFF-GENERAL-WALK-COEFF-BR` | blocked (artifact incomplete: design=ready) |
| `coeff-general-walk-dc-br` | `ENC-COEFF-GENERAL-WALK-DC-BR` | blocked (artifact incomplete: design=ready) |
| `coeff-general-walk-eob-extra` | `ENC-COEFF-GENERAL-WALK-EOB-EXTRA` | blocked (artifact incomplete: design=ready) |
| `coeff-general-walk-eob-extra-bits` | `ENC-COEFF-GENERAL-WALK-EOB-EXTRA-BITS` | blocked (artifact incomplete: design=ready) |
| `coeff-general-walk-golomb` | `ENC-COEFF-GENERAL-WALK-GOLOMB` | blocked (artifact incomplete: design=ready) |
| `coeff-general-walk-golomb-multi` | `ENC-COEFF-GENERAL-WALK-GOLOMB-MULTI` | blocked (artifact incomplete: design=ready) |
| `coeff-general-walk-hf-eob11` | `ENC-COEFF-GENERAL-WALK-HF-EOB11` | blocked (artifact incomplete: design=ready) |
| `coeff-general-walk-hf-multi` | `ENC-COEFF-GENERAL-WALK-HF-MULTI` | blocked (artifact incomplete: design=ready) |
| `coeff-general-walk-lf-base` | `ENC-COEFF-GENERAL-WALK-LF-BASE` | blocked (artifact incomplete: design=ready) |
| `decode-10bit-sequence-frontier` | `DECODE-10BIT-SEQUENCE-FRONTIER` | blocked (artifact incomplete: design=ready) |
| `decode-leading-obu-gate` | n/a | blocked (artifact incomplete: design=ready) |
| `decode-runtime-frame-gate` | n/a | blocked (artifact incomplete: design=ready) |
| `decode-sequence-chroma-frontier` | `DECODE-SEQUENCE-CHROMA-FRONTIER` | blocked (artifact incomplete: design=ready) |
| `decode-traversal-defaults` | `DECODE-BYTE-STREAM-PLANNER`, `DECODE-LIMITS-RUNTIME-API`, `DOC-DECODE-LIMITS-CONTRACT` | blocked (artifact incomplete: design=ready) |
| `decode-wienerns-bank-frontier` | `DECODE-WIENERNS-BANK-FRONTIER` | blocked (delta sync ambiguity; modified requirement target missing) |
| `decode-general-intra-rect-partition` | `DECODE-GENERAL-INTRA-RECT-PARTITION` | in-progress (unchecked tasks) |
| `decode-inter-grid-spatial` | `DECODE-GENERAL-INTRA-GRID`, `DECODE-INTER-GRID-SPATIAL`, `DECODE-INTER-MULTI-SB-SPATIAL`, `DECODE-INTER-MVSTACK-SPATIAL` | in-progress (artifact incomplete: design=ready; unchecked tasks) |
| `decode-inter-header-shared-tail` | n/a | blocked (delta sync ambiguity; modified requirement target missing) |
| `decode-inter-multi-sb-spatial` | `DECODE-INTER-MULTI-SB-SPATIAL`, `DECODE-INTER-MVSTACK-SPATIAL` | in-progress (artifact incomplete: design=ready; unchecked tasks) |
| `decode-inter-multiref-runtime` | `DECODE-INTER-MULTIREF-RUNTIME`, `DECODE-INTER-SINGLE-REF-SYMBOL` | in-progress (artifact incomplete: design=ready; unchecked tasks) |
| `decode-inter-multiref-runtime-gate-hardening` | `DECODE-INTER-MULTIREF-RUNTIME` | blocked (artifact incomplete: design=ready) |
| `decode-inter-mvorder-spatial` | `DECODE-INTER-GRID-SPATIAL`, `DECODE-INTER-MVORDER-SPATIAL`, `DECODE-INTER-MVSTACK-SPATIAL` | in-progress (artifact incomplete: design=ready; unchecked tasks) |
| `decode-inter-mvstack-spatial` | `DECODE-FIRST-INTER-FRAME-FRONTIER`, `DECODE-INTER-MVSTACK-SPATIAL`, `DECODE-INTER-RESIDUAL-DCT`, `DECODE-INTER-SUBPEL-MV` | in-progress (artifact incomplete: design=ready; unchecked tasks) |
| `decode-inter-residual-dct` | `DECODE-FIRST-INTER-FRAME-FRONTIER`, `DECODE-INTER-RESIDUAL-DCT`, `DECODE-INTER-SUBPEL-MV` | blocked (artifact incomplete: design=ready) |
| `decode-inter-single-ref-symbol` | `DECODE-INTER-SINGLE-REF-SYMBOL` | blocked (change text contains blocked marker; unchecked tasks) |
| `decode-inter-subpel-mv` | `DECODE-FIRST-INTER-FRAME-FRONTIER`, `DECODE-INTER-SUBPEL-MV`, `RECON-SUBPEL-MC` | blocked (artifact incomplete: design=ready) |
| `decode-ivf-grouped-frame-units` | n/a | blocked (artifact incomplete: design=ready) |
| `decoder-runtime-deslop` | `DECODE-GENERIC-RUNTIME-DESLOP` | in-progress (implemented; pending review) |
| `dupehound-duplication-gate` | `INFRA-DUPEHOUND-DUPLICATION-GATE` | in-progress (gate landed; dedup campaign ongoing) |
| `encoder-coeff-tokenize-16x16-base` | `ENC-COEFF-TOKENIZE-16X16-BASE` | blocked (artifact incomplete: design=ready) |
| `encoder-coeff-tokenize-16x16-dc` | `ENC-COEFF-TOKENIZE-16X16-DC` | blocked (artifact incomplete: design=ready) |
| `encoder-coeff-tokenize-16x16-refine` | `ENC-COEFF-TOKENIZE-16X16-REFINE` | blocked (artifact incomplete: design=ready) |
| `encoder-config-qp-field` | `ENC-CONFIG-QP-FIELD` | blocked (artifact incomplete: design=ready) |
| `encoder-context-receive-packet` | `ENC-CONTEXT-RECEIVE-PACKET` | blocked (artifact incomplete: design=ready) |
| `encoder-decide-rate-controller` | `ENC-DECIDE-RATE-CONTROLLER` | blocked (artifact incomplete: design=ready) |
| `encoder-general-intra-2d` | `ENC-GENERAL-INTRA-2D` | blocked (artifact incomplete: design=ready) |
| `encoder-general-intra-all-planes-coded` | `ENC-GENERAL-INTRA-ALL-PLANES-CODED` | blocked (artifact incomplete: design=ready) |
| `encoder-general-intra-coded-chroma-dc` | `ENC-GENERAL-INTRA-CODED-CHROMA-DC` | blocked (artifact incomplete: design=ready) |
| `encoder-general-intra-coded-chroma-v-dc` | `ENC-GENERAL-INTRA-CODED-CHROMA-V-DC` | blocked (artifact incomplete: design=ready) |
| `encoder-general-intra-eob3` | `ENC-GENERAL-INTRA-EOB3` | blocked (artifact incomplete: design=ready) |
| `encoder-general-intra-two-coeff` | `ENC-GENERAL-INTRA-TWO-COEFF` | blocked (artifact incomplete: design=ready) |
| `encoder-general-intra-two-nonzero` | `ENC-GENERAL-INTRA-TWO-NONZERO` | blocked (artifact incomplete: design=ready) |
| `encoder-general-intra-visible-ac` | `ENC-GENERAL-INTRA-VISIBLE-AC` | blocked (artifact incomplete: design=ready) |
| `encoder-partition-do-square-split` | `ENC-PARTITION-DO-SQUARE-SPLIT` | blocked (artifact incomplete: design=ready) |
| `extract-fuzz-corpus-seeding` | `INFRA-FUZZ-CORPUS-SEEDING` | in-progress (implemented; pending review) |
| `forward-dct-16x16` | `ENC-FORWARD-TRANSFORM-DCT-16X16` | blocked (artifact incomplete: design=ready) |
| `forward-dct-4x4-full` | `ENC-FORWARD-TRANSFORM-DCT-4X4` | blocked (artifact incomplete: design=ready) |
| `forward-quant-per-coeff-ac` | `ENC-FORWARD-TRANSFORM-DCT-4X4`, `ENC-FWD-QUANT-PER-COEFF-AC` | blocked (artifact incomplete: design=ready) |
| `recon-deblock-adaptive-strength` | `RECON-DEBLOCK-ADAPTIVE-STRENGTH`, `RECON-DEBLOCK-FILTER-MAX-WIDTH`, `RECON-DEBLOCK-SAMPLE-FILTER` | in-progress (unchecked tasks) |
| `recon-deblock-filter-choice` | `RECON-DEBLOCK-ADAPTIVE-STRENGTH`, `RECON-DEBLOCK-FILTER-CHOICE`, `RECON-DEBLOCK-FILTER-MAX-WIDTH`, `RECON-DEBLOCK-SAMPLE-FILTER` | in-progress (unchecked tasks) |
| `recon-deblock-filter-max-width` | `RECON-DEBLOCK-FILTER-MAX-WIDTH`, `RECON-DEBLOCK-SAMPLE-FILTER` | in-progress (unchecked tasks) |
| `recon-deblock-sample-filter` | `RECON-DEBLOCK-SAMPLE-FILTER` | in-progress (unchecked tasks) |
| `recon-dequant-process` | `RECON-DEQUANT-PROCESS` | in-progress (unchecked tasks) |
| `recon-dequant-qm-weight` | `INFRA-SHARED-SPEC-TABLES`, `RECON-DEQUANT-PROCESS`, `RECON-DEQUANT-QM-WEIGHT` | in-progress (unchecked tasks) |
| `recon-dequant-quantizer-index-resolution` | `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION`, `RECON-DEQUANT-QUANTIZER-LOOKUP` | in-progress (unchecked tasks) |
| `recon-dequant-quantizer-lookup` | `RECON-DEQUANT-QUANTIZER-LOOKUP` | in-progress (unchecked tasks) |
| `recon-dpcm-direction` | `RECON-DPCM-DIRECTION`, `RECON-GET-TRANSFORM-1D-TYPE`, `RECON-INVERSE-TRANSFORM-2D-OUTER`, `RECON-RESOLVE-2D-TRANSFORM-PARAMS`, `RECON-TRANSFORM-SHIFT-LOOKUP` | in-progress (unchecked tasks) |
| `recon-inverse-transform-1d` | `INFRA-SHARED-SPEC-TABLES`, `RECON-INVERSE-TRANSFORM-1D` | in-progress (unchecked tasks) |
| `recon-inverse-transform-2d` | `RECON-INVERSE-TRANSFORM-2D` | in-progress (unchecked tasks) |
| `recon-inverse-transform-2d-outer` | `RECON-INVERSE-TRANSFORM-2D-OUTER` | in-progress (unchecked tasks) |
| `recon-loop-restoration-source-read` | `RECON-LOOP-RESTORATION-SOURCE-READ` | in-progress (unchecked tasks) |
| `recon-loop-restoration-source-sample` | `RECON-LOOP-RESTORATION-SOURCE-SAMPLE` | in-progress (unchecked tasks) |
| `recon-reconstruct-transform-block` | `RECON-RECONSTRUCT-TRANSFORM-BLOCK`, `RECON-RESIDUAL-ADDITION` | in-progress (unchecked tasks) |
| `recon-reference-frame-store-refresh-flags` | `CONF-RECON-REFERENCE-FRAME-STORE-FUZZ`, `RECON-REFERENCE-FRAME-STORE` | in-progress (unchecked tasks) |
| `recon-residual-addition` | `RECON-RESIDUAL-ADDITION` | in-progress (unchecked tasks) |
| `recon-resolve-2d-transform-params` | `RECON-GET-TRANSFORM-1D-TYPE`, `RECON-INVERSE-TRANSFORM-2D-OUTER`, `RECON-RESOLVE-2D-TRANSFORM-PARAMS`, `RECON-TRANSFORM-SHIFT-LOOKUP` | in-progress (unchecked tasks) |
| `recon-secondary-inverse-transform` | `RECON-SECONDARY-INVERSE-TRANSFORM` | in-progress (unchecked tasks) |
| `recon-subpel-mc` | `RECON-SUBPEL-MC` | in-progress (unchecked tasks) |
| `recon-transform-matrix-free` | `RECON-INVERSE-TRANSFORM-1D`, `RECON-INVERSE-TRANSFORM-MATRIX-FREE` | in-progress (unchecked tasks) |
| `recon-wienerns-filter-primitive` | `RECON-WIENERNS-FILTER-PRIMITIVE` | in-progress (unchecked tasks) |
| `shared-spec-tables-crate` | `INFRA-SHARED-SPEC-TABLES` | in-progress (unchecked tasks) |
| `tile-txb-skip-context-derivation` | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | in-progress (unchecked tasks) |
| `tile-uv-mode-context-derivation` | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | in-progress (unchecked tasks) |
| `tile-y-mode-index-context-derivation` | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | in-progress (unchecked tasks) |
| `toy-intra-encoder-v0` | `CONF-AVM-DIFF-HARNESS`, `ENC-BITSTREAM-WRITER`, `ENC-INTRA-TOY-V0` | parked (encoder track behind validator roadmap fence) |
