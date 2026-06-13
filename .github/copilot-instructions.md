# GitHub Copilot instructions for splot

The canonical guide is [AGENTS.md](../AGENTS.md). Read it. The most critical rules:

- **Toolchain:** Rust **1.96.0**, edition **2024**, resolver **3**.
- **AV2, not AV1:** the OBU header is AV2 v1.0.0 § 5.2.2. Never copy AV1 OBU header
  fields, the AV1 OBU type table, `obu_forbidden_bit`, or `obu_has_size_field`.
  Never invent AV2 syntax — leave `// TODO(spec: <FEATURE-ID>): …` (a matrix id)
  instead; `cargo xtask check-feature-status` rejects bare/unknown spec TODOs.
- **No panics in libraries:** no `unwrap` / `expect` / `panic!` / `todo!` /
  `unimplemented!` reachable in library code. Stubs return
  `Error::Unimplemented { feature }`. `anyhow` is only for `splot-cli` and `xtask`.
- **Diagnostics are structured data:** every validator finding needs a stable
  `rule_id`, `severity`, `spec_section`, byte/bit offset, and `message`.
- **Preserve the dependency direction:** `splot-core` and `splot-recon` depend on
  no `splot-*` crate; `splot-decode` may depend only on `splot-core` and
  `splot-recon` once it has decode source needing them; `splot-cli` may depend
  on `splot-core`, `splot-decode`, `splot-validate`, and `splot-encode`;
  nothing depends on `splot-cli`; only `splot-cli` depends on `splot-encode`;
  `xtask` is standalone. Enforced by `cargo xtask check-dependency-direction`.
- **SPDX header** on every `.rs` file; **public docs** on every public item.
- **Feature tracking:** use Feature IDs from `docs/IMPLEMENTATION-MATRIX.toml` (the
  canonical status) for non-trivial work. Do not create TODOs without
  `TODO(spec: FEATURE-ID)`. Do not mark implementation stages as done unless proof
  is recorded. See [../docs/FEATURE-TRACKING.md](../docs/FEATURE-TRACKING.md).
- **Audit protocols:** documentation audits use `DOC-AUDIT-PROTOCOLS`; heavy AV2
  conformance audits start with `cargo xtask audit-scope --format json`
  (`XTASK-AUDIT-SCOPE`) so current and future workspace members are selected
  deterministically. See [../AGENTS.md](../AGENTS.md).
- **Commit messages:** every commit subject and pull request title must use
  Conventional Commits; CI enforces this with `cargo xtask check-conventional-title`
  and `cargo xtask check-conventional-commits`.

## Encoder research references

Before suggesting encoder implementation code, consult:

- `docs/references/ENCODER-RESEARCH-NOTES.md`
- `docs/references/THIRD-PARTY-NOTICES.md`
- `docs/references/RAV1E-SOURCE-MAP.md` for rav1e-inspired Rust/RDO/API/tiling ideas
- `docs/references/SVT-AV1-RESEARCH-MAPPING.md` for SVT-inspired pipeline/ME/RC/filter ideas

Use rav1e and SVT-AV1 only as architecture inspiration. Do not copy AV1 syntax, source code, tables,
constants, entropy CDFs, comments, or prose. AV2 syntax and decoder-visible behavior must be derived
from the AV2 specification and AVM.

Validate work with `cargo xtask ci` (which runs `cargo xtask check-feature-status`).
