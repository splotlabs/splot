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
- **Preserve the dependency direction:** `splot-core` depends on no `splot-*` crate;
  nothing depends on `splot-cli`; only `splot-cli` depends on `splot-encode`; `xtask`
  is standalone. Enforced by `cargo xtask check-dependency-direction`.
- **SPDX header** on every `.rs` file; **public docs** on every public item.
- **Feature tracking:** use Feature IDs from `docs/IMPLEMENTATION-MATRIX.toml` (the
  canonical status) for non-trivial work. Do not create TODOs without
  `TODO(spec: FEATURE-ID)`. Do not mark implementation stages as done unless proof
  is recorded. See [../docs/FEATURE-TRACKING.md](../docs/FEATURE-TRACKING.md).

Validate work with `cargo xtask ci` (which runs `cargo xtask check-feature-status`).
