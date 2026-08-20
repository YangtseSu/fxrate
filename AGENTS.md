# Project: huobi — Offline Currency Converter CLI

A Rust offline currency conversion command-line tool with historical
exchange-rate charts.

## Scope

This file documents the current behavior and project conventions.
`src/` is the implementation source of truth; `src/main.rs` is the entry
point, with the code split into modules (`cli`, `current`, `history`,
`provider`, `render`, `series`, `storage`). Verify behavior against the
source and tests when changing this document.

## Tech stack

- Rust with Cargo and standard library plus `reqwest`, `serde`, `serde_json`,
  `chrono`, `rusqlite` (bundled), `csv`, `zip`, `textplots`, and `libc`
- Source: `src/`; manifest: `Cargo.toml`
- Build: `cargo build --release --locked`
- Test: `cargo test --locked` (unit tests in modules + `tests/chart.rs`)
- Cargo build artifact: `target/release/huobi`; Cargo's `target/` directory is gitignored
- `rusqlite` bundled compiles C code, so the build needs a C compiler
  (already required by the `ring` dependency)

## Data source

Two rate providers, selected with `-p/--provider` or the `provider` config
key: `frankfurter` (the default) and `exchange-api`.

- Frankfurter API v2: `GET https://api.frankfurter.dev/v2/rates?base=EUR`
- exchange-api (fawazahmed0/exchange-api):
  `GET https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/eur.min.json`,
  falling back to `https://latest.currency-api.pages.dev/v1/currencies/eur.min.json`
  if the primary endpoint fails

Frankfurter responds with a JSON **array** of rows:
`[{"date","base","quote","rate"}, ...]`; the base currency itself is not
in the list. exchange-api responds with a single JSON object:
`{"date", "eur": {"usd": ..., "jpy": ..., ...}}`; the base currency is
included. Available currencies and counts may change by provider.

Both quote rates as units of the target currency per 1 EUR, so the cached
snapshot has the same shape either way. The full snapshot is cached locally.
All conversions are computed offline from the base-EUR snapshot:
`amount * rate[dst] / rate[src]` — no network request is needed while a
compatible fresh cache is available.

### Historical rates (chart command)

- Source: ECB reference rates, full history CSV:
  `https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip`
  (contains `eurofxref-hist.csv`; dates from 1999, values EUR-based, `N/A`
  for missing entries; old currency columns such as EEK/LTL are dynamic)
- The `huobi chart` command plots from the ECB history, and `huobi` (convert) reads the same ECB history when given `--date`. History is stored per provider in
  SQLite (`history.db`) and is never mixed with the `rates.json` snapshot
- On first chart use (or when the requested `--from`/`--to` range is not
  covered, or with `-u/--update`) the full CSV is downloaded and upserted
  in a transaction; a failed refresh keeps the cached data with a warning
  and exits 1 only when no local history exists
- `history_coverage` records successfully synced ranges so weekend-only
  ranges are not mistaken for missing downloads; no weekend/holiday data
  is fabricated and no interpolation is done
- Chart points are the per-date intersection of both currencies (both must
  have data that day), computed as `EUR→target / EUR→source`; EUR is a
  unity series over the trading-date universe

- `date` = the rates' business date from the API; `fetched_at` = local fetch
  time; both are stored in the cache
- The cache records the provider that produced it. A provider change triggers
  an immediate refresh; legacy caches without provider metadata are treated as
  mismatched. If that refresh fails, the existing stale-cache fallback remains
  available and identifies the cached provider when known

## Storage (XDG/HOME layout)

| Item    | Path |
|---------|------|
| Config  | `$XDG_CONFIG_HOME/huobi/config.json`, then `$HOME/.config/huobi/config.json`, otherwise `./huobi/config.json` |
| Cache   | `$XDG_DATA_HOME/huobi/rates.json`, then `$HOME/.local/share/huobi/rates.json`, otherwise `./huobi/rates.json` |
| History | `$XDG_DATA_HOME/huobi/history.db`, then `$HOME/.local/share/huobi/history.db`, otherwise `./huobi/history.db` (SQLite: `historical_rates`, `history_coverage`) |

- Config is auto-created with defaults on first run if missing
- Cache is written atomically (temp file + rename) to avoid corruption
- History is written through SQLite transactions (upsert, never delete+reinsert)
- Override both with the XDG env vars for isolated tests

### Config schema

```json
{
  "update_interval": "24h",
  "provider": "frankfurter",
  "multi_view": true,
  "currencies": ["USD", "EUR", "GBP", "JPY", "CNY", ...]
}
```

- `update_interval`: duration string such as `24h` or `1h30m`; auto-refresh threshold (default `24h`).
  A missing or empty value uses 24h; malformed or out-of-range values warn and fall back to 24h
- `provider`: rates source, `frankfurter` (default) or `exchange-api`. Invalid
  values warn and fall back to frankfurter; the CLI `-p/--provider` overrides
  the config value
- `multi_view`: whether the default multi-currency list is appended after
  explicit targets. Default `true`; an absent field also means enabled
  (backwards compatible). When no targets are given at all, the
  multi-currency view is always shown regardless of this setting
- `currencies`: default multi-currency view list. Available codes depend on
  the selected provider; unavailable entries warn and are skipped. The
  application does not prevalidate this list against a provider metadata endpoint

## CLI

```
huobi [options] AMOUNT SOURCE [TARGET...]
huobi chart [options] SOURCE TARGET
```

- No targets → **multi-currency view** over the config `currencies` list,
  always shown regardless of `multi_view`
- With targets: explicit targets are shown first (deduped, order preserved).
  When `multi_view` is enabled, the default list follows, separated by a rule
  line; when disabled, only the explicit targets are shown. Currencies
  already shown as explicit targets are not repeated in the default list
- The source currency is always excluded
- `-u`, `--update` — force-refresh rates, ignoring cache age
- `-d`, `--date <date>` — convert using the ECB historical rate for `YYYY-MM-DD` instead of the live snapshot. The `-p/--provider` selection is ignored (historical rates are always ECB). A weekend/holiday date falls back to the previous ECB business day's rate (noted on stderr); a date with no history at all (before the earliest available) is a runtime error (exit 1); an invalid date format is a usage error (exit 2)
- `-p`, `--provider <name>` — rates source override: `frankfurter` (default) or
  `exchange-api`. Unknown values or unknown options are usage errors (exit 2)
- Amounts must parse as finite numbers; missing arguments and invalid amounts
  are usage errors (exit 2). `-h`/`--help` exits 0
- Exit codes: `0` success · `1` runtime error (fetch failed with no cache, unknown source currency, chart with no history data, convert `--date` with no ECB history on or before that date) · `2` usage error

### Chart command

`huobi chart SOURCE TARGET` plots `1 SOURCE = x TARGET` over a date range
using ECB historical reference rates (charts are always EUR-based cross
rates; the convert command and its providers are unaffected).

- `--from <date>` / `--to <date>` — inclusive `YYYY-MM-DD` bounds
  (default: earliest/latest available data). Invalid dates or `from > to`
  are usage errors (exit 2)
- `--format <csv|json|text|auto>` — `auto` (default): text chart when
  stdout is a terminal (or when `--output` is given), CSV otherwise.
  `csv`/`json` emit text (`date,rate` rows / `{source, target, points}`);
  `text` emits the text chart
- `--output <path>` — write the chart to a file instead of stdout
  (never emits terminal escape sequences); with `auto` the file gets the
  text chart
- `-p`, `--provider <name>` — `ecb` only (default); anything else is a
  usage error (exit 2)
- Single trading day → prints `1 SOURCE = x TARGET (date)` instead of a
  chart; an empty range (e.g. a weekend with no data) is a runtime error
  (exit 1); unknown currencies are runtime errors (exit 1)
- Terminal charts: a textplots braille chart, 15 rows tall, sized to the
  terminal width (TIOCGWINSZ, 80×24 fallback). The x axis labels show the
  start/end dates, the y axis labels show rate values

## Behavior

- On startup: if the cache is missing, older than `update_interval`, or was
  produced by a different provider, fetch fresh rates. On fetch failure: keep
  the available cache, print a warning with the failure reason, last rate date,
  and cached provider when known, and continue. Exit 1 only when there is no
  cache at all and the fetch fails
- Force-update failure always exits 1
- stdout: the conversion table plus a footer line with the rate date —
  `rates date <date>`. When rates were refreshed during the run, the footer is
  `rates updated: <date>` instead, so the update status and date appear only
  once, at the bottom
- `--date` convert: opens `history.db` and syncs the ECB full history only when the date is not covered by `history_coverage` (or with `-u/--update`). On sync failure it falls back to cached data and exits 1 only when the date is uncovered and no local history exists. Each currency's rate is read from `historical_rates` for the effective date — the requested date if it has ECB data, otherwise the previous ECB business day (a weekend/holiday falls back, noted on stderr; the rate date shown is the effective day). Conversions use the same `amount * rate[dst] / rate[src]` cross formula, with EUR as unity. Only a date with no history at all (before the earliest available) is a runtime error (exit 1)
- stderr: notices and warnings (skipped currencies, invalid config, failed
  refresh fallback)
- Chart: on startup, sync the ECB full history when the requested range is
  not covered by `history_coverage`, when there is no coverage at all, or
  with `-u/--update`. A failed sync warns and falls back to cached data;
  exit 1 only when no local history exists. Covered ranges never touch the
  network

## Conventions

- User-facing messages are in English (only the human conversation around the
  repo is Chinese)
- Unknown currency in an explicit target → warn and skip (exit 0); the
  remaining valid conversions still print. Unknown source currency → fatal
  (exit 1), since no conversion is possible. Unknown currency in the config
  list → warn and skip
- If none of the explicit targets are valid, the multi-currency view is shown
  as a fallback (same rule as "no targets given")
- Invalid config values degrade to defaults with a warning, never a crash;
  missing or empty `update_interval` is the documented default case
- Currency codes are case-insensitive and normalized to uppercase in memory;
  the existing config file is not rewritten during normalization

## Releasing

Release flow for a new version:

1. **Tag first, PKGBUILD after.** The tag points at the last code commit on
   main; the PKGBUILD bump is a follow-up commit. This order is required: the
   PKGBUILD's `source` is GitHub's archive of the tag, so its sha256 can only be
   computed after the tag exists
2. `git tag vX.Y.Z && git push origin vX.Y.Z` — pushing a `v*` tag triggers
   `.github/workflows/release.yml`, which builds the four platform binaries.
   A new GitHub release gets auto-generated notes; an existing release has its
   assets updated

GitHub access: when `gh` is available, prefer it for everything GitHub —
`gh` for viewing (runs, releases, repos), and `gh auth setup-git` so `git
push` goes over HTTPS through the gh credential helper (SSH on port 22 is
often blocked on restricted networks; `gh auth login --web` stores the token
in the system keyring). `gh release view/upload/create` are also used to
inspect or repair a release. Note: `gh` reads `$XDG_CONFIG_HOME` — do not run
it with a test home's XDG vars exported in the same shell.
3. Download the tag archive and compute the checksum:
   `curl -fsSLo /tmp/huobi.tar.gz https://github.com/YangtseSu/huobi/archive/refs/tags/vX.Y.Z.tar.gz && sha256sum /tmp/huobi.tar.gz`
   — hash the GitHub-served tarball, not a local `git archive` (the PKGBUILD
   downloads from GitHub)
4. Update `packaging/arch/PKGBUILD`:
   - `pkgver=X.Y.Z`, `pkgrel=1` (resets to 1 on every new version)
   - `sha256sums=('<new hash>')` — single entry, replaced in full
   - Leave everything else (pkgname, arch, source URL pattern, build/check/
     package functions) untouched
5. Verify from `packaging/arch/` in a scratch dir: `makepkg -o` (checksum
   check), then a full `makepkg` build to exercise `build`/`check`/`package`
   and confirm the produced `.pkg.tar.zst` contains `/usr/bin/huobi` and the
   LICENSE
6. Commit `packaging: bump PKGBUILD to vX.Y.Z` and push

The PKGBUILD lives at `packaging/arch/` on main, never in the tagged tree.
Key fields: `arch=('x86_64' 'aarch64')`, `license=('GPL-3.0-only')`,
`makedepends=('rust')`, `source=("$url/archive/refs/tags/v$pkgver.tar.gz")`,
`cargo build --release --locked` with `cargo test --locked` as the check step.

## Testing notes

- Run the full suite with `cargo test --locked` (module unit tests plus
  `tests/chart.rs` integration tests, which are fully offline: they seed
  `rates.json` / `history.db` into a fresh XDG home and block the network
  with `HTTPS_PROXY=http://127.0.0.1:9`)
- Offline paths: seed a handcrafted `rates.json` (set `fetched_at` to a stale
  time), include the cached provider when testing provider switching, and block
  the network, e.g. `HTTPS_PROXY=http://127.0.0.1:9` — refresh fails and the
  cache fallback is exercised
- Chart offline paths: seed `history.db` with the same schema the app
  creates (`historical_rates`, `history_coverage`), mark the requested range
  covered, and block the network — a covered range must not attempt a sync
- Historical `--date` convert: a covered date reuses the cache with no network; an uncovered date tries a sync then exits 1 when no local history exists; a weekend/holiday date falls back to the previous business day (noted on stderr, rate date shown is that day); ECB EUR-based cross math matches `chart`
- Fresh XDG dirs simulate a first run: config auto-creation, no-cache
  failure, and no-history failure (chart exits 1 with no local data)
- When changing related behavior, exercise fresh-cache reuse, stale-cache
  refresh, provider-change refresh, explicit-first ordering/dedup, offline
  fallback math, force-update failure, interval handling, invalid config,
  usage errors, chart coverage/upsert/provider isolation, ECB CSV parsing
  (N/A, old currency columns, trailing commas), EUR cross rates, single-day
  charts, and empty ranges

- CI (`.github/workflows/ci.yml`) gates every push/PR on `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo build --release --locked`, and `cargo test --locked`. Before pushing, run `cargo fmt --all` and commit the resulting changes — use `cargo fmt` (not `rustfmt` directly) so the whole crate and the pinned toolchain match CI. Verify with `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings`; an unformatted commit fails the CI format check and is fixed in a follow-up `style:` commit
