// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Narrow `BLOCK_*` symbol resolver for generated AV2 § 9.2 partition-size tables.

use anyhow::{Result, bail};

const TABLES_WITH_BLOCK_SYMBOLS: &[&str] = &["H_Partition_Midsize", "Partition_Subsize"];

/// Resolves supported symbolic `BLOCK_*` table bodies into numeric bodies.
pub(super) fn resolve_body(table_name: &str, body: &str) -> Result<Option<String>> {
    if !TABLES_WITH_BLOCK_SYMBOLS.contains(&table_name) {
        return Ok(None);
    }

    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let symbol = &body[start..i];
            let Some(value) = block_size_symbol_value(symbol) else {
                bail!(
                    "gen-tables: unsupported symbol `{symbol}` in `{table_name}`; \
                     only AV2 block-size symbols are modeled"
                );
            };
            out.push_str(&value.to_string());
        } else {
            out.push(char::from(b));
            i += 1;
        }
    }

    Ok(Some(out))
}

// AV2 § 6.19.3 Table 6.22 defines valid block-size values 0..=28; AV2 § 3
// defines `BLOCK_INVALID = 29` and `BLOCK_SIZES = 29`.
fn block_size_symbol_value(symbol: &str) -> Option<i32> {
    Some(match symbol {
        "BLOCK_4X4" => 0,
        "BLOCK_4X8" => 1,
        "BLOCK_8X4" => 2,
        "BLOCK_8X8" => 3,
        "BLOCK_8X16" => 4,
        "BLOCK_16X8" => 5,
        "BLOCK_16X16" => 6,
        "BLOCK_16X32" => 7,
        "BLOCK_32X16" => 8,
        "BLOCK_32X32" => 9,
        "BLOCK_32X64" => 10,
        "BLOCK_64X32" => 11,
        "BLOCK_64X64" => 12,
        "BLOCK_64X128" => 13,
        "BLOCK_128X64" => 14,
        "BLOCK_128X128" => 15,
        "BLOCK_128X256" => 16,
        "BLOCK_256X128" => 17,
        "BLOCK_256X256" => 18,
        "BLOCK_4X16" => 19,
        "BLOCK_16X4" => 20,
        "BLOCK_8X32" => 21,
        "BLOCK_32X8" => 22,
        "BLOCK_16X64" => 23,
        "BLOCK_64X16" => 24,
        "BLOCK_4X32" => 25,
        "BLOCK_32X4" => 26,
        "BLOCK_8X64" => 27,
        "BLOCK_64X8" => 28,
        "BLOCK_INVALID" => 29,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_supported_block_size_table_body() -> Result<()> {
        assert_eq!(
            resolve_body(
                "Partition_Subsize",
                "{ BLOCK_4X4, BLOCK_64X8, BLOCK_INVALID }",
            )?,
            Some("{ 0, 28, 29 }".to_string())
        );
        Ok(())
    }

    #[test]
    fn ignores_other_symbolic_tables() -> Result<()> {
        assert_eq!(resolve_body("Tx_Size_Sqr", "{ TX_4X4 }")?, None);
        Ok(())
    }

    #[test]
    fn rejects_non_block_symbol_in_supported_table() {
        assert!(resolve_body("H_Partition_Midsize", "{ TX_4X4 }").is_err());
    }
}
