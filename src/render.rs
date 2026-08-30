// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// chart on a fixed 80×15 canvas).

use chrono::{Datelike, NaiveDate};
use textplots::{Chart, LabelBuilder, LabelFormat, Plot, Shape, TickDisplay, TickDisplayBuilder};
use unicode_width::UnicodeWidthStr;

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

/// Terminal size in columns and rows via the `terminal_size` crate, falling
/// back to 80x24 when it is unavailable (e.g. not attached to a terminal).
pub fn terminal_size() -> (u16, u16) {
    match terminal_size::terminal_size() {
        Some((width, height)) if width.0 > 0 && height.0 > 0 => (width.0, height.0),
        _ => (80, 24),
    }
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
/// (borders, axes, tick labels), bright cyan for the plotted line (a chart
/// accent that leaves green/red free to mean the change direction), and
/// green/red for the change cell.
const DIM: &str = "\x1b[90m";
const CYAN: &str = "\x1b[96m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

/// Display columns of a string: unicode-width's `UnicodeWidthStr`, which
/// measures a regional-indicator flag pair as the 2 cells terminals render
/// (per-codepoint counting inflates it to 4) and also covers VS16, ZWJ
/// sequences, and skin-tone modifiers that hand-rolled range tables miss.
pub(crate) fn display_width(s: &str) -> usize {
    s.width()
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

/// Color the chart canvas by character class: runs of braille cells with at
/// least one raised dot (the plotted line) go bright cyan; runs of anything
/// else that is not blank canvas or whitespace (borders, axes, tick labels)
/// go bright black; blank braille cells (U+2800 canvas padding), spaces, and
/// newlines pass through uncolored.
fn dim_axes(chart: &str) -> String {
    let tint_of = |c: char| -> Option<&'static str> {
        if ('\u{2801}'..='\u{28ff}').contains(&c) {
            Some(CYAN)
        } else if c == ' ' || c == '\n' || c == '\u{2800}' {
            None
        } else {
            Some(DIM)
        }
    };
    let mut out = String::with_capacity(chart.len() + 64);
    let mut run = String::new();
    let mut tint: Option<&'static str> = None;
    let mut flush = |tint: &mut Option<&'static str>, run: &mut String| {
        match tint.take() {
            Some(t) => {
                out.push_str(t);
                out.push_str(run);
                out.push_str(RESET);
            }
            None => out.push_str(run),
        }
        run.clear();
    };
    for c in chart.chars() {
        let next = tint_of(c);
        if next != tint {
            flush(&mut tint, &mut run);
            tint = next;
        }
        run.push(c);
    }
    flush(&mut tint, &mut run);
    out
}
/// Smallest "nice" step (1, 2, or 5 times a power of ten) that is at least
/// `raw`, so y-axis tick labels land on round values.
fn nice_step(raw: f64) -> f64 {
    let pow = 10f64.powf(raw.log10().floor());
    let unit = raw / pow;
    let mult = if unit <= 1.0 {
        1.0
    } else if unit <= 2.0 {
        2.0
    } else if unit <= 5.0 {
        5.0
    } else {
        10.0
    };
    mult * pow
}

/// Build the two lines that replace textplots' single x-axis label line: a
/// row of `+` tick marks under the canvas, then the date labels. The first
/// and last dates stay at the edges; intermediate ticks are month starts
/// (ranges of two months or more), Mondays (two weeks to two months), or
/// every third day (shorter ranges), thinned so the labels never overlap.
fn x_axis_lines(x_min: f64, x_max: f64, cols: usize) -> (String, String) {
    const LABEL_W: usize = 10; // YYYY-MM-DD
    let day0 = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let date_at = |x: f64| day0 + chrono::Duration::days(x.round() as i64);
    let first = date_at(x_min);
    let last = date_at(x_max);
    let col_of = |date: NaiveDate| -> usize {
        let frac = (days(date) as f64 - x_min) / (x_max - x_min);
        ((frac * (cols - 1) as f64).round() as usize).min(cols - 1)
    };
    let next_month = |date: NaiveDate| -> NaiveDate {
        if date.month() == 12 {
            NaiveDate::from_ymd_opt(date.year() + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
        }
        .expect("valid month start")
    };
    let mut candidates: Vec<NaiveDate> = Vec::new();
    let range_days = x_max - x_min;
    if range_days >= 60.0 {
        let mut d = next_month(first);
        while d < last {
            candidates.push(d);
            d = next_month(d);
        }
    } else if range_days >= 15.0 {
        let mut d = first + chrono::Duration::days(1);
        while d.weekday() != chrono::Weekday::Mon {
            d += chrono::Duration::days(1);
        }
        while d < last {
            candidates.push(d);
            d += chrono::Duration::days(7);
        }
    } else {
        let mut d = first + chrono::Duration::days(3);
        while d < last {
            candidates.push(d);
            d += chrono::Duration::days(3);
        }
    }
    // Greedy selection: keep a candidate only when its centered label
    // clears the previous one by two columns and leaves room for the
    // right-aligned end label.
    let mut ticks: Vec<(NaiveDate, usize)> = vec![(first, 0)];
    let mut prev_end = LABEL_W - 1;
    let last_start = cols - LABEL_W;
    for date in candidates {
        let col = col_of(date);
        let start = col.saturating_sub(LABEL_W / 2);
        if start >= prev_end + 2 && start + LABEL_W + 2 <= last_start {
            ticks.push((date, col));
            prev_end = start + LABEL_W - 1;
        }
    }
    ticks.push((last, cols - 1));

    let mut marks = vec![' '; cols];
    let mut labels = vec![' '; cols];
    for (date, col) in &ticks {
        marks[*col] = '+';
        let s = date.format("%Y-%m-%d").to_string();
        let start = if *col == 0 {
            0
        } else if *col == cols - 1 {
            cols - LABEL_W
        } else {
            col - LABEL_W / 2
        };
        for (i, c) in s.chars().enumerate() {
            labels[start + i] = c;
        }
    }
    (marks.into_iter().collect(), labels.into_iter().collect())
}

/// Swap textplots' single `xmin xmax` label line for the tick mark and
/// date label lines built by `x_axis_lines`.
fn replace_x_axis(chart: &str, marks: &str, labels: &str) -> String {
    let body = chart.trim_end_matches('\n');
    let (body, _) = body
        .rsplit_once('\n')
        .expect("chart ends with an x-axis line");
    format!("{body}\n{marks}\n{labels}")
}
/// Sparkline block characters, lowest to highest.
const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// Compact chart for terminals too short for the full output: the braille
/// plot with only the ymax/ymin values labeled — no tick marks, no x axis.
fn mini_chart(
    points: &[Point],
    cols: usize,
    rows: u32,
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
) -> String {
    let data: Vec<(f32, f32)> = points
        .iter()
        .map(|(date, rate)| (days(*date) as f32, *rate as f32))
        .collect();
    let shape = if points.len() == 1 {
        Shape::Points(&data)
    } else {
        Shape::Lines(&data)
    };
    let chart = chart_to_string(
        Chart::new_with_y_range(
            cols as u32 * 2,
            (rows - 1) * 4,
            x_min as f32,
            x_max as f32,
            y_min as f32,
            y_max as f32,
        )
        .y_label_format(LabelFormat::Custom(Box::new(|value| {
            fmt_value(value as f64)
        })))
        .y_tick_display(TickDisplay::None)
        .lineplot(&shape),
    );
    // Drop the x-axis line textplots always appends; the ymin label line
    // and the ymax label on the first braille row stay.
    let body = chart.trim_end_matches('\n');
    let (body, _) = body
        .rsplit_once('\n')
        .expect("chart ends with an x-axis line");
    format!("{body}\n")
}

/// One-line sparkline of the series: one block character per column, each
/// column averaging the points in its x-range (empty columns carry the last
/// value forward), normalized over the data range. Used on terminals too
/// short for the full chart.
fn sparkline(points: &[Point], cols: usize) -> String {
    if cols == 0 {
        return String::new();
    }
    let x_min = days(points[0].0) as f64;
    let x_max = days(points[points.len() - 1].0) as f64;
    let (mut y_min, mut y_max) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), point| {
            (lo.min(point.1), hi.max(point.1))
        });
    if y_max <= y_min {
        y_min -= 1.0;
        y_max += 1.0;
    }
    let mut out = String::with_capacity(cols);
    let mut last = points[0].1;
    let mut i = 0usize;
    for col in 0..cols {
        let lo = x_min + (x_max - x_min) * col as f64 / cols as f64;
        let hi = x_min + (x_max - x_min) * (col + 1) as f64 / cols as f64;
        let mut sum = 0.0;
        let mut n = 0usize;
        while i < points.len() && (days(points[i].0) as f64) < hi {
            if days(points[i].0) as f64 >= lo {
                sum += points[i].1;
                n += 1;
            }
            i += 1;
        }
        let value = if n > 0 { sum / n as f64 } else { last };
        last = value;
        let level = ((value - y_min) / (y_max - y_min) * 7.0)
            .round()
            .clamp(0.0, 7.0) as usize;
        out.push(SPARK[level]);
    }
    out
}

/// Default chart canvas size in terminal columns (dots are twice this) and
/// braille rows: 80×15 keeps the whole output (stats panel + chart + two
/// axis-label lines) at 22 lines; terminals too short for that get a compact
/// label-free chart, or a one-line sparkline on very short terminals.
const CHART_COLS: usize = 80;
const CHART_ROWS: u32 = 15;
/// Columns reserved on the right for the y-axis labels that trail the braille
/// rows, so they never wrap on a terminal exactly as wide as the canvas.
const Y_LABEL_W: usize = 9;

/// Render the series as a text chart: the stats box above a textplots braille
/// chart on an 80×15 canvas (smaller when the terminal cannot fit the whole
/// 22-line output). `cols`/`rows` are the terminal size; the canvas is never
/// wider than the terminal. The x axis maps dates to days since epoch and
/// shows tick marks with date labels at aligned dates (month starts, Mondays,
/// or every few days depending on the range); the y axis shows the cross rate
/// with dense ticks at round values. `color` emits the ANSI bright-black
/// chrome, the bright-cyan plotted line, and the green/red change; callers
/// writing to a file or a pipe must pass `false`.
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
    let chart_cols = CHART_COLS.min(cols.saturating_sub(Y_LABEL_W)).max(16);
    let (panel, panel_rows) = stats_panel(&stats, source, target, chart_cols, color);
    // Terminals too short for the full output (panel + 15 braille rows + two
    // axis-label lines: 22 rows with the standard panel) get a compact chart
    // without axis labels; when even that cannot fit, a one-line sparkline.
    // Two spare rows keep the shell prompt from pushing the panel's top off
    // the screen.
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

    if rows < panel_rows as u32 + CHART_ROWS + 4 {
        let avail = rows.saturating_sub(panel_rows as u32 + 1);
        if avail >= 3 {
            let chart = mini_chart(
                points,
                chart_cols,
                avail.min(9),
                x_min as f64,
                x_max as f64,
                y_min,
                y_max,
            );
            let chart = if color { dim_axes(&chart) } else { chart };
            return format!("{panel}\n{chart}");
        }
        let spark = sparkline(points, chart_cols.min(40));
        let spark = if color {
            format!("{CYAN}{spark}{RESET}")
        } else {
            spark
        };
        return format!("{panel}\n{spark}");
    }

    // Snap the padded range to a ladder of "nice" steps (1/2/5 × 10^k) so
    // the dense y tick labels are round values (7.6, 7.4, ...) instead of
    // arbitrary slices, while the axis still covers the data.
    let dot_height = (CHART_ROWS - 1) * 4;
    let num_steps = dot_height / 8; // TickDisplay::Dense labels every 2nd row
    let mut step = nice_step((y_max - y_min) / num_steps as f64);
    loop {
        let top = (y_max / step).ceil();
        let bottom = (top - num_steps as f64) * step;
        if bottom <= y_min {
            y_min = bottom;
            y_max = top * step;
            break;
        }
        step = nice_step(step * 2.0);
    }

    let data: Vec<(f32, f32)> = points
        .iter()
        .map(|(date, rate)| (days(*date) as f32, *rate as f32))
        .collect();
    let width = chart_cols as u32 * 2;
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
        .y_label_format(LabelFormat::Custom(Box::new(|value| {
            fmt_value(value as f64)
        })))
        .y_tick_display(TickDisplay::Dense)
        .lineplot(&shape),
    );
    let (marks, labels) = x_axis_lines(x_min as f64, x_max as f64, chart_cols);
    let chart = replace_x_axis(&chart, &marks, &labels);
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
        let text = render_text(&points, "USD", "CNY", 80, 24, false);
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
        // panel (2 rules + 3 stat lines) + 15 braille rows + tick marks + labels
        assert_eq!(lines.len(), 22);
        assert!(lines.iter().all(|line| line.chars().count() <= 90));
        // x axis: the short 5-day range gets an intermediate tick every 3 days
        assert_eq!(lines[20].chars().filter(|&c| c == '+').count(), 3);
        assert!(lines[21].contains("2025-01-05"));
    }

    #[test]
    fn text_chart_handles_flat_series() {
        let points = vec![(date("2025-01-02"), 7.3), (date("2025-01-03"), 7.3)];
        let text = render_text(&points, "USD", "CNY", 80, 24, false);
        // The axis snaps to a nice ladder around the flat value: 6.4..7.8
        // in 0.2 steps, with the padded range 7.3 ± 5% inside it.
        assert!(text.contains("7.6"));
        assert!(text.contains("6.4"));
    }

    #[test]
    fn text_chart_handles_single_point() {
        let points = vec![(date("2025-01-02"), 7.3)];
        let text = render_text(&points, "USD", "CNY", 80, 24, false);
        // The x range is widened around the single point.
        assert!(text.contains("2025-01-01"));
        assert!(text.contains("2025-01-03"));
        assert!(has_braille(&text));
    }

    #[test]
    fn x_axis_shows_intermediate_ticks_and_labels() {
        let base = date("2025-01-01");
        let points: Vec<Point> = (0..200)
            .map(|i| {
                (
                    base + chrono::Duration::days(i),
                    7.0 + ((i % 9) as f64) * 0.05,
                )
            })
            .collect();
        let text = render_text(&points, "USD", "CNY", 80, 24, false);
        let lines: Vec<&str> = text.lines().collect();
        let (marks, labels) = (lines[lines.len() - 2], lines[lines.len() - 1]);
        // Month-start ticks inside the range, thinned to 12-column spacing;
        // the 71-column canvas (80 minus the y-label reserve) keeps 03-01,
        // 04-01, 06-01 between the 2025-01-01 and 2025-07-19 endpoints.
        assert_eq!(marks.chars().filter(|&c| c == '+').count(), 5);
        assert_eq!(marks.chars().count(), 71);
        assert_eq!(labels.chars().count(), 71);
        assert!(labels.contains("2025-03-01"));
        assert!(labels.contains("2025-06-01"));
        assert!(labels.contains("2025-01-01")); // endpoints survive
        assert!(labels.contains("2025-07-19"));
    }

    #[test]
    fn display_width_counts_flags_and_wide_emoji_correctly() {
        assert_eq!(display_width("USD/CNY Trend"), 13);
        // A regional-indicator flag pair is the 2 cells terminals render,
        // not 4 from summing per-codepoint widths.
        assert_eq!(display_width("\u{1F1FA}\u{1F1F8}"), 2);
        assert_eq!(display_width("\u{1F1FA}\u{1F1F8} Euro"), 7);
        assert_eq!(display_width("Change: 🔴 -1.24%"), 17);
    }

    #[test]
    fn panel_is_rectangular_with_two_columns() {
        let points = vec![
            (date("2025-01-02"), 7.0),
            (date("2025-01-03"), 7.5),
            (date("2025-01-06"), 6.8),
        ];
        let text = render_text(&points, "USD", "CNY", 80, 24, false);
        let panel: Vec<&str> = text.lines().take(5).collect();
        let width = display_width(panel[0]);
        assert!(panel.iter().all(|line| display_width(line) == width));
        assert!(panel[1].contains(" | ")); // two columns fit the fixed width
        assert!(panel[3].contains("🔴 -2.86%")); // down over the range
    }

    #[test]
    fn chart_has_fixed_size() {
        let base = date("2025-01-02");
        let points: Vec<Point> = (0..60)
            .map(|i| (base + chrono::Duration::days(i), 7.0 + i as f64 * 0.01))
            .collect();
        let text = render_text(&points, "USD", "CNY", 80, 24, false);
        let lines: Vec<&str> = text.lines().collect();
        // panel + 15 braille rows + tick marks + labels
        assert_eq!(lines.len(), 22);
        assert_eq!(
            lines.iter().filter(|line| has_braille(line)).count(),
            CHART_ROWS as usize
        );
        // the y labels trailing the braille rows fit the 80-column terminal
        assert!(lines.iter().all(|line| line.chars().count() <= 80));
    }
    #[test]
    fn small_terminals_get_a_compact_chart() {
        let base = date("2025-01-02");
        let points: Vec<Point> = (0..60)
            .map(|i| (base + chrono::Duration::days(i), 7.0 + i as f64 * 0.01))
            .collect();
        // 21 rows: panel (5) + 9 braille rows + the ymin label line; no x axis
        let small = render_text(&points, "USD", "CNY", 80, 21, false);
        let lines: Vec<&str> = small.lines().collect();
        assert_eq!(lines.len(), 14);
        assert!(has_braille(&small));
        assert!(!lines[5..].iter().any(|line| line.contains('+')));
        assert!(lines[0].contains("USD/CNY Trend"));
        // 8 rows: even the compact chart cannot fit -> one-line sparkline
        let tiny = render_text(&points, "USD", "CNY", 80, 8, false);
        let lines: Vec<&str> = tiny.lines().collect();
        assert_eq!(lines.len(), 6);
        assert!(!has_braille(&tiny));
        assert_eq!(lines[5].chars().count(), 40);
        assert!(lines[5]
            .chars()
            .all(|c| ('\u{2581}'..='\u{2588}').contains(&c)));
        // 23 rows is still too short (panel + 15 + 2 axis lines + 2 spare)
        let tight = render_text(&points, "USD", "CNY", 80, 23, false);
        assert_eq!(tight.lines().count(), 14);
        assert!(has_braille(&tight));
        // 24 rows keeps the full chart
        let full = render_text(&points, "USD", "CNY", 80, 24, false);
        assert_eq!(full.lines().count(), 22);
        assert!(has_braille(&full));
    }

    #[test]
    fn sparkline_tracks_the_series_shape() {
        let base = date("2025-01-02");
        let rising: Vec<Point> = (0..40)
            .map(|i| (base + chrono::Duration::days(i), 7.0 + i as f64 * 0.05))
            .collect();
        let s = sparkline(&rising, 20);
        assert_eq!(s.chars().count(), 20);
        assert!(s.chars().all(|c| ('\u{2581}'..='\u{2588}').contains(&c)));
        assert!(s.chars().last().unwrap() > s.chars().next().unwrap());
        let flat: Vec<Point> = (0..10)
            .map(|i| (base + chrono::Duration::days(i), 7.3))
            .collect();
        let s = sparkline(&flat, 8);
        assert!(s.chars().all(|c| c == s.chars().next().unwrap()));
    }

    #[test]
    fn color_dims_chrome_and_tints_change() {
        let down = vec![(date("2025-01-02"), 7.5), (date("2025-01-03"), 7.0)];
        let text = render_text(&down, "USD", "CNY", 80, 24, true);
        assert!(text.contains("\u{1b}[90m")); // borders and axes are bright black
        assert!(text.contains("\u{1b}[96m")); // the plotted line is bright cyan
        let up = vec![(date("2025-01-02"), 7.0), (date("2025-01-03"), 7.2)];
        assert!(render_text(&up, "USD", "CNY", 80, 24, true).contains("\u{1b}[32m🟢 +2.86%"));
        assert!(!render_text(&up, "USD", "CNY", 80, 24, false).contains('\u{1b}'));
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
