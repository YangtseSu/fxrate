// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Currency display metadata resolved from the ISO 4217 data in the
// `iso_currency` crate: the symbol, the English name, and a derived flag
// emoji. Both rate providers (Frankfurter, exchange-api) emit standard ISO
// 4217 codes, which this crate covers in full.

use iso_currency::Currency;

/// Display metadata for a currency code.
///
/// `symbol` and `flag` are empty strings for unknown codes (the caller still
/// prints the raw code, so nothing is lost). `flag` is also suppressed for
/// supranational/special currencies that have no single country flag.
pub struct CurrencyMeta {
    pub symbol: String,
    pub name: String,
    pub flag: String,
}

/// Resolve display metadata for an ISO 4217 `code`.
pub fn meta(code: &str) -> CurrencyMeta {
    match Currency::from_code(code) {
        Some(currency) => CurrencyMeta {
            symbol: currency.symbol().to_string(),
            name: currency.name().to_string(),
            flag: if currency.is_special() || currency.is_fund() {
                String::new()
            } else {
                flag_emoji(currency.code())
            },
        },
        None => CurrencyMeta {
            symbol: String::new(),
            name: String::new(),
            flag: String::new(),
        },
    }
}

/// Map an ISO 4217 alpha-3 code to a flag emoji by converting its first two
/// letters to regional indicator symbols (e.g. "USD" -> "US" -> 🇺🇸,
/// "EUR" -> "EU" -> 🇪🇺).
///
/// ISO 4217 reserves the "X" prefix for supranational/special codes (XAU gold,
/// XCD, XOF, ...) that have no country flag, so those return an empty string
/// rather than a broken glyph.
fn flag_emoji(alpha3: &str) -> String {
    if alpha3.starts_with('X') {
        return String::new();
    }
    let prefix: Vec<char> = alpha3.chars().take(2).collect();
    if prefix.len() != 2 || !prefix.iter().all(|c| c.is_ascii_alphabetic()) {
        return String::new();
    }
    let to_indicator = |c: char| -> char {
        char::from_u32(0x1F1E6 + (c.to_ascii_uppercase() as u32 - b'A' as u32))
            .unwrap_or('\u{FFFD}')
    };
    let mut flag = String::with_capacity(8);
    flag.push(to_indicator(prefix[0]));
    flag.push(to_indicator(prefix[1]));
    flag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_known_currency_symbol_and_flag() {
        let m = meta("USD");
        assert_eq!(m.symbol, "$");
        assert!(m.name.contains("dollar"));
        assert_eq!(m.flag, "\u{1F1FA}\u{1F1F8}"); // 🇺🇸
    }

    #[test]
    fn euro_gets_eu_flag() {
        let m = meta("EUR");
        assert_eq!(m.flag, "\u{1F1EA}\u{1F1FA}"); // 🇪🇺
    }

    #[test]
    fn supranational_currency_has_no_flag() {
        let m = meta("XAU");
        assert!(m.flag.is_empty());
        assert!(!m.name.is_empty());
    }

    #[test]
    fn unknown_currency_is_empty() {
        let m = meta("ZZZ");
        assert!(m.symbol.is_empty());
        assert!(m.name.is_empty());
        assert!(m.flag.is_empty());
    }
}
