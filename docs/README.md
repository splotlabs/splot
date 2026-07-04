# Documentation

Retained human docs are intentionally small:

- [../README.md](../README.md) - user entry point and commands
- [../AGENTS.md](../AGENTS.md) - contributor and coding-agent rules
- [ARCHITECTURE.md](./ARCHITECTURE.md) - crate boundaries and runtime policies
- [TESTING.md](./TESTING.md) - test layers and proof rules
- [CONFORMANCE.md](./CONFORMANCE.md) - conformance claim policy
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) - CI-enforced diagnostic registry input
- [DECISIONS/](./DECISIONS/) - durable architectural decisions
- [references/THIRD-PARTY-NOTICES.md](./references/THIRD-PARTY-NOTICES.md) -
  legal and source-boundary notices

Machine-readable sources of truth:

- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `docs/LOCAL-REFERENCE-EVIDENCE.toml`
- `docs/audits/av2-conformance-ledger.json`
- `tests/conformance/manifest.toml`
- `tests/conformance/decoder-oracle.toml`
- `tests/conformance/decoder-oracle-coverage.toml`

Generated markdown status reports are not committed. Use:

```bash
cargo xtask feature-status
cargo xtask spec-coverage
cargo xtask decoder-support
cargo xtask decoder-conformance-coverage
cargo xtask decoder-fixtures coverage
```
