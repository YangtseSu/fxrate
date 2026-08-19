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
}
