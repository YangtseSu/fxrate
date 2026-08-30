// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Historical rates: ECB full-history CSV import into SQLite, coverage
// bookkeeping, and series queries. All historical rates are EUR-based
// (units of the quote currency per 1 EUR).

use chrono::{NaiveDate, Utc};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rusqlite::{params, Connection};
use std::error::Error;
use std::fs;
use std::io::Read;
use std::time::Duration;

use crate::current::{get_bytes, MAX_HISTORY_RESPONSE_SIZE};
use crate::provider::Provider;
use crate::storage::history_db_path;

pub const ECB_HIST_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip";
const CSV_ENTRY: &str = "eurofxref-hist.csv";

/// A (date, EUR-based rate) pair, dates ascending.
pub type Point = (chrono::NaiveDate, f64);

/// A parsed ECB CSV row: (date, quote currency, EUR-based rate).
type EcbRow = (chrono::NaiveDate, String, f64);

#[derive(Debug, Clone, Copy)]
pub struct Coverage {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

pub fn open_history_db() -> Result<Connection, Box<dyn Error>> {
    let path = history_db_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(&path)?;
    create_schema(&mut conn)?;
    Ok(conn)
}

fn create_schema(conn: &mut Connection) -> Result<(), Box<dyn Error>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS historical_rates (
            provider   TEXT NOT NULL,
            date       TEXT NOT NULL,
            quote      TEXT NOT NULL,
            rate       REAL NOT NULL,
            fetched_at TEXT NOT NULL,
            PRIMARY KEY (provider, date, quote)
        );
        CREATE INDEX IF NOT EXISTS historical_rates_lookup
            ON historical_rates(provider, quote, date);
        CREATE TABLE IF NOT EXISTS history_coverage (
            provider   TEXT PRIMARY KEY,
            start_date TEXT NOT NULL,
            end_date   TEXT NOT NULL,
            fetched_at TEXT NOT NULL
        );",
    )?;
    migrate_history_coverage(conn)
}

/// True when `history_coverage` still has the legacy multi-row schema (a
/// primary key spanning provider, start, and end) instead of one row per
/// provider.
fn legacy_coverage_schema(conn: &Connection) -> Result<bool, Box<dyn Error>> {
    let mut stmt = conn.prepare("PRAGMA table_info(history_coverage)")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        let pk: i64 = row.get(5)?;
        if (name == "start_date" || name == "end_date") && pk != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Fold a legacy multi-row `history_coverage` table into the single row per
/// provider. Full-history syncs keep at most one range per provider, so real
/// databases fold to the row they already have; the min/max envelope only
/// matters for hand-built databases with disjoint rows.
fn migrate_history_coverage(conn: &mut Connection) -> Result<(), Box<dyn Error>> {
    if !legacy_coverage_schema(conn)? {
        return Ok(());
    }
    let folded: Vec<(String, String, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT provider, MIN(start_date), MAX(end_date), MAX(fetched_at)
             FROM history_coverage GROUP BY provider",
        )?;
        let mut rows = stmt.query([])?;
        let mut folded = Vec::new();
        while let Some(row) = rows.next()? {
            folded.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
        }
        folded
    };
    let tx = conn.transaction()?;
    tx.execute_batch(
        "DROP TABLE history_coverage;
         CREATE TABLE history_coverage (
             provider   TEXT PRIMARY KEY,
             start_date TEXT NOT NULL,
             end_date   TEXT NOT NULL,
             fetched_at TEXT NOT NULL
         );",
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO history_coverage (provider, start_date, end_date, fetched_at)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (provider, start, end, fetched_at) in &folded {
            stmt.execute(params![provider, start, end, fetched_at])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Extract `eurofxref-hist.csv` from the ECB zip archive.
fn extract_ecb_csv(zip_bytes: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        if file.name() == CSV_ENTRY {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            return Ok(bytes);
        }
    }
    Err(crate::boxed_error(format!(
        "ECB archive contains no {CSV_ENTRY}"
    )))
}

/// Parse the ECB historical CSV into (date, quote, EUR-based rate) rows.
///
/// The header is dynamic: the first column must be `Date`, every other
/// non-empty header cell is a quote currency. `N/A` and empty cells are
/// missing values; a non-empty cell that is neither is a parse error.
/// Old currencies (EEK, LTL, ...) are handled through the dynamic header.
pub fn parse_ecb_csv(bytes: &[u8]) -> Result<Vec<EcbRow>, Box<dyn Error>> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
    let headers = reader.headers()?;
    let first = headers
        .get(0)
        .ok_or_else(|| crate::boxed_error("ECB CSV has no header"))?;
    if !first.eq_ignore_ascii_case("Date") {
        return Err(crate::boxed_error(format!(
            "ECB CSV header starts with {first:?}, expected \"Date\""
        )));
    }
    let quotes: Vec<String> = headers
        .iter()
        .skip(1)
        .filter(|field| !field.is_empty())
        .map(|field| field.to_ascii_uppercase())
        .collect();
    if quotes.is_empty() {
        return Err(crate::boxed_error("ECB CSV header has no currency columns"));
    }

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record?;
        let date_text = record
            .get(0)
            .ok_or_else(|| crate::boxed_error("ECB CSV row missing date"))?;
        let date = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").map_err(|error| {
            crate::boxed_error(format!("invalid ECB CSV date {date_text:?}: {error}"))
        })?;
        for (index, quote) in quotes.iter().enumerate() {
            let Some(field) = record.get(index + 1) else {
                continue;
            };
            let field = field.trim();
            if field.is_empty() || field.eq_ignore_ascii_case("N/A") {
                continue;
            }
            let rate = field.parse::<f64>().map_err(|error| {
                crate::boxed_error(format!(
                    "invalid ECB CSV rate {field:?} for {quote} on {date_text}: {error}"
                ))
            })?;
            if rate.is_finite() {
                rows.push((date, quote.clone(), rate));
            }
        }
    }
    Ok(rows)
}

/// A stderr progress indicator for history syncs. indicatif hides it
/// automatically when stderr is not a user-attended terminal, so pipes,
/// logs, and tests never see escape sequences.
fn sync_progress(message: &str) -> ProgressBar {
    let pb = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
    pb.set_style(
        ProgressStyle::with_template("[{spinner:.green}] {msg}")
            .expect("static template is valid")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
    );
    pb.set_message(message.to_owned());
    if !pb.is_hidden() {
        pb.enable_steady_tick(Duration::from_millis(100));
    }
    pb
}

/// Download the ECB full history and upsert it into the database in a
/// transaction, then record the covered range. Returns the imported
/// date span. On an interactive terminal a spinner on stderr tracks each
/// phase so a slow first sync is visibly progressing; it is cleared before
/// an error propagates, so the caller's warning prints cleanly.
pub fn sync_history(
    conn: &mut Connection,
    provider: Provider,
) -> Result<(NaiveDate, NaiveDate), Box<dyn Error>> {
    let pb = sync_progress("Downloading ECB history");
    let drawn = !pb.is_hidden();
    let result = import_ecb_history(conn, provider, &pb);
    pb.finish_and_clear();
    if let (Ok((start, end)), true) = (&result, drawn) {
        // Normal stderr notice, on its own clean line; a hidden bar stays
        // silent so piped output and logs are unchanged.
        eprintln!("ECB history synced: {start} to {end}");
    }
    result
}

/// The phase-by-phase body of [`sync_history`], reporting into `pb`.
fn import_ecb_history(
    conn: &mut Connection,
    provider: Provider,
    pb: &ProgressBar,
) -> Result<(NaiveDate, NaiveDate), Box<dyn Error>> {
    let bytes = get_bytes(ECB_HIST_URL, MAX_HISTORY_RESPONSE_SIZE)?;
    pb.set_message("Extracting and parsing ECB history CSV");
    let csv_bytes = extract_ecb_csv(&bytes)?;
    let rows = parse_ecb_csv(&csv_bytes)?;
    if rows.is_empty() {
        return Err(crate::boxed_error("ECB history CSV contained no rates"));
    }
    let fetched_at = Utc::now().to_rfc3339();
    pb.set_style(
        ProgressStyle::with_template("[{spinner:.green}] {msg} {bar:24.cyan/blue} {pos}/{len}")
            .expect("static template is valid")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
    );
    // The stderr draw target rate-limits redraws (20 Hz), so ticking every
    // row stays cheap even for a few hundred thousand upserts.
    pb.set_length(rows.len() as u64);
    pb.set_message("Importing rates into history database");
    {
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(provider, date, quote)
                 DO UPDATE SET rate = excluded.rate, fetched_at = excluded.fetched_at",
            )?;
            for (date, quote, rate) in &rows {
                stmt.execute(params![
                    provider.name(),
                    date.to_string(),
                    quote,
                    rate,
                    fetched_at
                ])?;
                pb.inc(1);
            }
        }
        tx.commit()?;
    }
    let start = rows.iter().map(|row| row.0).min().expect("rows non-empty");
    let end = rows.iter().map(|row| row.0).max().expect("rows non-empty");
    upsert_coverage(conn, provider, start, end, &fetched_at)?;
    Ok((start, end))
}

/// Record the synced `[start, end]` range for the provider. Every sync
/// imports the full history, so the new range subsumes the previous one and
/// the table holds exactly one row per provider.
fn upsert_coverage(
    conn: &Connection,
    provider: Provider,
    start: NaiveDate,
    end: NaiveDate,
    fetched_at: &str,
) -> Result<(), Box<dyn Error>> {
    conn.execute(
        "INSERT INTO history_coverage (provider, start_date, end_date, fetched_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(provider) DO UPDATE SET
            start_date = excluded.start_date,
            end_date = excluded.end_date,
            fetched_at = excluded.fetched_at",
        params![
            provider.name(),
            start.to_string(),
            end.to_string(),
            fetched_at
        ],
    )?;
    Ok(())
}

/// True when the recorded coverage range for the provider covers `[from, to]`.
pub fn coverage_covers(
    conn: &Connection,
    provider: Provider,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<bool, Box<dyn Error>> {
    let mut stmt = conn.prepare(
        "SELECT 1 FROM history_coverage
         WHERE provider = ?1 AND start_date <= ?2 AND end_date >= ?3
         LIMIT 1",
    )?;
    Ok(stmt.exists(params![provider.name(), from.to_string(), to.to_string()])?)
}

/// The recorded coverage range for the provider, if any.
pub fn coverage_range(
    conn: &Connection,
    provider: Provider,
) -> Result<Option<Coverage>, Box<dyn Error>> {
    let mut stmt = conn.prepare(
        "SELECT start_date, end_date FROM history_coverage
         WHERE provider = ?1",
    )?;
    let mut rows = stmt.query(params![provider.name()])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let start_text: String = row.get(0)?;
    let end_text: String = row.get(1)?;
    let parse = |text: &str| {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|error| {
            crate::boxed_error(format!("corrupted coverage date {text:?}: {error}"))
        })
    };
    Ok(Some(Coverage {
        start: parse(&start_text)?,
        end: parse(&end_text)?,
    }))
}

/// EUR-based rates for one quote in `[from, to]`, ascending by date.
pub fn rate_series(
    conn: &Connection,
    provider: Provider,
    quote: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Point>, Box<dyn Error>> {
    let mut stmt = conn.prepare(
        "SELECT date, rate FROM historical_rates
         WHERE provider = ?1 AND quote = ?2 AND date BETWEEN ?3 AND ?4
         ORDER BY date ASC",
    )?;
    let mut rows = stmt.query(params![
        provider.name(),
        quote,
        from.to_string(),
        to.to_string()
    ])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let date_text: String = row.get(0)?;
        let rate: f64 = row.get(1)?;
        let date = NaiveDate::parse_from_str(&date_text, "%Y-%m-%d").map_err(|error| {
            crate::boxed_error(format!("corrupted history date {date_text:?}: {error}"))
        })?;
        result.push((date, rate));
    }
    Ok(result)
}
/// EUR-based rate for one quote on a single date, or `None` if the date
/// has no entry (e.g. a weekend/holiday or a currency not yet tracked).
/// The base currency `EUR` is never stored and is treated as `1.0` by the
/// caller, so it is not queried here.
pub fn rate_on_date(
    conn: &Connection,
    provider: Provider,
    quote: &str,
    date: NaiveDate,
) -> Result<Option<f64>, Box<dyn Error>> {
    let mut stmt = conn.prepare(
        "SELECT rate FROM historical_rates
         WHERE provider = ?1 AND quote = ?2 AND date = ?3",
    )?;
    let mut rows = stmt.query(params![provider.name(), quote, date.to_string()])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}
/// The most recent trading date on or before `date` that has any ECB data
/// for the provider, or `None` if `date` precedes all available history.
/// Used to fall back from weekends/holidays to the prior business day.
pub fn prev_trading_day(
    conn: &Connection,
    provider: Provider,
    date: NaiveDate,
) -> Result<Option<NaiveDate>, Box<dyn Error>> {
    let mut stmt = conn.prepare(
        "SELECT MAX(date) FROM historical_rates
         WHERE provider = ?1 AND date <= ?2",
    )?;
    let mut rows = stmt.query(params![provider.name(), date.to_string()])?;
    match rows.next()? {
        Some(row) => {
            let text: Option<String> = row.get(0)?;
            match text {
                Some(text) => {
                    let parsed = NaiveDate::parse_from_str(&text, "%Y-%m-%d").map_err(|error| {
                        crate::boxed_error(format!("corrupted history date {text:?}: {error}"))
                    })?;
                    Ok(Some(parsed))
                }
                None => Ok(None),
            }
        }
        None => Ok(None),
    }
}

/// All dates present in the history table within `[from, to]`, ascending.
/// Used to synthesize the EUR series (rate 1.0 on every trading date).
pub fn date_universe(
    conn: &Connection,
    provider: Provider,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<NaiveDate>, Box<dyn Error>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT date FROM historical_rates
         WHERE provider = ?1 AND date BETWEEN ?2 AND ?3
         ORDER BY date ASC",
    )?;
    let mut rows = stmt.query(params![provider.name(), from.to_string(), to.to_string()])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let date_text: String = row.get(0)?;
        let date = NaiveDate::parse_from_str(&date_text, "%Y-%m-%d").map_err(|error| {
            crate::boxed_error(format!("corrupted history date {date_text:?}: {error}"))
        })?;
        result.push(date);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn in_memory_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        create_schema(&mut conn).unwrap();
        conn
    }

    const SAMPLE_CSV: &str = "Date,USD,JPY,EEK,GBP,ZAR,\n\
        2025-01-02,1.10,180.5,N/A,0.80,19.5,\n\
        2025-01-03,1.11,N/A,,0.81,,\n\
        2025-01-06,1.12,182.0,N/A,0.82,19.6,\n";

    #[test]
    fn ecb_csv_parses_dynamic_header_and_missing_values() {
        let rows = parse_ecb_csv(SAMPLE_CSV.as_bytes()).unwrap();
        let expected = vec![
            (date("2025-01-02"), "USD".to_owned(), 1.10),
            (date("2025-01-02"), "JPY".to_owned(), 180.5),
            (date("2025-01-02"), "GBP".to_owned(), 0.80),
            (date("2025-01-02"), "ZAR".to_owned(), 19.5),
            (date("2025-01-03"), "USD".to_owned(), 1.11),
            (date("2025-01-03"), "GBP".to_owned(), 0.81),
            (date("2025-01-06"), "USD".to_owned(), 1.12),
            (date("2025-01-06"), "JPY".to_owned(), 182.0),
            (date("2025-01-06"), "GBP".to_owned(), 0.82),
            (date("2025-01-06"), "ZAR".to_owned(), 19.6),
        ];
        assert_eq!(rows, expected);
    }

    #[test]
    fn ecb_csv_rejects_malformed_dates_and_rates() {
        let bad_date = "Date,USD,\n2025-13-40,1.1,\n";
        assert!(parse_ecb_csv(bad_date.as_bytes()).is_err());

        let bad_rate = "Date,USD,\n2025-01-02,not-a-rate,\n";
        assert!(parse_ecb_csv(bad_rate.as_bytes()).is_err());

        let bad_header = "USD,JPY,\n1.1,180,\n";
        assert!(parse_ecb_csv(bad_header.as_bytes()).is_err());
    }

    #[test]
    fn ecb_csv_handles_old_currency_columns() {
        let csv = "Date,DEM,EEK,USD,\n2025-01-02,1.9558,N/A,1.10,\n";
        let rows = parse_ecb_csv(csv.as_bytes()).unwrap();
        assert_eq!(
            rows,
            vec![
                (date("2025-01-02"), "DEM".to_owned(), 1.9558),
                (date("2025-01-02"), "USD".to_owned(), 1.10),
            ]
        );
    }

    #[test]
    fn ecb_extract_rejects_non_zip_bytes() {
        assert!(extract_ecb_csv(b"not a zip").is_err());
    }

    #[test]
    fn upsert_does_not_duplicate_rows() {
        let conn = in_memory_db();
        let fetched = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
             VALUES ('ecb', '2025-01-02', 'USD', 1.10, ?1)",
            params![fetched],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
             VALUES ('ecb', '2025-01-02', 'USD', 1.11, ?1)
             ON CONFLICT(provider, date, quote)
             DO UPDATE SET rate = excluded.rate, fetched_at = excluded.fetched_at",
            params![fetched],
        )
        .unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM historical_rates WHERE provider = 'ecb' AND date = '2025-01-02' AND quote = 'USD'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let rate: f64 = conn
            .query_row(
                "SELECT rate FROM historical_rates WHERE provider = 'ecb' AND date = '2025-01-02' AND quote = 'USD'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rate, 1.11);
    }

    #[test]
    fn history_is_isolated_by_provider() {
        let conn = in_memory_db();
        let fetched = Utc::now().to_rfc3339();
        for (provider, rate) in [("ecb", 1.10), ("frankfurter", 1.20)] {
            conn.execute(
                "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
                 VALUES (?1, '2025-01-02', 'USD', ?2, ?3)",
                params![provider, rate, fetched],
            )
            .unwrap();
        }
        let ecb = rate_series(
            &conn,
            Provider::Ecb,
            "USD",
            date("2025-01-01"),
            date("2025-01-31"),
        )
        .unwrap();
        assert_eq!(ecb, vec![(date("2025-01-02"), 1.10)]);
        let frank = rate_series(
            &conn,
            Provider::Frankfurter,
            "USD",
            date("2025-01-01"),
            date("2025-01-31"),
        )
        .unwrap();
        assert_eq!(frank, vec![(date("2025-01-02"), 1.20)]);
    }

    #[test]
    fn series_query_respects_range_and_order() {
        let conn = in_memory_db();
        let fetched = Utc::now().to_rfc3339();
        for day in ["2025-01-01", "2025-01-02", "2025-01-06"] {
            conn.execute(
                "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
                 VALUES ('ecb', ?1, 'USD', 1.1, ?2)",
                params![day, fetched],
            )
            .unwrap();
        }
        let series = rate_series(
            &conn,
            Provider::Ecb,
            "USD",
            date("2025-01-02"),
            date("2025-01-06"),
        )
        .unwrap();
        let dates: Vec<_> = series.iter().map(|point| point.0).collect();
        assert_eq!(dates, vec![date("2025-01-02"), date("2025-01-06")]);
        assert_eq!(
            date_universe(&conn, Provider::Ecb, date("2025-01-01"), date("2025-01-31")).unwrap(),
            vec![date("2025-01-01"), date("2025-01-02"), date("2025-01-06")]
        );
    }

    #[test]
    fn coverage_tracks_synced_ranges() {
        let conn = in_memory_db();
        assert!(coverage_range(&conn, Provider::Ecb).unwrap().is_none());
        assert!(
            !coverage_covers(&conn, Provider::Ecb, date("2025-01-01"), date("2025-03-31")).unwrap()
        );

        upsert_coverage(
            &conn,
            Provider::Ecb,
            date("2025-01-02"),
            date("2025-03-31"),
            "now",
        )
        .unwrap();
        assert!(
            coverage_covers(&conn, Provider::Ecb, date("2025-01-02"), date("2025-03-31")).unwrap()
        );
        assert!(
            coverage_covers(&conn, Provider::Ecb, date("2025-02-01"), date("2025-02-28")).unwrap()
        );
        assert!(
            !coverage_covers(&conn, Provider::Ecb, date("2024-12-01"), date("2025-01-02")).unwrap()
        );
        assert!(
            !coverage_covers(&conn, Provider::Ecb, date("2025-01-02"), date("2025-04-01")).unwrap()
        );

        // A newer, wider sync replaces the row: the table holds exactly one
        // range per provider.
        upsert_coverage(
            &conn,
            Provider::Ecb,
            date("2024-12-02"),
            date("2025-04-30"),
            "now",
        )
        .unwrap();
        let row_count = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM history_coverage WHERE provider = ?1",
                params![Provider::Ecb.name()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
        };
        let coverage = coverage_range(&conn, Provider::Ecb).unwrap().unwrap();
        assert_eq!(coverage.start, date("2024-12-02"));
        assert_eq!(coverage.end, date("2025-04-30"));
        assert_eq!(row_count(&conn), 1);

        // A later, disjoint write replaces the row rather than coexisting:
        // coverage is the latest synced range, never a union of stale ones.
        upsert_coverage(
            &conn,
            Provider::Ecb,
            date("2020-01-02"),
            date("2020-12-31"),
            "now",
        )
        .unwrap();
        let coverage = coverage_range(&conn, Provider::Ecb).unwrap().unwrap();
        assert_eq!(coverage.start, date("2020-01-02"));
        assert_eq!(coverage.end, date("2020-12-31"));
        assert_eq!(row_count(&conn), 1);
    }

    #[test]
    fn legacy_coverage_rows_are_folded_into_one() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE history_coverage (
                provider   TEXT NOT NULL,
                start_date TEXT NOT NULL,
                end_date   TEXT NOT NULL,
                fetched_at TEXT NOT NULL,
                PRIMARY KEY (provider, start_date, end_date)
            );
            INSERT INTO history_coverage VALUES
                ('ecb', '2020-01-02', '2020-12-31', 'old'),
                ('ecb', '2024-12-02', '2025-04-30', 'new');",
        )
        .unwrap();
        create_schema(&mut conn).unwrap();
        let coverage = coverage_range(&conn, Provider::Ecb).unwrap().unwrap();
        assert_eq!(coverage.start, date("2020-01-02"));
        assert_eq!(coverage.end, date("2025-04-30"));
        let row_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM history_coverage", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(row_count, 1);
    }

    #[test]
    fn rate_on_date_returns_stored_value_or_none() {
        let conn = in_memory_db();
        let fetched = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
             VALUES ('ecb', '2025-01-02', 'USD', 1.0322, ?1)",
            params![fetched],
        )
        .unwrap();
        assert_eq!(
            rate_on_date(&conn, Provider::Ecb, "USD", date("2025-01-02")).unwrap(),
            Some(1.0322)
        );
        assert_eq!(
            rate_on_date(&conn, Provider::Ecb, "USD", date("2025-01-03")).unwrap(),
            None
        );
        assert_eq!(
            rate_on_date(&conn, Provider::Frankfurter, "USD", date("2025-01-02")).unwrap(),
            None
        );
    }
    #[test]
    fn prev_trading_day_falls_back_to_last_business_day() {
        let conn = in_memory_db();
        let fetched = Utc::now().to_rfc3339();
        for day in ["2025-01-02", "2025-01-03", "2025-01-06"] {
            conn.execute(
                "INSERT INTO historical_rates (provider, date, quote, rate, fetched_at)
                 VALUES ('ecb', ?1, 'USD', 1.0, ?2)",
                params![day, fetched],
            )
            .unwrap();
        }
        assert_eq!(
            prev_trading_day(&conn, Provider::Ecb, date("2025-01-04")).unwrap(),
            Some(date("2025-01-03"))
        );
        assert_eq!(
            prev_trading_day(&conn, Provider::Ecb, date("2025-01-05")).unwrap(),
            Some(date("2025-01-03"))
        );
        assert_eq!(
            prev_trading_day(&conn, Provider::Ecb, date("2025-01-06")).unwrap(),
            Some(date("2025-01-06"))
        );
        assert_eq!(
            prev_trading_day(&conn, Provider::Ecb, date("2024-01-01")).unwrap(),
            None
        );
    }
}
