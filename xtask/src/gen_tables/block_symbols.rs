// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Narrow symbol resolver for generated AV2 § 9.2 conversion tables.

use anyhow::{Result, bail};

const TABLES_WITH_BLOCK_SYMBOLS: &[&str] = &["H_Partition_Midsize", "Partition_Subsize"];
const TABLES_WITH_TX_SIZE_SYMBOLS: &[&str] = &["Adjusted_Tx_Size", "Tx_Size_Sqr", "Tx_Size_Sqr_Up"];
const TABLES_WITH_TX_TYPE_SYMBOLS: &[&str] = &["Mode_To_Txfm"];

/// Resolves supported symbolic table bodies into numeric bodies.
pub(super) fn resolve_body(table_name: &str, body: &str) -> Result<Option<String>> {
    let resolver = if TABLES_WITH_BLOCK_SYMBOLS.contains(&table_name) {
        block_size_symbol_value
    } else if TABLES_WITH_TX_SIZE_SYMBOLS.contains(&table_name) {
        tx_size_symbol_value
    } else if TABLES_WITH_TX_TYPE_SYMBOLS.contains(&table_name) {
        tx_type_symbol_value
    } else {
        return Ok(None);
    };

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
            let Some(value) = resolver(symbol) else {
                bail!(
                    "gen-tables: unsupported symbol `{symbol}` in `{table_name}`; \
                     this table's symbolic enum domain is not modeled"
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

// AV2 § 6.19.6.1 defines TxSize values 0..=24 and `TX_INVALID = 255`.
fn tx_size_symbol_value(symbol: &str) -> Option<i32> {
    Some(match symbol {
        "TX_4X4" => 0,
        "TX_8X8" => 1,
        "TX_16X16" => 2,
        "TX_32X32" => 3,
        "TX_64X64" => 4,
        "TX_4X8" => 5,
        "TX_8X4" => 6,
        "TX_8X16" => 7,
        "TX_16X8" => 8,
        "TX_16X32" => 9,
        "TX_32X16" => 10,
        "TX_32X64" => 11,
        "TX_64X32" => 12,
        "TX_4X16" => 13,
        "TX_16X4" => 14,
        "TX_8X32" => 15,
        "TX_32X8" => 16,
        "TX_16X64" => 17,
        "TX_64X16" => 18,
        "TX_4X32" => 19,
        "TX_32X4" => 20,
        "TX_8X64" => 21,
        "TX_64X8" => 22,
        "TX_4X64" => 23,
        "TX_64X4" => 24,
        "TX_INVALID" => 255,
        _ => return None,
    })
}

// AV2 § 3 Table 3.1 defines TxType values 0..=15 and `TX_TYPES = 16`.
fn tx_type_symbol_value(symbol: &str) -> Option<i32> {
    Some(match symbol {
        "DCT_DCT" => 0,
        "ADST_DCT" => 1,
        "DCT_ADST" => 2,
        "ADST_ADST" => 3,
        "FLIPADST_DCT" => 4,
        "DCT_FLIPADST" => 5,
        "FLIPADST_FLIPADST" => 6,
        "ADST_FLIPADST" => 7,
        "FLIPADST_ADST" => 8,
        "IDTX" => 9,
        "V_DCT" => 10,
        "H_DCT" => 11,
        "V_ADST" => 12,
        "H_ADST" => 13,
        "V_FLIPADST" => 14,
        "H_FLIPADST" => 15,
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
    fn resolves_supported_tx_size_table_body() -> Result<()> {
        assert_eq!(
            resolve_body(
                "Adjusted_Tx_Size",
                "{ TX_4X4, TX_64X64, TX_4X64, TX_64X4, TX_INVALID }",
            )?,
            Some("{ 0, 4, 23, 24, 255 }".to_string())
        );
        Ok(())
    }

    #[test]
    fn resolves_supported_tx_type_table_body() -> Result<()> {
        assert_eq!(
            resolve_body(
                "Mode_To_Txfm",
                "{ DCT_DCT, ADST_DCT, DCT_ADST, ADST_ADST, IDTX, H_FLIPADST }",
            )?,
            Some("{ 0, 1, 2, 3, 9, 15 }".to_string())
        );
        Ok(())
    }

    #[test]
    fn rejects_non_block_symbol_in_supported_table() {
        assert!(resolve_body("H_Partition_Midsize", "{ TX_4X4 }").is_err());
    }

    #[test]
    fn rejects_non_tx_size_symbol_in_supported_table() {
        assert!(resolve_body("Tx_Size_Sqr", "{ BLOCK_4X4 }").is_err());
    }

    #[test]
    fn rejects_non_tx_type_symbol_in_supported_table() {
        assert!(resolve_body("Mode_To_Txfm", "{ TX_4X4 }").is_err());
    }

    #[test]
    fn ignores_other_symbolic_tables() -> Result<()> {
        assert_eq!(resolve_body("Max_Tx_Size_Rect", "{ TX_4X4 }")?, None);
        Ok(())
    }
}
