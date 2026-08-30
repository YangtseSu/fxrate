// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// End-to-end tests: convert regression and chart command, all offline.
// Every subprocess gets a fresh XDG home and a blocked network
// (HTTPS_PROXY to a closed port) so tests never touch the network.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fxrate-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("fxrate")).unwrap();
    dir
}

fn run(home: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fxrate"))
        .args(args)
        .env("XDG_DATA_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("TERM", "dumb")
        .output()
        .expect("fxrate binary should run")
}

fn seed_rates(home: &Path) {
    let rates = serde_json::json!({
        "base": "EUR",
        "date": "2026-08-17",
        "fetched_at": "2026-08-17T12:00:00Z",
        "provider": "frankfurter",
        "rates": {"USD": 1.1, "EUR": 1.0, "CNY": 7.8}
    });
    std::fs::write(home.join("fxrate").join("rates.json"), rates.to_string()).unwrap();
}

const SCHEMA: &str = "CREATE TABLE historical_rates (
    provider   TEXT NOT NULL,
    date       TEXT NOT NULL,
    quote      TEXT NOT NULL,
    rate       REAL NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (provider, date, quote)
);
CREATE INDEX historical_rates_lookup
    ON historical_rates(provider, quote, date);
CREATE TABLE history_coverage (
    provider   TEXT NOT NULL,
    start_date TEXT NOT NULL,
    end_date   TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (provider, start_date, end_date)
);";

/// Seed a covered 2025-01-02..2025-01-06 window for USD/CNY.
fn seed_history(home: &Path, provider: &str) {
    let conn = rusqlite::Connection::open(home.join("fxrate").join("history.db")).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    let rows = [
        ("2025-01-02", 1.0322, 7.5338),
        ("2025-01-03", 1.0317, 7.5371),
        ("2025-01-06", 1.0435, 7.6284),
    ];
    for (day, usd, cny) in rows {
        conn.execute(
            "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
             VALUES (?1, ?2, 'USD', ?3, 't')",
            rusqlite::params![provider, day, usd],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
             VALUES (?1, ?2, 'CNY', ?3, 't')",
            rusqlite::params![provider, day, cny],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO history_coverage (provider, start_date, end_date, fetched_at)
         VALUES (?1, '2025-01-02', '2025-01-06', 't')",
        rusqlite::params![provider],
    )
    .unwrap();
}

/// Seed ECB history through 2025-01-10 (a Friday); coverage ends that day.
fn seed_history_jan10(home: &Path) {
    let conn = rusqlite::Connection::open(home.join("fxrate").join("history.db")).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    let rows = [
        ("2025-01-02", 1.0322, 7.5338),
        ("2025-01-03", 1.0317, 7.5371),
        ("2025-01-06", 1.0435, 7.6284),
        ("2025-01-07", 1.0400, 7.6100),
        ("2025-01-08", 1.0450, 7.6400),
        ("2025-01-09", 1.0480, 7.6600),
        ("2025-01-10", 1.0500, 7.6800),
    ];
    for (day, usd, cny) in rows {
        conn.execute(
            "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
             VALUES ('ecb', ?1, 'USD', ?2, 't')",
            rusqlite::params![day, usd],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
             VALUES ('ecb', ?1, 'CNY', ?2, 't')",
            rusqlite::params![day, cny],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO history_coverage (provider, start_date, end_date, fetched_at)
         VALUES ('ecb', '2025-01-02', '2025-01-10', 't')",
        [],
    )
    .unwrap();
}

/// Seed a rates.json snapshot dated `date` (USD 1.06, CNY 7.8 per EUR).
fn seed_live_rates(home: &Path, date: &str) {
    let rates = serde_json::json!({
        "base": "EUR",
        "date": date,
        "fetched_at": format!("{date}T12:00:00Z"),
        "provider": "frankfurter",
        "rates": {"USD": 1.06, "EUR": 1.0, "CNY": 7.8}
    });
    std::fs::write(home.join("fxrate").join("rates.json"), rates.to_string()).unwrap();
}

#[test]
fn convert_works_offline_from_seeded_cache() {
    let home = temp_home("convert");
    seed_rates(&home);
    let out = run(&home, &["100", "USD", "CNY"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("$100.00 USD =")
            && stdout.contains("709.09 CNY")
            && stdout.contains("\u{1F1E8}\u{1F1F3}"),
        "{stdout}"
    );
    assert!(stdout.contains("rates date 2026-08-17"), "{stdout}");
}

#[test]
fn convert_usage_errors_exit_2() {
    let home = temp_home("usage");
    assert_eq!(
        run(&home, &["-p", "ecb", "100", "USD"]).status.code(),
        Some(2)
    );
    assert_eq!(run(&home, &["notanumber", "USD"]).status.code(), Some(2));
    assert_eq!(run(&home, &["100"]).status.code(), Some(2));
    assert_eq!(run(&home, &["--help"]).status.code(), Some(0));
    let version = run(&home, &["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("fxrate {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn convert_accepts_provider_aliases() {
    let home = temp_home("alias");
    seed_rates(&home);
    for name in ["exchange-api", "exchangeapi", "EXCHANGE-API"] {
        let out = run(&home, &["-p", name, "100", "USD", "CNY"]);
        assert!(
            out.status.success(),
            "-p {name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn convert_date_uses_ecb_historical_rates() {
    let home = temp_home("conv-date");
    seed_history(&home, "ecb");
    let out = run(&home, &["--date", "2025-01-02", "100", "USD", "CNY"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("$100.00 USD =")
            && stdout.contains("729.88 CNY")
            && stdout.contains("\u{1F1E8}\u{1F1F3}"),
        "{stdout}"
    );
    assert!(stdout.contains("rates date 2025-01-02"), "{stdout}");
}

#[test]
fn convert_date_without_local_data_and_no_network_exits_1() {
    let home = temp_home("conv-date-miss");
    seed_history(&home, "ecb");
    let out = run(&home, &["--date", "2020-01-01", "100", "USD", "CNY"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn convert_date_weekend_uses_previous_business_day() {
    let home = temp_home("conv-date-weekend");
    seed_history(&home, "ecb");
    // 2025-01-04 is a Saturday; the seeded history has 2025-01-03 as the
    // prior business day (USD 1.0317, CNY 7.5371 -> 100 USD = 730.55 CNY).
    let out = run(&home, &["--date", "2025-01-04", "100", "USD", "CNY"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("$100.00 USD =")
            && stdout.contains("730.55 CNY")
            && stdout.contains("\u{1F1E8}\u{1F1F3}"),
        "{stdout}"
    );
    assert!(stdout.contains("rates date 2025-01-03"), "{stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("using 2025-01-03"), "{stderr}");
}

#[test]
fn chart_csv_matches_cross_rates_offline() {
    let home = temp_home("csv");
    seed_history(&home, "ecb");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--format",
            "csv",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout,
        "date,rate\n2025-01-02,7.298779306335982\n2025-01-03,7.305515169138315\n2025-01-06,7.310397700047915\n"
    );
}

#[test]
fn chart_json_is_machine_readable() {
    let home = temp_home("json");
    seed_history(&home, "ecb");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-03",
            "--format",
            "json",
        ],
    );
    assert!(out.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("json output should parse");
    assert_eq!(json["source"], "USD");
    assert_eq!(json["target"], "CNY");
    assert_eq!(json["points"].as_array().unwrap().len(), 2);
    assert_eq!(json["points"][0]["date"], "2025-01-02");
}

#[test]
fn chart_writes_text_file() {
    let home = temp_home("textfile");
    seed_history(&home, "ecb");
    let target = home.join("chart.txt");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--output",
            target.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&target).unwrap();
    assert!(text.starts_with("╭"));
    assert!(text.contains("USD/CNY Trend"));
    assert!(text.contains("Current:"));
    assert!(text.contains("Change:"));
    assert!(text.contains("2025-01-02"));
    assert!(text.contains("2025-01-06"));
    // File output must never carry terminal escape sequences.
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn chart_writes_csv_file() {
    let home = temp_home("csvfile");
    seed_history(&home, "ecb");
    let target = home.join("chart.csv");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--format",
            "csv",
            "--output",
            target.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&target).unwrap();
    assert!(text.starts_with("date,rate\n"));
    assert!(text.contains("2025-01-02,"));
    assert!(text.contains("2025-01-06,"));
    // File output must never carry terminal escape sequences.
    assert!(!text.contains('\u{1b}'));
    // The payload goes to the file instead of stdout.
    assert!(out.stdout.is_empty());
}

#[test]
fn chart_writes_json_file() {
    let home = temp_home("jsonfile");
    seed_history(&home, "ecb");
    let target = home.join("chart.json");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--format",
            "json",
            "--output",
            target.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&target).unwrap())
        .expect("json output should parse");
    assert_eq!(json["source"], "USD");
    assert_eq!(json["target"], "CNY");
    assert_eq!(json["points"].as_array().unwrap().len(), 3);
    assert_eq!(json["points"][0]["date"], "2025-01-02");
    // The payload goes to the file instead of stdout.
    assert!(out.stdout.is_empty());
}

#[test]
fn chart_text_format_renders_braille_chart() {
    let home = temp_home("text");
    seed_history(&home, "ecb");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--format",
            "text",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("╭"));
    assert!(stdout.contains("USD/CNY Trend"));
    assert!(stdout.contains("Volatility:"));
    // Piped stdout is not a terminal: colors must stay off.
    assert!(!stdout.contains('\u{1b}'));
    assert!(stdout
        .chars()
        .any(|c| ('\u{2800}'..='\u{28ff}').contains(&c)));
    assert!(stdout.contains("2025-01-02"));
    assert!(stdout.contains("2025-01-06"));
}

#[test]
fn chart_usage_errors_exit_2() {
    let home = temp_home("chartusage");
    seed_history(&home, "ecb");
    assert_eq!(
        run(
            &home,
            &[
                "chart",
                "USD",
                "CNY",
                "--from",
                "2025-03-31",
                "--to",
                "2025-01-01",
                "--format",
                "csv"
            ],
        )
        .status
        .code(),
        Some(2)
    );
    assert_eq!(
        run(&home, &["chart", "USD", "CNY", "--format", "yaml"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        run(&home, &["chart", "USD", "CNY", "--protocol", "blink"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        run(&home, &["chart", "USD", "CNY", "--from", "2025-13-01"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(run(&home, &["chart", "USD"]).status.code(), Some(2));
    assert_eq!(
        run(&home, &["chart", "USD", "CNY", "GBP"]).status.code(),
        Some(2)
    );
    assert_eq!(
        run(&home, &["chart", "-p", "frankfurter", "USD", "CNY"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(run(&home, &["chart", "--help"]).status.code(), Some(0));
    assert_eq!(run(&home, &["chart", "--version"]).status.code(), Some(0));
}

#[test]
fn chart_unknown_currency_exits_1() {
    let home = temp_home("unknown");
    seed_history(&home, "ecb");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "XXX",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no historical rates for XXX"));
}

#[test]
fn chart_empty_range_is_a_runtime_error() {
    let home = temp_home("empty");
    seed_history(&home, "ecb");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-04",
            "--to",
            "2025-01-05",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no historical rates between"));
}

#[test]
fn chart_without_local_data_and_no_network_exits_1() {
    let home = temp_home("nodata");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no historical rates available"));
}

#[test]
fn chart_provider_isolation_prevents_cross_provider_use() {
    let home = temp_home("isolation");
    // History exists, but for frankfurter, not the chart default ecb.
    seed_history(&home, "frankfurter");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no historical rates available"));
}

#[test]
fn chart_covered_range_never_touches_the_network() {
    let home = temp_home("nocache");
    seed_history(&home, "ecb");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-02",
            "--to",
            "2025-01-06",
            "--format",
            "csv",
        ],
    );
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("warning"),
        "unexpected sync attempt: {stderr}"
    );
}

#[test]
fn chart_uncovered_range_tries_sync_then_uses_cache_warning() {
    let home = temp_home("uncovered");
    seed_history(&home, "ecb");
    // Request a range outside the covered window: sync is attempted,
    // fails (network blocked), warning is printed, cached data is used,
    // and the empty result is a runtime error.
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2024-01-01",
            "--to",
            "2024-12-31",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to update historical rates"),
        "{stderr}"
    );
    assert!(stderr.contains("using cached data"), "{stderr}");
}

#[test]
fn chart_default_range_extends_to_live_saturday() {
    let home = temp_home("livetail");
    seed_history_jan10(&home);
    seed_live_rates(&home, "2025-01-11");
    // 2025-01-11 is the Saturday after the seeded history; the default
    // range extends to it using the rates.json cross rate (7.8 / 1.06).
    let out = run(&home, &["chart", "USD", "CNY", "--format", "csv"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.lines().last(),
        Some("2025-01-11,7.3584905660377355"),
        "{stdout}"
    );
    // The ECB day before the live tail keeps its own value (7.68 / 1.05).
    assert!(stdout.contains("2025-01-10,7.314285714285714"), "{stdout}");
    // Default range, covered history: no sync, no warnings.
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("warning"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn chart_live_point_replaces_last_ecb_day() {
    let home = temp_home("livereplace");
    seed_history_jan10(&home);
    // Snapshot dated on the last ECB day itself: that point comes from
    // rates.json so chart and convert agree on today's rate.
    seed_live_rates(&home, "2025-01-10");
    let out = run(&home, &["chart", "USD", "CNY", "--format", "csv"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.lines().last(),
        Some("2025-01-10,7.3584905660377355"),
        "{stdout}"
    );
    assert!(!stdout.contains("7.314285714285714"), "{stdout}");
}

#[test]
fn chart_single_live_day_prints_one_row() {
    let home = temp_home("onelive");
    seed_history_jan10(&home);
    seed_live_rates(&home, "2025-01-11");
    // A weekend-only range has no ECB data; the live point alone renders
    // as the single-day row. The uncovered range still attempts a sync
    // (blocked here) and falls back to cached history with a warning.
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2025-01-11",
            "--to",
            "2025-01-11",
            "--format",
            "csv",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "date,rate\n2025-01-11,7.3584905660377355\n"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("using cached data"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn chart_stale_history_skips_live_tail() {
    let home = temp_home("staletail");
    seed_history(&home, "ecb");
    // Snapshot eleven days after the last ECB day: beyond the weekend
    // plus one holiday gap, so the history is treated as stale and the
    // live point is not spliced in.
    seed_live_rates(&home, "2025-01-17");
    let out = run(&home, &["chart", "USD", "CNY", "--format", "csv"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.lines().last(),
        Some("2025-01-06,7.310397700047915"),
        "{stdout}"
    );
    assert!(!stdout.contains("2025-01-17"), "{stdout}");
}

#[test]
fn chart_clamps_out_of_range_single_bounds_with_notes() {
    let home = temp_home("clamp");
    seed_history(&home, "ecb");
    // --from before the coverage starts: clamped up to the first day.
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2024-06-01",
            "--format",
            "csv",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no ECB rates before 2025-01-02"),
        "{stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("date,rate\n2025-01-02,"), "{stdout}");
    // --to after the coverage ends: clamped down to the last day.
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--to",
            "2025-06-01",
            "--format",
            "csv",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no ECB rates after 2025-01-06"), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout
            .lines()
            .last()
            .unwrap_or("")
            .starts_with("2025-01-06,"),
        "{stdout}"
    );
}

#[test]
fn chart_from_after_coverage_errors_with_coverage_context() {
    let home = temp_home("fromlate");
    seed_history(&home, "ecb");
    // Clamping a --from after the last day would render days the user
    // never asked for (the weekend-only live-tail range relies on this),
    // so the error keeps the requested range but names the coverage.
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--from",
            "2030-01-01",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "no historical rates between 2030-01-01 and 2025-01-06 \
             (history covers 2025-01-02 to 2025-01-06)"
        ),
        "{stderr}"
    );
}

#[test]
fn chart_to_before_coverage_errors() {
    let home = temp_home("toearly");
    seed_history(&home, "ecb");
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--to",
            "2024-01-01",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no historical rates on or before 2024-01-01 (history starts 2025-01-02)"),
        "{stderr}"
    );
}

#[test]
fn chart_to_clamped_beyond_coverage_still_reaches_live_tail() {
    let home = temp_home("toclamp");
    seed_history_jan10(&home);
    seed_live_rates(&home, "2025-01-11");
    // An explicit --to beyond the coverage is clamped with a note, but the
    // live point still extends the chart the way the default range does.
    let out = run(
        &home,
        &[
            "chart",
            "USD",
            "CNY",
            "--to",
            "2025-06-01",
            "--format",
            "csv",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no ECB rates after 2025-01-10"), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.lines().last(),
        Some("2025-01-11,7.3584905660377355"),
        "{stdout}"
    );
}

#[test]
fn chart_missing_live_currency_warns_and_ends_at_ecb() {
    let home = temp_home("missinglive");
    seed_history_jan10(&home);
    let rates = serde_json::json!({
        "base": "EUR",
        "date": "2025-01-11",
        "fetched_at": "2025-01-11T12:00:00Z",
        "provider": "frankfurter",
        "rates": {"USD": 1.06, "EUR": 1.0}
    });
    std::fs::write(home.join("fxrate").join("rates.json"), rates.to_string()).unwrap();
    let out = run(&home, &["chart", "USD", "CNY", "--format", "csv"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.lines().last(),
        Some("2025-01-10,7.314285714285714"),
        "{stdout}"
    );
    assert!(!stdout.contains("2025-01-11"), "{stdout}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("no rate for currency CNY (rates date 2025-01-11)"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn chart_default_without_rates_cache_ends_at_coverage() {
    let home = temp_home("nolivecache");
    seed_history(&home, "ecb");
    // No rates.json at all: the default range keeps ending at the last
    // ECB day, exactly as before the live-tail feature.
    let out = run(&home, &["chart", "USD", "CNY", "--format", "csv"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.lines().last(),
        Some("2025-01-06,7.310397700047915"),
        "{stdout}"
    );
}
