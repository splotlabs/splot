# Proposal: parse loop-restoration and CCSO frame-header params

## Feature IDs

- `AV2-5.18.7-SEGMENTATION-TILING` (lr_params § 5.18.7.11, ccso_params
  § 5.18.7.12 — the § 5.18.7 home row)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the intra-path stop point advances)

## Why

`lr_params()` (mirror `05-syntax-structures.md`:7097) and `ccso_params()`
(:7424) are the next two structures in the § 5.18.2 post-quantization tail
after the PR #57 filter cluster (call sites :5303-5305) and the two largest
filtering syntax bodies. Parsing them moves the intra-path stop point to
`read_tx_mode()` and clears the next residual on the § 5.18.7 row.

## What Changes

1. Parse `lr_params()` per § 5.18.7.11, gated on the parsed sequence
   restoration config exactly as the mirror prescribes (per-plane unit
   sizes/types per the grammar).
2. Parse `ccso_params()` per § 5.18.7.12, gated on the parsed `enable_ccso`
   sequence config.
3. Advance the intra-path stop status to the structure after
   `ccso_params()` in the § 5.18.2 tail (verify against the mirror —
   expected `read_tx_mode()`); EOF inside the new cluster preserves
   already-parsed facts per the PR #57 `StoppedInsideFilterParams`
   precedent (reuse or extend that status honestly).
4. `inspect` surfaces the new structures; the synced OpenSpec main-spec
   stop-point requirement is updated in the same PR (PR #57 lesson — no
   stale contradictory requirement).
5. Any § 6 bound on these fields that is locally decidable and unambiguous
   gets a diagnostic with citation; otherwise named residual.

## Non-goals

- Inter-path parsing; reconstruction semantics.
- read_tx_mode() and beyond (next changes).

## Acceptance criteria

- [ ] Both structures parse on the intra path; positive/negative/EOF tests
  per structure, every gating flag both ways; facts preserved on EOF
  inside the cluster; no constructed-view panic paths (PR #57 lesson:
  audit shifts/indexing).
- [ ] Stop-point move tested; OpenSpec main-spec consistent; matrix proof
  recorded; `cargo xtask ci` green.
