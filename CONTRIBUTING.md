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

`cargo xtask ci` is the single acceptance gate — run it before every change:

```bash
cargo xtask ci
```

It runs `fmt`, `clippy`, `build`, `test`, doctests (`cargo test --doc`), and the
repo checks, plus three external-tool checks: `typos`, `cargo machete
--with-metadata`, and `cargo deny check bans licenses sources`. Those three are
**external binaries** (not cargo dependencies). CI installs them so they always
gate; locally `cargo xtask ci` runs each one if present, otherwise it prints an
install hint and continues. To install them:

```bash
brew install typos-cli cargo-deny cargo-llvm-cov   # or the equivalent `cargo install`
cargo install cargo-machete cargo-fuzz --locked
```

The individual checks, to run directly:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo test --doc --workspace --locked
```

## Commit messages

Every commit subject and pull request title must use Conventional Commits:

```text
<type>[optional scope][!]: <description>
```

Allowed types are `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`,
`refactor`, `revert`, `style`, and `test`. Examples: `feat: add OBU parser`,
`fix(parser): reject truncated OBU headers`, `ci!: require conventional commits`.
CI checks every PR title with `cargo xtask check-conventional-title` and every PR
commit with `cargo xtask check-conventional-commits`.

Use squash or rebase merges only; GitHub's generated merge commits do not use
Conventional Commits subjects. Repository settings should keep "Allow merge
commits" disabled so push-to-main CI checks the resulting commit subject, not a
generated `Merge pull request ...` subject.

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
syntax element. If something is not yet modeled, leave a `// TODO(spec: <FEATURE-ID>): …`
marker referencing a row in `docs/IMPLEMENTATION-MATRIX.toml` rather than guessing
(`cargo xtask check-feature-status` rejects bare/unknown spec TODOs). The AV2 OBU
header is § 5.2.2 — not AV1.

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
