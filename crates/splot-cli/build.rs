// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Links the release `splot` binary on Linux without unwind tables.
//!
//! The release profile aborts on panic and never unwinds, but the tables stay:
//! the precompiled standard library carries them and LTO emits them for the
//! code merged with it, so `-C force-unwind-tables=no` removes nothing. Without
//! them a panic still prints its message and location, and `RUST_BACKTRACE`
//! prints no frames. A release test harness of the binary target is linked the
//! same way and aborts on the first failing test.

fn main() {
    println!("cargo::rerun-if-changed=no-unwind-tables.ld");
    let var = |name| std::env::var(name).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") == "linux" && var("PROFILE") == "release" {
        println!(
            "cargo::rustc-link-arg-bin=splot=-Wl,-T,{}/no-unwind-tables.ld",
            var("CARGO_MANIFEST_DIR")
        );
    }
}
