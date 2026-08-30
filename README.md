# fxrate

Offline currency conversion CLI with local rate caching, so conversions
continue to work without network access once rates are available. Also
plots historical exchange-rate charts from ECB reference rates, fully
offline after the first sync.

## Features

- Offline conversion from a locally cached rates snapshot
- Automatic refresh when the cache is older than a configurable interval
  (default 24h); a failed refresh falls back to the cached rates
- `-u` / `--update` to force a refresh
- `-p` / `--provider <name>` to choose between the built-in rate providers
- Multi-currency view: always shown when no targets are given; optionally appended after explicit targets (`multi_view` config, default on)
- Historical conversion: `--date <YYYY-MM-DD>` converts at the ECB rate for
  that day (weekends/holidays fall back to the previous business day)
- Historical charts (`fxrate chart`) from ECB reference rates: a stats panel
  (current, high/low with dates, change, average, volatility) above a
  fixed-size text chart (textplots), plus CSV/JSON output
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

Options may appear on either side of the `chart` subcommand
(`fxrate -u chart USD CNY` forces the history re-download).

### convert

Convert `AMOUNT` units of `SOURCE` into one or more `TARGET` currencies using
the locally cached base-EUR rates snapshot. Conversions are fully offline once
the snapshot is available.

- `AMOUNT` — amount to convert; a finite number (e.g. `100`, `12.5`), which
  may be negative (e.g. `-100`). Required

- `TARGET...` — one or more target currency codes. If omitted, the
  multi-currency view over the configured `currencies` list is shown

Options:

- `-u`, `--update` — force a refresh of the rates snapshot, ignoring cache age
  (a failed force-refresh exits 1)
- `-p`, `--provider <name>` — `frankfurter` (default) or `exchange-api`;
  overrides the config value
- `-d`, `--date <date>` — convert at the ECB historical rate for
  `YYYY-MM-DD`; weekends/holidays fall back to the previous business day,
  and the provider selection is ignored. An invalid date is a usage error
  (exit 2)
- `-h`, `--help` — show the convert usage and exit 0
- `-V`, `--version` — print the version and exit 0

### chart

Plots `1 SOURCE = x TARGET` over a date range using ECB reference rates
(charts are always EUR-based cross rates; the convert providers are unaffected).

Positional arguments:

- `SOURCE` — source currency code (case-insensitive, e.g. `USD`). Required
- `TARGET` — target currency code (case-insensitive, e.g. `CNY`). Exactly one;
  extra positional arguments are a usage error (exit 2)

Options:

- `--from <date>` / `--to <date>` — inclusive `YYYY-MM-DD` bounds. Default:
  earliest / latest available data, with the end extended to the cached
  snapshot's date when reachable. Out-of-range bounds are clamped with a
  stderr note, or rejected (exit 1) when nothing can be charted
- `--format <format>` — `csv`, `json`, `text`, or `auto` (default). `auto`
  emits a text chart on a TTY (or when `--output` is given) and CSV otherwise
- `--output <path>` — write the chart to a plain-text file instead of stdout
- `-p`, `--provider <name>` — history source; only `ecb` (default)
- `-u`, `--update` — force re-download of the ECB full history
- `-h`, `--help` — show the chart usage and exit 0
- `-V`, `--version` — print the version and exit 0

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

The first chart run downloads the ECB full history; afterwards charts work
offline. On an interactive terminal the sync shows a spinner on stderr.
The text chart shows a stats box above the plot and is colored only on a
terminal (`NO_COLOR` turns colors off; file and pipe output stay plain).
Charts have no data for weekends/holidays and are never interpolated. When
the cached rates snapshot reaches the chart's right edge, that point comes
from the snapshot and is marked `, live`. A single trading day prints
`1 SOURCE = x TARGET (date)` instead of a chart.

Unknown currencies are warned about and skipped; valid conversions still
print, and notices go to stderr. Exit codes: `0` success, `1` runtime
error, `2` usage error.

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

- `update_interval`: duration string such as `24h`, `90m`, `1h30m`, or `7d`
  (units: `ns`, `us`, `ms`, `s`, `m`, `h`, `d`; default `24h`)
- `provider`: `frankfurter` (default) or `exchange-api`; the CLI `-p`
  overrides it, and changing it triggers an immediate refresh
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
Skipping the move is safe: `fxrate` starts fresh and re-downloads on first use.

## Rate providers

Rates come from one of two providers: **Frankfurter** (default) or
**exchange-api** (with a fallback endpoint). Available currencies depend on
the selected provider and may change.

## Historical rates

Both `fxrate chart` and `fxrate --date` use the
[ECB reference rates](https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip),
updated once per working day. The full history is synced into `history.db`
on first use and only re-downloaded when a requested range is not covered
or with `-u/--update`. Missing entries and old currency columns are handled
automatically.

## License

GPLv3. See [LICENSE](LICENSE).
