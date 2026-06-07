# OpenSpec changes for splot

This directory holds **change intent**: why a feature exists, what is in and out of
scope, and the acceptance criteria. It is compatible with the
[OpenSpec](https://github.com/Fission-AI/OpenSpec) layout, but the OpenSpec CLI is
**optional** — normal CI does not require it.

OpenSpec is **not** the status source of truth. The canonical status of every
feature lives in [`docs/IMPLEMENTATION-MATRIX.toml`](../docs/IMPLEMENTATION-MATRIX.toml)
and is enforced by `cargo xtask check-feature-status`.

## Layout

```text
openspec/
  specs/            current capability specs (bitstream, validator, encoder, conformance)
  changes/          proposed changes; each is a folder with proposal/tasks/design
  templates/change/ copy this to start a new change
```

## Rules

- Every non-trivial feature should start with a change under
  `openspec/changes/<change-id>/`.
- Every change lists the **Feature IDs** it touches, taken from
  `docs/IMPLEMENTATION-MATRIX.toml`.
- A change should be small enough for one PR when practical. Split large AV2
  syntax (for example, the frame header) into several changes.
- A change records design intent and acceptance criteria; it does **not** restate
  the matrix. Update the matrix row(s) as part of the change.

## Validating (optional)

```bash
if command -v openspec >/dev/null 2>&1; then
  openspec validate --all --no-interactive
else
  echo "openspec not installed; skipping OpenSpec CLI validation"
fi
```
