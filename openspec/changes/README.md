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
| `add-bitstream-writer` | `ENC-BITSTREAM-WRITER` (+ `AV2-4.11.6-LEB128`, `AV2-5.2.2-OBU-HEADER`, `AV2-5.2.3-TRAILING-BITS`, `AV2-5.2.4-BYTE-ALIGNMENT` write stages) | proposed |
| `avm-differential-harness` | `CONF-AVM-DIFF-HARNESS` | proposed |
| `ci-quality-gates` | `XTASK-CI-QUALITY-GATES` (+ `CONF-FUZZ-NO-PANIC` note fix) | in progress |
| `toy-intra-encoder-v0` | `ENC-INTRA-TOY-V0` (deps: `ENC-BITSTREAM-WRITER`, `AV2-5.4-SEQUENCE-HEADER`, `AV2-5.18-FRAME-HEADER`, `AV2-5.19-TILE-GROUP`, `CONF-AVM-DIFF-HARNESS`) | proposed |
