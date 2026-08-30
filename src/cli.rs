// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Command-line parsing for the convert and chart commands.

use chrono::NaiveDate;
use std::path::PathBuf;

use crate::provider::Provider;
use crate::render::Format;
use crate::style;

#[derive(Debug, PartialEq)]
pub enum Command {
    Convert(ConvertArgs),
    Chart(ChartArgs),
}

#[derive(Debug, PartialEq)]
pub struct ConvertArgs {
    pub force: bool,
    pub provider: Option<String>,
    pub amount: f64,
    pub source: String,
    pub targets: Vec<String>,
    pub date: Option<NaiveDate>,
}

#[derive(Debug, PartialEq)]
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
    dispatch(&args)
}

fn dispatch(args: &[String]) -> Result<Command, i32> {
    match subcommand_index(args) {
        Some(index) => parse_chart_args(args[..index].iter().chain(args[index + 1..].iter())),
        None => parse_convert_args(args.iter()),
    }
}

// Index of the `chart` subcommand token, or None for a conversion. Option
// values are skipped so `-p chart` is not mistaken for the subcommand; the
// first free token other than `chart` starts convert operands. Unknown
// options are left for the command parsers to reject.
fn subcommand_index(args: &[String]) -> Option<usize> {
    let mut iter = args.iter().enumerate();
    while let Some((index, arg)) = iter.next() {
        let arg = arg.as_str();
        if !arg.starts_with('-') || negative_number(arg) {
            return (arg == "chart").then_some(index);
        }
        if option_takes_value(arg) {
            iter.next();
        }
    }
    None
}

// Options from either command whose next token is a value; only used to
// locate the subcommand.
fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-p" | "--provider" | "-d" | "--date" | "--from" | "--to" | "--format" | "--output"
    )
}

// `-` followed by a digit or a dot is a negative amount, not an option
// (`fxrate -100 USD CNY`); any other `-`-prefixed token stays an option.
fn negative_number(arg: &str) -> bool {
    matches!(arg.as_bytes().get(1), Some(b'0'..=b'9') | Some(b'.'))
}

fn parse_convert_args<'a>(args: impl Iterator<Item = &'a String>) -> Result<Command, i32> {
    let mut force = false;
    let mut provider = None;
    let mut date = None;
    let mut positional = Vec::new();
    let mut iter = args;
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-u" | "--update" => force = true,
            "-p" | "--provider" => {
                let Some(name) = iter.next() else {
                    eprintln!(
                        "{}",
                        style::error(format!("option {arg} requires a provider name"))
                    );
                    usage();
                    return Err(2);
                };
                provider = Some(name.clone());
            }
            "-d" | "--date" => {
                let Some(value) = iter.next() else {
                    eprintln!("{}", style::error(format!("option {arg} requires a date")));
                    usage();
                    return Err(2);
                };
                date = Some(NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
                    eprintln!(
                        "{}",
                        style::error(format!("invalid date {value:?} (expected YYYY-MM-DD)"))
                    );
                    usage();
                    2
                })?);
            }
            "-h" | "--help" => {
                usage();
                return Err(0);
            }
            "-V" | "--version" => {
                println!("fxrate {}", env!("CARGO_PKG_VERSION"));
                return Err(0);
            }
            _ if arg.starts_with('-') && !negative_number(arg) => {
                eprintln!("{}", style::error(format!("unknown option {arg}")));
                usage();
                return Err(2);
            }
            _ => positional.push(arg.clone()),
        }
    }
    if let Some(name) = &provider {
        if !Provider::accepted(name, &Provider::CONVERT_PROVIDERS) {
            let valid = Provider::names(&Provider::CONVERT_PROVIDERS);
            eprintln!(
                "{}",
                style::error(format!("unknown provider {name:?} (valid: {valid})"))
            );
            usage();
            return Err(2);
        }
    }
    if positional.len() < 2 {
        usage();
        return Err(2);
    }
    let amount = positional[0].parse::<f64>().map_err(|_| {
        eprintln!(
            "{}",
            style::error(format!("invalid amount {:?}", positional[0]))
        );
        2
    })?;
    if !amount.is_finite() {
        eprintln!("{}", style::error("amount must be finite"));
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
        eprintln!(
            "{}",
            style::error(format!("invalid date {value:?} (expected YYYY-MM-DD)"))
        );
        chart_usage();
        2
    })
}

fn parse_chart_args<'a>(args: impl Iterator<Item = &'a String>) -> Result<Command, i32> {
    let mut force = false;
    let mut provider: Option<String> = None;
    let mut from = None;
    let mut to = None;
    let mut format = Format::Auto;
    let mut output = None;
    let mut positional = Vec::new();
    let mut iter = args;
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-u" | "--update" => force = true,
            "-p" | "--provider" => {
                let Some(name) = iter.next() else {
                    eprintln!(
                        "{}",
                        style::error(format!("option {arg} requires a provider name"))
                    );
                    chart_usage();
                    return Err(2);
                };
                provider = Some(name.clone());
            }
            "--from" => {
                let Some(value) = iter.next() else {
                    eprintln!("{}", style::error(format!("option {arg} requires a date")));
                    chart_usage();
                    return Err(2);
                };
                from = Some(parse_date(value)?);
            }
            "--to" => {
                let Some(value) = iter.next() else {
                    eprintln!("{}", style::error(format!("option {arg} requires a date")));
                    chart_usage();
                    return Err(2);
                };
                to = Some(parse_date(value)?);
            }
            "--format" => {
                let Some(value) = iter.next() else {
                    eprintln!(
                        "{}",
                        style::error(format!("option {arg} requires a format"))
                    );
                    chart_usage();
                    return Err(2);
                };
                match Format::from_name(value) {
                    Some(parsed) => format = parsed,
                    None => {
                        eprintln!(
                            "{}",
                            style::error(format!(
                                "unknown format {value:?} (valid: csv, json, text, auto)"
                            ))
                        );
                        chart_usage();
                        return Err(2);
                    }
                }
            }
            "--output" => {
                let Some(value) = iter.next() else {
                    eprintln!("{}", style::error(format!("option {arg} requires a path")));
                    chart_usage();
                    return Err(2);
                };
                output = Some(PathBuf::from(value));
            }
            "-h" | "--help" => {
                chart_usage();
                return Err(0);
            }
            "-V" | "--version" => {
                println!("fxrate {}", env!("CARGO_PKG_VERSION"));
                return Err(0);
            }
            _ if arg.starts_with('-') => {
                eprintln!("{}", style::error(format!("unknown option {arg}")));
                chart_usage();
                return Err(2);
            }
            _ => positional.push(arg.clone()),
        }
    }
    if let Some(name) = &provider {
        if !Provider::accepted(name, &Provider::CHART_PROVIDERS) {
            let valid = Provider::names(&Provider::CHART_PROVIDERS);
            eprintln!(
                "{}",
                style::error(format!(
                    "unknown provider {name:?} for chart (valid: {valid})"
                ))
            );
            chart_usage();
            return Err(2);
        }
    }
    if positional.len() < 2 {
        chart_usage();
        return Err(2);
    }
    if positional.len() > 2 {
        eprintln!(
            "{}",
            style::error("too many arguments for chart (expected SOURCE TARGET)")
        );
        chart_usage();
        return Err(2);
    }
    if let (Some(from), Some(to)) = (from, to) {
        if from > to {
            eprintln!(
                "{}",
                style::error(format!("--from {from} must not be after --to {to}"))
            );
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
        "Usage: fxrate [options] AMOUNT SOURCE [TARGET...]
       fxrate chart [options] SOURCE TARGET

Offline currency converter. With no targets, shows the multi-currency view;
explicit targets are listed first, followed by the default multi-currency list.

Options:
  -d, --date <date>       use the ECB historical rate for a date (YYYY-MM-DD)
  -u, --update            force-refresh rates (ignore cache age)
  -p, --provider <name>   rates source: frankfurter (default) or exchange-api
                          (ignored with --date: history is always ECB)
  -h, --help              show this help and exit 0
  -V, --version           print version and exit 0"
    );
}

fn chart_usage() {
    eprintln!(
        "Usage: fxrate chart [options] SOURCE TARGET

Historical exchange-rate chart from ECB reference rates (1 SOURCE = x TARGET).

Options:
  --from <date>           inclusive start date (YYYY-MM-DD, default: earliest data)
  --to <date>             inclusive end date (YYYY-MM-DD, default: latest data)
  --format <format>       csv, json, text, or auto (default: auto)
  --output <path>         write the chart to a file instead of stdout
  -p, --provider <name>   history source: ecb (default)
  -u, --update            force re-download of historical rates
  -h, --help              show this help
  -V, --version           print version and exit 0"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn chart_options_before_the_subcommand_are_parsed() {
        let command = dispatch(&argv(&["-u", "chart", "usd", "cny"])).expect("parsed");
        let Command::Chart(chart) = command else {
            panic!("expected chart, got convert");
        };
        assert!(chart.force);
        assert_eq!(chart.source, "USD");
        assert_eq!(chart.target, "CNY");
        assert_eq!(chart.provider, None);
    }

    #[test]
    fn chart_options_on_both_sides_are_parsed_as_one_stream() {
        let command = dispatch(&argv(&["-p", "ecb", "chart", "USD", "CNY", "-u"])).expect("parsed");
        let Command::Chart(chart) = command else {
            panic!("expected chart, got convert");
        };
        assert!(chart.force);
        assert_eq!(chart.provider.as_deref(), Some("ecb"));
    }

    #[test]
    fn convert_only_option_before_chart_is_a_usage_error() {
        assert_eq!(
            dispatch(&argv(&["-d", "2024-01-02", "chart", "USD", "CNY"])),
            Err(2)
        );
    }

    #[test]
    fn invalid_provider_before_chart_is_a_usage_error() {
        assert_eq!(
            dispatch(&argv(&["-p", "frankfurter", "chart", "USD", "CNY"])),
            Err(2)
        );
    }

    #[test]
    fn help_and_version_before_chart_exit_zero() {
        assert_eq!(dispatch(&argv(&["--help", "chart"])), Err(0));
        assert_eq!(dispatch(&argv(&["-V", "chart", "USD", "CNY"])), Err(0));
    }

    #[test]
    fn option_values_are_never_taken_for_the_subcommand() {
        assert_eq!(
            subcommand_index(&argv(&["-u", "chart", "USD", "CNY"])),
            Some(1)
        );
        assert_eq!(subcommand_index(&argv(&["chart"])), Some(0));
        assert_eq!(subcommand_index(&argv(&[])), None);
        // `-p` consumes "chart" as its value; the first free token is "USD".
        assert_eq!(
            subcommand_index(&argv(&["-p", "chart", "USD", "CNY"])),
            None
        );
        assert_eq!(dispatch(&argv(&["-p", "chart", "USD", "CNY"])), Err(2));
        // A convert amount before any subcommand starts the operands.
        assert_eq!(subcommand_index(&argv(&["1.5", "chart"])), None);
        // "--output chart" consumes the subcommand token as its value.
        assert_eq!(
            dispatch(&argv(&["--output", "chart", "USD", "CNY"])),
            Err(2)
        );
    }

    #[test]
    fn convert_parsing_is_unchanged() {
        let command = dispatch(&argv(&["-u", "1.5", "usd", "cny", "gbp"])).expect("parsed");
        let Command::Convert(convert) = command else {
            panic!("expected convert, got chart");
        };
        assert!(convert.force);
        assert_eq!(convert.source, "USD");
        assert_eq!(convert.targets, vec!["CNY", "GBP"]);
    }

    #[test]
    fn negative_amounts_are_positionals_not_options() {
        let command = dispatch(&argv(&["-100", "USD", "CNY"])).expect("parsed");
        let Command::Convert(convert) = command else {
            panic!("expected convert, got chart");
        };
        assert_eq!(convert.amount, -100.0);
        assert_eq!(convert.source, "USD");
        let command = dispatch(&argv(&["-.5", "USD", "CNY"])).expect("parsed");
        let Command::Convert(convert) = command else {
            panic!("expected convert, got chart");
        };
        assert_eq!(convert.amount, -0.5);
        // A negative amount starts the operands like `1.5 chart` does.
        assert_eq!(subcommand_index(&argv(&["-100", "chart"])), None);
        // `--100` and a bare `-` are not amounts; they stay options.
        assert_eq!(dispatch(&argv(&["--100", "USD", "CNY"])), Err(2));
        assert_eq!(dispatch(&argv(&["-", "USD", "CNY"])), Err(2));
    }
}
