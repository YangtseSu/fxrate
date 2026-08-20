// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Command huobi is an offline currency conversion CLI with historical
// exchange-rate charts.

mod cli;
mod current;
mod history;
mod provider;
mod render;
mod series;
mod storage;

use chrono::{NaiveDate, Utc};
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal};
use std::process;

use cli::{ChartArgs, Command, ConvertArgs};
use provider::Provider;
use render::Format;

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

fn fatal(message: &str) -> ! {
    eprintln!("error: {message}");
    process::exit(1);
}

fn main() {
    let command = match cli::parse_args() {
        Ok(command) => command,
        Err(code) => process::exit(code),
    };
    match command {
        Command::Convert(args) => run_convert(args),
        Command::Chart(args) => run_chart(args),
    }
}

fn run_convert(args: ConvertArgs) {
    if let Some(date) = args.date {
        run_convert_historical(args, date);
        return;
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
                    "warning: invalid provider {:?} in config, falling back to frankfurter",
                    config.provider()
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
                    "warning: invalid update_interval={:?} in config, falling back to 24h",
                    config.update_interval()
                );
                storage::DEFAULT_INTERVAL
            }
        }
    };

    let mut snapshot = match storage::load_rates() {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            if error.downcast_ref::<io::Error>().is_none() {
                eprintln!("warning: failed to read local rates: {error}");
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
                let cached_provider = if cached.provider.is_empty() {
                    "unknown"
                } else {
                    cached.provider.as_str()
                };
                eprintln!(
                    "warning: failed to update rates: {error}; using cached rates (date {}, provider {cached_provider})",
                    cached.date
                );
            }
        }
    }
    let snapshot = snapshot.expect("fatal exits when no snapshot is available");
    render_convert(&snapshot, amount, &source, &explicit, &config, updated);
}

/// Render the conversion table and footer for a resolved rate snapshot.
///
/// Shared by the live-rates path and the historical (`--date`) path; the
/// only differences are the snapshot origin and the `updated` flag.
fn render_convert(
    snapshot: &current::RateSnapshot,
    amount: f64,
    source: &str,
    targets: &[String],
    config: &storage::Config,
    updated: bool,
) {
    if let Err(error) = current::currency_rate(snapshot, source) {
        fatal(&error.to_string());
    }
    let convert_list = |list: Vec<String>| {
        list.into_iter()
            .filter_map(
                |code| match current::convert(snapshot, source, &code, amount) {
                    Ok(value) => Some(Row { code, value }),
                    Err(error) => {
                        eprintln!("warning: {error}, skipped");
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

/// Historical conversion: resolve EUR-based rates for `date` from the ECB
/// history database and reuse the live rendering path. The `-p/--provider`
/// selection is ignored here because historical rates are always ECB.
fn run_convert_historical(args: ConvertArgs, date: NaiveDate) {
    let provider = Provider::Ecb;
    let mut conn = match history::open_history_db() {
        Ok(conn) => conn,
        Err(error) => fatal(&format!("failed to open history database: {error}")),
    };
    let covered = match history::coverage_covers(&conn, provider, date, date) {
        Ok(covered) => covered,
        Err(error) => fatal(&format!("failed to check history coverage: {error}")),
    };
    if args.force || !covered {
        match history::sync_history(&mut conn, provider) {
            Ok(_) => {}
            Err(error) if !covered => fatal(&format!(
                "no historical rates available for {date} and update failed: {error}"
            )),
            Err(error) => {
                eprintln!("warning: failed to update historical rates: {error}; using cached data");
            }
        }
    }
    let effective_date = match history::prev_trading_day(&conn, provider, date) {
        Ok(Some(effective)) => effective,
        Ok(None) => fatal(&format!(
            "no historical rates available on or before {date}"
        )),
        Err(error) => fatal(&format!("failed to read history coverage: {error}")),
    };
    if effective_date != date {
        eprintln!("warning: no ECB rate for {date}; using {effective_date}");
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
            Err(error) => fatal(&format!("failed to read historical rates: {error}")),
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
    );
}

fn run_chart(args: ChartArgs) {
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

    let mut conn = match history::open_history_db() {
        Ok(conn) => conn,
        Err(error) => fatal(&format!("failed to open history database: {error}")),
    };
    let mut coverage = match history::coverage_range(&conn, provider) {
        Ok(coverage) => coverage,
        Err(error) => fatal(&format!("failed to read history coverage: {error}")),
    };
    let mut need_sync = force || coverage.is_none();
    if !need_sync {
        if let (Some(from), Some(to)) = (from, to) {
            need_sync = match history::coverage_covers(&conn, provider, from, to) {
                Ok(covered) => !covered,
                Err(error) => fatal(&format!("failed to check history coverage: {error}")),
            };
        }
    }
    if need_sync {
        match history::sync_history(&mut conn, provider) {
            Ok((start, end)) => {
                coverage = Some(history::Coverage { start, end });
            }
            Err(error) if coverage.is_none() => fatal(&format!(
                "no historical rates available and update failed: {error}"
            )),
            Err(error) => {
                eprintln!("warning: failed to update historical rates: {error}; using cached data");
            }
        }
    }
    let Some(coverage) = coverage else {
        fatal("no historical rates available");
    };
    let from = from.unwrap_or(coverage.start);
    let to = to.unwrap_or(coverage.end);

    let universe = match history::date_universe(&conn, provider, from, to) {
        Ok(universe) => universe,
        Err(error) => fatal(&format!("failed to read historical rates: {error}")),
    };
    if universe.is_empty() {
        fatal(&format!("no historical rates between {from} and {to}"));
    }
    let load_series = |quote: &str| -> Vec<series::Point> {
        if quote == "EUR" {
            universe.iter().map(|date| (*date, 1.0)).collect()
        } else {
            match history::rate_series(&conn, provider, quote, from, to) {
                Ok(series) => series,
                Err(error) => fatal(&format!("failed to read historical rates: {error}")),
            }
        }
    };
    let source_series = load_series(&source);
    if source_series.is_empty() {
        fatal(&format!(
            "no historical rates for {source} between {from} and {to}"
        ));
    }
    let target_series = load_series(&target);
    if target_series.is_empty() {
        fatal(&format!(
            "no historical rates for {target} between {from} and {to}"
        ));
    }
    let points = series::cross_series(&source_series, &target_series);
    if points.is_empty() {
        fatal(&format!(
            "no overlapping data for {source} and {target} between {from} and {to}"
        ));
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
                let (cols, _) = render::terminal_size();
                render::render_text(&points, &source, &target, cols as usize)
            };
            match output {
                Some(path) => {
                    if let Err(error) = fs::write(&path, text) {
                        fatal(&format!("failed to write {}: {error}", path.display()));
                    }
                }
                None => print!("{text}"),
            }
        }
        Format::Auto => unreachable!("auto resolved above"),
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
}
