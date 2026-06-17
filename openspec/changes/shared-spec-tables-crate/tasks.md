## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `INFRA-SHARED-SPEC-TABLES` to the implementation matrix.

## 2. Crate And Generator

- [x] 2.1 Add the dependency-free `splot-tables` crate (Cargo.toml + lib.rs) with workspace lints and no dependencies.
- [x] 2.2 Add per-module output routing (`output_dir_for`) to `cargo xtask gen-tables`; write and drift-check both output directories.
- [x] 2.3 Regenerate tables: the §9.6/§9.7 transform modules move to `splot-tables`; the other five stay in `splot-core`.
- [x] 2.4 Register the crate in the workspace, `[workspace.dependencies]`, `Cargo.lock`, and the `check-dependency-direction` rules.

## 3. Tests And Docs

- [x] 3.1 Relocate the `transform_1d` mirror cross-check spot test to `splot-tables`.
- [x] 3.2 Update `AGENTS.md` and `docs/ARCHITECTURE.md` crate maps and dependency rules.
- [x] 3.3 Confirm byte-identical regeneration (the 236-table determinism count is unchanged).

## 4. Gates, Review, And PR Discipline

- [x] 4.1 Update implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate shared-spec-tables-crate --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
