# huobi

Offline currency conversion CLI. Rates are fetched from the [Frankfurter API](https://frankfurter.dev) (v2) and cached locally, so conversions work fully offline once a snapshot is available.

## Features

- Offline conversion from a locally cached rates snapshot (~164 currencies)
- Automatic refresh when the cache is older than a configurable interval (default 24h); a failed refresh falls back to the stale cache with a notice and the last rates date
- `-u` / `--update` to force a refresh
- Multi-currency view by default; explicitly requested targets are listed first
- XDG-compliant config and cache locations

## Build

Requires Go 1.26+ (standard library only, no dependencies).

```sh
go build -o huobi .
```

## Usage

```
huobi [options] AMOUNT SOURCE [TARGET...]
```

Examples:

```sh
huobi 100 USD              # multi-currency view over the configured list
huobi 100 USD EUR CNY      # EUR and CNY first, then the default list
huobi -u 100 USD           # force-refresh rates, then convert
```

Output shows the converted amounts and the rates date. Notices and warnings go to stderr. Exit codes: `0` success, `1` runtime error (e.g. fetch failed with no cache, unknown currency), `2` usage error.

## Configuration

Config file: `$XDG_CONFIG_HOME/huobi/config.json` (default `~/.config/huobi/config.json`), auto-created with defaults on first run.

```json
{
  "update_interval": "24h",
  "currencies": ["USD", "EUR", "GBP", "JPY", "CNY", "..."]
}
```

- `update_interval`: refresh threshold as a Go duration string (default `24h`)
- `currencies`: default multi-currency view list

Rates cache: `$XDG_DATA_HOME/huobi/rates.json` (default `~/.local/share/huobi/rates.json`), written atomically.

## License

GPLv3. See [LICENSE](LICENSE).
