// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Chart output: CSV / JSON / text (textplots braille chart), plus
// terminal size detection for width adaptation.

use chrono::NaiveDate;
use textplots::{Chart, LabelBuilder, LabelFormat, Plot, Shape, TickDisplay, TickDisplayBuilder};

use crate::series::Point;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Text,
    Auto,
}

impl Format {
    pub fn from_name(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "csv" => Some(Format::Csv),
            "json" => Some(Format::Json),
            "text" => Some(Format::Text),
            "auto" => Some(Format::Auto),
            _ => None,
        }
    }
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

/// Draw a chart's axes and figures and return the labeled frame as a
/// string. textplots' own `display()` prints to stdout, and its builder
/// methods borrow the chart for the shape's lifetime, so the builder
/// chain is one expression (see `render_text`) and the drawing happens
/// here, where borrows are sequential.
fn chart_to_string(chart: &mut Chart<'_>) -> String {
    chart.axis();
    chart.figures();
    format!("{chart}")
}

/// Render the series as a text chart with textplots. `cols` is the
/// terminal width in characters; the canvas is at least 32 dots wide
/// (16 characters), which textplots requires. The x axis maps dates to
/// days since epoch, the y axis to the cross rate.
pub fn render_text(points: &[Point], source: &str, target: &str, cols: usize) -> String {
    // 16 rows of braille text = 64 dots; TickDisplay::Sparse rounds the
    // canvas height to a multiple of 16, so the two stay in sync.
    const ROWS: u32 = 16;

    let (mut x_min, mut x_max) = (days(points[0].0), days(points[points.len() - 1].0));
    if x_min == x_max {
        // Single point: widen the x range so the axis is not empty.
        x_min -= 1;
        x_max += 1;
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

    let data: Vec<(f32, f32)> = points
        .iter()
        .map(|(date, rate)| (days(*date) as f32, *rate as f32))
        .collect();
    let x_label = |value: f32| -> String {
        let date = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
            + chrono::Duration::days(value.round() as i64);
        date.format("%Y-%m-%d").to_string()
    };
    let width = (cols as u32).max(32) * 2;
    let shape = if points.len() == 1 {
        Shape::Points(&data)
    } else {
        Shape::Lines(&data)
    };
    // The builder methods borrow the chart for the shape's lifetime, so
    // the whole chain must be a single expression.
    let chart = chart_to_string(
        Chart::new_with_y_range(
            width,
            ROWS * 4,
            x_min as f32,
            x_max as f32,
            y_min as f32,
            y_max as f32,
        )
        .x_label_format(LabelFormat::Custom(Box::new(x_label)))
        .y_label_format(LabelFormat::Custom(Box::new(|value| {
            fmt_value(value as f64)
        })))
        .y_tick_display(TickDisplay::Sparse)
        .lineplot(&shape),
    );
    format!("{source} \u{2192} {target}\n{chart}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn has_braille(text: &str) -> bool {
        text.chars().any(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
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
    fn text_chart_has_title_frame_labels_and_range() {
        let points = vec![
            (date("2025-01-02"), 7.0),
            (date("2025-01-03"), 7.5),
            (date("2025-01-06"), 8.0),
            (date("2025-01-07"), 7.2),
        ];
        let text = render_text(&points, "USD", "CNY", 60);
        assert!(text.starts_with("USD \u{2192} CNY\n"));
        assert!(has_braille(&text));
        assert!(text.contains("2025-01-02"));
        assert!(text.contains("2025-01-07"));
        assert!(text.contains("8"));
        assert!(text.contains("7"));
        let lines: Vec<&str> = text.lines().collect();
        // title + 17 braille rows + x-axis label line
        assert_eq!(lines.len(), 19);
        assert!(lines.iter().all(|line| line.chars().count() <= 68));
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
    fn text_chart_handles_single_point() {
        let points = vec![(date("2025-01-02"), 7.3)];
        let text = render_text(&points, "USD", "CNY", 40);
        // The x range is widened around the single point.
        assert!(text.contains("2025-01-01"));
        assert!(text.contains("2025-01-03"));
        assert!(has_braille(&text));
    }
}
