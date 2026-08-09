# Project: huobi — Offline Currency Converter CLI

A single-file offline currency conversion command-line tool.

## Status

Fully implemented and committed. All originally planned features are done;
the spec below describes the current behavior, not a wishlist.

## Tech stack

- Go 1.26, **standard library only** (no external deps). Build: `go build -o huobi .`
- Single source file: `main.go` (package `main`, module `huobi`)
- Repo is git-managed; the built binary `/huobi` is gitignored

## Data source

- Frankfurter API v2: `GET https://api.frankfurter.dev/v2/rates?base=EUR`
- Response is a JSON **array** of rows: `[{"date","base","quote","rate"}, ...]`
  (~164 currencies; the base currency itself is not in the list)
- The full snapshot is cached locally. All conversions are computed offline from
  the base-EUR snapshot: `amount * rate[dst] / rate[src]` — no per-run network
  calls needed once cached
- `date` = the rates' business date (from the API); `fetched_at` = local fetch
  time; both are stored in the cache

## Storage (XDG layout)

| Item    | Path                                                    |
|---------|---------------------------------------------------------|
| Config  | `$XDG_CONFIG_HOME/huobi/config.json` (default `~/.config/huobi/config.json`) |
| Cache   | `$XDG_DATA_HOME/huobi/rates.json` (default `~/.local/share/huobi/rates.json`) |

- Config is auto-created with defaults on first run if missing
- Cache is written atomically (temp file + rename) to avoid corruption
- Override both with the XDG env vars for isolated tests

### Config schema

```json
{
  "update_interval": "24h",
  "multi_view": true,
  "currencies": ["USD", "EUR", "GBP", "JPY", "CNY", ...]
}
```

- `update_interval`: Go duration string; auto-refresh threshold (default `24h`).
  Invalid values warn and fall back to 24h
- `multi_view`: whether the default multi-currency list is appended after
  explicit targets. Default `true`; an absent field also means enabled
  (backwards compatible). When no targets are given at all, the
  multi-currency view is always shown regardless of this setting
- `currencies`: default multi-currency view list. Note: Frankfurter no longer
  serves BGN — don't add it. Valid codes come from `GET /v2/currencies`

## CLI

```
huobi [options] AMOUNT SOURCE [TARGET...]
```

- No targets → **multi-currency view** over the config `currencies` list,
  always shown regardless of `multi_view`
- With targets: explicit targets are shown first (deduped, order preserved).
  When `multi_view` is enabled, the default list follows, separated by a blank
  line and a rule; when disabled, only the explicit targets are shown.
  Currencies already shown as explicit targets are not repeated in the
  default list
- The source currency is always excluded
- `-u`, `--update` — force-refresh rates, ignoring cache age
- Exit codes: `0` success · `1` runtime error (fetch failed with no cache,
  unknown source currency) · `2` usage error (missing args, bad amount)

## Behavior

- On startup: if the cache is missing or older than `update_interval`, fetch
  fresh rates. On fetch failure: keep stale cache, print a warning with the
  failure reason and the last rate date, and continue. Exit 1 only when there
  is no cache at all and the fetch fails
- Force-update failure always exits 1
- stdout: the conversion table plus a footer line with the rate date —
  `汇率日期 <date>`. When rates were refreshed during the run, the footer is
  `rates updated: <date>` instead, so the update status and the date appear
  only once, at the bottom
- stderr: notices and warnings (skipped currencies, invalid config, failed
  refresh fallback)

## Conventions

- User-facing messages are in English (only the human conversation around the
  repo is Chinese)
- Unknown currency in an explicit target → warn and skip (exit 0); the
  remaining valid conversions still print. Unknown source currency → fatal
  (exit 1), since no conversion is possible. Unknown currency in the config
  list → warn and skip
- If none of the explicit targets are valid, the multi-currency view is shown
  as a fallback (same rule as "no targets given")
- Invalid config values degrade to defaults with a warning, never a crash
- Currency codes are case-insensitive on input, stored uppercase

## Testing notes

- Offline paths: seed a handcrafted `rates.json` (set `fetched_at` to a stale
  time) and block the network, e.g. `HTTPS_PROXY=http://127.0.0.1:9` — the
  fetch fails and the cache fallback kicks in
- Fresh XDG dirs simulate a first run: config auto-creation, no-cache failure
- Verified end to end: live fetch, explicit-first ordering/dedup, offline
  fallback with correct math, force-update failure, interval honoring,
  invalid-interval fallback, usage errors
