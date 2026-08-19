// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// XDG paths, config, and JSON persistence for the rates cache.

use serde::{Deserialize, Serialize};
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::current::RateSnapshot;

pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

fn default_provider() -> String {
    "frankfurter".to_owned()
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    update_interval: String,
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default)]
    currencies: Vec<String>,
    #[serde(default)]
    multi_view: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            update_interval: "24h".to_owned(),
            provider: default_provider(),
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
    pub fn update_interval(&self) -> &str {
        &self.update_interval
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn currencies(&self) -> &[String] {
        &self.currencies
    }

    pub fn multi_view(&self) -> bool {
        self.multi_view.unwrap_or(true)
    }
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    env::var_os(var)
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
        .join("huobi")
        .join("config.json")
}

pub fn data_path() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
        .join("huobi")
        .join("rates.json")
}

pub fn history_db_path() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
        .join("huobi")
        .join("history.db")
}

pub fn save_json<T: Serialize>(path: &Path, value: &T, atomic: bool) -> Result<(), Box<dyn Error>> {
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

pub fn load_config() -> Config {
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

pub fn load_rates() -> Result<RateSnapshot, Box<dyn Error>> {
    let bytes = fs::read(data_path())?;
    serde_json::from_slice(&bytes)
        .map_err(|error| crate::boxed_error(format!("corrupted rate cache: {error}")))
}

pub fn save_rates(snapshot: &RateSnapshot) -> Result<(), Box<dyn Error>> {
    save_json(&data_path(), snapshot, true)
}

/// Parse a Go-style duration such as `24h` or `1h30m` into a [`Duration`].
pub fn parse_duration(value: &str) -> Option<Duration> {
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
        Duration::try_from_secs_f64(total).ok()
    } else {
        None
    }
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
    fn duration_parser_rejects_out_of_range_values() {
        assert!(parse_duration("999999999999999999999h").is_none());
    }
}
