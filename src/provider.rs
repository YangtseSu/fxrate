// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su

/// Rate source identifiers shared by the convert and chart commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Frankfurter,
    ExchangeApi,
    Ecb,
}

impl Provider {
    pub fn from_name(name: &str) -> Option<Provider> {
        match name.to_ascii_lowercase().as_str() {
            "frankfurter" => Some(Provider::Frankfurter),
            "exchange-api" | "exchangeapi" => Some(Provider::ExchangeApi),
            "ecb" => Some(Provider::Ecb),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Provider::Frankfurter => "frankfurter",
            Provider::ExchangeApi => "exchange-api",
            Provider::Ecb => "ecb",
        }
    }

    /// Comma-separated names of `providers`, for "valid: …" error messages.
    pub fn names(providers: &[Provider]) -> String {
        providers
            .iter()
            .map(|provider| provider.name())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Providers accepted by the convert command (`-p/--provider`).
    pub const CONVERT_PROVIDERS: [Provider; 2] = [Provider::Frankfurter, Provider::ExchangeApi];

    /// Providers accepted by the chart command.
    pub const CHART_PROVIDERS: [Provider; 1] = [Provider::Ecb];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_are_case_insensitive() {
        assert_eq!(
            Provider::from_name("frankfurter"),
            Some(Provider::Frankfurter)
        );
        assert_eq!(
            Provider::from_name("FRANKFURTER"),
            Some(Provider::Frankfurter)
        );
        assert_eq!(
            Provider::from_name("Exchange-Api"),
            Some(Provider::ExchangeApi)
        );
        assert_eq!(
            Provider::from_name("exchangeapi"),
            Some(Provider::ExchangeApi)
        );
        assert_eq!(Provider::from_name("ecb"), Some(Provider::Ecb));
        assert_eq!(Provider::from_name("fixer"), None);
    }

    #[test]
    fn chart_providers_are_ecb_only() {
        assert_eq!(Provider::CHART_PROVIDERS, [Provider::Ecb]);
    }

    #[test]
    fn names_lists_convert_providers_for_messages() {
        assert_eq!(
            Provider::names(&Provider::CONVERT_PROVIDERS),
            "frankfurter, exchange-api"
        );
        assert_eq!(Provider::names(&Provider::CHART_PROVIDERS), "ecb");
    }
}
