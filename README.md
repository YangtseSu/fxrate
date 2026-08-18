# huobi

Offline currency conversion CLI with local rate caching, so conversions
continue to work without network access once rates are available.

## Features

- Offline conversion from a locally cached rates snapshot
- Automatic refresh when the cache is older than a configurable interval (default 24h); a failed refresh falls back to the stale cache with a notice and the last rates date
- `-u` / `--update` to force a refresh
- `-p` / `--provider <name>` to choose between the built-in rate providers
- Multi-currency view: always shown when no targets are given; optionally appended after explicit targets (`multi_view` config, default on)
- XDG-compliant config and cache locations

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
```

Examples:

```sh
huobi 100 USD              # multi-currency view over the configured list
huobi 100 USD EUR CNY      # EUR and CNY first; with multi_view on, the
                           # default list follows after a blank line + rule
huobi -u 100 USD           # force-refresh rates, then convert
huobi -p exchange-api 100 USD EUR   # fetch from exchange-api instead
```

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

## Rate providers

Rates are fetched from one of two providers:

- **Frankfurter** (default): `GET https://api.frankfurter.dev/v2/rates?base=EUR`
- **exchange-api**: primary endpoint
  `GET https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/eur.min.json`,
  with `https://latest.currency-api.pages.dev/v1/currencies/eur.min.json` as
  fallback

Available currencies depend on the selected provider and may change.

## License

GPLv3. See [LICENSE](LICENSE).
