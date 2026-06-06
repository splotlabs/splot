# Contributing to splot

Thanks for your interest. `splot` is a solo-developer, source-available AV2 toolkit.
[AGENTS.md](./AGENTS.md) is the canonical guide; this file is the human-facing
summary.

## Development setup

- Install Rust **1.96.0** (edition 2024). The repo pins it via `rust-toolchain.toml`;
  with `rustup` the correct toolchain and components (`rustfmt`, `clippy`) install
  automatically.
- Clone, then run the acceptance gate:

  ```bash
  cargo xtask ci
  ```

## Acceptance commands

These must all pass before a change is complete:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo xtask ci
```

## Style rules

- **Library-first, thin CLI.** Logic lives in libraries; `splot-cli` is plumbing.
- **No `unwrap` / `expect` / `panic!` / `todo!` / `unimplemented!` in library code.**
  Return typed errors (`thiserror`) or structured diagnostics. `anyhow` is allowed
  only in `splot-cli` and `xtask`.
- **Public docs** on every public item; **SPDX header** on every `.rs` file.
- **Strong types** at public boundaries.
- Run `cargo fmt` before committing.

## Spec honesty

Never invent AV2 syntax, constants, or semantics. Cite the spec section for every
syntax element. If something is not yet modeled, leave a `// TODO(spec): …` marker
rather than guessing. The AV2 OBU header is § 5.2.2 — not AV1.

## Dependency policy

- Use current stable crate versions resolved by Cargo.
- Adding a new third-party dependency, or changing the crate dependency graph,
  requires maintainer sign-off. The one-way dependency direction is enforced by
  `cargo xtask check-dependency-direction`.

## Licensing of contributions

The repository is licensed under **PolyForm Noncommercial 1.0.0**. By contributing,
you agree your contributions are provided under the same license and grant the
copyright holder the rights needed to relicense (e.g. to offer commercial licenses).

> **TODO (legal):** finalize the contributor agreement (DCO sign-off or a CLA
> granting relicensing rights). Until then, open an issue before sending
> substantial contributions.
