# fxrate

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
- Historical conversion: `--date <YYYY-MM-DD>` converts at the ECB historical rate for that day; weekends/holidays fall back to the previous business day
- Historical charts (`fxrate chart`) from ECB reference rates: a stats panel
  (current, high/low with dates, change, average, volatility) above a
  terminal-sized text chart (textplots), plus CSV/JSON output
- XDG-compliant config, cache, and history locations

## Build

Requires Rust and Cargo.

```sh
cargo build --release --locked
```

The binary is `target/release/fxrate`. Arch Linux users can build and install
from `packaging/arch/`:

```sh
cd packaging/arch
makepkg -si
```

## Usage

```
fxrate [options] AMOUNT SOURCE [TARGET...]
fxrate chart [options] SOURCE TARGET
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
- `-d`, `--date <date>` — convert at the ECB historical rate for `YYYY-MM-DD`
  instead of the live snapshot. A weekend/holiday date falls back to the
  previous ECB business day (noted on stderr); a date with no history at all
  is a runtime error (exit 1). The `-p/--provider` selection is ignored
  (historical rates are always ECB). An invalid date is a usage error (exit 2)
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
fxrate 100 USD             # multi-currency view over the configured list
fxrate 100 USD EUR CNY     # EUR and CNY first; with multi_view on, the
                           # default list follows after a blank line + rule
fxrate -u 100 USD          # force-refresh rates, then convert
fxrate -p exchange-api 100 USD EUR   # fetch from exchange-api instead
fxrate --date 2025-01-02 100 USD CNY   # convert at a historical ECB rate

fxrate chart USD CNY --from 2025-01-01 --to 2025-03-31
                           # terminal text chart
fxrate chart USD CNY --from 2025-01-01 --to 2025-03-31 --format csv
fxrate chart USD CNY --from 2025-01-01 --to 2025-03-31 --output chart.txt
```

The first chart run downloads the ECB full history (about 0.6 MB); afterwards
everything works offline. On an interactive terminal the sync shows a spinner
on stderr while it downloads, parses, and imports the rates; when stderr is
piped or redirected nothing is printed. The text chart shows a stats box
above the plot; the chart takes half the terminal height (capped at 25 rows)
and the box is colored (green/red change, bright-black chrome) only on a
terminal — `NO_COLOR` disables it, and file/pipe output is always plain.
Charts have no data for weekends/holidays and are never interpolated. A
single trading day prints `1 SOURCE = x TARGET (date)` instead of a chart; an
empty range (e.g. a weekend with no data) is a runtime error (exit 1).

Output shows the converted amounts and the rates date. Currencies without
rate data are reported on stderr and skipped; valid conversions still print.
Notices and warnings go to stderr. Exit codes: `0` success, `1` runtime
error (for example, fetch failed with no cache or unknown source currency),
and `2` usage error.

## Configuration

Config file: `$XDG_CONFIG_HOME/fxrate/config.json` (default
`~/.config/fxrate/config.json`), auto-created with defaults on first run.

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

Rates cache: `$XDG_DATA_HOME/fxrate/rates.json` (default
`~/.local/share/fxrate/rates.json`), written atomically.

Chart history: `$XDG_DATA_HOME/fxrate/history.db` (default
`~/.local/share/fxrate/history.db`), a SQLite database with the ECB
reference-rate history, written transactionally.

### Migrating from `huobi`

Earlier releases were named `huobi` and stored their state under a `huobi`
directory. `fxrate` does not migrate that data — move it yourself, or let
`fxrate` start fresh:

```sh
mv ~/.config/huobi ~/.config/fxrate           # config.json
mv ~/.local/share/huobi ~/.local/share/fxrate # rates.json, history.db
```

Substitute your own `XDG_CONFIG_HOME` / `XDG_DATA_HOME` if you override them.
Skipping the move is safe: `fxrate` writes a default config and re-downloads the
rates snapshot and the ECB history on first use. Afterwards you can remove the
old `huobi` binary or package.

## Rate providers

Rates are fetched from one of two providers:

- **Frankfurter** (default): `GET https://api.frankfurter.dev/v2/rates?base=EUR`
- **exchange-api**: primary endpoint
  `GET https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/eur.min.json`,
  with `https://latest.currency-api.pages.dev/v1/currencies/eur.min.json` as
  fallback

Available currencies depend on the selected provider and may change.

## Historical rates

- Both `fxrate chart` and `fxrate --date` use the ECB reference rates
([eurofxref-hist.zip](https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip),
updated once per working day around 16:00 CET, for information only).
The full history is synced into `history.db` on first use and refreshed
when a requested date range is not covered locally or with `-u/--update`;
covered ranges never touch the network. Old currency columns and missing
entries (`N/A`) are handled automatically.

## License

GPLv3. See [LICENSE](LICENSE).
