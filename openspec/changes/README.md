# OpenSpec changes

Each subdirectory is one proposed change. Start a new change by copying the
template:

```bash
mkdir -p openspec/changes/<change-id>/specs/<area>
cp openspec/changes/.template/proposal.md openspec/changes/<change-id>/proposal.md
cp openspec/changes/.template/tasks.md openspec/changes/<change-id>/tasks.md
cp openspec/changes/.template/design.md openspec/changes/<change-id>/design.md
```

Then:

1. Fill in `proposal.md` (why, scope, non-goals, acceptance criteria) and list the
   Feature IDs from `docs/IMPLEMENTATION-MATRIX.toml`.
2. Record the `<change-id>` in each affected matrix row's `openspec_change` field.
3. Work through `tasks.md`.
4. Keep `design.md` for non-trivial design decisions; small changes may drop it.

A `<change-id>` is lowercase-kebab (for example, `parse-sequence-header`). Use the
same id in the matrix, the OpenSpec folder, and the GitHub issue/PR.

## Current changes

| Change | Feature IDs | State |
|---|---|---|
| `feature-tracking-framework` | `XTASK-FEATURE-STATUS`, `DOC-FEATURE-TRACKING` | implemented |
| `parse-annexb-and-obu-headers` | `AV2-4.11.6-LEB128`, `AV2-5.2.2-OBU-HEADER`, `AV2-5.2.1-OBU-TYPE`, `AV2-B-ANNEXB-OBU-ENVELOPE` | implemented |
| `add-bitstream-writer` | `ENC-BITSTREAM-WRITER` (+ `AV2-4.11.6-LEB128`, `AV2-5.2.2-OBU-HEADER`, `AV2-5.2.3-TRAILING-BITS`, `AV2-5.2.4-BYTE-ALIGNMENT` write stages) | proposed |
| `parse-sequence-header` | `AV2-5.4-SEQUENCE-HEADER` | proposed |
| `validator-coverage-roadmap` | validator coverage roadmap rows: descriptors, OBU dispatch, sequence-header children, §5.5-§5.17 top-level OBUs, frame-header/metadata/ordering children, Annex A/E, conformance, and docs rows | proposed |
| `avm-differential-harness` | `CONF-AVM-DIFF-HARNESS` | proposed |
| `toy-intra-encoder-v0` | `ENC-INTRA-TOY-V0` (deps: `ENC-BITSTREAM-WRITER`, `AV2-5.4-SEQUENCE-HEADER`, `AV2-5.18-FRAME-HEADER`, `AV2-5.19-TILE-GROUP`, `CONF-AVM-DIFF-HARNESS`) | proposed |
