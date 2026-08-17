// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Command huobi is an offline currency conversion CLI.

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::time::Duration;

const API_URL: &str = "https://api.frankfurter.dev/v2/rates?base=EUR";
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_SIZE: usize = 1 << 20;
const DEFAULT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    #[serde(default)]
    update_interval: String,
    #[serde(default)]
    currencies: Vec<String>,
    #[serde(default)]
    multi_view: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            update_interval: "24h".to_owned(),
            multi_view: Some(true),
            currencies: vec![
                "USD", "EUR", "GBP", "JPY", "CNY", "HKD", "CHF", "AUD", "CAD", "SGD", "SEK", "NOK",
                "DKK", "PLN", "CZK", "HUF", "RON", "KRW", "INR", "IDR", "MYR", "PHP", "THB", "ILS",
                "ISK", "TRY", "MXN", "BRL", "ZAR", "NZD",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }
}

impl Config {
    fn multi_view(&self) -> bool {
        self.multi_view.unwrap_or(true)
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct RateSnapshot {
    base: String,
    date: String,
    fetched_at: DateTime<Utc>,
    rates: std::collections::HashMap<String, f64>,
}

#[derive(Debug)]
struct HuobiError(String);

impl fmt::Display for HuobiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for HuobiError {}

fn boxed_error(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(HuobiError(message.into()))
}

fn config_path() -> PathBuf {
    let dir = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join("huobi").join("config.json")
}

fn data_path() -> PathBuf {
    let dir = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join("huobi").join("rates.json")
}

fn save_json<T: Serialize>(path: &Path, value: &T, atomic: bool) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    if atomic {
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, bytes)?;
        fs::rename(temp, path)?;
    } else {
        fs::write(path, bytes)?;
    }
    Ok(())
}

fn load_config() -> Config {
    let path = config_path();
    match fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("warning: malformed config: {error}, using defaults");
                Config::default()
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let config = Config::default();
            if let Err(error) = save_json(&path, &config, false) {
                eprintln!("warning: failed to write default config: {error}");
            }
            config
        }
        Err(error) => {
            eprintln!("warning: failed to read config: {error}, using defaults");
            Config::default()
        }
    }
}

fn load_rates() -> Result<RateSnapshot, Box<dyn Error>> {
    let bytes = fs::read(data_path())?;
    serde_json::from_slice(&bytes)
        .map_err(|error| boxed_error(format!("corrupted rate cache: {error}")))
}

fn save_rates(snapshot: &RateSnapshot) -> Result<(), Box<dyn Error>> {
    save_json(&data_path(), snapshot, true)
}

#[derive(Debug, Deserialize)]
struct ApiRate {
    date: String,
    base: String,
    quote: String,
    rate: f64,
}

fn fetch_rates() -> Result<RateSnapshot, Box<dyn Error>> {
    let client = Client::builder().timeout(HTTP_TIMEOUT).build()?;
    let response = client.get(API_URL).send()?;
    let status = response.status();
    let bytes = response.bytes()?.to_vec();
    if bytes.len() > MAX_RESPONSE_SIZE {
        return Err(boxed_error("API response exceeded 1 MiB"));
    }
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes);
        return Err(boxed_error(format!(
            "API returned {status}: {}",
            body.trim()
        )));
    }
    let rows: Vec<ApiRate> = serde_json::from_slice(&bytes)
        .map_err(|error| boxed_error(format!("failed to parse API response: {error}")))?;
    let first = rows
        .first()
        .ok_or_else(|| boxed_error("API response contained no rates"))?;
    let base = first.base.to_uppercase();
    let date = first.date.clone();
    let rates = rows
        .into_iter()
        .map(|row| (row.quote.to_uppercase(), row.rate))
        .collect();
    Ok(RateSnapshot {
        base,
        date,
        fetched_at: Utc::now(),
        rates,
    })
}

fn currency_rate(snapshot: &RateSnapshot, currency: &str) -> Result<f64, Box<dyn Error>> {
    if currency == snapshot.base {
        return Ok(1.0);
    }
    snapshot.rates.get(currency).copied().ok_or_else(|| {
        boxed_error(format!(
            "no rate for currency {currency} (rates date {})",
            snapshot.date
        ))
    })
}

fn convert(
    snapshot: &RateSnapshot,
    source: &str,
    target: &str,
    amount: f64,
) -> Result<f64, Box<dyn Error>> {
    let source_rate = currency_rate(snapshot, source)?;
    let target_rate = currency_rate(snapshot, target)?;
    Ok(amount * target_rate / source_rate)
}

fn parse_duration(value: &str) -> Option<Duration> {
    let mut total = 0.0_f64;
    let mut rest = value;
    while !rest.is_empty() {
        let digit_count = rest
            .char_indices()
            .find(|(_, ch)| !ch.is_ascii_digit() && *ch != '.')
            .map(|(index, _)| index)
            .unwrap_or(rest.len());
        if digit_count == 0 {
            return None;
        }
        let number: f64 = rest[..digit_count].parse().ok()?;
        rest = &rest[digit_count..];
        let units = ["ns", "us", "µs", "ms", "s", "m", "h"];
        let unit = units.iter().find(|unit| rest.starts_with(**unit))?;
        let multiplier = match *unit {
            "ns" => 1e-9,
            "us" | "µs" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            _ => unreachable!(),
        };
        total += number * multiplier;
        rest = &rest[unit.len()..];
    }
    if total.is_finite() && total > 0.0 {
        Some(Duration::from_secs_f64(total))
    } else {
        None
    }
}

fn dedupe_targets(currencies: &[String], source: &str, exclude: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    seen.insert(source.to_owned());
    for currency in exclude {
        seen.insert(currency.to_uppercase());
    }
    currencies
        .iter()
        .map(|currency| currency.to_uppercase())
        .filter(|currency| seen.insert(currency.clone()))
        .collect()
}

fn usage() {
    eprintln!(
        "Usage: huobi [options] AMOUNT SOURCE [TARGET...]

Offline currency converter. With no targets, shows the multi-currency view;
explicit targets are listed first, followed by the default multi-currency list.

Options:
  -u, --update  force-refresh rates (ignore cache age)"
    );
}

fn parse_args() -> Result<(bool, f64, String, Vec<String>), i32> {
    let mut force = false;
    let mut positional = Vec::new();
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "-u" | "--update" => force = true,
            "-h" | "--help" => {
                usage();
                return Err(0);
            }
            _ if arg.starts_with('-') && positional.is_empty() => {
                eprintln!("error: unknown option {arg}");
                usage();
                return Err(2);
            }
            _ => positional.push(arg),
        }
    }
    if positional.len() < 2 {
        usage();
        return Err(2);
    }
    let amount = positional[0].parse::<f64>().map_err(|_| {
        eprintln!("error: invalid amount {:?}", positional[0]);
        2
    })?;
    let source = positional[1].to_uppercase();
    let targets = positional[2..]
        .iter()
        .map(|target| target.to_uppercase())
        .collect();
    Ok((force, amount, source, targets))
}

struct Row {
    code: String,
    value: f64,
}

fn main() {
    let (force, amount, source, explicit) = match parse_args() {
        Ok(args) => args,
        Err(code) => process::exit(code),
    };
    let config = load_config();
    let interval = if config.update_interval.is_empty() {
        DEFAULT_INTERVAL
    } else {
        match parse_duration(&config.update_interval) {
            Some(duration) => duration,
            None => {
                eprintln!(
                    "warning: invalid update_interval={:?} in config, falling back to 24h",
                    config.update_interval
                );
                DEFAULT_INTERVAL
            }
        }
    };

    let mut snapshot = match load_rates() {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            if error.downcast_ref::<io::Error>().is_none() {
                eprintln!("warning: failed to read local rates: {error}");
            }
            None
        }
    };
    let stale = snapshot.as_ref().is_none_or(|snapshot| {
        Utc::now()
            .signed_duration_since(snapshot.fetched_at)
            .to_std()
            .map(|age| age > interval)
            .unwrap_or(false)
    });
    let mut updated = false;
    if force || stale {
        match fetch_rates() {
            Ok(fresh) => {
                if let Err(error) = save_rates(&fresh) {
                    eprintln!("warning: failed to save rates cache: {error}");
                }
                snapshot = Some(fresh);
                updated = true;
            }
            Err(error) if force => fatal(&format!("failed to update rates: {error}")),
            Err(error) if snapshot.is_none() => {
                fatal(&format!("no local rate cache and update failed: {error}"))
            }
            Err(error) => {
                let cached = snapshot.as_ref().expect("cache exists in fallback branch");
                eprintln!(
                    "warning: failed to update rates: {error}; using cached rates (date {})",
                    cached.date
                );
            }
        }
    }
    let snapshot = snapshot.expect("fatal exits when no snapshot is available");
    if let Err(error) = currency_rate(&snapshot, &source) {
        fatal(&error.to_string());
    }

    let convert_list = |list: Vec<String>| {
        list.into_iter()
            .filter_map(|code| match convert(&snapshot, &source, &code, amount) {
                Ok(value) => Some(Row { code, value }),
                Err(error) => {
                    eprintln!("warning: {error}, skipped");
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    let explicit_rows = convert_list(dedupe_targets(&explicit, &source, &[]));
    let multi_rows = if explicit_rows.is_empty() || config.multi_view() {
        let excluded = explicit_rows
            .iter()
            .map(|row| row.code.clone())
            .collect::<Vec<_>>();
        convert_list(dedupe_targets(&config.currencies, &source, &excluded))
    } else {
        Vec::new()
    };

    let amount_string = format!("{amount:.2}");
    let padding = amount_string.len() + source.len() + 4;
    let value_width = explicit_rows
        .iter()
        .chain(multi_rows.iter())
        .map(|row| format!("{:.2}", row.value).len())
        .max()
        .unwrap_or(0);
    let indent = " ".repeat(padding);
    let value_string = |value: f64| format!("{:>width$.2}", value, width = value_width);

    for (index, row) in explicit_rows.iter().enumerate() {
        if index == 0 {
            println!(
                "{amount_string} {source} = {} {}",
                value_string(row.value),
                row.code
            );
        } else {
            println!("{indent}{} {}", value_string(row.value), row.code);
        }
    }
    if !multi_rows.is_empty() {
        if !explicit_rows.is_empty() {
            println!("{}", "-".repeat(padding + value_width + 4));
        }
        for (index, row) in multi_rows.iter().enumerate() {
            if index == 0 && explicit_rows.is_empty() {
                println!(
                    "{amount_string} {source} = {} {}",
                    value_string(row.value),
                    row.code
                );
            } else {
                println!("{indent}{} {}", value_string(row.value), row.code);
            }
        }
    }
    if updated {
        println!("rates updated: {}", snapshot.date);
    } else {
        println!("rates date {}", snapshot.date);
    }
}

fn fatal(message: &str) -> ! {
    eprintln!("error: {message}");
    process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_parser_supports_compound_go_durations() {
        assert_eq!(parse_duration("1h30m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("24h"), Some(Duration::from_secs(86400)));
        assert_eq!(parse_duration("0s"), None);
        assert_eq!(parse_duration("tomorrow"), None);
    }

    #[test]
    fn targets_are_uppercased_deduplicated_and_excluded() {
        let currencies = vec![
            "usd".to_owned(),
            "EUR".to_owned(),
            "eur".to_owned(),
            "GBP".to_owned(),
        ];
        let excluded = vec!["gbp".to_owned()];
        assert_eq!(dedupe_targets(&currencies, "USD", &excluded), vec!["EUR"]);
    }
}
