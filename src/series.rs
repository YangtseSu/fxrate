// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Time series model and cross-rate math. All series are EUR-based
// (rate = units of the quote currency per 1 EUR); the target series is
// divided by the source series per date, keeping only dates where both
// currencies have data.

use chrono::NaiveDate;

/// A (date, EUR-based rate) point. Dates are sorted ascending.
pub type Point = (NaiveDate, f64);

/// Summary statistics of a date-ascending rate series, shown in the chart's
/// stats panel. `high`/`low` keep the first date the extreme occurred on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub current: f64,
    pub current_date: NaiveDate,
    pub high: f64,
    pub high_date: NaiveDate,
    pub low: f64,
    pub low_date: NaiveDate,
    pub average: f64,
    /// `(last - first) / first * 100`
    pub change_pct: f64,
    /// `(high - low) / average * 100` (amplitude relative to the mean)
    pub volatility_pct: f64,
}

/// Compute summary statistics, or `None` for an empty series.
pub fn stats(points: &[Point]) -> Option<Stats> {
    let first = *points.first()?;
    let last = *points.last()?;
    let mut high = first;
    let mut low = first;
    let mut sum = 0.0f64;
    for &(date, rate) in points {
        if rate > high.1 {
            high = (date, rate);
        }
        if rate < low.1 {
            low = (date, rate);
        }
        sum += rate;
    }
    let average = sum / points.len() as f64;
    // ECB rates are strictly positive, but keep the math safe for a
    // hand-seeded series that starts or averages at zero.
    let change_pct = if first.1 == 0.0 {
        0.0
    } else {
        (last.1 - first.1) / first.1 * 100.0
    };
    let volatility_pct = if average == 0.0 {
        0.0
    } else {
        (high.1 - low.1) / average * 100.0
    };
    Some(Stats {
        current: last.1,
        current_date: last.0,
        high: high.1,
        high_date: high.0,
        low: low.1,
        low_date: low.0,
        average,
        change_pct,
        volatility_pct,
    })
}

/// Compute `target / source` per date over the intersection of both series.
///
/// Dates present in only one series are dropped (no interpolation, no
/// carry-over from previous days). The result is sorted ascending.
pub fn cross_series(source: &[Point], target: &[Point]) -> Vec<Point> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < source.len() && j < target.len() {
        let (source_date, source_rate) = source[i];
        let (target_date, target_rate) = target[j];
        match source_date.cmp(&target_date) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                result.push((source_date, target_rate / source_rate));
                i += 1;
                j += 1;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn x_to_y_uses_both_eur_rates() {
        let source = [
            (date("2025-01-02"), 0.92), // EUR/USD
            (date("2025-01-03"), 0.93),
        ];
        let target = [
            (date("2025-01-02"), 7.8), // EUR/CNY
            (date("2025-01-03"), 7.9),
        ];
        let cross = cross_series(&source, &target);
        assert_eq!(cross.len(), 2);
        assert_eq!(cross[0], (date("2025-01-02"), 7.8 / 0.92));
        assert_eq!(cross[1], (date("2025-01-03"), 7.9 / 0.93));
    }

    #[test]
    fn same_currency_yields_one() {
        let series = [(date("2025-01-02"), 7.8), (date("2025-01-03"), 7.9)];
        let cross = cross_series(&series, &series);
        assert_eq!(cross.len(), 2);
        assert!((cross[0].1 - 1.0).abs() < 1e-12);
        assert!((cross[1].1 - 1.0).abs() < 1e-12);
    }

    #[test]
    fn eur_source_or_target_uses_unity_series() {
        // EUR -> CNY: target series only (source = 1.0 everywhere)
        let one = vec![(date("2025-01-02"), 1.0), (date("2025-01-03"), 1.0)];
        let cny = [(date("2025-01-02"), 7.8), (date("2025-01-03"), 7.9)];
        assert_eq!(
            cross_series(&one, &cny),
            vec![(date("2025-01-02"), 7.8), (date("2025-01-03"), 7.9)]
        );
        // CNY -> EUR: inverse of the target series
        let cross = cross_series(&cny, &one);
        assert!((cross[0].1 - 1.0 / 7.8).abs() < 1e-12);
    }

    #[test]
    fn missing_dates_are_dropped_without_carry_over() {
        let source = [
            (date("2025-01-02"), 0.92),
            (date("2025-01-03"), 0.93),
            (date("2025-01-06"), 0.94),
        ];
        let target = [(date("2025-01-02"), 7.8), (date("2025-01-06"), 7.9)];
        let cross = cross_series(&source, &target);
        assert_eq!(
            cross,
            vec![
                (date("2025-01-02"), 7.8 / 0.92),
                (date("2025-01-06"), 7.9 / 0.94)
            ]
        );
    }

    #[test]
    fn empty_series_yields_empty_cross() {
        assert!(cross_series(&[], &[(date("2025-01-02"), 7.8)]).is_empty());
        assert!(cross_series(&[(date("2025-01-02"), 0.92)], &[]).is_empty());
        assert!(cross_series(&[], &[]).is_empty());
    }

    #[test]
    fn input_series_need_not_be_aligned() {
        let source = [(date("2025-01-01"), 1.0), (date("2025-01-04"), 2.0)];
        let target = [(date("2025-01-02"), 10.0), (date("2025-01-04"), 20.0)];
        assert_eq!(
            cross_series(&source, &target),
            vec![(date("2025-01-04"), 10.0)]
        );
    }

    #[test]
    fn stats_track_extremes_with_dates() {
        let points = vec![
            (date("2025-01-02"), 7.0),
            (date("2025-01-03"), 8.0),
            (date("2025-01-06"), 7.5),
        ];
        let s = stats(&points).unwrap();
        assert_eq!(s.current, 7.5);
        assert_eq!(s.current_date, date("2025-01-06"));
        assert_eq!((s.high, s.high_date), (8.0, date("2025-01-03")));
        assert_eq!((s.low, s.low_date), (7.0, date("2025-01-02")));
        assert!((s.average - 7.5).abs() < 1e-12);
        assert!((s.change_pct - (0.5 / 7.0 * 100.0)).abs() < 1e-9);
        assert!((s.volatility_pct - (1.0 / 7.5 * 100.0)).abs() < 1e-9);
    }

    #[test]
    fn stats_first_occurrence_wins_for_tied_extremes() {
        let points = vec![
            (date("2025-01-02"), 8.0),
            (date("2025-01-03"), 8.0),
            (date("2025-01-06"), 6.0),
            (date("2025-01-07"), 6.0),
        ];
        let s = stats(&points).unwrap();
        assert_eq!(s.high_date, date("2025-01-02"));
        assert_eq!(s.low_date, date("2025-01-06"));
    }

    #[test]
    fn stats_on_single_point_and_empty() {
        assert!(stats(&[]).is_none());
        let s = stats(&[(date("2025-01-02"), 7.4)]).unwrap();
        assert_eq!(s.change_pct, 0.0);
        assert_eq!(s.volatility_pct, 0.0);
        assert_eq!((s.high, s.low, s.current, s.average), (7.4, 7.4, 7.4, 7.4));
    }
}
