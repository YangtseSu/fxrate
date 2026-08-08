# huobi

Offline currency conversion CLI. Rates are fetched from the [Frankfurter API](https://frankfurter.dev) (v2) and cached locally, so conversions work fully offline once a snapshot is available.

## Features

- Offline conversion from a locally cached rates snapshot (~164 currencies)
- Automatic refresh when the cache is older than a configurable interval (default 24h); a failed refresh falls back to the stale cache with a notice and the last rates date
- `-u` / `--update` to force a refresh
- Multi-currency view: always shown when no targets are given; optionally
  appended after explicit targets (`multi_view` config, default on)
- XDG-compliant config and cache locations

## Build

Requires Go 1.26+ (standard library only, no dependencies).

```sh
go build -o huobi .
```

Arch Linux: build and install from the bundled `PKGBUILD` with `makepkg -si`
in the repo root.

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
```

Output shows the converted amounts and the rates date. Currencies without
rate data are reported on stderr and skipped; the valid conversions still
print. Notices and warnings go to stderr. Exit codes: `0` success, `1` runtime
error (e.g. fetch failed with no cache, unknown source currency), `2` usage
error.

## Configuration

Config file: `$XDG_CONFIG_HOME/huobi/config.json` (default `~/.config/huobi/config.json`), auto-created with defaults on first run.

```json
{
  "update_interval": "24h",
  "multi_view": true,
  "currencies": ["USD", "EUR", "GBP", "JPY", "CNY", "..."]
}
```

- `update_interval`: refresh threshold as a Go duration string (default `24h`)
- `multi_view`: show the default list after explicit targets (default `true`).
  With no targets, the multi-currency view is always shown
- `currencies`: default multi-currency view list

Rates cache: `$XDG_DATA_HOME/huobi/rates.json` (default `~/.local/share/huobi/rates.json`), written atomically.

## License

GPLv3. See [LICENSE](LICENSE).
