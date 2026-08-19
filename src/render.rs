// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Chart output: CSV / JSON / PNG (plotters) / terminal image printing
// (viuer) / text fallback (Unicode half-blocks), plus terminal protocol
// detection and size adaptation.

use chrono::NaiveDate;
use image::{DynamicImage, RgbImage};
use plotters::prelude::*;
use std::env;
use std::error::Error;
use std::io::{self, Cursor, IsTerminal};

use crate::series::Point;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Png,
    Auto,
}

impl Format {
    pub fn from_name(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "csv" => Some(Format::Csv),
            "json" => Some(Format::Json),
            "png" => Some(Format::Png),
            "auto" => Some(Format::Auto),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Auto,
    Kitty,
    Sixel,
    Text,
}

impl Protocol {
    pub fn from_name(name: &str) -> Option<Protocol> {
        match name.to_ascii_lowercase().as_str() {
            "auto" => Some(Protocol::Auto),
            "kitty" => Some(Protocol::Kitty),
            "sixel" => Some(Protocol::Sixel),
            "text" => Some(Protocol::Text),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayProtocol {
    Kitty,
    Sixel,
    Text,
}

pub fn render_csv(points: &[Point]) -> String {
    let mut out = String::from("date,rate\n");
    for (date, rate) in points {
        out.push_str(&format!("{date},{rate}\n"));
    }
    out
}

pub fn render_json(points: &[Point], source: &str, target: &str) -> String {
    let rates: Vec<serde_json::Value> = points
        .iter()
        .map(|(date, rate)| serde_json::json!({ "date": date.to_string(), "rate": rate }))
        .collect();
    serde_json::json!({
        "source": source,
        "target": target,
        "points": rates,
    })
    .to_string()
}

fn days(date: NaiveDate) -> i64 {
    date.signed_duration_since(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        .num_days()
}

/// Chart label font: a subset of DejaVu Sans (printable ASCII + arrows),
/// embedded so PNG rendering has no system font dependency.
/// See assets/DejaVuSans-LICENSE.txt.
const CHART_FONT: &[u8] = include_bytes!("../assets/DejaVuSans-subset.ttf");

fn register_chart_font() -> Result<(), Box<dyn Error>> {
    plotters::style::register_font("sans-serif", plotters::style::FontStyle::Normal, CHART_FONT)
        .map_err(|_| crate::boxed_error("failed to register chart font"))
}

fn x_label_day(days: &i64) -> String {
    let date = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap() + chrono::Duration::days(*days);
    date.format("%Y-%m-%d").to_string()
}

fn x_label_month(days: &i64) -> String {
    let date = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap() + chrono::Duration::days(*days);
    date.format("%Y-%m").to_string()
}

fn y_label(value: &f64) -> String {
    fmt_value(*value)
}

/// Compact value formatting for chart labels.
pub fn fmt_value(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let abs = value.abs();
    let decimals = if abs >= 100.0 {
        1
    } else if abs >= 10.0 {
        2
    } else if abs >= 1.0 {
        3
    } else if abs >= 0.01 {
        4
    } else {
        6
    };
    let mut out = format!("{value:.decimals$}");
    while out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    if out == "-0" {
        out = "0".to_owned();
    }
    out
}

/// Render the series as a PNG, `width` x `height` pixels. Dates map to
/// days since epoch on the x axis so no extra chrono features are needed.
pub fn render_png(
    points: &[Point],
    source: &str,
    target: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if width == 0 || height == 0 {
        return Err(crate::boxed_error("image dimensions must be non-zero"));
    }
    register_chart_font()?;
    let mut buf = vec![0u8; width as usize * height as usize * 3];
    {
        let root = BitMapBackend::with_buffer(&mut buf, (width, height)).into_drawing_area();
        root.fill(&WHITE)?;
        let (mut x0, mut x1) = (days(points[0].0), days(points[points.len() - 1].0));
        if x0 == x1 {
            // Single point: widen the x range so the axis is not empty.
            x0 -= 1;
            x1 += 1;
        }
        let (mut y_min, mut y_max) = points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), point| {
                (lo.min(point.1), hi.max(point.1))
            });
        if y_max <= y_min {
            // Flat series: give the y axis a small band so it is not empty.
            let band = if y_min == 0.0 {
                1.0
            } else {
                y_min.abs() * 0.05
            };
            y_min -= band;
            y_max += band;
        } else {
            let pad = (y_max - y_min) * 0.05;
            y_min -= pad;
            y_max += pad;
        }
        let mut chart = ChartBuilder::on(&root)
            .caption(format!("{source} \u{2192} {target}"), ("sans-serif", 24))
            .margin(12)
            .x_label_area_size(28)
            .y_label_area_size(52)
            .build_cartesian_2d(x0..x1, y_min..y_max)?;
        let label: fn(&i64) -> String = if x1 - x0 > 150 {
            x_label_month
        } else {
            x_label_day
        };
        chart
            .configure_mesh()
            .x_labels(6)
            .y_labels(6)
            .x_label_formatter(&label)
            .y_label_formatter(&y_label)
            .axis_desc_style(("sans-serif", 14))
            .draw()?;
        if points.len() == 1 {
            chart.draw_series(PointSeries::of_element(
                points.iter().map(|(date, rate)| (days(*date), *rate)),
                5,
                &RGBColor(25, 108, 240),
                &|coord, size, style| {
                    plotters::element::EmptyElement::at(coord)
                        + plotters::element::Circle::new((0, 0), size, style.filled())
                },
            ))?;
        } else {
            chart.draw_series(LineSeries::new(
                points.iter().map(|(date, rate)| (days(*date), *rate)),
                &RGBColor(25, 108, 240),
            ))?;
        }
        root.present()?;
    }
    let image = DynamicImage::ImageRgb8(
        RgbImage::from_raw(width, height, buf)
            .ok_or_else(|| crate::boxed_error("failed to allocate image buffer"))?,
    );
    let mut png = Vec::new();
    image.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(png)
}

/// Terminal size in columns and rows via TIOCGWINSZ, falling back to
/// 80x24 when unavailable (including non-Unix platforms).
pub fn terminal_size() -> (u16, u16) {
    #[cfg(unix)]
    {
        // SAFETY: ws is written by the ioctl before we read it, and the
        // pointer is valid for the duration of the call.
        let mut ws = std::mem::MaybeUninit::<libc::winsize>::uninit();
        let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, ws.as_mut_ptr()) };
        if result == 0 {
            // SAFETY: ioctl success means ws is initialized.
            let ws = unsafe { ws.assume_init() };
            if ws.ws_col > 0 && ws.ws_row > 0 {
                return (ws.ws_col, ws.ws_row);
            }
        }
    }
    (80, 24)
}

#[derive(Debug, Default)]
struct ProtocolEnv<'a> {
    kitty_window_id: Option<&'a str>,
    term: Option<&'a str>,
    term_program: Option<&'a str>,
}

fn detect_protocol_from(env: &ProtocolEnv, is_tty: bool) -> DisplayProtocol {
    if !is_tty {
        return DisplayProtocol::Text;
    }
    let kitty = env.kitty_window_id.is_some()
        || env.term.map(|term| term == "xterm-kitty").unwrap_or(false);
    if kitty {
        return DisplayProtocol::Kitty;
    }
    let term = env.term.unwrap_or("");
    let term_program = env.term_program.unwrap_or("");
    let sixel = [
        "xterm", "foot", "wezterm", "mlterm", "contour", "vt340", "vt330",
    ]
    .iter()
    .any(|candidate| term.contains(candidate))
        || ["foot", "wezterm", "contour"]
            .iter()
            .any(|candidate| term_program.contains(candidate));
    if sixel {
        DisplayProtocol::Sixel
    } else {
        DisplayProtocol::Text
    }
}

/// Detect the terminal graphics protocol: explicit env hints, TTY check,
/// kitty, sixel, then text fallback.
pub fn detect_protocol() -> DisplayProtocol {
    detect_protocol_from(
        &ProtocolEnv {
            kitty_window_id: env::var_os("KITTY_WINDOW_ID")
                .and_then(|value| value.into_string().ok())
                .as_deref(),
            term: env::var("TERM").ok().as_deref(),
            term_program: env::var("TERM_PROGRAM").ok().as_deref(),
        },
        io::stdout().is_terminal(),
    )
}

/// Environment-only kitty hint (no TTY check). viuer's own support probe
/// is interactive and can hang on terminals that do not answer control
/// queries, so forced `--protocol kitty` is gated on this.
pub fn kitty_env_hint() -> bool {
    env::var_os("KITTY_WINDOW_ID").is_some()
        || env::var("TERM")
            .map(|term| term == "xterm-kitty")
            .unwrap_or(false)
}

/// Environment-only sixel hint (no TTY check), for the same hang guard.
pub fn sixel_env_hint() -> bool {
    let term = env::var("TERM").unwrap_or_default();
    let term_program = env::var("TERM_PROGRAM").unwrap_or_default();
    [
        "xterm", "foot", "wezterm", "mlterm", "contour", "vt340", "vt330",
    ]
    .iter()
    .any(|candidate| term.contains(candidate))
        || ["foot", "wezterm", "contour"]
            .iter()
            .any(|candidate| term_program.contains(candidate))
}

/// Print a PNG to the terminal through viuer using the given protocol.
pub fn print_terminal(
    png: &[u8],
    protocol: DisplayProtocol,
    cols: u16,
    rows: u16,
) -> Result<(), Box<dyn Error>> {
    let image = image::load_from_memory(png)?;
    let mut config = viuer::Config {
        absolute_offset: false,
        x: 0,
        y: 0,
        width: Some(cols as u32),
        height: Some(rows as u32),
        use_iterm: false,
        ..Default::default()
    };
    match protocol {
        DisplayProtocol::Kitty => {
            config.use_kitty = true;
            config.use_sixel = false;
        }
        DisplayProtocol::Sixel => {
            config.use_kitty = false;
            config.use_sixel = true;
        }
        DisplayProtocol::Text => {
            return Err(crate::boxed_error(
                "text protocol cannot print terminal images",
            ))
        }
    }
    viuer::print(&image, &config)
        .map_err(|error| crate::boxed_error(format!("failed to print image: {error}")))?;
    Ok(())
}

/// Unicode half-block / ASCII fallback chart.
///
/// Renders a fixed 15-row band chart with a left value label column and a
/// footer with the date range. Single-column bands use the min/max of the
/// points that fall in that column.
pub fn render_text(points: &[Point], source: &str, target: &str, cols: usize) -> String {
    const ROWS: usize = 15;
    const MARGIN: usize = 10;

    let (x0, x1) = (days(points[0].0), days(points[points.len() - 1].0));
    let (mut y_min, mut y_max) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), point| {
            (lo.min(point.1), hi.max(point.1))
        });
    if y_max <= y_min {
        let band = if y_min == 0.0 {
            1.0
        } else {
            y_min.abs() * 0.05
        };
        y_min -= band;
        y_max += band;
    }

    let plot_cols = cols.saturating_sub(MARGIN + 2).max(1);
    let pixels = ROWS * 2;
    let span = (x1 - x0).max(1) as f64;
    let mut bands: Vec<Option<(f64, f64)>> = vec![None; plot_cols];
    for (date, rate) in points {
        let column =
            (((days(*date) - x0) as f64) / span * (plot_cols as f64 - 1.0)).round() as usize;
        let band = bands[column].get_or_insert((*rate, *rate));
        band.0 = band.0.min(*rate);
        band.1 = band.1.max(*rate);
    }

    let to_pixel =
        |value: f64| ((value - y_min) / (y_max - y_min) * (pixels as f64 - 1.0)).round() as usize;
    let cell = |top_filled: bool, bottom_filled: bool| match (top_filled, bottom_filled) {
        (true, true) => '\u{2588}',
        (true, false) => '\u{2580}',
        (false, true) => '\u{2584}',
        (false, false) => ' ',
    };

    let mut out = String::new();
    out.push_str(&format!("{source} \u{2192} {target}\n"));
    for row in 0..ROWS {
        let label = if row == 0 {
            Some(y_max)
        } else if row == ROWS / 3 {
            Some(y_max - (y_max - y_min) / 3.0)
        } else if row == 2 * ROWS / 3 {
            Some(y_min + (y_max - y_min) / 3.0)
        } else if row == ROWS - 1 {
            Some(y_min)
        } else {
            None
        };
        let label = label
            .map(|value| format!("{:>width$}", fmt_value(value), width = MARGIN))
            .unwrap_or_else(|| " ".repeat(MARGIN));
        out.push_str(&label);
        out.push(' ');
        for band in &bands {
            let Some((lo, hi)) = band else {
                out.push(' ');
                continue;
            };
            let top = to_pixel(*hi);
            let bottom = to_pixel(*lo);
            let top_pixel = row * 2;
            let bottom_pixel = row * 2 + 1;
            let filled = |pixel: usize| top <= pixel && pixel <= bottom;
            out.push(cell(filled(top_pixel), filled(bottom_pixel)));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "{:>width$}  {} .. {}\n",
        "",
        points[0].0,
        points[points.len() - 1].0,
        width = MARGIN + 1
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn csv_has_header_and_rows() {
        let points = vec![(date("2025-01-02"), 7.8), (date("2025-01-03"), 7.9)];
        let csv = render_csv(&points);
        assert_eq!(csv, "date,rate\n2025-01-02,7.8\n2025-01-03,7.9\n");
    }

    #[test]
    fn json_has_source_target_and_points() {
        let points = vec![(date("2025-01-02"), 7.8)];
        let json: serde_json::Value =
            serde_json::from_str(&render_json(&points, "USD", "CNY")).unwrap();
        assert_eq!(json["source"], "USD");
        assert_eq!(json["target"], "CNY");
        assert_eq!(json["points"][0]["date"], "2025-01-02");
        assert_eq!(json["points"][0]["rate"], 7.8);
    }

    #[test]
    fn value_formatting_is_compact() {
        assert_eq!(fmt_value(7.2985), "7.298");
        assert_eq!(fmt_value(190.5), "190.5");
        assert_eq!(fmt_value(0.000023), "0.000023");
        assert_eq!(fmt_value(1.0), "1");
        assert_eq!(fmt_value(-7.3), "-7.3");
        assert_eq!(fmt_value(0.0), "0");
        assert_eq!(fmt_value(1234.56), "1234.6");
    }

    #[test]
    fn protocol_detection_prefers_kitty_then_sixel_then_text() {
        // Not a TTY: never emit escapes.
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: Some("1"),
                    term: Some("xterm-kitty"),
                    term_program: None,
                },
                false
            ),
            DisplayProtocol::Text
        );
        // Kitty env var wins.
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: Some("1"),
                    term: Some("xterm-256color"),
                    term_program: None,
                },
                true
            ),
            DisplayProtocol::Kitty
        );
        // TERM=xterm-kitty.
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: None,
                    term: Some("xterm-kitty"),
                    term_program: None,
                },
                true
            ),
            DisplayProtocol::Kitty
        );
        // Sixel terminals.
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: None,
                    term: Some("foot"),
                    term_program: None,
                },
                true
            ),
            DisplayProtocol::Sixel
        );
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: None,
                    term: Some("xterm-256color"),
                    term_program: Some("WezTerm"),
                },
                true
            ),
            DisplayProtocol::Sixel
        );
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: None,
                    term: Some("contour"),
                    term_program: None,
                },
                true
            ),
            DisplayProtocol::Sixel
        );
        // xterm is in the sixel list per plan.
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: None,
                    term: Some("xterm-256color"),
                    term_program: None,
                },
                true
            ),
            DisplayProtocol::Sixel
        );
        // Nothing known: text.
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: None,
                    term: Some("screen-256color"),
                    term_program: None,
                },
                true
            ),
            DisplayProtocol::Text
        );
        assert_eq!(
            detect_protocol_from(
                &ProtocolEnv {
                    kitty_window_id: None,
                    term: None,
                    term_program: None,
                },
                true
            ),
            DisplayProtocol::Text
        );
    }

    #[test]
    fn text_chart_contains_blocks_labels_and_range() {
        let points = vec![
            (date("2025-01-02"), 7.0),
            (date("2025-01-03"), 7.5),
            (date("2025-01-06"), 8.0),
            (date("2025-01-07"), 7.2),
        ];
        let text = render_text(&points, "USD", "CNY", 60);
        assert!(text.starts_with("USD \u{2192} CNY\n"));
        assert!(
            text.contains('\u{2588}') || text.contains('\u{2580}') || text.contains('\u{2584}')
        );
        assert!(text.contains("8"));
        assert!(text.contains("7"));
        assert!(text.contains("2025-01-02 .. 2025-01-07"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 17); // title + 15 rows + footer
        assert!(lines.iter().all(|line| line.chars().count() <= 60));
    }

    #[test]
    fn text_chart_handles_flat_series() {
        let points = vec![(date("2025-01-02"), 7.3), (date("2025-01-03"), 7.3)];
        let text = render_text(&points, "USD", "CNY", 40);
        // Axis padded around the flat value: 7.3 ± 5%.
        assert!(text.contains("7.665"));
        assert!(text.contains("6.935"));
    }

    #[test]
    fn png_renders_and_decodes() {
        let points = vec![
            (date("2025-01-02"), 7.0),
            (date("2025-01-03"), 7.5),
            (date("2025-01-06"), 8.0),
        ];
        let png = render_png(&points, "USD", "CNY", 320, 160).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!(decoded.width(), 320);
        assert_eq!(decoded.height(), 160);
    }

    #[test]
    fn png_rejects_zero_dimensions() {
        let points = vec![(date("2025-01-02"), 7.0), (date("2025-01-03"), 7.5)];
        assert!(render_png(&points, "USD", "CNY", 0, 100).is_err());
    }
}
