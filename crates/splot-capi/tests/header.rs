// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The committed C header must match what cbindgen generates from the crate.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn the_committed_header_matches_the_rust_api() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let config = cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml")).unwrap();
    let mut generated = Vec::new();
    cbindgen::generate_with_config(crate_dir, config)
        .unwrap()
        .write(&mut generated);
    let path = format!("{crate_dir}/include/splot.h");
    if std::env::var_os("SPLOT_BLESS_HEADER").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    assert_eq!(
        String::from_utf8(generated).unwrap(),
        std::fs::read_to_string(&path).unwrap_or_default(),
        "include/splot.h is stale; regenerate it with SPLOT_BLESS_HEADER=1 cargo test -p splot-capi --test header"
    );
}
