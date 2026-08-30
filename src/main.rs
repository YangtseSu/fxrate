// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Command fxrate is an offline currency conversion CLI with historical
// exchange-rate charts.

mod cli;
mod currency;
mod current;
mod history;
mod provider;
mod render;
mod series;
mod storage;
mod style;

use chrono::{NaiveDate, Utc};
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal};
use std::process;

use owo_colors::OwoColorize;

use cli::{ChartArgs, Command, ConvertArgs};
use provider::Provider;
use render::Format;

#[derive(Debug)]
struct FxrateError(String);

impl fmt::Display for FxrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for FxrateError {}

fn boxed_error(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(FxrateError(message.into()))
}

/// Largest allowed gap in days between the last ECB day and the live
/// snapshot date for the chart's live tail: a weekend plus one holiday.
const LIVE_TAIL_MAX_GAP_DAYS: i64 = 4;

fn main() {
    let command = match cli::parse_args() {
        Ok(command) => command,
        Err(code) => process::exit(code),
    };
    let result = match command {
        Command::Convert(args) => run_convert(args),
        Command::Chart(args) => run_chart(args),
    };
    if let Err(error) = result {
        eprintln!("{}", style::error(error.to_string()));
        process::exit(1);
    }
}

fn run_convert(args: ConvertArgs) -> Result<(), Box<dyn Error>> {
    if let Some(date) = args.date {
        return run_convert_historical(args, date);
    }
    let ConvertArgs {
        force,
        provider: cli_provider,
        amount,
        source,
        targets: explicit,
        ..
    } = args;
    let config = storage::load_config();
    let provider = match cli_provider {
        Some(name) => Provider::from_name(&name).expect("provider validated by parse_args"),
        None => match Provider::from_name(config.provider()) {
            Some(provider) => provider,
            None => {
                eprintln!(
                    "{}",
                    style::warning(format!(
                        "invalid provider {:?} in config, falling back to frankfurter (valid: {})",
                        config.provider(),
                        Provider::names(&Provider::CONVERT_PROVIDERS)
                    ))
                );
                Provider::Frankfurter
            }
        },
    };
    let interval = if config.update_interval().is_empty() {
        storage::DEFAULT_INTERVAL
    } else {
        match storage::parse_duration(config.update_interval()) {
            Some(duration) => duration,
            None => {
                eprintln!(
                    "{}",
                    style::warning(format!(
                        "invalid update_interval={:?} in config, falling back to 24h",
                        config.update_interval()
                    ))
                );
                storage::DEFAULT_INTERVAL
            }
        }
    };

    let mut snapshot = match storage::load_rates() {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            if error.downcast_ref::<io::Error>().is_none() {
                eprintln!(
                    "{}",
                    style::warning(format!("failed to read local rates: {error}"))
                );
            }
            None
        }
    };
    let stale = current::cache_needs_refresh(snapshot.as_ref(), provider, interval);
    let mut updated = false;
    if force || stale {
        match current::fetch_rates(provider) {
            Ok(fresh) => {
                if let Err(error) = storage::save_rates(&fresh) {
                    eprintln!(
                        "{}",
                        style::warning(format!("failed to save rates cache: {error}"))
                    );
                }
                snapshot = Some(fresh);
                updated = true;
            }
            Err(error) => {
                if force {
                    return Err(boxed_error(format!("failed to update rates: {error}")));
                }
                let Some(cached) = snapshot.as_ref() else {
                    return Err(boxed_error(format!(
                        "no local rate cache and update failed: {error}"
                    )));
                };
                let cached_provider = if cached.provider.is_empty() {
                    "unknown"
                } else {
                    cached.provider.as_str()
                };
                eprintln!(
                    "{}",
                    style::warning(format!(
                        "failed to update rates: {error}; using cached rates (date {}, provider {cached_provider})",
                        cached.date
                    ))
                );
            }
        }
    }
    let Some(snapshot) = snapshot else {
        return Err(boxed_error("no local rate cache available"));
    };
    render_convert(
        &snapshot,
        amount,
        &source,
        &explicit,
        &config,
        updated,
        style::stdout_color(),
    )
}

/// Render the conversion table and footer for a resolved rate snapshot.
///
/// Shared by the live-rates path and the historical (`--date`) path; the
/// only differences are the snapshot origin and the `updated` flag. Each row
/// shows the currency symbol before the value, then the code, a flag emoji,
/// and the English name.
fn render_convert(
    snapshot: &current::RateSnapshot,
    amount: f64,
    source: &str,
    targets: &[String],
    config: &storage::Config,
    updated: bool,
    color: bool,
) -> Result<(), Box<dyn Error>> {
    if let Err(error) = current::currency_rate(snapshot, source) {
        return Err(boxed_error(error.to_string()));
    }
    for line in convert_lines(snapshot, amount, source, targets, config, updated, color) {
        println!("{line}");
    }
    Ok(())
}

/// Build the conversion table lines with the footer last: converted amounts
/// bold and the footer bright black when `color`, plain otherwise. Warnings
/// for skipped currencies go to stderr inline; otherwise pure, so the
/// styling is unit-testable.
fn convert_lines(
    snapshot: &current::RateSnapshot,
    amount: f64,
    source: &str,
    targets: &[String],
    config: &storage::Config,
    updated: bool,
    color: bool,
) -> Vec<String> {
    let convert_list = |list: Vec<String>| {
        list.into_iter()
            .filter_map(
                |code| match current::convert(snapshot, source, &code, amount) {
                    Ok(value) => Some(Row { code, value }),
                    Err(error) => {
                        eprintln!("{}", style::warning(format!("{error}, skipped")));
                        None
                    }
                },
            )
            .collect::<Vec<_>>()
    };
    let explicit_rows = convert_list(dedupe_targets(targets, source, &[]));
    let multi_rows = if explicit_rows.is_empty() || config.multi_view() {
        let excluded = explicit_rows
            .iter()
            .map(|row| row.code.clone())
            .collect::<Vec<_>>();
        convert_list(dedupe_targets(config.currencies(), source, &excluded))
    } else {
        Vec::new()
    };

    // Width of the right-aligned value column (symbol + number) so decimal
    // points line up across rows that may carry different-length symbols.
    // Computed on raw strings: escape sequences must not shift the column.
    let value_width = explicit_rows
        .iter()
        .chain(multi_rows.iter())
        .map(|row| {
            let m = currency::meta(&row.code);
            render::display_width(&format!("{}{:.2}", m.symbol, row.value))
        })
        .max()
        .unwrap_or(0);

    let source_meta = currency::meta(source);
    let amount_string = format!("{}{:.2}", source_meta.symbol, amount);
    // Display columns of the first row's prefix `{amount} {source} = `:
    // the +4 is the one space after the amount plus the three of " = ".
    // Display width, not bytes: multi-byte symbols (€, ¥) are one column.
    let padding = render::display_width(&amount_string) + render::display_width(source) + 4;
    let indent = " ".repeat(padding);

    let render_row = |first: bool, row: &Row, color: bool| -> String {
        let m = currency::meta(&row.code);
        let value = format!("{}{:.2}", m.symbol, row.value);
        let value = format!("{value:>width$}", width = value_width);
        // Paint after padding so escape bytes stay out of the width math.
        let value = if color {
            value.bold().to_string()
        } else {
            value
        };
        let amount_cell = if color {
            amount_string.bold().to_string()
        } else {
            amount_string.clone()
        };
        let mut line = if first {
            format!("{amount_cell} {source} = {value} {}", row.code)
        } else {
            format!("{indent}{value} {}", row.code)
        };
        if !m.flag.is_empty() {
            line.push(' ');
            line.push_str(&m.flag);
        }
        if !m.name.is_empty() {
            line.push(' ');
            line.push_str(&m.name);
        }
        line
    };

    let mut lines: Vec<String> = explicit_rows
        .iter()
        .enumerate()
        .map(|(i, row)| render_row(i == 0, row, color))
        .collect();
    if !multi_rows.is_empty() {
        if !explicit_rows.is_empty() {
            // Width from the raw first row: painted rows carry escape bytes
            // that would inflate the rule length.
            let sep_len = explicit_rows
                .first()
                .map(|row| render::display_width(&render_row(true, row, false)))
                .unwrap_or(padding);
            lines.push("-".repeat(sep_len));
        }
        lines.extend(
            multi_rows
                .iter()
                .enumerate()
                .map(|(i, row)| render_row(i == 0 && explicit_rows.is_empty(), row, color)),
        );
    }
    let footer = if updated {
        format!("rates updated: {}", snapshot.date)
    } else {
        format!("rates date {}", snapshot.date)
    };
    lines.push(if color {
        footer.bright_black().to_string()
    } else {
        footer
    });
    lines
}

/// Historical conversion: resolve EUR-based rates for `date` from the ECB
/// history database and reuse the live rendering path. The `-p/--provider`
/// selection is ignored here because historical rates are always ECB.
fn run_convert_historical(args: ConvertArgs, date: NaiveDate) -> Result<(), Box<dyn Error>> {
    let provider = Provider::Ecb;
    let mut conn = history::open_history_db()
        .map_err(|error| boxed_error(format!("failed to open history database: {error}")))?;
    let covered = history::coverage_covers(&conn, provider, date, date)
        .map_err(|error| boxed_error(format!("failed to check history coverage: {error}")))?;
    if args.force || !covered {
        match history::sync_history(&mut conn, provider) {
            Ok(_) => {}
            Err(error) if !covered => {
                return Err(boxed_error(format!(
                    "no historical rates available for {date} and update failed: {error}"
                )));
            }
            Err(error) => {
                eprintln!(
                    "{}",
                    style::warning(format!(
                        "failed to update historical rates: {error}; using cached data"
                    ))
                );
            }
        }
    }
    let effective_date = match history::prev_trading_day(&conn, provider, date) {
        Ok(Some(effective)) => effective,
        Ok(None) => {
            return Err(boxed_error(format!(
                "no historical rates available on or before {date}"
            )));
        }
        Err(error) => {
            return Err(boxed_error(format!(
                "failed to read history coverage: {error}"
            )));
        }
    };
    if effective_date != date {
        eprintln!(
            "{}",
            style::warning(format!("no ECB rate for {date}; using {effective_date}"))
        );
    }
    let config = storage::load_config();
    let explicit = dedupe_targets(&args.targets, &args.source, &[]);
    let use_multi = explicit.is_empty() || config.multi_view();
    let mut codes: Vec<String> = explicit.clone();
    if use_multi {
        let excluded = codes.clone();
        codes.extend(dedupe_targets(config.currencies(), &args.source, &excluded));
    }
    let mut rates: HashMap<String, f64> = HashMap::new();
    rates.insert("EUR".to_owned(), 1.0);
    for code in codes.iter().chain(std::iter::once(&args.source)) {
        if code == "EUR" {
            continue;
        }
        match history::rate_on_date(&conn, provider, code, effective_date) {
            Ok(Some(rate)) => {
                rates.insert(code.clone(), rate);
            }
            Ok(None) => {}
            Err(error) => {
                return Err(boxed_error(format!(
                    "failed to read historical rates: {error}"
                )));
            }
        }
    }
    let snapshot = current::RateSnapshot {
        base: "EUR".to_owned(),
        date: effective_date.to_string(),
        fetched_at: Utc::now(),
        provider: "ecb".to_owned(),
        rates,
    };
    render_convert(
        &snapshot,
        args.amount,
        &args.source,
        &args.targets,
        &config,
        false,
        style::stdout_color(),
    )
}

fn run_chart(args: ChartArgs) -> Result<(), Box<dyn Error>> {
    let ChartArgs {
        force,
        provider: cli_provider,
        from,
        to,
        format,
        output,
        source,
        target,
    } = args;
    let provider =
        Provider::from_name(cli_provider.as_deref().unwrap_or("ecb")).expect("validated by cli");

    // The chart's right edge may extend to the live snapshot's date; that
    // point is read from the rates cache so chart and convert agree on
    // today's rate. The cache is used as-is (no fetch here): a missing or
    // unreadable cache simply disables the live tail.
    let snapshot = match storage::load_rates() {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            if error.downcast_ref::<io::Error>().is_none() {
                eprintln!(
                    "{}",
                    style::warning(format!("failed to read local rates: {error}"))
                );
            }
            None
        }
    };

    let mut conn = history::open_history_db()
        .map_err(|error| boxed_error(format!("failed to open history database: {error}")))?;
    let mut coverage = history::coverage_range(&conn, provider)
        .map_err(|error| boxed_error(format!("failed to read history coverage: {error}")))?;
    let mut need_sync = force || coverage.is_none();
    if !need_sync {
        if let (Some(from), Some(to)) = (from, to) {
            need_sync = !history::coverage_covers(&conn, provider, from, to).map_err(|error| {
                boxed_error(format!("failed to check history coverage: {error}"))
            })?;
        }
    }
    if need_sync {
        match history::sync_history(&mut conn, provider) {
            Ok((start, end)) => {
                coverage = Some(history::Coverage { start, end });
            }
            Err(error) if coverage.is_none() => {
                return Err(boxed_error(format!(
                    "no historical rates available and update failed: {error}"
                )));
            }
            Err(error) => {
                eprintln!(
                    "{}",
                    style::warning(format!(
                        "failed to update historical rates: {error}; using cached data"
                    ))
                );
            }
        }
    }
    let Some(coverage) = coverage else {
        return Err(boxed_error("no historical rates available"));
    };
    let from = from.unwrap_or(coverage.start);
    let to_explicit = to.is_some();
    let mut to = to.unwrap_or(coverage.end);

    // Live tail: when the cached snapshot's rate date reaches the last ECB
    // day (replacing that point) or extends past it (appending a new
    // point), the point comes from rates.json — the same EUR-based cross
    // math the convert command uses — so the chart's right edge matches
    // convert. The snapshot date must be within LIVE_TAIL_MAX_GAP_DAYS of
    // the last ECB day: a weekend plus one holiday is the largest plausible
    // gap, and anything larger means the history is stale, so no splice.
    let live_date = snapshot
        .as_ref()
        .and_then(|snapshot| NaiveDate::parse_from_str(&snapshot.date, "%Y-%m-%d").ok());
    let live_splice: Option<series::Point> = match (live_date, snapshot.as_ref()) {
        (Some(live_date), Some(snapshot))
            if live_date >= from
                && live_date >= coverage.end
                && live_date - coverage.end <= chrono::Duration::days(LIVE_TAIL_MAX_GAP_DAYS) =>
        {
            // The default range extends to the live date; an explicit
            // `--to` caps the range, so a live date beyond it is skipped.
            if !to_explicit && live_date > to {
                to = live_date;
            }
            if live_date > to {
                None
            } else {
                match (
                    current::currency_rate(snapshot, &source),
                    current::currency_rate(snapshot, &target),
                ) {
                    (Ok(source_rate), Ok(target_rate)) => {
                        Some((live_date, target_rate / source_rate))
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        eprintln!(
                            "{}",
                            style::warning(format!("{error}; chart ends at {}", coverage.end))
                        );
                        None
                    }
                }
            }
        }
        _ => None,
    };

    let universe = history::date_universe(&conn, provider, from, to)
        .map_err(|error| boxed_error(format!("failed to read historical rates: {error}")))?;
    if universe.is_empty() && live_splice.is_none() {
        return Err(boxed_error(format!(
            "no historical rates between {from} and {to}"
        )));
    }
    let load_series = |quote: &str| -> Result<Vec<series::Point>, Box<dyn Error>> {
        if quote == "EUR" {
            Ok(universe.iter().map(|date| (*date, 1.0)).collect())
        } else {
            history::rate_series(&conn, provider, quote, from, to)
                .map_err(|error| boxed_error(format!("failed to read historical rates: {error}")))
        }
    };
    let source_series = load_series(&source)?;
    if source_series.is_empty() && live_splice.is_none() {
        return Err(boxed_error(format!(
            "no historical rates for {source} between {from} and {to}"
        )));
    }
    let target_series = load_series(&target)?;
    if target_series.is_empty() && live_splice.is_none() {
        return Err(boxed_error(format!(
            "no historical rates for {target} between {from} and {to}"
        )));
    }
    let mut points = series::cross_series(&source_series, &target_series);
    if let Some((live_date, live_rate)) = live_splice {
        if let Some(point) = points.iter_mut().find(|(date, _)| *date == live_date) {
            point.1 = live_rate;
        } else {
            points.push((live_date, live_rate));
        }
    }
    if points.is_empty() {
        return Err(boxed_error(format!(
            "no overlapping data for {source} and {target} between {from} and {to}"
        )));
    }

    let tty = io::stdout().is_terminal();
    let resolved = match format {
        Format::Auto if output.is_some() => Format::Text,
        Format::Auto if tty => Format::Text,
        Format::Auto => Format::Csv,
        other => other,
    };

    match resolved {
        Format::Csv => print!("{}", render::render_csv(&points)),
        Format::Json => println!("{}", render::render_json(&points, &source, &target)),
        Format::Text => {
            let text = if points.len() == 1 {
                let (date, rate) = points[0];
                format!(
                    "1 {source} = {} {target} ({date})\n",
                    render::fmt_value(rate)
                )
            } else {
                let (cols, rows) = render::terminal_size();
                // Colors only on an interactive terminal; files and pipes
                // stay plain, and NO_COLOR turns them off.
                let color = output.is_none() && style::stdout_color();
                render::render_text(&points, &source, &target, cols as usize, rows as u32, color)
            };
            match output {
                Some(path) => {
                    fs::write(&path, text).map_err(|error| {
                        boxed_error(format!("failed to write {}: {error}", path.display()))
                    })?;
                }
                None => print!("{text}"),
            }
        }
        Format::Auto => unreachable!("auto resolved above"),
    }
    Ok(())
}

fn dedupe_targets(currencies: &[String], source: &str, exclude: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    seen.insert(source.to_uppercase());
    for currency in exclude {
        seen.insert(currency.to_uppercase());
    }
    currencies
        .iter()
        .map(|currency| currency.to_uppercase())
        .filter(|currency| seen.insert(currency.clone()))
        .collect()
}

struct Row {
    code: String,
    value: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn lowercase_source_is_still_filtered() {
        // The parser uppercases the source, but this function must not
        // depend on that upstream normalization.
        assert_eq!(
            dedupe_targets(&["usd".to_owned()], "usd", &[]),
            Vec::<String>::new()
        );
        assert_eq!(
            dedupe_targets(&["USD".to_owned()], "usd", &[]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn convert_rows_align_across_multibyte_symbols() {
        let config = storage::Config::default();
        let lines = convert_lines(
            &snapshot(),
            100.0,
            "CNY",
            &["USD".to_owned()],
            &config,
            false,
            false,
        );
        // lines: USD row, rule, CNY multi-view row, footer. "¥100.00 CNY = "
        // is 14 display columns; the two-byte yen sign must not push the
        // continuation rows' value column to the right.
        assert_eq!(lines[2].len() - lines[2].trim_start().len(), 14);
        // The rule line matches the first row's display width (the same
        // display_width convention the chart panel uses).
        assert!(lines[1].chars().all(|c| c == '-'), "lines: {lines:?}");
        assert_eq!(
            lines[1].len(),
            render::display_width(&lines[0]),
            "lines: {lines:?}"
        );
    }

    fn snapshot() -> current::RateSnapshot {
        current::RateSnapshot {
            base: "EUR".to_owned(),
            date: "2026-08-28".to_owned(),
            fetched_at: Utc::now(),
            provider: "frankfurter".to_owned(),
            rates: HashMap::from([
                ("EUR".to_owned(), 1.0),
                ("USD".to_owned(), 1.1),
                ("CNY".to_owned(), 7.8),
            ]),
        }
    }

    #[test]
    fn convert_lines_are_plain_without_color() {
        let config = storage::Config::default();
        let lines = convert_lines(
            &snapshot(),
            100.0,
            "USD",
            &["CNY".to_owned()],
            &config,
            false,
            false,
        );
        assert!(!lines.iter().any(|line| line.contains('\u{1b}')));
        assert_eq!(lines.last().unwrap(), "rates date 2026-08-28");
        // 100 USD = 100 * rate[CNY] / rate[USD] = 100 * 7.8 / 1.1
        assert!(lines[0].contains("709.09"));
    }

    #[test]
    fn convert_lines_bold_amounts_and_dim_footer_with_color() {
        let config = storage::Config::default();
        let lines = convert_lines(
            &snapshot(),
            100.0,
            "USD",
            &["CNY".to_owned()],
            &config,
            true,
            true,
        );
        assert!(lines[0].contains("\u{1b}[1m")); // bold amounts
        assert!(lines[0].contains("709.09"));
        assert!(lines
            .last()
            .unwrap()
            .contains("\u{1b}[90mrates updated: 2026-08-28"));
    }
}
