# Tasks: dispatch frame-carrying OBU payloads

## 1. Bookkeeping

- [x] 1.1 Matrix rows confirmed; `openspec_change` set; ground what is
  state-free per frame-carrying type (§ 5.18.1/.2 prefix, § 5.19
  prefix) vs state-dependent.

## 2. Implementation

- [x] 2.1 Dispatch arms for the 11 types: state-free prefix parsed, an
  honest state-dependent status for the rest (no blanket
  Unimplemented).
- [x] 2.2 Inspector/dispatch status consistency documented and tested.

## 3. Docs

- [x] 3.1 Matrix proof; generated docs; roadmap.

## 4. Verification

- [x] 4.1 Per-type positive/EOF tests; inspect snapshot.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
