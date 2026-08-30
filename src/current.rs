// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Latest-rates fetching (Frankfurter / exchange-api) and conversion math.

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::io::Read;
use std::sync::LazyLock;
use std::time::Duration;

use crate::provider::Provider;

pub const FRANKFURTER_URL: &str = "https://api.frankfurter.dev/v2/rates?base=EUR";
pub const EXCHANGE_API_URL: &str =
    "https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/eur.min.json";
pub const EXCHANGE_API_FALLBACK_URL: &str =
    "https://latest.currency-api.pages.dev/v1/currencies/eur.min.json";
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
/// Cap for the small JSON rate responses (convert command).
pub const MAX_RESPONSE_SIZE: usize = 1 << 20;
/// Cap for the ECB full-history zip; the CSV alone is ~2 MiB, so 16 MiB
/// leaves ample headroom without admitting absurd memory spikes.
pub const MAX_HISTORY_RESPONSE_SIZE: usize = 16 << 20;

#[derive(Debug, Deserialize, Serialize)]
pub struct RateSnapshot {
    pub base: String,
    pub date: String,
    pub fetched_at: DateTime<Utc>,
    #[serde(default)]
    pub provider: String,
    pub rates: HashMap<String, f64>,
}

#[derive(Debug, Deserialize)]
struct ApiRate {
    date: String,
    base: String,
    quote: String,
    rate: f64,
}

/// Fetch a URL with a response size cap, returning an error for
/// non-success statuses and oversized bodies. The body is streamed under a
/// hard cap so a response that lies about its Content-Length never buffers
/// fully in memory.
/// The process-wide HTTP client: one connection pool, built once, so
/// repeat fetches (e.g. the exchange-api fallback) never pay client
/// construction or a fresh TLS backend setup again.
fn http_client() -> &'static Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| {
        Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("HTTP client construction with only a timeout cannot fail")
    });
    &CLIENT
}

/// Fetch a URL with a response size cap, returning an error for
/// non-success statuses and oversized bodies. The body is streamed under a
/// hard cap so a response that lies about its Content-Length never buffers
/// fully in memory.
pub fn get_bytes(url: &str, max_size: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let response = http_client().get(url).send()?;
    let status = response.status();
    // Reject an announced oversized body before reading anything, then
    // stream with a hard cap: at most max_size + 1 bytes are ever buffered.
    if response
        .content_length()
        .is_some_and(|length| length > max_size as u64)
    {
        return Err(crate::boxed_error(format!(
            "API response exceeded {} bytes",
            max_size
        )));
    }
    let mut bytes = Vec::new();
    response.take(max_size as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_size {
        return Err(crate::boxed_error(format!(
            "API response exceeded {} bytes",
            max_size
        )));
    }
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes);
        return Err(crate::boxed_error(format!(
            "API returned {status}: {}",
            body.trim()
        )));
    }
    Ok(bytes)
}

fn fetch_frankfurter_rates() -> Result<RateSnapshot, Box<dyn Error>> {
    let bytes = get_bytes(FRANKFURTER_URL, MAX_RESPONSE_SIZE)?;
    let rows: Vec<ApiRate> = serde_json::from_slice(&bytes)
        .map_err(|error| crate::boxed_error(format!("failed to parse API response: {error}")))?;
    let first = rows
        .first()
        .ok_or_else(|| crate::boxed_error("API response contained no rates"))?;
    let base = first.base.to_uppercase();
    let date = first.date.clone();
    let rates = rows
        .into_iter()
        .map(|row| (row.quote.to_uppercase(), row.rate))
        .collect();
    Ok(RateSnapshot {
        base,
        date,
        fetched_at: Utc::now(),
        provider: String::new(),
        rates,
    })
}

pub fn parse_exchange_api(bytes: &[u8]) -> Result<RateSnapshot, Box<dyn Error>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| crate::boxed_error(format!("failed to parse API response: {error}")))?;
    let date = value
        .get("date")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| crate::boxed_error("API response missing date"))?
        .to_owned();
    let map = value
        .get("eur")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| crate::boxed_error("API response missing base currency rates"))?;
    let rates = map
        .iter()
        .map(|(code, rate)| {
            let rate = rate
                .as_f64()
                .ok_or_else(|| crate::boxed_error(format!("invalid rate for currency {code}")))?;
            Ok((code.to_uppercase(), rate))
        })
        .collect::<Result<_, Box<dyn Error>>>()?;
    Ok(RateSnapshot {
        base: "EUR".to_owned(),
        date,
        fetched_at: Utc::now(),
        provider: String::new(),
        rates,
    })
}

fn fetch_exchange_api_rates() -> Result<RateSnapshot, Box<dyn Error>> {
    let bytes = match get_bytes(EXCHANGE_API_URL, MAX_RESPONSE_SIZE) {
        Ok(bytes) => bytes,
        Err(primary_error) => {
            get_bytes(EXCHANGE_API_FALLBACK_URL, MAX_RESPONSE_SIZE).map_err(|fallback_error| {
                crate::boxed_error(format!(
                    "primary endpoint failed ({primary_error}); fallback endpoint failed ({fallback_error})"
                ))
            })?
        }
    };
    parse_exchange_api(&bytes)
}

pub fn fetch_rates(provider: Provider) -> Result<RateSnapshot, Box<dyn Error>> {
    let mut snapshot = match provider {
        Provider::Frankfurter => fetch_frankfurter_rates(),
        Provider::ExchangeApi => fetch_exchange_api_rates(),
        Provider::Ecb => return Err(crate::boxed_error("ECB has no latest-rate endpoint")),
    }?;
    snapshot.provider = provider.name().to_owned();
    Ok(snapshot)
}

pub fn currency_rate(snapshot: &RateSnapshot, currency: &str) -> Result<f64, Box<dyn Error>> {
    if currency == snapshot.base {
        return Ok(1.0);
    }
    snapshot.rates.get(currency).copied().ok_or_else(|| {
        crate::boxed_error(format!(
            "no rate for currency {currency} (rates date {})",
            snapshot.date
        ))
    })
}

pub fn convert(
    snapshot: &RateSnapshot,
    source: &str,
    target: &str,
    amount: f64,
) -> Result<f64, Box<dyn Error>> {
    let source_rate = currency_rate(snapshot, source)?;
    let target_rate = currency_rate(snapshot, target)?;
    Ok(amount * target_rate / source_rate)
}

pub fn cache_needs_refresh(
    snapshot: Option<&RateSnapshot>,
    provider: Provider,
    interval: Duration,
) -> bool {
    snapshot.is_none_or(|snapshot| {
        snapshot.provider != provider.name()
            || Utc::now()
                .signed_duration_since(snapshot.fetched_at)
                .to_std()
                .map(|age| age > interval)
                .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn exchange_api_payload_is_parsed() {
        let payload = br#"{"date":"2026-08-17","eur":{"usd":1.17,"jpy":190.5,"eur":1.0}}"#;
        let snapshot = parse_exchange_api(payload).unwrap();
        assert_eq!(snapshot.base, "EUR");
        assert_eq!(snapshot.date, "2026-08-17");
        assert_eq!(snapshot.rates.get("USD"), Some(&1.17));
        assert_eq!(snapshot.rates.get("JPY"), Some(&190.5));
    }

    #[test]
    fn exchange_api_payload_missing_rates_is_rejected() {
        let payload = br#"{"date":"2026-08-17"}"#;
        assert!(parse_exchange_api(payload).is_err());
    }

    #[test]
    fn cache_refreshes_when_provider_changes() {
        let snapshot = RateSnapshot {
            base: "EUR".to_owned(),
            date: "2026-08-17".to_owned(),
            fetched_at: Utc::now(),
            provider: "frankfurter".to_owned(),
            rates: HashMap::new(),
        };
        let interval = Duration::from_secs(3600);

        assert!(!cache_needs_refresh(
            Some(&snapshot),
            Provider::Frankfurter,
            interval
        ));
        assert!(cache_needs_refresh(
            Some(&snapshot),
            Provider::ExchangeApi,
            interval
        ));

        let mut legacy = snapshot;
        legacy.provider.clear();
        assert!(cache_needs_refresh(
            Some(&legacy),
            Provider::Frankfurter,
            interval
        ));
    }

    #[test]
    fn ecb_is_not_a_convert_provider() {
        assert!(fetch_rates(Provider::Ecb).is_err());
    }

    /// Serve one canned HTTP response on a random localhost port. Proxy env
    /// vars are scrubbed first: reqwest would otherwise honor a developer's
    /// HTTP_PROXY and route even localhost through it.
    fn serve(response: Vec<u8>) -> String {
        for variable in [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            std::env::remove_var(variable);
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream.write_all(&response).unwrap();
        });
        format!("http://127.0.0.1:{port}/capped")
    }

    #[test]
    fn oversized_content_length_is_rejected_before_reading() {
        let url = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\n".to_vec());
        let error = get_bytes(&url, 1024).unwrap_err().to_string();
        assert!(error.contains("exceeded 1024 bytes"), "{error}");
    }

    #[test]
    fn streaming_cap_stops_a_chunked_flood() {
        // No Content-Length: the streaming cap is the only bound. A chunked
        // body far above the cap must stop reading at max_size + 1 bytes.
        let mut response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1000\r\n".to_vec();
        response.extend(std::iter::repeat_n(b'x', 4096));
        response.extend_from_slice(b"\r\n0\r\n\r\n");
        let url = serve(response);
        let error = get_bytes(&url, 1024).unwrap_err().to_string();
        assert!(error.contains("exceeded 1024 bytes"), "{error}");
    }

    #[test]
    fn small_responses_stream_unchanged() {
        let url = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec());
        assert_eq!(get_bytes(&url, 1024).unwrap(), b"{}");
    }
}
