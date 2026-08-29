// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Chart output: CSV / JSON / text (a stats panel above a textplots braille
// chart sized to the terminal), plus terminal size detection.

use chrono::NaiveDate;
use textplots::{Chart, LabelBuilder, LabelFormat, Plot, Shape, TickDisplay, TickDisplayBuilder};

use crate::series::{stats as series_stats, Point, Stats};

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

/// Downsample a date-ascending series to at most `threshold` points with the
/// Largest-Triangle-Three-Buckets (LTTB) algorithm, always keeping the first
/// and last points. LTTB selects the point in each bucket that forms the
/// largest triangle with the previously chosen point and the average of the
/// next bucket, preserving the visual shape far better than uniform sampling.
/// X is the day index; it is unevenly spaced because weekends/holidays are
/// removed from the series, so the triangle areas use the real x values.
fn lttb(data: &[Point], threshold: usize) -> Vec<Point> {
    let n = data.len();
    if threshold >= n || threshold <= 2 {
        return data.to_vec();
    }
    let mut sampled: Vec<Point> = Vec::with_capacity(threshold);
    let every = (n as f64 - 2.0) / (threshold as f64 - 2.0);
    sampled.push(data[0]);
    let mut a = 0usize;
    for i in 0..threshold - 2 {
        // Average point (c) of the next bucket.
        let avg_start = ((i as f64 + 1.0) * every).floor() as usize + 1;
        let mut avg_end = ((i as f64 + 2.0) * every).floor() as usize + 1;
        if avg_end > n {
            avg_end = n;
        }
        let avg_len = avg_end - avg_start;
        let mut avg_x = 0.0f64;
        let mut avg_y = 0.0f64;
        for point in data.iter().take(avg_end).skip(avg_start) {
            avg_x += days(point.0) as f64;
            avg_y += point.1;
        }
        if avg_len > 0 {
            avg_x /= avg_len as f64;
            avg_y /= avg_len as f64;
        } else {
            // Degenerate final bucket: c is the last point itself.
            avg_x = days(data[n - 1].0) as f64;
            avg_y = data[n - 1].1;
        }
        // Current bucket range.
        let range_offs = (i as f64 * every).floor() as usize + 1;
        let range_to = ((i as f64 + 1.0) * every).floor() as usize + 1;
        let point_a_x = days(data[a].0) as f64;
        let point_a_y = data[a].1;
        let mut max_area = -1.0f64;
        let mut max_area_point = range_offs;
        for (j, point) in data.iter().enumerate().take(range_to).skip(range_offs) {
            // Area of triangle (a, candidate, c) via the shoelace formula.
            let area = ((point_a_x - avg_x) * (point.1 - point_a_y)
                - (point_a_x - days(point.0) as f64) * (avg_y - point_a_y))
                .abs()
                * 0.5;
            if area > max_area {
                max_area = area;
                max_area_point = j;
            }
        }
        sampled.push(data[max_area_point]);
        a = max_area_point;
    }
    sampled.push(data[n - 1]);
    sampled
}

/// ANSI foregrounds used when coloring is on: bright black for the chrome
/// (borders, axes, tick labels), green/red for the change cell.
const DIM: &str = "\x1b[90m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

/// Cell width in terminal columns. Everything generated here is ASCII except
/// box-drawing characters (1 cell each, so plain char counting is right) and
/// the status emoji (East-Asian wide); the CJK ranges are covered defensively
/// so panel alignment survives non-ASCII content in future.
fn char_width(c: char) -> usize {
    match c as u32 {
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F000..=0x1FAFF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// One label/value pair of the stats panel. `tint` is the ANSI color for the
/// value (the change cell); labels are always bright-black when coloring.
struct StatCell {
    label: &'static str,
    value: String,
    tint: Option<&'static str>,
}

fn cell_text(cell: &StatCell) -> String {
    format!("{}: {}", cell.label, cell.value)
}

fn cell_painted(cell: &StatCell, color: bool) -> String {
    if !color {
        return cell_text(cell);
    }
    match cell.tint {
        Some(tint) => format!("{DIM}{}:{RESET} {tint}{}{RESET}", cell.label, cell.value),
        None => format!("{DIM}{}:{RESET} {}", cell.label, cell.value),
    }
}

/// A rounded-rule border with the title set into the dashes (`╭── T ──╮`).
fn border_rule(left: char, right: char, title: &str, inner_w: usize, color: bool) -> String {
    let mut line = String::new();
    line.push(left);
    let title_w = display_width(title);
    if title.is_empty() || title_w + 2 > inner_w {
        line.push_str(&"─".repeat(inner_w));
    } else {
        let gap = inner_w - title_w - 2;
        line.push_str(&"─".repeat(gap / 2));
        line.push_str(&format!(" {title} "));
        line.push_str(&"─".repeat(gap - gap / 2));
    }
    line.push(right);
    if color {
        format!("{DIM}{line}{RESET}")
    } else {
        line
    }
}

/// Render the stats box shown above the chart: current value (with its date),
/// high/low with the extreme dates, the signed (green/red) range change, the
/// average, and the amplitude `(high - low) / average`. Two stat columns when
/// they fit in `cols`, one stat per line otherwise. Returns the box without a
/// trailing newline plus its line count.
fn stats_panel(
    stats: &Stats,
    source: &str,
    target: &str,
    cols: usize,
    color: bool,
) -> (String, usize) {
    let at = |rate: f64, date: NaiveDate| format!("{} ({})", fmt_value(rate), date);
    let change = if stats.change_pct > 0.0 {
        StatCell {
            label: "Change",
            value: format!("🟢 +{:.2}%", stats.change_pct),
            tint: Some(GREEN),
        }
    } else if stats.change_pct < 0.0 {
        StatCell {
            label: "Change",
            value: format!("🔴 -{:.2}%", stats.change_pct.abs()),
            tint: Some(RED),
        }
    } else {
        StatCell {
            label: "Change",
            value: "±0.00%".to_owned(),
            tint: None,
        }
    };
    let cells = [
        StatCell {
            label: "Current",
            value: at(stats.current, stats.current_date),
            tint: None,
        },
        StatCell {
            label: "Average",
            value: fmt_value(stats.average),
            tint: None,
        },
        StatCell {
            label: "High",
            value: at(stats.high, stats.high_date),
            tint: None,
        },
        StatCell {
            label: "Low",
            value: at(stats.low, stats.low_date),
            tint: None,
        },
        change,
        StatCell {
            label: "Volatility",
            value: format!("{:.2}%", stats.volatility_pct),
            tint: None,
        },
    ];
    let width = |cell: &StatCell| display_width(&cell_text(cell));
    let pairs = [
        (&cells[0], &cells[1]),
        (&cells[2], &cells[3]),
        (&cells[4], &cells[5]),
    ];
    let left_w = pairs.iter().map(|(l, _)| width(l)).max().unwrap_or(0);
    let two_col_w = pairs
        .iter()
        .map(|(_, r)| left_w + 3 + width(r))
        .max()
        .unwrap_or(0)
        + 4; // side padding + borders
    let (raw, painted): (Vec<String>, Vec<String>) = if two_col_w <= cols.max(32) {
        let sep = if color {
            format!("{DIM} | {RESET}")
        } else {
            " | ".to_owned()
        };
        let raw = pairs
            .iter()
            .map(|(l, r)| {
                format!(
                    "{}{} | {}",
                    cell_text(l),
                    " ".repeat(left_w - width(l)),
                    cell_text(r)
                )
            })
            .collect();
        let painted = pairs
            .iter()
            .map(|(l, r)| {
                format!(
                    "{}{}{sep}{}",
                    cell_painted(l, color),
                    " ".repeat(left_w - width(l)),
                    cell_painted(r, color)
                )
            })
            .collect();
        (raw, painted)
    } else {
        let raw = cells.iter().map(cell_text).collect();
        let painted = cells.iter().map(|c| cell_painted(c, color)).collect();
        (raw, painted)
    };

    let inner_w = raw.iter().map(|l| display_width(l)).max().unwrap_or(0) + 2;
    let side = if color {
        format!("{DIM}│{RESET}")
    } else {
        "│".to_owned()
    };
    let mut out = border_rule(
        '╭',
        '╮',
        &format!("{source}/{target} Trend"),
        inner_w,
        color,
    );
    for (line_raw, line_painted) in raw.iter().zip(&painted) {
        let pad = inner_w - 1 - display_width(line_raw);
        out.push('\n');
        out.push_str(&format!("{side} {line_painted}{}{side}", " ".repeat(pad)));
    }
    out.push('\n');
    out.push_str(&border_rule('╰', '╯', "", inner_w, color));
    let rows = painted.len() + 2;
    (out, rows)
}

/// Braille rows for the chart: half the terminal height capped at 25 (the
/// stats panel keeps the other half) and further bounded by what is left
/// under the panel, floored at 9. Snapped to a `4k+1` row count so the dot
/// height stays a multiple of 16 — otherwise `TickDisplay::Sparse` rounds
/// the canvas and the chart silently differs from the requested height.
fn chart_rows(term_rows: u32, panel_rows: usize) -> u32 {
    let avail = term_rows.saturating_sub(panel_rows as u32 + 1);
    let target = (term_rows / 2).min(avail).clamp(9, 25);
    let k = ((target - 1) as f64 / 4.0).round() as u32;
    4 * k + 1
}

/// Wrap every run of axis/label characters (anything that is not braille or
/// whitespace) in bright black so the plotted line keeps the visual focus.
fn dim_axes(chart: &str) -> String {
    let mut out = String::with_capacity(chart.len() + 64);
    let mut run = String::new();
    for c in chart.chars() {
        let keep = c == ' ' || c == '\n' || ('\u{2800}'..='\u{28ff}').contains(&c);
        if keep {
            if !run.is_empty() {
                out.push_str(DIM);
                out.push_str(&run);
                out.push_str(RESET);
                run.clear();
            }
            out.push(c);
        } else {
            run.push(c);
        }
    }
    if !run.is_empty() {
        out.push_str(DIM);
        out.push_str(&run);
        out.push_str(RESET);
    }
    out
}

/// Render the series as a text chart: the stats box above a textplots braille
/// chart. `cols`/`rows` are the terminal size; the canvas is at least 32 dots
/// (16 characters) wide, which textplots requires. The x axis maps dates to
/// days since epoch, the y axis to the cross rate. `color` emits the ANSI
/// bright-black chrome and green/red change; callers writing to a file or a
/// pipe must pass `false`.
pub fn render_text(
    points: &[Point],
    source: &str,
    target: &str,
    cols: usize,
    rows: u32,
    color: bool,
) -> String {
    // Braille cells overplot and smear the line ("ghosting") once the series
    // is denser than the canvas. Beyond 200 points, downsample with LTTB,
    // which preserves the shape while keeping the first and last dates (and
    // therefore the x range) intact. CSV/JSON exports stay full-resolution.
    const MAX_POINTS: usize = 200;

    let stats = series_stats(points).expect("render_text requires a non-empty series");
    let (panel, panel_rows) = stats_panel(&stats, source, target, cols, color);

    let sampled: Vec<Point> = if points.len() > MAX_POINTS {
        lttb(points, MAX_POINTS)
    } else {
        points.to_vec()
    };
    let points = &sampled;

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
    let dot_height = (chart_rows(rows, panel_rows) - 1) * 4;
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
            dot_height,
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
    let chart = if color { dim_axes(&chart) } else { chart };
    format!("{panel}\n{chart}")
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
    fn text_chart_has_panel_frame_labels_and_range() {
        let points = vec![
            (date("2025-01-02"), 7.0),
            (date("2025-01-03"), 7.5),
            (date("2025-01-06"), 8.0),
            (date("2025-01-07"), 7.2),
        ];
        let text = render_text(&points, "USD", "CNY", 60, 24, false);
        assert!(text.starts_with("╭"));
        assert!(text.contains("USD/CNY Trend"));
        assert!(text.contains("Current: 7.2 (2025-01-07)"));
        assert!(text.contains("High: 8 (2025-01-06)"));
        assert!(text.contains("Low: 7 (2025-01-02)"));
        assert!(text.contains("Average: 7.425"));
        assert!(text.contains("Change: 🟢 +2.86%"));
        assert!(text.contains("Volatility: 13.47%"));
        assert!(has_braille(&text));
        let lines: Vec<&str> = text.lines().collect();
        // panel (2 rules + 3 stat lines) + 13 braille rows + x-axis labels
        assert_eq!(lines.len(), 19);
        assert!(lines.iter().all(|line| line.chars().count() <= 68));
    }

    #[test]
    fn text_chart_handles_flat_series() {
        let points = vec![(date("2025-01-02"), 7.3), (date("2025-01-03"), 7.3)];
        let text = render_text(&points, "USD", "CNY", 40, 24, false);
        // Axis padded around the flat value: 7.3 ± 5%.
        assert!(text.contains("7.665"));
        assert!(text.contains("6.935"));
    }

    #[test]
    fn text_chart_handles_single_point() {
        let points = vec![(date("2025-01-02"), 7.3)];
        let text = render_text(&points, "USD", "CNY", 40, 24, false);
        // The x range is widened around the single point.
        assert!(text.contains("2025-01-01"));
        assert!(text.contains("2025-01-03"));
        assert!(has_braille(&text));
    }

    #[test]
    fn display_width_counts_wide_emoji_twice() {
        assert_eq!(display_width("USD/CNY Trend"), 13);
        assert_eq!(display_width("Change: 🔴 -1.24%"), 17);
    }

    #[test]
    fn panel_is_rectangular_and_collapses_to_one_column() {
        let points = vec![
            (date("2025-01-02"), 7.0),
            (date("2025-01-03"), 7.5),
            (date("2025-01-06"), 6.8),
        ];
        let wide = render_text(&points, "USD", "CNY", 60, 24, false);
        let panel: Vec<&str> = wide.lines().take(5).collect();
        let width = display_width(panel[0]);
        assert!(panel.iter().all(|line| display_width(line) == width));
        assert!(panel[1].contains(" | ")); // two columns fit
        assert!(panel[3].contains("🔴 -2.86%")); // down over the range

        let narrow = render_text(&points, "USD", "CNY", 40, 24, false);
        let panel: Vec<&str> = narrow.lines().take(9).collect();
        let width = display_width(panel[0]);
        assert!(panel
            .iter()
            .take(8)
            .all(|line| display_width(line) == width));
        // one stat per line (rules + 6 stats); the 9th line is chart output
        assert!(panel[7].starts_with('╰'));
        assert!(panel[1..7].iter().all(|line| line.starts_with('│')));
        assert!(!panel.iter().any(|line| line.contains(" | ")));
    }

    #[test]
    fn chart_height_adapts_to_terminal() {
        let base = date("2025-01-02");
        let points: Vec<Point> = (0..60)
            .map(|i| (base + chrono::Duration::days(i), 7.0 + i as f64 * 0.01))
            .collect();
        let braille_rows = |rows: u32| {
            render_text(&points, "USD", "CNY", 60, rows, false)
                .lines()
                .filter(|line| has_braille(line))
                .count()
        };
        assert_eq!(braille_rows(24), 13); // about half the terminal
        assert_eq!(braille_rows(80), 25); // capped at 25
        assert_eq!(braille_rows(10), 9); // floor
    }

    #[test]
    fn color_dims_chrome_and_tints_change() {
        let down = vec![(date("2025-01-02"), 7.5), (date("2025-01-03"), 7.0)];
        let text = render_text(&down, "USD", "CNY", 60, 24, true);
        assert!(text.contains("\u{1b}[90m")); // borders and axes are bright black
        assert!(text.contains("\u{1b}[31m🔴 -6.67%"));
        let up = vec![(date("2025-01-02"), 7.0), (date("2025-01-03"), 7.2)];
        assert!(render_text(&up, "USD", "CNY", 60, 24, true).contains("\u{1b}[32m🟢 +2.86%"));
        assert!(!render_text(&up, "USD", "CNY", 60, 24, false).contains('\u{1b}'));
    }

    #[test]
    fn lttb_returns_full_series_below_threshold() {
        let points = vec![
            (date("2025-01-02"), 7.0),
            (date("2025-01-03"), 7.5),
            (date("2025-01-06"), 8.0),
        ];
        assert_eq!(lttb(&points, 200).len(), 3);
    }

    #[test]
    fn lttb_downsamples_and_keeps_endpoints() {
        // 500 trading days of a sawtooth; dense enough to overplot a braille chart.
        let mut points = Vec::new();
        let base = date("2000-01-01");
        for i in 0..500u32 {
            let d = base + chrono::Duration::days(i as i64);
            let rate = 7.0 + ((i % 9) as f64) * 0.13;
            points.push((d, rate));
        }
        let out = lttb(&points, 200);
        assert_eq!(out.len(), 200);
        // First and last dates (hence the x range) are preserved.
        assert_eq!(out[0], points[0]);
        assert_eq!(*out.last().unwrap(), *points.last().unwrap());
        // LTTB never reorders or duplicates dates.
        for w in out.windows(2) {
            assert!(w[0].0 < w[1].0);
        }
    }

    #[test]
    fn render_text_applies_lttb_above_threshold() {
        let mut points = Vec::new();
        let base = date("2000-01-01");
        for i in 0..500u32 {
            let d = base + chrono::Duration::days(i as i64);
            let rate = 7.0 + ((i % 9) as f64) * 0.13;
            points.push((d, rate));
        }
        let text = render_text(&points, "USD", "CNY", 80, 24, false);
        assert!(text.starts_with("╭"));
        assert!(has_braille(&text));
        // Endpoints remain in the frame after downsampling.
        assert!(text.contains("2000-01-01"));
        assert!(text.contains("2001-05-14"));
    }
}
