// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Command-line parsing for the convert and chart commands.

use chrono::NaiveDate;
use std::path::PathBuf;

use crate::provider::Provider;
use crate::render::Format;

pub enum Command {
    Convert(ConvertArgs),
    Chart(ChartArgs),
}

pub struct ConvertArgs {
    pub force: bool,
    pub provider: Option<String>,
    pub amount: f64,
    pub source: String,
    pub targets: Vec<String>,
    pub date: Option<NaiveDate>,
}

pub struct ChartArgs {
    pub force: bool,
    pub provider: Option<String>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub format: Format,
    pub output: Option<PathBuf>,
    pub source: String,
    pub target: String,
}

pub fn parse_args() -> Result<Command, i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first_positional = args.iter().position(|arg| !arg.starts_with('-'));
    match first_positional {
        Some(index) if args[index] == "chart" => parse_chart_args(&args[index + 1..]),
        _ => parse_convert_args(&args),
    }
}

fn parse_convert_args(args: &[String]) -> Result<Command, i32> {
    let mut force = false;
    let mut provider = None;
    let mut date = None;
    let mut positional = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-u" | "--update" => force = true,
            "-p" | "--provider" => {
                let Some(name) = iter.next() else {
                    eprintln!("error: option {arg} requires a provider name");
                    usage();
                    return Err(2);
                };
                provider = Some(name.clone());
            }
            "-d" | "--date" => {
                let Some(value) = iter.next() else {
                    eprintln!("error: option {arg} requires a date");
                    usage();
                    return Err(2);
                };
                date = Some(NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
                    eprintln!("error: invalid date {value:?} (expected YYYY-MM-DD)");
                    usage();
                    2
                })?);
            }
            "-h" | "--help" => {
                usage();
                return Err(0);
            }
            _ if arg.starts_with('-') => {
                eprintln!("error: unknown option {arg}");
                usage();
                return Err(2);
            }
            _ => positional.push(arg.clone()),
        }
    }
    if let Some(name) = &provider {
        if !Provider::CONVERT_PROVIDERS
            .iter()
            .any(|provider| provider.name() == name.to_ascii_lowercase())
        {
            let valid = Provider::CONVERT_PROVIDERS
                .iter()
                .map(|provider| provider.name())
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("error: unknown provider {name:?} (valid: {valid})");
            usage();
            return Err(2);
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
    if !amount.is_finite() {
        eprintln!("error: amount must be finite");
        usage();
        return Err(2);
    }
    let source = positional[1].to_uppercase();
    let targets = positional[2..]
        .iter()
        .map(|target| target.to_uppercase())
        .collect();
    Ok(Command::Convert(ConvertArgs {
        force,
        provider,
        amount,
        source,
        targets,
        date,
    }))
}

fn parse_date(value: &str) -> Result<NaiveDate, i32> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        eprintln!("error: invalid date {value:?} (expected YYYY-MM-DD)");
        chart_usage();
        2
    })
}

fn parse_chart_args(args: &[String]) -> Result<Command, i32> {
    let mut force = false;
    let mut provider: Option<String> = None;
    let mut from = None;
    let mut to = None;
    let mut format = Format::Auto;
    let mut output = None;
    let mut positional = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-u" | "--update" => force = true,
            "-p" | "--provider" => {
                let Some(name) = iter.next() else {
                    eprintln!("error: option {arg} requires a provider name");
                    chart_usage();
                    return Err(2);
                };
                provider = Some(name.clone());
            }
            "--from" => {
                let Some(value) = iter.next() else {
                    eprintln!("error: option {arg} requires a date");
                    chart_usage();
                    return Err(2);
                };
                from = Some(parse_date(value)?);
            }
            "--to" => {
                let Some(value) = iter.next() else {
                    eprintln!("error: option {arg} requires a date");
                    chart_usage();
                    return Err(2);
                };
                to = Some(parse_date(value)?);
            }
            "--format" => {
                let Some(value) = iter.next() else {
                    eprintln!("error: option {arg} requires a format");
                    chart_usage();
                    return Err(2);
                };
                match Format::from_name(value) {
                    Some(parsed) => format = parsed,
                    None => {
                        eprintln!("error: unknown format {value:?} (valid: csv, json, text, auto)");
                        chart_usage();
                        return Err(2);
                    }
                }
            }
            "--output" => {
                let Some(value) = iter.next() else {
                    eprintln!("error: option {arg} requires a path");
                    chart_usage();
                    return Err(2);
                };
                output = Some(PathBuf::from(value));
            }
            "-h" | "--help" => {
                chart_usage();
                return Err(0);
            }
            _ if arg.starts_with('-') => {
                eprintln!("error: unknown option {arg}");
                chart_usage();
                return Err(2);
            }
            _ => positional.push(arg.clone()),
        }
    }
    if let Some(name) = &provider {
        if !Provider::CHART_PROVIDERS
            .iter()
            .any(|provider| provider.name() == name.to_ascii_lowercase())
        {
            let valid = Provider::CHART_PROVIDERS
                .iter()
                .map(|provider| provider.name())
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("error: unknown provider {name:?} for chart (valid: {valid})");
            chart_usage();
            return Err(2);
        }
    }
    if positional.len() < 2 {
        chart_usage();
        return Err(2);
    }
    if positional.len() > 2 {
        eprintln!("error: too many arguments for chart (expected SOURCE TARGET)");
        chart_usage();
        return Err(2);
    }
    if let (Some(from), Some(to)) = (from, to) {
        if from > to {
            eprintln!("error: --from {from} must not be after --to {to}");
            chart_usage();
            return Err(2);
        }
    }
    Ok(Command::Chart(ChartArgs {
        force,
        provider,
        from,
        to,
        format,
        output,
        source: positional[0].to_uppercase(),
        target: positional[1].to_uppercase(),
    }))
}

fn usage() {
    eprintln!(
        "Usage: huobi [options] AMOUNT SOURCE [TARGET...]
       huobi chart [options] SOURCE TARGET

Offline currency converter. With no targets, shows the multi-currency view;
explicit targets are listed first, followed by the default multi-currency list.

Options:
  -d, --date <date>       use the ECB historical rate for a date (YYYY-MM-DD)
  -u, --update            force-refresh rates (ignore cache age)
  -p, --provider <name>   rates source: frankfurter (default) or exchange-api
  -h, --help              show this help and exit 0"
    );
}

fn chart_usage() {
    eprintln!(
        "Usage: huobi chart [options] SOURCE TARGET

Historical exchange-rate chart from ECB reference rates (1 SOURCE = x TARGET).

Options:
  --from <date>           inclusive start date (YYYY-MM-DD, default: earliest data)
  --to <date>             inclusive end date (YYYY-MM-DD, default: latest data)
  --format <format>       csv, json, text, or auto (default: auto)
  --output <path>         write the chart to a file instead of stdout
  -p, --provider <name>   history source: ecb (default)
  -u, --update            force re-download of historical rates
  -h, --help              show this help"
    );
}
