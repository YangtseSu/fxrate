# huobi

Offline currency conversion CLI with local rate caching, so conversions
continue to work without network access once rates are available. Also
plots historical exchange-rate charts from ECB reference rates, fully
offline after the first sync.

## Features

- Offline conversion from a locally cached rates snapshot
- Automatic refresh when the cache is older than a configurable interval (default 24h); a failed refresh falls back to the stale cache with a notice and the last rates date
- `-u` / `--update` to force a refresh
- `-p` / `--provider <name>` to choose between the built-in rate providers
- Multi-currency view: always shown when no targets are given; optionally appended after explicit targets (`multi_view` config, default on)
- Historical charts (`huobi chart`) from ECB reference rates: a terminal
  text chart (textplots), plus CSV/JSON output
- XDG-compliant config, cache, and history locations

## Build

Requires Rust and Cargo.

```sh
cargo build --release --locked
```

The binary is `target/release/huobi`. Arch Linux users can build and install
from `packaging/arch/`:

```sh
cd packaging/arch
makepkg -si
```

## Usage

```
huobi [options] AMOUNT SOURCE [TARGET...]
huobi chart [options] SOURCE TARGET
```

### convert

Convert `AMOUNT` units of `SOURCE` into one or more `TARGET` currencies using
the locally cached base-EUR rates snapshot. Conversions are fully offline once
the snapshot is available.

Positional arguments:

- `AMOUNT` — amount to convert; a finite number (e.g. `100`, `12.5`). Required
- `SOURCE` — source currency code (case-insensitive, e.g. `USD`). Required
- `TARGET...` — one or more target currency codes. If omitted, the
  multi-currency view over the configured `currencies` list is shown

Options:

- `-u`, `--update` — force a refresh of the rates snapshot, ignoring cache age
  (a failed force-refresh exits 1)
- `-p`, `--provider <name>` — rates source override: `frankfurter` (default)
  or `exchange-api`. Unknown providers are a usage error (exit 2); this
  overrides the config `provider` value
- `-h`, `--help` — show the convert usage and exit 0

### chart

Plots `1 SOURCE = x TARGET` over a date range using ECB reference rates
(charts are always EUR-based cross rates; the convert providers are unaffected).

Positional arguments:

- `SOURCE` — source currency code (case-insensitive, e.g. `USD`). Required
- `TARGET` — target currency code (case-insensitive, e.g. `CNY`). Exactly one;
  extra positional arguments are a usage error (exit 2)

Options:

- `--from <date>` — inclusive start date, `YYYY-MM-DD`. Default: earliest
  available data
- `--to <date>` — inclusive end date, `YYYY-MM-DD`. Default: latest available
  data. Must not be earlier than `--from` (otherwise a usage error, exit 2)
- `--format <format>` — `csv`, `json`, `text`, or `auto` (default). `auto`
  emits a text chart on a TTY (or when `--output` is given) and CSV otherwise
- `--output <path>` — write the chart to a file instead of stdout; with
  `auto` the file receives the text chart (never terminal escape sequences)
- `-p`, `--provider <name>` — history source; only `ecb` (default) is
  accepted. Anything else is a usage error (exit 2)
- `-u`, `--update` — force re-download of the ECB full history
- `-h`, `--help` — show the chart usage and exit 0

Examples:

```sh
huobi 100 USD              # multi-currency view over the configured list
huobi 100 USD EUR CNY      # EUR and CNY first; with multi_view on, the
                           # default list follows after a blank line + rule
huobi -u 100 USD           # force-refresh rates, then convert
huobi -p exchange-api 100 USD EUR   # fetch from exchange-api instead

huobi chart USD CNY --from 2025-01-01 --to 2025-03-31
                           # terminal text chart
huobi chart USD CNY --from 2025-01-01 --to 2025-03-31 --format csv
huobi chart USD CNY --from 2025-01-01 --to 2025-03-31 --output chart.txt
```

The first chart run downloads the ECB full history (about 0.6 MB); afterwards
everything works offline. Charts have no data for weekends/holidays and are
never interpolated. A single trading day prints `1 SOURCE = x TARGET (date)`
instead of a chart; an empty range (e.g. a weekend with no data) is a runtime
error (exit 1).

Output shows the converted amounts and the rates date. Currencies without
rate data are reported on stderr and skipped; valid conversions still print.
Notices and warnings go to stderr. Exit codes: `0` success, `1` runtime
error (for example, fetch failed with no cache or unknown source currency),
and `2` usage error.

## Configuration

Config file: `$XDG_CONFIG_HOME/huobi/config.json` (default
`~/.config/huobi/config.json`), auto-created with defaults on first run.

```json
{
  "update_interval": "24h",
  "provider": "frankfurter",
  "multi_view": true,
  "currencies": ["USD", "EUR", "GBP", "JPY", "CNY", "..."]
}
```

- `update_interval`: duration string such as `24h` or `1h30m` (default `24h`)
- `provider`: rates source, `frankfurter` (default) or `exchange-api`. Invalid
  values warn and fall back to `frankfurter`; the CLI `-p` / `--provider`
  overrides the config value. Changing provider triggers an immediate refresh
- `multi_view`: show the default list after explicit targets (default `true`).
  With no targets, the multi-currency view is always shown
- `currencies`: default multi-currency view list

Rates cache: `$XDG_DATA_HOME/huobi/rates.json` (default
`~/.local/share/huobi/rates.json`), written atomically.

Chart history: `$XDG_DATA_HOME/huobi/history.db` (default
`~/.local/share/huobi/history.db`), a SQLite database with the ECB
reference-rate history, written transactionally.

## Rate providers

Rates are fetched from one of two providers:

- **Frankfurter** (default): `GET https://api.frankfurter.dev/v2/rates?base=EUR`
- **exchange-api**: primary endpoint
  `GET https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/eur.min.json`,
  with `https://latest.currency-api.pages.dev/v1/currencies/eur.min.json` as
  fallback

Available currencies depend on the selected provider and may change.

## Historical rates

Charts use the ECB reference rates
([eurofxref-hist.zip](https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip),
updated once per working day around 16:00 CET, for information only).
The full history is synced into `history.db` on first use and refreshed
when a requested date range is not covered locally or with `-u/--update`;
covered ranges never touch the network. Old currency columns and missing
entries (`N/A`) are handled automatically.

## License

GPLv3. See [LICENSE](LICENSE).
