# Tasks

## 1. Baseline and Planning

- [x] 1.1 Record baseline inventory in `target/decoder-structure-audit.md`.
- [x] 1.2 Add `DECODE-RUNTIME-STRUCTURE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.3 Validate the OpenSpec change.

## 2. Mechanical Domain Moves

- [x] 2.1 Move stream planning and tile payload code under `bitstream/`.
- [x] 2.2 Move hash/raw/Y4M adapters under `output/`.
- [x] 2.3 Move runtime helpers under `pipeline/`, `prediction/`, `residual/`,
  `reference/`, `filters/`, `support/`, and `tile/`.
- [x] 2.4 Remove production `runtime_minimal` and `runtime_minimal_recon`
  modules from `lib.rs`.

## 3. Import and Naming Cleanup

- [x] 3.1 Rewrite old `crate::runtime_minimal*` paths to domain module paths.
- [x] 3.2 Replace old runtime type/function names at internal handoff points.
- [x] 3.3 Run `cargo fmt --all` and focused `splot-decode` tests.

## 4. Documentation

- [x] 4.1 Add `docs/DECISIONS/decoder-runtime-structure.md`.
- [x] 4.2 Update `README.md`, `AGENTS.md`, `docs/ARCHITECTURE.md`,
  `docs/DECODER-ARCHITECTURE.md`, and `docs/DECODER-ROADMAP.md`.
- [x] 4.3 Regenerate generated status docs with repo xtask commands.

## 5. Verification

- [x] 5.1 Run focused decoder and CLI decode tests.
- [x] 5.2 Run dependency, feature-status, source-line, comment, and duplication
  gates.
- [x] 5.3 Run `cargo xtask ci`.
- [x] 5.4 Run the old-name cleanup gate and record allowed historical matches.
